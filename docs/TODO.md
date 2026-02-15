# Master TODO - Modular Synth (v0.129.0)

## Fas 1: Grundläggande Funktionalitet & Workflow (Kort sikt)
*Mål: Göra klart påbörjad arkitektur och fixa de mest irriterande begränsningarna för användaren.*

1.  **Stereo Routing & Mixning** (Från nuvarande P1)
    * [x] ~~Flytta limiter/master-volym från röst-nivå till `Instrument`-nivå för att spara CPU.~~ *(0.33.6: Soft clipper på instrument-nivå)*
2.  **Solo & Mute-logik** (Från nuvarande P2)
    * [x] ~~Lägg till Solo-knapp. Implementera "Exclusive Solo" logik i mixern.~~ *(0.33.6: Solo-logik i engine, GUI behövs)*
3.  **Bypass-knappar** (Från nuvarande P2)
    * [x] ~~Lägg till Power-knapp i modul-headern för effekter.~~ *(0.33.9)*
4.  **Attenuverters & Input Gain** (Från nuvarande P2 - Kritiskt för ljuddesign)
    * [x] ~~Lägg till skalning på CV-ingångar (t.ex. LFO -> Filter) så man slipper externa VCA:er för enkla modulationer.~~ *(0.33.13: Filter CutoffMod, Oscillator FmAmount)*
5.  **Ljudinspelning / Tape Recorder** (Nytt - Prio Hög)
    * [ ] Implementera en global "Record"-knapp som streamar output till WAV-fil. Detta gör synten omedelbart användbar för produktion.

---

## Fas 2: Kreativ Expansion (Medellång sikt)
*Mål: Göra synten rolig och musikalisk att använda.*

6.  **Sampling & Audio Assets** (Från nuvarande P3)
    * [x] ~~`SampleManager` och `SamplePlayer`-modul. Detta öppnar upp för trummor och texturer.~~ *(0.33.20: SamplePlayer med pitch tracking, loop modes, interpolation)*
7.  **Patch Browser & Taggar** (Nytt - UX)
    * [ ] Byt ut fil-dialogen mot en inbyggd browser med tag-filtrering (Bass, Pad, FX).
8.  **Makro-system & Mod Matrix** (Nytt - Ljuddesign)
    * [x] ~~Mod Matrix: 8-slot modulationsrouting med 10 källor och 11 destinationer per röst~~ *(0.107.0)*
    * [x] ~~Polyfon Aftertouch som ModSource~~ *(0.121.0)*
    * [ ] Skapa 4 globala Makro-rattar som kan styra flera parametrar. Detta ersätter behovet av komplex kabeldragning för "Performance"-rattar.
9.  **Dynamic MIDI Learn** (Nytt - UX)
    * [ ] Högerklick på parameter -> "Learn MIDI CC". Nödvändigt för hårdvarukontroll.
10. **Visualiseringar: Spectrum Analyzer (FFT)** (Från nuvarande P4)
    * [x] ~~FFT-infrastruktur (FftProcessor, StftProcessor, PartitionedConvolver)~~ *(0.121.0: synth_dsp::spectral)*
    * [x] ~~Bygg visuell spectrum analyzer med FFT-infrastrukturen.~~ *(0.122.0: 2048-punkt FFT, logaritmisk frekvensaxel)*

---

## Fas 3: Workstation & Ljudkvalitet (Lång sikt)
*Mål: Förvandla synten till en professionell miljö.*

11. **Oversampling** (Nytt - Ljudkvalitet)
    * [x] ~~Implementera 2x/4x oversampling för att minska aliasing i distortion och FM.~~ *(0.122.0: Per-instrument 2x/4x med half-band FIR)*
12. **Undo / Redo** (Nytt - UX)
    * [ ] Implementera Command-historik för `PatchEditor`. Kritiskt när man bygger komplexa patchar.
