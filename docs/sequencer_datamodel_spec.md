# Sequencer Datamodell - Specifikation v3

## Översikt

Detta dokument beskriver datamodellen för en flexibel sequencer som stödjer både tracker-stil komposition (inspirerad av ProTracker/FastTracker) och modern piano roll-stil. Datamodellen är helt frikopplad från visualisering och input.

### Kärnprincip: Storage vs Runtime

- **Storage (Pattern):** Noter lagras som objekt med starttid och längd. Optimerat för redigering.
- **Runtime (Engine):** Noter konverteras till `NoteOn`/`NoteOff`-strömmar vid uppspelning.

## Designprinciper

1. **Typsäkerhet** - Newtypes och enums för alla domänkoncept. Inga råa primitiver.
2. **Enhetlig tidsrepresentation** - All tid i ticks med 960 PPQN.
3. **Vy-agnostisk** - Samma data renderas som tracker-rader eller piano roll.
4. **Objektbaserade noter** - Noter har `duration` istället för separata on/off-events.
5. **Effektflexibilitet** - Varje not kan ha godtyckligt många effekter.
6. **Separation av input** - Abstraherade input-kommandon från alla källor.

---

## Del 1: Grundtyper

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
```

**Notlängder vid 960 PPQN:**

| Notvärde | Ticks |
|----------|-------|
| Helnot | 3840 |
| Halvnot | 1920 |
| Fjärdedelsnot | 960 |
| Åttondelsnot | 480 |
| Sextondelsnot | 240 |
| 32-delsnot | 120 |
| Triol-fjärdedel | 640 |
| Triol-åttondel | 320 |

**Implementera för alla tidstyper:**
- `Add`, `Sub`, `AddAssign`, `SubAssign`
- `Ord`, `Eq`, `Hash`
- `Copy`, `Clone`, `Debug`
- `Serialize`, `Deserialize` (serde)

**Konverteringsmetoder på `Tick`:**

```rust
impl Tick {
    pub fn from_pattern_tick(pattern_start: Tick, offset: PatternTick) -> Self;
    pub fn to_seconds(&self, tempo_bpm: f32) -> f64;
    pub fn from_seconds(seconds: f64, tempo_bpm: f32) -> Self;
    pub fn to_bar_beat_tick(&self, time_sig: TimeSignature) -> (u32, u32, u32);
    pub fn from_bar_beat_tick(bar: u32, beat: u32, tick: u32, time_sig: TimeSignature) -> Self;
}
```

### 1.2 Identifierare

Alla ID-typer är newtypes för typsäkerhet:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PatternId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstrumentId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteId(pub u64);
```

`NoteId` används för att referera till specifika noter vid redigering (undo/redo, selektion).

### 1.3 Tonhöjd och Velocity

```rust
/// MIDI-kompatibel tonhöjd (0-127)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pitch(u8);

impl Pitch {
    pub const MIN: Pitch = Pitch(0);
    pub const MAX: Pitch = Pitch(127);
    pub const MIDDLE_C: Pitch = Pitch(60);
    
    pub fn new(midi_note: u8) -> Option<Self> {
        (midi_note <= 127).then_some(Self(midi_note))
    }
    
    pub fn from_octave_note(octave: i8, note: NoteName) -> Option<Self> {
        let midi = (octave + 1) as i16 * 12 + note as i16;
        (0..=127).contains(&midi).then_some(Self(midi as u8))
    }
    
    pub fn octave(&self) -> i8 {
        (self.0 / 12) as i8 - 1
    }
    
    pub fn note_name(&self) -> NoteName {
        NoteName::from_midi(self.0 % 12)
    }
    
    pub fn frequency(&self, a4_hz: f32) -> f32 {
        a4_hz * 2.0_f32.powf((self.0 as f32 - 69.0) / 12.0)
    }
    
    pub fn as_midi(&self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NoteName {
    C = 0, Cs = 1, D = 2, Ds = 3, E = 4, F = 5,
    Fs = 6, G = 7, Gs = 8, A = 9, As = 10, B = 11,
}

impl NoteName {
    pub fn from_midi(value: u8) -> Self {
        match value % 12 {
            0 => Self::C, 1 => Self::Cs, 2 => Self::D, 3 => Self::Ds,
            4 => Self::E, 5 => Self::F, 6 => Self::Fs, 7 => Self::G,
            8 => Self::Gs, 9 => Self::A, 10 => Self::As, _ => Self::B,
        }
    }
}

/// Velocity (0-127)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Velocity(u8);

impl Velocity {
    pub const OFF: Velocity = Velocity(0);
    pub const PPP: Velocity = Velocity(16);
    pub const PP: Velocity = Velocity(32);
    pub const P: Velocity = Velocity(48);
    pub const MP: Velocity = Velocity(64);
    pub const MF: Velocity = Velocity(80);
    pub const F: Velocity = Velocity(96);
    pub const FF: Velocity = Velocity(112);
    pub const MAX: Velocity = Velocity(127);
    
    pub fn new(vel: u8) -> Option<Self> {
        (vel <= 127).then_some(Self(vel))
    }
    
    pub fn as_u8(&self) -> u8 {
        self.0
    }
    
    pub fn as_f32(&self) -> f32 {
        self.0 as f32 / 127.0
    }
}
```

