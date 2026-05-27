//! Auto-inference of [`InstrumentProfile`] from existing state.
//!
//! `analyze_harmony` historically required users to manually tag drum
//! instruments via `set_instrument_category`; when the tag was missing, the
//! drum-filter became a silent no-op and percussion polluted chord
//! identification. This module fills that gap by deriving per-instrument
//! characteristics from three sources that always exist:
//!
//! 1. [`InstrumentSnapshot`] — name, manual category, etc.
//! 2. [`ModuleStateSnapshot`] — voice-graph contents (oscillators, noise,
//!    envelopes, ...) and their parameter values.
//! 3. Notes the instrument plays — extracted from `Song` placements.
//!
//! The profile exposes several independent axes (envelope shape, pitch role,
//! register, texture, role) so future consumers can pick whichever axis is
//! relevant to their task without `role` being the single source of truth.

use std::collections::HashSet;

use serde::Serialize;

use synth_core::params::EnvelopeParam;
use synth_core::{ModuleType, Param, SamplerParam, SamplerPlayMode};
use synth_engine::state::EngineState;
use synth_engine::{InstrumentCategory, InstrumentSnapshot, ModuleStateSnapshot};
use synth_sequencer::{SeqInstrumentId, SequencerTrack, Song};

/// Full profile for a single instrument across several independent axes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InstrumentProfile {
    /// Sequencer instrument id (matches `SeqInstrumentId.0`).
    pub instrument_id: u16,
    pub instrument_name: String,
    pub role: RoleInference,
    pub envelope_shape: EnvelopeShape,
    pub pitch_role: PitchRole,
    pub register: Register,
    pub texture: Texture,
}

/// Best-guess role for an instrument plus a confidence score and a trail of
/// signals that contributed to the classification.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RoleInference {
    pub role: Role,
    /// 0.0..=1.0 — how strongly the heuristics agree.
    pub confidence: f32,
    pub signals: Vec<ProfileSignal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Drums,
    Bass,
    Lead,
    Pad,
    Pluck,
    Keys,
    Fx,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeShape {
    /// Sustain near zero and decay+release < 200 ms.
    Percussive,
    /// Sustain < 0.3 and release < 500 ms.
    Plucked,
    /// Sustain >= 0.3.
    Sustained,
    /// Attack > 500 ms (classic pad).
    Evolving,
    /// No amp-envelope found in the voice graph.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitchRole {
    /// >= 5 distinct pitch classes across all notes played.
    Tonal,
    /// <= 2 distinct pitch classes.
    Atonal,
    /// Strictly between Atonal and Tonal.
    Mixed,
    /// No notes assigned to this instrument.
    Unused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Register {
    /// Median MIDI < 40.
    Sub,
    /// Median MIDI 40..=55.
    Bass,
    /// Median MIDI 56..=72.
    Mid,
    /// Median MIDI > 72.
    High,
    /// Note spread > 36 semitones — supersedes a median-based bucket.
    FullRange,
    Unused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Texture {
    /// Max 1 note sounding simultaneously.
    Monophonic,
    /// 2..=4 simultaneous notes.
    Polyphonic,
    /// >= 5 simultaneous notes.
    Chordal,
    Unused,
}

/// Which input fed into a classification decision. Closed set — a UI/tool
/// can render this without surprises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalAxis {
    Name,
    Graph,
    Envelope,
    Pattern,
    Manual,
    Decision,
}

impl SignalAxis {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Graph => "graph",
            Self::Envelope => "envelope",
            Self::Pattern => "pattern",
            Self::Manual => "manual",
            Self::Decision => "decision",
        }
    }
}

impl std::fmt::Display for SignalAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single signal that contributed to a classification decision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ProfileSignal {
    pub axis: SignalAxis,
    /// e.g. "kick", "noise-no-osc", "percussive", "atonal", ...
    pub detail: &'static str,
}

impl ProfileSignal {
    const fn new(axis: SignalAxis, detail: &'static str) -> Self {
        Self { axis, detail }
    }
}

/// Notes belonging to one instrument, flattened across all placements that
/// reference its tracks. Pitch is already transposed by the placement's
/// `transpose` field; notes that would fall outside the MIDI range are
/// dropped before being collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteRef {
    pub pitch: u8,
    pub start_tick: u64,
    pub duration_ticks: Option<u64>,
}

