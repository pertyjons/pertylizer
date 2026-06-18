//! Offline audio preview rendering for MCP.
//!
//! Renders a single note on an instrument to an in-memory f32 buffer or WAV.
//! The render path is split from WAV encoding so it can be reused by the
//! `analyze_note` MCP tool, which runs offline analysis on the f32 buffer
//! directly without needing WAV bytes.

use std::sync::Arc;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, MidiNote, Velocity};
use synth_engine::commands::{InstrumentParam, PortId};
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::{EngineCommand, SynthEngine};

use synth_mcp::error::McpBridgeError;
use synth_mcp::types::AudioPreview;
use synth_sampler::{Sample, SampleLibrary};

use crate::session::SynthSession;

pub type SharedSampleLibrary = Arc<std::sync::RwLock<SampleLibrary>>;

/// Build the `LoadSampleData` command for a single sampler. Shared between the
/// live engine (`egui_backend::send_loaded_sample_data`) and the offline
/// renderers — the audio buffer never travels through the `SampleSelect`
/// parameter, only its id does.
pub(crate) fn load_sample_data_command(
    instrument_id: InstrumentId,
    module_id: synth_engine::commands::ModuleId,
    sample: &Sample,
) -> EngineCommand {
    EngineCommand::LoadSampleData {
        instrument_id,
        module_id,
        data: Arc::clone(&sample.data),
        channels: sample.meta.channels,
        frame_count: sample.meta.frame_count.as_usize(),
        root_note: sample
            .meta
            .root_note
            .unwrap_or(synth_core::MidiNote::new(60)),
    }
}

/// Sample rate for preview rendering (44.1 kHz — sufficient for previews).
pub(crate) const PREVIEW_SAMPLE_RATE: u32 = 44100;

/// Buffer size in frames per render call.
const BUFFER_SIZE: usize = 256;

/// Output of [`render_note_to_buffer`] — the raw f32 audio plus metadata.
pub struct RenderedNote {
    /// Stereo-interleaved f32 samples (L0, R0, L1, R1, ...).
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Total duration including tail, in seconds.
    pub duration_seconds: f32,
    /// Channel count (always 2).
    pub channels: u16,
    /// Effective MIDI note that was actually played, after the patch's
    /// `octave_offset` was applied. The caller may want to know this for
    /// reporting, e.g. the `analyze_note` tool returns it as `note_played`.
    pub effective_note: MidiNote,
    /// Frame index at which note-off was sent. Useful for downstream analysis
    /// that wants to anchor "release tail" relative to the actual note-off
    /// event rather than the absolute end of the render.
    pub note_off_frame: u64,
    /// Non-fatal warnings emitted during the offline render: modules that
    /// could not be instantiated, connections that failed to resolve, etc.
    /// Empty when the render was clean.
    pub warnings: Vec<String>,
}

/// `Param::Sampler(SampleSelect)` carries the real `SampleId` in the typed
/// enum; `Param::as_f32()` returns 0.0 for that variant, so the f32 path
/// silently loses the id. Reads must go through the typed match.
pub(crate) fn sampler_sample_id(
    snapshot: &synth_engine::ModuleStateSnapshot,
) -> Option<synth_sampler::SampleId> {
    snapshot.parameters.iter().find_map(|p| match p {
        synth_core::Param::Sampler(synth_core::SamplerParam::SampleSelect(sid)) if sid.0 != 0 => {
            Some(synth_sampler::SampleId::new(sid.0))
        }
        _ => None,
    })
}

