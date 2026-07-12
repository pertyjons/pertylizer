//! Binaural stereo spatializer using ITD (interaural time difference)
//! and ILD (interaural level difference).
//!
//! Ported from the retired AWE monolith into the per-voice `SpatialPanner`.
//! Uses two interpolated delay lines (one per ear) for ITD, gain-per-ear for
//! ILD, and a one-pole low-pass head-shadow filter on the far ear.
//!
//! Unlike the original AWE version, the ILD gains and head-shadow coefficients
//! are **smoothed per sample** (via [`OnePoleSmooth`]) toward the values
//! computed by [`Spatializer::update`]. Because `x`/`y`/`z` are per-voice
//! modulation targets in `SpatialPanner`, the source can move fast (e.g. an
//! envelope at note-on); applying the recomputed gains as block-constant values
//! would produce audible zipper noise at block boundaries. The ITD needs no
//! smoothing — the interpolated delay already makes delay changes continuous
//! (and yields a natural Doppler on movement).

use synth_core::{
    BipolarValue, FilterState, NormalizedValue, OnePoleSmooth, SampleCount, SampleRate, Seconds,
};
use synth_core::{MetersPerSecond, Position3, SampleOffset};
use synth_dsp::InterpolatedDelayLine;

/// Head width in meters (average distance between human ears).
const HEAD_WIDTH_M: f32 = 0.17;

/// Maximum delay line size in samples.
/// At 96 kHz: 0.17 / 343 * 96000 ~ 47 samples. Pre-allocate 64 for headroom.
const MAX_ITD_SAMPLES: SampleCount = SampleCount::new(64);

/// Time constant for smoothing ILD gains and head-shadow coefficients (~5 ms).
const SMOOTH_SECONDS: Seconds = Seconds::new(0.005);

/// Build a gain smoother initialised at unity (centred, no attenuation).
fn unity_gain_smoother() -> OnePoleSmooth {
    let mut s = OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY);
    s.set(1.0);
    s
}

/// Binaural spatializer using ITD and ILD with per-sample parameter smoothing.
#[derive(Debug, Clone)]
#[must_use]
pub(crate) struct Spatializer {
    delay_left: InterpolatedDelayLine,
    delay_right: InterpolatedDelayLine,
    /// Left-ear ITD in samples (applied directly — delay interpolation smooths it).
    itd_left: SampleOffset,
    /// Right-ear ITD in samples.
    itd_right: SampleOffset,
    /// Target ILD gains (per ear), approached by the smoothers below.
    target_gain_left: f32,
    target_gain_right: f32,
    /// Target head-shadow LP coefficients (per ear).
    target_shadow_left: f32,
    target_shadow_right: f32,
    /// Per-sample smoothers for the ILD gains and head-shadow coefficients.
    gain_left_smooth: OnePoleSmooth,
    gain_right_smooth: OnePoleSmooth,
    shadow_left_smooth: OnePoleSmooth,
    shadow_right_smooth: OnePoleSmooth,
    /// One-pole filter state for the left-ear head shadow.
    shadow_state_left: FilterState,
    /// One-pole filter state for the right-ear head shadow.
    shadow_state_right: FilterState,
}

