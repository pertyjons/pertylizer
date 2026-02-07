//! Sample Player module for playing audio samples.
//!
//! Plays loaded WAV samples with pitch tracking, loop modes, and interpolation.
//! Samples are loaded via `SampleManager` in the GUI thread and sent to
//! the engine via `Arc<Sample>` for thread-safe sharing.
//!
//! # Thread Safety
//!
//! - The `Sample` data is immutable and shared via `Arc`
//! - No heap allocations in the audio thread
//! - Sample loading happens in the GUI thread
//! - Position buffer uses atomics for lock-free GUI updates

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, Gain, Interpolation, MidiNote, Milliseconds, NormalizedValue, NoteReleaseState,
    PlaybackDirection, PlaybackPosition, PlaybackState, Sample, SampleRate, SampleValue, Velocity,
    WaveformOverview,
};
use synth_core::{LoopMode, ModuleType, Param, ReleaseMode, SamplePlayerParam};

// ============================================================================
// PLAYBACK POSITION BUFFER
// ============================================================================

/// Lock-free buffer for sharing playback position with GUI.
#[derive(Debug, Default)]
pub struct PlaybackPositionBuffer {
    /// Normalized position (0.0-1.0) stored as bits.
    position: AtomicU32,
}

impl PlaybackPositionBuffer {
    /// Create a new position buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            position: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    /// Set the position (called from audio thread).
    pub fn set(&self, position: f32) {
        self.position
            .store(position.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Get the position (called from GUI thread).
    #[must_use]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.position.load(Ordering::Relaxed))
    }
}

impl Clone for PlaybackPositionBuffer {
    fn clone(&self) -> Self {
        Self {
            position: AtomicU32::new(self.position.load(Ordering::Relaxed)),
        }
    }
}

// ============================================================================
// SAMPLE PLAYER
// ============================================================================

/// Sample player module for audio sample playback.
#[derive(Clone)]
pub struct SamplePlayer {
    // Parameters
    speed: NormalizedValue,
    start: NormalizedValue,
    end: NormalizedValue,
    loop_mode: LoopMode,
    loop_start: NormalizedValue,
    loop_end: NormalizedValue,
    loop_crossfade: Milliseconds,
    level: Gain,
    pan: BipolarValue,
    velocity_sensitivity: NormalizedValue,
    release_mode: ReleaseMode,

    // State
    sample: Option<Arc<Sample>>,
    position: PlaybackPosition,
    direction: PlaybackDirection,
    playback_state: PlaybackState,
    current_note: Option<MidiNote>,
    current_velocity: NormalizedValue,
    note_release_state: NoteReleaseState,

    // Multisample bank (for XM/IT instruments with multiple samples)
    sample_bank: Vec<Arc<Sample>>,
    /// Keymap: maps MIDI note (0-127) to sample index in sample_bank.
    /// If None, uses the primary `sample` field for all notes.
    sample_keymap: Option<Vec<usize>>,

    // Exact loop bounds from sample metadata (avoids f32 precision loss)
    /// Exact loop start position in frames (from sample.loop_info).
    exact_loop_start: Option<usize>,
    /// Exact loop end position in frames (from sample.loop_info).
    exact_loop_end: Option<usize>,

    // Config
    interpolation: Interpolation,
    sample_rate: SampleRate,

    // Visualization
    waveform_overview: Option<WaveformOverview>,
    position_buffer: Arc<PlaybackPositionBuffer>,

    // Output buffers
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl SamplePlayer {
    /// Create a new sample player.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Parameters
            speed: NormalizedValue::new(1.0),
            start: NormalizedValue::new(0.0),
            end: NormalizedValue::new(1.0),
            loop_mode: LoopMode::Off,
            loop_start: NormalizedValue::new(0.0),
            loop_end: NormalizedValue::new(1.0),
            loop_crossfade: Milliseconds::new(0.0),
            level: Gain::UNITY,
            pan: BipolarValue::CENTER,
            velocity_sensitivity: NormalizedValue::new(0.5),
            release_mode: ReleaseMode::Immediate,

            // State
            sample: None,
            position: PlaybackPosition::ZERO,
            direction: PlaybackDirection::Forward,
            playback_state: PlaybackState::Stopped,
            current_note: None,
            current_velocity: NormalizedValue::new(1.0),
            note_release_state: NoteReleaseState::Held,

            // Multisample bank
            sample_bank: Vec::new(),
            sample_keymap: None,

            // Exact loop bounds
            exact_loop_start: None,
            exact_loop_end: None,

            // Config
            interpolation: Interpolation::Cubic,
            sample_rate: SampleRate::DVD_QUALITY,

            // Visualization
            waveform_overview: None,
            position_buffer: Arc::new(PlaybackPositionBuffer::new()),

            // Output buffers
            output_left: AudioBuffer::new(256),
            output_right: AudioBuffer::new(256),
        }
    }

