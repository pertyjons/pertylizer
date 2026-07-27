//! LFO (Low Frequency Oscillator) module.

use std::collections::HashMap;

use synth_core::hash::RtRng;
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, PortValueDomain, ProcessContext,
    ResponseCurve, WidgetHint,
};
use synth_core::{
    BeatDivision, BipolarValue, Hertz, MidiNote, NormalizedValue, Phase, PortName, RetriggerMode,
    SampleRate, SyncMode, Velocity,
};
use synth_core::{LfoParam, LfoWaveform, ModuleType, Param};

/// LFO output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoMode {
    Bipolar,
    Unipolar,
}

/// Low Frequency Oscillator.
#[derive(Clone)]
pub struct Lfo {
    waveform: LfoWaveform,
    rate: Hertz,
    depth: NormalizedValue,
    phase_offset: Phase,
    mode: LfoMode,
    phase: Phase,
    sample_rate: SampleRate,
    sh_value: f32,
    sh_trigger_prev: NormalizedValue,
    /// Smooth random: current and target values for interpolation.
    smooth_random_current: f32,
    smooth_random_target: f32,
    rng: RtRng,
    rng_seed: u64,
    sync_mode: SyncMode,
    sync_division: BeatDivision,
    retrigger_mode: RetriggerMode,
    /// Previous retrigger signal value for edge detection (persists across buffers).
    prev_retrigger: NormalizedValue,
    // Mod matrix offsets
    /// Rate offset (additive Hz, from mod matrix).
    mod_offset_rate: BipolarValue,
    /// Depth offset (additive, from mod matrix).
    mod_offset_depth: BipolarValue,
    /// Generic mod-matrix offsets for the non-rate/depth params (phase).
    mod_offsets: ParamModOffsets,
    output_buffer: AudioBuffer,
}

impl Lfo {
    pub fn new() -> Self {
        Self {
            waveform: LfoWaveform::Sine,
            rate: Hertz::new(1.0),
            depth: NormalizedValue::MAX,
            phase_offset: Phase::ZERO,
            mode: LfoMode::Bipolar,
            phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            sh_value: 0.0,
            sh_trigger_prev: NormalizedValue::MIN,
            smooth_random_current: 0.0,
            smooth_random_target: 0.0,
            rng: RtRng::new(0x4C46_4F01),
            rng_seed: 0x4C46_4F01,
            sync_mode: SyncMode::Free,
            sync_division: BeatDivision::QUARTER,
            retrigger_mode: RetriggerMode::Continue,
            prev_retrigger: NormalizedValue::MIN,
            mod_offset_rate: BipolarValue::CENTER,
            mod_offset_depth: BipolarValue::CENTER,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    #[inline]
    fn random(&mut self) -> f32 {
        self.rng.next_bipolar()
    }

    #[inline]
    fn generate_sample(&mut self, effective_rate: Hertz, effective_phase_offset: f32) -> f32 {
        let phase_inc = effective_rate.phase_increment(self.sample_rate);
        let phase_wrapped = self.phase.advance(effective_phase_offset);
        let phase = phase_wrapped.as_f32();

        let raw = match self.waveform {
            LfoWaveform::Sine => phase_wrapped.sin(),
            LfoWaveform::Triangle => phase_wrapped.triangle(),
            LfoWaveform::Sawtooth => phase_wrapped.sawtooth(),
            LfoWaveform::Square => phase_wrapped.pulse(NormalizedValue::CENTER),
            LfoWaveform::SampleAndHold => {
                let trigger = if phase < phase_inc { 1.0 } else { 0.0 };
                if trigger > 0.5 && self.sh_trigger_prev.as_f32() <= 0.5 {
                    self.sh_value = self.random();
                }
                self.sh_trigger_prev = NormalizedValue::new(trigger);
                self.sh_value
            }
            LfoWaveform::SmoothRandom => {
                // Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/269-smooth-random-lfo-generator.rst
                // From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
                // Pick new target when phase wraps around
                if phase < phase_inc {
                    self.smooth_random_target = self.random();
                }
                // Cosine interpolation for smooth transitions
                let t = phase;
                let interp = 0.5 * (1.0 - (t * std::f32::consts::PI).cos());
                self.smooth_random_current = self.smooth_random_current * (1.0 - interp)
                    + self.smooth_random_target * interp;
                self.smooth_random_current
            }
        };

        self.phase = self.phase.advance(phase_inc);

        let output = match self.mode {
            LfoMode::Bipolar => raw,
            LfoMode::Unipolar => crate::math::bipolar_to_unipolar(raw),
        };

        let effective_depth =
            (self.depth.as_f32() + self.mod_offset_depth.as_f32()).clamp(0.0, 1.0);
        output * effective_depth
    }

    pub fn retrigger(&mut self) {
        self.phase = Phase::ZERO;
    }
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Lfo {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("lfo", "LFO")
            .width(synth_core::ModuleWidth::Large)
            .description("Low Frequency Oscillator")
            .category(ModuleCategory::LFO)
            .tag("lfo")
            .tag("modulation")
            .parameter(
                ParameterDescriptor::choice(
                    "waveform",
                    Param::Lfo(LfoParam::Waveform(LfoWaveform::Sine)),
                    "Waveform",
                    LfoWaveform::to_choices(),
                )
                .description("LFO waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::float(
                    "rate",
                    Param::Lfo(LfoParam::Rate(Hertz::new(1.0))),
                    "Rate",
                )
                .description("LFO rate")
                .value_range(Hertz::LFO_RANGE)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::Lfo(LfoParam::Depth(NormalizedValue::MAX)),
                    "Depth",
                )
                .description("Modulation depth (0 = off, 1 = full)")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "phase",
                    Param::Lfo(LfoParam::Phase(Phase::ZERO)),
                    "Phase",
                )
                .description("Starting phase of the LFO cycle")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tempo_sync",
                    Param::Lfo(LfoParam::tempo_sync_default()),
                    "Tempo Sync",
                )
                .description("Sync rate to host tempo (uses Division instead of Rate)")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sync_division",
                    Param::Lfo(LfoParam::sync_division_default()),
                    "Division",
                )
                .description("Beats per LFO cycle when tempo-synced (1 = quarter note)")
                .range(0.125, 4.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "retrigger",
                    Param::Lfo(LfoParam::retrigger_default()),
                    "Retrigger",
                )
                .description("Restart the cycle on the Retrig gate edge")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .port(
                PortDescriptor::gate_input("retrigger", "Retrig").description(
                    "Restarts the LFO cycle. Connect: Envelope Gate, another LFO, Euclidean Gate",
                ),
            )
            .port(
                PortDescriptor::control_input("rate_cv", "Rate CV").description(
                    "Modulates LFO rate. Connect: another LFO, Envelope, Kinetic Modulator",
                ),
            )
            .port(
                PortDescriptor::control_output("out", "Out")
                    .value_domain(PortValueDomain::Bipolar)
                    .description(
                        "LFO signal (±1). Connect to: Oscillator FM/PM/PWM, Filter Cutoff CV, Amplifier CV",
                    ),
            )
    }
}

