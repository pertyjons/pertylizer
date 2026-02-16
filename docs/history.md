# Version History

## [0.137.0] - 2026-02-16
### Förbättrad — Auto-layout baserad på signalflödesanalys

**Ny 5-fas layoutalgoritm:**
- Fas 1: Klassificerar moduler i fyra grupper — SignalChain, Modulation, Global (Effect/Visualizer/Utility), Disconnected
- Fas 2: Topologisk djuptilldelning via Kahns algoritm → kolumner vänster-till-höger med longest-path
- Fas 3: Vertikal ordning inom kolumner med median-heuristik (minimerar kabelkorsningar)
- Fas 4: Modulationskällor (Envelope/LFO) placeras under sina primära signalkedjemål
- Fas 5: Pixelpositioner med fasta estimerade storlekar (ScrollArea hanterar overflow)

**Förbättringar jämfört med tidigare BFS-layout:**
- Parallella signalvägar (t.ex. två oscillatorer → mixer) hanteras korrekt
- Output-moduler tvingas till sista signalkolumnen
- Utility-moduler placeras i global-zonen (ej disconnected)
- Cykler hanteras gracefully via Kahns algoritm
- Moduler överlappar inte längre varandra

**Nya tester:** `test_multi_source_to_mixer`, `test_complex_patch`, `test_no_overlap`, `test_output_rightmost`, `test_utility_is_global`

## [0.136.0] - 2026-02-16
### Förbättrad — Moduler klipps av paneler

**Area + Frame istället för Window:**
- Moduler renderas nu med `egui::Area` + `Frame::window` istället för `egui::Window`
- Moduler hamnar i `Order::Background` (samma lager som paneler) istället för `Order::Middle`
- Varje modul-Area klipps till scroll-ytans synliga rektangel via `set_clip_rect(visible_rect)`
- Moduler som sticker ut utanför patch-editorn klipps nu av omgivande paneler

**Manuell titelrad:**
- Ersätter Windows inbyggda titelrad med rubrik + stängknapp (✕)
- Stängknapp visas bara för okopplade moduler (Disconnected)
- Kablar och toolbar-overlay renderas fortfarande i förgrunden

## [0.135.0] - 2026-02-16
### Förbättrad — ScrollArea i patch-editorn

**ScrollArea med constrain_to:**
- Patch-editorns innehåll wrappat i `egui::ScrollArea::both()` — scrollbars visas automatiskt när moduler inte ryms
- Varje modul-fönster använder `Window::constrain_to(scroll_rect)` så moduler inte kan dras utanför ytan
- Borttagen manuell canvas-panering (`canvas_offset`) — ScrollArea hanterar scrollning inbyggt
- Grid-linjer ritas relativt till scroll-ytan utan offset-beräkning
- Auto-layout fungerar direkt med scroll-rektangeln utan offset-konvertering
- Toolbar förblir synlig i förgrundslagret, positionerad relativt till den synliga ytan

## [0.134.0] - 2026-02-15
### Förbättrad — AWE perceptuellt distinkt absorption

**Perceptuella materialvärden:**
- Alla 15 materials absorptionsvärden uppjusterade från fysikaliskt exakta till perceptuellt distinkta
- Metall/kakel/betong nu tydligt hårdare; trä/vatten/tyg/matta tydligt mjukare
- Varje material har en unik klangkaraktär (t.ex. glas: tunt i basen, is: krispig HF-absorption)

**sqrt()-mappning ersätter linjär amplifiering:**
- Tar bort `ABSORPTION_AMPLIFICATION` (3.0x) från alla tre DSP-filer
- sqrt()-mappning sprider hårda material bättre utan att saturera mjuka
- Vidgade LP/HP-koefficient-ranges ger större perceptuella skillnader

**Aggressivare room modes feedback:**
- Feedback-dämpning ökad från `avg * 0.5` till `avg * 0.8` för tydligare materialskillnad

## [0.133.0] - 2026-02-15
### Fixed - AWE frekvensberoende absorption

**Frekvensberoende dämpning genom hela DSP-kedjan:**
- Ersätter `Material::average_absorption()` (en enda skalär) med per-band absorption (low/mid/high) genom alla DSP-steg
- Early Reflections: varje tap har nu separata LP- och HP-filter istället för ett enda dämpningsfilter
- Room Modes: varje combfilter har nu LP + HP i feedback-loopen
- FDN: `lp_coeff` beräknas från `absorption_high`, `hp_coeff` från `absorption_low`
- Absorption Amplification-faktor (3.0x) för att sprida små fysikaliska skillnader till hörbara filterskillnader
- Olika material (betong, metall, glas, trä, tyg) ger nu markant olika klangkaraktär

**Fixar i Spatializer (huvud-skuggning):**
- Inverterade head shadow-koefficienter: one_pole med coeff 1.0 = full LP (inte pass-through)
- Nära örat får nu coeff ≈ 0 (pass-through), avlägset öra får högre coeff (mer HF-dämpning)

**Fixar i tester:**
- Alla awe_engine-tester uppdaterade med korrekta newtype-wrappers (NormalizedValue, SampleRate, Meters, etc.)
- presets-test fixat med `.as_f32()` konvertering

## [0.132.0] - 2026-02-15
### Added - Kinetic Modulator

**Ny modulationsmodul: Kinetic Modulator**
- Gate-triggad kontrollmodul som genererar Position/Velocity/Acceleration via Penner easing-kurvor
- 10 easing-kurvor: Linear, QuadOut, CubicOut, QuartOut, QuintOut, ExpoOut, CircOut, BackOut, ElasticOut, BounceOut
- Alla kurvor med analytiskt beräknade derivator (velocity och acceleration)
- Konfigurerbar duration (0.01–10s), overshoot (Back/Elastic), bipolar-läge
- Loop-lägen: OneShot, Loop, PingPong
- Retrigger-stöd vid ny note-on

**Mod Matrix: 3 nya modulationskällor**
- KineticPos — position från easing-kurvan
- KineticVel — normaliserad hastighet (derivata)
- KineticAcc — normaliserad acceleration (andra derivatan)
- ModSource utökad från 11 till 14 varianter

**Nya patches**
- "Kinetic Pluck" — CubicOut-kurva (150ms) driver filter och amplitud
- "Kinetic Pad" — ElasticOut-kurva (2s) i loop-läge för evolverande padljud

## [0.131.0] - 2026-02-15
### Changed - Isometrisk 3D AWE-vy med ljudanimationer

**Isometrisk 3D-rendering (cutaway-stil):**
- Ersätter 2D planritning med isometrisk 3D-vy
- Cutaway-stil: bakvägg, högervägg och golv synliga, framväggar utelämnade
- Solid skuggning: golv mörkast, väggar ljusare med alpha-transparens
- Alla 6 rumsformer renderas isometriskt: Box, Cylinder, L-Shape, Sphere, Dome, Tube
- Dimensionslabels placerade längs isometriska golvkanter

**Expanderande ljudringar:**
- Animerade ringar expanderar från källan på golvplanet
- Ny ring var 0.5s, max 6 samtidiga, minskar i opacity med ålder
- Ringar ritas som isometriska ellipser (48 punkter)
- Reflektionsringar spawnas från spegelkällor vid väggar (Box/Tube)

**Animerade reflektionslinjer (marching ants):**
- Streckade reflektionslinjer animeras med löpande offset
- Strecken flödar kontinuerligt S → vägg → L
- Ersätter statiska streckade linjer

**Uppdaterad interaktion:**
- Drag använder invers isometrisk projektion (screen_to_floor)
- Markörer (S/L) placeras via iso_to_screen på golvplanet
- Spatial mapping dots projiceras isometriskt

## [0.130.0] - 2026-02-15
### Changed - AWE-vy: Förbättrad grafisk representation

**Formspecifik rumskontur i planritningen:**
- Box: rektangel (som tidigare)
- Cylinder: rektangel med rundade kortsidor
- L-Shape: L-formad polygon
- Sphere: cirkel
- Dome: cirkel med streckad undre halva (halvsfär)
- Tube: rektangel med streckade öppna kortsidor

**Reflektionsvägar:**
- Första ordningens reflektioner visas som streckade linjer (källa → vägg → lyssnare)
- Box: 4 reflektioner (alla väggar), Tube: 2 reflektioner (lång­sidorna)
- Beräknas med spegelkällemetoden

**Info-ruta i planritningen:**
- Visar avstånd (S→L), RT60 (Sabines formel), och rumsvolym
- Halvtransparent bakgrund för läsbarhet

**Förbättrade markörer:**
- Större cirklar (14px) med outline-ring
- Pil från källa till lyssnare (triangelformat pilhuvud)
- Hover-text "Källa" / "Lyssnare" vid markörerna

**Förenklad kontrollpanel:**
- LFO-sektioner ihopfällbara (stängda som default) via CollapsingHeader
- "Impossible" omdöpt till "Effekter" med undertext "Effekter bortom fysiken"
- Undertext "Balans mellan torr/våt signal" under Mix-rubriken
- Tooltips på alla parametrar: Dry/Wet, Early/Late, Modes, Tail, Freq Warp, Resonance, Portal, Diffusion

## [0.129.0] - 2026-02-15
### Added - AWE-indikator, Oscilloskop vid piano & AWE newtype-migrering

**AWE-indikator i toolbar:**
- Knappen visar grön prick (●) när AWE-effekten är aktiv, dämpad/grå när av.
- Fungerar fortfarande som vy-växlare (AWE/Rack).

**Master-output oscilloskop:**
- Vänster och höger kanal visas som oscilloskop bredvid pianot i bottenpanelen.
- Tar automatiskt det utrymme som blir över om pianot inte fyller hela bredden.
- Döljs om skärmen är för smal (< 120px kvar).
- Cyan färg för vänster kanal, grön för höger.

**VisualizationBuffer i EngineState:**
- Ny `master_scope` buffer i `EngineState` för master-output waveform-data.
- `SynthEngine` skriver final output (efter master volume) till buffern varje callback.

### Changed - synth_awe: Komplett newtype-migrering

**Ny `types`-modul** med 7 AWE-lokala newtypes:
- `Meters`, `SquareMeters`, `CubicMeters` — rumdimensioner och ytor/volymer.
- `MetersPerSecond` — ljudhastighet.
- `SampleOffset` — fraktionell sample-position för interpolerade delay-lines.
- `StretchFactor` — tail stretch (0.5–4.0, clampat).
- `Position3` — 3D-position `[Meters; 3]` med `x()/y()/z()` accessors.

**Migrering från råa primitiver till typade domänvärden** i hela craten:
- `f32` → `Meters` (alla rumsdimensioner i `RoomShape`, `EarlyReflections`, `RoomModeBank`, `SpatialContext`, `Spatializer`).
- `f32` → `NormalizedValue` (absorption, diffusion, dry/wet, portal amount, LFO amount m.fl.).
- `f32` → `Gain` (feedback, tap gains).
- `f32` → `FilterState` (one-pole filter states).
- `f32` → `SampleOffset` (delay tap positioner).
- `f32` → `SampleRate`, `Seconds`, `Hertz`, `BipolarValue` (alla publika API:er).
- `f32` → `StretchFactor` (tail stretch parameter).
- `[f32; 3]` → `Position3` (käll-/lyssnarpositioner).
- `usize` → `SampleCount` (delay-buffertstorlekar, block sizes).
- `u8` → `MidiNote` (per-röst spatialisering).

**DSP-förbättringar (utan beteendeändringar):**
- One-pole filter: manuell beräkning ersatt med `FilterState::one_pole()`.
- Gain-applicering: manuell multiplikation ersatt med `Gain::apply()`.
- Hot-path-optimering: filter-state och mix-parametrar hissade till lokala variabler före per-sample-loop.
- Magiska siffror ersatta med namngivna konstanter (`PORTAL_MAX_DELAY`, `PORTAL_MAX_DELAY_SAMPLES`).

**Alla 36 presets** uppdaterade med `.into()`-konvertering.
**GUI (awe_view.rs)** uppdaterad med `.as_f32()`-extraktion och newtype-wrapping vid gränssnittet mot sliders.

## [0.128.0] - 2026-02-15
### Added - AWE: Nya material, fler presets, diffusion & LFO-stabilitet

**9 nya material:**
- Marble, Ice, Carpet, Water, Void, Prism, Plasma, Membrane, Nanogel.
- Kreativa/icke-fysiska material (Void, Prism, Plasma, Membrane, Nanogel) för extrema ljuddesigner.

**36 presets (upp från 14):**
- 22 nya presets: bl.a. Basaltklyfta, Aurorahall, Gravitationstunnel, Regnrum, Svävande Kör, Spegelplan, Kristallvalv.
- 6 "EXT:"-presets med extrema material: Singularitet, Plasmastorm, Prismaspiral, Membranhåla, Nanodimma, Antigrav.
- Befintliga presets finjusterade (realistiskare dimensioner och positioner).
- Preset-menyn uppdelad i Standard / Extreme-sektioner.

**Materialdiffusion i DSP:**
- `Material::diffusion` påverkar nu FDN-diffusion (0.35 + diffusion × 0.55).
- Tidiga reflektioner: per-tap jitter baserat på diffusion samt reducerade riktningscues.
- Delay-buffert utökad till 1.0 s (stöd för rum upp till ~170 m).

**LFO-stabilitet (base value tracking):**
- `base_room` och `base_snapshot` sparar användarens inställda värden.
- LFO:er återställer basvärden före varje modulations-pass — eliminerar drift.

**Buffertförstoringar:**
- FDN: pre-allokering ×48 (från ×32) för stora rum med tail stretch.
- Room modes: max delay 48 000 samples (från 5 000).

**GUI:**
- 15 material i materialväljaren (från 6).
- Bättre material-matchning med flerbands-jämförelse.

**Övrigt:**
- `Instrument::process_visualizers()` anropas efter effektkedjan.
- Borttagen `docs/AWE-Implementation-Review.md`.

## [0.127.0] - 2026-02-15
### Added - AWE: Nya rumsformer + Preset-meny

**Nya rumsformer (3 st):**
- **Sphere**: Sfäriskt rum (alla dimensioner = diameter). Sammanfallande moder ger fokuserad resonans.
- **Dome**: Halvsfär (höjd = radie, bredd/längd = diameter). Kupol-reflektioner.
- **Tube**: Öppet rör utan ändlock. Mindre yta ger längre RT60 och flutterekos.
- Korrekta geometriformler (volym, ytarea, axiella moder) för alla nya former.
- LFO-modulering av RoomLength/RoomWidth fungerar med alla 6 former.

**AWE Preset-meny (14 presets):**
- Ny preset-väljare i AWE-toolbaren med hover-beskrivningar.
- 14 kreativa presets: Katedral, Badrum, Grotta, Pipeline, Konserthall, Sci-Fi Korridor, Dröm, Underjorden, Industrihall, Liten Studio, Rymdstation, Bergseko, Kupol, Portal.
- Presets demonstrerar alla 6 rumsformer, alla material, och Impossible-parametrar.
- Val av preset laddar fullständigt AWE-tillstånd (rum, material, mix, LFO:er, spatial).
- Manuella ändringar nollställer preset-valet.

**GUI:**
- RoomShapeKind utökad med Sphere, Dome, Tube.
- Dimensionssliders för alla nya former (radius, length).
- `restore_from()` och `to_awe_state()` hanterar alla 6 former korrekt.
- Fixat höjdberäkning för source/listener z-position (använder effektiv rumshöjd).

## [0.126.0] - 2026-02-15
### Added - AWE Fas 3: Per-röst Spatialisering

**Per-röst rumspositionering:**
- Varje aktiv röst kan tilldelas en egen position i rummet baserat på MIDI-not.
- 4 mappningslägen: Off, Linear X, Linear Y, Circular.
- Individuella tidiga reflektioner (ISM) per röst med egna `EarlyReflections`-instanser.
- Individuell spatializer (ITD/ILD) per röst.
- Delad FDN-reverb och rumsmoder matade av summerad mono.

**SpatialVoiceBank & SpatialVoicePool:**
- Pre-allokerad bank med 16 mono-buffertar (4096 samples var) - ~1.3 MB totalt.
- Per-röst DSP-pool med 16 slots: `EarlyReflections` (16K delay) + `Spatializer`.
- `NotePositionMapping` enum med `position_for_note()` och `pan_for_note()`.
- `SpatialContext` struct för att kommunicera spatial-kontext till instrument.

**Instrument per-röst capture:**
- `Instrument::process()` tar nu emot `SpatialContext` och `SpatialVoiceBank`.
- Per-röst mono-capture till spatial bank i både normal och oversampled path.
- Per-röst dry panning baserat på notens position relativt lyssnaren.

**GUI:**
- Ny "Spatial"-sektion i AWE-kontrollpanelen med On/Off-toggle och Mapping-väljare.
- Visualisering av not-positioner som svaga prickar i floor plan.

**Persistence:**
- `spatial_enabled` och `note_mapping` i AweSnapshot, AweState och patch-format.
- Bakåtkompatibel deserialisering via `#[serde(default)]`.

**Ny konstruktor:**
- `EarlyReflections::with_max_delay()` för anpassningsbar delay-storlek.

## [0.125.0] - 2026-02-15
### Added - AWE Fas 2: Avancerad Geometri & Kreativa Features

**Nya rumsformer:**
- Cylinder-rum (pipeline/tunnel mode) med radie och längd.
- L-format rum (två sammankopplade rektanglar) med individuella dimensioner.
- Nya `RoomShape`-varianter med volume/surface_area/axial_modes-stöd.
- `DEFAULT_CYLINDER` (r=1m, L=20m) och `DEFAULT_LSHAPE` (8×5 + 6×4, H=3m) konstanter.

**"Omöjliga rum"-parametrar:**
- Freq Warp: Modulerar FDN LP-damping — positiv ger mer HF-reverb.
- Resonance Boost: Adderar energi till FDN-feedback (klampad vid 0.97).
- GUI-sliders i ny "Impossible"-sektion.

**Akustisk portal:**
- Extra stereo delay-path med feedback som simulerar angränsande virtuellt rum.
- One-pole LP-damping för mumlande portalljud.
- Portal Amount-kontroll (0–1) med smooth ramping.
- `PortalAmount` som nytt LFO-target.

**4 interna LFO:er (utökat från 2):**
- LFO 3 & 4 med Rate/Amount/Target-kontroller.
- 13 modulations-targets (utökat från 8): +EarlyLate, ModesAmount, ResonanceBoost, TailStretch, PortalAmount.
- Full GUI med 4 LFO-sektioner.

**GUI-förbättringar:**
- Rumsform-väljare (ComboBox: Box/Cylinder/L-Shape).
- Dimensionssliders anpassade per rumsform.
- Floor plan anpassad till effektiva dimensioner oavsett form.
- Slider-range utökat till 100m för Box (stödjer "The Void"-preset).

## [0.124.0] - 2026-02-15
### Added - AWE Fas 1: Parametriskt Rum

**Early Reflections (ISM):**
- Image Source Method med 6 taps (en per vägg i rektangulärt rum).
- Fraktionella delays via `InterpolatedDelayLine`, per-tap one-pole LP-damping.
- Avståndsberoende gain (1/r), pan baserat på speglad källas X-offset.
- Automatisk geometriuppdatering vid rum-/positionsändringar.

**FDN Late Reverb (geometridrivet):**
- `FdnCore`-baserad sen reverb med parametrar härledda från rumsgeometri.
- RT60 via Sabines formel, feedback-gain beräknad från RT60.
- Delay-tider skalade efter rumsdimensioner, damping från materialabsorption.

**Room Mode Bank (comb-filter):**
- 3 axiella rumsmoder (längd/bredd/höjd) som parallella comb-filter.
- Feedbackstyrka och damping beräknade från absorption.
- Modes amount-kontroll (0–1) för blandning.

**Spatializer (ITD + ILD):**
- Interaural time difference via två `InterpolatedDelayLine` (max 64 samples).
- Interaural level difference via equal-power panning.
- Head shadow one-pole LP per öra, mer dämpning på avskärmade örat.

**AWE-interna LFO:er:**
- 2 kontroll-rate LFO:er (sine, 0.01–20 Hz) som körs per block.
- 8 targets: RoomLength, RoomWidth, SourceX/Y, ListenerX/Y, DryWet, FreqWarp.
- Smooth 5 ms parameter-ramping i DSP-loopen.

**2D Floor Plan GUI:**
- Top-down planritning med rum-outline, dimensionslabels.
- Dragbara källa (S) och lyssnare (L) markörer.
- Material-väljare (6 presets).
- Sliders: Dry/Wet, Early/Late, Modes, Tail Stretch.
- LFO 1 & 2: Rate, Amount, Target-väljare.

**Ny params:**
- `AweLfoTarget` enum och `AweLfoState` struct för LFO-persistence.
- `AweSnapshot` utökad med `lfo1`/`lfo2` fält.
- `AweParam` utökad med Lfo1Rate/Amount/Target, Lfo2Rate/Amount/Target.
- `RoomShape` dimension-accessors: `length()`, `width()`, `height()`.
- `SPEED_OF_SOUND` som publik konstant i `room.rs`.

## [0.123.0] - 2026-02-14
### Added - AWE (Acoustic World Engine) Fas 0 — Infrastruktur

**FDN-extraktion (synth_dsp):**
- Ny `fdn`-modul med `FdnCore` struct — extraherad från Reverb.
- 8-kanal FDN med Hadamard-matris, per-kanal damping/lowcut, modulerade delay-linjer.
- `FdnCore::process_sample()` tar mono in, returnerar stereo `FdnStereoOutput`.
- Reverb delegerar nu till `FdnCore` — alla befintliga tester passerar.

**synth_awe crate (ny):**
- `AweEngine`: Pass-through processor (ingen DSP i Fas 0).
- `AweParam`: Enum med RoomShape, Material, SourcePos, ListenerPos, DryWet, m.fl.
- `AweSnapshot`: Copy-struct för batch-uppdatering av numeriska parametrar.
- `AweState`: Serde-serialiserbar state för patch-persistence.
- `RoomShape`: Box-variant med length/width/height, volume(), surface_area(), axial_modes().
- `Material`: 6 konstanter (CONCRETE, WOOD, GLASS, METAL, FABRIC, TILE) med frekvens-beroende absorption.

**Engine-integration:**
- Tre nya `EngineCommand`-varianter: `SetAweParameter`, `SetAweEnabled`, `SetAweState`.
- `AweEngine`-fält i `SynthEngine`, processas efter master effects.
- Master-level visualizers körs nu efter AWE (visar slutsignal).

**GUI:**
- Ny `AppView::AcousticWorld` variant med toggle-knapp i menyraden.
- Placeholder AWE-vy med enable/disable toggle.

**Persistence:**
- `PatchSettings.awe: Option<AweState>` — sparas/laddas automatiskt.

## [0.122.0] - 2026-02-14
### Added - Vosim, Spectrum Analyzer, Oversampling

**Vosim (MathOscillator):**
- Ny `Vosim` algoritm i MathOscillator — klassisk röstsyntes via kvadrerade sinuspulser.
- 3 parametrar: Formant (1x-20x basfrekvens), Decay (0.3-0.99), Pulser (1-6 st).
- Ger vokalliknande ljud, perfekt för pads och experimentella texturer.

