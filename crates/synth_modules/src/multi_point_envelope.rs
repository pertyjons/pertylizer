//! Multi-point envelope module.
//!
//! Supports advanced envelope features:
//! - Up to 25 points with linear interpolation
//! - Sustain point (holds until release)
//! - Loop region (loops between two points)
//! - Fadeout runs in parallel with envelope after release
//! - Sustain/loop interaction

use std::collections::HashMap;

use arrayvec::ArrayVec;
use synth_core::{
    AudioBuffer, Describable, EnvelopeFrame, EnvelopePointIndex, EnvelopeValue, FadeoutRate,
    InputPorts, MidiNote, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext,
    SampleRate, Velocity, WidgetHint,
};

/// Maximum envelope points.
pub const MAX_ENVELOPE_POINTS: usize = 25;

/// A single envelope point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopePoint {
    /// Frame position (x-axis, in envelope ticks).
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
}

/// Multi-point envelope module.
///
/// This envelope generator supports:
/// - Up to 25 arbitrary points
/// - Linear interpolation between points
/// - Optional sustain point (holds until note-off)
/// - Optional loop region
/// - Linear fadeout running in parallel with envelope after release
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
    fadeout_level: f32, // 1.0 -> 0.0 during fadeout (linear subtraction)
    released: bool,     // True after note-off (fadeout runs in parallel)
    velocity: NormalizedValue,
    velocity_sensitivity: NormalizedValue,

    // Cached fadeout amount per sample (computed once per buffer)
    fadeout_per_sample: f32,

    // Output mode
    output_bipolar: bool, // If true, remap 0.0-1.0 to -1.0..+1.0 (for panning envelope)

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
            released: false,
            velocity: NormalizedValue::MAX,
            velocity_sensitivity: NormalizedValue::MAX,
            fadeout_per_sample: 0.0,
            output_bipolar: false,
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

    /// Set output to bipolar mode (-1.0 to +1.0) for panning envelope.
    pub fn set_output_bipolar(&mut self, bipolar: bool) {
        self.output_bipolar = bipolar;
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
    pub fn trigger(&mut self, velocity: Velocity) {
        self.stage = if self.points.is_empty() {
            MultiPointStage::Idle
        } else {
            MultiPointStage::Playing
        };
        self.current_frame = 0.0;
        self.fadeout_level = 1.0;
        self.released = false;
        self.velocity = NormalizedValue::new(velocity.as_f32());
    }

    /// Release the envelope (note off).
    ///
    /// In FT2, release causes:
    /// 1. Envelope continues advancing past sustain point
    /// 2. Fadeout starts running in parallel (subtracted each tick)
    /// 3. Loop continues UNLESS sustain_point == loop_end (then loop stops)
    pub fn release(&mut self) {
        match self.stage {
            MultiPointStage::Sustaining => {
                // Resume playing past sustain point
                self.stage = MultiPointStage::Playing;
                self.released = true;
            }
            MultiPointStage::Playing => {
                // Already playing - just start fadeout in parallel
                self.released = true;
            }
            MultiPointStage::Idle => {}
        }
    }

    /// Update timing calculations based on sample rate and tick rate.
    fn update_timing(&mut self) {
        self.samples_per_frame = self.sample_rate.as_f32() / self.tick_rate.max(1.0);
        // Cache fadeout per sample for linear fadeout
        let fade_per_tick = self.fadeout_rate.to_linear_fade_per_tick();
        self.fadeout_per_sample = fade_per_tick / self.samples_per_frame.max(1.0);
    }

    /// Calculate velocity scale factoring in sensitivity.
    /// At sensitivity=0: always 1.0; at sensitivity=1: use raw velocity.
    #[inline]
    fn velocity_scale(&self) -> f32 {
        1.0 - self.velocity_sensitivity.as_f32() * (1.0 - self.velocity.as_f32())
    }

    /// Apply output mode transformation (unipolar or bipolar).
    #[inline]
    fn apply_output_mode(&self, value: f32) -> f32 {
        if self.output_bipolar {
            value * 2.0 - 1.0
        } else {
            value
        }
    }

    /// Process a single sample.
    #[allow(clippy::too_many_lines)]
    #[inline]
    fn process_sample(&mut self) -> f32 {
        match self.stage {
            MultiPointStage::Idle => 0.0,

            MultiPointStage::Playing => {
                // Advance frame position
                self.current_frame += 1.0 / self.samples_per_frame;

                // Check sustain point (only while NOT released)
                if !self.released
                    && let Some(sustain_idx) = self.sustain_point
                    && let Some(sustain_pt) = self.points.get(sustain_idx.as_usize())
                {
                    let sustain_frame = sustain_pt.frame.as_u16() as f32;
                    if self.current_frame >= sustain_frame {
                        self.current_frame = sustain_frame;
                        self.stage = MultiPointStage::Sustaining;
                        let value = self.interpolate_at_frame(self.current_frame);
                        let out = value.as_f32() * self.velocity_scale();
                        return self.apply_output_mode(out);
                    }
                }

                // Check loop region
                // FT2 interaction: if sustain_point == loop_end AND released, do NOT loop
                if let (Some(start_idx), Some(end_idx)) = (self.loop_start, self.loop_end)
                    && let Some(end_pt) = self.points.get(end_idx.as_usize())
                {
                    let end_frame = end_pt.frame.as_u16() as f32;
                    if self.current_frame >= end_frame {
                        // FT2: if sustain == loop_end and note is released, skip loop
                        let sustain_is_loop_end = self.sustain_point == Some(end_idx);
                        if !(sustain_is_loop_end && self.released)
                            && let Some(start_pt) = self.points.get(start_idx.as_usize())
                        {
                            self.current_frame = start_pt.frame.as_u16() as f32;
                        }
                    }
                }

                // Get interpolated value at current position
                let value = self.interpolate_at_frame(self.current_frame);

                // Check if envelope has completed (past last point)
                // Key held: clamp to last frame (implicit sustain at end)
                // Released: let it stay past end, fadeout brings to silence
                if !self.released
                    && let Some(last) = self.points.last()
                    && self.current_frame >= last.frame.as_u16() as f32
                {
                    self.current_frame = last.frame.as_u16() as f32;
                }

                // Apply linear fadeout in parallel if released
                if self.released {
                    self.fadeout_level = (self.fadeout_level - self.fadeout_per_sample).max(0.0);
                    if self.fadeout_level <= 0.0 {
                        self.stage = MultiPointStage::Idle;
                        return 0.0;
                    }
                }

                let scale = if self.released {
                    self.fadeout_level * self.velocity_scale()
                } else {
                    self.velocity_scale()
                };

                self.apply_output_mode(value.as_f32() * scale)
            }

            MultiPointStage::Sustaining => {
                // Hold at sustain point value
                let value = self.interpolate_at_frame(self.current_frame);
                let out = value.as_f32() * self.velocity_scale();
                self.apply_output_mode(out)
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
            .description("Multi-point envelope with sustain and loop")
            .category(ModuleCategory::Envelope)
            .tag("envelope")
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
        // Update tick_rate from current tempo (dynamic BPM support)
        self.tick_rate = (context.tempo.as_f32() * 2.0 / 5.0).max(1.0);
        self.update_timing();
        self.output_buffer.resize(context.samples.as_usize());

        let gate_input = inputs.get(PortName::GATE);
        let velocity_input = inputs.get(PortName::VELOCITY);

        for i in 0..context.samples.as_usize() {
            if let Some(gate) = gate_input {
                let gate_val = gate[i];
                if gate_val > 0.5 && self.prev_gate <= 0.5 {
                    let vel = Velocity::new(velocity_input.map(|v| v[i]).unwrap_or(1.0));
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
            self.velocity_sensitivity = v;
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Envelope(synth_core::EnvelopeParam::VelocitySensitivity(_)) = param {
            return Some(self.velocity_sensitivity.as_f32());
        }
        None
    }

    fn get_params(&self) -> Vec<Param> {
        vec![Param::Envelope(
            synth_core::EnvelopeParam::VelocitySensitivity(self.velocity_sensitivity),
        )]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::MultiPointEnvelope
    }

    fn reset(&mut self) {
        self.stage = MultiPointStage::Idle;
        self.current_frame = 0.0;
        self.fadeout_level = 1.0;
        self.released = false;
        self.prev_gate = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        self.trigger(velocity);
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
        env.trigger(Velocity::MAX);
        assert_eq!(env.stage(), MultiPointStage::Playing);
        assert!(!env.released);
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
        env.trigger(Velocity::MAX);

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
        env.trigger(Velocity::MAX);

        // Process to sustain
        for _ in 0..50000 {
            env.process_sample();
        }

        env.release();
        // After release from sustain, stage should be Playing (continues past sustain)
        assert_eq!(env.stage(), MultiPointStage::Playing);
        assert!(env.released);
    }

    #[test]
    fn test_parallel_fadeout() {
        // Verify fadeout runs in parallel with envelope after release
        let mut env = MultiPointEnvelope::with_points(&[(0, 1.0), (100, 1.0)]);
        env.set_fadeout_rate(FadeoutRate::FAST); // 4096 => 0.125 per tick => 8 ticks to silence
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(Velocity::MAX);

        // Process a few samples to start
        for _ in 0..100 {
            env.process_sample();
        }

        // Release while still in Playing stage
        env.release();
        assert!(env.released);
        assert_eq!(env.stage(), MultiPointStage::Playing);

        // Fadeout should bring level to 0 within ~8 ticks = 8*960 = 7680 samples
        let mut last_val = 1.0_f32;
        for _ in 0..8000 {
            let val = env.process_sample();
            // Value should be decreasing
            assert!(val <= last_val + 0.001); // Allow tiny float error
            last_val = val;
        }

        // Should be at or near idle after 8 ticks of fadeout
        assert!(env.fadeout_level < 0.01);
    }

    #[test]
    fn test_linear_fadeout_timing() {
        // FadeoutRate(4096) at 50Hz tick rate should silence in 8 ticks
        // 8 ticks = 8 * (48000/50) = 8 * 960 = 7680 samples
        let mut env = MultiPointEnvelope::with_points(&[(0, 1.0), (1000, 1.0)]);
        env.set_fadeout_rate(FadeoutRate::FAST);
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(Velocity::MAX);

        // Process 1 tick worth of samples
        for _ in 0..960 {
            env.process_sample();
        }

        // Release
        env.release();

        // After 1 tick of fadeout (960 samples), level should be ~0.875
        for _ in 0..960 {
            env.process_sample();
        }
        assert!((env.fadeout_level - 0.875).abs() < 0.01);

        // After 7 more ticks (6720 samples), should reach ~0
        for _ in 0..6720 {
            env.process_sample();
        }
        assert!(env.fadeout_level < 0.01);
    }

    #[test]
    fn test_sustain_loop_interaction() {
        // FT2: when sustain_point == loop_end and note is released, loop stops
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.0), (10, 1.0), (20, 0.8), (30, 0.0)]);
        env.set_sustain_point(Some(2)); // Sustain at point 2 (frame 20)
        env.set_loop(Some(1), Some(2)); // Loop between points 1 and 2 (frames 10-20)
        // sustain_point (2) == loop_end (2) — so after release, loop should stop
        env.set_fadeout_rate(FadeoutRate::NONE);
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(Velocity::MAX);

        // Process to sustain
        for _ in 0..50000 {
            env.process_sample();
        }
        assert_eq!(env.stage(), MultiPointStage::Sustaining);

        // Release
        env.release();
        assert!(env.released);

        // Process past the old loop end — envelope should continue to point 3 (frame 30, value 0.0)
        // rather than looping back to point 1
        for _ in 0..50000 {
            env.process_sample();
        }

        // Should have reached end of envelope (frame 30, value 0.0)
        let value = env.interpolate_at_frame(env.current_frame);
        assert!(
            value.as_f32() < 0.1,
            "Envelope should have passed loop end to reach final value"
        );
    }

    #[test]
    fn test_bipolar_output() {
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.5), (10, 0.5)]);
        env.set_output_bipolar(true);
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(Velocity::MAX);

        // Value 0.5 in bipolar mode should be 0.0 (center)
        let val = env.process_sample();
        assert!(
            val.abs() < 0.01,
            "0.5 in bipolar should map to ~0.0, got {val}"
        );
    }

    #[test]
    fn test_release_from_playing() {
        // Release while playing (before reaching sustain) should start fadeout in parallel
        let mut env = MultiPointEnvelope::with_points(&[(0, 0.0), (100, 1.0), (200, 0.0)]);
        env.set_sustain_point(Some(1)); // Sustain at point 1 (frame 100)
        env.set_fadeout_rate(FadeoutRate::FAST);
        env.sample_rate = SampleRate::new(48000.0);
        env.tick_rate = 50.0;
        env.update_timing();
        env.trigger(Velocity::MAX);

        // Process a bit but NOT enough to reach sustain
        for _ in 0..100 {
            env.process_sample();
        }
        assert_eq!(env.stage(), MultiPointStage::Playing);

        // Release while still playing
        env.release();
        assert!(env.released);
        assert_eq!(env.stage(), MultiPointStage::Playing); // Still playing, fadeout in parallel
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
