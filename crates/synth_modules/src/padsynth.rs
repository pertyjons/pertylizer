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
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve,
    WidgetHint,
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

    /// Set when a wavetable-affecting input (bandwidth/tilt/detune/base_freq or
    /// sample_rate) changed since the cached `wavetable` was built. note_on only
    /// runs the O(N²) `build_wavetable` when this is set, so a plain note press
    /// at unchanged params reuses the existing table (RT-safe). The table is
    /// pitch-independent — the MIDI note only sets `phase_increment` — so a new
    /// note never dirties it.
    wavetable_dirty: bool,

    /// Sample rate the cached `wavetable` was built at. A change forces a rebuild
    /// because bin positions are derived from the sample rate.
    table_sample_rate: SampleRate,

    // Playback state
    phase: f64,
    phase_increment: f64,
    /// The pitch the note should sound at. `phase_increment` is *derived* from
    /// this and the current sample rate once per block in `process()` — never
    /// baked at `note_on`/`set_voice_pitch`. The engine propagates the render
    /// rate to voice-graph modules only via `ProcessContext`, so deferring the
    /// `pitch / sample_rate` division to `process()` (where the rate is known
    /// current) avoids the octave-off glitch a `note_on` fired before the first
    /// `process()` at a non-default rate would otherwise produce.
    current_pitch: Hertz,
    active: bool,
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets. Only `level` is a control-rate destination —
    /// bandwidth/tilt/detune/base_freq bake into the wavetable at note_on and
    /// can't be modulated mid-note without an (RT-unsafe) table rebuild.
    mod_offsets: ParamModOffsets,

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
            // No table built yet — force the first build on the first note_on.
            wavetable_dirty: true,
            table_sample_rate: SampleRate::DVD_QUALITY,

            phase: 0.0,
            phase_increment: 0.0,
            current_pitch: Hertz::new(261.63), // C4
            active: false,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),

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
        // Cache the inputs this table was built from so note_on can skip the
        // O(N²) rebuild when nothing changed.
        self.table_sample_rate = self.sample_rate;
        self.wavetable_dirty = false;

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
                // Bakes into the wavetable at note_on (no mid-note rebuild), so
                // it is not a control-rate mod/automation destination.
                .modulatable(false)
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
                // Wavetable-build param (see bandwidth) — not control-rate.
                .modulatable(false)
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
                // Wavetable-build param (see bandwidth) — not control-rate.
                .modulatable(false)
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
                .curve(ResponseCurve::Exponential)
                // Wavetable-build param (see bandwidth) — not control-rate.
                .modulatable(false)
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

        let level = self.mod_offsets.effective("level", self.level.as_f32());

        // Derive the phase increment from the current pitch and the render sample
        // rate here — not at note_on/set_voice_pitch — so it always tracks the rate
        // the engine just propagated via `context.sample_rate` (set above). Clamp via
        // the shared `Hertz::OSC_RANGE` preset, the same idiom every other voice-pitch
        // module uses.
        let sr = self.sample_rate.as_f32() as f64;
        let pitch = Hertz::new(Hertz::OSC_RANGE.clamp(self.current_pitch.as_f32()));
        self.phase_increment = f64::from(pitch.as_f32()) / sr;
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
                // Wavetable-build params: mark dirty so the next note_on rebuilds.
                PadSynthParam::Bandwidth(v) => {
                    self.bandwidth = v;
                    self.wavetable_dirty = true;
                }
                PadSynthParam::Tilt(v) => {
                    self.tilt = v;
                    self.wavetable_dirty = true;
                }
                PadSynthParam::Detune(v) => {
                    self.detune = v;
                    self.wavetable_dirty = true;
                }
                PadSynthParam::BaseFreq(hz) => {
                    self.base_freq = hz;
                    self.wavetable_dirty = true;
                }
                // Level is applied at process-time, not baked into the table.
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.active = false;
        self.wavetable.fill(0.0);
        // Table was zeroed — force a rebuild on the next note_on.
        self.wavetable_dirty = true;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        let note_freq = note.to_frequency();

        // Build the wavetable using the PADsynth algorithm — but ONLY when its
        // inputs changed. The table is built from base_freq for harmonic
        // placement (plus bandwidth/tilt/detune and the sample rate); the MIDI
        // note pitch is applied purely via phase_increment below, so it does
        // NOT affect the table. Rebuilding unconditionally here would run an
        // O(N²) inverse-DFT on the real-time audio thread on every note press,
        // causing CPU spikes / dropouts. Rebuild only if a wavetable-affecting
        // param is dirty or the sample rate changed; otherwise reuse the table.
        if self.wavetable_dirty || self.sample_rate != self.table_sample_rate {
            self.build_wavetable(self.base_freq);
        }

        // The wavetable encodes one cycle of the base_freq waveform; the note
        // pitch is applied purely via `phase_increment`, which `process()` derives
        // from `current_pitch` and the (then-current) sample rate. Just record the
        // target pitch here — do not bake the increment at the (possibly stale) rate.
        self.current_pitch = note_freq;

        self.phase = 0.0;
        self.active = true;
    }

    fn set_voice_pitch(&mut self, freq: Hertz) {
        // The wavetable is baked from base_freq and stays put; the note pitch is
        // only the phase increment, which `process()` derives from `current_pitch`
        // and the current sample rate (no O(N²) table rebuild). Just record the
        // target pitch — the clamp + division happen at the single site in process().
        self.current_pitch = freq;
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
    use crate::voice_pitch_harness::{amdf_fundamental, render_mono};
    use synth_core::{MidiNote, NormalizedValue, SampleRate, Velocity};

    /// `set_voice_pitch` retunes the pad by recomputing the phase increment (the
    /// baked wavetable stays): 2× voice pitch doubles the rendered fundamental;
    /// a static note holds. Measured at a low note (A2) with narrow bandwidth so
    /// the 64-harmonic table stays below Nyquist at both F and 2F (at high notes
    /// the upper harmonics alias and corrupt the estimate); pitch *tracking* is
    /// independent of pitch and bandwidth.
    #[test]
    fn padsynth_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let note = MidiNote(45); // A2 ≈ 110 Hz → 2F = 220, alias-free
        let f = note.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        let narrow = || {
            let mut s = PadSynth::new();
            s.set_param(Param::PadSynth(PadSynthParam::Bandwidth(
                NormalizedValue::new(0.02),
            )));
            s.note_on(note, Velocity::MAX);
            s
        };

        let mut s = narrow();
        let stat = render_mono(&mut s, sr, 4, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[2048..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static {est_stat}");

        let mut s2 = narrow();
        let up = render_mono(&mut s2, sr, 4, 1024, |m| {
            m.set_voice_pitch(Hertz::new(f * 2.0));
        });
        let est_up = amdf_fundamental(&up[2048..], srf, f * 2.0);
        assert!(cents(est_up, f * 2.0).abs() < 50.0, "2x {est_up}");
    }

    /// Regression: the phase increment must track the *render* sample rate, not the
    /// rate that happened to be cached when `note_on` fired. The engine propagates
    /// the rate only via `ProcessContext`, so a note triggered before the first
    /// `process()` at a non-default rate used to render an octave off. With the
    /// division deferred to `process()`, the increment follows the context rate.
    #[test]
    fn phase_increment_follows_render_rate_not_note_on_rate() {
        // Construct at the 48 kHz default, trigger A4 while still at 48 kHz...
        let mut pad = PadSynth::new();
        pad.note_on(MidiNote(69), Velocity::MAX); // A4 = 440 Hz
        let f = MidiNote(69).to_frequency().as_f32() as f64;

        // ...then render the first block at 96 kHz.
        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::HIGH_RES,
            ..ProcessContext::default()
        };
        let mut outs = HashMap::new();
        outs.insert(PortName::OUT, AudioBuffer::new(64));
        pad.process(InputPorts::empty(), &mut outs, &ctx);

        let expected = f / SampleRate::HIGH_RES.as_f32() as f64; // 440 / 96000
        let wrong = f / SampleRate::DVD_QUALITY.as_f32() as f64; // 440 / 48000 (the old bug)
        assert!(
            (pad.phase_increment - expected).abs() < 1e-9,
            "increment {} should track render rate ({expected}), not note_on rate ({wrong})",
            pad.phase_increment
        );
    }

    #[test]
    fn test_padsynth_creation() {
        let pad = PadSynth::new();
        assert!((pad.bandwidth.as_f32() - 0.4).abs() < 0.001);
        assert!(!pad.active);
    }

    /// `level` is a working control-rate mod destination via the generic store;
    /// the wavetable-build params are marked non-modulatable.
    #[test]
    fn level_mod_offset_scales_output() {
        let mut pad = PadSynth::new();
        let desc = pad.descriptor();
        pad.mod_offsets_mut().unwrap().populate(&desc);
        // Only `level` should be registered as modulatable.
        assert!(
            !desc
                .parameters
                .iter()
                .any(|p| p.type_id == "bandwidth" && p.modulatable)
        );

        pad.note_on(MidiNote::new(60), Velocity::new(0.8));
        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(pad: &mut PadSynth, ctx: &ProcessContext) -> f32 {
            pad.phase = 0.0; // start each measurement at the same table phase
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            pad.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut pad, &ctx);
        assert!(base > 1e-3, "base output present, got {base}");

        pad.set_mod_offset("level", -0.8);
        let quieter = peak(&mut pad, &ctx);
        assert!(
            quieter < base * 0.5,
            "level offset should reduce output: {quieter} vs {base}"
        );

        pad.clear_mod_offsets();
        assert!((peak(&mut pad, &ctx) - base).abs() < base * 0.1);
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
