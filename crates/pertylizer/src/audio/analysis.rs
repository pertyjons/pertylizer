//! Offline audio analysis primitives.
//!
//! Used by the MCP `analyze_note` tool to inspect a buffer of samples that
//! has already been rendered to a `Vec<f32>` (e.g. from
//! [`crate::audio::preview::render_note_to_buffer`]). None of these
//! functions are real-time safe — they allocate, build FFT plans, and
//! return owned containers. Do not call them from the audio thread.
//!
//! Provides:
//! - basic statistics (`peak_amplitude`, `rms_overall`, `dc_offset`,
//!   `count_clipped`)
//! - block-wise envelopes (`rms_envelope`, `centroid_envelope`,
//!   `pitch_envelope`)
//! - spectral metrics (`spectrum_top_peaks`, `fundamental_frequency`,
//!   `energy_bands`, `harmonic_content`)
//! - structure summaries (`envelope_estimate`, `centroid_trend`,
//!   `stereo_correlation`)

use synth_core::types::StereoSample;
use synth_dsp::{Complex, FftProcessor, WindowType, fill_window};

/// One spectral peak: frequency in Hz and magnitude in dB relative to the
/// loudest peak in the spectrum (loudest = 0 dB).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SpectrumPeak {
    pub freq_hz: f32,
    pub magnitude_db: f32,
}

/// Per-band RMS energies for a single buffer.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct EnergyBands {
    /// RMS energy 0-100 Hz (sub).
    pub sub: f32,
    /// RMS energy 100-500 Hz (low/mid bass).
    pub low: f32,
    /// RMS energy 500-2000 Hz (presence).
    pub mid: f32,
    /// RMS energy 2000+ Hz (brilliance).
    pub high: f32,
}

impl EnergyBands {
    const fn zero() -> Self {
        Self {
            sub: 0.0,
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        }
    }

    /// Shannon entropy across the four bands, normalized so a perfectly
    /// uniform distribution scores 1.0 and a single-band signal scores 0.0.
    /// Silent buffers (total ≤ 1e-9) return 0.0.
    #[must_use]
    pub fn shannon_entropy_normalized(&self) -> f32 {
        let bands = [self.sub, self.low, self.mid, self.high];
        let total: f32 = bands.iter().sum();
        if total <= 1e-9 {
            return 0.0;
        }
        let mut h = 0.0_f32;
        for b in &bands {
            let p = b / total;
            if p > 1e-9 {
                h -= p * p.ln();
            }
        }
        (h / 4.0_f32.ln()).clamp(0.0, 1.0)
    }
}

/// Harmonic structure summary for a tonal signal at a known fundamental.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct HarmonicContent {
    /// Total harmonic distortion in dB. Power ratio of all harmonics
    /// (excluding fundamental) over the fundamental's power:
    /// `10 * log10(sum(|X(n*f0)|^2 for n>=2) / |X(f0)|^2)`.
    /// More negative = cleaner; -inf clamped to -120 dB.
    pub thd_db: f32,
    /// Power ratio of odd harmonics (3f0, 5f0, 7f0, ...) to even harmonics
    /// (2f0, 4f0, 6f0, ...), in dB. Positive = odd-dominant (square-like),
    /// negative = even-dominant (asymmetric clipping / tube saturation).
    /// 0.0 when either side is empty.
    pub odd_even_ratio_db: f32,
    /// Count of harmonics 2f0..N*f0 whose magnitude is at least 40 dB below
    /// the fundamental (i.e. visible above the noise floor for typical bass
    /// patches). Capped at 20.
    pub n_harmonics: u32,
}

impl HarmonicContent {
    const fn zero() -> Self {
        Self {
            thd_db: 0.0,
            odd_even_ratio_db: 0.0,
            n_harmonics: 0,
        }
    }
}

/// ADSR-like envelope estimate, derived purely from a precomputed RMS
/// envelope.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct EnvelopeEstimate {
    /// Time from index 0 to the index of the maximum RMS window, in ms.
    pub attack_ms: f32,
    /// Time from the RMS peak (within the held portion) to the first
    /// window where RMS reaches 1.2 × the average sustain level, in ms.
    /// Measured purely from `rms_envelope`. 0.0 if envelope never decays.
    pub decay_ms: f32,
    /// Average RMS over the second half of the held portion, divided by
    /// the peak RMS. Range [0.0, 1.0].
    pub sustain_level: f32,
    /// Time from note-off (= `note_duration_ms` mark in the envelope) to
    /// the first window where RMS falls below 5 % of the peak, in ms.
    /// Returns the envelope tail length if it never reaches that floor.
    pub release_ms: f32,
}

impl EnvelopeEstimate {
    const fn zero() -> Self {
        Self {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain_level: 0.0,
            release_ms: 0.0,
        }
    }
}

/// Floor used when converting magnitudes / powers to dB so we never call
/// `log10(0)`.
const MAG_FLOOR: f32 = 1.0e-12;

/// Floor for THD reporting. -120 dB is well below typical noise floors.
const THD_FLOOR_DB: f32 = -120.0;

/// Maximum number of harmonics counted by [`harmonic_content`] (2f0..21f0).
const MAX_HARMONICS: u32 = 20;

/// Choose an FFT size: largest power of two that fits in `len`, capped so
/// we don't grind through giant transforms on long buffers, and at least
/// 64 so very short buffers still produce something meaningful.
fn pick_fft_size(len: usize) -> usize {
    if len < 64 {
        return 0;
    }
    let mut size = 1usize;
    while size.saturating_mul(2) <= len {
        size *= 2;
    }
    size.min(32_768)
}

/// Round `n` up to the next power of two, capped at 65 536. Returns 0 for
/// `n == 0`.
fn fft_size_for(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    n.next_power_of_two().min(65_536)
}

/// Convert a window length in milliseconds to a sample count. Returns 0
/// if the inputs are invalid (zero rate, non-finite or non-positive ms).
fn window_samples(window_ms: f32, sample_rate: u32) -> usize {
    if sample_rate == 0 || !window_ms.is_finite() || window_ms <= 0.0 {
        return 0;
    }
    let samples = ((window_ms * 0.001) * sample_rate as f32).round() as i64;
    if samples < 1 { 0 } else { samples as usize }
}

