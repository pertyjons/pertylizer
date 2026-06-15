//! Oscillator module with multiple waveforms and intra-voice unison.
//!
//! Features:
//! - Multiple waveforms (sine, triangle, saw, square, pulse)
//! - Band-limited waveforms using PolyBLEP for anti-aliasing
//! - Pulse width modulation
//! - Hard sync
//! - FM and PM inputs
//! - Intra-voice unison with detune and stereo spread

use std::collections::HashMap;

use synth_core::{AntiAliasMode, FmMode, ModuleType, OscillatorParam, Param};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, Cents, Gain, Hertz, MidiNote, NormalizedValue, Phase, PortName,
    PulseWidth as PulseWidthParam, SampleRate, Semitones, Velocity, VoiceCount, Waveform,
};
use synth_dsp::oscillators::{poly_blamp, poly_blep};

use std::sync::LazyLock;

/// Shared MinBLEP lookup table (allocated once, read-only after init).
static MINBLEP_TABLE: LazyLock<synth_dsp::oscillators::MinBlepTable> =
    LazyLock::new(|| synth_dsp::oscillators::MinBlepTable::new(4, 64));

/// Maximum number of unison voices per oscillator.
const MAX_UNISON_VOICES: usize = 7;

/// Mod-matrix pitch scaling for the `detune` target: how many semitones a
/// full-scale (`±1`) modulation offset shifts the oscillator. Matches the detune
/// knob's own `±100 ¢` range (= ±1 semitone), so a normalized routing on `detune`
/// spans exactly that — the same musical unit as the knob.
///
/// This is the legacy native-unit (semitone) write path, like the dedicated
/// `pitch` target and the filter cutoff. It happens to equal the plan's locked
/// normalized-through-range contract *only because* detune's positive travel is
/// exactly 1 semitone; if the detune range ever changes, this should move to a
/// per-descriptor `mod_scale` hint (control-script-plan.md, scaling contract #2).
const DETUNE_MOD_SEMITONES: f32 = 1.0;

/// Mod-matrix pitch scaling for the `frequency` target: a full-scale (`±1`)
/// offset shifts the oscillator by ±12 semitones (one octave) — a deliberately
/// wide musical range, since the base `frequency` param is Hz on a log curve and
/// a raw normalized offset there would be a meaningless octave-ish smear. `detune`
/// stays narrow (±1 semitone) for fine vibrato; `frequency` is the coarse-sweep
/// target. (Decided ±12 with the user; widen/narrow here if a different feel is
/// wanted.)
const FREQUENCY_MOD_SEMITONES: f32 = 12.0;

/// A band-limited oscillator with optional intra-voice unison.
#[derive(Clone)]
pub struct Oscillator {
    // Parameters
    waveform: Waveform,
    frequency: Hertz,
    detune: Cents,
    octave: Semitones,
    pulse_width: PulseWidthParam,
    level: Gain,
    phase_offset: Phase,
    fm_mode: FmMode,
    fm_amount: BipolarValue,

    // Unison parameters
    unison_voice_count: VoiceCount,
    unison_detune: Cents,
    unison_spread: NormalizedValue,
    unison_phase_random: NormalizedValue,

    // Unison pre-computed state (updated on param change)
    unison_detune_ratios: [f32; MAX_UNISON_VOICES],
    unison_pans: [f32; MAX_UNISON_VOICES],
    /// Pre-computed equal-power pan gains (left, right) per unison voice.
    unison_pan_gains: [(f32, f32); MAX_UNISON_VOICES],

    // State
    unison_phases: [Phase; MAX_UNISON_VOICES],
    sample_rate: SampleRate,
    /// Previous sync signal value for edge detection (persists across buffers).
    prev_sync: NormalizedValue,

    // Anti-aliasing mode
    aa_mode: AntiAliasMode,

    // Cross-modulation
    /// Cross-modulation amount (0.0 = off, 1.0 = full).
    cross_mod_amount: NormalizedValue,

    // Mod matrix offsets
    /// Pitch offset in semitones (from mod matrix).
    mod_offset_pitch: Semitones,
    /// Level offset (additive, from mod matrix).
    mod_offset_level: BipolarValue,

    // Transient automation overrides (replace the base value while active,
    // cleared on transport stop; the base param is never mutated).
    /// Detune override from sequencer automation.
    override_detune: Option<Cents>,
    /// Pulse-width override from sequencer automation.
    override_pulse_width: Option<PulseWidthParam>,

