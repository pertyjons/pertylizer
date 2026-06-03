//! Shared vowel formant tables for the formant-based voice modules.
//!
//! Single source of truth for the A→E→I→O→U formant data used by
//! `voice_synth`, `formant_filter` and `am_formant`. Previously each module
//! carried its own private copy; folding tuning changes (or a new band) into
//! one place keeps the vowels from silently diverging across modules.
//!
//! ## Band layout
//! - Bands 0–2 are the classic F1/F2/F3 vowel formants. Their values are
//!   unchanged from the historical per-module tables, so 3-band consumers
//!   (`formant_filter`, `am_formant`) that read only `[..3]` stay numerically
//!   identical.
//! - Band 3 is the **singer's formant** — a high resonance (~3.3–3.7 kHz) that
//!   adds the "ring"/brilliance trained singers and choirs project with. Only
//!   `voice_synth` reads it (4 bands); the speech/effect modules ignore it.
//!
//! Consumers decide how many bands they use via their own `NUM_BANDS` and index
//! `FORMANT_*[vowel][band]`; the tables are always [`NUM_BANDS`] wide.

/// Number of vowels (A, E, I, O, U).
pub const NUM_VOWELS: usize = 5;
/// Number of formant bands provided per vowel (F1, F2, F3, singer's formant).
pub const NUM_BANDS: usize = 4;

/// Formant centre frequencies for each vowel `[vowel][band]` in Hz.
pub const FORMANT_FREQ: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [800.0, 1150.0, 2900.0, 3400.0], // A (as in "father")
    [350.0, 2000.0, 2800.0, 3600.0], // E (as in "bed")
    [270.0, 2140.0, 3200.0, 3700.0], // I (as in "heed")
    [450.0, 800.0, 2830.0, 3400.0],  // O (as in "hot")
    [325.0, 700.0, 2530.0, 3300.0],  // U (as in "boot")
];

/// Formant bandwidths for each vowel `[vowel][band]` in Hz.
pub const FORMANT_BW: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [80.0, 90.0, 120.0, 180.0],  // A
    [60.0, 100.0, 120.0, 180.0], // E
    [60.0, 90.0, 100.0, 160.0],  // I
    [70.0, 80.0, 100.0, 180.0],  // O
    [50.0, 60.0, 170.0, 200.0],  // U
];

/// Formant gains (linear) for each vowel `[vowel][band]`.
pub const FORMANT_GAIN: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [1.0, 0.5, 0.25, 0.15],  // A
    [1.0, 0.5, 0.2, 0.15],   // E
    [1.0, 0.35, 0.15, 0.13], // I
    [1.0, 0.35, 0.2, 0.13],  // O
    [1.0, 0.3, 0.15, 0.1],   // U
];