---

## Del 2: Effektsystem

### 2.1 EffectCommand

Omfattande enum för tracker-stil effekter. Varje not kan ha en `Vec<EffectCommand>`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EffectCommand {
    // === Pitch-effekter ===
    /// Växla mellan grundton, +x halvtoner, +y halvtoner per tick
    Arpeggio { x: u8, y: u8 },
    /// Glid tonhöjd uppåt (enheter per tick)
    PortamentoUp(u8),
    /// Glid tonhöjd nedåt
    PortamentoDown(u8),
    /// Glid mot angiven målton
    TonePortamento { speed: u8, target: Option<Pitch> },
    /// Periodisk tonhöjdsvariation
    Vibrato { speed: u8, depth: u8 },
    /// Vågform för vibrato
    VibratoWaveform(Waveform),
    /// Kvantisera portamento till halvtoner
    Glissando(bool),
    /// Finjustera tonhöjd (-128 till +127 cent)
    FineTune(i8),
    
    // === Volym-effekter ===
    /// Sätt volym (0-64 tracker-stil)
    SetVolume(u8),
    /// Gradvis volymändring per tick
    VolumeSlide { up: u8, down: u8 },
    /// Finare volymändring (per rad istället för tick)
    FineVolumeSlide { up: u8, down: u8 },
    /// Periodisk volymvariation
    Tremolo { speed: u8, depth: u8 },
    /// Vågform för tremolo
    TremoloWaveform(Waveform),
    
    // === Panning-effekter ===
    /// Sätt panorering (0=vänster, 128=center, 255=höger)
    SetPanning(u8),
    /// Gradvis panoreringsändring
    PanningSlide { left: u8, right: u8 },
    
    // === Sample/playback-effekter ===
    /// Starta sample från position (i 256-dels enheter)
    SampleOffset(u16),
    /// Retriggra not med intervall och volymändring
    Retrigger { interval: u8, volume_change: i8 },
    /// Tysta not efter n ticks
    NoteCut(u8),
    /// Fördröj not n ticks
    NoteDelay(u8),
    /// Gradvis uttoning
    NoteFadeOut(u8),
    /// Spela sample baklänges
    Reverse,
    
    // === Timing-effekter (globala) ===
    /// Ändra tempo (BPM)
    SetTempo(u16),
    /// Ändra speed (ticks per rad, tracker-kompatibilitet)
    SetSpeed(u8),
    /// Fördröj pattern n rader
    PatternDelay(u8),
    
    // === Navigering (globala) ===
    /// Loopa sektion, count=0 sätter loop-start
    PatternLoop { count: u8 },
    /// Hoppa till pattern i orderlistan
    PatternJump(u8),
    /// Avbryt pattern, hoppa till rad i nästa
    PatternBreak(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waveform {
    Sine,
    Ramp,
    Square,
    Random,
}
```

### 2.2 Effekt-kategorisering

För UI och validering:

```rust
impl EffectCommand {
    /// Returnerar true om effekten är global (påverkar hela pattern/song)
    pub fn is_global(&self) -> bool {
        matches!(self, 
            Self::SetTempo(_) | Self::SetSpeed(_) | Self::PatternDelay(_) |
            Self::PatternLoop { .. } | Self::PatternJump(_) | Self::PatternBreak(_)
        )
    }
    
    /// Returnerar true om effekten är kontinuerlig (körs varje tick)
    pub fn is_continuous(&self) -> bool {
        matches!(self,
            Self::Arpeggio { .. } | Self::PortamentoUp(_) | Self::PortamentoDown(_) |
            Self::TonePortamento { .. } | Self::Vibrato { .. } | Self::VolumeSlide { .. } |
            Self::Tremolo { .. } | Self::PanningSlide { .. }
        )
    }
}
```

---

## Del 3: Not-lagring (Storage)

### 3.1 Note

Kärnstrukturen för lagring. Har starttid och längd - inte separata on/off-events.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Unikt ID för denna not (för redigering/selektion)
    pub id: NoteId,
    /// Startposition inom pattern
    pub start: PatternTick,
    /// Längd i ticks (None = "till nästa NoteOff" för tracker-kompatibilitet)
    pub duration: Option<Duration>,
    /// Tonhöjd
    pub pitch: Pitch,
    /// Anslagsstyrka
    pub velocity: Velocity,
    /// Vilket instrument som spelar noten
    pub instrument: InstrumentId,
    /// Effekter som appliceras vid notstart
    pub effects: Vec<EffectCommand>,
}

impl Note {
    pub fn new(
        id: NoteId,
        start: PatternTick,
        pitch: Pitch,
        velocity: Velocity,
        instrument: InstrumentId,
    ) -> Self {
        Self {
            id,
            start,
            duration: None,
            pitch,
            velocity,
            instrument,
            effects: Vec::new(),
        }
    }
    
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }
    
    pub fn with_effect(mut self, effect: EffectCommand) -> Self {
        self.effects.push(effect);
        self
    }
    
    pub fn with_effects(mut self, effects: impl IntoIterator<Item = EffectCommand>) -> Self {
        self.effects.extend(effects);
        self
    }
    
    /// Beräknad sluttid (returnerar None om duration är None)
    pub fn end(&self) -> Option<PatternTick> {
        self.duration.map(|d| PatternTick(self.start.0 + d.0))
    }
}
```

### 3.2 Automation

Parameterstyrning över tid, separat från noter.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub tick: PatternTick,
    pub value: f32,  // 0.0 - 1.0 normaliserat
    pub curve: CurveType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurveType {
    /// Linjär interpolation till nästa punkt
    Linear,
    /// Stegvis (håll värde till nästa punkt)
    Step,
    /// Exponentiell kurva (parameter anger styrka)
    Exponential(i8),
    /// S-kurva
    SCurve,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    pub points: Vec<AutomationPoint>,  // Sorterad på tick
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    /// Instrument-parameter
    Instrument {
        instrument: InstrumentId,
        param: InstrumentParam,
    },
    /// Track-parameter
    Track {
        track: TrackId,
        param: TrackParam,
    },
    /// Global parameter
    Global(GlobalParam),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstrumentParam {
    Volume,
    Pan,
    FilterCutoff,
    FilterResonance,
    Attack,
    Decay,
    Sustain,
    Release,
    // Utökas efter behov, matchar din VoiceModule
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackParam {
    Volume,
    Pan,
    Mute,
    Solo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalParam {
    Tempo,
    MasterVolume,
    Swing,
}

impl AutomationLane {
    /// Hämta interpolerat värde vid given tick
    pub fn value_at(&self, tick: PatternTick) -> Option<f32> {
        if self.points.is_empty() {
            return None;
        }
        
        // Hitta omgivande punkter och interpolera
        let idx = self.points.partition_point(|p| p.tick <= tick);
        
        if idx == 0 {
            return Some(self.points[0].value);
        }
        if idx >= self.points.len() {
            return Some(self.points.last().unwrap().value);
        }
        
        let before = &self.points[idx - 1];
        let after = &self.points[idx];
        
        let t = (tick.0 - before.tick.0) as f32 / (after.tick.0 - before.tick.0) as f32;
        
        Some(match before.curve {
            CurveType::Linear => before.value + (after.value - before.value) * t,
            CurveType::Step => before.value,
            CurveType::Exponential(strength) => {
                let exp_t = if strength >= 0 {
                    t.powf(1.0 + strength as f32 * 0.1)
                } else {
                    1.0 - (1.0 - t).powf(1.0 - strength as f32 * 0.1)
                };
                before.value + (after.value - before.value) * exp_t
            }
            CurveType::SCurve => {
                let s = t * t * (3.0 - 2.0 * t);  // Smoothstep
                before.value + (after.value - before.value) * s
            }
        })
    }
}
```

---

## Del 4: Pattern

### 4.1 RowResolution

Konfigurerar hur patterns mappas till tracker-rader.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowResolution {
    /// Antal rader i detta pattern
    pub rows: u16,
    /// Ticks per rad
    pub ticks_per_row: u16,
}

impl RowResolution {
    /// Standard tracker: 64 rader, 16 rader per takt vid 4/4
    pub fn standard_64() -> Self {
        Self { rows: 64, ticks_per_row: 240 }
    }
    
    /// Kort pattern: 16 rader
    pub fn short_16() -> Self {
        Self { rows: 16, ticks_per_row: 240 }
    }
    
    /// Hög upplösning: 128 rader
    pub fn high_128() -> Self {
        Self { rows: 128, ticks_per_row: 120 }
    }
    
    /// Beräkna pattern-längd i ticks
    pub fn total_ticks(&self) -> Duration {
        Duration(self.rows as u32 * self.ticks_per_row as u32)
    }
    
    /// Konvertera rad till tick
    pub fn row_to_tick(&self, row: u16) -> PatternTick {
        PatternTick(row as u32 * self.ticks_per_row as u32)
    }
    
    /// Konvertera tick till rad (avrundad nedåt)
    pub fn tick_to_row(&self, tick: PatternTick) -> u16 {
        (tick.0 / self.ticks_per_row as u32) as u16
    }
    
    /// Kvantisera tick till närmaste rad
    pub fn quantize(&self, tick: PatternTick) -> PatternTick {
        let row = (tick.0 + self.ticks_per_row as u32 / 2) / self.ticks_per_row as u32;
        PatternTick(row * self.ticks_per_row as u32)
    }
}
```

### 4.2 Pattern

Behållare för noter och automation.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: PatternId,
    pub name: String,
    /// Längd i ticks
    pub length: Duration,
    /// Alla noter, sorterade på start-tick
    notes: Vec<Note>,
    /// Automationsbanor
    pub automation: Vec<AutomationLane>,
    /// Rad-upplösning för tracker-vy
    pub row_resolution: RowResolution,
    /// Nästa not-ID (för att generera unika ID:n)
    next_note_id: u64,
}

impl Pattern {
    pub fn new(id: PatternId, length: Duration) -> Self {
        Self {
            id,
            name: String::new(),
            length,
            notes: Vec::new(),
            automation: Vec::new(),
            row_resolution: RowResolution::standard_64(),
            next_note_id: 0,
        }
    }
    
    pub fn with_row_resolution(mut self, resolution: RowResolution) -> Self {
        self.length = resolution.total_ticks();
        self.row_resolution = resolution;
        self
    }
    
    // === Not-hantering ===
    
    /// Generera nytt unikt not-ID
    fn next_id(&mut self) -> NoteId {
        let id = NoteId(self.next_note_id);
        self.next_note_id += 1;
        id
    }
    
    /// Lägg till not (returnerar tilldelat ID)
    pub fn add_note(
        &mut self,
        start: PatternTick,
        pitch: Pitch,
        velocity: Velocity,
        instrument: InstrumentId,
    ) -> NoteId {
        let id = self.next_id();
        let note = Note::new(id, start, pitch, velocity, instrument);
        
        // Infoga sorterat på start-tick
        let pos = self.notes.partition_point(|n| n.start <= start);
        self.notes.insert(pos, note);
        id
    }
    
    /// Lägg till komplett not
    pub fn insert_note(&mut self, mut note: Note) -> NoteId {
        note.id = self.next_id();
        let id = note.id;
        let pos = self.notes.partition_point(|n| n.start <= note.start);
        self.notes.insert(pos, note);
        id
    }
    
    /// Ta bort not med givet ID
    pub fn remove_note(&mut self, id: NoteId) -> Option<Note> {
        let pos = self.notes.iter().position(|n| n.id == id)?;
        Some(self.notes.remove(pos))
    }
    
    /// Hämta not via ID
    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }
    
    /// Hämta mutable not via ID
    pub fn note_mut(&mut self, id: NoteId) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }
    
    /// Alla noter (sorterade)
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }
    
    /// Noter inom tick-intervall
    pub fn notes_in_range(&self, start: PatternTick, end: PatternTick) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| {
            n.start < end && n.end().map_or(true, |e| e > start)
        })
    }
    
    /// Noter som startar på exakt rad
    pub fn notes_at_row(&self, row: u16) -> impl Iterator<Item = &Note> {
        let tick = self.row_resolution.row_to_tick(row);
        let next_tick = self.row_resolution.row_to_tick(row + 1);
        self.notes.iter().filter(move |n| n.start >= tick && n.start < next_tick)
    }
    
    // === Bulk-operationer ===
    
    /// Flytta not till ny position
    pub fn move_note(&mut self, id: NoteId, new_start: PatternTick) -> bool {
        if let Some(note) = self.remove_note(id) {
            let mut moved = note;
            moved.start = new_start;
            self.insert_note(moved);
            true
        } else {
            false
        }
    }
    
    /// Ändra längd på not
    pub fn resize_note(&mut self, id: NoteId, new_duration: Duration) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.duration = Some(new_duration);
            true
        } else {
            false
        }
    }
    
    /// Kvantisera alla noter till row-grid
    pub fn quantize_notes(&mut self) {
        for note in &mut self.notes {
            note.start = self.row_resolution.quantize(note.start);
        }
        // Re-sortera efter kvantisering
        self.notes.sort_by_key(|n| n.start);
    }
    
    /// Kvantisera med styrka (0.0 = ingen, 1.0 = full)
    pub fn quantize_notes_with_strength(&mut self, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for note in &mut self.notes {
            let quantized = self.row_resolution.quantize(note.start);
            let diff = quantized.0 as f32 - note.start.0 as f32;
            note.start = PatternTick((note.start.0 as f32 + diff * strength) as u32);
        }
        self.notes.sort_by_key(|n| n.start);
    }
}
```

---

## Del 5: Track och Song

### 5.1 Track

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    /// Instrument som detta spår styr (None = MIDI out eller inget)
    pub instrument: Option<InstrumentId>,
    /// Volym (0.0 - 1.0)
    pub volume: f32,
    /// Panorering (0.0 = vänster, 0.5 = center, 1.0 = höger)
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Track {
    pub fn new(id: TrackId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            instrument: None,
            volume: 1.0,
            pan: 0.5,
            mute: false,
            solo: false,
        }
    }
}
```