13. **Sequencer GUI & Transport** (Från nuvarande P5)
    * [ ] Ny sequencer-arkitektur (skrivs från scratch efter v0.99.0 tracker-rensning)
    * [ ] Piano Roll-vy
    * [ ] Record-knapp i transport
14. **Generativa Moduler** (Nytt - Kreativitet)
    * [x] ~~Euclidean Sequencer, Turing Machine, Random Gates.~~ *(0.119.0)*
15. **Projekt-filer (.msproject)** (Från nuvarande Övrigt)
    * [ ] Spara hela sessionen (alla instrument + sequencer), inte bara enstaka patchar.

---

## Fas 4: "Nice to Have" / Nischade Features
*Mål: Specialfunktioner för specifika användare.*

16. **Live Performance View** (Nytt)
    * [ ] En förenklad vy för scenbruk (stora mätare, makron, setlist).
17. **Plugin-stöd (CLAP/VST3)** (Nytt)
    * [ ] Wrappa motorn med `nih-plug` för att köra inuti en DAW.
18. **Microtuning / Scala-filer** (Nytt)
    * [x] ~~Stöd för .scl filer för icke-västerländska skalor.~~ *(0.120.0: TuningTable med 5 presets + Scala-parser)*
19. **M/S Processing** (Från nuvarande Övrigt)
    * [x] ~~Mid/Side-läge på EQ och kompressor.~~ *(0.119.0: MidSide-effekt med width, mid/side gain)*
20. **Avancerade Visualiseringar** (Från nuvarande P4)
    * [ ] Vectorscope, Tuner, 3D-vyer.

## Fas 5: Prestanda & Optimering (Ny sektion)
21. **Cargo Workspace Refactoring**
    * [x] ~~Dela upp monolitisk crate i 6 separata crates~~ *(0.52.0)*
    * [x] ~~synth_core: Types, traits, audio abstractions~~
    * [x] ~~synth_dsp: DSP primitives~~
    * [x] ~~synth_sequencer: Pattern, song, events~~
    * [x] ~~synth_modules: Synth modules and effects~~
    * [x] ~~synth_engine: Voice allocation, graph~~
    * [x] ~~modular_synth: GUI, main, backends~~
22. **Realtime Audio Thread Safety**
    * [x] ~~Eliminera AudioBuffer::new() i Instrument::process()~~ *(0.33.10)*
    * [x] ~~Ändra Connection till PortName (Copy) istället för String~~ *(0.33.10)*
    * [x] ~~Refaktorera PolyModule::process() till InputPorts istället för HashMap~~ *(0.33.11)*
    * [ ] Eliminera kvarvarande Vec-allokering i process_module() (kräver stack-array eller arrayvec)
22. **"Baked Graph" / Graf-kompilering** (Tillagd)
    * [ ] Implementera ett "kompileringssteg" som omvandlar `ModuleGraph` (HashMap) till en linjär lista av operationer (`Vec<Op>`) och en platt minnesbuffert.
    * [ ] Mål: Eliminera alla hash-uppslagningar och pointer-jumps i ljudtråden för maximal cache-lokalitet och prestanda.
23. **Oversampling**
    * [x] ~~Stöd för 2x/4x oversampling internt i rösterna för minskad aliasing.~~ *(0.122.0)*

### Analys av förändringen
Denna nya lista prioriterar **användbarhet** (Inspelning, Browser, MIDI Learn) högre än ren teknik (Sequencer GUI, Avancerade Visualiseringar).

* **Varför flytta ner Sequencer?** Motorn har redan en sequencer, men att bygga ett bra *GUI* för den (Piano Roll) är ett enormt projekt. Det är bättre att först göra synten till ett grymt instrument som kan spelas med externt tangentbord/DAW, innan man bygger en hel DAW inuti den.
* **Varför flytta upp Inspelning?** Det är en "lågt hängande frukt" (lätt att koda) som ger enormt värde direkt ("Jag gjorde ett ljud, jag vill spara det som WAV").