pub(crate) fn load_sample_data_for_samplers(
    handle: &mut synth_engine::EngineHandle,
    instrument_id: InstrumentId,
    modules: &[&synth_engine::ModuleStateSnapshot],
    sample_library: &SharedSampleLibrary,
    warnings: &mut Vec<String>,
) {
    let lib = match sample_library.read() {
        Ok(lib) => lib,
        Err(_) => {
            warnings.push("offline render: sample library lock poisoned".to_string());
            return;
        }
    };
    for module_snap in modules {
        if module_snap.module_type != synth_core::ModuleType::Sampler {
            continue;
        }
        let Some(sample_id) = sampler_sample_id(module_snap) else {
            continue;
        };
        let Some(sample) = lib.get(sample_id) else {
            warnings.push(format!(
                "offline render: sampler {} references sample id {sample_id} not in library",
                module_snap.id
            ));
            continue;
        };
        let sent = handle.send_blocking(load_sample_data_command(
            instrument_id,
            module_snap.id,
            sample,
        ));
        if !sent {
            warnings.push(format!(
                "offline render: failed to enqueue LoadSampleData for module {}",
                module_snap.id
            ));
        }
    }
}

/// Drain cap and silence floor for the voice-bleed drain between renders on a
/// reused [`OfflineNoteSession`]. Mirror the arrangement-render values.
const VOICE_DRAIN_MAX_MS: f32 = 400.0;
const DRAIN_SILENCE_EPSILON: f32 = 1e-7;
/// Output channel count for offline note renders (always stereo).
const CHANNELS: usize = 2;

/// A reusable offline render engine with one instrument's patch pre-loaded.
///
/// The expensive setup — snapshotting the live instrument, building a fresh
/// [`SynthEngine`], loading its module graph / effects / connections / sample
/// data — runs once in [`new`](Self::new); each [`render`](Self::render) call
/// then plays one note on the warmed-up engine. Sweep tools that render many
/// notes/velocities on the same instrument (`analyze_instrument_range`,
/// `analyze_velocity_response`) build the session once instead of one engine
/// per step.
///
/// Mirrors [`crate::audio::arrangement_render::OfflineEngineSession`]: the first
/// `render` warms the engine up, and every subsequent call first drains any
/// voices still ringing from the previous note so renders stay independent.
pub struct OfflineNoteSession {
    engine: SynthEngine,
    handle: synth_engine::EngineHandle,
    /// Patch octave shift, captured at construction and applied to every note.
    octave_offset: i32,
    /// True once the first `render` has run its warm-up block. Gates warm-up
    /// (first call) vs. voice-bleed drain (subsequent calls).
    first_call_done: bool,
}

