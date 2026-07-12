//! Image Source Method (ISM) early reflections for a rectangular room.
//!
//! Ported from the retired AWE monolith into the per-voice `SpatialPanner`.
//! Computes first-order reflections from the six walls of a box-shaped room.
//! Each wall produces one mirror source; the delay and gain of each tap are
//! derived from the distance between the listener and the mirror source.
//!
//! Because `x`/`y`/`z` are per-voice modulation targets in `SpatialPanner`, the
//! source can move continuously (e.g. an orbiting LFO/script). `update_geometry`
//! recomputes each tap's delay + gains once per block; applying those as
//! block-constant values makes a *moving* source **step** at block boundaries
//! (gain zipper + delay jumps across all six taps). So — mirroring the direct
//! path in [`super::spatializer`] — each tap's delay and gains are **smoothed
//! per sample** toward their block targets. A static source converges and the
//! smoothers go inert, so a fixed placement costs a single MAC per tap.

use synth_core::{
    BipolarValue, FilterState, NormalizedValue, OnePoleSmooth, SampleCount, SampleRate, Seconds,
};
use synth_core::{Meters, MetersPerSecond, Position3, SampleOffset};
use synth_dsp::InterpolatedDelayLine;

/// Number of first-order reflection taps (one per wall face).
const MAX_EARLY_TAPS: usize = 6;

/// Maximum delay in seconds for ISM calculations.
const MAX_DELAY_SECONDS: Seconds = Seconds::new(1.0);

/// Maximum per-tap jitter in seconds for diffusion.
const MAX_JITTER_SECONDS: Seconds = Seconds::from_millis(3.0);

/// Time constant for smoothing per-tap delay + gains (~5 ms), matching the
/// direct-path spatializer so both halves glide at the same rate on movement.
const SMOOTH_SECONDS: Seconds = Seconds::new(0.005);

/// Deterministic jitter pattern per tap (scaled by diffusion).
const JITTER_PATTERN: [f32; MAX_EARLY_TAPS] = [-0.9, 0.7, -0.4, 1.0, -0.6, 0.3];

/// Minimum distance (meters) to prevent infinite gain when source and
/// listener collapse to the same position as a mirror.
const MIN_DISTANCE: Meters = Meters::new(0.1);

/// A single early reflection tap with per-wall delay, stereo gain, and
/// frequency-dependent absorption filters (LP for HF, HP for LF).
///
/// `target_*` are recomputed per block by [`EarlyReflections::update_geometry`];
/// the `*_smooth` one-poles glide the actually-applied values toward them each
/// sample so a moving source does not step at block boundaries.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EarlyTap {
    /// Target fractional delay in samples for this reflection.
    target_delay: SampleOffset,
    /// Target left-channel gain (distance attenuation, absorption, and pan).
    target_gain_left: f32,
    /// Target right-channel gain.
    target_gain_right: f32,
    /// One-pole lowpass coefficient for high-frequency damping (0..1).
    lp_coeff: NormalizedValue,
    /// One-pole highpass coefficient for low-frequency absorption.
    hp_coeff: NormalizedValue,
    /// Per-sample smoother for the fractional read delay (also yields a natural
    /// Doppler on movement).
    delay_smooth: OnePoleSmooth,
    /// Per-sample smoother for the left-channel gain (kills block zipper).
    gain_left_smooth: OnePoleSmooth,
    /// Per-sample smoother for the right-channel gain.
    gain_right_smooth: OnePoleSmooth,
    /// Filter state for the one-pole lowpass.
    lp_state: FilterState,
    /// Filter state for the one-pole highpass.
    hp_state: FilterState,
}

impl EarlyTap {
    /// A silent tap: unit read delay, zero gain, smoothers pre-seeded to match.
    fn silent() -> Self {
        let mut delay_smooth = OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY);
        delay_smooth.set(1.0);
        Self {
            target_delay: SampleOffset::new(1.0),
            target_gain_left: 0.0,
            target_gain_right: 0.0,
            lp_coeff: NormalizedValue::new(0.3),
            hp_coeff: NormalizedValue::new(0.997),
            delay_smooth,
            // OnePoleSmooth defaults to state 0.0, matching the zero gains.
            gain_left_smooth: OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY),
            gain_right_smooth: OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY),
            lp_state: FilterState::ZERO,
            hp_state: FilterState::ZERO,
        }
    }
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
    /// Snap (instead of glide) the tap smoothers on the first `update_geometry`
    /// after construction / a reset, so note-on lands at the right geometry
    /// without a startup Doppler swoop. Cleared by [`Self::clear`].
    initialized: bool,
}

