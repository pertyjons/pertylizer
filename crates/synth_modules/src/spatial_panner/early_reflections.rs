//! Image Source Method (ISM) early reflections for a rectangular room.
//!
//! Ported from the retired AWE monolith into the per-voice `SpatialPanner`.
//! Computes first-order reflections from the six walls of a box-shaped room.
//! Each wall produces one mirror source; the delay and gain of each tap are
//! derived from the distance between the listener and the mirror source.

use synth_core::{
    BipolarValue, FilterState, Gain, NormalizedValue, SampleCount, SampleRate, Seconds,
};
use synth_core::{Meters, MetersPerSecond, Position3, SampleOffset};
use synth_dsp::InterpolatedDelayLine;

/// Number of first-order reflection taps (one per wall face).
const MAX_EARLY_TAPS: usize = 6;

/// Maximum delay in seconds for ISM calculations.
const MAX_DELAY_SECONDS: Seconds = Seconds::new(1.0);

/// Maximum per-tap jitter in seconds for diffusion.
const MAX_JITTER_SECONDS: Seconds = Seconds::from_millis(3.0);

/// Deterministic jitter pattern per tap (scaled by diffusion).
const JITTER_PATTERN: [f32; MAX_EARLY_TAPS] = [-0.9, 0.7, -0.4, 1.0, -0.6, 0.3];

/// Minimum distance (meters) to prevent infinite gain when source and
/// listener collapse to the same position as a mirror.
const MIN_DISTANCE: Meters = Meters::new(0.1);

/// A single early reflection tap with per-wall delay, stereo gain, and
/// frequency-dependent absorption filters (LP for HF, HP for LF).
#[derive(Debug, Clone, Copy)]
pub(crate) struct EarlyTap {
    /// Fractional delay in samples for this reflection.
    delay_samples: SampleOffset,
    /// Left channel gain (distance attenuation, absorption, and pan).
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

/// ISM early reflections processor.
///
/// Maintains a single shared mono [`InterpolatedDelayLine`] and six taps
/// corresponding to first-order mirror reflections from the six walls of a
/// rectangular room.
#[derive(Debug, Clone)]
#[must_use]
pub(crate) struct EarlyReflections {
    taps: [EarlyTap; MAX_EARLY_TAPS],
    delay_line: InterpolatedDelayLine,
}

impl EarlyReflections {
    /// Create with a given maximum delay-line size (in samples).
    pub(crate) fn with_max_delay(max_samples: SampleCount) -> Self {
        Self {
            taps: [EarlyTap::SILENT; MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(max_samples.as_usize()),
        }
    }

    /// Recalculate tap parameters from room geometry and positions.
    ///
    /// Call when room dimensions, source/listener positions, or material
    /// absorption change — not per sample.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_geometry(
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
        let [sx, sy, sz] = source_pos.as_f32();
        let [lx, ly, lz] = listener_pos.as_f32();

        // Mirror source positions for each of the six walls.
        // Room spans (0,0,0) to (length, width, height).
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

        // Mid-weighted average for overall reflection energy.
        let weighted_avg = absorption_low.as_f32() * 0.2
            + absorption_mid.as_f32() * 0.5
            + absorption_high.as_f32() * 0.3;

        let max_delay = SampleOffset::new(MAX_DELAY_SECONDS.as_f32() * sample_rate.as_f32());
        let reflection_coeff = NormalizedValue::new(1.0 - weighted_avg.clamp(0.0, 0.99));
        let jitter_max = SampleOffset::new(MAX_JITTER_SECONDS.as_f32() * sample_rate.as_f32());

        // Frequency-dependent damping: sqrt() mapping for perceptual spread.
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

            // Boost LP damping proportional to air_absorption and tap distance.
            let air_lp_boost = air_absorption.as_f32() * (distance.as_f32() / 20.0).min(1.0) * 0.3;
            let tap_lp =
                NormalizedValue::new((lp_damping.as_f32() + air_lp_boost).clamp(0.0, 0.999));

            self.taps[i].delay_samples = SampleOffset::new(delay_clamped);
            self.taps[i].gain_left = Gain::new((1.0 - pan.as_f32()) * 0.5 * total_gain);
            self.taps[i].gain_right = Gain::new((1.0 + pan.as_f32()) * 0.5 * total_gain);
            self.taps[i].lp_coeff = tap_lp;
            self.taps[i].hp_coeff = hp_damping;
            // Filter states are preserved across updates for smooth transitions.
        }
    }

    /// Process a single mono input sample and return stereo early reflections.
    #[inline]
    pub(crate) fn process(&mut self, input: f32) -> (f32, f32) {
        self.delay_line.write(input);

        let mut left = 0.0_f32;
        let mut right = 0.0_f32;

        for tap in &mut self.taps {
            let raw = self
                .delay_line
                .read_interpolated(tap.delay_samples.as_f32());
            // LP removes highs (material HF absorption); HP removes lows (LF absorption).
            let lp_out = tap.lp_state.one_pole(raw, tap.lp_coeff.as_f32());
            let filtered = tap.hp_state.one_pole_hp(lp_out, tap.hp_coeff.as_f32());
            left += tap.gain_left.apply(filtered);
            right += tap.gain_right.apply(filtered);
        }

        (left, right)
    }

    /// Clear all delay line contents and reset filter states.
    pub(crate) fn clear(&mut self) {
        self.delay_line.clear();
        for tap in &mut self.taps {
            tap.lp_state.reset();
            tap.hp_state.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);
    const MAX_DELAY: SampleCount = SampleCount::new(16_384);

    fn pos(x: f32, y: f32, z: f32) -> Position3 {
        Position3::new(Meters::new(x), Meters::new(y), Meters::new(z))
    }

    fn geo(er: &mut EarlyReflections, source: Position3, listener: Position3) {
        er.update_geometry(
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            source,
            listener,
            NormalizedValue::new(0.1),
            NormalizedValue::new(0.2),
            NormalizedValue::new(0.3),
            NormalizedValue::new(0.0),
            NormalizedValue::MIN,
            SPEED_OF_SOUND,
            SampleRate::new(48000.0),
        );
    }

    #[test]
    fn new_produces_silence() {
        let mut er = EarlyReflections::with_max_delay(MAX_DELAY);
        let (l, r) = er.process(0.0);
        assert!(l.abs() < 1e-10 && r.abs() < 1e-10);
    }

    #[test]
    fn impulse_response_nonzero() {
        let mut er = EarlyReflections::with_max_delay(MAX_DELAY);
        geo(&mut er, pos(4.0, 2.5, 1.5), pos(2.0, 2.5, 1.5));
        er.process(1.0);
        let mut found = false;
        for _ in 0..14_400 {
            let (l, r) = er.process(0.0);
            if l.abs() > 1e-6 || r.abs() > 1e-6 {
                found = true;
                break;
            }
        }
        assert!(found, "expected at least one non-zero reflection tap");
    }

    #[test]
    fn clear_resets_state() {
        let mut er = EarlyReflections::with_max_delay(MAX_DELAY);
        geo(&mut er, pos(4.0, 2.5, 1.5), pos(2.0, 2.5, 1.5));
        for _ in 0..1000 {
            er.process(0.5);
        }
        er.clear();
        let (l, r) = er.process(0.0);
        assert!(l.abs() < 1e-10 && r.abs() < 1e-10);
    }

    #[test]
    fn gain_finite_at_zero_distance() {
        let mut er = EarlyReflections::with_max_delay(MAX_DELAY);
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
            assert!(tap.gain_left.as_f32().is_finite());
            assert!(tap.gain_right.as_f32().is_finite());
        }
    }
}
