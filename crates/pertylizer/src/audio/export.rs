//! Offline audio export (WAV rendering).
//!
//! Renders the current project to a WAV file in a background thread,
//! faster than realtime. Progress is reported via atomic counters
//! so the GUI can display a progress bar without blocking.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use synth_core::audio::DeviceSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, DenormalGuard};
use synth_engine::{CommandCapacity, EngineCommand, EngineHandle, SynthEngine};
use synth_sampler::SampleLibrary;

use crate::project::ProjectFile;
use crate::session::SynthSession;

/// Shared sample library handle — the audio buffers Sampler modules reference by
/// id. The `ProjectFile` only carries the ids; the buffers live here, so the
/// export needs this to render samplers instead of silence. Matches the alias
/// used by the project loader.
pub(crate) type SharedSampleLibrary = Arc<std::sync::RwLock<SampleLibrary>>;

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

/// Write an already-rendered interleaved f32 buffer to a 32-bit float WAV file.
///
/// `samples` is channel-interleaved (`L0, R0, L1, R1, …` for stereo). This is
/// the writer the offline analysis tools (`render_to_wav`) reuse instead of
/// hand-rolling a WAV header — it shares `hound` with the project exporter
/// above. Returns the absolute peak sample amplitude seen in the buffer (0.0
/// for silence), so callers can report whether the render clipped or was empty.
pub(crate) fn write_interleaved_wav_f32(
    path: &std::path::Path,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<f32, ExportError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    let mut peak = 0.0_f32;
    for &sample in samples {
        peak = peak.max(sample.abs());
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(peak)
}

/// Start an offline WAV export in a background thread.
///
/// Returns an `ExportProgress` handle that can be polled from the GUI
/// to show a progress bar and detect completion.
pub fn start_export(
    project: ProjectFile,
    sample_library: SharedSampleLibrary,
    config: ExportConfig,
) -> ExportProgress {
    let progress = ExportProgress::new(config.total_frames());
    let progress_clone = progress.clone_internals();

    let spawn_result = std::thread::Builder::new()
        .name("wav-export".to_string())
        .spawn(move || {
            match render_to_wav(&project, &sample_library, &config, &progress_clone) {
                Ok(warnings) => {
                    if !warnings.is_empty() {
                        let msg =
                            format!("Export completed with warnings: {}", warnings.join("; "));
                        let mut err = progress_clone
                            .error
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *err = Some(msg);
                    }
                }
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
        });

    if let Err(e) = spawn_result {
        let mut err = progress.error.lock().unwrap_or_else(|e| e.into_inner());
        *err = Some(format!("Failed to spawn export thread: {e}"));
        progress.completed.store(true, Ordering::Release);
    }

    progress
}

/// Build a fresh offline engine with the *entire* project loaded and ready to
/// play: instrument voice graphs + per-instrument effect chains, the global mix
/// chain (send/return-bus channels, return effects, master effects), sampler
/// audio buffers, the Mod Grid runtime, and the master volume. This is the same
/// signal path the live engine plays, so the exported WAV matches playback.
///
/// The load runs in two phases separated by a silent `process` call: the
/// per-instrument commands are enqueued and drained first, then the mix chain +
/// sample data are enqueued into the now-empty queue. That ordering matters —
/// nothing drains the engine's command ring until `process` runs, so batching a
/// whole project at once could fill the configured ring (the same constraint
/// the MCP offline renderer works under).
fn build_loaded_export_engine(
    project: &ProjectFile,
    sample_library: &SharedSampleLibrary,
    sample_rate: u32,
) -> Result<(SynthEngine, EngineHandle, Vec<String>), ExportError> {
    build_loaded_export_engine_with_capacity(
        project,
        sample_library,
        sample_rate,
        CommandCapacity::DEFAULT,
    )
}

fn build_loaded_export_engine_with_capacity(
    project: &ProjectFile,
    sample_library: &SharedSampleLibrary,
    sample_rate: u32,
    command_capacity: CommandCapacity,
) -> Result<(SynthEngine, EngineHandle, Vec<String>), ExportError> {
    // 1. Fresh engine + a session for loading.
    let (mut engine, mut handle) = SynthEngine::with_command_capacity(command_capacity);
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    // 2. Attach the song.
    let song = synth_engine::shared_song(project.song.clone());
    let _ = handle.send_blocking(EngineCommand::SetSong {
        song: Arc::clone(&song),
    });

    // 3. Stream configuration first — sets the engine sample rate (so effects
    // added below are stamped with it) AND lets us drain the command ring by
    // processing silent blocks *during* the load. `on_stream_start` before the
    // instrument load also matches the live app, where instruments are always
    // added after the audio stream starts.
    let device_sample_rate = DeviceSampleRate::new(sample_rate);
    let stream_info = synth_core::StreamInfo {
        sample_rate: device_sample_rate,
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);
    let mut buffer = vec![0.0f32; 256 * 2];
    let drain_ctx = AudioCallbackContext {
        sample_rate: device_sample_rate,
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };

    // 4. Load all instruments (voice graphs + per-instrument effect chains),
    // draining the command ring after each so a large multi-instrument project
    // (the SetSong command included) cannot overflow it.
    let warnings = load_project_into_engine(
        project,
        &session,
        &mut handle,
        &mut engine,
        &mut buffer,
        &drain_ctx,
    )?;

    // 5. Build and ship the Mod Grid runtime so an offline render reproduces the
    // live control-rate modulation (seeded random nodes render bit-identically).
    // Sent *after* the instruments so their InstrumentId → InstrumentId mapping
    // exists when `SetModGrid` pre-creates the per-instrument offset slots for
    // `Instrument { Volume | Pan }` targets — otherwise those resolve to nothing
    // and the channel modulation silently drops (the track-scope path needs no
    // mapping, so it was unaffected).
    let mod_grid = crate::mod_grid_build::build_mod_grid_runtime(&song.read());
    let _ = handle.send_blocking(EngineCommand::SetModGrid {
        runtime: Box::new(mod_grid),
    });

    // 6. Global settings, then drain the mod-grid + global-settings commands.
    let _ = handle.send_blocking(EngineCommand::SetMasterVolume(project.global.master_volume));
    let _ = handle.send_blocking(EngineCommand::SetGlideTime(project.global.glide_time));
    crate::audio::drain_command_queue(&mut engine, &mut buffer, &drain_ctx);

    // 7. Reconstruct the global mix chain (send/return busses + return effects +
    // master effects) into the drained engine, reusing the canonical loader the
    // live app uses. `send_blocking` because this engine is not draining on its
    // own between commands — without a running audio thread, non-blocking sends
    // would drop under backpressure.
    let sender = handle.command_sender();
    crate::project_apply::apply_global_mix_chain(project, |c| sender.send_blocking(c).is_ok());

    // 8. Sampler audio buffers — referenced by id in the patch, but the data
    // lives in the shared library, not the `ProjectFile`. Without this, samplers
    // export as silence.
    crate::project_apply::push_loaded_sample_data(&sender, project, sample_library);

    // 9. Drain the mix-chain + sample-data commands before the caller plays.
    crate::audio::drain_command_queue(&mut engine, &mut buffer, &drain_ctx);

    Ok((engine, handle, warnings))
}

/// Render the project to a WAV file (blocking, runs in background thread).
///
/// Returns a list of non-fatal warnings (e.g. instruments that failed to load).
fn render_to_wav(
    project: &ProjectFile,
    sample_library: &SharedSampleLibrary,
    config: &ExportConfig,
    progress: &ExportProgress,
) -> Result<Vec<String>, ExportError> {
    // Flush denormals (FTZ/DAZ) for the whole export, matching the real-time
    // audio callback so the rendered WAV agrees with live playback and avoids
    // denormal slowdowns on decaying filter tails. Restored on return by RAII.
    let _denormal_guard = DenormalGuard::new();

    // Build a fully-loaded engine: instruments + per-instrument effects + the
    // send/return + master mix chain + sampler audio + Mod Grid + master volume.
    let (mut engine, mut handle, warnings) =
        build_loaded_export_engine(project, sample_library, config.sample_rate)?;

    let device_sample_rate = DeviceSampleRate::new(config.sample_rate);
    let channels: usize = 2;
    let buffer_size: usize = 256;
    let mut buffer = vec![0.0f32; buffer_size * channels];

    // Start playback via the handle (sends Play command through ring buffer).
    let _ = handle.send_blocking(EngineCommand::Rewind);
    let _ = handle.send_blocking(EngineCommand::Play);

    // Process one buffer to let the Play command take effect.
    buffer.fill(0.0);
    let warmup_context = AudioCallbackContext {
        sample_rate: device_sample_rate,
        frames: buffer_size,
        channels: channels as u16,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
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
            sample_rate: device_sample_rate,
            frames: this_buffer,
            channels: channels as u16,
            stream_time: frames_written as f64 / f64::from(config.sample_rate),
            sample_position: frames_written,
            output_latency: synth_core::Seconds::ZERO,
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

    Ok(warnings)
}

/// Load a `ProjectFile` into a fresh engine through the canonical project
/// instrument installer.
///
/// The command ring is drained (via `engine`/`drain_buf`/`drain_ctx`) after each
/// instrument, so a project with many instruments cannot overflow the fixed-size
/// ring while no audio callback is running.
fn load_project_into_engine(
    project: &ProjectFile,
    session: &SynthSession,
    handle: &mut synth_engine::EngineHandle,
    engine: &mut SynthEngine,
    drain_buf: &mut [f32],
    drain_ctx: &AudioCallbackContext,
) -> Result<Vec<String>, ExportError> {
    let mut warnings = Vec::new();
    let sender = handle.command_sender();

    for inst_state in &project.instruments {
        if let Err(e) = crate::project_apply::install_instrument(session, &sender, inst_state) {
            let msg = format!("Failed to load instrument '{}': {e}", inst_state.name);
            eprintln!("Warning: {msg}");
            warnings.push(msg);
        }
        crate::audio::drain_command_queue(engine, drain_buf, drain_ctx);
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::patch::{ModuleState, Position};
    use crate::project::{GlobalProjectState, ReturnBusEffectsState};
    use synth_core::ModuleType;
    use synth_engine::{InstrumentId, ModuleId};
    use synth_sequencer::Song;

    /// A minimal effect `ModuleState` with a parseable id and no parameters.
    fn effect_module(module_type: ModuleType, instance: u16) -> ModuleState {
        ModuleState {
            id: ModuleId::new(module_type, instance).to_string(),
            module_type,
            position: Position::default(),
            description: String::new(),
            parameters: BTreeMap::new(),
            scripts: BTreeMap::new(),
        }
    }

    /// Regression guard for the reported "Export WAV drops effects" bug: the
    /// offline export engine must reconstruct the *whole* mix chain — the master
    /// effect chain and the send/return busses (with their return effects) — not
    /// just the dry instrument voices. Before the fix `build_loaded_export_engine`
    /// loaded only instruments + master volume, so master and return effects were
    /// silently absent from the rendered WAV (and every send routed nowhere).
    #[test]
    fn export_engine_loads_master_and_return_mix_chain() {
        // A song with one return bus, plus a master effect and a return effect in
        // the global mix state. No instruments are needed to prove the mix chain
        // is installed — this isolates the stage that used to be dropped.
        let mut song = Song::new("Export Mix");
        let bus_id = song.create_return_bus("Reverb Return");

        let global = GlobalProjectState {
            master_effects: vec![effect_module(ModuleType::Delay, 1)],
            return_bus_effects: vec![ReturnBusEffectsState {
                id: bus_id.0,
                effects: vec![effect_module(ModuleType::Reverb, 1)],
            }],
            ..Default::default()
        };
        let project = ProjectFile::new(Vec::new(), 0, None, song, global);

        let sample_library: SharedSampleLibrary =
            Arc::new(std::sync::RwLock::new(SampleLibrary::default()));

        let (mut engine, handle, _warnings) =
            build_loaded_export_engine(&project, &sample_library, 44_100)
                .expect("build export engine");

        // Drive a few silent blocks so any residual queued commands drain and the
        // shared snapshots the assertions read are up to date.
        let ctx = AudioCallbackContext {
            sample_rate: DeviceSampleRate::new(44_100),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let mut buffer = vec![0.0f32; 256 * 2];
        for _ in 0..4 {
            buffer.fill(0.0);
            engine.process(&mut buffer, &ctx);
        }

        assert!(
            !handle.state.master_effects.read().is_empty(),
            "export engine must install the master effect chain"
        );
        let returns = handle.state.return_bus_effects.read();
        assert!(
            returns.iter().any(|bus| !bus.effects.is_empty()),
            "export engine must install send/return-bus effect chains"
        );
    }

    /// Loading a project whose command count far exceeds a deliberately small
    /// ring must not silently drop any commands.
    #[test]
    fn many_instrument_project_loads_without_dropping_commands() {
        let instruments: Vec<_> = (1..=40u64)
            .map(|i| {
                let mut inst = crate::project::default_instrument_state();
                inst.id = InstrumentId::new(i);
                inst.name = format!("inst{i}");
                inst
            })
            .collect();
        let project = ProjectFile::new(
            instruments,
            1,
            None,
            Song::new("Many"),
            GlobalProjectState::default(),
        );
        let sample_library: SharedSampleLibrary =
            Arc::new(std::sync::RwLock::new(SampleLibrary::default()));

        let Some(command_capacity) = CommandCapacity::new(64) else {
            panic!("test command capacity must be non-zero");
        };
        let (mut engine, handle, warnings) = build_loaded_export_engine_with_capacity(
            &project,
            &sample_library,
            44_100,
            command_capacity,
        )
        .expect("build export engine");
        assert!(
            warnings.is_empty(),
            "no instrument should fail to load: {warnings:?}"
        );

        // Drive a few silent blocks so instrument-snapshot mirroring settles.
        let ctx = AudioCallbackContext {
            sample_rate: DeviceSampleRate::new(44_100),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let mut buffer = vec![0.0f32; 256 * 2];
        for _ in 0..4 {
            buffer.fill(0.0);
            engine.process(&mut buffer, &ctx);
        }

        let loaded = handle.state.instrument_snapshots.read().len();
        assert_eq!(
            loaded, 40,
            "all 40 instruments must load; a dropped command would leave fewer"
        );
    }

    #[test]
    fn export_uses_the_canonical_instrument_state_hydration() {
        use synth_core::{BipolarValue, Cents, Gain, NormalizedValue, VoiceCount};
        use synth_engine::voice_allocator::{AllocationMode, StealingStrategy};

        let mut inst = crate::project::default_instrument_state();
        inst.name = "Hydrated".to_string();
        inst.description = "Shared loader metadata".to_string();
        inst.color = Some("#AABBCCFF".to_string());
        inst.category = 3;
        inst.volume = Gain::new(0.42);
        inst.pan = BipolarValue::new(-0.25);
        inst.allocation_mode = AllocationMode::Unison;
        inst.stealing_strategy = StealingStrategy::Quietest;
        inst.unison_detune = Cents::new(23.0);
        inst.unison_spread = NormalizedValue::new(0.6);
        inst.max_voices = VoiceCount::new(5);
        inst.velocity_amp_sensitivity = NormalizedValue::new(0.7);
        inst.velocity_filter_sensitivity = NormalizedValue::new(0.4);
        inst.patch.description = Some("Patch intent".to_string());
        inst.patch.color = Some("#112233FF".to_string());

        let project = ProjectFile::new(
            vec![inst],
            1,
            None,
            Song::new("Hydration"),
            GlobalProjectState::default(),
        );
        let sample_library: SharedSampleLibrary =
            Arc::new(std::sync::RwLock::new(SampleLibrary::default()));
        let (mut engine, handle, warnings) =
            build_loaded_export_engine(&project, &sample_library, 44_100)
                .expect("build export engine");
        assert!(warnings.is_empty());

        let ctx = AudioCallbackContext {
            sample_rate: DeviceSampleRate::new(44_100),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let mut buffer = vec![0.0f32; 256 * 2];
        for _ in 0..4 {
            engine.process(&mut buffer, &ctx);
        }

        let snapshots = handle.state.instrument_snapshots.read();
        let snapshot = snapshots.first().expect("hydrated instrument snapshot");
        assert_eq!(snapshot.name, "Hydrated");
        assert_eq!(snapshot.description, "Shared loader metadata");
        assert_eq!(snapshot.color.as_deref(), Some("#AABBCCFF"));
        assert_eq!(snapshot.patch_description.as_deref(), Some("Patch intent"));
        assert_eq!(snapshot.patch_color.as_deref(), Some("#112233FF"));
        assert_eq!(
            snapshot.category,
            synth_engine::InstrumentCategory::from_u8(3)
        );
        assert_eq!(snapshot.volume, Gain::new(0.42));
        assert_eq!(snapshot.pan, BipolarValue::new(-0.25));
        assert_eq!(snapshot.allocation_mode, AllocationMode::Unison);
        assert_eq!(snapshot.stealing_strategy, StealingStrategy::Quietest);
        assert_eq!(snapshot.unison_detune, Cents::new(23.0));
        assert_eq!(snapshot.unison_spread, NormalizedValue::new(0.6));
        assert_eq!(snapshot.max_voices, VoiceCount::new(5));
        assert_eq!(snapshot.velocity_amp_sensitivity, NormalizedValue::new(0.7));
        assert_eq!(
            snapshot.velocity_filter_sensitivity,
            NormalizedValue::new(0.4)
        );
    }

    /// A large single instrument must load intact through the same installer as
    /// the live project path. This guards against future offline-only shortcuts.
    #[test]
    fn single_large_instrument_loads_all_modules() {
        use crate::patch::{ModuleBuilder, Patch};
        use synth_core::ModuleType;

        const N: u16 = 100;
        let mut patch = Patch::new("Big");
        for i in 1..=N {
            patch.add_module(
                ModuleBuilder::new(i, ModuleType::Oscillator)
                    .waveform("sawtooth")
                    .param_f("level", 0.5)
                    .build(),
            );
        }
        let mut inst = crate::project::default_instrument_state();
        inst.id = InstrumentId::new(1);
        inst.name = "big".to_string();
        inst.patch = patch;
        let project = ProjectFile::new(
            vec![inst],
            1,
            None,
            Song::new("Big"),
            GlobalProjectState::default(),
        );
        let sample_library: SharedSampleLibrary =
            Arc::new(std::sync::RwLock::new(SampleLibrary::default()));

        let (mut engine, handle, warnings) =
            build_loaded_export_engine(&project, &sample_library, 44_100)
                .expect("build export engine");
        assert!(
            warnings.is_empty(),
            "no module should fail to load: {warnings:?}"
        );

        let ctx = AudioCallbackContext {
            sample_rate: DeviceSampleRate::new(44_100),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let mut buffer = vec![0.0f32; 256 * 2];
        for _ in 0..4 {
            buffer.fill(0.0);
            engine.process(&mut buffer, &ctx);
        }

        let osc_count = handle
            .state
            .shared_graph
            .get_modules_for_instrument(InstrumentId::new(1))
            .iter()
            .filter(|m| m.module_type == ModuleType::Oscillator)
            .count();
        assert_eq!(
            osc_count, N as usize,
            "all {N} oscillators must load; dropped commands would leave fewer"
        );
    }
}