### 5.2 Tempo och taktart

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl TimeSignature {
    pub const COMMON: TimeSignature = TimeSignature { numerator: 4, denominator: 4 };
    pub const WALTZ: TimeSignature = TimeSignature { numerator: 3, denominator: 4 };
    
    /// Ticks per takt
    pub fn ticks_per_bar(&self) -> u32 {
        TICKS_PER_QUARTER * self.numerator as u32 * 4 / self.denominator as u32
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TempoChange {
    pub tick: Tick,
    pub bpm: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeSignatureChange {
    pub tick: Tick,
    pub signature: TimeSignature,
}
```

### 5.3 PatternPlacement

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternPlacement {
    pub pattern_id: PatternId,
    pub track_id: TrackId,
    pub start: Tick,
    /// Transponering i halvtoner
    pub transpose: i8,
    /// Volym-skalning (1.0 = normal)
    pub gain: f32,
}

impl PatternPlacement {
    pub fn new(pattern_id: PatternId, track_id: TrackId, start: Tick) -> Self {
        Self {
            pattern_id,
            track_id,
            start,
            transpose: 0,
            gain: 1.0,
        }
    }
    
    /// Sluttid baserat på pattern-längd
    pub fn end(&self, pattern_length: Duration) -> Tick {
        Tick(self.start.0 + pattern_length.0 as u64)
    }
}
```

### 5.4 Song

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    pub author: String,
    
    // Pattern-lagring
    patterns: HashMap<PatternId, Pattern>,
    next_pattern_id: u32,
    
    // Track-lagring
    tracks: HashMap<TrackId, Track>,
    next_track_id: u16,
    
    // Arrangemang
    arrangement: Vec<PatternPlacement>,
    
    // Tempo och taktart
    tempo_changes: Vec<TempoChange>,
    time_signature_changes: Vec<TimeSignatureChange>,
    pub default_tempo: f32,
    pub default_time_signature: TimeSignature,
}

impl Song {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            patterns: HashMap::new(),
            next_pattern_id: 0,
            tracks: HashMap::new(),
            next_track_id: 0,
            arrangement: Vec::new(),
            tempo_changes: Vec::new(),
            time_signature_changes: Vec::new(),
            default_tempo: 120.0,
            default_time_signature: TimeSignature::COMMON,
        }
    }
    
    // === Pattern-hantering ===
    
    pub fn create_pattern(&mut self, length: Duration) -> PatternId {
        let id = PatternId(self.next_pattern_id);
        self.next_pattern_id += 1;
        self.patterns.insert(id, Pattern::new(id, length));
        id
    }
    
    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.get(&id)
    }
    
    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
        self.patterns.get_mut(&id)
    }
    
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.values()
    }
    
    pub fn delete_pattern(&mut self, id: PatternId) -> Option<Pattern> {
        // Ta också bort från arrangement
        self.arrangement.retain(|p| p.pattern_id != id);
        self.patterns.remove(&id)
    }
    
    // === Track-hantering ===
    
    pub fn create_track(&mut self, name: impl Into<String>) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id += 1;
        self.tracks.insert(id, Track::new(id, name));
        id
    }
    
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.get(&id)
    }
    
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.get_mut(&id)
    }
    
    pub fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.values()
    }
    
    // === Arrangemang ===
    
    pub fn place_pattern(&mut self, pattern_id: PatternId, track_id: TrackId, start: Tick) {
        let placement = PatternPlacement::new(pattern_id, track_id, start);
        
        // Håll sorterat på starttid
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
    }
    
    pub fn arrangement(&self) -> &[PatternPlacement] {
        &self.arrangement
    }
    
    pub fn placements_in_range(&self, start: Tick, end: Tick) -> impl Iterator<Item = &PatternPlacement> {
        self.arrangement.iter().filter(move |p| {
            let pattern_end = self.patterns.get(&p.pattern_id)
                .map(|pat| p.end(pat.length))
                .unwrap_or(p.start);
            p.start < end && pattern_end > start
        })
    }
    
    // === Tempo ===
    
    pub fn set_tempo_at(&mut self, tick: Tick, bpm: f32) {
        // Ta bort existerande vid samma tick
        self.tempo_changes.retain(|t| t.tick != tick);
        
        let change = TempoChange { tick, bpm };
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        self.tempo_changes.insert(pos, change);
    }
    
    pub fn tempo_at(&self, tick: Tick) -> f32 {
        self.tempo_changes
            .iter()
            .rev()
            .find(|t| t.tick <= tick)
            .map(|t| t.bpm)
            .unwrap_or(self.default_tempo)
    }
    
    pub fn time_signature_at(&self, tick: Tick) -> TimeSignature {
        self.time_signature_changes
            .iter()
            .rev()
            .find(|t| t.tick <= tick)
            .map(|t| t.signature)
            .unwrap_or(self.default_time_signature)
    }
    
    // === Tidskonvertering ===
    
    /// Konvertera tick till sekunder (hanterar tempo-ändringar)
    pub fn tick_to_seconds(&self, target: Tick) -> f64 {
        let mut seconds = 0.0;
        let mut current_tick = Tick(0);
        let mut current_tempo = self.default_tempo;
        
        for change in &self.tempo_changes {
            if change.tick >= target {
                break;
            }
            
            // Beräkna tid till denna tempo-ändring
            let ticks = change.tick.0 - current_tick.0;
            let beats = ticks as f64 / TICKS_PER_QUARTER as f64;
            seconds += beats * 60.0 / current_tempo as f64;
            
            current_tick = change.tick;
            current_tempo = change.bpm;
        }
        
        // Resterande ticks med nuvarande tempo
        let remaining_ticks = target.0 - current_tick.0;
        let remaining_beats = remaining_ticks as f64 / TICKS_PER_QUARTER as f64;
        seconds += remaining_beats * 60.0 / current_tempo as f64;
        
        seconds
    }
    
    /// Beräkna total längd baserat på arrangemang
    pub fn calculate_length(&self) -> Tick {
        self.arrangement.iter()
            .filter_map(|p| {
                self.patterns.get(&p.pattern_id)
                    .map(|pat| p.end(pat.length))
            })
            .max()
            .unwrap_or(Tick(0))
    }
}
```

---

## Del 6: Runtime Events

Events som genereras vid uppspelning. Dessa är **inte** lagrade - de skapas i realtid.

```rust
#[derive(Debug, Clone)]
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
    Effect {
        tick: Tick,
        effect: EffectCommand,
    },
    Parameter {
        tick: Tick,
        target: AutomationTarget,
        value: f32,
    },
}

