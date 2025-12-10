//! Stereo Output Module - Final output destination for the module graph.
//!
//! The StereoOutput module serves as the final stage in the voice signal chain.
//! It collects audio from connected modules and provides the final stereo
//! output with master volume, pan, and optional limiting.
//!
//! The processed audio is available via the "left" and "right" output ports
//! for the Voice to read, as well as via the `get_output()` method.

use std::collections::HashMap;

use crate::engine::typed_params::{AmplifierParam, MixerParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{
    Amplitude, BipolarValue, Decibels, Gain, LimitMode, MidiNote, MuteState, PortName, StereoSample,
};

/// Stereo output module - the final destination in the audio graph.
///
/// Features:
/// - Stereo input (left/right or mono summed to stereo)
/// - Master volume control
/// - Pan control
/// - Soft limiting to prevent clipping
/// - Peak/RMS metering (accessible via getters)
#[derive(Debug, Clone)]
pub struct StereoOutput {
    /// Master volume (0.0 - 1.0)
    master_level: Gain,
    /// Master pan (-1.0 = left, 0.0 = center, 1.0 = right)
    pan: BipolarValue,
    /// Limiter mode
    limit_mode: LimitMode,
    /// Limiter threshold in dB (typically -0.1 to -3.0)
    limit_threshold: Decibels,
    /// Current peak level (left)
    peak_l: Amplitude,
    /// Current peak level (right)
    peak_r: Amplitude,
    /// Output buffer (interleaved stereo)
    output_buffer: Vec<f32>,
    /// Mute state
    mute_state: MuteState,
}

impl StereoOutput {
    /// Create a new stereo output with default settings.
    pub fn new() -> Self {
        Self {
            master_level: Gain::new(0.8),
            pan: BipolarValue::CENTER,
            limit_mode: LimitMode::Enabled,
            limit_threshold: Decibels::new(-0.3),
            peak_l: Amplitude::ZERO,
            peak_r: Amplitude::ZERO,
            output_buffer: Vec::new(),
            mute_state: MuteState::Unmuted,
        }
    }

    /// Get the processed stereo output buffer (interleaved L, R, L, R...).
    pub fn get_output(&self) -> &[f32] {
        &self.output_buffer
    }

    /// Get the current peak levels (left, right).
    pub fn get_peak_levels(&self) -> (Amplitude, Amplitude) {
        (self.peak_l, self.peak_r)
    }

    /// Get the master level.
    pub fn get_master_level(&self) -> Gain {
        self.master_level
    }

    /// Set the master level.
    pub fn set_master_level(&mut self, level: f32) {
        self.master_level = Gain::new(level.clamp(0.0, 1.0));
    }

    /// Get mute state.
    pub fn is_muted(&self) -> bool {
        self.mute_state.is_muted()
    }

    /// Set mute state.
    pub fn set_muted(&mut self, muted: bool) {
        self.mute_state = MuteState::from(muted);
    }

    /// Apply soft limiting to a single channel.
    fn soft_limit_channel(&self, sample: f32) -> f32 {
        if !self.limit_mode.is_enabled() {
            return sample.clamp(-1.0, 1.0);
        }

        // Convert threshold from dB to linear
        let threshold = self.limit_threshold.to_linear();

        // Soft knee limiting using tanh
        if sample.abs() > threshold {
            let sign = sample.signum();
            let excess = sample.abs() - threshold;
            let limited = threshold + (1.0 - threshold) * (excess / (1.0 + excess)).tanh();
            sign * limited
        } else {
            sample
        }
    }

    /// Apply soft limiting to a stereo sample.
    #[inline]
    fn soft_limit_stereo(&self, sample: StereoSample) -> StereoSample {
        StereoSample::new(
            self.soft_limit_channel(sample.left),
            self.soft_limit_channel(sample.right),
        )
    }

    /// Calculate pan coefficients (equal power panning).
    fn pan_coefficients(&self) -> (Gain, Gain) {
        Gain::from_pan(self.pan)
    }
}

impl Default for StereoOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for StereoOutput {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("stereo_output", "Stereo Output")
            .description("Master stereo output with volume, pan, and limiting")
            .category(ModuleCategory::Output)
            // Inputs - accepts both mono and stereo
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Mono audio input (summed to stereo)"),
            )
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left channel input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right channel input"))
            // Output ports for Voice to read processed audio
            .port(
                PortDescriptor::audio_output("left", "Left Out")
                    .description("Processed left channel output"),
            )
            .port(
                PortDescriptor::audio_output("right", "Right Out")
                    .description("Processed right channel output"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Mono mix output (for graph compatibility)"),
            )
            // Parameters
            .parameter(
                ParameterDescriptor::float(
                    Param::Mixer(MixerParam::Master(Gain::new(0.8))),
                    "Master Level",
                )
                .description("Master output volume")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Slider)
                .curve(ResponseCurve::Squared),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Amplifier(AmplifierParam::Pan(BipolarValue::CENTER)),
                    "Pan",
                )
                .description("Master stereo pan")
                .range(-1.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::PanKnob),
            )
            .parameter(
                ParameterDescriptor::float(Param::Mixer(MixerParam::Limit(true)), "Limiter")
                    .description("Enable soft limiter to prevent clipping")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(Param::Mixer(MixerParam::Mute(false)), "Mute")
                    .description("Mute output")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Toggle),
            )
            .tag("output")
            .tag("master")
            .tag("stereo")
    }
}

