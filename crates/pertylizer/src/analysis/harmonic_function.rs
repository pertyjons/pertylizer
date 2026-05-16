//! Tonal-function labels and cadence detection on top of `analyze_harmony`.
//!
//! Pure symbolic — consumes the chord-event stream + inferred key already
//! produced by `crate::harmony::analyze`. Assigns each chord a scale-degree
//! Roman numeral (I, V7, IV, vii°, …), a tonal function (Tonic / Subdominant
//! / Dominant / Other / Chromatic), and a per-chord tension score, then
//! detects cadences in consecutive chord pairs (Authentic V → I, Plagal IV
//! → I, Half (anything → V), Deceptive V → vi).
//!
//! Chord labels and key are produced by `crate::harmony` — this module never
//! looks at MIDI notes directly.

/// Tonal function bucket. Simplified Riemannian model: every diatonic chord
/// is mapped to one of T / S / D, mediants (iii, vi) and other diatonic
/// chords get `Other`, and chords whose root is not in the inferred key's
/// scale get `Chromatic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarmonicFunction {
    Tonic,
    Subdominant,
    Dominant,
    Other,
    Chromatic,
}

impl HarmonicFunction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tonic => "tonic",
            Self::Subdominant => "subdominant",
            Self::Dominant => "dominant",
            Self::Other => "other",
            Self::Chromatic => "chromatic",
        }
    }

    /// Default tension contribution for this function. The analyzer
    /// composes these with chord quality (dominant 7th adds tension) when
    /// assigning the final per-chord tension.
    #[must_use]
    pub fn base_tension(self) -> f32 {
        match self {
            Self::Tonic => 0.0,
            Self::Subdominant => 0.3,
            Self::Dominant => 0.7,
            Self::Other => 0.5,
            Self::Chromatic => 1.0,
        }
    }
}

/// Cadence kind detected between two consecutive chord events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CadenceKind {
    /// V → I (or V7 → I).
    Authentic,
    /// IV → I.
    Plagal,
    /// Anything → V. Often appears at the end of a phrase before a return.
    HalfCadence,
    /// V → vi (in major) or V → VI (in minor).
    Deceptive,
}

impl CadenceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authentic => "authentic",
            Self::Plagal => "plagal",
            Self::HalfCadence => "half_cadence",
            Self::Deceptive => "deceptive",
        }
    }
}

/// One annotated chord event in the output stream.
#[derive(Debug, Clone)]
pub struct ChordFunction {
    /// Original chord symbol, e.g. `"Cm7"`.
    pub symbol: String,
    /// Scale-degree index (0..7 for diatonic, `None` for chromatic).
    pub scale_degree: Option<u8>,
    /// Roman numeral with quality decoration, e.g. `"V7"`, `"ii7"`, `"vii°"`.
    /// `"?"` when no chord was identified at this position.
    pub roman_numeral: String,
    pub function: HarmonicFunction,
    /// 0.0..=1.0. Per-chord tension contribution.
    pub tension: f32,
    /// `chord.in_key` from the harmony analyzer — true when every pitch in
    /// the window belongs to the inferred key's scale, regardless of
    /// chord-root diatonicism.
    pub in_key: bool,
    /// Cadence ending at this chord, if one was detected on the
    /// `(previous, this)` pair. `None` for the first chord and for pairs
    /// that don't match a cadence template.
    pub cadence: Option<CadenceKind>,
}

#[derive(Debug, Clone, Copy)]
pub struct CadenceEvent {
    /// Index into the chord-function array.
    pub chord_index: u32,
    pub kind: CadenceKind,
}

/// Per-function counts across the chord stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct FunctionDistribution {
    pub tonic: u32,
    pub subdominant: u32,
    pub dominant: u32,
    pub other: u32,
    pub chromatic: u32,
}

/// Tension-curve summary.
#[derive(Debug, Clone, Copy, Default)]
pub struct TensionStats {
    pub mean: f32,
    pub peak: f32,
    pub trough: f32,
    /// Standard deviation across all annotated chords (0 for ≤ 1 chord).
    pub std_dev: f32,
}

/// Aggregate analysis output. Bridge code maps this onto the wire-format
/// `AnalyzeHarmonicFunctionResult`.
#[derive(Debug, Clone)]
pub struct HarmonicFunctionAnalysis {
    pub chords: Vec<ChordFunction>,
    pub cadences: Vec<CadenceEvent>,
    pub function_distribution: FunctionDistribution,
    pub tension: TensionStats,
    pub warnings: Vec<String>,
}

