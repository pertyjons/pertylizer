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
    Gain, Interpolation, MidiNote, Milliseconds, NormalizedValue, NoteReleaseState,
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
            loop_crossfade: Milliseconds::new(5.0),
            level: Gain::UNITY,
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
        // Apply loop settings from sample metadata (from tracker import)
        if let Some(loop_info) = &sample.loop_info {
            self.loop_start = NormalizedValue::new(loop_info.loop_start);
            self.loop_end = NormalizedValue::new(loop_info.loop_end);
            if loop_info.enabled {
                if loop_info.ping_pong {
                    self.loop_mode = LoopMode::PingPong;
                } else {
                    self.loop_mode = LoopMode::Forward;
                }
            } else {
                self.loop_mode = LoopMode::Off;
            }
        }

        // Apply default volume from sample metadata
        if let Some(volume) = sample.default_volume {
            self.level = Gain::new(volume);
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
        let loop_start = self.loop_start_frame() as f64;
        let loop_end = self.loop_end_frame() as f64;
        let start = self.start_frame() as f64;
        let end = self.end_frame() as f64;

        // When releasing, don't loop
        if self.note_release_state.is_released() {
            if pos >= end || pos < start {
                self.playback_state = PlaybackState::Stopped;
                self.position = if pos >= end {
                    PlaybackPosition::new(end - 1.0)
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
                if pos >= loop_end {
                    self.position = PlaybackPosition::new(
                        loop_start + (pos - loop_end).rem_euclid(loop_end - loop_start),
                    );
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
                if self.direction == PlaybackDirection::Forward && pos >= loop_end {
                    self.direction = PlaybackDirection::Backward;
                    self.position = PlaybackPosition::new(loop_end - (pos - loop_end));
                } else if self.direction == PlaybackDirection::Backward && pos < loop_start {
                    self.direction = PlaybackDirection::Forward;
                    self.position = PlaybackPosition::new(loop_start + (loop_start - pos));
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

    /// Read sample with loop crossfade.
    fn read_with_crossfade(
        &self,
        sample: &Sample,
        position: PlaybackPosition,
    ) -> (SampleValue, SampleValue) {
        let crossfade_samples = self.crossfade_samples();

        // Only apply crossfade in loop modes
        if self.loop_mode == LoopMode::Off || crossfade_samples == 0 {
            return sample.read(position, self.interpolation);
        }

        let pos = position.as_f64();
        let loop_start = self.loop_start_frame() as f64;
        let loop_end = self.loop_end_frame() as f64;
        let distance_to_end = loop_end - pos;

        // Check if we're in the crossfade region
        if distance_to_end < crossfade_samples as f64 && distance_to_end > 0.0 {
            let fade_amount = (distance_to_end / crossfade_samples as f64) as f32;

            // Read current position
            let (l1, r1) = sample.read(position, self.interpolation);

            // Read from loop start (offset by how far we are from loop end)
            let loop_start_offset = crossfade_samples as f64 - distance_to_end;
            let crossfade_pos = PlaybackPosition::new(loop_start + loop_start_offset);
            let (l2, r2) = sample.read(crossfade_pos, self.interpolation);

            // Crossfade
            let left = l1.scale(fade_amount) + l2.scale(1.0 - fade_amount);
            let right = r1.scale(fade_amount) + r2.scale(1.0 - fade_amount);

            (left, right)
        } else {
            sample.read(position, self.interpolation)
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
                    Param::SamplePlayer(SamplePlayerParam::LoopCrossfade(Milliseconds::new(5.0))),
                    "X-Fade",
                )
                .description("Loop crossfade time")
                .range(0.0, 50.0)
                .default(5.0)
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

        // Get pitch modulation input (semitones)
        let pitch_mod = inputs.get(PortName::intern("pitch_mod"));

        for i in 0..context.samples.as_usize() {
            if self.playback_state.is_playing() {
                // Apply per-sample pitch modulation
                // pitch_mod is in semitones: speed_ratio = 2^(semitones/12)
                let pitch_offset = pitch_mod.map(|cv| cv[i]).unwrap_or(0.0);
                let speed = base_speed * 2.0_f64.powf(pitch_offset as f64 / 12.0);

                let (left, right) = self.read_with_crossfade(&sample, self.position);
                self.output_left[i] = left.as_f32() * level;
                self.output_right[i] = right.as_f32() * level;
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
        self.current_note = Some(note);
        self.current_velocity = NormalizedValue::new(velocity.as_f32());
        self.position = PlaybackPosition::new(self.start_frame() as f64);
        self.direction = PlaybackDirection::Forward;
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

    fn load_sample(&mut self, sample: std::sync::Arc<Sample>) -> bool {
        // Use the existing load_sample method
        SamplePlayer::load_sample(self, sample);
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
