//! Gated integrated loudness, per ITU-R BS.1770-4.
//!
//! The V2 comparison harness reports integrated loudness because peak and RMS
//! both answer the wrong question when a change alters the *shape* of a mix
//! rather than its ceiling: two renders can share a peak and an RMS and still
//! differ by several LU, and a listener hears the LU. It lives here rather than
//! in the comparison module because it is an analysis primitive — one signal in,
//! one number out — and the comparison's job is to subtract two of them.
//!
//! # What the standard specifies, and what is derived
//!
//! BS.1770 tabulates its K-weighting coefficients at 48 kHz only. The filter is
//! defined by its *analogue* prototype — a high-frequency shelf followed by an
//! RLB high-pass — so the coefficients here are recomputed from that prototype
//! at whatever rate the signal carries, by the bilinear transform the standard's
//! own tabulation comes from. At 48 kHz they reproduce the published table,
//! including its unnormalized RLB numerator and the small rate-dependent
//! passband gain that comes with it — under a hundredth of a decibel, constant
//! per rate, and therefore cancelling in the difference between two same-rate
//! measurements, which is the only way the comparison harness reads this.
//!
//! Gating is the standard's: 400 ms blocks at 75 % overlap, an absolute gate at
//! -70 LUFS, then a relative gate 10 LU below the absolute-gated mean.
//!
//! Not implemented: the surround channel weights. Everything this application
//! renders is mono or stereo, both of which weight every channel at 1.0, and a
//! weight table for channels that cannot occur would be untested code claiming
//! standard conformance it never demonstrates.

use std::fmt;

/// Loudness in LUFS — decibels relative to full scale, K-weighted and gated.
///
/// Its own type rather than [`synth_core::Decibels`]: a dB of amplitude and a
/// loudness unit are not interchangeable, and the one arithmetic operation that
/// makes sense between two loudness values — their difference, in LU — produces
/// a third kind of quantity again.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Lufs(f32);

impl Lufs {
    /// Wrap a measured loudness.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// The underlying value.
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// `self - other`, in loudness units.
    ///
    /// Returns a bare `f32` because the result is an interval, not a level: a
    /// difference of 3 LU is the same interval at -23 LUFS as at -9, whereas
    /// `Lufs(3.0)` would be an absurdly loud programme.
    #[must_use]
    pub fn difference_lu(self, other: Self) -> f32 {
        self.0 - other.0
    }
}

impl fmt::Display for Lufs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} LUFS", self.0)
    }
}

/// Block length the standard gates on.
const BLOCK_MS: f32 = 400.0;

/// Overlap between consecutive blocks: 75 %, so the step is a quarter block.
const BLOCK_STEP_MS: f32 = 100.0;

/// Absolute gate. Blocks quieter than this never contribute.
const ABSOLUTE_GATE_LUFS: f32 = -70.0;

/// Relative gate offset below the absolute-gated mean.
const RELATIVE_GATE_LU: f32 = -10.0;

/// The standard's calibration offset.
const CALIBRATION_DB: f32 = -0.691;

