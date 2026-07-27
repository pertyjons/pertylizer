//! MSEG (Multi-Stage Envelope Generator) module.
//!
//! A flexible envelope generator with up to 16 segments, each with configurable
//! time, level, and curve. Supports sustain hold, looping, and trigger/gate input.
//!
//! Features:
//! - Up to 16 segments with independent time, level, and curve
//! - Sustain segment: holds until gate off
//! - Loop support with configurable start/end points
//! - Exponential curve interpolation per segment
//! - Gate and trigger inputs

use std::collections::HashMap;
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Seconds, TimeScale,
    Velocity,
};
use synth_core::{ModuleType, MsegParam, Param};

/// Maximum number of segments in the MSEG.
const MAX_SEGMENTS: usize = 16;

/// A single segment of the MSEG envelope.
#[derive(Debug, Clone, Copy)]
struct MsegSegment {
    /// Duration of this segment.
    time: Seconds,
    /// Target level at end of segment.
    level: NormalizedValue,
    /// Curve shape: -1 = logarithmic, 0 = linear, +1 = exponential.
    curve: BipolarValue,
}

impl Default for MsegSegment {
    fn default() -> Self {
        Self {
            time: Seconds::new(0.1),
            level: NormalizedValue::MIN,
            curve: BipolarValue::CENTER,
        }
    }
}

/// State of the MSEG envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MsegState {
    /// Envelope is idle (output 0).
    Idle,
    /// Envelope is running through segment at the given index.
    Running(u8),
    /// Envelope is holding at the sustain segment.
    Sustain,
    /// Envelope is in release (continuing from sustain through remaining segments).
    Release(u8),
}

/// Multi-Stage Envelope Generator module.
#[derive(Clone)]
pub struct Mseg {
    // Parameters
    segments: [MsegSegment; MAX_SEGMENTS],
    segment_count: u8,
    sustain_segment: u8,
    loop_start: u8,
    loop_end: u8,
    loop_enabled: bool,
    time_scale: TimeScale,