// ---------------------------------------------------------------------------
// Axis 1: signals from the instrument and track names.
// ---------------------------------------------------------------------------

/// Lowercase, then split on non-alphanumeric and drop empty fragments. Used
/// by every name-vocabulary check in the analysis module — word-match (not
/// substring) so `"bassoon"` doesn't trigger `"bass"`.
#[must_use]
pub(super) fn tokenize_name(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

fn tokenize_combined_names(instrument_name: &str, track_name: Option<&str>) -> Vec<String> {
    match track_name {
        Some(t) => tokenize_name(&format!("{instrument_name} {t}")),
        None => tokenize_name(instrument_name),
    }
}

/// Tokenize on non-alphanumeric, lowercase, then word-match against a small
/// vocabulary. Returns `None` when no word maps to a role.
#[must_use]
pub fn role_from_name(instrument_name: &str, track_name: Option<&str>) -> Option<Role> {
    let tokens = tokenize_combined_names(instrument_name, track_name);
    let has = |word: &str| tokens.iter().any(|t| t == word);

    // Order matters: Bass needs to be checked after Drums so "bass drum"
    // resolves as Drums, but standalone "bass" still wins later.
    if has("kick")
        || has("bd")
        || has("bassdrum")
        || has("snare")
        || has("sd")
        || has("clap")
        || has("hat")
        || has("hihat")
        || has("hh")
        || has("cymbal")
        || has("ride")
        || has("crash")
        || has("tom")
        || has("perc")
        || has("drum")
        || has("drums")
        || has("impact")
    {
        return Some(Role::Drums);
    }
    if has("bass") || has("sub") || has("808") {
        return Some(Role::Bass);
    }
    if has("lead") || has("solo") || has("brass") || has("arp") || has("supersaw") {
        return Some(Role::Lead);
    }
    if has("pad") || has("string") || has("strings") || has("choir") {
        return Some(Role::Pad);
    }
    if has("pluck")
        || has("harp")
        || has("stab")
        || has("stabs")
        || has("chime")
        || has("chimes")
        || has("bell")
        || has("bells")
    {
        return Some(Role::Pluck);
    }
    if has("keys") || has("piano") || has("epiano") || has("ep") || has("rhodes") || has("organ") {
        return Some(Role::Keys);
    }
    if has("fx")
        || has("riser")
        || has("sweep")
        || has("noise")
        || has("atmo")
        || has("ambience")
        || has("drone")
    {
        return Some(Role::Fx);
    }
    None
}

/// Re-check the patch/track name against the dedicated FX-override vocabulary.
/// Used by the post-cascade override that rescues patches like `"Sweep FX"`
/// from drums mis-classification when the DSP signals look percussive but the
/// name screams FX. Distinct from [`role_from_name`]'s FX bucket because the
/// override vocabulary includes percussive-sounding words (`impact`) that
/// otherwise belong to Drums for non-name-conflict cases.
#[must_use]
fn name_is_explicit_fx(instrument_name: &str, track_name: Option<&str>) -> bool {
    tokenize_combined_names(instrument_name, track_name)
        .iter()
        .any(|t| {
            matches!(
                t.as_str(),
                "fx" | "sweep" | "riser" | "impact" | "atmo" | "ambience" | "drone"
            )
        })
}

// ---------------------------------------------------------------------------
// Axis 2: signals from the voice graph contents.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphSignals {
    pub has_oscillator: bool,
    pub has_noise_source: bool,
    pub has_sampler: bool,
    /// Sampler module whose `PlayMode` parameter is set to `OneShot`. This is
    /// the canonical configuration for sample-based percussion and one-shot FX
    /// hits — a strong percussive signal even when the patch has no envelope.
    pub has_oneshot_sampler: bool,
    pub has_physical: bool,
    pub osc_count: usize,
}