impl SequencerEvent {
    pub fn tick(&self) -> Tick {
        match self {
            Self::NoteOn { tick, .. } => *tick,
            Self::NoteOff { tick, .. } => *tick,
            Self::Effect { tick, .. } => *tick,
            Self::Parameter { tick, .. } => *tick,
        }
    }
}
```

### Event-generering från Pattern

```rust
impl Pattern {
    /// Generera runtime-events för uppspelning inom intervall
    pub fn generate_events(
        &self,
        pattern_start: Tick,
        range_start: Tick,
        range_end: Tick,
        transpose: i8,
        instrument_override: Option<InstrumentId>,
    ) -> Vec<SequencerEvent> {
        let mut events = Vec::new();
        
        // Konvertera song-ticks till pattern-lokala ticks
        let local_start = if range_start.0 > pattern_start.0 {
            PatternTick((range_start.0 - pattern_start.0) as u32)
        } else {
            PatternTick(0)
        };
        let local_end = PatternTick((range_end.0 - pattern_start.0) as u32);
        
        for note in &self.notes {
            let instrument = instrument_override.unwrap_or(note.instrument);
            
            // Transponera pitch
            let transposed_pitch = Pitch::new(
                (note.pitch.as_midi() as i16 + transpose as i16).clamp(0, 127) as u8
            ).unwrap_or(note.pitch);
            
            // NoteOn
            if note.start >= local_start && note.start < local_end {
                let absolute_tick = Tick(pattern_start.0 + note.start.0 as u64);
                events.push(SequencerEvent::NoteOn {
                    tick: absolute_tick,
                    pitch: transposed_pitch,
                    velocity: note.velocity,
                    instrument,
                    effects: note.effects.clone(),
                });
            }
            
            // NoteOff
            if let Some(end) = note.end() {
                if end >= local_start && end < local_end {
                    let absolute_tick = Tick(pattern_start.0 + end.0 as u64);
                    events.push(SequencerEvent::NoteOff {
                        tick: absolute_tick,
                        pitch: transposed_pitch,
                        instrument,
                    });
                }
            }
        }
        
        // Sortera på tick
        events.sort_by_key(|e| e.tick());
        events
    }
}
```

---

## Del 7: Input-abstraktion

### 7.1 InputCommand

```rust
#[derive(Debug, Clone)]
pub enum InputCommand {
    // === Realtids-input ===
    /// Not på (från MIDI, tangentbord, etc)
    NoteOn {
        pitch: Pitch,
        velocity: Velocity,
        instrument: Option<InstrumentId>,
    },
    /// Not av
    NoteOff {
        pitch: Pitch,
    },
    
