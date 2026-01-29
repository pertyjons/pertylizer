//! Multi-point envelope for tracker compatibility.
//!
//! Supports XM/IT envelope features:
//! - Up to 25 points with linear interpolation
//! - Sustain point (holds until release)
//! - Loop region (loops between two points)
//! - Fadeout after release
//!
//! This module is designed for accurate XM/IT playback, replacing the
//! ADSR approximation used in earlier import versions.

use std::collections::HashMap;

use arrayvec::ArrayVec;
use synth_core::{
    AudioBuffer, Describable, EnvelopeFrame, EnvelopePointIndex, EnvelopeValue, FadeoutRate,
    InputPorts, MidiNote, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext,
    SampleRate, Velocity, WidgetHint,
};

/// Maximum envelope points (IT format limit).
pub const MAX_ENVELOPE_POINTS: usize = 25;

/// A single envelope point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopePoint {
    /// Frame position (x-axis, in tracker ticks).
    pub frame: EnvelopeFrame,
    /// Value at this point (y-axis, 0.0-1.0).
    pub value: EnvelopeValue,
}

impl EnvelopePoint {
    /// Create a new envelope point.
    #[must_use]
    pub const fn new(frame: EnvelopeFrame, value: EnvelopeValue) -> Self {
        Self { frame, value }
    }
}

/// Envelope playback stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiPointStage {
    /// Not playing - output is 0.
    #[default]
    Idle,
    /// Playing through envelope points.
    Playing,
    /// Holding at sustain point.
    Sustaining,
    /// Released - continuing through remaining points or fading out.
    Releasing,
    /// Fading out after envelope completes.
    Fadeout,
}

/// Multi-point envelope module.
///
/// This envelope generator supports the full XM/IT envelope specification:
/// - Up to 25 arbitrary points
/// - Linear interpolation between points
/// - Optional sustain point (holds until note-off)
/// - Optional loop region (loops while sustained)
/// - Fadeout after release
///
/// Unlike ADSR envelopes, this preserves the exact shape designed by
/// the tracker musician.
#[derive(Clone)]
pub struct MultiPointEnvelope {
    // Configuration (stored as ArrayVec for no heap in audio thread)
    points: ArrayVec<EnvelopePoint, MAX_ENVELOPE_POINTS>,
    sustain_point: Option<EnvelopePointIndex>,
    loop_start: Option<EnvelopePointIndex>,
    loop_end: Option<EnvelopePointIndex>,
    fadeout_rate: FadeoutRate,

    // Playback state
    stage: MultiPointStage,
    current_frame: f32, // Fractional frame position for sample-accurate timing
    fadeout_level: f32, // 1.0 -> 0.0 during fadeout
    velocity: NormalizedValue,

    // Timing
    sample_rate: SampleRate,
    tick_rate: f32,         // Envelope ticks per second (typically BPM * 2 / 5)
    samples_per_frame: f32, // Cached for efficiency

    // Output
    output_buffer: AudioBuffer,

    /// Previous gate value for edge detection (persists across buffers).
    prev_gate: f32,
}

