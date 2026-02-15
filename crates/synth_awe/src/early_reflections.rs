//! Image Source Method (ISM) early reflections for a rectangular room.
//!
//! Computes first-order reflections from six walls of a box-shaped room.
//! Each wall produces one mirror source; the delay and gain of each tap
//! are derived from the distance between the listener and the mirror source.

use synth_dsp::InterpolatedDelayLine;

use crate::room::SPEED_OF_SOUND;

/// Number of first-order reflection taps (one per wall face).
const MAX_EARLY_TAPS: usize = 6;

/// Maximum delay in seconds for ISM calculations.
/// Covers rooms up to roughly 30 m diagonal at 96 kHz.
const MAX_DELAY_SECONDS: f32 = 0.15;

/// Maximum delay line size in samples (0.15 s at 96 kHz).
const MAX_DELAY_SAMPLES: usize = 14_400;

/// Minimum distance (meters) to prevent infinite gain when source
/// and listener collapse to the same position as a mirror.
const MIN_DISTANCE: f32 = 0.1;

/// A single early reflection tap with per-wall delay, stereo gain,
/// and a one-pole lowpass for frequency-dependent absorption.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EarlyTap {
    /// Fractional delay in samples for this reflection.
    delay_samples: f32,
    /// Left channel gain (includes distance attenuation, absorption, and pan).
    gain_left: f32,
    /// Right channel gain.
    gain_right: f32,
    /// One-pole lowpass coefficient for high-frequency damping (0..1).
    damping_coeff: f32,
    /// Filter state for the one-pole lowpass.
    filter_state: f32,
}

