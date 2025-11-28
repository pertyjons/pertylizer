# TODO - Modular Synth


## 🟡 Kvarstående brister

### 1. Keyboard Mapping är hårdkodad
* **Status:** Tangentbordet (Z-M, Q-I) är hårdkodat i `egui_backend.rs`.
* **Problem:** Det fungerar dåligt på icke-QWERTY layouter (t.ex. AZERTY) och går inte att ändra.
* **Prioritet:** Låg

---

## 📝 Nästa steg

### Steg 0: Lägga till möjlighet att spela in noter och ljud till en hel komposition

#### Förenklad approach: Tracker-stil

Istället för att bygga en komplex DAW direkt, börja med en enkel **tracker-inspirerad** sequencer (som Soundtracker, FastTracker, etc). Bygg sedan ut stegvis.

---

## 🎯 Fas 1: Minimal Pattern Player (BÖRJA HÄR)

**Mål:** En enkel vy som visar ett pattern med 16 steg och spelar upp det i loop.

### GUI-skiss (Minimal)
```
┌─ PATTERN ────────────────────────────┐
│  BPM: [120]  [▶ Play] [⏹ Stop]       │
├──────┬───────────────────────────────┤
│ Step │ Not   │ Vel │                 │
├──────┼───────┼─────┤                 │
│  00  │  --   │ --- │                 │
│  01  │  C-4  │ 100 │ ◀── Playhead    │
│  02  │  --   │ --- │                 │
│  03  │  E-4  │  80 │                 │
│  04  │  --   │ --- │                 │
│  05  │  G-4  │  90 │                 │
│  06  │  --   │ --- │                 │
│  07  │  C-5  │ 100 │                 │
│  08  │  --   │ --- │                 │
│  09  │  --   │ --- │                 │
│  10  │  G-4  │  70 │                 │
│  11  │  --   │ --- │                 │
│  12  │  E-4  │  80 │                 │
│  13  │  --   │ --- │                 │
│  14  │  --   │ --- │                 │
│  15  │  C-4  │ 100 │                 │
└──────┴───────┴─────┴─────────────────┘
  [Klicka på rad + tryck tangent = sätt not]
```

### Datastruktur (Minimal)
```rust
// src/sequencer/mod.rs

/// Ett enda steg i ett pattern
#[derive(Clone, Default)]
pub struct Step {
    pub note: Option<u8>,    // MIDI-not, None = tyst
    pub velocity: f32,       // 0.0-1.0
}

/// Ett pattern med fasta antal steg
pub struct Pattern {
    pub steps: Vec<Step>,    // T.ex. 16 steg
    pub steps_per_beat: u8,  // 4 = 16-delar
}

impl Pattern {
    pub fn new(length: usize) -> Self {
        Self {
            steps: vec![Step::default(); length],
            steps_per_beat: 4,
        }
    }
}
```

### Minimal uppspelningslogik
```rust
/// Enkel pattern-spelare som körs i GUI-tråden med timer
pub struct PatternPlayer {
    pattern: Pattern,
    current_step: usize,
    is_playing: bool,
    bpm: f32,
    last_step_time: Instant,
}

impl PatternPlayer {
    /// Anropas varje frame - kollar om det är dags för nästa steg
    pub fn update(&mut self, handle: &EngineHandle) {
        if !self.is_playing {
            return;
        }

        let step_duration = Duration::from_secs_f32(
            60.0 / (self.bpm * self.pattern.steps_per_beat as f32)
        );

        if self.last_step_time.elapsed() >= step_duration {
            // Spela nästa steg
            self.play_current_step(handle);
            self.current_step = (self.current_step + 1) % self.pattern.steps.len();
            self.last_step_time = Instant::now();
        }
    }

    fn play_current_step(&self, handle: &EngineHandle) {
        let step = &self.pattern.steps[self.current_step];
        if let Some(note) = step.note {
            // Skicka note-off för föregående, note-on för denna
            handle.note_on(note, step.velocity);
            // Note-off hanteras vid nästa steg eller efter kort tid
        }
    }
}
```

### Implementation (steg för steg)

**Steg 1.1: Skapa grundstrukturer**
- [ ] Skapa `src/sequencer/mod.rs` med `Step` och `Pattern`
- [ ] Skapa `PatternPlayer` med play/stop/update

**Steg 1.2: Enkel GUI-vy**
- [ ] Skapa `src/gui/pattern_view.rs`
- [ ] Visa 16 rader med step-nummer och not
- [ ] Markera nuvarande step (playhead)
- [ ] Play/Stop-knappar

**Steg 1.3: Redigering**
- [ ] Klicka på rad för att välja
- [ ] Tryck tangent (Z-M, Q-I) för att sätta not
- [ ] Delete/Backspace för att ta bort not

**Steg 1.4: Grundläggande timing**
- [ ] BPM-kontroll
- [ ] Loop-uppspelning

