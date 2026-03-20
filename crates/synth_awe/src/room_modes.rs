//! Room modes implemented as a bank of comb filters.
//!
//! Models axial, tangential, and oblique standing waves in a rectangular room.
//! Axial modes: `f = c/(2L)` — one dimension only.
//! Tangential modes: `f = c/2 * sqrt((n/L)² + (m/W)²)` — two dimensions.
//! Oblique modes: `f = c/2 * sqrt((n/L)² + (m/W)² + (p/H)²)` — all three.

use synth_core::{FilterState, Gain, Hertz, NormalizedValue, SampleCount, SampleRate};

use crate::types::{Meters, MetersPerSecond};
use synth_dsp::DelayLine;

/// Total number of room modes modeled.
///
/// 3 axial (1,0,0), (0,1,0), (0,0,1)
/// 3 axial overtones (2,0,0), (0,2,0), (0,0,2)
/// 3 tangential (1,1,0), (1,0,1), (0,1,1)
/// 2 higher tangential (2,1,0), (1,2,0)
/// 1 oblique (1,1,1)
const NUM_MODES: usize = 12;

/// Precomputed reciprocal of `NUM_MODES` to avoid division in the audio path.
const INV_NUM_MODES: f32 = 1.0 / NUM_MODES as f32;

/// Maximum delay line size in samples.
/// At 96 kHz the lowest mode for a 150 m room is ~2.3 Hz -> 96000/2.3 ≈ 42000.
/// We pre-allocate 48000 to cover large-room presets with headroom.
const MAX_DELAY_SAMPLES: SampleCount = SampleCount::new(48_000);

/// Mode indices for the three quantum numbers (nx, ny, nz).
/// Each mode is defined by its quantum numbers along the room's L, W, H axes.
const MODE_INDICES: [(u32, u32, u32); NUM_MODES] = [
    // Axial fundamentals
    (1, 0, 0),
    (0, 1, 0),
    (0, 0, 1),
    // Axial overtones
    (2, 0, 0),
    (0, 2, 0),
    (0, 0, 2),
    // Tangential (two-wall)
    (1, 1, 0),
    (1, 0, 1),
    (0, 1, 1),
    // Higher tangential
    (2, 1, 0),
    (1, 2, 0),
    // Oblique (three-wall)
    (1, 1, 1),
];

/// Relative gain for each mode type.
/// Tangential modes are weaker than axial (~0.7x),
/// oblique modes weaker still (~0.5x).
const MODE_GAINS: [Gain; NUM_MODES] = [
    // Axial fundamentals — full strength
    Gain::UNITY, Gain::UNITY, Gain::UNITY,
    // Axial overtones — slightly reduced
    Gain::new(0.8), Gain::new(0.8), Gain::new(0.8),
    // Tangential
    Gain::new(0.7), Gain::new(0.7), Gain::new(0.7),
    // Higher tangential
    Gain::new(0.5), Gain::new(0.5),
    // Oblique
    Gain::new(0.4),
];

/// A single comb filter for one room mode.
///
/// The feedback path includes frequency-dependent damping:
/// a one-pole LP for high-frequency absorption and a one-pole HP
/// for low-frequency absorption per reflection cycle.
///
/// ```text
/// input --> (+) --> delay_line --> output
///            ^                    |
///            |   LP -> HP filter  |
///            +--- feedback * -----+
/// ```
#[derive(Debug, Clone)]
pub(crate) struct CombFilter {
    delay_line: DelayLine,
    delay_samples: SampleCount,
    feedback: Gain,
    gain: Gain,
    lp_coeff: NormalizedValue,
    hp_coeff: NormalizedValue,
    lp_state: FilterState,
    hp_state: FilterState,
}

impl CombFilter {
    /// Create a new comb filter with pre-allocated delay line.
    #[must_use]
    fn new() -> Self {
        Self {
            delay_line: DelayLine::new(MAX_DELAY_SAMPLES.as_usize()),
            delay_samples: SampleCount::new(1),
            feedback: Gain::MUTE,
            gain: Gain::new(1.0),
            lp_coeff: NormalizedValue::new(0.3),
            hp_coeff: NormalizedValue::new(0.997),
            lp_state: FilterState::ZERO,
            hp_state: FilterState::ZERO,
        }
    }

    /// Update the comb filter from a mode frequency and relative gain.
    fn update(
        &mut self,
        mode_freq: Hertz,
        mode_gain: Gain,
        absorption_low: NormalizedValue,
        absorption_high: NormalizedValue,
        sample_rate: SampleRate,
    ) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let raw_delay = (sample_rate.as_f32() / mode_freq.as_f32().max(1.0)) as usize;
        let max_delay = self.delay_line.len().saturating_sub(1);
        self.delay_samples = SampleCount::new(raw_delay.clamp(1, max_delay));