    /// Load a sample into this player.
    ///
    /// This is called from the engine when handling `LoadSample` command.
    /// Automatically applies loop and volume settings from sample metadata.
    pub fn load_sample(&mut self, sample: Arc<Sample>) {
        let sample_len = sample.len().as_usize();

        // Apply loop settings from sample metadata (from tracker import)
        if let Some(loop_info) = &sample.loop_info {
            // Convert exact sample positions to normalized values for UI
            self.loop_start = NormalizedValue::new(loop_info.normalized_start(sample_len));
            self.loop_end = NormalizedValue::new(loop_info.normalized_end(sample_len));

            // Store exact loop bounds to avoid f32 precision loss in advance_position()
            self.exact_loop_start = Some(loop_info.loop_start as usize);
            self.exact_loop_end = Some(loop_info.loop_end as usize);

            if loop_info.enabled {
                if loop_info.ping_pong {
                    self.loop_mode = LoopMode::PingPong;
                } else {
                    self.loop_mode = LoopMode::Forward;
                }
            } else {
                self.loop_mode = LoopMode::Off;
            }
        } else {
            // No loop_info - clear exact bounds, use normalized values
            self.exact_loop_start = None;
            self.exact_loop_end = None;
        }

        // Apply default volume from sample metadata
        if let Some(volume) = sample.default_volume {
            self.level = Gain::new(volume);
        }

        // Apply default panning from sample metadata
        // Sample panning is 0.0=left, 0.5=center, 1.0=right
        // BipolarValue is -1.0=left, 0.0=center, 1.0=right
        if let Some(panning) = sample.default_panning {
            self.pan = BipolarValue::new((panning * 2.0) - 1.0);
        }

        // Set release mode based on loop settings:
        // - Looped samples: Immediate (stop at note-off, common for sustained sounds)
        // - Non-looped samples: PlayToEnd (let sample play through, typical for drums/one-shots)
        if self.loop_mode == LoopMode::Off {
            self.release_mode = ReleaseMode::PlayToEnd;
        } else {
            self.release_mode = ReleaseMode::Immediate;
        }

        // Generate waveform overview for visualization
        self.waveform_overview = Some(WaveformOverview::generate(&sample, 200));
        self.sample = Some(sample);
        self.reset();
    }

    /// Clear the loaded sample.
    pub fn clear_sample(&mut self) {
        self.sample = None;
        self.waveform_overview = None;
        self.playback_state = PlaybackState::Stopped;
        self.position = PlaybackPosition::ZERO;
        self.exact_loop_start = None;
        self.exact_loop_end = None;
    }

    /// Add a sample to the sample bank (for multisample instruments).
    ///
    /// The sample bank is used with `sample_keymap` to select different
    /// samples based on MIDI note. Call this for each sample in the instrument,
    /// then use `set_sample_keymap` to define the note-to-sample mapping.
    pub fn add_sample_to_bank(&mut self, sample: Arc<Sample>) {
        self.sample_bank.push(sample);
    }

    /// Set the sample keymap for multisample instruments.
    ///
    /// The keymap should have 128 entries (one per MIDI note 0-127),
    /// where each entry is an index into the sample_bank.
    /// If set, note_on will select the appropriate sample from the bank.
    pub fn set_sample_keymap(&mut self, keymap: Vec<usize>) {
        self.sample_keymap = Some(keymap);
    }