impl PolyModule for StereoOutput {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        // Resize output buffer if needed (interleaved stereo)
        let stereo_samples = context.samples.as_usize() * 2;
        if self.output_buffer.len() != stereo_samples {
            self.output_buffer.resize(stereo_samples, 0.0);
        }

        // Get input buffers
        let mono_in = inputs.get(PortName::IN);
        let left_in = inputs.get(PortName::IN_L);
        let right_in = inputs.get(PortName::IN_R);

        // Calculate pan coefficients
        let (pan_l, pan_r) = self.pan_coefficients();

        // Reset peaks
        let mut peak_l = Amplitude::ZERO;
        let mut peak_r = Amplitude::ZERO;

        // Process each sample
        for i in 0..context.samples.as_usize() {
            // Get input samples - handle partial connections
            let input = match (left_in, right_in, mono_in) {
                // Full stereo input
                (Some(l), Some(r), _) => StereoSample::new(l[i], r[i]),
                // Only left input - duplicate to both channels
                (Some(l), None, _) => StereoSample::from_mono(l[i]),
                // Only right input - duplicate to both channels
                (None, Some(r), _) => StereoSample::from_mono(r[i]),
                // Mono input - duplicate to both channels
                (None, None, Some(m)) => StereoSample::from_mono(m[i]),
                // No input - silence
                (None, None, None) => StereoSample::ZERO,
            };

            // Process stereo sample
            let processed = if self.mute_state.is_muted() {
                StereoSample::ZERO
            } else {
                // Apply master level with pan
                let gained = input.apply_stereo_gain(
                    self.master_level.as_f32() * pan_l.as_f32(),
                    self.master_level.as_f32() * pan_r.as_f32(),
                );
                // Apply soft limiting
                self.soft_limit_stereo(gained)
            };

            // Store in interleaved output buffer
            self.output_buffer[i * 2] = processed.left;
            self.output_buffer[i * 2 + 1] = processed.right;

            // Track peaks
            peak_l.update_peak(processed.left);
            peak_r.update_peak(processed.right);
        }

        // Update peak meters with decay
        const DECAY: f32 = 0.95;
        self.peak_l.decay(DECAY);
        self.peak_l = Amplitude::new(self.peak_l.as_f32().max(peak_l.as_f32()));
        self.peak_r.decay(DECAY);
        self.peak_r = Amplitude::new(self.peak_r.as_f32().max(peak_r.as_f32()));

        // Write to stereo output ports for Voice to read
        if let Some(left_out) = outputs.get_mut("left") {
            for i in 0..context.samples.as_usize().min(left_out.len()) {
                left_out[i] = self.output_buffer[i * 2];
            }
        }

        if let Some(right_out) = outputs.get_mut("right") {
            for i in 0..context.samples.as_usize().min(right_out.len()) {
                right_out[i] = self.output_buffer.get(i * 2 + 1).copied().unwrap_or(0.0);
            }
        }