#[must_use]
pub fn graph_signals(modules: &[ModuleStateSnapshot]) -> GraphSignals {
    let mut sig = GraphSignals {
        has_oscillator: false,
        has_noise_source: false,
        has_sampler: false,
        has_oneshot_sampler: false,
        has_physical: false,
        osc_count: 0,
    };
    for m in modules {
        match m.module_type {
            ModuleType::Oscillator
            | ModuleType::MathOscillator
            | ModuleType::SubOscillator
            | ModuleType::WavetableOsc
            | ModuleType::AdditiveOsc
            | ModuleType::FractalOsc
            | ModuleType::GranularOsc
            | ModuleType::LaSynth
            | ModuleType::VectorMixer => {
                sig.has_oscillator = true;
                sig.osc_count += 1;
            }
            ModuleType::Noise | ModuleType::MechanicalNoise => sig.has_noise_source = true,
            ModuleType::Sampler => {
                sig.has_sampler = true;
                for p in &m.parameters {
                    if let Param::Sampler(SamplerParam::PlayMode(SamplerPlayMode::OneShot)) = p {
                        sig.has_oneshot_sampler = true;
                        break;
                    }
                }
            }
            ModuleType::BodyResonance | ModuleType::ModalResonator => sig.has_physical = true,
            _ => {}
        }
    }
    sig
}

// ---------------------------------------------------------------------------
// Axis 3: amp-envelope shape.
// ---------------------------------------------------------------------------

#[must_use]
pub fn envelope_shape(modules: &[ModuleStateSnapshot]) -> EnvelopeShape {
    // V1 heuristic: pick the Envelope module with the smallest instance number
    // (typically "env-1"). Fully tracing env→Amp via the graph is doable but
    // not worth the complexity for the first iteration — 90 % of patches use
    // a single envelope for amplitude.
    let mut envelopes: Vec<&ModuleStateSnapshot> = modules
        .iter()
        .filter(|m| m.module_type == ModuleType::Envelope)
        .collect();
    if envelopes.is_empty() {
        return EnvelopeShape::Unknown;
    }
    envelopes.sort_by_key(|m| m.id.instance);
    let first = envelopes[0];

    let mut attack = 0.0_f32;
    let mut decay = 0.0_f32;
    let mut sustain = 1.0_f32;
    let mut release = 0.0_f32;
    for p in &first.parameters {
        if let Param::Envelope(ep) = p {
            match ep {
                EnvelopeParam::Attack(s) => attack = s.as_f32(),
                EnvelopeParam::Decay(s) => decay = s.as_f32(),
                EnvelopeParam::Sustain(v) => sustain = v.as_f32(),
                EnvelopeParam::Release(s) => release = s.as_f32(),
                _ => {}
            }
        }
    }

    // Order matters — return the first matching bucket.
    if sustain < 0.05 && (decay + release) < 0.2 {
        return EnvelopeShape::Percussive;
    }
    if sustain < 0.3 && release < 0.5 {
        return EnvelopeShape::Plucked;
    }
    if attack > 0.5 {
        return EnvelopeShape::Evolving;
    }
    if sustain >= 0.3 {
        return EnvelopeShape::Sustained;
    }
    EnvelopeShape::Unknown
}

// ---------------------------------------------------------------------------
// Axis 4: stats over the notes the instrument plays.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatternStats {
    pub distinct_pitch_classes: u8,
    pub median_pitch: u8,
    /// max - min, in semitones. Zero when there are fewer than two notes.
    pub pitch_spread: u8,
    pub max_simultaneous: u8,
    pub note_count: usize,
}

impl PatternStats {
    fn empty() -> Self {
        Self {
            distinct_pitch_classes: 0,
            median_pitch: 0,
            pitch_spread: 0,
            max_simultaneous: 0,
            note_count: 0,
        }
    }
}