    /// Select the active sample from the bank based on MIDI note.
    ///
    /// If a keymap is set and samples are in the bank, this selects the
    /// appropriate sample and applies its settings. Otherwise uses the
    /// primary `sample` field.
    fn select_sample_for_note(&mut self, note: MidiNote) {
        // If no keymap or empty bank, use primary sample
        let Some(keymap) = &self.sample_keymap else {
            return;
        };
        if self.sample_bank.is_empty() {
            return;
        }

        // Get sample index from keymap (clamp note to valid range)
        let note_idx = (note.as_u8() as usize).min(127);
        let sample_idx = keymap.get(note_idx).copied().unwrap_or(0);
        let sample_idx = sample_idx.min(self.sample_bank.len().saturating_sub(1));

        // Get the sample from the bank
        if let Some(sample) = self.sample_bank.get(sample_idx) {
            // Apply sample-specific settings without regenerating waveform
            let sample_len = sample.len().as_usize();

            // Apply loop settings from this sample
            if let Some(loop_info) = &sample.loop_info {
                self.loop_start = NormalizedValue::new(loop_info.normalized_start(sample_len));
                self.loop_end = NormalizedValue::new(loop_info.normalized_end(sample_len));

                // Store exact loop bounds to avoid f32 precision loss
                self.exact_loop_start = Some(loop_info.loop_start as usize);
                self.exact_loop_end = Some(loop_info.loop_end as usize);

                if loop_info.enabled {
                    self.loop_mode = if loop_info.ping_pong {
                        LoopMode::PingPong
                    } else {
                        LoopMode::Forward
                    };
                } else {
                    self.loop_mode = LoopMode::Off;
                }
            } else {
                self.exact_loop_start = None;
                self.exact_loop_end = None;
            }

            // Apply volume from sample
            if let Some(volume) = sample.default_volume {
                self.level = Gain::new(volume);
            }

            // Apply panning from sample
            if let Some(panning) = sample.default_panning {
                self.pan = BipolarValue::new((panning * 2.0) - 1.0);
            }

            // Set release mode based on loop
            self.release_mode = if self.loop_mode == LoopMode::Off {
                ReleaseMode::PlayToEnd
            } else {
                ReleaseMode::Immediate
            };

            // Set the active sample
            self.sample = Some(Arc::clone(sample));
        }
    }

    /// Get the waveform overview for visualization.
    #[must_use]
    pub fn waveform_overview(&self) -> Option<&WaveformOverview> {
        self.waveform_overview.as_ref()
    }

    /// Get the position buffer for GUI sync.
    #[must_use]
    pub fn position_buffer(&self) -> Arc<PlaybackPositionBuffer> {
        Arc::clone(&self.position_buffer)
    }

    /// Get the sample length in frames.
    fn sample_len(&self) -> usize {
        self.sample
            .as_ref()
            .map(|s| s.len().as_usize())
            .unwrap_or(0)
    }

    /// Get the effective start position in frames.
    fn start_frame(&self) -> usize {
        let len = self.sample_len();
        (self.start.as_f32() * len as f32) as usize
    }

    /// Get the effective end position in frames.
    fn end_frame(&self) -> usize {
        let len = self.sample_len();
        ((self.end.as_f32() * len as f32) as usize).min(len)
    }

    /// Get the loop start position in frames.
    fn loop_start_frame(&self) -> usize {
        let len = self.sample_len();
        (self.loop_start.as_f32() * len as f32) as usize
    }

    /// Get the loop end position in frames.
    fn loop_end_frame(&self) -> usize {
        let len = self.sample_len();
        ((self.loop_end.as_f32() * len as f32) as usize).min(len)
    }

    /// Get crossfade length in samples.
    fn crossfade_samples(&self) -> usize {
        (self.loop_crossfade.as_f32() * self.sample_rate.as_f32() / 1000.0) as usize
    }

    /// Calculate playback speed including pitch tracking.
    fn effective_speed(&self) -> f64 {
        let base_speed = self.speed.as_f32() as f64;

        // Apply pitch ratio if we have a sample and a current note
        let pitch_ratio = if let (Some(sample), Some(note)) = (&self.sample, self.current_note) {
            sample.pitch_ratio(note) as f64
        } else {
            1.0
        };

        // Apply sample rate conversion
        let rate_ratio = if let Some(sample) = &self.sample {
            sample.sample_rate.as_f32() as f64 / self.sample_rate.as_f32() as f64
        } else {
            1.0
        };

        base_speed * pitch_ratio * rate_ratio
    }

