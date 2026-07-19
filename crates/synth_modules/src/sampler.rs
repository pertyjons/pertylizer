//! Sampler module — plays samples as instruments in the patch editor.
//!
//! This is a voice module (`PolyModule`) that can be placed in the Rack view,
//! just like an oscillator. On `NoteOn`, it starts playback of the assigned
//! sample at the appropriate pitch. On `NoteOff`, it releases.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::ChannelCount;
use synth_core::VoicePitch;
use synth_core::{
    AudioBuffer, Cents, Describable, Gain, InputPorts, MidiNote, ModuleCategory, ModuleDescriptor,
    ModuleType, NormalizedValue, Param, ParamModOffsets, ParameterDescriptor, ParameterUnit,
    PlayDirection, PolyModule, PortDescriptor, PortName, ProcessContext, SampleId, SampleRate,
    SamplerParam, SamplerPlayMode, Velocity, WidgetHint,
};
use synth_sampler::playback::{PlaybackState, SamplePlayer};
use synth_sampler::types::{CropRegion, LoopRegion};

/// Pitch-CV input range in octaves (Eurorack 1V/oct: `1.0` = +1 octave). The
/// raw CV is clamped to ±this before being applied, to keep playback speed sane
/// and the audio thread safe against runaway/denormal speeds.
const PITCH_CV_RANGE_OCTAVES: f32 = 6.0;

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
    start_offset: NormalizedValue,

    // Sample data (set via LoadSample command or SampleSelect param)
    sample_data: Option<Arc<[f32]>>,
    sample_channels: ChannelCount,
    sample_frame_count: usize,
    sample_crop: Option<CropRegion>,
    sample_loop: Option<LoopRegion>,
    root_note: MidiNote,

    // Whether a sample reload is pending (set by SampleSelect param change)
    needs_sample_reload: bool,

    // Playback state
    player: Option<SamplePlayer>,
    sample_rate: SampleRate,
    /// Base playback speed for the held note (note pitch × fine-tune). Set at
    /// note-on and refreshed every block by `set_voice_pitch`, so the sampler
    /// follows continuous voice pitch (glide / vibrato / pitch-bend) instead of
    /// the pitch latched at trigger. The per-block `pitch_cv` input is
    /// multiplied on top of this in `process`.
    base_speed: f64,
    /// Effective fine-tune (cents) sampled at note-on; folded into `base_speed`.
    active_fine_tune_cents: f64,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    /// `level` is resolved per block; `fine_tune` / `start_offset` /
    /// `velocity_sensitivity` are sampled at note-on (they configure the player).
    mod_offsets: ParamModOffsets,

    // Pre-allocated output buffer (stereo interleaved for player)
    render_buffer: Vec<f32>,
    output_buffer: AudioBuffer,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sample_id: SampleId::new(0),
            pitch_tracking: true,
            level: Gain::new(0.8),
            play_mode: SamplerPlayMode::Sustain,
            direction: PlayDirection::Forward,
            velocity_sensitivity: NormalizedValue::new(1.0),
            fine_tune: Cents::ZERO,
            start_offset: NormalizedValue::new(0.0),

            sample_data: None,
            sample_channels: ChannelCount::Stereo,
            sample_frame_count: 0,
            sample_crop: None,
            sample_loop: None,
            root_note: MidiNote::new(60), // C4

            needs_sample_reload: false,

            player: None,
            sample_rate: SampleRate::DVD_QUALITY,
            base_speed: 1.0,
            active_fine_tune_cents: 0.0,
            mod_offsets: ParamModOffsets::new(),

            // Pre-allocate for up to 8192 stereo frames. Grow-only: if a larger
            // block is encountered the Vec will resize once and stay at that size.
            render_buffer: vec![0.0; 16384],
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
        self.needs_sample_reload = false;
    }

    /// Clear sample data.
    pub fn unload_sample(&mut self) {
        self.sample_data = None;
        self.player = None;
        self.sample_frame_count = 0;
    }

    /// Check if a sample reload is pending (set when SampleSelect changes).
    pub fn needs_sample_reload(&self) -> bool {
        self.needs_sample_reload
    }

    /// Get the currently selected sample ID.
    pub fn sample_id(&self) -> SampleId {
        self.sample_id
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
            .width(synth_core::ModuleWidth::Large)
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
                // Boolean toggle, not a continuous modulation target.
                .modulatable(false)
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
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "start_offset",
                    Param::Sampler(SamplerParam::StartOffset(NormalizedValue::new(0.0))),
                    "Start",
                )
                .description("Playback start offset (0.0 = beginning, 1.0 = end)")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            // SampleSelect, PlayMode, Direction must be in the descriptor so
            // project save serializes them — the patch loop iterates
            // descriptor.parameters and silently drops anything missing here.
            // SampleSelect uses Hidden because the patch editor renders its
            // own sample-picker combo box (see patch_editor.rs).
            .parameter(
                ParameterDescriptor::float("sample_select", Param::sample_select(0), "Sample")
                    .description("Assigned sample (selected via the sample picker)")
                    // Discrete sample selector, not a continuous modulation target.
                    .modulatable(false)
                    .widget(WidgetHint::Hidden),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "play_mode",
                    Param::Sampler(SamplerParam::PlayMode(SamplerPlayMode::Sustain)),
                    "Play Mode",
                    vec![
                        synth_core::module_traits::ChoiceOption::new("one_shot", "One-shot")
                            .with_description("Plays the full sample, ignores note-off"),
                        synth_core::module_traits::ChoiceOption::new("sustain", "Sustain")
                            .with_description("Plays once, releases on note-off"),
                        synth_core::module_traits::ChoiceOption::new("loop", "Loop")
                            .with_description("Loops the loop region while held"),
                    ],
                )
                .description("How the sample responds to note-on/off"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "direction",
                    Param::Sampler(SamplerParam::Direction(PlayDirection::Forward)),
                    "Direction",
                    vec![
                        synth_core::module_traits::ChoiceOption::new("forward", "Forward")
                            .with_description("Plays the sample from start to end."),
                        synth_core::module_traits::ChoiceOption::new("reverse", "Reverse")
                            .with_description("Plays the sample backwards."),
                        synth_core::module_traits::ChoiceOption::new("ping_pong", "Ping-Pong")
                            .with_description("Alternates forward and backward each pass."),
                    ],
                )
                .description("Playback direction"),
            )
            .port(
                PortDescriptor::control_input("pitch_cv", "Pitch CV").description(
                    "Modulates playback pitch (v/oct: +1.0 = +1 octave), on top of the note \
                     pitch. Connect: LFO for vibrato, Envelope for pitch sweep, Mod Matrix.",
                ),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Sample audio output"))
    }
}