### Filstruktur (Minimal)
```
src/
├── sequencer/
│   ├── mod.rs          # Step, Pattern, PatternPlayer
│   └── (det är allt för nu!)
└── gui/
    └── pattern_view.rs # Enkel tracker-vy
```

---

## 🎯 Fas 2: Förbättringar (efter Fas 1 fungerar)

**Bygg vidare när grunderna fungerar:**

### 2.1 Note-off hantering
```
┌──────┬───────┬─────┬─────┐
│ Step │ Not   │ Vel │ Len │  ← Lägg till längd
├──────┼───────┼─────┼─────┤
│  00  │  C-4  │ 100 │  2  │  ← Spelar i 2 steg
│  01  │  ---  │ --- │ --- │
│  02  │  E-4  │  80 │  1  │
```

### 2.2 Flera kanaler/spår
```
┌──────┬─────────────┬─────────────┐
│ Step │   Kanal 1   │   Kanal 2   │
├──────┼───────┬─────┼───────┬─────┤
│  00  │  C-4  │ 100 │  --   │ --- │
│  01  │  --   │ --- │  G-3  │  80 │
│  02  │  E-4  │  80 │  --   │ --- │
```

### 2.3 Pattern-längd
- Välj 16, 32, eller 64 steg
- Olika taktarter

### 2.4 Flera patterns
```
Pattern: [01 ▼]  [Nytt] [Kopiera] [Ta bort]
```

---

## 🎯 Fas 3: Song mode (långt senare)

**Först när patterns fungerar bra:**

```
┌─ SONG ───────────────────────────────┐
│  Position  │ Pattern                 │
├────────────┼─────────────────────────┤
│     0      │ Pattern 01 (Intro)      │
│     1      │ Pattern 02 (Vers)       │
│     2      │ Pattern 02 (Vers)       │
│     3      │ Pattern 03 (Refräng)    │
│     4      │ Pattern 02 (Vers)       │
│    ...     │ ...                     │
└────────────┴─────────────────────────┘
```

---

## Varför denna approach är bättre

| Komplex DAW-approach | Tracker-approach |
|---------------------|------------------|
| Kräver sample-exakt timing | GUI-timer räcker till att börja med |
| Behöver atomic state, lock-free queues | Enkel struct i GUI-tråden |
| Piano roll är komplext att rita | Lista med rader är enkelt |
| Många beroenden | Nästan inga beroenden |
| Veckor att implementera | Kan fungera på en dag |

**Evolutionsväg:**
```
Enkel pattern    →    Flera kanaler    →    Song mode    →    Piano roll
   (Fas 1)              (Fas 2)              (Fas 3)          (Framtid)
```

---

## Detaljerad implementation av Fas 1

### Steg 1.1: Grundstrukturer

Skapa `src/sequencer/mod.rs`:
```rust
//! Minimal tracker-style sequencer

use std::time::{Duration, Instant};

/// Ett steg i ett pattern
#[derive(Clone, Default)]
pub struct Step {
    /// MIDI-not (21-108 för piano), None = tyst steg
    pub note: Option<u8>,
    /// Velocity 0.0-1.0
    pub velocity: f32,
}

impl Step {
    pub fn new(note: u8, velocity: f32) -> Self {
        Self {
            note: Some(note),
            velocity,
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Formatera not för visning: "C-4", "F#5", "--" för tom
    pub fn note_name(&self) -> String {
        match self.note {
            Some(n) => {
                let names = ["C-", "C#", "D-", "D#", "E-", "F-",
                            "F#", "G-", "G#", "A-", "A#", "B-"];
                let octave = (n / 12) as i32 - 1;
                let name = names[(n % 12) as usize];
                format!("{}{}", name, octave)
            }
            None => "---".to_string(),
        }
    }
}

/// Ett pattern med steg
pub struct Pattern {
    pub steps: Vec<Step>,
    pub name: String,
}

impl Pattern {
    pub fn new(length: usize) -> Self {
        Self {
            steps: vec![Step::default(); length],
            name: "Pattern 01".to_string(),
        }
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }
}

/// Enkel pattern-spelare
pub struct PatternPlayer {
    pub pattern: Pattern,
    pub current_step: usize,
    pub is_playing: bool,
    pub bpm: f32,
    pub steps_per_beat: u8,
    last_step_time: Instant,
    last_note: Option<u8>,
}

impl PatternPlayer {
    pub fn new() -> Self {
        Self {
            pattern: Pattern::new(16),
            current_step: 0,
            is_playing: false,
            bpm: 120.0,
            steps_per_beat: 4,  // 16-delar
            last_step_time: Instant::now(),
            last_note: None,
        }
    }

    pub fn play(&mut self) {
        self.is_playing = true;
        self.last_step_time = Instant::now();
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
        self.current_step = 0;
    }

    pub fn step_duration(&self) -> Duration {
        Duration::from_secs_f32(60.0 / (self.bpm * self.steps_per_beat as f32))
    }

    /// Anropas varje frame, returnerar not att spela (om någon)
    pub fn update(&mut self) -> Option<StepEvent> {
        if !self.is_playing {
            return None;
        }

        if self.last_step_time.elapsed() >= self.step_duration() {
            let step = &self.pattern.steps[self.current_step];
            let event = StepEvent {
                note_off: self.last_note,
                note_on: step.note.map(|n| (n, step.velocity)),
            };

            self.last_note = step.note;
            self.current_step = (self.current_step + 1) % self.pattern.len();
            self.last_step_time = Instant::now();

            Some(event)
        } else {
            None
        }
    }
}

/// Event som returneras när ett steg spelas
pub struct StepEvent {
    pub note_off: Option<u8>,
    pub note_on: Option<(u8, f32)>,  // (not, velocity)
}
```

