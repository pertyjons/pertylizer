# TODO - Modular Synth (v0.25.0)

## 🟢 Prioritet 1: Sampling & Audio Assets
Nu när motorn klarar flera instrument måste vi kunna spela upp samplingar (trummor, tracker-import).

- [ ] **Beroenden**
  - [ ] Lägg till `hound` (WAV) och `rfd` (File Dialog) i `Cargo.toml`.
- [ ] **Datatyper (`src/types/sample.rs`)**
  - [ ] Skapa `struct Sample` som håller ljuddata (`Arc<Vec<f32>>`), sample rate och metadata trådsäkert.
- [ ] **Laddare (`src/io/sample_loader.rs`)**
  - [ ] Implementera inläsning av WAV-filer från disk (i GUI-tråden).
- [ ] **SamplePlayer Modul (`src/modules/sample_player.rs`)**
  - [ ] Implementera en `VoiceModule` som spelar upp en `Sample`.
  - [ ] Implementera resampling (pitch-shifting) för kromatisk uppspelning.
- [ ] **Integration**
  - [ ] Lägg till `LoadSample` i `EngineCommand`.

## 🟡 Prioritet 2: Slutför Expressivitet
Vi har lagt till kommandon för Pitch Bend/Mod Wheel, men de måste implementeras fullt ut i DSP-koden.

- [ ] **Implementera Fastrand**
  - [ ] Gå igenom `Oscillator`, `Lfo`, `MathOscillator` och byt ut `NoiseState` mot `fastrand`.
- [ ] **DSP-implementering i Voice**
  - [ ] Se till att `pitch_bend` faktiskt påverkar oscillatorernas frekvens i `Voice::process_audio`.
  - [ ] Koppla `mod_wheel` till vibrato-djup (LFO -> Osc FM).
  - [ ] Koppla Velocity till Filter Cutoff.

## 🟡 Prioritet 3: Sequencer GUI
Motorn kan spela, men vi har inget gränssnitt för att skapa musiken än.

- [ ] **Transport Bar**
  - [ ] Knappar för Play, Stop, Rewind kopplade till `EngineHandle`.
  - [ ] BPM-kontroll och Loop-knapp.
- [ ] **Part Manager (Instrument Rack)**
  - [ ] En lista i GUI som visar aktiva Parts.
  - [ ] Knapp: "+ Add Instrument" (skapar ny Part).
  - [ ] Väljare för vilket ljud (Oscillator/Sampler) varje part ska ha.
- [ ] **Tracker View / Pattern Editor**
  - [ ] Implementera rendering av `Pattern` data i ett rutnät.
  - [ ] Implementera inmatning av noter.

## 🔵 Prioritet 4: Tracker Import (.MOD/.XM)
När `SamplePlayer` fungerar kan vi börja importera riktig musik.

- [ ] Skapa `src/io/import/mod_import.rs`.
- [ ] Mappa Tracker-instrument till `SynthPart` + `SamplePlayer`.
- [ ] Mappa Tracker-noter till `Pattern` data.

---

## ✅ Klart (Historik)
- [x] **Dynamisk Multitimbralitet:** `SynthEngine` har nu `Vec<SynthPart>` och kan hantera flera instrument.
- [x] **Sequencer Integration:** `SequencerEngine` körs i ljudloopen och routar noter till rätt Part.
- [x] **Stark Typsäkerhet:** `PartId`, `MidiChannel`, `Gain` används konsekvent i `EngineCommand`.
- [x] **Arkitektur:** Separation mellan GUI, Engine, Sequencer och Parts är mycket tydlig.