//! Stereo Output Module - Final output destination for the module graph.
//!
//! The StereoOutput module serves as the final stage in the voice signal chain.
//! It collects audio from connected modules and provides the final stereo
//! output with master volume, pan, and optional limiting.
//!
//! The processed audio is available via the "out_l" and "out_r" output ports
//! for the Voice to read, as well as via the `get_output()` method.

use std::collections::HashMap;

use synth_core::{AmplifierParam, MixerParam, ModuleType, Param};
use synth_core::{
    Amplitude, BipolarValue, Decibels, Gain, LimitMode, MidiNote, MuteState, PortName,
    StereoSample, Velocity,
};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
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
    /// Whether noise-shaped dithering is enabled
    dither_enabled: bool,
    /// Dither error feedback state for left channel (internal filter state, raw f32 OK)
    dither_error_l: f32,
    /// Dither error feedback state for right channel (internal filter state, raw f32 OK)
    dither_error_r: f32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
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
            output_buffer: vec![0.0; 1024 * 2],
            mute_state: MuteState::Unmuted,
            dither_enabled: false,
            dither_error_l: 0.0,
            dither_error_r: 0.0,
            mod_offsets: ParamModOffsets::new(),
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
        let threshold = self.limit_threshold.to_linear();
        crate::math::soft_knee_limit(sample, threshold)
    }

    /// Apply soft limiting to a stereo sample.
    #[inline]
    fn soft_limit_stereo(&self, sample: StereoSample) -> StereoSample {
        StereoSample::new(
            self.soft_limit_channel(sample.left),
            self.soft_limit_channel(sample.right),
        )
    }

    /// Calculate pan coefficients (equal power panning) for a given pan position.
    fn pan_coefficients(pan: BipolarValue) -> (Gain, Gain) {
        Gain::from_pan(pan)
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
            .width(synth_core::ModuleWidth::Large)
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
            // Canonical audio-port order: mono `out` first, then the stereo pair
            // (matches `in, in_l, in_r` and osc/vox/fof). Names are by-value so the
            // order is purely the visual column order.
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Mono mix output (for graph compatibility)"),
            )
            .port(
                PortDescriptor::audio_output("out_l", "Out L")
                    .description("Processed left channel output"),
            )
            .port(
                PortDescriptor::audio_output("out_r", "Out R")
                    .description("Processed right channel output"),
            )
            // Parameters
            .parameter(
                ParameterDescriptor::float(
                    "master",
                    Param::Mixer(MixerParam::Master(Gain::new(0.8))),
                    "Master",
                )
                .description("Master output volume")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Slider)
                .curve(ResponseCurve::Squared),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pan",
                    Param::Amplifier(AmplifierParam::Pan(BipolarValue::CENTER)),
                    "Pan",
                )
                .description("Master stereo pan")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::PanKnob),
            )
            .parameter(
                ParameterDescriptor::float("limit", Param::Mixer(MixerParam::Limit(true)), "Limit")
                    .description("Enable soft limiter to prevent clipping")
                    .range(0.0, 1.0)
                    .default(1.0)
                    // Boolean toggle, not a continuous modulation target.
                    .modulatable(false)
                    .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float("mute", Param::Mixer(MixerParam::Mute(false)), "Mute")
                    .description("Mute output")
                    .range(0.0, 1.0)
                    .default(0.0)
                    // Boolean toggle, not a continuous modulation target.
                    .modulatable(false)
                    .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "dither",
                    Param::Mixer(MixerParam::Dither(false)),
                    "Dither",
                )
                .description("Enable 24-bit noise-shaped dithering")
                .range(0.0, 1.0)
                .default(0.0)
                // Boolean toggle, not a continuous modulation target.
                .modulatable(false)
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
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        // Ensure output buffer is large enough (interleaved stereo).
        // Pre-allocated in new(); only re-allocates if block size grows beyond initial capacity.
        let stereo_samples = context.samples.as_usize() * 2;
        if self.output_buffer.len() < stereo_samples {
            self.output_buffer.resize(stereo_samples, 0.0);
        }

        // Get input buffers
        let mono_in = inputs.reader(PortName::IN, 0.0);
        let left_in = inputs.reader(PortName::IN_L, 0.0);
        let right_in = inputs.reader(PortName::IN_R, 0.0);

        // Generic mod offsets — per-block constants (whole-mix tremolo / auto-pan).
        let eff_master = self
            .mod_offsets
            .effective("master", self.master_level.as_f32());
        let eff_pan = self.mod_offsets.effective("pan", self.pan.as_f32());

        // Calculate pan coefficients from the effective (mod-applied) pan.
        let (pan_l, pan_r) = Self::pan_coefficients(BipolarValue::new(eff_pan));

        // Reset peaks
        let mut peak_l = Amplitude::ZERO;
        let mut peak_r = Amplitude::ZERO;

        // Determine connection topology once before the loop
        let has_left = left_in.is_connected();
        let has_right = right_in.is_connected();
        let has_mono = mono_in.is_connected();

        // Process each sample
        for i in 0..context.samples.as_usize() {
            // Get input samples - handle partial connections
            let input = match (has_left, has_right, has_mono) {
                // Full stereo input
                (true, true, _) => StereoSample::new(left_in[i], right_in[i]),
                // Only left input - duplicate to both channels
                (true, false, _) => StereoSample::from_mono(left_in[i]),
                // Only right input - duplicate to both channels
                (false, true, _) => StereoSample::from_mono(right_in[i]),
                // Mono input - duplicate to both channels
                (false, false, true) => StereoSample::from_mono(mono_in[i]),
                // No input - silence
                (false, false, false) => StereoSample::ZERO,
            };

            // Process stereo sample
            let processed = if self.mute_state.is_muted() {
                StereoSample::ZERO
            } else {
                // Apply master level with pan
                let gained = input.apply_stereo_gain(
                    Gain::new(eff_master * pan_l.as_f32()),
                    Gain::new(eff_master * pan_r.as_f32()),
                );
                // Apply soft limiting
                self.soft_limit_stereo(gained)
            };

            // Apply noise-shaped dither for 24-bit output if enabled
            let final_sample = if self.dither_enabled {
                StereoSample::new(
                    crate::math::noise_shaped_dither(processed.left, 24, &mut self.dither_error_l),
                    crate::math::noise_shaped_dither(processed.right, 24, &mut self.dither_error_r),
                )
            } else {
                processed
            };

            // Store in interleaved output buffer
            self.output_buffer[i * 2] = final_sample.left;
            self.output_buffer[i * 2 + 1] = final_sample.right;

            // Track peaks
            peak_l.update_peak(final_sample.left);
            peak_r.update_peak(final_sample.right);
        }

        // Update peak meters with decay
        const DECAY: f32 = 0.95;
        self.peak_l.decay(DECAY);
        self.peak_l = Amplitude::new(self.peak_l.as_f32().max(peak_l.as_f32()));
        self.peak_r.decay(DECAY);
        self.peak_r = Amplitude::new(self.peak_r.as_f32().max(peak_r.as_f32()));

        // Write to stereo output ports for Voice to read
        if let Some(left_out) = outputs.get_mut(&PortName::OUT_L) {
            for i in 0..context.samples.as_usize().min(left_out.len()) {
                left_out[i] = self.output_buffer[i * 2];
            }
        }

        if let Some(right_out) = outputs.get_mut(&PortName::OUT_R) {
            for i in 0..context.samples.as_usize().min(right_out.len()) {
                right_out[i] = self.output_buffer.get(i * 2 + 1).copied().unwrap_or(0.0);
            }
        }

        // Also write to "out" port if present (mono compatibility)
        if let Some(out_buf) = outputs.get_mut(&PortName::OUT) {
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
            Param::Mixer(MixerParam::Dither(d)) => {
                self.dither_enabled = d;
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
            Param::Mixer(MixerParam::Dither(_)) => {
                Some(if self.dither_enabled { 1.0 } else { 0.0 })
            }
            Param::Amplifier(AmplifierParam::Pan(_)) => Some(self.pan.as_f32()),
            _ => None,
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Mixer(MixerParam::Master(self.master_level)),
            Param::Mixer(MixerParam::Mute(self.mute_state.is_muted())),
            Param::Mixer(MixerParam::Limit(self.limit_mode.is_enabled())),
            Param::Mixer(MixerParam::Dither(self.dither_enabled)),
            Param::Amplifier(AmplifierParam::Pan(self.pan)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::StereoOutput
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.peak_l = Amplitude::ZERO;
        self.peak_r = Amplitude::ZERO;
        self.output_buffer.clear();
        self.dither_error_l = 0.0;
        self.dither_error_r = 0.0;
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
        let (l, r) = StereoOutput::pan_coefficients(output.pan);
        assert!((l.as_f32() - r.as_f32()).abs() < 0.01);

        // Full left should have higher left coefficient
        output.pan = BipolarValue::MIN;
        let (l, r) = StereoOutput::pan_coefficients(output.pan);
        assert!(l.as_f32() > r.as_f32());

        // Full right should have higher right coefficient
        output.pan = BipolarValue::MAX;
        let (l, r) = StereoOutput::pan_coefficients(output.pan);
        assert!(r.as_f32() > l.as_f32());
    }

    #[test]
    fn test_mute() {
        use synth_core::PortName;

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

    /// `master` is a working mod destination via the generic store (whole-mix
    /// tremolo): driving it to 0 silences the output, and clearing reverts.
    #[test]
    fn master_mod_offset_scales_output() {
        let context = ProcessContext::default();
        let mut test_buf = AudioBuffer::new(context.samples.as_usize());
        for i in 0..context.samples.as_usize() {
            test_buf[i] = 0.5;
        }
        let input_slice: [(PortName, &AudioBuffer); 1] = [(PortName::intern("in"), &test_buf)];

        let render = |offset: f32| -> f32 {
            let mut output = StereoOutput::new();
            let desc = output.descriptor();
            output.mod_offsets_mut().unwrap().populate(&desc);
            if offset != 0.0 {
                output.set_mod_offset("master", offset);
            }
            output.process(InputPorts::new(&input_slice), &mut HashMap::new(), &context);
            output
                .get_output()
                .iter()
                .map(|s| s.abs())
                .fold(0.0_f32, f32::max)
        };

        let base = render(0.0);
        assert!(base > 1e-3, "default master should pass signal, got {base}");

        let silent = render(-1.0); // master → 0
        assert!(
            silent < base * 0.05,
            "master→0 should silence output: {silent}"
        );
    }
}
