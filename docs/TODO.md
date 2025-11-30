# TODO - Modular Synth

## 🟢 Prioritet 1: Sampling & Audio Assets (Nytt Fokus)
För att kunna importera Tracker-filer och skapa beats måste vi kunna hantera ljudfiler.

- [ ] **Beroenden**
   - Lägg till `hound` (WAV) och `rfd` (File Dialog) i `Cargo.toml`.
- [ ] **Datatyper (`src/types/sample.rs`)**
   - Skapa `struct Sample` som håller ljuddata (`Arc<Vec<f32>>`), sample rate och metadata trådsäkert.
- [ ] **Laddare (`src/io/sample_loader.rs`)**
   - Implementera inläsning av WAV-filer från disk.
- [ ] **SamplePlayer Modul (`src/modules/sample_player.rs`)**
   - Implementera en `VoiceModule` som spelar upp en `Sample` med resampling (pitch-shifting) för att kunna spelas kromatiskt.
- [ ] **Integration**
   - Lägg till `LoadSample` i `EngineCommand`.
   - Lägg till `SamplePlayer` i `ModuleType` och GUI.

## 🟡 Prioritet 2: Integration av Arkitektur (The Big Rewire)
Vi har byggstenarna (`SequencerEngine`, `SynthPart`) men de måste kopplas ihop i `SynthEngine` för att ljud ska komma ut.

- [ ] **Dynamisk Multitimbralitet (Parts)**
   - [x] `SynthPart` struct definierad.
   - [ ] **Refaktorera `SynthEngine`:** Byt ut `voice_allocator` mot `parts: Vec<Box<SynthPart>>`.
   - [ ] Implementera `AddPart` / `RemovePart` kommandon.
   - [ ] Uppdatera `NoteOn` att routa till rätt Part baserat på kanal.
- [ ] **Sequencer Integration**
   - [x] `SequencerEngine` logik klar.
   - [ ] **Koppla in:** Anropa `sequencer.process()` i `SynthEngine`s ljudloop.
   - [ ] **Routing:** Se till att sequencer-events (`NoteOn`) skickas till rätt `Part` i motorn.

## 🟡 Prioritet 3: Expressivitet & Modulation
Gör synthen mer levande att spela på.

- [ ] **Modulations-kopplingar**
   - [ ] LFO → Pitch (Vibrato).
   - [ ] LFO → Amplitude (Tremolo).
   - [ ] Velocity → Filter Cutoff & Amp Level.
- [ ] **Makro-kontroller**
   - [ ] Pitch Bend & Mod Wheel stöd.
- [ ] **Byt Slumptalsgenerator**
   - [ ] Byt manuell Xorshift mot `fastrand` i `Cargo.toml` och moduler för bättre brus.

## 🔵 Prioritet 4: GUI för Musikskapande
När motorn kan spela upp låtar bygger vi gränssnittet för att skapa dem.

- [ ] **Sequencer Transport UI** (Play/Stop/BPM).
- [ ] **Part Manager UI** (Lägg till/ta bort instrument, välj ljud för varje part).
- [ ] **Tracker View / Pattern Editor**.
- [ ] **Song Mode Editor**.

## 🟣 Framtida Mål (Långsiktigt)

- [ ] **Tracker Import (.MOD/.XM)** (Kräver fungerande SamplePlayer + Sequencer).
- [ ] **Avancerade Visualiseringar** (Spectrum Analyzer, Vectorscope i separat fönster).
- [ ] **VST/Plugin-stöd**.

---

## ✅ Klart (Historik)
- [x] **Starka Typer:** `Hertz`, `Seconds`, `Gain`, `Phase` etc. implementerade.
- [x] **"Zero-Cost" Aritmetik:** Makron för smidig användning av typerna utan `.as_f32()`.
- [x] **Modul-refaktorisering:** Alla kärnmoduler (`Oscillator`, `Filter`, `Envelope`, `LFO`, `Amp`) och effekter (`Delay`, `Reverb`, `Distortion` etc) använder nu starka typer internt.
- [x] **Filstruktur:** Uppdelat i `src/engine/params/`, `src/gui/widgets/` och `src/io/`.
- [x] **IO Separation:** `PatchManager` hanterar filer.
- [x] **Sequencer Datamodell:** `Song`, `Pattern`, `Track` strukturerna är klara.