    /// Calculate effective level including velocity.
    fn effective_level(&self) -> f32 {
        let vel_factor =
            1.0 - self.velocity_sensitivity.as_f32() * (1.0 - self.current_velocity.as_f32());
        self.level.as_f32() * vel_factor
    }

    /// Advance position and handle loop modes.
    fn advance_position(&mut self, speed: f64) {
        let delta = speed * self.direction.sign();
        self.position = self.position.advance(delta);

        let pos = self.position.as_f64();
        // Use exact loop bounds if available (from sample.loop_info) to avoid f32 precision loss.
        // This ensures advance_position() and read_with_crossfade() use the same loop points.
        let loop_start = self
            .exact_loop_start
            .unwrap_or_else(|| self.loop_start_frame()) as f64;
        let loop_end = self.exact_loop_end.unwrap_or_else(|| self.loop_end_frame()) as f64;
        let start = self.start_frame() as f64;
        let end = self.end_frame() as f64;

        // When releasing, don't loop - play to end (or loop end for PlayToLoop) and stop
        if self.note_release_state.is_released() {
            // For PlayToLoop mode with active loop, stop at loop_end instead of sample end
            let stop_position = if self.release_mode == ReleaseMode::PlayToLoop
                && self.loop_mode != LoopMode::Off
            {
                loop_end
            } else {
                end
            };

            // Account for position being clamped to sample_len-1
            let at_end =
                pos >= stop_position || pos >= (self.sample_len().saturating_sub(1)) as f64;
            if at_end || pos < start {
                self.playback_state = PlaybackState::Stopped;
                self.position = if at_end {
                    PlaybackPosition::new(stop_position - 1.0)
                } else {
                    PlaybackPosition::new(start)
                };
            }
            return;
        }

        match self.loop_mode {
            LoopMode::Off => {
                // Stop at end (or start if playing backward)
                if pos >= end || pos < start {
                    self.playback_state = PlaybackState::Stopped;
                    self.position = if pos >= end {
                        PlaybackPosition::new(end - 1.0)
                    } else {
                        PlaybackPosition::new(start)
                    };
                }
            }
            LoopMode::Forward => {
                // Note: pos is clamped to sample_len-1, so we need to check >= loop_end - 1
                // when loop_end equals sample_len
                let at_loop_end = pos >= loop_end || (loop_end >= end && pos >= end - 1.0);
                if at_loop_end {
                    let overshoot = pos - loop_end;
                    let new_pos = if overshoot >= 0.0 {
                        // Actual overshoot - wrap within loop region
                        loop_start + overshoot.rem_euclid(loop_end - loop_start)
                    } else {
                        // Triggered by clamping (pos at end-1) - go directly to loop start
                        loop_start
                    };
                    self.position = PlaybackPosition::new(new_pos);
                }
            }
            LoopMode::Backward => {
                if pos < loop_start {
                    self.position = PlaybackPosition::new(
                        loop_end - (loop_start - pos).rem_euclid(loop_end - loop_start),
                    );
                }
            }
            LoopMode::PingPong => {
                // Account for position being clamped to sample_len-1
                let at_loop_end = pos >= loop_end || (loop_end >= end && pos >= end - 1.0);
                if self.direction == PlaybackDirection::Forward && at_loop_end {
                    // Preserve overshoot: reflect excess distance back from loop_end
                    let overshoot = (pos - loop_end).max(0.0);
                    self.direction = PlaybackDirection::Backward;
                    self.position =
                        PlaybackPosition::new((loop_end - 1.0 - overshoot).max(loop_start));
                } else if self.direction == PlaybackDirection::Backward && pos <= loop_start {
                    // Preserve overshoot: reflect excess distance forward from loop_start
                    let overshoot = (loop_start - pos).max(0.0);
                    self.direction = PlaybackDirection::Forward;
                    self.position =
                        PlaybackPosition::new((loop_start + overshoot).min(loop_end - 1.0));
                }
            }
        }

        // Clamp position to valid range
        let pos = self
            .position
            .as_f64()
            .clamp(0.0, (self.sample_len().saturating_sub(1)) as f64);
        self.position = PlaybackPosition::new(pos);
    }