    // Outputs
    output_buffer: AudioBuffer,
    output_buffer_left: AudioBuffer,
    output_buffer_right: AudioBuffer,
}

impl Oscillator {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sawtooth,
            frequency: Hertz::A4,
            detune: Cents::ZERO,
            octave: Semitones::ZERO,
            pulse_width: PulseWidthParam::SQUARE,
            level: Gain::UNITY,
            phase_offset: Phase::ZERO,
            fm_mode: FmMode::Exponential,
            fm_amount: BipolarValue::MAX,
            unison_voice_count: VoiceCount::new(1),
            unison_detune: Cents::new(10.0),
            unison_spread: NormalizedValue::CENTER,
            unison_phase_random: NormalizedValue::MAX,
            aa_mode: AntiAliasMode::PolyBlep,
            cross_mod_amount: NormalizedValue::MIN,
            unison_detune_ratios: [1.0; MAX_UNISON_VOICES],
            unison_pans: [0.0; MAX_UNISON_VOICES],
            unison_pan_gains: [(
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ); MAX_UNISON_VOICES],
            unison_phases: [Phase::ZERO; MAX_UNISON_VOICES],
            sample_rate: SampleRate::DVD_QUALITY,
            prev_sync: NormalizedValue::MIN,
            mod_offset_pitch: Semitones::ZERO,
            mod_offset_level: BipolarValue::CENTER,
            override_detune: None,
            override_pulse_width: None,
            output_buffer: AudioBuffer::new(1024),
            output_buffer_left: AudioBuffer::new(1024),
            output_buffer_right: AudioBuffer::new(1024),
        }
    }

    /// Calculate the actual frequency including detune, octave, and mod matrix pitch offset.
    #[inline]
    fn actual_frequency(&self) -> Hertz {
        let octave_mult = crate::math::semitones_to_ratio(self.octave.as_f32());
        // Apply mod matrix pitch offset (in semitones)
        let mod_mult = crate::math::semitones_to_ratio(self.mod_offset_pitch.as_f32());
        let detune = self.override_detune.unwrap_or(self.detune);
        Hertz::new(detune.apply(self.frequency).as_f32() * octave_mult * mod_mult)
    }

    /// Recalculate unison detune ratios and pan positions.
    /// Called when unison parameters change.
    fn recalculate_unison_spread(&mut self) {
        let n = self.unison_voice_count.as_usize();
        if n <= 1 {
            self.unison_detune_ratios[0] = 1.0;
            self.unison_pans[0] = 0.0;
            self.unison_pan_gains[0] = crate::math::equal_power_pan(0.0);
            return;
        }
        let total_cents = self.unison_detune.as_f32();
        let spread = self.unison_spread.as_f32();
        #[allow(clippy::cast_precision_loss)]
        let n_minus_1 = (n - 1) as f32;
        for i in 0..n {
            #[allow(clippy::cast_precision_loss)]
            let t = (i as f32) / n_minus_1 * 2.0 - 1.0; // -1.0 to 1.0
            let cents = t * total_cents * 0.5;
            self.unison_detune_ratios[i] = crate::math::cents_to_ratio(cents);
            self.unison_pans[i] = t * spread;
            self.unison_pan_gains[i] = crate::math::equal_power_pan(t * spread);
        }
    }

    /// MinBLEP correction at a discontinuity point.
    /// Returns the residual correction (similar to poly_blep but using the lookup table).
    #[inline]
    fn minblep_correction(phase: f32, dt: f32) -> f32 {
        let table = &*MINBLEP_TABLE;
        #[allow(clippy::cast_precision_loss)]
        let table_len = table.length_samples() as f32;
        // Distance from discontinuity in samples
        let d = phase / dt;
        if d < table_len {
            table.lookup(d / table_len) - 1.0
        } else {
            let d2 = (1.0 - phase) / dt;
            if d2 < table_len {
                1.0 - table.lookup(d2 / table_len)
            } else {
                0.0
            }
        }
    }

    /// Generate a single sample for one oscillator voice (no level applied).
    /// Returns the sample and the advanced phase.
    #[inline]
    fn generate_single_sample(
        &self,
        freq: Hertz,
        phase: Phase,
        phase_mod: f32,
        effective_pulse_width: NormalizedValue,
    ) -> (f32, Phase) {
        let dt = freq.phase_increment(self.sample_rate);
        let p = (phase.advance(phase_mod).as_f32() + self.phase_offset.as_f32()) % 1.0;

        let sample = match self.waveform {
            Waveform::Sine => crate::math::fast_sin_turns(p),
            Waveform::Triangle => {
                // Triangle uses BLAMP (not BLEP) — no MinBLEP alternative needed.
                // Raw mode skips the band-limiting for the naive aliased edge.
                let mut tri = Phase::new_unchecked(p).triangle();
                if self.aa_mode != AntiAliasMode::Raw {
                    let dist_to_peak = p - 0.25;
                    tri += poly_blamp(dist_to_peak, dt) * 4.0;
                    let dist_to_trough = p - 0.75;
                    tri -= poly_blamp(dist_to_trough, dt) * 4.0;
                }
                tri
            }
            Waveform::Sawtooth => {
                let mut saw = Phase::new_unchecked(p).sawtooth();
                match self.aa_mode {
                    AntiAliasMode::PolyBlep => saw -= poly_blep(p, dt),
                    AntiAliasMode::MinBlep => saw -= Self::minblep_correction(p, dt),
                    AntiAliasMode::Raw => {} // naive — leave the raw aliased edge
                }
                saw
            }
            Waveform::Square => {
                let mut sq = Phase::new_unchecked(p).pulse(NormalizedValue::CENTER);
                match self.aa_mode {
                    AntiAliasMode::PolyBlep => {
                        sq += poly_blep(p, dt);
                        sq -= poly_blep((p + 0.5).rem_euclid(1.0), dt);
                    }
                    AntiAliasMode::MinBlep => {
                        sq += Self::minblep_correction(p, dt);
                        sq -= Self::minblep_correction((p + 0.5).rem_euclid(1.0), dt);
                    }
                    AntiAliasMode::Raw => {} // naive — leave the raw aliased edge
                }
                sq
            }
            Waveform::Pulse => {
                let mut pulse = Phase::new_unchecked(p).pulse(effective_pulse_width);
                let pw = effective_pulse_width.as_f32().clamp(0.01, 0.99);
                match self.aa_mode {
                    AntiAliasMode::PolyBlep => {
                        pulse += poly_blep(p, dt);
                        pulse -= poly_blep((p + (1.0 - pw)).rem_euclid(1.0), dt);
                    }
                    AntiAliasMode::MinBlep => {
                        pulse += Self::minblep_correction(p, dt);
                        pulse -= Self::minblep_correction((p + (1.0 - pw)).rem_euclid(1.0), dt);
                    }
                    AntiAliasMode::Raw => {} // naive — leave the raw aliased edge
                }
                pulse
            }
            Waveform::DsfSaw => {
                // Inherently band-limited (harmonic sum) — ignores `aa_mode`,
                // including Raw: there is no aliased edge to leave in.
                #[allow(clippy::cast_possible_truncation)]
                let num_harmonics =
                    (self.sample_rate.as_f32() / (2.0 * freq.as_f32())).max(1.0) as u32;
                synth_dsp::oscillators::dsf_sawtooth(
                    Phase::new_unchecked(p),
                    num_harmonics,
                    NormalizedValue::new(0.95),
                )
            }
        };

        (sample, phase.advance(dt))
    }

    /// Set frequency from MIDI note.
    pub fn set_note(&mut self, note: MidiNote) {
        self.frequency = note.to_frequency();
    }

    /// Set frequency using the type-safe Hertz type.
    pub fn set_frequency(&mut self, freq: Hertz) {
        self.frequency = freq;
    }

    /// Get current frequency as type-safe Hertz.
    pub fn get_frequency(&self) -> Hertz {
        self.frequency
    }
}