**Spectrum Analyzer (Visualizer):**
- Ny visualizer-modul som visar frekvensspektrum via FFT (2048-punkt).
- Logaritmisk frekvensaxel (20Hz-20kHz) med dB-skala.
- Hann-fönster, mono (L+R)/2 analys, Gain-parameter för vertikal skalning.
- Grid-linjer vid 100Hz, 1kHz, 10kHz + etiketter.
- Full patch save/load-stöd.

**Oversampling (2x/4x):**
- Per-instrument oversampling för minskad aliasing från oscillatorer och waveshapers.
- Ny `OversamplingFactor` (Off/2x/4x) i instrument rack UI.
- 11-tap half-band FIR anti-aliasing filter (~60dB stopband rejection).
- 4x använder kaskaderade 2x-steg (4x → 2x → 1x).
- Pre-allokerade buffertar — noll overhead vid Off (1x), inga heap-allokeringar.
- Röster processas vid högre sample rate, effektkedjan kör vid originalrate.

## [0.121.0] - 2026-02-14
### Added - Polyfon Aftertouch, Granulär Syntes, Convolution Reverb, Phase Vocoder & FFT-infrastruktur

**Polyfon Aftertouch:**
- Ny `PolyAftertouch` modulationskälla i mod matrix (per-ton tryck).
- Separat från kanal-aftertouch — varje röst har eget aftertouch-värde.

**FFT-infrastruktur (synth_dsp):**
- Ny `spectral`-modul med `realfft`-baserade verktyg.
- `FftProcessor`: Pre-allokerad real FFT wrapper med forward/inverse.
- `StftProcessor`: Overlap-add STFT med ring buffer och spectral callback.
- `PartitionedConvolver`: Uniform partitioned convolution för långa impulse responses.
- `WindowType`: Hann, Hamming, Blackman-Harris fönsterfunktioner.

**Granulär Syntes (GranularOsc):**
- Ny PolyModule med 32 simultana grains (fixed-array, ingen heap i process).
- 5 källvågformer: Saw, Sine, Square, Triangle, Noise.
- 3 fönstertyper: Hann, Gaussian, Trapezoid.
- Parametrar: GrainSize, Density, Position, PositionSpread, PitchSpread, PanSpread.
- Freeze-mode för att låsa grain-position.
- RT-säker xorshift32 PRNG.

**Convolution Reverb (Convolver):**
- Ny AudioEffect med partitioned FFT-convolution (stereo).
- 4 matematiskt genererade impulse responses: Plate, Room, Spring, Hall.
- Pre-delay (0-200ms), Decay Trim, Brightness (one-pole LP) och Mix.
- Automatisk IR-rebuild vid parameterändringar.

**Phase Vocoder:**
- Ny AudioEffect med STFT-baserad pitch shifting (stereo).
- Pitch shift: -24 till +24 halvtoner med fas-ackumulering.
- Spectral freeze-mode (håller nuvarande spektrum).
- Konfigurerbar FFT-storlek: 512, 1024, 2048, 4096.

## [0.120.0] - 2026-02-14
### Added - Cross-Modulation, FDN Reverb, Sidechain & Microtonal Tuning

**Oscillator — Cross-Modulation:**
- Ny `cross_mod` audio-ingång på varje oscillator för bilateral FM mellan oscillatorer.
- `CrossModAmount`-parameter (0.0-1.0) styr modulationsdjup.
- Kombineras med befintlig FM-ingång för komplex frekvensmodulering.

**FDN Reverb (Feedback Delay Network):**
- Helt ny 8-kanals FDN-implementation ersätter tidigare Schroeder-reverb.
- Hadamard-mixningsmatris (8×8) för maximal energispridning.
- Primtalsbaserade delay-tider (2039-3511 samples) skalade med Size-parameter.
- Per-kanal LFO-modulerade delay-tider (~0.3 Hz) för tätare reverb.
- Frekvensberoende dämpning: lowpass (Damping) och highpass (LowCut) per kanal.
- Pre-delay upp till 500ms, Decay- och Diffusion-kontroller.
- Stereo-output med konfigurerbar Width.

**Compressor — Sidechain:**
- Ny `SidechainEnabled`-toggle och `SidechainFilter`-parameter (20-500 Hz HPF).
- `set_sidechain_input()` för extern sidechain-signal som detektionskälla.
- One-pole highpass-filter på sidechain för att isolera transient-detektion.

**Microtonal Tuning:**
- Nytt `TuningTable`-system med [Hertz; 128] MIDI note → frekvens-mappning.
- 5 inbyggda presets: Equal Temperament (12-TET), Just Intonation, Pythagorean, 19-TET, 31-TET.
- Scala-parser: stöd för .scl (skalfiler) och .kbm (keyboard mapping).
- `TuningPreset`-enum med `ALL`, `name()`, `id()`, `from_id()`, `to_table()`.
- Integration i Voice: `set_tuning_table()` ersätter statisk frekvenstabell.

## [0.119.0] - 2026-02-14
### Added - 10 nya ljudtekniker: MSEG, BBD Delay, Additive Synth, Generativa moduler m.m.

**Nya effekter (AudioEffect):**
- **BBD Delay**: Analog bucket-brigade delay-emulering med kompander (tanh), bandbreddsbegränsning, wow & flutter LFO, clock noise och feedback med per-repeat mörkläggning.
- **Brickwall Limiter**: Look-ahead limiter med true peak detection, konfigurerbart ceiling/release, soft knee och gain reduction metering.
- **Mid/Side Processing**: Stereo breddkontroll med M/S encoding, width (0-2x), mid/side gain och mix.

**Nya voice-moduler (PolyModule):**
- **MSEG (Multi-Stage Envelope Generator)**: 16-segments envelope med per-segment time/level/curve, loop-stöd, sustain-segment och tempo-sync. Preset-mallar: ADSR, Tremolo, Sidechain Pump.
- **Additive Oscillator**: 32 harmoniska med spektral profil (Tilt, Odd/Even, Brightness), spektral stretch för inharmonicitet, fas-randomisering vid note-on.
- **Euclidean Sequencer**: Björklund-algoritm för jämnt fördelade pulspar. Steps (1-32), pulses, rotation, swing. Gate + accent CV-output.
- **Turing Machine**: 16-bit skiftregister för semi-slumpmässiga melodier. Mutation rate (locked→evolving→random), skalkvantisering (chromatic/major/minor/pentatonic), pitch CV + gate output.
- **Random Gates**: Probabilistisk gate-generator med density, burst mode, konfigurerbar gate-längd. Gate + random CV output.

**GUI-integration:**
- Alla nya effekter tillgängliga i Effect-menyn och Master Effects-panelen med fullständiga parameter-sliders.
- Additive Oscillator i Oscillator-submenyn.
- MSEG i Modulation-submenyn.
- Ny "Generative"-submeny med Euclidean, Turing Machine och Random Gates.
- Patch-serialisering (save/load) stöds för alla nya modultyper.

## [0.118.0] - 2026-02-14
### Added - 8 nya kreativa patchar med Ring Mod, Envelope Follower & Wavetable
- **Vocal Pad**: Eterisk körliknande pad med Formant-wavetable och LFO-driven vokal-morphing genom vowel-shapes.
- **Metallic Bell**: Skimrande metallisk klocka — Digital wavetable genom Ring Mod med keyboard tracking skapar inharmoniska övertoner.
- **Auto-Wah Bass**: Funky auto-wah bass där Envelope Follower trackar speladynamik och driver Acid-filtrets cutoff i realtid.
- **Digital Chime**: Kristallint chime-ljud med Digital wavetable och envelope-driven position-sweep genom komplexa vågformer.
- **Warm Evolving**: Långsamt evolverande ambient-textur med Warm wavetable, mycket långsam LFO-scanning och djup reverb.
- **Harmonic Lead**: Expressiv lead med Harmonics wavetable — envelope sveper från enkel sinuston till 32 harmoniska på varje ton.
- **Ring Mod Drone**: Djupt evolverande drone — LFO modulerar Ring Mod carrier-frekvens för kontinuerligt skiftande sidband.
- **PWM E-Piano**: Varmt elpiano med PWM wavetable, envelope-driven pulsbredd-sweep, Fluid-filter och klassisk chorus.

## [0.117.0] - 2026-02-14
### Improved - Kategoriserade undermenyer för Example Patches
- **Kategoriserad patch-meny**: Example Patches-menyn visar nu patchar i 8 undermenyer grupperade efter kategori istället för en platt lista med 35 patchar.
- **Kategorier**: Keys & Piano, Bass, Lead, Pad, Drums, Strings & Bell, Experimental, Ambient & Texture.
- **Ny funktion `categorized_patches()`**: Returnerar patchar grupperade per kategori, används av menyn. `example_patches()` finns kvar som platt lista.

## [0.116.0] - 2026-02-13
### Added - Våg 1: Ring Modulation, Envelope Follower, Wavetable Syntes
- **Ring Modulation**: Ny voice-modul som multiplicerar insignal med intern carrier-oscillator. Stöder 5 vågformer (sine/tri/saw/square/pulse), keyboard tracking, frekvensratio (0.25x-4.0x), dry/wet mix och freq CV-ingång. Ger metalliska klockljud, sci-fi-texturer och inharmoniska övertoner.
- **Envelope Follower**: Ny voice-modul som trackar amplituden av en insignal och producerar en kontrollsignal (0.0-1.0). One-pole filter med separata attack/release-koefficienter och sensitivity-kontroll. Användbar för auto-wah, sidechain-liknande effekter och dynamisk modulering.
- **Wavetable Syntes**: Ny voice-modul med 6 inbyggda wavetable-banker:
  - **Basic**: Sine → Triangle → Saw → Square morph (64 frames)
  - **Harmonics**: 1→32 harmoniska additiv syntes (32 frames)
  - **PWM**: Pulsbredd 50%→5% (32 frames)
  - **Formant**: Vokalformanter a/e/i/o/u med interpolation (32 frames)
  - **Digital**: FM, hard sync, bitcrush, ring mod-liknande (32 frames)
  - **Warm**: Mjuka analoga varianter med harmonisk saturation (32 frames)
- **Wavetable-scanning**: Position-parameter (0.0-1.0) med CV-modulering för timbral rörelse.
- **GUI-integration**: Wavetable i Oscillator-menyn, Ring Mod och Env Follower i ny "Modulation"-submeny.
- **Patch-serialisering**: Alla tre moduler stöder save/load med PatchModuleType.

## [0.115.0] - 2026-02-13
### Added - Character Filters (Analog filtermodeller)
- **FilterModel-parameter**: Ny Model-väljare i Filter-modulen med 4 alternativ: Standard, Fluid, Screamer, Acid.
- **Fluid** (Oberheim-inspirerad): SVF med normaliserad tanh-saturation och kontinuerlig Morph-kontroll (LP→BP→HP→Notch) via constant-power crossfade.
- **Screamer** (MS-20-inspirerad): Sallen-Key HP→LP-kaskad med asymmetrisk diod-clipping i feedbackloopen. Aggressiv, skrikande resonans.
- **Acid** (Steiner-Parker-inspirerad): ZDF 2-pol med resonansberoende variabel saturation (tanh→sine-fold blend). Stöder LP/BP/HP-modes.
- **Morph-knob**: Ny parameter för Fluid-modellen som smidigt korsfadar mellan filterutgångar.
- **Realtidssäkert**: Alla filterstructs är Copy med enbart f32-fält, inga heap-allokeringar.
- **Patch: Fluid Pad**: Evolverande pad med Fluid-filtrets morph svept av LFO, 5-voice triangle unison och reverb.
- **Patch: Fluid Keys**: Varmt elpiano med Fluid-morph som skapar klockliknande övertoner via envelope-sweep, chorus.
- **Patch: Screamer Lead**: Aggressiv lead med Screamer-filtrets diod-clipping, hög resonans och snabb envelope-sweep.
- **Patch: Acid Bass**: Klassisk 303-acid med Acid-filtrets variabla saturation, square-oscillator och snabb cutoff-sweep.

## [0.114.0] - 2026-02-13
### Added - Intra-voice Unison i Oscillator
- **Unison-röster**: 1-7 detunade oscillatorkopior inuti varje röst, ger fett unison-ljud utan att multiplicera röstkostnaden.
- **Parametrar**: Unison (antal röster), Uni Detune (0-100 cent spridning), Uni Spread (stereo-panorering), Uni Phase (fasrandomisering vid note-on).
- **Stereo-utgångar**: Nya `out_l` och `out_r`-portar med constant-power panorering för stereo-unison. Mono `out`-porten är bakåtkompatibel.
- **Realtidssäkert**: Alla arrayer är fixed-size [T; 7], ingen heap-allokering i process(). Fasrandomisering via lock-free fastrand.
- **n=1 specialfall**: När unison är av (1 röst) skippas panorering helt, exakt samma beteende som innan.
- **Patch: Unison Supersaw**: Klassisk trance-supersaw med 7-voice unison, filter-envelope och chorus.
- **Patch: Stereo Unison Pad**: Bred ambient-pad med triangle-unison via stereo-utgångar (out_l/out_r) direkt till amp.
- **Patch: Unison Sync Lead**: Aggressiv hard-sync lead med tight 3-voice mono-unison och waveshaper.
- **Patch: Unison PWM Strings**: Lush strängensemble med puls-PWM, 5-voice stereo-unison, chorus och reverb.

## [0.113.0] - 2026-02-13
### Added - Waveshaper-modul
- **Waveshaper-effekt**: Ny kreativ waveshaping-modul med 6 kurvor: Soft Clip, Asymmetric, Fold, Chebyshev, Sine Fold, Quantize.
- **Parametrar**: Curve (kurvval), Drive (exponentiell 1x-20x), Mix (dry/wet), Bias (DC-offset), Symmetry (asymmetrisk kontroll).
- **GUI-integration**: Tillgänglig i effektpaletten, master-bus dropdown, och patch-editor.
- **Patch: Waveshaper Lead**: Skarp lead med Sine Fold-kurva, saw-oscillator och filter-envelope-modulation.
- **Patch: Glitch Pad**: Evolverande pad med Fold-kurva, triangle-oscillator och LFO-filtermodulation.

## [0.112.0] - 2026-02-13
### Changed - Mod Matrix: enabled-checkbox & Amount-label
- **Enabled-checkbox**: Varje slot har nu en checkbox bredvid Amount-knoben för att aktivera/inaktivera sloten.
- **Kortare knob-label**: Knoben visar nu "Amount" istället för "Slot X Amount".
- **SlotEnabled-param**: Ny parameter `SlotEnabled` per slot styr om modulering är aktiv.

## [0.111.0] - 2026-02-13
### Fixed - Mod Matrix Grid-layout
- **Lika stora slots**: Alla slots i rutnätet har nu samma fasta bredd, beräknad från tillgängligt utrymme och antal kolumner.
- **Amount-etikett innanför ramen**: "Slot X Amount"-texten under knoben hamnar nu helt innanför gruppramens kant.
- **Dynamisk ComboBox-bredd**: Dropdowns anpassar sin bredd till slotens storlek istället för fast 80px.

## [0.110.0] - 2026-02-13
### Changed - Mod Matrix Grid-redesign
- **Grid-baserad layout**: Mod Matrix renderas nu som ett rutnät istället för en platt lista. Grid-storlek väljs via selectbox: 1×1, 2×2 (standard), 3×3, 4×4 — ger 1, 4, 9 eller 16 slots.
- **16 slots**: Max antal slots utökat från 8 till 16 (4×4 grid).
- **Borttagen enabled-toggle**: Slots med Source=None är automatiskt inaktiva, separat on/off-toggle borttagen.
- **Kompakta celler**: Varje cell i rutnätet visar Source- och Destination-dropdowns samt Amount-knob.
- **Grid size-param**: Ny `GridSize`-parameter styr hur många slots som processas och visas.

## [0.109.0] - 2026-02-13
### Changed - Konsekvent modulnamngivning
- **Alltid siffersuffix**: Alla moduler visar nu alltid instansnummer (t.ex. "LFO 1", "Oscillator 1", "Filter 1") även om det bara finns en modul av den typen. Selectboxar matchar nu alltid modulnamnen i vyn.

## [0.108.0] - 2026-02-13
### Improved - Smarta modulnamn & filtrerade Mod Matrix-val
- **Numrerade modultitlar**: Har man 2+ moduler av samma typ visas "Oscillator 1" / "Oscillator 2". En ensam modul visas utan nummer ("LFO").
- **Filtrerade Mod Matrix-dropdowns**: Sources och destinations som refererar till moduler som inte finns i patchen döljs. T.ex. "LFO 2" visas inte som källa om bara en LFO finns, "Osc 2 Pitch" döljs utan en andra oscillator.
- **PatchAnalysis**: Ny intern analysstruktur som räknar modultyper per frame och driver både namngivning och filtrering.

## [0.107.0] - 2026-02-13
### Added - Mod Matrix (8-slot modulationsrouting)
- **Mod Matrix modul**: Nytt 8-slot modulationsroutingsystem som lever i varje röst
- **10 modulationskällor**: None, LFO 1/2, Env 1/2, Velocity, Note Number, Aftertouch, Mod Wheel, Pitch Bend
- **11 modulationsdestinationer**: None, Osc 1/2 Pitch, Osc 1 Level, Filter 1/2 Cutoff, Filter 1 Reso, Amp Level/Pan, LFO 1 Rate/Depth
- **Bipolär amount**: Varje slot har -1.0 till +1.0 skalning med knob-kontroll
- **Enable/disable per slot**: Toggle för att aktivera/inaktivera enskilda slots
- **Voice-integration**: Modulering appliceras automatiskt innan grafprocessning, med one-block latency (~1ms)
- **Mod offsets i destinationsmoduler**: Filter, oscillator, amplifier och LFO stödjer nu moduleringsoffsets
- **GUI**: Mod Matrix tillgänglig via "Mod Matrix" knapp i modulpaletten (Utility-kategori)
- **Patch-serialisering**: Mod Matrix-inställningar sparas och laddas med patches

## [0.106.0] - 2026-02-13
### Changed - Klaviatur visar fler tangenter vid bredare fönster
- **Fasta tangentstorlekar**: Vita tangenter 24px, svarta 14px — storleken ändras aldrig
- **Fler tangenter vid bredd**: Bredare fönster visar fler av de 88 tangenterna, centrerade om alla ryms
- **Villkorlig scroll**: Scroll och scroll-indikatorer visas bara när alla tangenter inte ryms

## [0.105.0] - 2026-02-13
### Changed - Resizable modulfönster
- **Resizable modulfönster**: Moduler kan nu dras bredare/smalare med `resizable(true)`. Mittinnehållet fyller tillgänglig bredd, OUT-portar sitter alltid mot högerkanten.

## [0.104.0] - 2026-02-13
### Fixed - Kompakta modulfönster
- **Auto-fit höjd**: Ersatt `StripBuilder::horizontal` (som expanderade celler till full tillgänglig höjd) med `ui.horizontal` + `ui.vertical` — modulfönster anpassar nu höjden till sitt innehåll utan dödyta
- **Fast modulbredd**: Moduler expanderar inte längre med huvudfönstret — content-kolumnen använder `set_min_width` istället för `available_width()`
- **Icke-resizable fönster**: Modulfönster är nu `resizable(false)` och auto-fitar alltid till innehållet
- **Borttaget `StripBuilder`-beroende**: `egui_extras::StripBuilder` och `Size` används inte längre i modullayouten

## [0.103.0] - 2026-02-13
### Changed - StripBuilder för modullayout
- **StripBuilder-layout**: Ersatt manuell `ui.horizontal()` + gap-fill med `egui_extras::StripBuilder` för tre-kolumnslayouten (IN | innehåll | OUT). Portkolumner använder `Size::exact()` och mittinnehållet `Size::remainder()`, vilket ger exakta kolumnbredder utan fragil gap-beräkning.
- **Högerkolumn flush**: OUT-portar sitter nu garanterat flush mot modulens högerkant tack vare StripBuilders fasta kolumnstorlekar.
- **Förenklad `draw_port_column`**: Borttaget manuellt `set_min_width`/`set_max_width` — StripBuilder hanterar kolumnbredden.

## [0.102.0] - 2026-02-11
### Changed - Ny modullayout: portar på sidorna
- **Tre-kolumnlayout**: Portar renderas nu i vertikala kolumner till vänster (IN) och höger (OUT) om modulinnehållet, istället för i en horisontell sektion mellan header och parametrar. Minskar modulhöjden och eliminerar dött utrymme.
- **Portkolumner**: 28px breda kolumner med portar centrerade vertikalt, labels visas som tooltips vid hover
- **Effekt/Visualizer-moduler**: Behåller full bredd utan portkolumner (har inga portar)
- **Ökad minsta modulbredd**: 140px → 180px (28px portkolumn + 100px innehåll + 28px portkolumn + marginaler)
- **Nya theme-konstanter**: `port_column_width`, `port_vertical_spacing`, `module_content_min_width` i `Sizes`
- **Auto-layout uppdaterad**: `MIN_MODULE_WIDTH` ökad till 180px

## [0.101.0] - 2026-02-11
### Fixed - 3 GUI↔Engine-buggar
- **Kabelbortkoppling (KRITISK)**: `connections_to_remove` från patch editor processerades aldrig — Disconnect-kommandon skickas nu till engine
- **Bypass för voice-moduler (HÖG)**: `SetBypass` sökte bara i effect chain. Bypass stöds nu i `ModuleGraph` (voice graph) med nollställda outputs för bypassed moduler
- **SetTempo (LÅG)**: `EngineCommand::SetTempo` fångades av catch-all `_ => {}` — kopplas nu till `TransportState::set_tempo()`
- Tog bort onåbar catch-all (`_ => {}`) i command-matchning nu när alla varianter hanteras

## [0.100.0] - 2026-02-11
### Removed - Död kod och icke-fungerande GUI-element
- **Mixer-vy**: Hela placeholder-vyn (8 dummy-faders + "coming soon"-text), `AppView::Mixer`-variant, vy-selektorn i top bar
- **layout.rs**: Oanvänd alternativ top bar-implementation (290 rader), `TopPanel`-enum
- **Sample-dialog**: `OpenSample`-variant, `open_sample_dialog()`-metod, matcharm i fildialogshanteringen (aldrig anropad, handler var TODO)
- **"Audio settings coming soon"**: Placeholder-sektion i Settings-dialogen
- **MIDI Refresh-knapp**: Knapp i MIDI-dropdown som bara stängde menyn utan att göra något
- **MultiPointEnvelope**: Tracker-specifika envelope-typer (`MultiPointEnvelope`, `EnvelopePoint`, `EnvelopeType`)
- **Tracker-referenser**: Kvarvarande död kod och importer relaterade till borttagen tracker-funktionalitet
- **Sample-kod**: `SamplePlayer` pitch mod, `WaveformOverview`, `PlaybackPositionBuffer`, hound-referens i ARCHITECTURE.md