    /// Read sample with loop-aware interpolation and crossfade.
    ///
    /// Uses exact loop points from sample metadata when available to ensure
    /// click-free playback at loop boundaries.
    fn read_with_crossfade(
        &self,
        sample: &Sample,
        position: PlaybackPosition,
    ) -> (SampleValue, SampleValue) {
        let crossfade_samples = self.crossfade_samples();

        // Get exact loop bounds (prefer metadata, fall back to normalized values)
        let (loop_start, loop_end) = if let Some(loop_info) = &sample.loop_info {
            (loop_info.loop_start as usize, loop_info.loop_end as usize)
        } else {
            (self.loop_start_frame(), self.loop_end_frame())
        };

        // Use standard read for non-looping mode
        if self.loop_mode == LoopMode::Off {
            return sample.read(position, self.interpolation);
        }

        // Use loop-aware interpolation for looping modes
        let pos = position.as_f64();
        let distance_to_end = loop_end as f64 - pos;

        // Check if we're in the crossfade region
        if crossfade_samples > 0
            && distance_to_end < crossfade_samples as f64
            && distance_to_end > 0.0
        {
            let fade_amount = (distance_to_end / crossfade_samples as f64) as f32;

            // Read current position with loop-aware interpolation
            let (l1, r1) = sample.read_looped(position, self.interpolation, loop_start, loop_end);

            // Read from loop start (offset by how far we are from loop end)
            let loop_start_offset = crossfade_samples as f64 - distance_to_end;
            let crossfade_pos = PlaybackPosition::new(loop_start as f64 + loop_start_offset);
            let (l2, r2) =
                sample.read_looped(crossfade_pos, self.interpolation, loop_start, loop_end);

            // Crossfade
            let left = l1.scale(fade_amount) + l2.scale(1.0 - fade_amount);
            let right = r1.scale(fade_amount) + r2.scale(1.0 - fade_amount);

            (left, right)
        } else {
            // Use loop-aware interpolation even outside crossfade region
            sample.read_looped(position, self.interpolation, loop_start, loop_end)
        }
    }
}

impl Default for SamplePlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for SamplePlayer {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("sample_player", "Sample Player")
            .description("Play audio samples with pitch tracking and looping")
            .category(ModuleCategory::Oscillator)
            .tag("sample")
            .tag("playback")
            .tag("audio")
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::Speed(NormalizedValue::new(1.0))),
                    "Speed",
                )
                .description("Playback speed multiplier")
                .range(0.1, 4.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::Start(NormalizedValue::new(0.0))),
                    "Start",
                )
                .description("Start position in sample")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::End(NormalizedValue::new(1.0))),
                    "End",
                )
                .description("End position in sample")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    Param::SamplePlayer(SamplePlayerParam::LoopMode(LoopMode::Off)),
                    "Loop",
                    LoopMode::to_choices(),
                )
                .description("Loop playback mode")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::LoopStart(NormalizedValue::new(0.0))),
                    "Loop Start",
                )
                .description("Loop start position")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::LoopEnd(NormalizedValue::new(1.0))),
                    "Loop End",
                )
                .description("Loop end position")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::LoopCrossfade(Milliseconds::new(0.0))),
                    "X-Fade",
                )
                .description("Loop crossfade time")
                .range(0.0, 50.0)
                .default(0.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::VelocitySensitivity(
                        NormalizedValue::new(0.5),
                    )),
                    "Vel Sens",
                )
                .description("Velocity sensitivity")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    Param::SamplePlayer(SamplePlayerParam::ReleaseMode(ReleaseMode::Immediate)),
                    "Release",
                    ReleaseMode::to_choices(),
                )
                .description("Note-off behavior")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SamplePlayer(SamplePlayerParam::Pan(BipolarValue::CENTER)),
                    "Pan",
                )
                .description("Stereo pan position (-1 left, 0 center, +1 right)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    Param::SamplePlayer(SamplePlayerParam::Interpolation(Interpolation::Cubic)),
                    "Interp",
                    Interpolation::to_choices(),
                )
                .description("Sample interpolation quality")
                .widget(WidgetHint::Dropdown),
            )
            .port(
                PortDescriptor::control_input("pitch_mod", "Pitch")
                    .description("Pitch modulation in semitones"),
            )
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(PortDescriptor::audio_output("out", "Out").description("Mono output"))
    }
}

