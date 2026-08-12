//! The measurements a comparison reports, and how each is derived.
//!
//! Every metric here answers one of the master plan's Phase 0A comparison
//! requirements: peak and RMS error, onset offset, fundamental frequency and
//! pitch drift, envelope landmarks, stereo correlation and channel energy,
//! spectrum-band differences, integrated loudness, and the render determinism
//! digest (which is [`crate::render::receipt::FileDigest`], computed by the
//! command rather than here).
//!
//! # Direction
//!
//! Every `delta` is **candidate minus reference**. A positive `peak_delta_db`
//! means the candidate is louder. Stated once here because a sign convention
//! that has to be rediscovered per field is a sign convention that gets read
//! backwards.
//!
//! # Absence
//!
//! A metric that cannot be computed is `None`, never a zero. A zero difference
//! is the strongest possible claim these reports make — that two renders agree —
//! and it must never be what a missing measurement looks like. Each absence is
//! accompanied by a warning on the report saying which precondition failed.

use serde::Serialize;

use crate::audio::analysis::loudness::{Lufs, integrated_loudness};
use crate::audio::analysis::spectrum::{ENV_ALIGN_WINDOW_MS, envelope_align};
use crate::audio::analysis::{
    BandEnergy, fundamental_frequency, octave_band_energies, peak_amplitude, pitch_envelope,
    rms_envelope, rms_overall, stereo_correlation,
};

use super::signal::Signal;

/// Amplitude treated as the bottom of the dB scale, -180 dBFS.
///
/// Well below both the `f32` render path's noise floor and 24-bit resolution, so
/// it only ever replaces a true zero — but it means a silent channel reports a
/// finite -180 rather than an infinity that JSON cannot carry.
const DB_FLOOR: f32 = 1.0e-9;

/// Window the envelope-derived metrics work on.
///
/// The same 10 ms as [`ENV_ALIGN_WINDOW_MS`], so the onset times, the landmark
/// times, and the alignment lag are all quantized to one grid and can be read
/// against each other.
const ENVELOPE_WINDOW_MS: f32 = ENV_ALIGN_WINDOW_MS;

/// Widest misalignment the timing search will consider, in ms.
///
/// Half a second is far more than any block-boundary or latency difference
/// between two renders of one project, and bounding it keeps a badly matched
/// pair from reporting a spurious lag found in unrelated material.
const MAX_ALIGNMENT_LAG_MS: f32 = 500.0;

/// Envelope level, relative to the envelope's peak, at which an onset is called.
///
/// -40 dB: high enough to ignore a reverb tail's last breath and a dither-level
/// noise floor, low enough that an onset is detected on the attack rather than
/// partway up it.
const ONSET_THRESHOLD: f32 = 0.01;

/// Level, in dB below the louder envelope's peak, under which two windows stop
/// contributing to the gated envelope difference.
///
/// -60 dB is a coverage threshold, not a claim about audibility — 60 dB below
/// peak is inaudible in most material and plainly audible in a quiet passage,
/// and this module cannot tell which it is looking at. It sits low enough to
/// keep a real reverb tail in the comparison and high enough to drop the
/// difference between one silence and another. It is reported in every result
/// (`EnvelopeDifference::floor_db`) precisely because it is a choice; the
/// ungated maximum is reported next to it so no threshold hides anything.
const ENVELOPE_FLOOR_DB: f32 = -60.0;

/// Frequency search range for every pitch metric, in Hz.
///
/// The low end sits below the lowest note a corpus fixture plays; the high end
/// is above the top of the range but well below Nyquist at every supported rate,
/// so the search never runs into the anti-alias region.
///
/// The low end is generous rather than usable to its last hertz: the per-window
/// estimate is FFT-based over [`PITCH_WINDOW_MS`], which at 44.1 kHz gives
/// roughly 21 Hz bins, so a genuine 30 Hz fundamental would sit in the second
/// bin and inside DC leakage. Nothing in the corpus plays below 65 Hz. A case
/// that needs sub-bass pitch tracking has to widen the window first, not the
/// range.
const PITCH_MIN_HZ: f32 = 30.0;
/// Upper bound of the pitch search. See [`PITCH_MIN_HZ`].
const PITCH_MAX_HZ: f32 = 4_000.0;

/// RMS below which a window is not asked for a pitch.
const PITCH_SILENCE_RMS: f32 = 1.0e-3;

/// Window the pitch envelope is measured over.
///
/// 50 ms is the compromise between frequency resolution and time resolution:
/// long enough to give the underlying FFT usable bins at musical pitches (about
/// 21 Hz at 44.1 kHz), short enough to follow a note change rather than
/// averaging across one.
const PITCH_WINDOW_MS: f32 = 50.0;

/// Linear amplitude as dBFS, floored at -180 dB so silence stays finite.
#[must_use]
pub fn dbfs(linear: f32) -> f32 {
    20.0 * linear.abs().max(DB_FLOOR).log10()
}

/// The interval from `reference` to `candidate` in cents, or `None` if either is
/// not a positive frequency.
#[must_use]
pub fn cents_between(reference: f32, candidate: f32) -> Option<f32> {
    (reference > 0.0 && candidate > 0.0 && reference.is_finite() && candidate.is_finite())
        .then(|| 1200.0 * (candidate / reference).log2())
}

/// Peak and RMS on both sides, and their differences.
#[derive(Debug, Clone, Serialize)]
pub struct LevelDifference {
    /// Reference peak amplitude, dBFS.
    pub reference_peak_dbfs: f32,
    /// Candidate peak amplitude, dBFS.
    pub candidate_peak_dbfs: f32,
    /// Candidate minus reference peak, dB.
    pub peak_delta_db: f32,
    /// Reference RMS, dBFS.
    pub reference_rms_dbfs: f32,
    /// Candidate RMS, dBFS.
    pub candidate_rms_dbfs: f32,
    /// Candidate minus reference RMS, dB.
    pub rms_delta_db: f32,
}

impl LevelDifference {
    /// Measure both signals' level.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Self {
        let reference_peak_dbfs = dbfs(peak_amplitude(&reference.interleaved));
        let candidate_peak_dbfs = dbfs(peak_amplitude(&candidate.interleaved));
        let reference_rms_dbfs = dbfs(rms_overall(&reference.interleaved));
        let candidate_rms_dbfs = dbfs(rms_overall(&candidate.interleaved));
        Self {
            reference_peak_dbfs,
            candidate_peak_dbfs,
            peak_delta_db: candidate_peak_dbfs - reference_peak_dbfs,
            reference_rms_dbfs,
            candidate_rms_dbfs,
            rms_delta_db: candidate_rms_dbfs - reference_rms_dbfs,
        }
    }
}