impl Default for Oscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Oscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("oscillator", "Oscillator")
            .description("Band-limited oscillator with multiple waveforms and unison")
            .category(ModuleCategory::Oscillator)
            .tag("oscillator")
            .tag("source")
            .parameter(
                ParameterDescriptor::choice(
                    "waveform",
                    Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)),
                    "Waveform",
                    Waveform::to_choices(),
                )
                .description("Oscillator waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::float(
                    "frequency",
                    Param::Oscillator(OscillatorParam::Frequency(Hertz::A4)),
                    "Frequency",
                )
                .description("Base frequency")
                .range(1.0, 20000.0)
                .default(440.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "detune",
                    Param::Oscillator(OscillatorParam::Detune(Cents::ZERO)),
                    "Detune",
                )
                .description("Fine tune in cents")
                .range(-100.0, 100.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pulse_width",
                    Param::Oscillator(OscillatorParam::PulseWidth(PulseWidthParam::SQUARE)),
                    "Pulse Width",
                )
                .description("Pulse width for pulse waveform")
                .range(0.01, 0.99)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Oscillator(OscillatorParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "fm_mode",
                    Param::Oscillator(OscillatorParam::FmMode(FmMode::Exponential)),
                    "FM Mode",
                    FmMode::to_choices(),
                )
                .description("FM input mode: Exponential (1V/oct) or Linear (Hz)")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "fm_amt",
                    Param::Oscillator(OscillatorParam::FmAmount(BipolarValue::MAX)),
                    "FM Amt",
                )
                .description("FM input attenuverter (-1 to +1)")
                .range(-1.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "unison",
                    Param::Oscillator(OscillatorParam::unison_voices_default()),
                    "Unison",
                )
                .description("Number of unison voices (1 = off)")
                .range(1.0, 7.0)
                .default(1.0)
                // Structural/sizing param: voices appear/disappear, so it is not
                // a continuous automation/modulation target.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "uni_detune",
                    Param::Oscillator(OscillatorParam::unison_detune_default()),
                    "Uni Detune",
                )
                .description("Unison detune spread in cents")
                .range(0.0, 100.0)
                .default(10.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "uni_spread",
                    Param::Oscillator(OscillatorParam::unison_spread_default()),
                    "Uni Spread",
                )
                .description("Unison stereo spread (0 = mono, 1 = full)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "uni_phase",
                    Param::Oscillator(OscillatorParam::unison_phase_random_default()),
                    "Uni Phase",
                )
                .description("Phase randomization on note-on")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("fm", "FM").description("Modulates pitch. Connect: LFO for vibrato, Envelope for pitch sweep, another Oscillator for FM synthesis"),
            )
            .port(PortDescriptor::control_input("pm", "PM").description("Modulates phase — gives FM-like sound. Connect: LFO, Envelope, another Oscillator"))
            .port(PortDescriptor::control_input("pwm", "PWM").description("Modulates pulse width (square/pulse). Connect: LFO for classic PWM sound, Envelope"))
            .port(
                PortDescriptor::control_input("cross_mod", "X-Mod")
                    .description("Cross-modulates frequency with another oscillator. Connect: another Oscillator for metallic/bell sounds"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "x_mod",
                    Param::Oscillator(OscillatorParam::cross_mod_amount_default()),
                    "X-Mod",
                )
                .description("Cross-modulation amount (0 = off, 1 = full)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "anti_alias",
                    Param::Oscillator(OscillatorParam::AntiAlias(AntiAliasMode::PolyBlep)),
                    "Anti-Alias",
                    AntiAliasMode::ALL
                        .iter()
                        .map(|m| {
                            synth_core::module_traits::ChoiceOption::new(m.id(), m.name())
                                .with_description(m.description())
                        })
                        .collect(),
                )
                .description("Anti-aliasing algorithm"),
            )
            .port(PortDescriptor::gate_input("sync", "Sync").description("Resets phase on gate. Connect: another Oscillator's output for hard sync sounds"))
            .port(PortDescriptor::audio_output("out", "Out").description("Audio output (mono sum)"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Stereo left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Stereo right output"))
    }
}

