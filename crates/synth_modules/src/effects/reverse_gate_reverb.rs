//! Reverse / gated reverb (buffer stutter) effect.
//!
//! Features:
//! - Stereo capture buffer with periodic or threshold triggering
//! - Three playback modes: Reverse, Gate, Stutter
//! - Envelope shaping with configurable gate time
//! - Mix with dry signal

use synth_core::{
    AudioEffect, Decibels, Describable, Milliseconds, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ProcessContext, ReverseGateMode,
    ReverseGateReverbParam, ReverseGateTrigger, SampleRate, StereoSample, WidgetHint,
};

/// Maximum window time in milliseconds.
const MAX_WINDOW_MS: f32 = 2000.0;

/// Reverse/gate reverb effect.
pub struct ReverseGateReverb {
    // Parameters
    window_time: Milliseconds,
    mode: ReverseGateMode,
    trigger: ReverseGateTrigger,
    threshold: Decibels,
    gate_time: Milliseconds,
    mix: NormalizedValue,

    // Capture buffers (stereo)
    capture_l: Vec<f32>,
    capture_r: Vec<f32>,
    capture_pos: usize,
    window_samples: usize,

    // Playback state
    playback_l: Vec<f32>,
    playback_r: Vec<f32>,
    play_pos: usize,
    playing: bool,
    play_len: usize,

    // RMS for threshold detection
    rms_acc: f32,
    rms_count: usize,

    // Periodic trigger counter
    periodic_counter: usize,

    sample_rate: SampleRate,
}