impl MultiPointEnvelope {
    /// Create a new empty multi-point envelope.
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: ArrayVec::new(),
            sustain_point: None,
            loop_start: None,
            loop_end: None,
            fadeout_rate: FadeoutRate::DEFAULT,
            stage: MultiPointStage::Idle,
            current_frame: 0.0,
            fadeout_level: 1.0,
            velocity: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            tick_rate: 50.0, // Default: 125 BPM * 2 / 5 = 50 Hz
            samples_per_frame: 960.0,
            output_buffer: AudioBuffer::new(256),
            prev_gate: 0.0,
        }
    }

    /// Create an envelope with predefined points.
    ///
    /// Points should be provided in ascending frame order.
    #[must_use]
    pub fn with_points(points: &[(u16, f32)]) -> Self {
        let mut envelope = Self::new();
        for &(frame, value) in points.iter().take(MAX_ENVELOPE_POINTS) {
            envelope.points.push(EnvelopePoint {
                frame: EnvelopeFrame::new(frame),
                value: EnvelopeValue::new(value),
            });
        }
        envelope
    }

    /// Set envelope points (clears existing).
    pub fn set_points(&mut self, points: &[(u16, f32)]) {
        self.points.clear();
        for &(frame, value) in points.iter().take(MAX_ENVELOPE_POINTS) {
            self.points.push(EnvelopePoint {
                frame: EnvelopeFrame::new(frame),
                value: EnvelopeValue::new(value),
            });
        }
    }

    /// Add a single point to the envelope.
    ///
    /// Returns `true` if the point was added, `false` if at capacity.
    pub fn add_point(&mut self, frame: u16, value: f32) -> bool {
        if self.points.len() < MAX_ENVELOPE_POINTS {
            self.points.push(EnvelopePoint {
                frame: EnvelopeFrame::new(frame),
                value: EnvelopeValue::new(value),
            });
            true
        } else {
            false
        }
    }

    /// Get the current number of points.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// Get a reference to the points.
    #[must_use]
    pub fn points(&self) -> &[EnvelopePoint] {
        &self.points
    }

    /// Set the sustain point index.
    pub fn set_sustain_point(&mut self, index: Option<u8>) {
        self.sustain_point = index.map(EnvelopePointIndex::new);
    }

    /// Set the loop region.
    pub fn set_loop(&mut self, start: Option<u8>, end: Option<u8>) {
        self.loop_start = start.map(EnvelopePointIndex::new);
        self.loop_end = end.map(EnvelopePointIndex::new);
    }

    /// Set the fadeout rate.
    pub fn set_fadeout_rate(&mut self, rate: FadeoutRate) {
        self.fadeout_rate = rate;
    }

    /// Set the envelope tick rate (typically BPM * 2 / 5).
    pub fn set_tick_rate(&mut self, tick_rate: f32) {
        self.tick_rate = tick_rate.max(1.0);
        self.update_timing();
    }

    /// Get the current stage.
    #[must_use]
    pub fn stage(&self) -> MultiPointStage {
        self.stage
    }

    /// Check if the envelope is currently active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.stage != MultiPointStage::Idle
    }

    /// Trigger the envelope with a velocity.
    pub fn trigger(&mut self, velocity: f32) {
        self.stage = if self.points.is_empty() {
            MultiPointStage::Idle
        } else {
            MultiPointStage::Playing
        };
        self.current_frame = 0.0;
        self.fadeout_level = 1.0;
        self.velocity = NormalizedValue::new(velocity);
    }

    /// Release the envelope (note off).
    pub fn release(&mut self) {
        match self.stage {
            MultiPointStage::Sustaining => {
                self.stage = MultiPointStage::Releasing;
            }
            MultiPointStage::Playing => {
                // If we're playing but not at sustain, go to fadeout
                self.stage = MultiPointStage::Fadeout;
            }
            _ => {}
        }
    }

    /// Update timing calculations based on sample rate and tick rate.
    fn update_timing(&mut self) {
        self.samples_per_frame = self.sample_rate.as_f32() / self.tick_rate.max(1.0);
    }

    /// Process a single sample.
    #[inline]
    fn process_sample(&mut self) -> f32 {
        match self.stage {
            MultiPointStage::Idle => 0.0,

            MultiPointStage::Playing | MultiPointStage::Releasing => {
                // Advance frame position
                self.current_frame += 1.0 / self.samples_per_frame;

                // Get interpolated value
                let value = self.interpolate_at_frame(self.current_frame);

                // Check sustain point (only while playing, not releasing)
                if self.stage == MultiPointStage::Playing {
                    if let Some(sustain_idx) = self.sustain_point
                        && let Some(sustain_pt) = self.points.get(sustain_idx.as_usize())
                    {
                        let sustain_frame = sustain_pt.frame.as_u16() as f32;
                        if self.current_frame >= sustain_frame {
                            self.current_frame = sustain_frame;
                            self.stage = MultiPointStage::Sustaining;
                        }
                    }

                    // Check loop (only while playing)
                    if let (Some(start_idx), Some(end_idx)) = (self.loop_start, self.loop_end)
                        && let Some(end_pt) = self.points.get(end_idx.as_usize())
                    {
                        let end_frame = end_pt.frame.as_u16() as f32;
                        if self.current_frame >= end_frame
                            && let Some(start_pt) = self.points.get(start_idx.as_usize())
                        {
                            self.current_frame = start_pt.frame.as_u16() as f32;
                        }
                    }
                }

                // Check if envelope has completed
                if let Some(last) = self.points.last()
                    && self.current_frame >= last.frame.as_u16() as f32
                {
                    if self.stage == MultiPointStage::Releasing {
                        // Note released - go to fadeout
                        self.stage = MultiPointStage::Fadeout;
                    } else {
                        // Key still held - stay at final value (implicit sustain at end)
                        // This matches XM behavior: without sustain point, envelope holds at end
                        self.current_frame = last.frame.as_u16() as f32;
                    }
                }

                value.as_f32() * self.velocity.as_f32()
            }

            MultiPointStage::Sustaining => {
                // Hold at sustain point value
                let value = self.interpolate_at_frame(self.current_frame);
                value.as_f32() * self.velocity.as_f32()
            }

            MultiPointStage::Fadeout => {
                // Apply fadeout
                let decay_per_sample = self
                    .fadeout_rate
                    .to_decay_per_tick()
                    .powf(1.0 / self.samples_per_frame);
                self.fadeout_level *= decay_per_sample;

                if self.fadeout_level < 0.001 {
                    self.stage = MultiPointStage::Idle;
                    return 0.0;
                }

                // Continue from last envelope position during fadeout
                let value = if let Some(last) = self.points.last() {
                    last.value.as_f32()
                } else {
                    1.0
                };

                value * self.fadeout_level * self.velocity.as_f32()
            }
        }
    }

    /// Interpolate envelope value at a given frame position.
    fn interpolate_at_frame(&self, frame: f32) -> EnvelopeValue {
        if self.points.is_empty() {
            return EnvelopeValue::MAX;
        }
        if self.points.len() == 1 {
            return self.points[0].value;
        }

        // Find the surrounding points
        let mut prev_idx = 0;
        for (i, point) in self.points.iter().enumerate() {
            if point.frame.as_u16() as f32 > frame {
                break;
            }
            prev_idx = i;
        }

        let next_idx = (prev_idx + 1).min(self.points.len() - 1);
        if prev_idx == next_idx {
            return self.points[prev_idx].value;
        }

        let prev = &self.points[prev_idx];
        let next = &self.points[next_idx];

        let prev_frame = prev.frame.as_u16() as f32;
        let next_frame = next.frame.as_u16() as f32;

        // Linear interpolation
        let t = if next_frame > prev_frame {
            (frame - prev_frame) / (next_frame - prev_frame)
        } else {
            0.0
        };

        EnvelopeValue::new(prev.value.as_f32() + (next.value.as_f32() - prev.value.as_f32()) * t)
    }
}