impl PolyModule for Lfo {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let retrigger_reader = inputs.reader(PortName::RETRIGGER, 0.0);
        let rate_cv_reader = inputs.reader(PortName::RATE_CV, 0.0);

        // In tempo sync mode, derive phase directly from beat position for lock
        let use_beat_sync = self.sync_mode.is_tempo_sync() && context.is_playing;

        // `phase` is a mod destination via the generic store (per-block constant).
        let eff_phase_offset = self
            .mod_offsets
            .effective("phase", self.phase_offset.as_f32());

        for i in 0..context.samples.as_usize() {
            // Always track retrigger input for edge detection, but only act on it
            // when retrigger_mode is enabled. This prevents stale prev_retrigger
            // state when the mode is toggled.
            if retrigger_reader.is_connected() {
                let val = retrigger_reader[i];
                if self.retrigger_mode.should_retrigger()
                    && crate::math::rising_edge(val, self.prev_retrigger.as_f32())
                {
                    self.retrigger();
                }
                self.prev_retrigger = NormalizedValue::new(val);
            }

            if use_beat_sync {
                // Calculate phase from beat position for perfect sync
                let sample_offset = i as f32 / self.sample_rate.as_f32();
                let beat_offset = sample_offset * context.tempo.as_f32() / 60.0;
                let current_beat = context.position_beats.as_f32() + beat_offset;
                // Phase cycles through one LFO cycle per sync_division beats
                let lfo_phase = (current_beat / self.sync_division.as_f32()).rem_euclid(1.0);
                self.phase = Phase::new(lfo_phase);
                self.output_buffer[i] = self.generate_sample(Hertz::new(1.0), eff_phase_offset); // Rate doesn't matter in sync
            } else {
                let base_rate = if self.sync_mode.is_tempo_sync() {
                    Hertz::new(context.tempo.as_f32() / 60.0 / self.sync_division.as_f32())
                } else {
                    // Apply mod matrix rate offset
                    Hertz::new(
                        Hertz::LFO_RANGE.clamp(self.rate.as_f32() + self.mod_offset_rate.as_f32()),
                    )
                };

                let effective_rate = if rate_cv_reader.is_connected() {
                    let mod_amount = rate_cv_reader.get(i);
                    Hertz::new(
                        Hertz::LFO_RANGE
                            .clamp(base_rate.apply_fm(BipolarValue::new(mod_amount)).as_f32()),
                    )
                } else {
                    base_rate
                };

                self.output_buffer[i] = self.generate_sample(effective_rate, eff_phase_offset);
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Lfo(lfo_param) = param {
            match lfo_param {
                LfoParam::Waveform(w) => self.waveform = w,
                LfoParam::Rate(r) => self.rate = Hertz::new(Hertz::LFO_RANGE.clamp(r.as_f32())),
                LfoParam::Depth(d) => self.depth = d,
                LfoParam::Phase(p) => self.phase_offset = p,
                LfoParam::TempoSync(s) => self.sync_mode = SyncMode::from(s),
                LfoParam::SyncDivision(d) => self.sync_division = d,
                LfoParam::Retrigger(r) => self.retrigger_mode = RetriggerMode::from(r),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Lfo(lfo_param) = param {
            Some(match lfo_param {
                LfoParam::Waveform(_) => self.waveform.index() as f32,
                LfoParam::Rate(_) => self.rate.as_f32(),
                LfoParam::Depth(_) => self.depth.as_f32(),
                LfoParam::Phase(_) => self.phase_offset.as_f32(),
                LfoParam::TempoSync(_) => {
                    if self.sync_mode.is_tempo_sync() {
                        1.0
                    } else {
                        0.0
                    }
                }
                LfoParam::SyncDivision(_) => self.sync_division.as_f32(),
                LfoParam::Retrigger(_) => {
                    if self.retrigger_mode.should_retrigger() {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Lfo(LfoParam::Waveform(self.waveform)),
            Param::Lfo(LfoParam::Rate(self.rate)),
            Param::Lfo(LfoParam::Depth(self.depth)),
            Param::Lfo(LfoParam::Phase(self.phase_offset)),
            Param::Lfo(LfoParam::TempoSync(self.sync_mode.is_tempo_sync())),
            Param::Lfo(LfoParam::SyncDivision(self.sync_division)),
            Param::Lfo(LfoParam::Retrigger(self.retrigger_mode.should_retrigger())),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Lfo
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
        self.sh_value = 0.0;
        self.prev_retrigger = NormalizedValue::MIN;
        self.rng.reseed(self.rng_seed);
    }

    fn set_seed(&mut self, seed: u64) {
        self.rng_seed = seed ^ 0x4C46_4F01;
        self.rng.reseed(self.rng_seed);
    }

    fn set_voice_index(&mut self, voice_index: u32) {
        self.set_seed(u64::from(voice_index));
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn set_mod_offset(&mut self, target: &str, value: f32) {
        match target {
            "rate" => {
                self.mod_offset_rate = BipolarValue::new(self.mod_offset_rate.as_f32() + value)
            }
            "depth" => {
                self.mod_offset_depth = BipolarValue::new(self.mod_offset_depth.as_f32() + value)
            }
            // phase goes through the generic store (scaled through its range).
            other => self.mod_offsets.add(other, value),
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_rate = BipolarValue::CENTER;
        self.mod_offset_depth = BipolarValue::CENTER;
        self.mod_offsets.clear();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lfo_creation() {
        let lfo = Lfo::new();
        assert_eq!(lfo.waveform, LfoWaveform::Sine);
    }

    /// Drift guard: the `rate` descriptor range, `Hertz::LFO_RANGE`, and
    /// `clamp_lfo` are all the same single source of truth.
    #[test]
    fn rate_descriptor_matches_lfo_range() {
        let d = Lfo::new().descriptor();
        let rate = d
            .parameters
            .iter()
            .find(|p| p.type_id == "rate")
            .expect("missing rate param");
        assert_eq!(rate.range, Hertz::LFO_RANGE);
        assert_eq!(Hertz::new(0.005).clamp_lfo().as_f32(), Hertz::LFO_RANGE.min);
        assert_eq!(Hertz::new(100.0).clamp_lfo().as_f32(), Hertz::LFO_RANGE.max);
        assert_eq!(Hertz::new(5.0).clamp_lfo().as_f32(), 5.0);
    }

    /// `phase` used to hit the dropped `_ => {}` arm; it now flows through the
    /// generic store and shifts where the sine LFO starts. A sine at phase 0
    /// begins near 0; a phase offset moves the first sample, and clearing
    /// reverts.
    #[test]
    fn phase_mod_offset_shifts_waveform() {
        let mut lfo = Lfo::new();
        let desc = lfo.descriptor();
        lfo.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(1),
            ..ProcessContext::default()
        };
        fn first(lfo: &mut Lfo, ctx: &ProcessContext) -> f32 {
            lfo.reset();
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(1));
            lfo.process(InputPorts::empty(), &mut outs, ctx);
            outs[&PortName::OUT][0]
        }

        let base = first(&mut lfo, &ctx); // sine at phase ~0 → near 0
        lfo.set_mod_offset("phase", 0.25); // quarter turn → toward the peak
        let shifted = first(&mut lfo, &ctx);
        assert!(
            (shifted - base).abs() > 0.1,
            "phase offset should shift the LFO output: {shifted} vs {base}"
        );

        lfo.clear_mod_offsets();
        assert!((first(&mut lfo, &ctx) - base).abs() < 1e-4);
    }
}
