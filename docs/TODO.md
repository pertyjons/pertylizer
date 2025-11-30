# TODO - Modular Synth (v0.26.0)

## 🟢 Prioritet 1: Sampling & Audio Assets (Högsta Prio)
Nu när motorn är uttrycksfull och multitimbral är avsaknaden av samplingar (trummor!) det enda hindret för att göra "riktig" musik.

- [ ] **Beroenden**
  - [ ] Lägg till `hound` (WAV) och `rfd` (File Dialog) i `Cargo.toml`.
- [ ] **Datatyper (`src/types/sample.rs`)**
  - [ ] Skapa `struct Sample` som håller ljuddata (`Arc<Vec<f32>>`), sample rate och metadata trådsäkert.
- [ ] **Laddare (`src/io/sample_loader.rs`)**
  - [ ] Implementera inläsning av WAV-filer från disk.
- [ ] **SamplePlayer Modul (`src/modules/sample_player.rs`)**
  - [ ] Implementera en `VoiceModule` som spelar upp en `Sample`.
  - [ ] Implementera resampling (pitch-shifting) för kromatisk uppspelning.
- [ ] **Integration**
  - [ ] Lägg till `LoadSample` i `EngineCommand`.

## 🟡 Prioritet 2: Sequencer GUI
Motorn kan spela och låter bra, men vi behöver knappar.

- [ ] **Transport Bar**
  - [ ] Knappar: Play, Stop, Rewind.
  - [ ] BPM-display och kontroll.
- [ ] **Part Manager (Instrument Rack)**
  - [ ] Lista över aktiva Parts (Instrument).
  - [ ] "+ Add Instrument"-knapp.
  - [ ] Väljare för ljudtyp (Synth/Sampler).
- [ ] **Pattern Editor / Tracker View**
  - [ ] Rita ut noter i ett rutnät/tracker-lista.

## 🔵 Prioritet 3: Framtida Mål

- [ ] **Tracker Import (.MOD/.XM)** (Kräver SamplePlayer).
- [ ] **Avancerade Visualiseringar** (Spectrum Analyzer).
- [ ] **VST/Plugin-stöd**.

---

## ✅ Klart (Historik v0.26.0)
- [x] **Expressivitet:** Pitch Bend, Mod Wheel och Velocity påverkar nu ljudet i `Voice::process_audio`.
- [x] **Bättre Brus:** Både Oscillator och LFO använder nu `fastrand`.
- [x] **Multitimbralitet:** `SynthEngine` hanterar dynamiska `Parts`.
- [x] **Sequencer Integration:** Sequencern körs i ljudloopen.
- [x] **Typsäkerhet:** `Gain`, `Hertz`, `PartId` används konsekvent.