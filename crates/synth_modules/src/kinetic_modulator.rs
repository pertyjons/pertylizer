//! Kinetic Modulator — gate-triggered easing curve generator.
//!
//! Generates Position, Velocity, and Acceleration outputs from Penner easing curves.
//! Triggered by note-on, produces three mod matrix sources (KineticPos, KineticVel, KineticAcc).

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, PortName, ProcessContext, ResponseCurve,
    WidgetHint,
};
use synth_core::{
    EasingCurve, KineticLoopMode, KineticParam, ModuleType, Param, easing_acceleration,
    easing_position, easing_velocity,
};
use synth_core::{MidiNote, NormalizedValue, SampleRate, Seconds, Velocity};

/// Kinetic Modulator — easing-curve-based modulation source.
#[derive(Clone)]
pub struct KineticModulator {
    /// Current phase position in the curve [0.0, 1.0].
    phase: f32,
    /// Whether the modulator is active (started on note-on).
    active: bool,
    /// Direction for ping-pong mode (+1 or -1).
    direction: i8,
    /// Curve duration.
    duration: Seconds,
    /// Current easing curve type.
    curve_type: EasingCurve,
    /// Overshoot strength for Back/Elastic curves.
    overshoot: NormalizedValue,
    /// Bipolar output mode.
    bipolar: bool,
    /// Loop mode.
    loop_mode: KineticLoopMode,
    /// Retrigger on note-on.
    retrigger: bool,
    /// Current sample rate.
    sample_rate: SampleRate,
    /// Latest position output.
    output_pos: f32,
    /// Latest velocity output.
    output_vel: f32,
    /// Latest acceleration output.
    output_acc: f32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Cached interned name for the "duration_cv" control input (interning locks
    /// an internal table, so it must not run on the audio thread — see
    /// [`PortName::intern`]). The Trigger gate uses the `TRIGGER` constant.
    duration_cv_port: PortName,
    /// Previous Trigger-gate level for rising-edge detection (persists across blocks).
    prev_trigger: f32,
    /// Output buffer for audio graph compatibility.
    output_buffer: AudioBuffer,
}

impl KineticModulator {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            active: false,
            direction: 1,
            duration: Seconds::new(0.5),
            curve_type: EasingCurve::CubicOut,
            overshoot: NormalizedValue::new(0.5),
            bipolar: false,
            loop_mode: KineticLoopMode::OneShot,
            retrigger: true,
            sample_rate: SampleRate::DVD_QUALITY,
            output_pos: 0.0,
            output_vel: 0.0,
            output_acc: 0.0,
            mod_offsets: ParamModOffsets::new(),
            duration_cv_port: PortName::intern("duration_cv"),
            prev_trigger: 0.0,
            output_buffer: AudioBuffer::new(1024),
        }
    }
}

impl Default for KineticModulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for KineticModulator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("kinetic_modulator", "Kinetic Mod")
            .width(synth_core::ModuleWidth::Large)
            .description("Easing curve modulation generator")
            .category(ModuleCategory::LFO)
            .tag("modulation")
            .tag("kinetic")
            .tag("easing")
            .parameter(
                ParameterDescriptor::float(
                    "duration",
                    Param::Kinetic(KineticParam::Duration(Seconds::new(0.5))),
                    "Duration",
                )
                .description("Length of one modulation sweep")
                .range(0.01, 10.0)
                .default(0.5)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "curve",
                    Param::Kinetic(KineticParam::CurveType(EasingCurve::CubicOut)),
                    "Curve",
                    EasingCurve::to_choices(),
                )
                .description("Easing curve shape"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "overshoot",
                    Param::Kinetic(KineticParam::Overshoot(NormalizedValue::new(0.5))),
                    "Overshoot",
                )
                .description("Overshoot amount for back/elastic curves")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "loop_mode",
                    Param::Kinetic(KineticParam::LoopMode(KineticLoopMode::OneShot)),
                    "Loop Mode",
                    KineticLoopMode::to_choices(),
                )
                .description("Loop behaviour (one-shot, loop, ping-pong)"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "bipolar",
                    Param::Kinetic(KineticParam::Bipolar(false)),
                    "Bipolar",
                )
                .description("Output range (0 = 0..1, 1 = -1..+1)")
                .range(0.0, 1.0)
                .default(0.0)
                // Boolean mode toggle, not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "retrigger",
                    Param::Kinetic(KineticParam::Retrigger(true)),
                    "Retrigger",
                )
                .description("Restart on gate (0 = off, 1 = on)")
                .range(0.0, 1.0)
                .default(1.0)
                // Boolean toggle read only at note-on, not a continuous target.
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .port(
                PortDescriptor::gate_input("trigger", "Trigger")
                    .description("Rising edge restarts the sweep. Connect: Gate, Clock, LFO"),
            )
            .port(
                PortDescriptor::control_input("duration_cv", "Dur CV")
                    .description("Modulate sweep duration (exp). Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Modulation signal. Connect to: Oscillator FM, Filter Cutoff CV"),
            )
    }
}

