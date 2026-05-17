//! Offline arrangement rendering for MCP analysis tools.
//!
//! Renders an arrangement range (a `[start_tick, end_tick)` slice of the
//! song) to an in-memory stereo f32 buffer, with no real-time playback.
//! Mirrors the multi-instrument layout of the live engine: snapshots all
//! instruments from the live engine state, builds an isolated offline
//! `SynthEngine`, loads every instrument's patch into it, attaches a
//! shared `Song` reference, seeks to the requested start tick, and runs
//! the sequencer-driven engine forward by exactly the number of frames
//! that the tick range spans.
//!
//! Used by the `analyze_mix_bus` and `analyze_section` MCP tools to obtain
//! a deterministic, fast (faster-than-real-time) rendering of the master
//! bus output of a song region.
//!
//! **Limitations (v1):**
//! - No per-track stems. The output is the master mix only.
//! - Notes that started before `start_tick` are not pre-rolled. A range
//!   that begins mid-note will not produce sound for that note. Surrounded
//!   ranges are rendered exactly; choose ranges that start on note
//!   boundaries for fidelity.

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::commands::{InstrumentParam, PortId};
use synth_engine::instrument::MidiChannel;
use synth_engine::{EngineCommand, SynthEngine};
use synth_sequencer::{Song, Tick};

use synth_mcp::error::McpBridgeError;

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Sample rate of the offline render. Matches the single-note preview path.
const RENDER_SAMPLE_RATE: u32 = 44100;

/// Block size in frames per `engine.process()` call.
const BUFFER_SIZE: usize = 256;

/// Output channel count — always stereo.
const CHANNELS: usize = 2;

/// Hard ceiling on how many seconds an arrangement render may produce, to
/// keep the MCP request bounded. 5 minutes at 44.1 kHz stereo ≈ 105 MB f32
/// — comfortably above any reasonable analysis window.
const MAX_RENDER_SECONDS: f32 = 300.0;

/// Fixed seed for `fastrand`'s thread-local RNG at the start of every offline
/// render. Several DSP modules pull from `fastrand` during processing —
/// noise/LFO-S&H output, oscillator phase randomization on note-on, mechanical
/// noise, drift. Without a deterministic seed, two consecutive renders of the
/// same song-state diverge by enough to mask sub-dB mix differences.
/// `fastrand`'s free functions use a thread-local RNG, so reseeding here
/// doesn't disturb the audio thread.
pub(crate) const OFFLINE_RENDER_SEED: u64 = 0x5EED_5EED_5EED_5EED;

/// Upper bound on how long the voice-bleed drain in `render_range` may run
/// between two consecutive renders, in milliseconds. The drain exits early as
/// soon as a fully-silent block is produced (see `DRAIN_SILENCE_EPSILON`).
/// 400 ms covers the 50 ms envelope release used by the determinism test
/// suite with margin while still capping the per-iteration cost in the per-
/// track `analyze_section` loop.
const VOICE_DRAIN_MAX_MS: f32 = 400.0;

/// Per-sample amplitude under which the drain block is considered silent for
/// the purpose of the early-exit. Tight enough that residual filter / delay
/// energy does not contaminate the next render's first sample at the precision
/// the determinism tests assert (f32 bit-exact).
const DRAIN_SILENCE_EPSILON: f32 = 1e-7;

/// Build an [`AudioCallbackContext`] for a full-buffer offline `engine.process`
/// call. Used for the drain, warm-up, and main-render contexts; the latter
/// passes a real `sample_position` / `stream_time` to advance audio time, the
/// other two pass `u64::MAX` / `0.0` to mark the block as non-time-advancing.
fn offline_callback_ctx(
    frames: usize,
    sample_position: u64,
    stream_time: f64,
) -> AudioCallbackContext {
    AudioCallbackContext {
        sample_rate: HwSampleRate(RENDER_SAMPLE_RATE),
        frames,
        channels: CHANNELS as u16,
        stream_time,
        sample_position,
        output_latency: synth_core::Seconds::ZERO,
    }
}

/// Output of [`render_arrangement_to_buffer`].
pub struct RenderedArrangement {
    /// Stereo-interleaved f32 samples (L0, R0, L1, R1, ...).
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Total render duration in seconds.
    pub duration_seconds: f32,
    /// Channel count (always 2).
    pub channels: u16,
    /// Tick range that was rendered (echoed back for caller convenience).
    pub start_tick: u64,
    pub end_tick: u64,
    /// Non-fatal warnings emitted during the render — failed module loads,
    /// missing patterns, oversize ranges that were clamped, etc. Empty when
    /// the render was clean.
    pub warnings: Vec<String>,
}

