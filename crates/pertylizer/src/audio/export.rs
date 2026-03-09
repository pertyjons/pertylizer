//! Offline audio export (WAV rendering).
//!
//! Renders the current project to a WAV file in a background thread,
//! faster than realtime. Progress is reported via atomic counters
//! so the GUI can display a progress bar without blocking.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use synth_core::{AudioCallbackContext, AudioProcessor, audio::SampleRate as HwSampleRate};
use synth_engine::commands::{EffectType, PortId};
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::{AllocationMode, AllocatorConfig, EngineCommand, SynthEngine};

use crate::patch::ParamValue;
use crate::project::ProjectFile;
use crate::session::SynthSession;

/// Bit depth for WAV export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    /// 16-bit signed integer.
    Sixteen,
    /// 24-bit signed integer.
    TwentyFour,
    /// 32-bit IEEE float.
    ThirtyTwoFloat,
}

impl BitDepth {
    /// Display label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sixteen => "16-bit",
            Self::TwentyFour => "24-bit",
            Self::ThirtyTwoFloat => "32-bit float",
        }
    }

    /// All available bit depths for iteration.
    pub const ALL: [Self; 3] = [Self::Sixteen, Self::TwentyFour, Self::ThirtyTwoFloat];
}

/// Configuration for a WAV export.
pub struct ExportConfig {
    /// Output file path.
    pub path: PathBuf,
    /// Sample rate (e.g. 44100, 48000, 96000).
    pub sample_rate: u32,
    /// Bit depth of the output WAV.
    pub bit_depth: BitDepth,
    /// Duration in seconds to render.
    pub duration_seconds: f64,
    /// Extra tail time in seconds for reverb/delay tails.
    pub tail_seconds: f32,
}

impl ExportConfig {
    /// Total number of stereo frames to render.
    fn total_frames(&self) -> u64 {
        let total_secs = self.duration_seconds + f64::from(self.tail_seconds);
        (total_secs * f64::from(self.sample_rate)) as u64
    }
}

/// Thread-safe progress tracking for an ongoing export.
pub struct ExportProgress {
    /// Number of sample frames rendered so far.
    pub frames_rendered: Arc<AtomicU64>,
    /// Total number of sample frames to render.
    pub total_frames: u64,
    /// Whether the export has completed (successfully or with error).
    pub completed: Arc<AtomicBool>,
    /// Whether a cancel was requested.
    pub cancelled: Arc<AtomicBool>,
    /// Error message, if any.
    pub error: Arc<Mutex<Option<String>>>,
}

