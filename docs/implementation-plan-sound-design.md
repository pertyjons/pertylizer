# Implementeringsplan: 60 tekniker och algoritmer för ljudmotorn

> Status: AKTIV | Datum: 2026-02-14 | Basversion: 0.121.0

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
| Del 1: Ljudgenerering | 16 | 2 | 2 | 20 |
| Del 2: Effekter | 16 | 0 | 4 | 20 |
| Del 3: Hemliga ingredienser | 17 | 1 | 2 | 20 |
| **Totalt** | **49** | **3** | **8** | **60** |

**82% fullt implementerade, 5% delvis, 13% saknas**

---

## Del 1: Ljudgenerering (Syntes) — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 1 | **Additive** | KLAR | AdditiveOsc med 32 harmoniska, spektral profil (Tilt/OddEven/Brightness), stretch, fas-randomisering |
| 2 | **Subtractive** | KLAR | Oscillator -> Filter med SVF, 7 filtertyper, 4 karaktärsmodeller (Standard/Fluid/Screamer/Acid) |
| 3 | **FM (Frequency Modulation)** | KLAR | SineFM + FeedbackFM i Math Oscillator + FM-ingång på huvudoscillatorn |
| 4 | **PM (Phase Modulation)** | KLAR | PM-modulationsingång på huvudoscillatorn |
| 5 | **Karplus-Strong** | KLAR | KarplusStrong-algoritm i Math Oscillator |
| 6 | **Wavetable** | KLAR | WavetableOsc med 6 inbyggda banker (Basic/Harmonics/PWM/Formant/Digital/Warm), scanbar position, FM, detune, octave |
| 7 | **Granular** | KLAR | GranularOsc med 32 grains, 5 källor (Saw/Sine/Square/Triangle/Noise), 3 fönster (Hann/Gaussian/Trapezoid), freeze, RT-säker PRNG |
| 8 | **Physical Modeling** | DELVIS | Body Resonance + Mechanical Noise, men inga fullständiga modeller (strängar, rör) |
| 9 | **Vector Synthesis** | KLAR | VectorMixer-modul med 4 audio-ingångar, XY-position, equal-power bilineär interpolation, CV-modulering |
| 10 | **Phase Distortion** | KLAR | PhaseDist-algoritm i Math Oscillator (Casio CZ-stil) |
| 11 | **Formant** | KLAR | Formant-algoritm i Math Oscillator |
| 12 | **LA (Linear Arithmetic)** | KLAR | LaSynth-modul med 4 attack-typer (click/noise/pluck/hammer), crossfade till sustain, brightness-filter |
| 13 | **Cellular Automata** | SAKNAS | Ingen cellulär automat-syntes |
| 14 | **Stochastic** | KLAR | StochasticCloud (Xenakis-stil random walk) + ChuaCircuit (kaotisk dubbel-scroll) i Math Oscillator |
| 15 | **Frequency Shifting** | KLAR | FrequencyShifter-effekt med Bode/Hilbert-transform, up/down/stereo-lägen, mix-kontroll |
| 16 | **Waveshaping** | KLAR | WaveFolder i Math Oscillator + Waveshaper-effekt med 6 kurvor |
| 17 | **Vosim** | SAKNAS | Ingen pulsbaserad vokalsyntes |
| 18 | **Scanned Synthesis** | DELVIS | Karplus-Strong kan ses som enkel fysisk modell, men ingen fullständig scanned synthesis |
| 19 | **Wave Terrain** | SAKNAS | Ingen wave terrain-syntes |
| 20 | **Brownian Noise** | KLAR | Brown noise (-6dB/oktav) via integrator i Noise-modulen |

---

