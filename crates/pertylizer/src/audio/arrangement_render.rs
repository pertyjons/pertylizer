//! Offline arrangement rendering.
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
//! Used by the `analyze_mix_bus` / `analyze_section` MCP tools and by the
//! headless render pipeline (`crate::render`, the `pertylizer render`
//! command) to obtain a deterministic, fast (faster-than-real-time)
//! rendering of the master bus output of a song region.
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

use synth_core::audio::DeviceSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, DenormalGuard};
use synth_engine::commands::InstrumentParam;
use synth_engine::instrument::MidiChannelSelection;
use synth_engine::{EngineCommand, SynthEngine};
use synth_sequencer::{Song, Tick};

use synth_core::AnalysisScope;

use crate::session::SynthSession;

/// A failed offline arrangement render: the offline engine could not be built
/// or the requested range could not be rendered.
///
/// One rendered message rather than variants: every failure here is reported
/// to callers as text (the render command's `RenderError::Render`, the MCP
/// bridge's error string), and the typed problem model planned in
/// `plans/mcp-agent-api-redesign.md` replaces this wholesale.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct OfflineRenderError(pub(crate) String);

/// Block size in frames per `engine.process()` call.
///
/// Public so a measurement can report the value it was built with rather than
/// the value its operator believed it was built with: it is a compile-time
/// constant, so the only way a harness can label its own data honestly is to
/// read it. See the `render_cost` binary and ADR-0037's proxy measurement in
/// `plans/v2/`.
pub const BUFFER_SIZE: usize = 256;

/// Output channel count — always stereo.
const CHANNELS: usize = 2;

/// Hard ceiling on how many seconds an arrangement render may produce, to
/// keep an offline render request bounded. 5 minutes at 44.1 kHz stereo
/// ≈ 105 MB f32 — comfortably above any reasonable analysis window.
///
/// A range longer than this is *clamped* here, with a warning. The `render`
/// command checks its `--seconds` against this up front instead, because a
/// harness that asked for ten minutes and silently got five would compare the
/// wrong audio.
pub(crate) const MAX_RENDER_SECONDS: f32 = 300.0;

/// Maximum amount of pre-roll (audio before the requested `start_tick`) that
/// the renderer will run to seed sustained notes that started before the
/// range. The prefix is rendered and then discarded; this cap keeps an extreme
/// drone or 10-minute pad from forcing an analyze_section call to render the
/// entire song. Notes that started further back than this are silent in the
/// output — same as the pre-pre-roll behaviour, but with a warning.
const MAX_PREROLL_SECONDS: f32 = 30.0;

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
        sample_rate: DeviceSampleRate::new(sample_rate),
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
    /// Tick range that was rendered. Matches the requested range, except that
    /// `end_tick` is pulled in to the tick actually reached when the range was
    /// clamped to the render budget (see `MAX_RENDER_SECONDS`).
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
    song: &Arc<synth_sequencer::SharedSong>,
    start_tick: u64,
    end_tick: u64,
) -> Result<RenderedArrangement, OfflineRenderError> {
    render_arrangement_to_buffer_with_scope(
        session,
        sample_library,
        song,
        start_tick,
        end_tick,
        AnalysisScope::default(),
    )
}