#[must_use]
pub fn pattern_stats(notes: &[NoteRef]) -> PatternStats {
    if notes.is_empty() {
        return PatternStats::empty();
    }
    let mut pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    pitches.sort_unstable();
    let median = pitches[pitches.len() / 2];
    let spread = pitches[pitches.len() - 1] - pitches[0];

    let mut classes: HashSet<u8> = HashSet::new();
    for p in &pitches {
        classes.insert(p % 12);
    }

    // Sweep-line for max simultaneous: build a (tick, delta) event list and
    // accumulate. Notes without a duration are treated as one-tick blips
    // (consistent with how the harmony analyzer treats them — they don't
    // overlap with anything else by default).
    let mut events: Vec<(u64, i32)> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        let start = n.start_tick;
        let end = match n.duration_ticks {
            Some(d) if d > 0 => start.saturating_add(d),
            _ => start.saturating_add(1),
        };
        events.push((start, 1));
        events.push((end, -1));
    }
    // Sort by tick; for equal ticks, process end (-1) before start (+1) so a
    // note that ends exactly when another starts isn't counted as overlap.
    events.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut current = 0i32;
    let mut peak = 0i32;
    for (_, delta) in events {
        current += delta;
        if current > peak {
            peak = current;
        }
    }

    PatternStats {
        distinct_pitch_classes: classes.len() as u8,
        median_pitch: median,
        pitch_spread: spread,
        max_simultaneous: peak.max(0) as u8,
        note_count: notes.len(),
    }
}

#[must_use]
pub fn pitch_role_from_stats(stats: &PatternStats) -> PitchRole {
    if stats.note_count == 0 {
        return PitchRole::Unused;
    }
    if stats.distinct_pitch_classes <= 2 {
        PitchRole::Atonal
    } else if stats.distinct_pitch_classes >= 5 {
        PitchRole::Tonal
    } else {
        PitchRole::Mixed
    }
}

#[must_use]
pub fn register_from_stats(stats: &PatternStats) -> Register {
    if stats.note_count == 0 {
        return Register::Unused;
    }
    if stats.pitch_spread > 36 {
        return Register::FullRange;
    }
    match stats.median_pitch {
        0..=39 => Register::Sub,
        40..=55 => Register::Bass,
        56..=72 => Register::Mid,
        _ => Register::High,
    }
}

#[must_use]
pub fn texture_from_stats(stats: &PatternStats) -> Texture {
    if stats.note_count == 0 {
        return Texture::Unused;
    }
    match stats.max_simultaneous {
        0 | 1 => Texture::Monophonic,
        2..=4 => Texture::Polyphonic,
        _ => Texture::Chordal,
    }
}

// ---------------------------------------------------------------------------
// Decision tree: combine all signals into a Role + confidence.
// ---------------------------------------------------------------------------

/// Pitch spread (semitones) below which a pitched percussive instrument still
/// counts as drum-like — covers pitch-swept synth kicks, tom-roll variations,
/// and chromatic snare layering.
const DRUM_PITCH_SPREAD_LIMIT: u8 = 5;

/// Relaxed limit applied when the instrument's name says Drums (Tom, Snare,
/// Kick, etc.). Tom patterns commonly tune 2-3 toms across ~8 semitones; a
/// strict 5-st cap would otherwise push those into Pluck-gate.
const DRUM_PITCH_SPREAD_LIMIT_NAMED: u8 = 12;

