//! Detailed offline spectrum analysis: detected partials, harmonicity, and
//! timbre descriptors that separate sounds the coarse 4-band
//! [`energy_bands`](super::energy_bands) metric cannot (a plain triangle, a
//! ring-modulated triangle, and a metallic carrier have near-identical 4-band
//! energy yet very different partial structure).
//!
//! Everything here is a pure function over `&[f32] + sample_rate` so it is unit
//! testable without an engine or MCP. None of it is real-time safe — it
//! allocates and builds FFT plans.
//!
//! Design notes (see `plans/spectral-analysis-mcp-plan.md`):
//! - The frame is windowed (Blackman-Nuttall by default, ≈ −98 dB sidelobes so
//!   weak partials next to strong ones survive) then **zero-padded to a power of
//!   two** before the real FFT. Zero-padding interpolates the spectrum so an
//!   isolated peak is *located* precisely even from a short frame; it does not
//!   *resolve* partials closer than the window's main lobe (a hard limit).
//! - Peaks are refined with parabolic interpolation on the **log**-magnitude
//!   spectrum (sub-bin frequency accuracy).
//! - f0 uses a normalized square-difference function (McLeod / NSDF), **not**
//!   FFT-peak or HPS, because triangles / ring-mod tones have a weak or missing
//!   fundamental. An *unvoiced* verdict (`f0 = None`) keeps noise frames from
//!   emitting a garbage fundamental that would poison the harmonic metrics.

use synth_core::types::{Cents, Decibels, Hertz, NormalizedValue, Seconds};
use synth_dsp::WindowType;

use super::{EnergyBands, MAG_FLOOR, MagnitudeWorkspace};

/// NSDF clarity below which a frame is declared unvoiced (no reliable pitch).
const VOICED_NSDF_THRESHOLD: f32 = 0.4;
/// Spectral flatness above which a frame is declared unvoiced (noise-like).
const VOICED_FLATNESS_THRESHOLD: f32 = 0.5;

/// Default FFT size (zero-pad target). 8192 bins → ≈ 5.4 Hz raw spacing at
/// 44.1 kHz, sharpened well below that by parabolic interpolation.
const DEFAULT_FFT_SIZE: usize = 8192;
/// Default number of partials returned.
const DEFAULT_MAX_PARTIALS: u32 = 48;

/// Lower edge of the fixed log-bin reference band (Hz).
const LOG_BINS_BOTTOM_HZ: f32 = 20.0;
/// Upper edge of the fixed log-bin reference band (Hz). Capped at 16 kHz — at or
/// below the Nyquist of every supported rate (32 kHz emulator dumps included) —
/// so a bin never sits in a band one source physically cannot represent. Kept
/// sample-rate independent so `compare`'s broadband distance aligns bins across
/// differing sample rates (a 32 kHz reference vs a 44.1 kHz candidate).
const LOG_BINS_TOP_HZ: f32 = 16_000.0;

/// Digital-floor clamp for `log_spectral_distance`: a bin is clamped *up* to
/// this before diffing, so a −70 dB reference floor vs a −120 dB digital floor
/// reads as a 10 dB difference, not 50.
const LSD_FLOOR_DB: f32 = -80.0;
/// Peak-relative shelf below which a bin carries no timbral information: bins
/// this far below BOTH sources' own peaks are shared nulls (inter-partial gaps,
/// resampler leakage) and are excluded from the RMS and from `floor_coverage`.
/// −50 dB ≈ "inaudible relative to the loudest partial" — deep enough to keep
/// real low harmonics, shallow enough to drop the nulls that otherwise dominate
/// the scalar at sparse-harmonic pitches (calibration plan, TODO §4.1).
const LSD_SHELF_DB: f32 = -50.0;
/// Live-bin fraction below which the distance rests on too few informative bins
/// to trust as a broadband scalar — the caller should read `missing/extra
/// partials` + `centroid_delta` instead (`floor_limited`).
const FLOOR_COVERAGE_MIN: f32 = 0.15;
/// Absolute broadband RMS below which a frame is unconditionally silent for
/// [`compare`]'s guard (≈ −80 dBFS). Peak-normalising a near-silent frame
/// amplifies its noise floor to full scale, so silence-vs-silence otherwise
/// explodes into 100+ dB of meaningless noise-vs-noise distance.
const SILENCE_RMS: f32 = 1e-4;
/// Broadband RMS at/below which a voice is *effectively* silent (≈ −50 dBFS):
/// the DAC/resampler residue a chip leaves when it collapses a voice. Two
/// sources both under this agree (distance 0, `floor_limited`), and a frame
/// under it never reads `voiced` (so it can't arm the voicing-mismatch penalty
/// on near-silence). Must stay below the quietest musically-real sustain.
const NEAR_FLOOR_RMS: f32 = 3.0e-3;

/// Lowest fundamental the pitch tracker will consider (Hz) when no hint is set.
const F0_SEARCH_MIN_HZ: f32 = 40.0;
/// Highest fundamental the pitch tracker will consider (Hz) when no hint is set.
const F0_SEARCH_MAX_HZ: f32 = 2000.0;

/// Window applied before the FFT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFn {
    /// Blackman-Nuttall — very low sidelobes (≈ −98 dB); the default, best for
    /// resolving weak partials next to strong ones.
    BlackmanNuttall,
    /// Hann — narrower main lobe but much higher sidelobes (≈ −32 dB); faster
    /// but lets strong partials mask weak neighbours.
    Hann,
    /// Hamming.
    Hamming,
}

impl WindowFn {
    fn to_dsp(self) -> WindowType {
        match self {
            Self::BlackmanNuttall => WindowType::BlackmanNuttall,
            Self::Hann => WindowType::Hann,
            Self::Hamming => WindowType::Hamming,
        }
    }
}

/// Options controlling [`analyze_spectrum`].
#[derive(Debug, Clone, Copy)]
pub struct SpectrumOpts {
    /// Restricts the pitch-tracker's lag search to a fifth either side of this
    /// frequency, killing octave errors while still tracking vibrato/detune.
    /// `None` searches the full musical range.
    pub f0_hint: Option<Hertz>,
    /// Maximum number of partials returned (descending amplitude).
    pub max_partials: u32,
    /// Number of log-spaced magnitude bins to emit (0 = off). Used by
    /// [`compare`] for the broadband log-spectral distance.
    pub log_bins: u32,
    /// Window function applied before the FFT.
    pub window: WindowFn,
    /// FFT size (zero-pad target); rounded up to a power of two internally.
    pub fft_size: usize,
}

impl Default for SpectrumOpts {
    fn default() -> Self {
        Self {
            f0_hint: None,
            max_partials: DEFAULT_MAX_PARTIALS,
            log_bins: 0,
            window: WindowFn::BlackmanNuttall,
            fft_size: DEFAULT_FFT_SIZE,
        }
    }
}