/// Reusable FFT + Hann-window workspace. Owns a pre-allocated FFT processor,
/// window buffer, and scratch input/output buffers so callers in a loop
/// don't re-plan the FFT or rebuild the window on every iteration.
///
/// The Hann window is sized to the *data* length (not the FFT size). For
/// short buffers this keeps the tapering at the edges of the actual signal
/// instead of the zero-padded FFT frame, which dramatically improves
/// harmonic resolution for tonal analysis.
struct MagnitudeWorkspace {
    fft: FftProcessor,
    window: Vec<f32>,
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
}

impl MagnitudeWorkspace {
    fn new(fft_size: usize) -> Self {
        let complex_size = fft_size / 2 + 1;
        Self {
            fft: FftProcessor::new(fft_size),
            window: Vec::new(),
            fft_in: vec![0.0; fft_size],
            fft_out: vec![Complex::new(0.0, 0.0); complex_size],
        }
    }

    fn fft_size(&self) -> usize {
        self.fft.fft_size()
    }

    /// Window-and-FFT `samples` (zero-padded if shorter than `fft_size`)
    /// and return the magnitude of every bin (length = `fft_size / 2 + 1`).
    fn magnitudes(&mut self, samples: &[f32]) -> Vec<f32> {
        let n = self.fft_size();
        let take = samples.len().min(n);
        if self.window.len() != take {
            self.window.resize(take, 0.0);
            fill_window(&mut self.window, WindowType::Hann);
        }
        for i in 0..take {
            self.fft_in[i] = samples[i] * self.window[i];
        }
        for slot in &mut self.fft_in[take..] {
            *slot = 0.0;
        }
        self.fft.forward(&mut self.fft_in, &mut self.fft_out);
        self.fft_out.iter().map(|c| c.norm()).collect()
    }
}

/// Top `n` spectral peaks by magnitude (descending), each as
/// `(freq_hz, magnitude_db)`. Magnitudes are normalized so the loudest
/// peak is 0 dB. The DC bin is skipped. The signal is windowed with a
/// Hann window before the FFT.
///
/// Returns an empty vector if the buffer is too short to FFT or if the
/// FFT fails.
#[must_use]
pub fn spectrum_top_peaks(samples: &[f32], sample_rate: u32, n: usize) -> Vec<SpectrumPeak> {
    if n == 0 || sample_rate == 0 {
        return Vec::new();
    }
    let fft_size = pick_fft_size(samples.len());
    if fft_size == 0 || samples.len() < fft_size {
        return Vec::new();
    }
    let mut workspace = MagnitudeWorkspace::new(fft_size);
    let mags = workspace.magnitudes(&samples[..fft_size]);
    if mags.len() < 3 {
        return Vec::new();
    }

    // Find local maxima above the DC bin. Bin k (1 <= k < len-1) is a
    // peak when mags[k] strictly exceeds both neighbours.
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    for k in 1..mags.len() - 1 {
        let m = mags[k];
        if m > mags[k - 1] && m > mags[k + 1] && m > 0.0 {
            peaks.push((k, m));
        }
    }

    if peaks.is_empty() {
        return Vec::new();
    }

    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    peaks.truncate(n);

    let max_mag = peaks
        .first()
        .map(|(_, m)| *m)
        .unwrap_or(MAG_FLOOR)
        .max(MAG_FLOOR);

    let sr = sample_rate as f32;
    let fsz = fft_size as f32;
    peaks
        .into_iter()
        .map(|(k, m)| {
            let refined_bin = parabolic_refine_bin(&mags, k);
            let freq_hz = refined_bin * sr / fsz;
            let magnitude_db = 20.0 * (m.max(MAG_FLOOR) / max_mag).log10();
            SpectrumPeak {
                freq_hz,
                magnitude_db,
            }
        })
        .collect()
}

/// Parabolic interpolation around bin `k`. Returns the (possibly
/// fractional) bin index of the true peak. Falls back to `k` if `k` is
/// at the edge or the parabola is degenerate.
fn parabolic_refine_bin(mags: &[f32], k: usize) -> f32 {
    if k == 0 || k + 1 >= mags.len() {
        return k as f32;
    }
    let y0 = mags[k - 1];
    let y1 = mags[k];
    let y2 = mags[k + 1];
    let denom = y0 - 2.0 * y1 + y2;
    if denom.abs() < f32::EPSILON {
        return k as f32;
    }
    // Clamp so a noisy spectrum can't push the refined bin far away from
    // the discrete peak.
    let offset = (0.5 * (y0 - y2) / denom).clamp(-1.0, 1.0);
    k as f32 + offset
}

/// Loudest bin in `mags` whose centre frequency lies in `[min_hz, max_hz]`,
/// refined with parabolic interpolation. Returns 0.0 if the search range
/// is empty or the range is silent.
fn fundamental_in_mags(
    mags: &[f32],
    sample_rate: u32,
    fft_size: usize,
    min_hz: f32,
    max_hz: f32,
) -> f32 {
    fundamental_in_mags_with_confidence(mags, sample_rate, fft_size, min_hz, max_hz).0
}