    // === Pattern-redigering ===
    /// Lägg till not i pattern
    AddNote {
        pattern: PatternId,
        start: PatternTick,
        duration: Option<Duration>,
        pitch: Pitch,
        velocity: Velocity,
    },
    /// Ta bort not
    RemoveNote {
        pattern: PatternId,
        note_id: NoteId,
    },
    /// Flytta not
    MoveNote {
        pattern: PatternId,
        note_id: NoteId,
        new_start: PatternTick,
    },
    /// Ändra längd
    ResizeNote {
        pattern: PatternId,
        note_id: NoteId,
        new_duration: Duration,
    },
    /// Lägg till effekt på not
    AddEffect {
        pattern: PatternId,
        note_id: NoteId,
        effect: EffectCommand,
    },
    
    // === Selektion ===
    SelectNotes {
        pattern: PatternId,
        note_ids: Vec<NoteId>,
    },
    SelectRange {
        pattern: PatternId,
        start: PatternTick,
        end: PatternTick,
        pitch_min: Option<Pitch>,
        pitch_max: Option<Pitch>,
    },
    ClearSelection,
    DeleteSelection,
    CopySelection,
    PasteAt {
        pattern: PatternId,
        tick: PatternTick,
    },
    
    // === Kvantisering ===
    Quantize {
        pattern: PatternId,
        strength: f32,
    },
    