## Del 2: Ljudmanipulering (Effekter) — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 21 | **FFT** | KLAR | FftProcessor, StftProcessor, PartitionedConvolver, WindowType (Hann/Hamming/BlackmanHarris) i synth_dsp::spectral |
| 22 | **Convolution** | KLAR | Convolver med partitioned FFT-convolution, 4 genererade IRs (Plate/Room/Spring/Hall), pre-delay, decay trim, brightness |
| 23 | **Phase Vocoder** | KLAR | PhaseVocoder med STFT pitch shifting (-24 till +24 st), spectral freeze, konfigurerbar FFT-storlek (512-4096) |
| 24 | **Ring Modulation** | KLAR | RingMod-modul med intern carrier (5 vågformer), keyboard tracking, freq ratio, dry/wet mix |
| 25 | **All-pass Filters** | KLAR | Phaser (kaskadade all-pass) + Schroeder-reverb (2 serie-allpass) |
| 26 | **FDN (Feedback Delay Network)** | KLAR | 8-kanals FDN med Hadamard-matris, modulerade delays, damping, low-cut, pre-delay, decay, diffusion |
| 27 | **Compression** | KLAR | Compressor med threshold, ratio, attack, release, makeup gain |
| 28 | **Brickwall Limiting** | KLAR | Limiter med look-ahead buffer, true peak detection, konfigurerbart ceiling/release |
| 29 | **Bitcrushing** | KLAR | Bitcrush i Distortion + Quantize i Waveshaper |
| 30 | **Hilbert Transform** | SAKNAS | Ingen Hilbert-transform |
| 31 | **Chorus** | KLAR | Chorus-effekt med LFO-modulerade delays och stereobreddning |
| 32 | **Flanging** | KLAR | Flanger-effekt med modulerad delay och feedback |
| 33 | **Sidechaining** | KLAR | Sidechain-input på Compressor med HPF-filter (20-500 Hz), extern detektionskälla |
| 34 | **Auto-correlation** | KLAR | PitchTracker-modul med autokorrelation, 1V/oct CV-output, gate, ringbuffer 2048 samples |
| 35 | **Spectral Subtraction** | SAKNAS | Ingen spektral brusborttagning |
| 36 | **Wavelet Transform** | SAKNAS | Ingen wavelet-analys |
| 37 | **Adaptive Filtering** | SAKNAS | Inga adaptiva filter |
| 38 | **Soft Clipping** | KLAR | SoftClip (tanh) i Distortion + Waveshaper, Tube-distortion |
| 39 | **Envelope Following** | KLAR | EnvelopeFollower-modul med attack/release/sensitivity, one-pole tracking |
| 40 | **Mid-Side Processing** | KLAR | MidSide-effekt med width (0-2x), mid/side gain, M/S encoding/decoding |

---

## Del 3: Hemliga ingredienser — 20 tekniker

| # | Teknik | Status | Detaljer |
|---|--------|--------|----------|
| 41 | **Chaos Generators (Lorenz)** | KLAR | Lorenz-attraktor i Math Oscillator |
| 42 | **Audio Rate Modulation** | KLAR | FM/PM-ingångar på oscillatorer körs i audio rate |
| 43 | **Cross-Modulation** | KLAR | cross_mod audio-ingång på oscillatorer med CrossModAmount-parameter, kombineras med FM |
| 44 | **Probability Gates** | KLAR | RandomGates med density, burst mode, gate length + Euclidean Sequencer + Turing Machine |
| 45 | **Microtonal Tuning** | KLAR | TuningTable med 5 presets (12-TET, JI, Pythagorean, 19-TET, 31-TET), Scala-parser (.scl/.kbm), per-voice tuning |
| 46 | **Self-Oscillating Filters** | KLAR | Screamer + Acid vid hög resonans |
| 47 | **Slew Rate Limiting** | KLAR | Glide/portamento implementerat i voice.rs med GlideState + GUI-slider |
| 48 | **Logic Operators** | KLAR | BitWise-algoritm i Math Oscillator |
| 49 | **Jitter & Drift** | KLAR | Phase randomization i unison + Lorenz/Logistic som modulationskällor |
| 50 | **Sample & Hold** | KLAR | S&H-vågform i LFO |
| 51 | **Feedback Loops** | DELVIS | FeedbackFM, feedback i delay/reverb, men ingen generell feedback-routing |
| 52 | **Morphing SVF** | KLAR | Fluid-filtret har morph LP <-> BP <-> HP <-> Notch |
| 53 | **BBD Emulation** | KLAR | BbdDelay med kompander, bandbreddsbegränsning, wow & flutter, clock noise, feedback med mörkning |
| 54 | **Wavefolding** | KLAR | WaveFolder i Math Osc + Foldback i Distortion + Fold/SineFold i Waveshaper |
| 55 | **Look-ahead Processing** | KLAR | Limiter med look-ahead ring buffer (1-5ms), true peak detection |
| 56 | **Polyphonic Aftertouch** | KLAR | PolyAftertouch ModSource i Mod Matrix, per-voice aftertouch-värde, MIDI 0xA0 parsing |
| 57 | **Round Robin Sampling** | SAKNAS | Ingen sampling-engine med round robin |
| 58 | **FFM (FM-kedjor)** | DELVIS | FeedbackFM finns, men inte fria FM-kedjor mellan multipla operatörer |
| 59 | **Resampled Interpolation Errors** | SAKNAS | Bitcrush/Quantize ger lo-fi, men ingen medveten aliasing via resampling |
| 60 | **Physical Control Mapping** | SAKNAS | Ingen fysik-baserad kontrollmappning |