### Steg 1.2: GUI-vy

Skapa `src/gui/pattern_view.rs`:
```rust
//! Enkel tracker-vy för pattern

use eframe::egui::{self, Color32, RichText, Ui};
use crate::engine::EngineHandle;
use crate::sequencer::{PatternPlayer, Step};

pub struct PatternView {
    pub player: PatternPlayer,
    selected_step: Option<usize>,
}

impl PatternView {
    pub fn new() -> Self {
        Self {
            player: PatternPlayer::new(),
            selected_step: None,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, handle: &EngineHandle) {
        // Uppdatera spelare och hantera events
        if let Some(event) = self.player.update() {
            if let Some(note) = event.note_off {
                handle.note_off(note);
            }
            if let Some((note, vel)) = event.note_on {
                handle.note_on(note, vel);
            }
        }

        // Header med kontroller
        ui.horizontal(|ui| {
            ui.label("BPM:");
            ui.add(egui::DragValue::new(&mut self.player.bpm)
                .range(60.0..=200.0)
                .speed(1.0));

            ui.separator();

            if self.player.is_playing {
                if ui.button("⏹ Stop").clicked() {
                    self.player.stop();
                    // Stoppa eventuell klingande not
                    if let Some(note) = self.player.last_note {
                        handle.note_off(note);
                    }
                }
            } else {
                if ui.button("▶ Play").clicked() {
                    self.player.play();
                }
            }
        });

        ui.separator();

        // Pattern-grid
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, step) in self.player.pattern.steps.iter().enumerate() {
                let is_current = self.player.is_playing && i == self.player.current_step;
                let is_selected = self.selected_step == Some(i);

                let bg_color = if is_current {
                    Color32::from_rgb(80, 60, 20)  // Orange-ish för playhead
                } else if is_selected {
                    Color32::from_rgb(40, 50, 70)  // Blå för vald
                } else if i % 4 == 0 {
                    Color32::from_rgb(35, 38, 45)  // Mörkare var 4:e rad
                } else {
                    Color32::from_rgb(30, 33, 40)
                };

                let response = ui.horizontal(|ui| {
                    ui.painter().rect_filled(
                        ui.available_rect_before_wrap(),
                        0.0,
                        bg_color
                    );

                    // Step-nummer
                    ui.label(RichText::new(format!("{:02}", i))
                        .monospace()
                        .color(Color32::from_rgb(100, 100, 110)));

                    ui.separator();

                    // Not
                    ui.label(RichText::new(step.note_name())
                        .monospace()
                        .color(if step.note.is_some() {
                            Color32::from_rgb(255, 180, 80)
                        } else {
                            Color32::from_rgb(80, 80, 90)
                        }));

                    ui.separator();

                    // Velocity
                    let vel_str = if step.note.is_some() {
                        format!("{:3}", (step.velocity * 100.0) as u8)
                    } else {
                        "---".to_string()
                    };
                    ui.label(RichText::new(vel_str)
                        .monospace()
                        .color(Color32::from_rgb(150, 150, 160)));
                });

                // Klickbar rad
                if response.response.interact(egui::Sense::click()).clicked() {
                    self.selected_step = Some(i);
                }
            }
        });

        // Keyboard input för att sätta noter
        self.handle_keyboard_input(ui, handle);
    }

    fn handle_keyboard_input(&mut self, ui: &Ui, handle: &EngineHandle) {
        if let Some(step_idx) = self.selected_step {
            let key_map: &[(egui::Key, u8)] = &[
                (egui::Key::Z, 48), (egui::Key::S, 49), (egui::Key::X, 50),
                (egui::Key::D, 51), (egui::Key::C, 52), (egui::Key::V, 53),
                (egui::Key::G, 54), (egui::Key::B, 55), (egui::Key::H, 56),
                (egui::Key::N, 57), (egui::Key::J, 58), (egui::Key::M, 59),
                (egui::Key::Q, 60), (egui::Key::Num2, 61), (egui::Key::W, 62),
                (egui::Key::Num3, 63), (egui::Key::E, 64), (egui::Key::R, 65),
                (egui::Key::Num5, 66), (egui::Key::T, 67), (egui::Key::Num6, 68),
                (egui::Key::Y, 69), (egui::Key::Num7, 70), (egui::Key::U, 71),
            ];

            ui.input(|input| {
                // Sätt not med tangent
                for (key, note) in key_map {
                    if input.key_pressed(*key) {
                        self.player.pattern.steps[step_idx] = Step::new(*note, 0.8);
                        // Förhandsgranska noten
                        handle.note_on(*note, 0.8);
                        // Gå till nästa rad
                        self.selected_step = Some((step_idx + 1) % self.player.pattern.len());
                    }
                }

                // Ta bort not med Delete
                if input.key_pressed(egui::Key::Delete) ||
                   input.key_pressed(egui::Key::Backspace) {
                    self.player.pattern.steps[step_idx] = Step::empty();
                }

                // Navigera med piltangenter
                if input.key_pressed(egui::Key::ArrowUp) && step_idx > 0 {
                    self.selected_step = Some(step_idx - 1);
                }
                if input.key_pressed(egui::Key::ArrowDown) {
                    self.selected_step = Some((step_idx + 1) % self.player.pattern.len());
                }
            });
        }
    }
}
```

