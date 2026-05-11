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
use synth_core::{ModuleType, Param};
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
    FX,
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

/// Tokenize on non-alphanumeric, lowercase, then word-match against a small
/// vocabulary. Returns `None` when no word maps to a role.
///
/// Word-match (not substring) so "bassoon" doesn't trigger "bass".
#[must_use]
pub fn role_from_name(instrument_name: &str, track_name: Option<&str>) -> Option<Role> {
    let mut combined = instrument_name.to_string();
    if let Some(t) = track_name {
        combined.push(' ');
        combined.push_str(t);
    }
    let lowered = combined.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let has = |word: &str| tokens.contains(&word);

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
    {
        return Some(Role::Drums);
    }
    if has("bass") || has("sub") || has("808") {
        return Some(Role::Bass);
    }
    if has("lead") || has("solo") {
        return Some(Role::Lead);
    }
    if has("pad") || has("string") || has("strings") || has("choir") {
        return Some(Role::Pad);
    }
    if has("pluck") || has("harp") {
        return Some(Role::Pluck);
    }
    if has("keys") || has("piano") || has("epiano") || has("ep") || has("rhodes") || has("organ") {
        return Some(Role::Keys);
    }
    if has("fx") || has("riser") || has("impact") || has("sweep") || has("noise") {
        return Some(Role::FX);
    }
    None
}

// ---------------------------------------------------------------------------
// Axis 2: signals from the voice graph contents.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphSignals {
    pub has_oscillator: bool,
    pub has_noise_source: bool,
    pub has_sampler: bool,
    pub has_physical: bool,
    pub osc_count: usize,
}

#[must_use]
pub fn graph_signals(modules: &[ModuleStateSnapshot]) -> GraphSignals {
    let mut sig = GraphSignals {
        has_oscillator: false,
        has_noise_source: false,
        has_sampler: false,
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
            ModuleType::Sampler => sig.has_sampler = true,
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

#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn classify_role(
    name_hint: Option<Role>,
    graph: &GraphSignals,
    envelope: EnvelopeShape,
    pitch_role: PitchRole,
    register: Register,
    texture: Texture,
) -> RoleInference {
    // First matching gate wins. Bonus signals are counted on top of the
    // gate so a confident classification can reach 1.0.

    // 1. Drums.
    let drum_gate = pitch_role == PitchRole::Atonal
        && (envelope == EnvelopeShape::Percussive || graph.has_noise_source);
    if drum_gate {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "drums-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::Drums) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "drums"));
            bonus += 1;
        }
        if envelope == EnvelopeShape::Percussive {
            signals.push(ProfileSignal::new(SignalAxis::Envelope, "percussive"));
            bonus += 1;
        }
        if graph.has_noise_source && !graph.has_oscillator && !graph.has_sampler {
            signals.push(ProfileSignal::new(SignalAxis::Graph, "noise-no-osc"));
            bonus += 1;
        }
        let conf = (0.6_f32 + 0.2_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Drums, conf, signals, name_hint);
    }

    // 2. Bass.
    let bass_gate = matches!(pitch_role, PitchRole::Tonal | PitchRole::Mixed)
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

    // 3. Pad.
    if envelope == EnvelopeShape::Evolving && texture == Texture::Chordal {
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
        let conf = (0.7_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::Pad, conf, signals, name_hint);
    }

    // 4. Keys.
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

    // 5. Pluck.
    if envelope == EnvelopeShape::Plucked && texture == Texture::Monophonic {
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

    // 6. Lead.
    if matches!(envelope, EnvelopeShape::Plucked | EnvelopeShape::Sustained)
        && texture == Texture::Monophonic
        && matches!(register, Register::Mid | Register::High)
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

    // 7. FX.
    if pitch_role == PitchRole::Atonal && envelope != EnvelopeShape::Percussive {
        let mut signals = vec![ProfileSignal::new(SignalAxis::Decision, "fx-gate")];
        let mut bonus = 0;
        if name_hint == Some(Role::FX) {
            signals.push(ProfileSignal::new(SignalAxis::Name, "fx"));
            bonus += 1;
        }
        let conf = (0.5_f32 + 0.15_f32 * bonus as f32).min(1.0);
        return apply_name_override(Role::FX, conf, signals, name_hint);
    }

    // 8. Unknown.
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
    let role = classify_role(name_hint, &graph, envelope, pitch_role, register, texture);

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
        InstrumentCategory::FX => Role::FX,
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
    let referenced: HashSet<SeqInstrumentId> = song.tracks().filter_map(|t| t.instrument).collect();
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
        let tracks: Vec<&SequencerTrack> = song
            .tracks()
            .filter(|t| t.instrument == Some(seq_id))
            .collect();
        let notes = collect_notes_for_instrument(song, seq_id, &tracks);
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
fn collect_notes_for_instrument(
    song: &Song,
    seq_id: SeqInstrumentId,
    tracks: &[&SequencerTrack],
) -> Vec<NoteRef> {
    let mut notes: Vec<NoteRef> = Vec::new();
    for track in tracks {
        for placement in song.placements_on_track(track.id) {
            let Some(pattern) = song.pattern(placement.pattern_id) else {
                continue;
            };
            for note in pattern.notes() {
                // The pattern may contain notes for other instruments; only
                // pick up notes routed to this instrument.
                if note.instrument != seq_id {
                    continue;
                }
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
