# Instrument Profile — Implementation Plan (§8.2b)

> **Date:** 2026-05-11
> **Status:** Planerad. Konkretiserar `docs/mcp-music-tools-plan.md` §8.2b.
> **Scope:** Auto-inferera instrumentens karaktär längs flera oberoende axlar; använd det för
> att fixa `analyze_harmony`s tysta no-op när inga instrument har `category == Drums` manuellt
> satt, och bygg samtidigt återanvändbar infrastruktur för framtida verktyg.

---

## Översikt och designprinciper

**Vi bygger:** en `InstrumentProfile` med oberoende axlar plus en `infer_instrument_profile`-funktion
som härleder den från tre datakällor som redan finns: `InstrumentSnapshot` (engine),
`SequencerTrack` (sequencer), `Pattern`-noter (sequencer).

**Återanvändbarhet bygger på tre val:**

1. **Inferensen är ren** — ingen audio-render, ingen lock-hållning, ingen sidoeffekt. Tar tre
   referenser, returnerar en struct. Lätt att unit-testa, lätt att anropa från valfri tråd.
2. **Axlarna är oberoende** — `role` är inte sanningskällan. Varje konsument kan välja vilken
   axel som är relevant (`register == Bass`, `envelope_shape == Percussive`, etc.). Detta gör
   profilen användbar för verktyg vi inte har designat än.
3. **MCP-ytan minimeras i v1** — endast det `analyze_harmony` behöver exponeras nu. Den
   fristående `get_instrument_profile`-resursen läggs till när första externa konsument finns.

**Vad det inte är:** ingen ersättning för manuell `set_instrument_category` (som blir kvar som
override-mekanism), inget realtidsverktyg, ingen rendering av audio.

---

## Steg 1 — Typer i ny modul `crates/pertylizer/src/analysis/`

Skapa katalogen `crates/pertylizer/src/analysis/` med två filer:

- **`mod.rs`** — `pub mod instrument_profile;` plus `pub use instrument_profile::*;`
- **`instrument_profile.rs`** — innehåller typerna nedan. Lägg in modulen i
  `crates/pertylizer/src/lib.rs` med `pub mod analysis;`.

Typerna (alla `#[derive(Debug, Clone, PartialEq, Serialize)]`, serde med
`rename_all = "snake_case"`):

```rust
pub struct InstrumentProfile {
    pub instrument_id: u16,            // seq_instrument_id, matchar MCP-ytan
    pub instrument_name: String,
    pub role: RoleInference,
    pub envelope_shape: EnvelopeShape,
    pub pitch_role: PitchRole,
    pub register: Register,
    pub texture: Texture,
}

pub struct RoleInference {
    pub role: Role,
    pub confidence: f32,               // 0.0..=1.0
    pub signals: Vec<ProfileSignal>,   // varför vi gissade så
}

pub enum Role {
    Drums, Bass, Lead, Pad, Pluck, Keys, FX, Unknown,
}

pub enum EnvelopeShape {
    Percussive,    // sustain≈0, total<200ms
    Plucked,       // sustain<0.3, release<500ms
    Sustained,     // sustain≥0.3
    Evolving,      // attack>500ms (klassisk pad)
    Unknown,       // ingen Amp-envelope hittad
}

pub enum PitchRole {
    Tonal,         // ≥5 distinkta pitch-klasser i använda pattern
    Atonal,        // ≤2 distinkta pitch-klasser
    Mixed,         // däremellan
    Unused,        // inga noter spelar instrumentet
}

pub enum Register {
    Sub,           // median MIDI < 40
    Bass,          // 40..=55
    Mid,           // 56..=72
    High,          // > 72
    FullRange,     // spridning > 36 halvtoner
    Unused,
}

pub enum Texture {
    Monophonic,    // max 1 not samtidigt över alla pattern
    Polyphonic,    // max 2-4
    Chordal,       // max ≥5
    Unused,
}

pub struct ProfileSignal {
    pub axis: &'static str,       // "name" | "graph" | "envelope" | "pattern"
    pub detail: &'static str,     // "kick" | "noise-no-osc" | "short-percussive" | etc.
}
```

**Varför `instrument_id: u16`:** matchar `seq_instrument_id`-fältet som `TrackContribution` och
liknande MCP-typer redan använder. Inga nya newtypes behöver korsa MCP-gränsen.