/// Inputs derived from `crate::harmony::analyze` output. Decoupled so this
/// module never depends on the harmony module directly — the bridge
/// converts the harmony output into these inputs before calling
/// [`analyze`].
#[derive(Debug, Clone)]
pub struct ChordInput {
    pub symbol: Option<String>,
    /// Root pitch class (0..12). `None` when no chord was identified.
    pub root: Option<u8>,
    /// Quality template name e.g. `"minor7"`, `"major"`. Matches the
    /// `quality` field of `crate::harmony::Chord`. `None` when no chord
    /// was identified.
    pub quality: Option<String>,
    pub in_key: bool,
}

/// Mode of the inferred key — only major and minor are distinguished here,
/// matching the Krumhansl-Schmuckler output of `crate::harmony::infer_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    Major,
    Minor,
}

impl KeyMode {
    /// Parse from `KeyEstimate.mode` (`"major"` / `"minor"`); anything else
    /// falls back to `Major` so a downstream analysis still runs.
    #[must_use]
    pub fn from_label(mode: &str) -> Self {
        match mode {
            "minor" => Self::Minor,
            _ => Self::Major,
        }
    }
}

/// Diatonic Roman numerals in major. The diminished `vii` chord gets its
/// `°` suffix from `quality_decoration` so the numeral here stays plain.
const MAJOR_ROMANS: [&str; 7] = ["I", "ii", "iii", "IV", "V", "vi", "vii"];
/// Natural-minor diatonic Roman numerals. `ii` and the leading-tone get
/// their diminished suffix from `quality_decoration`.
const MINOR_ROMANS: [&str; 7] = ["i", "ii", "III", "iv", "v", "VI", "VII"];

/// Function bucket per scale degree (`I` = 0, `ii` = 1, …). Captures the
/// simplified Riemannian assignment used by `HarmonicFunction::base_tension`.
const FUNCTION_BY_DEGREE_MAJOR: [HarmonicFunction; 7] = [
    HarmonicFunction::Tonic,       // I
    HarmonicFunction::Subdominant, // ii
    HarmonicFunction::Other,       // iii (mediant)
    HarmonicFunction::Subdominant, // IV
    HarmonicFunction::Dominant,    // V
    HarmonicFunction::Other,       // vi (submediant)
    HarmonicFunction::Dominant,    // vii (diminished)
];
const FUNCTION_BY_DEGREE_MINOR: [HarmonicFunction; 7] = [
    HarmonicFunction::Tonic,       // i
    HarmonicFunction::Subdominant, // ii°
    HarmonicFunction::Other,       // III
    HarmonicFunction::Subdominant, // iv
    HarmonicFunction::Dominant,    // V (often raised 7th in practice)
    HarmonicFunction::Other,       // VI
    HarmonicFunction::Dominant,    // VII
];