impl EarlyReflections {
    /// Create with a given maximum delay-line size (in samples).
    pub(crate) fn with_max_delay(max_samples: SampleCount) -> Self {
        Self {
            taps: [EarlyTap::silent(); MAX_EARLY_TAPS],
            delay_line: InterpolatedDelayLine::new(max_samples.as_usize()),
            initialized: false,
        }
    }

    /// Recalculate tap parameters from room geometry and positions.
    ///
    /// Call when room dimensions, source/listener positions, or material
    /// absorption change — not per sample. Writes each tap's *targets*; the
    /// per-sample smoothers in [`Self::process`] glide toward them (or snap, on
    /// the first update after a reset).
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

        // Snap (not glide) on the first update after construction / reset.
        let snap = !self.initialized;

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

            let tap = &mut self.taps[i];
            tap.target_delay = SampleOffset::new(delay_clamped);
            tap.target_gain_left = (1.0 - pan.as_f32()) * 0.5 * total_gain;
            tap.target_gain_right = (1.0 + pan.as_f32()) * 0.5 * total_gain;
            tap.lp_coeff = tap_lp;
            tap.hp_coeff = hp_damping;
            // Filter states are preserved across updates for smooth transitions.

            // Keep the smoothing time correct if the sample rate changed.
            tap.delay_smooth.set_time(SMOOTH_SECONDS, sample_rate);
            tap.gain_left_smooth.set_time(SMOOTH_SECONDS, sample_rate);
            tap.gain_right_smooth.set_time(SMOOTH_SECONDS, sample_rate);

            // First update after a reset: snap so note-on lands at the right
            // geometry; afterwards the per-sample smoothers glide.
            if snap {
                tap.delay_smooth.set(tap.target_delay.as_f32());
                tap.gain_left_smooth.set(tap.target_gain_left);
                tap.gain_right_smooth.set(tap.target_gain_right);
            }
        }
        self.initialized = true;
    }

    /// Process a single mono input sample and return stereo early reflections.
    ///
    /// Each tap glides its delay and gains one step toward the block targets
    /// (via [`OnePoleSmooth`]) before reading, so a moving source produces
    /// continuous motion instead of block-boundary steps.
    #[inline]
    pub(crate) fn process(&mut self, input: f32) -> (f32, f32) {
        self.delay_line.write(input);

        let mut left = 0.0_f32;
        let mut right = 0.0_f32;

        for tap in &mut self.taps {
            let delay = tap.delay_smooth.process(tap.target_delay.as_f32());
            let gain_left = tap.gain_left_smooth.process(tap.target_gain_left);
            let gain_right = tap.gain_right_smooth.process(tap.target_gain_right);

            let raw = self.delay_line.read_interpolated(delay);
            // LP removes highs (material HF absorption); HP removes lows (LF absorption).
            let lp_out = tap.lp_state.one_pole(raw, tap.lp_coeff.as_f32());
            let filtered = tap.hp_state.one_pole_hp(lp_out, tap.hp_coeff.as_f32());
            left += gain_left * filtered;
            right += gain_right * filtered;
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
        // Next update_geometry snaps the tap smoothers instead of gliding.
        self.initialized = false;
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
            assert!(tap.target_gain_left.is_finite());
            assert!(tap.target_gain_right.is_finite());
        }
    }

    #[test]
    fn moving_source_gain_glides_not_steps() {
        // A large jump in source position must not land the applied gain on its
        // new target in a single sample — proves per-sample smoothing.
        let mut er = EarlyReflections::with_max_delay(MAX_DELAY);
        geo(&mut er, pos(1.0, 2.5, 1.5), pos(4.0, 2.5, 1.5));
        // Settle the smoothers at the first position.
        for _ in 0..4096 {
            er.process(0.0);
        }
        let before: Vec<f32> = er
            .taps
            .iter()
            .map(|t| t.gain_left_smooth.current())
            .collect();
        // Jump the source to the opposite side and advance a single sample.
        geo(&mut er, pos(7.0, 2.5, 1.5), pos(4.0, 2.5, 1.5));
        er.process(1.0);
        let mut glided = false;
        for (i, tap) in er.taps.iter().enumerate() {
            let target = tap.target_gain_left;
            let now = tap.gain_left_smooth.current();
            // Still short of target after one step, and it moved from `before`.
            if (now - target).abs() > 1e-4 && (now - before[i]).abs() > 1e-6 {
                glided = true;
            }
        }
        assert!(
            glided,
            "tap gains should glide toward the new target, not snap"
        );
    }
}
