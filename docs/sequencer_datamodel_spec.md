````markdown
# Sequencer Datamodell & Implementation - Referens (v0.23.0)

## Översikt

Detta dokument beskriver den implementerade datamodellen för sequencern i Modular Synth. Modellen stödjer både tracker-stil (mönsterbaserad) och piano roll-komposition.

Koden återfinns i `src/sequencer/` och är helt frikopplad från GUI och ljudmotor. Integrationen sker via `SequencerEngine` i `src/engine/`.

### Kärnprincip: Storage vs Runtime

- **Storage (Pattern/Song):** Noter lagras som objekt med starttid (`PatternTick`) och längd (`Duration`). Optimerat för redigering och serialisering.
- **Runtime (Engine):** Data konverteras till en ström av `SequencerEvent` (NoteOn/NoteOff) i realtid av `SequencerEngine`.

## Designprinciper

1. **Typsäkerhet** - Newtypes (`Tick`, `Pitch`, `Velocity`) för alla värden.
2. **Enhetlig tidsrepresentation** - All tid är i ticks med 960 PPQN (Pulses Per Quarter Note).
3. **Vy-agnostisk** - Samma data kan visas som tracker-rader eller piano roll.
4. **Objektbaserade noter** - Noter har `duration`, inte separata on/off-events i lagringen.
5. **Separation av ansvar** - Sequencern vet inget om oscillatorer eller samplingar, den skickar bara `InstrumentId`.

---

## Del 1: Grundtyper (`src/sequencer/time.rs`, `pitch.rs`, `ids.rs`)

### 1.1 Tidstyper

```rust
/// Absolut position på song-tidslinjen
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

/// Relativ position inom ett pattern (0 = pattern-start)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatternTick(pub u32);

/// Längd/varaktighet i ticks
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(pub u32);

pub const TICKS_PER_QUARTER: u32 = 960;
````

### 1.2 Identifierare

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatternId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstrumentId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoteId(pub u64);
```

### 1.3 Tonhöjd och Velocity

```rust
/// MIDI-kompatibel tonhöjd (0-127)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pitch(u8);

/// Velocity (0-127)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Velocity(u8);
```

-----

## Del 2: Effektsystem (`src/sequencer/effects.rs`)

Varje not kan ha en lista av `EffectCommand` för tracker-liknande modulation.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EffectCommand {
    // Pitch
    Arpeggio { x: u8, y: u8 },
    PortamentoUp(u8),
    PortamentoDown(u8),
    Vibrato { speed: u8, depth: u8 },
    
    // Volym & Pan
    SetVolume(u8),
    VolumeSlide { up: u8, down: u8 },
    SetPanning(u8),
    
    // Timing & Flöde
    SetTempo(u16),
    PatternBreak(u8),
    
    // ... fler effekter finns i koden
}
```

-----

## Del 3: Not-lagring (`src/sequencer/note.rs`, `pattern.rs`)

### 3.1 Note

```rust
pub struct Note {
    pub id: NoteId,
    pub start: PatternTick,
    pub duration: Option<Duration>, // None = Tracker-stil "tills vidare"
    pub pitch: Pitch,
    pub velocity: Velocity,
    pub instrument: InstrumentId,
    pub effects: Vec<EffectCommand>,
}
```

### 3.2 Pattern

Behållare för noter och automation.

```rust
pub struct Pattern {
    pub id: PatternId,
    pub length: Duration,
    pub row_resolution: RowResolution, // För tracker-grid (t.ex. 16 rader)
    notes: Vec<Note>,                  // Alltid sorterad på start-tid
    pub automation: Vec<AutomationLane>,
}
```

-----

## Del 4: Struktur (`src/sequencer/song.rs`, `track.rs`)

### 4.1 Arrangemang

Låten byggs upp av `PatternPlacement` som placerar patterns på tidslinjen.

```rust
pub struct PatternPlacement {
    pub pattern_id: PatternId,
    pub track_id: TrackId,
    pub start: Tick,
    pub transpose: i8,
    pub gain: f32,
}
```

### 4.2 Song

Huvudstrukturen som håller allt.

```rust
pub struct Song {
    pub name: String,
    patterns: HashMap<PatternId, Pattern>,
    tracks: HashMap<TrackId, Track>,
    arrangement: Vec<PatternPlacement>,
    tempo_changes: Vec<TempoChange>,
    // ...
}
```

-----

## Del 5: Integration i Audio-motorn (`src/engine/sequencer_engine.rs`)

Detta är bryggan mellan datamodellen och ljudet.

### 5.1 SequencerEngine

En komponent som lever i `SynthEngine` (ljudtråden).

* **Ansvar:**
    * Hålla koll på `current_tick`.
    * Konvertera `SampleCount` (från ljudkortet) till `Tick`-delta baserat på tempo.
    * Läsa `Song` och hitta vilka events som ska triggas just nu.
    * Generera `SequencerEvent` (NoteOn/NoteOff).

### 5.2 Mapping: Instrument -\> Part

För att ljud ska höras måste sequencerns `InstrumentId` kopplas till en ljudkälla (`SynthPart`).

* **Sequencer:** Skickar `InstrumentId(1)`.
* **SynthEngine:** Måste mappa detta ID till en `PartId`.
* **Strategi:**
    * Enklast: `PartId` = `InstrumentId`. Instrument 1 styr Part 1 (t.ex. Bas), Instrument 2 styr Part 2 (t.ex. Trummor).
    * Avancerat: En mappningstabell i `Song` eller `SynthEngine`.

-----

## Del 6: Runtime Events (`src/sequencer/events.rs`)

Events som genereras i realtid av `SequencerEngine` och skickas till `SynthEngine` för exekvering.

```rust
pub enum SequencerEvent {
    NoteOn {
        tick: Tick,
        pitch: Pitch,
        velocity: Velocity,
        instrument: InstrumentId,
        effects: Vec<EffectCommand>,
    },
    NoteOff {
        tick: Tick,
        pitch: Pitch,
        instrument: InstrumentId,
    },
    // ...
}
```

-----

## Del 7: Koppling till Typsystemet (`src/types/`)

Sequencern använder sina egna typer för lagring (`Tick`, `Duration`), men dessa konverteras till motorns starka typer vid uppspelning.

| Sequencer Typ | Konverteras till (Engine) | Användning |
|---------------|---------------------------|------------|
| `Pitch`       | `Hertz`                   | Oscillator-frekvens |
| `Velocity`    | `Gain` / `NormalizedValue`| Amp-nivå / Modulering |
| `Tick`        | `Seconds`                 | Tidsstyrning (via BPM) |

-----

## Del 8: Tracker-vy (`src/sequencer/view/tracker.rs`)

Hjälpstrukturer för att rendera patterns som en klassisk tracker.

* **`to_tracker_rows()`**: Konverterar en `Pattern` till en lista av `TrackerRow` baserat på `RowResolution`. Detta gör det enkelt för GUI:t att rita ett rutnät utan att behöva förstå den underliggande tidsmodellen.

<!-- end list -->

```
```