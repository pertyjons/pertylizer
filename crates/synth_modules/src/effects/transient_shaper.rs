//! Transient shaper — a differential-envelope transient designer.
//!
//! Two envelope followers track the input level at different speeds: a *fast*
//! one that snaps to onsets and a *slow* one that trails the body. Their
//! normalized difference is ~1 during an attack transient and ~0 during the
//! sustain/decay, so the Attack and Sustain gains can be applied independently
//! to the onset and the body without touching the underlying amplitude (or any
//! amplitude automation already drawn on the track).

use synth_core::SampleRate;
use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    PortDescriptor, ProcessContext, StereoSample, WidgetHint,
};
use synth_core::{
    Decibels, Milliseconds, ModuleType, NormalizedValue, Param, TransientShaperParam,
};

/// Minimum detection-window length in ms (fast enough to catch a kick attack).
const WINDOW_MIN_MS: f32 = 1.0;
/// Maximum detection-window length in ms.
const WINDOW_MAX_MS: f32 = 50.0;
/// Attack/Sustain gain range in dB (symmetric boost/cut).
const GAIN_DB_LIMIT: f32 = 24.0;
/// Fixed fast-follower attack — near-instant so onsets are caught immediately.
const FAST_ATTACK_MS: f32 = 0.5;

pub struct TransientShaper {
    // Parameters
    attack: Decibels,
    sustain: Decibels,
    sensitivity: NormalizedValue,
    window: Milliseconds,
    mix: NormalizedValue,

    // Envelope state
    fast_env: f32,
    slow_env: f32,

    sample_rate: SampleRate,
}

