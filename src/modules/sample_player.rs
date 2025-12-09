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

use std::collections::HashMap;
use std::sync::Arc;

use crate::engine::typed_params::{LoopMode, ModuleType, Param, SamplePlayerParam};
use crate::modules::core::*;
use crate::types::{
    Gain, Interpolation, MidiNote, NormalizedValue, PlaybackDirection, PlaybackPosition, Sample,
    SampleRate,
};

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
    level: Gain,

    // State
    sample: Option<Arc<Sample>>,
    position: PlaybackPosition,
    direction: PlaybackDirection,
    playing: bool,
    current_note: Option<MidiNote>,

    // Config
    interpolation: Interpolation,
    sample_rate: SampleRate,

    // Output buffers
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl SamplePlayer {
    /// Create a new sample player.
    pub fn new() -> Self {
        Self {
            // Parameters
            speed: NormalizedValue::new(1.0),
            start: NormalizedValue::new(0.0),
            end: NormalizedValue::new(1.0),
            loop_mode: LoopMode::Off,
            loop_start: NormalizedValue::new(0.0),
            loop_end: NormalizedValue::new(1.0),
            level: Gain::UNITY,

            // State
            sample: None,
            position: PlaybackPosition::ZERO,
            direction: PlaybackDirection::Forward,
            playing: false,
            current_note: None,

            // Config
            interpolation: Interpolation::Linear,
            sample_rate: SampleRate::DVD_QUALITY,

            // Output buffers
            output_left: AudioBuffer::new(256),
            output_right: AudioBuffer::new(256),
        }
    }

    /// Load a sample into this player.
    ///
    /// This is called from the engine when handling `LoadSample` command.
    pub fn load_sample(&mut self, sample: Arc<Sample>) {
        self.sample = Some(sample);
        self.reset();
    }

    /// Clear the loaded sample.
    pub fn clear_sample(&mut self) {
        self.sample = None;
        self.playing = false;
        self.position = PlaybackPosition::ZERO;
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

    /// Advance position and handle loop modes.
    fn advance_position(&mut self, speed: f64) {
        let delta = speed * self.direction.sign();
        self.position = self.position.advance(delta);

        let pos = self.position.as_f64();
        let loop_start = self.loop_start_frame() as f64;
        let loop_end = self.loop_end_frame() as f64;
        let start = self.start_frame() as f64;
        let end = self.end_frame() as f64;

        match self.loop_mode {
            LoopMode::Off => {
                // Stop at end (or start if playing backward)
                if pos >= end || pos < start {
                    self.playing = false;
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
            .clamp(0.0, (self.sample_len() - 1).max(0) as f64);
        self.position = PlaybackPosition::new(pos);
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
                    Param::SamplePlayer(SamplePlayerParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(PortDescriptor::audio_output("out", "Out").description("Mono output"))
    }
}

impl PolyModule for SamplePlayer {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples);
        self.output_right.resize(context.samples);

        // Clone Arc for use in the loop (avoids borrow conflict)
        let sample = match &self.sample {
            Some(s) => Arc::clone(s),
            None => {
                // No sample loaded - output silence
                self.output_left.clear();
                self.output_right.clear();

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

        let speed = self.effective_speed();
        let level = self.level.as_f32();

        for i in 0..context.samples {
            if self.playing {
                let (left, right) = sample.read(self.position, self.interpolation);
                self.output_left[i] = left.as_f32() * level;
                self.output_right[i] = right.as_f32() * level;
                self.advance_position(speed);
            } else {
                self.output_left[i] = 0.0;
                self.output_right[i] = 0.0;
            }
        }

        if let Some(out_l) = outputs.get_mut("out_l") {
            out_l.copy_from(&self.output_left);
        }
        if let Some(out_r) = outputs.get_mut("out_r") {
            out_r.copy_from(&self.output_right);
        }
        if let Some(out) = outputs.get_mut("out") {
            // Mono output = average of L/R
            for i in 0..context.samples {
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
                SamplePlayerParam::Level(g) => self.level = g,
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
                SamplePlayerParam::Level(_) => self.level.as_f32(),
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
            Param::SamplePlayer(SamplePlayerParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SamplePlayer
    }

    fn reset(&mut self) {
        self.position = PlaybackPosition::new(self.start_frame() as f64);
        self.direction = PlaybackDirection::Forward;
        self.playing = false;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: f32) {
        self.current_note = Some(note);
        self.position = PlaybackPosition::new(self.start_frame() as f64);
        self.direction = PlaybackDirection::Forward;
        self.playing = true;
    }

    fn note_off(&mut self) {
        // Stop playing unless we're in a loop mode
        if self.loop_mode == LoopMode::Off {
            self.playing = false;
        }
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelMode, SampleValue};

    fn create_test_sample() -> Arc<Sample> {
        // Create a simple mono sine wave sample
        let data: Vec<SampleValue> = (0..1000)
            .map(|i| SampleValue::new((i as f32 * 0.01).sin()))
            .collect();

        Arc::new(Sample::new(
            "test".to_string(),
            data,
            ChannelMode::Mono,
            SampleRate::CD_QUALITY,
        ))
    }

    #[test]
    fn test_sample_player_creation() {
        let player = SamplePlayer::new();
        assert!(!player.playing);
        assert!(player.sample.is_none());
    }

    #[test]
    fn test_load_sample() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();

        player.load_sample(sample);
        assert!(player.sample.is_some());
        assert_eq!(player.sample_len(), 1000);
    }

    #[test]
    fn test_note_on_starts_playback() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();
        player.load_sample(sample);

        player.note_on(MidiNote::C4, 1.0);
        assert!(player.playing);
    }

    #[test]
    fn test_note_off_stops_playback() {
        let mut player = SamplePlayer::new();
        let sample = create_test_sample();
        player.load_sample(sample);

        player.note_on(MidiNote::C4, 1.0);
        player.note_off();

        // Should stop since loop mode is Off
        assert!(!player.playing);
    }
}