---

## Sammanfattning: Börja enkelt!

**Fas 1 är allt du behöver för att komma igång:**
1. `Step` och `Pattern` structs (~50 rader)
2. `PatternPlayer` med timer (~80 rader)
3. `PatternView` GUI (~100 rader)

**Total kod: ~230 rader** för en fungerande tracker!

Sedan kan du bygga vidare när det fungerar.

---

## ARKIV: Komplex DAW-approach (för framtida referens)

<details>
<summary>Klicka för att visa den ursprungliga komplexa planen</summary>

#### Översikt
Implementera ett sekvenser/DAW-liknande system som kan spela in, spela upp och exportera musikkompositioner. Systemet delas in i tre huvuddelar:

#### Del A: MIDI-inspelning (Note Sequencer)
Spela in noter från tangentbord/MIDI som kan spelas upp och redigeras.

**Datastrukturer:**
```rust
// src/sequencer/mod.rs
pub struct NoteEvent {
    tick: u64,              // Position i ticks (t.ex. 480 ticks per taktslag)
    note: u8,               // MIDI-not 0-127
    velocity: f32,          // 0.0-1.0
    duration_ticks: u64,    // Längd
}

pub struct Track {
    name: String,
    events: Vec<NoteEvent>,
    muted: bool,
    solo: bool,
}

pub struct Sequence {
    tracks: Vec<Track>,
    tempo: f32,             // BPM
    time_signature: (u8, u8), // t.ex. (4, 4)
    ticks_per_beat: u64,    // Upplösning, standard 480
}
```

**Komponenter:**
1. **Transport** (`src/sequencer/transport.rs`)
   - Play/Stop/Record-knappar
   - Tidslinje med playhead (nuvarande position)
   - Tempo-kontroll (BPM)
   - Loop-region (start/slut-markörer)

2. **Recording** (`src/sequencer/recorder.rs`)
   - Fånga `NoteOn`/`NoteOff` events med exakt timing
   - Använd `std::time::Instant` för high-resolution timing
   - Quantize-funktion (snäpp till närmaste 1/4, 1/8, 1/16, etc.)

3. **Playback** (`src/sequencer/player.rs`)
   - Tick-baserad uppspelning synkad med audio-callback
   - Skicka `EngineCommand::NoteOn/NoteOff` vid rätt tidpunkt
   - Loopar automatiskt om loop är aktiverad

**GUI-komponenter:**
- Piano roll-vy för att visa/redigera noter
- Tidslinje med taktstreck
- Transport-kontroller i toolbar

#### GUI-diagram och layout