#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn classify_role(
    name_hint: Option<Role>,
    graph: &GraphSignals,
    envelope: EnvelopeShape,
    pitch_role: PitchRole,
    pitch_spread: u8,
    register: Register,
    texture: Texture,
) -> RoleInference {
    // 1. Drums.
    //
    // Pitch (fix #4): Atonal OR pitch_spread ≤ 5 semitones — covers synth
    // kicks with pitch-swept oscillators and toms that play 1-3 tuned hits.
    // Envelope (fix #3): Percussive, OR Plucked when register is sub/bass —
    // many synth-kicks have plucked-shaped envelopes (slightly longer release
    // than the Percussive threshold).
    // Noise (fix #5): noise-source alone is not enough — long sustained noise
    // is a sweep/pad, not a drum. Require a short envelope alongside.
    let pure_oneshot_sampler =
        graph.has_oneshot_sampler && !graph.has_oscillator && !graph.has_noise_source;
    // §8.5.3: when the name shouts Drums, allow wider pitch spreads so
    // Tom-style patches (1-3 tuned hits across ~8 st) still fire drum-gate.
    let drum_pitch_limit = if name_hint == Some(Role::Drums) {
        DRUM_PITCH_SPREAD_LIMIT_NAMED
    } else {
        DRUM_PITCH_SPREAD_LIMIT
    };
    let drum_pitch_ok = matches!(pitch_role, PitchRole::Atonal) || pitch_spread <= drum_pitch_limit;
    let plucked_in_bass =
        envelope == EnvelopeShape::Plucked && matches!(register, Register::Sub | Register::Bass);
    let drum_envelope_ok = envelope == EnvelopeShape::Percussive || plucked_in_bass;
    let noise_short_envelope = graph.has_noise_source
        && !matches!(envelope, EnvelopeShape::Sustained | EnvelopeShape::Evolving);

    if drum_pitch_ok && (drum_envelope_ok || noise_short_envelope || pure_oneshot_sampler) {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "drums-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Drums) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "drums"));
            bonus += 1;
        }
        if envelope == EnvelopeShape::Percussive {
            signals.push(ProfileSignal::new(SignalAxis::Envelope, "percussive"));
            bonus += 1;
        } else if plucked_in_bass {
            signals.push(ProfileSignal::new(SignalAxis::Envelope, "plucked-bass"));
            bonus += 1;
        }
        if noise_short_envelope && !graph.has_oscillator && !graph.has_sampler {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "noise-no-osc"));
            bonus += 1;
        }
        if pure_oneshot_sampler {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "oneshot-sampler"));
            bonus += 1;
        }
        if !matches!(pitch_role, PitchRole::Atonal) && pitch_spread <= drum_pitch_limit {
            signals.push(ProfileSignal::new(
                SignalAxis::Pattern,
                "narrow-pitch-spread",
            ));
        }
        // §8.5.6.2(c): base 0.65 (not the 0.6 the other gates use) so DSP-
        // driven drum classifications with a `name-conflict` penalty (-0.2)
        // still clear the 0.60 auto-exclude threshold analyze_harmony uses.
        let conf = (0.65_f32 + 0.2_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Drums, conf, signals, name_hint);
    }

    // 2. Pluck (fix #7 + §8.5.6.1 name-priority guard).
    // Plucked envelope + Monophonic is a strong pluck signature regardless of
    // register — fires before Bass so plucks playing in the bass register
    // don't get swallowed by Bass-gate. Exception: when the user named the
    // patch "Lead", defer to Lead-precedence (step 3) — an arpeggiated lead
    // synth has plucked DSP shape but the user's name expresses Lead intent.
    if envelope == EnvelopeShape::Plucked
        && texture == Texture::Monophonic
        && name_hint != Some(Role::Lead)
    {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "pluck-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Pluck) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "pluck"));
            bonus += 1;
        }
        if graph.has_physical {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "physical"));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Pluck, conf, signals, name_hint);
    }

    // 3. Lead-precedence-by-name (fix #6 + §8.5.4).
    // When the user named it Lead, respect that ahead of register-only Bass
    // ("Sub Lead" should still be Lead) and ahead of the relaxed Pad-gate
    // (a "Lead" patch that happens to play 2-4 simultaneous notes should
    // still be a Lead, not a Pad).
    let lead_envelope_ok = matches!(envelope, EnvelopeShape::Plucked | EnvelopeShape::Sustained);
    if name_hint == Some(Role::Lead)
        && lead_envelope_ok
        && matches!(texture, Texture::Monophonic | Texture::Polyphonic)
    {
        let mut signals = vec![
            ProfileSignal::new(SignalAxis::Decision, "lead-gate"),
            ProfileSignal::new(SignalAxis::Name, "lead"),
        ];
        let mut bonus = 1;
        if graph.has_oscillator {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "oscillator"));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Lead, conf, signals, name_hint);
    }

    // 4. Pad-precedence-by-name (§8.5.1).
    // When the user named it Pad/Strings/Choir, respect that ahead of
    // register-only Bass — a Pad that happens to play in the bass register
    // (Fractal Pad, Pad layered with sub, …) should still be a Pad. Shares
    // the gate body with step 6; the only difference is the precondition.
    if name_hint == Some(Role::Pad) && pad_shape_matches(envelope, texture, pitch_role) {
        return build_pad_inference(name_hint, graph, envelope);
    }

    // 5. Bass.
    // §8.5.2: when the user named it Bass and it sits in the sub/bass register,
    // accept Atonal pitch_role too — sub basses that hammer a single note all
    // bar would otherwise leak to FX-gate (Atonal + non-percussive).
    let bass_pitch_ok = matches!(pitch_role, PitchRole::Tonal | PitchRole::Mixed)
        || (name_hint == Some(Role::Bass) && pitch_role == PitchRole::Atonal);
    let bass_gate = bass_pitch_ok
        && matches!(register, Register::Sub | Register::Bass)
        && matches!(texture, Texture::Monophonic | Texture::Polyphonic);
    if bass_gate {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "bass-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Bass) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "bass"));
            bonus += 1;
        }
        if graph.has_oscillator {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "oscillator"));
            bonus += 1;
        }
        if matches!(envelope, EnvelopeShape::Sustained | EnvelopeShape::Plucked) {
            signals.push(ProfileSignal::new(
                SignalAxis::Envelope,
                "sustained-or-plucked",
            ));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Bass, conf, signals, name_hint);
    }

    // 6. Pad (fix #1 — relaxed).
    // Real pad patches typically have Polyphonic (2-4) texture and Sustained
    // envelope, not the stricter Chordal (≥5) + Evolving the old gate required.
    // Mixed pitch is still musical content (modal/quartal pads etc.).
    if pad_shape_matches(envelope, texture, pitch_role) {
        return build_pad_inference(name_hint, graph, envelope);
    }

    // 7. Keys.
    if envelope == EnvelopeShape::Plucked
        && matches!(texture, Texture::Polyphonic | Texture::Chordal)
        && pitch_role == PitchRole::Tonal
    {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "keys-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Keys) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "keys"));
            bonus += 1;
        }
        if graph.has_oscillator {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "oscillator"));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Keys, conf, signals, name_hint);
    }

    // 8. Lead (default — Mid/High register).
    // Requires Tonal/Mixed pitch: atonal monophonic signals are sweeps/FX,
    // not melodic leads — the lead-precedence-by-name gate (step 3) catches
    // user-named exceptions before this point.
    if lead_envelope_ok
        && texture == Texture::Monophonic
        && matches!(register, Register::Mid | Register::High)
        && matches!(pitch_role, PitchRole::Tonal | PitchRole::Mixed)
    {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "lead-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Lead) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "lead"));
            bonus += 1;
        }
        if graph.has_oscillator {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "oscillator"));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Lead, conf, signals, name_hint);
    }

    // 9. FX.
    if pitch_role == PitchRole::Atonal && envelope != EnvelopeShape::Percussive {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "fx-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Fx) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "fx"));
            bonus += 1;
        }
        let conf = (0.5_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Fx, conf, signals, name_hint);
    }

    // 10. Envelope-Unknown fallback (fix #2b).
    // When envelope_shape is Unknown, every primary gate fails because they
    // all key on a specific shape. Without this, common patches that lack a
    // single dominant Envelope module (modulation envs only, multiple
    // envelopes confusing the heuristic) become silent Unknowns. Soft-classify
    // from pitch+texture+name with lower confidence than the primary gates.
    if envelope == EnvelopeShape::Unknown && pitch_role != PitchRole::Unused {
        let fallback_role = if let Some(hint) = name_hint {
            hint
        } else if matches!(pitch_role, PitchRole::Tonal | PitchRole::Mixed) {
            if matches!(texture, Texture::Polyphonic | Texture::Chordal) {
                Role::Pad
            } else {
                Role::Lead
            }
        } else {
            Role::Fx
        };
        let signals = vec![ProfileSignal::new(
            SignalAxis::Decision,
            "envelope-unknown-fallback",
        )];
        return apply_name_override(fallback_role, 0.4_f32, signals, name_hint);
    }

    // 11. Unknown.
    RoleInference {
        role: Role::Unknown,
        confidence: 0.0,
        signals: vec![ProfileSignal::new(SignalAxis::Decision, "unknown")],
    }
}