## [0.99.0] - 2026-02-10
### Removed - All tracker import functionality
- **Beslut**: Efter v0.87–v0.98 (2 dagar, 9 buggfixar) insåg vi att tracker-uppspelning (XM/MOD/S3M via xmrs) inte passar arkitekturen. Syntmotorn är polyfonisk/semitone-baserad medan tracker kräver period-baserad pitch med tight-kopplad effektprocessering. Varje fix avslöjade nya buggar.
- **Borttaget**: Tracker-import (`TrackerImporter`), tracker-effektprocessering (`tracker_effects.rs`, ~2200 rader), tracker-patterns (`tracker_pattern.rs`, ~670 rader), tracker-effekttyper (`effects.rs`), tracker-vyer, tracker-specifika fält i Voice/SynthEngine/Song, Sequencer GUI-vy, alla tracker-analysexempel (9 st), alla tracker-referensdokument
- **Behållet**: Grundläggande `SequencerEngine` (enkel NoteOn/NoteOff-uppspelning för piano-patterns), `synth_sequencer` crate med Pattern/Song/Note-typer
- **Taggat**: `v0.98.0-tracker-experiment` — sista versionen med tracker-kod
- **Se**: `docs/tracker-experiment-summary.md` för fullständig analys och framtida alternativ

## [0.98.0] - 2026-02-09
### Fixed - Vibrato/arpeggio appliceras felaktigt vid tick 0
- **Problem**: `ChannelEffectState.current_tick` nollställdes aldrig vid radstart. Fältet behöll värdet från förra radens sista `process_tick()` (t.ex. tick 4 vid speed=5). När `current_modulation()` anropades vid tick 0, var `current_tick.as_u8() > 0` sant, vilket fick vibrato att appliceras — trots att FT2 använder `realPeriod` (noll vibrato-offset) vid tick 0.
- **Konsekvens**: Varje rad med vibrato hade en felaktig pitch-offset vid tick 0. Med vibrato depth=1 kunde offset vara upp till ±12.45ct (ca 1/8 halvton) istället för 0ct. I FT2 fungerar tick 0 som en "ankarpunkt" där pitchen återgår till basnoten, men vår kod hade en slumpmässig vibrato-offset (beroende på fas från förra radens sista tick). Med 113 vibrato-effekter i en enda pattern lät detta genomgående "falskt".
- **Fix**: Sätter `state.current_tick = TickInRow::ZERO` i `process_row_start()` direkt efter att kanalens state hämtas. Vibrato-checken `current_tick > 0` i `current_modulation()` evalueras nu korrekt till false vid tick 0.
- **Verifiering**: Debug-output visar nu `pitch=+0.00ct` vid tick 0 för alla vibrato-rader (bekräftat med `debug_playback`).
- **Påverkan**: Alla XM/MOD/S3M-filer med vibrato (4xx), vibrato+volslide (6xx), eller arpeggio-effekter. Arpeggio tick-cykel (tick % 3) var också felaktig vid tick 0.

## [0.97.0] - 2026-02-09
### Fixed - TonePortamento 4x för långsam i Amiga-läge
- **Problem**: `apply_amiga_tone_portamento()` delade `tone_porta_speed` med 4.0 innan den användes som period-steg. Kommentaren hävdade "FT2 Amiga tone portamento uses raw_param", men FT2-källkoden visar att `portaSpeed = param << 2` (multiplicerar med 4 vid setup) och sedan använder `portaSpeed` direkt i `tonePorta()` — exakt samma som vanlig portamento.
- **Konsekvens**: Tone portamento (3xx/5xx) gick 4x långsammare än i FT2. Med param=16 (effekt 310) och speed=5 ger FT2 64 period-enheter/tick × 4 ticks = 256 perioder/rad = 4 halvtoner. Vår kod gav 16 period-enheter/tick × 4 ticks = 64 perioder/rad = 1 halvton. Tone slides nådde aldrig sina targets i tid, vilket resulterade i hörbart falska toner.
- **Fix**: Tog bort `/4.0` divisionen i `apply_amiga_tone_portamento()`. `tone_porta_speed` (= raw_param × 4.0) används nu direkt som period-steg, identiskt med `apply_amiga_portamento()`.
- **Verifiering**: Jämförelse med FT2-clone källkod (`ft2_replayer.c`: `tonePorta()` + `getNewNote()`) bekräftar att `portaSpeed` används utan division.
- **Påverkan**: Alla XM/MOD/S3M-filer med Amiga-frekvensläge och TonePortamento-effekter (3xx, 5xx). 567 TonePortamento-effekter i testfilen `joli_untouched.xm`.

## [0.96.0] - 2026-02-09
### Fixed - Extra effekt-tick per rad (25% för mycket portamento/vibrato/volume slide)
- **Problem**: `process_tick()` anropades 5 gånger per rad istället för 4 vid speed=5. I FT2 med speed=5 finns det 5 ticks (0-4): tick 0 hanteras av `process_row_start()`, ticks 1-4 hanteras av `process_tick()`. Men vår engine anropade `process_tick()` var 40:e song-tick, och med 200 song-ticks per rad (5×40) blev det 200/40 = 5 anrop istället för 4.
- **Konsekvens**: Alla kontinuerliga effekter fick **25% mer effekt per rad** (5/4 = 1.25x):
  - PortamentoDown(5): -156.25ct/rad istället för -125.00ct/rad
  - Vibrato: fasen avancerades 25% snabbare → snabbare och bredare vibrato
  - Volume slide: volymförändring 25% snabbare
  - Tone portamento: glide nådde mål 25% snabbare
  - **Över 2 rader blev portamento-driften -312.50ct istället för -250.00ct — en skillnad på 62.5ct (~0.6 halvtoner), tydligt hörbara "falska toner".**
- **Fix**: I `process_tick()`, om `tick_in_row >= speed`, returneras nuvarande modulation utan att applicera effekter. Den 5:e process_tick-iterationen (tick_in_row=5 vid speed=5) skippas nu korrekt.
- **Påverkan**: Alla XM-filer med kontinuerliga effekter (portamento, vibrato, tremolo, volume slide, panning slide).

## [0.95.0] - 2026-02-09
### Fixed - pitch_offset läcker från PortamentoUp till TonePortamento
- **Problem**: `apply_amiga_portamento()` (1xx/2xx) ackumulerade pitch-ändring i `pitch_offset`, medan `apply_amiga_tone_portamento()` (3xx) arbetade på `current_pitch`. I FT2 finns bara en periodvariabel, men vår kod har två separata (`current_pitch` + `pitch_offset`). När TonePortamento följde efter PortamentoUp absorberades inte den ackumulerade `pitch_offset`, vilket orsakade att den lades ovanpå TonePortamentos resultat.
- **Konsekvens**: Med PortamentoUp(2) på 5 ticks ackumulerades +62.5ct i `pitch_offset`. När TonePortamento sedan slidde `current_pitch` mot målnoten (t.ex. E-6 = 88.0), hamnade slutpitchen på 88.0 + 0.625 = 88.625 halvtoner — **62.5 cent för högt**. Vibrato oscillerade sedan runt denna felaktiga pitch.
- **Fix**: I `process_row_start`, när TonePortamento detekteras, absorberas befintlig `pitch_offset` in i `current_pitch` och `pitch_offset` nollställs. TonePortamento slider sedan från korrekt startposition.
- **Påverkan**: Alla mönster där PortamentoUp/Down (1xx/2xx) följs av TonePortamento (3xx).

## [0.94.0] - 2026-02-09
### Fixed - Tone Portamento target en rad försenad
- **Problem**: I XM-importen (`process_track_unit_to_cell`) uppdaterades `last_porta_target` EFTER att effekterna processades. TonePortamento (3xx) fick därmed föregående rads not som target istället för den aktuella radens not.
- **Konsekvens**: I uppspelningskoden satte `trigger_note` korrekt `tone_porta_target` till aktuell not, men sedan överskrev effektprocesseringen med det felaktiga (gamla) targetvärdet. Portamento-slidet startade alltid en rad för sent, och första raden med portamento gav ingen tonändring alls.
- **Fix**: Pitch beräknas och `last_porta_target` uppdateras nu INNAN effektloopen körs, så att TonePortamento alltid får rätt target-not.
- **Påverkan**: Alla kanaler med TonePortamento-effekter (3xx) i importerade XM/MOD/S3M-filer.

## [0.93.0] - 2026-02-09
### Enhanced - Utökad Debug-knapp i Sequencer-vyn
- **Kanal mute-status**: Visar aktiv/MUTED-status för varje track
- **Instrument defaults**: Visar volym och panning per instrument (matchar `analyze_tracker`-format)
- **Fullständig pattern-grid**: Skriver ut hela pattern-innehållet i samma format som `analyze_tracker`-exemplet (noter, instrument, volym, effekter)
- **Effektsammanfattning**: Räknar och listar alla använda effekter i aktiv pattern sorterade efter frekvens
- Debug-output är nu direkt jämförbar med `cargo run --example analyze_tracker`

## [0.92.0] - 2026-02-07
### Fixed - Tremolo, vibrato och volume slide noggrannhet

#### Tremolo-djup ~4x för svagt
- **Problem**: `TremoloDepth::from_param` använde `depth / 64.0` men FT2-formeln är `(waveform_peak * depth) >> 6` där peak=255. Resultatet: tremolo var ~4x för svag och nästan ohörbar.
- **Fix**: Ny formel `depth * 255.0 / 64.0 / 64.0` matchar FT2:s djupskala.

#### Vibrato-offset appliceras på tick 0
- **Problem**: Vibrato-offset beräknades och applicerades på alla ticks inklusive tick 0. FT2 nollställer vibrato-offset på tick 0 (`outPeriod = realPeriod`) och kör inte `doVibrato` förrän tick 1+.
- **Fix**: Vibrato-offset appliceras nu bara på ticks 1+. Vibrato/tremolo-fas avanceras också bara på ticks 1+.

#### Volume slide prioritetsregel felaktig
- **Problem**: `SlideRate::from_volume_slide` subtraherade `up - down`, men FT2 ger övre nibble (UP) prioritet när båda är icke-noll. Samma bugg i `from_panning_slide`.
- **Fix**: Ny prioritetslogik — om `up > 0` ignoreras `down` (och vice versa för panning slide).

### Added - Effekt-noggrannhetsanalys
- Ny fil `docs/effect-accuracy-analysis.md` — genomgående analys av alla tracker-effekter mot FT2-referens
- Ny fil `docs/references/ft2-effect-reference.md` — komplett FT2-effektreferens med exakta formler och C-kod från ft2-clone

## [0.91.0] - 2026-02-07
### Fixed - Continuous effects läcker mellan rader

#### Continuous effects (volume slide, portamento, vibrato, etc.) stoppas inte mellan rader
- **Problem**: Effekter som volume slide, portamento, vibrato, tremolo och panning slide fortsatte köra på rader där de inte angavs. State resättades aldrig — bara uppdaterades NÄR effekten fanns. Resultatet: effekter "läckte" och körde oändligt tills en ny note med fresh attack triggades.
- **Rotorsak**: `process_row_start()` processade effekter i en match-loop men resetade aldrig continuous state innan loopen. XM effect memory (param=0 = "continue") blandades ihop med "effekten är aktiv".
- **Fix**: Continuous effect state resättas nu INNAN effect-loopen varje rad. Effect memory sparas i lokala variabler och återställs bara NÄR effekten faktiskt finns på raden (med param=0). Nytt `tone_porta_active`-fält skiljer "aktiv denna rad" från "har minnesvärde".

#### Berörda effekter
- **Volume slide (Axy)**: Stoppas på rader utan Axy
- **Portamento up/down (1xx/2xx)**: Stoppas på rader utan 1xx/2xx
- **Tone portamento (3xx)**: Stoppas på rader utan 3xx/5xx (nytt `tone_porta_active`-fält)
- **Vibrato (4xy)**: Depth nollställs på rader utan 4xy (fas bevaras)
- **Tremolo (7xy)**: Depth nollställs på rader utan 7xy
- **Panning slide (Pxy)**: Stoppas på rader utan Pxy
- **Fine volume slide (EAx/EBx)**: Stoppas på rader utan EAx/EBx
- **Arpeggio (0xy)**: Stoppas på rader utan 0xy
- **Retrigger (E9x)**: Stoppas på rader utan E9x
- **Note cut/delay/fadeout**: Rensas varje rad

#### Känd begränsning
- XM effekt 5xy (TonePortamento+VolumeSlide) och 6xy (Vibrato+VolumeSlide) med param 0 hanteras inte fullt korrekt av xmrs-biblioteket — det droppar VolumeSlide(0,0). Med denna fix stoppas volume slide korrekt, men 500/600 "continue both" fortsätter bara den första sub-effekten.

## [0.90.0] - 2026-02-07
### Fixed - Portamento-konvertering i Amiga-läge (och linjärt läge)

#### Portamento ~41x för snabb och inverterad riktning (KRITISK)
- **Problem**: Portamento-effekter (1xx/2xx) glider ~41x för snabbt och i fel riktning. Tre buggar samverkar:
  1. Import multiplicerar med `* 16.0` istället för att dividera med `/ 4.0` för att återställa raw param
  2. Import inverterar riktning (xmrs negativ = porta UP, inte DOWN)
  3. Effektprocessorn konverterar speed felaktigt med `/ 100.0 * 64.0` istället för att använda direkt
- **Fix**: Import återställer nu raw param korrekt (`speed.abs() / 4.0`), riktning fixad, effektprocessorn använder period-enheter direkt i Amiga-läge

#### Tone portamento felaktig skalning
- **Problem**: Tone portamento (3xx) skalas felaktigt i både import och effektprocessor
- **Fix**: Import hanterar nu Amiga/Linear separat. Effektprocessorn konverterar korrekt: Amiga `/4.0`, Linear `*1200.0/768.0/100.0`

#### Linjär portamento felaktig konvertering
- **Problem**: Linjär portamento adderar period-enheter direkt som cents, utan konvertering
- **Fix**: Konverterar nu period-enheter till cents med `* 1200.0 / 768.0`

#### Debug-verktyg (analyze_tracker_raw)
- Fixad riktning och skalning i `format_track_effect` för Portamento och TonePortamento

## [0.89.0] - 2026-02-07
### Fixed - XM Speed Effect Silence & GUI Row Sync

#### Tystnad vid dynamisk speed-ändring (KRITISK)
- **Problem**: XM-moduler som byter speed med Fxx-effekt (t.ex. F03 = speed 3) orsakade tystnad efter halva patternet. Pattern-placeringarna beräknades vid import med default speed (6, 240 ticks/rad), men dynamisk speed 3 (120 ticks/rad) processade alla rader dubbelt så snabbt — halvvägs genom tick-fönstret var alla rader klara och resten blev tyst.
- **Fix**: Ny metod `auto_advance_if_past_pattern()` i sequencer-motorn som detekterar när alla rader processats och hoppar direkt till nästa pattern. Noter bevaras över pattern-gränser (ingen release).

#### GUI tracker-rad ur synk vid speed-ändring
- **Problem**: Sequencer-vyn använde statisk `ticks_per_row` (240) för att beräkna vilken rad som visas. Med dynamisk speed 3 gick GUI:t i halv hastighet och bröt vid halva patternet.
- **Fix**: Dynamisk `ticks_per_row` delas nu från audio-tråden till GUI:t via `TransportState` (atomisk). GUI:t beräknar raden med `offset / dynamic_ticks_per_row` istället för patternets statiska värde.

## [0.88.0] - 2026-02-07
### Changed - Solo/Mute per kanal & Modulnamn i toolbar

#### Solo/Mute-knappar i Sequencer
- **Solo (S):** Inte längre togglebar — klick mutar alla andra kanaler och unmutar den valda
- **Mute (M):** Ny knapp per kanal — togglebar individuell mute med röd/grå indikering
- **"Unmute All"-knapp** i toolbarn för att snabbt ta bort alla mutar
- Ersatt `solo_track: Option<TrackId>` med `muted_tracks: Vec<bool>` genom hela stacken (state, engine, commands)
- Flera kanaler kan nu vara unmutade samtidigt (inte begränsat till en solo-kanal)

#### Modulnamn i toolbar
- Modulnamn (song.name) visas nu i sequencer-toolbarn efter Debug-knappen

#### Solo/Mute i Rack-vyn
- Solo-knappen i instrument-racket fungerar nu som i sequencern: klick mutar alla andra instrument
- Ny **"Unmute All"-knapp** i instrument-rackens header
- Borttagen toggle-baserad solo-state (`InstrumentUiState::solo` används ej längre för solo-toggle)

## [0.87.0] - 2026-02-06
### Fixed - XM Playback & Tracker Display Improvements

#### Tracker velocity fix (KRITISK)
- **Problem**: `Velocity::FF` (112/127 = 0.882) användes för tracker NoteOn istället för `Velocity::MAX` (1.0), vilket gav 12% volymreduktion
- **Fix**: Ändrat till `Velocity::MAX` i `sequencer_engine.rs` för tracker-läge

#### Vibrato fasbevarande (FT2-kompatibilitet)
- **Problem**: Vibrato-fas återställdes till noll vid varje ny not, men FT2 bevarar fasen mellan noter
- **Fix**: Borttagen `vibrato_phase = Phase::ZERO` från `trigger_note()` i `tracker_effects.rs`

#### GUI tracker display
- **Fix**: `SetVolume`-effekten separeras nu till volymkolumnen istället för att visas som "C08" i effektkolumnen
- Borttagen redundant effektkolumn — XM har bara 1 effektkolumn, `fasttracker()` config ändrad från 2→1
- Volymkolumnen visar nu bara hex-värde (t.ex. `08`) utan "C"-prefix

#### Analysverktyg
- Ny `analyze_tracker_raw.rs` — visar rå xmrs-representation (PatternSlot, TrackUnit)
- Omskriven `analyze_tracker.rs` — visar intern representation (Cell, EffectCommand)
- Ny `debug_playback.rs` — tick-för-tick uppspelningslogg
- Raw-analyzern separerar nu Volume-effekter till volymkolumnen (matchar intern representation)
- Regenererade alla debug-filer i `docs/debug/`

#### CLAUDE.md
- Dokumenterat debug- och analysverktyg, debug-output, GUI debug-knapp
- Instruktion att nya tekniska referenser ska sparas i `docs/references/` med uppdaterad README.md

## [0.86.0] - 2026-02-06
### Added - Sequencer Debug Button & Tracker Analysis Column Fix

#### Debug-knapp i sequencer toolbar
- Ny "Debug"-knapp i sequencerns toolbar (till höger om Pattern-navigering)
- Skriver ut song-info, aktiv pattern-data och arrangement till konsolen
- Visar: song-namn, BPM, speed, frequency mode, antal tracks/patterns/instruments
- Aktiv pattern: namn, rader, kanaler, ticks per row, antal noter/noteoffs/effekter
- Arrangement: alla pattern-placeringar med pattern-ID, track-ID och tick-position
- Matchar format från `analyze_tracker.rs` för enkel jämförelse

#### Fixad kolumnformatering i analyze_tracker
- Effektkolumnen paddas nu till 4 tecken (`{:<4}`) i `format_cell()`
- Löser inkonsekvent bredd: 3-teckens effektkoder (t.ex. `F70`) vs 4-teckens tom (`....`)
- Alla celler har nu konsekvent 14 teckens bredd, kolumner ligger rakt

## [0.85.0] - 2026-02-06
### Fixed - XM Playback Bugs (Double Volume, Fine Volume Slide Memory, Amiga Portamento)

Tre buggar som påverkade XM-uppspelningskvaliteten (identifierade via joli_suspiria.xm och joli_untouched.xm):

#### Bugg 1: Dubbel volymapplicering (KRITISK)
- **Problem**: Sample `default_volume` multiplicerades in BÅDE via `SamplePlayer.level` OCH via tracker_volume kanal-modulation → `output × vol²` istället för `output × vol`
- **Fix**: Borttagen `.with_default_volume()` från tracker sample-import (`tracker.rs`). Volym hanteras enbart via `TrackerInstrumentDefaults` → channel modulation
- **Effekt**: Korrekt volymbalans, inga onaturligt tysta instrument

#### Bugg 2: FineVolumeSlide saknade effektminne (MEDEL)
- **Problem**: `EA0`/`EB0` (fine volume slide med param=0) gjorde ingenting, borde återanvända senaste slide-värde
- **Fix**: Nytt fält `fine_volume_slide: SlideRate` i `ChannelEffectState` med effektminne — param 0 fortsätter med föregående värde
- **Effekt**: Gradvisa fade-in/out i untouched.xm fungerar korrekt (1586 fine volume slides)

#### Bugg 3: Amiga portamento i linjärt rum (MEDEL)
- **Problem**: XM-filer med `AmigaFrequencies` körde portamento i cents-rum (linjärt), men Amiga-mode ska använda period-rum (hyperboliskt)
- **Fix**:
  - Ny enum `TrackerFrequencyMode { Linear, Amiga }` i `synth_sequencer::song`
  - Lagras i `Song.tracker_frequency_mode`, sätts vid import
  - `ChannelEffectProcessor.amiga_mode` vidarebefordras från `SequencerEngine`
  - Period-baserad portamento: `period = 7680 - (semitones - 12) * 64`, ger karaktäristisk icke-linjär pitch-kurva
  - Påverkar PortamentoUp/Down och TonePortamento

## [0.84.0] - 2026-02-06
### Improved - Enhanced Tracker Analysis Tool & History Date Corrections

#### analyze_tracker.rs — Komplett omskrivning
- **Full pattern grid dump**: Tracker-stil format med NOT IN VL EFCT-kolumner per kanal, hex radnummer
- **Panning envelope**: Visar panning envelope-punkter, sustain, loop (pitch envelope finns ej i xmrs)
- **Full pattern order**: Visar hela pattern-ordningen utan trunkering
- **Helper-funktioner**: `format_note()`, `format_cell()`, `format_track_effect()`, `format_global_effect()`
- **Effects summary**: Räknar alla effekttyper över alla patterns, sorterat efter frekvens
- **Extra debug-info**: Key-off-räkning, keymap-intervall för multi-sample instrument, sample format/finetune/panning, frekvenstyp

#### docs/history.md — Datumkorrigeringar
- Ersatt 130+ "2025" årsangivelser med faktiska YYYY-MM-DD datum från git-historiken
- 8 versioner utan git-commits (0.64, 0.65, 0.47, 0.49, 0.33.13, 0.33.17, 0.24, 0.13.2) fick interpolerade datum

## [0.83.0] - 2026-02-06
### Fixed - Envelope Bugfixes for XM/S3M/MOD Playback

9 buggar identifierade i `docs/instrument-envelope-analysis.md` åtgärdade. Envelope-rendering följer nu FT2-specifikationen betydligt bättre.

#### Prio 1 — Felaktig rendering (påverkade ljud direkt)
- **Linjär fadeout istället för exponentiell**: `FadeoutRate::to_linear_fade_per_tick()` — FT2 använder linjär subtraktion (`fadeoutVol -= speed`), inte multiplikativ decay. Fadeout med rate 4096 når nu tystnad på 8 ticks (0.16s) istället för 107 ticks (2.14s).
- **Parallell fadeout + envelope**: `MultiPointEnvelope` kör nu fadeout parallellt med envelope-avancering efter release, precis som FT2. Borttagna `Fadeout` och `Releasing` stages, ersatta med `released`-flagga.
- **FT2 sustain/loop-interaktion**: Om `sustain_point == loop_end` och noten har släppts, stoppas loopen och envelopet fortsätter förbi loop end.