---

## Prioriterad implementeringsordning

Prioriteringen baseras på tre faktorer:
- **Effekt** — Hur stor kreativ/musikalisk vinst ger tekniken?
- **Komplexitet** — Hur mycket arbete krävs? Bygger den på befintlig infrastruktur?
- **Synergi** — Förstärker den andra features i motorn?

### ~~Vag 1: Grundläggande luckor~~ KLAR (0.116.0)

| Prio | Teknik | Status |
|:---:|--------|--------|
| 1 | ~~**Slew Rate / Glide** (#47)~~ | KLAR (0.116.0) |
| 2 | ~~**Ring Modulation** (#24)~~ | KLAR (0.116.0) |
| 3 | ~~**Envelope Follower** (#39)~~ | KLAR (0.116.0) |
| 4 | ~~**Wavetable Syntes** (#6)~~ | KLAR (0.116.0) |

### ~~Vag 2: Kreativ expansion~~ KLAR (0.119.0)

| Prio | Teknik | Status |
|:---:|--------|--------|
| 5 | ~~**MSEG**~~ | KLAR (0.119.0) |
| 6 | ~~**BBD Emulation** (#53)~~ | KLAR (0.119.0) |
| 7 | ~~**Cross-Modulation** (#43)~~ | KLAR (0.120.0) |
| 8 | ~~**Mid-Side Processing** (#40)~~ | KLAR (0.119.0) |

### ~~Vag 3: Avancerad bearbetning~~ KLAR (0.119.0-0.120.0)

| Prio | Teknik | Status |
|:---:|--------|--------|
| 9 | ~~**FDN Reverb** (#26)~~ | KLAR (0.120.0) |
| 10 | ~~**Brickwall Limiter + Look-ahead** (#28+#55)~~ | KLAR (0.119.0) |
| 11 | ~~**Additive Syntes** (#1)~~ | KLAR (0.119.0) |
| 12 | ~~**Sidechaining** (#33)~~ | KLAR (0.120.0) |

### ~~Vag 4: Generativt och algoritmiskt~~ KLAR (0.119.0-0.121.0)

| Prio | Teknik | Status |
|:---:|--------|--------|
| 13 | ~~**Generativa moduler** (#44)~~ | KLAR (0.119.0) |
| 14 | ~~**Microtonal Tuning** (#45)~~ | KLAR (0.120.0) |
| 15 | ~~**Polyphonic Aftertouch** (#56)~~ | KLAR (0.121.0) |

### ~~Vag 5: Granulär och spektral~~ KLAR (0.121.0)

| Prio | Teknik | Status |
|:---:|--------|--------|
| 16 | ~~**FFT-infrastruktur** (#21)~~ | KLAR (0.121.0) |
| 17 | ~~**Granular Syntes** (#7)~~ | KLAR (0.121.0) |
| 18 | ~~**Convolution Reverb** (#22)~~ | KLAR (0.121.0) |
| 19 | ~~**Phase Vocoder** (#23)~~ | KLAR (0.121.0) |

### Vag 6: Avancerad syntes (NÄSTA)

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
| 34 | **Spectral Subtraction** (#35) | ~200 | Brusborttagning (kräver FFT — nu tillgängligt) |
| 35 | **Adaptive Filtering** (#37) | ~300 | Eko/brusborttagning |
| 36 | **Wavelet Transform** (#36) | ~400 | Multiskalanalys |

**Total: ~1 500 rader**

### Sammanfattning per vag

| Vag | Tema | Tekniker | Status |
|:---:|------|:---:|--------|
| 1 | Grundläggande luckor | 4 | KLAR (0.116.0) |
| 2 | Kreativ expansion | 4 | KLAR (0.119.0-0.120.0) |
| 3 | Avancerad bearbetning | 4 | KLAR (0.119.0-0.120.0) |
| 4 | Generativt | 3 | KLAR (0.119.0-0.121.0) |
| 5 | Granulär & spektral | 4 | KLAR (0.121.0) |
| 6 | Avancerad syntes | 5 | **NÄSTA** (~1 700 rader) |
| 7 | Experimentellt | 6 | Framtid (~1 800 rader) |
| 8 | Lo-fi & special | 6 | Framtid (~1 500 rader) |
| **Totalt kvar** | | **17** | **~5 000 rader** |

*43 av 60 tekniker är fullt implementerade. 5 delvis. 12 kvar.*

---

## Genomförda features (historik)

### Redan implementerade tekniker (43 st helt, 5 st delvis)

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
| FDN Reverb (#26) | 8-kanals FDN med Hadamard-matris, modulerade delays, damping, low-cut | 0.120.0 |
| Sidechaining (#33) | Sidechain-input på Compressor med HPF-filter | 0.120.0 |
| Cross-Modulation (#43) | cross_mod audio-ingång med CrossModAmount-parameter | 0.120.0 |
| Microtonal Tuning (#45) | TuningTable med 5 presets + Scala-parser | 0.120.0 |
| Polyphonic Aftertouch (#56) | PolyAftertouch ModSource, per-voice aftertouch | 0.121.0 |
| FFT (#21) | FftProcessor, StftProcessor, PartitionedConvolver i synth_dsp | 0.121.0 |
| Granular (#7) | GranularOsc med 32 grains, 5 källor, 3 fönster, freeze | 0.121.0 |
| Convolution (#22) | Convolver med partitioned FFT, 4 IRs, pre-delay, brightness | 0.121.0 |
| Phase Vocoder (#23) | PhaseVocoder med STFT pitch shift, spectral freeze | 0.121.0 |

### Genomförda features från tidigare plan

| Version | Feature | Rader |
|---------|---------|:---:|
| 0.107.0-0.112.0 | Mod Matrix (8 slots, 10 källor, 11 destinationer) | ~700 |
| 0.113.0 | Waveshaper-effekt (6 kurvor) | ~250 |
| 0.114.0 | Intra-voice Unison (1-7 röster, detune, stereo) | ~400 |
| 0.115.0 | Character Filters (Fluid, Screamer, Acid) | ~410 |
| 0.116.0 | Ring Modulation, Envelope Follower, Wavetable Syntes | ~950 |
| 0.119.0 | MSEG, BBD Delay, Additive, Limiter, Mid-Side, Generativa moduler | ~2 780 |
| 0.120.0 | Cross-Modulation, FDN Reverb, Sidechain, Microtonal Tuning | ~1 100 |
| 0.121.0 | Poly Aftertouch, FFT-infrastruktur, Granulär, Convolution, Phase Vocoder | ~2 650 |