/// A direct-form-I biquad in `f64`.
///
/// `f64` rather than the engine's `f32` biquads: this filter runs once over a
/// whole file offline, where precision is free, and its RLB stage has a corner
/// at 38 Hz — a pole close enough to DC that `f32` state accumulates visible
/// error over a multi-minute render.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// Normalize raw coefficients by `a0` and start from rest.
    const fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// The K-weighting pre-filter: a high-frequency shelf, +4 dB above ~1.7 kHz.
    ///
    /// The constants are the analogue prototype BS.1770's 48 kHz table is
    /// derived from, so this reproduces that table at 48 kHz and generalizes
    /// elsewhere.
    fn shelf(sample_rate: f64) -> Self {
        const F0: f64 = 1_681.974_450_955_533;
        const GAIN_DB: f64 = 3.999_843_853_973_347;
        const Q: f64 = 0.707_175_236_955_419_6;
        // The prototype's shelf midpoint: Vh^0.5 in an ideal shelf, moved very
        // slightly by the standard's fit.
        const VB_EXPONENT: f64 = 0.499_666_774_154_541_6;

        let k = (std::f64::consts::PI * F0 / sample_rate).tan();
        let vh = 10.0_f64.powf(GAIN_DB / 20.0);
        let vb = vh.powf(VB_EXPONENT);
        let kk = k * k;
        let a0 = 1.0 + k / Q + kk;
        Self::new(
            vh + vb * k / Q + kk,
            2.0 * (kk - vh),
            vh - vb * k / Q + kk,
            a0,
            2.0 * (kk - 1.0),
            1.0 - k / Q + kk,
        )
    }

    /// The RLB high-pass: removes the low end that contributes level without
    /// contributing loudness.
    ///
    /// The numerator is passed with `a0 = 1.0` rather than normalized by the
    /// denominator's, which is what BS.1770-4's published table does — and it
    /// leaves the stage a rate-dependent passband gain of about +0.047 dB at
    /// 44.1 kHz, +0.043 dB at 48 kHz, and +0.022 dB at 96 kHz. Kept, because
    /// matching the normative table matters more than the hundredth of a
    /// decibel: it is a constant per rate, so it cancels entirely in the
    /// difference between two same-rate measurements, which is the only way the
    /// comparison harness reads this.
    fn rlb(sample_rate: f64) -> Self {
        const F0: f64 = 38.135_470_876_024_44;
        const Q: f64 = 0.500_327_037_323_877_3;

        let k = (std::f64::consts::PI * F0 / sample_rate).tan();
        let kk = k * k;
        let a0 = 1.0 + k / Q + kk;
        Self::new(
            1.0,
            -2.0,
            1.0,
            1.0,
            2.0 * (kk - 1.0) / a0,
            (1.0 - k / Q + kk) / a0,
        )
    }
}

