
# 📋 Master TODO - Modular Synth (v0.33+)

## 🔴 Prioritet 1: Arkitektur & Multi-Instrument (Kritiskt)
*Mål: Få GUI och Backend att prata samma språk gällande instrument. Just nu är backend redo men GUI:t blandar ihop alla instrument i en vy.*

1.  **Per-Instrument PatchEditor (GUI Refactor)** ✅ *Klart i v0.32.19*
  * [x] Flytta ägandeskapet av `PatchEditor` från `SynthApp` till `InstrumentUiState`.
  * [x] Se till att GUI:t byter `PatchEditor`-instans när man klickar på ett nytt instrument i listan.
  * [x] Uppdatera `load_patch` så att den laddar in i *aktivt* instrument istället för att nollställa hela motorn.

2.  **Per-Instrument Voice Graph (Backend Refactor - Fas 3)** ✅ *Klart i v0.32.18*
  * [x] Implementera ändringarna i `src/engine/instrument.rs` (flytta `voice_graph` dit).
  * [x] Uppdatera `SynthEngine` så att den inte längre har en global `voice_template`.
  * [x] Se till att `AddModuleInstance` och `Connect` routas till rätt instruments graf.

3.  **Per-Instrument Effects (Fas 3.5)** ✅ *Klart i v0.32.20*
  * [x] Flytta `MasterBus` till `Instrument` och döp om till `EffectChain`.
  * [x] Varje instrument äger nu sin egen effektkedja (insert effects).
  * [x] Laddning av patch påverkar endast det instrumentets effekter.

4.  **Stereo Routing & Mixning**
  * [ ] Utred `StereoOutput`-modulens roll. Ska den ligga i varje röst (nuvarande) eller vara en fixerad del av instrumentet? (Rekommendation: Flytta limiter/master-volym till `Instrument`-nivå och låt `PolyModule`-grafen bara summera till L/R).

---

## 🟠 Prioritet 2: Best Practices & Workflow ("Pro features")
*Mål: Göra synthen smidigare att jobba med och mer kapabel för ljuddesign.*

5.  **Bypass-knappar**
  * [ ] **Effekter:** Lägg till en "Power"-knapp i modul-headern som skickar `SetBypass`.
  * [ ] **Röst-moduler:** Lägg till en `bypass`-parameter i `Filter`, `Distortion` etc. och uppdatera DSP-koden att skicka vidare input om den är aktiv.

6.  **Attenuverters (CV-skalning)**
  * [ ] Uppdatera moduler (Osc, Filter) att ha en "Input Gain"-parameter för varje CV-ingång (t.ex. `FM Amount`, `Cutoff Mod Amount`).
  * [ ] Gör det möjligt att invertera signalen (negativ gain).

7.  **Solo-funktion**
  * [ ] Lägg till "Solo"-knapp (S) bredvid Mute (M) i Instrument Rack.
  * [ ] Uppdatera mixern i `SynthEngine` att tysta alla icke-soloade instrument om något instrument är soloat.

8.  **Modul-verktyg**
  * [ ] **Init:** Högerklick på modul -> "Reset to Default".
  * [ ] **Randomize:** Högerklick -> "Randomize Parameters" (för snabb inspiration).
  * [ ] **CPU-mätare:** Visa en liten %-siffra eller stapel på varje modul (data finns redan i `ModuleCpuTracker`).

---

## 🟡 Prioritet 3: Sampling (Audio Assets)
*Mål: Kunna använda trumsamplingar och loopar.*

9.  **Sample Infrastructure**
  * [ ] Lägg till `hound` (WAV-loading) och `rfd` (File Dialog) i dependencies.
  * [ ] Skapa `SampleManager` för att ladda och cacha ljudfiler i minnet.

10. **SamplePlayer Modul**
  * [ ] Skapa en `PolyModule` som spelar upp en buffer.
  * [ ] Parametrar: Start, End, Loop, Pitch, Direction.
  * [ ] Integration i `Add Module`-menyn.

---

## 🟢 Prioritet 4: Visualiseringar
*Mål: Ge visuell feedback på vad som händer med ljudet.*

11. **Spectrum Analyzer (FFT)**
  * [ ] Lägg till `rustfft`.
  * [ ] Skapa en ny Visualizer-modul som visar frekvensspektrum.

12. **Visual Feedback i Racket**
  * [ ] **Portar:** Låt portarna lysa/blinka baserat på om det går signal genom dem (kräver att RMS-värden skickas från motorn, kan vara tungt).
  * [ ] **Kablar:** (Experimentellt) Animera kablar som har signal.

13. **Vectorscope & Tuner**
  * [ ] Implementera Vectorscope (L vs R) för att se stereobredd.
  * [ ] Implementera en enkel Tuner för att stämma oscillatorer.

---

## 🔵 Prioritet 5: Sequencer & Transport
*Mål: Göra det möjligt att bygga låtar, inte bara ljud.*

14. **Transport Bar**
  * [ ] Lägg till en panel i toppen med Play, Stop, BPM, Time Signature.
  * [ ] Koppla knapparna till `EngineCommand::Play`/`Stop`.

15. **Tracker / Piano Roll View**
  * [ ] Skapa ett GUI för att redigera `Pattern`-data som redan finns i backend.
  * [ ] Koppla detta till det aktiva instrumentet.

---

## 🟣 Övrigt / Långsiktiga Mål

16. **Projekt-filer**
  * [ ] Skapa `.msproject`-format som sparar hela "Racket" (alla instrument + sequencer), inte bara en patch.

17. **M/S Processing**
  * [ ] Lägg till Mid/Side-läge på EQ och Kompressor.

18. **Undo/Redo**
  * [ ] Implementera en Undo-stack för `PatchEditor`-ändringar.