**Helhetsbild - Synth med Sequencer:**
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  File  Edit  View  Help                              CPU: 12%  Voices: 3    │
├─────────────────────────────────────────────────────────────────────────────┤
│ ┌─ TRANSPORT ─────────────────────────────────────────────────────────────┐ │
│ │  [⏮] [⏹] [▶] [⏺]  │  ♩ 120.0 BPM  │  4/4  │  001:01:000  │ [🔁 Loop]  │ │
│ └─────────────────────────────────────────────────────────────────────────┘ │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │                      MODULE RACK VIEW                               │   │
│   │    ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐              │   │
│   │    │   OSC   │  │ FILTER  │  │   ENV   │  │   AMP   │              │   │
│   │    │  ~~~~   │  │  ~~~~   │  │  /\__   │  │   ○     │              │   │
│   │    └─────────┘  └─────────┘  └─────────┘  └─────────┘              │   │
│   │                                                                     │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
├─────────────────────────────────────────────────────────────────────────────┤
│ ┌─ SEQUENCER ───────────────────────────────────────────────────────────┐   │
│ │  [Piano Roll ▼]  [Snap: 1/16 ▼]  [Velocity ▼]  Zoom: [−][████][+]    │   │
│ ├───────────────────────────────────────────────────────────────────────┤   │
│ │      │    1    │    2    │    3    │    4    │    5    │              │   │
│ │      │ ┆   ┆   │ ┆   ┆   │ ┆   ┆   │ ┆   ┆   │ ┆   ┆   │  ← Tidslinje │   │
│ │      │    ▼    │         │         │         │         │  ← Playhead  │   │
│ ├──────┼─────────┴─────────┴─────────┴─────────┴─────────┤              │   │
│ │  C5  │                                                 │              │   │
│ │ ▓B4  │                                                 │              │   │
│ │  A4  │         ┌─────────┐                             │              │   │
│ │ ▓G#4 │         │         │                             │              │   │
│ │  G4  │ ┌───┐   └─────────┘       ┌───────────────┐     │  ← Piano     │   │
│ │ ▓F#4 │ │   │                     │               │     │    Roll      │   │
│ │  F4  │ └───┘         ┌───┐       └───────────────┘     │              │   │
│ │  E4  │               │   │                             │              │   │
│ │ ▓D#4 │    ┌──────────┴───┴───┐                         │              │   │
│ │  D4  │    │                  │                         │              │   │
│ │ ▓C#4 │    └──────────────────┘                         │              │   │
│ │  C4  │                                                 │              │   │
│ └──────┴─────────────────────────────────────────────────┘              │   │
│                                                                         │   │
└─────────────────────────────────────────────────────────────────────────────┘
│                           KEYBOARD (88 keys)                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Transport-kontroller (detalj):**
```
┌─ TRANSPORT ──────────────────────────────────────────────────────────────────┐
│                                                                              │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐    ┌─────────────┐   ┌───────┐   ┌───────┐ │
│  │ ⏮  │ │ ⏹  │ │ ▶  │ │ ⏺  │    │ ♩ 120.0   │   │  4/4  │   │001:01 │ │
│  │     │ │     │ │     │ │(röd)│    │ [−] BPM [+] │   │ [▼]   │   │ :000  │ │
│  │Start│ │Stop │ │Play │ │ Rec │    └─────────────┘   └───────┘   └───────┘ │
│  └─────┘ └─────┘ └─────┘ └─────┘                                            │
│                                     Tempo           Taktart    Position     │
│                                                                              │
│  ┌─ Loop ────────────────────────┐  ┌─ Metronome ─┐  ┌─ Quantize ─────────┐ │
│  │ [🔁] Start: 1:1  End: 5:1    │  │ [🔔 On/Off] │  │ [Snap: 1/16 ▼]     │ │
│  └───────────────────────────────┘  └─────────────┘  └────────────────────┘ │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘

Knapparnas funktioner:
  ⏮  = Gå till start (position 0) eller loop-start
  ⏹  = Stopp (stannar uppspelning, behåller position)
  ▶  = Play/Pause (toggle)
  ⏺  = Record (aktiverar inspelning, börjar spela om stoppat)

Keyboard shortcuts:
  Space     = Play/Pause
  Enter     = Stop och gå till start
  R         = Toggle Record
  L         = Toggle Loop
  +/-       = Tempo upp/ner
```