/// A detected spectral peak.
#[derive(Debug, Clone, Copy)]
pub struct Partial {
    /// Refined peak frequency.
    pub frequency: Hertz,
    /// Peak-normalised amplitude (loudest partial = 0 dB).
    pub amplitude: Decibels,
    /// Nearest harmonic number `n` in `n·f0`; `None` when the frame is unvoiced.
    pub harmonic_number: Option<u32>,
    /// Signed deviation of `frequency` from `harmonic_number·f0` in cents
    /// (0 when unvoiced / untagged).
    pub inharmonicity: Cents,
}

/// Full spectrum descriptor for one frame.
#[derive(Debug, Clone)]
pub struct SpectrumResult {
    /// Detected fundamental; `None` = unvoiced (noise frame).
    pub f0: Option<Hertz>,
    /// `false` → harmonic metrics are not meaningful; treat as noise.
    pub voiced: bool,
    /// Top-N peaks, descending amplitude.
    pub partials: Vec<Partial>,
    /// Spectral centroid (brightness).
    pub centroid: Hertz,
    /// 0 = pure tone … 1 = white noise (geometric/arithmetic mean ratio).
    pub flatness: NormalizedValue,
    /// Frequency below which 85 % of the energy lies.
    pub rolloff: Hertz,
    /// Aggregate energy-weighted `|partial − n·f0| / f0` (0 if unvoiced).
    pub inharmonicity: NormalizedValue,
    /// Σ odd-harmonic / Σ even-harmonic power (dimensionless). Large (→ ~1000)
    /// for odd-only tones (square), ~1 balanced, → 0 even-dominant; 0 if
    /// unvoiced or no harmonics above the fundamental.
    pub odd_even_ratio: f32,
    /// The existing 4-band metric, for continuity with `analyze_mix_bus`.
    pub bands: EnergyBands,
    /// Optional log-spaced magnitude bins (dB, peak-normalised); empty when
    /// `opts.log_bins == 0`.
    pub log_bins: Vec<Decibels>,
    /// Broadband time-domain RMS of the analysed (DC-removed) frame — the
    /// absolute-level signal [`compare`]'s silence guard needs, since the log
    /// bins and partials are peak-normalised.
    pub frame_rms: f32,
}

impl SpectrumResult {
    /// A well-defined empty/unvoiced result for silent or degenerate input.
    fn empty(bands: EnergyBands) -> Self {
        Self {
            f0: None,
            voiced: false,
            partials: Vec::new(),
            centroid: Hertz::new(0.0),
            flatness: NormalizedValue::new_unchecked(0.0),
            rolloff: Hertz::new(0.0),
            inharmonicity: NormalizedValue::new_unchecked(0.0),
            odd_even_ratio: 0.0,
            bands,
            log_bins: Vec::new(),
            frame_rms: 0.0,
        }
    }
}

/// Internal peak with the linear magnitude kept around for energy ratios.
#[derive(Debug, Clone, Copy)]
struct Peak {
    frequency: f32,
    magnitude: f32,
    harmonic_number: Option<u32>,
    inharmonicity_cents: f32,
}

/// Analyze one mono frame and return its full spectral descriptor.
///
/// `signal` is mono (the caller mono-sums stereo first). Returns a finite,
/// well-defined result for any input — silence, DC, and noise never panic or
/// produce `NaN`.
#[must_use]
pub fn analyze_spectrum(signal: &[f32], sample_rate: u32, opts: SpectrumOpts) -> SpectrumResult {
    let fft_size = opts.fft_size.max(64).next_power_of_two();
    let mut workspace = MagnitudeWorkspace::with_window(fft_size, opts.window.to_dsp());
    analyze_with_workspace(signal, sample_rate, opts, &mut workspace)
}

/// Analyse one frame using a caller-provided (and reused) FFT workspace. The
/// workspace's `fft_size` and window are authoritative — `opts.fft_size`/
/// `opts.window` are ignored here, since the spectrogram path builds the
/// workspace once and slides it across every frame.
fn analyze_with_workspace(
    signal: &[f32],
    sample_rate: u32,
    opts: SpectrumOpts,
    workspace: &mut MagnitudeWorkspace,
) -> SpectrumResult {
    let bands = super::energy_bands(signal, sample_rate);

    if sample_rate == 0 || signal.len() < 2 {
        return SpectrumResult::empty(bands);
    }
    // Remove DC before analysis: a constant (pure-DC) frame is not pitched, and
    // DC leakage through the window would otherwise smear the low bins. After
    // removal a constant frame has zero energy and is reported unvoiced.
    let mean = signal.iter().sum::<f32>() / signal.len() as f32;
    let demeaned: Vec<f32> = signal.iter().map(|&s| s - mean).collect();
    let energy: f32 = demeaned.iter().map(|&s| s * s).sum();
    if energy <= MAG_FLOOR {
        return SpectrumResult::empty(bands);
    }
    #[allow(clippy::cast_precision_loss)]
    let frame_rms = (energy / demeaned.len() as f32).sqrt();
    let signal = &demeaned[..];

    let fft_size = workspace.fft_size();
    let mags = workspace.magnitudes(signal);
    // Usable bins: skip DC (0) and Nyquist (last) so aliasing artefacts at
    // Fs/2 never appear as partials.
    if mags.len() < 3 {
        return SpectrumResult::empty(bands);
    }
    let sr = sample_rate as f32;
    let bin_hz = sr / fft_size as f32;

    let flatness = spectral_flatness(&mags);
    let centroid = spectral_centroid(&mags, bin_hz);
    let rolloff = spectral_rolloff(&mags, bin_hz, 0.85);

    // Pitch via NSDF on the time-domain frame. A near-silent frame never reads
    // voiced: NSDF finds a spurious "pitch" in DAC/resampler noise, which would
    // otherwise arm `compare`'s +60 dB voicing-mismatch penalty against a
    // collapsed voice (calibration plan, TODO §4.1).
    let (f0, clarity) = detect_f0(signal, sample_rate, opts.f0_hint);
    let voiced = frame_rms > NEAR_FLOOR_RMS
        && clarity >= VOICED_NSDF_THRESHOLD
        && flatness <= VOICED_FLATNESS_THRESHOLD;
    let f0 = if voiced { f0 } else { None };

    // Peak picking → parabolic refine (log magnitude) → harmonic tagging.
    let mut peaks = pick_peaks(&mags, bin_hz);
    peaks.sort_by(|a, b| b.magnitude.total_cmp(&a.magnitude));
    peaks.truncate(opts.max_partials as usize);

    if let Some(f0_hz) = f0 {
        for peak in &mut peaks {
            tag_harmonic(peak, f0_hz.0);
        }
    }

    let inharmonicity = aggregate_inharmonicity(&peaks, f0);
    let odd_even_ratio = odd_even_ratio(&peaks, voiced);

    // Peak-normalise amplitudes to the loudest partial.
    let max_mag = peaks.iter().map(|p| p.magnitude).fold(MAG_FLOOR, f32::max);
    let partials = peaks
        .iter()
        .map(|p| Partial {
            frequency: Hertz::new(p.frequency),
            amplitude: Decibels::from_linear((p.magnitude / max_mag).max(MAG_FLOOR)),
            harmonic_number: p.harmonic_number,
            inharmonicity: Cents::new(p.inharmonicity_cents),
        })
        .collect();

    let log_bins = if opts.log_bins > 0 {
        log_spaced_bins(&mags, bin_hz, opts.log_bins as usize)
    } else {
        Vec::new()
    };

    SpectrumResult {
        f0,
        voiced,
        partials,
        centroid: Hertz::new(centroid),
        flatness: NormalizedValue::new_unchecked(flatness),
        rolloff: Hertz::new(rolloff),
        inharmonicity: NormalizedValue::new_unchecked(inharmonicity),
        odd_even_ratio,
        bands,
        log_bins,
        frame_rms,
    }
}