impl PolyModule for SamplePlayer {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples.as_usize());
        self.output_right.resize(context.samples.as_usize());

        // Clone Arc for use in the loop (avoids borrow conflict)
        let sample = match &self.sample {
            Some(s) => Arc::clone(s),
            None => {
                // No sample loaded - output silence
                self.output_left.clear();
                self.output_right.clear();
                self.position_buffer.set(0.0);

                if let Some(out_l) = outputs.get_mut("out_l") {
                    out_l.copy_from(&self.output_left);
                }
                if let Some(out_r) = outputs.get_mut("out_r") {
                    out_r.copy_from(&self.output_right);
                }
                if let Some(out) = outputs.get_mut("out") {
                    out.copy_from(&self.output_left);
                }
                return;
            }
        };

        let base_speed = self.effective_speed();
        let level = self.effective_level();
        let sample_len = self.sample_len();

        // Calculate pan coefficients (constant for this buffer)
        let (pan_left, pan_right) = Gain::from_pan(self.pan);
        let pan_l = pan_left.as_f32();
        let pan_r = pan_right.as_f32();

        // Get pitch modulation input (semitones)
        let pitch_mod = inputs.get(PortName::intern("pitch_mod"));

        // Optimization: if no pitch modulation, use constant speed (avoids expensive powf per sample)
        let has_pitch_mod = pitch_mod.is_some();

        for i in 0..context.samples.as_usize() {
            if self.playback_state.is_playing() {
                // Apply per-sample pitch modulation only if present
                // pitch_mod is in semitones: speed_ratio = 2^(semitones/12)
                let speed = if has_pitch_mod {
                    let pitch_offset = pitch_mod.map(|cv| cv[i]).unwrap_or(0.0);
                    base_speed * 2.0_f64.powf(pitch_offset as f64 / 12.0)
                } else {
                    base_speed
                };

                let (left, right) = self.read_with_crossfade(&sample, self.position);
                // Apply level and panning
                self.output_left[i] = left.as_f32() * level * pan_l;
                self.output_right[i] = right.as_f32() * level * pan_r;
                self.advance_position(speed);
            } else {
                self.output_left[i] = 0.0;
                self.output_right[i] = 0.0;
            }
        }

        // Update position buffer for GUI (normalized 0.0-1.0)
        if sample_len > 0 {
            let normalized_pos = self.position.as_f64() / sample_len as f64;
            self.position_buffer.set(normalized_pos as f32);
        }

        if let Some(out_l) = outputs.get_mut("out_l") {
            out_l.copy_from(&self.output_left);
        }
        if let Some(out_r) = outputs.get_mut("out_r") {
            out_r.copy_from(&self.output_right);
        }
        if let Some(out) = outputs.get_mut("out") {
            // Mono output = average of L/R
            for i in 0..context.samples.as_usize() {
                out[i] = (self.output_left[i] + self.output_right[i]) * 0.5;
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SamplePlayer(sp) = param {
            match sp {
                SamplePlayerParam::Speed(v) => self.speed = v,
                SamplePlayerParam::Start(v) => self.start = v,
                SamplePlayerParam::End(v) => self.end = v,
                SamplePlayerParam::LoopMode(m) => self.loop_mode = m,
                SamplePlayerParam::LoopStart(v) => self.loop_start = v,
                SamplePlayerParam::LoopEnd(v) => self.loop_end = v,
                SamplePlayerParam::LoopCrossfade(ms) => self.loop_crossfade = ms,
                SamplePlayerParam::Level(g) => self.level = g,
                SamplePlayerParam::Pan(p) => self.pan = p,
                SamplePlayerParam::VelocitySensitivity(v) => self.velocity_sensitivity = v,
                SamplePlayerParam::ReleaseMode(m) => self.release_mode = m,
                SamplePlayerParam::Interpolation(i) => self.interpolation = i,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SamplePlayer(sp) = param {
            Some(match sp {
                SamplePlayerParam::Speed(_) => self.speed.as_f32(),
                SamplePlayerParam::Start(_) => self.start.as_f32(),
                SamplePlayerParam::End(_) => self.end.as_f32(),
                SamplePlayerParam::LoopMode(_) => self.loop_mode.index() as f32,
                SamplePlayerParam::LoopStart(_) => self.loop_start.as_f32(),
                SamplePlayerParam::LoopEnd(_) => self.loop_end.as_f32(),
                SamplePlayerParam::LoopCrossfade(_) => self.loop_crossfade.as_f32(),
                SamplePlayerParam::Level(_) => self.level.as_f32(),
                SamplePlayerParam::Pan(_) => self.pan.as_f32(),
                SamplePlayerParam::VelocitySensitivity(_) => self.velocity_sensitivity.as_f32(),
                SamplePlayerParam::ReleaseMode(_) => self.release_mode.index() as f32,
                SamplePlayerParam::Interpolation(_) => self.interpolation.index() as f32,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::SamplePlayer(SamplePlayerParam::Speed(self.speed)),
            Param::SamplePlayer(SamplePlayerParam::Start(self.start)),
            Param::SamplePlayer(SamplePlayerParam::End(self.end)),
            Param::SamplePlayer(SamplePlayerParam::LoopMode(self.loop_mode)),
            Param::SamplePlayer(SamplePlayerParam::LoopStart(self.loop_start)),
            Param::SamplePlayer(SamplePlayerParam::LoopEnd(self.loop_end)),
            Param::SamplePlayer(SamplePlayerParam::LoopCrossfade(self.loop_crossfade)),
            Param::SamplePlayer(SamplePlayerParam::Level(self.level)),
            Param::SamplePlayer(SamplePlayerParam::VelocitySensitivity(
                self.velocity_sensitivity,
            )),
            Param::SamplePlayer(SamplePlayerParam::Pan(self.pan)),
            Param::SamplePlayer(SamplePlayerParam::ReleaseMode(self.release_mode)),
            Param::SamplePlayer(SamplePlayerParam::Interpolation(self.interpolation)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SamplePlayer
    }

    fn reset(&mut self) {
        self.position = PlaybackPosition::new(self.start_frame() as f64);
        self.direction = PlaybackDirection::Forward;
        self.playback_state = PlaybackState::Stopped;
        self.note_release_state = NoteReleaseState::Held;
    }

    fn note_on(&mut self, note: MidiNote, velocity: Velocity) {
        // Select sample from bank if multisample keymap is set
        self.select_sample_for_note(note);

        self.current_note = Some(note);
        self.current_velocity = NormalizedValue::new(velocity.as_f32());

        // Set initial direction and position based on loop mode.
        // Backward mode starts from loop end playing in reverse.
        if self.loop_mode == LoopMode::Backward {
            self.direction = PlaybackDirection::Backward;
            let loop_end = self.exact_loop_end.unwrap_or_else(|| self.loop_end_frame());
            self.position = PlaybackPosition::new(loop_end.saturating_sub(1) as f64);
        } else {
            self.direction = PlaybackDirection::Forward;
            self.position = PlaybackPosition::new(self.start_frame() as f64);
        }

        self.playback_state = PlaybackState::Playing;
        self.note_release_state = NoteReleaseState::Held;
    }

    fn note_off(&mut self) {
        match self.release_mode {
            ReleaseMode::Immediate => {
                // Stop immediately unless in a loop mode
                if self.loop_mode == LoopMode::Off {
                    self.playback_state = PlaybackState::Stopped;
                } else {
                    // In loop mode, start releasing (will play to end)
                    self.note_release_state = NoteReleaseState::Released;
                }
            }
            ReleaseMode::PlayToEnd => {
                // Disable looping and play to end
                self.note_release_state = NoteReleaseState::Released;
            }
            ReleaseMode::PlayToLoop => {
                // Play to loop end, then stop
                self.note_release_state = NoteReleaseState::Released;
            }
        }
    }

    fn retrigger_with_offset(&mut self, sample_offset: NormalizedValue) {
        // Retrigger playback from the given offset position
        // Used for tracker retrigger effects (Exy) and note delay (EDx)
        let sample_len = self.sample_len();
        if sample_len == 0 {
            return;
        }

        // Calculate start position from offset (0.0-1.0 of sample length)
        let start_pos = (sample_offset.as_f32() * sample_len as f32) as f64;
        let clamped_pos = start_pos.clamp(0.0, (sample_len - 1) as f64);

        self.position = PlaybackPosition::new(clamped_pos);
        self.direction = PlaybackDirection::Forward;
        self.playback_state = PlaybackState::Playing;
        self.note_release_state = NoteReleaseState::Held;
    }

    fn load_sample(&mut self, sample: std::sync::Arc<Sample>) -> bool {
        // Use the existing load_sample method
        Self::load_sample(self, sample);
        true
    }

    fn load_sample_bank(
        &mut self,
        samples: Vec<std::sync::Arc<Sample>>,
        keymap: Vec<usize>,
    ) -> bool {
        // Load all samples into the sample bank
        self.sample_bank = samples;
        self.sample_keymap = Some(keymap);

        // Also load the first sample as the primary sample for initial playback
        // (will be overridden by select_sample_for_note on note_on)
        if let Some(first_sample) = self.sample_bank.first() {
            Self::load_sample(self, Arc::clone(first_sample));
        }

        true
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::ChannelMode;

    fn create_test_sample() -> Arc<Sample> {
        // Create a simple mono sine wave sample
        let data: Vec<SampleValue> = (0..1000)
            .map(|i| SampleValue::new((i as f32 * 0.01).sin()))
            .collect();

        Arc::new(Sample::new(
            "test",
            data,
            ChannelMode::Mono,
            SampleRate::CD_QUALITY,
        ))
    }

    #[test]
    fn test_sample_player_creation() {
        let player = SamplePlayer::new();
        assert!(player.playback_state.is_stopped());
        assert!(player.sample.is_none());
    }

    #[test]
    fn test_load_sample() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();

        player.load_sample(sample);
        assert!(player.sample.is_some());
        assert_eq!(player.sample_len(), 1000);
        assert!(player.waveform_overview.is_some());
    }

    #[test]
    fn test_note_on_starts_playback() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();
        player.load_sample(sample);

        player.note_on(MidiNote::C4, Velocity::MAX);
        assert!(player.playback_state.is_playing());
    }

    #[test]
    fn test_note_off_releases_playback() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();
        player.load_sample(sample);

        player.note_on(MidiNote::C4, Velocity::MAX);
        player.note_off();

        // With PlayToEnd release mode (default for non-looped samples),
        // note_off marks the note as released but lets it play to the end
        assert!(matches!(
            player.note_release_state,
            NoteReleaseState::Released
        ));
        // Still playing until sample reaches end
        assert!(player.playback_state.is_playing());
    }

    #[test]
    fn test_note_off_immediate_stops_playback() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();
        player.load_sample(sample);
        // Override to Immediate mode
        player.release_mode = ReleaseMode::Immediate;

        player.note_on(MidiNote::C4, Velocity::MAX);
        player.note_off();

        // With Immediate release mode and no loop, should stop immediately
        assert!(player.playback_state.is_stopped());
    }

    #[test]
    fn test_velocity_sensitivity() {
        let mut player = SamplePlayer::new();
        player.velocity_sensitivity = NormalizedValue::new(1.0);
        player.current_velocity = NormalizedValue::new(0.5);

        // With full sensitivity and 0.5 velocity, level should be 0.5
        assert!((player.effective_level() - 0.5).abs() < 0.001);

        player.velocity_sensitivity = NormalizedValue::new(0.0);
        // With no sensitivity, level should be 1.0
        assert!((player.effective_level() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_position_buffer() {
        let player = SamplePlayer::new();
        let buffer = player.position_buffer();

        buffer.set(0.5);
        assert!((buffer.get() - 0.5).abs() < 0.001);
    }
}
