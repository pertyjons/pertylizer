//! Sampler module — plays samples as instruments in the patch editor.
//!
//! This is a voice module (`PolyModule`) that can be placed in the Rack view,
//! just like an oscillator. On `NoteOn`, it starts playback of the assigned
//! sample at the appropriate pitch. On `NoteOff`, it releases.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::ChannelCount;
use synth_core::{
    AudioBuffer, Cents, Describable, Gain, InputPorts, MidiNote, ModuleCategory, ModuleDescriptor,
    ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PlayDirection,
    PolyModule, PortDescriptor, PortName, ProcessContext, SampleId, SampleRate, SamplerParam,
    SamplerPlayMode, Velocity, WidgetHint,
};
use synth_sampler::playback::{PlaybackState, SamplePlayer};
use synth_sampler::types::{CropRegion, LoopRegion};

/// Sampler module — plays samples as voice sources.
#[derive(Clone)]
pub struct Sampler {
    // Parameters
    sample_id: SampleId,
    pitch_tracking: bool,
    level: Gain,
    play_mode: SamplerPlayMode,
    direction: PlayDirection,
    velocity_sensitivity: NormalizedValue,
    fine_tune: Cents,

    // Sample data (set via LoadSample command)
    sample_data: Option<Arc<[f32]>>,
    sample_channels: ChannelCount,
    sample_frame_count: usize,
    sample_crop: Option<CropRegion>,
    sample_loop: Option<LoopRegion>,
    root_note: MidiNote,

    // Playback state
    player: Option<SamplePlayer>,
    sample_rate: SampleRate,

    // Pre-allocated output buffer (stereo interleaved for player)
    render_buffer: Vec<f32>,
    output_buffer: AudioBuffer,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sample_id: SampleId(0),
            pitch_tracking: true,
            level: Gain::new(0.8),
            play_mode: SamplerPlayMode::Sustain,
            direction: PlayDirection::Forward,
            velocity_sensitivity: NormalizedValue::new(1.0),
            fine_tune: Cents::ZERO,

            sample_data: None,
            sample_channels: ChannelCount::Stereo,
            sample_frame_count: 0,
            sample_crop: None,
            sample_loop: None,
            root_note: MidiNote(60), // C4

            player: None,
            sample_rate: SampleRate::DVD_QUALITY,

            render_buffer: vec![0.0; 2048],
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Load sample data for playback. Called from the engine when a sample is assigned.
    pub fn load_sample(
        &mut self,
        data: Arc<[f32]>,
        channels: ChannelCount,
        frame_count: usize,
        crop: Option<CropRegion>,
        loop_region: Option<LoopRegion>,
        root_note: MidiNote,
    ) {
        self.sample_data = Some(data);
        self.sample_channels = channels;
        self.sample_frame_count = frame_count;
        self.sample_crop = crop;
        self.sample_loop = loop_region;
        self.root_note = root_note;
    }

    /// Clear sample data.
    pub fn unload_sample(&mut self) {
        self.sample_data = None;
        self.player = None;
        self.sample_frame_count = 0;
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Sampler {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("sampler", "Sampler")
            .description("Sample playback module — plays WAV samples as instruments")
            .category(ModuleCategory::Sampler)
            .tag("sampler")
            .tag("source")
            .tag("sample")
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Sampler(SamplerParam::Level(Gain::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pitch_tracking",
                    Param::Sampler(SamplerParam::PitchTracking(true)),
                    "Pitch Track",
                )
                .description("Follow MIDI note pitch")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "velocity_sensitivity",
                    Param::Sampler(SamplerParam::VelocitySensitivity(NormalizedValue::new(1.0))),
                    "Vel Sens",
                )
                .description("Velocity sensitivity")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "fine_tune",
                    Param::Sampler(SamplerParam::FineTune(Cents::ZERO)),
                    "Fine Tune",
                )
                .description("Fine-tune in cents")
                .range(-100.0, 100.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Sample audio output"))
    }
}

impl PolyModule for Sampler {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        let n_samples = context.samples.as_usize();
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(n_samples);

