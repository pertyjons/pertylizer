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

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::commands::{InstrumentParam, PortId};
use synth_engine::instrument::MidiChannel;
use synth_engine::{EngineCommand, SynthEngine};
use synth_sequencer::Tick;

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
    shared: &McpSharedState,
    start_tick: u64,
    end_tick: u64,
) -> Result<RenderedArrangement, McpBridgeError> {
    if end_tick <= start_tick {
        return Err(McpBridgeError::Other(format!(
            "Arrangement range invalid: end_tick ({end_tick}) must be greater than start_tick ({start_tick})"
        )));
    }

    let mut warnings: Vec<String> = Vec::new();

    // Wall-clock duration of the tick range, computed via the song's own
    // tick→second conversion so tempo changes inside the range are honoured.
    let duration_seconds = {
        let song = shared.song.read();
        let start_s = song.tick_to_seconds(Tick(start_tick));
        let end_s = song.tick_to_seconds(Tick(end_tick));
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
            "Arrangement range resolves to zero render duration — check tempo settings".to_string(),
        ));
    }

    let total_frames = (f64::from(duration_seconds) * f64::from(RENDER_SAMPLE_RATE)).ceil() as u64;
    if total_frames == 0 {
        return Err(McpBridgeError::Other(
            "Arrangement range too short to produce any samples".to_string(),
        ));
    }

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

    // Use each instrument's live ID so the sequencer's SeqInstrumentId →
    // InstrumentId mapping survives into the offline engine.
    for inst_snap in &live_instruments {
        if let Err(e) = tmp_session.add_instrument_with_id(inst_snap.id, &inst_snap.name) {
            warnings.push(format!(
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
            &mut warnings,
        );
    }

    // The shared Song is read-only from the sequencer's perspective
    // (`SequencerEngine` only does try_read), so handing the live Arc to the
    // offline engine is safe even while the live engine runs.
    handle.send_blocking(EngineCommand::SetSong {
        song: Arc::clone(&shared.song),
    });

    let hw_sample_rate = HwSampleRate(RENDER_SAMPLE_RATE);
    let stream_info = synth_core::StreamInfo {
        sample_rate: hw_sample_rate,
        buffer_size: synth_core::BufferSize(BUFFER_SIZE as u32),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    // Sentinel sample_position in the warm-up block keeps the engine from
    // seeing a duplicate position 0 when the real render begins.
    let mut block = vec![0.0f32; BUFFER_SIZE * CHANNELS];
    let warmup_ctx = AudioCallbackContext {
        sample_rate: hw_sample_rate,
        frames: BUFFER_SIZE,
        channels: CHANNELS as u16,
        stream_time: 0.0,
        sample_position: u64::MAX,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &warmup_ctx);

    // Play, then Seek — in that order. Play transitions the sequencer from
    // Stopped → Playing, which has the side effect of resetting current_tick
    // to zero (see `SequencerEngine::play`). Sending Seek *after* Play
    // overrides that reset so the real render starts at `start_tick`.
    // Both commands drain together at the top of the first real process
    // call below, so the sequencer hasn't advanced yet when Seek lands.
    handle.send_blocking(EngineCommand::Play);
    handle.send_blocking(EngineCommand::Seek {
        tick: Tick(start_tick),
    });

    let mut samples: Vec<f32> = Vec::with_capacity((total_frames as usize) * CHANNELS);
    let mut frames_written: u64 = 0;

    while frames_written < total_frames {
        let remaining = (total_frames - frames_written) as usize;
        let this_buffer = remaining.min(BUFFER_SIZE);
        let sample_count = this_buffer * CHANNELS;

        block[..sample_count].fill(0.0);

        let context = AudioCallbackContext {
            sample_rate: hw_sample_rate,
            frames: this_buffer,
            channels: CHANNELS as u16,
            stream_time: frames_written as f64 / f64::from(RENDER_SAMPLE_RATE),
            sample_position: frames_written,
            output_latency: synth_core::Seconds::ZERO,
        };

        engine.process(&mut block[..sample_count], &context);
        samples.extend_from_slice(&block[..sample_count]);
        frames_written += this_buffer as u64;
    }

    // Stop the sequencer cleanly. Not strictly required for an offline render
    // we're about to drop, but it lets the engine release voices in case any
    // background work runs on Drop.
    handle.send_blocking(EngineCommand::Stop);

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