/// Run the harmonic-function analysis.
///
/// Returns one annotated `ChordFunction` per input chord, in input order.
/// `inferred_key_tonic` is the pitch class of the inferred key tonic (0 =
/// C, 11 = B); pass `None` to skip the diatonic analysis (every chord
/// labels as `Chromatic` with tension 1.0).
#[must_use]
pub fn analyze(
    chords: &[ChordInput],
    inferred_key_tonic: Option<u8>,
    key_mode: KeyMode,
) -> HarmonicFunctionAnalysis {
    let mut warnings: Vec<String> = Vec::new();
    if chords.is_empty() {
        warnings.push("No chord events to analyze".to_string());
        return HarmonicFunctionAnalysis {
            chords: Vec::new(),
            cadences: Vec::new(),
            function_distribution: FunctionDistribution::default(),
            tension: TensionStats::default(),
            warnings,
        };
    }

    let Some(tonic) = inferred_key_tonic.filter(|t| *t < 12) else {
        warnings.push(
            "No key was inferred (or it was invalid) — every chord labels as chromatic".to_string(),
        );
        let mut out: Vec<ChordFunction> = chords
            .iter()
            .map(|c| ChordFunction {
                symbol: c.symbol.clone().unwrap_or_else(|| "?".to_string()),
                scale_degree: None,
                roman_numeral: c.symbol.clone().unwrap_or_else(|| "?".to_string()),
                function: HarmonicFunction::Chromatic,
                tension: HarmonicFunction::Chromatic.base_tension(),
                in_key: c.in_key,
                cadence: None,
            })
            .collect();
        let cadences = detect_cadences(&mut out);
        let function_distribution = aggregate_functions(&out);
        let tension = aggregate_tension(&out);
        return HarmonicFunctionAnalysis {
            chords: out,
            cadences,
            function_distribution,
            tension,
            warnings,
        };
    };

    let scale_name = match key_mode {
        KeyMode::Major => "major",
        KeyMode::Minor => "minor",
    };
    let scale = crate::harmony::scale_by_name(scale_name).intervals;
    let (romans, functions) = match key_mode {
        KeyMode::Major => (MAJOR_ROMANS, FUNCTION_BY_DEGREE_MAJOR),
        KeyMode::Minor => (MINOR_ROMANS, FUNCTION_BY_DEGREE_MINOR),
    };

    let mut out: Vec<ChordFunction> = chords
        .iter()
        .map(|c| match (c.root, c.quality.as_deref()) {
            (Some(root), Some(quality)) => {
                let symbol = c.symbol.clone().unwrap_or_else(|| "?".to_string());
                let interval = (12 + root - tonic) % 12;
                let degree_idx = scale.iter().position(|&i| i == interval).map(|i| i as u8);
                match degree_idx {
                    Some(idx) => {
                        let base_roman = romans[idx as usize];
                        let decoration = quality_decoration(quality);
                        let roman = format!("{base_roman}{decoration}");
                        let function = functions[idx as usize];
                        let tension = chord_tension(function, quality);
                        ChordFunction {
                            symbol,
                            scale_degree: Some(idx + 1),
                            roman_numeral: roman,
                            function,
                            tension,
                            in_key: c.in_key,
                            cadence: None,
                        }
                    }
                    None => {
                        let alteration = altered_roman(interval, quality, key_mode);
                        // Raised-7 leading-tone in minor with a major /
                        // dominant quality is the borrowed-V from harmonic
                        // minor — still a Dominant, not Chromatic.
                        let function = if key_mode == KeyMode::Minor
                            && is_dominant_or_major(quality)
                            && interval == 11
                        {
                            HarmonicFunction::Dominant
                        } else {
                            HarmonicFunction::Chromatic
                        };
                        let tension = chord_tension(function, quality);
                        ChordFunction {
                            symbol,
                            scale_degree: None,
                            roman_numeral: alteration,
                            function,
                            tension,
                            in_key: c.in_key,
                            cadence: None,
                        }
                    }
                }
            }
            _ => ChordFunction {
                symbol: c.symbol.clone().unwrap_or_else(|| "?".to_string()),
                scale_degree: None,
                roman_numeral: "?".to_string(),
                function: HarmonicFunction::Other,
                tension: HarmonicFunction::Other.base_tension(),
                in_key: c.in_key,
                cadence: None,
            },
        })
        .collect();

    let cadences = detect_cadences(&mut out);
    let function_distribution = aggregate_functions(&out);
    let tension = aggregate_tension(&out);

    HarmonicFunctionAnalysis {
        chords: out,
        cadences,
        function_distribution,
        tension,
        warnings,
    }
}

fn quality_decoration(quality: &str) -> &'static str {
    match quality {
        "major7" | "major7_sharp11" => "maj7",
        "minor7" => "7",
        "dominant7" | "dominant7_sus4" => "7",
        "minor7_flat5" => "ø7",
        "diminished7" => "°7",
        "major6" => "6",
        "minor6" => "6",
        "minor_major7" => "(mMaj7)",
        "augmented7" => "+7",
        "augmented" => "+",
        "diminished" => "°",
        "sus4" => "sus4",
        "sus2" => "sus2",
        "power" => "5",
        _ => "",
    }
}

fn is_dominant_or_major(quality: &str) -> bool {
    matches!(
        quality,
        "major" | "major7" | "dominant7" | "dominant7_sus4" | "major6"
    )
}

fn chord_tension(function: HarmonicFunction, quality: &str) -> f32 {
    let mut t = function.base_tension();
    // Dominant 7ths add extra tension on top of the function's base.
    if matches!(quality, "dominant7" | "dominant7_sus4" | "augmented7") {
        t = (t + 0.15).min(1.0);
    }
    // Diminished / half-diminished chords are extra unstable.
    if matches!(quality, "diminished" | "diminished7" | "minor7_flat5") {
        t = (t + 0.2).min(1.0);
    }
    t.clamp(0.0, 1.0)
}

