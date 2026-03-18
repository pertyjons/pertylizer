//! Image Source Method (ISM) early reflections for a rectangular room.
//!
//! Computes first-order reflections from six walls of a box-shaped room.
//! Each wall produces one mirror source; the delay and gain of each tap
//! are derived from the distance between the listener and the mirror source.

use synth_core::{
    BipolarValue, FilterState, Gain, NormalizedValue, SampleCount, SampleRate, Seconds,
};
use synth_dsp::InterpolatedDelayLine;

use crate::types::{Meters, MetersPerSecond, Position3, SampleOffset};

/// Number of first-order reflection taps (one per wall face).
const MAX_EARLY_TAPS: usize = 6;

/// Maximum delay in seconds for ISM calculations.
/// Covers rooms up to ~170 m diagonal (mirror source at ~340 m / 343 m/s ≈ 1 s).
const MAX_DELAY_SECONDS: Seconds = Seconds::new(1.0);

/// Maximum delay line size in samples (1.0 s at 96 kHz).
const MAX_DELAY_SAMPLES: SampleCount = SampleCount::new(96_000);

/// Maximum per-tap jitter in seconds for diffusion.
const MAX_JITTER_SECONDS: Seconds = Seconds::from_millis(3.0);

/// Deterministic jitter pattern per tap (scaled by diffusion).
const JITTER_PATTERN: [f32; MAX_EARLY_TAPS] = [-0.9, 0.7, -0.4, 1.0, -0.6, 0.3];

/// Minimum distance (meters) to prevent infinite gain when source
/// and listener collapse to the same position as a mirror.
const MIN_DISTANCE: Meters = Meters::new(0.1);

/// A single early reflection tap with per-wall delay, stereo gain,
/// and frequency-dependent absorption filters (LP for HF, HP for LF).
#[derive(Debug, Clone, Copy)]
pub(crate) struct EarlyTap {
    /// Fractional delay in samples for this reflection.
    delay_samples: SampleOffset,
    /// Left channel gain (includes distance attenuation, absorption, and pan).
    gain_left: Gain,
    /// Right channel gain.
    gain_right: Gain,
    /// One-pole lowpass coefficient for high-frequency damping (0..1).
    lp_coeff: NormalizedValue,
    /// One-pole highpass coefficient for low-frequency absorption.
    hp_coeff: NormalizedValue,
    /// Filter state for the one-pole lowpass.
    lp_state: FilterState,
    /// Filter state for the one-pole highpass.
    hp_state: FilterState,
}

impl EarlyTap {
    const SILENT: Self = Self {
        delay_samples: SampleOffset::new(1.0),
        gain_left: Gain::MUTE,
        gain_right: Gain::MUTE,
        lp_coeff: NormalizedValue::new_unchecked(0.3),
        hp_coeff: NormalizedValue::new_unchecked(0.997),
        lp_state: FilterState::ZERO,
        hp_state: FilterState::ZERO,
    };
}

impl Default for EarlyTap {
    fn default() -> Self {
        Self::SILENT
    }
}

/// ISM early reflections processor.
///
/// Maintains a single shared mono [`InterpolatedDelayLine`] and six taps
/// corresponding to first-order mirror reflections from the six walls of
/// a rectangular room.
#[must_use]
pub struct EarlyReflections {
    taps: [EarlyTap; MAX_EARLY_TAPS],
    delay_line: InterpolatedDelayLine,
    /// Air absorption amount (0.0 = off, 1.0 = max).
    air_absorption: NormalizedValue,
}

