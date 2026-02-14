//! Axial room modes implemented as a bank of comb filters.
//!
//! Each axis of a rectangular room produces a standing wave at
//! `f = c / (2 * L)` where `c` is the speed of sound and `L` is the
//! dimension. This module models the three axial modes (length, width,
//! height) as parallel comb filters with damped feedback paths.

use crate::room::SPEED_OF_SOUND;
use synth_dsp::DelayLine;

/// Number of axial modes (one per room axis).
const NUM_MODES: usize = 3;

/// Maximum delay line size in samples.
/// At 96 kHz the lowest mode we'd model is ~20 Hz -> 96000/20 = 4800 samples.
/// We pre-allocate 5000 to have some headroom.
const MAX_DELAY_SAMPLES: usize = 5000;

/// A single comb filter for one axial room mode.
///
/// The feedback path includes a one-pole low-pass filter that models
/// high-frequency absorption on each reflection cycle.
///
/// ```text
/// input --> (+) --> delay_line --> output
///            ^                    |
///            |   damping_filter   |
///            +--- feedback * -----+
/// ```
#[derive(Debug, Clone)]
pub(crate) struct CombFilter {
    delay_line: DelayLine,
    delay_samples: usize,
    feedback: f32,
    damping_coeff: f32,
    damping_state: f32,
}

impl CombFilter {
    /// Create a new comb filter with pre-allocated delay line.
    #[must_use]
    fn new() -> Self {
        Self {
            delay_line: DelayLine::new(MAX_DELAY_SAMPLES),
            delay_samples: 1,
            feedback: 0.0,
            damping_coeff: 0.3,
            damping_state: 0.0,
        }
    }

    /// Update the comb filter parameters for a given room dimension.
    fn update(&mut self, dimension: f32, absorption: f32, sample_rate: f32) {
        let mode_freq = SPEED_OF_SOUND / (2.0 * dimension);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let raw_delay = (sample_rate / mode_freq) as usize;
        self.delay_samples = raw_delay.clamp(1, self.delay_line.len() - 1);
        self.feedback = 0.85 * (1.0 - absorption * 0.5);
        self.damping_coeff = 0.3 + absorption * 0.4;
    }

    /// Process a single sample through the comb filter.
    #[inline]
    #[must_use]
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.delay_line.read(self.delay_samples);
        // One-pole low-pass in the feedback path for HF absorption
        self.damping_state += self.damping_coeff * (delayed - self.damping_state);
        let feedback_signal = self.damping_state * self.feedback;
        self.delay_line.write(input + feedback_signal);
        delayed
    }

    /// Clear all internal state.
    fn clear(&mut self) {
        self.delay_line.clear();
        self.damping_state = 0.0;
    }
}

impl Default for CombFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Bank of axial room mode comb filters.
///
/// Models three axial standing waves (one per room dimension) as parallel
/// comb filters. The `amount` parameter controls how much of the modes
/// signal is mixed into the output.
#[derive(Debug, Clone)]
#[must_use]
pub struct RoomModeBank {
    modes: [CombFilter; NUM_MODES],
    amount: f32,
}

impl RoomModeBank {
    /// Create a new room mode bank with pre-allocated delay lines.
    ///
    /// All modes are initially silent with zero amount.
    pub fn new() -> Self {
        Self {
            modes: [CombFilter::new(), CombFilter::new(), CombFilter::new()],
            amount: 0.0,
        }
    }

    /// Recalculate mode frequencies from room dimensions.
    ///
    /// # Arguments
    ///
    /// * `room_length` - Length of the room in meters (x-axis).
    /// * `room_width` - Width of the room in meters (y-axis).
    /// * `room_height` - Height of the room in meters (z-axis).
    /// * `absorption` - Average wall absorption coefficient (0.0 to 1.0).
    /// * `sample_rate` - Current sample rate in Hz.
    pub fn update_geometry(
        &mut self,
        room_length: f32,
        room_width: f32,
        room_height: f32,
        absorption: f32,
        sample_rate: f32,
    ) {
        let dimensions = [room_length, room_width, room_height];
        for (mode, &dim) in self.modes.iter_mut().zip(dimensions.iter()) {
            mode.update(dim, absorption, sample_rate);
        }
    }

    /// Process a single input sample, returns the modes contribution (mono).
    ///
    /// The input is fed into all three comb filters in parallel and the
    /// outputs are summed and scaled by `amount`.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32) -> f32 {
        if self.amount <= 0.0 {
            // Still advance delay lines to keep state consistent
            for mode in &mut self.modes {
                mode.delay_line.write(input);
            }
            return 0.0;
        }

        let mut sum = 0.0;
        for mode in &mut self.modes {
            sum += mode.process(input);
        }