fn altered_roman(interval: u8, quality: &str, key_mode: KeyMode) -> String {
    // Best-effort Roman label for non-diatonic roots. The flat/sharp prefix
    // identifies which scale degree the alteration moves from; the quality
    // is appended for clarity.
    let base = match interval {
        1 => "bII",
        3 => "bIII",
        6 => "bV",
        8 => "bVI",
        10 => "bVII",
        11 => match key_mode {
            KeyMode::Major => "vii",
            KeyMode::Minor => "VII",
        },
        _ => "?",
    };
    let decoration = quality_decoration(quality);
    format!("{base}{decoration}")
}

fn detect_cadences(chords: &mut [ChordFunction]) -> Vec<CadenceEvent> {
    let mut cadences: Vec<CadenceEvent> = Vec::new();
    for i in 1..chords.len() {
        let prev = &chords[i - 1];
        let kind = match (prev.scale_degree, chords[i].scale_degree) {
            // V → I (or V7 → I) authentic.
            (Some(5), Some(1)) => Some(CadenceKind::Authentic),
            // IV → I plagal.
            (Some(4), Some(1)) => Some(CadenceKind::Plagal),
            // V → vi deceptive.
            (Some(5), Some(6)) => Some(CadenceKind::Deceptive),
            // anything → V half-cadence (skip V → V).
            (Some(p), Some(5)) if p != 5 => Some(CadenceKind::HalfCadence),
            _ => None,
        };
        if let Some(kind) = kind {
            chords[i].cadence = Some(kind);
            cadences.push(CadenceEvent {
                chord_index: i as u32,
                kind,
            });
        }
    }
    cadences
}

fn aggregate_functions(chords: &[ChordFunction]) -> FunctionDistribution {
    let mut dist = FunctionDistribution::default();
    for c in chords {
        match c.function {
            HarmonicFunction::Tonic => dist.tonic += 1,
            HarmonicFunction::Subdominant => dist.subdominant += 1,
            HarmonicFunction::Dominant => dist.dominant += 1,
            HarmonicFunction::Other => dist.other += 1,
            HarmonicFunction::Chromatic => dist.chromatic += 1,
        }
    }
    dist
}