/// If the name vocabulary points at the same role as the decision tree, cap
/// confidence to at least 0.85 (the agreement is itself strong evidence).
/// If it points at a different role, keep the decision-tree role but subtract
/// 0.2 from confidence and record a `name-conflict` signal.
fn apply_name_override(
    role: Role,
    base_confidence: f32,
    mut signals: Vec<ProfileSignal>,
    name_hint: Option<Role>,
) -> RoleInference {
    let confidence = match name_hint {
        Some(hint) if hint == role => base_confidence.clamp(0.85, 1.0),
        Some(_) => {
            signals.push(ProfileSignal::new(SignalAxis::Name, "name-conflict"));
            (base_confidence - 0.2).max(0.0)
        }
        None => base_confidence,
    };
    RoleInference {
        role,
        confidence,
        signals,
    }
}

/// Shared shape predicate for the Pad-precedence-by-name (step 4) and the
/// default Pad-gate (step 6). No register check — the differentiator between
/// the two call sites is the name precondition + cascade position.
fn pad_shape_matches(envelope: EnvelopeShape, texture: Texture, pitch_role: PitchRole) -> bool {
    matches!(envelope, EnvelopeShape::Sustained | EnvelopeShape::Evolving)
        && matches!(texture, Texture::Polyphonic | Texture::Chordal)
        && matches!(pitch_role, PitchRole::Tonal | PitchRole::Mixed)
}