**Tidslinje med taktstreck (detalj):**
```
┌─ TIDSLINJE ──────────────────────────────────────────────────────────────────┐
│                                                                              │
│   Loop-region (orange markering)                                             │
│   ╔══════════════════════════════════════════╗                               │
│   ║                                          ║                               │
│ ──╫────┬────┬────┬────╫────┬────┬────┬────╫──╫──┬────┬────┬────┬────┬────┬──│
│   1    │    │    │    2    │    │    │    3    │    │    │    4    │    │   │
│   ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆    ┆   │
│   │                   │                   │                   │              │
│   └── Taktstreck ─────┴── Taktstreck ─────┴── Taktstreck ─────┘              │
│        (stort)             (stort)             (stort)                       │
│                                                                              │
│   ▼ ← Playhead (röd vertikal linje som rör sig under uppspelning)           │
│   │                                                                          │
│                                                                              │
│   Zoom-nivåer:                                                               │
│   - Utzoomad: Visar hela takter (1, 2, 3, 4...)                             │
│   - Medium:   Visar taktslag (1.1, 1.2, 1.3, 1.4, 2.1...)                   │
│   - Inzoomad: Visar 16-delar med rutnät                                     │
│                                                                              │
│   Interaktion:                                                               │
│   - Klicka: Flytta playhead till position                                   │
│   - Dra loop-markörer: Ändra loop-region                                    │
│   - Scroll-hjul: Zooma in/ut                                                │
│   - Shift+scroll: Panorera horisontellt                                     │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Piano Roll (detalj):**
```
┌─ PIANO ROLL ─────────────────────────────────────────────────────────────────┐
│                                                                              │
│  ┌─ Toolbar ────────────────────────────────────────────────────────────┐   │
│  │ [✏ Draw] [⌫ Erase] [◇ Select] │ Snap: [1/16▼] │ Vel: [100▼] │ [━━━] │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│        │    1         │    2         │    3         │    4         │        │
│  ┌─────┼──┬──┬──┬──┬──┼──┬──┬──┬──┬──┼──┬──┬──┬──┬──┼──┬──┬──┬──┬──┼────┐   │
│  │     │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ C5  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │▓B4  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ A4  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │▓G#4 │  ┆  ╔══════╗ │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ G4  │  ┆  ║ Not  ║ │  ┆  ┆  ┆  ┆  │  ╔════════════════╗  ┆  ┆  │    │   │
│  │▓F#4 │  ┆  ╚══════╝ │  ┆  ┆  ┆  ┆  │  ║   Lång not     ║  ┆  ┆  │    │   │
│  │ F4  │  ┆  ┆  ┆  ┆  │  ┆  ╔════╗┆  │  ╚════════════════╝  ┆  ┆  │    │   │
│  │ E4  │  ┆  ┆  ╔══╗  │  ┆  ║    ║┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │▓D#4 │  ┆  ┆  ║  ║  │  ┆  ╚════╝┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ D4  │  ┆  ┆  ╚══╝  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │▓C#4 │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ C4  │  ┆  ┆  ┆  ╔════════╗ ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │▓B3  │  ┆  ┆  ┆  ║        ║ ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  │ A3  │  ┆  ┆  ┆  ╚════════╝ ┆  ┆  │  ┆  ┆  ┆  ┆  │  ┆  ┆  ┆  ┆  │    │   │
│  └─────┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴──┴────┘   │
│  ┌─ Velocity Lane ──────────────────────────────────────────────────────┐   │
│  │ █    █  ██        █  ██             ███████                          │   │
│  │ █    █  ██        █  ██             ███████                          │   │
│  │ ██   █  ██       ██  ██             ███████                          │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  Not-färger baserade på velocity:                                           │
│  ████ = Hög velocity (100-127) - Stark orange                               │
│  ████ = Medium velocity (64-99) - Normal orange                             │
│  ████ = Låg velocity (1-63) - Svag orange                                   │
│                                                                              │
│  Interaktion:                                                                │
│  - Klicka (Draw): Lägg till not vid position                                │
│  - Dubbelklicka: Ta bort not                                                │
│  - Dra not: Flytta position                                                 │
│  - Dra not-kant: Ändra längd                                                │
│  - Ctrl+klicka: Välj flera noter                                            │
│  - Dra i velocity-lane: Ändra velocity                                      │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Not-representation i kod:**
```rust
struct NoteWidget {
    note_event: NoteEvent,
    rect: Rect,          // Beräknad position på skärmen
    is_selected: bool,
    is_hovered: bool,
}

// Beräkna not-position på skärmen
fn note_to_rect(event: &NoteEvent, view: &PianoRollView) -> Rect {
    let x = ticks_to_pixels(event.tick, view.zoom, view.scroll_x);
    let y = note_to_y(event.note, view.scroll_y);
    let width = ticks_to_pixels(event.duration_ticks, view.zoom, 0.0);
    let height = view.note_height;
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
}
```

---

#### Förutsättningar (måste implementeras först)

Innan sequencer-systemet kan byggas behöver följande finnas på plats:

**Fas 0: Förberedelser**

**0.1 Global tidskälla i audio-tråden**
- [ ] Lägg till `sample_counter: u64` i `SynthEngine` som räknas upp för varje sample
- [ ] Exponera `Arc<AtomicU64>` så GUI kan läsa nuvarande sample-position
- [ ] Beräkna tick från samples: `tick = (samples * ticks_per_beat * bpm) / (sample_rate * 60)`

```rust
// I SynthEngine
pub struct SynthEngine {
    // ... befintliga fält
    sample_position: Arc<AtomicU64>,  // Delad med GUI
}

impl SynthEngine {
    pub fn process(&mut self, buffer: &mut [f32]) {
        for frame in buffer.chunks_mut(2) {
            // ... befintlig processing
            self.sample_position.fetch_add(1, Ordering::Relaxed);
        }
    }
}
```

**0.2 Transport-kommandon i EngineCommand**
- [ ] Lägg till nya kommandon för sequencer-styrning:

```rust
pub enum EngineCommand {
    // ... befintliga kommandon

    // Transport
    TransportPlay,
    TransportStop,
    TransportSetPosition(u64),  // I samples

    // Sequencer
    SequencerSetTempo(f32),     // BPM
    SequencerSetLoop(Option<(u64, u64)>),  // Start/end i ticks
    SequencerLoadSequence(Arc<Sequence>),  // Ladda noter för uppspelning
}
```