impl EarlyTap {
    const SILENT: Self = Self {
        delay_samples: 1.0,
        gain_left: 0.0,
        gain_right: 0.0,
        damping_coeff: 0.3,
        filter_state: 0.0,
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
}

impl EarlyReflections {
    /// Create a new early reflections processor with pre-allocated buffers.
    pub fn new() -> Self {
        Self {
            taps: [EarlyTap::SILENT; MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(MAX_DELAY_SAMPLES),
        }
    }

    /// Create with a custom maximum delay line size (in samples).
    ///
    /// Use this for per-voice instances where the default 14400 samples
    /// may be too large or too small.
    pub fn with_max_delay(max_samples: usize) -> Self {
        Self {
            taps: [EarlyTap::SILENT; MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(max_samples),
        }
    }

    /// Recalculate tap parameters from room geometry and positions.
    ///
    /// Call this when room dimensions, source/listener positions, or
    /// material absorption change -- not per sample.
    ///
    /// # Arguments
    ///
    /// * `room_length` - Room extent along x-axis (meters).
    /// * `room_width`  - Room extent along y-axis (meters).
    /// * `room_height` - Room extent along z-axis (meters).
    /// * `source_pos`  - Sound source position `[x, y, z]` in meters.
    /// * `listener_pos` - Listener position `[x, y, z]` in meters.
    /// * `absorption`  - Average material absorption coefficient (0.0--1.0).
    /// * `sample_rate`  - Current sample rate in Hz.
    #[allow(clippy::too_many_arguments)]
    pub fn update_geometry(
        &mut self,
        room_length: f32,
        room_width: f32,
        room_height: f32,
        source_pos: [f32; 3],
        listener_pos: [f32; 3],
        absorption: f32,
        sample_rate: f32,
    ) {
        let [sx, sy, sz] = source_pos;
        let [lx, ly, lz] = listener_pos;

        // Mirror source positions for each of the six walls.
        //
        // Room spans (0,0,0) to (length, width, height).
        // Wall +x (x = L): mirror_x = 2L - sx
        // Wall -x (x = 0): mirror_x = -sx
        // Wall +y (y = W): mirror_y = 2W - sy
        // Wall -y (y = 0): mirror_y = -sy
        // Wall +z (z = H): mirror_z = 2H - sz
        // Wall -z (z = 0): mirror_z = -sz
        let mirrors: [[f32; 3]; MAX_EARLY_TAPS] = [
            [2.0 * room_length - sx, sy, sz], // +x wall
            [-sx, sy, sz],                    // -x wall
            [sx, 2.0 * room_width - sy, sz],  // +y wall
            [sx, -sy, sz],                    // -y wall
            [sx, sy, 2.0 * room_height - sz], // +z wall
            [sx, sy, -sz],                    // -z wall
        ];

        let max_delay = MAX_DELAY_SECONDS * sample_rate;
        let reflection_coeff = 1.0 - absorption.clamp(0.0, 0.99);
        let damping = 0.3 + absorption.clamp(0.0, 1.0) * 0.5;

        for i in 0..MAX_EARLY_TAPS {
            let [mx, my, mz] = mirrors[i];
            let dx = lx - mx;
            let dy = ly - my;
            let dz = lz - mz;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt().max(MIN_DISTANCE);

            let delay = (distance / SPEED_OF_SOUND) * sample_rate;
            let delay_clamped = delay.clamp(1.0, max_delay);

            let total_gain = (1.0 / distance) * reflection_coeff;

            // Pan based on X-axis offset between mirror source and listener.
            let pan = if room_length > 0.0 {
                ((mx - lx) / room_length).clamp(-1.0, 1.0)
            } else {
                0.0
            };

            self.taps[i].delay_samples = delay_clamped;
            self.taps[i].gain_left = (1.0 - pan) * 0.5 * total_gain;
            self.taps[i].gain_right = (1.0 + pan) * 0.5 * total_gain;
            self.taps[i].damping_coeff = damping;
            // Preserve filter_state across geometry updates for smooth transitions.
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
            let raw = self.delay_line.read_interpolated(tap.delay_samples);

            // One-pole lowpass: y[n] = (1 - c) * x[n] + c * y[n-1]
            let filtered = (1.0 - tap.damping_coeff) * raw + tap.damping_coeff * tap.filter_state;
            tap.filter_state = filtered;

            left += filtered * tap.gain_left;
            right += filtered * tap.gain_right;
        }

        (left, right)
    }

    /// Clear all delay line contents and reset filter states.
    pub fn clear(&mut self) {
        self.delay_line.clear();
        for tap in &mut self.taps {
            tap.filter_state = 0.0;
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
        let sample_rate = 48000.0;

        // Room 8x5x3 m, source at center, listener offset.
        er.update_geometry(
            8.0,
            5.0,
            3.0,
            [4.0, 2.5, 1.5],
            [2.0, 2.5, 1.5],
            0.2,
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
        let sample_rate = 48000.0;

        // Source and listener both on the x-axis centerline.
        er.update_geometry(
            10.0,
            6.0,
            3.0,
            [5.0, 3.0, 1.5],
            [5.0, 3.0, 1.5],
            0.1,
            sample_rate,
        );

        // For a symmetric room with source == listener at center,
        // +x and -x mirrors should pan opposite but have equal magnitude,
        // and same for +y/-y and +z/-z.
        let tap_plus_x = &er.taps[0];
        let tap_minus_x = &er.taps[1];

        // Gains should be symmetric: left of +x == right of -x.
        assert!(
            (tap_plus_x.gain_left - tap_minus_x.gain_right).abs() < 1e-5,
            "Expected symmetric panning for +x/-x taps"
        );
        assert!(
            (tap_plus_x.gain_right - tap_minus_x.gain_left).abs() < 1e-5,
            "Expected symmetric panning for +x/-x taps"
        );
    }

    #[test]
    fn test_clear_resets_state() {
        let mut er = EarlyReflections::new();
        er.update_geometry(
            8.0,
            5.0,
            3.0,
            [4.0, 2.5, 1.5],
            [2.0, 2.5, 1.5],
            0.2,
            48000.0,
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
            8.0,
            5.0,
            3.0,
            [4.0, 2.5, 1.5],
            [2.0, 2.5, 1.5],
            0.1,
            48000.0,
        );
        er_high.update_geometry(
            8.0,
            5.0,
            3.0,
            [4.0, 2.5, 1.5],
            [2.0, 2.5, 1.5],
            0.9,
            48000.0,
        );

        // Higher absorption should yield higher damping coefficient.
        assert!(er_high.taps[0].damping_coeff > er_low.taps[0].damping_coeff);
    }

    #[test]
    fn test_delay_increases_with_distance() {
        let mut er_near = EarlyReflections::new();
        let mut er_far = EarlyReflections::new();

        // Listener near the +x wall.
        er_near.update_geometry(
            10.0,
            6.0,
            3.0,
            [5.0, 3.0, 1.5],
            [9.0, 3.0, 1.5],
            0.2,
            48000.0,
        );
        // Listener far from the +x wall.
        er_far.update_geometry(
            10.0,
            6.0,
            3.0,
            [5.0, 3.0, 1.5],
            [1.0, 3.0, 1.5],
            0.2,
            48000.0,
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
            2.0,
            2.0,
            2.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            0.0,
            48000.0,
        );

        for tap in &er.taps {
            assert!(
                tap.gain_left.is_finite(),
                "Left gain must be finite, got {}",
                tap.gain_left
            );
            assert!(
                tap.gain_right.is_finite(),
                "Right gain must be finite, got {}",
                tap.gain_right
            );
        }
    }
}
