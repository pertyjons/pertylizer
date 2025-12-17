//! IT-compatible resonant lowpass filter with Zxx control.
//!
//! This filter is specifically designed for XM/IT tracker playback compatibility.
//! It uses the tracker-native 0-127 cutoff and resonance values from Zxx/Z8x effects.
//!
//! The cutoff mapping follows IT's exponential curve:
//! - Z00 (0): ~130 Hz
//! - Z7F (127): ~8 kHz
//!
//! Resonance mapping:
//! - Z80 (0 resonance): Q = 0.5 (no resonance)
//! - ZFF (127 resonance): Q = 12 (high resonance)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, FilterState, Hertz, InputPorts, MidiNote, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    PolyModule, PortDescriptor, PortName, ProcessContext, SampleRate, TrackerCutoff,
    TrackerResonance, Velocity, WidgetHint,
};

/// IT-style resonant lowpass filter.
///
/// This module implements the IT resonant filter using native tracker values
/// (0-127 for both cutoff and resonance). It's designed for accurate playback
/// of XM/IT modules that use Zxx filter effects.
///
/// # Zxx Effects
///
/// In IT format:
/// - Z00-Z7F: Set cutoff (0 = closed, 127 = open)
/// - Z80-ZFF: Set resonance (0 = no resonance, 127 = high resonance)
///
/// # Filter Characteristics
///
/// Uses a 2-pole state variable filter (SVF) in lowpass mode.
/// The cutoff frequency follows an exponential curve matching IT's behavior.
#[derive(Clone)]
pub struct TrackerFilter {
    // Parameters (tracker native values)
    cutoff: TrackerCutoff,
    resonance: TrackerResonance,

    // SVF state (type-safe DSP state)
    sample_rate: SampleRate,
    ic1eq: FilterState,
    ic2eq: FilterState,

    // Output
    output_buffer: AudioBuffer,
}

impl TrackerFilter {
    /// Create a new tracker filter with default (open) settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cutoff: TrackerCutoff::OPEN,
            resonance: TrackerResonance::DEFAULT,
            sample_rate: SampleRate::DVD_QUALITY,
            ic1eq: FilterState::ZERO,
            ic2eq: FilterState::ZERO,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Get the current cutoff value (0-127).
    #[must_use]
    pub fn cutoff(&self) -> TrackerCutoff {
        self.cutoff
    }

    /// Get the current resonance value (0-127).
    #[must_use]
    pub fn resonance(&self) -> TrackerResonance {
        self.resonance
    }

    /// Set cutoff from Zxx effect (Z00-Z7F).
    ///
    /// The value should be in the range 0-127. Values above 127 are clamped.
    pub fn set_cutoff_zxx(&mut self, value: u8) {
        self.cutoff = TrackerCutoff::from_zxx(value);
    }

    /// Set resonance from Z8x effect (Z80-ZFF).
    ///
    /// The value should be in the range 0-127 (extracted from Z80-ZFF).
    pub fn set_resonance(&mut self, value: u8) {
        self.resonance = TrackerResonance::new(value);
    }

    /// Set both cutoff and resonance from a single Zxx byte.
    ///
    /// This handles the IT convention:
    /// - Z00-Z7F: Set cutoff
    /// - Z80-ZFF: Set resonance (lower 7 bits)
    pub fn apply_zxx_effect(&mut self, value: u8) {
        if value < 0x80 {
            // Z00-Z7F: Set cutoff
            self.cutoff = TrackerCutoff::from_zxx(value);
        } else {
            // Z80-ZFF: Set resonance
            self.resonance = TrackerResonance::new(value & 0x7F);
        }
    }

    /// Process a single sample through the filter.
    #[inline]
    fn process_sample(&mut self, input: f32, cutoff_mod: f32) -> f32 {
        // Apply CV modulation (0-1 range maps to 0-127 offset)
        let mod_cutoff = (self.cutoff.as_u8() as f32 + cutoff_mod * 127.0).clamp(0.0, 127.0) as u8;
        let cutoff_hz = TrackerCutoff::new(mod_cutoff).to_hertz();

        // Clamp to Nyquist
        let cutoff = Hertz::new(cutoff_hz.as_f32().clamp(20.0, self.sample_rate.as_f32() * 0.49));

        // SVF coefficients
        let g = cutoff.to_tan_coeff(self.sample_rate);
        let q = self.resonance.to_q();
        let k = 1.0 / q;

        // SVF calculation (Chamberlin form)
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let ic1 = self.ic1eq.as_f32();
        let ic2 = self.ic2eq.as_f32();

        let v3 = input - ic2;
        let v1 = a1 * ic1 + a2 * v3;
        let v2 = ic2 + a2 * ic1 + a3 * v3;

        self.ic1eq = FilterState::new(2.0 * v1 - ic1);
        self.ic2eq = FilterState::new(2.0 * v2 - ic2);

        v2 // Lowpass output
    }
}