/// Like [`fundamental_in_mags`] but also returns a confidence in `0.0..=1.0`.
///
/// Confidence is computed from the *prominence* of the loudest in-range bin
/// over the next-loudest bin that is far enough away not to be a parabolic-
/// interpolation neighbor. For a clean tonal signal the loudest bin towers
/// over everything else (confidence near 1.0); for noise or a signal with
/// multiple competing peaks (e.g. a Karplus delay-line with a strong octave
/// partial) the second-best bin is comparable in magnitude and confidence
/// drops toward 0.0.
fn fundamental_in_mags_with_confidence(
    mags: &[f32],
    sample_rate: u32,
    fft_size: usize,
    min_hz: f32,
    max_hz: f32,
) -> (f32, f32) {
    if mags.len() < 3 || sample_rate == 0 || min_hz < 0.0 || max_hz <= min_hz {
        return (0.0, 0.0);
    }
    let sr = sample_rate as f32;
    let fsz = fft_size as f32;
    let bin_hz = sr / fsz;

    let min_bin = (min_hz / bin_hz).ceil().max(1.0) as usize;
    let max_bin_inclusive = (max_hz / bin_hz).floor().max(0.0) as usize;
    let last_bin = mags.len().saturating_sub(2);
    let max_bin = max_bin_inclusive.min(last_bin);

    if min_bin > max_bin {
        return (0.0, 0.0);
    }

    let mut best_bin = min_bin;
    let mut best_mag = mags[min_bin];
    for k in (min_bin + 1)..=max_bin {
        if mags[k] > best_mag {
            best_mag = mags[k];
            best_bin = k;
        }
    }

    if best_mag <= 0.0 {
        return (0.0, 0.0);
    }

    // Find the next-best peak that is at least `min_separation` bins away
    // from `best_bin` to avoid counting the parabolic interpolation
    // neighbours of the same peak.
    const MIN_SEPARATION: usize = 3;
    let mut second_best: f32 = 0.0;
    for k in min_bin..=max_bin {
        if best_bin >= k && best_bin - k < MIN_SEPARATION {
            continue;
        }
        if k > best_bin && k - best_bin < MIN_SEPARATION {
            continue;
        }
        if mags[k] > second_best {
            second_best = mags[k];
        }
    }

    // Prominence-based confidence: 1.0 when the second-best peak is silent,
    // approaching 0 when the second-best peak matches the best. Clamp so
    // numerical noise can't push it slightly negative.
    let confidence = (1.0 - (second_best / best_mag)).clamp(0.0, 1.0);

    let freq = parabolic_refine_bin(mags, best_bin) * bin_hz;
    (freq, confidence)
}

/// Detect the fundamental frequency in Hz using FFT-peak with parabolic
/// interpolation around the loudest bin in the range `[min_hz, max_hz]`.
/// The signal is windowed with a Hann window. The DC bin is skipped.
///
/// Returns `0.0` if:
/// - the buffer is too short to FFT,
/// - the search range is empty / invalid,
/// - the loudest bin in range has zero magnitude.
#[must_use]
pub fn fundamental_frequency(samples: &[f32], sample_rate: u32, min_hz: f32, max_hz: f32) -> f32 {
    fundamental_frequency_with_confidence(samples, sample_rate, min_hz, max_hz).0
}

/// Below this confidence, the loudest in-range bin barely dominates its
/// competitors — typical for filter-resonance latching, Karplus delay-line
/// octave doubling, or formant-heavy / noise-based patches. Reported
/// `pitch_error_cents` should be treated as unreliable.
pub const PITCH_CONFIDENCE_RELIABLE_FLOOR: f32 = 0.3;

/// Like [`fundamental_frequency`] but additionally returns a confidence
/// estimate in `0.0..=1.0` based on how much the loudest in-range bin
/// dominates the next-loudest non-adjacent bin. See
/// [`PITCH_CONFIDENCE_RELIABLE_FLOOR`].
#[must_use]
pub fn fundamental_frequency_with_confidence(
    samples: &[f32],
    sample_rate: u32,
    min_hz: f32,
    max_hz: f32,
) -> (f32, f32) {
    if sample_rate == 0 || min_hz < 0.0 || max_hz <= min_hz {
        return (0.0, 0.0);
    }
    let fft_size = pick_fft_size(samples.len());
    if fft_size == 0 || samples.len() < fft_size {
        return (0.0, 0.0);
    }
    let mut workspace = MagnitudeWorkspace::new(fft_size);
    let mags = workspace.magnitudes(&samples[..fft_size]);
    fundamental_in_mags_with_confidence(&mags, sample_rate, fft_size, min_hz, max_hz)
}

/// Block-wise RMS over consecutive non-overlapping windows of `window_ms`
/// milliseconds. Returns one value per complete window. Trailing partial
/// windows are dropped.
#[must_use]
pub fn rms_envelope(samples: &[f32], sample_rate: u32, window_ms: f32) -> Vec<f32> {
    let n = window_samples(window_ms, sample_rate);
    if n == 0 || samples.len() < n {
        return Vec::new();
    }
    let inv_n = 1.0 / n as f32;
    samples
        .chunks_exact(n)
        .map(|chunk| {
            let mut sum_sq = 0.0_f32;
            for s in chunk {
                sum_sq += *s * *s;
            }
            (sum_sq * inv_n).sqrt()
        })
        .collect()
}

/// Block-wise spectral centroid in Hz over consecutive non-overlapping
/// windows of `window_ms`. Each window is Hann-windowed before its FFT.
/// The centroid for a silent window is reported as `0.0`. Trailing
/// partial windows are dropped.
#[must_use]
pub fn centroid_envelope(samples: &[f32], sample_rate: u32, window_ms: f32) -> Vec<f32> {
    let raw_n = window_samples(window_ms, sample_rate);
    if raw_n == 0 || samples.len() < raw_n {
        return Vec::new();
    }

    // Round window length up to a power of two so the FFT processor accepts
    // it. The window count is still driven by the requested raw window
    // length — any tail beyond a full raw window is dropped.
    let fft_size = raw_n.next_power_of_two().max(2);
    let num_windows = samples.len() / raw_n;
    if num_windows == 0 {
        return Vec::new();
    }

    let mut workspace = MagnitudeWorkspace::new(fft_size);
    let bin_hz = sample_rate as f32 / fft_size as f32;

    let mut out = Vec::with_capacity(num_windows);
    for w in 0..num_windows {
        let start = w * raw_n;
        // Slice exactly raw_n samples (= the requested window) and let
        // MagnitudeWorkspace zero-pad up to fft_size internally. Slicing
        // up to start+fft_size as we previously did meant adjacent windows
        // overlapped by (fft_size - raw_n) samples — e.g. a 50 ms window
        // at 44.1 kHz pulled in 93 ms of audio per window.
        let end = (start + raw_n).min(samples.len());
        let mags = workspace.magnitudes(&samples[start..end]);

        // Skip DC bin. f64 accumulators avoid loss of precision on long
        // FFTs with many small magnitudes.
        let mut weighted_sum = 0.0_f64;
        let mut total = 0.0_f64;
        for (k, m) in mags.iter().enumerate().skip(1) {
            let mag = f64::from(*m);
            weighted_sum += (k as f64) * f64::from(bin_hz) * mag;
            total += mag;
        }
        let centroid = if total > 0.0 {
            (weighted_sum / total) as f32
        } else {
            0.0
        };
        out.push(centroid);
    }

    out
}