/// Sample-by-sample agreement, for two signals on the same time base.
#[derive(Debug, Clone, Serialize)]
pub struct SampleDifference {
    /// Whether every sample matches bit for bit.
    ///
    /// The determinism claim a corpus case makes is answered by this and by the
    /// two file digests; they are separate because a file can differ in its
    /// header while its audio is identical.
    pub identical: bool,
    /// First frame at which the two differ, absent when they do not.
    pub first_divergence_frame: Option<u64>,
    /// Largest absolute difference between corresponding samples.
    pub max_abs_error: f32,
    /// RMS of the difference signal, dBFS.
    pub error_rms_dbfs: f32,
    /// Difference RMS relative to the reference's own RMS, dB.
    ///
    /// The scale-free form of the number above, and the one worth a threshold: a
    /// -60 dB error is inaudible against a loud render and obvious against a
    /// quiet one, and only this ratio says which case applies.
    pub error_to_signal_db: f32,
}

impl SampleDifference {
    /// Compare two signals sample by sample.
    ///
    /// Returns `None` unless the two agree on sample rate, channel count, and
    /// length — with any of those different there is no correspondence between
    /// sample *n* on one side and sample *n* on the other, and subtracting them
    /// would produce a large, confident, meaningless number.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Option<Self> {
        if reference.sample_rate != candidate.sample_rate
            || reference.channels != candidate.channels
            || reference.interleaved.len() != candidate.interleaved.len()
        {
            return None;
        }
        let channels = usize::from(reference.channels).max(1);
        let mut first_divergence = None;
        let mut max_abs_error = 0.0_f32;
        let mut sum_squares = 0.0_f64;
        for (index, (a, b)) in reference
            .interleaved
            .iter()
            .zip(&candidate.interleaved)
            .enumerate()
        {
            if first_divergence.is_none() && a.to_bits() != b.to_bits() {
                first_divergence = Some((index / channels) as u64);
            }
            let error = b - a;
            max_abs_error = max_abs_error.max(error.abs());
            sum_squares += f64::from(error) * f64::from(error);
        }
        let error_rms = (sum_squares / reference.interleaved.len() as f64).sqrt() as f32;
        let error_rms_dbfs = dbfs(error_rms);
        Some(Self {
            identical: first_divergence.is_none(),
            first_divergence_frame: first_divergence,
            max_abs_error,
            error_rms_dbfs,
            error_to_signal_db: error_rms_dbfs - dbfs(rms_overall(&reference.interleaved)),
        })
    }
}

/// When each signal starts, and how far apart the two are overall.
#[derive(Debug, Clone, Serialize)]
pub struct TimingDifference {
    /// Reference onset, ms from the start.
    pub reference_onset_ms: f32,
    /// Candidate onset, ms from the start.
    pub candidate_onset_ms: f32,
    /// Candidate minus reference onset, ms. The plan's "onset offset".
    pub onset_delta_ms: f32,
    /// Lag that best aligns the candidate's whole envelope to the reference's,
    /// ms. Positive means the candidate lags.
    ///
    /// Reported next to the onset delta because the two disagree in a useful
    /// way: matching onsets with a non-zero overall lag means the *first* event
    /// is aligned and later ones drift, which is what a tempo or scheduling
    /// change looks like.
    ///
    /// Absent when the reference's envelope is too featureless to align against.
    /// A sustained tone has the same envelope at every offset, so the
    /// cross-correlation's peak is decided by how much the two buffers happen to
    /// overlap rather than by the material — and it reports that arbitrary
    /// number with no less confidence than a real one. Only an envelope with
    /// structure can answer this question, so only one is asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope_lag_ms: Option<f32>,
}

impl TimingDifference {
    /// Locate both onsets and, where the material allows it, the best overall
    /// alignment.
    ///
    /// Returns `None` when either signal has no measurable onset — too short for
    /// one envelope window, or silent.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Option<Self> {
        let reference_mono = reference.mono();
        let candidate_mono = candidate.mono();
        let reference_onset_ms = onset_ms(&reference_mono, reference.sample_rate)?;
        let candidate_onset_ms = onset_ms(&candidate_mono, candidate.sample_rate)?;

        // Both sides, not just the reference. A cross-correlation needs shape on
        // each side to have a peak that means anything: pulses against a
        // sustained tone will still report a confident lag, decided by how much
        // the two buffers happen to overlap rather than by the audio.
        let reference_envelope =
            rms_envelope(&reference_mono, reference.sample_rate, ENVELOPE_WINDOW_MS);
        let candidate_envelope =
            rms_envelope(&candidate_mono, candidate.sample_rate, ENVELOPE_WINDOW_MS);
        let alignable = has_alignable_structure(&reference_envelope)
            && has_alignable_structure(&candidate_envelope);
        let envelope_lag_ms = alignable.then(|| {
            let windows = envelope_align(
                &reference_mono,
                reference.sample_rate,
                &candidate_mono,
                candidate.sample_rate,
                MAX_ALIGNMENT_LAG_MS,
            );
            windows as f32 * ENV_ALIGN_WINDOW_MS
        });

        Some(Self {
            reference_onset_ms,
            candidate_onset_ms,
            onset_delta_ms: candidate_onset_ms - reference_onset_ms,
            envelope_lag_ms,
        })
    }
}

/// Smallest coefficient of variation an envelope needs before a cross-correlation
/// lag means anything.
///
/// A steady tone's envelope varies by a fraction of a percent (window-boundary
/// ripple only); anything with note attacks in it is far above this. The exact
/// value is not delicate — the two populations are orders of magnitude apart.
const MIN_ENVELOPE_VARIATION: f32 = 0.05;

/// Whether an envelope varies enough over time for its shape to identify a
/// position within it.
fn has_alignable_structure(envelope: &[f32]) -> bool {
    if envelope.len() < 2 {
        return false;
    }
    let mean = envelope.iter().map(|x| f64::from(*x)).sum::<f64>() / envelope.len() as f64;
    if mean <= f64::from(DB_FLOOR) {
        return false;
    }
    let variance = envelope
        .iter()
        .map(|x| {
            let d = f64::from(*x) - mean;
            d * d
        })
        .sum::<f64>()
        / envelope.len() as f64;
    (variance.sqrt() / mean) as f32 > MIN_ENVELOPE_VARIATION
}

/// Time of the first envelope window above [`ONSET_THRESHOLD`] of the
/// envelope's peak, in ms.
fn onset_ms(mono: &[f32], sample_rate: u32) -> Option<f32> {
    let envelope = rms_envelope(mono, sample_rate, ENVELOPE_WINDOW_MS);
    let peak = envelope.iter().copied().fold(0.0_f32, f32::max);
    if envelope.is_empty() || peak <= DB_FLOOR {
        return None;
    }
    let threshold = peak * ONSET_THRESHOLD;
    envelope
        .iter()
        .position(|&level| level >= threshold)
        .map(|index| index as f32 * ENVELOPE_WINDOW_MS)
}