impl ReverseGateReverb {
    pub fn new() -> Self {
        let max_samples = (MAX_WINDOW_MS / 1000.0 * 48000.0) as usize;
        Self {
            window_time: Milliseconds::new(600.0),
            mode: ReverseGateMode::Reverse,
            trigger: ReverseGateTrigger::Periodic,
            threshold: Decibels::new(-24.0),
            gate_time: Milliseconds::new(120.0),
            mix: NormalizedValue::MAX,

            capture_l: vec![0.0; max_samples],
            capture_r: vec![0.0; max_samples],
            capture_pos: 0,
            window_samples: (600.0 / 1000.0 * 48000.0) as usize,

            playback_l: vec![0.0; max_samples],
            playback_r: vec![0.0; max_samples],
            play_pos: 0,
            playing: false,
            play_len: 0,

            rms_acc: 0.0,
            rms_count: 0,

            periodic_counter: 0,

            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn update_window_samples(&mut self) {
        let samples = self.window_time.to_samples(self.sample_rate);
        self.window_samples = samples.clamp(1, self.capture_l.len());
    }

    fn trigger_playback(&mut self) {
        // Copy capture buffer to playback buffer based on mode
        let len = self.window_samples;

        match self.mode {
            ReverseGateMode::Reverse => {
                // Copy in reverse order
                for i in 0..len {
                    let src = (self.capture_pos + len - 1 - i) % len;
                    self.playback_l[i] = self.capture_l[src];
                    self.playback_r[i] = self.capture_r[src];
                }
            }
            ReverseGateMode::Gate | ReverseGateMode::Stutter => {
                // Copy forward
                for i in 0..len {
                    let src = (self.capture_pos + i) % len;
                    self.playback_l[i] = self.capture_l[src];
                    self.playback_r[i] = self.capture_r[src];
                }
            }
        }

        self.play_pos = 0;
        self.play_len = len;
        self.playing = true;
    }
}

impl Default for ReverseGateReverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ReverseGateReverb {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("reverse_gate_reverb", "Reverse/Gate Reverb")
            .description("Buffer capture with reverse/gate/stutter playback")
            .category(ModuleCategory::Effect)
            .tag("reverse")
            .tag("gate")
            .tag("effect")
            .parameter(
                ParameterDescriptor::float(
                    "window",
                    Param::ReverseGateReverb(ReverseGateReverbParam::WindowTime(
                        Milliseconds::new(600.0),
                    )),
                    "Window",
                )
                .description("Capture window length")
                .range(100.0, 2000.0)
                .default(600.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "mode",
                    Param::ReverseGateReverb(ReverseGateReverbParam::Mode(
                        ReverseGateMode::Reverse,
                    )),
                    "Mode",
                    ReverseGateMode::to_choices(),
                )
                .description("Playback mode"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "trigger",
                    Param::ReverseGateReverb(ReverseGateReverbParam::Trigger(
                        ReverseGateTrigger::Periodic,
                    )),
                    "Trigger",
                    ReverseGateTrigger::to_choices(),
                )
                .description("Trigger mode"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "threshold",
                    Param::ReverseGateReverb(ReverseGateReverbParam::Threshold(Decibels::new(
                        -24.0,
                    ))),
                    "Threshold",
                )
                .description("Threshold for trigger detection")
                .range(-60.0, 0.0)
                .default(-24.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "gate_time",
                    Param::ReverseGateReverb(ReverseGateReverbParam::GateTime(Milliseconds::new(
                        120.0,
                    ))),
                    "Gate Time",
                )
                .description("Envelope gate/decay time")
                .range(20.0, 500.0)
                .default(120.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::ReverseGateReverb(ReverseGateReverbParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for ReverseGateReverb {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        let mix = self.mix.as_f32();
        let threshold_linear = self.threshold.to_linear();
        let gate_samples = self.gate_time.to_samples(self.sample_rate).max(1);

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let dry_l = dry.left;
            let dry_r = dry.right;

            // Write to capture buffer
            let cap_idx = self.capture_pos % self.window_samples;
            self.capture_l[cap_idx] = dry_l;
            self.capture_r[cap_idx] = dry_r;
            self.capture_pos = (self.capture_pos + 1) % self.window_samples;

            // RMS accumulation for threshold mode
            let mono = (dry_l + dry_r) * 0.5;
            self.rms_acc += mono * mono;
            self.rms_count += 1;

            // Check triggers
            let should_trigger = match self.trigger {
                ReverseGateTrigger::Periodic => {
                    self.periodic_counter += 1;
                    if self.periodic_counter >= self.window_samples {
                        self.periodic_counter = 0;
                        true
                    } else {
                        false
                    }
                }
                ReverseGateTrigger::Threshold => {
                    if self.rms_count >= 64 {
                        let rms = (self.rms_acc / self.rms_count as f32).sqrt();
                        self.rms_acc = 0.0;
                        self.rms_count = 0;
                        !self.playing && rms > threshold_linear
                    } else {
                        false
                    }
                }
            };

            if should_trigger {
                self.trigger_playback();
            }

            // Generate playback output
            let (wet_l, wet_r) = if self.playing && self.play_pos < self.play_len {
                let l = self.playback_l[self.play_pos];
                let r = self.playback_r[self.play_pos];

                // Envelope
                let env = match self.mode {
                    ReverseGateMode::Reverse => {
                        // Fade in at start of reversed playback
                        let t = self.play_pos as f32 / gate_samples as f32;
                        t.min(1.0)
                    }
                    ReverseGateMode::Gate => {
                        // Exponential decay
                        (-(self.play_pos as f32) / gate_samples as f32).exp()
                    }
                    ReverseGateMode::Stutter => {
                        // No envelope for stutter, just loop
                        1.0
                    }
                };

                self.play_pos += 1;

                // Stutter mode: loop
                if self.mode == ReverseGateMode::Stutter && self.play_pos >= self.play_len {
                    self.play_pos = 0;
                }

                // End playback for non-stutter modes
                if self.play_pos >= self.play_len && self.mode != ReverseGateMode::Stutter {
                    self.playing = false;
                }

                (l * env, r * env)
            } else {
                (0.0, 0.0)
            };

            let result =
                StereoSample::new(dry_l, dry_r).blend(StereoSample::new(wet_l, wet_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.capture_l.fill(0.0);
        self.capture_r.fill(0.0);
        self.capture_pos = 0;
        self.playback_l.fill(0.0);
        self.playback_r.fill(0.0);
        self.play_pos = 0;
        self.playing = false;
        self.periodic_counter = 0;
        self.rms_acc = 0.0;
        self.rms_count = 0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }
    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ReverseGateReverb(p) = param {
            match p {
                ReverseGateReverbParam::WindowTime(v) => {
                    self.window_time = Milliseconds::new(v.as_f32().clamp(100.0, MAX_WINDOW_MS));
                    self.update_window_samples();
                }
                ReverseGateReverbParam::Mode(v) => self.mode = v,
                ReverseGateReverbParam::Trigger(v) => self.trigger = v,
                ReverseGateReverbParam::Threshold(v) => {
                    self.threshold = Decibels::new(v.as_f32().clamp(-60.0, 0.0));
                }
                ReverseGateReverbParam::GateTime(v) => {
                    self.gate_time = Milliseconds::new(v.as_f32().clamp(20.0, 500.0));
                }
                ReverseGateReverbParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ReverseGateReverb(p) = param {
            Some(match p {
                ReverseGateReverbParam::WindowTime(_) => self.window_time.as_f32(),
                ReverseGateReverbParam::Mode(_) => self.mode.index() as f32,
                ReverseGateReverbParam::Trigger(_) => self.trigger.index() as f32,
                ReverseGateReverbParam::Threshold(_) => self.threshold.as_f32(),
                ReverseGateReverbParam::GateTime(_) => self.gate_time.as_f32(),
                ReverseGateReverbParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::ReverseGateReverb(ReverseGateReverbParam::WindowTime(self.window_time)),
            Param::ReverseGateReverb(ReverseGateReverbParam::Mode(self.mode)),
            Param::ReverseGateReverb(ReverseGateReverbParam::Trigger(self.trigger)),
            Param::ReverseGateReverb(ReverseGateReverbParam::Threshold(self.threshold)),
            Param::ReverseGateReverb(ReverseGateReverbParam::GateTime(self.gate_time)),
            Param::ReverseGateReverb(ReverseGateReverbParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ReverseGateReverb
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        let max_samples = (MAX_WINDOW_MS / 1000.0 * sample_rate.as_f32()) as usize;
        if self.capture_l.len() != max_samples {
            self.capture_l.resize(max_samples, 0.0);
            self.capture_r.resize(max_samples, 0.0);
            self.playback_l.resize(max_samples, 0.0);
            self.playback_r.resize(max_samples, 0.0);
        }
        self.update_window_samples();
    }
}