impl Default for MultiPointEnvelope {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for MultiPointEnvelope {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("multi_point_envelope", "Multi-Point Env")
            .description("Multi-point envelope for tracker compatibility (XM/IT)")
            .category(ModuleCategory::Envelope)
            .tag("envelope")
            .tag("tracker")
            .tag("xm")
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(synth_core::EnvelopeParam::VelocitySensitivity(
                        NormalizedValue::MAX,
                    )),
                    "Vel Sens",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("gate", "Gate"))
            .port(PortDescriptor::control_input("velocity", "Vel"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl PolyModule for MultiPointEnvelope {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.update_timing();
        self.output_buffer.resize(context.samples.as_usize());

        let gate_input = inputs.get(PortName::GATE);
        let velocity_input = inputs.get(PortName::VELOCITY);

        for i in 0..context.samples.as_usize() {
            if let Some(gate) = gate_input {
                let gate_val = gate[i];
                if gate_val > 0.5 && self.prev_gate <= 0.5 {
                    let vel = velocity_input.map(|v| v[i]).unwrap_or(1.0);
                    self.trigger(vel);
                } else if gate_val <= 0.5 && self.prev_gate > 0.5 {
                    self.release();
                }
                self.prev_gate = gate_val;
            }
            self.output_buffer[i] = self.process_sample();
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        // Multi-point envelope uses direct configuration methods
        // rather than Param variants for point data
        if let Param::Envelope(synth_core::EnvelopeParam::VelocitySensitivity(v)) = param {
            self.velocity = v;
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Envelope(synth_core::EnvelopeParam::VelocitySensitivity(_)) = param {
            return Some(self.velocity.as_f32());
        }
        None
    }

    fn get_params(&self) -> Vec<Param> {
        vec![Param::Envelope(
            synth_core::EnvelopeParam::VelocitySensitivity(self.velocity),
        )]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::MultiPointEnvelope
    }

    fn reset(&mut self) {
        self.stage = MultiPointStage::Idle;
        self.current_frame = 0.0;
        self.fadeout_level = 1.0;
        self.prev_gate = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        self.trigger(velocity.as_f32());
    }

    fn note_off(&mut self) {
        self.release();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_envelope() {
        let env = MultiPointEnvelope::new();
        assert_eq!(env.stage(), MultiPointStage::Idle);
        assert_eq!(env.point_count(), 0);
    }

    #[test]
    fn test_with_points() {
        let env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0), (20, 0.5), (30, 0.0)]);
        assert_eq!(env.point_count(), 4);
    }

    #[test]
    fn test_trigger() {
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0)]);
        env.trigger(1.0);
        assert_eq!(env.stage(), MultiPointStage::Playing);
    }

    #[test]
    fn test_interpolation() {
        let env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0)]);
        // At frame 5, should be 0.5
        let value = env.interpolate_at_frame(5.0);
        assert!((value.as_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_interpolation_beyond_end() {
        let env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0)]);
        // At frame 15, should clamp to last value
        let value = env.interpolate_at_frame(15.0);
        assert!((value.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_sustain_point() {
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0), (20, 0.5)]);
        env.set_sustain_point(Some(1)); // Sustain at point 1 (frame 10, value 1.0)
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(1.0);

        // Process enough samples to reach sustain
        for _ in 0..50000 {
            env.process_sample();
        }

        assert_eq!(env.stage(), MultiPointStage::Sustaining);
    }

    #[test]
    fn test_release_from_sustain() {
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0), (20, 0.5)]);
        env.set_sustain_point(Some(1));
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(1.0);

        // Process to sustain
        for _ in 0..50000 {
            env.process_sample();
        }

        env.release();
        assert_eq!(env.stage(), MultiPointStage::Releasing);
    }

    #[test]
    fn test_max_points() {
        let mut env = MultiPointEnvelope::new();
        for i in 0..30 {
            env.add_point(i as u16, 0.5);
        }
        assert_eq!(env.point_count(), MAX_ENVELOPE_POINTS);
    }
}
