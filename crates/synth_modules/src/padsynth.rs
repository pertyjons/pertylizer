//! PADsynth voice module.
//!
//! Generates lush pad sounds using the PADsynth algorithm.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/213-padsynth-synthesys-method.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
//!
//! The PADsynth algorithm builds a wavetable by placing bandwidth-spread
//! Gaussian profiles around each harmonic in the frequency domain, then
//! performing an inverse FFT to produce a single-cycle (or long-period)
//! waveform. During playback the wavetable is read at the rate determined
//! by the current MIDI note.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, PadSynthParam, Param};

/// Wavetable size (must be a power of two for the FFT).
const TABLE_SIZE: usize = 4096;

/// Half the table size — number of positive frequency bins.
const HALF_TABLE: usize = TABLE_SIZE / 2;

/// Maximum number of harmonics to place in the spectrum.
const MAX_HARMONICS: usize = 64;

/// PADsynth voice module.
#[derive(Clone)]
pub struct PadSynth {
    // Parameters
    bandwidth: NormalizedValue,
    tilt: NormalizedValue,
    detune: NormalizedValue,
    base_freq: Hertz,
    level: NormalizedValue,

    // Wavetable (pre-allocated, filled on note_on)
    wavetable: Vec<f32>,

    // Scratch buffers for wavetable generation (avoid allocation in note_on)
    freq_amp: Vec<f32>,