impl PolyModule for KineticModulator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let sr = self.sample_rate.as_f32();
        self.output_buffer.resize(context.samples.as_usize());

        // Generic mod offsets — per-block constants. `duration` is additionally a
        // per-sample CV destination (Dur CV) below.
        let dur_secs = self
            .mod_offsets
            .effective("duration", self.duration.as_f32())
            .max(0.001);
        let base_phase_inc = 1.0 / (dur_secs * sr);
        let overshoot_val = self
            .mod_offsets
            .effective("overshoot", self.overshoot.as_f32());

        // Gate/CV inputs (unconnected readers return 0.0 → no change): a Trigger
        // rising edge restarts the sweep (like note-on), Dur CV exp-scales the
        // sweep duration.
        let trigger = inputs.reader(PortName::TRIGGER, 0.0);
        let dur_cv = inputs.reader(self.duration_cv_port, 0.0);

        for i in 0..context.samples.as_usize() {
            // Restart on a rising edge of the Trigger gate, mirroring note_on
            // (honouring the Retrigger toggle).
            let g = trigger.get(i);
            if trigger.is_connected() && crate::math::rising_edge(g, self.prev_trigger) {
                if self.retrigger || !self.active {
                    self.phase = 0.0;
                    self.direction = 1;
                }
                self.active = true;
            }
            self.prev_trigger = g;

            // Dur CV exp-scales the per-sample phase advance (cv=+1 → 4× longer).
            let phase_inc = if dur_cv.is_connected() {
                let eff_dur = (dur_secs * (dur_cv.get(i) * 2.0).exp2()).max(0.001);
                1.0 / (eff_dur * sr)
            } else {
                base_phase_inc
            };

            if self.active {
                // Advance phase
                if self.direction >= 0 {
                    self.phase += phase_inc;
                } else {
                    self.phase -= phase_inc;
                }

                // Handle loop/wrap/clamp
                if self.phase >= 1.0 {
                    match self.loop_mode {
                        KineticLoopMode::OneShot => {
                            self.phase = 1.0;
                            self.active = false;
                        }
                        KineticLoopMode::Loop => {
                            self.phase -= 1.0;
                        }
                        KineticLoopMode::PingPong => {
                            self.phase = 2.0 - self.phase;
                            self.direction = -1;
                        }
                    }
                } else if self.phase <= 0.0 && self.direction < 0 {
                    match self.loop_mode {
                        KineticLoopMode::PingPong => {
                            self.phase = -self.phase;
                            self.direction = 1;
                        }
                        _ => {
                            self.phase = 0.0;
                        }
                    }
                }
            }

            let t = self.phase.clamp(0.0, 1.0);
            let pos = easing_position(self.curve_type, t, overshoot_val);
            let vel = easing_velocity(self.curve_type, t, overshoot_val);
            let acc = easing_acceleration(self.curve_type, t, overshoot_val);

            // Apply bipolar remapping
            let (pos_out, vel_out, acc_out) = if self.bipolar {
                (crate::math::unipolar_to_bipolar(pos), vel, acc)
            } else {
                (pos, vel, acc)
            };

            self.output_pos = pos_out;
            self.output_vel = vel_out;
            self.output_acc = acc_out;
            self.output_buffer[i] = pos_out;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Kinetic(kp) = param {
            match kp {
                KineticParam::Duration(s) => {
                    self.duration = Seconds::new(s.as_f32().clamp(0.01, 10.0));
                }
                KineticParam::CurveType(c) => self.curve_type = c,
                KineticParam::Overshoot(v) => self.overshoot = v,
                KineticParam::Bipolar(b) => self.bipolar = b,
                KineticParam::LoopMode(m) => self.loop_mode = m,
                KineticParam::Retrigger(r) => self.retrigger = r,
                KineticParam::OutputVel(_) | KineticParam::OutputAcc(_) => {
                    // Read-only outputs, ignore set
                }
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Kinetic(kp) = param {
            Some(match kp {
                KineticParam::Duration(_) => self.duration.as_f32(),
                KineticParam::CurveType(_) => self.curve_type.index() as f32,
                KineticParam::Overshoot(_) => self.overshoot.as_f32(),
                KineticParam::Bipolar(_) => {
                    if self.bipolar {
                        1.0
                    } else {
                        0.0
                    }
                }
                KineticParam::LoopMode(_) => self.loop_mode.index() as f32,
                KineticParam::Retrigger(_) => {
                    if self.retrigger {
                        1.0
                    } else {
                        0.0
                    }
                }
                KineticParam::OutputVel(_) => self.output_vel,
                KineticParam::OutputAcc(_) => self.output_acc,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Kinetic(KineticParam::Duration(self.duration)),
            Param::Kinetic(KineticParam::CurveType(self.curve_type)),
            Param::Kinetic(KineticParam::Overshoot(self.overshoot)),
            Param::Kinetic(KineticParam::Bipolar(self.bipolar)),
            Param::Kinetic(KineticParam::LoopMode(self.loop_mode)),
            Param::Kinetic(KineticParam::Retrigger(self.retrigger)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::KineticModulator
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.active = false;
        self.direction = 1;
        self.output_pos = 0.0;
        self.output_vel = 0.0;
        self.output_acc = 0.0;
        self.prev_trigger = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        if self.retrigger || !self.active {
            self.phase = 0.0;
            self.direction = 1;
        }
        self.active = true;
    }

    fn note_off(&mut self) {
        // Kinetic modulator doesn't respond to note off —
        // it completes its curve or loops independently.
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_kinetic_modulator_basic() {
        let km = KineticModulator::new();
        assert_eq!(km.module_type(), ModuleType::KineticModulator);
        assert!(!km.active);
    }

    #[test]
    fn test_kinetic_modulator_note_on_triggers() {
        let mut km = KineticModulator::new();
        km.note_on(MidiNote::A4, Velocity::MAX);
        assert!(km.active);
        assert_eq!(km.phase, 0.0);
    }

    #[test]
    fn test_kinetic_modulator_process() {
        let mut km = KineticModulator::new();
        km.note_on(MidiNote::A4, Velocity::MAX);

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(64),
            ..ProcessContext::default()
        };

        km.process(InputPorts::empty(), &mut outputs, &context);

        // After processing, phase should have advanced
        assert!(km.phase > 0.0);
        // Output should be non-zero (CubicOut starts with steep slope)
        assert!(km.output_pos > 0.0);
    }

    #[test]
    fn test_kinetic_modulator_one_shot_completes() {
        let mut km = KineticModulator::new();
        km.duration = Seconds::new(0.01); // Very short
        km.loop_mode = KineticLoopMode::OneShot;
        km.note_on(MidiNote::A4, Velocity::MAX);

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(1024));

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(1024),
            ..ProcessContext::default()
        };

        km.process(InputPorts::empty(), &mut outputs, &context);

        // Should have completed
        assert!(!km.active);
        assert!((km.phase - 1.0).abs() < 0.01);
    }

    /// `duration` used to be dropped (KineticModulator never overrode
    /// set_mod_offset); it now scales the phase-advance rate through the generic
    /// store, so a routing changes the sweep speed and the output trajectory.
    #[test]
    fn test_kinetic_duration_mod_offset_changes_rate() {
        let render = |offset: f32| -> Vec<f32> {
            let mut km = KineticModulator::new();
            let desc = km.descriptor();
            km.mod_offsets_mut().unwrap().populate(&desc);
            km.set_param(Param::Kinetic(KineticParam::Duration(Seconds::new(2.0))));
            if offset != 0.0 {
                km.set_mod_offset("duration", offset);
            }
            km.note_on(MidiNote::A4, Velocity::MAX);
            let n = 512;
            let mut out = HashMap::new();
            out.insert(PortName::OUT, AudioBuffer::new(n));
            let context = ProcessContext {
                sample_rate: SampleRate::DVD_QUALITY,
                samples: SampleCount::new(n),
                ..ProcessContext::default()
            };
            km.process(InputPorts::empty(), &mut out, &context);
            (0..n).map(|i| out[&PortName::OUT][i]).collect()
        };
        let base = render(0.0);
        let faster = render(-0.5); // shorter duration → faster sweep
        let diff: f32 = base.iter().zip(&faster).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 0.01,
            "duration offset should change sweep rate, diff={diff}"
        );

        // Clearing the offset reverts to the baseline trajectory.
        let mut km = KineticModulator::new();
        let desc = km.descriptor();
        km.mod_offsets_mut().unwrap().populate(&desc);
        km.set_param(Param::Kinetic(KineticParam::Duration(Seconds::new(2.0))));
        km.set_mod_offset("duration", -0.5);
        km.clear_mod_offsets();
        km.note_on(MidiNote::A4, Velocity::MAX);
        let n = 512;
        let mut out = HashMap::new();
        out.insert(PortName::OUT, AudioBuffer::new(n));
        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(n),
            ..ProcessContext::default()
        };
        km.process(InputPorts::empty(), &mut out, &context);
        let reverted: Vec<f32> = (0..n).map(|i| out[&PortName::OUT][i]).collect();
        let back: f32 = base.iter().zip(&reverted).map(|(a, b)| (a - b).abs()).sum();
        assert!(back < 1e-3, "clearing reverts duration, residual={back}");
    }

    #[test]
    fn test_kinetic_modulator_get_param_outputs() {
        let mut km = KineticModulator::new();
        km.output_vel = 0.5;
        km.output_acc = -0.3;

        let vel = km
            .get_param(&Param::Kinetic(KineticParam::OutputVel(0.0)))
            .unwrap();
        assert!((vel - 0.5).abs() < 0.001);

        let acc = km
            .get_param(&Param::Kinetic(KineticParam::OutputAcc(0.0)))
            .unwrap();
        assert!((acc - (-0.3)).abs() < 0.001);
    }
}