    // State
    state: MsegState,
    /// Phase within the current segment (0.0 to 1.0).
    phase: Phase,
    /// Level at the start of the current segment (for interpolation).
    start_level: NormalizedValue,
    /// Previous gate value for edge detection.
    prev_gate: NormalizedValue,
    /// Previous trigger value for edge detection.
    prev_trigger: NormalizedValue,
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl Mseg {
    pub fn new() -> Self {
        // Default: 4 segments forming a basic ADSR-like shape
        let mut segments = [MsegSegment::default(); MAX_SEGMENTS];
        // Attack: 0 -> 1
        segments[0] = MsegSegment {
            time: Seconds::new(0.01),
            level: NormalizedValue::MAX,
            curve: BipolarValue::CENTER,
        };
        // Decay: 1 -> 0.7
        segments[1] = MsegSegment {
            time: Seconds::new(0.1),
            level: NormalizedValue::new(0.7),
            curve: BipolarValue::CENTER,
        };
        // Sustain: 0.7 -> 0.7 (hold)
        segments[2] = MsegSegment {
            time: Seconds::new(0.5),
            level: NormalizedValue::new(0.7),
            curve: BipolarValue::CENTER,
        };
        // Release: 0.7 -> 0
        segments[3] = MsegSegment {
            time: Seconds::new(0.3),
            level: NormalizedValue::MIN,
            curve: BipolarValue::CENTER,
        };

        Self {
            segments,
            segment_count: 4,
            sustain_segment: 2,
            loop_start: 0,
            loop_end: 1,
            loop_enabled: false,
            time_scale: TimeScale::UNITY,
            state: MsegState::Idle,
            phase: Phase::ZERO,
            start_level: NormalizedValue::MIN,
            prev_gate: NormalizedValue::MIN,
            prev_trigger: NormalizedValue::MIN,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Create a preset ADSR envelope.
    ///
    /// 4 segments: Attack (0->1), Decay (1->0.7), Sustain hold, Release (0.7->0).
    #[must_use]
    pub fn preset_adsr() -> Self {
        let mut mseg = Self::new();
        mseg.segment_count = 4;
        mseg.sustain_segment = 2;
        mseg.loop_enabled = false;

        // Attack
        mseg.segments[0] = MsegSegment {
            time: Seconds::new(0.01),
            level: NormalizedValue::MAX,
            curve: BipolarValue::new(0.3),
        };
        // Decay
        mseg.segments[1] = MsegSegment {
            time: Seconds::new(0.15),
            level: NormalizedValue::new(0.7),
            curve: BipolarValue::new(-0.3),
        };
        // Sustain (holds here)
        mseg.segments[2] = MsegSegment {
            time: Seconds::new(1.0),
            level: NormalizedValue::new(0.7),
            curve: BipolarValue::CENTER,
        };
        // Release
        mseg.segments[3] = MsegSegment {
            time: Seconds::new(0.3),
            level: NormalizedValue::MIN,
            curve: BipolarValue::new(-0.5),
        };

        mseg
    }

    /// Create a preset tremolo (looped triangle).
    ///
    /// 2 segments looping: rise (0->1), fall (1->0).
    #[must_use]
    pub fn preset_tremolo() -> Self {
        let mut mseg = Self::new();
        mseg.segment_count = 2;
        mseg.sustain_segment = 15; // No sustain hold (beyond segment count)
        mseg.loop_enabled = true;
        mseg.loop_start = 0;
        mseg.loop_end = 1;

        // Rise
        mseg.segments[0] = MsegSegment {
            time: Seconds::new(0.25),
            level: NormalizedValue::MAX,
            curve: BipolarValue::CENTER,
        };
        // Fall
        mseg.segments[1] = MsegSegment {
            time: Seconds::new(0.25),
            level: NormalizedValue::MIN,
            curve: BipolarValue::CENTER,
        };

        mseg
    }

    /// Create a preset sidechain pump effect.
    ///
    /// 3 segments: fast duck (1->0), slow rise (0->1), hold at top.
    #[must_use]
    pub fn preset_sidechain_pump() -> Self {
        let mut mseg = Self::new();
        mseg.segment_count = 3;
        mseg.sustain_segment = 15; // No sustain hold
        mseg.loop_enabled = true;
        mseg.loop_start = 0;
        mseg.loop_end = 2;

        // Fast duck down
        mseg.segments[0] = MsegSegment {
            time: Seconds::new(0.01),
            level: NormalizedValue::MIN,
            curve: BipolarValue::new(-0.8),
        };
        // Slow pump back up
        mseg.segments[1] = MsegSegment {
            time: Seconds::new(0.3),
            level: NormalizedValue::MAX,
            curve: BipolarValue::new(0.5),
        };
        // Brief hold at top
        mseg.segments[2] = MsegSegment {
            time: Seconds::new(0.19),
            level: NormalizedValue::MAX,
            curve: BipolarValue::CENTER,
        };

        mseg
    }

    /// Get the current output level based on state and phase.
    #[inline]
    fn current_level(&self) -> f32 {
        match self.state {
            MsegState::Idle => 0.0,
            MsegState::Sustain => {
                let idx = self
                    .sustain_segment
                    .min(self.segment_count.saturating_sub(1));
                self.segments[idx as usize].level.as_f32()
            }
            MsegState::Running(idx) | MsegState::Release(idx) => {
                let seg = &self.segments[idx as usize];
                let target = seg.level.as_f32();
                Self::interpolate_curve(
                    self.start_level.as_f32(),
                    target,
                    self.phase.as_f32(),
                    seg.curve,
                )
            }
        }
    }

    /// Interpolate between two levels using the curve parameter.
    ///
    /// curve = 0: linear
    /// curve < 0: logarithmic (fast start, slow end)
    /// curve > 0: exponential (slow start, fast end)
    #[inline]
    fn interpolate_curve(from: f32, to: f32, t: f32, curve: BipolarValue) -> f32 {
        let t_clamped = t.clamp(0.0, 1.0);
        let c = curve.as_f32();
        crate::math::interpolate_with_curve(from, to, t_clamped, c)
    }

    /// Get the effective segment time, scaled by the time_scale parameter.
    ///
    /// `time_scale` is stored as the direct multiplier (0.01..10.0).
    #[inline]
    fn effective_time(&self, seg_idx: u8) -> f32 {
        let raw_time = self.segments[seg_idx as usize].time.as_f32();
        // Generic mod offset on the global time scale.
        raw_time
            * self
                .mod_offsets
                .effective("time_scale", self.time_scale.as_f32())
    }

    /// Advance to the next segment, handling sustain and looping.
    #[inline]
    fn advance_to_next_segment(&mut self, current_idx: u8) {
        let next_idx = current_idx + 1;

        // Check sustain hold
        if current_idx == self.sustain_segment && current_idx < self.segment_count {
            self.state = MsegState::Sustain;
            return;
        }

        // Check loop
        if self.loop_enabled
            && current_idx == self.loop_end
            && self.loop_start < self.segment_count
            && self.loop_end < self.segment_count
            && self.loop_start <= self.loop_end
        {
            let loop_idx = self.loop_start;
            self.start_level = self.segments[current_idx as usize].level;
            self.state = MsegState::Running(loop_idx);
            self.phase = Phase::ZERO;
            return;
        }

        // Move to next segment or finish
        if next_idx >= self.segment_count {
            self.state = MsegState::Idle;
        } else {
            self.state = MsegState::Running(next_idx);
            self.phase = Phase::ZERO;
            self.start_level = self.segments[current_idx as usize].level;
        }
    }

    /// Advance to next segment during release (no sustain hold, no looping).
    #[inline]
    fn advance_release_segment(&mut self, current_idx: u8) {
        let next_idx = current_idx + 1;
        if next_idx >= self.segment_count {
            self.state = MsegState::Idle;
        } else {
            self.state = MsegState::Release(next_idx);
            self.phase = Phase::ZERO;
            self.start_level = self.segments[current_idx as usize].level;
        }
    }

    /// Begin the envelope from segment 0.
    fn trigger_envelope(&mut self) {
        if self.segment_count == 0 {
            self.state = MsegState::Idle;
            return;
        }
        self.state = MsegState::Running(0);
        self.phase = Phase::ZERO;
        self.start_level = NormalizedValue::MIN;
    }

    /// Release the envelope (exit sustain, continue through remaining segments).
    fn release_envelope(&mut self) {
        match self.state {
            MsegState::Sustain => {
                let next_idx = self.sustain_segment + 1;
                if next_idx >= self.segment_count {
                    self.state = MsegState::Idle;
                } else {
                    self.state = MsegState::Release(next_idx);
                    self.phase = Phase::ZERO;
                    self.start_level = self.segments[self.sustain_segment as usize].level;
                }
            }
            MsegState::Running(idx) => {
                // If currently running before sustain, skip to first post-sustain segment
                if idx <= self.sustain_segment {
                    let release_idx = self.sustain_segment + 1;
                    if release_idx >= self.segment_count {
                        self.state = MsegState::Idle;
                    } else {
                        self.state = MsegState::Release(release_idx);
                        self.phase = Phase::ZERO;
                        // Start release from the current interpolated level for smooth transition
                        self.start_level = NormalizedValue::new(self.current_level());
                    }
                } else {
                    // Already past sustain, just switch to release mode
                    self.state = MsegState::Release(idx);
                }
            }
            MsegState::Idle | MsegState::Release(_) => {
                // Already idle or releasing, do nothing
            }
        }
    }
}

impl Default for Mseg {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Mseg {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("mseg", "MSEG")
            .width(synth_core::ModuleWidth::ExtraLarge)
            .description("Multi-Stage Envelope Generator with up to 16 segments")
            .category(ModuleCategory::Envelope)
            .tag("envelope")
            .tag("mseg")
            .tag("modulation")
            .parameter(
                ParameterDescriptor::float(
                    "segments",
                    Param::Mseg(MsegParam::SegmentCount(4)),
                    "Segments",
                )
                .description("Number of active segments (1-16)")
                .range(1.0, 16.0)
                .default(4.0)
                // Structural/sizing count: not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sustain_seg",
                    Param::Mseg(MsegParam::SustainSegment(2)),
                    "Sustain Seg",
                )
                .description("Segment index where envelope holds until gate off (0-15)")
                .range(0.0, 15.0)
                .default(2.0)
                // Discrete segment index: not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "loop",
                    Param::Mseg(MsegParam::LoopEnabled(false)),
                    "Loop",
                )
                .description("Enable segment looping")
                .range(0.0, 1.0)
                .default(0.0)
                // Boolean toggle, not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "loop_start",
                    Param::Mseg(MsegParam::LoopStart(0)),
                    "Loop Start",
                )
                .description("First segment of the loop (0-15)")
                .range(0.0, 15.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "loop_end",
                    Param::Mseg(MsegParam::LoopEnd(1)),
                    "Loop End",
                )
                .description("Last segment of the loop (0-15)")
                .range(0.0, 15.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "time_scale",
                    Param::Mseg(MsegParam::TimeScale(TimeScale::UNITY)),
                    "Time Scale",
                )
                .description("Global time scale multiplier (0.01-10.0)")
                .range(0.01, 10.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            );

        // Per-segment shape (time/level/curve) for every slot. Hidden from the
        // auto-renderer (MSEG wants a graphical editor) but kept in the descriptor
        // so the envelope shape round-trips through descriptor-driven save/load
        // and is settable via MCP. Inactive slots (index >= segment_count) have no
        // matching get_params() entry and are simply skipped on save.
        for i in 0..MAX_SEGMENTS as u8 {
            desc = desc
                .parameter(
                    ParameterDescriptor::float(
                        format!("seg{i}_time"),
                        Param::Mseg(MsegParam::SegmentTime(i, Seconds::new(0.1))),
                        format!("Seg {i} Time"),
                    )
                    .description("Segment duration in seconds")
                    .range(0.0, 60.0)
                    .default(0.1)
                    .modulatable(false)
                    .widget(WidgetHint::Hidden),
                )
                .parameter(
                    ParameterDescriptor::float(
                        format!("seg{i}_level"),
                        Param::Mseg(MsegParam::SegmentLevel(i, NormalizedValue::MIN)),
                        format!("Seg {i} Level"),
                    )
                    .description("Segment target level")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .modulatable(false)
                    .widget(WidgetHint::Hidden),
                )
                .parameter(
                    ParameterDescriptor::float(
                        format!("seg{i}_curve"),
                        Param::Mseg(MsegParam::SegmentCurve(i, BipolarValue::CENTER)),
                        format!("Seg {i} Curve"),
                    )
                    .description("Segment curve (-1 log, 0 linear, +1 exp)")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .modulatable(false)
                    .widget(WidgetHint::Hidden),
                );
        }

        desc.port(PortDescriptor::gate_input("gate", "Gate").description("Gate input (>0.5 = on)"))
            .port(
                PortDescriptor::gate_input("trigger", "Trigger")
                    .description("Trigger input (retrigger on rising edge)"),
            )
            .port(PortDescriptor::control_output("out", "Out").description("Envelope output (0-1)"))
    }
}

impl PolyModule for Mseg {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let gate_input = inputs.get(PortName::GATE);
        let trigger_input = inputs.get(PortName::TRIGGER);