    // Playback state
    phase: f64,
    phase_increment: f64,
    active: bool,
    sample_rate: SampleRate,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl PadSynth {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bandwidth: NormalizedValue::new(0.4),
            tilt: NormalizedValue::new(0.5),
            detune: NormalizedValue::new(0.0),
            base_freq: Hertz::new(261.63), // C4
            level: NormalizedValue::new(0.8),

            wavetable: vec![0.0; TABLE_SIZE],
            freq_amp: vec![0.0; HALF_TABLE],

            phase: 0.0,
            phase_increment: 0.0,
            active: false,
            sample_rate: SampleRate::DVD_QUALITY,

            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Build the wavetable using the PADsynth algorithm.
    ///
    /// 1. Compute harmonic amplitudes with tilt rolloff.
    /// 2. For each harmonic, spread a Gaussian profile in the frequency domain
    ///    whose width is controlled by the bandwidth parameter.
    /// 3. Apply a real-valued inverse FFT to obtain the time-domain wavetable.
    fn build_wavetable(&mut self, note_freq: Hertz) {
        let sr = self.sample_rate.as_f32() as f64;
        let base = note_freq.as_f32() as f64;
        let bw_cents = 25.0 + self.bandwidth.as_f32() as f64 * 175.0; // 25..200 cents
        let tilt = self.tilt.as_f32() as f64;
        let detune_amt = self.detune.as_f32() as f64;

        // Clear frequency amplitude buffer
        for v in &mut self.freq_amp {
            *v = 0.0;
        }

        // Deterministic pseudo-random seed based on base frequency
        let seed = (base * 1000.0) as u32;

        // For each harmonic, spread a Gaussian profile in the frequency bins
        for h in 1..=MAX_HARMONICS {
            let hf = h as f64;

            // Harmonic amplitude with tilt rolloff: amp = 1 / h^tilt
            let amp = 1.0 / hf.powf(tilt);
            if amp < 1e-6 {
                break;
            }

            // Slight per-harmonic detune (deterministic pseudo-random)
            let hash =
                ((h as u32).wrapping_mul(2654435761).wrapping_add(seed)) as f64 / u32::MAX as f64;
            let detune_factor = 1.0 + detune_amt * 0.005 * (hash * 2.0 - 1.0);

            // Center frequency of this harmonic in Hz
            let freq_hz = base * hf * detune_factor;

            // Bandwidth of this harmonic in Hz (PADsynth formula):
            // bw_hz = (2^(bw_cents/1200) - 1) * freq_hz
            let bw_hz = (2.0_f64.powf(bw_cents / 1200.0) - 1.0) * freq_hz;

            // Convert to bin units
            let freq_bin = freq_hz * TABLE_SIZE as f64 / sr;
            let bw_bins = bw_hz * TABLE_SIZE as f64 / sr;

            if bw_bins < 0.01 {
                continue;
            }

            // Spread the Gaussian across nearby bins
            let sigma = bw_bins / (2.0 * std::f64::consts::LN_2).sqrt();
            let inv_2sigma2 = 1.0 / (2.0 * sigma * sigma);

            // Only iterate over bins within ~4 sigma of center
            let spread = (4.0 * sigma).ceil() as i64;
            let center_bin = freq_bin.round() as i64;
            let lo = (center_bin - spread).max(1) as usize;
            let hi = (center_bin + spread).min(HALF_TABLE as i64 - 1) as usize;

            for bin in lo..=hi {
                let dist = bin as f64 - freq_bin;
                let gauss = amp * (-dist * dist * inv_2sigma2).exp();
                self.freq_amp[bin] += gauss as f32;
            }
        }

        // Inverse FFT via direct summation (TABLE_SIZE is small enough).
        // Each freq bin contributes a cosine with a deterministic random phase.
        // This is the "brute force IFFT" approach suitable for our fixed table size.
        let inv_n = 1.0 / TABLE_SIZE as f64;
        let two_pi = std::f64::consts::TAU;

        // Generate random phases per bin (deterministic)
        // We do this inline to avoid extra allocation
        for i in 0..TABLE_SIZE {
            let mut sum = 0.0_f64;
            let t = i as f64 * inv_n;
            for bin in 1..HALF_TABLE {
                let a = self.freq_amp[bin] as f64;
                if a < 1e-8 {
                    continue;
                }
                // Deterministic phase per bin
                let phase_offset = ((bin as u32).wrapping_mul(2654435761).wrapping_add(seed))
                    as f64
                    / u32::MAX as f64
                    * two_pi;
                sum += a * (two_pi * bin as f64 * t + phase_offset).cos();
            }
            self.wavetable[i] = sum as f32;
        }

        // Normalize the wavetable to [-1, 1]
        let mut max_abs = 0.0_f32;
        for sample in &self.wavetable {
            let a = sample.abs();
            if a > max_abs {
                max_abs = a;
            }
        }
        if max_abs > 1e-6 {
            let scale = 1.0 / max_abs;
            for sample in &mut self.wavetable {
                *sample *= scale;
            }
        }
    }

    /// Read from the wavetable with linear interpolation (RT-safe, no allocation).
    #[inline]
    fn read_wavetable(&self, phase: f64) -> f32 {
        let pos = phase * TABLE_SIZE as f64;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let idx0 = idx % TABLE_SIZE;
        let idx1 = (idx + 1) % TABLE_SIZE;
        self.wavetable[idx0] * (1.0 - frac) + self.wavetable[idx1] * frac
    }
}

impl Default for PadSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for PadSynth {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("padsynth", "PADsynth")
            .description("Lush pad synthesis using the PADsynth algorithm")
            .category(ModuleCategory::Oscillator)
            .tag("padsynth")
            .tag("synthesis")
            .tag("pad")
            .parameter(
                ParameterDescriptor::float(
                    "bandwidth",
                    Param::PadSynth(PadSynthParam::Bandwidth(NormalizedValue::new(0.4))),
                    "Bandwidth",
                )
                .description("Harmonic bandwidth spread (wider = lusher)")
                .range(0.0, 1.0)
                .default(0.4)
                .curve(ResponseCurve::Squared)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tilt",
                    Param::PadSynth(PadSynthParam::Tilt(NormalizedValue::new(0.5))),
                    "Tilt",
                )
                .description("Harmonic rolloff per octave")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "detune",
                    Param::PadSynth(PadSynthParam::Detune(NormalizedValue::new(0.0))),
                    "Detune",
                )
                .description("Random per-harmonic frequency shift")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "base_freq",
                    Param::PadSynth(PadSynthParam::BaseFreq(Hertz::new(261.63))),
                    "Base Freq",
                )
                .description("Base frequency for wavetable generation")
                .range(20.0, 2000.0)
                .default(261.63)
                .unit(ParameterUnit::Hertz)
                .curve(ResponseCurve::Exponential)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::PadSynth(PadSynthParam::Level(NormalizedValue::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("PADsynth output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for PadSynth {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        if !self.active {
            for i in 0..num_samples {
                self.output_buffer[i] = 0.0;
            }
            if let Some(out) = outputs.get_mut(&PortName::OUT) {
                out.copy_from(&self.output_buffer);
            }
            return;
        }

        let level = self.level.as_f32();
        let inc = self.phase_increment;

        for i in 0..num_samples {
            let sample = self.read_wavetable(self.phase);
            self.output_buffer[i] = sample * level;
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::PadSynth(p) = param {
            match p {
                PadSynthParam::Bandwidth(v) => self.bandwidth = v,
                PadSynthParam::Tilt(v) => self.tilt = v,
                PadSynthParam::Detune(v) => self.detune = v,
                PadSynthParam::BaseFreq(hz) => self.base_freq = hz,
                PadSynthParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::PadSynth(p) = param {
            Some(match p {
                PadSynthParam::Bandwidth(_) => self.bandwidth.as_f32(),
                PadSynthParam::Tilt(_) => self.tilt.as_f32(),
                PadSynthParam::Detune(_) => self.detune.as_f32(),
                PadSynthParam::BaseFreq(_) => self.base_freq.as_f32(),
                PadSynthParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::PadSynth(PadSynthParam::Bandwidth(self.bandwidth)),
            Param::PadSynth(PadSynthParam::Tilt(self.tilt)),
            Param::PadSynth(PadSynthParam::Detune(self.detune)),
            Param::PadSynth(PadSynthParam::BaseFreq(self.base_freq)),
            Param::PadSynth(PadSynthParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::PadSynth
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.active = false;
        self.wavetable.fill(0.0);
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        let note_freq = note.to_frequency();

        // Build wavetable using the PADsynth algorithm.
        // This uses the base_freq for harmonic placement, while note_freq
        // controls the playback rate. When note_freq == base_freq the table
        // plays back at its natural rate; other notes pitch-shift by adjusting
        // the phase increment.
        self.build_wavetable(self.base_freq);

        // The wavetable encodes one cycle of the base_freq waveform.
        // To play at note_freq, advance phase so one full traversal takes
        // sr/note_freq samples: phase_inc = note_freq / sr.
        let sr = self.sample_rate.as_f32() as f64;
        let note_f = note_freq.as_f32() as f64;
        self.phase_increment = note_f / sr;

        self.phase = 0.0;
        self.active = true;
    }

    fn note_off(&mut self) {
        // Let envelope handle fade-out; keep active for tail
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padsynth_creation() {
        let pad = PadSynth::new();
        assert!((pad.bandwidth.as_f32() - 0.4).abs() < 0.001);
        assert!(!pad.active);
    }

    #[test]
    fn test_padsynth_produces_sound() {
        let mut pad = PadSynth::new();
        pad.note_on(MidiNote::new(60), Velocity::new(0.8));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(256));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        pad.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..256).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "PADsynth should produce sound, max={max}");
    }

    #[test]
    fn test_padsynth_silent_without_note() {
        let mut pad = PadSynth::new();

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        pad.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..64).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max < 0.001,
            "PADsynth should be silent without note, max={max}"
        );
    }

    #[test]
    fn test_padsynth_params() {
        let mut pad = PadSynth::new();
        pad.set_param(Param::PadSynth(PadSynthParam::Bandwidth(
            NormalizedValue::new(0.7),
        )));
        assert!((pad.bandwidth.as_f32() - 0.7).abs() < 0.001);

        pad.set_param(Param::PadSynth(PadSynthParam::Level(NormalizedValue::new(
            0.5,
        ))));
        assert!((pad.level.as_f32() - 0.5).abs() < 0.001);

        let params = pad.get_params();
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn test_padsynth_wavetable_normalized() {
        let mut pad = PadSynth::new();
        pad.note_on(MidiNote::new(60), Velocity::new(0.8));

        let max = pad
            .wavetable
            .iter()
            .map(|s| s.abs())
            .fold(0.0_f32, f32::max);
        assert!(
            (max - 1.0).abs() < 0.01,
            "Wavetable should be normalized to ~1.0, max={max}"
        );
    }
}