**Varför `signals: Vec<ProfileSignal>` med `&'static str`:** användaren får se *varför* vi
klassade ett spår som trummor. `&'static str` undviker alloc per signal — vi listar bara från
en fast vokabulär. Vi kan dyka ner i konkreta värden (`"name: 'kick' matched"`) i en senare
iteration.

**Checkpoint efter steg 1:** `cargo build` ska gå igenom. Inga konsumenter än.

---

## Steg 2 — Inferenslogik per axel

Lägg till i `instrument_profile.rs` (eller dela upp i underfiler om det blir > 400 rader):

### 2a. Namnsignaler (gratis)

```rust
fn role_from_name(instrument_name: &str, track_name: Option<&str>) -> Option<Role>;
```

Kombinerade namn → lowercase → tokenize på icke-alfanumeriskt. Ord-match (inte substring) mot:

| Match | Role |
|-------|------|
| `kick`, `bd`, `bassdrum` | Drums |
| `snare`, `sd`, `clap` | Drums |
| `hat`, `hihat`, `hh`, `cymbal`, `ride`, `crash` | Drums |
| `tom`, `perc`, `drum`, `drums` | Drums |
| `bass`, `sub`, `808` | Bass |
| `lead`, `solo` | Lead |
| `pad`, `string`, `strings`, `choir` | Pad |
| `pluck`, `harp` | Pluck |
| `keys`, `piano`, `epiano`, `ep`, `rhodes`, `organ` | Keys |
| `fx`, `riser`, `impact`, `sweep`, `noise` | FX |

Substring `"bass"` inuti `"bassoon"` undviks genom token-match. `808` är specialcase —
kontexten "808 bass" får båda matchningarna och slutar som Bass.

### 2b. Grafsignaler (gratis)

```rust
fn graph_signals(modules: &[ModuleStateSnapshot]) -> GraphSignals;

struct GraphSignals {
    has_oscillator: bool,    // Oscillator | MathOscillator | SubOscillator | WavetableOsc
                             // | AdditiveOsc | FractalOsc | GranularOsc | LaSynth | VectorMixer
    has_noise_source: bool,  // Noise | MechanicalNoise | (MathOscillator i noise-läge — skippa i v1)
    has_sampler: bool,       // Sampler
    has_physical: bool,      // BodyResonance | ModalResonator
    osc_count: usize,
}
```

Reglerna som matar in i role-klassningen:

- `has_noise_source && !has_oscillator && !has_sampler` → starkt trumma/FX-signal
- `osc_count >= 2` → "thick" — boost för Bass/Lead/Pad
- `has_physical && !has_oscillator` → Pluck-signal

`ModuleStateSnapshot` hämtas via `engine_state.shared_graph.get_modules_for_instrument(instrument_id)`
(redan en publik metod på `crates/synth_engine/src/shared_state.rs:472`).

### 2c. Envelop-form (gratis)

```rust
fn envelope_shape(modules: &[ModuleStateSnapshot]) -> EnvelopeShape;
```

Algoritm: hitta första modul med `module_type == ModuleType::Envelope` *vars output ansluter
(direkt eller via ModMatrix) till Amplifier*. För v1 räcker enklare heuristik: ta den envelope
med lägst `ModuleId.instance` (typiskt "env-1"). Läs ut ADSR från `parameters: Vec<Param>`
(matcha `Param::Envelope(EnvelopeParam::Attack/Decay/Sustain/Release)`).

| Villkor | Shape |
|---------|-------|
| `sustain < 0.05 && (decay + release) < 0.2s` | Percussive |
| `sustain < 0.3 && release < 0.5s` | Plucked |
| `attack > 0.5s` | Evolving |
| `sustain >= 0.3` | Sustained |
| Ingen envelope hittad | Unknown |

**Varför enklast först:** att fullt traversera grafen för att verifiera env→Amp-anslutningen är
möjligt men inte värt komplexiteten i v1. 90 % av patcher har en huvud-envelope som "env-1".

### 2d. Pattern-stats (gratis)

```rust
fn pattern_stats(notes: &[NoteRef]) -> PatternStats;

struct NoteRef<'a> {
    pitch: u8,                   // MIDI 0-127
    start_tick: u64,
    duration_ticks: Option<u64>,
}

struct PatternStats {
    distinct_pitch_classes: u8,  // 0..=12
    median_pitch: u8,
    pitch_spread: u8,            // max - min
    max_simultaneous: u8,
    max_duration_beats: f32,
    note_count: usize,
}
```