    // === Transport ===
    Play,
    Stop,
    Pause,
    ToggleRecord,
    SetPosition(Tick),
    SetLoop {
        start: Tick,
        end: Tick,
        enabled: bool,
    },
    SetTempo(f32),
}
```

### 7.2 InputSource Trait

```rust
pub trait InputSource: Send {
    /// Polla efter nya kommandon
    fn poll(&mut self) -> Vec<InputCommand>;
    
    /// Namn för UI
    fn name(&self) -> &str;
    
    /// Är källan ansluten/aktiv?
    fn is_active(&self) -> bool;
}
```

---

## Del 8: Tracker-vy Helpers

Strukturer för att rendera Pattern som tracker-rader. **Inte** del av kärnmodellen.

```rust
/// En cell i tracker-vyn
#[derive(Debug, Clone, Default)]
pub struct TrackerCell {
    pub note: Option<TrackerNoteDisplay>,
    pub instrument: Option<InstrumentId>,
    pub volume: Option<u8>,
    pub effects: Vec<EffectCommand>,
}

#[derive(Debug, Clone)]
pub struct TrackerNoteDisplay {
    pub pitch: Pitch,
    pub is_note_off: bool,
}

/// En rad i tracker-vyn
#[derive(Debug, Clone)]
pub struct TrackerRow {
    pub row_index: u16,
    pub columns: Vec<TrackerCell>,
}