**0.3 Atomic state för transport**
- [ ] Skapa delat state mellan GUI och audio-tråd:

```rust
pub struct TransportState {
    pub is_playing: AtomicBool,
    pub is_recording: AtomicBool,
    pub sample_position: AtomicU64,
    pub tempo_bpm: AtomicU32,  // Multiplicerat med 100 för decimaler (12000 = 120.00 BPM)
}
```

**0.4 Utöka befintlig EngineHandle**
- [ ] Lägg till metoder för transport:

```rust
impl EngineHandle {
    pub fn play(&self) { ... }
    pub fn stop(&self) { ... }
    pub fn set_position(&self, tick: u64) { ... }
    pub fn is_playing(&self) -> bool { ... }
    pub fn current_tick(&self) -> u64 { ... }
    pub fn set_tempo(&self, bpm: f32) { ... }
}
```

**0.5 Event-queue för inspelning**
- [ ] Skapa en lock-free queue för att skicka spelade noter till GUI:

```rust
// Noter som spelas skickas från audio-tråd till GUI för inspelning
pub struct RecordedEvent {
    pub sample_position: u64,
    pub event_type: RecordedEventType,
}

pub enum RecordedEventType {
    NoteOn { note: u8, velocity: f32 },
    NoteOff { note: u8 },
}
```

**Implementationsordning (uppdaterad med förberedelser):**

```
Fas 0: Förberedelser          Fas 1: Transport           Fas 2: Inspelning
─────────────────────         ──────────────────         ─────────────────
     │                              │                          │
     ▼                              ▼                          ▼
┌─────────────┐              ┌─────────────┐            ┌─────────────┐
│ Sample      │              │ Transport   │            │ MIDI        │
│ counter i   │──────────────▶ GUI med     │────────────▶ Recording   │
│ audio-tråd  │              │ Play/Stop   │            │ med timing  │
└─────────────┘              └─────────────┘            └─────────────┘
     │                              │                          │
     ▼                              ▼                          ▼
┌─────────────┐              ┌─────────────┐            ┌─────────────┐
│ Transport   │              │ Tick-baserad │            │ Quantize    │
│ commands i  │──────────────▶ klocka och  │────────────▶ och lagra   │
│ EngineCmd   │              │ BPM-sync    │            │ i Track     │
└─────────────┘              └─────────────┘            └─────────────┘
     │                              │                          │
     ▼                              ▼                          ▼
┌─────────────┐              ┌─────────────┐            ┌─────────────┐
│ Atomic      │              │ Tidslinje-  │            │ Uppspelning │
│ transport   │──────────────▶ visning med │────────────▶ av inspelade│
│ state       │              │ playhead    │            │ noter       │
└─────────────┘              └─────────────┘            └─────────────┘
                                                              │
                                                              ▼
                                   Fas 3: Piano Roll    Fas 4: Export
                                   ─────────────────    ──────────────
                                         │                    │
                                         ▼                    ▼
                                   ┌─────────────┐      ┌─────────────┐
                                   │ Piano roll  │      │ Audio       │
                                   │ visning av  │      │ recording   │
                                   │ noter       │      │ till WAV    │
                                   └─────────────┘      └─────────────┘
                                         │                    │
                                         ▼                    ▼
                                   ┌─────────────┐      ┌─────────────┐
                                   │ Redigering: │      │ MIDI-fil    │
                                   │ rita, flytta│      │ export      │
                                   │ ta bort     │      │ (SMF)       │
                                   └─────────────┘      └─────────────┘
```

#### Del B: Audio-inspelning (Bounce to Disk)
Spela in ljud-output till WAV-fil.

**Implementation:**
```rust
// src/audio/recorder.rs
pub struct AudioRecorder {
    buffer: Vec<f32>,       // Stereo-interleaved samples
    sample_rate: u32,
    is_recording: bool,
    max_duration_secs: f32, // Begränsa minneanvändning
}

impl AudioRecorder {
    pub fn start(&mut self) { ... }
    pub fn stop(&mut self) -> Vec<f32> { ... }
    pub fn write_sample(&mut self, left: f32, right: f32) { ... }
    pub fn save_wav(&self, path: &Path) -> Result<()> { ... }
}
```

**Integration:**
1. Lägg till `AudioRecorder` i `SynthEngine`
2. I audio-callback: om recording är aktivt, kopiera output-samples till buffer
3. Använd `hound`-crate för WAV-export (redan finns liknande kod för sampler?)

**GUI:**
- "Record Audio"-knapp (röd cirkel)
- Visuell indikator när inspelning pågår
- Dialog för att välja filnamn vid stopp