/// How the two signals' pitch compares.
#[derive(Debug, Clone, Serialize)]
pub struct PitchDifference {
    /// Reference fundamental at the head of the signal, Hz, or absent when the
    /// signal has none.
    ///
    /// An `Option` rather than the estimator's `0.0` sentinel: 0 Hz is not a
    /// frequency, and a percussive render reporting it reads as a measurement of
    /// something rather than the absence of one.
    ///
    /// One transform over the leading window
    /// ([`fundamental_frequency`] caps its FFT at 32768 samples, 0.74 s at
    /// 44.1 kHz), *not* the whole render: on material that changes pitch this is
    /// the opening note's fundamental and nothing else. [`Self::drift`] is the
    /// field that covers the whole signal, window by window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_hz: Option<f32>,
    /// Candidate fundamental at the head of the signal. See
    /// [`Self::reference_hz`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_hz: Option<f32>,
    /// Candidate minus reference over the two head fundamentals above, in cents.
    /// Absent when either side is unpitched.
    pub delta_cents: Option<f32>,
    /// Window-by-window pitch agreement. Absent when the two cannot be compared
    /// window for window, or when no window is voiced on both sides.
    pub drift: Option<PitchDrift>,
}

/// A fundamental the estimator actually found.
///
/// It reports `0.0` for "no pitch here", which is a sentinel and not a
/// frequency; this turns it back into an absence.
fn pitched(hz: f32) -> Option<f32> {
    (hz > 0.0 && hz.is_finite()).then_some(hz)
}

/// Window-by-window pitch agreement — the plan's "pitch drift".
#[derive(Debug, Clone, Serialize)]
pub struct PitchDrift {
    /// Windows voiced on both sides, and therefore compared.
    pub compared_windows: usize,
    /// Mean absolute difference over those windows, in cents.
    pub mean_abs_cents: f32,
    /// Largest absolute difference over those windows, in cents.
    pub max_abs_cents: f32,
    /// Where the largest difference occurred, ms from the start.
    pub max_at_ms: f32,
}

impl PitchDifference {
    /// Measure the leading fundamental on each side, and the per-window drift
    /// between them over the whole signal.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Self {
        let reference_mono = reference.mono();
        let candidate_mono = candidate.mono();
        let reference_hz = fundamental_frequency(
            &reference_mono,
            reference.sample_rate,
            PITCH_MIN_HZ,
            PITCH_MAX_HZ,
        );
        let candidate_hz = fundamental_frequency(
            &candidate_mono,
            candidate.sample_rate,
            PITCH_MIN_HZ,
            PITCH_MAX_HZ,
        );
        Self {
            reference_hz: pitched(reference_hz),
            candidate_hz: pitched(candidate_hz),
            delta_cents: cents_between(reference_hz, candidate_hz),
            drift: PitchDrift::measure(
                &reference_mono,
                reference.sample_rate,
                &candidate_mono,
                candidate.sample_rate,
            ),
        }
    }
}

impl PitchDrift {
    /// Compare the two pitch envelopes window for window.
    ///
    /// Only windows voiced on *both* sides count: a window where one side found
    /// a pitch and the other did not is a voicing difference, not a pitch
    /// difference, and averaging it in as some number of cents would be
    /// inventing an interval to an unpitched signal.
    #[must_use]
    fn measure(
        reference: &[f32],
        reference_rate: u32,
        candidate: &[f32],
        candidate_rate: u32,
    ) -> Option<Self> {
        let reference_pitches = pitch_envelope(
            reference,
            reference_rate,
            PITCH_WINDOW_MS,
            PITCH_MIN_HZ,
            PITCH_MAX_HZ,
            PITCH_SILENCE_RMS,
        );
        let candidate_pitches = pitch_envelope(
            candidate,
            candidate_rate,
            PITCH_WINDOW_MS,
            PITCH_MIN_HZ,
            PITCH_MAX_HZ,
            PITCH_SILENCE_RMS,
        );

        let mut compared = 0_usize;
        let mut total = 0.0_f64;
        let mut max_abs = 0.0_f32;
        let mut max_at = 0_usize;
        for (index, (a, b)) in reference_pitches.iter().zip(&candidate_pitches).enumerate() {
            let Some(cents) = cents_between(*a, *b) else {
                continue;
            };
            compared += 1;
            total += f64::from(cents.abs());
            if cents.abs() > max_abs {
                max_abs = cents.abs();
                max_at = index;
            }
        }
        (compared > 0).then(|| Self {
            compared_windows: compared,
            mean_abs_cents: (total / compared as f64) as f32,
            max_abs_cents: max_abs,
            max_at_ms: max_at as f32 * PITCH_WINDOW_MS,
        })
    }
}

/// Times, in ms, at which an RMS envelope reaches its landmarks.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct EnvelopeLandmarks {
    /// When the envelope reaches its maximum.
    pub peak_ms: f32,
    /// When it first reaches 90 % of that maximum — the end of the attack.
    pub rise_to_90_ms: f32,
    /// When it first falls to half the maximum after the peak, or the last
    /// envelope window if it never does.
    pub fall_to_50_ms: f32,
    /// The last time it is above 1 % of the maximum — the end of the tail.
    pub tail_end_ms: f32,
}

impl EnvelopeLandmarks {
    /// Locate the landmarks of `envelope`, whose steps are `window_ms` apart.
    fn of(envelope: &[f32], window_ms: f32) -> Option<Self> {
        let (peak_index, peak) = envelope
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, level)| (index, *level))?;
        if peak <= DB_FLOOR {
            return None;
        }
        let at = |index: usize| index as f32 * window_ms;
        let rise = envelope
            .iter()
            .position(|&level| level >= peak * 0.9)
            .unwrap_or(peak_index);
        let fall = envelope[peak_index..]
            .iter()
            .position(|&level| level <= peak * 0.5)
            .map_or(envelope.len().saturating_sub(1), |offset| {
                peak_index + offset
            });
        let tail = envelope
            .iter()
            .rposition(|&level| level > peak * 0.01)
            .unwrap_or(peak_index);
        Some(Self {
            peak_ms: at(peak_index),
            rise_to_90_ms: at(rise),
            fall_to_50_ms: at(fall),
            tail_end_ms: at(tail),
        })
    }

    /// `self - other`, field by field.
    fn delta(self, other: Self) -> Self {
        Self {
            peak_ms: self.peak_ms - other.peak_ms,
            rise_to_90_ms: self.rise_to_90_ms - other.rise_to_90_ms,
            fall_to_50_ms: self.fall_to_50_ms - other.fall_to_50_ms,
            tail_end_ms: self.tail_end_ms - other.tail_end_ms,
        }
    }
}