/// One frame of a spectrogram: its window-centre time plus the full spectrum.
#[derive(Debug, Clone)]
pub struct SpectrogramFrame {
    /// Window-centre time of this frame.
    pub time: Seconds,
    /// The frame's spectrum descriptor.
    pub spectrum: SpectrumResult,
}

/// Hard cap on spectrogram frames, so a tiny `hop` can't exhaust memory. When a
/// run hits this, the caller should warn that coverage was truncated.
pub const MAX_SPECTROGRAM_FRAMES: usize = 4096;

/// Slide a window across `signal` and analyse each frame, **reusing one FFT
/// workspace** across all frames (the planner is built once, not per frame —
/// this is what makes a one-render spectrogram cheap). `hop_samples` is the step
/// between frame starts; `window_len_samples` is the analysed frame length
/// (zero-padded to the FFT size). Each frame's `time` is its window centre.
///
/// The per-frame `voiced` flag lets a caller read a source's time evolution
/// directly — e.g. a SID voice alternating pitched/​noise every video frame.
#[must_use]
pub fn analyze_spectrogram(
    signal: &[f32],
    sample_rate: u32,
    hop_samples: usize,
    window_len_samples: usize,
    opts: SpectrumOpts,
) -> Vec<SpectrogramFrame> {
    if sample_rate == 0 || signal.is_empty() || hop_samples == 0 || window_len_samples == 0 {
        return Vec::new();
    }
    // The FFT must be at least the window length so the frame isn't truncated.
    let fft_size = opts
        .fft_size
        .max(window_len_samples)
        .max(64)
        .next_power_of_two();
    let mut workspace = MagnitudeWorkspace::with_window(fft_size, opts.window.to_dsp());
    let sr = sample_rate as f32;

    let mut frames = Vec::new();
    let mut start = 0usize;
    while start < signal.len() && frames.len() < MAX_SPECTROGRAM_FRAMES {
        let end = (start + window_len_samples).min(signal.len());
        let frame = &signal[start..end];
        let spectrum = analyze_with_workspace(frame, sample_rate, opts, &mut workspace);
        let centre = start as f32 + frame.len() as f32 / 2.0;
        frames.push(SpectrogramFrame {
            time: Seconds::new(centre / sr),
            spectrum,
        });
        // Stop once the window has reached the end, so an overlapping hop does
        // not emit a run of ever-shorter tail frames.
        if end == signal.len() {
            break;
        }
        start += hop_samples;
    }
    frames
}

/// Geometric-mean / arithmetic-mean spectral flatness over bins `1..len-1`
/// (excluding DC and Nyquist). 0 = pure tone, → 1 = white noise.
fn spectral_flatness(mags: &[f32]) -> f32 {
    let band = usable_band(mags);
    if band.is_empty() {
        return 0.0;
    }
    let mut log_sum = 0.0f64;
    let mut arith_sum = 0.0f64;
    for &m in band {
        let p = f64::from((m * m).max(MAG_FLOOR));
        log_sum += p.ln();
        arith_sum += p;
    }
    let n = band.len() as f64;
    let geo = (log_sum / n).exp();
    let arith = arith_sum / n;
    if arith <= 0.0 {
        0.0
    } else {
        (geo / arith).clamp(0.0, 1.0) as f32
    }
}

/// Index range of usable bins: skip DC (0) and Nyquist (last), matching
/// [`spectral_flatness`] and [`pick_peaks`].
fn usable_band(mags: &[f32]) -> &[f32] {
    if mags.len() < 3 {
        &[]
    } else {
        &mags[1..mags.len() - 1]
    }
}

/// Magnitude-weighted spectral centroid in Hz (DC and Nyquist excluded).
fn spectral_centroid(mags: &[f32], bin_hz: f32) -> f32 {
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for (i, &m) in usable_band(mags).iter().enumerate() {
        let k = i + 1; // band starts at bin 1
        let mag = f64::from(m);
        num += f64::from(k as f32 * bin_hz) * mag;
        den += mag;
    }
    if den <= 0.0 { 0.0 } else { (num / den) as f32 }
}

/// Frequency below which `fraction` of the total energy lies (DC and Nyquist
/// excluded).
fn spectral_rolloff(mags: &[f32], bin_hz: f32, fraction: f32) -> f32 {
    let band = usable_band(mags);
    let total: f64 = band.iter().map(|&m| f64::from(m * m)).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let target = total * f64::from(fraction);
    let mut acc = 0.0f64;
    for (i, &m) in band.iter().enumerate() {
        acc += f64::from(m * m);
        if acc >= target {
            return (i + 1) as f32 * bin_hz;
        }
    }
    band.len() as f32 * bin_hz
}

/// Local maxima above an adaptive noise floor, parabolic-refined on the
/// log-magnitude spectrum. Excludes DC and Nyquist.
fn pick_peaks(mags: &[f32], bin_hz: f32) -> Vec<Peak> {
    // Adaptive floor: mean magnitude of the usable band. A peak must clear it.
    let band = usable_band(mags);
    let mean: f32 = band.iter().sum::<f32>() / band.len().max(1) as f32;
    let floor = mean.max(MAG_FLOOR);

    let mut peaks = Vec::new();
    for k in 1..mags.len() - 1 {
        let m = mags[k];
        if m > mags[k - 1] && m > mags[k + 1] && m > floor {
            let refined_bin = parabolic_refine_log(mags, k);
            let frequency = refined_bin * bin_hz;
            // A parabola at the lowest bin can pull the refined index to ~0;
            // a 0 Hz "partial" is meaningless and would divide by zero in the
            // cents math of `diff_partials`, so drop it.
            if frequency > 0.0 {
                peaks.push(Peak {
                    frequency,
                    magnitude: m,
                    harmonic_number: None,
                    inharmonicity_cents: 0.0,
                });
            }
        }
    }
    peaks
}

