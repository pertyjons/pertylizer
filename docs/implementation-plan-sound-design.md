# Implementeringsplan: 60 tekniker och algoritmer för ljudmotorn

> Status: AKTIV | Datum: 2026-02-13 | Basversion: 0.115.0

## Innehåll

1. [Sammanfattning och status](#sammanfattning-och-status)
2. [Del 1: Ljudgenerering (Syntes)](#del-1-ljudgenerering-syntes--20-tekniker)
3. [Del 2: Ljudmanipulering (Effekter)](#del-2-ljudmanipulering-effekter--20-tekniker)
4. [Del 3: Hemliga ingredienser](#del-3-hemliga-ingredienser--20-tekniker)
5. [Prioriterad implementeringsordning](#prioriterad-implementeringsordning)
6. [Genomförda features (historik)](#genomförda-features-historik)

---

## Sammanfattning och status

| Kategori | Implementerad | Delvis | Saknas | Totalt |
|----------|:---:|:---:|:---:|:---:|
| Del 1: Ljudgenerering | 10 | 4 | 6 | 20 |
| Del 2: Effekter | 7 | 1 | 12 | 20 |
| Del 3: Hemliga ingredienser | 10 | 5 | 5 | 20 |
| **Totalt** | **27** | **10** | **23** | **60** |

**45% fullt implementerade, 17% delvis, 38% saknas**

---

## Del 1: Ljudgenerering (Syntes) — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 1 | **Additive** | DELVIS | Chebyshev i Math Oscillator genererar harmoniska, men ingen ren additiv syntes med individuella partialtoner |
| 2 | **Subtractive** | KLAR | Oscillator -> Filter med SVF, 7 filtertyper, 4 karaktärsmodeller (Standard/Fluid/Screamer/Acid) |
| 3 | **FM (Frequency Modulation)** | KLAR | SineFM + FeedbackFM i Math Oscillator + FM-ingång på huvudoscillatorn |
| 4 | **PM (Phase Modulation)** | KLAR | PM-modulationsingång på huvudoscillatorn |
| 5 | **Karplus-Strong** | KLAR | KarplusStrong-algoritm i Math Oscillator |
| 6 | **Wavetable** | KLAR | WavetableOsc med 6 inbyggda banker (Basic/Harmonics/PWM/Formant/Digital/Warm), scanbar position, FM, detune, octave |
| 7 | **Granular** | SAKNAS | Ingen granulärsyntes |
| 8 | **Physical Modeling** | DELVIS | Body Resonance + Mechanical Noise, men inga fullständiga modeller (strängar, rör) |
| 9 | **Vector Synthesis** | SAKNAS | Ingen 2D-mixning mellan 4 källor |
| 10 | **Phase Distortion** | KLAR | PhaseDist-algoritm i Math Oscillator (Casio CZ-stil) |
| 11 | **Formant** | KLAR | Formant-algoritm i Math Oscillator |
| 12 | **LA (Linear Arithmetic)** | SAKNAS | Ingen PCM-attack + syntetisk sustain |
| 13 | **Cellular Automata** | SAKNAS | Ingen cellulär automat-syntes |
| 14 | **Stochastic** | DELVIS | Logistic chaos map i Math Oscillator, men inte fullständig stokastisk syntes |
| 15 | **Frequency Shifting** | SAKNAS | Ingen frekvensskiftning (kräver Hilbert Transform) |
| 16 | **Waveshaping** | KLAR | WaveFolder i Math Oscillator + Waveshaper-effekt med 6 kurvor |
| 17 | **Vosim** | SAKNAS | Ingen pulsbaserad vokalsyntes |
| 18 | **Scanned Synthesis** | SAKNAS | Ingen scanned synthesis |
| 19 | **Wave Terrain** | SAKNAS | Ingen wave terrain-syntes |
| 20 | **Brownian Noise** | KLAR | Brown noise (-6dB/oktav) via integrator i Noise-modulen |

---

## Del 2: Ljudmanipulering (Effekter) — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 21 | **FFT** | SAKNAS | Ingen FFT-baserad bearbetning |
| 22 | **Convolution** | SAKNAS | Ingen faltningsreverb/kabinettsimulering |
| 23 | **Phase Vocoder** | SAKNAS | Ingen pitch-shifting oberoende av tid |
| 24 | **Ring Modulation** | KLAR | RingMod-modul med intern carrier (5 vågformer), keyboard tracking, freq ratio, dry/wet mix |
| 25 | **All-pass Filters** | KLAR | Phaser (kaskadade all-pass) + Schroeder-reverb (2 serie-allpass) |
| 26 | **FDN (Feedback Delay Network)** | DELVIS | Schroeder-reverb (4 comb + 2 allpass), men inte fullständigt FDN |
| 27 | **Compression** | KLAR | Compressor med threshold, ratio, attack, release, makeup gain |
| 28 | **Brickwall Limiting** | SAKNAS | Ingen dedikerad brickwall limiter med look-ahead |
| 29 | **Bitcrushing** | KLAR | Bitcrush i Distortion + Quantize i Waveshaper |
| 30 | **Hilbert Transform** | SAKNAS | Ingen Hilbert-transform |
| 31 | **Chorus** | KLAR | Chorus-effekt med LFO-modulerade delays och stereobreddning |
| 32 | **Flanging** | KLAR | Flanger-effekt med modulerad delay och feedback |
| 33 | **Sidechaining** | SAKNAS | Ingen sidechain-routing |
| 34 | **Auto-correlation** | SAKNAS | Ingen pitch-tracking |
| 35 | **Spectral Subtraction** | SAKNAS | Ingen spektral brusborttagning |
| 36 | **Wavelet Transform** | SAKNAS | Ingen wavelet-analys |
| 37 | **Adaptive Filtering** | SAKNAS | Inga adaptiva filter |
| 38 | **Soft Clipping** | KLAR | SoftClip (tanh) i Distortion + Waveshaper, Tube-distortion |
| 39 | **Envelope Following** | KLAR | EnvelopeFollower-modul med attack/release/sensitivity, one-pole tracking |
| 40 | **Mid-Side Processing** | SAKNAS | Ingen M/S-bearbetning |

---

## Del 3: Hemliga ingredienser — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 41 | **Chaos Generators (Lorenz)** | KLAR | Lorenz-attraktor i Math Oscillator |
| 42 | **Audio Rate Modulation** | KLAR | FM/PM-ingångar på oscillatorer körs i audio rate |
| 43 | **Cross-Modulation** | DELVIS | FM-input finns men ingen dedikerad korsmodulering osc 1 <-> 2 |
| 44 | **Probability Gates** | SAKNAS | Ingen sannolikhetsstyrd triggering |
| 45 | **Microtonal Tuning** | SAKNAS | Ingen mikrotonalitet/Scala-stöd |
| 46 | **Self-Oscillating Filters** | KLAR | Screamer + Acid vid hög resonans |
| 47 | **Slew Rate Limiting** | KLAR | Glide/portamento implementerat i voice.rs med GlideState + GUI-slider |
| 48 | **Logic Operators** | KLAR | BitWise-algoritm i Math Oscillator |
| 49 | **Jitter & Drift** | KLAR | Phase randomization i unison + Lorenz/Logistic som modulationskällor |
| 50 | **Sample & Hold** | KLAR | S&H-vågform i LFO |
| 51 | **Feedback Loops** | DELVIS | FeedbackFM, feedback i delay/reverb, men ingen generell feedback-routing |
| 52 | **Morphing SVF** | KLAR | Fluid-filtret har morph LP <-> BP <-> HP <-> Notch |
| 53 | **BBD Emulation** | SAKNAS | Ingen bucket brigade delay-emulering |
| 54 | **Wavefolding** | KLAR | WaveFolder i Math Osc + Foldback i Distortion + Fold/SineFold i Waveshaper |
| 55 | **Look-ahead Processing** | SAKNAS | Ingen look-ahead limiter |
| 56 | **Polyphonic Aftertouch** | DELVIS | Aftertouch som mod-källa, men oklart om det stöder polyfon aftertouch |
| 57 | **Round Robin Sampling** | SAKNAS | Ingen sampling-engine med round robin |
| 58 | **FFM (FM-kedjor)** | DELVIS | FeedbackFM finns, men inte fria FM-kedjor mellan multipla operatörer |
| 59 | **Resampled Interpolation Errors** | DELVIS | Bitcrush/Quantize ger lo-fi, men ingen medveten aliasing via resampling |
| 60 | **Physical Control Mapping** | SAKNAS | Ingen fysik-baserad kontrollmappning |

---

## Prioriterad implementeringsordning

Prioriteringen baseras på tre faktorer:
- **Effekt** — Hur stor kreativ/musikalisk vinst ger tekniken?
- **Komplexitet** — Hur mycket arbete krävs? Bygger den på befintlig infrastruktur?
- **Synergi** — Förstärker den andra features i motorn?

### Tier 1 — Hög effekt, rimlig komplexitet (Nästa att implementera)

Dessa ger störst bang-for-the-buck och bygger direkt på befintlig infrastruktur.

#### 1.1 Wavetable Syntes (#6)
**Prioritet: MYCKET HÖG** | Uppskattning: ~600 rader | Nya filer: 3

En av de mest efterfrågade syntestyperna. Scanbar position genom wavetable ger timbral rörelse som saknas helt idag. Integrerar direkt med Mod Matrix (position som destination).

- Ny `WavetableOscillator`-modul med scanbar position (0.0-1.0)
- 6+ inbyggda matematiskt genererade wavetables (Basic morph, Harmonics, PWM, Formant, Digital, Warm)
- 2048-sample frames med linjär interpolation mellan frames
- ModMatrix-destination: `WavetablePosition`
- Unison-stöd (återanvänd befintlig unison-infrastruktur)

#### 1.2 Ring Modulation (#24)
**Prioritet: HÖG** | Uppskattning: ~150 rader | Nya filer: 1

Extremt enkel att implementera (multiplikation av två signaler) men ger en helt ny ljudpalett — klockliknande övertoner, metalliska ljud, sci-fi-effekter. Kan göras som voice-modul (osc1 * osc2) eller som global effekt med intern oscillator.

- Ny `RingMod`-modul: multiplicerar input med intern carrier (sine/tri/saw/square)
- Carrier-frekvens modulerbar via CV/Mod Matrix
- Mix-kontroll (dry/wet)
- Carrier freq range: 0.1 Hz - 20 kHz (från sub-audio till audio rate)

#### 1.3 Envelope Follower (#39)
**Prioritet: HÖG** | Uppskattning: ~200 rader | Nya filer: 1

Gör att en signals volym kan styra andra parametrar. Tillsammans med Mod Matrix och Character Filters ger detta dynamisk, levande bearbetning. Kan implementeras som ny ModSource.

- Ny `EnvelopeFollower`-modul med attack/release-tider
- Output: 0.0-1.0 som speglar inkommande signals amplitud
- Sensitivity och threshold
- Registrera som ModSource i Mod Matrix

#### 1.4 MSEG / Looping Envelopes (#4 i befintlig plan)
**Prioritet: HÖG** | Uppskattning: ~830 rader | Nya filer: 2

Multi-stage envelope ger evolverande pads, rytmiska effekter och komplexa transienter. Max 16 segment med loop-punkter, sustain-hold och kurvkontroll. Kräver visuell editor i GUI.

- `Mseg`-modul: 16 segment, loop, sustain-hold, kurvkontroll per segment
- Tempo-synkade segment (BeatDivision)
- Preset-mallar: ADSR, Tremolo, Sidechain-pump
- Visuell MSEG-editor med dragbara brytpunkter

#### 1.5 Slew Rate Limiting / Glide (#47)
**Prioritet: HÖG** | Uppskattning: ~120 rader | Ändringar i befintliga filer

Portamento/glide är en grundfunktion som saknas. Skapar mjuka övergångar mellan noter och kan även mjuka upp modulationssignaler. Enkel implementation — en one-pole lowpass på pitch CV.

- Glide-parameter på voice-nivå (tid i ms, 0 = av)
- Glide-modes: Always, Legato only
- Kan även exponeras som fristående Slew-modul för att mjuka upp CV-signaler

### Tier 2 — Stark kreativ effekt, medelhög komplexitet

#### 2.1 Additive Syntes (fullständig) (#1)
**Prioritet: MEDEL-HÖG** | Uppskattning: ~400 rader | Nya filer: 1

Utöka befintlig Chebyshev till en riktig additiv engine med individuella partialtoner. Ger full kontroll över harmoniskt innehåll — grunden för organ-ljud, klockljud och experimentell syntes.

- Ny `AdditiveOscillator` med 32 individuellt styrda harmoniska
- Spektral profil-kontroll: odd/even-balance, rolloff, tilt
- Per-partial randomisering (fas, amplitud) för organisk känsla
- ModMatrix-destination: harmonisk tilt/rolloff

#### 2.2 Granular Syntes (#7)
**Prioritet: MEDEL-HÖG** | Uppskattning: ~600 rader | Nya filer: 2

Skapar helt unika ljud genom att bryta isär ljud i mikroskopiska fragment. Kräver grain-pool, fönsterfunktioner och stochastisk spridning. Kan arbeta med intern wavetable-data eller sample-buffert.

- Grain-pool (max 64 simultana grains)
- Parametrar: grain size (1-200ms), density, pitch scatter, position scatter
- Window-funktioner: Hann, Gaussian, Trapezoid
- Freeze-mode (loopa position)
- Källa: intern oscillator, wavetable-frame, eller sample-buffert

#### 2.3 BBD Emulation / Analog Delay (#53)
**Prioritet: MEDEL-HÖG** | Uppskattning: ~300 rader | Nya filer: 1

Bucket Brigade Device-emulering ger varm, mörk, analog delay-karaktär som saknas helt. Kompandersystem (kompression -> delay -> expansion), clock noise, och bandbreddsbegränsning.

- Kompander: 2:1 kompression innan delay, 1:2 expansion efter
- Clock noise: subtilt brus kopplat till delay-tid
- Bandbreddsbegränsning: 6kHz lowpass som mörkas vid längre tider
- Wow & flutter: långsam pitch-modulation
- Feedback med mörkning per repeat

#### 2.4 FDN Reverb (fullständigt) (#26)
**Prioritet: MEDEL-HÖG** | Uppskattning: ~400 rader | Nya filer: 1

Uppgradera befintlig Schroeder-reverb till Feedback Delay Network med N kanaler (8 eller 16), Hadamard-matris och frekvensberoende dämpning. Ger mycket tätare, naturligare reverb.

- 8-kanals FDN med Hadamard-mixningsmatris
- Per-kanal tonkontroll (low/high damping)
- Modulerade delay-tider för täthet
- Pre-delay, size, decay, diffusion-kontroller

#### 2.5 Mid-Side Processing (#40)
**Prioritet: MEDEL** | Uppskattning: ~150 rader | Ändringar i befintliga filer

M/S-encoding/decoding ger separat hantering av center (mono) och sidor (stereo). Kan läggas till som läge på EQ och Compressor, eller som fristående modul.

- M/S-encoding/decoding som utility-modul
- M/S-läge på befintlig EQ (separata band för mid/side)
- M/S-läge på befintlig Compressor
- Width-kontroll (0.0 = mono, 1.0 = normal, 2.0 = extra bred)

#### 2.6 Generativa Moduler (#44 Probability Gates + Euclidean etc.)
**Prioritet: MEDEL** | Uppskattning: ~900 rader | Nya filer: 4

Tre generativa moduler: Euclidean Sequencer (algoritmiska rytmer), Turing Machine (muterande skiftregister), Random Gates (probabilistiska triggers). Producerar gate/CV för routing genom Mod Matrix.

- Euclidean: Björklund-algoritm, 1-32 steg, rotation, swing
- Turing Machine: binärt skiftregister med mutation, skalkvantisering
- Random Gates: densitetsstyrd trigger med seed för reproducerbarhet
- Alla tempo-synkade med extern clock-input

### Tier 3 — Nischad men värdefull

#### 3.1 Cross-Modulation (fullständig) (#43)
**Prioritet: MEDEL** | Uppskattning: ~200 rader | Ändringar i befintliga filer

Möjliggör att osc 1 och osc 2 modulerar varandra simultant. Ger kaotiska, organiska ljud som inte kan skapas med enkel FM. Kräver att oscillatorernas process-ordning hanteras korrekt (one-sample delay feedback).

- Bilateral FM-routing mellan Osc 1 och Osc 2
- Cross-mod amount per oscillator
- One-sample delay feedback för stabilitet

#### 3.2 Microtonal Tuning / Scala (#45)
**Prioritet: MEDEL** | Uppskattning: ~300 rader | Nya filer: 1

Stöd för .scl/.kbm-filer (Scala) ger icke-västerländska skalor och experimentella tunings. Implementeras som global tuning-tabell som alla oscillatorer refererar.

- Parser för .scl (skala) och .kbm (keyboard mapping)
- Global tuning-tabell: MIDI note -> frekvens
- Inbyggda presets: just intonation, pythagorean, 19-TET, 31-TET, arabisk
- Per-instrument tuning-val

#### 3.3 Convolution Reverb / Cabinet Sim (#22)
**Prioritet: MEDEL** | Uppskattning: ~500 rader | Nya filer: 2

Imponerar verkliga rum/kabinett via impulssvar. Kräver FFT-implementation (partitioned convolution för realtid). Stor effekt för realism, men hög komplexitet.

- Partitioned convolution (uniform block size = audio buffer size)
- FFT via `realfft` crate (ren Rust, ingen extern dependency)
- Inbyggda korta IR:er (room, plate, spring, cabinet)
- Laddning av externa .wav IR-filer
- Dry/wet, pre-delay, decay trim

#### 3.4 Phase Vocoder (#23)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~500 rader | Nya filer: 2

Pitch-shifting oberoende av tid. Kräver FFT (kan dela implementation med convolution). STFT med overlap-add, fasinterpolation och pitch-scaling.

- STFT-analys/resyntes med overlap-add
- Pitch shift: -24 till +24 semitoner
- Time stretch (om applicerbart)
- Spectral freeze/smearing som kreativ effekt

#### 3.5 Brickwall Limiter med Look-ahead (#28 + #55)
**Prioritet: MEDEL** | Uppskattning: ~200 rader | Nya filer: 1

Kombinerar brickwall limiting med look-ahead (analyserar signal i förväg). Ger clean limitering utan klick/distortion. Viktigt för master-kedjan.

- Look-ahead buffer (1-5ms)
- True peak detection
- Attack: instant, release: konfigurerbar
- Ceiling-kontroll
- Gain reduction meter

#### 3.6 Sidechaining (#33)
**Prioritet: MEDEL** | Uppskattning: ~200 rader | Ändringar i befintliga filer

Låter en signals amplitud styra en annan signals volym. Klassisk kick-vs-bas ducking, men också kreativ pumping. Kan implementeras som sidechain-input på Compressor.

- Sidechain-input (extern key) på befintlig Compressor
- Sidechain-filter (HPF/LPF på detector-signalen)
- Intern sidechain-routing mellan instrument

### Tier 4 — Experimentell / specialiserad

#### 4.1 Vector Synthesis (#9)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~400 rader | Nya filer: 1

Dynamisk mixning mellan 4 ljudkällor i ett 2D-plan (joystick X/Y). Kräver 4 oscillatorer och en 2D-mixer med modulerbara X/Y-koordinater.

- 4-source mixer med X/Y-kontroll
- Sources: valfri kombination av oscillatorer
- X/Y modulerbart via Mod Matrix (LFO, envelope, etc.)
- Sekvensbar X/Y-path

#### 4.2 LA (Linear Arithmetic) (#12)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~300 rader | Ändringar i befintliga filer

PCM-attack + syntetisk sustain. Kombinerar SamplePlayer (redan finns) med oscillatorer via en crossfade-envelope. Ger realistiska attacker med syntetiska sustains.

- Attack-fas: SamplePlayer med one-shot triggering
- Crossfade-envelope: morphar från sample till oscillator
- Crossfade-tid: 10ms-500ms
- Velocity-kontroll av crossfade-punkt

#### 4.3 FFM (FM-kedjor) (#58)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~500 rader | Nya filer: 1

Fullständig FM-engine med multipla operatörer (4-6 stycken) och fritt konfigurerbara algoritm-grafer (som DX7). Stor implementation men ger enorm syntes-kraft.

- 4-6 operatörer med frekvens-ratio, level, feedback
- 8-16 förkonfigurerade algoritm-grafer (som DX7:s 32 algoritmer)
- Per-operatör envelope
- Kan implementeras som ny modultyp eller utökning av Math Oscillator

#### 4.4 Stochastic Syntes (fullständig) (#14)
**Prioritet: LÅG** | Uppskattning: ~300 rader | Nya filer: 1

Sannolikhetsbaserad syntes i Xenakis-anda. Genererar vågformer baserade på statistiska fördelningar (Gaussian, Cauchy, exponentiell). Experimentellt men unikt.

- Sannolikhetsfördelningar: Gaussian, Cauchy, Poisson, Beta
- Density-kontroll (antal events per sekund)
- Registerkontroll (frekvensbegränsning)
- Kan kombineras med granulär engine

#### 4.5 Cellular Automata (#13)
**Prioritet: LÅG** | Uppskattning: ~250 rader | Nya filer: 1

Genererar mönster via regler som Game of Life eller Wolfram's elementära automater. Används som modulationskälla eller direkt som ljudgenerator.

- 1D automater (Rule 30, 90, 110, etc.) med konfigurerbar regel
- Grid-storlek: 8-64 celler
- Clock-input för steg-avancering
- Output: gate-mönster + CV (från cellernas state)

#### 4.6 Vosim (#17)
**Prioritet: LÅG** | Uppskattning: ~200 rader | Kan läggas till i Math Oscillator

Pulsbaserad vokalsyntes. Serier av sin^2-pulser med avtagande amplitud per grundperiod. Ger vokalliknande ljud med enkel implementation.

- Kan läggas till som ny algoritm i Math Oscillator (#19)
- Parametrar: antal pulser per period, decay-rate, formant-frekvens
- Ger vokaler (a, e, i, o, u) beroende på formant-frekvens

#### 4.7 Wave Terrain (#19)
**Prioritet: LÅG** | Uppskattning: ~300 rader | Nya filer: 1

En bana (orbit) rör sig över en 3D-funktionsyta. Orbits x/y-koordinater moduleras (t.ex. av LFO:er) och höjdvärdet = output. Experimentellt och visuellt intressant.

- Terrain-funktioner: sin(x)*cos(y), distance, ripple, noise
- Orbit: cirkulär, lissajous, kaotisk
- X/Y-hastighet och radie modulerbart
- Visuell 3D-vy (nice-to-have)

#### 4.8 Scanned Synthesis (#18)
**Prioritet: LÅG** | Uppskattning: ~400 rader | Nya filer: 1

Animerar ett haptiskt system (massa-fjäder-nätverk) i slow motion och skannar resultatet som en vågform. Ger unika, levande timbres som inte kan skapas med andra metoder.

- 1D massa-fjäder-nätverk (32-128 noder)
- Scanning-frekvens = pitch
- Excitation via hammer/impulse vid note-on
- Damping, stiffness, mass-parametrar
- Långsam animation-hastighet som förändrar timbren

#### 4.9 Frequency Shifting (#15)
**Prioritet: LÅG** | Uppskattning: ~300 rader | Nya filer: 1

Förskjuter alla deltoner linjärt (inte multiplikativt som pitch shift). Skapar inharmoniska, metalliska ljud. Kräver Hilbert Transform (#30).

- Hilbert Transform via all-pass nätverk (Hartley-metod, ~20 all-pass filters)
- Single sideband modulation (SSB): shift up eller down
- Shift range: -5000 Hz till +5000 Hz
- Feedback för resonanta effekter
- Förutsättning: implementeras samtidigt som Hilbert Transform

#### 4.10 Adaptive Filtering (#37)
**Prioritet: MYCKET LÅG** | Uppskattning: ~300 rader | Nya filer: 1

Filter som anpassar sig automatiskt (LMS/NLMS-algoritm). Används för ekoborttagning eller noise cancellation. Nischad användning i en synth, men kan vara intressant som kreativ effekt.

- LMS (Least Mean Squares) adaptiv algoritm
- Reference input + signal input
- Convergence rate parameter
- Kreativ användning: subtraherar en signal från en annan

#### 4.11 Auto-correlation / Pitch Tracking (#34)
**Prioritet: LÅG** | Uppskattning: ~250 rader | Nya filer: 1

Detekterar grundfrekvensen i en signal. Användbart för auto-tune, pitch-till-CV, och intelligent effektbearbetning.

- YIN-algoritm eller normalized auto-correlation
- Output: detekterad frekvens som CV
- Confidence-output (hur säker detektionen är)
- Kan driva oscillator-pitch eller filter-cutoff

#### 4.12 Spectral Subtraction (#35)
**Prioritet: MYCKET LÅG** | Uppskattning: ~200 rader | Kräver FFT

Brusborttagning genom att analysera och subtrahera brusspektrum. Kräver FFT-implementation. Nischad i synth-kontext men användbar för sampling.

- Brus-profil-capture (analysera tyst passage)
- Subtrahera brusprofil i frekvensdomänen
- Over-subtraction parameter (aggressivitet)
- Kräver: FFT-implementation (#21)

#### 4.13 Wavelet Transform (#36)
**Prioritet: MYCKET LÅG** | Uppskattning: ~400 rader | Nya filer: 1

Multiskalanalys av ljud. Ger tids-frekvens-representation med varierande upplösning. Akademiskt intressant men begränsat kreativt värde jämfört med FFT.

- Continuous Wavelet Transform (CWT) med Morlet-wavelet
- Multiband-decomposition
- Per-band processing (gain, pan)
- Kreativ effekt: time-stretch per frekvensskala

#### 4.14 Polyphonic Aftertouch (fullständig) (#56)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~150 rader | Ändringar i befintliga filer

Fullständigt stöd för polyfon aftertouch (individuellt tryck per tangent). Kräver att MIDI-hanteringen skickar per-note pressure till rätt voice.

- MIDI: Polyphonic Key Pressure (0xA0) parsing
- Per-voice aftertouch-värde (istället för globalt)
- Ny ModSource: `PolyAftertouch` i Mod Matrix

#### 4.15 Physical Control Mapping (#60)
**Prioritet: LÅG** | Uppskattning: ~250 rader | Nya filer: 1

Mappar fysiska lagar (gravitation, friktion, tröghet) till parameterförändringar. En kontrollsignal "faller" mot ett mål med realistisk fysik istället för att hoppa direkt.

- Mass-spring-damper modell för parameter-smoothing
- Gravity, friction, bounce-parametrar
- Kan appliceras på valfri ModMatrix-destination
- Ger naturligare, mer expressiva parameterövergångar

#### 4.16 Round Robin Sampling (#57)
**Prioritet: LÅG** | Uppskattning: ~200 rader | Ändringar i SamplePlayer

Växlar mellan olika samplingar vid upprepad triggering för att undvika "maskingevärseffekt". Utökar befintlig SamplePlayer.

- Multipla samples per note/velocity-lager
- Round-robin cykling (1->2->3->1->...)
- Random selection-mode
- Velocity layers (2-4 lager)

#### 4.17 Resampled Interpolation Errors (fullständig) (#59)
**Prioritet: LÅG** | Uppskattning: ~150 rader | Nya filer: 0

Medveten aliasing genom att avsiktligt använda dålig interpolation (nearest-neighbor) eller omsampling vid felaktig rate. Lo-fi estetik.

- Downsampling-faktor (2x-64x) utan anti-aliasing filter
- Nearest-neighbor interpolation (sample-and-hold)
- Bit depth reduction med dithering-val (none, triangular, noise-shaped)
- Kan läggas till som ny kurva i befintlig Waveshaper eller som del av Bitcrush

#### 4.18 Feedback Loops (generell) (#51)
**Prioritet: MEDEL-LÅG** | Uppskattning: ~300 rader | Ändringar i engine

Generell feedback-routing i modulgrafen. Tillåter att en moduls output kopplas tillbaka till sin egen input (eller en tidigare moduls input). Kräver one-sample delay i feedback-pathen.

- Feedback-port på moduler (explicit markering)
- One-sample delay buffer per feedback-path
- Feedback amount-kontroll med safety limiter
- Kan skapa self-generating soundscapes

---

## Prioriterad implementeringsordning

Baserad på effekt, komplexitet och synergier. Grupperat i implementeringsvågor.

### Vag 1: Grundläggande luckor (Nästa)

Fyller de mest uppenbara luckorna med hög effekt-per-arbetsinsats.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 1 | **Slew Rate / Glide** (#47) | ~120 | Grundfunktion som saknas, enkel implementation, varje synth behöver detta |
| 2 | **Ring Modulation** (#24) | ~150 | Trivial implementation (signal * signal), stor kreativ vinst |
| 3 | **Envelope Follower** (#39) | ~200 | Ny ModSource, dynamisk interaktion mellan signaler |
| 4 | **Wavetable Syntes** (#6) | ~600 | Mest efterfrågade saknade syntestypen, scanbar position med Mod Matrix |

**Total: ~1 070 rader**

### Vag 2: Kreativ expansion

Nya verktyg för evolverande och komplexa ljud.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 5 | **MSEG** (befintlig plan) | ~830 | Evolverande envelopes, looping, kreativ kraft |
| 6 | **BBD Emulation** (#53) | ~300 | Analog delay-karaktär, stor skillnad i hur delays låter |
| 7 | **Cross-Modulation** (#43) | ~200 | Kaotiska, organiska ljud från osc-interaktion |
| 8 | **Mid-Side Processing** (#40) | ~150 | Stereo-kontroll på EQ och Compressor |

**Total: ~1 480 rader**

### Vag 3: Avancerad bearbetning

Spektral bearbetning och modern reverb.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 9 | **FDN Reverb** (#26) | ~400 | Uppgradering av Schroeder till modernt reverb |
| 10 | **Brickwall Limiter + Look-ahead** (#28+#55) | ~200 | Professionell master-kedja |
| 11 | **Additive Syntes** (#1) | ~400 | Fullständig harmonisk kontroll, organ/klocka/experimentellt |
| 12 | **Sidechaining** (#33) | ~200 | Dynamisk interaktion mellan instrument |

**Total: ~1 200 rader**

### Vag 4: Generativt och algoritmiskt

Självgenererande musik och avancerade sekvenser.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 13 | **Generativa moduler** (Euclidean, Turing, Probability Gates) (#44) | ~900 | Algoritmiska rytmer och melodier |
| 14 | **Microtonal Tuning** (#45) | ~300 | Icke-västerländska skalor, experimentell musik |
| 15 | **Polyphonic Aftertouch** (#56) | ~150 | Expressivt spel, MPE-förberedelse |

**Total: ~1 350 rader**

### Vag 5: Granulär och spektral

FFT-baserade verktyg och granulär bearbetning.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 16 | **FFT-infrastruktur** (#21) | ~300 | Grundsten för convolution, phase vocoder, spectral |
| 17 | **Granular Syntes** (#7) | ~600 | Helt unik ljudpalett, cloud-liknande texturer |
| 18 | **Convolution Reverb** (#22) | ~500 | Realistisk rumsljud, kabinett-simulering (kräver FFT) |
| 19 | **Phase Vocoder** (#23) | ~500 | Pitch-shift, time-stretch, spectral freeze (kräver FFT) |

**Total: ~1 900 rader**

### Vag 6: Avancerad syntes

Nischade men kraftfulla syntesmetoder.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 20 | **FFM (FM-kedjor)** (#58) | ~500 | DX7-stil multi-operatör FM |
| 21 | **Vector Synthesis** (#9) | ~400 | 4-source 2D-mixning |
| 22 | **LA (Linear Arithmetic)** (#12) | ~300 | Sample-attack + synth-sustain |
| 23 | **Vosim** (#17) | ~200 | Vokalsyntes som Math Osc-algoritm |
| 24 | **Stochastic (fullständig)** (#14) | ~300 | Xenakis-inspirerad experimentell syntes |

**Total: ~1 700 rader**

### Vag 7: Experimentellt och nischat

Unika, experimentella tekniker.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 25 | **Feedback Loops (generell)** (#51) | ~300 | Self-generating soundscapes |
| 26 | **Frequency Shifting + Hilbert** (#15+#30) | ~300 | Metalliska, inharmoniska effekter |
| 27 | **Wave Terrain** (#19) | ~300 | 3D-funktionsyta som ljudkälla |
| 28 | **Scanned Synthesis** (#18) | ~400 | Massa-fjäder-nätverk som vågform |
| 29 | **Cellular Automata** (#13) | ~250 | Game of Life som modulationskälla |
| 30 | **Physical Control Mapping** (#60) | ~250 | Fysik-baserade parameterövergångar |

**Total: ~1 800 rader**

### Vag 8: Lo-fi och specialeffekter

Resterande tekniker med lägre prioritet.

| Prio | Teknik | Uppskattade rader | Motivering |
|:---:|--------|:---:|------------|
| 31 | **Resampled Interpolation Errors** (#59) | ~150 | Lo-fi aliasing-effekt |
| 32 | **Round Robin Sampling** (#57) | ~200 | Naturligare sample-playback |
| 33 | **Auto-correlation / Pitch Tracking** (#34) | ~250 | Pitch-till-CV |
| 34 | **Spectral Subtraction** (#35) | ~200 | Brusborttagning (kräver FFT) |
| 35 | **Adaptive Filtering** (#37) | ~300 | Eko/brusborttagning |
| 36 | **Wavelet Transform** (#36) | ~400 | Multiskalanalys |

**Total: ~1 500 rader**

### Sammanfattning per vag

| Vag | Tema | Tekniker | Rader | Kumulativt |
|:---:|------|:---:|:---:|:---:|
| 1 | Grundläggande luckor | 4 | ~1 070 | ~1 070 |
| 2 | Kreativ expansion | 4 | ~1 480 | ~2 550 |
| 3 | Avancerad bearbetning | 4 | ~1 200 | ~3 750 |
| 4 | Generativt | 3 | ~1 350 | ~5 100 |
| 5 | Granulär & spektral | 4 | ~1 900 | ~7 000 |
| 6 | Avancerad syntes | 5 | ~1 700 | ~8 700 |
| 7 | Experimentellt | 6 | ~1 800 | ~10 500 |
| 8 | Lo-fi & special | 6 | ~1 500 | ~12 000 |
| **Totalt** | | **36** | **~12 000** | |

*Not: 23 tekniker är redan implementerade (helt eller delvis) och ingår inte i estimaten.*

---

## Genomförda features (historik)

### Redan implementerade tekniker (23 st helt, 10 st delvis)

| Teknik | Implementation | Version |
|--------|---------------|---------|
| Subtractive (#2) | Oscillator -> Filter (SVF, 7 modes, 4 modeller) | Grundversion |
| FM (#3) | SineFM + FeedbackFM i Math Oscillator | Grundversion |
| PM (#4) | PM-input på huvudoscillatorn | Grundversion |
| Karplus-Strong (#5) | Math Oscillator-algoritm | Grundversion |
| Phase Distortion (#10) | PhaseDist i Math Oscillator | Grundversion |
| Formant (#11) | Formant i Math Oscillator | Grundversion |
| Waveshaping (#16) | WaveFolder + Waveshaper-effekt (6 kurvor) | 0.113.0 |
| Brownian Noise (#20) | Brown noise i Noise-modul | Grundversion |
| All-pass Filters (#25) | Phaser + Schroeder reverb | Grundversion |
| Compression (#27) | Compressor-effekt | Grundversion |
| Bitcrushing (#29) | Distortion + Waveshaper Quantize | 0.113.0 |
| Chorus (#31) | Chorus-effekt | Grundversion |
| Flanging (#32) | Flanger-effekt | Grundversion |
| Soft Clipping (#38) | SoftClip (tanh) + Tube i Distortion | Grundversion |
| Chaos Generators (#41) | Lorenz i Math Oscillator | Grundversion |
| Audio Rate Modulation (#42) | FM/PM audio rate inputs | Grundversion |
| Self-Oscillating Filters (#46) | Screamer + Acid vid hög resonans | 0.115.0 |
| Logic Operators (#48) | BitWise i Math Oscillator | Grundversion |
| Jitter & Drift (#49) | Phase randomization i unison | 0.114.0 |
| Sample & Hold (#50) | S&H-vågform i LFO | Grundversion |
| Morphing SVF (#52) | Fluid-filtrets morph-parameter | 0.115.0 |
| Wavefolding (#54) | WaveFolder + Foldback + Fold/SineFold | 0.113.0 |

### Genomförda features från tidigare plan

| Version | Feature | Rader |
|---------|---------|:---:|
| 0.107.0-0.112.0 | Mod Matrix (8 slots, 10 källor, 11 destinationer) | ~700 |
| 0.113.0 | Waveshaper-effekt (6 kurvor) | ~250 |
| 0.114.0 | Intra-voice Unison (1-7 röster, detune, stereo) | ~400 |
| 0.115.0 | Character Filters (Fluid, Screamer, Acid) | ~410 |
