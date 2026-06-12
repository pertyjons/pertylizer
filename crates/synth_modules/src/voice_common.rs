//! Shared building blocks for the unison/choir voice engines (`VoiceSynth`,
//! `Fof`). Both run a fixed bank of decorrelated sub-voices whose detune,
//! vibrato rate, formant jitter, pan and onset stagger are derived
//! deterministically from the sub-voice index and the played note — identical
//! logic that previously lived twice. The per-voice *DSP* (bandpass bank vs FOF
//! grain rings) stays in each module; only the decorrelation maths are shared.

/// Maximum number of unison sub-voices (fixed arrays — no heap in `process()`).
pub const MAX_UNISON: usize = 16;
/// Non-zero PRNG seed (golden-ratio constant) for per-voice noise.
pub const RNG_SEED: u32 = 0x9E37_79B9;
/// Maximum per-voice onset stagger, in seconds, for choir attack decorrelation.
pub const ONSET_MAX_SECS: f32 = 0.004;

/// Deterministic decorrelation hash in `[0, 1)` from (voice index, note, salt).
/// Reproducible and allocation-free — used instead of `Math.random` so a given
/// note always decorrelates its voices the same way.
#[inline]
#[must_use]
pub fn decorr_hash(voice: usize, note: f32, salt: f32) -> f32 {
    let x = voice as f32 * 0.618_034 + note * 0.019_3 + salt;
    let h = (x.sin() * 43758.547).abs();
    h - h.floor()
}

/// Number of active unison sub-voices (`1..=max`) from a normalized knob.
#[inline]
#[must_use]
pub fn unison_count(norm: f32, max: usize) -> usize {
    let n = 1 + (norm * (max - 1) as f32).round() as usize;
    n.clamp(1, max)
}

/// Per-voice onset stagger, in samples, deterministic from (voice, note).
#[inline]
#[must_use]
pub fn onset_samples(voice: usize, note: f32, onset_max_samples: f32) -> u32 {
    (decorr_hash(voice, note, 5.0) * onset_max_samples) as u32
}

/// Per-voice decorrelation offsets for one unison sub-voice: detune, vibrato
/// rate multiplier, formant jitter and equal-power stereo pan.
#[derive(Clone, Copy)]
pub struct VoiceSpread {
    pub detune_cents: f32,
    pub vib_rate_mult: f32,
    pub formant_jitter: f32,
    pub pan_l: f32,
    pub pan_r: f32,
}

impl VoiceSpread {
    /// A centred, un-detuned solo voice (used when only one voice is active, so
    /// a solo exactly matches the single-voice signal path).
    #[must_use]
    pub fn solo() -> Self {
        Self {
            detune_cents: 0.0,
            vib_rate_mult: 1.0,
            formant_jitter: 1.0,
            pan_l: std::f32::consts::FRAC_1_SQRT_2,
            pan_r: std::f32::consts::FRAC_1_SQRT_2,
        }
    }

    /// Decorrelated sub-voice `voice` of `active` (`active >= 2`), deterministic
    /// from the note. `detune_cents` is the full spread; `spread` the stereo
    /// width (0 = mono, 1 = full).
    #[must_use]
    pub fn derive(voice: usize, active: usize, note: f32, detune_cents: f32, spread: f32) -> Self {
        let detune = (decorr_hash(voice, note, 1.0) * 2.0 - 1.0) * detune_cents;
        let vib_rate_mult = 1.0 + (decorr_hash(voice, note, 2.0) * 2.0 - 1.0) * 0.08;
        let formant_jitter = 1.0 + (decorr_hash(voice, note, 3.0) * 2.0 - 1.0) * 0.03;
        let pos = (voice as f32 / (active - 1) as f32) * 2.0 - 1.0;
        let (pan_l, pan_r) = crate::math::equal_power_pan(pos * spread);
        Self {
            detune_cents: detune,
            vib_rate_mult,
            formant_jitter,
            pan_l,
            pan_r,
        }
    }
}