/// How the two amplitude envelopes compare.
#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeDifference {
    /// Reference landmarks.
    pub reference: EnvelopeLandmarks,
    /// Candidate landmarks.
    pub candidate: EnvelopeLandmarks,
    /// Candidate minus reference, per landmark, in ms.
    pub delta_ms: EnvelopeLandmarks,
    /// Pearson correlation between the two envelopes over their common length.
    /// 1.0 means the same shape at any level.
    ///
    /// Absent when either envelope is too featureless to correlate, for the same
    /// reason [`TimingDifference::envelope_lag_ms`] is: a sustained tone's
    /// envelope varies only by window-boundary ripple, so the correlation is
    /// computed from noise and lands anywhere in [-1, 1] while looking like a
    /// measurement. A constant envelope has no variance at all, and a
    /// correlation of 0 for it would assert "unrelated" rather than
    /// "not measured".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation: Option<f32>,
    /// Largest level difference between any pair of corresponding envelope
    /// windows, dB — every window, with no threshold applied.
    ///
    /// Reported first and unconditionally because it is the raw measurement.
    /// It is also the one most easily dominated by material nobody can hear: a
    /// candidate whose reverb tail decays to 1e-6 where the reference reaches
    /// exactly zero shows 120 dB here, with `max_delta_at_ms` pointing into
    /// silence. Read it together with the gated figure below.
    pub max_delta_db: f32,
    /// Where that largest difference occurred, ms from the start.
    pub max_delta_at_ms: f32,
    /// The same measurement over the windows above [`Self::floor_db`] only.
    ///
    /// Correlation ignores gain, so it would score a candidate 6 dB down as a
    /// perfect match; this and `max_delta_db` are what catch that. Windows where
    /// *either* side clears the floor are compared, so a candidate that dropped
    /// a tail the reference really had is reported at full size — it is only
    /// two mutually inaudible tails that stop counting.
    pub max_delta_above_floor_db: f32,
    /// Where *that* largest difference occurred, ms from the start.
    pub max_delta_above_floor_at_ms: f32,
    /// The level, in dB below the louder render's envelope peak, that a window
    /// had to clear to count toward `max_delta_above_floor_db`.
    ///
    /// Reported rather than left implicit because it is a judgement, and this
    /// module's whole claim is that it makes none silently. A reader who
    /// disagrees with the value can see it, and a reader comparing two reports
    /// can tell whether they used the same one. It is a coverage threshold, not
    /// a claim about audibility: -60 dB relative to peak is inaudible in most
    /// material and obvious in a quiet passage, and nothing here knows which
    /// this is.
    pub floor_db: f32,
    /// How many envelope windows cleared the floor and were compared.
    ///
    /// Reported so `max_delta_above_floor_db` can be read in proportion: one
    /// window out of four hundred is a click, three hundred is a different
    /// render.
    pub compared_windows: usize,
    /// How many envelope windows the two signals share.
    pub total_windows: usize,
}

impl EnvelopeDifference {
    /// Compare the two amplitude envelopes.
    ///
    /// Returns `None` when either signal is too short for one envelope window or
    /// has no measurable peak.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Option<Self> {
        let reference_env =
            rms_envelope(&reference.mono(), reference.sample_rate, ENVELOPE_WINDOW_MS);
        let candidate_env =
            rms_envelope(&candidate.mono(), candidate.sample_rate, ENVELOPE_WINDOW_MS);
        let reference_marks = EnvelopeLandmarks::of(&reference_env, ENVELOPE_WINDOW_MS)?;
        let candidate_marks = EnvelopeLandmarks::of(&candidate_env, ENVELOPE_WINDOW_MS)?;

        // Relative to the louder of the two peaks, so neither side's silence
        // sets the bar for the other's.
        let peak = envelope_peak(&reference_env).max(envelope_peak(&candidate_env));
        let floor = peak * 10.0_f32.powf(ENVELOPE_FLOOR_DB / 20.0);

        let mut raw = (0.0_f32, 0_usize);
        let mut gated = (0.0_f32, 0_usize);
        let mut compared_windows = 0_usize;
        let mut total_windows = 0_usize;
        for (index, (a, b)) in reference_env.iter().zip(&candidate_env).enumerate() {
            total_windows += 1;
            let delta = (dbfs(*b) - dbfs(*a)).abs();
            if delta > raw.0 {
                raw = (delta, index);
            }
            // Either side, not both: a window where the reference sounds and the
            // candidate is silent is the most important difference there is.
            if *a < floor && *b < floor {
                continue;
            }
            compared_windows += 1;
            if delta > gated.0 {
                gated = (delta, index);
            }
        }

        Some(Self {
            reference: reference_marks,
            candidate: candidate_marks,
            delta_ms: candidate_marks.delta(reference_marks),
            correlation: (has_alignable_structure(&reference_env)
                && has_alignable_structure(&candidate_env))
            .then(|| correlation(&reference_env, &candidate_env)),
            max_delta_db: raw.0,
            max_delta_at_ms: raw.1 as f32 * ENVELOPE_WINDOW_MS,
            max_delta_above_floor_db: gated.0,
            max_delta_above_floor_at_ms: gated.1 as f32 * ENVELOPE_WINDOW_MS,
            floor_db: ENVELOPE_FLOOR_DB,
            compared_windows,
            total_windows,
        })
    }
}

/// Largest value in an envelope, or 0 for an empty one.
fn envelope_peak(envelope: &[f32]) -> f32 {
    envelope.iter().copied().fold(0.0_f32, f32::max)
}

/// Whether a channel varies at all. Two constant samples have no correlation
/// with anything, and a Pearson figure computed from them is a division by zero
/// dressed up as a number.
fn has_variance(channel: &[f32]) -> bool {
    let Some(first) = channel.first() else {
        return false;
    };
    channel.iter().any(|s| s.to_bits() != first.to_bits())
}

/// Pearson correlation over the two slices' common length. 0.0 when either has
/// no variance — callers guard against that case rather than reading the 0, see
/// [`EnvelopeDifference::correlation`].
fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }
    let (a, b) = (&a[..n], &b[..n]);
    let mean = |v: &[f32]| v.iter().map(|x| f64::from(*x)).sum::<f64>() / n as f64;
    let (mean_a, mean_b) = (mean(a), mean(b));
    let (mut cov, mut var_a, mut var_b) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (f64::from(*x) - mean_a, f64::from(*y) - mean_b);
        cov += dx * dy;
        var_a += dx * dx;
        var_b += dy * dy;
    }
    let denominator = (var_a * var_b).sqrt();
    if denominator <= 0.0 {
        return 0.0;
    }
    (cov / denominator).clamp(-1.0, 1.0) as f32
}