impl Default for TrackerFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for TrackerFilter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("tracker_filter", "Tracker Filter")
            .description("IT-compatible resonant lowpass filter with Zxx control")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("tracker")
            .tag("it")
            .tag("xm")
            .parameter(
                ParameterDescriptor::float(
                    // Use existing FilterParam for compatibility
                    Param::Filter(synth_core::FilterParam::Cutoff(Hertz::new(8000.0))),
                    "Cutoff",
                )
                .description("Filter cutoff (Z00-Z7F)")
                .range(0.0, 127.0)
                .default(127.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(synth_core::FilterParam::Resonance(NormalizedValue::MIN)),
                    "Resonance",
                )
                .description("Filter resonance (Z80-ZFF)")
                .range(0.0, 127.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Slider),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(
                PortDescriptor::control_input("cutoff_cv", "Cutoff CV")
                    .description("Cutoff modulation (0-1)"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl PolyModule for TrackerFilter {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.get(PortName::IN);
        let cutoff_cv = inputs.get(PortName::CUTOFF_CV);

        for i in 0..context.samples.as_usize() {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            let cv = cutoff_cv.map(|b| b[i]).unwrap_or(0.0);
            self.output_buffer[i] = self.process_sample(input, cv);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Filter(filter_param) = param {
            match filter_param {
                synth_core::FilterParam::Cutoff(hz) => {
                    // Map Hertz value to tracker range
                    // This is a rough approximation for GUI compatibility
                    let tracker_val = (hz.as_f32() / 8000.0 * 127.0).clamp(0.0, 127.0) as u8;
                    self.cutoff = TrackerCutoff::new(tracker_val);
                }
                synth_core::FilterParam::Resonance(r) => {
                    let tracker_val = (r.as_f32() * 127.0).clamp(0.0, 127.0) as u8;
                    self.resonance = TrackerResonance::new(tracker_val);
                }
                _ => {}
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Filter(filter_param) = param {
            Some(match filter_param {
                synth_core::FilterParam::Cutoff(_) => self.cutoff.as_u8() as f32,
                synth_core::FilterParam::Resonance(_) => self.resonance.as_u8() as f32,
                _ => return None,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Filter(synth_core::FilterParam::Cutoff(self.cutoff.to_hertz())),
            Param::Filter(synth_core::FilterParam::Resonance(NormalizedValue::new(
                self.resonance.as_u8() as f32 / 127.0,
            ))),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::TrackerFilter
    }

    fn reset(&mut self) {
        self.ic1eq = FilterState::ZERO;
        self.ic2eq = FilterState::ZERO;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

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

    #[test]
    fn test_tracker_filter_creation() {
        let filter = TrackerFilter::new();
        assert_eq!(filter.cutoff(), TrackerCutoff::OPEN);
        assert_eq!(filter.resonance(), TrackerResonance::DEFAULT);
    }

    #[test]
    fn test_set_cutoff_zxx() {
        let mut filter = TrackerFilter::new();
        filter.set_cutoff_zxx(64);
        assert_eq!(filter.cutoff().as_u8(), 64);
    }

    #[test]
    fn test_set_resonance() {
        let mut filter = TrackerFilter::new();
        filter.set_resonance(100);
        assert_eq!(filter.resonance().as_u8(), 100);
    }

    #[test]
    fn test_apply_zxx_cutoff() {
        let mut filter = TrackerFilter::new();
        filter.apply_zxx_effect(0x40); // Z40 = cutoff 64
        assert_eq!(filter.cutoff().as_u8(), 64);
    }

    #[test]
    fn test_apply_zxx_resonance() {
        let mut filter = TrackerFilter::new();
        filter.apply_zxx_effect(0xC0); // ZC0 = resonance 64
        assert_eq!(filter.resonance().as_u8(), 64);
    }

    #[test]
    fn test_filter_stability() {
        let mut filter = TrackerFilter::new();
        filter.sample_rate = SampleRate::DVD_QUALITY;
        filter.set_cutoff_zxx(32); // Low cutoff
        filter.set_resonance(120); // High resonance

        for _ in 0..1000 {
            let out = filter.process_sample(0.5, 0.0);
            assert!(out.is_finite(), "Filter output is not finite");
            assert!(out.abs() < 100.0, "Filter output exploded");
        }
    }

    #[test]
    fn test_cutoff_range() {
        let filter = TrackerFilter::new();

        // Verify cutoff frequency mapping
        // Formula: 110 Hz * 2^(cutoff * 6.5 / 127)
        // MIN (0): 110 Hz
        // MAX (127): 110 * 2^6.5 ≈ 9955 Hz
        let min_hz = TrackerCutoff::MIN.to_hertz().as_f32();
        let max_hz = TrackerCutoff::MAX.to_hertz().as_f32();

        assert!(min_hz > 100.0 && min_hz < 200.0, "Min cutoff ~110 Hz");
        assert!(
            max_hz > 7000.0 && max_hz < 10500.0,
            "Max cutoff ~10 kHz, got {max_hz}"
        );

        // Verify default is fully open
        assert_eq!(filter.cutoff(), TrackerCutoff::OPEN);
    }

    #[test]
    fn test_resonance_q_range() {
        // Verify Q factor mapping
        let min_q = TrackerResonance::MIN.to_q();
        let max_q = TrackerResonance::MAX.to_q();

        assert!(
            min_q > 0.4 && min_q < 0.6,
            "Min resonance Q should be ~0.5"
        );
        assert!(max_q > 11.0 && max_q < 13.0, "Max resonance Q should be ~12");
    }
}