`distinct_pitch_classes` → `PitchRole` enligt tröskelvärdena ovan. `median_pitch` → `Register`.
`max_simultaneous` → `Texture` (samtidighet beräknas över alla noter sorterade på `start_tick`
med sweep-line, O(n log n)).

**Hur vi får noterna:** för varje track med `instrument == seq_id`, iterera alla placements via
`song.placements_in_range` eller direkt över `song.placements()`, hitta `Pattern`, iterera
`pattern.notes()`. Det är samma mönster som `analyze_song_harmony` redan använder.

### 2e. Sammanvägning till `Role` och `confidence`

```rust
fn classify_role(
    name_hint: Option<Role>,
    graph: &GraphSignals,
    envelope: EnvelopeShape,
    pitch_role: PitchRole,
    register: Register,
    texture: Texture,
) -> RoleInference;
```

Beslutsträd (returnera först som matchar):

1. **Drums:** `pitch_role == Atonal && (envelope == Percussive || graph.has_noise_source)`.
   Confidence: 0.6 bas + 0.2 per ytterligare matchande signal (namn, graf, envelope). Max 1.0.
2. **Bass:** `pitch_role in [Tonal, Mixed] && register in [Sub, Bass]
   && texture in [Monophonic, Polyphonic]`.
   Confidence: 0.6 + 0.15 per matchande signal.
3. **Pad:** `envelope == Evolving && texture == Chordal`. Confidence: 0.7 + 0.15 per matchning.
4. **Keys:** `envelope == Plucked && texture in [Polyphonic, Chordal] && pitch_role == Tonal`.
   Confidence: 0.6 + 0.15.
5. **Pluck:** `envelope == Plucked && texture == Monophonic`. Confidence: 0.6 + 0.15.
6. **Lead:** `envelope in [Plucked, Sustained] && texture == Monophonic
   && register in [Mid, High]`.
   Confidence: 0.6 + 0.15.
7. **FX:** `pitch_role == Atonal && envelope != Percussive`. Confidence: 0.5 + 0.15.
8. **Unknown:** annars, confidence 0.0.

**Namn-override-regel:** om `name_hint` finns och pekar på *samma* role som beslutsträdet →
confidence kappas till `max(0.85, beräknad)`. Om `name_hint` pekar på *annan* role → behåll
beslutsträdets role men sänk confidence med 0.2 (signalerar konflikt) och lägg signalen
`"name-conflict"`. Detta undviker att en felnamngiven patch tar över när grafen + mönstret säger
något annat.

**Tröskel för auto-exklusion (i §8.2-fixen):** `role == Drums && confidence >= 0.6`. Justeras
efter testlåtsdata.

**Checkpoint efter steg 2:** alla axel-funktioner har unit-tester med fixturer (se steg 5).
`cargo test -p pertylizer instrument_profile` går igenom.

---

## Steg 3 — Toppnivåfunktion `infer_instrument_profile`

I `instrument_profile.rs`:

```rust
pub fn infer_instrument_profile(
    snapshot: &InstrumentSnapshot,
    modules: &[ModuleStateSnapshot],
    tracks_assigned: &[&SequencerTrack],  // tracks vars instrument == snapshot.seq_id
    notes: &[NoteRef],                    // alla noter dessa tracks spelar (transponerade)
) -> InstrumentProfile;
```

Plus en hjälp-funktion som gör hela jobbet från en `&Song` + `&McpSharedState`:

```rust
pub fn infer_all_profiles(
    song: &Song,
    engine_state: &SharedEngineState,
) -> Vec<InstrumentProfile>;
```

Den senare är vad MCP-bridgen kommer kalla. Den löser upp snapshot→track-noter-relationen en
gång och anropar `infer_instrument_profile` per instrument. Båda i samma modul, men endast
`infer_all_profiles` är `pub` ut ur `pertylizer`-crate; per-axel-funktionerna är `pub(crate)`
så testerna kommer åt dem men inte externa konsumenter.

**Manuell `category` har företräde:** om `snapshot.category != Uncategorized`, returnera den som
`role` med confidence 1.0 och signal `"manual-override"`. Inferensen sker bara för okategoriserade
instrument. Detta bevarar användarens kontroll och gör att existerande `set_instrument_category`-
anrop fortsätter fungera.