/// Konfiguration för tracker-vy
#[derive(Debug, Clone)]
pub struct TrackerViewConfig {
    /// Antal synliga effektkolumner (1-8)
    pub effect_columns: u8,
    /// Visa volym-kolumn
    pub show_volume: bool,
    /// Visa instrument-kolumn
    pub show_instrument: bool,
    /// Visa rad-nummer som hex
    pub hex_row_numbers: bool,
}

impl Default for TrackerViewConfig {
    fn default() -> Self {
        Self {
            effect_columns: 2,
            show_volume: true,
            show_instrument: true,
            hex_row_numbers: true,
        }
    }
}

impl Pattern {
    /// Konvertera pattern till tracker-rader för rendering
    pub fn to_tracker_rows(&self, config: &TrackerViewConfig) -> Vec<TrackerRow> {
        let num_rows = self.row_resolution.rows as usize;
        let mut rows: Vec<TrackerRow> = (0..num_rows)
            .map(|i| TrackerRow {
                row_index: i as u16,
                columns: vec![TrackerCell::default()],
            })
            .collect();
        
        for note in &self.notes {
            let row_idx = self.row_resolution.tick_to_row(note.start) as usize;
            if row_idx >= num_rows {
                continue;
            }
            
            let row = &mut rows[row_idx];
            
            // Hitta ledig kolumn eller skapa ny
            let col_idx = row.columns
                .iter()
                .position(|c| c.note.is_none())
                .unwrap_or_else(|| {
                    row.columns.push(TrackerCell::default());
                    row.columns.len() - 1
                });
            
            let cell = &mut row.columns[col_idx];
            cell.note = Some(TrackerNoteDisplay {
                pitch: note.pitch,
                is_note_off: false,
            });
            cell.instrument = Some(note.instrument);
            cell.volume = Some((note.velocity.as_f32() * 64.0) as u8);
            
            // Kopiera effekter (upp till konfigurerat antal)
            cell.effects = note.effects
                .iter()
                .take(config.effect_columns as usize)
                .copied()
                .collect();
            
            // NoteOff på slutraden
            if let Some(end) = note.end() {
                let end_row_idx = self.row_resolution.tick_to_row(end) as usize;
                if end_row_idx < num_rows && end_row_idx > row_idx {
                    let end_row = &mut rows[end_row_idx];
                    
                    // Hitta matchande kolumn eller skapa
                    while end_row.columns.len() <= col_idx {
                        end_row.columns.push(TrackerCell::default());
                    }
                    
                    if end_row.columns[col_idx].note.is_none() {
                        end_row.columns[col_idx].note = Some(TrackerNoteDisplay {
                            pitch: note.pitch,
                            is_note_off: true,
                        });
                    }
                }
            }
        }
        
        rows
    }
}
```

---

## Del 9: Filstruktur

```
src/sequencer/
├── mod.rs              // Re-exports
├── time.rs             // Tick, PatternTick, Duration, TimeSignature
├── pitch.rs            // Pitch, NoteName, Velocity
├── ids.rs              // PatternId, TrackId, InstrumentId, NoteId
├── effects.rs          // EffectCommand, Waveform
├── note.rs             // Note
├── automation.rs       // AutomationPoint, AutomationLane, AutomationTarget
├── pattern.rs          // Pattern, RowResolution
├── track.rs            // Track
├── song.rs             // Song, PatternPlacement, TempoChange
├── events.rs           // SequencerEvent (runtime)
├── input.rs            // InputCommand, InputSource trait
└── view/
    ├── mod.rs
    └── tracker.rs      // TrackerRow, TrackerCell, TrackerViewConfig