/// Build the `RoleInference` body shared by both Pad cascade steps. The
/// `Name("pad")` signal is pushed when `name_hint == Some(Pad)` — already true
/// at step 4, may or may not hold at step 6.
fn build_pad_inference(
    name_hint: Option<Role>,
    graph: &GraphSignals,
    envelope: EnvelopeShape,
) -> RoleInference {
    let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "pad-gate")];
    let mut bonus = 0;
    if name_hint == Some(Role::Pad) {
        signals.push(ProfileSignal::new(SignalAxis::Name, "pad"));
        bonus += 1;
    }
    if graph.osc_count >= 2 {
        signals.push(ProfileSignal::new(SignalAxis::Graph, "thick"));
        bonus += 1;
    }
    if envelope == EnvelopeShape::Evolving {
        signals.push(ProfileSignal::new(SignalAxis::Envelope, "evolving"));
        bonus += 1;
    }
    let conf = (0.6_f32 + 0.15_f32 * bonus as f32).min(1.0);
    apply_name_override(Role::Pad, conf, signals, name_hint)
}

// ---------------------------------------------------------------------------
// Top-level inference.
// ---------------------------------------------------------------------------

/// Build a profile for a single instrument from precomputed inputs. Pure —
/// no audio, no locks, no engine access. Tests reach for this directly.
///
/// `tracks_assigned` are tracks whose `instrument` field equals
/// `snapshot.seq_instrument_id`; the first one's name (if any) becomes the
/// secondary input to the name vocabulary.
#[must_use]
pub fn infer_instrument_profile(
    snapshot: &InstrumentSnapshot,
    modules: &[ModuleStateSnapshot],
    tracks_assigned: &[&SequencerTrack],
    notes: &[NoteRef],
) -> InstrumentProfile {
    // Manual category wins. The role stays user-controlled when set.
    if snapshot.category != InstrumentCategory::Uncategorized {
        let role = map_category_to_role(snapshot.category);
        let stats = pattern_stats(notes);
        return InstrumentProfile {
            instrument_id: snapshot.seq_instrument_id,
            instrument_name: snapshot.name.clone(),
            role: RoleInference {
                role,
                confidence: 1.0,
                signals: vec![ProfileSignal::new(SignalAxis::Manual, "manual-override")],
            },
            envelope_shape: envelope_shape(modules),
            pitch_role: pitch_role_from_stats(&stats),
            register: register_from_stats(&stats),
            texture: texture_from_stats(&stats),
        };
    }

    let track_name = tracks_assigned.first().map(|t| t.name.as_str());
    let name_hint = role_from_name(&snapshot.name, track_name);
    let graph = graph_signals(modules);
    let envelope = envelope_shape(modules);
    let stats = pattern_stats(notes);
    let pitch_role = pitch_role_from_stats(&stats);
    let register = register_from_stats(&stats);
    let texture = texture_from_stats(&stats);
    let role = classify_role(
        name_hint,
        &graph,
        envelope,
        pitch_role,
        stats.pitch_spread,
        register,
        texture,
    );

    // Rescue patches whose name explicitly says FX from a confident non-FX
    // classification. Fires only when the cascade already disagreed with the
    // name (`name-conflict` signal present) — patches whose decision tree
    // agreed with the name keep their original confidence path. Confidence
    // drops to 0.85× so the override is reflected in the score.
    let name_conflict_fired = role.signals.iter().any(|s| s.detail == "name-conflict");
    let role = if role.role != Role::Fx
        && name_conflict_fired
        && name_is_explicit_fx(&snapshot.name, track_name)
    {
        let mut signals = role.signals;
        signals.push(ProfileSignal::new(SignalAxis::Name, "explicit-fx-override"));
        RoleInference {
            role: Role::Fx,
            confidence: (role.confidence * 0.85).clamp(0.0, 1.0),
            signals,
        }
    } else {
        role
    };

    InstrumentProfile {
        instrument_id: snapshot.seq_instrument_id,
        instrument_name: snapshot.name.clone(),
        role,
        envelope_shape: envelope,
        pitch_role,
        register,
        texture,
    }
}