fn aggregate_tension(chords: &[ChordFunction]) -> TensionStats {
    if chords.is_empty() {
        return TensionStats::default();
    }
    let n = chords.len() as f32;
    let mut peak = f32::NEG_INFINITY;
    let mut trough = f32::INFINITY;
    let mut sum = 0.0f32;
    for c in chords {
        peak = peak.max(c.tension);
        trough = trough.min(c.tension);
        sum += c.tension;
    }
    let mean = sum / n;
    let std_dev = if chords.len() < 2 {
        0.0
    } else {
        let var = chords
            .iter()
            .map(|c| (c.tension - mean).powi(2))
            .sum::<f32>()
            / n;
        var.sqrt()
    };
    TensionStats {
        mean,
        peak,
        trough,
        std_dev,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ch(symbol: &str, root: u8, quality: &str) -> ChordInput {
        ChordInput {
            symbol: Some(symbol.to_string()),
            root: Some(root),
            quality: Some(quality.to_string()),
            in_key: true,
        }
    }

    #[test]
    fn empty_input_warns() {
        let out = analyze(&[], Some(0), KeyMode::Major);
        assert!(out.warnings.iter().any(|w| w.contains("No chord events")));
    }

    #[test]
    fn c_major_one_four_five_one_resolves_to_authentic_cadence() {
        // C, F, G, C in C major → I, IV, V, I.
        let chords = vec![
            ch("C", 0, "major"),
            ch("F", 5, "major"),
            ch("G", 7, "major"),
            ch("C", 0, "major"),
        ];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[0].roman_numeral, "I");
        assert_eq!(out.chords[0].function, HarmonicFunction::Tonic);
        assert_eq!(out.chords[1].roman_numeral, "IV");
        assert_eq!(out.chords[1].function, HarmonicFunction::Subdominant);
        assert_eq!(out.chords[2].roman_numeral, "V");
        assert_eq!(out.chords[2].function, HarmonicFunction::Dominant);
        assert_eq!(out.chords[3].roman_numeral, "I");
        assert_eq!(out.chords[3].cadence, Some(CadenceKind::Authentic));
        // Cadences: i=1 IV→V is half-cadence, i=2 V→I authentic, plus
        // implicit IV before V on i=1.
        assert!(
            out.cadences
                .iter()
                .any(|c| c.kind == CadenceKind::Authentic)
        );
    }

    #[test]
    fn plagal_cadence_detected_iv_to_i() {
        let chords = vec![ch("F", 5, "major"), ch("C", 0, "major")];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[1].cadence, Some(CadenceKind::Plagal));
        assert_eq!(out.cadences.len(), 1);
    }

    #[test]
    fn deceptive_cadence_v_to_vi() {
        let chords = vec![ch("G", 7, "major"), ch("Am", 9, "minor")];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[1].cadence, Some(CadenceKind::Deceptive));
    }

    #[test]
    fn half_cadence_into_v() {
        let chords = vec![ch("C", 0, "major"), ch("G", 7, "major")];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[1].cadence, Some(CadenceKind::HalfCadence));
    }

    #[test]
    fn minor_key_ivi_progression_labeled() {
        // C minor: Cm, Fm, G, Cm → i, iv, V, i.
        let chords = vec![
            ch("Cm", 0, "minor"),
            ch("Fm", 5, "minor"),
            ch("G", 7, "major"),
            ch("Cm", 0, "minor"),
        ];
        let out = analyze(&chords, Some(0), KeyMode::Minor);
        assert_eq!(out.chords[0].roman_numeral, "i");
        assert_eq!(out.chords[1].roman_numeral, "iv");
        // G in C minor: interval 7 → diatonic v in natural minor, ROMANS index 4.
        assert!(out.chords[2].roman_numeral.starts_with("v"));
        assert_eq!(out.chords[2].function, HarmonicFunction::Dominant);
        // V (major triad) → i is authentic.
        assert_eq!(out.chords[3].cadence, Some(CadenceKind::Authentic));
    }

    #[test]
    fn chromatic_chord_labeled_chromatic() {
        // C major, Db chord = chromatic (bII).
        let chords = vec![ch("Db", 1, "major")];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[0].function, HarmonicFunction::Chromatic);
        assert!(out.chords[0].roman_numeral.starts_with("bII"));
        assert!((out.chords[0].tension - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tension_curve_peaks_on_dominant_seventh() {
        let chords = vec![
            ch("C", 0, "major"),      // tonic
            ch("G7", 7, "dominant7"), // dominant 7 — extra tension
            ch("C", 0, "major"),      // back to tonic
        ];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert!(out.chords[1].tension > out.chords[0].tension);
        assert!(out.chords[1].tension > out.chords[2].tension);
    }

    #[test]
    fn no_key_inferred_everything_chromatic() {
        let chords = vec![ch("C", 0, "major"), ch("G", 7, "major")];
        let out = analyze(&chords, None, KeyMode::Major);
        assert!(out.warnings.iter().any(|w| w.contains("No key")));
        assert!(
            out.chords
                .iter()
                .all(|c| c.function == HarmonicFunction::Chromatic)
        );
    }

    #[test]
    fn function_distribution_counts_buckets() {
        // I, ii, V, I — 2 T, 1 S, 1 D.
        let chords = vec![
            ch("C", 0, "major"),
            ch("Dm", 2, "minor"),
            ch("G", 7, "major"),
            ch("C", 0, "major"),
        ];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.function_distribution.tonic, 2);
        assert_eq!(out.function_distribution.subdominant, 1);
        assert_eq!(out.function_distribution.dominant, 1);
    }

    #[test]
    fn diminished_vii_in_major() {
        // C major, B°: vii° → dominant function.
        let chords = vec![ch("B°", 11, "diminished")];
        let out = analyze(&chords, Some(0), KeyMode::Major);
        assert_eq!(out.chords[0].roman_numeral, "vii°");
        assert_eq!(out.chords[0].function, HarmonicFunction::Dominant);
    }

    #[test]
    fn missing_chord_falls_back_to_question_mark() {
        let input = vec![ChordInput {
            symbol: None,
            root: None,
            quality: None,
            in_key: false,
        }];
        let out = analyze(&input, Some(0), KeyMode::Major);
        assert_eq!(out.chords[0].roman_numeral, "?");
    }
}