/// How the two signals' stereo image compares.
#[derive(Debug, Clone, Serialize)]
pub struct StereoDifference {
    /// Reference left-right correlation, -1 to 1, or absent when a channel is
    /// silent or constant.
    ///
    /// Pearson correlation is undefined without variance on both sides, and the
    /// underlying estimator returns `0.0` there — which asserts "uncorrelated",
    /// the single most misleading answer available. A silent channel is not
    /// uncorrelated with anything; it is unmeasured. Same reasoning as
    /// [`EnvelopeDifference::correlation`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_correlation: Option<f32>,
    /// Candidate left-right correlation. See [`Self::reference_correlation`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_correlation: Option<f32>,
    /// Candidate minus reference correlation, absent when either side is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_delta: Option<f32>,
    /// Reference left-channel RMS, dBFS.
    pub reference_left_dbfs: f32,
    /// Reference right-channel RMS, dBFS.
    pub reference_right_dbfs: f32,
    /// Candidate left-channel RMS, dBFS.
    pub candidate_left_dbfs: f32,
    /// Candidate right-channel RMS, dBFS.
    pub candidate_right_dbfs: f32,
    /// Candidate minus reference channel balance (right minus left), dB.
    ///
    /// The balance rather than the two channel levels, because a change that
    /// moves the whole mix shows up in `rms_delta_db` already; this isolates the
    /// part that moved the image.
    pub balance_delta_db: f32,
}

impl StereoDifference {
    /// Compare the stereo image.
    ///
    /// Returns `None` unless both signals are stereo: there is no image to
    /// compare on a mono file, and reporting a correlation of 1.0 for one would
    /// claim a measurement that was never made.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Option<Self> {
        if reference.channels != 2 || candidate.channels != 2 {
            return None;
        }
        let reference_left_dbfs = dbfs(rms_overall(&reference.channel(0)));
        let reference_right_dbfs = dbfs(rms_overall(&reference.channel(1)));
        let candidate_left_dbfs = dbfs(rms_overall(&candidate.channel(0)));
        let candidate_right_dbfs = dbfs(rms_overall(&candidate.channel(1)));
        // `stereo_correlation` returns 0.0 for a channel with no variance, so
        // the variance is checked here rather than reading that 0 as an answer.
        let correlated = |signal: &Signal| {
            (has_variance(&signal.channel(0)) && has_variance(&signal.channel(1)))
                .then(|| stereo_correlation(&signal.interleaved))
        };
        let reference_correlation = correlated(reference);
        let candidate_correlation = correlated(candidate);
        Some(Self {
            reference_correlation,
            candidate_correlation,
            correlation_delta: reference_correlation
                .zip(candidate_correlation)
                .map(|(r, c)| c - r),
            reference_left_dbfs,
            reference_right_dbfs,
            candidate_left_dbfs,
            candidate_right_dbfs,
            balance_delta_db: (candidate_right_dbfs - candidate_left_dbfs)
                - (reference_right_dbfs - reference_left_dbfs),
        })
    }
}

/// One octave band's level on both sides.
#[derive(Debug, Clone, Serialize)]
pub struct BandDifference {
    /// Lower edge, inclusive.
    pub low_hz: f32,
    /// Upper edge, exclusive.
    pub high_hz: f32,
    /// Reference energy in the band, dBFS.
    pub reference_dbfs: f32,
    /// Candidate energy in the band, dBFS.
    pub candidate_dbfs: f32,
    /// Candidate minus reference, dB.
    pub delta_db: f32,
}

/// How the two signals' spectra compare, band by band.
#[derive(Debug, Clone, Serialize)]
pub struct SpectrumDifference {
    /// Every band both signals cover.
    pub bands: Vec<BandDifference>,
    /// The largest absolute band difference, dB.
    pub max_abs_delta_db: f32,
    /// Which band that was, as its lower edge in Hz. Always the lower edge of
    /// one of the bands in [`Self::bands`] — when every band agrees exactly it
    /// is the lowest of them, never a 0 Hz that resolves to no band at all.
    pub max_at_hz: f32,
    /// Root-mean-square of the band differences, dB. One number for "how far
    /// apart are these spectra", where the maximum answers "where".
    pub rms_delta_db: f32,
}

impl SpectrumDifference {
    /// Compare the two spectra band by band.
    ///
    /// Returns `None` when the two sample rates differ — the top band would then
    /// cover a different frequency span on each side, and the difference would
    /// report a spectral change that is really a rate change — or when either
    /// signal is too short for one analysis frame.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Option<Self> {
        if reference.sample_rate != candidate.sample_rate {
            return None;
        }
        let reference_bands = octave_band_energies(&reference.mono(), reference.sample_rate);
        let candidate_bands = octave_band_energies(&candidate.mono(), candidate.sample_rate);
        if reference_bands.is_empty() || candidate_bands.is_empty() {
            return None;
        }

        let mut bands = Vec::with_capacity(reference_bands.len());
        let mut max_abs_delta_db = 0.0_f32;
        // Seeded from the first paired band rather than left at 0.0: two
        // agreeing renders leave every delta at exactly zero, and 0 Hz is not
        // one of the octave bands — a consumer resolving `max_at_hz` back to a
        // band would find nothing. The lowest compared band is the honest
        // answer when no band stands out.
        let mut max_at_hz: Option<f32> = None;
        let mut sum_squares = 0.0_f64;
        for (reference_band, candidate_band) in pair_bands(&reference_bands, &candidate_bands) {
            let reference_dbfs = dbfs(reference_band.rms);
            let candidate_dbfs = dbfs(candidate_band.rms);
            let delta_db = candidate_dbfs - reference_dbfs;
            if max_at_hz.is_none() || delta_db.abs() > max_abs_delta_db {
                max_abs_delta_db = delta_db.abs();
                max_at_hz = Some(reference_band.low_hz);
            }
            sum_squares += f64::from(delta_db) * f64::from(delta_db);
            bands.push(BandDifference {
                low_hz: reference_band.low_hz,
                high_hz: reference_band.high_hz,
                reference_dbfs,
                candidate_dbfs,
                delta_db,
            });
        }
        let max_at_hz = max_at_hz?;
        if bands.is_empty() {
            return None;
        }
        Some(Self {
            rms_delta_db: (sum_squares / bands.len() as f64).sqrt() as f32,
            max_abs_delta_db,
            max_at_hz,
            bands,
        })
    }
}

/// Pair bands by their lower edge, keeping only those present on both sides.
///
/// Matched by frequency rather than by index because a band absent above Nyquist
/// on one side would otherwise shift every later band against its neighbour and
/// report a large difference in every one of them.
///
/// The comparison is on the bit patterns, which is safe here only because of
/// where the values come from: every `low_hz` is copied verbatim out of the one
/// `OCTAVE_BAND_EDGES_HZ` array, so two edges that name the same band are the
/// same `f32` bit for bit and no arithmetic has touched them. That array is the
/// canonical representation this identity rests on — a computed edge would need
/// a tolerance instead.
fn pair_bands<'a>(
    reference: &'a [BandEnergy],
    candidate: &'a [BandEnergy],
) -> impl Iterator<Item = (&'a BandEnergy, &'a BandEnergy)> {
    reference.iter().filter_map(move |r| {
        candidate
            .iter()
            .find(|c| c.low_hz.to_bits() == r.low_hz.to_bits())
            .map(|c| (r, c))
    })
}

