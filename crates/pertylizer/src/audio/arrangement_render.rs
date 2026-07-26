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
//! - Notes that started before `start_tick` are pre-rolled by seeking the
//!   offline engine to the earliest crossing note's start (capped by
//!   `MAX_PREROLL_SECONDS`) and trimming the prefix from the output. Pre-roll
//!   that would push total render time above `MAX_RENDER_SECONDS` is shrunk
//!   to fit, with a warning. Notes that started further back than the cap are
//!   silent for the duration of the render.

use std::sync::Arc;

use synth_core::ModuleParam;
use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, DenormalGuard};
use synth_engine::commands::{InstrumentParam, PortId};
use synth_engine::instrument::MidiChannel;
use synth_engine::{EngineCommand, SynthEngine};
use synth_sequencer::{Song, Tick};

use synth_mcp::AnalysisScope;
use synth_mcp::error::McpBridgeError;

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Block size in frames per `engine.process()` call.
const BUFFER_SIZE: usize = 256;

/// Output channel count — always stereo.
const CHANNELS: usize = 2;

/// Hard ceiling on how many seconds an arrangement render may produce, to
/// keep the MCP request bounded. 5 minutes at 44.1 kHz stereo ≈ 105 MB f32
/// — comfortably above any reasonable analysis window.
const MAX_RENDER_SECONDS: f32 = 300.0;

/// Maximum amount of pre-roll (audio before the requested `start_tick`) that
/// the renderer will run to seed sustained notes that started before the
/// range. The prefix is rendered and then discarded; this cap keeps an extreme
/// drone or 10-minute pad from forcing an analyze_section call to render the
/// entire song. Notes that started further back than this are silent in the
/// output — same as the pre-pre-roll behaviour, but with a warning.
const MAX_PREROLL_SECONDS: f32 = 30.0;

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
    sample_rate: u32,
) -> AudioCallbackContext {
    AudioCallbackContext {
        sample_rate: HwSampleRate::new(sample_rate),
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
        AnalysisScope::default(),
    )
}