/// Render an arrangement range to a stereo f32 buffer.
///
/// `start_tick` is inclusive, `end_tick` is exclusive. Both are absolute
/// song ticks. The output buffer covers exactly the wall-clock duration
/// between those two ticks at the song's tempo (honouring tempo changes
/// inside the range).
pub fn render_arrangement_to_buffer(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
) -> Result<RenderedArrangement, McpBridgeError> {
    render_arrangement_to_buffer_with_song(
        session,
        sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
}

/// Render an arrangement range against an explicit `Song` handle.
///
/// Identical to [`render_arrangement_to_buffer`] except the sequencer source is
/// supplied directly. Used by `analyze_section` to render per-track soloed
/// variants from a cloned `Song` without mutating the live shared instance.
///
/// Thin wrapper: builds a single [`OfflineEngineSession`] and renders one range.
/// Callers that need N renders against the same engine state (per-track loop in
/// `analyze_section`) should construct an [`OfflineEngineSession`] directly and
/// call [`OfflineEngineSession::render_range`] N times instead — that amortizes
/// the engine + instrument-load cost across the loop.
pub(crate) fn render_arrangement_to_buffer_with_song(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    song: &Arc<RwLock<Song>>,
    start_tick: u64,
    end_tick: u64,
) -> Result<RenderedArrangement, McpBridgeError> {
    let (mut sess, setup_warnings) = OfflineEngineSession::new(session, sample_library)?;
    let mut rendered = sess.render_range(song, start_tick, end_tick)?;
    // Preserve the pre-§7.1 contract: setup warnings appear in the single
    // render's `warnings` vec. The session API itself returns them separately.
    if !setup_warnings.is_empty() {
        let mut combined = setup_warnings;
        combined.append(&mut rendered.warnings);
        rendered.warnings = combined;
    }
    Ok(rendered)
}

/// Owns an offline `SynthEngine` plus a fully-loaded module graph, so the
/// expensive setup (snapshotting live instruments, creating modules,
/// applying parameters / connections / sample data) runs once and is
/// amortized across N render calls.
///
/// The intended call pattern is:
///
/// ```ignore
/// let mut sess = OfflineEngineSession::new(&session, &sample_library)?;
/// for tick_range in ranges {
///     let rendered = sess.render_range(&song_arc, tick_range.start, tick_range.end)?;
///     // analyse rendered.samples …
/// }
/// ```
///
/// Each `render_range` call reseeds `fastrand`, sends `Stop` (to drop any
/// voices still ringing from the previous render), re-attaches the song, and
/// runs Play → Seek → process. The dual-oscillator `arrangement_render_determinism`
/// test enforces that consecutive `render_range` calls are bit-exact.
pub struct OfflineEngineSession {
    engine: SynthEngine,
    handle: synth_engine::EngineHandle,
    /// True once the first `render_range` call has finished its warm-up
    /// process. Gates two related behaviors: subsequent calls skip the warm-up
    /// block AND run the voice-bleed drain instead (a freshly-built engine has
    /// no prior voices to drain).
    first_call_done: bool,
}

impl OfflineEngineSession {
    /// Snapshot the live instruments, build a fresh offline engine, load every
    /// instrument's module graph + parameters + sample data, and start the
    /// audio stream. Does not attach a song or play anything — that's done per
    /// [`render_range`](Self::render_range).
    ///
    /// Returns the session paired with any warnings collected during the
    /// instrument-load setup (failed module adds, missing patterns, etc.).
    /// Those are surfaced once, by the caller — they describe the *engine*
    /// state, not any one render's tick range, so callers running a per-track
    /// loop should emit them outside the per-track prefix scheme.
    pub fn new(
        session: &SynthSession,
        sample_library: &crate::audio::preview::SharedSampleLibrary,
    ) -> Result<(Self, Vec<String>), McpBridgeError> {
        let engine_state = session.state();
        let live_instruments: Vec<synth_engine::shared_state::InstrumentSnapshot> = engine_state
            .instrument_snapshots
            .read()
            .iter()
            .cloned()
            .collect();
        if live_instruments.is_empty() {
            return Err(McpBridgeError::Other(
                "No instruments loaded — nothing to render".to_string(),
            ));
        }

        let (mut engine, mut handle) = SynthEngine::new();
        let tmp_session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

        let mut setup_warnings: Vec<String> = Vec::new();

        // Use each instrument's live ID so the sequencer's SeqInstrumentId →
        // InstrumentId mapping survives into the offline engine.
        for inst_snap in &live_instruments {
            if let Err(e) = tmp_session.add_instrument_with_id(inst_snap.id, &inst_snap.name) {
                setup_warnings.push(format!(
                    "arrangement_render: failed to add instrument {}: {e}",
                    inst_snap.id.as_u64()
                ));
                continue;
            }
            tmp_session.reset_counters_for_instrument(inst_snap.id);

            load_instrument_into_offline(
                inst_snap,
                engine_state,
                &tmp_session,
                &mut handle,
                sample_library,
                &mut setup_warnings,
            );
        }

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate(RENDER_SAMPLE_RATE),
            buffer_size: synth_core::BufferSize(BUFFER_SIZE as u32),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);

        Ok((
            Self {
                engine,
                handle,
                first_call_done: false,
            },
            setup_warnings,
        ))
    }

    /// Render one `[start_tick, end_tick)` range against `song`. Safe to call
    /// repeatedly on the same session — reseeds `fastrand`, flushes voice
    /// state via `Stop`, and re-attaches the song each call so consecutive
    /// renders are bit-exact regardless of what the previous render did.
    pub fn render_range(
        &mut self,
        song: &Arc<RwLock<Song>>,
        start_tick: u64,
        end_tick: u64,
    ) -> Result<RenderedArrangement, McpBridgeError> {
        if end_tick <= start_tick {
            return Err(McpBridgeError::Other(format!(
                "Arrangement range invalid: end_tick ({end_tick}) must be greater than start_tick ({start_tick})"
            )));
        }

        fastrand::seed(OFFLINE_RENDER_SEED);

        let mut warnings: Vec<String> = Vec::new();

        // Wall-clock duration of the tick range, computed via the song's own
        // tick→second conversion so tempo changes inside the range are honoured.
        let duration_seconds = {
            let song_read = song.read();
            let start_s = song_read.tick_to_seconds(Tick(start_tick));
            let end_s = song_read.tick_to_seconds(Tick(end_tick));
            let dur = (end_s - start_s).max(0.0) as f32;
            if dur > MAX_RENDER_SECONDS {
                warnings.push(format!(
                    "Requested arrangement range is {dur:.1}s; clamping to {MAX_RENDER_SECONDS:.0}s",
                ));
                MAX_RENDER_SECONDS
            } else {
                dur
            }
        };

        if duration_seconds <= 0.0 {
            return Err(McpBridgeError::Other(
                "Arrangement range resolves to zero render duration — check tempo settings"
                    .to_string(),
            ));
        }

        let total_frames =
            (f64::from(duration_seconds) * f64::from(RENDER_SAMPLE_RATE)).ceil() as u64;
        if total_frames == 0 {
            return Err(McpBridgeError::Other(
                "Arrangement range too short to produce any samples".to_string(),
            ));
        }

        // Drop any voices still active from a previous render. No-op on a
        // freshly-built session. The PREVIOUS `render_range` does not send
        // its own trailing Stop — letting this Stop do double duty (flushing
        // both the previous render's voices and any state from this one if it
        // bails early) saves one queue push per call.
        self.handle.send_blocking(EngineCommand::Stop);

        let mut block = vec![0.0f32; BUFFER_SIZE * CHANNELS];

        // Voice-bleed drain: `Stop` flips active envelopes into release but
        // does not advance them — without this, the next render's first
        // sample inherits whatever amplitude the released voice still holds.
        // Process silent blocks (sequencer Stopped → song does not advance)
        // until the output goes fully silent, capped at `VOICE_DRAIN_MAX_MS`.
        // Long reverb / delay tails on real-world patches may not fully
        // converge inside the cap — bit-exactness across renders is best-
        // effort for those, but RMS / LUFS metrics on the per-track soloed
        // renders that `analyze_section` returns are unaffected by sub-ms
        // residual filter state.
        if self.first_call_done {
            let drain_ctx = offline_callback_ctx(BUFFER_SIZE, u64::MAX, 0.0);
            let max_blocks = ((VOICE_DRAIN_MAX_MS / 1000.0) * RENDER_SAMPLE_RATE as f32
                / BUFFER_SIZE as f32)
                .ceil() as usize;
            for _ in 0..max_blocks {
                block.fill(0.0);
                self.engine.process(&mut block, &drain_ctx);
                if block.iter().all(|s| s.abs() < DRAIN_SILENCE_EPSILON) {
                    break;
                }
            }
        }

        // The supplied Song is read-only from the sequencer's perspective
        // (`SequencerEngine` only does try_read), so handing an Arc to the
        // offline engine is safe even when it points at the live shared
        // instance. Re-sent every call so callers may render against different
        // songs (or the same song with mutated solo flags) without restart.
        self.handle.send_blocking(EngineCommand::SetSong {
            song: Arc::clone(song),
        });

        // Sentinel sample_position in the warm-up block keeps the engine from
        // seeing a duplicate position 0 when the real render begins. Only
        // needed on the first call — subsequent renders inherit the same
        // warmed-up engine.
        if !self.first_call_done {
            let warmup_ctx = offline_callback_ctx(BUFFER_SIZE, u64::MAX, 0.0);
            block.fill(0.0);
            self.engine.process(&mut block, &warmup_ctx);
            self.first_call_done = true;
        }

        // Play, then Seek — in that order. Play transitions the sequencer from
        // Stopped → Playing, which has the side effect of resetting current_tick
        // to zero (see `SequencerEngine::play`). Sending Seek *after* Play
        // overrides that reset so the real render starts at `start_tick`.
        // Both commands drain together at the top of the first real process
        // call below, so the sequencer hasn't advanced yet when Seek lands.
        self.handle.send_blocking(EngineCommand::Play);
        self.handle.send_blocking(EngineCommand::Seek {
            tick: Tick(start_tick),
        });

        let mut samples: Vec<f32> = Vec::with_capacity((total_frames as usize) * CHANNELS);
        let mut frames_written: u64 = 0;

        while frames_written < total_frames {
            let remaining = (total_frames - frames_written) as usize;
            let this_buffer = remaining.min(BUFFER_SIZE);
            let sample_count = this_buffer * CHANNELS;

            block[..sample_count].fill(0.0);

            let context = offline_callback_ctx(
                this_buffer,
                frames_written,
                frames_written as f64 / f64::from(RENDER_SAMPLE_RATE),
            );

            self.engine.process(&mut block[..sample_count], &context);
            samples.extend_from_slice(&block[..sample_count]);
            frames_written += this_buffer as u64;
        }

        // No trailing Stop: the next `render_range` issues its own Stop at the
        // top, which handles both flushing this render's voices and any prior
        // state. Dropping the session without a follow-up render is also fine
        // — the engine cleans up its own state on drop.

        Ok(RenderedArrangement {
            samples,
            sample_rate: RENDER_SAMPLE_RATE,
            duration_seconds,
            channels: CHANNELS as u16,
            start_tick,
            end_tick,
            warnings,
        })
    }
}