#### Prio 2 — Saknad funktionalitet
- **Panning envelope**: Extraherar nu `pan_envelope` från xmrs, skapar en andra `MultiPointEnvelope` med bipolar output (-1.0 till +1.0) kopplad till Amplifier `pan_cv`.
- **Gate envelope för instrument utan volume envelope**: MOD/S3M-instrument (inga envelopes) får nu en minimal ADSR-gate (A=0.001, D=0.001, S=1.0, R=0.005) så att NoteOff kan tysta loopad sample.
- **Dynamisk tick_rate från BPM**: `MultiPointEnvelope` läser nu `context.tempo` i `process()` och uppdaterar tick_rate automatiskt. Initial tick_rate sätts från songens BPM vid import.

#### Prio 3 — Förbättringar
- **ADSR release-beräkning fixad**: `convert_envelope_to_adsr()` använder nu korrekt formel `32768 / fadeout / tick_rate` istället för `1 / fadeout` — release-tider stämmer nu med FT2.
- **Korrekt fadeout-divisor**: Linjär fadeout normaliserar mot 32768 (FT2-standard), inte 65536.

#### Filer ändrade
- `synth_core/src/types/tracker.rs`: `to_linear_fade_per_tick()`, uppdaterad `estimated_duration()`
- `synth_modules/src/multi_point_envelope.rs`: Omdesignad stage-maskin, parallell fadeout, bipolar output, dynamisk tick_rate
- `modular_synth/src/io/import/mod.rs`: Panning envelope-fält i `ImportedInstrument`
- `modular_synth/src/io/import/tracker.rs`: Extraherar panning envelope, fixad ADSR release
- `modular_synth/src/gui/egui_backend.rs`: Panning envelope-modul, gate envelope, tick_rate setup

## [0.82.0] - 2026-02-06
### Improved - Tick-Segmented Chunk Rendering for Tracker Playback

Tracker-mode (XM/MOD) renderar nu ljud i tick-segmenterade chunks istället för att applicera alla events vid buffer-start. Detta ger korrekt per-tick modulationsupplösning för vibrato, volume slides, arpeggio, tremolo och andra tick-baserade effekter.

#### Ändringar
- **`SequencerEngine::process_until_next_tick()`**: Ny publik metod som processar exakt ett tick-segment och returnerar chunk-storlek i samples. Befintlig `process()` refaktorerad att anropa den internt — identiskt beteende för alla anropare.
- **`SequencerEngine::is_tracker_mode()`**: Ny metod som detekterar tracker-songs (baserat på `tracker_pattern_count > 0`). Sätts automatiskt vid `set_song()` och `with_song()`.
- **`route_sequencer_events()`**: Extraherad fristående funktion för event-routing (NoteOn, NoteOff, Modulation, VoiceOff). Används av båda render-patherna — ingen kodduplicering.
- **Tick-segmenterad render-loop**: I tracker-mode renderas varje tick-chunk separat med korrekt modulation state. Synth-mode-pathen är helt oförändrad.
- **`chunk_buffer`**: Pre-allokerad AudioBuffer för chunk-rendering, ingen heap-allokering i audio thread.

## [0.81.0] - 2026-02-05
### Fixed - Synth Modules Review Verified Bugfixes

Alla kvarvarande buggar från den verifierade granskningen (docs/synth_modules_review_verified.md) är åtgärdade.

#### Prio 1 — Buggar som påverkade ljud
- **Oscillator prev_sync**: `prev_sync` var lokal variabel i `process()` — retriggrade vid varje buffergräns om sync hölls hög. Flyttad till struct-fält.
- **LFO prev_retrigger**: Identiskt problem — `prev_retrigger` lokal i `process()`. Flyttad till struct-fält.
- **MathOscillator FM base frequency**: FM-modulering hardkodade 440 Hz istället för aktuell basfrekvens. Lagt till `base_frequency`-fält som sätts vid `note_on()` och `set_param()`.
- **MultiPointEnvelope VelocitySensitivity**: Parametern skrev över velocity direkt istället för att kontrollera sensitivity. Nytt `velocity_sensitivity`-fält med korrekt formel: `1.0 - sensitivity * (1.0 - velocity)`.

#### Prio 2 — Vilseledande UI/parametrar
- **Filter env_amount**: Borttagen från descriptor och get_params() — parametern visades i UI men påverkade inte ljudet (ingen envelope-input port finns).
- **SamplePlayer Pan**: Exponerad i descriptor och get_params() — panning fungerade internt men kunde inte ses eller justeras i UI.
- **Compressor sidechain**: Port borttagen från descriptor — var deklarerad men aldrig läst i process().

#### Prio 3 — Kodkvalitet & Newtypes
- **GlideState newtypes**: Alla fält konverterade till domäntyper (`Hertz`, `Seconds`, `NormalizedValue`). Visibility sänkt till `pub(crate)`.
- **Envelope/MultiPointEnvelope trigger()**: Tar nu `Velocity` istället för rå `f32`.
- **MechanicalNoise envelope_phase**: Oanvänt fält borttaget.

#### Prio 4 — Ljud/prestanda-förbättringar
- **Distortion foldback**: While-loop ersatt med for-loop (max 16 iterationer) + final clamp för realtidssäkerhet.
- **Phaser stereo offset**: Höger kanal har nu 90° LFO-fasförskjutning — kommentaren sade "offset phase for stereo width" men båda kanalerna använde samma koefficient.
- **Reverb dirty flag**: `update_filters()` körs nu bara vid parameterändringar (via `params_dirty`-flagga), inte varje buffer.
- **EQ sample_rate**: Jämförelse ändrad från `abs() > 1.0` till korrekt `!=` equality.

#### Verifiering
- Alla ändringar passerar `RUSTFLAGS="-D warnings" cargo build`, clippy (strict), 132 tester och fmt.

## [0.80.0] - 2026-02-05
### Fixed - XM/MOD Playback Accuracy

Omfattande buggfixar för tracker-uppspelning, verifierade mot BassoonTracker och FT2-specifikation.

#### Effekt-buggar
- **Tick-0 modulering saknas**: `process_row_start()` emitterade ingen Modulation-event för tick 0 — stale volym/panning kunde kvarstå efter ny not. Fixat med `emit_tick0_modulation()` helper.
- **is_significant() filtrerade bort nödvändiga moduleringar**: Volym 1.0 och center panning betraktades som "insignifikanta" — tick-0 moduleringar emitteras nu alltid utan filter.
- **XM Arpeggio-ordning**: ProTracker-ordning (0,x,y) → FT2-ordning (0,y,x).
- **PatternDelay (EEx) inte implementerat**: `GlobalCommand::PatternDelay` var no-op. Nu fryser raden N extra ticks med `pattern_delay_rows_remaining`.
- **EffectWaveform::Random deterministisk**: `sample(phase)` returnerade samma hash för samma fas. Nu använder xorshift32 per tick (FT2-beteende).
- **Vibrato depth scaling**: `* 4.0` → `* 2.0` för korrekt FT2-skalning (~30 cents max vid depth 15).
- **Tremolo modell**: Ändrad från multiplikativ (`volume *= 1+tremolo`) till additiv (`volume += tremolo`) per tracker-standard.

#### Sample Player-buggar
- **LoopMode::Backward fungerade inte**: `note_on()` satte alltid `direction = Forward`. Nu startar backward-loopar från loop_end med backward-riktning.
- **PingPong-overshoot förloras**: PingPong satte position till `loop_end - 1.0` utan overshoot-beräkning. Nu bevaras overshoot vid riktningsbyte.
- **Sample offset vid initial NoteOn**: Sample offset-effekt appliceras nu vid första noten.

#### Tracker Engine
- **PatternBreak timing**: Korrekt hantering av pattern breaks vid radgränser.
- **Loop precision**: Exakta loop-punkter (u32) istället för normaliserade f32-värden.

#### Verifiering
- Jämförd med BassoonTracker (JavaScript tracker player) — sample import, loop-hantering och effektprocessing verifierade.
- Alla ändringar passerar `RUSTFLAGS="-D warnings" cargo build`, clippy, tester och fmt.

## [0.79.0] - 2026-01-29
### Fixed - Synth Modules Review Issues

Fixar baserade på omfattande kodgranskning av synth_modules (se docs/synth_modules_review.md).

#### Kritiska buggar
- **Envelope/MultiPointEnvelope gate edge detection**: `prev_gate` var lokal variabel → retriggrades vid buffer-gräns. Fixat genom att flytta till struct-fält.
- **SamplePlayer ReleaseMode::PlayToLoop**: Beter sig nu korrekt - stannar vid loop-slut istället för sample-slut.
- **MechanicalNoise divide-by-zero**: `envelope_samples` kunde bli 0 → division by zero. Fixat med `.max(1)`.

#### Oanvända parametrar fixade
- **Oscillator phase_offset**: Nu applicerad i `generate_sample()` för statisk fasoffset.
- **LFO retrigger_mode**: Nu respekterad - `Continue` ignorerar retrigger input, `Retrigger` aktiverar den.
- **Flanger/Phaser rate_cv**: Port borttagen - AudioEffect trait stöder inte named CV inputs.
- **Chorus Voices parameter**: Nu exponerad i descriptor.

#### Oanvända parametrar implementerade
- **Distortion BitDepth**: Nu exponerad i descriptor och använd i bitcrush-läge.
- **Mixer Input5-8**: Nivåparametrar för alla 8 ingångar nu tillgängliga.
- **Mixer LimitMode**: Soft limiting (tanh) implementerat i process().
- **Filter Drive**: Nu exponerad i descriptor och applicerad som pre-gain med soft saturation.

#### Rust best practices
- `#[must_use]` tillagt på `SampleValue` och `SampleIndex`.
- `velocity: f32` → `Velocity` i VoiceAllocator, Instrument, EngineCommand och EngineEvent.

#### Ändringar
- `Envelope.prev_gate: f32` - nytt fält för edge detection
- `MultiPointEnvelope.prev_gate: f32` - nytt fält
- `SamplePlayer.advance_position()` - stöd för `PlayToLoop`
- `MechanicalNoise.trigger()` - guard mot envelope_samples=0
- `Oscillator.generate_sample()` - adderar phase_offset till fas
- `Lfo.process()` - respekterar retrigger_mode
- Flanger/Phaser descriptor - rate_cv port borttagen
- Chorus descriptor - Voices parameter tillagd
- `DistortionParam::BitDepth` - nytt enum-variant
- `MixerParam::Input5-8` - nya enum-varianter
- `Mixer.process()` - soft limiting via tanh()
- `Filter.process_sample()` - drive pre-gain med saturation
- Velocity typ-säkerhet genom hela note-flödet

---

## [0.77.0] - 2026-01-29
### Fixed - Tracker Effect Issues

Ytterligare fixar för korrekt tracker-uppspelning baserat på djupgående kodgranskning.

#### Sample offset (9xx) vid initial NoteOn
- **Problem**: 9xx-effekten applicerades endast vid retrigger, inte vid första note-start
- **Orsak**: NoteOn-eventet saknade sample_offset fält, offset kom endast via Modulation
- **Fix**:
  - Lagt till `sample_offset: NormalizedValue` i `SequencerEvent::NoteOn`
  - `process_row_start()` returnerar nu även sample_offset
  - `synth_engine.rs` applicerar offset direkt efter `note_on_fixed_voice()` via `retrigger_with_offset()`

#### PatternBreak (Dxx) timing korrigerad
- **Problem**: PatternBreak väntade på pattern-slut istället för rad-slut
- **Orsak**: Villkoret var `current_tick >= current_end` (pattern end) istället för `at_row_boundary`
- **Fix**: PatternBreak triggas nu vid rad-gräns, precis som PatternJump (Bxx)
- Resultat: Dxx hoppar nu omedelbart vid rad-slut, inte vid pattern-slut

#### Loop-precision mismatch åtgärdad
- **Problem**: Olika loop-punkter i `advance_position()` vs `read_with_crossfade()` orsakade klick
- **Orsak**: `advance_position()` använde f32-normaliserade värden som förlorade precision
- **Fix**: Nya fält i SamplePlayer:
  - `exact_loop_start: Option<usize>` - exakt loop-start från sample.loop_info
  - `exact_loop_end: Option<usize>` - exakt loop-slut från sample.loop_info
  - `advance_position()` använder nu dessa exakta värden när de finns

#### Tekniska ändringar
- `SequencerEvent::NoteOn.sample_offset: NormalizedValue` - nytt fält
- `ChannelEffectProcessor::process_row_start()` returnerar nu `(Vec<GlobalCommand>, bool, NormalizedValue)`
- `SamplePlayer.exact_loop_start/exact_loop_end` - exakta loop-bounds
- PatternBreak använder nu `at_row_boundary` istället för `current_tick >= current_end`

---

## [0.76.0] - 2026-01-29
### Fixed - Tracker Effect Accuracy

Omfattande fix av 6 kritiska tracker-effektproblem som orsakade felaktig uppspelning av XM/MOD-filer.

#### SetSpeed (Fxx) påverkar row-timing dynamiskt
- **Problem**: SetSpeed-effekten ändrade bara tempo, inte ticks per rad
- **Orsak**: `ticks_per_row` beräknades en gång vid start, uppdaterades aldrig
- **Fix**: Nytt fält `current_ticks_per_row` som uppdateras vid varje SetSpeed-kommando
- `collect_tracker_pattern_events()` använder nu den dynamiska ticks_per_row

#### Pattern navigation timing (Bxx/Dxx/E6x)
- **Problem**: PatternBreak/Jump/Loop triggades på godtyckliga tick-positioner
- **Orsak**: Navigation skedde direkt istället för vid rad-gränser
- **Fix**: Nya fält `pending_pattern_loop_back` och `last_row_index`
- Navigation väntar nu på rad-gränser innan den appliceras
- Loop-state rensas korrekt vid pattern-byte

#### NoteDelay (EDx) fungerar korrekt
- **Problem**: Noten triggades direkt vid rad-start, ignorerade delay
- **Orsak**: `trigger_note()` anropade `note_state.trigger()` ovillkorligt
- **Fix**: Ny `has_note_delay` parameter i `trigger_note()`
- Noten triggas inte direkt om EDx finns - sker i `process_tick()` vid rätt tick

#### Sample offset (9xx) beräkning korrigerad
- **Problem**: Offset-värdet var fel - delades med 256 istället för att användas direkt
- **Orsak**: xmrs ger oss `parameter * 256`, men importen antog råvärdet
- **Fix**: `let raw_param = ((*offset) / 256).min(255) as u16;` i tracker.rs
- Nu ger 980 (param 0x80 = 128) korrekt 50% av samplelängden

#### Instrumentbyte kapar föregående röst
- **Problem**: Vid instrumentbyte fortsatte gamla instrumentet att spela
- **Orsak**: Endast nya instrumentet triggades, gamla tystades inte
- **Fix**: Vid NoteOn i tracker-läge anropas nu `voice.reset()` på alla andra instruments voice på samma kanal
- Förhindrar överlappande ljud vid instrumentbyte

#### Multisample-instrument med keymap
- **Problem**: XM-instrument med flera samples ignorerade keymap
- **Orsak**: Endast första samplet laddades, keymap aldrig användes
- **Fix**: Ny sample bank-arkitektur i SamplePlayer:
  - `sample_bank: Vec<Arc<Sample>>` - alla samples för instrumentet
  - `sample_keymap: Option<Vec<usize>>` - MIDI-not till sample-index
  - `select_sample_for_note()` - väljer rätt sample vid note_on
- Nytt `LoadSampleBank` kommando för att ladda flera samples med keymap
- Import-koden laddar nu alla samples och keymappen

#### Tekniska ändringar
- `SequencerEngine.current_ticks_per_row: u32` - dynamisk speed
- `SequencerEngine.pending_pattern_loop_back: bool` - rad-boundary navigation
- `SequencerEngine.last_row_index: Option<u32>` - spårar senaste rad
- `ChannelEffectState.trigger_note()` - ny `has_note_delay` parameter
- `SamplePlayer.sample_bank: Vec<Arc<Sample>>` - multisample stöd
- `SamplePlayer.sample_keymap: Option<Vec<usize>>` - not-till-sample mappning
- `SamplePlayer::select_sample_for_note()` - väljer sample från bank
- `SamplePlayer::add_sample_to_bank()` - lägger till sample i bank
- `SamplePlayer::set_sample_keymap()` - sätter keymap
- `PolyModule::load_sample_bank()` - ny trait-metod
- `EngineCommand::LoadSampleBank` - nytt kommando
- `Graph::load_sample_bank()` - ny metod

---

## [0.75.0] - 2026-01-23
### Fixed - Critical Code Issues

Löser 5 kritiska problem identifierade i kodanalysen.

#### Zero-allocation Mixer Hot Path
- **Problem**: `format!("in{i}")` allokerade minne 8 gånger per audio frame i Mixer
- **Orsak**: Dynamiska portnamn krävde String-allokering
- **Fix**: Nya statiska konstanter `PortName::IN1..IN8` och `PortName::MIXER_INPUTS`
- Interned strings i synth_core: "in1" genom "in8" (ID 23-30)
- Mixer använder nu `inputs.get(PortName::MIXER_INPUTS[idx])` - noll allokering

#### Exponential Backoff i send_blocking()
- **Problem**: 10 sekunder timeout (10000 × 1ms) kunde frysa GUI
- **Orsak**: Konstant 1ms sleep oavsett köläge
- **Fix**: Exponential backoff `[0, 0, 1, 2, 4, 8, 16, 32, 64, 100]` ms
- Max 50 försök × ~10ms = ~500ms worst-case (istället för 10s)
- Initiala försök använder `yield_now()` för minimal latens

#### Säker .expect() hantering i GUI
- **Problem**: 5× `.expect()` på instrument-lookup kunde panica vid race conditions
- **Orsak**: `active_instrument_id` kunde bli osynkad vid deletion/creation
- **Fix**: `active_patch_editor()` returnerar nu `Option<&mut PatchEditor>`
- Alla 12 anropsställen uppdaterade med `let Some(editor) = ... else { return; }`
- GUI visar "No active instrument" istället för panic

#### Audio Error Tracking
- **Problem**: Audio stream errors syntes bara på stderr, ej i GUI
- **Fix**: `CpalStream` har nu `error_count: Arc<AtomicU64>` och `last_error`
- Fel loggas till stderr OCH sparas för framtida GUI-display
- Infrastruktur redo för statusfält/notifikationer

#### Realtime-Safe Effect Processing
- **Problem**: Delay/Reverb/Chorus/Flanger anropade `resize_buffers()` i `process()`
- **Orsak**: Buffer-resize kan allokera minne i audio thread
- **Fix**: Ny `set_sample_rate()` implementation för varje effekt
- `process()` anropar inte längre resize - bara `debug_assert!` för verifiering
- Bufferallokering sker nu enbart från main thread

#### Tekniska ändringar
- `PortName::IN1..IN8` - kompilerade konstanter för mixer-portar
- `PortName::MIXER_INPUTS: [PortName; 8]` - iteration utan allokering
- `CommandSender::send_blocking()` - exponential backoff
- `SynthApp::active_patch_editor()` - returnerar `Option`
- `CpalStream.error_count/last_error` - error tracking
- `Delay/Reverb/Chorus/Flanger::set_sample_rate()` - buffer-resize

---

## [0.74.0] - 2026-01-23
### Fixed - Sample Loop Click Prevention

Eliminerar klick och hack vid loop-punkter i importerade tracker-moduler.

#### Exakta loop-punkter (ingen precisionsförlust)
- **Problem**: Loop-punkter konverterades från exakta u32 till normaliserade f32
- **Orsak**: Precisionsförlust på flera samples för stora samplar (>100k frames)
- **Fix**: `SampleLoopInfo` lagrar nu `loop_start` och `loop_end` som `u32`
- Nya metoder `normalized_start()` och `normalized_end()` för GUI-kompatibilitet
- xmrs loop-punkter (u32) bevaras exakt genom hela kedjan

#### Loop-medveten interpolation
- **Problem**: Interpolation (Cubic, Hermite, Sinc etc.) läste samples utanför loop-regionen
- **Orsak**: `clamp(0, len-1)` använde sample-gränser, inte loop-gränser
- **Fix**: Ny metod `Sample::read_looped()` med loop-aware sample-hämtning
- Vid loop_end wrapar interpolation tillbaka till loop_start
- Alla 7 interpolationslägen har nu loop-medvetna varianter

#### Förbättrad crossfade
- **Fix**: `read_with_crossfade()` använder nu `read_looped()` konsekvent
- Crossfade-regionen läser också med korrekt loop-wrapping
- Föredragna loop-gränser från sample-metadata (exakta heltal)

#### Tekniska ändringar
- `SampleLoopInfo.loop_start: u32` - exakt sample-position (ej f32)
- `SampleLoopInfo.loop_end: u32` - exakt sample-position (ej f32)
- `SampleLoopInfo::from_normalized()` - bakåtkompatibilitet
- `SampleLoopInfo::normalized_start/end()` - för GUI
- `Sample::read_looped()` - loop-aware interpolation
- Loop-aware versioner av: cubic, hermite, lagrange, sinc8, sinc16
- `get_looped_sample_mono/stereo()` - sample-hämtning med loop-wrapping

---

## [0.73.0] - 2026-01-23
### Fixed - Tracker Module Playback

Grundlig analys och fix av XM/MOD-moduluppspelning som hade flera kritiska problem.

#### Tone Portamento (3xx/5xx) fungerade inte
- **Problem**: Tone portamento-pitch ignorerades helt i synth_engine
- **Fix**: `tracker_tone_porta_pitch` fält i Voice som override:ar base pitch
- **Fix**: Modulation-events extraherar nu `tone_porta_pitch` och applicerar det
- Glide-effekter fungerar nu korrekt istället för diskreta hopp

#### Initial tracker speed ignorerades vid import
- **Problem**: XM-modulers `default_tempo` (speed) lästes inte vid import
- **Fix**: Nytt fält `default_tracker_speed` i Song-strukturen
- **Fix**: SequencerEngine läser och applicerar speed vid `with_song()` och `set_song()`
- Moduler spelas nu i korrekt tempo från start

#### Tone Portamento triggade felaktigt nya noter
- **Problem**: Noter med tone portamento-effekt skapade NoteOn-events
- **Fix**: `process_row_start()` returnerar nu `(Vec<GlobalCommand>, bool)`
- **Fix**: NoteOn skapas endast om `should_trigger_note` är true
- Förhindrar sample-retrigger vid glide, bevarar envelope

#### Stop-knappen fungerar nu för tracker-moduler
- **Problem**: Voices fortsatte spela efter stop - krävde panic för att tysta
- **Orsak**: `sequencer.stop()` returnerade VoiceOff-events men de ignorerades
- **Fix**: `EngineCommand::Stop` anropar nu `all_notes_off()` på alla instrument direkt
- Voices tystnar nu korrekt vid stop