**Checkpoint efter steg 3:** integration-test med en mock-`Song` + fyra instrument
(kick/bass/pad/uncategorized) verifierar att toppnivåfunktionen returnerar rätt profiler.

---

## Steg 4 — Integrera i `analyze_song_harmony`

I `crates/pertylizer/src/mcp_bridge.rs`, ersätt block på rad 4615–4631 (drum_track_ids-loopen):

```rust
let drum_track_ids: HashSet<TrackId> = if exclude_drums {
    let profiles = infer_all_profiles(&song, &session.state());
    let drum_seq_ids: HashSet<SeqInstrumentId> = profiles
        .iter()
        .filter(|p| p.role.role == Role::Drums && p.role.confidence >= 0.6)
        .map(|p| SeqInstrumentId(p.instrument_id))
        .collect();
    song.tracks()
        .filter_map(|t| t.instrument.and_then(|s| drum_seq_ids.contains(&s).then_some(t.id)))
        .collect()
} else { HashSet::new() };
```

Och utöka warning-texten (rad 4637–4648): inkludera *vilken signal* som triggade — exempelvis
`"Auto-excluded 3 track(s) as drums: Kick(0) [name:kick, graph:noise-no-osc, envelope:percussive], Snare(1) [name:snare, ...], Hat(2) [name:hat, ...]"`.
Användaren ser då både att filtret fungerade och *varför*.

**Behåll `exclude_drums` som bool-flagga** — semantiken är nu "auto-detektera och exkludera".
Manuell kategorisering fortsätter ha företräde (steg 3).

**Checkpoint efter steg 4:** `cargo test -p pertylizer analyze_harmony_default_excludes_drum_tracks`
ska fortsätta passera (den använder manuellt satta kategorier — träffar fortfarande
`manual-override`-grenen). Lägg till en ny test:
`analyze_harmony_default_excludes_inferred_drum_tracks` som skapar drum-tracks utan att sätta
`category` och verifierar att filtret ändå fungerar. Detta är *exakt* §8.2-buggen.

---

## Steg 5 — Testfixturer och testtäckning

Skapa `crates/pertylizer/tests/instrument_profile.rs`:

**Fixture-helpers:**

- `fn kick_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>)` — Noise → Env(perc) → Amp, namn "Kick".
- `fn bass_patch() -> ...` — två Oscillators → Filter → Env(sust) → Amp, namn "Sub Bass", register Bass.
- `fn pad_patch() -> ...` — Oscillator → Filter → Env(slow attack) → Amp, namn "Strings".
- `fn pluck_patch() -> ...` — Oscillator → Env(plucked) → Amp.
- `fn unnamed_kick_patch() -> ...` — som kick men namn "Track 5" (testar att graf+envelope ensamt räcker).

**Per-axel-tester (10–12 st):**

- `envelope_shape_classifies_percussive`, `_plucked`, `_sustained`, `_evolving`, `_unknown_when_no_envelope`
- `pitch_role_atonal_for_single_pitch_class`, `_tonal_for_diatonic_use`, `_unused_for_empty`
- `register_sub_for_low_median`, `_full_range_when_spread_large`
- `texture_monophonic_when_serial`, `_chordal_when_5plus`
- `graph_signals_detects_noise_only`, `_detects_dual_oscillator`

**Beslutsträd-tester (8 st):**

- `kick_classified_as_drums_via_name`
- `unnamed_kick_classified_as_drums_via_graph_and_envelope` ← *den viktiga*
- `bass_classified_as_bass`
- `pad_classified_as_pad`
- `pluck_classified_as_pluck`
- `lead_classified_as_lead`
- `manual_category_overrides_inference`
- `name_conflict_lowers_confidence`

**Integration-test (i `analyze_tier1_follow_ups.rs`):**

- `analyze_harmony_excludes_uncategorized_inferred_drums` — bygger en mini-song med en
  pad-instrument + en namnlös kick (Noise+Env), inga manuella kategorier satta. Asserterar att
  `F#m7b5`-buggens ekvivalent inte uppstår.

**Checkpoint efter steg 5:** `cargo test -p pertylizer` 100 % grön.

---

## Steg 6 — MCP-yta v1 (minimal)