/// Integrated loudness on both sides.
#[derive(Debug, Clone, Serialize)]
pub struct LoudnessDifference {
    /// Reference integrated loudness. Absent when unmeasurable — see
    /// [`integrated_loudness`].
    pub reference_lufs: Option<Lufs>,
    /// Candidate integrated loudness.
    pub candidate_lufs: Option<Lufs>,
    /// Candidate minus reference, in LU. Absent when either side is.
    pub delta_lu: Option<f32>,
}

impl LoudnessDifference {
    /// Measure both signals' integrated loudness.
    #[must_use]
    pub fn measure(reference: &Signal, candidate: &Signal) -> Self {
        let reference_lufs = integrated_loudness(
            &reference.interleaved,
            reference.sample_rate,
            reference.channels,
        );
        let candidate_lufs = integrated_loudness(
            &candidate.interleaved,
            candidate.sample_rate,
            candidate.channels,
        );
        Self {
            reference_lufs,
            candidate_lufs,
            delta_lu: reference_lufs
                .zip(candidate_lufs)
                .map(|(r, c)| c.difference_lu(r)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    /// A stereo sine of `hz` at `amp`, `seconds` long, delayed by `delay_ms` of
    /// leading silence.
    fn tone(hz: f32, amp: f32, seconds: f32, delay_ms: f32) -> Signal {
        let lead = ((delay_ms * 0.001) * SR as f32) as usize;
        let frames = (seconds * SR as f32) as usize;
        let mut interleaved = vec![0.0_f32; lead * 2];
        for frame in 0..frames {
            let t = frame as f32 / SR as f32;
            let s = amp * (std::f32::consts::TAU * hz * t).sin();
            interleaved.push(s);
            interleaved.push(s);
        }
        Signal {
            interleaved,
            sample_rate: SR,
            channels: 2,
        }
    }

    /// A signal identical to itself must report zero everywhere. This is the
    /// baseline every other assertion is a deviation from, and the property a
    /// determinism check depends on.
    #[test]
    fn a_signal_compared_with_itself_reports_no_difference() {
        // Pulsed rather than sustained, so every metric is exercised — including
        // the alignment lag, which a featureless envelope declines to report.
        let signal = pulses(440.0, 0.5, 6, 150.0, 150.0, 0.0);
        let level = LevelDifference::measure(&signal, &signal);
        assert!(level.peak_delta_db.abs() < 1e-5);
        assert!(level.rms_delta_db.abs() < 1e-5);

        let samples = SampleDifference::measure(&signal, &signal).expect("same time base");
        assert!(samples.identical);
        assert_eq!(samples.first_divergence_frame, None);
        assert_eq!(samples.max_abs_error, 0.0);

        let timing = TimingDifference::measure(&signal, &signal).expect("has an onset");
        assert_eq!(timing.onset_delta_ms, 0.0);
        assert_eq!(timing.envelope_lag_ms, Some(0.0));

        let envelope = EnvelopeDifference::measure(&signal, &signal).expect("measurable");
        assert!(envelope.max_delta_db < 1e-3, "{envelope:?}");
        let correlation = envelope
            .correlation
            .expect("pulsed material is correlatable");
        assert!((correlation - 1.0).abs() < 1e-5, "{envelope:?}");
        assert!(
            envelope.compared_windows > 0,
            "some window must clear the audibility floor: {envelope:?}"
        );

        let spectrum = SpectrumDifference::measure(&signal, &signal).expect("comparable");
        assert!(spectrum.max_abs_delta_db < 1e-3, "{spectrum:?}");
        // Even with nothing to point at, `max_at_hz` names a band that is
        // actually in the report: a consumer resolving it back to a band must
        // not be handed a 0 Hz that matches none of them.
        assert!(
            spectrum
                .bands
                .iter()
                .any(|band| band.low_hz.to_bits() == spectrum.max_at_hz.to_bits()),
            "max_at_hz {} is not the lower edge of any reported band: {:?}",
            spectrum.max_at_hz,
            spectrum.bands.iter().map(|b| b.low_hz).collect::<Vec<_>>()
        );

        let loudness = LoudnessDifference::measure(&signal, &signal);
        assert!(loudness.delta_lu.expect("measurable").abs() < 1e-3);
    }

    /// A known gain change must come back as exactly that many dB on peak, RMS,
    /// every spectrum band, and loudness. Halving the amplitude is -6.02 dB.
    #[test]
    fn a_six_decibel_gain_change_is_reported_as_six_decibels() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let candidate = tone(440.0, 0.25, 2.0, 0.0);

        let level = LevelDifference::measure(&reference, &candidate);
        assert!((level.peak_delta_db + 6.0206).abs() < 0.01, "{level:?}");
        assert!((level.rms_delta_db + 6.0206).abs() < 0.01, "{level:?}");

        let spectrum = SpectrumDifference::measure(&reference, &candidate).expect("comparable");
        let band = spectrum
            .bands
            .iter()
            .find(|b| 440.0 >= b.low_hz && 440.0 < b.high_hz)
            .expect("the tone's band");
        assert!((band.delta_db + 6.0206).abs() < 0.05, "{band:?}");

        let loudness = LoudnessDifference::measure(&reference, &candidate);
        assert!((loudness.delta_lu.expect("measurable") + 6.0206).abs() < 0.05);
    }

    /// A pulsed tone: `pulses` bursts of `hz` at `amp`, each `on_ms` long
    /// followed by `off_ms` of silence, after `delay_ms` of leading silence.
    /// Envelope structure is what makes an alignment lag meaningful.
    fn pulses(hz: f32, amp: f32, count: usize, on_ms: f32, off_ms: f32, delay_ms: f32) -> Signal {
        let ms_to_frames = |ms: f32| ((ms * 0.001) * SR as f32) as usize;
        let mut interleaved = vec![0.0_f32; ms_to_frames(delay_ms) * 2];
        let mut frame = 0_usize;
        for _ in 0..count {
            for _ in 0..ms_to_frames(on_ms) {
                let s = amp * (std::f32::consts::TAU * hz * frame as f32 / SR as f32).sin();
                interleaved.push(s);
                interleaved.push(s);
                frame += 1;
            }
            interleaved.extend(std::iter::repeat_n(0.0, ms_to_frames(off_ms) * 2));
        }
        Signal {
            interleaved,
            sample_rate: SR,
            channels: 2,
        }
    }

    /// A known delay must come back as that delay, both as an onset offset and
    /// as an envelope lag.
    #[test]
    fn a_known_delay_is_reported_as_that_delay() {
        let reference = pulses(440.0, 0.5, 6, 150.0, 150.0, 0.0);
        let candidate = pulses(440.0, 0.5, 6, 150.0, 150.0, 100.0);
        let timing = TimingDifference::measure(&reference, &candidate).expect("has onsets");
        assert!(
            (timing.onset_delta_ms - 100.0).abs() <= ENVELOPE_WINDOW_MS,
            "{timing:?}"
        );
        let lag = timing
            .envelope_lag_ms
            .expect("pulsed material is alignable");
        assert!((lag - 100.0).abs() <= ENV_ALIGN_WINDOW_MS, "{timing:?}");
    }

    /// A sustained tone carries no information about *where* in it you are, so
    /// the cross-correlation's peak is decided by buffer overlap rather than by
    /// the audio. Reporting that number would be reporting an artefact with the
    /// same confidence as a measurement, so it must be omitted — while the onset
    /// delta, which is still well defined, is not.
    #[test]
    fn a_featureless_envelope_reports_no_alignment_lag() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let candidate = tone(440.0, 0.5, 2.0, 100.0);
        let timing = TimingDifference::measure(&reference, &candidate).expect("has onsets");
        assert_eq!(timing.envelope_lag_ms, None, "{timing:?}");
        assert!(
            (timing.onset_delta_ms - 100.0).abs() <= ENVELOPE_WINDOW_MS,
            "the onset is still measurable: {timing:?}"
        );
    }

    /// Both envelopes have to carry shape, not just the reference. A structured
    /// reference against a featureless candidate has no meaningful alignment
    /// either — the correlation peak is then decided by how far the two buffers
    /// overlap — and checking only one side reported a confident 390 ms for a
    /// pair that has no lag to find.
    #[test]
    fn a_featureless_candidate_also_suppresses_the_alignment_lag() {
        let structured = pulses(440.0, 0.5, 6, 150.0, 150.0, 0.0);
        let featureless = tone(440.0, 0.5, 2.0, 0.0);

        let timing = TimingDifference::measure(&structured, &featureless).expect("has onsets");
        assert_eq!(
            timing.envelope_lag_ms, None,
            "a featureless candidate must not produce a lag: {timing:?}"
        );
        // And the mirror image, which the reference-only check already caught.
        let timing = TimingDifference::measure(&featureless, &structured).expect("has onsets");
        assert_eq!(timing.envelope_lag_ms, None, "{timing:?}");
    }

    /// A known pitch change must come back in cents. A semitone is 100.
    #[test]
    fn a_semitone_of_detune_is_reported_as_a_hundred_cents() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let candidate = tone(440.0 * 2.0_f32.powf(1.0 / 12.0), 0.5, 2.0, 0.0);
        let pitch = PitchDifference::measure(&reference, &candidate);
        let delta = pitch.delta_cents.expect("both pitched");
        assert!((delta - 100.0).abs() < 15.0, "{pitch:?}");
        assert!(pitch.reference_hz.is_some() && pitch.candidate_hz.is_some());
        let drift = pitch.drift.expect("voiced windows");
        assert!(drift.compared_windows > 10, "{drift:?}");
        assert!((drift.mean_abs_cents - 100.0).abs() < 15.0, "{drift:?}");
    }