impl EarlyReflections {
    /// Create a new early reflections processor with pre-allocated buffers.
    pub fn new() -> Self {
        Self {
            taps: [EarlyTap::SILENT; MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(MAX_DELAY_SAMPLES.as_usize()),
            air_absorption: NormalizedValue::MIN,
        }
    }

    /// Create with a custom maximum delay line size (in samples).
    ///
    /// Use this for per-voice instances where the default 96000 samples
    /// may be too large or too small.
    pub fn with_max_delay(max_samples: SampleCount) -> Self {
        Self {
            taps: [EarlyTap::SILENT; MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(max_samples.as_usize()),
            air_absorption: NormalizedValue::MIN,
        }
    }

    /// Set the air absorption amount.
    pub fn set_air_absorption(&mut self, air_absorption: NormalizedValue) {
        self.air_absorption = air_absorption;
    }

    /// Recalculate tap parameters from room geometry and positions.
    ///
    /// Call this when room dimensions, source/listener positions, or
    /// material absorption change -- not per sample.
    ///
    /// # Arguments
    ///
    /// * `room_length`     - Room extent along x-axis (meters).
    /// * `room_width`      - Room extent along y-axis (meters).
    /// * `room_height`     - Room extent along z-axis (meters).
    /// * `source_pos`      - Sound source position `[x, y, z]` in meters.
    /// * `listener_pos`    - Listener position `[x, y, z]` in meters.
    /// * `absorption_low`  - Low-frequency absorption coefficient (0.0--1.0).
    /// * `absorption_mid`  - Mid-frequency absorption coefficient (0.0--1.0).
    /// * `absorption_high` - High-frequency absorption coefficient (0.0--1.0).
    /// * `diffusion`       - Material diffusion (0.0--1.0).
    /// * `sample_rate`     - Current sample rate in Hz.
    #[allow(clippy::too_many_arguments)]
    pub fn update_geometry(
        &mut self,
        room_length: Meters,
        room_width: Meters,
        room_height: Meters,
        source_pos: Position3,
        listener_pos: Position3,
        absorption_low: NormalizedValue,
        absorption_mid: NormalizedValue,
        absorption_high: NormalizedValue,
        diffusion: NormalizedValue,
        air_absorption: NormalizedValue,
        speed_of_sound: MetersPerSecond,
        sample_rate: SampleRate,
    ) {
        self.air_absorption = air_absorption;
        let [sx, sy, sz] = source_pos.as_f32();
        let [lx, ly, lz] = listener_pos.as_f32();

        // Mirror source positions for each of the six walls.
        //
        // Room spans (0,0,0) to (length, width, height).
        // Wall +x (x = L): mirror_x = 2L - sx
        // Wall -x (x = 0): mirror_x = -sx
        // Wall +y (y = W): mirror_y = 2W - sy
        // Wall -y (y = 0): mirror_y = -sy
        // Wall +z (z = H): mirror_z = 2H - sz
        // Wall -z (z = 0): mirror_z = -sz
        let room_length_f = room_length.as_f32();
        let room_width_f = room_width.as_f32();
        let room_height_f = room_height.as_f32();
        let mirrors: [[f32; 3]; MAX_EARLY_TAPS] = [
            [2.0 * room_length_f - sx, sy, sz], // +x wall
            [-sx, sy, sz],                      // -x wall
            [sx, 2.0 * room_width_f - sy, sz],  // +y wall
            [sx, -sy, sz],                      // -y wall
            [sx, sy, 2.0 * room_height_f - sz], // +z wall
            [sx, sy, -sz],                      // -z wall
        ];

        // Mid-weighted average for overall reflection energy
        let weighted_avg = absorption_low.as_f32() * 0.2
            + absorption_mid.as_f32() * 0.5
            + absorption_high.as_f32() * 0.3;

        let max_delay = SampleOffset::new(MAX_DELAY_SECONDS.as_f32() * sample_rate.as_f32());
        let reflection_coeff = NormalizedValue::new(1.0 - weighted_avg.clamp(0.0, 0.99));
        let jitter_max = SampleOffset::new(MAX_JITTER_SECONDS.as_f32() * sample_rate.as_f32());

        // Frequency-dependent damping: sqrt() mapping for perceptual spread
        let abs_high_eff = absorption_high.as_f32().sqrt();
        let abs_low_eff = absorption_low.as_f32().sqrt();
        let lp_damping = NormalizedValue::new((0.15 + abs_high_eff * 0.75).clamp(0.0, 0.999));
        let hp_damping = NormalizedValue::new((0.997 - abs_low_eff * 0.40).clamp(0.0, 0.999));

        for i in 0..MAX_EARLY_TAPS {
            let [mx, my, mz] = mirrors[i];
            let dx = lx - mx;
            let dy = ly - my;
            let dz = lz - mz;
            let distance = Meters::new((dx * dx + dy * dy + dz * dz).sqrt()).max(MIN_DISTANCE);

            let jitter = JITTER_PATTERN[i] * diffusion.as_f32() * jitter_max.as_f32();
            let delay =
                (distance.as_f32() / speed_of_sound.as_f32()) * sample_rate.as_f32() + jitter;
            let delay_clamped = delay.clamp(1.0, max_delay.as_f32());

            let total_gain = ((1.0 / distance.as_f32()) * reflection_coeff.as_f32()).min(2.0);

            // Pan based on X-axis offset between mirror source and listener.
            let mut pan = if room_length_f > 0.0 {
                BipolarValue::new((mx - lx) / room_length_f)
            } else {
                BipolarValue::CENTER
            };
            // Diffusion reduces directional cues.
            pan = BipolarValue::new(pan.as_f32() * (1.0 - diffusion.as_f32() * 0.6));

            // Boost LP damping proportional to air_absorption and tap distance
            let air_lp_boost = air_absorption.as_f32() * (distance.as_f32() / 20.0).min(1.0) * 0.3;
            let tap_lp =
                NormalizedValue::new((lp_damping.as_f32() + air_lp_boost).clamp(0.0, 0.999));

            self.taps[i].delay_samples = SampleOffset::new(delay_clamped);
            self.taps[i].gain_left = Gain::new((1.0 - pan.as_f32()) * 0.5 * total_gain);
            self.taps[i].gain_right = Gain::new((1.0 + pan.as_f32()) * 0.5 * total_gain);
            self.taps[i].lp_coeff = tap_lp;
            self.taps[i].hp_coeff = hp_damping;
            // Preserve filter states across geometry updates for smooth transitions.
        }
    }

    /// Process a single mono input sample and return stereo early reflections.
    ///
    /// The input is written once to the shared delay line; each of the six
    /// taps reads at its own fractional delay position, applies a one-pole
    /// lowpass, and is panned into the stereo output.
    #[inline]
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        // Write the mono input into the shared delay line.
        self.delay_line.write(input);

        let mut left = 0.0_f32;
        let mut right = 0.0_f32;

        for tap in &mut self.taps {
            let raw = self
                .delay_line
                .read_interpolated(tap.delay_samples.as_f32());

            // Frequency-dependent absorption:
            // LP removes high frequencies (material HF absorption)
            let lp_out = tap.lp_state.one_pole(raw, tap.lp_coeff.as_f32());
            // HP removes low frequencies (material LF absorption)
            let filtered = tap.hp_state.one_pole_hp(lp_out, tap.hp_coeff.as_f32());

            left += tap.gain_left.apply(filtered);
            right += tap.gain_right.apply(filtered);
        }

        (left, right)
    }

    /// Clear all delay line contents and reset filter states.
    pub fn clear(&mut self) {
        self.delay_line.clear();
        for tap in &mut self.taps {
            tap.lp_state.reset();
            tap.hp_state.reset();
        }
    }
}

impl Default for EarlyReflections {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);