**Inget nytt MCP-tool i v1.** Det enda som ändras externt är att `analyze_harmony` med
default-parametrar nu *fungerar* på okategoriserade trummor, och att warning-texten är rikare.

**Reserverat för framtida commit (v2):** ny MCP-resurs `get_instrument_profiles` som returnerar
`Vec<InstrumentProfileResult>` (serialiserbart spegelobjekt av `InstrumentProfile`). Skissa
filerna men implementera *inte* förrän en konsument finns:

- `crates/synth_mcp/src/types.rs` — lägg till `InstrumentProfileResult` (samma fält,
  `Serialize`-bara).
- `crates/synth_mcp/src/bridge.rs` + `server.rs` — `get_instrument_profiles()`-metod.

**Varför vänta:** vi vet inte vilken JSON-form externa konsumenter vill ha förrän nästa
Tier-1-verktyg (`analyze_pattern`, `analyze_groove`) kräver profilen. Frysa en MCP-form i
förskott är dyrare än att lägga till en när behovet uppstår.

---

## Steg 7 — Live-verifiering + dokumentation

1. Kör hela suiten mot "Tung Synthpop"-låten manuellt via MCP-bridgen. Förvänta:
   `analyze_harmony` på arrangemanget med default-parametrar exkluderar Kick/Snare/Hat *utan*
   manuell kategorisering, och `F#m7b5`-felet från §6 är borta.
2. Uppdatera `docs/mcp-music-tools-plan.md`:
   - Markera §8.2b som shippad.
   - Lägg in resultat under §6 punkt 5 (`F#m7b5`-buggen): "Helt fixat i v0.278.0 — auto-inferens,
     manuell `set_instrument_category` ej längre nödvändig."
   - Lägg till `analysis::instrument_profile` som åtagande under §7 ("kärna är på plats,
     framtida MCP-yta öppen").

---

## Filsumma

| Fil | Ändring |
|-----|---------|
| `crates/pertylizer/src/analysis/mod.rs` | **Ny.** Modul-deklaration. |
| `crates/pertylizer/src/analysis/instrument_profile.rs` | **Ny.** ~400 rader: typer, per-axel-funktioner, `infer_instrument_profile`, `infer_all_profiles`. |
| `crates/pertylizer/src/lib.rs` | `pub mod analysis;` |
| `crates/pertylizer/src/mcp_bridge.rs` | ~20-radersändring runt rad 4615–4648: byt manuell-kategori-loopen mot `infer_all_profiles` + utökad warning-text. |
| `crates/pertylizer/tests/instrument_profile.rs` | **Ny.** ~600 rader fixturer + per-axel-tester + beslutsträd-tester. |
| `crates/pertylizer/tests/analyze_tier1_follow_ups.rs` | Lägg till `analyze_harmony_excludes_uncategorized_inferred_drums`. |
| `docs/mcp-music-tools-plan.md` | Uppdatera §6 punkt 5 + §8.2 status. |

---

## Ordningsföljd för commits

1. **Commit 1:** Steg 1 + 2 + 3 + 5 (typer, inferens, alla unit-tester). Inga konsumenter —
   fristående, körbart.
2. **Commit 2:** Steg 4 (integrera i `analyze_song_harmony`) + integrationstest.
3. **Commit 3:** Steg 7 (live-verifiering + docs-uppdatering).

Tre tydliga, var och en byggbara och testbara. Om commit 2 introducerar regression kan den
revertas utan att förlora typer/tester.

---

## Huvudsakliga risker att hålla ögonen på under implementation

- **Sweep-line för `max_simultaneous`** — lätt att få fel när noter delar exakt samma
  `start_tick`. Skriv tre-not-edge-case-tester först.
- **Envelope-modulen identifiering** — antagandet "lägst instance" kan brytas av patcher med
  flera envelopes (modulations-env utan koppling till Amp). Om testlåten har sådana fall
  behöver vi traversera grafen för att hitta envelope→Amp-vägen. Fallback: returnera `Unknown`
  när det finns ≥2 envelopes och vi inte kan välja entydigt.
- **Tröskel 0.6 för Drums-exklusion** — kan vara för aggressiv om en kort plucked bass-not
  råkar uppfylla Percussive + Atonal. Testlåtens basspår är skarpa testfall — verifiera att de
  inte felklassas innan vi gör commit 2.
