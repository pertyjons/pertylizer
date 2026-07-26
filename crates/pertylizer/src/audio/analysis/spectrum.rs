//! Detailed offline spectrum analysis: detected partials, harmonicity, and
//! timbre descriptors that separate sounds the coarse 4-band
//! [`energy_bands`](crate::audio::analysis::energy_bands) metric cannot (a plain triangle, a
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
    /// Number of log-mel filterbank bands to emit (0 = off). Used by [`compare`]
    /// for the perceptual `mel_l2_distance`.
    pub mel_bands: u32,
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
            mel_bands: 0,
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
    /// Optional log-mel filterbank bands (dB, peak-normalised); empty when
    /// `opts.mel_bands == 0`. Used by [`compare`] for `mel_l2_distance`.
    pub mel_bins: Vec<Decibels>,
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
            mel_bins: Vec::new(),
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
            tag_harmonic(peak, f0_hz.as_f32());
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

    let mel_bins = if opts.mel_bands > 0 {
        mel_spaced_bins(&mags, bin_hz, opts.mel_bands as usize)
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
        mel_bins,
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
    if f0.as_f32() <= 0.0 {
        return 0.0;
    }
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for p in peaks {
        let Some(n) = p.harmonic_number else { continue };
        let e = f64::from(p.magnitude * p.magnitude);
        let dev = f64::from((p.frequency - n as f32 * f0.as_f32()).abs());
        num += e * dev;
        den += e;
    }
    if den <= 0.0 {
        0.0
    } else {
        ((num / den) / f64::from(f0.as_f32())).clamp(0.0, 1.0) as f32
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

/// HTK mel scale: `mel = 2595·log10(1 + f/700)`.
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Inverse HTK mel scale.
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
}

/// `n` log-mel filterbank bands (dB, peak-normalised) over the fixed reference
/// band ([`LOG_BINS_BOTTOM_HZ`] … [`LOG_BINS_TOP_HZ`], clamped to Nyquist). A
/// triangular mel filterbank is applied to the linear magnitude spectrum — `n+2`
/// band edges spaced evenly on the mel scale give `n` overlapping triangles, and
/// each band's weighted magnitude sum is peak-normalised to the loudest band and
/// converted to dB. Like [`log_spaced_bins`] the edges are fixed (deliberately
/// NOT this source's Nyquist) so band `i` covers the same mel range at every
/// sample rate, letting [`compare`]'s `mel_l2_distance` align bands across two
/// spectra rendered or sampled at different rates. A band above this source's
/// Nyquist finds no FFT bins and reads as the floor.
fn mel_spaced_bins(mags: &[f32], bin_hz: f32, n: usize) -> Vec<Decibels> {
    if n == 0 || mags.len() < 2 || bin_hz <= 0.0 {
        return Vec::new();
    }
    let f_lo = LOG_BINS_BOTTOM_HZ;
    let f_hi = LOG_BINS_TOP_HZ;
    if f_hi <= f_lo {
        return Vec::new();
    }
    let mel_lo = hz_to_mel(f_lo);
    let mel_hi = hz_to_mel(f_hi);
    // `n+2` mel-spaced edges → `n` triangular filters (left, centre, right).
    let mut edges_hz = Vec::with_capacity(n + 2);
    for i in 0..n + 2 {
        let mel = mel_lo + (mel_hi - mel_lo) * i as f32 / (n + 1) as f32;
        edges_hz.push(mel_to_hz(mel));
    }

    let mut out = Vec::with_capacity(n);
    let mut global_max = MAG_FLOOR;
    let mut acc = vec![0.0f32; n];
    for (f, band) in acc.iter_mut().enumerate() {
        let left = edges_hz[f];
        let centre = edges_hz[f + 1];
        let right = edges_hz[f + 2];
        // FFT bins spanning [left, right); triangular weight peaking at `centre`.
        let k_lo = (left / bin_hz).floor().max(1.0) as usize;
        let k_hi = ((right / bin_hz).ceil() as usize).min(mags.len());
        let mut sum = 0.0f32;
        for k in k_lo..k_hi {
            let f_k = k as f32 * bin_hz;
            let w = if f_k <= centre {
                (f_k - left) / (centre - left)
            } else {
                (right - f_k) / (right - centre)
            };
            if w > 0.0 {
                sum += w * mags[k];
            }
        }
        *band = sum;
        global_max = global_max.max(sum);
    }
    for &band in &acc {
        out.push(Decibels::from_linear((band / global_max).max(MAG_FLOOR)));
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
        Some(h) if h.as_f32() > 0.0 => {
            let period = sr / h.as_f32();
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

/// Distance between two spectra. `log_spectral_distance` is the primary spectral
/// scalar to minimise; `missing_partials` is the actionable list ("the target has
/// a strong partial at 1153 Hz your patch lacks"). A voiced-vs-unvoiced mismatch
/// is reported *separately* in `voicing_penalty_db` (not folded into
/// `log_spectral_distance`), so the pure spectral number still ranks candidates
/// under a mismatch; a caller wanting the old single combined score adds the two.
#[derive(Debug, Clone)]
pub struct SpectrumDistance {
    /// Pure RMS dB difference over the shared log-spaced bins (dimensionless).
    /// Carries **no** voicing-mismatch penalty — see `voicing_penalty_db`.
    pub log_spectral_distance: f32,
    /// The voiced-vs-unvoiced mismatch penalty applied to the *combined* score,
    /// reported on its own so it never saturates `log_spectral_distance`:
    /// `VOICING_MISMATCH_PENALTY_DB` when `voicing_mismatch`, else 0. The old
    /// single-scalar behaviour is `log_spectral_distance + voicing_penalty_db`.
    pub voicing_penalty_db: f32,
    /// True L2 (Euclidean) distance over the shared log-mel bands (dB): the
    /// perceptual counterpart of `log_spectral_distance`. Scales with the band
    /// count (unlike the RMS `log_spectral_distance`); 0 when either side has no
    /// mel bands. Carries no voicing-mismatch penalty — a pitched-vs-noise gross
    /// mismatch already shows up as a large mel envelope difference.
    pub mel_l2_distance: f32,
    /// Candidate − target centroid.
    pub centroid_delta: Hertz,
    /// Candidate − target spectral rolloff (Hz). The rolloff frequency tracks
    /// filter-slope steepness (12 vs 24 dB/oct), so this delta is the direct
    /// signal for calibrating a low-pass model against a reference.
    pub rolloff_delta: Hertz,
    /// Candidate − target flatness (signed; both operands are in [0, 1] so the
    /// delta is in [-1, 1], hence a plain `f32` rather than `NormalizedValue`).
    pub flatness_delta: f32,
    /// Candidate − target aggregate inharmonicity (signed; see `flatness_delta`).
    pub inharmonicity_delta: f32,
    /// Candidate − target odd/even harmonic balance, in dB
    /// (`10·log10(cand) − 10·log10(target)` over the linear power ratios). Odd/
    /// even balance encodes pulse duty cycle (a 50 % square has no even
    /// harmonics → a very high ratio), so this delta drives pulse-width matching.
    /// Positive = candidate is more odd-dominant than the target.
    pub odd_even_ratio_delta_db: f32,
    /// Strong in target, absent in candidate.
    pub missing_partials: Vec<PartialDiff>,
    /// Present in candidate, not in target.
    pub extra_partials: Vec<PartialDiff>,
    /// `true` when exactly one frame is voiced — a pitched-vs-noise mismatch;
    /// partial matching is skipped and `voicing_penalty_db` is charged.
    pub voicing_mismatch: bool,
    /// Fraction of compared log bins that carry timbral information — above the
    /// peak-relative shelf on at least one side (0..1). The distance is computed
    /// over exactly these bins; a sparse or collapsed spectrum reports a low
    /// value.
    pub floor_coverage: f32,
    /// `true` when the distance scalar should not be trusted: both sources
    /// near-floor (distance forced to 0 — the *character* agrees), or the
    /// informative-bin fraction is below `FLOOR_COVERAGE_MIN` so the scalar
    /// rests on a handful of bins. Read `missing/extra_partials` +
    /// `centroid_delta` instead.
    pub floor_limited: bool,
}

/// Tolerance for matching a candidate partial to a target partial.
const PARTIAL_MATCH_CENTS: f32 = 50.0;
/// Voiced-vs-unvoiced mismatch penalty, reported in `voicing_penalty_db` (kept
/// out of `log_spectral_distance` so the spectral scalar never saturates).
const VOICING_MISMATCH_PENALTY_DB: f32 = 60.0;

/// Odd/even harmonic power ratio in dB. The stored ratio is a linear power
/// quotient (Σ odd / Σ even), so dB is `10·log10`. A small floor keeps a fully
/// even-dominant frame (ratio 0) and the log finite; the result saturates near
/// −120 dB there rather than diverging.
fn odd_even_ratio_db(ratio: f32) -> f32 {
    10.0 * (ratio.max(1.0e-12)).log10()
}

/// Compare two spectra. Handles the three voicing cases (both voiced → full
/// partial diff; one voiced → `voicing_penalty_db` charged separately, no partial
/// match; both unvoiced → distance from the log bins only), and guards the
/// empty-bin / silence floor
/// (TODO §4.1): near-silent-vs-near-silent reads as distance 0 with
/// `floor_limited` set, and floor-vs-floor bins never inflate the scalar.
#[must_use]
pub fn compare(target: &SpectrumResult, candidate: &SpectrumResult) -> SpectrumDistance {
    let centroid_delta = Hertz::new(candidate.centroid.as_f32() - target.centroid.as_f32());
    let rolloff_delta = Hertz::new(candidate.rolloff.as_f32() - target.rolloff.as_f32());
    let flatness_delta = candidate.flatness.as_f32() - target.flatness.as_f32();
    let inharmonicity_delta = candidate.inharmonicity.as_f32() - target.inharmonicity.as_f32();
    let odd_even_ratio_delta_db =
        odd_even_ratio_db(candidate.odd_even_ratio) - odd_even_ratio_db(target.odd_even_ratio);

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
            voicing_penalty_db: 0.0,
            mel_l2_distance: 0.0,
            centroid_delta,
            rolloff_delta,
            flatness_delta,
            inharmonicity_delta,
            odd_even_ratio_delta_db,
            missing_partials: Vec::new(),
            extra_partials: Vec::new(),
            voicing_mismatch: false,
            floor_coverage: 0.0,
            floor_limited: true,
        };
    }

    let (lsd, floor_coverage) = log_spectral_distance(&target.log_bins, &candidate.log_bins);
    let mel_l2 = mel_l2_distance(&target.mel_bins, &candidate.mel_bins);

    let voicing_mismatch = target.voiced != candidate.voiced;

    // Only diff partials when both frames are voiced; otherwise partial-matching
    // across pitched/noise content is meaningless.
    let (missing, extra) = if target.voiced && candidate.voiced {
        diff_partials(&target.partials, &candidate.partials)
    } else {
        (Vec::new(), Vec::new())
    };

    // Keep the penalty out of the spectral scalar: fold it in and two candidates
    // that both mismatch the target's voicing peg to the same saturated value,
    // erasing the ranking signal `lsd`/`mel_l2` still carry (feedback §voicing).
    let voicing_penalty_db = if voicing_mismatch {
        VOICING_MISMATCH_PENALTY_DB
    } else {
        0.0
    };

    SpectrumDistance {
        log_spectral_distance: lsd,
        voicing_penalty_db,
        mel_l2_distance: mel_l2,
        centroid_delta,
        rolloff_delta,
        flatness_delta,
        inharmonicity_delta,
        odd_even_ratio_delta_db,
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
        let av = a[i].as_f32().max(LSD_FLOOR_DB);
        let bv = b[i].as_f32().max(LSD_FLOOR_DB);
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

/// True L2 (Euclidean) distance over the shared (min-length) prefix of two
/// peak-normalised log-mel band sets: `sqrt(Σ (aᵢ − bᵢ)²)`. Each band is clamped
/// up to [`LSD_FLOOR_DB`] first so a digital-silence band (≈ −240 dB) can't swamp
/// the sum. Unlike [`log_spectral_distance`] every band is kept (no shelf
/// exclusion): the perceptual mel axis already concentrates resolution in the
/// informative region, and "L2 over N bands" is the defined quantity. Returns 0
/// when either side is empty.
fn mel_l2_distance(a: &[Decibels], b: &[Decibels]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for i in 0..n {
        let av = f64::from(a[i].as_f32().max(LSD_FLOOR_DB));
        let bv = f64::from(b[i].as_f32().max(LSD_FLOOR_DB));
        let d = av - bv;
        sum += d * d;
    }
    sum.sqrt() as f32
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
            let cents = 1200.0 * (c.frequency.as_f32() / t.frequency.as_f32()).log2();
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

/// Peak-relative share of the loudest target frame's RMS below which a frame is
/// silence-in-time under [`FrameMask::TargetEnergy`]. Matches the external
/// ground-truth measure (keep only frames where the target actually has energy).
/// A frame's release tail is quiet relative to the burst *peak*, but per-frame
/// peak-normalisation still makes that tail's spectrum visible once it clears
/// this gate.
const TARGET_ENERGY_MASK_FRACTION: f32 = 0.05;
/// How many most-diverging frames [`compare_time_resolved`] reports.
const WORST_FRAMES: usize = 5;

/// Which frames a time-resolved comparison scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMask {
    /// Compare only frames whose *target* broadband RMS is voiced-relevant —
    /// above `max(NEAR_FLOOR_RMS, TARGET_ENERGY_MASK_FRACTION · peak_frame_rms)`.
    /// Frames below the gate (silence/decayed tail on the target side) are
    /// counted as masked, not compared, so silence never averages into the
    /// distance.
    TargetEnergy,
    /// Compare every paired frame.
    None,
}

/// One diverging frame from [`compare_time_resolved`]: its window-centre time
/// and per-frame log-spectral distance. The worst few point the caller at the
/// timestamps to listen to / re-window.
#[derive(Debug, Clone, Copy)]
pub struct WorstFrame {
    /// Window-centre time of the frame.
    pub time: Seconds,
    /// The frame's per-frame log-spectral distance.
    pub lsd: f32,
}

/// Result of [`compare_time_resolved`].
#[derive(Debug, Clone)]
pub struct TimeResolvedDistance {
    /// RMS of the per-frame log-spectral distance over the compared frames.
    pub lsd: f32,
    /// RMS of the per-frame mel L2 distance over the compared frames.
    pub mel_l2: f32,
    /// Frames actually compared (paired, and above the mask threshold).
    pub frames_compared: u32,
    /// Frames dropped by the mask (paired, but the target was below threshold).
    pub frames_masked: u32,
    /// The most-diverging compared frames, descending `lsd`, at most
    /// `WORST_FRAMES`.
    pub worst_frames: Vec<WorstFrame>,
}

/// Per-frame peak-normalised log-spectral + mel L2 distance for one aligned
/// frame pair. Each frame's bins are already peak-normalised to that frame's own
/// peak, so quiet-in-time content (a decaying tail) is scored at full contrast.
///
/// A silent side carries no bins (its frame fell below the analysis energy
/// floor); substitute a flat [`LSD_FLOOR_DB`] band the length of the other side
/// so a candidate that dropped content the target still carries reads as a large
/// distance rather than a spurious 0 (a missing bin set would otherwise make the
/// shared-length prefix empty → distance 0, ranking a silenced tail as a perfect
/// match).
fn frame_distance(target: &SpectrumResult, candidate: &SpectrumResult) -> (f32, f32) {
    let lsd = masked_bins_distance(&target.log_bins, &candidate.log_bins, |a, b| {
        log_spectral_distance(a, b).0
    });
    let mel = masked_bins_distance(&target.mel_bins, &candidate.mel_bins, mel_l2_distance);
    (lsd, mel)
}

/// Apply `dist` to two peak-normalised bin sets, substituting a flat
/// [`LSD_FLOOR_DB`] band for a *single* empty (silent) side so dropped content
/// scores as distance, not a spurious match. Allocates only on that empty-side
/// path; the common both-present case (and the both-empty case) passes the
/// slices straight through.
fn masked_bins_distance(
    a: &[Decibels],
    b: &[Decibels],
    dist: impl Fn(&[Decibels], &[Decibels]) -> f32,
) -> f32 {
    match (a.is_empty(), b.is_empty()) {
        (false, true) => dist(a, &vec![Decibels::new(LSD_FLOOR_DB); a.len()]),
        (true, false) => dist(&vec![Decibels::new(LSD_FLOOR_DB); b.len()], b),
        // Both present → compare directly; both empty → `dist` short-circuits to 0.
        _ => dist(a, b),
    }
}

/// Compare two spectrograms frame-by-frame, masking on the target's per-frame
/// energy, and return the RMS per-frame distance over the compared frames.
///
/// The aggregate [`compare`] averages a whole window into one spectrum, so on
/// silence-dominated / time-sparse material the loud frames drown out the quiet
/// content that actually distinguishes candidates. This measure instead scores
/// each frame on its own — with each frame's bins peak-normalised to that
/// frame's own peak — so a quiet-in-time release tail is compared at full
/// contrast. Frames are paired by index (the caller frames both sources with the
/// same `hop_ms`/`frame_len_ms`, at each source's own rate, after aligning the
/// two buffers), so a shared frame grid is assumed.
#[must_use]
pub fn compare_time_resolved(
    target_frames: &[SpectrogramFrame],
    candidate_frames: &[SpectrogramFrame],
    mask: FrameMask,
) -> TimeResolvedDistance {
    let n = target_frames.len().min(candidate_frames.len());

    // The mask threshold is relative to the target's own loudest frame: a tail
    // frame is silence-in-time relative to the burst peak, not to the window.
    let peak_frame_rms = target_frames
        .iter()
        .take(n)
        .map(|f| f.spectrum.frame_rms)
        .fold(0.0_f32, f32::max);
    let threshold = match mask {
        FrameMask::TargetEnergy => {
            (TARGET_ENERGY_MASK_FRACTION * peak_frame_rms).max(NEAR_FLOOR_RMS)
        }
        FrameMask::None => f32::NEG_INFINITY,
    };

    let mut sum_lsd = 0.0f64;
    let mut sum_mel = 0.0f64;
    let mut compared = 0u32;
    let mut masked = 0u32;
    let mut worst: Vec<WorstFrame> = Vec::new();

    for i in 0..n {
        let target = &target_frames[i].spectrum;
        if target.frame_rms < threshold {
            masked += 1;
            continue;
        }
        let candidate = &candidate_frames[i].spectrum;
        let (lsd, mel) = frame_distance(target, candidate);
        sum_lsd += f64::from(lsd) * f64::from(lsd);
        sum_mel += f64::from(mel) * f64::from(mel);
        compared += 1;
        worst.push(WorstFrame {
            time: target_frames[i].time,
            lsd,
        });
    }

    let rms = |sum: f64, count: u32| -> f32 {
        if count == 0 {
            0.0
        } else {
            (sum / f64::from(count)).sqrt() as f32
        }
    };
    worst.sort_by(|a, b| b.lsd.total_cmp(&a.lsd));
    worst.truncate(WORST_FRAMES);

    TimeResolvedDistance {
        lsd: rms(sum_lsd, compared),
        mel_l2: rms(sum_mel, compared),
        frames_compared: compared,
        frames_masked: masked,
        worst_frames: worst,
    }
}

/// RMS-envelope block size (ms) for [`envelope_align`], and the granularity of
/// the lag it returns. 10 ms is fine enough to align staccato onsets to within
/// one hop while keeping the cross-correlation cheap.
pub const ENV_ALIGN_WINDOW_MS: f32 = 10.0;

/// Cross-correlate the [`ENV_ALIGN_WINDOW_MS`] RMS envelopes of two mono buffers
/// and return the lag, **in whole envelope windows**, that best aligns the
/// candidate to the target. Positive = the candidate lags the target (shift the
/// candidate earlier by this many windows to align); negative = it leads; 0 when
/// either buffer is too short to yield an envelope.
///
/// Each envelope is built at its own sample rate but onto the same 10 ms time
/// grid, so the two sources may differ in rate (a 32 kHz reference vs a 44.1 kHz
/// render) — the caller converts the returned window lag back to samples at each
/// buffer's own rate. Both envelopes are mean-removed before correlating, so a
/// constant level offset between the sources doesn't bias the match. The search
/// is bounded to ±`max_lag_ms` (converted to windows and rounded).
#[must_use]
pub fn envelope_align(
    target: &[f32],
    target_sr: u32,
    candidate: &[f32],
    candidate_sr: u32,
    max_lag_ms: f32,
) -> i64 {
    let env_t = super::rms_envelope(target, target_sr, ENV_ALIGN_WINDOW_MS);
    let env_c = super::rms_envelope(candidate, candidate_sr, ENV_ALIGN_WINDOW_MS);
    if env_t.is_empty() || env_c.is_empty() {
        return 0;
    }
    let demean = |v: &[f32]| -> Vec<f64> {
        let mean = f64::from(v.iter().sum::<f32>()) / v.len() as f64;
        v.iter().map(|&x| f64::from(x) - mean).collect()
    };
    let env_t = demean(&env_t);
    let env_c = demean(&env_c);

    let (len_t, len_c) = (env_t.len() as i64, env_c.len() as i64);
    // Beyond ±max(len_t, len_c) windows the envelopes no longer overlap, so
    // clamp the search there. This also tames a non-finite or absurd
    // `max_lag_ms` — the saturating float→int cast (`inf` → `i64::MAX`) would
    // otherwise make the loop bound overflow and spin ~2^63 times.
    let overlap_bound = len_t.max(len_c);
    let max_lag = ((max_lag_ms.max(0.0) / ENV_ALIGN_WINDOW_MS).round() as i64).min(overlap_bound);

    let mut best_lag = 0i64;
    let mut best_corr = f64::NEG_INFINITY;
    for lag in -max_lag..=max_lag {
        // Sum over the indices where both envelopes overlap:
        // i ∈ [max(0, -lag), min(len_t, len_c - lag)).
        let i_start = (-lag).max(0);
        let i_end = len_t.min(len_c - lag);
        if i_start >= i_end {
            continue; // no overlap at this lag
        }
        let mut corr = 0.0f64;
        for i in i_start..i_end {
            let (ti, ci) = (i as usize, (i + lag) as usize);
            corr += env_t[ti] * env_c[ci];
        }
        // Strictly-greater → on an exact tie the first (most-negative) lag wins;
        // real envelopes have a unique peak, so ties don't arise in practice.
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }
    best_lag
}

/// Fraction (0..1) of a buffer's [`ENV_ALIGN_WINDOW_MS`] RMS windows whose level
/// clears `NEAR_FLOOR_RMS` — i.e. how much of the window actually carries
/// signal rather than silence/decay. The aggregate [`compare`] averages over the
/// whole window, so a low active-time fraction flags material where that scalar
/// is averaging over silence and the caller wants the time-resolved path
/// instead. Returns 0 when the buffer is too short to yield a window.
#[must_use]
pub fn active_time_fraction(samples: &[f32], sample_rate: u32) -> f32 {
    let env = super::rms_envelope(samples, sample_rate, ENV_ALIGN_WINDOW_MS);
    if env.is_empty() {
        return 0.0;
    }
    let active = env.iter().filter(|&&r| r > NEAR_FLOOR_RMS).count();
    active as f32 / env.len() as f32
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
        let f0 = r.f0.expect("voiced → f0").as_f32();
        assert!((f0 - 1000.0).abs() < 5.0, "f0 ≈ 1 kHz, got {f0}");
        let top = r.partials.first().expect("a partial");
        assert!(
            (top.frequency.as_f32() - 1000.0).abs() < 5.0,
            "loudest partial ≈ 1 kHz, got {}",
            top.frequency.as_f32()
        );
        assert!(
            r.flatness.as_f32() < 0.1,
            "pure tone flat, got {}",
            r.flatness.as_f32()
        );
        assert!(
            r.inharmonicity.as_f32() < 0.01,
            "harmonic, got {}",
            r.inharmonicity.as_f32()
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
        assert!(
            r.flatness.as_f32() > 0.3,
            "noise flat-ish, got {}",
            r.flatness.as_f32()
        );
        assert!(!r.voiced, "white noise should be unvoiced");
        assert!(r.f0.is_none(), "unvoiced → no f0");
        assert!(
            r.inharmonicity.as_f32() == 0.0,
            "no harmonic tagging when unvoiced"
        );
    }

    #[test]
    fn saw_is_voiced_with_harmonic_series() {
        let sig = saw(200.0, 16_384, 12);
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        assert!(r.voiced, "saw is pitched");
        let f0 = r.f0.expect("f0").as_f32();
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
                p.inharmonicity.as_f32().abs() < 30.0,
                "harmonic {} within 30 cents, got {} at {} Hz",
                p.harmonic_number.unwrap(),
                p.inharmonicity.as_f32(),
                p.frequency.as_f32()
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
            .filter(|p| p.amplitude.as_f32() > -30.0)
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
            .filter_map(|p| p.harmonic_number.map(|_| p.inharmonicity.as_f32().abs()))
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
            (top.frequency.as_f32() - 150.0).abs() < 5.0,
            "zero-pad + parabolic should locate the 150 Hz peak, got {} Hz",
            top.frequency.as_f32()
        );
    }

    #[test]
    fn silence_and_dc_do_not_nan() {
        for sig in [vec![0.0f32; 8192], vec![0.5f32; 8192]] {
            let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
            assert!(r.centroid.as_f32().is_finite());
            assert!(r.flatness.as_f32().is_finite());
            assert!(r.rolloff.as_f32().is_finite());
            assert!(r.inharmonicity.as_f32().is_finite());
            assert!(!r.voiced, "silence/DC is not voiced");
        }
    }

    #[test]
    fn peaks_stay_below_nyquist() {
        let sig = sine(21_000.0, 16_384, 0.8); // near Fs/2
        let r = analyze_spectrum(&sig, SR, SpectrumOpts::default());
        for p in &r.partials {
            assert!(
                p.frequency.as_f32() < SR as f32 / 2.0,
                "partial {} Hz at/above Nyquist",
                p.frequency.as_f32()
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
    fn mel_bins_emitted_and_l2_separates_timbres() {
        let opts = SpectrumOpts {
            log_bins: 64,
            mel_bands: 40,
            ..Default::default()
        };
        let saw_rich = analyze_spectrum(&saw(500.0, 16_384, 8), SR, opts);
        let sine_pure = analyze_spectrum(&sine(500.0, 16_384, 0.8), SR, opts);
        assert_eq!(saw_rich.mel_bins.len(), 40, "mel_bands controls band count");
        assert_eq!(sine_pure.mel_bins.len(), 40);

        // A rich saw and a bare sine at the same pitch have very different mel
        // envelopes → a clearly non-zero L2. The same spectrum against itself is 0.
        let cross = compare(&saw_rich, &sine_pure);
        let same = compare(&saw_rich, &saw_rich);
        assert!(
            cross.mel_l2_distance > 1.0,
            "saw vs sine should carry real mel-L2, got {}",
            cross.mel_l2_distance
        );
        assert!(
            same.mel_l2_distance.abs() < f32::EPSILON,
            "identical spectra must read mel-L2 0, got {}",
            same.mel_l2_distance
        );
    }

    #[test]
    fn mel_l2_is_euclidean_norm_of_band_differences() {
        // Hand-set mel bands: a fixed dB offset on 4 of 5 bands → the L2 is the
        // exact Euclidean norm sqrt(Σ dᵢ²), NOT the RMS the log-spectral distance
        // uses. Bands are within the LSD floor so no clamping alters the math.
        let mk = |db: &[f32]| SpectrumResult {
            mel_bins: db.iter().map(|&d| Decibels::new(d)).collect(),
            ..spectrum_from_bins(&[], 0.08, true)
        };
        let a = mk(&[0.0, -10.0, -20.0, -30.0, -40.0]);
        let b = mk(&[-3.0, -14.0, -20.0, -34.0, -45.0]);
        // Differences: 3, 4, 0, 4, 5 → sqrt(9+16+0+16+25) = sqrt(66) ≈ 8.124.
        let d = compare(&a, &b);
        assert!(
            (d.mel_l2_distance - 66.0f32.sqrt()).abs() < 1.0e-3,
            "expected true L2 sqrt(66) ≈ 8.124, got {}",
            d.mel_l2_distance
        );
    }

    #[test]
    fn compare_reports_rolloff_and_odd_even_deltas() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        // Target: bright saw (odd + even harmonics, energy up high). Candidate:
        // pure sine (all energy at the fundamental, only odd "harmonic").
        let saw_rich = analyze_spectrum(&saw(300.0, 16_384, 24), SR, opts);
        let sine_pure = analyze_spectrum(&sine(300.0, 16_384, 0.8), SR, opts);
        let d = compare(&saw_rich, &sine_pure);
        // The sine rolls off far lower than the harmonic-rich saw → negative.
        assert!(
            d.rolloff_delta.as_f32() < -200.0,
            "sine candidate should roll off well below the saw, got {} Hz",
            d.rolloff_delta.as_f32()
        );
        // A square (odd-only) candidate is more odd-dominant than a saw target.
        let square_odd = analyze_spectrum(&square(300.0, 16_384, 24), SR, opts);
        let d2 = compare(&saw_rich, &square_odd);
        assert!(
            d2.odd_even_ratio_delta_db > 6.0,
            "odd-only square vs saw should be markedly more odd-dominant, got {} dB",
            d2.odd_even_ratio_delta_db
        );
        // Same spectrum → both deltas ~0.
        let same = compare(&saw_rich, &saw_rich);
        assert!(same.rolloff_delta.as_f32().abs() < f32::EPSILON);
        assert!(same.odd_even_ratio_delta_db.abs() < 1.0e-3);
    }

    #[test]
    fn mel_bins_align_across_sample_rates() {
        // Same fixed mel grid as the log bins: a 2 kHz tone at 32 kHz and 44.1
        // kHz must peak in the same mel band, so a cross-rate mel-L2 is meaningful.
        fn tone(freq: f32, rate: u32, len: usize) -> Vec<f32> {
            (0..len)
                .map(|i| 0.8 * (TAU * freq * i as f32 / rate as f32).sin())
                .collect()
        }
        let opts = SpectrumOpts {
            mel_bands: 40,
            ..Default::default()
        };
        let lo = analyze_spectrum(&tone(2_000.0, 32_000, 16_384), 32_000, opts);
        let hi = analyze_spectrum(&tone(2_000.0, 44_100, 16_384), 44_100, opts);
        let argmax = |bins: &[Decibels]| -> usize {
            bins.iter()
                .enumerate()
                .max_by(|a, b| a.1.as_f32().total_cmp(&b.1.as_f32()))
                .map(|(i, _)| i)
                .unwrap_or(0)
        };
        assert!(
            argmax(&lo.mel_bins).abs_diff(argmax(&hi.mel_bins)) <= 1,
            "2 kHz tone should land in the same mel band at both rates"
        );
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
                .max_by(|a, b| a.1.as_f32().total_cmp(&b.1.as_f32()))
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
        // The penalty is reported on its own field, NOT folded into the spectral
        // scalar. Prove it: the scalar equals the raw log-bin RMS with the penalty
        // absent, so two mismatched candidates can still be ranked by it.
        assert_eq!(
            d.voicing_penalty_db, VOICING_MISMATCH_PENALTY_DB,
            "mismatch should charge the penalty on its own field"
        );
        let (raw_lsd, _) = log_spectral_distance(&voiced.log_bins, &unvoiced.log_bins);
        assert_eq!(
            d.log_spectral_distance, raw_lsd,
            "the spectral scalar must be the pure lsd, penalty not folded in"
        );
        assert!(
            d.missing_partials.is_empty(),
            "no partial matching across a voicing mismatch"
        );
    }

    /// The bug this fix targets: against one unvoiced target, two spectrally
    /// different voiced candidates used to peg to the same saturated scalar
    /// (`lsd + 60`), erasing the ranking signal. With the penalty on its own
    /// field, `log_spectral_distance` stays the pure spectral number, so the
    /// two candidates are distinguishable again.
    #[test]
    fn mismatch_scalar_still_ranks_two_candidates() {
        let opts = SpectrumOpts {
            log_bins: 64,
            ..Default::default()
        };
        let mut x = 0xC0FF_EE00u32;
        let noise: Vec<f32> = (0..16_384)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        let unvoiced = analyze_spectrum(&noise, SR, opts);

        // Two voiced candidates with clearly different spectra (low vs high f0).
        let low = compare(
            &analyze_spectrum(&saw(200.0, 16_384, 12), SR, opts),
            &unvoiced,
        );
        let high = compare(
            &analyze_spectrum(&saw(1500.0, 16_384, 6), SR, opts),
            &unvoiced,
        );

        assert!(low.voicing_mismatch && high.voicing_mismatch);
        assert_eq!(low.voicing_penalty_db, high.voicing_penalty_db);
        assert!(
            (low.log_spectral_distance - high.log_spectral_distance).abs() > 1.0,
            "the pure scalar must separate the two candidates ({} vs {})",
            low.log_spectral_distance,
            high.log_spectral_distance
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
            mel_bins: Vec::new(),
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
        assert!(
            frames[1].time.as_f32() > frames[0].time.as_f32(),
            "timestamps ascend"
        );

        // A frame well inside the tone is voiced; one well inside the noise is
        // not — the spectrogram reads the transition directly.
        let early = &frames[2];
        let late = &frames[frames.len() - 3];
        assert!(early.spectrum.voiced, "early (tone) frame should be voiced");
        let f0 = early.spectrum.f0.expect("tone frame has f0").as_f32();
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

    // --- time-resolved comparison ------------------------------------------

    /// Options that emit the log + mel bins `compare_time_resolved` needs.
    fn tr_opts() -> SpectrumOpts {
        SpectrumOpts {
            log_bins: 128,
            mel_bands: 40,
            ..SpectrumOpts::default()
        }
    }

    /// 40 ms window / 20 ms hop spectrogram, with bins enabled.
    fn tr_spectrogram(sig: &[f32]) -> Vec<SpectrogramFrame> {
        let win = (0.040 * SR as f32) as usize;
        let hop = (0.020 * SR as f32) as usize;
        analyze_spectrogram(sig, SR, hop, win, tr_opts())
    }

    /// One or two summed sines over `len` samples at amplitude `amp`.
    fn two_tone(f_a: f32, f_b: Option<f32>, len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let mut s = (TAU * f_a * t).sin();
                if let Some(fb) = f_b {
                    s += (TAU * fb * t).sin();
                }
                amp * s
            })
            .collect()
    }

    /// Deterministic white noise in [-amp, amp].
    fn noise(len: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn time_resolved_mask_ignores_junk_in_silent_gaps() {
        // Target: three 440 Hz bursts with silent gaps between them.
        let burst = sine(440.0, 6000, 0.6);
        let gap = vec![0.0f32; 6000];
        let mut target = Vec::new();
        for _ in 0..3 {
            target.extend_from_slice(&burst);
            target.extend_from_slice(&gap);
        }
        // Candidate: the same bursts; the junk noise sits in the *middle* of each
        // gap, guarded by silence so it never overlaps a frame the target burst
        // still occupies — it is purely gap content.
        let mut candidate = Vec::new();
        for k in 0..3 {
            candidate.extend_from_slice(&burst);
            candidate.extend_from_slice(&vec![0.0f32; 2000]);
            candidate.extend_from_slice(&noise(2000, 0.3, 7 + k));
            candidate.extend_from_slice(&vec![0.0f32; 2000]);
        }

        let tf = tr_spectrogram(&target);
        let cf = tr_spectrogram(&candidate);

        let masked = compare_time_resolved(&tf, &cf, FrameMask::TargetEnergy);
        let unmasked = compare_time_resolved(&tf, &cf, FrameMask::None);

        assert!(
            masked.frames_masked > 0,
            "the silent gaps should be masked, got {}",
            masked.frames_masked
        );
        // With the junk masked out, only the (matching) burst frames are scored.
        assert!(
            masked.lsd < 2.0,
            "masked distance should be ≈ 0 (junk ignored), got {}",
            masked.lsd
        );
        // Seeing the junk drives the distance up sharply.
        assert!(
            unmasked.lsd > masked.lsd + 10.0,
            "unmasked distance {} should dwarf masked {}",
            unmasked.lsd,
            masked.lsd
        );
    }

    #[test]
    fn time_resolved_ranks_tails_the_aggregate_cannot() {
        // A loud two-tone burst, then a quiet decaying-tail region that differs
        // between candidates. The burst is identical across all sources.
        let burst = two_tone(500.0, Some(1500.0), 8192, 1.0);
        let tail_full = two_tone(500.0, Some(1500.0), 8192, 0.15); // both partials
        let tail_partial = two_tone(500.0, None, 8192, 0.15); // 1500 Hz dropped
        let tail_silent = vec![0.0f32; 8192];

        let cat = |tail: &[f32]| {
            let mut v = burst.clone();
            v.extend_from_slice(tail);
            v
        };
        let target = cat(&tail_full);
        let cand_a = cat(&tail_full); // correct tail
        let cand_b = cat(&tail_partial); // wrong tail (held-tone-like)
        let cand_c = cat(&tail_silent); // silenced tail

        let tf = tr_spectrogram(&target);
        let tr =
            |c: &[f32]| compare_time_resolved(&tf, &tr_spectrogram(c), FrameMask::TargetEnergy);
        let (a, b, c) = (tr(&cand_a), tr(&cand_b), tr(&cand_c));

        // The whole point: per-frame distance ranks the three tails.
        assert!(
            a.lsd < b.lsd,
            "correct tail A {} < wrong tail B {}",
            a.lsd,
            b.lsd
        );
        assert!(
            b.lsd < c.lsd,
            "wrong tail B {} < silenced tail C {}",
            b.lsd,
            c.lsd
        );

        // The aggregate (whole-window) distance is dominated by the identical
        // loud burst, so it separates B from C far less than the framed measure
        // — the blind spot this plan exists for.
        let agg = |c: &[f32]| {
            let ta = analyze_spectrum(&target, SR, tr_opts());
            let ca = analyze_spectrum(c, SR, tr_opts());
            compare(&ta, &ca).log_spectral_distance
        };
        let (gb, gc) = (agg(&cand_b), agg(&cand_c));
        let tr_gap = c.lsd - b.lsd;
        assert!(
            tr_gap > 3.0,
            "time-resolved separates B/C clearly, gap {tr_gap}"
        );
        assert!(
            (gc - gb).abs() < 0.3 * tr_gap,
            "aggregate barely separates B ({gb}) from C ({gc}) vs time-resolved gap {tr_gap}"
        );
    }

    // --- envelope alignment ------------------------------------------------

    /// A `total`-sample buffer of silence with a 50 ms 440 Hz tone burst placed
    /// at `start`.
    fn burst_at(total: usize, start: usize) -> Vec<f32> {
        let dur = (0.050 * SR as f32) as usize;
        let mut v = vec![0.0f32; total];
        for i in 0..dur {
            if start + i < total {
                v[start + i] = 0.8 * (TAU * 440.0 * i as f32 / SR as f32).sin();
            }
        }
        v
    }

    #[test]
    fn envelope_align_recovers_known_delay() {
        let total = SR as usize; // 1 s
        let delay = (0.150 * SR as f32) as usize; // 150 ms = 15 windows
        let target = burst_at(total, (0.200 * SR as f32) as usize);
        let candidate = burst_at(total, (0.200 * SR as f32) as usize + delay);

        // Positive lag = candidate lags the target, in 10 ms windows.
        let lag = envelope_align(&target, SR, &candidate, SR, 250.0);
        assert!(
            (lag - 15).abs() <= 1,
            "expected +15 windows (150 ms) within one hop, got {lag}"
        );

        // Aligning a buffer with itself returns 0.
        assert_eq!(envelope_align(&target, SR, &target, SR, 250.0), 0);
    }

    #[test]
    fn envelope_align_respects_max_lag_clamp() {
        let total = SR as usize;
        let delay = (0.150 * SR as f32) as usize; // real delay 15 windows
        let target = burst_at(total, (0.200 * SR as f32) as usize);
        let candidate = burst_at(total, (0.200 * SR as f32) as usize + delay);

        // Cap the search at 50 ms (5 windows); the true 15-window lag is
        // unreachable, so the result is clamped inside the search bound rather
        // than reporting the real +15.
        let lag = envelope_align(&target, SR, &candidate, SR, 50.0);
        assert!(
            lag.abs() <= 5,
            "lag must stay within the ±5-window bound, got {lag}"
        );
        assert_ne!(
            lag, 15,
            "the clamp prevents reaching the true 15-window lag"
        );
    }

    #[test]
    fn envelope_align_short_buffers_return_zero() {
        assert_eq!(envelope_align(&[], SR, &[0.0; 4096], SR, 100.0), 0);
        assert_eq!(envelope_align(&[0.0; 4096], SR, &[], SR, 100.0), 0);
    }

    #[test]
    fn active_time_fraction_low_for_sparse_high_for_sustained() {
        // Sustained pad: energy nearly everywhere → high active fraction.
        let pad = sine(220.0, SR as usize, 0.5); // 1 s continuous tone
        let pad_frac = active_time_fraction(&pad, SR);
        assert!(
            pad_frac > 0.9,
            "sustained pad active fraction {pad_frac} > 0.9"
        );

        // Staccato: one short 50 ms burst in a 1 s window of silence → sparse.
        let sparse = burst_at(SR as usize, (0.400 * SR as f32) as usize);
        let sparse_frac = active_time_fraction(&sparse, SR);
        assert!(
            sparse_frac < 0.2,
            "staccato active fraction {sparse_frac} < 0.2"
        );
        // Below the 0.6 guard threshold the aggregate-mode warning uses.
        assert!(sparse_frac < 0.6);
    }

    #[test]
    fn envelope_align_non_finite_max_lag_is_bounded() {
        // A non-finite / absurd max_lag must not overflow the loop bound or hang;
        // the search is clamped to the envelope overlap and still finds the delay.
        let total = SR as usize;
        let delay = (0.150 * SR as f32) as usize;
        let target = burst_at(total, (0.200 * SR as f32) as usize);
        let candidate = burst_at(total, (0.200 * SR as f32) as usize + delay);
        for max_lag in [f32::INFINITY, 1.0e30, -f32::INFINITY, f32::NAN] {
            let lag = envelope_align(&target, SR, &candidate, SR, max_lag);
            assert!(
                lag.abs() <= total as i64,
                "bounded lag for max_lag={max_lag}"
            );
        }
        // The unbounded (inf) search still recovers the true +15-window delay.
        assert!((envelope_align(&target, SR, &candidate, SR, f32::INFINITY) - 15).abs() <= 1);
    }
}