    fn pos(x: f32, y: f32, z: f32) -> Position3 {
        Position3::new(Meters::new(x), Meters::new(y), Meters::new(z))
    }

    #[test]
    fn test_new_produces_silence() {
        let mut er = EarlyReflections::new();
        let (l, r) = er.process(0.0);
        assert!((l).abs() < 1e-10);
        assert!((r).abs() < 1e-10);
    }

    #[test]
    fn test_impulse_response_nonzero() {
        let mut er = EarlyReflections::new();
        let sample_rate = SampleRate::new(48000.0);

        // Room 8x5x3 m, source at center, listener offset.
        er.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            pos(4.0, 2.5, 1.5),
            pos(2.0, 2.5, 1.5),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.2),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            sample_rate,
        );

        // Feed an impulse and run enough samples for the reflections to arrive.
        er.process(1.0);
        let mut found_nonzero = false;
        for _ in 0..14_400 {
            let (l, r) = er.process(0.0);
            if l.abs() > 1e-6 || r.abs() > 1e-6 {
                found_nonzero = true;
                break;
            }
        }
        assert!(
            found_nonzero,
            "Expected at least one non-zero reflection tap"
        );
    }

    #[test]
    fn test_symmetric_source_produces_equal_pan() {
        let mut er = EarlyReflections::new();
        let sample_rate = SampleRate::new(48000.0);

        // Source and listener both on the x-axis centerline.
        er.update_geometry(
            Meters::new(10.0),
            Meters::new(6.0),
            Meters::new(3.0),
            pos(5.0, 3.0, 1.5),
            pos(5.0, 3.0, 1.5),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.15),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            sample_rate,
        );

        // For a symmetric room with source == listener at center,
        // +x and -x mirrors should pan opposite but have equal magnitude,
        // and same for +y/-y and +z/-z.
        let tap_plus_x = &er.taps[0];
        let tap_minus_x = &er.taps[1];

        // Gains should be symmetric: left of +x == right of -x.
        assert!(
            (tap_plus_x.gain_left.as_f32() - tap_minus_x.gain_right.as_f32()).abs() < 1e-5,
            "Expected symmetric panning for +x/-x taps"
        );
        assert!(
            (tap_plus_x.gain_right.as_f32() - tap_minus_x.gain_left.as_f32()).abs() < 1e-5,
            "Expected symmetric panning for +x/-x taps"
        );
    }

    #[test]
    fn test_clear_resets_state() {
        let mut er = EarlyReflections::new();
        er.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            pos(4.0, 2.5, 1.5),
            pos(2.0, 2.5, 1.5),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.2),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        // Feed some signal.
        for _ in 0..1000 {
            er.process(0.5);
        }

        er.clear();

        // After clearing, output should be silent.
        let (l, r) = er.process(0.0);
        assert!(l.abs() < 1e-10);
        assert!(r.abs() < 1e-10);
    }

    #[test]
    fn test_absorption_affects_damping() {
        let mut er_low = EarlyReflections::new();
        let mut er_high = EarlyReflections::new();

        er_low.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            pos(4.0, 2.5, 1.5),
            pos(2.0, 2.5, 1.5),
            NormalizedValue::new(0.05),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        er_high.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            pos(4.0, 2.5, 1.5),
            pos(2.0, 2.5, 1.5),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.5),
            NormalizedValue::new(0.9),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        // Higher HF absorption should yield higher LP damping coefficient.
        assert!(er_high.taps[0].lp_coeff > er_low.taps[0].lp_coeff);
        // Higher LF absorption should yield lower HP coefficient (more bass cut).
        assert!(er_high.taps[0].hp_coeff < er_low.taps[0].hp_coeff);
    }

    #[test]
    fn test_delay_increases_with_distance() {
        let mut er_near = EarlyReflections::new();
        let mut er_far = EarlyReflections::new();

        // Listener near the +x wall.
        er_near.update_geometry(
            Meters::new(10.0),
            Meters::new(6.0),
            Meters::new(3.0),
            pos(5.0, 3.0, 1.5),
            pos(9.0, 3.0, 1.5),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.2),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
        // Listener far from the +x wall.
        er_far.update_geometry(
            Meters::new(10.0),
            Meters::new(6.0),
            Meters::new(3.0),
            pos(5.0, 3.0, 1.5),
            pos(1.0, 3.0, 1.5),
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.2),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        // The +x wall reflection (tap 0) should have shorter delay when
        // the listener is near that wall.
        assert!(
            er_near.taps[0].delay_samples < er_far.taps[0].delay_samples,
            "Expected shorter delay when listener is closer to the +x wall"
        );
    }

    #[test]
    fn test_default_matches_new() {
        let from_new = EarlyReflections::new();
        let from_default = EarlyReflections::default();

        // Both should produce identical silence.
        assert_eq!(
            from_new.taps[0].delay_samples,
            from_default.taps[0].delay_samples
        );
        assert_eq!(from_new.taps[0].gain_left, from_default.taps[0].gain_left);
    }

    #[test]
    fn test_gain_clamped_at_zero_distance() {
        let mut er = EarlyReflections::new();

        // Source directly at a wall corner — mirror is very close to listener.
        // This should not produce infinite gain.
        er.update_geometry(
            Meters::new(2.0),
            Meters::new(2.0),
            Meters::new(2.0),
            pos(0.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );

        for tap in &er.taps {
            assert!(
                tap.gain_left.as_f32().is_finite(),
                "Left gain must be finite, got {}",
                tap.gain_left
            );
            assert!(
                tap.gain_right.as_f32().is_finite(),
                "Right gain must be finite, got {}",
                tap.gain_right
            );
        }
    }
}