impl ExportProgress {
    /// Create a new progress tracker for the given total frame count.
    fn new(total_frames: u64) -> Self {
        Self {
            frames_rendered: Arc::new(AtomicU64::new(0)),
            total_frames,
            completed: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the progress as a fraction in 0.0..=1.0.
    pub fn fraction(&self) -> f32 {
        if self.total_frames == 0 {
            return 1.0;
        }
        let rendered = self.frames_rendered.load(Ordering::Relaxed);
        (rendered as f32) / (self.total_frames as f32)
    }

    /// Check if the export is done.
    pub fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Check if there was an error.
    pub fn error_message(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn clone_internals(&self) -> Self {
        Self {
            frames_rendered: Arc::clone(&self.frames_rendered),
            total_frames: self.total_frames,
            completed: Arc::clone(&self.completed),
            cancelled: Arc::clone(&self.cancelled),
            error: Arc::clone(&self.error),
        }
    }
}

/// Error type for export operations.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// WAV writer error.
    #[error("WAV write error: {0}")]
    Wav(String),
    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Cancelled by user.
    #[error("export cancelled")]
    Cancelled,
    /// Engine setup error.
    #[error("engine setup error: {0}")]
    EngineSetup(String),
}

impl From<hound::Error> for ExportError {
    fn from(e: hound::Error) -> Self {
        Self::Wav(e.to_string())
    }
}

/// Start an offline WAV export in a background thread.
///
/// Returns an `ExportProgress` handle that can be polled from the GUI
/// to show a progress bar and detect completion.
pub fn start_export(project: ProjectFile, config: ExportConfig) -> ExportProgress {
    let progress = ExportProgress::new(config.total_frames());
    let progress_clone = progress.clone_internals();

    std::thread::Builder::new()
        .name("wav-export".to_string())
        .spawn(move || {
            match render_to_wav(&project, &config, &progress_clone) {
                Ok(()) => {}
                Err(ExportError::Cancelled) => {
                    // Clean up partial file on cancel
                    let _ = std::fs::remove_file(&config.path);
                }
                Err(e) => {
                    let mut err = progress_clone
                        .error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    *err = Some(e.to_string());
                }
            }
            progress_clone.completed.store(true, Ordering::Release);
        })
        .ok();

    progress
}

/// Render the project to a WAV file (blocking, runs in background thread).
fn render_to_wav(
    project: &ProjectFile,
    config: &ExportConfig,
    progress: &ExportProgress,
) -> Result<(), ExportError> {
    // 1. Create a fresh engine
    let allocator_config = AllocatorConfig {
        max_voices: synth_core::VoiceCount::OCTO,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (mut engine, mut handle) = SynthEngine::with_config(allocator_config);

    // 2. Create a session for loading
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    // 3. Set up the song
    let song = Arc::new(std::sync::RwLock::new(project.song.clone()));
    handle.send_blocking(EngineCommand::SetSong {
        song: Arc::clone(&song),
    });

    // 4. Load all instruments and their patches into the engine
    load_project_into_engine(project, &session, &mut handle)?;

    // 5. Apply global settings
    handle.send_blocking(EngineCommand::SetMasterVolume(project.global.master_volume));
    handle.send_blocking(EngineCommand::SetGlideTime(project.global.glide_time));
    if let Some(awe) = &project.global.awe {
        handle.send_blocking(EngineCommand::SetAweEnabled {
            enabled: awe.enabled,
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::RoomShape(awe.room),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::Material(awe.material),
        });
        handle.send_blocking(EngineCommand::SetAweState {
            snapshot: awe.to_snapshot(),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::SpatialEnabled(awe.spatial_enabled),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::NoteMapping(awe.note_mapping),
        });
    }

    // 6. Notify the engine about the stream configuration
    let hw_sample_rate = HwSampleRate(config.sample_rate);
    let stream_info = synth_core::StreamInfo {
        sample_rate: hw_sample_rate,
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    // 7. Process one buffer of silence to let the engine initialize
    let channels: usize = 2;
    let buffer_size: usize = 256;
    let mut buffer = vec![0.0f32; buffer_size * channels];
    let init_context = AudioCallbackContext {
        sample_rate: hw_sample_rate,
        frames: buffer_size,
        channels: channels as u16,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: 0.0,
    };
    engine.process(&mut buffer, &init_context);

    // 8. Start playback via the handle (sends Play command through ring buffer)
    handle.send_blocking(EngineCommand::Rewind);
    handle.send_blocking(EngineCommand::Play);

    // Process one more buffer to let the Play command take effect
    buffer.fill(0.0);
    let warmup_context = AudioCallbackContext {
        sample_rate: hw_sample_rate,
        frames: buffer_size,
        channels: channels as u16,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: 0.0,
    };
    engine.process(&mut buffer, &warmup_context);

    // 9. Create WAV writer
    let spec = match config.bit_depth {
        BitDepth::Sixteen => hound::WavSpec {
            channels: channels as u16,
            sample_rate: config.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
        BitDepth::TwentyFour => hound::WavSpec {
            channels: channels as u16,
            sample_rate: config.sample_rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
        BitDepth::ThirtyTwoFloat => hound::WavSpec {
            channels: channels as u16,
            sample_rate: config.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    };

    let mut writer = hound::WavWriter::create(&config.path, spec)?;

    // 10. Render loop
    let total_frames = config.total_frames();
    let mut frames_written: u64 = 0;

    while frames_written < total_frames {
        // Check cancellation
        if progress.cancelled.load(Ordering::Relaxed) {
            // Finalize the writer so the file is valid (or we can delete it)
            drop(writer);
            return Err(ExportError::Cancelled);
        }

        let remaining = (total_frames - frames_written) as usize;
        let this_buffer = remaining.min(buffer_size);
        let sample_count = this_buffer * channels;

        buffer.fill(0.0);

        let context = AudioCallbackContext {
            sample_rate: hw_sample_rate,
            frames: this_buffer,
            channels: channels as u16,
            stream_time: frames_written as f64 / f64::from(config.sample_rate),
            sample_position: frames_written,
            output_latency: 0.0,
        };

        engine.process(&mut buffer[..sample_count], &context);

        // Write samples to WAV
        match config.bit_depth {
            BitDepth::Sixteen => {
                for &sample in &buffer[..sample_count] {
                    let clamped = sample.clamp(-1.0, 1.0);
                    let int_val = (clamped * f32::from(i16::MAX)) as i16;
                    writer.write_sample(int_val)?;
                }
            }
            BitDepth::TwentyFour => {
                for &sample in &buffer[..sample_count] {
                    let clamped = sample.clamp(-1.0, 1.0);
                    let int_val = (clamped * 8_388_607.0) as i32; // 2^23 - 1
                    writer.write_sample(int_val)?;
                }
            }
            BitDepth::ThirtyTwoFloat => {
                for &sample in &buffer[..sample_count] {
                    writer.write_sample(sample)?;
                }
            }
        }

        frames_written += this_buffer as u64;
        progress
            .frames_rendered
            .store(frames_written, Ordering::Relaxed);
    }

    // 11. Finalize WAV file
    writer.finalize()?;

    Ok(())
}

/// Load a `ProjectFile` into a fresh engine via commands.
///
/// This is a simplified version of `egui_backend::load_project_data` that
/// doesn't need any GUI types (no `PatchEditor`, `PianoKeyboard`, etc.).
fn load_project_into_engine(
    project: &ProjectFile,
    session: &SynthSession,
    handle: &mut synth_engine::EngineHandle,
) -> Result<(), ExportError> {
    // Clear the default instrument's graph
    let _ = session.clear_graph(InstrumentId::FIRST);

    for inst_state in &project.instruments {
        let inst_id = inst_state.id;

        // Create instrument (FIRST already exists)
        if inst_id != InstrumentId::FIRST
            && let Err(e) = session.add_instrument_with_id(inst_id, &inst_state.name)
        {
            eprintln!(
                "Warning: failed to create instrument {}: {e}",
                inst_state.name
            );
            continue;
        }

        // Reset counters before loading
        session.reset_counters_for_instrument(inst_id);

        // Set instrument properties
        let channel = MidiChannel::from_one_indexed(inst_state.channel).unwrap_or(MidiChannel::CH1);
        let _ = session.rename_instrument(inst_id, &inst_state.name);
        let _ = session.set_instrument_volume(inst_id, inst_state.volume);
        let _ = session.set_instrument_pan(inst_id, inst_state.pan);
        let _ = session.set_instrument_mute(inst_id, inst_state.muted);
        let _ = session.set_instrument_solo(inst_id, inst_state.solo);
        let _ = session.set_instrument_midi_channel(inst_id, channel);

        // Send oversampling, key range, transpose
        handle.send_blocking(EngineCommand::SetInstrumentParameter {
            instrument_id: inst_id,
            param: synth_engine::InstrumentParam::OversamplingFactor(
                match inst_state.oversampling {
                    2 => synth_dsp::OversamplingFactor::X2,
                    4 => synth_dsp::OversamplingFactor::X4,
                    _ => synth_dsp::OversamplingFactor::X1,
                },
            ),
        });
        handle.send_blocking(EngineCommand::SetInstrumentParameter {
            instrument_id: inst_id,
            param: synth_engine::InstrumentParam::KeyRange(
                synth_engine::instrument::KeyRange::new(
                    synth_core::MidiNote::new(inst_state.key_range.0),
                    synth_core::MidiNote::new(inst_state.key_range.1),
                ),
            ),
        });
        handle.send_blocking(EngineCommand::SetInstrumentParameter {
            instrument_id: inst_id,
            param: synth_engine::InstrumentParam::Transpose(inst_state.transpose),
        });

        // Load modules (skip visualizers - they need GUI)
        load_patch_modules(&inst_state.patch, inst_id, session, handle);

        // Enable the instrument
        handle.send_blocking(EngineCommand::SetInstrumentEnabled {
            instrument_id: inst_id,
            enabled: true,
        });
    }

    Ok(())
}

/// Load modules and connections from a patch into the engine (no GUI state).
fn load_patch_modules(
    patch: &crate::patch::Patch,
    instrument_id: InstrumentId,
    session: &SynthSession,
    handle: &mut synth_engine::EngineHandle,
) {
    use synth_engine::ModuleId;

    // Add modules
    for module_state in &patch.modules {
        let module_id: ModuleId = match module_state.id.parse() {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Skip visualizer modules (they need VisualizationBuffer from GUI)
        if module_state.module_type.is_visualizer() {
            continue;
        }

        // Add the module via session
        let descriptor =
            match session.add_module_with_id(instrument_id, module_id, module_id.module_type) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Warning: failed to load module {}: {e}", module_state.id);
                    continue;
                }
            };

        // Apply parameters
        let effect_type = EffectType::from_module_type(module_id.module_type);
        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            effect_type,
            handle,
            instrument_id,
        );
    }

    // Add connections
    for conn in &patch.connections {
        if let (Ok(from_id), Ok(to_id)) = (
            conn.from.0.parse::<synth_engine::ModuleId>(),
            conn.to.0.parse::<synth_engine::ModuleId>(),
        ) {
            // Skip connections involving visualizer modules
            if from_id.module_type.is_visualizer() || to_id.module_type.is_visualizer() {
                continue;
            }

            handle.send_blocking(EngineCommand::Connect {
                instrument_id: Some(instrument_id),
                from: PortId::new(from_id, &*conn.from.1),
                to: PortId::new(to_id, &*conn.to.1),
            });
        }
    }

    // Apply patch-level settings (master volume, glide)
    handle.send_blocking(EngineCommand::SetMasterVolume(patch.settings.master_volume));
    handle.send_blocking(EngineCommand::SetGlideTime(patch.settings.glide_time));

    // Apply AWE settings from patch
    if let Some(awe) = &patch.settings.awe {
        handle.send_blocking(EngineCommand::SetAweEnabled {
            enabled: awe.enabled,
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::RoomShape(awe.room),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::Material(awe.material),
        });
        handle.send_blocking(EngineCommand::SetAweState {
            snapshot: awe.to_snapshot(),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::SpatialEnabled(awe.spatial_enabled),
        });
        handle.send_blocking(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::NoteMapping(awe.note_mapping),
        });
    }
}

/// Apply parameters to a module without GUI types.
///
/// Simplified version of `patch_bridge::apply_module_parameters`.
fn apply_module_parameters(
    module_id: synth_engine::ModuleId,
    descriptor: &synth_core::ModuleDescriptor,
    parameters: &HashMap<String, ParamValue>,
    effect_type: Option<EffectType>,
    handle: &mut synth_engine::EngineHandle,
    instrument_id: InstrumentId,
) {
    for (param_name, value) in parameters {
        // Match on type_id first (stable JSON key), fall back to name for legacy files.
        let param_desc = descriptor
            .parameters
            .iter()
            .find(|p| p.type_id.to_lowercase() == param_name.to_lowercase())
            .or_else(|| {
                descriptor
                    .parameters
                    .iter()
                    .find(|p| p.name.to_lowercase() == param_name.to_lowercase())
            });

        if let Some(param_desc) = param_desc {
            let f32_value = match value {
                ParamValue::Float(f) => *f,
                ParamValue::Int(i) => *i as f32,
                ParamValue::Bool(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                ParamValue::Choice(s) => {
                    if let Some(ref choices) = param_desc.choices {
                        choices
                            .iter()
                            .position(|c| c.id == *s)
                            .map(|i| i as f32)
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    }
                }
            };

            let param = param_desc.id.with_f32(f32_value);

            if let Some(et) = effect_type {
                handle.send_blocking(EngineCommand::SetEffectParameter {
                    instrument_id: Some(instrument_id),
                    effect_type: et,
                    param,
                });
            } else {
                handle.send_blocking(EngineCommand::SetModuleParameter {
                    instrument_id: Some(instrument_id),
                    module_id,
                    param,
                });
            }
        }
    }
}