#### Tekniska ändringar
- `Voice.tracker_tone_porta_pitch: Option<Semitones>` - tone portamento pitch override
- `Song.default_tracker_speed: u8` - initial speed (ticks per row)
- `ChannelEffectProcessor.process_row_start()` - utökad returtyp
- `SequencerEngine.stop()` - skickar VoiceOff för alla MonoVoice-tracks
- Pitch beräknas korrekt: `440.0 * 2^((semitones - 69) / 12)`

---

## [0.72.0] - 2026-01-09
### Improved - Global Module Handling

#### Globala moduler utan portar
- **Effektmoduler** (Reverb, Delay, Chorus, Distortion): Portarna borttagna
- **Visualizer-moduler** (Oscilloscope, LevelMeter): Portarna borttagna
- Tydliggör att globala moduler processas automatiskt via effect chain
- Informativ text visas istället för portar: "Processed automatically via effect chain"

#### Förbättrad visuell feedback för globala moduler
- **Opacity**: Globala moduler dimmas inte längre (alltid full opacity)
- **Ny indikator**: Blå diamant (◆) med tooltip "⚡ Global Module"
- Tydligt att dessa moduler alltid är aktiva

#### Auto-layout förbättringar
- Globala moduler placeras nu längst till höger i rack-vyn
- Egen kolumn efter signalkedjan, före bortkopplade moduler
- Bättre visuell separation mellan signalflöde och globala effekter

#### Patch-filer korrigerade
- 13 exempelpatchar återställda från felaktig effekt-routing
- Effekter hanteras via effect chain, inte via manuella kablar

---

## [0.71.0] - 2026-01-09
### Improved - Rack GUI UX

#### Port-highlighting vid kabeldragning
- **Nytt**: När man drar en kabel lyser kompatibla portar upp med en glödande effekt
- Porten glöder i sin egen färg (Audio=grön, Control=blå, Gate=gul, MIDI=lila)
- Visar tydligt vilka portar som går att koppla till (rätt typ + rätt riktning)
- Implementerat i `PortWidget` med ny `highlighted()` builder-metod

#### Större portar för bättre precision
- **Port-storlek**: Ökad från 14px till 20px för lättare klickning
- **Port-etiketter**: Ökad från 9px till 11px för bättre läsbarhet
- Förbättrar UX på högupplösta skärmar och för pekskärmar

#### Zoom-funktionalitet borttagen
- Tog bort icke-fungerande zoom i Rack-vyn
- Renare kod utan trasig funktionalitet

---

## [0.70.0] - 2026-01-09
### Improved - Module Stability & Performance

#### Envelope (ADSR) förbättringar
- Ersatte `NormalizedValue::new_unchecked()` med säker `new()` + `.clamp(0.0, 1.0)`
- Förhindrar potentiella out-of-bounds värden vid extrema förhållanden

#### Filter stabilitet
- **SVF Filter**: Resonans clampad till max 0.99 för att förhindra instabilitet vid självoscillation
- **Ladder Filter**: Samma resonans-begränsning för stabilitet
- **Denormal prevention**: Lade till `flush_denormals()` för konsekvent prestanda i båda filtertyper

#### Oscillator anti-aliasing
- Lade till PolyBLAMP-korrektion för triangelvågor vid fasens hörnpunkter (0.25 och 0.75)
- Ny `poly_blamp()` funktion i synth_dsp för band-limited triangle waves

#### LFO beat-synkronisering
- LFO-fasen beräknas nu från `context.position_beats` när tempo sync är aktivt
- Perfekt taktlåsning istället för fri löpande frekvens

#### Amplifier (VCA) ny funktion
- Ny parameter `CV Bipolar` för att tillåta negativ CV-modulation
- Möjliggör ring modulation-effekter via VCA

#### DSP primitiver
- **FilterState**: Ny `flush_denormals()` metod förhindrar prestandaproblem med denormala tal
- **InterpolatedDelayLine**: Ny `read_cubic()` metod med Hermite-interpolation för högre kvalitet

#### Body Resonance
- Dynamisk Nyquist-begränsning baserat på sample rate
- Denormal prevention i filter states

#### Sample Player optimering
- Skippar dyra `powf()` per-sample beräkningar när ingen pitch modulation används
- Märkbar CPU-besparing vid normal uppspelning

---

## [0.69.0] - 2026-01-09
### Refactoring - Clippy `use_self` Compliance

#### Systematisk kodstädning
- **Omfattning**: ~150 `use_self` varningar fixade i hela kodbasen
- **Syfte**: Följer Rust best practice att använda `Self` istället för typnamn i impl-block

#### Påverkade crates
- **synth_core** (12 fixar): amplitude.rs, audio.rs, frequency.rs, normalized.rs, samples.rs, module_traits.rs
- **synth_modules** (1 fix): sample_player.rs
- **synth_sequencer** (64 fixar): time.rs, track.rs, pitch.rs, view/state.rs
- **synth_engine** (~70 fixar): transactions.rs, connectivity.rs, event_priority.rs, visual_state.rs, commands.rs, hub.rs, graph.rs
- **modular_synth** (2 fixar): midi.rs, debug_pattern.rs

#### Övriga förbättringar
- Fixade `collapsible_if` lint i debug_pattern.rs
- Alla kontroller passerar: build, clippy, test, fmt

---

## [0.68.0] - 2026-01-09
### Fixed - CPAL 0.17 API Compatibility

#### Uppdaterad cpal-backend för cpal 0.17
- **Problem**: Kompileringsfel efter cpal 0.17 uppgradering
- **Orsak**: cpal 0.17 ändrade flera API:er:
  - `SampleRate` är nu en type alias för `u32`, inte en tuple struct
  - `min_sample_rate()`, `max_sample_rate()`, `sample_rate()` returnerar nu `u32` direkt
  - `Device::name()` är deprecated

#### Ändringar i cpal_backend.rs
- Tog bort `cpal::SampleRate(...)` konstruktor, använder `u32` direkt
- Tog bort `.0` access på sample rate-returvärden
- Bytte från `device.name()` till `device.description().name()` för att undvika deprecation-varningar

---

## [0.67.0] - 2025-12-27
### Fixed - XM Effect Memory & Arpeggio Tick

#### Effect Memory implementerad
- **Problem**: `A00` (volume slide), `100`/`200` (portamento), `P00` (panning slide) nollställde effekten istället för att fortsätta med föregående värde
- **Orsak**: XM-format använder parameter 0 för "fortsätt med föregående hastighet", men koden skrev alltid över effekt-state
- **Lösning**: Lade till villkor `if param > 0` för:
  - `VolumeSlide { up, down }` - behåller slide vid `up=0, down=0`
  - `PortamentoUp(speed)` - behåller speed vid `speed=0`
  - `PortamentoDown(speed)` - behåller speed vid `speed=0`
  - `PanningSlide { left, right }` - behåller slide vid `left=0, right=0`

#### Arpeggio tick-fix
- **Problem**: Arpeggio (0xy) använde `vibrato_phase` för att cykla noter, vilket gjorde att arpeggio inte fungerade när vibrato var av
- **Lösning**:
  - Nytt fält `current_tick: TickInRow` i `ChannelEffectState`
  - `process_tick()` sparar nu aktuell tick
  - Arpeggio använder `self.current_tick.as_u8() % 3` för notcykling

#### Påverkade låtar
- `joli_suspiria.xm` position 3+ låter nu korrekt (massvis av `A00` kommandon)

---

## [0.66.0] - 2025-12-23
### Fixed - XM Vibrato/Tremolo Speed Import

#### Korrigerad effect-skalning
- **Problem**: Vibrato/tremolo visades som `412` istället för `462` i tracker-vyn
- **Orsak**: xmrs normaliserar vibrato/tremolo speed med divisor 64, inte 16
- **Lösning**: Ändrade multiplikator från `* 16.0` till `* 64.0` för speed-parametern

#### xmrs Parameter Memory
- Beslutat att förlita sig på xmrs's inbyggda "parameter memory"
- Continuation effects (`400`) visas nu som resolved values (`462`)
- Förenklar import-koden och följer xmrs design

#### Kod-städning
- Fixade clippy `double_must_use` varningar i `tracker_effects.rs`
- Tog bort redundanta `#[must_use]` attribut på konstruktorer som returnerar Self
- Nytt debug-verktyg: `dump_patterns.rs` example för XM pattern-analys

---

## [0.65.0] - 2025-12-22
### Feature - Extended Tracker Navigation

#### Pattern-navigering under uppspelning
- `<`/`>` knappar fungerar nu även under uppspelning
- Sequencern söker till det nya patterns startposition

#### Play Pattern-funktion
- Ny knapp (🔁) i tracker toolbar
- Spelar endast aktivt pattern i loop
- Nytt `EngineCommand::PlayPattern`

#### Resume Song
- Play-knappen (▶) startar nu från aktivt patterns början
- Istället för att alltid starta från tick 0
- Nytt `EngineCommand::PlayFromPattern`

#### Nya EngineCommands
- `Seek { tick }` - Hoppa till specifik tick-position
- `PlayPattern { pattern_id }` - Loopa ett pattern
- `PlayFromPattern { pattern_id }` - Starta från patterns början

---

## [0.64.0] - 2025-12-22
### Documentation - ARCHITECTURE.md

#### Ny arkitekturdokumentation
- Skapad `ARCHITECTURE.md` för AI-assisterad utveckling
- Dokumenterar crate-struktur och beroendekedja
- Beskriver dataflöde mellan UI och audio-tråd
- Definierar nyckelkoncept (Voice, Instrument, ModuleGraph, etc.)
- Listar kritiska invarianter (realtidssäkerhet, newtype-mönster)
- Inkluderar vanliga operationer (lägga till modul, effekt)

---

## [0.63.0] - 2025-12-22
### Refactoring - Newtype Pattern för synth_engine

#### tracker_effects.rs - Beskrivande typer
- **`RetriggerState`** - Kapslar in retrigger-logik (interval, counter, volume_change)
- **`TickCount`** - Newtype för tick-räkning
- **`Glissando`** enum - `Smooth` / `Quantized` ersätter `bool`
- **`NoteState`** - Ersätter tre booleans med:
  - `PlayingState` enum: `Stopped`, `Playing`
  - `TriggerAction` enum: `None`, `Trigger`
  - `CutAction` enum: `None`, `Cut`

#### effect_chain.rs - Slot-tillstånd
- **`EnabledState`** enum - `Active` / `Bypassed` ersätter `enabled: bool`
- Metoder: `is_active()`, `is_bypassed()`, `toggle()`

#### connectivity.rs - Port och fel-typer
- **`ConnectionState`** enum - `Disconnected` / `Connected`
- **`SignalActivity`** enum - `Inactive` / `Active`
- **`ConnectionCount`** newtype - Ersätter `usize`
- **`SampleTimestamp`** newtype - Ersätter `timestamp: u64` (med serde)
- **`OccurrenceCount`** newtype - Ersätter `occurrence_count: u32`

#### instrument.rs - Mute/Solo med synth_core typer
- Använder **`MuteState`** från synth_core istället för `enabled: bool`
- Använder **`SoloState`** från synth_core istället för `solo: bool`
- Nya metoder: `mute_state()`, `set_mute_state()`, `solo_state()`, `set_solo_state()`

#### Förbättrad typsäkerhet
- Följer CLAUDE.md regler för newtype-mönstret
- Eliminerar primitiver i publika API:er
- Alla 87 tester passerar

---

## [0.62.0] - 2025-12-21
### Fixed - Volume Reset Bug in Tracker Effects

#### Bugfix: Kanalvolym återställs vid ny not med instrument
- **Problem**: Efter volume slide down (t.ex. `A05`) gick kanalvolymen till 0, och nya noter med instrument förblev tysta
- **Orsak**: `process_row_start()` i effektprocessorn fick aldrig information om instrumentbyte, så volymen återställdes aldrig
- **Lösning**:
  - Ny parameter `instrument_volume: Option<NormalizedValue>` i `process_row_start()`
  - När en not med explicit instrument spelas, återställs kanalvolymen till notens velocity
  - Volume slide nollställs för att stoppa pågående slide
  - SetVolume-effekter kan fortfarande override:a default-volymen

#### XM-beteende implementerat
| Scenario | Beteende |
|----------|----------|
| Not + explicit instrument | Återställ volym till notens velocity |
| Not + ärvt instrument (inherit) | Behåll nuvarande kanalvolym |
| SetVolume på samma rad | Override:ar default-volymen |

#### Nya tester
- `test_volume_reset_on_new_instrument` - Verifierar volymåterställning
- `test_volume_not_reset_on_inherit_instrument` - Verifierar att inherit behåller volym
- `test_set_volume_overrides_instrument_default` - Verifierar SetVolume override

---

## [0.61.0] - 2025-12-18
### Added - Track Solo & UI Improvements

#### Track Solo Button
- Ny solo-knapp per track i tracker-vyn
- Klicka "S" för att isolera en track (endast den tracken spelar)
- Klicka igen för att stänga av solo
- Orange bakgrund när aktiv, grå när inaktiv
- Filtrerar NoteOn/Modulation events baserat på voice_index/track

#### UI-förbättringar
- **Instrumentval-knapp**: Nu synlig som radio-knapp (● fylld / ○ tom)
- **Solo-knapp**: Tydlig toggle-stil med synlig bakgrund

#### Borttaget: TrackerFilter
- Tog bort oanvänd `TrackerFilter` modul
- XM-formatet stödjer inte Zxx filter-effekter (endast IT-format)
- Relaterade typer `TrackerCutoff` och `TrackerResonance` borttagna

#### Optimering: Tomma instrument
- Instrument utan giltiga samples skapas nu utan moduler
- Sparar minne och CPU för tomma instrument-platser i XM-filer
- Indexering bevaras för korrekt MIDI-kanal-routing

---

## [0.60.0] - 2025-12-18
### Fixed - SamplePlayer Loop & Keyboard Routing

#### SamplePlayer loop fix
- **Problem**: Loop returnerade till fel position (stannade vid slutet istället för loop_start)
- **Orsak**: När `loop_end == sample_len`, klampas position till `sample_len-1`, vilket gav negativt overshoot i loop-beräkningen
- **Lösning**: Om overshoot är negativt (triggered by clamping), hoppa direkt till `loop_start`

#### Keyboard routing fix
- **Problem**: Flera instrument spelade samtidigt vid tangentbordsinput efter XM-import
- **Orsak**: Tangentbordet skickade noter på instrumentets MIDI-kanal, men `focused_instrument` filtrerade bara på kanal 0
- **Lösning**: Tangentbord skickar alltid på CH1, `focused_instrument` styr routing

#### Sequencer routing
- Sekvensern spelar nu alla instrument oavsett `focused_instrument`-inställning
- `focused_instrument` påverkar endast tangentbordsinput, inte sekvenser-uppspelning

---

## [0.59.0] - 2025-12-17
### Fixed - Tracker Effects (Vibrato, Portamento, Volume Slide)

Fixade två buggar som förhindrade tracker-effekter från att höras:

#### Problem 1: process_tick() anropades aldrig
- `ChannelEffectProcessor::process_tick()` beräknade effekter men resultaten ignorerades
- **Lösning**: Anropa `process_tick()` varje tick och emittera `Modulation` events

#### Problem 2: Steppy pitch modulation
- `SamplePlayer::effective_speed()` beräknades en gång per buffer (~5.3ms)
- **Lösning**: Lade till `pitch_mod` CV-input för per-sample pitch modulation

#### Nya typer och events
- `SequencerEvent::Modulation` - Per-tick modulation från tracker-effekter
  - `pitch_cents: Cents` - Pitch offset (100 cents = 1 halvton)
  - `volume: NormalizedValue` - Volymmodulation
  - `panning: BipolarValue` - Panoreringsmodulation
  - `note_cut: bool` - Note cut flag
  - `tone_porta_pitch: Option<f32>` - Tone portamento target

#### Voice tracker modulation
- Nya fält i `Voice`:
  - `tracker_pitch_cents: Cents`
  - `tracker_volume: NormalizedValue`
  - `tracker_panning: BipolarValue`
- Tracker pitch appliceras både på oscillatorfrekvens och SamplePlayer

#### SamplePlayer pitch_mod input
- Ny CV-input `pitch_mod` (semitones)
- Per-sample pitch modulation: `speed = base_speed * 2^(pitch_offset/12)`

#### ModuleGraph extern pitch modulation
- `set_sample_player_pitch_mod(semitones: f32)` - Sätter extern pitch modulation
- Automatisk injektion av pitch_mod buffer till SamplePlayer-moduler

#### Filer som ändrats
- `crates/synth_sequencer/src/events.rs` - `SequencerEvent::Modulation`
- `crates/synth_engine/src/sequencer_engine.rs` - Anropar `process_tick()`
- `crates/synth_engine/src/synth_engine.rs` - Hanterar `Modulation` events
- `crates/synth_engine/src/voice.rs` - Tracker modulation fält och applicering
- `crates/synth_engine/src/graph.rs` - Extern pitch mod infrastruktur
- `crates/synth_modules/src/sample_player.rs` - `pitch_mod` CV-input

---

## [0.58.0] - 2025-12-17
### Added - Tracker-Compatible Modules (MultiPointEnvelope & TrackerFilter)

Implementerade två nya moduler for forbattrad XM/IT tracker-kompatibilitet.

#### MultiPointEnvelope
Full XM/IT envelope-support med:
- Upp till 25 arbitrara punkter
- Linear interpolation mellan punkter
- Sustain-punkt (haller tills note-off)
- Loop-region (loopar medan sustained)
- Fadeout efter release
- Heap-fri implementation med ArrayVec

#### TrackerFilter
IT-kompatibelt resonant lowpass-filter med:
- Zxx-kontroll (0-127 cutoff/resonance)
- SVF (State Variable Filter) implementation
- Exponentiell cutoff-kurva (~110 Hz till ~10 kHz)
- Q-faktor 0.5-12.0

#### Nya typer i synth_core
- `EnvelopeFrame` - Envelope frame position (0-65535)
- `EnvelopeValue` - Envelope value (0.0-1.0)
- `EnvelopePointIndex` - Point index (0-24)
- `FadeoutRate` - Fadeout rate
- `TrackerCutoff` - Filter cutoff (0-127)
- `TrackerResonance` - Filter resonance (0-127)

#### XM Import Integration
- `ImportedInstrument` utokad med:
  - `envelope_points: Vec<(u16, f32)>` - Ra envelope-punkter
  - `envelope_sustain: Option<u8>` - Sustain-punkt index
  - `envelope_loop: Option<(u8, u8)>` - Loop region
  - `fadeout: f32` - Fadeout rate
- Debug import visar nu envelope-info per instrument

#### ModuleType registrering
- `ModuleType::MultiPointEnvelope`
- `ModuleType::TrackerFilter`

#### Filer som andrats
- `crates/synth_core/src/types/tracker.rs` - NY FIL: Alla tracker-newtypes
- `crates/synth_core/src/types/mod.rs` - Export tracker module
- `crates/synth_core/src/params/mod.rs` - Nya ModuleType varianter
- `crates/synth_modules/src/multi_point_envelope.rs` - NY FIL: MultiPointEnvelope
- `crates/synth_modules/src/tracker_filter.rs` - NY FIL: TrackerFilter
- `crates/synth_modules/src/lib.rs` - Export nya moduler
- `crates/modular_synth/src/io/import/mod.rs` - Utokad ImportedInstrument
- `crates/modular_synth/src/io/import/tracker.rs` - Extraherar envelope-data
- `crates/modular_synth/src/main.rs` - Debug output for envelope info
- `Cargo.toml` - arrayvec beroende

---

## [0.57.0] - 2025-12-17
### Added - CLI Debug Import Command

Ny kommandoradsparameter `--debug-import` / `-d` för att importera tracker-filer och visa debuginformation utan att starta GUI.

#### Användning
```bash
modular-synth --debug-import /path/to/song.xm
modular-synth -d /path/to/song.xm
```

#### Output
- Songinformation (namn, tempo, antal patterns/samples/instrument)
- Instrumentlista med sample-referenser och volym
- Samplelista med antal frames, kanaler och loop-info
- Arrangement med pattern-placeringar
- Första pattern med noter och duration-info
- Varningar för saknade samples eller ogiltiga instrument-referenser

#### Filer som ändrats
- `crates/modular_synth/src/main.rs` - Ny `CliAction` enum, `--debug-import` argument-parsing, `run_debug_import()` funktion

---

## [0.56.0] - 2025-12-17
### Fixed - XM Import Key Off and Effect-Only Rows

Fixade tre kritiska buggar i XM tracker import/uppspelning som gjorde att musiken lät fel.

#### Problemen
1. **Key Off ignorerades helt** - Noter utan explicit duration spelades för evigt (loopade stråkar, pads slutade aldrig)
2. **Effect-only rows ignorerades** - Rader med bara effekter (volume slides, vibrato continuation) kastades bort
3. **SetSpeed runtime ignorerades** - Tempo-ändringar via Fxx-effekt (x < 0x20) fungerade inte under uppspelning

#### Lösningen

**Key Off:**
- Lade till `last_note_index` och `last_note_start_tick` i `ChannelState` för att spåra aktiva noter
- Vid Key Off: beräknar duration från föregående not och sätter `note.duration`
- Vid ny not på samma kanal: implicit note-off på föregående not (tracker-beteende)

**Effect-only rows:**
- Ny `EffectOnlyEvent` typ i `synth_sequencer::pattern`
- Import skapar `EffectOnlyEvent` för rader utan noter men med effekter
- `SequencerEngine` emittar `SequencerEvent::Effect` för dessa

**SetSpeed runtime:**
- Nya fält `base_tempo` och `tracker_speed` i `SequencerEngine`
- `GlobalCommand::SetSpeed` justerar `cached_tempo` via formeln: `effective_tempo = base_tempo * (6 / speed)`
- Ny metod `recalculate_effective_tempo()` för konsistent tempo-hantering

#### Filer som ändrats
- `crates/synth_sequencer/src/pattern.rs` - Ny `EffectOnlyEvent` typ, `note_by_index_mut()`, `effect_events` fält
- `crates/synth_sequencer/src/lib.rs` - Export av `EffectOnlyEvent`
- `crates/synth_engine/src/sequencer_engine.rs` - SetSpeed runtime, effect-only event emission
- `crates/modular_synth/src/io/import/tracker.rs` - Key Off och effect-only row hantering

---

## [0.55.0] - 2025-12-12
### Fixed - XM/MOD Import Pitch Accuracy

Fixade pitch-beräkning för tracker-moduler genom att hantera FrequencyType (Amiga vs Linear).

#### Problemen
1. **FrequencyType ignorerades** - XM-filer kan använda Amiga eller Linear frekvensberäkning. Amiga C-4 = 8297 Hz vs Linear C-4 = 8363 Hz (~14 cents skillnad).
2. **Dubbel relative_pitch** - `relative_pitch` applicerades på både `sample_rate` OCH `root_note`, vilket tog ut varandra.
3. **Finetune ignorerades** - Sample finetune (-1..1 semitoner) användes inte alls.

#### Lösningen
- Beräknar korrekt basfrekvens baserat på `module.frequency_type` (Amiga: 8297 Hz, Linear: 8363 Hz)
- Applicerar `finetune` på sample_rate via formeln `2^(finetune/12)`
- Använder `relative_pitch` enbart för `root_note`
- Lade till `default_panning` fält i Sample struct