/// Load one instrument's voice graph + effect chain into the offline engine.
///
/// Mirrors the per-instrument load logic from
/// `crate::audio::preview::render_note_to_buffer`, but writes into the
/// offline engine under the live instrument's own `InstrumentId` (instead of
/// `InstrumentId::FIRST`) so the sequencer's SeqInstrumentId → engine ID
/// mapping survives.
fn load_instrument_into_offline(
    inst_snap: &synth_engine::shared_state::InstrumentSnapshot,
    engine_state: &synth_engine::EngineState,
    tmp_session: &SynthSession,
    handle: &mut synth_engine::EngineHandle,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    warnings: &mut Vec<String>,
) {
    let instrument_id = inst_snap.id;

    let modules = engine_state
        .shared_graph
        .get_modules_for_instrument(instrument_id);
    let connections = engine_state
        .shared_graph
        .get_connections_for_instrument(instrument_id);

    if modules.is_empty() {
        warnings.push(format!(
            "arrangement_render: instrument {} has no modules — silent in render",
            instrument_id.as_u64()
        ));
        return;
    }

    let effect_chain_order: Vec<synth_engine::commands::ModuleId> =
        inst_snap.effect_chain_order.clone();

    let apply_module_state = |handle: &mut synth_engine::EngineHandle,
                              module_snap: &synth_engine::ModuleStateSnapshot,
                              descriptor: &synth_core::ModuleDescriptor,
                              warnings: &mut Vec<String>| {
        let module_id = module_snap.id;
        let is_effect = module_id.module_type.is_effect();
        for desc_param in &descriptor.parameters {
            if let Some(ep) = module_snap
                .parameters
                .iter()
                .find(|p| p.same_kind(&desc_param.id))
            {
                let param = desc_param.id.with_f32(ep.as_f32());
                let sent = if is_effect {
                    handle.send_blocking(EngineCommand::SetEffectParameter {
                        instrument_id: Some(instrument_id),
                        module_id,
                        param,
                    })
                } else {
                    handle.send_blocking(EngineCommand::SetModuleParameter {
                        instrument_id: Some(instrument_id),
                        module_id,
                        param,
                    })
                };
                if !sent {
                    warnings.push(format!(
                        "arrangement_render: failed to enqueue parameter for module {module_id}"
                    ));
                }
            }
        }
        let is_bypassed = matches!(module_snap.bypass_state, synth_core::BypassState::Bypassed);
        if is_bypassed {
            if is_effect {
                handle.send_blocking(EngineCommand::SetEffectEnabled {
                    instrument_id: Some(instrument_id),
                    module_id,
                    enabled: false,
                });
            } else {
                handle.send_blocking(EngineCommand::SetBypass {
                    instrument_id: Some(instrument_id),
                    module: module_id,
                    bypass: true,
                });
            }
        }
    };

    let mut voice_modules: Vec<&synth_engine::ModuleStateSnapshot> = Vec::new();
    let mut effect_modules: std::collections::HashMap<
        synth_engine::commands::ModuleId,
        &synth_engine::ModuleStateSnapshot,
    > = std::collections::HashMap::new();
    for m in &modules {
        if m.module_type.is_visualizer() {
            continue;
        }
        if m.module_type.is_effect() {
            effect_modules.insert(m.id, m);
        } else {
            voice_modules.push(m);
        }
    }

    for module_snap in &voice_modules {
        let module_id = module_snap.id;
        let descriptor =
            match tmp_session.add_module_with_id(instrument_id, module_id, module_id.module_type) {
                Ok(d) => d,
                Err(e) => {
                    warnings.push(format!(
                        "arrangement_render: failed to add module {module_id}: {e}"
                    ));
                    continue;
                }
            };
        apply_module_state(handle, module_snap, &descriptor, warnings);
    }

    crate::audio::preview::load_sample_data_for_samplers(
        handle,
        instrument_id,
        &voice_modules,
        sample_library,
        warnings,
    );

    let mut handled: std::collections::HashSet<synth_engine::commands::ModuleId> =
        std::collections::HashSet::new();
    for module_id in &effect_chain_order {
        if let Some(module_snap) = effect_modules.get(module_id) {
            handled.insert(*module_id);
            let descriptor = match tmp_session.add_module_with_id(
                instrument_id,
                *module_id,
                module_id.module_type,
            ) {
                Ok(d) => d,
                Err(e) => {
                    warnings.push(format!(
                        "arrangement_render: failed to add effect {module_id}: {e}"
                    ));
                    continue;
                }
            };
            apply_module_state(handle, module_snap, &descriptor, warnings);
        }
    }
    for (module_id, module_snap) in &effect_modules {
        if handled.contains(module_id) {
            continue;
        }
        warnings.push(format!(
            "arrangement_render: effect {module_id} present but missing from chain order — appending"
        ));
        let descriptor = match tmp_session.add_module_with_id(
            instrument_id,
            *module_id,
            module_id.module_type,
        ) {
            Ok(d) => d,
            Err(e) => {
                warnings.push(format!(
                    "arrangement_render: failed to add effect {module_id}: {e}"
                ));
                continue;
            }
        };
        apply_module_state(handle, module_snap, &descriptor, warnings);
    }

    for conn in &connections {
        if conn.from_module.module_type.is_visualizer()
            || conn.to_module.module_type.is_visualizer()
        {
            continue;
        }
        let sent = handle.send_blocking(EngineCommand::Connect {
            instrument_id: Some(instrument_id),
            from: PortId::new(conn.from_module, conn.from_port),
            to: PortId::new(conn.to_module, conn.to_port),
        });
        if !sent {
            warnings.push(format!(
                "arrangement_render: failed to enqueue connection {} → {}",
                conn.from_module, conn.to_module
            ));
        }
    }

    // Mirror live enable/mix state. `inst_snap.enabled` already encodes the
    // muted+enabled live behavior (an instrument muted live is reported as
    // disabled), so we forward it directly. Track-level mutes inside the
    // arrangement are honored by the shared sequencer automatically.
    handle.send_blocking(EngineCommand::SetInstrumentEnabled {
        instrument_id,
        enabled: inst_snap.enabled && !inst_snap.muted,
    });
    // MIDI channel only affects external MIDI input, which an offline render
    // doesn't have — default to channel 1 for all instruments.
    handle.send_blocking(EngineCommand::SetInstrumentMidiChannel {
        instrument_id,
        channel: MidiChannel::CH1,
    });
    handle.send_blocking(EngineCommand::SetInstrumentParameter {
        instrument_id,
        param: InstrumentParam::Volume(inst_snap.volume),
    });
    handle.send_blocking(EngineCommand::SetInstrumentParameter {
        instrument_id,
        param: InstrumentParam::Pan(inst_snap.pan),
    });
    handle.send_blocking(EngineCommand::SetInstrumentParameter {
        instrument_id,
        param: InstrumentParam::Solo(inst_snap.solo),
    });
}