/// Integrated loudness of an interleaved buffer.
///
/// `None` when nothing can be measured: an empty or too-short buffer (the
/// standard's first block is 400 ms, so shorter material has no block at all),
/// a zero channel count or sample rate, or a signal where every block falls
/// below the absolute gate. All four are cases where a number would be an
/// invention rather than a measurement — silence is not "-70 LUFS", it is
/// unmeasured.
#[must_use]
pub fn integrated_loudness(interleaved: &[f32], sample_rate: u32, channels: u16) -> Option<Lufs> {
    let channels = usize::from(channels);
    if channels == 0 || sample_rate == 0 || interleaved.is_empty() {
        return None;
    }
    let frames = interleaved.len() / channels;
    let block_frames = ((BLOCK_MS * 0.001) * sample_rate as f32).round() as usize;
    let step_frames = ((BLOCK_STEP_MS * 0.001) * sample_rate as f32).round() as usize;
    if block_frames == 0 || step_frames == 0 || frames < block_frames {
        return None;
    }

    // K-weight each channel independently, into a deinterleaved buffer. Two
    // filter instances per channel, carrying their own state across the whole
    // signal — filtering per block instead would restart the RLB pole at every
    // block boundary and put a transient in each one.
    let mut weighted = vec![0.0_f64; frames * channels];
    for channel in 0..channels {
        let mut shelf = Biquad::shelf(f64::from(sample_rate));
        let mut rlb = Biquad::rlb(f64::from(sample_rate));
        for frame in 0..frames {
            let sample = f64::from(interleaved[frame * channels + channel]);
            weighted[frame * channels + channel] = rlb.process(shelf.process(sample));
        }
    }

    // Per-block channel-summed mean square. Stereo and mono weight every channel
    // at 1.0, so the sum is unweighted — see the module docs.
    let mut block_power = Vec::new();
    let mut start = 0_usize;
    while start + block_frames <= frames {
        let mut sum = 0.0_f64;
        for frame in start..start + block_frames {
            for channel in 0..channels {
                let s = weighted[frame * channels + channel];
                sum += s * s;
            }
        }
        block_power.push(sum / block_frames as f64);
        start += step_frames;
    }
    if block_power.is_empty() {
        return None;
    }

    // `MIN_POSITIVE`, not `MIN`: `f64::MIN` is the most negative *finite* value,
    // so `power.max(f64::MIN)` is a no-op for the non-negative powers this sees
    // and a silent block would take `log10(0.0)` — an infinity that `serde_json`
    // writes as `null`, i.e. indistinguishable from "not measured". The gates
    // below happen to drop such a block first; the floor is here so that stays
    // true whatever order they are applied in.
    let loudness_of =
        |power: f64| f64::from(CALIBRATION_DB) + 10.0 * power.max(f64::MIN_POSITIVE).log10();

    // Absolute gate, then a relative gate 10 LU below what survives it. The
    // relative threshold is computed from the *mean power* of the
    // absolutely-gated blocks, not from the mean of their loudnesses: averaging
    // in dB would weight a quiet block as heavily as a loud one.
    let absolute: Vec<f64> = block_power
        .iter()
        .copied()
        .filter(|&p| loudness_of(p) > f64::from(ABSOLUTE_GATE_LUFS))
        .collect();
    if absolute.is_empty() {
        return None;
    }
    let absolute_mean = absolute.iter().sum::<f64>() / absolute.len() as f64;
    let relative_gate = loudness_of(absolute_mean) + f64::from(RELATIVE_GATE_LU);

    let gated: Vec<f64> = absolute
        .into_iter()
        .filter(|&p| loudness_of(p) > relative_gate)
        .collect();
    if gated.is_empty() {
        return None;
    }
    let mean = gated.iter().sum::<f64>() / gated.len() as f64;
    Some(Lufs::new(loudness_of(mean) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `hz` and amplitude `amp`, `seconds` long, in `channels`
    /// identical channels.
    fn sine(hz: f32, amp: f32, seconds: f32, sample_rate: u32, channels: u16) -> Vec<f32> {
        let frames = (seconds * sample_rate as f32) as usize;
        let mut out = Vec::with_capacity(frames * usize::from(channels));
        for frame in 0..frames {
            let t = frame as f32 / sample_rate as f32;
            let s = amp * (std::f32::consts::TAU * hz * t).sin();
            for _ in 0..channels {
                out.push(s);
            }
        }
        out
    }

    /// Doubling the amplitude must read exactly 6.02 LU louder. This is the
    /// property that makes the measurement usable as a *difference*, which is
    /// the only way the comparison harness reads it, and it holds independently
    /// of every calibration constant.
    #[test]
    fn doubling_the_amplitude_adds_six_lu() {
        let quiet = sine(1_000.0, 0.1, 3.0, 48_000, 2);
        let loud = sine(1_000.0, 0.2, 3.0, 48_000, 2);
        let a = integrated_loudness(&quiet, 48_000, 2).expect("measurable");
        let b = integrated_loudness(&loud, 48_000, 2).expect("measurable");
        let delta = b.difference_lu(a);
        assert!(
            (delta - 6.0206).abs() < 0.01,
            "doubling read as {delta} LU, expected 6.02"
        );
    }

    /// K-weighting is normalized to be near unity at 1 kHz, so a 1 kHz stereo
    /// sine must land within a decibel of the calibration formula's prediction.
    /// A filter with a wrong shelf gain or a transposed coefficient fails here
    /// while still passing the relative test above.
    #[test]
    fn a_one_kilohertz_sine_matches_the_calibration_formula() {
        let amp = 0.5_f32;
        let signal = sine(1_000.0, amp, 3.0, 48_000, 2);
        let measured = integrated_loudness(&signal, 48_000, 2)
            .expect("measurable")
            .as_f32();
        // Two channels, mean square amp²/2 each, summed at weight 1.0.
        let predicted = CALIBRATION_DB + 10.0 * (amp * amp).log10();
        assert!(
            (measured - predicted).abs() < 1.0,
            "measured {measured} LUFS against an unweighted prediction of {predicted}"
        );
    }

    /// The measurement must not depend on the sample rate: the coefficients are
    /// recomputed per rate precisely so that it does not. This is the test that
    /// would fail if the 48 kHz table had been hardcoded.
    #[test]
    fn the_measurement_is_stable_across_sample_rates() {
        let mut previous: Option<f32> = None;
        for rate in [44_100_u32, 48_000, 96_000] {
            let signal = sine(1_000.0, 0.25, 3.0, rate, 2);
            let measured = integrated_loudness(&signal, rate, 2)
                .expect("measurable")
                .as_f32();
            if let Some(previous) = previous {
                assert!(
                    (measured - previous).abs() < 0.1,
                    "{rate} Hz read {measured} LUFS against {previous}"
                );
            }
            previous = Some(measured);
        }
    }

    /// The loudness of a steady sine scales with the K-weighting's magnitude
    /// response squared, so the LU difference between two tones is exactly the
    /// filter's dB difference between their frequencies. That makes the shape of
    /// the response directly assertable, and these are the numbers the analogue
    /// prototype gives:
    ///
    /// | f      | RLB      | shelf    | K-weighted |
    /// |--------|----------|----------|------------|
    /// | 30 Hz  | -8.35 dB |  0.00 dB |  -8.35 dB  |
    /// | 1 kHz  | -0.01 dB | +0.67 dB |  +0.66 dB  |
    /// | 8 kHz  |  0.00 dB | +3.99 dB |  +3.99 dB  |
    ///
    /// A transposed coefficient, a shelf built as a low shelf, or a missing
    /// stage moves one of these by decibels while still passing the relative and
    /// calibration tests above. The tolerance covers the bilinear transform's
    /// frequency warping at 48 kHz, which is well under a tenth of a decibel at
    /// all three frequencies.
    #[test]
    fn the_weighting_curve_matches_its_prototype() {
        let at = |hz: f32| {
            integrated_loudness(&sine(hz, 0.25, 3.0, 48_000, 2), 48_000, 2).expect("measurable")
        };
        let (low, mid, high) = (at(30.0), at(1_000.0), at(8_000.0));

        let low_cut = mid.difference_lu(low);
        assert!(
            (low_cut - 9.01).abs() < 0.3,
            "30 Hz sits {low_cut} LU below 1 kHz, expected 9.01 — the RLB stage is wrong"
        );
        let high_boost = high.difference_lu(mid);
        assert!(
            (high_boost - 3.33).abs() < 0.3,
            "8 kHz sits {high_boost} LU above 1 kHz, expected 3.33 — the shelf stage is wrong"
        );
    }

    /// Silence has no loudness to report. Returning a number here would let a
    /// comparison of two silent renders produce a confident zero delta from two
    /// measurements that never happened.
    #[test]
    fn silence_is_unmeasured_rather_than_very_quiet() {
        assert!(integrated_loudness(&vec![0.0_f32; 48_000 * 2], 48_000, 2).is_none());
    }

    /// Material shorter than one 400 ms block has no block to gate, and the
    /// standard says nothing about what to report for it.
    #[test]
    fn material_shorter_than_a_block_is_unmeasured() {
        let short = sine(1_000.0, 0.5, 0.2, 48_000, 2);
        assert!(integrated_loudness(&short, 48_000, 2).is_none());
    }

    /// Degenerate inputs must return `None` rather than divide by zero or index
    /// out of bounds.
    #[test]
    fn degenerate_inputs_are_rejected() {
        assert!(integrated_loudness(&[], 48_000, 2).is_none());
        assert!(integrated_loudness(&[0.1, 0.1], 0, 2).is_none());
        assert!(integrated_loudness(&[0.1, 0.1], 48_000, 0).is_none());
    }

    /// The relative gate exists to stop a long quiet passage from dragging the
    /// integrated value down. A signal that is loud for a second and near-silent
    /// for four must read close to the loud part alone.
    #[test]
    fn the_relative_gate_excludes_a_quiet_passage() {
        let mut signal = sine(1_000.0, 0.5, 1.0, 48_000, 2);
        signal.extend(sine(1_000.0, 0.001, 4.0, 48_000, 2));
        let gated = integrated_loudness(&signal, 48_000, 2).expect("measurable");
        let loud_only = integrated_loudness(&sine(1_000.0, 0.5, 1.0, 48_000, 2), 48_000, 2)
            .expect("measurable");
        assert!(
            gated.difference_lu(loud_only).abs() < 1.5,
            "gated {gated} against {loud_only} for the loud passage alone — \
             the quiet tail is not being gated out"
        );
    }
}