        self.gain = mode_gain;

        // Feedback from average of low/high bands
        let avg = (absorption_low.as_f32() + absorption_high.as_f32()) * 0.5;
        self.feedback = Gain::new((0.85 * (1.0 - avg * 0.8)).max(0.1));

        // Frequency-dependent damping: sqrt() mapping for perceptual spread
        let abs_high_eff = absorption_high.as_f32().sqrt();
        let abs_low_eff = absorption_low.as_f32().sqrt();
        self.lp_coeff = NormalizedValue::new((0.15 + abs_high_eff * 0.70).clamp(0.0, 0.999));
        self.hp_coeff = NormalizedValue::new((0.997 - abs_low_eff * 0.35).clamp(0.0, 0.999));
    }

    /// Process a single sample through the comb filter.
    #[inline]
    #[must_use]
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.delay_line.read(self.delay_samples.as_usize());
        // LP in feedback path: HF absorption (material-dependent)
        let lp_out = self.lp_state.one_pole(delayed, self.lp_coeff.as_f32());
        // HP in feedback path: LF absorption (material-dependent)
        let filtered = self.hp_state.one_pole_hp(lp_out, self.hp_coeff.as_f32());
        let feedback_signal = self.feedback.apply(filtered);
        self.delay_line.write(input + feedback_signal);
        self.gain.apply(delayed)
    }

    /// Clear all internal state.
    fn clear(&mut self) {
        self.delay_line.clear();
        self.lp_state.reset();
        self.hp_state.reset();
    }
}

impl Default for CombFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the frequency of a room mode from its quantum numbers.
///
/// General formula: `f = c/2 * sqrt((nx/L)² + (ny/W)² + (nz/H)²)`
#[inline]
#[must_use]
fn mode_frequency(
    nx: u32,
    ny: u32,
    nz: u32,
    length: Meters,
    width: Meters,
    height: Meters,
    speed_of_sound: MetersPerSecond,
) -> Hertz {
    let nl = f64::from(nx) / f64::from(length.as_f32().max(0.001));
    let nw = f64::from(ny) / f64::from(width.as_f32().max(0.001));
    let nh = f64::from(nz) / f64::from(height.as_f32().max(0.001));
    let freq = f64::from(speed_of_sound.as_f32()) * 0.5 * (nl * nl + nw * nw + nh * nh).sqrt();
    #[allow(clippy::cast_possible_truncation)]
    Hertz::new(freq as f32)
}

/// Bank of room mode comb filters.
///
/// Models axial, tangential, and oblique standing waves as parallel
/// comb filters. The `amount` parameter controls how much of the modes
/// signal is mixed into the output.
#[derive(Debug, Clone)]
#[must_use]
pub struct RoomModeBank {
    modes: [CombFilter; NUM_MODES],
    amount: NormalizedValue,
}

impl RoomModeBank {
    /// Create a new room mode bank with pre-allocated delay lines.
    ///
    /// All modes are initially silent with zero amount.
    pub fn new() -> Self {
        Self {
            modes: std::array::from_fn(|_| CombFilter::new()),
            amount: NormalizedValue::MIN,
        }
    }

    /// Recalculate mode frequencies from room dimensions.
    ///
    /// Computes axial, tangential, and oblique modes using the general formula:
    /// `f = c/2 * sqrt((nx/L)² + (ny/W)² + (nz/H)²)`
    #[allow(clippy::too_many_arguments)]
    pub fn update_geometry(
        &mut self,
        room_length: Meters,
        room_width: Meters,
        room_height: Meters,
        absorption_low: NormalizedValue,
        absorption_high: NormalizedValue,
        speed_of_sound: MetersPerSecond,
        sample_rate: SampleRate,
    ) {
        for (i, mode) in self.modes.iter_mut().enumerate() {
            let (nx, ny, nz) = MODE_INDICES[i];
            let freq = mode_frequency(
                nx,
                ny,
                nz,
                room_length,
                room_width,
                room_height,
                speed_of_sound,
            );
            mode.update(
                freq,
                MODE_GAINS[i],
                absorption_low,
                absorption_high,
                sample_rate,
            );
        }
    }

    /// Process a single input sample, returns the modes contribution (mono).
    ///
    /// The input is fed into all comb filters in parallel and the
    /// outputs are summed and scaled by `amount`.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32) -> f32 {
        if self.amount.as_f32() <= 0.0 {
            // Still process all modes to keep filter state consistent
            for mode in &mut self.modes {
                let _ = mode.process(input);
            }
            return 0.0;
        }

        let mut sum = 0.0;
        for mode in &mut self.modes {
            sum += mode.process(input);
        }