        // Also write to "out" port if present (mono compatibility)
        if let Some(out_buf) = outputs.get_mut("out") {
            for i in 0..context.samples.as_usize().min(out_buf.len()) {
                let stereo = StereoSample::new(
                    self.output_buffer[i * 2],
                    self.output_buffer.get(i * 2 + 1).copied().unwrap_or(0.0),
                );
                out_buf[i] = stereo.to_mono();
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        match param {
            Param::Mixer(MixerParam::Master(g)) => {
                self.master_level = Gain::new(g.as_f32().clamp(0.0, 1.0));
            }
            Param::Mixer(MixerParam::Mute(m)) => {
                self.mute_state = MuteState::from(m);
            }
            Param::Mixer(MixerParam::Limit(l)) => {
                self.limit_mode = LimitMode::from(l);
            }
            Param::Amplifier(AmplifierParam::Pan(p)) => {
                self.pan = p;
            }
            _ => {}
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        match param {
            Param::Mixer(MixerParam::Master(_)) => Some(self.master_level.as_f32()),
            Param::Mixer(MixerParam::Mute(_)) => {
                Some(if self.mute_state.is_muted() { 1.0 } else { 0.0 })
            }
            Param::Mixer(MixerParam::Limit(_)) => Some(if self.limit_mode.is_enabled() {
                1.0
            } else {
                0.0
            }),
            Param::Amplifier(AmplifierParam::Pan(_)) => Some(self.pan.as_f32()),
            _ => None,
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Mixer(MixerParam::Master(self.master_level)),
            Param::Mixer(MixerParam::Mute(self.mute_state.is_muted())),
            Param::Mixer(MixerParam::Limit(self.limit_mode.is_enabled())),
            Param::Amplifier(AmplifierParam::Pan(self.pan)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        // Treat as Mixer type for compatibility (Output category)
        ModuleType::Mixer
    }

    fn reset(&mut self) {
        self.peak_l = Amplitude::ZERO;
        self.peak_r = Amplitude::ZERO;
        self.output_buffer.clear();
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stereo_output_creation() {
        let output = StereoOutput::new();
        assert!((output.master_level.as_f32() - 0.8).abs() < 0.001);
        assert!((output.pan.as_f32() - 0.0).abs() < 0.001);
        assert!(output.limit_mode.is_enabled());
        assert!(!output.mute_state.is_muted());
    }

    #[test]
    fn test_soft_limiting() {
        let output = StereoOutput::new();

        // Values below threshold should pass through
        assert!((output.soft_limit_channel(0.5) - 0.5).abs() < 0.01);

        // Values above 1.0 should be limited
        let limited = output.soft_limit_channel(2.0);
        assert!(limited <= 1.0);
        assert!(limited > 0.9);

        // Test stereo limiting
        let stereo = StereoSample::new(2.0, -2.0);
        let limited_stereo = output.soft_limit_stereo(stereo);
        assert!(limited_stereo.left <= 1.0 && limited_stereo.left > 0.9);
        assert!(limited_stereo.right >= -1.0 && limited_stereo.right < -0.9);
    }

    #[test]
    fn test_pan_coefficients() {
        let mut output = StereoOutput::new();

        // Center pan should be equal
        output.pan = BipolarValue::CENTER;
        let (l, r) = output.pan_coefficients();
        assert!((l.as_f32() - r.as_f32()).abs() < 0.01);

        // Full left should have higher left coefficient
        output.pan = BipolarValue::MIN;
        let (l, r) = output.pan_coefficients();
        assert!(l.as_f32() > r.as_f32());

        // Full right should have higher right coefficient
        output.pan = BipolarValue::MAX;
        let (l, r) = output.pan_coefficients();
        assert!(r.as_f32() > l.as_f32());
    }

    #[test]
    fn test_mute() {
        use crate::types::PortName;

        let mut output = StereoOutput::new();
        let context = ProcessContext::default();

        // Create test input
        let mut test_buf = AudioBuffer::new(context.samples.as_usize());
        for i in 0..context.samples.as_usize() {
            test_buf[i] = 0.5;
        }

        let input_slice: [(PortName, &AudioBuffer); 1] = [(PortName::intern("in"), &test_buf)];
        let inputs = InputPorts::new(&input_slice);
        let mut outputs = HashMap::new();

        // Process unmuted
        output.process(inputs, &mut outputs, &context);
        let unmuted_output = output.get_output().to_vec();

        // Process muted
        output.set_muted(true);
        let inputs = InputPorts::new(&input_slice);
        output.process(inputs, &mut outputs, &context);
        let muted_output = output.get_output();

        // Muted should be silence
        assert!(muted_output.iter().all(|&s| s == 0.0));
        // Unmuted should have signal
        assert!(unmuted_output.iter().any(|&s| s != 0.0));
    }
}