impl PolyModule for Oscillator {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let n_samples = context.samples.as_usize();
        self.output_buffer.resize(n_samples);
        self.output_buffer_left.resize(n_samples);
        self.output_buffer_right.resize(n_samples);

        let fm_reader = inputs.reader(PortName::FM, 0.0);
        let pm_reader = inputs.reader(PortName::PM, 0.0);
        #[allow(clippy::similar_names)]
        // pm = phase mod, pwm = pulse width mod - different concepts
        let pwm_reader = inputs.reader(PortName::PWM, 0.0);
        let sync_reader = inputs.reader(PortName::SYNC, 0.0);
        let cross_mod_reader = inputs.reader(PortName::CROSS_MOD, 0.0);

        let voice_count = self.unison_voice_count.as_usize();
        let base_freq = self.actual_frequency();

        // An automation override replaces the base pulse width while active;
        // the base param is left untouched.
        let pulse_width = self.override_pulse_width.unwrap_or(self.pulse_width);

        // Hoist the unmodulated form so the audio loop's else-branch is a
        // straight copy rather than re-clamping per sample. `PulseWidth`'s
        // range [0.01, 0.99] is already a subset of NormalizedValue's
        // [0, 1], so `new_unchecked` is sound here.
        let pulse_width_unmodulated = NormalizedValue::new_unchecked(pulse_width.as_f32());