impl TransientShaper {
    #[must_use]
    pub fn new() -> Self {
        Self {
            attack: Decibels::new(0.0),
            sustain: Decibels::new(0.0),
            sensitivity: NormalizedValue::MAX,
            window: Milliseconds::new(10.0),
            mix: NormalizedValue::MAX,
            fast_env: 0.0,
            slow_env: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// One-pole follower coefficient for a time constant in seconds.
    #[inline]
    fn coeff(&self, secs: f32) -> f32 {
        crate::math::envelope_coeff(secs, self.sample_rate.as_f32())
    }
}

impl Default for TransientShaper {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for TransientShaper {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("transient_shaper", "Transient Shaper")
            .width(synth_core::ModuleWidth::Medium)
            .description(
                "Differential-envelope transient designer: independent attack/sustain gain shaping",
            )
            .category(ModuleCategory::Effect)
            .tag("transient")
            .tag("effect")
            .tag("dynamics")
            .tag("drums")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .parameter(
                ParameterDescriptor::float(
                    "attack",
                    Param::TransientShaper(TransientShaperParam::Attack(Decibels::new(0.0))),
                    "Attack",
                )
                .description("Onset gain in dB (+ adds punch, − softens the attack)")
                .range(-GAIN_DB_LIMIT, GAIN_DB_LIMIT)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sustain",
                    Param::TransientShaper(TransientShaperParam::Sustain(Decibels::new(0.0))),
                    "Sustain",
                )
                .description("Body/tail gain in dB (+ lengthens/fills, − tightens/gates)")
                .range(-GAIN_DB_LIMIT, GAIN_DB_LIMIT)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sensitivity",
                    Param::TransientShaper(TransientShaperParam::Sensitivity(NormalizedValue::MAX)),
                    "Sensitivity",
                )
                .description("Overall effect amount (0 = off, 1 = full shaping)")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "window",
                    Param::TransientShaper(TransientShaperParam::Window(Milliseconds::new(10.0))),
                    "Window",
                )
                .description("Transient detection window in ms — how fast onsets are detected")
                .range(WINDOW_MIN_MS, WINDOW_MAX_MS)
                .default(10.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::TransientShaper(TransientShaperParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for TransientShaper {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;

        let window_s = self.window.to_seconds().as_f32();
        // Fast follower: near-instant attack, releases within the window.
        let fast_atk = self.coeff(FAST_ATTACK_MS / 1000.0);
        let fast_rel = self.coeff(window_s);
        // Slow follower: attacks over the window, releases well beyond it so the
        // body/decay keeps `slow_env >= fast_env` (→ sustain shaping).
        let slow_atk = self.coeff(window_s);
        let slow_rel = self.coeff(window_s * 4.0);

        let attack_db = self.attack.as_f32();
        let sustain_db = self.sustain.as_f32();
        let amount = self.sensitivity.as_f32();
        let mix = self.mix.as_f32();

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let x = dry.left.abs().max(dry.right.abs());

            let fc = if x > self.fast_env {
                fast_atk
            } else {
                fast_rel
            };
            self.fast_env = fc * self.fast_env + (1.0 - fc) * x;
            let sc = if x > self.slow_env {
                slow_atk
            } else {
                slow_rel
            };
            self.slow_env = sc * self.slow_env + (1.0 - sc) * x;

            // Transient amount: how far the fast env overshoots the slow one,
            // normalized to 0..1. ~1 at a sharp onset, ~0 through the body/decay.
            let t_norm = if self.fast_env > 1e-6 {
                ((self.fast_env - self.slow_env) / self.fast_env).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let gain_db = amount * (attack_db * t_norm + sustain_db * (1.0 - t_norm));
            let gain = Decibels::new(gain_db).to_linear();

            let wet = StereoSample::new(dry.left * gain, dry.right * gain);
            StereoSample::write_frame(output, frame, dry.blend(wet, mix));
        }
    }

    fn reset(&mut self) {
        self.fast_env = 0.0;
        self.slow_env = 0.0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::TransientShaper(p) = param {
            match p {
                TransientShaperParam::Attack(db) => {
                    self.attack = Decibels::new(db.as_f32().clamp(-GAIN_DB_LIMIT, GAIN_DB_LIMIT));
                }
                TransientShaperParam::Sustain(db) => {
                    self.sustain = Decibels::new(db.as_f32().clamp(-GAIN_DB_LIMIT, GAIN_DB_LIMIT));
                }
                TransientShaperParam::Sensitivity(v) => self.sensitivity = v,
                TransientShaperParam::Window(ms) => {
                    self.window =
                        Milliseconds::new(ms.as_f32().clamp(WINDOW_MIN_MS, WINDOW_MAX_MS));
                }
                TransientShaperParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::TransientShaper(p) = param {
            Some(match p {
                TransientShaperParam::Attack(_) => self.attack.as_f32(),
                TransientShaperParam::Sustain(_) => self.sustain.as_f32(),
                TransientShaperParam::Sensitivity(_) => self.sensitivity.as_f32(),
                TransientShaperParam::Window(_) => self.window.as_f32(),
                TransientShaperParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::TransientShaper(TransientShaperParam::Attack(self.attack)),
            Param::TransientShaper(TransientShaperParam::Sustain(self.sustain)),
            Param::TransientShaper(TransientShaperParam::Sensitivity(self.sensitivity)),
            Param::TransientShaper(TransientShaperParam::Window(self.window)),
            Param::TransientShaper(TransientShaperParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::TransientShaper
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    fn ctx(frames: usize) -> ProcessContext<'static> {
        ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(frames),
            ..Default::default()
        }
    }

    /// A percussive burst: a sharp onset (a few loud frames) then a lower,
    /// decaying tail. Interleaved stereo.
    fn burst(frames: usize) -> Vec<f32> {
        let mut buf = vec![0.0f32; frames * 2];
        for f in 0..frames {
            let env = if f < 4 {
                1.0 // sharp onset
            } else {
                0.3 * (-((f - 4) as f32) / 400.0).exp() // decaying body
            };
            buf[f * 2] = env;
            buf[f * 2 + 1] = env;
        }
        buf
    }

    #[test]
    fn zero_sensitivity_is_transparent() {
        let mut fx = TransientShaper::new();
        fx.set_param(Param::TransientShaper(TransientShaperParam::Sensitivity(
            NormalizedValue::new(0.0),
        )));
        fx.set_param(Param::TransientShaper(TransientShaperParam::Attack(
            Decibels::new(12.0),
        )));
        let input = burst(256);
        let mut output = vec![0.0f32; input.len()];
        fx.process(&input, &mut output, &ctx(256));
        for (i, o) in input.iter().zip(output.iter()) {
            assert!((i - o).abs() < 1e-4, "sensitivity 0 must pass through");
        }
    }

    #[test]
    fn attack_boost_raises_the_onset() {
        let input = burst(256);

        let mut flat = TransientShaper::new();
        flat.set_param(Param::TransientShaper(TransientShaperParam::Sensitivity(
            NormalizedValue::new(0.0),
        )));
        let mut dry = vec![0.0f32; input.len()];
        flat.process(&input, &mut dry, &ctx(256));

        let mut punch = TransientShaper::new();
        punch.set_param(Param::TransientShaper(TransientShaperParam::Attack(
            Decibels::new(12.0),
        )));
        let mut wet = vec![0.0f32; input.len()];
        punch.process(&input, &mut wet, &ctx(256));

        let peak = |b: &[f32]| b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak(&wet) > peak(&dry) * 1.05,
            "attack boost should raise the onset peak: dry={} wet={}",
            peak(&dry),
            peak(&wet)
        );
    }
}