```

---

## Del 10: Implementationsordning

### Fas 1: Grundtyper (ingen extern beroenden)
1. `time.rs` - Tick, PatternTick, Duration
2. `pitch.rs` - Pitch, NoteName, Velocity
3. `ids.rs` - Alla ID-typer
4. **Tester:** Konverteringar, validering, overflow

### Fas 2: Effekter och Noter
1. `effects.rs` - EffectCommand, Waveform
2. `note.rs` - Note med builder-metoder
3. **Tester:** Builder-pattern, effekt-kategorisering

### Fas 3: Pattern
1. `automation.rs` - AutomationPoint, AutomationLane
2. `pattern.rs` - Pattern, RowResolution
3. **Tester:** Add/remove/move noter, kvantisering, range-queries

### Fas 4: Song
1. `track.rs` - Track
2. `song.rs` - Song, PatternPlacement, TempoChange
3. **Tester:** Tidskonvertering med tempo-ändringar, arrangemang

### Fas 5: Runtime och Input
1. `events.rs` - SequencerEvent, generate_events
2. `input.rs` - InputCommand, InputSource
3. **Tester:** Event-generering, transponering

### Fas 6: Vy-helpers
1. `view/tracker.rs` - TrackerRow, TrackerCell, to_tracker_rows
2. **Tester:** Pattern-till-tracker konvertering

---

## Del 11: Testfall

### Kritiska tester

```rust
#[test]
fn tick_to_seconds_constant_tempo() {
    let song = Song::new("test");
    // 120 BPM = 2 beats/sec = 1920 ticks/sec
    assert_eq!(song.tick_to_seconds(Tick(1920)), 1.0);
}

#[test]
fn tick_to_seconds_with_tempo_change() {
    let mut song = Song::new("test");
    song.set_tempo_at(Tick(960), 240.0);  // Dubbla tempot efter 1 beat
    // Första beat: 0.5 sek (120 BPM)
    // Andra beat: 0.25 sek (240 BPM)
    assert!((song.tick_to_seconds(Tick(1920)) - 0.75).abs() < 0.001);
}

#[test]
fn note_add_and_retrieve() {
    let mut pattern = Pattern::new(PatternId(0), Duration(3840));
    let id = pattern.add_note(
        PatternTick(0),
        Pitch::new(60).unwrap(),
        Velocity::MF,
        InstrumentId(0),
    );
    assert!(pattern.note(id).is_some());
}

#[test]
fn notes_stay_sorted_after_insert() {
    let mut pattern = Pattern::new(PatternId(0), Duration(3840));
    pattern.add_note(PatternTick(480), Pitch::new(60).unwrap(), Velocity::MF, InstrumentId(0));
    pattern.add_note(PatternTick(0), Pitch::new(62).unwrap(), Velocity::MF, InstrumentId(0));
    pattern.add_note(PatternTick(240), Pitch::new(64).unwrap(), Velocity::MF, InstrumentId(0));
    
    let ticks: Vec<_> = pattern.notes().iter().map(|n| n.start.0).collect();
    assert_eq!(ticks, vec![0, 240, 480]);
}

#[test]
fn tracker_row_conversion() {
    let mut pattern = Pattern::new(PatternId(0), Duration(3840))
        .with_row_resolution(RowResolution::standard_64());
    
    // Not på rad 0
    pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF, InstrumentId(0));
    
    let rows = pattern.to_tracker_rows(&TrackerViewConfig::default());
    assert!(rows[0].columns[0].note.is_some());
    assert_eq!(rows[0].columns[0].note.as_ref().unwrap().pitch.as_midi(), 60);
}

#[test]
fn quantize_with_strength() {
    let mut pattern = Pattern::new(PatternId(0), Duration(3840))
        .with_row_resolution(RowResolution::standard_64());
    
    // Not mellan rad 0 och 1 (tick 120, mitt emellan 0 och 240)
    let id = pattern.add_note(PatternTick(120), Pitch::new(60).unwrap(), Velocity::MF, InstrumentId(0));
    
    // 50% kvantisering
    pattern.quantize_notes_with_strength(0.5);
    
    // Ska flyttas halvvägs mot 240 (närmaste rad), dvs till 180
    let note = pattern.note(id).unwrap();
    assert_eq!(note.start.0, 180);
}
```

---

## Del 12: Prestandaöverväganden

1. **Sorterade vektorer** - Använd `partition_point` och `binary_search` för O(log n) sökning
2. **SmallVec för effekter** - De flesta noter har 0-2 effekter, undvik heap-allokering
3. **Inga allokeringar i generate_events hot path** - Överväg object pool
4. **Lazy event-generering** - Generera bara events för synligt/spelande intervall
5. **Inkrementell uppdatering** - När en not ändras, uppdatera inte hela strukturen

---

## Del 13: Framtida utökningar

Strukturen är förberedd för:

1. **MPE (MIDI Polyphonic Expression)** - Note har redan per-not effects
2. **Mikrotonal** - Pitch kan utökas med cent-offset
3. **Probabilistisk sekvensering** - Lägg till `probability: f32` på Note
4. **Polymetriska patterns** - Patterns kan ha olika längder
5. **Live looping** - PatternPlacement kan markeras som "recording"