/// Parabolic interpolation on the **log**-magnitude spectrum around bin `k`,
/// returning a fractional bin index. Guards a degenerate (flat) parabola.
fn parabolic_refine_log(mags: &[f32], k: usize) -> f32 {
    if k == 0 || k + 1 >= mags.len() {
        return k as f32;
    }
    let a = mags[k - 1].max(MAG_FLOOR).ln();
    let b = mags[k].max(MAG_FLOOR).ln();
    let c = mags[k + 1].max(MAG_FLOOR).ln();
    let denom = a - 2.0 * b + c;
    let offset = if denom.abs() > 1.0e-9 {
        (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    k as f32 + offset
}

/// Tag a peak with its nearest harmonic of `f0` and the signed cents deviation.
fn tag_harmonic(peak: &mut Peak, f0: f32) {
    if f0 <= 0.0 || peak.frequency <= 0.0 {
        return;
    }
    let n = (peak.frequency / f0).round().max(1.0);
    let expected = n * f0;
    peak.harmonic_number = Some(n as u32);
    peak.inharmonicity_cents = 1200.0 * (peak.frequency / expected).log2();
}

/// Energy-weighted aggregate inharmonicity, normalised: `Σ E·|f − n·f0| /
/// (f0·ΣE)`. 0 when unvoiced.
fn aggregate_inharmonicity(peaks: &[Peak], f0: Option<Hertz>) -> f32 {
    let Some(f0) = f0 else { return 0.0 };
    if f0.0 <= 0.0 {
        return 0.0;
    }
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for p in peaks {
        let Some(n) = p.harmonic_number else { continue };
        let e = f64::from(p.magnitude * p.magnitude);
        let dev = f64::from((p.frequency - n as f32 * f0.0).abs());
        num += e * dev;
        den += e;
    }
    if den <= 0.0 {
        0.0
    } else {
        ((num / den) / f64::from(f0.0)).clamp(0.0, 1.0) as f32
    }
}

/// Σ odd-harmonic / Σ even-harmonic power from tagged partials. 0 when unvoiced
/// or when there is no even-harmonic energy.
fn odd_even_ratio(peaks: &[Peak], voiced: bool) -> f32 {
    if !voiced {
        return 0.0;
    }
    let mut odd = 0.0f64;
    let mut even = 0.0f64;
    for p in peaks {
        let Some(n) = p.harmonic_number else { continue };
        let power = f64::from(p.magnitude * p.magnitude);
        if n == 1 {
            continue; // fundamental counts as neither odd nor even harmonic
        }
        if n % 2 == 0 {
            even += power;
        } else {
            odd += power;
        }
    }
    // Floor the denominator at a small fraction of the total harmonic energy so
    // an odd-only tone (square wave: even ≈ 0) yields a large ratio rather than
    // collapsing to 0 — which would be indistinguishable from an even-dominant
    // tone. Naturally bounds the result to ≈ [0, 1000].
    let total = odd + even;
    if total <= 0.0 {
        return 0.0;
    }
    (odd / even.max(total * 1.0e-3)) as f32
}

/// `n` log-spaced magnitude bins (dB, peak-normalised) over a fixed reference
/// band (≈ 20 Hz … [`LOG_BINS_TOP_HZ`], clamped to Nyquist). The band is
/// deliberately *not* each spectrum's own Nyquist: a fixed grid keeps bin `i`
/// covering the same frequency range regardless of sample rate, so
/// [`compare`]'s `log_spectral_distance` aligns bins across two spectra rendered
/// or sampled at different rates.
fn log_spaced_bins(mags: &[f32], bin_hz: f32, n: usize) -> Vec<Decibels> {
    if n == 0 || mags.len() < 2 {
        return Vec::new();
    }
    // Fixed absolute band edges — deliberately NOT this source's Nyquist — so
    // bin `i` covers the same Hz range at every sample rate. A bin that falls
    // above this source's Nyquist simply finds no FFT bins and reads as the
    // floor (rather than panicking or shifting the grid).
    let f_lo = LOG_BINS_BOTTOM_HZ;
    let f_hi = LOG_BINS_TOP_HZ;
    if f_hi <= f_lo {
        return Vec::new();
    }
    let log_lo = f_lo.ln();
    let log_hi = f_hi.ln();
    let global_max = mags.iter().copied().fold(MAG_FLOOR, f32::max);

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = (log_lo + (log_hi - log_lo) * i as f32 / n as f32).exp();
        let hi = (log_lo + (log_hi - log_lo) * (i + 1) as f32 / n as f32).exp();
        let k_lo = (lo / bin_hz).floor().max(1.0) as usize;
        let k_hi = ((hi / bin_hz).ceil() as usize).max(k_lo + 1);
        let hi_idx = k_hi.min(mags.len());
        let mut peak = MAG_FLOOR;
        // `k_lo >= hi_idx` ⇒ the whole bin sits above Nyquist ⇒ stays at floor.
        if k_lo < hi_idx {
            for &m in &mags[k_lo..hi_idx] {
                peak = peak.max(m);
            }
        }
        out.push(Decibels::from_linear((peak / global_max).max(MAG_FLOOR)));
    }
    out
}

/// Detect f0 (Hz) via the normalized square-difference function (NSDF/MPM) on
/// the time-domain frame. Returns `(Some(f0), clarity)` or `(None, clarity)`
/// when no lag in range clears the search; `clarity` is the peak NSDF in
/// `[-1, 1]` (used for the voiced verdict).
fn detect_f0(signal: &[f32], sample_rate: u32, hint: Option<Hertz>) -> (Option<Hertz>, f32) {
    let n = signal.len();
    if n < 4 {
        return (None, 0.0);
    }
    let sr = sample_rate as f32;

    // Lag search range. With a hint, restrict to a fifth (×1.5) either side of
    // the hinted period; otherwise the full musical range.
    let (lag_min, lag_max) = match hint {
        Some(h) if h.0 > 0.0 => {
            let period = sr / h.0;
            (
                (period / 1.5).floor() as usize,
                (period * 1.5).ceil() as usize,
            )
        }
        _ => (
            (sr / F0_SEARCH_MAX_HZ).floor() as usize,
            (sr / F0_SEARCH_MIN_HZ).ceil() as usize,
        ),
    };
    let lag_min = lag_min.max(2);
    let lag_max = lag_max.min(n / 2);
    if lag_min >= lag_max {
        return (None, 0.0);
    }

    // Precompute the NSDF over the whole search range, then pick the FIRST key
    // maximum near the global best (McLeod MPM). Selecting the global-max lag
    // outright causes octave/sub-octave errors: a periodic tone has near-equal
    // NSDF peaks at every multiple of its period, so the largest is arbitrary.
    // Compute one lag past `lag_max` so the local-maximum test (which needs the
    // right neighbour) can still consider `lag_max` itself.
    let nsdf_top = (lag_max + 1).min(n - 1);
    let mut nsdf = vec![0.0f32; nsdf_top + 1];
    for lag in 1..=nsdf_top {
        let mut ac = 0.0f64; // Σ x[i]·x[i+lag]
        let mut sq = 0.0f64; // Σ x[i]² + x[i+lag]²
        for i in 0..(n - lag) {
            let a = f64::from(signal[i]);
            let b = f64::from(signal[i + lag]);
            ac += a * b;
            sq += a * a + b * b;
        }
        nsdf[lag] = if sq > 0.0 {
            (2.0 * ac / sq) as f32
        } else {
            0.0
        };
    }

    // Local maxima (key maxima) within the search range, inclusive of lag_max.
    let mut key_maxima: Vec<usize> = Vec::new();
    let search_top = lag_max.min(nsdf.len() - 2);
    for lag in lag_min..=search_top {
        if nsdf[lag] > nsdf[lag - 1] && nsdf[lag] >= nsdf[lag + 1] {
            key_maxima.push(lag);
        }
    }
    if key_maxima.is_empty() {
        return (None, 0.0);
    }
    let n_max = key_maxima.iter().map(|&l| nsdf[l]).fold(f32::MIN, f32::max);
    if n_max < VOICED_NSDF_THRESHOLD {
        return (None, n_max.max(0.0));
    }
    // First key maximum within 90 % of the global best is the true period.
    let threshold = 0.9 * n_max;
    let chosen = key_maxima
        .iter()
        .copied()
        .find(|&l| nsdf[l] >= threshold)
        .unwrap_or(key_maxima[0]);

    let refined = parabolic_peak_lag(&nsdf, chosen);
    let f0 = sr / refined;
    (Some(Hertz::new(f0)), nsdf[chosen])
}

/// Parabolic refinement of an NSDF peak lag from the three values around it.
fn parabolic_peak_lag(nsdf: &[f32], lag: usize) -> f32 {
    if lag < 1 || lag + 1 >= nsdf.len() {
        return lag as f32;
    }
    let a = nsdf[lag - 1];
    let b = nsdf[lag];
    let c = nsdf[lag + 1];
    let denom = a - 2.0 * b + c;
    if denom.abs() > 1.0e-9 {
        lag as f32 + (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
    } else {
        lag as f32
    }
}

// === Comparison ===

/// One unmatched partial, named source-neutrally because the same type carries
/// both `missing_partials` (the partial belongs to the target) and
/// `extra_partials` (it belongs to the candidate).
#[derive(Debug, Clone, Copy)]
pub struct PartialDiff {
    /// The partial's frequency.
    pub frequency: Hertz,
    /// The partial's peak-normalised amplitude (dB).
    pub amplitude: Decibels,
}

/// Distance between two spectra. `log_spectral_distance` is the primary scalar
/// to minimise; `missing_partials` is the actionable list ("the target has a
/// strong partial at 1153 Hz your patch lacks").
#[derive(Debug, Clone)]
pub struct SpectrumDistance {
    /// RMS dB difference over the shared log-spaced bins (dimensionless).
    pub log_spectral_distance: f32,
    /// Candidate − target centroid.
    pub centroid_delta: Hertz,
    /// Candidate − target flatness (signed; both operands are in [0, 1] so the
    /// delta is in [-1, 1], hence a plain `f32` rather than `NormalizedValue`).
    pub flatness_delta: f32,
    /// Candidate − target aggregate inharmonicity (signed; see `flatness_delta`).
    pub inharmonicity_delta: f32,
    /// Strong in target, absent in candidate.
    pub missing_partials: Vec<PartialDiff>,
    /// Present in candidate, not in target.
    pub extra_partials: Vec<PartialDiff>,
    /// `true` when exactly one frame is voiced — a pitched-vs-noise mismatch;
    /// partial matching is skipped and the distance is penalised.
    pub voicing_mismatch: bool,
    /// Fraction of compared log bins that carry timbral information — above the
    /// peak-relative shelf on at least one side (0..1). The distance is computed
    /// over exactly these bins; a sparse or collapsed spectrum reports a low
    /// value.
    pub floor_coverage: f32,
    /// `true` when the distance scalar should not be trusted: both sources
    /// near-floor (distance forced to 0 — the *character* agrees), or the
    /// informative-bin fraction is below [`FLOOR_COVERAGE_MIN`] so the scalar
    /// rests on a handful of bins. Read `missing/extra_partials` +
    /// `centroid_delta` instead.
    pub floor_limited: bool,
}

/// Tolerance for matching a candidate partial to a target partial.
const PARTIAL_MATCH_CENTS: f32 = 50.0;
/// Penalty added to `log_spectral_distance` for a voiced-vs-unvoiced mismatch.
const VOICING_MISMATCH_PENALTY_DB: f32 = 60.0;

/// Compare two spectra. Handles the three voicing cases (both voiced → full
/// partial diff; one voiced → severe penalty, no partial match; both unvoiced →
/// distance from the log bins only), and guards the empty-bin / silence floor
/// (TODO §4.1): near-silent-vs-near-silent reads as distance 0 with
/// `floor_limited` set, and floor-vs-floor bins never inflate the scalar.
#[must_use]
pub fn compare(target: &SpectrumResult, candidate: &SpectrumResult) -> SpectrumDistance {
    let centroid_delta = Hertz::new(candidate.centroid.0 - target.centroid.0);
    let flatness_delta = candidate.flatness.0 - target.flatness.0;
    let inharmonicity_delta = candidate.inharmonicity.0 - target.inharmonicity.0;

    // Silence guard: peak-normalised log bins amplify a near-silent frame's
    // noise floor to full scale, so two effectively-silent sources would read
    // as 100+ dB of noise-vs-noise. Absolute level says they *agree*. The
    // `NEAR_FLOOR_RMS` net additionally catches a voice a chip has collapsed to
    // DAC/resampler residue (≈ −50 dBFS), which sits above the −80 dBFS
    // `SILENCE_RMS` floor but is still perceptually silent (calibration plan,
    // TODO §4.1). Requiring BOTH sides near-floor keeps a soft-but-real note
    // vs a loud one out of this early-out.
    let both_silent = target.frame_rms <= SILENCE_RMS && candidate.frame_rms <= SILENCE_RMS;
    let both_near_floor =
        target.frame_rms <= NEAR_FLOOR_RMS && candidate.frame_rms <= NEAR_FLOOR_RMS;
    if both_silent || both_near_floor {
        return SpectrumDistance {
            log_spectral_distance: 0.0,
            centroid_delta,
            flatness_delta,
            inharmonicity_delta,
            missing_partials: Vec::new(),
            extra_partials: Vec::new(),
            voicing_mismatch: false,
            floor_coverage: 0.0,
            floor_limited: true,
        };
    }

    let (lsd, floor_coverage) = log_spectral_distance(&target.log_bins, &candidate.log_bins);

    let voicing_mismatch = target.voiced != candidate.voiced;

    // Only diff partials when both frames are voiced; otherwise partial-matching
    // across pitched/noise content is meaningless.
    let (missing, extra) = if target.voiced && candidate.voiced {
        diff_partials(&target.partials, &candidate.partials)
    } else {
        (Vec::new(), Vec::new())
    };

    let log_spectral_distance = if voicing_mismatch {
        lsd + VOICING_MISMATCH_PENALTY_DB
    } else {
        lsd
    };

    SpectrumDistance {
        log_spectral_distance,
        centroid_delta,
        flatness_delta,
        inharmonicity_delta,
        missing_partials: missing,
        extra_partials: extra,
        voicing_mismatch,
        floor_coverage,
        floor_limited: floor_coverage < FLOOR_COVERAGE_MIN,
    }
}

/// RMS dB difference over the shared (min-length) prefix of two peak-normalised
/// log-bin sets. Each bin is clamped up to [`LSD_FLOOR_DB`] before diffing (a
/// −70 dB reference floor vs a −120 dB digital floor is a 10 dB difference, not
/// 50), and a bin that is below [`LSD_SHELF_DB`] on BOTH sides is a shared null
/// (inter-partial gap / resampler leakage) — excluded from both the RMS and the
/// coverage count, since it carries no timbral information. Returns `(distance,
/// live-bin coverage 0..1)`; `(0, 0)` when either side is empty or no bin is
/// live.
fn log_spectral_distance(a: &[Decibels], b: &[Decibels]) -> (f32, f32) {
    let n = a.len().min(b.len());
    if n == 0 {
        return (0.0, 0.0);
    }
    let mut sum = 0.0f64;
    let mut live = 0usize;
    for i in 0..n {
        let av = a[i].0.max(LSD_FLOOR_DB);
        let bv = b[i].0.max(LSD_FLOOR_DB);
        if av <= LSD_SHELF_DB && bv <= LSD_SHELF_DB {
            continue; // shared null: ≥ |LSD_SHELF_DB| below both peaks
        }
        live += 1;
        let d = f64::from(av - bv);
        sum += d * d;
    }
    if live == 0 {
        return (0.0, 0.0);
    }
    #[allow(clippy::cast_precision_loss)]
    ((sum / live as f64).sqrt() as f32, live as f32 / n as f32)
}

/// Match each target partial to the nearest candidate partial within
/// [`PARTIAL_MATCH_CENTS`]; unmatched target partials are "missing", unmatched
/// candidate partials are "extra".
fn diff_partials(
    target: &[Partial],
    candidate: &[Partial],
) -> (Vec<PartialDiff>, Vec<PartialDiff>) {
    let mut candidate_used = vec![false; candidate.len()];
    let mut missing = Vec::new();

    for t in target {
        let mut best: Option<(usize, f32)> = None;
        for (j, c) in candidate.iter().enumerate() {
            if candidate_used[j] {
                continue;
            }
            let cents = 1200.0 * (c.frequency.0 / t.frequency.0).log2();
            if cents.abs() <= PARTIAL_MATCH_CENTS && best.is_none_or(|(_, b)| cents.abs() < b.abs())
            {
                best = Some((j, cents));
            }
        }
        match best {
            Some((j, _cents)) => {
                // Matched within tolerance — not a difference worth surfacing.
                candidate_used[j] = true;
            }
            None => missing.push(PartialDiff {
                frequency: t.frequency,
                amplitude: t.amplitude,
            }),
        }
    }

    let extra = candidate
        .iter()
        .zip(candidate_used)
        .filter(|(_, used)| !used)
        .map(|(c, _)| PartialDiff {
            frequency: c.frequency,
            amplitude: c.amplitude,
        })
        .collect();

    (missing, extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: u32 = 44_100;

    fn sine(freq: f32, len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (TAU * freq * i as f32 / SR as f32).sin())
            .collect()
    }

    fn saw(freq: f32, len: usize, harmonics: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (1..=harmonics)
                    .map(|h| (TAU * freq * h as f32 * t).sin() / h as f32)
                    .sum()
            })
            .collect()
    }

    #[test]
    fn pure_sine_has_one_dominant_partial_zero_inharmonicity() {
        let sig = sine(1000.0, 16_384, 0.8);
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        assert!(r.voiced, "1 kHz sine should be voiced");
        let f0 = r.f0.expect("voiced → f0").0;
        assert!((f0 - 1000.0).abs() < 5.0, "f0 ≈ 1 kHz, got {f0}");
        let top = r.partials.first().expect("a partial");
        assert!(
            (top.frequency.0 - 1000.0).abs() < 5.0,
            "loudest partial ≈ 1 kHz, got {}",
            top.frequency.0
        );
        assert!(r.flatness.0 < 0.1, "pure tone flat, got {}", r.flatness.0);
        assert!(
            r.inharmonicity.0 < 0.01,
            "harmonic, got {}",
            r.inharmonicity.0
        );
    }

    #[test]
    fn white_noise_has_high_flatness_and_is_unvoiced() {
        // Deterministic pseudo-noise.
        let mut x = 0x1234_5678u32;
        let sig: Vec<f32> = (0..16_384)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        assert!(r.flatness.0 > 0.3, "noise flat-ish, got {}", r.flatness.0);
        assert!(!r.voiced, "white noise should be unvoiced");
        assert!(r.f0.is_none(), "unvoiced → no f0");
        assert!(
            r.inharmonicity.0 == 0.0,
            "no harmonic tagging when unvoiced"
        );
    }

    #[test]
    fn saw_is_voiced_with_harmonic_series() {
        let sig = saw(200.0, 16_384, 12);
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        assert!(r.voiced, "saw is pitched");
        let f0 = r.f0.expect("f0").0;
        assert!((f0 - 200.0).abs() < 4.0, "f0 ≈ 200, got {f0}");
        // The first few harmonics should be tagged near-integer multiples.
        let tagged = r
            .partials
            .iter()
            .filter(|p| p.harmonic_number.is_some())
            .count();
        assert!(
            tagged >= 4,
            "expected several tagged harmonics, got {tagged}"
        );
        for p in r.partials.iter().filter(|p| p.harmonic_number.is_some()) {
            assert!(
                p.inharmonicity.0.abs() < 30.0,
                "harmonic {} within 30 cents, got {} at {} Hz",
                p.harmonic_number.unwrap(),
                p.inharmonicity.0,
                p.frequency.0
            );
        }
    }

    fn square(freq: f32, len: usize, harmonics: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (1..=harmonics)
                    .filter(|h| h % 2 == 1)
                    .map(|h| (TAU * freq * h as f32 * t).sin() / h as f32)
                    .sum()
            })
            .collect()
    }

    #[test]
    fn square_has_odd_harmonics_dominant() {
        let sig = square(220.0, 16_384, 15);
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        assert!(r.voiced);
        // A square wave carries only odd harmonics, so odd ≫ even energy.
        assert!(
            r.odd_even_ratio > 5.0,
            "square should be odd-dominant, ratio = {}",
            r.odd_even_ratio
        );
        // The strong tagged partials should be odd multiples of f0.
        let strong_even = r
            .partials
            .iter()
            .filter(|p| p.amplitude.0 > -30.0)
            .filter_map(|p| p.harmonic_number)
            .filter(|n| *n > 1 && n % 2 == 0)
            .count();
        assert_eq!(strong_even, 0, "no strong even harmonics in a square wave");
    }

    #[test]
    fn inharmonic_pair_is_flagged() {
        let mut sig = sine(1000.0, 16_384, 0.6);
        for (i, s) in sig.iter_mut().enumerate() {
            *s += 0.6 * (TAU * 1410.0 * i as f32 / SR as f32).sin();
        }
        let opts = SpectrumOpts {
            f0_hint: Some(Hertz::new(1000.0)),
            ..Default::default()
        };
        let r = analyze_spectrum(&sig, SR, opts);
        // The 1410 Hz tone sits ~half-way between the 1st and 2nd harmonic of
        // 1 kHz, so its nearest-harmonic deviation must be large.
        let worst = r
            .partials
            .iter()
            .filter_map(|p| p.harmonic_number.map(|_| p.inharmonicity.0.abs()))
            .fold(0.0, f32::max);
        assert!(worst > 100.0, "inharmonic partial flagged, worst = {worst}");
    }

    #[test]
    fn low_f0_short_window_locates_partial_via_zero_pad() {
        // A low 150 Hz sine in an 882-sample (20 ms) frame. The raw FFT bin
        // spacing of that frame is Fs/882 ≈ 50 Hz, so without zero-padding the
        // peak would quantise to the nearest 50 Hz grid point (within ±25 Hz).
        // Zero-padding to 8192 + parabolic interpolation must locate it within a
        // few Hz. (Note: zero-padding *locates* an isolated peak accurately; it
        // cannot *resolve* partials spaced closer than the window's main lobe —
        // that is a hard time-frequency limit, not something this test claims.)
        let sig = sine(150.0, 882, 0.8);
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        let top = r.partials.first().expect("a partial");
        assert!(
            (top.frequency.0 - 150.0).abs() < 5.0,
            "zero-pad + parabolic should locate the 150 Hz peak, got {} Hz",
            top.frequency.0
        );
    }

    #[test]
    fn silence_and_dc_do_not_nan() {
        for sig in [vec![0.0f32; 8192], vec![0.5f32; 8192]] {
            let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
            assert!(r.centroid.0.is_finite());
            assert!(r.flatness.0.is_finite());
            assert!(r.rolloff.0.is_finite());
            assert!(r.inharmonicity.0.is_finite());
            assert!(!r.voiced, "silence/DC is not voiced");
        }
    }

    #[test]
    fn peaks_stay_below_nyquist() {
        let sig = sine(21_000.0, 16_384, 0.8); // near Fs/2
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        for p in &r.partials {
            assert!(
                p.frequency.0 < SR as f32 / 2.0,
                "partial {} Hz at/above Nyquist",
                p.frequency.0
            );
        }
    }

    #[test]
    fn compare_reports_missing_partial() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        // Target: 500 Hz saw (rich). Candidate: 500 Hz sine (only fundamental).
        let target = analyze_spectrum(&saw(500.0, 16_384, 8), SR, opts);
        let candidate = analyze_spectrum(&sine(500.0, 16_384, 0.8), SR, opts);
        let d = compare(&target, &candidate);
        assert!(
            !d.missing_partials.is_empty(),
            "saw's upper harmonics should be missing from the sine candidate"
        );
        assert!(d.log_spectral_distance > 0.0);
    }

    #[test]
    fn log_bins_align_across_sample_rates() {
        // The same 2 kHz tone captured at 32 kHz and 44.1 kHz must land in the
        // SAME log bin — the fixed absolute grid is what makes a cross-rate
        // log_spectral_distance meaningful (a 32 kHz SID dump vs a 44.1 kHz
        // render). A Nyquist-relative grid would shift the peak between them.
        fn tone(freq: f32, rate: u32, len: usize) -> Vec<f32> {
            (0..len)
                .map(|i| 0.8 * (TAU * freq * i as f32 / rate as f32).sin())
                .collect()
        }
        let opts = SpectrumOpts {
            log_bins: 96,
            ..Default::default()
        };
        let lo = analyze_spectrum(&tone(2_000.0, 32_000, 16_384), 32_000, opts);
        let hi = analyze_spectrum(&tone(2_000.0, 44_100, 16_384), 44_100, opts);

        let argmax = |bins: &[Decibels]| -> usize {
            bins.iter()
                .enumerate()
                .max_by(|a, b| a.1.0.total_cmp(&b.1.0))
                .map(|(i, _)| i)
                .unwrap_or(0)
        };
        let bin_lo = argmax(&lo.log_bins);
        let bin_hi = argmax(&hi.log_bins);
        assert_eq!(lo.log_bins.len(), 96);
        assert_eq!(hi.log_bins.len(), 96);
        assert!(
            bin_lo.abs_diff(bin_hi) <= 1,
            "2 kHz tone should land in the same log bin at both rates: 32k→{bin_lo}, 44.1k→{bin_hi}"
        );
    }

    #[test]
    fn compare_voicing_mismatch_is_penalised() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        let voiced = analyze_spectrum(&saw(300.0, 16_384, 8), SR, opts);
        let mut x = 0xDEAD_BEEFu32;
        let noise: Vec<f32> = (0..16_384)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        let unvoiced = analyze_spectrum(&noise, SR, opts);
        let d = compare(&voiced, &unvoiced);
        assert!(d.voicing_mismatch, "voiced vs noise is a mismatch");
        assert!(
            d.log_spectral_distance >= VOICING_MISMATCH_PENALTY_DB,
            "mismatch should carry the penalty, got {}",
            d.log_spectral_distance
        );
        assert!(
            d.missing_partials.is_empty(),
            "no partial matching across a voicing mismatch"
        );
    }

    /// TODO §4.1 case 2: two near-silent sources must compare as distance 0
    /// with `floor_limited` set — peak-normalising their noise floors would
    /// otherwise explode into meaningless noise-vs-noise dB.
    #[test]
    fn compare_near_silence_is_floor_limited_zero() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        // Two different, tiny noise floors (~ −90 dBFS).
        let tiny = |seed: u32| -> Vec<f32> {
            let mut x = seed;
            (0..16_384)
                .map(|_| {
                    x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 3.0e-5
                })
                .collect()
        };
        let a = analyze_spectrum(&tiny(0xDEAD_BEEF), SR, opts);
        let b = analyze_spectrum(&tiny(0x1234_5678), SR, opts);
        let d = compare(&a, &b);
        assert!(d.floor_limited, "near-silence must be flagged");
        assert!(
            d.log_spectral_distance.abs() < f32::EPSILON,
            "silence-vs-silence must read 0, got {}",
            d.log_spectral_distance
        );
        assert!(!d.voicing_mismatch, "silence agrees with silence");
    }

    /// TODO §4.1 case 1: bins below both sources' peaks by ≥ the shelf are
    /// excluded, so a broadband tone against the same tone with a slightly
    /// different noise floor stays near the true (small) distance instead of
    /// being dominated by the noise-floor mismatch, and stays *unflagged*
    /// because its harmonics fill the band.
    #[test]
    fn compare_ignores_shared_floor_bins() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        // A harmonic-rich saw (fills the band up to ~14 kHz), like a real SID
        // saw — the healthy "broadband tone stays unflagged" reference.
        let clean = saw(440.0, 16_384, 32);
        // Same tone + a tiny broadband floor (~ −85 dB below the saw).
        let mut x = 0xCAFE_F00Du32;
        let noisy: Vec<f32> = clean
            .iter()
            .map(|&v| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                v + ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 5.0e-5
            })
            .collect();
        let a = analyze_spectrum(&clean, SR, opts);
        let b = analyze_spectrum(&noisy, SR, opts);
        let d = compare(&a, &b);
        assert!(
            d.log_spectral_distance < 6.0,
            "identical harmonics must stay near the floor-guarded distance, got {}",
            d.log_spectral_distance
        );
        assert!(
            d.floor_coverage > FLOOR_COVERAGE_MIN,
            "a broadband tone's harmonics fill the band: coverage {}",
            d.floor_coverage
        );
        assert!(
            !d.floor_limited,
            "a real broadband tone is not floor-limited"
        );
    }

    /// A minimal voiced, non-silent [`SpectrumResult`] with hand-set log bins
    /// (dB, peak-normalised) — exercises the floor-guard/shelf logic in
    /// [`compare`] deterministically, without FFT-level calibration.
    fn spectrum_from_bins(log_bins_db: &[f32], frame_rms: f32, voiced: bool) -> SpectrumResult {
        SpectrumResult {
            f0: voiced.then(|| Hertz::new(1000.0)),
            voiced,
            partials: Vec::new(),
            centroid: Hertz::new(1000.0),
            flatness: NormalizedValue::new_unchecked(0.2),
            rolloff: Hertz::new(2000.0),
            inharmonicity: NormalizedValue::new_unchecked(0.0),
            odd_even_ratio: 1.0,
            bands: EnergyBands {
                sub: 0.0,
                low: 0.0,
                mid: 0.0,
                high: 0.0,
            },
            log_bins: log_bins_db.iter().map(|&d| Decibels::new(d)).collect(),
            frame_rms,
        }
    }

    /// Calibration plan case 1 (the ring-mod row): two spectra whose partials
    /// match but whose inter-partial nulls differ by 15 dB — *below* the
    /// peak-relative shelf on both sides — collapse toward the matched-partial
    /// distance and self-report `floor_limited`. Above the shelf the same 15 dB
    /// difference is audible content and drives a real, unflagged distance.
    #[test]
    fn compare_sparse_matching_partials_below_shelf_is_low_distance() {
        let n = 64;
        let partial_bins = [10usize, 22, 38];
        // Sub-shelf nulls (−60 vs −75): a shared, information-free floor.
        let mut a = vec![-60.0f32; n];
        let mut b = vec![-75.0f32; n];
        for &k in &partial_bins {
            a[k] = 0.0;
            b[k] = 0.0;
        }
        let d = compare(
            &spectrum_from_bins(&a, 0.08, true),
            &spectrum_from_bins(&b, 0.08, true),
        );
        assert!(
            d.log_spectral_distance < 6.0,
            "sub-shelf nulls must not inflate the distance, got {}",
            d.log_spectral_distance
        );
        assert!(
            d.floor_limited && d.floor_coverage < FLOOR_COVERAGE_MIN,
            "a 3-partial spectrum is floor-limited: coverage {}",
            d.floor_coverage
        );

        // Contrast: lift both floors above the shelf (−30 vs −45). Now the
        // 15 dB gap is real content — kept, high distance, not flagged.
        let a2: Vec<f32> = a
            .iter()
            .map(|&v| if v < -50.0 { -30.0 } else { v })
            .collect();
        let b2: Vec<f32> = b
            .iter()
            .map(|&v| if v < -50.0 { -45.0 } else { v })
            .collect();
        let d2 = compare(
            &spectrum_from_bins(&a2, 0.08, true),
            &spectrum_from_bins(&b2, 0.08, true),
        );
        assert!(
            d2.log_spectral_distance > 10.0,
            "above-shelf level differences must count, got {}",
            d2.log_spectral_distance
        );
        assert!(!d2.floor_limited, "a filled spectrum is not floor-limited");
    }

    /// Calibration plan case 2 (the 0x61 collapsed-voice row): two sources both
    /// below `NEAR_FLOOR_RMS` (but above the absolute `SILENCE_RMS`) agree —
    /// distance 0, `floor_limited`, and NO voicing-mismatch penalty even when
    /// one stale-reads voiced.
    #[test]
    fn compare_both_near_floor_is_floor_limited_zero() {
        // frame_rms 2e-3 ∈ (SILENCE_RMS 1e-4, NEAR_FLOOR_RMS 3e-3).
        let target = spectrum_from_bins(&vec![-40.0; 64], 2.0e-3, true);
        let candidate = spectrum_from_bins(&vec![-60.0; 64], 2.0e-3, false);
        let d = compare(&target, &candidate);
        assert_eq!(d.log_spectral_distance, 0.0, "both near-floor → distance 0");
        assert!(d.floor_limited, "near-floor pair must be flagged");
        assert!(
            !d.voicing_mismatch,
            "near-floor must not arm the voicing penalty"
        );
    }

    /// Calibration plan case 3 (voicing gated on level): a clean sine is voiced
    /// at full scale, but the same sine near the floor is NOT — otherwise its
    /// NSDF clarity would arm `compare`'s voicing-mismatch penalty on a
    /// perceptually-silent frame.
    #[test]
    fn near_floor_sine_is_not_voiced() {
        let loud = analyze_spectrum(&sine(440.0, 16_384, 0.8), SR, SpectrumOpts::default());
        assert!(loud.voiced, "a full-scale sine is voiced");
        // amp 1.5e-3 → frame_rms ≈ 1.06e-3 < NEAR_FLOOR_RMS.
        let quiet = analyze_spectrum(&sine(440.0, 16_384, 1.5e-3), SR, SpectrumOpts::default());
        assert!(
            !quiet.voiced,
            "a near-floor sine must not read voiced (frame_rms {})",
            quiet.frame_rms
        );
    }

    #[test]
    fn spectrogram_tracks_voiced_to_noise_transition() {
        // First half: 440 Hz sine (voiced). Second half: white noise (unvoiced).
        let half = 22_050; // 0.5 s
        let mut sig = sine(440.0, half, 0.8);
        let mut x = 0x9E37_79B9u32;
        sig.extend((0..half).map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        }));

        let win = (0.040 * SR as f32) as usize; // 40 ms window
        let hop = (0.020 * SR as f32) as usize; // 20 ms hop
        let frames = analyze_spectrogram(&sig, SR, hop, win, SpectrumOpts::default());
        assert!(
            frames.len() > 10,
            "expected many frames, got {}",
            frames.len()
        );

        // Timestamps increase monotonically.
        assert!(frames[1].time.0 > frames[0].time.0, "timestamps ascend");

        // A frame well inside the tone is voiced; one well inside the noise is
        // not — the spectrogram reads the transition directly.
        let early = &frames[2];
        let late = &frames[frames.len() - 3];
        assert!(early.spectrum.voiced, "early (tone) frame should be voiced");
        let f0 = early.spectrum.f0.expect("tone frame has f0").0;
        assert!(
            (f0 - 440.0).abs() < 6.0,
            "early frame f0 ≈ 440 Hz, got {f0}"
        );
        assert!(
            !late.spectrum.voiced,
            "late (noise) frame should be unvoiced"
        );
    }

    #[test]
    fn spectrogram_degenerate_inputs_are_empty() {
        let sig = sine(440.0, 4096, 0.5);
        assert!(analyze_spectrogram(&sig, SR, 0, 512, SpectrumOpts::default()).is_empty());
        assert!(analyze_spectrogram(&sig, SR, 256, 0, SpectrumOpts::default()).is_empty());
        assert!(analyze_spectrogram(&[], SR, 256, 512, SpectrumOpts::default()).is_empty());
    }
}