    /// A near-flat envelope carries no shape to correlate: the Pearson figure
    /// comes out of window-boundary ripple and can land anywhere in [-1, 1]
    /// while looking exactly like a measurement. It has to be omitted, on the
    /// same grounds as the alignment lag — while the landmarks and the level
    /// difference, which stay well defined, are not.
    #[test]
    fn a_featureless_envelope_reports_no_correlation() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let candidate = tone(440.0, 0.25, 2.0, 0.0);
        let envelope = EnvelopeDifference::measure(&reference, &candidate).expect("measurable");
        assert_eq!(envelope.correlation, None, "{envelope:?}");
        assert!(
            (envelope.max_delta_db - 6.0206).abs() < 0.1,
            "the level difference is still measured: {envelope:?}"
        );
    }

    /// The audibility floor is what keeps a decayed tail from dominating the
    /// report. Two renders that agree everywhere audible, but differ between
    /// "exactly zero" and "one part in a million" once the tail has decayed,
    /// must not come back as a 120 dB difference located in silence.
    #[test]
    fn an_inaudible_tail_does_not_dominate_the_envelope_difference() {
        let mut reference = pulses(440.0, 0.5, 3, 150.0, 150.0, 0.0);
        let mut candidate = reference.clone();
        // A tail below the floor on both sides, at wildly different levels.
        let tail = (SR as usize) * 2;
        reference
            .interleaved
            .extend(std::iter::repeat_n(0.0_f32, tail));
        candidate
            .interleaved
            .extend(std::iter::repeat_n(1.0e-6_f32, tail));

        let envelope = EnvelopeDifference::measure(&reference, &candidate).expect("measurable");
        assert!(
            envelope.max_delta_above_floor_db < 1.0,
            "an inaudible tail must not reach the gated figure: {envelope:?}"
        );
        // The raw maximum still reports it, which is the point of keeping both:
        // the threshold changes which number a reader weighs, not what was
        // measured.
        assert!(
            envelope.max_delta_db > 40.0,
            "the ungated maximum must still see the tail: {envelope:?}"
        );
        assert!(
            envelope.compared_windows < envelope.total_windows,
            "the floor must have excluded something: {envelope:?}"
        );

        // The same test in reverse: a tail the reference can actually be heard
        // in, and the candidate cannot, is a real difference and must survive.
        let mut dropped = reference.clone();
        let audible_from = dropped.interleaved.len() - tail;
        dropped.interleaved[audible_from..].fill(0.0);
        reference.interleaved[audible_from..].fill(0.2);
        let envelope = EnvelopeDifference::measure(&reference, &dropped).expect("measurable");
        assert!(
            envelope.max_delta_db > 60.0,
            "a dropped audible tail must register: {envelope:?}"
        );
    }

    /// A spectral change confined to one band must show up in that band and not
    /// in its neighbours — the whole reason for reporting per-band deltas rather
    /// than one spectral distance.
    #[test]
    fn a_band_limited_change_stays_in_its_band() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        // Same fundamental, plus a tone three octaves up. Only the high band
        // gains energy.
        let mut candidate = reference.clone();
        for (index, sample) in candidate.interleaved.iter_mut().enumerate() {
            let t = (index / 2) as f32 / SR as f32;
            *sample += 0.25 * (std::f32::consts::TAU * 3_520.0 * t).sin();
        }

        let spectrum = SpectrumDifference::measure(&reference, &candidate).expect("comparable");
        let at = |hz: f32| {
            spectrum
                .bands
                .iter()
                .find(|b| hz >= b.low_hz && hz < b.high_hz)
                .unwrap_or_else(|| panic!("no band contains {hz} Hz"))
        };
        assert!(at(3_520.0).delta_db > 20.0, "{:?}", at(3_520.0));
        assert!(at(440.0).delta_db.abs() < 0.5, "{:?}", at(440.0));
        assert!((spectrum.max_at_hz - at(3_520.0).low_hz).abs() < f32::EPSILON);
    }

    /// A difference in only one channel must move the stereo metrics and not the
    /// mono ones. Correlation is the metric that sees it.
    #[test]
    fn a_one_channel_change_moves_the_stereo_metrics() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let mut candidate = reference.clone();
        // Attenuate the right channel by 6 dB.
        for frame in candidate.interleaved.chunks_exact_mut(2) {
            frame[1] *= 0.5;
        }
        let stereo = StereoDifference::measure(&reference, &candidate).expect("both stereo");
        assert!((stereo.reference_correlation.expect("both channels vary") - 1.0).abs() < 1e-3);
        assert!((stereo.candidate_correlation.expect("both channels vary") - 1.0).abs() < 1e-3);
        assert!(
            (stereo.balance_delta_db + 6.0206).abs() < 0.05,
            "{stereo:?}"
        );
        assert!((stereo.candidate_right_dbfs - stereo.reference_right_dbfs + 6.0206).abs() < 0.05);
        assert!((stereo.candidate_left_dbfs - stereo.reference_left_dbfs).abs() < 1e-3);
    }

    /// Inverting one channel takes correlation from +1 to -1. A comparison that
    /// only looked at per-channel energy would score this as identical.
    #[test]
    fn an_inverted_channel_is_caught_by_correlation() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let mut candidate = reference.clone();
        for frame in candidate.interleaved.chunks_exact_mut(2) {
            frame[1] = -frame[1];
        }
        let stereo = StereoDifference::measure(&reference, &candidate).expect("both stereo");
        let delta = stereo.correlation_delta.expect("both sides measurable");
        assert!((delta + 2.0).abs() < 0.01, "{stereo:?}");
        let level = LevelDifference::measure(&reference, &candidate);
        assert!(
            level.rms_delta_db.abs() < 1e-3,
            "the change is invisible to level, which is the point"
        );
    }

    /// Envelope landmarks must track a change in envelope shape. A signal that
    /// stops halfway has an earlier tail end and an earlier 50 % fall.
    #[test]
    fn envelope_landmarks_track_a_shortened_signal() {
        let reference = tone(440.0, 0.5, 2.0, 0.0);
        let mut candidate = reference.clone();
        let half = candidate.interleaved.len() / 2;
        candidate.interleaved[half..].fill(0.0);

        let envelope = EnvelopeDifference::measure(&reference, &candidate).expect("measurable");
        assert!(
            envelope.delta_ms.tail_end_ms < -900.0,
            "tail end moved by {} ms, expected about -1000",
            envelope.delta_ms.tail_end_ms
        );
        assert!(
            envelope.max_delta_db > 60.0,
            "silence against a tone should be a large level difference, got {}",
            envelope.max_delta_db
        );
    }

    /// Sample-by-sample comparison must find the first divergence exactly, and
    /// must refuse signals with no frame-to-frame correspondence rather than
    /// subtracting them anyway.
    #[test]
    fn sample_comparison_locates_the_divergence_and_refuses_mismatches() {
        let reference = tone(440.0, 0.5, 1.0, 0.0);
        let mut candidate = reference.clone();
        candidate.interleaved[500] += 0.25;
        let difference = SampleDifference::measure(&reference, &candidate).expect("same base");
        assert!(!difference.identical);
        assert_eq!(difference.first_divergence_frame, Some(250));
        assert!((difference.max_abs_error - 0.25).abs() < 1e-6);

        let shorter = tone(440.0, 0.5, 0.5, 0.0);
        assert!(SampleDifference::measure(&reference, &shorter).is_none());
        let resampled = Signal {
            sample_rate: 48_000,
            ..reference.clone()
        };
        assert!(SampleDifference::measure(&reference, &resampled).is_none());
    }

    /// Metrics whose preconditions fail must be absent, never zero. A zero is
    /// this report's strongest claim and it must not be what "not measured"
    /// looks like.
    #[test]
    fn unmeasurable_metrics_are_absent_rather_than_zero() {
        let stereo = tone(440.0, 0.5, 2.0, 0.0);
        let mono = Signal {
            interleaved: stereo.mono(),
            sample_rate: SR,
            channels: 1,
        };
        assert!(StereoDifference::measure(&stereo, &mono).is_none());

        let other_rate = Signal {
            sample_rate: 48_000,
            ..stereo.clone()
        };
        assert!(SpectrumDifference::measure(&stereo, &other_rate).is_none());

        let silent = Signal {
            interleaved: vec![0.0; SR as usize * 2],
            sample_rate: SR,
            channels: 2,
        };
        assert!(TimingDifference::measure(&silent, &silent).is_none());
        assert!(EnvelopeDifference::measure(&silent, &silent).is_none());
        assert_eq!(LoudnessDifference::measure(&silent, &silent).delta_lu, None);

        // A silent channel is not "uncorrelated" with anything, and an unpitched
        // render's fundamental is not 0 Hz. Both underlying estimators return a
        // zero sentinel; neither may reach the report as one.
        let silent_stereo = StereoDifference::measure(&silent, &silent).expect("both stereo");
        assert_eq!(silent_stereo.reference_correlation, None);
        assert_eq!(silent_stereo.correlation_delta, None);
        assert!(
            silent_stereo.reference_left_dbfs.is_finite(),
            "energy is still reported"
        );

        let silent_pitch = PitchDifference::measure(&silent, &silent);
        assert_eq!(silent_pitch.reference_hz, None);
        assert_eq!(silent_pitch.candidate_hz, None);
        assert_eq!(silent_pitch.delta_cents, None);
    }

    /// The dB floor keeps silence finite: JSON has no representation for an
    /// infinity, and a report that fails to serialize after the work is done is
    /// worse than one that says -180.
    #[test]
    fn silence_reports_a_finite_floor() {
        assert!(dbfs(0.0).is_finite());
        assert!((dbfs(0.0) + 180.0).abs() < 0.01);
        assert!((dbfs(1.0)).abs() < 1e-6);
        assert!((dbfs(0.5) + 6.0206).abs() < 0.01);
    }

    /// Cents are undefined against an unpitched signal, and must not be invented.
    #[test]
    fn cents_need_two_real_frequencies() {
        assert!(cents_between(0.0, 440.0).is_none());
        assert!(cents_between(440.0, 0.0).is_none());
        assert!(cents_between(440.0, f32::NAN).is_none());
        assert!((cents_between(440.0, 880.0).expect("an octave") - 1200.0).abs() < 1e-3);
    }
}