        // Scale by 1/NUM_MODES to normalize and then by amount
        sum * self.amount.as_f32() * INV_NUM_MODES
    }

    /// Set the amount (wet mix level for modes).
    ///
    /// * `0.0` = modes are completely off.
    /// * `1.0` = full modes contribution.
    pub fn set_amount(&mut self, amount: NormalizedValue) {
        self.amount = amount;
    }

    /// Clear all internal state (delay lines and damping filters).
    pub fn clear(&mut self) {
        for mode in &mut self.modes {
            mode.clear();
        }
    }
}

impl Default for RoomModeBank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);

    #[test]
    fn test_default_is_silent() {
        let mut bank = RoomModeBank::new();
        // With amount=0, output should always be 0
        let out = bank.process(1.0);
        assert!((out).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mode_frequency_axial() {
        // Axial mode (1,0,0) for L=8m: f = 343/(2*8) = 21.4375 Hz
        let freq = mode_frequency(
            1,
            0,
            0,
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            SPEED_OF_SOUND,
        );
        assert!((freq.as_f32() - 21.4375).abs() < 0.01);
    }

    #[test]
    fn test_mode_frequency_tangential() {
        // Tangential mode (1,1,0) for L=8m, W=5m:
        // f = 343/2 * sqrt((1/8)² + (1/5)²) = 171.5 * sqrt(0.015625 + 0.04) = 171.5 * 0.2358 ≈ 40.44
        let freq = mode_frequency(
            1,
            1,
            0,
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            SPEED_OF_SOUND,
        );
        let expected = 343.0_f32 / 2.0 * ((1.0 / 8.0_f32).powi(2) + (1.0 / 5.0_f32).powi(2)).sqrt();
        assert!((freq.as_f32() - expected).abs() < 0.1);
    }

    #[test]
    fn test_mode_frequency_oblique() {
        // Oblique mode (1,1,1) for L=8, W=5, H=3:
        // f = 343/2 * sqrt((1/8)² + (1/5)² + (1/3)²)
        let freq = mode_frequency(
            1,
            1,
            1,
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            SPEED_OF_SOUND,
        );
        let expected = 343.0 / 2.0
            * ((1.0 / 8.0_f32).powi(2) + (1.0 / 5.0_f32).powi(2) + (1.0 / 3.0_f32).powi(2)).sqrt();
        assert!((freq.as_f32() - expected).abs() < 0.1);
    }

    #[test]
    fn test_update_geometry_sets_delay() {
        let mut bank = RoomModeBank::new();
        let sample_rate = SampleRate::new(48000.0);
        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            sample_rate,
        );

        // Length mode (1,0,0): 343/(2*8) = 21.4375 Hz -> 48000/21.4375 ≈ 2239 samples
        let expected_length = (sample_rate.as_f32() / (SPEED_OF_SOUND.as_f32() / 16.0)) as usize;
        assert_eq!(bank.modes[0].delay_samples.as_usize(), expected_length);

        // Width mode (0,1,0): 343/(2*5) = 34.3 Hz -> 48000/34.3 ≈ 1399 samples
        let expected_width = (sample_rate.as_f32() / (SPEED_OF_SOUND.as_f32() / 10.0)) as usize;
        assert_eq!(bank.modes[1].delay_samples.as_usize(), expected_width);

        // Height mode (0,0,1): 343/(2*3) ≈ 57.167 Hz -> 48000/57.167 ≈ 839 samples
        let expected_height = (sample_rate.as_f32() / (SPEED_OF_SOUND.as_f32() / 6.0)) as usize;
        assert_eq!(bank.modes[2].delay_samples.as_usize(), expected_height);
    }

    #[test]
    fn test_tangential_modes_have_shorter_delay() {
        let mut bank = RoomModeBank::new();
        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        // Tangential (1,1,0) should have higher frequency (shorter delay) than
        // either axial (1,0,0) or (0,1,0)
        let axial_l = bank.modes[0].delay_samples.as_usize();
        let axial_w = bank.modes[1].delay_samples.as_usize();
        let tangential_lw = bank.modes[6].delay_samples.as_usize();

        assert!(tangential_lw < axial_l);
        assert!(tangential_lw < axial_w);
    }

    #[test]
    fn test_feedback_scales_with_absorption() {
        let mut bank = RoomModeBank::new();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let fb_no_absorption = bank.modes[0].feedback.as_f32();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(1.0),
            NormalizedValue::new(1.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let fb_full_absorption = bank.modes[0].feedback.as_f32();

        // More absorption should give less feedback
        assert!(fb_no_absorption > fb_full_absorption);
        // Zero absorption: 0.85 * (1.0 - 0.0) = 0.85
        assert!((fb_no_absorption - 0.85).abs() < f32::EPSILON);
        // Full absorption: (0.85 * (1.0 - 0.8)).max(0.1) = 0.17
        assert!((fb_full_absorption - 0.17).abs() < f32::EPSILON);
    }

    #[test]
    fn test_lp_coeff_scales_with_hf_absorption() {
        let mut bank = RoomModeBank::new();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let lp_no_absorption = bank.modes[0].lp_coeff.as_f32();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(1.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let lp_full_hf_absorption = bank.modes[0].lp_coeff.as_f32();

        // More HF absorption should give higher LP coefficient (more damping)
        assert!(lp_full_hf_absorption > lp_no_absorption);
        // Zero absorption: 0.15 + 0.0 * 0.70 = 0.15
        assert!((lp_no_absorption - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hp_coeff_scales_with_lf_absorption() {
        let mut bank = RoomModeBank::new();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let hp_no_absorption = bank.modes[0].hp_coeff.as_f32();

        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(1.0),
            NormalizedValue::new(0.0),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        let hp_full_lf_absorption = bank.modes[0].hp_coeff.as_f32();

        // More LF absorption should give lower HP coefficient (more bass cut)
        assert!(hp_full_lf_absorption < hp_no_absorption);
    }

    #[test]
    fn test_process_produces_output() {
        let mut bank = RoomModeBank::new();
        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        bank.set_amount(NormalizedValue::new(1.0));

        // Feed an impulse
        let first_out = bank.process(1.0);
        // First sample: delay lines are empty, so output is 0
        assert!((first_out).abs() < f32::EPSILON);

        // Process silence until the shortest delay echoes back
        // Height mode (0,0,1) ≈ 839 samples
        let height_delay = bank.modes[2].delay_samples.as_usize();
        for _ in 1..height_delay {
            let _ = bank.process(0.0);
        }

        // At exactly the height delay, we should get the impulse echo
        let echo = bank.process(0.0);
        assert!(
            echo.abs() > 0.0,
            "Expected non-zero echo at delay {height_delay}"
        );
    }

    #[test]
    fn test_clear_resets_state() {
        let mut bank = RoomModeBank::new();
        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        bank.set_amount(NormalizedValue::new(1.0));

        // Feed some signal
        for _ in 0..1000 {
            let _ = bank.process(0.5);
        }

        bank.clear();

        // After clearing, output should be silent
        let out = bank.process(0.0);
        assert!((out).abs() < f32::EPSILON);
    }

    #[test]
    fn test_amount_clamps() {
        let mut bank = RoomModeBank::new();
        bank.set_amount(NormalizedValue::new(2.0));
        assert!((bank.amount.as_f32() - 1.0).abs() < f32::EPSILON);

        bank.set_amount(NormalizedValue::new(-0.5));
        assert!(bank.amount.as_f32().abs() < f32::EPSILON);
    }

    #[test]
    fn test_delay_clamped_to_buffer_size() {
        let mut bank = RoomModeBank::new();
        // Very large room: 100m -> 343/(200) ≈ 1.715 Hz -> 48000/1.715 ≈ 27988
        bank.update_geometry(
            Meters::new(100.0),
            Meters::new(100.0),
            Meters::new(100.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        for mode in &bank.modes {
            assert!(mode.delay_samples.as_usize() < MAX_DELAY_SAMPLES.as_usize());
        }
    }

    #[test]
    fn test_comb_filter_energy_decays() {
        let mut comb = CombFilter::new();
        let freq = mode_frequency(
            1,
            0,
            0,
            Meters::new(5.0),
            Meters::new(5.0),
            Meters::new(3.0),
            SPEED_OF_SOUND,
        );
        comb.update(
            freq,
            Gain::UNITY,
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.3),
            SampleRate::new(48000.0),
        );

        // Feed impulse
        let _ = comb.process(1.0);

        // Process many samples and collect peak energy
        let mut peak_early = 0.0_f32;
        let mut peak_late = 0.0_f32;
        for i in 1..20_000 {
            let out = comb.process(0.0);
            if i < 5000 {
                peak_early = peak_early.max(out.abs());
            } else {
                peak_late = peak_late.max(out.abs());
            }
        }

        // Energy should decay over time
        assert!(
            peak_late < peak_early,
            "Late energy {peak_late} should be less than early energy {peak_early}"
        );
    }

    #[test]
    fn test_mode_gains_applied() {
        let mut bank = RoomModeBank::new();
        bank.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.15),
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        // Axial modes should have full gain
        assert_eq!(bank.modes[0].gain, Gain::UNITY);
        // Oblique mode (last) should have 0.4 gain
        assert_eq!(bank.modes[11].gain, Gain::new(0.4));
        // Tangential modes should have 0.7 gain
        assert_eq!(bank.modes[6].gain, Gain::new(0.7));
    }

    #[test]
    fn test_twelve_modes_present() {
        assert_eq!(NUM_MODES, 12);
        assert_eq!(MODE_INDICES.len(), NUM_MODES);
        assert_eq!(MODE_GAINS.len(), NUM_MODES);
    }
}