fn map_category_to_role(cat: InstrumentCategory) -> Role {
    match cat {
        InstrumentCategory::Drums => Role::Drums,
        InstrumentCategory::Bass => Role::Bass,
        InstrumentCategory::Lead => Role::Lead,
        InstrumentCategory::Pad => Role::Pad,
        InstrumentCategory::Keys => Role::Keys,
        InstrumentCategory::FX => Role::Fx,
        InstrumentCategory::Arp => Role::Lead,
        InstrumentCategory::Uncategorized => Role::Unknown,
    }
}

/// Profile every instrument that at least one of the song's tracks routes
/// to. Instruments with no track assignment are skipped — they can't
/// contribute to any analysis that walks `song.tracks()`, and skipping them
/// also avoids one `shared_graph` lock acquisition per orphan instrument.
///
/// Acquires the `instrument_snapshots` read-lock once and the shared graph's
/// internal lock once per profiled instrument. Does not block on the audio
/// thread.
#[must_use]
pub fn infer_all_profiles(song: &Song, engine_state: &EngineState) -> Vec<InstrumentProfile> {
    let referenced: HashSet<SeqInstrumentId> = song.tracks().map(|t| t.instrument).collect();
    if referenced.is_empty() {
        return Vec::new();
    }

    let snapshots = engine_state.instrument_snapshots.read();
    let mut profiles: Vec<InstrumentProfile> = Vec::with_capacity(referenced.len());

    for snapshot in snapshots.iter() {
        let seq_id = SeqInstrumentId(snapshot.seq_instrument_id);
        if !referenced.contains(&seq_id) {
            continue;
        }
        let modules = engine_state
            .shared_graph
            .get_modules_for_instrument(snapshot.id);
        let tracks: Vec<&SequencerTrack> =
            song.tracks().filter(|t| t.instrument == seq_id).collect();
        let notes = collect_notes_for_instrument(song, &tracks);
        profiles.push(infer_instrument_profile(
            snapshot, &modules, &tracks, &notes,
        ));
    }
    profiles
}

/// Flatten every placement that lives on one of the instrument's tracks into
/// a list of [`NoteRef`]s with absolute song-tick start times and the
/// placement's transpose applied. Notes whose transpose pushes them outside
/// the MIDI range are silently dropped.
///
/// Note routing follows track assignment — every note in a placement on a
/// track plays through that track's instrument. `Note.instrument` is left
/// alone by most editing paths (it gets a default value), so we deliberately
/// don't filter on it.
fn collect_notes_for_instrument(song: &Song, tracks: &[&SequencerTrack]) -> Vec<NoteRef> {
    let mut notes: Vec<NoteRef> = Vec::new();
    for track in tracks {
        for placement in song.placements_on_track(track.id) {
            let Some(pattern) = song.pattern(placement.pattern_id) else {
                continue;
            };
            for note in pattern.notes() {
                let Some(transposed) = note.pitch.transpose(placement.transpose) else {
                    continue;
                };
                let abs_start = placement.start.0.saturating_add(u64::from(note.start.0));
                let dur = note.duration.map(|d| u64::from(d.0));
                notes.push(NoteRef {
                    pitch: transposed.as_midi(),
                    start_tick: abs_start,
                    duration_ticks: dur,
                });
            }
        }
    }
    notes
}