impl PolyModule for Sampler {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext<'_>,
    ) {
        let n_samples = context.samples.as_usize();
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(n_samples);

        // Ensure render buffer is large enough (stereo interleaved).
        // Only grows, never shrinks — avoids allocation on the audio thread
        // when block sizes fluctuate below the high-water mark.
        let render_len = n_samples * 2;
        if self.render_buffer.len() < render_len {
            self.render_buffer.resize(render_len, 0.0);
        }

        // Per-block pitch-CV (control rate): a v/oct modulation multiplied on top
        // of the note pitch (`base_speed`, maintained by `set_voice_pitch`). An
        // unconnected port reads 0.0 → factor 1.0, so this is a no-op unless a
        // mod source (LFO / envelope / mod matrix) is patched in.
        let pitch_cv = inputs.reader(PortName::PITCH_CV, 0.0);
        let cv_octaves = (if n_samples > 0 { pitch_cv.get(0) } else { 0.0 })
            .clamp(-PITCH_CV_RANGE_OCTAVES, PITCH_CV_RANGE_OCTAVES);
        let target_speed = self.base_speed * 2.0_f64.powf(f64::from(cv_octaves));

        if let Some(ref mut player) = self.player {
            // Drive continuous pitch: note pitch × pitch-CV, applied each block.
            player.set_speed(target_speed);

            // Clear render buffer
            self.render_buffer[..render_len].fill(0.0);

            // Render sample audio (stereo interleaved)
            let still_active = player.render(&mut self.render_buffer[..render_len], n_samples);

            // Mix stereo to mono for the output port (L+R)/2
            let level = self.mod_offsets.effective("level", self.level.as_f32());
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
                SamplerParam::SampleSelect(id) => {
                    if self.sample_id != id {
                        self.sample_id = id;
                        self.needs_sample_reload = true;
                    }
                }
                SamplerParam::PitchTracking(b) => self.pitch_tracking = b,
                SamplerParam::Level(g) => self.level = Gain::new(g.as_f32().clamp(0.0, 1.0)),
                SamplerParam::PlayMode(m) => self.play_mode = m,
                SamplerParam::Direction(d) => self.direction = d,
                SamplerParam::VelocitySensitivity(v) => self.velocity_sensitivity = v,
                SamplerParam::FineTune(c) => self.fine_tune = c,
                SamplerParam::StartOffset(v) => self.start_offset = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Sampler(sp) = param {
            match sp {
                // SampleSelect is not slider-compatible; use get_params() for full typed access
                SamplerParam::SampleSelect(_) => None,
                SamplerParam::PitchTracking(_) => Some(if self.pitch_tracking { 1.0 } else { 0.0 }),
                SamplerParam::Level(_) => Some(self.level.as_f32()),
                SamplerParam::PlayMode(_) => Some(self.play_mode as u8 as f32),
                SamplerParam::Direction(_) => Some(self.direction as u8 as f32),
                SamplerParam::VelocitySensitivity(_) => Some(self.velocity_sensitivity.as_f32()),
                SamplerParam::FineTune(_) => Some(self.fine_tune.as_f32()),
                SamplerParam::StartOffset(_) => Some(self.start_offset.as_f32()),
            }
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
            Param::Sampler(SamplerParam::StartOffset(self.start_offset)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Sampler
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.player = None;
    }

    fn load_sample_data(
        &mut self,
        data: std::sync::Arc<[f32]>,
        channels: ChannelCount,
        frame_count: usize,
        root_note: MidiNote,
    ) {
        self.load_sample(data, channels, frame_count, None, None, root_note);
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        // Follow the voice's modulated note pitch continuously (glide / vibrato /
        // pitch-bend). When pitch tracking is off the sampler ignores the played
        // note (fixed-rate playback), so leave `base_speed` at its note-on,
        // fine-tune-only value. `process` multiplies `pitch_cv` on top of this.
        if !self.pitch_tracking {
            return;
        }
        let root_freq = self.root_note.to_frequency().as_f32();
        let played = pitch.played.as_f32();
        if root_freq > 0.0 && played > 0.0 {
            // speed = freq / root_freq × fine-tune. At freq == note pitch this
            // equals the note-on `base_speed`; it scales with the modulation.
            let fine_factor = 2.0_f64.powf(self.active_fine_tune_cents / 1200.0);
            self.base_speed = (f64::from(played) / f64::from(root_freq)) * fine_factor;
        }
    }

    fn note_on(&mut self, note: MidiNote, velocity: Velocity) {
        let Some(ref data) = self.sample_data else {
            return;
        };

        // Note-on-time params: sampled through the generic mod store at trigger,
        // so a routing active at note-on configures this note's player.
        let fine_tune = self
            .mod_offsets
            .effective("fine_tune", self.fine_tune.as_f32());
        let start_offset = self
            .mod_offsets
            .effective("start_offset", self.start_offset.as_f32());
        let vel_amount = self
            .mod_offsets
            .effective("velocity_sensitivity", self.velocity_sensitivity.as_f32());

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

        // Pitch: seed this note's base playback speed (note pitch × fine-tune).
        // `set_voice_pitch` then refreshes `base_speed` every block while the
        // note sounds (so glide / vibrato / pitch-bend track continuously) and
        // `process` multiplies the `pitch_cv` input on top. Store the effective
        // fine-tune for that per-block refresh.
        self.active_fine_tune_cents = f64::from(fine_tune);
        if self.pitch_tracking {
            // Seed through the exact same formula the per-block refresh uses, so
            // the note-on speed and the first `set_voice_pitch` agree.
            self.set_voice_pitch(VoicePitch::tracking(note.to_frequency()));
        } else {
            // Fixed-rate playback: fine-tune only (the played note is ignored).
            self.base_speed = 2.0_f64.powf(self.active_fine_tune_cents / 1200.0);
        }
        player.set_speed(self.base_speed);

        // Set velocity
        let vel_gain = 1.0 - vel_amount + vel_amount * velocity.as_f32();
        player.set_velocity(Velocity::new(vel_gain));

        // Set looping
        player.set_looping(self.play_mode == SamplerPlayMode::Loop);

        // Apply direction
        match self.direction {
            PlayDirection::Reverse => player.set_reverse(),
            PlayDirection::PingPong => player.set_ping_pong(),
            PlayDirection::Forward => {} // default
        }

        // Apply start offset
        if start_offset > 0.001 {
            player.set_start_offset(f64::from(start_offset));
        }

        self.player = Some(player);
    }

    fn note_off(&mut self) {
        if self.play_mode == SamplerPlayMode::OneShot {
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

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::Hertz;

    fn ctx(n: usize) -> ProcessContext<'static> {
        ProcessContext {
            samples: synth_core::SampleCount::new(n),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        }
    }

    /// A sampler with the mod store populated and a constant-0.5 stereo sample
    /// loaded at root pitch, so playback is a steady tone.
    fn loaded_sampler() -> Sampler {
        let mut s = Sampler::new();
        let desc = s.descriptor();
        s.mod_offsets_mut().unwrap().populate(&desc);
        let data: Arc<[f32]> = vec![0.5_f32; 4096 * 2].into();
        s.load_sample(
            data,
            ChannelCount::Stereo,
            4096,
            None,
            None,
            MidiNote::new(60),
        );
        s
    }

    fn render_energy(s: &mut Sampler, n: usize) -> f32 {
        let mut out = HashMap::new();
        out.insert(PortName::OUT, AudioBuffer::new(n));
        s.process(InputPorts::empty(), &mut out, &ctx(n));
        (0..n).map(|i| out[&PortName::OUT][i].abs()).sum::<f32>()
    }

    /// `level` used to be dropped (Sampler never overrode set_mod_offset); it now
    /// scales the per-block output through the generic store. Driving it to 0
    /// silences playback; clearing reverts.
    #[test]
    fn test_sampler_level_mod_offset_scales_output() {
        let mut s = loaded_sampler();
        s.note_on(MidiNote::new(60), Velocity::new(1.0));

        // Warm up past any player fade-in so the steady-state level is reached.
        let _ = render_energy(&mut s, 256);
        let base = render_energy(&mut s, 64);
        assert!(base > 0.01, "baseline should produce sound, base={base}");

        s.set_mod_offset("level", -1.0); // level → 0 = silence
        let quiet = render_energy(&mut s, 64);
        assert!(
            quiet < base * 0.01,
            "level offset should silence output: base={base}, quiet={quiet}"
        );

        s.clear_mod_offsets();
        let restored = render_energy(&mut s, 64);
        assert!(
            (restored - base).abs() < base * 0.1,
            "clearing reverts level: base={base}, restored={restored}"
        );
    }

    /// Render `pitch_cv` (octaves) connected, counting frames until the
    /// non-looping sample is exhausted. Higher playback speed → exhausts sooner.
    fn frames_until_silent_with_cv(cv_octaves: f32) -> usize {
        let mut s = loaded_sampler();
        s.note_on(MidiNote::new(60), Velocity::new(1.0)); // C4 == root → base_speed 1
        let mut cv = AudioBuffer::new(64);
        for i in 0..64 {
            cv[i] = cv_octaves;
        }
        let mut total = 0;
        for _ in 0..400 {
            let mut out = HashMap::new();
            out.insert(PortName::OUT, AudioBuffer::new(64));
            let ports = [(PortName::PITCH_CV, &cv)];
            s.process(InputPorts::new(&ports), &mut out, &ctx(64));
            total += 64;
            let e: f32 = (0..64).map(|i| out[&PortName::OUT][i].abs()).sum();
            if e < 1e-6 {
                break;
            }
        }
        total
    }

    /// A — `set_voice_pitch` makes the sampler follow continuous voice pitch:
    /// doubling the note frequency each block doubles playback speed, so a
    /// non-looping sample is consumed ~twice as fast.
    #[test]
    fn voice_pitch_modulates_sampler_playback_speed() {
        fn frames_until_silent(pitch_mul: f32) -> usize {
            let mut s = loaded_sampler();
            s.note_on(MidiNote::new(60), Velocity::new(1.0)); // C4 == root
            let root = MidiNote::new(60).to_frequency().as_f32();
            let mut total = 0;
            for _ in 0..400 {
                s.set_voice_pitch(VoicePitch::tracking(Hertz::new(root * pitch_mul)));
                let e = render_energy(&mut s, 64);
                total += 64;
                if e < 1e-6 {
                    break;
                }
            }
            total
        }
        let normal = frames_until_silent(1.0);
        let fast = frames_until_silent(2.0);
        assert!(
            fast < normal,
            "2x voice pitch should exhaust the sample sooner: normal={normal}, fast={fast}"
        );
        assert!(
            (fast as f32) <= (normal as f32) * 0.75,
            "2x pitch should roughly halve the duration: normal={normal}, fast={fast}"
        );
    }

    /// B — the `pitch_cv` input port modulates playback pitch on top of the note
    /// pitch: +1 octave of CV doubles the speed; an unconnected/0 CV is a no-op.
    #[test]
    fn pitch_cv_input_modulates_sampler_playback_speed() {
        let normal = frames_until_silent_with_cv(0.0);
        let up = frames_until_silent_with_cv(1.0); // +1 octave → 2x speed
        assert!(
            up < normal,
            "+1 octave pitch CV should exhaust the sample sooner: normal={normal}, up={up}"
        );
        assert!(
            (up as f32) <= (normal as f32) * 0.75,
            "+1 octave CV should roughly halve the duration: normal={normal}, up={up}"
        );
    }
}