        let sample_dt = 1.0 / self.sample_rate.as_f32();

        for i in 0..num_samples {
            // Read gate
            let gate_val = gate_input.map_or(0.0, |buf| buf[i]);
            let trigger_val = trigger_input.map_or(0.0, |buf| buf[i]);

            // Detect gate rising edge -> start envelope
            let gate_rising = crate::math::rising_edge(gate_val, self.prev_gate.as_f32());
            // Detect gate falling edge -> release
            let gate_falling = gate_val <= 0.5 && self.prev_gate.as_f32() > 0.5;
            // Detect trigger rising edge -> retrigger
            let trigger_rising = crate::math::rising_edge(trigger_val, self.prev_trigger.as_f32());

            if gate_rising || trigger_rising {
                self.trigger_envelope();
            } else if gate_falling {
                self.release_envelope();
            }

            self.prev_gate = NormalizedValue::new(gate_val);
            self.prev_trigger = NormalizedValue::new(trigger_val);

            // Advance phase based on current state
            match self.state {
                MsegState::Idle => {
                    // Output silence
                }
                MsegState::Sustain => {
                    // Hold at sustain level, nothing to advance
                }
                MsegState::Running(idx) => {
                    let seg_time = self.effective_time(idx);
                    if seg_time <= 0.001 {
                        // Instant segment: snap to end level, move to next
                        self.start_level = self.segments[idx as usize].level;
                        self.advance_to_next_segment(idx);
                    } else {
                        let phase_inc = sample_dt / seg_time;
                        let new_phase = self.phase.as_f32() + phase_inc;
                        if new_phase >= 1.0 {
                            // Segment complete
                            self.start_level = self.segments[idx as usize].level;
                            self.advance_to_next_segment(idx);
                        } else {
                            self.phase = Phase::new(new_phase);
                        }
                    }
                }
                MsegState::Release(idx) => {
                    let seg_time = self.effective_time(idx);
                    if seg_time <= 0.001 {
                        self.start_level = self.segments[idx as usize].level;
                        self.advance_release_segment(idx);
                    } else {
                        let phase_inc = sample_dt / seg_time;
                        let new_phase = self.phase.as_f32() + phase_inc;
                        if new_phase >= 1.0 {
                            self.start_level = self.segments[idx as usize].level;
                            self.advance_release_segment(idx);
                        } else {
                            self.phase = Phase::new(new_phase);
                        }
                    }
                }
            }

            self.output_buffer[i] = self.current_level();
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Mseg(p) = param {
            match p {
                MsegParam::SegmentCount(n) => {
                    self.segment_count = n.clamp(1, MAX_SEGMENTS as u8);
                }
                MsegParam::SustainSegment(n) => {
                    self.sustain_segment = n.min(MAX_SEGMENTS as u8 - 1);
                }
                MsegParam::LoopStart(n) => {
                    self.loop_start = n.min(MAX_SEGMENTS as u8 - 1);
                }
                MsegParam::LoopEnd(n) => {
                    self.loop_end = n.min(MAX_SEGMENTS as u8 - 1);
                }
                MsegParam::LoopEnabled(b) => {
                    self.loop_enabled = b;
                }
                MsegParam::TimeScale(v) => {
                    self.time_scale = v;
                }
                MsegParam::SegmentTime(idx, t) => {
                    let i = (idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].time = Seconds::new(t.as_f32().max(0.0));
                }
                MsegParam::SegmentLevel(idx, v) => {
                    let i = (idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].level = v;
                }
                MsegParam::SegmentCurve(idx, v) => {
                    let i = (idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].curve = v;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Mseg(p) = param {
            Some(match p {
                MsegParam::SegmentCount(_) => self.segment_count as f32,
                MsegParam::SustainSegment(_) => self.sustain_segment as f32,
                MsegParam::LoopStart(_) => self.loop_start as f32,
                MsegParam::LoopEnd(_) => self.loop_end as f32,
                MsegParam::LoopEnabled(_) => {
                    if self.loop_enabled {
                        1.0
                    } else {
                        0.0
                    }
                }
                MsegParam::TimeScale(_) => self.time_scale.as_f32(),
                MsegParam::SegmentTime(idx, _) => {
                    let i = (*idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].time.as_f32()
                }
                MsegParam::SegmentLevel(idx, _) => {
                    let i = (*idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].level.as_f32()
                }
                MsegParam::SegmentCurve(idx, _) => {
                    let i = (*idx as usize).min(MAX_SEGMENTS - 1);
                    self.segments[i].curve.as_f32()
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        let mut params = vec![
            Param::Mseg(MsegParam::SegmentCount(self.segment_count)),
            Param::Mseg(MsegParam::SustainSegment(self.sustain_segment)),
            Param::Mseg(MsegParam::LoopStart(self.loop_start)),
            Param::Mseg(MsegParam::LoopEnd(self.loop_end)),
            Param::Mseg(MsegParam::LoopEnabled(self.loop_enabled)),
            Param::Mseg(MsegParam::TimeScale(self.time_scale)),
        ];

        for i in 0..self.segment_count.min(MAX_SEGMENTS as u8) {
            let seg = &self.segments[i as usize];
            params.push(Param::Mseg(MsegParam::SegmentTime(i, seg.time)));
            params.push(Param::Mseg(MsegParam::SegmentLevel(i, seg.level)));
            params.push(Param::Mseg(MsegParam::SegmentCurve(i, seg.curve)));
        }

        params
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Mseg
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.state = MsegState::Idle;
        self.phase = Phase::ZERO;
        self.start_level = NormalizedValue::MIN;
        self.prev_gate = NormalizedValue::MIN;
        self.prev_trigger = NormalizedValue::MIN;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        self.trigger_envelope();
    }

    fn note_off(&mut self) {
        self.release_envelope();
    }

    fn is_release_done(&self) -> bool {
        matches!(self.state, MsegState::Idle)
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;
    use synth_core::SampleCount;

    fn make_context<'a>(samples: usize) -> ProcessContext<'a> {
        ProcessContext {
            samples: SampleCount::new(samples),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        }
    }

    /// `time_scale` used to be dropped (Mseg never overrode set_mod_offset); it
    /// now scales every segment's duration through the generic store, so a
    /// routing changes how fast the envelope progresses.
    #[test]
    fn test_mseg_time_scale_mod_offset_changes_rate() {
        let render = |offset: f32| -> Vec<f32> {
            let mut m = Mseg::new();
            let desc = m.descriptor();
            m.mod_offsets_mut().unwrap().populate(&desc);
            if offset != 0.0 {
                m.set_mod_offset("time_scale", offset);
            }
            m.note_on(MidiNote::new(60), Velocity::MAX);
            let n = 2048;
            let mut out = HashMap::new();
            out.insert(PortName::OUT, AudioBuffer::new(n));
            m.process(InputPorts::empty(), &mut out, &make_context(n));
            (0..n).map(|i| out[&PortName::OUT][i]).collect()
        };
        let base = render(0.0);
        let slowed = render(0.5); // larger time_scale → slower envelope
        let diff: f32 = base.iter().zip(&slowed).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 0.1,
            "time_scale offset should change envelope rate, diff={diff}"
        );

        // Clearing reverts to the baseline trajectory.
        let mut m = Mseg::new();
        let desc = m.descriptor();
        m.mod_offsets_mut().unwrap().populate(&desc);
        m.set_mod_offset("time_scale", 0.5);
        m.clear_mod_offsets();
        m.note_on(MidiNote::new(60), Velocity::MAX);
        let n = 2048;
        let mut out = HashMap::new();
        out.insert(PortName::OUT, AudioBuffer::new(n));
        m.process(InputPorts::empty(), &mut out, &make_context(n));
        let reverted: Vec<f32> = (0..n).map(|i| out[&PortName::OUT][i]).collect();
        let back: f32 = base.iter().zip(&reverted).map(|(a, b)| (a - b).abs()).sum();
        assert!(back < 1e-3, "clearing reverts time_scale, residual={back}");
    }

    #[test]
    fn test_mseg_creation() {
        let mseg = Mseg::new();
        assert_eq!(mseg.segment_count, 4);
        assert_eq!(mseg.sustain_segment, 2);
        assert!(!mseg.loop_enabled);
        assert_eq!(mseg.state, MsegState::Idle);
    }

    #[test]
    fn test_mseg_default() {
        let mseg = Mseg::default();
        assert_eq!(mseg.segment_count, 4);
    }

    #[test]
    fn test_mseg_idle_output_silence() {
        let mut mseg = Mseg::new();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = make_context(64);
        mseg.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        for i in 0..64 {
            assert!(
                out[i].abs() < 0.001,
                "Expected silence when idle, got {} at sample {}",
                out[i],
                i
            );
        }
    }

    #[test]
    fn test_mseg_trigger_starts_envelope() {
        let mut mseg = Mseg::new();
        mseg.note_on(MidiNote::new(60), Velocity::MAX);
        assert_matches!(mseg.state, MsegState::Running(0));
    }

    #[test]
    fn test_mseg_set_params() {
        let mut mseg = Mseg::new();
        mseg.set_param(Param::Mseg(MsegParam::SegmentCount(8)));
        assert_eq!(mseg.segment_count, 8);

        mseg.set_param(Param::Mseg(MsegParam::LoopEnabled(true)));
        assert!(mseg.loop_enabled);

        mseg.set_param(Param::Mseg(MsegParam::SegmentTime(0, Seconds::new(0.5))));
        assert!((mseg.segments[0].time.as_f32() - 0.5).abs() < 0.001);

        mseg.set_param(Param::Mseg(MsegParam::SegmentLevel(
            1,
            NormalizedValue::new(0.8),
        )));
        assert!((mseg.segments[1].level.as_f32() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_mseg_get_params() {
        let mseg = Mseg::new();
        let params = mseg.get_params();
        // 6 global params + 4 segments * 3 params each = 18
        assert_eq!(params.len(), 18);
    }

    #[test]
    fn test_mseg_get_param() {
        let mseg = Mseg::new();
        let count = mseg.get_param(&Param::Mseg(MsegParam::SegmentCount(0)));
        assert_eq!(count, Some(4.0));

        let loop_en = mseg.get_param(&Param::Mseg(MsegParam::LoopEnabled(false)));
        assert_eq!(loop_en, Some(0.0));
    }

    #[test]
    fn test_mseg_reset() {
        let mut mseg = Mseg::new();
        mseg.note_on(MidiNote::new(60), Velocity::MAX);
        assert!(mseg.state != MsegState::Idle);

        mseg.reset();
        assert_eq!(mseg.state, MsegState::Idle);
        assert!((mseg.phase.as_f32()).abs() < 0.001);
    }

    #[test]
    fn test_mseg_module_type() {
        let mseg = Mseg::new();
        assert_eq!(mseg.module_type(), ModuleType::Mseg);
    }

    #[test]
    fn test_mseg_preset_adsr() {
        let mseg = Mseg::preset_adsr();
        assert_eq!(mseg.segment_count, 4);
        assert_eq!(mseg.sustain_segment, 2);
        assert!(!mseg.loop_enabled);
    }

    #[test]
    fn test_mseg_preset_tremolo() {
        let mseg = Mseg::preset_tremolo();
        assert_eq!(mseg.segment_count, 2);
        assert!(mseg.loop_enabled);
        assert_eq!(mseg.loop_start, 0);
        assert_eq!(mseg.loop_end, 1);
    }

    #[test]
    fn test_mseg_preset_sidechain_pump() {
        let mseg = Mseg::preset_sidechain_pump();
        assert_eq!(mseg.segment_count, 3);
        assert!(mseg.loop_enabled);
    }

    #[test]
    fn test_interpolate_curve_linear() {
        let result = Mseg::interpolate_curve(0.0, 1.0, 0.5, BipolarValue::CENTER);
        assert!(
            (result - 0.5).abs() < 0.01,
            "Linear midpoint should be ~0.5, got {result}"
        );
    }

    #[test]
    fn test_interpolate_curve_endpoints() {
        // At t=0, should be at start
        let result = Mseg::interpolate_curve(0.0, 1.0, 0.0, BipolarValue::new(0.5));
        assert!(result.abs() < 0.01, "At t=0 should be ~0, got {result}");

        // At t=1, should be at end
        let result = Mseg::interpolate_curve(0.0, 1.0, 1.0, BipolarValue::new(-0.5));
        assert!(
            (result - 1.0).abs() < 0.01,
            "At t=1 should be ~1, got {result}"
        );
    }

    #[test]
    fn test_mseg_descriptor() {
        let mseg = Mseg::new();
        let desc = mseg.descriptor();
        assert_eq!(desc.type_id.as_str(), "mseg");
        assert_eq!(desc.category, ModuleCategory::Envelope);
    }

    #[test]
    fn test_mseg_box_clone() {
        let mseg = Mseg::new();
        let _cloned = mseg.box_clone();
    }
}