        // Ensure render buffer is large enough (stereo interleaved)
        let render_len = n_samples * 2;
        if self.render_buffer.len() < render_len {
            self.render_buffer.resize(render_len, 0.0);
        }

        if let Some(ref mut player) = self.player {
            // Clear render buffer
            self.render_buffer[..render_len].fill(0.0);

            // Render sample audio (stereo interleaved)
            let still_active = player.render(&mut self.render_buffer[..render_len], n_samples);

            // Mix stereo to mono for the output port (L+R)/2
            let level = self.level.as_f32();
            for i in 0..n_samples {
                let left = self.render_buffer[i * 2];
                let right = self.render_buffer[i * 2 + 1];
                self.output_buffer[i] = (left + right) * 0.5 * level;
            }

            if !still_active {
                self.player = None;
            }
        } else {
            // No active player — silence
            for i in 0..n_samples {
                self.output_buffer[i] = 0.0;
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Sampler(sp) = param {
            match sp {
                SamplerParam::SampleSelect(id) => self.sample_id = id,
                SamplerParam::PitchTracking(b) => self.pitch_tracking = b,
                SamplerParam::Level(g) => self.level = Gain::new(g.as_f32().clamp(0.0, 1.0)),
                SamplerParam::PlayMode(m) => self.play_mode = m,
                SamplerParam::Direction(d) => self.direction = d,
                SamplerParam::VelocitySensitivity(v) => self.velocity_sensitivity = v,
                SamplerParam::FineTune(c) => self.fine_tune = c,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Sampler(sp) = param {
            Some(match sp {
                SamplerParam::SampleSelect(_) => self.sample_id.0 as f32,
                SamplerParam::PitchTracking(_) => {
                    if self.pitch_tracking {
                        1.0
                    } else {
                        0.0
                    }
                }
                SamplerParam::Level(_) => self.level.as_f32(),
                SamplerParam::PlayMode(_) => self.play_mode as u8 as f32,
                SamplerParam::Direction(_) => self.direction as u8 as f32,
                SamplerParam::VelocitySensitivity(_) => self.velocity_sensitivity.as_f32(),
                SamplerParam::FineTune(_) => self.fine_tune.0,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Sampler(SamplerParam::SampleSelect(self.sample_id)),
            Param::Sampler(SamplerParam::PitchTracking(self.pitch_tracking)),
            Param::Sampler(SamplerParam::Level(self.level)),
            Param::Sampler(SamplerParam::PlayMode(self.play_mode)),
            Param::Sampler(SamplerParam::Direction(self.direction)),
            Param::Sampler(SamplerParam::VelocitySensitivity(self.velocity_sensitivity)),
            Param::Sampler(SamplerParam::FineTune(self.fine_tune)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Sampler
    }

    fn reset(&mut self) {
        self.player = None;
    }

    fn note_on(&mut self, note: MidiNote, velocity: Velocity) {
        let Some(ref data) = self.sample_data else {
            return;
        };

        let loop_region = if self.play_mode == SamplerPlayMode::Loop {
            self.sample_loop
        } else {
            None
        };

        let mut player = SamplePlayer::new(
            Arc::clone(data),
            self.sample_channels,
            self.sample_frame_count,
            self.sample_crop,
            loop_region,
        );

        // Set pitch
        if self.pitch_tracking {
            player.set_pitch(note, self.root_note);
        }

        // Set velocity
        let vel_amount = self.velocity_sensitivity.as_f32();
        let vel_gain = 1.0 - vel_amount + vel_amount * velocity.0;
        player.set_velocity(vel_gain);

        // Set looping
        player.set_looping(self.play_mode == SamplerPlayMode::Loop);

        self.player = Some(player);
    }

    fn note_off(&mut self) {
        if self.play_mode == SamplerPlayMode::OneShot {
            // OneShot ignores note-off
            return;
        }
        if let Some(ref mut player) = self.player {
            player.note_off();
        }
    }

    fn is_release_done(&self) -> bool {
        match &self.player {
            Some(player) => player.state() == PlaybackState::Finished,
            None => true,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