        // Scale by 1/NUM_MODES to normalize and then by amount
        sum * self.amount / NUM_MODES as f32
    }

    /// Set the amount (wet mix level for modes).
    ///
    /// * `0.0` = modes are completely off.
    /// * `1.0` = full modes contribution.
    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
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

    #[test]
    fn test_default_is_silent() {
        let mut bank = RoomModeBank::new();
        // With amount=0, output should always be 0
        let out = bank.process(1.0);
        assert!((out).abs() < f32::EPSILON);
    }

    #[test]
    fn test_update_geometry_sets_delay() {
        let mut bank = RoomModeBank::new();
        let sample_rate = 48000.0;
        bank.update_geometry(8.0, 5.0, 3.0, 0.1, sample_rate);

        // Length mode: 343/(2*8) = 21.4375 Hz -> 48000/21.4375 ≈ 2239 samples
        let expected_length = (sample_rate / (SPEED_OF_SOUND / 16.0)) as usize;
        assert_eq!(bank.modes[0].delay_samples, expected_length);

        // Width mode: 343/(2*5) = 34.3 Hz -> 48000/34.3 ≈ 1399 samples
        let expected_width = (sample_rate / (SPEED_OF_SOUND / 10.0)) as usize;
        assert_eq!(bank.modes[1].delay_samples, expected_width);

        // Height mode: 343/(2*3) ≈ 57.167 Hz -> 48000/57.167 ≈ 839 samples
        let expected_height = (sample_rate / (SPEED_OF_SOUND / 6.0)) as usize;
        assert_eq!(bank.modes[2].delay_samples, expected_height);
    }

    #[test]
    fn test_feedback_scales_with_absorption() {
        let mut bank = RoomModeBank::new();

        bank.update_geometry(8.0, 5.0, 3.0, 0.0, 48000.0);
        let fb_no_absorption = bank.modes[0].feedback;

        bank.update_geometry(8.0, 5.0, 3.0, 1.0, 48000.0);
        let fb_full_absorption = bank.modes[0].feedback;

        // More absorption should give less feedback
        assert!(fb_no_absorption > fb_full_absorption);
        // Zero absorption: 0.85 * (1.0 - 0.0) = 0.85
        assert!((fb_no_absorption - 0.85).abs() < f32::EPSILON);
        // Full absorption: 0.85 * (1.0 - 0.5) = 0.425
        assert!((fb_full_absorption - 0.425).abs() < f32::EPSILON);
    }

    #[test]
    fn test_damping_coeff_scales_with_absorption() {
        let mut bank = RoomModeBank::new();

        bank.update_geometry(8.0, 5.0, 3.0, 0.0, 48000.0);
        let damp_no_absorption = bank.modes[0].damping_coeff;

        bank.update_geometry(8.0, 5.0, 3.0, 1.0, 48000.0);
        let damp_full_absorption = bank.modes[0].damping_coeff;

        // 0.3 + 0.0 * 0.4 = 0.3
        assert!((damp_no_absorption - 0.3).abs() < f32::EPSILON);
        // 0.3 + 1.0 * 0.4 = 0.7
        assert!((damp_full_absorption - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn test_process_produces_output() {
        let mut bank = RoomModeBank::new();
        bank.update_geometry(8.0, 5.0, 3.0, 0.1, 48000.0);
        bank.set_amount(1.0);

        // Feed an impulse
        let first_out = bank.process(1.0);
        // First sample: delay lines are empty, so output is 0
        assert!((first_out).abs() < f32::EPSILON);

        // Process silence until the shortest delay echoes back
        // Height mode ≈ 839 samples
        let height_delay = bank.modes[2].delay_samples;
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
        bank.update_geometry(8.0, 5.0, 3.0, 0.1, 48000.0);
        bank.set_amount(1.0);

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
        bank.set_amount(2.0);
        assert!((bank.amount - 1.0).abs() < f32::EPSILON);

        bank.set_amount(-0.5);
        assert!((bank.amount).abs() < f32::EPSILON);
    }

    #[test]
    fn test_delay_clamped_to_buffer_size() {
        let mut bank = RoomModeBank::new();
        // Very large room: 100m -> 343/(200) ≈ 1.715 Hz -> 48000/1.715 ≈ 27988
        // This exceeds MAX_DELAY_SAMPLES and should be clamped
        bank.update_geometry(100.0, 100.0, 100.0, 0.1, 48000.0);
        for mode in &bank.modes {
            assert!(mode.delay_samples < MAX_DELAY_SAMPLES);
        }
    }

    #[test]
    fn test_comb_filter_energy_decays() {
        let mut comb = CombFilter::new();
        comb.update(5.0, 0.3, 48000.0);

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
}