impl Spatializer {
    /// Create a new spatializer with default (center) positioning.
    pub(crate) fn new() -> Self {
        Self {
            delay_left: InterpolatedDelayLine::new(MAX_ITD_SAMPLES.as_usize()),
            delay_right: InterpolatedDelayLine::new(MAX_ITD_SAMPLES.as_usize()),
            itd_left: SampleOffset::new(0.0),
            itd_right: SampleOffset::new(0.0),
            target_gain_left: 1.0,
            target_gain_right: 1.0,
            target_shadow_left: 0.0,
            target_shadow_right: 0.0,
            gain_left_smooth: unity_gain_smoother(),
            gain_right_smooth: unity_gain_smoother(),
            shadow_left_smooth: OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY),
            shadow_right_smooth: OnePoleSmooth::new(SMOOTH_SECONDS, SampleRate::DVD_QUALITY),
            shadow_state_left: FilterState::ZERO,
            shadow_state_right: FilterState::ZERO,
        }
    }

    /// Recompute ITD targets and ILD/head-shadow targets from source and
    /// listener positions. Call at control rate (block start), never per sample.
    ///
    /// Coordinate convention: x = left/right, y = front/back, z = up/down.
    /// Angle 0 = directly ahead, positive = source to the right.
    pub(crate) fn update(
        &mut self,
        source_pos: Position3,
        listener_pos: Position3,
        speed_of_sound: MetersPerSecond,
        sample_rate: SampleRate,
    ) {
        // Keep the smoothing time correct if the sample rate changed.
        self.gain_left_smooth.set_time(SMOOTH_SECONDS, sample_rate);
        self.gain_right_smooth.set_time(SMOOTH_SECONDS, sample_rate);
        self.shadow_left_smooth
            .set_time(SMOOTH_SECONDS, sample_rate);
        self.shadow_right_smooth
            .set_time(SMOOTH_SECONDS, sample_rate);

        let dx = source_pos.x().as_f32() - listener_pos.x().as_f32();
        let dy = source_pos.y().as_f32() - listener_pos.y().as_f32();

        // Azimuth angle: 0 = directly ahead (positive y), positive = right.
        let angle = dx.atan2(dy);

        // --- ITD --- path-length difference around the head.
        let itd_seconds = HEAD_WIDTH_M / speed_of_sound.as_f32() * angle.sin();
        // Positive angle (source right): left ear is farther, gets more delay.
        self.itd_left = SampleOffset::new(itd_seconds.max(0.0) * sample_rate.as_f32());
        self.itd_right = SampleOffset::new((-itd_seconds).max(0.0) * sample_rate.as_f32());

        // --- ILD --- equal-power panning based on angle.
        let pan = BipolarValue::new(angle.sin());
        self.target_gain_left = (1.0 - pan.as_f32() * 0.5).sqrt();
        self.target_gain_right = (1.0 + pan.as_f32() * 0.5).sqrt();

        // --- Head shadow --- more HF attenuation on the far ear.
        let shadow_amount =
            NormalizedValue::new(0.3 + 0.5 * angle.abs() / std::f32::consts::PI).as_f32();
        if angle >= 0.0 {
            // Source to the right: left ear is shadowed.
            self.target_shadow_left = shadow_amount;
            self.target_shadow_right = 0.0;
        } else {
            self.target_shadow_left = 0.0;
            self.target_shadow_right = shadow_amount;
        }
    }

    /// Process a mono sample into spatialized stereo. Returns `(left, right)`.
    ///
    /// Smooths the ILD gains and head-shadow coefficients one step toward their
    /// targets before applying them, so block-boundary parameter jumps do not
    /// click.
    #[inline]
    #[must_use]
    pub(crate) fn process(&mut self, input: f32) -> (f32, f32) {
        let gain_left = self.gain_left_smooth.process(self.target_gain_left);
        let gain_right = self.gain_right_smooth.process(self.target_gain_right);
        let shadow_left = self.shadow_left_smooth.process(self.target_shadow_left);
        let shadow_right = self.shadow_right_smooth.process(self.target_shadow_right);

        // Write mono input to both delay lines, read back with per-ear ITD.
        self.delay_left.write(input);
        self.delay_right.write(input);
        let left_raw = self.delay_left.read_interpolated(self.itd_left.as_f32());
        let right_raw = self.delay_right.read_interpolated(self.itd_right.as_f32());

        // Head shadow (one-pole LP) per ear, then ILD gain.
        let left_filtered = self.shadow_state_left.one_pole(left_raw, shadow_left);
        let right_filtered = self.shadow_state_right.one_pole(right_raw, shadow_right);

        (gain_left * left_filtered, gain_right * right_filtered)
    }

    /// Clear all internal state (delay buffers and filter states).
    pub(crate) fn clear(&mut self) {
        self.delay_left.clear();
        self.delay_right.clear();
        self.shadow_state_left.reset();
        self.shadow_state_right.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::Meters;

    const SAMPLE_RATE: SampleRate = SampleRate(48_000.0);
    const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);

    fn pos(x: f32, y: f32, z: f32) -> Position3 {
        Position3::new(Meters::new(x), Meters::new(y), Meters::new(z))
    }

    /// Run enough samples that the smoothers settle.
    fn settle(spat: &mut Spatializer) {
        for _ in 0..4096 {
            let _ = spat.process(0.0);
        }
    }

    #[test]
    fn center_source_equal_output() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(0.0, 5.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SAMPLE_RATE,
        );
        assert!((spat.itd_left.as_f32()).abs() < 1e-6);
        assert!((spat.itd_right.as_f32()).abs() < 1e-6);
        assert!((spat.target_gain_left - spat.target_gain_right).abs() < 1e-6);
    }

    #[test]
    fn right_source_more_left_delay() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(5.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SAMPLE_RATE,
        );
        assert!(spat.itd_left.as_f32() > spat.itd_right.as_f32());
        assert!(spat.target_gain_right > spat.target_gain_left);
    }

    #[test]
    fn gains_ramp_toward_target() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(8.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SAMPLE_RATE,
        );
        // One step must not jump straight to the target (proves smoothing).
        let (_l, _r) = spat.process(1.0);
        assert!((spat.gain_right_smooth.current() - spat.target_gain_right).abs() > 1e-4);
        settle(&mut spat);
        assert!((spat.gain_right_smooth.current() - spat.target_gain_right).abs() < 1e-3);
    }

    #[test]
    fn max_itd_within_bounds() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(10.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SampleRate::new(96_000.0),
        );
        assert!(spat.itd_left.as_f32() < MAX_ITD_SAMPLES.as_usize() as f32);
        assert!(spat.itd_right.as_f32() < MAX_ITD_SAMPLES.as_usize() as f32);
    }

    #[test]
    fn clear_resets_state() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(5.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SAMPLE_RATE,
        );
        for _ in 0..50 {
            let _ = spat.process(1.0);
        }
        spat.clear();
        let (l, r) = spat.process(0.0);
        assert!(l.abs() < 1e-10);
        assert!(r.abs() < 1e-10);
    }

    #[test]
    fn head_shadow_far_ear_attenuated() {
        let mut spat = Spatializer::new();
        spat.update(
            pos(10.0, 0.0, 0.0),
            pos(0.0, 0.0, 0.0),
            SPEED_OF_SOUND,
            SAMPLE_RATE,
        );
        assert!(spat.target_shadow_left > spat.target_shadow_right);
    }
}