        for i in 0..n_samples {
            let fm = fm_reader[i] * self.fm_amount.as_f32();

            // Cross-modulation: adds to FM signal
            let cross_mod = cross_mod_reader[i] * self.cross_mod_amount.as_f32();

            let total_fm = fm + cross_mod;

            let freq = match self.fm_mode {
                FmMode::Exponential => base_freq.apply_fm(BipolarValue::new(total_fm)),
                FmMode::Linear => {
                    Hertz::new((base_freq.as_f32() + total_fm * base_freq.as_f32() * 4.0).max(1.0))
                }
            };

            let effective_pulse_width = if pwm_reader.is_connected() {
                NormalizedValue::new(
                    (pulse_width.as_f32() + pwm_reader[i] * 0.49).clamp(0.01, 0.99),
                )
            } else {
                pulse_width_unmodulated
            };

            if sync_reader.is_connected() {
                let sync_val = sync_reader[i];
                if sync_val > 0.5 && self.prev_sync.as_f32() <= 0.5 {
                    for phase in &mut self.unison_phases[..voice_count] {
                        *phase = Phase::ZERO;
                    }
                }
                self.prev_sync = NormalizedValue::new(sync_val);
            }

            let pm = pm_reader[i];
            let effective_level =
                (self.level.as_f32() + self.mod_offset_level.as_f32()).clamp(0.0, 2.0);

            if voice_count == 1 {
                // Single voice: no panning, write directly to all ports
                let phase = self.unison_phases[0];
                let (sample, new_phase) =
                    self.generate_single_sample(freq, phase, pm, effective_pulse_width);
                self.unison_phases[0] = new_phase;
                let out = sample * effective_level;
                self.output_buffer[i] = out;
                self.output_buffer_left[i] = out;
                self.output_buffer_right[i] = out;
            } else {
                let mut left = 0.0f32;
                let mut right = 0.0f32;
                #[allow(clippy::cast_precision_loss)]
                let gain = crate::math::normalization_gain(voice_count);

                for j in 0..voice_count {
                    let voice_freq = Hertz::new(freq.as_f32() * self.unison_detune_ratios[j]);
                    let phase = self.unison_phases[j];
                    let (sample, new_phase) =
                        self.generate_single_sample(voice_freq, phase, pm, effective_pulse_width);
                    self.unison_phases[j] = new_phase;

                    let (pan_l, pan_r) = self.unison_pan_gains[j];
                    left += sample * pan_l * gain;
                    right += sample * pan_r * gain;
                }

                self.output_buffer[i] = (left + right) * 0.5 * effective_level;
                self.output_buffer_left[i] = left * effective_level;
                self.output_buffer_right[i] = right * effective_level;
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
        if let Some(out_l) = outputs.get_mut(&PortName::OUT_L) {
            out_l.copy_from(&self.output_buffer_left);
        }
        if let Some(out_r) = outputs.get_mut(&PortName::OUT_R) {
            out_r.copy_from(&self.output_buffer_right);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Oscillator(osc_param) = param {
            match osc_param {
                OscillatorParam::Waveform(w) => self.waveform = w,
                OscillatorParam::Frequency(f) => {
                    self.frequency = Hertz::new(f.as_f32().clamp(1.0, 20000.0));
                }
                OscillatorParam::Detune(d) => self.detune = d.clamp_detune(),
                OscillatorParam::Octave(o) => self.octave = o,
                OscillatorParam::PulseWidth(pw) => {
                    self.pulse_width = pw;
                }
                OscillatorParam::Level(l) => self.level = l,
                OscillatorParam::Phase(p) => self.phase_offset = p,
                OscillatorParam::FmMode(m) => self.fm_mode = m,
                OscillatorParam::FmAmount(a) => self.fm_amount = a,
                OscillatorParam::UnisonVoices(n) => {
                    self.unison_voice_count = VoiceCount::new(n.clamp(1, 7));
                    self.recalculate_unison_spread();
                }
                OscillatorParam::UnisonDetune(c) => {
                    self.unison_detune = Cents::new(c.as_f32().clamp(0.0, 100.0));
                    self.recalculate_unison_spread();
                }
                OscillatorParam::UnisonSpread(v) => {
                    self.unison_spread = v;
                    self.recalculate_unison_spread();
                }
                OscillatorParam::UnisonPhaseRandom(v) => self.unison_phase_random = v,
                OscillatorParam::CrossModAmount(v) => self.cross_mod_amount = v,
                OscillatorParam::AntiAlias(m) => self.aa_mode = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Oscillator(osc_param) = param {
            Some(match osc_param {
                OscillatorParam::Waveform(_) => self.waveform.index() as f32,
                OscillatorParam::Frequency(_) => self.frequency.as_f32(),
                OscillatorParam::Detune(_) => self.detune.as_f32(),
                OscillatorParam::Octave(_) => self.octave.as_f32(),
                OscillatorParam::PulseWidth(_) => self.pulse_width.as_f32(),
                OscillatorParam::Level(_) => self.level.as_f32(),
                OscillatorParam::Phase(_) => self.phase_offset.as_f32(),
                OscillatorParam::FmMode(_) => self.fm_mode.index() as f32,
                OscillatorParam::FmAmount(_) => self.fm_amount.as_f32(),
                OscillatorParam::UnisonVoices(_) => f32::from(self.unison_voice_count.as_u8()),
                OscillatorParam::UnisonDetune(_) => self.unison_detune.as_f32(),
                OscillatorParam::UnisonSpread(_) => self.unison_spread.as_f32(),
                OscillatorParam::UnisonPhaseRandom(_) => self.unison_phase_random.as_f32(),
                OscillatorParam::CrossModAmount(_) => self.cross_mod_amount.as_f32(),
                #[allow(clippy::cast_precision_loss)]
                OscillatorParam::AntiAlias(_) => self.aa_mode.index() as f32,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Oscillator(OscillatorParam::Waveform(self.waveform)),
            Param::Oscillator(OscillatorParam::Frequency(self.frequency)),
            Param::Oscillator(OscillatorParam::Detune(self.detune)),
            Param::Oscillator(OscillatorParam::Octave(self.octave)),
            Param::Oscillator(OscillatorParam::PulseWidth(self.pulse_width)),
            Param::Oscillator(OscillatorParam::Level(self.level)),
            Param::Oscillator(OscillatorParam::Phase(self.phase_offset)),
            Param::Oscillator(OscillatorParam::FmMode(self.fm_mode)),
            Param::Oscillator(OscillatorParam::FmAmount(self.fm_amount)),
            Param::Oscillator(OscillatorParam::UnisonVoices(
                self.unison_voice_count.as_u8(),
            )),
            Param::Oscillator(OscillatorParam::UnisonDetune(self.unison_detune)),
            Param::Oscillator(OscillatorParam::UnisonSpread(self.unison_spread)),
            Param::Oscillator(OscillatorParam::UnisonPhaseRandom(self.unison_phase_random)),
            Param::Oscillator(OscillatorParam::CrossModAmount(self.cross_mod_amount)),
            Param::Oscillator(OscillatorParam::AntiAlias(self.aa_mode)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Oscillator
    }

    fn reset(&mut self) {
        self.unison_phases = [Phase::ZERO; MAX_UNISON_VOICES];
        self.prev_sync = NormalizedValue::MIN;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.set_note(note);
        let n = self.unison_voice_count.as_usize();
        let random = self.unison_phase_random.as_f32();
        for phase in &mut self.unison_phases[..n] {
            *phase = if random > 0.0 {
                Phase::new(fastrand::f32() * random)
            } else {
                Phase::ZERO
            };
        }
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn set_mod_offset(&mut self, target: &str, value: f32) {
        match target {
            // `pitch`, `detune` and `frequency` are all pitch shifts, so they
            // accumulate into the shared semitone offset applied in
            // `actual_frequency`; the per-target scale sets the musical range.
            "pitch" => {
                self.mod_offset_pitch = Semitones::new(self.mod_offset_pitch.as_f32() + value)
            }
            "detune" => {
                self.mod_offset_pitch =
                    Semitones::new(self.mod_offset_pitch.as_f32() + value * DETUNE_MOD_SEMITONES)
            }
            "frequency" => {
                self.mod_offset_pitch =
                    Semitones::new(self.mod_offset_pitch.as_f32() + value * FREQUENCY_MOD_SEMITONES)
            }
            "level" => {
                self.mod_offset_level = BipolarValue::new(self.mod_offset_level.as_f32() + value)
            }
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_pitch = Semitones::ZERO;
        self.mod_offset_level = BipolarValue::CENTER;
    }

    fn set_param_override(&mut self, param: Param) {
        if let Param::Oscillator(osc_param) = param {
            match osc_param {
                OscillatorParam::Detune(d) => self.override_detune = Some(d.clamp_detune()),
                OscillatorParam::PulseWidth(pw) => self.override_pulse_width = Some(pw),
                // Base frequency is note-driven; remaining params are not
                // pitch/PWM automation targets in this first cut.
                _ => {}
            }
        }
    }

    fn clear_param_overrides(&mut self) {
        self.override_detune = None;
        self.override_pulse_width = None;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn test_oscillator_param_automatable_allowlist() {
        let d = Oscillator::new().descriptor();
        let auto = |id: &str| {
            d.parameters
                .iter()
                .find(|p| p.type_id == id)
                .unwrap_or_else(|| panic!("missing param {id}"))
                .is_automatable()
        };
        // Continuous params (incl. the A1 override targets) are automatable.
        assert!(auto("detune"));
        assert!(auto("pulse_width"));
        // Choice/enum params are excluded.
        assert!(!auto("waveform"));
        assert!(!auto("fm_mode"));
        // Structural/sizing param is excluded (modulatable(false)).
        assert!(!auto("unison"));
    }

    /// A mod-matrix routing to `osc.detune` actually shifts pitch now (it used
    /// to hit the `_ => {}` arm and vanish). Full-scale (`+1`) = +1 semitone,
    /// matching the detune knob's own ±100 ¢ range; clearing reverts.
    #[test]
    fn detune_mod_offset_shifts_pitch_one_semitone_full_scale() {
        let mut osc = Oscillator::new();
        osc.set_param(Param::Oscillator(OscillatorParam::Frequency(Hertz::new(
            440.0,
        ))));
        let base = osc.actual_frequency().as_f32();

        osc.set_mod_offset("detune", 1.0);
        let modded = osc.actual_frequency().as_f32();
        let semitone = 2f32.powf(1.0 / 12.0);
        assert!(
            (modded / base - semitone).abs() < 1e-3,
            "detune +1 should raise pitch one semitone: ratio {}",
            modded / base
        );

        osc.clear_mod_offsets();
        assert!((osc.actual_frequency().as_f32() - base).abs() < 0.5);
    }

    /// A routing to `osc.frequency` is the coarse pitch-sweep target: full-scale
    /// (`+1`) raises pitch a full octave (±12 semitones). Previously dropped.
    #[test]
    fn frequency_mod_offset_sweeps_one_octave_full_scale() {
        let mut osc = Oscillator::new();
        osc.set_param(Param::Oscillator(OscillatorParam::Frequency(Hertz::new(
            220.0,
        ))));
        let base = osc.actual_frequency().as_f32();

        osc.set_mod_offset("frequency", 1.0);
        assert!(
            (osc.actual_frequency().as_f32() / base - 2.0).abs() < 1e-3,
            "frequency +1 should raise pitch one octave: ratio {}",
            osc.actual_frequency().as_f32() / base
        );
        // Negative full-scale drops an octave.
        osc.clear_mod_offsets();
        osc.set_mod_offset("frequency", -1.0);
        assert!((osc.actual_frequency().as_f32() / base - 0.5).abs() < 1e-3);
    }

    #[test]
    fn test_oscillator_param_override_replaces_base_and_reverts() {
        let mut osc = Oscillator::new();
        osc.set_param(Param::Oscillator(OscillatorParam::Frequency(Hertz::new(
            440.0,
        ))));
        osc.set_param(Param::Oscillator(OscillatorParam::Detune(Cents::ZERO)));
        osc.set_param(Param::Oscillator(OscillatorParam::PulseWidth(
            PulseWidthParam::SQUARE,
        )));

        let base_freq = osc.actual_frequency().as_f32();
        assert!((base_freq - 440.0).abs() < 0.5);

        // +100 cents (one semitone, the clamp_detune ceiling) raises the pitch
        // by the 12-TET ratio 2^(1/12).
        osc.set_param_override(Param::Oscillator(OscillatorParam::Detune(Cents::new(
            100.0,
        ))));
        let override_freq = osc.actual_frequency().as_f32();
        assert!((override_freq / base_freq - 2.0_f32.powf(1.0 / 12.0)).abs() < 1e-3);

        // Pulse-width override is stored without touching the base.
        osc.set_param_override(Param::Oscillator(OscillatorParam::PulseWidth(
            PulseWidthParam::new(0.25),
        )));
        assert_matches!(osc.override_pulse_width, Some(pw) if (pw.as_f32() - 0.25).abs() < 1e-6);

        // Base params are never mutated by the override.
        assert!(osc.detune.as_f32().abs() < 1e-6);
        assert!((osc.pulse_width.as_f32() - PulseWidthParam::SQUARE.as_f32()).abs() < 1e-6);

        // Clearing reverts to the base.
        osc.clear_param_overrides();
        assert!((osc.actual_frequency().as_f32() - 440.0).abs() < 0.5);
        assert!(osc.override_detune.is_none());
        assert!(osc.override_pulse_width.is_none());
    }

    #[test]
    fn test_oscillator_creation() {
        let osc = Oscillator::new();
        assert_eq!(osc.waveform, Waveform::Sawtooth);
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);
        assert_eq!(osc.unison_voice_count, VoiceCount::MONO);
    }

    #[test]
    fn test_note_to_frequency() {
        let mut osc = Oscillator::new();
        osc.set_note(MidiNote::A4); // A4
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);

        osc.set_note(MidiNote::C4); // C4
        assert!((osc.frequency.as_f32() - 261.63).abs() < 1.0);
    }

    #[test]
    fn test_waveform_output() {
        let mut osc = Oscillator::new();
        osc.waveform = Waveform::Sine;
        osc.frequency = Hertz::new(1000.0);
        osc.sample_rate = SampleRate::DVD_QUALITY;

        let effective_pulse_width = NormalizedValue::new(osc.pulse_width.as_f32());
        let freq = osc.actual_frequency();
        for _ in 0..100 {
            let phase = osc.unison_phases[0];
            let (sample, new_phase) =
                osc.generate_single_sample(freq, phase, 0.0, effective_pulse_width);
            osc.unison_phases[0] = new_phase;
            assert!(
                (-1.0..=1.0).contains(&sample),
                "sample {sample} out of range"
            );
        }
    }

    #[test]
    fn test_unison_spread_calculation() {
        let mut osc = Oscillator::new();
        osc.unison_voice_count = VoiceCount::new(3);
        osc.unison_detune = Cents::new(20.0);
        osc.unison_spread = NormalizedValue::new(0.8);
        osc.recalculate_unison_spread();

        // Detune ratios should be symmetric around 1.0
        assert!(
            osc.unison_detune_ratios[0] < 1.0,
            "first voice should be detuned down"
        );
        assert!(
            (osc.unison_detune_ratios[1] - 1.0).abs() < 0.0001,
            "center voice should be at 1.0"
        );
        assert!(
            osc.unison_detune_ratios[2] > 1.0,
            "last voice should be detuned up"
        );

        // Pans should be symmetric around center
        assert!(osc.unison_pans[0] < 0.0, "first voice should pan left");
        assert!(
            osc.unison_pans[1].abs() < 0.0001,
            "center voice should be center"
        );
        assert!(osc.unison_pans[2] > 0.0, "last voice should pan right");

        // Symmetry: ratios should mirror, pans should be opposite
        let ratio_diff =
            (osc.unison_detune_ratios[0] - 1.0).abs() - (osc.unison_detune_ratios[2] - 1.0).abs();
        assert!(ratio_diff.abs() < 0.0001, "detune should be symmetric");
        assert!(
            (osc.unison_pans[0] + osc.unison_pans[2]).abs() < 0.0001,
            "pans should be symmetric"
        );
    }

    #[test]
    fn test_unison_single_voice_unchanged() {
        let mut osc = Oscillator::new();
        osc.unison_voice_count = VoiceCount::MONO;
        osc.recalculate_unison_spread();

        assert!(
            (osc.unison_detune_ratios[0] - 1.0).abs() < 0.0001,
            "single voice should have no detune"
        );
        assert!(
            osc.unison_pans[0].abs() < 0.0001,
            "single voice should be center"
        );
    }

    #[test]
    fn test_unison_generates_valid_samples() {
        let mut osc = Oscillator::new();
        osc.waveform = Waveform::Sawtooth;
        osc.frequency = Hertz::new(440.0);
        osc.sample_rate = SampleRate::DVD_QUALITY;
        osc.unison_voice_count = VoiceCount::new(5);
        osc.unison_detune = Cents::new(30.0);
        osc.unison_spread = NormalizedValue::MAX;
        osc.recalculate_unison_spread();

        let freq = osc.actual_frequency();
        for j in 0..5 {
            let voice_freq = Hertz::new(freq.as_f32() * osc.unison_detune_ratios[j]);
            let phase = osc.unison_phases[j];
            let (sample, _) = osc.generate_single_sample(
                voice_freq,
                phase,
                0.0,
                NormalizedValue::new(osc.pulse_width.as_f32()),
            );
            assert!(
                sample.abs() <= 1.5,
                "voice {j} sample {sample} out of reasonable range"
            );
        }
    }
}
