# TODO - Modular Synth

## ✅ Lösta problem (sedan 0.12.0)

1. **Allokering i Audio-loopen** - Fixat
2. **Kompileringsfel** - Fixat
3. **"The Grand Cleanup" (TypedParam)** - Genomfört
4. **Trådsäker Visualisering** - Implementerat
5. **Glide (Portamento) UI-koppling** - Implementerat (slider i toolbar)
6. **Effekter: Phaser, Flanger, Compressor, EQ** - Implementerade

---

## 🟡 Kvarstående brister

### 1. Keyboard Mapping är hårdkodad
* **Status:** Tangentbordet (Z-M, Q-I) är hårdkodat i `egui_backend.rs`.
* **Problem:** Det fungerar dåligt på icke-QWERTY layouter (t.ex. AZERTY) och går inte att ändra.
* **Prioritet:** Låg

---

## 📝 Nästa steg

### Steg 1: MIDI-stöd (Högsta Prio för spelbarhet)
Just nu kan man bara spela på datorns tangentbord.
* **Implementera:** Lägg till `midir`-biblioteket.
* **Koppla:** Skapa en tråd som lyssnar på MIDI-portar och skickar `EngineCommand::NoteOn` / `NoteOff` / `SetVoiceParameter` (för rattar/CC) till motorn.

### Steg 2: Sampler-modul
Du har förberett typerna (`SamplePlayerParam`).
* **Implementera:** En `SamplePlayer`-modul som kan ladda en `.wav`-fil till minnet (i en `Arc<AudioBuffer>`) och spela upp den med pitch-tracking.
* **Utmaning:** Att ladda filen måste ske i GUI-tråden, sedan skickas bufferten till ljudtråden.

### Steg 3: Spara/Ladda hela projektet ("State")
Du har patch-systemet, men det sparar just nu bara enskilda "presets".
* **Implementera:** Spara hela grafen (alla moduler, alla kablar, alla inställningar) till en fil. Du har redan `serde` på plats, så det handlar mest om att serialisera `ModuleGraph` och `EffectChain`.

### Steg 4: Voice Control-modul (delvis löst)
För att styra globala röst-parametrar. Glide Time finns nu i toolbar, men en dedikerad modul skulle kunna innehålla:
* ~~Glide Time~~ (finns i toolbar)
* Unison Detune Amount
* Pitch Bend Range
* Polyphony Limit