#### Del C: Arrangement/Komposition
Kombinera MIDI-tracks till en fullständig komposition.

**Utökad datastruktur:**
```rust
pub struct Project {
    name: String,
    sequences: Vec<Sequence>,    // Flera patterns/loopar
    arrangement: Vec<ArrangementClip>, // Placerade clips på tidslinje
    tempo_map: Vec<TempoChange>, // Tempo-automatisering
    patch: Patch,                // Nuvarande synth-inställningar
}

pub struct ArrangementClip {
    sequence_index: usize,  // Vilken sequence
    start_tick: u64,        // Var på tidslinjen
    length_ticks: u64,      // Kan vara kortare än original
    track_lane: u8,         // Visuell placering
}
```

#### Implementationsordning (rekommenderad)

**Fas 1: Grundläggande transport och timing**
- [ ] Skapa `src/sequencer/mod.rs` med grundläggande structs
- [ ] Implementera tick-baserad klocka synkad med sample rate
- [ ] Lägg till transport-kontroller (Play/Stop) i GUI
- [ ] Visa BPM och position i toolbar

**Fas 2: MIDI-inspelning**
- [ ] Fånga noter från tangentbord/MIDI med timing
- [ ] Spara till `Track`-struktur
- [ ] Implementera uppspelning av inspelade noter
- [ ] Lägg till Record-knapp

**Fas 3: Piano Roll GUI**
- [ ] Skapa `src/gui/piano_roll.rs`
- [ ] Visa noter som rektanglar på rutnät
- [ ] Klicka för att lägga till/ta bort noter
- [ ] Dra för att ändra längd/position

**Fas 4: Audio-export**
- [ ] Implementera `AudioRecorder`
- [ ] Real-time recording av output
- [ ] WAV-export med `hound`

**Fas 5: Projekt-hantering**
- [ ] Utöka patch-systemet till fullständiga projekt
- [ ] Spara/ladda hela kompositioner (JSON + WAV-assets)
- [ ] Export till standard MIDI-fil (SMF)

#### Tekniska överväganden

**Thread Safety:**
- Sequencer-state måste delas mellan GUI och audio-tråd
- Använd `Arc<AtomicU64>` för playhead-position
- Använd lock-free queue (som befintlig `EngineCommand`) för transport-kommandon

**Timing-precision:**
- Beräkna tick-position från sample-position: `tick = (sample / sample_rate) * (bpm / 60) * ticks_per_beat`
- Kompensera för audio-latens vid inspelning

**Minneshantering:**
- Begränsa max inspelningstid (t.ex. 30 min)
- Använd `Vec::with_capacity()` för förallokering
- Överväg memory-mapped files för långa inspelningar

#### Beroenden att lägga till
```toml
[dependencies]
hound = "3.5"  # WAV-läsning/skrivning (kanske redan finns?)
```

#### Filstruktur
```
src/
├── sequencer/
│   ├── mod.rs          # Publika exports
│   ├── transport.rs    # Play/Stop/Record-logik
│   ├── sequence.rs     # NoteEvent, Track, Sequence
│   ├── recorder.rs     # MIDI-inspelning
│   ├── player.rs       # Uppspelning
│   └── clock.rs        # Tick-timing
├── gui/
│   ├── piano_roll.rs   # Piano roll-editor
│   └── transport_bar.rs # Transport-kontroller
└── audio/
    └── wav_recorder.rs # Audio-till-disk
```

</details>

---

### Steg 2: MIDI-stöd (Högsta Prio för spelbarhet)
Just nu kan man bara spela på datorns tangentbord.
* **Implementera:** Lägg till `midir`-biblioteket.
* **Koppla:** Skapa en tråd som lyssnar på MIDI-portar och skickar `EngineCommand::NoteOn` / `NoteOff` / `SetVoiceParameter` (för rattar/CC) till motorn.

### Steg 3: Sampler-modul
Du har förberett typerna (`SamplePlayerParam`).
* **Implementera:** En `SamplePlayer`-modul som kan ladda en `.wav`-fil till minnet (i en `Arc<AudioBuffer>`) och spela upp den med pitch-tracking.
* **Utmaning:** Att ladda filen måste ske i GUI-tråden, sedan skickas bufferten till ljudtråden.

### Steg 4: Spara/Ladda hela projektet ("State")
Du har patch-systemet, men det sparar just nu bara enskilda "presets".
* **Implementera:** Spara hela grafen (alla moduler, alla kablar, alla inställningar) till en fil. Du har redan `serde` på plats, så det handlar mest om att serialisera `ModuleGraph` och `EffectChain`.

### Steg 5: Voice Control-modul (delvis löst)
För att styra globala röst-parametrar. Glide Time finns nu i toolbar, men en dedikerad modul skulle kunna innehålla:
* ~~Glide Time~~ (finns i toolbar)
* Unison Detune Amount
* Pitch Bend Range
* Polyphony Limit