/// Like [`render_arrangement_to_buffer`] but reconstructs the optional signal
/// stages requested by `scope` (master effects, return-bus effects, …) so the
/// analysis hears more than the dry instrument sum. `AnalysisScope::default()`
/// behaves identically to [`render_arrangement_to_buffer`].
pub fn render_arrangement_to_buffer_with_scope(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
    scope: AnalysisScope,
) -> Result<RenderedArrangement, McpBridgeError> {
    render_arrangement_to_buffer_with_song(
        session,
        sample_library,
        &shared.song,
        start_tick,
        end_tick,
        scope,
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
    song: &Arc<synth_sequencer::SharedSong>,
    start_tick: u64,
    end_tick: u64,
    scope: AnalysisScope,
) -> Result<RenderedArrangement, McpBridgeError> {
    let (mut sess, setup_warnings) =
        OfflineEngineSession::new_with_scope(session, sample_library, scope)?;
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
/// Each `render_range` call sends `Stop` (to drop any
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
    /// Which optional signal stages this session reconstructs (master/return
    /// effects). Fixed at construction so every `render_range` call on the
    /// session — including the per-track soloed renders — uses the same scope.
    scope: AnalysisScope,
    /// Master effect chain captured from the live engine at construction time.
    /// Replayed (via `ClearMasterEffects` + re-add) each `render_range` so a
    /// reused session rebuilds fresh, zero-state master effects per render
    /// instead of carrying tails across soloed per-track renders. Empty unless
    /// `scope.master_effects`.
    master_effect_snapshot: Vec<synth_engine::shared_state::ReturnEffectSnapshot>,
    /// Return-bus effect chains captured from the live engine at construction
    /// time. Replayed each `render_range` because `ClearReturnBusses` wipes the
    /// offline buses' chains. Empty unless `scope.return_effects`.
    return_effect_snapshots: Vec<synth_engine::shared_state::ReturnBusSnapshot>,
    /// Optional cap on how many leading master effects `render_range` loads.
    /// `None` (default) loads the full chain. Used by `analyze_master_chain` to
    /// measure the master output after each effect by rendering successive
    /// chain prefixes. Only meaningful with `scope.master_effects`.
    master_effect_prefix: Option<usize>,
    /// Render sample rate (from `scope.render_sample_rate`). Baked into the
    /// engine's stream at construction, so it is fixed for the session's life.
    sample_rate: u32,
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
        Self::new_with_scope(session, sample_library, AnalysisScope::default())
    }

    /// Like [`new`](Self::new) but reconstructs the optional signal stages named
    /// by `scope`. Master and return effect chains are snapshotted here and
    /// replayed fresh per [`render_range`](Self::render_range) so a reused
    /// session never carries effect state across renders. With
    /// `AnalysisScope::default()` this is identical to [`new`](Self::new).
    pub fn new_with_scope(
        session: &SynthSession,
        sample_library: &crate::audio::preview::SharedSampleLibrary,
        scope: AnalysisScope,
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

        // Start the audio stream *before* loading instruments so we can drain the
        // command ring by processing silent blocks during the load (see the loop
        // below), and so the render sample rate is stamped onto each instrument's
        // modules at add time — matching the live app, where instruments are
        // always added after the stream has started.
        let sample_rate = scope.render_sample_rate;
        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(sample_rate),
            buffer_size: synth_core::BufferSize::new(BUFFER_SIZE as u32),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        let mut drain_buf = vec![0.0f32; BUFFER_SIZE * CHANNELS];
        // Non-time-advancing sentinel position: a pure command-drain block. No
        // song is attached yet and the sequencer is Stopped, so it advances no
        // audio; it only lets the engine drain its queued load commands.
        let drain_ctx = offline_callback_ctx(BUFFER_SIZE, u64::MAX, 0.0, sample_rate);

        // Use each instrument's live ID so the sequencer's InstrumentId →
        // InstrumentId mapping survives into the offline engine. Drain the
        // command ring after each instrument so a many-instrument project cannot
        // overflow the fixed 256-slot ring — nothing else drains it during setup,
        // and `send_blocking` drops (rather than blocks) once the ring is full.
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
                &mut engine,
                &mut drain_buf,
                &drain_ctx,
            );
            // Apply this instrument's remaining commands before the next one.
            crate::audio::drain_command_queue(&mut engine, &mut drain_buf, &drain_ctx);
        }

        // Capture the master + return effect chains here, but replay them per
        // `render_range` (not once) so a session reused across per-track soloed
        // renders rebuilds *fresh* effect instances each time. Loading master
        // effects once and leaving them resident would let stateful DSP (reverb
        // / delay / compressor tails) bleed from one soloed track's render into
        // the next, making per-track metrics order-dependent.
        let master_effect_snapshot = if scope.master_effects {
            engine_state.master_effects.read().clone()
        } else {
            Vec::new()
        };

        let return_effect_snapshots = if scope.return_effects {
            engine_state.return_bus_effects.read().clone()
        } else {
            Vec::new()
        };

        // The master fader is always in the live output path, so offline
        // analysis must reflect it — without this, `set_master_volume` has no
        // effect on `analyze_mix_bus` / `analyze_section` metrics. Sent once;
        // it persists across `render_range` calls (no Clear command resets it).
        // Drains at the first `render_range` warm-up (one command — no overflow).
        handle.send_blocking(EngineCommand::SetMasterVolume(synth_core::Gain::new(
            engine_state.master_volume.load(),
        )));

        Ok((
            Self {
                engine,
                handle,
                first_call_done: false,
                scope,
                master_effect_snapshot,
                return_effect_snapshots,
                master_effect_prefix: None,
                sample_rate,
            },
            setup_warnings,
        ))
    }

    /// Limit how many leading master effects the next `render_range` calls load,
    /// so a caller can measure the master output after a chain prefix. `Some(k)`
    /// loads the first `k` effects (clamped to the chain length); `None` (the
    /// default) restores the full chain. No effect unless the session was built
    /// with `scope.master_effects`.
    pub fn set_master_effect_prefix(&mut self, prefix: Option<usize>) {
        self.master_effect_prefix = prefix;
    }

    /// Render one `[start_tick, end_tick)` range against `song`. Safe to call
    /// repeatedly on the same session — flushes voice
    /// state via `Stop`, and re-attaches the song each call so consecutive
    /// renders are bit-exact regardless of what the previous render did.
    pub fn render_range(
        &mut self,
        song: &Arc<synth_sequencer::SharedSong>,
        start_tick: u64,
        end_tick: u64,
    ) -> Result<RenderedArrangement, McpBridgeError> {
        self.render_range_with_tail(song, start_tick, end_tick, synth_core::Seconds::ZERO)
    }

    /// Render a range, then stop the transport at `end_tick` and capture an
    /// additional release/effect tail without triggering later arrangement
    /// events.
    pub fn render_range_with_tail(
        &mut self,
        song: &Arc<synth_sequencer::SharedSong>,
        start_tick: u64,
        end_tick: u64,
        tail: synth_core::Seconds,
    ) -> Result<RenderedArrangement, McpBridgeError> {
        if end_tick <= start_tick {
            return Err(McpBridgeError::Other(format!(
                "Arrangement range invalid: end_tick ({end_tick}) must be greater than start_tick ({start_tick})"
            )));
        }

        // Flush denormals (FTZ/DAZ) for the whole offline render, matching the
        // real-time audio callback. Without this an offline render can diverge
        // from live playback at the denormal level (recursive-filter tails) and
        // pay the same denormal-slowdown the live path avoids. Restored on
        // return by RAII.
        let _denormal_guard = DenormalGuard::new();

        let mut warnings: Vec<String> = Vec::new();

        let (visible_seconds, prefix_seconds, effective_start_tick) = {
            let song_read = song.read();
            let start_s = song_read.tick_to_seconds(Tick(start_tick));
            let end_s = song_read.tick_to_seconds(Tick(end_tick));
            let raw_effective_s =
                song_read.tick_to_seconds(earliest_active_note_start(&song_read, Tick(start_tick)));
            let raw_prefix_s = (start_s - raw_effective_s).max(0.0);

            let prefix_s = raw_prefix_s.min(f64::from(MAX_PREROLL_SECONDS));
            if raw_prefix_s > f64::from(MAX_PREROLL_SECONDS) {
                warnings.push(format!(
                    "Pre-roll requested {raw_prefix_s:.1}s; capping at {MAX_PREROLL_SECONDS:.0}s. \
                     Notes that began earlier are silent in the output."
                ));
            }

            let raw_visible_s = (end_s - start_s).max(0.0);
            let max_visible_s = f64::from(MAX_RENDER_SECONDS) - prefix_s;
            let visible_s = if raw_visible_s > max_visible_s {
                warnings.push(format!(
                    "Requested arrangement range is {raw_visible_s:.1}s; clamping to {max_visible_s:.1}s \
                     (pre-roll uses {prefix_s:.1}s of the {MAX_RENDER_SECONDS:.0}s render budget).",
                ));
                max_visible_s.max(0.0)
            } else {
                raw_visible_s
            };

            // `Song::seconds_to_tick` is the exact tempo-aware inverse of
            // `tick_to_seconds`, so the Seek lands on the tick whose wall-clock
            // position is `prefix_s` before `start_tick`.
            let effective_tick = if prefix_s > 0.0 {
                song_read.seconds_to_tick(start_s - prefix_s)
            } else {
                Tick(start_tick)
            };

            (visible_s as f32, prefix_s as f32, effective_tick)
        };

        if visible_seconds <= 0.0 {
            return Err(McpBridgeError::Other(
                "Arrangement range resolves to zero render duration — check tempo settings"
                    .to_string(),
            ));
        }

        let visible_frames =
            (f64::from(visible_seconds) * f64::from(self.sample_rate)).ceil() as u64;
        let prefix_frames =
            (f64::from(prefix_seconds) * f64::from(self.sample_rate)).round() as u64;
        let tail_frames =
            u64::try_from(tail.to_samples(synth_core::SampleRate::new(self.sample_rate as f32)))
                .unwrap_or(u64::MAX);
        let stop_frame = prefix_frames + visible_frames;
        let total_frames = stop_frame.saturating_add(tail_frames);
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
            let drain_ctx = offline_callback_ctx(BUFFER_SIZE, u64::MAX, 0.0, self.sample_rate);
            let max_blocks = ((VOICE_DRAIN_MAX_MS / 1000.0) * self.sample_rate as f32
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
        // Ship the Mod Grid runtime for this song so control-rate modulation is
        // present in the offline render (matches the live engine's pre-pass).
        let mod_grid = crate::mod_grid_build::build_mod_grid_runtime(&song.read());
        self.handle.send_blocking(EngineCommand::SetModGrid {
            runtime: Box::new(mod_grid),
        });

        // Reconstruct the song's return-bus channels so sends route correctly
        // in the offline render. Reset first so channels from a prior render of
        // a different song don't linger (this session is reused across calls).
        // Faders are read live from the song. Return-bus effect chains are
        // reconstructed only when `scope.return_effects` is set (replayed every
        // render because `ClearReturnBusses` wipes them); otherwise offline
        // renders hear the dry-summed returns.
        self.handle.send_blocking(EngineCommand::ClearReturnBusses);
        for bus in song.read().return_busses() {
            self.handle
                .send_blocking(EngineCommand::CreateReturnBus { id: bus.id });
        }
        if self.scope.return_effects {
            load_return_effects_into_offline(
                &mut self.handle,
                &self.return_effect_snapshots,
                &mut warnings,
            );
        }
        // Rebuild the master chain fresh every render (`load_master_effects_into_offline`
        // begins with `ClearMasterEffects`), so a reused session starts each
        // soloed render with zero-state master effects — no tail bleed between
        // tracks. Cheap relative to the render itself.
        if self.scope.master_effects {
            let len = self.master_effect_snapshot.len();
            let prefix = self.master_effect_prefix.unwrap_or(len).min(len);
            load_master_effects_into_offline(
                &mut self.handle,
                &self.master_effect_snapshot[..prefix],
                &mut warnings,
            );
        }

        // Sentinel sample_position in the warm-up block keeps the engine from
        // seeing a duplicate position 0 when the real render begins. Only
        // needed on the first call — subsequent renders inherit the same
        // warmed-up engine.
        if !self.first_call_done {
            let warmup_ctx = offline_callback_ctx(BUFFER_SIZE, u64::MAX, 0.0, self.sample_rate);
            block.fill(0.0);
            self.engine.process(&mut block, &warmup_ctx);
            self.first_call_done = true;
        }

        // Play, then Seek — in that order. Play transitions the sequencer from
        // Stopped → Playing, which has the side effect of resetting current_tick
        // to zero (see `SequencerEngine::play`). Sending Seek *after* Play
        // overrides that reset so the real render starts at `effective_start_tick`.
        // Both commands drain together at the top of the first real process
        // call below, so the sequencer hasn't advanced yet when Seek lands.
        self.handle.send_blocking(EngineCommand::Play);
        self.handle.send_blocking(EngineCommand::Seek {
            tick: effective_start_tick,
        });

        let mut samples: Vec<f32> = Vec::with_capacity((total_frames as usize) * CHANNELS);
        let mut frames_written: u64 = 0;
        let mut tail_started = false;

        while frames_written < total_frames {
            let remaining = (total_frames - frames_written) as usize;
            let before_stop = if !tail_started && frames_written < stop_frame {
                usize::try_from(stop_frame - frames_written).unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            let this_buffer = remaining.min(BUFFER_SIZE).min(before_stop);
            let sample_count = this_buffer * CHANNELS;

            block[..sample_count].fill(0.0);

            let context = offline_callback_ctx(
                this_buffer,
                frames_written,
                frames_written as f64 / f64::from(self.sample_rate),
                self.sample_rate,
            );

            self.engine.process(&mut block[..sample_count], &context);
            samples.extend_from_slice(&block[..sample_count]);
            frames_written += this_buffer as u64;
            if tail_frames > 0 && !tail_started && frames_written >= stop_frame {
                self.handle.send_blocking(EngineCommand::Stop);
                tail_started = true;
            }
        }

        // Drop the pre-roll prefix so the returned buffer covers exactly
        // [start_tick, end_tick). The prefix was rendered to seed sustained
        // notes that started before `start_tick`; it must not appear in the
        // analysed output.
        let prefix_samples = (prefix_frames as usize).saturating_mul(CHANNELS);
        if prefix_samples > 0 && prefix_samples <= samples.len() {
            samples.drain(0..prefix_samples);
        }

        // No trailing Stop: the next `render_range` issues its own Stop at the
        // top, which handles both flushing this render's voices and any prior
        // state. Dropping the session without a follow-up render is also fine
        // — the engine cleans up its own state on drop.

        Ok(RenderedArrangement {
            samples,
            sample_rate: self.sample_rate,
            duration_seconds: visible_seconds + tail.as_f32(),
            channels: CHANNELS as u16,
            start_tick,
            end_tick,
            warnings,
        })
    }
}

/// Earliest absolute tick at which a note that is still ringing at
/// `start_tick` was triggered. Returns `start_tick` when no note overlaps.
///
/// Mute / solo respected so a soloed per-track render does not pre-roll for
/// notes on tracks that will be silent anyway.
fn earliest_active_note_start(song: &Song, start_tick: Tick) -> Tick {
    let any_solo = song.any_solo();
    let mut earliest = start_tick;

    for placement in song.arrangement() {
        if placement.start >= start_tick {
            continue;
        }
        if let Some(track) = song.track(placement.track_id)
            && !track.is_audible(any_solo)
        {
            continue;
        }
        let Some(pattern) = song.pattern(placement.pattern_id) else {
            continue;
        };

        for note in pattern.notes() {
            let abs_start = Tick::from_pattern_tick(placement.start, note.start);
            if abs_start >= start_tick {
                continue;
            }
            // Notes with no duration ring indefinitely until cut.
            let still_active = match note.duration {
                Some(d) => abs_start.0 + u64::from(d.0) > start_tick.0,
                None => true,
            };
            if still_active && abs_start < earliest {
                earliest = abs_start;
            }
        }
    }

    earliest
}

/// Load one instrument's voice graph + effect chain into the offline engine.
///
/// Mirrors the per-instrument load logic from
/// `crate::audio::preview::render_note_to_buffer`, but writes into the
/// offline engine under the live instrument's own `InstrumentId` (instead of
/// `InstrumentId::FIRST`) so the sequencer's InstrumentId → engine ID
/// mapping survives.
///
/// `offline_engine` is the engine being built (distinct from `engine_state`, the
/// live source read from); it plus `drain_buf`/`drain_ctx` let the load
/// adaptively drain the command ring per module/connection, so a single large
/// voice graph can't overflow the fixed 256-slot ring.
#[allow(clippy::too_many_arguments)]
fn load_instrument_into_offline(
    inst_snap: &synth_engine::shared_state::InstrumentSnapshot,
    engine_state: &synth_engine::EngineState,
    tmp_session: &SynthSession,
    handle: &mut synth_engine::EngineHandle,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    warnings: &mut Vec<String>,
    offline_engine: &mut SynthEngine,
    drain_buf: &mut [f32],
    drain_ctx: &AudioCallbackContext,
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
                              warnings: &mut Vec<String>,
                              offline_engine: &mut SynthEngine,
                              drain_buf: &mut [f32],
                              drain_ctx: &AudioCallbackContext| {
        let module_id = module_snap.id;
        let is_effect = module_id.module_type.is_effect();
        for desc_param in &descriptor.parameters {
            if let Some(ep) = module_snap
                .parameters
                .iter()
                .find(|p| p.same_kind(&desc_param.id))
            {
                // Send the snapshot's full typed param, NOT an f32 round-trip.
                // `with_f32(as_f32())` collapses address-carrying params — the
                // Mod Matrix `SlotSource`/`SlotDestination` hold a `SrcAddr` /
                // `DestAddr` — down to a legacy enum index, silently dropping
                // any address the legacy `ModSource`/`ModDestination` enum lacks
                // (e.g. `spp-1.x`). The modulation would then route nowhere in
                // the offline render (scripted spp motion rendered as mono). The
                // live/GUI load path already sends the full param.
                let param = *ep;
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

        // Replay YAMS scripts (scr / mmx / asc) — the one step the offline loader
        // previously skipped, which left every scripted modulation silent in
        // renders/analyze (shared with the preview note renderer; see
        // `audio::replay_module_scripts`).
        crate::audio::replay_module_scripts(
            handle,
            instrument_id,
            module_snap,
            warnings,
            "arrangement_render",
        );

        // Bound the command ring on a large voice graph: one module's params +
        // scripts is a handful of commands, so an adaptive check per module keeps
        // an arbitrarily large instrument from overflowing the 256-slot ring.
        crate::audio::drain_if_ring_filling(offline_engine, handle, drain_buf, drain_ctx);
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
        apply_module_state(
            handle,
            module_snap,
            &descriptor,
            warnings,
            offline_engine,
            drain_buf,
            drain_ctx,
        );
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
            apply_module_state(
                handle,
                module_snap,
                &descriptor,
                warnings,
                offline_engine,
                drain_buf,
                drain_ctx,
            );
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
        apply_module_state(
            handle,
            module_snap,
            &descriptor,
            warnings,
            offline_engine,
            drain_buf,
            drain_ctx,
        );
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
        // Same adaptive drain for the connection phase (a heavily-patched graph
        // can have hundreds of cables).
        crate::audio::drain_if_ring_filling(offline_engine, handle, drain_buf, drain_ctx);
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

/// Replay the live master effect chain into the offline engine.
///
/// `instrument_id: None` targets the master chain (mirrors `add_master_effect`).
/// The snapshot already carries each effect's full parameter set, so no
/// descriptor walk is needed — the params are forwarded verbatim.
fn load_master_effects_into_offline(
    handle: &mut synth_engine::EngineHandle,
    snapshot: &[synth_engine::shared_state::ReturnEffectSnapshot],
    warnings: &mut Vec<String>,
) {
    handle.send_blocking(EngineCommand::ClearMasterEffects);
    for eff in snapshot {
        let Some((effect, _descriptor)) = crate::module_factory::create_effect(eff.module_type)
        else {
            warnings.push(format!(
                "arrangement_render: master effect {:?} is not an effect type — skipped",
                eff.module_type
            ));
            continue;
        };
        if !handle.send_blocking(EngineCommand::AddEffectInstance {
            instrument_id: None,
            id: eff.module_id,
            effect,
        }) {
            warnings.push(format!(
                "arrangement_render: failed to add master effect {}",
                eff.module_id
            ));
            continue;
        }
        for param in &eff.parameters {
            handle.send_blocking(EngineCommand::SetEffectParameter {
                instrument_id: None,
                module_id: eff.module_id,
                param: *param,
            });
        }
        if eff.bypassed {
            handle.send_blocking(EngineCommand::SetEffectEnabled {
                instrument_id: None,
                module_id: eff.module_id,
                enabled: false,
            });
        }
    }
}

/// Replay the live return-bus effect chains into the offline engine. Must run
/// after the matching `CreateReturnBus` commands so each `return_id` resolves.
fn load_return_effects_into_offline(
    handle: &mut synth_engine::EngineHandle,
    snapshots: &[synth_engine::shared_state::ReturnBusSnapshot],
    warnings: &mut Vec<String>,
) {
    for bus in snapshots {
        for eff in &bus.effects {
            let Some((effect, _descriptor)) = crate::module_factory::create_effect(eff.module_type)
            else {
                warnings.push(format!(
                    "arrangement_render: return effect {:?} is not an effect type — skipped",
                    eff.module_type
                ));
                continue;
            };
            if !handle.send_blocking(EngineCommand::AddReturnEffect {
                return_id: bus.id,
                id: eff.module_id,
                effect,
            }) {
                warnings.push(format!(
                    "arrangement_render: failed to add return effect {} on bus {}",
                    eff.module_id, bus.id.0
                ));
                continue;
            }
            for param in &eff.parameters {
                handle.send_blocking(EngineCommand::SetReturnEffectParameter {
                    return_id: bus.id,
                    module_id: eff.module_id,
                    param: *param,
                });
            }
            if eff.bypassed {
                handle.send_blocking(EngineCommand::SetReturnEffectEnabled {
                    return_id: bus.id,
                    module_id: eff.module_id,
                    enabled: false,
                });
            }
        }
    }
}