/// Mean of the samples (DC offset). Returns `0.0` for an empty buffer.
#[must_use]
pub fn dc_offset(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    for s in samples {
        sum += f64::from(*s);
    }
    (sum / samples.len() as f64) as f32
}

/// Count of samples whose absolute value is `>= threshold`.
#[must_use]
pub fn count_clipped(samples: &[f32], threshold: f32) -> u32 {
    let mut count: u32 = 0;
    for s in samples {
        if s.abs() >= threshold {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Peak absolute amplitude. Returns `0.0` for an empty buffer.
#[must_use]
pub fn peak_amplitude(samples: &[f32]) -> f32 {
    let mut peak = 0.0_f32;
    for s in samples {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    peak
}

/// Overall RMS of the entire buffer. Returns `0.0` for an empty buffer.
#[must_use]
pub fn rms_overall(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum_sq = 0.0_f64;
    for s in samples {
        sum_sq += f64::from(*s) * f64::from(*s);
    }
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Per-window fundamental frequency. One value per non-overlapping window
/// of `window_ms`. Restricts the search to [`min_hz`, `max_hz`] each window.
/// Returns 0.0 for windows whose RMS is below `silence_rms` (caller picks a
/// threshold; 1e-3 is a reasonable default for unity-scaled audio).
#[must_use]
pub fn pitch_envelope(
    samples: &[f32],
    sample_rate: u32,
    window_ms: f32,
    min_hz: f32,
    max_hz: f32,
    silence_rms: f32,
) -> Vec<f32> {
    let n = window_samples(window_ms, sample_rate);
    if n == 0 || samples.len() < n {
        return Vec::new();
    }
    let fft_size = pick_fft_size(n);
    if fft_size == 0 {
        return Vec::new();
    }
    let mut workspace = MagnitudeWorkspace::new(fft_size);
    samples
        .chunks_exact(n)
        .map(|chunk| {
            if rms_overall(chunk) < silence_rms {
                0.0
            } else {
                let take = chunk.len().min(fft_size);
                let mags = workspace.magnitudes(&chunk[..take]);
                fundamental_in_mags(&mags, sample_rate, fft_size, min_hz, max_hz)
            }
        })
        .collect()
}

/// Pearson correlation between L and R channels of a stereo-interleaved
/// buffer (L0, R0, L1, R1, …). Returns 0.0 for empty/mono buffers or when
/// either channel has zero variance. Range [-1.0, 1.0]; 1.0 = identical
/// signals (mono duplicated to both channels), 0.0 = uncorrelated, -1.0 =
/// anti-phase.
#[must_use]
pub fn stereo_correlation(stereo_interleaved: &[f32]) -> f32 {
    let n_frames = stereo_interleaved.len() / 2;
    if n_frames == 0 {
        return 0.0;
    }

    // Single-pass: accumulate sums and cross-products, then derive the
    // covariance and variances at the end. cov(L,R) = E[LR] - E[L]E[R].
    let mut sum_l = 0.0_f64;
    let mut sum_r = 0.0_f64;
    let mut sum_ll = 0.0_f64;
    let mut sum_rr = 0.0_f64;
    let mut sum_lr = 0.0_f64;
    for frame in StereoSample::iter_frames(stereo_interleaved, n_frames) {
        let l = f64::from(frame.left);
        let r = f64::from(frame.right);
        sum_l += l;
        sum_r += r;
        sum_ll += l * l;
        sum_rr += r * r;
        sum_lr += l * r;
    }
    let n = n_frames as f64;
    let cov = sum_lr - sum_l * sum_r / n;
    let var_l = sum_ll - sum_l * sum_l / n;
    let var_r = sum_rr - sum_r * sum_r / n;
    let denom = (var_l * var_r).sqrt();
    if denom <= 0.0 {
        return 0.0;
    }
    (cov / denom).clamp(-1.0, 1.0) as f32
}

/// Split the spectrum into 4 bands and report the RMS energy in each.
/// Computed by Hann-windowed FFT then summing |X(f)|^2 across each band's
/// frequency bins (Parseval), reporting sqrt of that sum normalized by the
/// number of samples per band so the values are comparable to time-domain
/// RMS.
#[must_use]
pub fn energy_bands(samples: &[f32], sample_rate: u32) -> EnergyBands {
    if samples.is_empty() || sample_rate == 0 {
        return EnergyBands::zero();
    }
    let fft_size = fft_size_for(samples.len());
    if fft_size < 4 {
        return EnergyBands::zero();
    }
    let mut workspace = MagnitudeWorkspace::new(fft_size);
    let mags = workspace.magnitudes(samples);
    if mags.is_empty() {
        return EnergyBands::zero();
    }

    let sr = sample_rate as f32;
    let fsz = fft_size as f32;
    let bin_hz = sr / fsz;

    // Per-bin power. Skip DC. Parseval-style accumulation: sum of |X(k)|^2
    // in the band. The Hann window has a coherent gain of 0.5; we don't
    // try to undo that here because all four bands share the same scaling.
    let mut sums = [0.0_f64; 4];
    let mut counts = [0_usize; 4];

    let nyquist = sr * 0.5;
    let cutoffs = [100.0_f32, 500.0, 2000.0, nyquist.max(2000.0)];

    for (k, m) in mags.iter().enumerate().skip(1) {
        let f = (k as f32) * bin_hz;
        if f > nyquist {
            break;
        }
        let p = f64::from(*m) * f64::from(*m);
        let band = if f < cutoffs[0] {
            0
        } else if f < cutoffs[1] {
            1
        } else if f < cutoffs[2] {
            2
        } else {
            3
        };
        sums[band] += p;
        counts[band] += 1;
    }

    let mut out = [0.0_f32; 4];
    for i in 0..4 {
        if counts[i] > 0 {
            // realfft returns un-normalised forward DFT, so |X(k)|^2 is
            // roughly fft_size^2 / 4 times the per-sample power for a
            // unity sine. Divide by fft_size to bring the magnitude back
            // into time-domain scale, then by 2 for the Hann coherent-gain
            // factor.
            let mean_p = sums[i] / counts[i] as f64;
            let scale = 4.0_f64 / (fsz as f64 * fsz as f64);
            out[i] = (mean_p * scale).sqrt() as f32;
        }
    }

    EnergyBands {
        sub: out[0],
        low: out[1],
        mid: out[2],
        high: out[3],
    }
}

/// Analyze the harmonic structure of a (presumed-tonal) signal at a known
/// fundamental. Hann-windows the input, takes one FFT, looks up the
/// fundamental and integer-multiple bins. Returns an all-zero
/// `HarmonicContent` when `fundamental_hz <= 0.0` or the buffer is silent.
#[must_use]
pub fn harmonic_content(samples: &[f32], sample_rate: u32, fundamental_hz: f32) -> HarmonicContent {
    if !fundamental_hz.is_finite() || fundamental_hz <= 0.0 || sample_rate == 0 {
        return HarmonicContent::zero();
    }
    if samples.is_empty() {
        return HarmonicContent::zero();
    }

    let fft_size = fft_size_for(samples.len());
    if fft_size < 4 {
        return HarmonicContent::zero();
    }
    let mut workspace = MagnitudeWorkspace::new(fft_size);
    let mags = workspace.magnitudes(samples);
    if mags.len() < 4 {
        return HarmonicContent::zero();
    }

    let bin_hz = sample_rate as f32 / fft_size as f32;
    if bin_hz <= 0.0 {
        return HarmonicContent::zero();
    }

    let f0_bin = (fundamental_hz / bin_hz).round() as i64;
    if f0_bin < 1 || (f0_bin as usize) >= mags.len() {
        return HarmonicContent::zero();
    }
    let f0_mag = mags[f0_bin as usize];
    let f0_power = f0_mag * f0_mag;
    if f0_power <= 0.0 {
        return HarmonicContent::zero();
    }

    // 40 dB below fundamental magnitude = factor 0.01: used for the
    // n_harmonics "visible" count.
    let visibility_threshold = f0_mag * 0.01;

    let mut odd_power = 0.0_f64;
    let mut even_power = 0.0_f64;
    let mut total_harmonic_power = 0.0_f64;
    let mut n_harmonics: u32 = 0;

    for n in 2..=(MAX_HARMONICS + 1) {
        let bin = (fundamental_hz * n as f32 / bin_hz).round() as i64;
        if bin < 1 || (bin as usize) >= mags.len() {
            break;
        }
        let m = mags[bin as usize];
        let p = f64::from(m) * f64::from(m);
        total_harmonic_power += p;
        if n % 2 == 0 {
            even_power += p;
        } else {
            odd_power += p;
        }
        if m >= visibility_threshold {
            n_harmonics = n_harmonics.saturating_add(1);
        }
    }

    let thd_db = if total_harmonic_power <= 0.0 {
        THD_FLOOR_DB
    } else {
        let ratio = total_harmonic_power / f64::from(f0_power);
        let db = 10.0 * ratio.max(f64::from(MAG_FLOOR)).log10();
        (db as f32).max(THD_FLOOR_DB)
    };

    let odd_even_ratio_db = if odd_power <= 0.0 || even_power <= 0.0 {
        0.0
    } else {
        let ratio = odd_power / even_power;
        (10.0 * ratio.log10()) as f32
    };

    HarmonicContent {
        thd_db,
        odd_even_ratio_db,
        n_harmonics: n_harmonics.min(MAX_HARMONICS),
    }
}

/// Estimate ADSR-like values from an RMS envelope. `window_ms` is the
/// envelope's per-step duration; `note_duration_ms` is the held-note
/// duration (used to find note-off). All times are returned in ms.
#[must_use]
pub fn envelope_estimate(
    rms_envelope: &[f32],
    window_ms: f32,
    note_duration_ms: u32,
) -> EnvelopeEstimate {
    if rms_envelope.is_empty() || !window_ms.is_finite() || window_ms <= 0.0 {
        return EnvelopeEstimate::zero();
    }
    let len = rms_envelope.len();

    // f64 so very long notes don't overflow the floor() → i64 conversion.
    let note_off_idx_raw =
        ((f64::from(note_duration_ms) / f64::from(window_ms)).floor()).max(0.0) as i64;
    let note_off_idx = (note_off_idx_raw as usize).min(len);

    // Held portion is [0, note_off_idx). If the envelope ends before the
    // notional note-off, treat the whole envelope as held and skip the
    // release stage.
    let held_end = note_off_idx.min(len);
    if held_end == 0 {
        return EnvelopeEstimate::zero();
    }
    let held = &rms_envelope[..held_end];

    let mut peak = 0.0_f32;
    let mut peak_idx = 0_usize;
    for (i, v) in held.iter().enumerate() {
        if *v > peak {
            peak = *v;
            peak_idx = i;
        }
    }

    let attack_ms = (peak_idx as f32) * window_ms;

    let sustain_start = held_end / 2;
    let sustain_slice = &held[sustain_start..];
    let sustain_avg = if sustain_slice.is_empty() {
        0.0
    } else {
        let mut s = 0.0_f64;
        for v in sustain_slice {
            s += f64::from(*v);
        }
        (s / sustain_slice.len() as f64) as f32
    };
    let sustain_level = if peak > 0.0 {
        (sustain_avg / peak).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let decay_target = 1.2 * sustain_avg;
    let mut decay_ms = 0.0_f32;
    if peak > sustain_avg && peak_idx + 1 < held_end && sustain_avg > 0.0 {
        for i in (peak_idx + 1)..held_end {
            if held[i] <= decay_target {
                decay_ms = ((i - peak_idx) as f32) * window_ms;
                break;
            }
        }
    }

    // Each entry represents the RMS over a complete window — observing the
    // floor at window i means we needed `(i + 1)` windows of elapsed time
    // after note-off to confirm it.
    let release_ms = if note_off_idx >= len || peak <= 0.0 {
        0.0
    } else {
        let floor = peak * 0.05;
        let tail = &rms_envelope[note_off_idx..];
        let mut release_idx: Option<usize> = None;
        for (i, v) in tail.iter().enumerate() {
            if *v < floor {
                release_idx = Some(i);
                break;
            }
        }
        match release_idx {
            Some(i) => ((i + 1) as f32) * window_ms,
            None => (tail.len() as f32) * window_ms,
        }
    };

    EnvelopeEstimate {
        attack_ms,
        decay_ms,
        sustain_level,
        release_ms,
    }
}

/// Linear-regression slope of the centroid envelope over the held-note
/// region, in Hz per second. Positive = filter opening over the note,
/// negative = closing. Returns 0.0 for fewer than 3 windows.
#[must_use]
pub fn centroid_trend(centroid_envelope: &[f32], window_ms: f32) -> f32 {
    if centroid_envelope.len() < 3 || !window_ms.is_finite() || window_ms <= 0.0 {
        return 0.0;
    }
    let dt = f64::from(window_ms) * 0.001;
    if dt <= 0.0 {
        return 0.0;
    }

    let n = centroid_envelope.len() as f64;
    let mut sum_x = 0.0_f64;
    let mut sum_y = 0.0_f64;
    let mut sum_xx = 0.0_f64;
    let mut sum_xy = 0.0_f64;
    for (i, y) in centroid_envelope.iter().enumerate() {
        let x = (i as f64) * dt;
        let y = f64::from(*y);
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }

    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    slope as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn energy_bands_entropy_uniform_is_max() {
        let bands = EnergyBands {
            sub: 0.25,
            low: 0.25,
            mid: 0.25,
            high: 0.25,
        };
        assert!((bands.shannon_entropy_normalized() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn energy_bands_entropy_single_band_is_zero() {
        let bands = EnergyBands {
            sub: 1.0,
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        };
        assert!(bands.shannon_entropy_normalized().abs() < 1e-4);
    }

    fn sine(freq_hz: f32, amplitude: f32, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        let sr = sample_rate as f32;
        (0..num_samples)
            .map(|i| amplitude * (TAU * freq_hz * (i as f32) / sr).sin())
            .collect()
    }

    fn saw(freq_hz: f32, amplitude: f32, sample_rate: u32, num_samples: usize) -> Vec<f32> {
        // Naive (band-unlimited) saw — fine for spectrum smoke tests.
        let sr = sample_rate as f32;
        (0..num_samples)
            .map(|i| {
                let phase = ((i as f32) * freq_hz / sr).fract();
                amplitude * (2.0 * phase - 1.0)
            })
            .collect()
    }

    /// Approximate band-limited square wave as the sum of odd harmonics
    /// 1, 3, 5, ... up to (but excluding) Nyquist, with amplitude 1/n for
    /// each harmonic — the truncated Fourier series of a unit square.
    fn square_additive(
        freq_hz: f32,
        sample_rate: u32,
        num_samples: usize,
        max_harmonics: u32,
    ) -> Vec<f32> {
        let sr = sample_rate as f32;
        let nyquist = sr * 0.5;
        let mut out = vec![0.0_f32; num_samples];
        let mut n = 1_u32;
        while n <= max_harmonics * 2 + 1 {
            let f = freq_hz * n as f32;
            if f >= nyquist {
                break;
            }
            let amp = 1.0 / n as f32;
            for (i, slot) in out.iter_mut().enumerate() {
                *slot += amp * (TAU * f * (i as f32) / sr).sin();
            }
            n += 2;
        }
        out
    }

    #[test]
    fn fundamental_pure_sine_440() {
        let samples = sine(440.0, 0.8, 44_100, 44_100);
        let f0 = fundamental_frequency(&samples, 44_100, 50.0, 5_000.0);
        assert!((f0 - 440.0).abs() < 2.0, "expected ~440 Hz, got {f0}");
    }

    #[test]
    fn peak_and_rms_of_sine() {
        let samples = sine(100.0, 0.5, 44_100, 44_100);
        let peak = peak_amplitude(&samples);
        assert!((peak - 0.5).abs() < 1.0e-3, "peak was {peak}");
        let rms = rms_overall(&samples);
        let expected = 0.5 / 2.0_f32.sqrt();
        assert!(
            (rms - expected).abs() < 1.0e-3,
            "rms was {rms}, expected ~{expected}",
        );
    }

    #[test]
    fn dc_offset_constant() {
        let samples = vec![0.3_f32; 1_024];
        let dc = dc_offset(&samples);
        assert!((dc - 0.3).abs() < 1.0e-6, "dc was {dc}");
    }

    #[test]
    fn spectrum_top_peak_saw_200() {
        let samples = saw(200.0, 0.7, 44_100, 44_100);
        let peaks = spectrum_top_peaks(&samples, 44_100, 5);
        assert!(!peaks.is_empty(), "expected at least one peak");
        let first = peaks[0];
        // Saw fundamental should dominate; allow a few Hz of slack for
        // the peak bin + parabolic refinement.
        assert!(
            (first.freq_hz - 200.0).abs() < 5.0,
            "first peak was {} Hz, expected ~200",
            first.freq_hz,
        );
        assert!(
            first.magnitude_db.abs() < 1.0e-3,
            "loudest peak should be 0 dB, got {}",
            first.magnitude_db,
        );
    }

    #[test]
    fn silent_buffer_behavior() {
        let samples = vec![0.0_f32; 8_192];
        let f0 = fundamental_frequency(&samples, 44_100, 50.0, 5_000.0);
        assert_eq!(f0, 0.0);

        let centroid = centroid_envelope(&samples, 44_100, 10.0);
        assert!(!centroid.is_empty(), "expected at least one window");
        for c in centroid {
            assert_eq!(c, 0.0, "silent window centroid must be 0.0");
        }
    }

    #[test]
    fn count_clipped_all_ones() {
        let samples = vec![1.0_f32; 256];
        assert_eq!(count_clipped(&samples, 0.999), samples.len() as u32);
    }

    #[test]
    fn rms_envelope_window_count() {
        // 100 samples at 1 kHz, 10 ms window = 10 samples → 10 windows.
        let samples = vec![1.0_f32; 100];
        let env = rms_envelope(&samples, 1_000, 10.0);
        assert_eq!(env.len(), 10);
        for v in env {
            assert!((v - 1.0).abs() < 1.0e-6);
        }
    }

    #[test]
    fn centroid_envelope_drops_partial_window() {
        // 105 samples at 1 kHz, 10 ms window = 10 samples → 10 full
        // windows, last 5 dropped.
        let samples = vec![0.5_f32; 105];
        let env = centroid_envelope(&samples, 1_000, 10.0);
        assert_eq!(env.len(), 10);
    }

    #[test]
    fn empty_buffer_defaults() {
        let empty: Vec<f32> = Vec::new();
        assert_eq!(peak_amplitude(&empty), 0.0);
        assert_eq!(rms_overall(&empty), 0.0);
        assert_eq!(dc_offset(&empty), 0.0);
        assert_eq!(count_clipped(&empty, 0.999), 0);
        assert!(spectrum_top_peaks(&empty, 44_100, 4).is_empty());
        assert_eq!(fundamental_frequency(&empty, 44_100, 50.0, 5_000.0), 0.0);
        assert!(rms_envelope(&empty, 44_100, 10.0).is_empty());
        assert!(centroid_envelope(&empty, 44_100, 10.0).is_empty());
    }

    #[test]
    fn pitch_envelope_pure_sine_100() {
        let samples = sine(100.0, 0.6, 44_100, 44_100);
        let env = pitch_envelope(&samples, 44_100, 100.0, 50.0, 5_000.0, 1.0e-3);
        assert_eq!(env.len(), 10);
        for (i, f) in env.iter().enumerate() {
            assert!(
                (f - 100.0).abs() < 2.0,
                "window {i}: expected ~100 Hz, got {f}",
            );
        }
    }

    #[test]
    fn pitch_envelope_silence() {
        let samples = vec![0.0_f32; 44_100];
        let env = pitch_envelope(&samples, 44_100, 100.0, 50.0, 5_000.0, 1.0e-3);
        assert_eq!(env.len(), 10);
        for f in env {
            assert_eq!(f, 0.0, "silent window must report 0 Hz");
        }
    }

    #[test]
    fn stereo_correlation_identical() {
        let mut buf = Vec::with_capacity(2_000);
        for i in 0..1_000 {
            let s = (TAU * 100.0 * (i as f32) / 44_100.0).sin();
            buf.push(s);
            buf.push(s);
        }
        let c = stereo_correlation(&buf);
        assert!((c - 1.0).abs() < 1.0e-3, "expected ~1.0, got {c}");
    }

    #[test]
    fn stereo_correlation_anti_phase() {
        let mut buf = Vec::with_capacity(2_000);
        for i in 0..1_000 {
            let s = (TAU * 100.0 * (i as f32) / 44_100.0).sin();
            buf.push(s);
            buf.push(-s);
        }
        let c = stereo_correlation(&buf);
        assert!((c + 1.0).abs() < 1.0e-3, "expected ~-1.0, got {c}");
    }

    #[test]
    fn stereo_correlation_uncorrelated() {
        // Two unrelated sines at incommensurate frequencies for a long
        // buffer — Pearson correlation should be near zero.
        let mut buf = Vec::with_capacity(8_000);
        for i in 0..4_000 {
            let l = (TAU * 137.0 * (i as f32) / 44_100.0).sin();
            let r = (TAU * 911.0 * (i as f32) / 44_100.0).cos();
            buf.push(l);
            buf.push(r);
        }
        let c = stereo_correlation(&buf);
        assert!(c.abs() < 0.5, "expected near 0, got {c}");
    }

    #[test]
    fn energy_bands_sub_dominant() {
        let samples = sine(50.0, 1.0, 44_100, 44_100);
        let bands = energy_bands(&samples, 44_100);
        assert!(
            bands.sub > bands.low * 5.0,
            "sub={}, low={}, mid={}, high={}",
            bands.sub,
            bands.low,
            bands.mid,
            bands.high,
        );
        assert!(bands.sub > bands.mid * 5.0);
        assert!(bands.sub > bands.high * 5.0);
    }

    #[test]
    fn energy_bands_mid_dominant() {
        let samples = sine(1_000.0, 1.0, 44_100, 44_100);
        let bands = energy_bands(&samples, 44_100);
        assert!(
            bands.mid > bands.sub * 5.0
                && bands.mid > bands.low * 5.0
                && bands.mid > bands.high * 5.0,
            "expected mid-dominant: sub={}, low={}, mid={}, high={}",
            bands.sub,
            bands.low,
            bands.mid,
            bands.high,
        );
    }

    #[test]
    fn harmonic_content_square_odd_dominant() {
        let samples = square_additive(200.0, 44_100, 44_100, 20);
        let h = harmonic_content(&samples, 44_100, 200.0);
        assert!(
            h.odd_even_ratio_db > 20.0,
            "expected odd-dominant ratio > 20 dB, got {} dB",
            h.odd_even_ratio_db,
        );
        assert!(h.n_harmonics > 0, "expected to count some harmonics");
    }

    #[test]
    fn harmonic_content_pure_sine() {
        let samples = sine(440.0, 0.8, 44_100, 44_100);
        let h = harmonic_content(&samples, 44_100, 440.0);
        assert!(
            h.thd_db < -80.0,
            "expected very low THD for pure sine, got {} dB",
            h.thd_db,
        );
        assert_eq!(h.n_harmonics, 0, "pure sine should report 0 harmonics");
    }

    #[test]
    fn envelope_estimate_basic() {
        let env = [0.1_f32, 1.0, 0.8, 0.6, 0.5, 0.5, 0.5, 0.0, 0.0];
        // 7 windows of "held" portion at 10 ms each = 70 ms note.
        let result = envelope_estimate(&env, 10.0, 70);
        assert!(
            (result.attack_ms - 10.0).abs() < 1.0e-4,
            "attack_ms = {}",
            result.attack_ms,
        );
        // Sustain: avg of windows [3..7] = (0.6 + 0.5 + 0.5 + 0.5)/4 = 0.525,
        // peak = 1.0 → sustain_level ≈ 0.525.
        assert!(
            (result.sustain_level - 0.525).abs() < 0.05,
            "sustain_level = {}",
            result.sustain_level,
        );
        assert!(
            result.release_ms > 0.0,
            "release_ms should be positive, got {}",
            result.release_ms,
        );
    }

    #[test]
    fn envelope_estimate_empty() {
        let env: Vec<f32> = Vec::new();
        let r = envelope_estimate(&env, 10.0, 100);
        assert_eq!(r.attack_ms, 0.0);
        assert_eq!(r.decay_ms, 0.0);
        assert_eq!(r.sustain_level, 0.0);
        assert_eq!(r.release_ms, 0.0);
    }

    #[test]
    fn centroid_trend_linear() {
        // [100, 200, 300, 400, 500] with 100 ms windows → 1000 Hz/s.
        let env = [100.0_f32, 200.0, 300.0, 400.0, 500.0];
        let slope = centroid_trend(&env, 100.0);
        assert!((slope - 1_000.0).abs() < 1.0, "slope = {slope}");
    }

    #[test]
    fn centroid_trend_flat() {
        let env = [500.0_f32, 500.0, 500.0, 500.0];
        let slope = centroid_trend(&env, 100.0);
        assert!(slope.abs() < 1.0e-3, "slope = {slope}");
    }

    #[test]
    fn centroid_trend_too_short() {
        let env = [500.0_f32, 600.0];
        assert_eq!(centroid_trend(&env, 100.0), 0.0);
    }

    // ------------------------------------------------------------------
    // Tests for code-review fixes (centroid window, pitch confidence)
    // ------------------------------------------------------------------

    #[test]
    fn pitch_confidence_high_for_pure_sine() {
        // A pure 440 Hz sine should have very high confidence — no other
        // significant peaks in the spectrum compete.
        let s = sine(440.0, 1.0, 44_100, 8192);
        let (freq, conf) = fundamental_frequency_with_confidence(&s, 44_100, 100.0, 1000.0);
        assert!((freq - 440.0).abs() < 5.0, "freq = {freq}");
        assert!(
            conf > 0.9,
            "pure sine should have confidence > 0.9, got {conf}"
        );
    }

    #[test]
    fn pitch_confidence_low_for_white_noise() {
        // White noise has no dominant peak; confidence should be low.
        // Use a deterministic LCG so the test is reproducible.
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let noise: Vec<f32> = (0..8192)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let s = (state >> 32) as i32 as f32 / i32::MAX as f32;
                s * 0.5
            })
            .collect();
        let (_freq, conf) = fundamental_frequency_with_confidence(&noise, 44_100, 100.0, 5000.0);
        assert!(
            conf < 0.5,
            "white noise should have confidence < 0.5, got {conf}"
        );
    }

    #[test]
    fn pitch_confidence_low_for_two_equal_peaks() {
        // Sum of two pure sines at 440 Hz and 880 Hz with equal amplitude.
        // The detector picks one as the fundamental, but the confidence
        // should be near 0 because the second peak is exactly as loud.
        let sr: u32 = 44_100;
        let n = 8192;
        let s1 = sine(440.0, 0.5, sr, n);
        let s2 = sine(880.0, 0.5, sr, n);
        let mixed: Vec<f32> = s1.iter().zip(s2.iter()).map(|(a, b)| a + b).collect();
        let (_freq, conf) = fundamental_frequency_with_confidence(&mixed, sr, 100.0, 2000.0);
        assert!(
            conf < 0.2,
            "two-equal-peaks should have low confidence (≈0), got {conf}"
        );
    }

    #[test]
    fn pitch_confidence_zero_for_silent() {
        let silent = vec![0.0_f32; 4096];
        let (freq, conf) = fundamental_frequency_with_confidence(&silent, 44_100, 100.0, 1000.0);
        assert_eq!(freq, 0.0);
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn fundamental_frequency_back_compat() {
        // The old single-return API must keep working unchanged.
        let s = sine(440.0, 1.0, 44_100, 8192);
        let f = fundamental_frequency(&s, 44_100, 100.0, 1000.0);
        assert!((f - 440.0).abs() < 5.0, "f = {f}");
    }

    #[test]
    fn centroid_envelope_window_does_not_overlap() {
        // Regression test for finding #7. Build a buffer of exactly 8 raw
        // windows (8 × 50 ms = 400 ms at 44.1 kHz). The first half is a
        // 200 Hz sine, the second half a 4000 Hz sine. With the previous
        // bug, the FFT slice for window N pulled in samples from window
        // N+1 (because fft_size > raw_n), so the centroid mid-buffer
        // would smear across the boundary. After the fix, each window
        // sees only its own raw_n samples.
        let sr = 44_100u32;
        let raw_n = 2_205_usize; // 50 ms
        let total = 8 * raw_n;
        let mut buf = Vec::with_capacity(total);
        let half = total / 2;
        for i in 0..total {
            let t = (i as f32) / (sr as f32);
            let v = if i < half {
                (TAU * 200.0 * t).sin()
            } else {
                (TAU * 4000.0 * t).sin()
            };
            buf.push(v);
        }
        let env = centroid_envelope(&buf, sr, 50.0);
        assert_eq!(env.len(), 8, "expected 8 windows, got {}", env.len());
        // Windows 0..3 should be near 200 Hz (well below 1000 Hz);
        // windows 4..7 should be near 4000 Hz (well above 1000 Hz).
        for (i, c) in env.iter().enumerate().take(4) {
            assert!(
                *c < 1000.0,
                "window {i} expected to track 200 Hz tone, centroid = {c}"
            );
        }
        for (i, c) in env.iter().enumerate().skip(4) {
            assert!(
                *c > 1500.0,
                "window {i} expected to track 4000 Hz tone, centroid = {c}"
            );
        }
    }

    #[test]
    fn centroid_envelope_step_at_window_boundary_is_clean() {
        // Strict test: the centroid at the LAST window of the low-frequency
        // half should be close to 200 Hz, and the FIRST window of the
        // high-frequency half should be close to 4000 Hz. With the old
        // overlapping-window bug, window 3 (the last low-freq one) would
        // have been smeared by samples from the first high-freq window.
        let sr = 44_100u32;
        let raw_n = 2_205_usize;
        let total = 8 * raw_n;
        let mut buf = Vec::with_capacity(total);
        let half = total / 2;
        for i in 0..total {
            let t = (i as f32) / (sr as f32);
            let v = if i < half {
                (TAU * 200.0 * t).sin()
            } else {
                (TAU * 4000.0 * t).sin()
            };
            buf.push(v);
        }
        let env = centroid_envelope(&buf, sr, 50.0);
        // Last low-frequency window must NOT be contaminated by the
        // high-frequency content that follows. Previous bug would push
        // this centroid up dramatically.
        assert!(
            env[3] < 600.0,
            "last low-freq window centroid leaked: {}",
            env[3]
        );
        // First high-frequency window must NOT be pulled down by leftover
        // low-frequency content.
        assert!(
            env[4] > 2500.0,
            "first high-freq window centroid pulled down: {}",
            env[4]
        );
    }
}