/// Like [`render_arrangement_to_buffer`] but reconstructs the optional signal
/// stages requested by `scope` (master effects, return-bus effects, …) so the
/// analysis hears more than the dry instrument sum. `AnalysisScope::default()`
/// behaves identically to [`render_arrangement_to_buffer`].
///
/// Thin wrapper: builds a single [`OfflineEngineSession`] and renders one range.
/// Callers that need N renders against the same engine state (per-track loop in
/// `analyze_section`) should construct an [`OfflineEngineSession`] directly and
/// call [`OfflineEngineSession::render_range`] N times instead — that amortizes
/// the engine + instrument-load cost across the loop.
pub fn render_arrangement_to_buffer_with_scope(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    song: &Arc<synth_sequencer::SharedSong>,
    start_tick: u64,
    end_tick: u64,
    scope: AnalysisScope,
) -> Result<RenderedArrangement, OfflineRenderError> {
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
    /// block AND send `ResetDsp` to clear voices/effect state left by the
    /// previous render (a freshly-built engine has nothing to clear).
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
    ) -> Result<(Self, Vec<String>), OfflineRenderError> {
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
    ) -> Result<(Self, Vec<String>), OfflineRenderError> {
        let engine_state = session.state();
        let live_instruments: Vec<synth_engine::shared_state::InstrumentSnapshot> = engine_state
            .instrument_snapshots
            .read()
            .iter()
            .cloned()
            .collect();
        if live_instruments.is_empty() {
            return Err(OfflineRenderError(
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
        let sample_rate = scope.render_sample_rate.as_u32();
        let stream_info = synth_core::StreamInfo {
            sample_rate: DeviceSampleRate::new(sample_rate),
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
        // fill the configured ring — nothing else drains it during setup.
        for inst_snap in &live_instruments {
            // With the allocator config, not without it. `max_voices`, `mode`,
            // and `stealing` fix the size and behaviour of a pre-allocated voice
            // pool, so this constructor is the only place they can be set — an
            // offline session built without them renders every instrument as a
            // default 8-voice polyphonic one however the project was
            // configured, and does it silently.
            let allocator = synth_engine::voice_allocator::AllocatorConfig {
                max_voices: inst_snap.max_voices,
                mode: inst_snap.allocation_mode,
                stealing: inst_snap.stealing_strategy,
                unison_detune: inst_snap.unison_detune,
                unison_spread: inst_snap.unison_spread,
                ..Default::default()
            };
            if let Err(e) = tmp_session.add_instrument_with_id_and_config(
                inst_snap.id,
                &inst_snap.name,
                Some(allocator),
            ) {
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
        let master = synth_core::Gain::new(engine_state.master_volume.load());
        if let Err(e) = handle.send_blocking(EngineCommand::SetMasterVolume(master)) {
            setup_warnings.push(format!(
                "arrangement_render: could not set the master volume to {}: {e} — \
                 the render is at unity instead",
                master.as_f32()
            ));
        }

        // Glide is global in the project and per-instrument in the engine, so it
        // reaches no instrument snapshot — which is exactly how it survived the
        // per-instrument sweep above. A project with a non-zero glide time
        // rendered here without it plays every interval as a jump. Sent after
        // the instruments exist, because the engine applies it to the
        // instruments it has.
        let glide = synth_core::Seconds::new(engine_state.glide_time.load());
        if let Err(e) = handle.send_blocking(EngineCommand::SetGlideTime(glide)) {
            setup_warnings.push(format!(
                "arrangement_render: could not set the glide time to {glide}: {e} — \
                 every interval will render as a jump"
            ));
        }

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
    ) -> Result<RenderedArrangement, OfflineRenderError> {
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
    ) -> Result<RenderedArrangement, OfflineRenderError> {
        if end_tick <= start_tick {
            return Err(OfflineRenderError(format!(
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

        let (visible_seconds, prefix_seconds, effective_start_tick, effective_end_tick) = {
            let song_read = song.read();
            let start_s = song_read.tick_to_seconds(Tick(start_tick));
            let end_s = song_read.tick_to_seconds(Tick(end_tick));
            let raw_effective_s =
                song_read.tick_to_seconds(earliest_active_note_start(&song_read, Tick(start_tick)));
            let raw_prefix_s = (start_s - raw_effective_s).max(0.0);

            let mut prefix_s = raw_prefix_s.min(f64::from(MAX_PREROLL_SECONDS));
            if raw_prefix_s > f64::from(MAX_PREROLL_SECONDS) {
                warnings.push(format!(
                    "Pre-roll requested {raw_prefix_s:.1}s; capping at {MAX_PREROLL_SECONDS:.0}s. \
                     Notes that began earlier are silent in the output."
                ));
            }

            // The requested (visible) range gets the whole render budget…
            let raw_visible_s = (end_s - start_s).max(0.0);
            let visible_s = if raw_visible_s > f64::from(MAX_RENDER_SECONDS) {
                warnings.push(format!(
                    "Requested arrangement range is {raw_visible_s:.1}s; clamping to the \
                     {MAX_RENDER_SECONDS:.0}s render budget.",
                ));
                f64::from(MAX_RENDER_SECONDS)
            } else {
                raw_visible_s
            };

            // …and pre-roll that would push the total past the budget is
            // shrunk to fit (the caller asked for the visible range; pre-roll
            // is best-effort seeding), matching the module-doc contract.
            let budget_prefix_s = (f64::from(MAX_RENDER_SECONDS) - visible_s).max(0.0);
            if prefix_s > budget_prefix_s {
                warnings.push(format!(
                    "Pre-roll shrunk from {prefix_s:.1}s to {budget_prefix_s:.1}s so the render \
                     fits the {MAX_RENDER_SECONDS:.0}s budget. Notes that began earlier are \
                     silent in the output."
                ));
                prefix_s = budget_prefix_s;
            }

            // `Song::seconds_to_tick` is the exact tempo-aware inverse of
            // `tick_to_seconds`, so the Seek lands on the tick whose wall-clock
            // position is `prefix_s` before `start_tick`.
            let effective_tick = if prefix_s > 0.0 {
                song_read.seconds_to_tick(start_s - prefix_s)
            } else {
                Tick(start_tick)
            };

            // When the visible range was clamped, report the tick actually
            // reached so `RenderedArrangement` describes the audio it carries
            // rather than the range that was asked for.
            let effective_end = if visible_s < raw_visible_s {
                song_read.seconds_to_tick(start_s + visible_s).0
            } else {
                end_tick
            };

            (
                visible_s as f32,
                prefix_s as f32,
                effective_tick,
                effective_end,
            )
        };

        if visible_seconds <= 0.0 {
            return Err(OfflineRenderError(
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
            return Err(OfflineRenderError(
                "Arrangement range too short to produce any samples".to_string(),
            ));
        }

        // Drop transport and DSP state left by a previous render. `Stop`
        // releases voices and clears transient automation overrides;
        // `ResetDsp` then clears every voice, delay/reverb line, effect chain,
        // modular-graph node, and oversampling filter before the next render
        // produces a sample. A bounded silence drain cannot provide that
        // contract for effects with multi-second or infinite feedback tails.
        let _ = self.handle.send_blocking(EngineCommand::Stop);
        let mut block = vec![0.0f32; BUFFER_SIZE * CHANNELS];
        if self.first_call_done {
            let _ = self.handle.send_blocking(EngineCommand::ResetDsp);
        }

        // The supplied Song is read-only from the sequencer's perspective
        // (`SequencerEngine` only does try_read), so handing an Arc to the
        // offline engine is safe even when it points at the live shared
        // instance. Re-sent every call so callers may render against different
        // songs (or the same song with mutated solo flags) without restart.
        let _ = self.handle.send_blocking(EngineCommand::SetSong {
            song: Arc::clone(song),
        });
        // Ship the Mod Grid runtime for this song so control-rate modulation is
        // present in the offline render (matches the live engine's pre-pass).
        let mod_grid = crate::mod_grid_build::build_mod_grid_runtime(&song.read());
        let _ = self.handle.send_blocking(EngineCommand::SetModGrid {
            runtime: Box::new(mod_grid),
        });

        // Reconstruct the song's return-bus channels so sends route correctly
        // in the offline render. Reset first so channels from a prior render of
        // a different song don't linger (this session is reused across calls).
        // Faders are read live from the song. Return-bus effect chains are
        // reconstructed only when `scope.return_effects` is set (replayed every
        // render because `ClearReturnBusses` wipes them); otherwise offline
        // renders hear the dry-summed returns.
        let _ = self.handle.send_blocking(EngineCommand::ClearReturnBusses);
        for bus in song.read().return_busses() {
            let _ = self
                .handle
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
        let _ = self.handle.send_blocking(EngineCommand::Play);
        let _ = self.handle.send_blocking(EngineCommand::Seek {
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
                let _ = self.handle.send_blocking(EngineCommand::Stop);
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
            end_tick: effective_end_tick,
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
/// voice graph cannot fill the configured ring.
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

    crate::audio::instrument_hydration::hydrate_snapshot_instrument(
        tmp_session,
        handle,
        crate::audio::instrument_hydration::SnapshotHydration {
            instrument_id,
            modules: &modules,
            connections: &connections,
            effect_chain_order: &effect_chain_order,
            sample_library,
            context: "arrangement_render",
        },
        warnings,
        |handle| {
            crate::audio::drain_if_ring_filling(offline_engine, handle, drain_buf, drain_ctx);
        },
    );

    // Mirror live enable/mix state. `inst_snap.enabled` already encodes the
    // muted+enabled live behavior (an instrument muted live is reported as
    // disabled), so we forward it directly. Track-level mutes inside the
    // arrangement are honored by the shared sequencer automatically.
    let enabled = inst_snap.enabled && !inst_snap.muted;
    if let Err(e) = handle.send_blocking(EngineCommand::SetInstrumentEnabled {
        instrument_id,
        enabled,
    }) {
        warnings.push(format!(
            "arrangement_render: instrument {} rendered enabled when it should be {}: {e}",
            instrument_id.as_u64(),
            if enabled { "enabled" } else { "muted" }
        ));
    }
    // MIDI channel only affects external MIDI input, which an offline render
    // doesn't have — default to channel 1 for all instruments. A failure here
    // is reported anyway: the rule is that a dropped command is never silent,
    // and "this one happens not to matter" is exactly the reasoning that left
    // the parameter sweep below missing for as long as it was.
    if let Err(e) = handle.send_blocking(EngineCommand::SetInstrumentMidiChannel {
        instrument_id,
        channel: MidiChannelSelection::CH1,
    }) {
        warnings.push(format!(
            "arrangement_render: instrument {} kept its live MIDI channel: {e} \
             (harmless offline — nothing sends it MIDI — but the command was dropped)",
            instrument_id.as_u64()
        ));
    }
    // Every instrument parameter the snapshot carries, not just the mix three.
    //
    // This used to send `Volume`, `Pan`, and `Solo` and stop, which left the
    // rest at their engine defaults — and every one of them changes the audio.
    // A four-voice instrument rendered as eight (so a project that steals voices
    // live never stole one offline), a transposed instrument rendered at concert
    // pitch, a key-split instrument rendered across the whole keyboard. None of
    // it warned, because nothing was missing: the values were simply never sent.
    //
    // The list mirrors `project_apply::push_instrument_params`, which is what
    // the live load path sends, plus the velocity sensitivities. The allocator
    // fields appear both here and in the `AllocatorConfig` the instrument was
    // constructed with, exactly as they do live.
    //
    // A send that fails is reported rather than discarded. Nothing drains this
    // ring but this thread, so a full ring makes `send_blocking` time out and
    // return an error — and swallowing it would leave that parameter at the
    // engine default, which is precisely the silent defaulting this list exists
    // to end.
    for param in [
        InstrumentParam::Volume(inst_snap.volume),
        InstrumentParam::Pan(inst_snap.pan),
        InstrumentParam::Solo(inst_snap.solo),
        InstrumentParam::OversamplingFactor(inst_snap.oversampling),
        InstrumentParam::KeyRange(inst_snap.key_range),
        InstrumentParam::Transpose(inst_snap.transpose),
        InstrumentParam::AllocationMode(inst_snap.allocation_mode),
        InstrumentParam::StealingStrategy(inst_snap.stealing_strategy),
        InstrumentParam::UnisonDetune(inst_snap.unison_detune),
        InstrumentParam::UnisonSpread(inst_snap.unison_spread),
        InstrumentParam::MaxVoices(inst_snap.max_voices),
        InstrumentParam::VelocityAmpSensitivity(inst_snap.velocity_amp_sensitivity),
        InstrumentParam::VelocityFilterSensitivity(inst_snap.velocity_filter_sensitivity),
    ] {
        // `param` is `Copy`, so the command below borrows nothing from it and
        // it is still nameable in the warning.
        if let Err(e) = handle.send_blocking(EngineCommand::SetInstrumentParameter {
            instrument_id,
            param,
        }) {
            warnings.push(format!(
                "arrangement_render: instrument {} kept the engine default for {param:?}: {e}",
                instrument_id.as_u64()
            ));
        }
    }

    // Sidechain routing is audio, not metadata: a compressor whose source is
    // unset ducks nothing, so a project built around sidechain compression
    // renders without any of it.
    if let Some(source) = inst_snap.sidechain_source_id
        && let Err(e) = handle.send_blocking(EngineCommand::SetSidechainSource {
            instrument_id,
            source: Some(source),
        })
    {
        warnings.push(format!(
            "arrangement_render: instrument {} rendered without its sidechain source {}: {e}",
            instrument_id.as_u64(),
            source.as_u64()
        ));
    }
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
    let _ = handle.send_blocking(EngineCommand::ClearMasterEffects);
    for eff in snapshot {
        let Some((effect, _descriptor)) = crate::module_factory::create_effect(eff.module_type)
        else {
            warnings.push(format!(
                "arrangement_render: master effect {:?} is not an effect type — skipped",
                eff.module_type
            ));
            continue;
        };
        if handle
            .send_blocking(EngineCommand::AddEffectInstance {
                instrument_id: None,
                id: eff.module_id,
                effect,
            })
            .is_err()
        {
            warnings.push(format!(
                "arrangement_render: failed to add master effect {}",
                eff.module_id
            ));
            continue;
        }
        for param in &eff.parameters {
            let _ = handle.send_blocking(EngineCommand::SetEffectParameter {
                instrument_id: None,
                module_id: eff.module_id,
                param: *param,
            });
        }
        if eff.bypassed {
            let _ = handle.send_blocking(EngineCommand::SetEffectEnabled {
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
            if handle
                .send_blocking(EngineCommand::AddReturnEffect {
                    return_id: bus.id,
                    id: eff.module_id,
                    effect,
                })
                .is_err()
            {
                warnings.push(format!(
                    "arrangement_render: failed to add return effect {} on bus {}",
                    eff.module_id, bus.id.0
                ));
                continue;
            }
            for param in &eff.parameters {
                let _ = handle.send_blocking(EngineCommand::SetReturnEffectParameter {
                    return_id: bus.id,
                    module_id: eff.module_id,
                    param: *param,
                });
            }
            if eff.bypassed {
                let _ = handle.send_blocking(EngineCommand::SetReturnEffectEnabled {
                    return_id: bus.id,
                    module_id: eff.module_id,
                    enabled: false,
                });
            }
        }
    }
}