#### Filer som ändrats
- `crates/modular_synth/src/io/import/tracker.rs` - FrequencyType-hantering, finetune-fix
- `crates/synth_core/src/types/sample.rs` - Lade till `default_panning` fält

---

## [0.54.0] - 2025-12-12
### Fixed - Tracker Voice Allocation (TrackId Mismatch)

Fixade tracker-style voice allocation för korrekt mono-per-kanal uppspelning.

#### Problemet
Vid import av tracker-moduler (MOD/XM/S3M) skapades `TrackId` med `TrackId::new(channel_idx)` i `convert_pattern()`, men detta matchade inte de `TrackId`:s som skapats av `song.create_track()`. Resultatet var att `get_voice_index_for_track()` returnerade `None`, vilket orsakade polyfon allokering istället för tracker-style mono-per-kanal.

#### Lösningen
Skickade `track_ids` vektorn (från `song.create_track()`) till `convert_pattern()` och använde dessa ID:n istället för att skapa nya.

#### Filer som ändrats
- `crates/modular_synth/src/io/import/tracker.rs` - Skickar `track_ids` till `convert_pattern()`
- `crates/synth_engine/src/sequencer_engine.rs` - Förenklad `get_voice_index_for_track()`
- `crates/synth_engine/src/voice_allocator.rs` - Rensad debug-utskrift
- `crates/modular_synth/src/gui/egui_backend.rs` - Rensad debug-utskrift

---

## [0.53.0] - 2025-12-12
### Fixed - Rack View Silent Audio Bug

Kritisk fix för tyst ljud i Rack View efter graf-rebuild.

#### Problemet
Efter att ha valt ett instrument i Rack View producerade grafen inget ljud trots att oscillatorer och envelopes kördes korrekt. Endast den första audio-buffern efter note_on innehöll ljud.

#### Rotorsaken
`ModuleGraph::process_module()` återanvände `input_buffers` Vec mellan moduler men clearade bara **buffrarnas innehåll** - inte **listan av port-entries**.

Om modul A processades med port "in" och sedan modul B (t.ex. `StereoOutput`) också hade port "in", hittade den en **stale entry** från modul A istället för att skapa en ny från rätt koppling. Resultatet blev att signaler routades fel eller försvann.

#### Lösningen
Ändrade från att cleara buffrarnas innehåll till att cleara hela Vec:en med `self.input_buffers.clear()` vid början av varje modul-process.

#### Filer som ändrats
- `crates/synth_engine/src/graph.rs` - Fixade input_buffers hantering i `process_module()`

---

## [0.52.0] - 2025-12-12
### Changed - Cargo Workspace Refactoring

Delade upp monolitisk crate (~21k LOC) i 6 separata crates för bättre kompileringstider och arkitektur.

#### Ny Crate-struktur

```
modular-synth/
├── Cargo.toml (workspace root)
└── crates/
    ├── synth_core/      # Types, traits, audio abstractions
    ├── synth_dsp/       # DSP primitives (oscillators, filters)
    ├── synth_sequencer/ # Pattern, song, events
    ├── synth_modules/   # Synth modules and effects
    ├── synth_engine/    # Voice allocation, graph, sequencer engine
    └── modular_synth/   # GUI, main, audio backends
```

#### Beroendegraf

```
synth_core (types, traits)
    ↑
    ├── synth_dsp (oscillators, filters)
    ├── synth_sequencer (pattern, song)
    │
    └── synth_modules (oscillator, filter, effects)
            ↑
            └── synth_engine (graph, voice, instrument)
                    ↑
                    └── modular_synth (gui, main, cpal)
```

#### Fördelar

- **Snabbare inkrementell kompilering** - Ändringar i GUI behöver inte rekompilera DSP-kod
- **Bättre separation of concerns** - Tydliga gränser mellan moduler
- **Enklare testning** - Varje crate kan testas isolerat
- **Möjliggör framtida plugin-stöd** - `synth_engine` kan användas utan GUI

#### Borttagna filer

- `src/` mappen (all kod flyttad till `crates/`)
- `examples/` (flyttad till `crates/modular_synth/examples/`)

---

## [0.51.0] - 2025-12-12
### Added - Type Safety & Operator Overloading

Utökade typsystemet med nya domäntyper och operator overloading för renare DSP-kod.

#### Nya Cross-Type Operators

| Operation | Resultat | Användning |
|-----------|----------|------------|
| `Hertz / SampleRate` | `f32` | Phase increment för oscillatorer |
| `BipolarValue * NormalizedValue` | `f32` | LFO × mod depth (attenuering) |
| `SampleCount / SampleRate` | `Seconds` | Bufferstorlek → tid |

Exempel på förenklad kod:
```rust
// Före
let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();

// Efter
let phase_inc = self.rate / self.sample_rate;
```

#### Nya Typer (`src/types/audio.rs`)

- **`CpuUsage(f32)`** - CPU-belastning (0.0-1.0) med `is_warning()`, `is_critical()`
- **`StereoLevels`** - Peak-mätning för meters (`left`, `right`: Amplitude)
- **`TrackCount(usize)`** - Antal kanaler med konstanter: `MOD_STANDARD`, `THIRTYTWO`
- **`PatternIndex(usize)`** - Position i song arrangement
- **`RowIndex(u32)`** - Radnummer i pattern med `is_beat()`, `is_bar()`
- **`TrackerSpeed(u8)`** - Ticks per row (1-31, DEFAULT=6)

#### Uppdateringar till VoiceCount

- **`new()` clampar nu till 1-128** (var obegränsat)
- **`new_unchecked()`** för performance-kritiska paths
- **`THIRTYTWO`** konstant (32 voices för tracker)
- **`MAX_ALLOCATOR`** konstant (128 voices)
- **`clamp_allocator()`** metod för allocator range
- **`From<usize>`** implementation

#### Filer som ändrats
- `src/types/frequency.rs` - `Hertz / SampleRate` operator
- `src/types/normalized.rs` - `BipolarValue * NormalizedValue` operator
- `src/types/samples.rs` - `SampleCount / SampleRate` operator
- `src/types/audio.rs` - Nya typer och VoiceCount-uppdateringar

---

## [0.50.0] - 2025-12-12
### Added - Tracker Voice Allocation (Mono-Per-Channel)

Implementerade tracker-style voice allocation för korrekt MOD/XM/S3M-uppspelning.

#### Problemet
Importerade tracker-filer hade "ghost notes" - loopade ljud fortsatte spela även när nya noter startade på samma kanal. Tracker-moduler är mono-per-kanal, men synten använde polyfon röstallokering.

#### Lösningen
Ny `Tracker` allocation mode där varje kanal får en dedikerad röst (fixed voice index). Nya noter på samma kanal gör en legato-style retrigger utan att nollställa envelope.

#### Nya typer (`src/types/audio.rs`)
- **`VoiceIndex`** - Index för en specifik röst (0-255)
- **`VoiceCount`** - Antal röster med konstanter: `DUAL`, `QUAD`, `OCTO`, `SIXTEEN`, `THIRTYTWO`

#### TrackMode (`src/sequencer/track.rs`)
- **`TrackMode::MonoVoice(VoiceIndex)`** - Tracker-style, kanal i → röst i
- **`TrackMode::Polyphonic`** - Keyboard/MIDI-style, dynamisk allokering

#### Voice Allocator (`src/engine/voice_allocator.rs`)
- **`AllocationMode::Tracker`** - Ny allocation mode
- **`note_on_fixed_voice(voice_index, note, velocity)`** - Triggrar not på specifik röst
- **`note_off_fixed_voice(voice_index)`** - Släpper specifik röst
- **`resize(count)` / `resize_with_graph(count, template)`** - Dynamisk omstorlek av voice pool
- **`AllocatorConfig::max_voices`** är nu `VoiceCount` istället för `usize`

#### Sequencer Event
- **`NoteOn::voice_index: Option<VoiceIndex>`** - Specifierar vilken röst som ska användas

#### Import (`src/io/import/`)
- **`ImportedSong::min_voices`** - Minimum antal röster (antal tracker-kanaler)
- Tracker-import skapar nu en track per kanal med `TrackMode::MonoVoice`
- GUI skapar instrument med `AllocationMode::Tracker` och rätt antal röster

#### Filer som ändrats
- `src/types/audio.rs` - VoiceIndex, VoiceCount newtypes
- `src/engine/voice_allocator.rs` - Fixed voice allocation, resize
- `src/engine/sequencer_engine.rs` - voice_index routing
- `src/engine/synth_engine.rs` - note_on_fixed_voice() integration
- `src/sequencer/track.rs` - TrackMode enum
- `src/sequencer/events.rs` - voice_index i NoteOn
- `src/io/import/tracker.rs` - MonoVoice per kanal
- `src/io/import/mod.rs` - min_voices i ImportedSong
- `src/gui/egui_backend.rs` - Tracker mode vid import

---

## [0.49.0] - 2025-12-12
### Added - Debug System

Implementerade ett komplett debug-system för offline-analys av audio-motorn.

#### Nya komponenter

**SignalProbe** - Inspektera sample-värden vid specifika punkter
- Spela in samples från godtyckliga portar i grafen
- Beräkna statistik (RMS, peak, DC offset, min/max)
- Jämför signaler (korrelation, skillnad)
- Max-samples-gräns för att förhindra minnesläckor

**GraphDebugger** - Verifiera modulkopplingar
- Lista alla connections i grafen
- Sök path mellan moduler
- Hitta okopplade inputs/outputs
- Diagnostisera vanliga problem (envelope utan gate, amplifier utan CV)
- Human-readable graph description

**SequencerDebugger** - Stega genom songs tick-för-tick
- Stega framåt tick-för-tick offline
- Event-logg med tidsstämplar och källa
- Snapshot av aktiva noter och state
- Sammanfattning av genererade events

**VoiceDebugger** - Förstå voice allocation
- Spåra allokeringar, releases, steals
- State change-historik
- Snapshot av alla voices vid given tidpunkt
- Analysera voice reuse och unique voices

#### Feature flag

Debug-systemet är opt-in via `--features debug-tools`. Ingen overhead vid normal körning.

```toml
[features]
debug-tools = []
```

#### Filer som lagts till
- `src/debug/mod.rs` - Modul-struktur och re-exports
- `src/debug/signal_probe.rs` - SignalProbe implementation
- `src/debug/graph_debugger.rs` - GraphDebugger implementation
- `src/debug/sequencer_debugger.rs` - SequencerDebugger implementation
- `src/debug/voice_debugger.rs` - VoiceDebugger implementation

#### Dokumentation
- `docs/DEBUG_SYSTEM_DESIGN.md` - Design-dokument med API-specifikation

---

## [0.48.0] - 2025-12-12
### Fixed - Tracker Playback Volume (Silent XM Fix)

Kritisk fix för tyst uppspelning av importerade tracker-filer.

#### Problemet
Importerade XM/MOD-filer spelades nästan tyst (2-16% volym) trots korrekt import.

#### Rotorsaken
Volymen applicerades **dubbelt**:
1. Notens velocity (från volume-kolumnen) = t.ex. 0.12 (12%)
2. Effektprocessorns `SetVolume` = t.ex. 0.12 (12%)
3. Resultat: `0.12 × 0.12 = 0.014` (1.4% volym) - ohörbart!

#### Lösningen
I tracker-format representerar notens velocity volymkolumnens värde direkt.
Effektprocessorns volymmodulering ska endast påverka UNDER uppspelning (volume slide, tremolo), inte vid note-onset.

Ändrade `sequencer_engine.rs` att använda notens velocity direkt istället för att multiplicera med effektprocessorns volym.

#### Filer som ändrats
- `src/engine/sequencer_engine.rs` - Tog bort volym-multiplikation vid note-onset
- `src/engine/synth_engine.rs` - Rensade temporär debug-logging

---

## [0.47.0] - 2025-12-11
### Fixed - Multi-Channel Tracker Display & Sample Index

Fixar för korrekt visning av tracker-filer med många kanaler och sample-indexering.

#### Bugfixar
- **Multi-channel display:** Pattern visar nu alla kanaler (30 kanaler för joli_untouched.xm), inte bara 4
  - `Pattern.set_num_tracks()` - ny setter-metod för att sätta antal kanaler
  - Tracker-import sätter nu `num_tracks` baserat på modulens kanalantal
  - Tangentbordsnavigering i Sequencer-vyn respekterar nu antal kanaler från pattern
- **Sample-indexering:** Fixade bug där instrument utan samples fick felaktig sample_index
  - Kontrollerar nu `sample_count > 0` istället för `!instr.sample.is_empty()`
- **Standard volym:** `ChannelState.last_volume` sätts nu till 1.0 (full volym) som default

#### Filer som ändrats
- `src/sequencer/pattern.rs` - Lade till `set_num_tracks()` metod
- `src/io/import/tracker.rs` - Fixade sample-indexering och volym-default
- `src/gui/views/sequencer.rs` - Hämtar antal tracks från pattern

---

## [0.46.0] - 2025-12-11
### Added - Tracker Effects Implementation

Fullständig implementation av tracker-effekter för MOD/XM/S3M-import.

#### Effekter Implementerade
- **Volume Effects:** SetVolume, VolumeSlide, FineVolumeSlide, Tremolo
- **Pitch Effects:** Arpeggio, PortamentoUp, PortamentoDown, TonePortamento, Vibrato
- **Panning Effects:** SetPanning, PanningSlide
- **Timing Effects:** NoteDelay, NoteCut, NoteFadeOut, Retrigger
- **Global Effects:** SetTempo, SetSpeed, PatternBreak, PatternJump, PatternLoop, PatternDelay
- **Miscellaneous:** SampleOffset, FineTune, Glissando, VibratoWaveform, TremoloWaveform

#### Nya Typer (typsäkra wrappers)
- **`TrackerSpeed`** - Ticks per rad (default: 6)
- **`TickInRow`** - Position inom en rad
- **`TrackerSampleOffset`** - Startposition för sample-uppspelning
- **`PitchCents`** - Tonhöjdsförskjutning i cents
- **`PortamentoDirection`** - Up/Down/Off för portamento

#### Arkitektur
- **`ChannelEffectProcessor`** - Hanterar alla kanal-effekter
- **`ChannelEffectState`** - Per-kanal-tillstånd (volym, panning, vibrato-fas, etc.)
- **`ChannelModulation`** - Modulationsvärden som appliceras på noter
- **`GlobalCommand`** - Kommandon som påverkar sequencer-nivån

#### Effektkonvertering (xmrs → vår modell)
- `TrackEffect` → `EffectCommand` (not-nivå effekter)
- `GlobalEffect` → `EffectCommand` (pattern-nivå effekter)
- Waveform-mapping för vibrato/tremolo

#### Analysverktyg
- **`analyze_all_trackers`** - Analyserar alla tracker-filer i en katalog
  - Rapporterar vilka effekter som används
  - Identifierar saknade/ostödda funktioner
  - Listar envelope- och sample-features

```bash
cargo run --example analyze_all_trackers -- /path/to/music
```

#### Filer som ändrats
- `src/engine/tracker_effects.rs` - **NY** - Effektprocessing
- `src/engine/mod.rs` - Exporterar tracker_effects
- `src/engine/sequencer_engine.rs` - Integrerar effektprocessing
- `src/io/import/tracker.rs` - Konverterar xmrs-effekter till EffectCommand
- `examples/analyze_all_trackers.rs` - **NY** - Analysverktyg

---

## [0.45.0] - 2025-12-11
### Fixed - Tracker Import & Sequencer Playback

Kritiska fixar för tyst uppspelning av importerade tracker-moduler.

#### Bugfixar
- **Tyst XM-uppspelning:** `focused_instrument` rensas nu efter import så att alla instrument spelas i Sequencer-vyn
- **Fokuserad routing:** Sequencer-händelser respekterar nu `focused_instrument` för solo-läge i Rack-vyn

#### Nytt verktyg
- **`analyze_tracker`** - Diagnostikverktyg för MOD/XM/S3M-filer
  - Visar modulinfo (namn, BPM, tempo, kanaler, mönster)
  - Instrumentdetaljer (samples, volym-envelopes, pan, fadeout)
  - Sample-information (datalängd, volym, loop-inställningar)
  - Mönsterordning och noter i första mönstret
  - Sammanfattning med statistik

```bash
cargo run --example analyze_tracker -- /path/to/file.mod
cargo run --example analyze_tracker -- /path/to/file.xm
cargo run --example analyze_tracker -- /path/to/file.s3m
```

#### Filer som ändrats
- `src/gui/egui_backend.rs` - Rensar `focused_instrument` efter import till Sequencer-vy
- `src/engine/synth_engine.rs` - Kollapsade if-satser (clippy-fix)
- `examples/analyze_tracker.rs` - Nytt verktyg (ersätter analyze_xm.rs)

---

## [0.44.0] - 2025-12-11
### Added - Native File Dialogs

Ersätter manuella fildialog med `egui-file-dialog` crate för native filbläddrare.

#### File Dialog Integration
- **`egui-file-dialog` v0.12** - Native filväljare med filterstöd
- **Öppna patch** - Ny menypost "Open Patch..." för att öppna sparade patches
- **Spara patch** - "Save Patch..." öppnar filväljare med .json-filter
- **Import song** - "Import Song..." med filter för .mod/.xm/.s3m
- **Load Built-in** - Behållen för inbyggda example-patches

#### API-ändringar
- **`FileDialogMode`** enum: `OpenPatch`, `SavePatch`, `ImportSong`, `OpenSample`
- **`FileDialogResult`** enum: `Picked(PathBuf, mode)`, `Saved(PathBuf, mode)`
- **`DialogState`** metoder:
  - `open_open_patch_dialog()` - Öppna patch-fil
  - `open_save_patch_dialog(default_name)` - Spara patch
  - `open_import_song_dialog()` - Import tracker-fil
  - `open_sample_dialog()` - Öppna WAV-sample (förberett för framtida bruk)
  - `update_file_dialog(ctx)` - Hanterar dialog-state och returnerar resultat

#### Filer som ändrats
- `Cargo.toml` - `egui-file-dialog = "0.12"` tillagt
- `src/gui/dialogs.rs` - FileDialog integration, nya typer och metoder
- `src/gui/egui_backend.rs` - Uppdaterade menyer och FileDialogResult-hantering

---

## [0.43.0] - 2025-12-11
### Added - Focused Instrument & Tracker Playback Fixes

Lösningar för MIDI-kanalkonflikter och korrekt tracker-uppspelning.

#### Focused Instrument (Keyboard Routing)
- **Problem:** Med >16 instrument delade flera samma MIDI-kanal, alla spelade samtidigt
- **Lösning:** "Focused Instrument" - keyboard input (kanal 0) går endast till valt instrument
- **`EngineState`:** `focused_instrument: AtomicU32` för trådsäker state
- **`EngineCommand::SetFocusedInstrument`** - Nytt kommando för GUI-styrning
- **`EngineHandle`:** `set_focused_instrument()`, `get_focused_instrument()` metoder
- **GUI-integration:** Automatiskt fokus vid instrumentval, import, och borttagning

#### Mono-per-Track (Tracker-beteende)
- **Problem:** Loopade ljud försvann inte vid nya noter på samma track
- **Lösning:** `TrackId` newtype för typsäker track-identifiering
- **`Note::track`:** `Option<TrackId>` för mono-per-track routing
- **Sequencer:** `stop_notes_on_track()` stänger av föregående not automatiskt
- **Tracker-import:** Sätter `TrackId` för varje kanal vid import

#### Sample Player Fixes
- **Release Mode:** Non-looped samples använder nu `ReleaseMode::PlayToEnd` (default)
  - Drums/one-shots spelar klart istället för abrupt stopp
  - Looped samples behåller `ReleaseMode::Immediate`
- **Interpolation:** Konfigurerbar i GUI med Cubic som default
  - Nearest, Linear, Cubic, Hermite, Lagrange, Sinc8, Sinc16

#### Pattern View Improvements
- **Follow Playback:** Pattern-vyn följer nu aktiv rad under uppspelning
- **Auto-switch Pattern:** Automatiskt byte till aktivt pattern
- **Dynamiska tracks:** Visar rätt antal tracks från importerad fil (inte hårdkodat 4)
- **Pattern display:** Visar "1/3" format (aktuellt/totalt) istället för hex

#### Filer som ändrats
- `src/engine/state.rs` - `focused_instrument`, `NO_FOCUSED_INSTRUMENT`
- `src/engine/commands.rs` - `SetFocusedInstrument` kommando
- `src/engine/synth_engine.rs` - note routing, handle methods
- `src/sequencer/note.rs` - `track: Option<TrackId>`, `with_track()`
- `src/sequencer/ids.rs` - `TrackId` exporteras
- `src/engine/sequencer_engine.rs` - `ActiveNote::track`, `stop_notes_on_track()`
- `src/io/import/tracker.rs` - Sätter `TrackId` vid import
- `src/modules/sample_player.rs` - Smart `ReleaseMode` i `load_sample()`
- `src/gui/instrument_rack.rs` - Focused instrument vid val/borttagning
- `src/gui/egui_backend.rs` - Focused instrument vid start/import

---

## [0.42.0] - 2025-12-11
### Added - Song Playback & Sample Waveform Display

Uppspelning av importerade tracker-filer och visualisering av samples.

#### Transport-kontroll
- **`SetSong` kommando** - Skickar song till `SequencerEngine` för uppspelning
- **Transport-kommandon** i `synth_engine.rs`: `Play`, `Stop`, `Pause`, `Rewind`
- **GUI-knappar** i sequencer-vyn kopplade till engine

#### Sample Waveform-visning
- **`WaveformOverview`** utökad med stereo-stöd:
  - `peaks_left` / `peaks_right` för separata kanaler
  - `is_stereo` flagga
  - Nya metoder `peak_left_at()`, `peak_right_at()`
- **Ny widget** `draw_sample_waveform()` i `widgets/waveform_display.rs`:
  - Mono: Enkel centrerad vågform
  - Stereo: L/R separerade (ovanför/under mittlinje)
  - Stöd för playback-position indikator
- **Integration** i `patch_editor.rs` - Waveform visas automatiskt i SamplePlayer-moduler

#### Filer som ändrats
- `src/engine/commands.rs` - `SetSong` kommando
- `src/engine/synth_engine.rs` - Transport-hantering
- `src/engine/hub.rs`, `src/engine/transactions.rs` - SetSong stöd
- `src/gui/views/sequencer.rs` - `TransportAction` enum, knappkoppling
- `src/gui/egui_backend.rs` - Transport-routing, SetSong vid import
- `src/types/sample.rs` - Stereo `WaveformOverview`
- `src/gui/widgets/waveform_display.rs` - Ny widget
- `src/gui/patch_editor.rs` - Waveform i SamplePlayer
- `src/gui/module_panel.rs` - `waveform_overview` i `ModulePanelState`

---

## [0.41.0] - 2025-12-11
### Added - Tracker Import (MOD/XM/S3M)

Import av klassiska tracker-filer direkt i synthen.

#### Import-arkitektur (`src/io/import/`)