impl OfflineNoteSession {
    /// Snapshot the live instrument, build a fresh offline engine, and load its
    /// module graph + parameters + sample data. Does not play anything — that's
    /// done per [`render`](Self::render). Returns the session paired with any
    /// non-fatal setup warnings (failed module adds, missing connections, …).
    pub fn new(
        session: &SynthSession,
        sample_library: &SharedSampleLibrary,
        instrument_id: InstrumentId,
    ) -> Result<(Self, Vec<String>), McpBridgeError> {
        // Validate instrument exists
        if !session.instrument_exists(instrument_id) {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id.as_u64()));
        }

        // Snapshot modules, connections, and parameters from the live engine
        let engine_state = session.state();

        // Mirror the on-screen keyboard's octave shift (applied when the patch
        // was loaded). Without this, a bass patch with `octave_offset = -2`
        // would preview two octaves higher than what the user hears live.
        // Applied per note in `render`.
        let octave_offset = engine_state.get_octave_offset(instrument_id);

        let modules = engine_state
            .shared_graph
            .get_modules_for_instrument(instrument_id);
        let connections = engine_state
            .shared_graph
            .get_connections_for_instrument(instrument_id);

        if modules.is_empty() {
            return Err(McpBridgeError::Other(
                "Instrument has no modules — nothing to render".to_string(),
            ));
        }

        // Snapshot the instrument's "soft" state (volume, pan, solo, mute, MIDI
        // channel) and the effect-chain processing order. Without these, offline
        // renders ignore the live mix state and the effect chain falls back to
        // insertion order rather than the chain's actual slot order.
        let inst_snapshot = engine_state
            .instrument_snapshots
            .read()
            .iter()
            .find(|s| s.id == instrument_id)
            .cloned();

        // Effect-chain order: ordered list of effect ModuleIds. Used to add
        // effects in the right processing order. Empty when no snapshot.
        let effect_chain_order: Vec<synth_engine::commands::ModuleId> = inst_snapshot
            .as_ref()
            .map(|s| s.effect_chain_order.clone())
            .unwrap_or_default();

        let mut warnings: Vec<String> = Vec::new();

        // Create a fresh offline engine
        let (mut engine, mut handle) = SynthEngine::new();

        // Create a temporary session and instrument for loading modules
        let tmp_session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        tmp_session
            .add_instrument_with_id(InstrumentId::FIRST, "Preview")
            .map_err(|e| {
                McpBridgeError::Other(format!("Failed to create preview instrument: {e}"))
            })?;
        tmp_session.reset_counters_for_instrument(InstrumentId::FIRST);

        // Helper to apply parameters + bypass for a given module snapshot.
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
                            instrument_id: Some(InstrumentId::FIRST),
                            module_id,
                            param,
                        })
                    } else {
                        handle.send_blocking(EngineCommand::SetModuleParameter {
                            instrument_id: Some(InstrumentId::FIRST),
                            module_id,
                            param,
                        })
                    };
                    if !sent {
                        warnings.push(format!(
                            "preview: failed to enqueue parameter for module {module_id}"
                        ));
                    }
                }
            }

            // Mirror bypass state from the live snapshot. Effects use
            // SetEffectEnabled (which inverts: enabled = !bypassed); voice-graph
            // modules use SetBypass directly.
            let is_bypassed = matches!(module_snap.bypass_state, synth_core::BypassState::Bypassed);
            if is_bypassed {
                if is_effect {
                    handle.send_blocking(EngineCommand::SetEffectEnabled {
                        instrument_id: Some(InstrumentId::FIRST),
                        module_id,
                        enabled: false,
                    });
                } else {
                    handle.send_blocking(EngineCommand::SetBypass {
                        instrument_id: Some(InstrumentId::FIRST),
                        module: module_id,
                        bypass: true,
                    });
                }
            }
        };

        // Split modules into voice-graph and effect-chain buckets so we can
        // process them in the right order. add_module_with_id dispatches on
        // module_type.is_effect() under the hood, but we need effect-chain
        // processing order (from the live chain) to be preserved.
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

        // Load voice-graph modules first.
        for module_snap in &voice_modules {
            let module_id = module_snap.id;
            let descriptor = match tmp_session.add_module_with_id(
                InstrumentId::FIRST,
                module_id,
                module_id.module_type,
            ) {
                Ok(d) => d,
                Err(e) => {
                    warnings.push(format!("preview: failed to add module {module_id}: {e}"));
                    continue;
                }
            };
            apply_module_state(&mut handle, module_snap, &descriptor, &mut warnings);
        }

        load_sample_data_for_samplers(
            &mut handle,
            InstrumentId::FIRST,
            &voice_modules,
            sample_library,
            &mut warnings,
        );

        // Load effects in chain order so the offline chain mirrors the live one.
        // Any effects that are present in the snapshot but not in the chain order
        // (shouldn't happen) are added afterwards in arbitrary order.
        let mut handled: std::collections::HashSet<synth_engine::commands::ModuleId> =
            std::collections::HashSet::new();
        for module_id in &effect_chain_order {
            if let Some(module_snap) = effect_modules.get(module_id) {
                handled.insert(*module_id);
                let descriptor = match tmp_session.add_module_with_id(
                    InstrumentId::FIRST,
                    *module_id,
                    module_id.module_type,
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        warnings.push(format!("preview: failed to add effect {module_id}: {e}"));
                        continue;
                    }
                };
                apply_module_state(&mut handle, module_snap, &descriptor, &mut warnings);
            }
        }
        for (module_id, module_snap) in &effect_modules {
            if handled.contains(module_id) {
                continue;
            }
            warnings.push(format!(
                "preview: effect {module_id} present but missing from chain order — appending"
            ));
            let descriptor = match tmp_session.add_module_with_id(
                InstrumentId::FIRST,
                *module_id,
                module_id.module_type,
            ) {
                Ok(d) => d,
                Err(e) => {
                    warnings.push(format!("preview: failed to add effect {module_id}: {e}"));
                    continue;
                }
            };
            apply_module_state(&mut handle, module_snap, &descriptor, &mut warnings);
        }

        // Load connections (voice graph internal — effects are wired implicitly).
        for conn in &connections {
            // Skip connections involving visualizer modules
            if conn.from_module.module_type.is_visualizer()
                || conn.to_module.module_type.is_visualizer()
            {
                continue;
            }
            let sent = handle.send_blocking(EngineCommand::Connect {
                instrument_id: Some(InstrumentId::FIRST),
                from: PortId::new(conn.from_module, conn.from_port),
                to: PortId::new(conn.to_module, conn.to_port),
            });
            if !sent {
                warnings.push(format!(
                    "preview: failed to enqueue connection {} → {}",
                    conn.from_module, conn.to_module
                ));
            }
        }

        // Enable the instrument
        handle.send_blocking(EngineCommand::SetInstrumentEnabled {
            instrument_id: InstrumentId::FIRST,
            enabled: true,
        });
        handle.send_blocking(EngineCommand::SetInstrumentMidiChannel {
            instrument_id: InstrumentId::FIRST,
            channel: MidiChannel::CH1,
        });

        // Mirror live instrument volume/pan/solo. Mute is intentionally NOT
        // mirrored — a muted instrument would render silence, which makes the
        // analysis useless. We keep the offline instrument enabled so the user
        // gets a real measurement of the patch even when it is muted in the live
        // mix; analysis tools that need the muted state can read it from the
        // RenderedNote metadata if we add it later.
        if let Some(snap) = inst_snapshot.as_ref() {
            handle.send_blocking(EngineCommand::SetInstrumentParameter {
                instrument_id: InstrumentId::FIRST,
                param: InstrumentParam::Volume(snap.volume),
            });
            handle.send_blocking(EngineCommand::SetInstrumentParameter {
                instrument_id: InstrumentId::FIRST,
                param: InstrumentParam::Pan(snap.pan),
            });
            // Solo from a single instrument is meaningless in an offline render
            // (only one instrument exists), but mirror it for completeness.
            handle.send_blocking(EngineCommand::SetInstrumentParameter {
                instrument_id: InstrumentId::FIRST,
                param: InstrumentParam::Solo(snap.solo),
            });
        }

        // Set up stream
        let hw_sample_rate = HwSampleRate(PREVIEW_SAMPLE_RATE);
        let stream_info = synth_core::StreamInfo {
            sample_rate: hw_sample_rate,
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
                octave_offset,
                first_call_done: false,
            },
            warnings,
        ))
    }

    /// Play one note on the pre-loaded engine and capture the rendered
    /// stereo-interleaved f32 buffer. The first call warms the engine up; every
    /// later call first drains voices still ringing from the previous note so
    /// renders stay independent. Reseeds `fastrand` per call for determinism.
    pub fn render(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        duration_ms: u32,
        tail_ms: u32,
    ) -> Result<RenderedNote, McpBridgeError> {
        // Deterministic offline render — see `OFFLINE_RENDER_SEED` docstring.
        fastrand::seed(crate::audio::arrangement_render::OFFLINE_RENDER_SEED);

        let mut warnings: Vec<String> = Vec::new();

        // Apply the patch's octave shift, captured at construction.
        let effective_note = if self.octave_offset == 0 {
            note
        } else {
            let shifted = i32::from(note.as_u8()) + self.octave_offset * 12;
            MidiNote::new(shifted.clamp(0, 127) as u8)
        };

        let hw_sample_rate = HwSampleRate(PREVIEW_SAMPLE_RATE);
        let mut block = vec![0.0f32; BUFFER_SIZE * CHANNELS];

        if self.first_call_done {
            // Voice-bleed drain: process silent blocks until the previous note's
            // released voices fall below the silence floor, capped at
            // `VOICE_DRAIN_MAX_MS`. Long reverb/delay tails are best-effort.
            let drain_ctx = AudioCallbackContext {
                sample_rate: hw_sample_rate,
                frames: BUFFER_SIZE,
                channels: CHANNELS as u16,
                stream_time: 0.0,
                sample_position: u64::MAX,
                output_latency: synth_core::Seconds::ZERO,
            };
            let max_blocks = ((VOICE_DRAIN_MAX_MS / 1000.0) * PREVIEW_SAMPLE_RATE as f32
                / BUFFER_SIZE as f32)
                .ceil() as usize;
            for _ in 0..max_blocks {
                block.fill(0.0);
                self.engine.process(&mut block, &drain_ctx);
                if block.iter().all(|s| s.abs() < DRAIN_SILENCE_EPSILON) {
                    break;
                }
            }
        } else {
            // Warm up: process one buffer so the engine applies queued commands
            // and initializes. Sentinel sample_position keeps the engine from
            // seeing a duplicate position 0 when real rendering starts.
            let init_context = AudioCallbackContext {
                sample_rate: hw_sample_rate,
                frames: BUFFER_SIZE,
                channels: CHANNELS as u16,
                stream_time: 0.0,
                sample_position: u64::MAX,
                output_latency: synth_core::Seconds::ZERO,
            };
            self.engine.process(&mut block, &init_context);
            self.first_call_done = true;
        }

        // Calculate frame counts
        let note_frames = (f64::from(duration_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
        let tail_frames = (f64::from(tail_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
        let total_frames = note_frames + tail_frames;

        if total_frames == 0 {
            return Err(McpBridgeError::Other(
                "Total render duration is 0 — duration_ms and tail_ms must not both be 0"
                    .to_string(),
            ));
        }
        let total_seconds = total_frames as f32 / PREVIEW_SAMPLE_RATE as f32;

        // Pre-allocate the output buffer for the entire render.
        let mut samples: Vec<f32> = Vec::with_capacity(total_frames as usize * CHANNELS);

        // Send note on
        self.handle.send_blocking(EngineCommand::NoteOn {
            note: effective_note,
            velocity,
            channel: MidiChannel::CH1,
        });

        let mut frames_written: u64 = 0;
        let mut note_off_sent = false;
        let mut note_off_frame: u64 = note_frames;

        while frames_written < total_frames {
            let remaining = (total_frames - frames_written) as usize;
            // Bound the block size to never cross the note-off boundary in a
            // single process() call. Without this, NoteOff is sent *after* the
            // block that crossed the boundary has already been rendered with the
            // note still held, plus the command sits in the queue until the
            // next process() — adding up to two block-lengths of false sustain.
            // By trimming the block at exactly note_frames, the NoteOff command
            // is queued before the engine drains commands for the next block,
            // so the note releases at the precise sample boundary.
            let block_cap = if !note_off_sent && frames_written < note_frames {
                (note_frames - frames_written) as usize
            } else {
                usize::MAX
            };
            let this_buffer = remaining.min(BUFFER_SIZE).min(block_cap);
            let sample_count = this_buffer * CHANNELS;

            block.fill(0.0);

            let context = AudioCallbackContext {
                sample_rate: hw_sample_rate,
                frames: this_buffer,
                channels: CHANNELS as u16,
                stream_time: frames_written as f64 / f64::from(PREVIEW_SAMPLE_RATE),
                sample_position: frames_written,
                output_latency: synth_core::Seconds::ZERO,
            };

            self.engine.process(&mut block[..sample_count], &context);

            // Append this block's samples to the output buffer.
            samples.extend_from_slice(&block[..sample_count]);

            frames_written += this_buffer as u64;

            // Send note-off the moment we've rendered up to (and not past) the
            // note-off boundary. Sending here — after the block but before the
            // next iteration — ensures the next process() call drains the
            // NoteOff command before producing any post-note-off samples.
            if !note_off_sent && frames_written >= note_frames {
                let sent = self.handle.send_blocking(EngineCommand::NoteOff {
                    note: effective_note,
                    channel: MidiChannel::CH1,
                });
                if !sent {
                    warnings.push("preview: failed to enqueue NoteOff".to_string());
                }
                note_off_sent = true;
                note_off_frame = frames_written;
            }
        }

        Ok(RenderedNote {
            samples,
            sample_rate: PREVIEW_SAMPLE_RATE,
            duration_seconds: total_seconds,
            channels: CHANNELS as u16,
            effective_note,
            note_off_frame,
            warnings,
        })
    }
}

/// Render a note on the given instrument into an in-memory f32 buffer.
///
/// Convenience wrapper around [`OfflineNoteSession`] for callers that render a
/// single note: builds a session, renders once, and merges the setup warnings
/// into the returned [`RenderedNote`] so the combined-warnings contract that
/// existing callers rely on is preserved.
pub fn render_note_to_buffer(
    session: &SynthSession,
    sample_library: &SharedSampleLibrary,
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
) -> Result<RenderedNote, McpBridgeError> {
    let (mut sess, setup_warnings) =
        OfflineNoteSession::new(session, sample_library, instrument_id)?;
    let mut rendered = sess.render(note, velocity, duration_ms, tail_ms)?;
    if !setup_warnings.is_empty() {
        let mut combined = setup_warnings;
        combined.extend(rendered.warnings);
        rendered.warnings = combined;
    }
    Ok(rendered)
}

/// Encode a stereo-interleaved (or mono) f32 buffer as a 16-bit signed WAV.
///
/// Samples outside ±1.0 are clamped before conversion. The resulting bytes
/// are a complete in-memory WAV file (header + PCM data).
pub fn encode_buffer_as_wav(
    buffer: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<Vec<u8>, McpBridgeError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav_cursor = std::io::Cursor::new(Vec::with_capacity(buffer.len() * 2 + 44));
    let mut writer = hound::WavWriter::new(&mut wav_cursor, spec)
        .map_err(|e| McpBridgeError::Other(format!("WAV writer error: {e}")))?;

    for &sample in buffer {
        let clamped = sample.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let int_val = (clamped * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(int_val)
            .map_err(|e| McpBridgeError::Other(format!("WAV write error: {e}")))?;
    }

    writer
        .finalize()
        .map_err(|e| McpBridgeError::Other(format!("WAV finalize error: {e}")))?;
    Ok(wav_cursor.into_inner())
}

/// Render a note preview for the given instrument and return WAV bytes.
///
/// Thin wrapper that calls [`render_note_to_buffer`] and then encodes the
/// resulting f32 buffer with [`encode_buffer_as_wav`]. Existing callers
/// (e.g. the `preview_note` MCP tool) keep using this function unchanged.
pub fn render_note_preview(
    session: &SynthSession,
    sample_library: &SharedSampleLibrary,
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
) -> Result<AudioPreview, McpBridgeError> {
    let rendered = render_note_to_buffer(
        session,
        sample_library,
        instrument_id,
        note,
        velocity,
        duration_ms,
        tail_ms,
    )?;

    let wav_data =
        encode_buffer_as_wav(&rendered.samples, rendered.sample_rate, rendered.channels)?;

    Ok(AudioPreview {
        wav_data,
        sample_rate: rendered.sample_rate,
        duration_seconds: rendered.duration_seconds,
    })
}