- **`SongImporter` trait** - Utökningsbar arkitektur för filformat:
  - `name()` - Importernamn
  - `extensions()` - Stödda filändelser
  - `can_import()` - Formatdetektering
  - `import()` - Returnerar `ImportResult<ImportedSong>`

- **`ImportedSong`** - Resultat med:
  - `song: Song` - Konverterade patterns och arrangement
  - `samples: Vec<Arc<Sample>>` - Extraherade samples

- **`ImportError`** enum - Typade fel (NotFound, Io, UnsupportedFormat, Parse, InvalidData)

#### Tracker-loader (`src/io/import/tracker.rs`)

- **`TrackerImporter`** - Stöd för MOD, XM, S3M via `xmrs` crate
- **Sample-konvertering**:
  - 8-bit: `i8 / 128.0` → `SampleValue`
  - 16-bit: `i16 / 32768.0` → `SampleValue`
  - Stereo och float-format stöds
- **Pattern-konvertering**:
  - xmrs `TrackUnit` → interna `Note`
  - Beräknar ticks per rad baserat på tracker-speed
  - Hanterar `Pitch::None` och `Pitch::Off` (keyoff)
  - Sätter `RowResolution` för korrekt tracker-visning
- **Tempo-hantering**: Använder modulens `default_bpm` och `default_tempo`

#### GUI-integration

- **File → Import Song...** menyval
- **Import Song-dialog** (`src/gui/dialogs.rs`):
  - Sökvägs-inmatning
  - Validering (filen måste finnas)
  - Stödda format visas (.mod, .xm, .s3m)
- **Automatisk vy-växling** till Sequencer efter import
- **Status-toast** visar filnamn och antal samples

#### Beroenden

- `xmrs = { version = "0.9", features = ["std", "import"] }`

---

## [0.40.0] - 2025-12-10
### Added - The Hybrid Tracker

En modern tracker-arkitektur med View Adapter-mönster för framtida Piano Roll-stöd.

#### Datastruktur (`src/sequencer/pattern.rs`)

- **`TrackCell` enum** - Cell-baserad representation för tracker-style editing:
  - `Empty` - Tom cell
  - `Note { pitch, instrument, velocity }` - Not-event
  - `NoteOff` - Not-avstängning (`===`)
  - `Effect { command, value }` - Effekt-cell

- **`TrackerGrid`** - Grid-baserad lagring (rows × tracks):
  - `get()` / `set()` / `clear()` - Cell-operationer
  - `resize()` - Dynamisk storlek
  - `effects()` - Effekt-kolumner per cell

- **Pattern dual representation**:
  - `notes: Vec<Note>` - Piano roll-format (start, duration)
  - `grid: Option<TrackerGrid>` - Tracker-format (lazy-initialized)
  - `sync_grid_from_notes()` / `sync_notes_from_grid()` - Synkronisering

#### View Adapter (`src/sequencer/view/render.rs`)

- **`ColumnType`** enum: `RowIdx`, `Note`, `Instrument`, `Volume`, `EffectType`, `EffectValue`
- **`render_cell_text()`** - Returnerar `Cow<'static, str>` för noll-allokering på statiska strängar
- **`cell_color()`** - Färgbestämning baserat på cell-typ och cursor
- **`draw_track_cell()`** - Ny rendering via View Adapter
- **`draw_tracker_grid_from_pattern()`** - Optimerad rendering direkt från TrackerGrid

#### Input & Fokus (`src/sequencer/view/input.rs`)

- **`TrackerCommand`** enum - Engine-kommunikation (SetNote, SetNoteOff, ClearCell, etc.)
- **`TrackerCursor`** - Cursor-position (row, track, column)
- **`key_to_semitone()`** - Piano-keyboard layout (Z=C, S=C#, Q=C+12)
- **`hex_char_to_value()`** - Hex-inmatning för instrument/volym

#### Prestanda

- Virtual scrolling via `egui_extras::TableBuilder`
- `Cow<str>` för statiska strängar (---,  ===, ..) undviker heap-allokering
- Beat-markering var 4:e rad

#### Nya tester

- `test_track_cell_creation`
- `test_tracker_grid_basic`
- `test_tracker_grid_resize`
- `test_pattern_grid_sync`
- `test_pattern_set_cell`
- `test_grid_note_off_handling`
- Input-tester: `test_key_to_semitone`, `test_to_midi_note`, `test_hex_char_to_value`

---

## [0.39.0] - 2025-12-10
### Changed - Eliminera Duplicerade Typnamn

Alla strukturer och typer med samma namn i olika moduler har bytt namn till unika, beskrivande namn.

- **DSP-typer:**
  - `dsp::FilterType` → `SvfFilterType` (SVF = State Variable Filter)

- **Sequencer-typer:**
  - `sequencer::InstrumentId` → `SeqInstrumentId` (+ type alias för bakåtkompatibilitet)
  - `sequencer::InstrumentParam` → `AutoInstrumentParam` (automation-relaterad)

- **Engine shared state:**
  - `shared_state::MeterState` → `SharedMeterState` (trådsäker version)
  - `shared_state::TransportState` → `SharedTransportState` (trådsäker version)

- **Patch-serialisering:**
  - `patch::ModuleType` → `PatchModuleType`
  - `params::to_module_type()` → `to_patch_module_type()`

- **GUI widgets:**
  - `widgets::Port` → `PortWidget`
  - `widgets::PortType` → `WidgetPortType`
  - `widgets::PortDirection` → `WidgetPortDirection`
  - `patch_editor::VisualizerType` → `PaletteVisualizerType`

- **SampleRate:**
  - Behålls som två typer: `audio::SampleRate(u32)` för hårdvara, `types::SampleRate(f32)` för DSP
  - `From`-implementationer finns för konvertering mellan dem

### Why
Eliminerar förvirring vid import och gör det tydligt vilken typ som avses i varje kontext.

---

## [0.38.0] - 2025-12-10
### Added - Typsäkra Newtypes i Core Traits

Fullständig typning av `ProcessContext`, `PolyModule` och `AudioEffect` med newtype-mönstret.

- **Nya typer (`src/types/`):**
  - `BeatPosition` - Position i beats (musikalisk tid, f64)
    - Metoder: `bar()`, `beat_in_bar()`, `to_seconds()`, `quantize()`, `advance_samples()`
    - Konstant: `ZERO`
  - `Velocity` - MIDI velocity (0.0-1.0)
    - Metoder: `from_midi()`, `to_midi()`, `curve()`, `scale()`, `lerp()`
    - Konstanter: `ZERO`, `MAX`, `DEFAULT`, `PIANO`, `FORTE`
  - `MidiChannel` - MIDI-kanal (1-16)
    - Metoder: `as_u8()`, `as_index()`
    - Konstanter: `CH1`, `DRUMS`

- **Uppdaterad `ProcessContext`:**
  - `samples: usize` → `samples: SampleCount`
  - `position_beats: f64` → `position_beats: BeatPosition`

- **Uppdaterad `PolyModule` trait:**
  - `note_on(_note: MidiNote, _velocity: f32)` → `note_on(_note: MidiNote, _velocity: Velocity)`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

- **Uppdaterad `AudioEffect` trait:**
  - `set_mix(mix: f32)` → `set_mix(mix: NormalizedValue)`
  - `get_mix() -> f32` → `get_mix() -> NormalizedValue`
  - `tail_samples() -> usize` → `tail_samples() -> SampleCount`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

### Changed
- Alla 8 effekter uppdaterade med typsäkra signaturer
- Alla moduler som implementerar `PolyModule` uppdaterade
- `VoiceState`, `VoiceAllocator`, `Voice` använder nu `Velocity`
- Engine-filer använder typsäkra typer genomgående

---

## [0.37.0] - 2025-12-10
### Added - ValueRange Typ för Parameterhantering

- **Ny `ValueRange`-typ (`src/types/range.rs`):**
  - Kapslar in `min`, `max`, `default` i en enda typ
  - Fördefinierade konstanter: `UNIT`, `UNIT_ZERO`, `UNIT_ONE`, `BIPOLAR`, `PERCENT`, `TOGGLE`
  - Konstruktorer: `new()`, `symmetric()`, `from_min()`, `from_max()`
  - Metoder: `span()`, `contains()`, `clamp()`, `normalize()`, `denormalize()`, `lerp()`

- **Uppdaterad `ParameterDescriptor`:**
  - Ersatte separata `min`, `max`, `default` fält med `range: ValueRange`
  - Ny `value_range()` builder-metod
  - Bakåtkompatibla accessor-metoder: `min()`, `max()`, `default_value()`

- **Uppdaterad `ResponseCurve`:**
  - `normalize()` och `denormalize()` tar nu `ValueRange` istället för separata parametrar

### Changed
- GUI widgets (`Knob`, `module_panel`, `patch_editor`) använder nu `param.range.default/min/max`
- `ParameterWidget` i `src/ui/mod.rs` använder `range.normalize()` och `range.span()`

---

## [0.36.1] - 2025-12-10
### Changed - Kompaktare Modulstorlekar

- **Minskade modulstorlekar för att matcha kompakta widgets:**
  - Min bredd: 180px → 140px
  - Min höjd: 100px → 80px
  - Auto-layout gap: 10px → 8px
  - Modul X-offset: 210px → 160px
  - Modul Y-offset: 320px → 200px

- **Uppdaterade visualizer-storlekar:**
  - Meter: 80x100 → 60x80
  - Oscilloscope: 160x80 → 120x60
  - ADSR: 140x50 → 120x50

---

## [0.36.0] - 2025-12-10
### Added - ADSR Envelope Editor & Kompakta Knobs

- **Interaktiv EnvelopeEditor (`src/gui/widgets/envelope.rs`):**
  - Draggbara kontrollpunkter för Attack, Decay, Sustain, Release
  - Grid-bakgrund (5x5 linjer) för visuell referens
  - Tooltips visar värden vid hover/drag (ms/s för tid, % för sustain)
  - Total ljudtid (Σ A+D+R) visas i övre högra hörnet
  - Dynamisk skalning anpassar sig efter faktiska värden
  - Glow-effekt runt aktiva kontrollpunkter

- **Kompakta Knobs (`src/gui/widgets/knob.rs`):**
  - Storlek minskad: 72px → 36px (default), 56px → 28px (small), 88px → 48px (large)
  - Värde visas nu som tooltip istället för text i mitten
  - Borttagen yttre ram för kompaktare utseende
  - Arc-bredd och indikator skalas med knob-storlek

- **Återanvändbar Tooltip-modul (`src/gui/widgets/tooltip.rs`):**
  - `draw_value_tooltip()` - generell tooltip på valfri position
  - `draw_tooltip_right_of()` - för knobs (höger om cirkeln)
  - `draw_tooltip_above()` - för envelope-punkter
  - Ritas på `Order::Tooltip`-lagret för att alltid visas överst

- **Förbättrad Port-layout (`src/gui/patch_editor.rs`):**
  - Inputs vänsterställda, outputs högerställda med flexibelt mellanrum
  - Mindre labels (9px) för kompaktare vy
  - Tooltips med fullständig beskrivning vid hover

- **Förbättrade Topbar-knappar:**
  - Power: `●`/`○` med grön/grå färg och detaljerad hover-text
  - Connectivity: `◆`/`◇` med färgkodning och förklaringar
  - Större klickyta (20x20px) för bättre användbarhet

### Changed
- ADSR-moduler använder nu EnvelopeEditor istället för sliders
- Endast knob-parametrar (Vel Sens, kurvor) visas under envelope-editorn

---

## [0.35.0] - 2025-12-10
### Added - Tracker View (FastTracker II-inspirerad sequencer)
- **Ny TrackerViewState (`src/sequencer/view/state.rs`):**
  - Stark typ `RowIndex` för typsäker rad-navigering
  - `TrackerColumn` enum: Note, Instrument, Volume, EffectType, EffectValue
  - `TrackerViewState` med: cursor_row, cursor_track, cursor_column, octave, step_size, etc.
  - Hjälpmetoder: `cursor_down/up/left/right`, `ensure_cursor_visible`, `octave_up/down`

- **Tracker Grid Rendering (`src/sequencer/view/render.rs`):**
  - Använder `egui_extras::TableBuilder` för virtuell scrollning
  - `draw_tracker_grid()` - renderar pattern med färgkodade celler
  - `TrackerColors` - anpassningsbara färger för tracker-vyn
  - Stöd för Note, Instrument, Volume och Effect-kolumner

- **Sequencer View (`src/gui/views/sequencer.rs`):**
  - Toolbar med: Transport (⏮▶⏹⏺), Octave-väljare (F1/F2), Step-väljare, Follow-toggle
  - Full tangentbordsnavigering: Piltangenter, Home/End, PageUp/Down, Tab
  - Piano-tangentbordslayout för noter: Z=C, S=C#, X=D... Q=C+1, W=D+1, etc.
  - Placeholder för "No Song" med kortkommandon

- **Beroenden:**
  - Lagt till `egui_extras = "0.33"` för TableBuilder

---

## [0.34.11] - 2025-12-09
### Fixed - Auto Layout: ADSR/Modulation-moduler håller sig inom vyn
- **Korrigerad beräkning av modulation-radens Y-position:**
  - `mod_row_index` sätts nu korrekt till `main_rows` (efter alla huvudmoduler)
  - `mod_base_y` begränsas med `.min(max_y)` för att garantera att modulen håller sig inom canvas
- **ADSR/Envelope-moduler överlappar inte längre pianot/keyboardet**

---

## [0.34.10] - 2025-12-09
### Fixed - Auto Layout: Moduler håller sig garanterat inom vyn
- **Modulhöjden anpassas nu automatiskt:**
  - Om alla rader inte får plats med MIN_MODULE_HEIGHT (140px), krymps modulerna
  - Absolut minimum 60px höjd för att alltid passa
  - Formeln: `((available_height - GAP) / total_rows - GAP).max(60.0)`
- **Undersidan av moduler går aldrig utanför canvas:**
  - `max_y` beräknas som `available_rect.max.y - module_height - GAP`
  - `clamp_pos()` garanterar att modulens position + höjd alltid är inom vyn
- **Inga moduler överlappar keyboard/piano**

---

## [0.34.9] - 2025-12-09
### Improved - Auto Layout med strikta gränser
- **Okopplade moduler hanteras separat:**
  - Moduler utan kopplingar placeras nu i högra kolumnen
  - Staplas vertikalt för att undvika överlappning
- **Strikt gränskontroll:**
  - `clamp_pos()` funktion säkerställer att ALLA moduler håller sig inom canvas
  - Inga moduler kan överlappa pianot/keyboardet
  - `max_x` och `max_y` beräknas från modulstorlek
- **Förbättrad modulkategorisering:**
  - Kopplade huvudmoduler: vänster-till-höger efter signalflöde
  - Kopplade modulationsmoduler: under deras mål
  - Okopplade moduler: egen kolumn till höger
- **Nya tester:**
  - `test_disconnected_modules_in_corner` - verifierar att okopplade moduler placeras rätt
  - `test_linear_chain_within_bounds` - verifierar gränser

---

## [0.34.8] - 2025-12-09
### Improved - Auto Layout fyller hela vyn
- **Algoritmen omskriven för att dynamiskt beräkna modulstorlek:**
  - Modulbredden beräknas så att alla kolumner fyller tillgänglig bredd
  - Modulhöjden beräknas så att alla rader (inklusive modulation) fyller tillgänglig höjd
  - Minimumstorleks-begränsningar: 150x120 px
- **Moduler håller sig nu inom vyn:**
  - Canvas-rektangeln (exklusive sidopaneler och keyboard) används som begränsning
  - Modulation placeras under huvudsignalvägen med 25px gap
- **Förenklad API:**
  - `calculate_layout()` tar nu `Rect` direkt istället för `LayoutConfig`
  - Alla beräkningar sker internt baserat på tillgänglig yta

---

## [0.34.7] - 2025-12-09
### Improved - Auto Layout toolbar och höjdberäkning
- **Auto Layout-knappen ritas nu i foreground layer** - syns alltid överst, även när moduler dras över
- **Förbättrad layoutalgoritm:**
  - Moduler på samma djup-nivå staplas nu vertikalt (samma kolumn)
  - Djup-nivåer fortsätter till nästa kolumn
  - Om kolumnerna tar slut, fortsätter layouten på en ny "rad av kolumner"
- **Korrekt höjdhantering:**
  - Modulationsmoduler placeras baserat på faktiskt antal rader som används
  - Begränsning till `max_main_rows + 1` för att undvika att modulation hamnar för långt ner

---

## [0.34.6] - 2025-12-09
### Improved - Auto Layout respekterar tillgängligt utrymme
- **Algoritmen tar nu hänsyn till:**
  - Den tillgängliga canvas-ytan (exklusive sidopaneler)
  - Modulernas storlek (200x180 px)
  - Gap mellan moduler (20 px)
  - Radbrytning när kolumner inte får plats
- **Nya konfigurations-parametrar:**
  - `area_min`, `area_max` - Tillgänglig layoutyta
  - `module_size` - Modulstorlek för beräkningar
  - `gap_x`, `gap_y` - Avstånd mellan moduler
  - `modulation_gap` - Extra avstånd till modulations-raden
- **Moduler överlappar inte längre** - algoritmen placerar moduler i ett rutnät
- **Canvas-rect skickas nu till layoutfunktionen** för korrekt placering

---

## [0.34.5] - 2025-12-09
### Fixed - Auto Layout-knappen fungerar nu
- **Problemet:** Auto Layout-knappen uppdaterade interna positioner men egui:s `Window`-widget ignorerade detta eftersom `default_pos()` bara sätter positionen första gången fönstret ritas.
- **Lösningen:** Lade till `needs_reposition: HashSet<ModuleId>` i `PatchEditor` som markerar moduler som behöver omplaceras. När en modul är markerad används `current_pos()` istället för `default_pos()`, vilket tvingar fönstret till den nya positionen.
- Auto Layout fungerar nu som förväntat - moduler flyttas till sina beräknade positioner baserat på signalflödet.

---

## [0.34.4] - 2025-12-09
### Added - Workspace GUI Navigation
- **gui/app/state.rs** - Nya navigations-enums:
  - `AppView` - Rack/Sequencer/Mixer vy-val med `icon()` och `label()` metoder
  - `TopPanel` - None/Midi/Engine för expanderbara paneler (förberett för framtida funktionalitet)
- **gui/views/layout.rs** - Top bar och drawer komponenter:
  - `TopBarContext` struct för att skicka state till top bar
  - `draw_top_bar()` - Komplett menybar med vy-flikar
  - `draw_top_drawer()` - Expanderbar panel för MIDI/Engine (förberedd)
- **gui/views/rack.rs** - Rack-vy komponent:
  - `RackContext` struct för instrument rack state
  - `draw_instrument_rack()` och `draw_empty_state()` funktioner
- **gui/views/sequencer.rs** - Sequencer-vy (placeholder)
- **gui/views/mixer.rs** - Mixer-vy (placeholder)

### Changed - View Routing
- **egui_backend.rs** - Vy-router implementerad:
  - `active_view: AppView` fält i `SynthApp`
  - Vy-flikar i menyraden (🎛️ Rack, 🎹 Sequencer, 🎚️ Mixer)
  - Toolbar och instrument rack visas endast i Rack-vyn
  - Match-statement för vy-routing i CentralPanel

---

## [0.34.3] - 2025-12-09
### Improved - Prestandaoptimering & Slutfört Type Hardening

**Del 1: InternPool optimering (prestandakritisk)**
- `PortName` har nu **compile-time konstanter** (`PortName::IN`, `PortName::OUT`, etc.)
- **Ingen låsning krävs** för standardportnamn i ljudtråden
- 23 fördefinierade port-IDn: `IN`, `OUT`, `IN_L`, `IN_R`, `OUT_L`, `OUT_R`, `FREQ`, `FREQ_CV`, `GATE`, `CUTOFF_CV`, `RESONANCE_CV`, `PWM`, `FM`, `PM`, `SYNC`, `LEVEL`, `PAN`, `RATE_CV`, `CV`, `PAN_CV`, `LEFT`, `RIGHT`, `VELOCITY`
- Gamla metoder (`input()`, `output()`, etc.) är deprecated

**Del 2: InputPorts::get optimering**
- `InputPorts::get(name: PortName)` - direkt `u32`-jämförelse, **O(1) utan strängjämförelse**
- `InputPorts::get_str(name: &str)` - convenience-metod för dynamiska portnamn
- Alla moduler uppdaterade att använda `PortName::*` konstanter

**Del 3: Nya state enums**
- `FreezeState` - `Unfrozen`/`Frozen` för reverb/oscilloscope freeze
- `Polarity` - `Normal`/`Inverted` med `multiplier()` metod
- `TempoSyncState` - alias för `SyncMode`

**Del 4: Modulmigrering bool → enum**
- `KeyboardPanner`: `invert: bool` → `polarity: Polarity`
- `Delay`: `tempo_sync: bool` → `tempo_sync: TempoSyncState`
- `Mixer`: `mute: bool` → `mute_state: MuteState`, `limit: bool` → `limit_mode: LimitMode`

**Del 5: Parametrar uppdaterade**
- `KeyboardPannerParam::Invert(Polarity)` istället för `bool`
- Alla `as_f32()` och `with_f32()` metoder uppdaterade

**Del 6: SamplePlayer (redan optimerad)**
- `Arc::clone` sker redan innan for-loopen - korrekt implementerad

---

## [0.34.2] - 2025-12-09
### Improved - Fas 3 & 4: Type Hardening Complete
- **Fas 3 - Arkitektur & Prestanda:** Verifierad - redan optimerad
  - `PortName` (internad sträng) redan implementerad i `types/interned.rs`
  - `EffectCommand` enum redan typat i `sequencer/effects.rs`
  - HashMap-lookups använder `&str` utan allokeringar i audio-tråden
- **Fas 4 - FilterState wrapper:**
  - `Filter` (SVF) använder nu `FilterState` istället för råa `f32`
  - `ic1eq` och `ic2eq` integrator-state är nu typsäkra
  - Konsistent med `LadderFilter` som redan använde `FilterState`

---

## [0.34.1] - 2025-12-09
### Improved - Fas 2: Sampling & Uppspelning
- **types/sample.rs** - `Interpolation` enum utökad med GUI-stöd:
  - `ALL` konstant med alla 7 interpolationslägen
  - `name()`, `id()`, `index()`, `from_index()` metoder
  - `to_choices()` för GUI-dropdown
- **types/state.rs** - `NoteReleaseState` enum tillagd:
  - `Held` - Ton hålls, normal uppspelning med looping
  - `Released` - Ton släppt, spelar till slut utan looping
  - Metoder: `is_released()`, `is_held()`, `release()`, `hold()`
- **SamplePlayer** - Refaktorerad med semantisk state:
  - `note_release_state: NoteReleaseState` istället för `releasing: bool`
  - Tydligare kodintention vid note-on/note-off hantering

---

## [0.34.0] - 2025-12-09
### Added - Type Hardening: Semantic State Enums
- **types/state.rs** - Ny modul för semantiska tillstånds-enums som eliminerar "Boolean Blindness"
  - `EnableState` - Enabled/Disabled
  - `MuteState` - Unmuted/Muted
  - `SoloState` - Normal/Solo
  - `BypassState` - Active/Bypassed
  - `SyncMode` - Free/TempoSync
  - `RetriggerMode` - Continue/Retrigger
  - `ClipMode` - Off/Soft/Hard
  - `LimitMode` - Enabled/Disabled
- **types/sample.rs** - Flyttade `LoopMode` och `ReleaseMode` från engine/params/modules.rs
  - Tillhör nu types-modulen där sampling-relaterade typer samlas
  - Bakåtkompatibla re-exports i engine/params/modules.rs

### Changed - Module Refactoring with State Enums
- **SharedEngineState** - `ModuleStateSnapshot` använder nu:
  - `bypass_state: BypassState` istället för `bypassed: bool`
  - `mute_state: MuteState` istället för `muted: bool`
  - `solo_state: SoloState` istället för `solo: bool`
- **Amplifier** - `clip_mode: ClipMode` istället för `soft_clip: bool`
  - Stöd för Off, Soft (tanh), och Hard clipping
- **LFO** - Använder nu:
  - `sync_mode: SyncMode` istället för `tempo_sync: bool`
  - `retrigger_mode: RetriggerMode` istället för `retrigger: bool`
- **StereoOutput** - Använder nu:
  - `mute_state: MuteState` istället för `muted: bool`
  - `limit_mode: LimitMode` istället för `limit_enabled: bool`

---

## [0.33.27] - 2025-12-09
### Improved - GUI Views Module
- **gui/views/** - Ny modul för återanvändbara GUI-komponenter
  - `views/master_effects.rs` - `MasterEffectParams` och `MasterEffectUiState` typer
  - `views/meters.rs` - `draw_meter()` och `draw_meter_horizontal()` funktioner
  - Reducerade egui_backend.rs från 2698 till 2482 rader (~216 rader flyttade)

---

## [0.33.26] - 2025-12-09
### Improved - Code Organization Refactoring
- **gui/input.rs** - Ny modul för keyboard input hantering
  - Extraherade `KEY_MAP` konstant och `handle_keyboard_input()` från egui_backend.rs
  - `KeyboardInputState` struct för att samla input state innan mutation
  - Reducerade egui_backend.rs med ~65 rader
- **SynthEngine command handlers** - Refaktorerade 620-raders `handle_command()` match-block
  - Extraherade 30+ handler-metoder grupperade i kategorier:
    - Instrument management (add, remove, set params, channel, enabled, solo)
    - Note control (note on, note off, all notes off)
    - MIDI controllers (pitch bend, mod wheel, aftertouch, poly aftertouch)
    - Global parameters (master volume, glide time)
    - Voice/module parameters (set voice param, set module param)
    - Reset/clear (reset, clear all modules)
    - Effects (bypass, effect param, enabled, visualizers, add/remove effect)
    - Modular routing (add/remove module, connect, disconnect)
  - Tydligare kodseparation och enklare underhåll

---

## [0.33.25] - 2025-12-09
### Added - StereoSample Type & DSP Module
- **StereoSample** - Ny typ i `types/audio.rs` för stereo-samples
  - Ersätter `(f32, f32)` och `[f32; 2]` för stereosignaler
  - Metoder: `new()`, `from_mono()`, `apply_gain()`, `apply_pan()`, `to_mono()`, `mix()`, `soft_clip()`, `hard_clip()`, `peak()`
  - Implementerar `Add`, `Sub`, `Mul<f32>`, `From<(f32, f32)>`, `From<[f32; 2]>`
- **src/dsp/** - Ny modul för återanvändbara DSP-primitiver
  - `dsp/oscillators.rs` - `poly_blep()` och `poly_blep_integrated()` för band-limiterade vågformer
  - `dsp/filters.rs` - `SvfCoeffs` och `BiquadCoeffs` för filterberäkningar
  - `dsp/delay.rs` - `DelayLine` och `InterpolatedDelayLine` för delay-effekter

### Improved - Module Refactoring
- **KeyboardPanner** - Använder nu `StereoSample::apply_pan()` internt
- **StereoOutput** - Refaktorerad att använda `StereoSample` för beräkningar
- **Delay effect** - Använder nu `StereoSample` för stereobearbetning
- **Reverb effect** - Använder nu `StereoSample` för stereobearbetning
- **Oscillator** - Använder nu `dsp::oscillators::poly_blep()` istället för lokal metod

---

## [0.33.24] - 2025-12-09
### Improved - Type Methods & Sequencer Types
- **MidiNote::transpose** - Flyttade transponeringslogik till typen, returnerar `Option<MidiNote>`
- **Pitch::transpose** - Sequencer-typ uppdaterad till `Semitones`, returnerar `Option<Pitch>`
- **PatternPlacement** - `transpose` nu `Semitones`, `gain` nu `Gain`
- **TempoChange/Song** - `bpm` och `default_tempo` nu `Bpm` istället för `f32`
- **Pattern::generate_events** - Använder nu `Semitones` för transponering
- **Instrument::transpose_note** - Delegerar nu till `MidiNote::transpose`

---

## [0.33.23] - 2025-12-09
### Improved - Strict Type Hardening
- **StereoBalance** - Ny typ för stereopanorering med constant-power gains
- **KeyboardPanner** - Använder nu `MidiNote`, `BipolarValue`, `StereoBalance` istället för primitiver
- **BodyResonance** - Filterstate arrays använder nu `FilterState` istället för `f32`
- **Instrument transpose** - Använder nu `Semitones` istället för `i8`
- **InstrumentParam::Transpose** - Uppdaterad till `Semitones`
- **Oscilloscope** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **LevelMeter** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **GUI InstrumentUiState** - `transpose` använder nu `Semitones`

---

## [0.33.22] - 2025-12-09
### Optimized - GUI Rendering with egui Shape Primitives
- **Kablar** - Ersatte manuell Bézier-loop (32 segment × 3 lager) med `CubicBezierShape`
- **Oscilloskop** - Ersatte ~200 `line_segment()` med en `Shape::line()`
- **Waveform-väljare** - Ersatte 31 `line_segment()` per ikon med `Shape::line()`
- **ADSR Envelope** - Ersatte 4 `line_segment()` med `Shape::line()`
- **Draw Call Batching** - GPU kan nu rita alla punkter i en operation
- **Automatisk LOD** - `CubicBezierShape` hanterar detaljnivå automatiskt

---

## [0.33.21] - 2025-12-09
### Improved - Enhanced Sample Player
- **PlaybackState** - Ersatte `bool` med typat enum för tydligare state
- **SampleName** - Newtype för sample-namn istället för rå `String`
- **ReleaseMode** - Tre lägen: `Immediate`, `PlayToEnd`, `PlayToLoop` för flexibel note-off hantering
- **Velocity Sensitivity** - Parameter för hur mycket velocity påverkar volym (0-100%)
- **Nya interpolationer** - Hermite, Lagrange, Sinc8, Sinc16 för högkvalitativ uppspelning
- **Loop Crossfade** - 0-50ms crossfade vid loop-punkter för klickfri looping
- **Root Key Detection** - Automatisk detektion av grundton från filnamn (t.ex. "Piano_C3.wav")
- **WaveformOverview** - Pre-beräknad waveform för effektiv visualisering
- **PlaybackPositionBuffer** - Atomic position buffer för lock-free GUI-synk

### Changed
- Konsoliderade `LoopMode` - endast en definition i params/modules.rs
- Tog bort `loop_mode` från `Sample` struct (hör till spelaren, inte samplen)

---

## [0.33.20] - 2025-12-09
### Added - Sample Player & Sample Manager
- **SamplePlayer** - Ny modul för uppspelning av WAV-samples
  - Pitch tracking (transponerar automatiskt baserat på spelade noter)
  - Loop modes: Off, Forward, Backward, PingPong
  - Start/End positions för sample trimming
  - Loop Start/End för preciserad loop-region
  - Speed-kontroll (0.1x - 4.0x)
  - Interpolation: Nearest, Linear, Cubic (Catmull-Rom)
  - Stereo och mono-samples stöds
- **SampleManager** - GUI-thread sample loader med caching
  - Laddar WAV-filer (8/16/24/32-bit int, 32-bit float)
  - Cache förhindrar dubbel-laddning av samma fil
  - Thread-safe via `Arc<Sample>`
- **Nya typer** - `SampleValue`, `PlaybackPosition`, `SampleIndex`, `PlaybackSpeed`, `ChannelMode`, `Interpolation`, `PlaybackDirection`
- **hound** - Nytt beroende för WAV-läsning

---

## [0.33.19] - 2025-12-08
### Added - Improved Cables & Auto-Layout
- **Kablar med gravitation** - Kablar hänger nedåt med naturlig "sag" (15% av avståndet)
- **Skuggor** - Svart skugga under kablar för djupkänsla
- **Semi-transparens** - Kablar är delvis genomskinliga (alpha 180)
- **Highlight på hover** - Röd glow-effekt från theme().colors.accent_red
- **Dragging-kablar** - Mindre sag (5%) för responsiv känsla
- **Auto-Layout** - Ny knapp "📐 Auto Layout" som organiserar moduler
  - Vänster→höger baserat på signalflöde (BFS från sources)
  - Envelopes/LFOs placeras under huvudsignalvägen
  - Konfigurerbara avstånd via LayoutConfig

---

## [0.33.18] - 2025-12-08
### Fixed - Parameter Routing for Arbitrary Modules
- **SetModuleParameter** används nu för alla voice-moduler istället för SetVoiceParameter
- Fixar parameter-routing för moduler utanför PolyModule enum (env-3, amp-2, sub-1, nse-1, kbp-1, etc)
- **Grand Piano patch** fungerar nu korrekt med alla 3 envelopes
- Tog bort redundant get_voice_module_for_param funktion
- Rensade oanvända imports (PolyModule, Param)

---

## [0.33.17] - 2025-12-08
### Changed - Physical Modeling Cleanup
- **Removed** StringResonator, ResonatorBank, VelocityMapper (fungerade inte korrekt)
- **KeyboardPanner** - Not-baserad stereopanorering nu registrerad i GUI-menyn
- **BodyResonance** - Resonanskropp-simulering nu registrerad i GUI-menyn
- **MechanicalNoise** - Mekaniska ljud nu registrerad i GUI-menyn
- **Grand Piano patch** - Uppdaterad med KeyboardPanner för stereo-imaging
- **Physical-menyn** i modulpaletten med de 3 fungerade modulerna

---

## [0.33.16] - 2025-12-08
### Added - Physical Modeling Modules
- **StringResonator** - Karplus-Strong string synthesis med inharmonicitet och dämpning
- **ResonatorBank** - Sympatisk resonans med 1-12 avstämbara strängar
- **KeyboardPanner** - Not-baserad stereopanorering för piano-liknande stereo
- **BodyResonance** - Resonanskropp-simulering (soundboard)
- **VelocityMapper** - Velocity-kurvor (Linear, Soft, Hard, S-Curve, Fixed)
- **MechanicalNoise** - Mekaniska ljud (tangent ner/upp, pedal, hammare)
- **ModuleCategory::PhysicalModeling** - Ny kategori för modulpalett

---

## [0.33.15] - 2025-12-08
### Added - Keyboard Splitting & MIDI Learn
- **KeyRange** - Ny typ för att definiera vilka noter ett instrument svarar på (keyboard splitting)
- **LearnState** - State machine för MIDI learn (Idle, WaitingForLowNote, WaitingForHighNote)
- **Transpose** - Semitone-offset per instrument (-24 till +24)
- **Instrument Rack UI** - Ny rad med Range-visning, Learn-knapp, Full-knapp, Transpose-kontroll
- **KeyRangeLearned event** - Engine skickar event till GUI när range lärs in
- **note_on/note_off** - Kollar key_range och applicerar transpose

---

## [0.33.14] - 2025-12-08
### Improved - GUI Styling & Theme Consistency
- **Master FX Sliders** - Bättre synlighet med mörkare bakgrund och tydlig kontrast
- **WidgetStyle** - Nya fält: knob_arc_segments, slider_rail_height, slider_handle_radius
- **knob.rs** - Använder nu theme().style istället för hårdkodade värden
- **meter.rs** - Använder nu theme().style istället för hårdkodade värden

---

## [0.33.13] - 2025-12-08
### Added - Attenuverters for CV Inputs
- **Filter CutoffMod** - Ny "CV Amt" parameter (-1.0 till +1.0) för cutoff CV
- **Oscillator FmAmount** - Ny "FM Amt" parameter (-1.0 till +1.0) för FM input
- Tog bort MIDI debug-spam

---

## [0.33.12] - 2025-12-08
### Added - Theme System
- 8 färgteman: Dark, Light, Vintage, Neon, Studio, Dracula, Monokai, Solarized Dark
- Tema-väljare i Settings, WidgetStyle för konsistent styling

---

## [0.33.11] - 2025-12-08
### Changed - InputPorts Refactor
- `PolyModule::process()` använder `InputPorts` wrapper istället för HashMap
- Eliminerar HashMap-allokering per audio frame

---

## [0.33.10] - 2025-12-08
### Fixed - Realtime Audio Allocations
- Eliminerade `AudioBuffer::new()` i audio thread (~187 allok/sek)
- `Connection` använder `PortName` (Copy) istället för String

---

## [0.33.9] - 2025-12-08
### Added - Bypass-knappar
- Power-knapp (⏻) i varje moduls header, bypassade moduler dimmas till 40%

---

## [0.33.8] - 2025-12-05
### Added - Master FX Parameters
- Resizable sidopanel, fullständiga parameterkontroller för alla 8 effekttyper

---

## [0.33.7] - 2025-12-05
### Added - Master FX Sidebar
- Kollapsbar effektlista i sidopanelen med bypass/remove per effekt

---

## [0.33.6] - 2025-12-05
### Added - Mixer & Master Bus
- Solo-knapp, global master effects chain, soft clipper per instrument

---

## [0.33.5] - 2025-12-05
### Fixed - Eliminated unwrap/expect
- Fixade 112 Clippy-varningar för unwrap/expect i produktionskod

---

## [0.33.4] - 2025-12-05
### Maintenance
- Rensade 29 onödiga clippy allows, uppdaterade 15 beroenden

---

## [0.33.3] - 2025-12-05
### Refactored - Clippy Pedantic
- Konfigurerade ~70 pedantic/nursery lints för synth-lämpliga undantag

---

## [0.33.2] - 2025-12-05
### Refactored - Best Practices
- `#[must_use]` på transformationsmetoder, tog bort global `#![allow(dead_code)]`
- Konverterade error-typer till thiserror, reducerade unsafe från 5 till 1 block

---

## [0.33.1] - 2025-12-04
### Refactored - Idiomatic Iterators
- Konverterade for-loopar till iteratorer utanför hot path, behöll for-loopar i DSP

---

## [0.33.0] - 2025-12-04
### Added - Type System Extensions
- `PortName` interning för zero-allocation, `FilterState` DSP-metoder
- `Hertz`, `NormalizedValue`, `MidiChannel`, `BeatDivision` extensions

---

## [0.32.25] - 2025-12-04
### Fixed - Instrument Channel Isolation
- Standardinstrument använder CH1 istället för OMNI
- Real-time safe sequencer (pre-allokerad event buffer)

---

## [0.32.24] - 2025-12-04
### Refactored - DSP Type Hardening
- `FilterState` i Reverb/Phaser, `Hertz::to_tan_coeff()`, `Milliseconds::to_samples()`

---

## [0.32.23] - 2025-12-04
### Fixed - PatchEditor GUI ID Collision
- Window ID inkluderar instrument_id för unika GUI-identifierare
- LadderFilter och NoiseGenerator använder `FilterState`

---

## [0.32.22] - 2025-12-04
### Refactored - Total Type Hardening
- `MidiNote` för PolyModule/EngineCommand, `SamplePosition`/`SampleCount` i Voice
- `BufferIndex` i Flanger/Chorus, `NormalizedValue` i SequencerTrack

---

## [0.32.21] - 2025-12-04
### Refactored - Type Safety and Enums
- `ModuleType::is_voice_module()`, unified `EffectChain` med `ChainSlot`
- Data-bärande `VoiceState` enum (Idle/Active/Releasing/Stealing)

---

## [0.32.20] - 2025-12-04
### Refactored - Per-Instrument Effects
- Flyttade effekter från global MasterBus till per-instrument EffectChain

---

## [0.32.19] - 2025-12-04
### Refactored - Per-Instrument PatchEditor
- Varje instrument äger sin egen PatchEditor, patch-laddning per instrument

---

## [0.32.18] - 2025-12-04
### Refactored - Per-Instrument Voice Architecture
- `voice_graph` ägs av Instrument istället för SynthEngine

---

## [0.32.17] - 2025-12-03
### Refactored - Architectural Terminology
- Renamed: RackView→PatchEditor, VoiceModule→PolyModule, EffectModule→AudioEffect
- SynthPart→Instrument, EffectChain→MasterBus

---

## [0.32.16] - 2025-12-03
### Added - Part Manager UI
- Multi-instrument support med Part Manager panel, MIDI-kanal per part

---

## [0.32.15] - 2025-12-03
### Improved - Knob Widget
- Värde visas i knob-cirkeln, centraliserad formatering i ParameterUnit
- Custom "Share Tech Mono" font

---

## [0.32.14] - 2025-12-03
### Added - MIDI Input Support
- Hardware MIDI via midir med GUI port-väljare, velocity visualization
- Type-safe MIDI parsing, pitch bend, mod wheel, aftertouch

---

## [0.32.13] - 2025-12-02
### Fixed - Stereo Output Parameters
- ModuleCategory::Output tillagd i parameter change handling

---

## [0.32.12] - 2025-12-02
### Removed - Performance Panel
- Tog bort GUI-komponenten, behöll engine-kommandona för framtida MIDI

---

## [0.32.11] - 2025-12-02
### Fixed - Real-time Parameter Updates
- `SetModuleParameter` uppdaterar nu voice_template + alla aktiva voices
- Tog bort 29 ogiltiga oscilloskop-kopplingar från patches

---

## [0.32.10] - 2025-12-02
### Fixed - Critical Audio Routing
- StereoOutput klassificeras som voice module, partiella inputs fungerar
- `ClearAllModules` rensar voice_template

---

## [0.32.9] - 2025-12-02
### Added - Dynamic Module Routing
- Moduler routas automatiskt till rätt graf baserat på typ

---

## [0.32.8] - 2025-12-01
### Refactored - Unified Voice/Graph
- Voice äger ModuleGraph istället för hårdkodad modullista (~400 rader borttaget)

---

## [0.32.7] - 2025-12-01
### Refactored - Unified Param Architecture
- `Param` enum med inbakade typade värden, tog bort `TypedValue`

---

## [0.32.6] - 2025-11-30
### Fixed - Dropdown Parameter Sync
- Dropdown-handlers skickar rätt TypedValue-variant

---

## [0.32.5] - 2025-11-30
### Added - Domain Types for Effects
- `Ratio` (kompression), `BeatDivision` (tempo-sync), `VoiceCount`

---

## [0.32.4] - 2025-11-30
### Fixed - Waveform Selection
- GUI skickar TypedValue::Waveform, tog bort noise från Oscillator (använd NoiseGenerator)

---

## [0.32.3] - 2025-11-30
### Fixed - CV Modulation Drift
- LadderFilter, LFO, Oscillator använder effective values utan att modifiera parametrar

---

## [0.32.2] - 2025-11-30
### Fixed - GUI/Engine Sync
- Startup använder patch_bridge::load_patch() för synkronisering

---

## [0.32.1] - 2025-11-30
### Fixed - Ghost Sound Bug
- ClearAllModules inaktiverar parts

---

## [0.32.0] - 2025-11-30
### Added - Type Safety & GUI
- Type-safe public APIs med Hertz, Cents, Gain, NormalizedValue
- "New Patch" i File-menyn

---

## [0.31.0] - 2025-11-30
### Added - GUI Module Support
- SubOscillator och NoiseGenerator i module palette
- 3 nya example patches

---

## [0.30.0] - 2025-11-30
### Added - DSP Improvements
- Envelope curves (attack_curve, decay_curve, release_curve)
- SubOscillator modul (sine, square, -1/-2 oktav)
- NoiseGenerator modul (white, pink, brown, blue, violet)

---

## [0.29.0] - 2025-11-30
### Refactored - Modular Patch Structure
- 16 patches extraherade till individuella filer i src/patches/

---

## [0.28.1] - 2025-11-30
### Added - Performance Fixes
- Velocity mapping till engine, tempo sync för Delay och LFO
- Module connectivity visualization (Connected/Orphaned/Disconnected)

---

## [0.28.0] - 2025-11-30
### Added - Performance Panel
- Pitch bend (spring-back), mod wheel, velocity mapping knobs

---

## [0.27.0] - 2025-11-30
### Added - Enhanced Oscilloscope & LFO Tempo Sync
- Oscilloskop med waveform history, time division, trigger modes
- LFO tempo sync med beat divisions

---

## [0.26.0] - 2025-11-30
### Added - Engine Events & CPU Tracking
- EngineEvent system med prioriterad kanal
- CPU usage tracking per modul

---

## [0.25.0] - 2025-11-30
### Added - Effect Bypass & Master Volume
- Effect bypass per slot, master volume control

---

## [0.24.0] - 2025-11-29
### Refactored - Hub Architecture
- EventHub för GUI-engine kommunikation, ersatte direkt polling

---

## [0.23.0] - 2025-11-29
### Added - Visual States & Animations
- ModuleVisualState för visuell feedback, cable animations

---

## [0.22.0] - 2025-11-29
### Added - Sequencer Engine
- SequencerEngine med transport, looping, note events

---

## [0.21.0] - 2025-11-29
### Added - Sequencer Data Model
- Song, Pattern, Note, Track, Automation strukturer

---

## [0.20.0] - 2025-11-29
### Added - Effect Chain & Visualizers
- MasterBus med insert effects och visualizers

---

## [0.19.0] - 2025-11-29
### Added - Oscilloscope Widget
- Real-time waveform display i GUI

---

## [0.18.0] - 2025-11-29
### Added - Level Meter Widget
- VU-meter med peak hold och gradient

---

## [0.17.0] - 2025-11-29
### Added - Patch Save/Load
- JSON-baserat patch-format med ModuleBuilder API

---

## [0.16.0] - 2025-11-29
### Added - Voice Allocator
- Polyfoni med voice stealing, mono/poly modes

---

## [0.13.2] - 2025-11-29
### Fixed - Command Queue Overflow
- Ökade COMMAND_BUFFER_SIZE, DroppedModule wrapper

---

## [0.13.1] - 2025-11-29
### Added - Pink Noise & Linear FM
- Pink noise (Voss-McCartney), Linear FM mode, velocity sensitivity

---

## [0.13.0] - 2025-11-29
### Added - Math Oscillator
- 18 algoritmer: SineFM, TanChaos, SuperSaw, WaveFolder, Lorenz, KarplusStrong, etc.
- 6 nya example patches

---

## [0.12.0] - 2025-11-28
### Initial Release
- Moduler: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effekter: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- 10 example patches, piano keyboard, visual cable connections
