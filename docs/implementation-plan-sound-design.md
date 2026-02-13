# Implementeringsplan: Funktioner for bättre ljud

> Status: UTKAST | Datum: 2026-02-13 | Basversion: 0.106.0

## Innehåll

1. [Modulationsmatris (Mod Matrix)](#1-modulationsmatris-mod-matrix) — KLAR (0.107.0-0.112.0)
2. [Unison / Supervoices](#2-unison--supervoices) — KLAR (0.114.0)
3. [Waveshaper / Wavetable-position](#3-waveshaper--wavetable-position) — Waveshaper KLAR (0.113.0)
4. [MSEG / Looping Envelopes](#4-mseg--looping-envelopes)
5. [Generativ sekvensering](#5-generativ-sekvensering)
6. [Character Filters](#6-character-filters-analog-filtermodeller)
7. [Implementeringsordning](#7-implementeringsordning)

---

## 1. Modulationsmatris (Mod Matrix)

### Vad och varför

Idag kopplas modulation via kablar i modulgraf — LFO-ut till filter-cutoff_cv osv. Det fungerar men har begränsningar:

- **En källa per destination** — cutoff kan bara ta emot en CV-kabel åt gången
- **Ingen skalning utan extra modul** — behöver en VCA bara för att dämpa en LFO
- **Inget per-voice context** — velocity, note number och aftertouch finns i Voice men exponeras inte som modulationskällor till moduler

En Mod Matrix löser alla tre problemen. Den lever *inuti varje röst* och applicerar modulationer *innan* modulernas process() körs.

### Arkitektur

```
┌─────────────────────────────────────────────┐
│                  Voice                       │
│                                              │
│  ┌────────────┐                              │
│  │ Mod Matrix │ ← velocity, note, aftertouch │
│  │            │ ← LFO outputs (förra blocket)│
│  │            │ ← Envelope outputs            │
│  │  8 slots:  │                              │
│  │  src→dest  │                              │
│  │  + amount  │                              │
│  └─────┬──────┘                              │
│        │ applicerar offset på                │
│        ▼ modulparametrar                     │
│  ┌─────────────────────────┐                 │
│  │ Normal modulprocessning │                 │
│  │ (Osc → Filter → Amp)   │                 │
│  └─────────────────────────┘                 │
└─────────────────────────────────────────────┘
```

### Nya typer (synth_core)

```rust
/// En modulationskälla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModSource {
    /// LFO-utgång (index = vilken LFO, 0-baserat)
    Lfo(u8),
    /// Envelope-utgång (index = vilken envelope)
    Envelope(u8),
    /// MIDI velocity (0.0-1.0 per röst)
    Velocity,
    /// MIDI note number (0.0-1.0, C0=0, C8≈1)
    NoteNumber,
    /// Channel aftertouch (0.0-1.0)
    Aftertouch,
    /// Mod wheel CC1 (0.0-1.0)
    ModWheel,
    /// Pitch bend (-1.0 till 1.0)
    PitchBend,
}

/// En modulationsdestination — identifieras av modul + parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModDestination {
    /// Oscillator pitch i semitoner
    OscPitch(u8),
    /// Oscillator level
    OscLevel(u8),
    /// Oscillator pulse width
    OscPulseWidth(u8),
    /// Filter cutoff i semitoner relativt basvärde
    FilterCutoff(u8),
    /// Filter resonance
    FilterResonance(u8),
    /// Amplifier level
    AmpLevel(u8),
    /// Amplifier pan
    AmpPan(u8),
    /// LFO rate
    LfoRate(u8),
    /// LFO depth
    LfoDepth(u8),
}

/// Ett modulationsslot: källa → destination med skalning.
#[derive(Debug, Clone, Copy)]
pub struct ModSlot {
    pub source: ModSource,
    pub destination: ModDestination,
    /// Skalning: -1.0 (inverterad full) till +1.0 (full)
    pub amount: BipolarValue,
    /// Om slottet är aktivt
    pub enabled: bool,
}

/// Modulationsmatrisen — 8 slots, pre-allokerad.
pub struct ModMatrix {
    slots: [ModSlot; Self::MAX_SLOTS],
    active_count: u8,
    /// Cache av senaste källvärden (uppdateras varje block)
    source_values: [f32; 16],
}

impl ModMatrix {
    pub const MAX_SLOTS: usize = 8;
}
```

### Parametrar (synth_core/params)

```rust
pub enum ModMatrixParam {
    SlotSource(u8, ModSource),       // slot index, källa
    SlotDestination(u8, ModDestination), // slot index, destination
    SlotAmount(u8, BipolarValue),    // slot index, skalning
    SlotEnabled(u8, bool),           // slot index, aktiv
}
```

### Bearbetningsflöde (synth_engine)

Modmatrisen processar **före** modulernas process() i varje voice:

```rust
// I Voice::process_audio():

// 1. Samla in källvärden från föregående block
self.mod_matrix.update_source(ModSource::Velocity, self.velocity());
self.mod_matrix.update_source(ModSource::NoteNumber, self.note_normalized());
self.mod_matrix.update_source(ModSource::Aftertouch, self.aftertouch.as_f32());
self.mod_matrix.update_source(ModSource::ModWheel, self.mod_wheel.as_f32());

// LFO/Env-utgångar från förra blocket (one-block latency, acceptabelt)
for (i, lfo_id) in self.graph.lfo_ids().enumerate() {
    let val = self.graph.last_output(lfo_id);
    self.mod_matrix.update_source(ModSource::Lfo(i as u8), val);
}

// 2. Beräkna och applicera modulationer
for slot in self.mod_matrix.active_slots() {
    let mod_value = self.mod_matrix.source_value(slot.source) * slot.amount.as_f32();
    self.apply_modulation(slot.destination, mod_value);
}

// 3. Kör normal modulprocessning
self.graph.process(output, context);

// 4. Återställ modulerade parametrar till basvärden
self.restore_base_params();
```

**Viktigt — realtidssäkerhet:** `ModMatrix` allokerar ingenting. 8 slots = 8 × 12 bytes = 96 bytes per röst. Inga HashMap-uppslagningar — source/destination mappas till index via match.

### Applicering av modulation

```rust
impl Voice {
    fn apply_modulation(&mut self, dest: ModDestination, value: f32) {
        match dest {
            ModDestination::FilterCutoff(idx) => {
                // Addera semitoner till cutoff (exponentiell skala)
                // value * 48.0 ger ±48 semitoner vid fullt utslag
                let semitones = value * 48.0;
                self.graph.modulate_filter_cutoff(idx, semitones);
            }
            ModDestination::OscPitch(idx) => {
                // Addera semitoner till pitch
                let semitones = value * 24.0; // ±24 semitoner
                self.graph.modulate_osc_pitch(idx, semitones);
            }
            ModDestination::AmpLevel(idx) => {
                // Multiplicera amp level (0.0-2.0 range)
                let gain = (1.0 + value).max(0.0);
                self.graph.modulate_amp_level(idx, gain);
            }
            // ... etc
        }
    }
}
```

### Bas/modulerat värde-mönster

Varje modulerbar parameter behöver spara sitt *basvärde* (det användaren ställt in) separat från det *effektiva värdet* (bas + modulering):

```rust
// Lägg till i moduler som stöder modulation
pub struct Filter {
    cutoff_base: Hertz,       // Användaren ställer in detta
    cutoff_mod_offset: f32,   // Mod matrix skriver hit (semitoner)
    // ...
}

impl Filter {
    fn effective_cutoff(&self) -> Hertz {
        let semitones = self.cutoff_mod_offset;
        Hertz::new(self.cutoff_base.as_f32() * (semitones / 12.0).exp2())
    }

    pub fn set_mod_offset(&mut self, offset: f32) {
        self.cutoff_mod_offset = offset;
    }

    pub fn reset_mod_offset(&mut self) {
        self.cutoff_mod_offset = 0.0;
    }
}
```

### GUI (modular_synth)

Mod Matrix renderas som en **egen modul-panel** med en 8-rads tabell:

```
┌─ Mod Matrix ──────────────────────────┐
│ Source      │ Dest        │ Amount     │
│─────────────┼─────────────┼────────────│
│ [LFO 1  ▾] │ [Cutoff  ▾] │ ◉───── 45% │
│ [Env 2  ▾] │ [Osc Ptch▾] │ ──◉─── -20%│
│ [Vel    ▾] │ [Cutoff  ▾] │ ─────◉ 80% │
│ [  ---  ▾] │ [  ---   ▾] │ ──●── 0%   │
│ ...                                    │
└────────────────────────────────────────┘
```

### Patchserialisering

```rust
// I Patch
pub struct ModMatrixState {
    pub slots: Vec<ModSlotState>,
}

pub struct ModSlotState {
    pub source: String,      // "lfo_0", "velocity", etc.
    pub destination: String, // "filter_cutoff_0", "osc_pitch_0", etc.
    pub amount: f32,         // -1.0 till 1.0
    pub enabled: bool,
}
```

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/mod.rs` | `ModMatrixParam`, `ModSource`, `ModDestination` |
| synth_core | `params/mod_matrix.rs` | **NY** — typer och enums |
| synth_core | `module_traits.rs` | `ModuleType::ModMatrix` |
| synth_modules | `mod_matrix.rs` | **NY** — `ModMatrix` struct och logik |
| synth_engine | `voice.rs` | Integrera mod matrix i `process_audio()` |
| synth_engine | `graph.rs` | `modulate_*()` och `reset_mod_offsets()` metoder |
| synth_modules | `filter.rs` | `cutoff_mod_offset`, `effective_cutoff()` |
| synth_modules | `oscillator.rs` | `pitch_mod_offset`, `level_mod_offset` |
| synth_modules | `amplifier.rs` | `level_mod_offset`, `pan_mod_offset` |
| modular_synth | `gui/module_panel.rs` | Mod matrix UI-tabell |
| modular_synth | `patch.rs` | `ModMatrixState` serialisering |

### Uppskattad omfattning

- **synth_core:** ~150 rader (typer + params)
- **synth_modules:** ~200 rader (ModMatrix struct)
- **synth_engine:** ~150 rader (Voice-integration, Graph-modulation)
- **modular_synth:** ~200 rader (GUI + patch)
- **Totalt:** ~700 rader

---

## 2. Unison / Supervoices

### Vad och varför

`AllocationMode::Unison` finns redan i `VoiceAllocator` och fördelar detune jämnt bland rösterna. Men nuvarande implementation saknar:

- **Stereo spread** — alla röster panoreras centrerat
- **Per-voice phase randomization** — alla röster startar i fas = tunnare ljud
- **UI-kontroller** — unison detune/spread exponeras inte i GUI
- **Unison count oberoende av max polyphony** — om man vill ha 4 unison-röster + 4 polyfoni behövs 16 röster totalt. Idag är det antingen unison ELLER polyfoni.

### Föreslagen lösning: Intra-voice unison

Istället för att använda fler *röster* för unison, lägger vi till **unison-oscillatorer inuti varje röst**. Detta ger unison + polyfoni utan att multiplicera röstkostnaden.

```
┌─── Voice (en per tangent) ──────────────────┐
│                                              │
│  Oscillator (med inbyggd unison)             │
│  ┌──────────────────────────────────┐        │
│  │ Sub-osc 1: detune -15ct, pan -0.7│        │
│  │ Sub-osc 2: detune  -5ct, pan -0.3│        │
│  │ Sub-osc 3: detune   0ct, pan  0.0│ ← center│
│  │ Sub-osc 4: detune  +5ct, pan +0.3│        │
│  │ Sub-osc 5: detune +15ct, pan +0.7│        │
│  └──────────────────────────────────┘        │
│        ↓ summerat stereo                     │
│  Filter → Amp → Output                      │
│                                              │
└──────────────────────────────────────────────┘
```

### Nya typer

```rust
/// Unison-konfiguration per oscillator.
#[derive(Debug, Clone, Copy)]
pub struct UnisonConfig {
    /// Antal unison-röster (1 = ingen unison, max 7)
    pub voice_count: u8,
    /// Total detune-spridning i cent
    pub detune: Cents,
    /// Stereo spread (0.0 = mono, 1.0 = full)
    pub spread: NormalizedValue,
    /// Fasrandomisering vid note-on
    pub phase_random: NormalizedValue,
}

impl Default for UnisonConfig {
    fn default() -> Self {
        Self {
            voice_count: 1,
            detune: Cents::new(10.0),
            spread: NormalizedValue::new(0.5),
            phase_random: NormalizedValue::new(1.0),
        }
    }
}
```

### Ändringar i Oscillator

```rust
pub struct Oscillator {
    // ... befintliga fält ...

    // Unison
    unison: UnisonConfig,
    /// Fas-state per unison-röst (max 7)
    unison_phases: [Phase; 7],
    /// Detune-offset per röst (beräknas vid config-ändring)
    unison_detunes: [f32; 7],  // frekvens-multiplikator
    /// Pan per röst (-1.0 till 1.0)
    unison_pans: [f32; 7],
    /// Randomiserade startfaser
    unison_phase_offsets: [Phase; 7],
}

impl Oscillator {
    fn recalculate_unison_spread(&mut self) {
        let n = self.unison.voice_count as usize;
        let total_detune = self.unison.detune.as_f32();
        let spread = self.unison.spread.as_f32();

        for i in 0..n {
            // Jämn spridning: -total/2 till +total/2
            let t = if n == 1 { 0.0 } else { (i as f32 / (n - 1) as f32) * 2.0 - 1.0 };

            // Detune: cent → frekvens-multiplikator
            let cents = t * total_detune * 0.5;
            self.unison_detunes[i] = (cents / 1200.0).exp2();

            // Pan: jämn fördelning
            self.unison_pans[i] = t * spread;
        }
    }

    fn generate_sample_unison(&mut self, freq: Hertz, /* ... */) -> (f32, f32) {
        let n = self.unison.voice_count as usize;
        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        let gain = 1.0 / (n as f32).sqrt(); // Constant-power normalisering

        for i in 0..n {
            let voice_freq = Hertz::new(freq.as_f32() * self.unison_detunes[i]);
            let sample = self.generate_single_sample(
                voice_freq,
                &mut self.unison_phases[i],
            );

            // Constant-power panning
            let pan = self.unison_pans[i];
            let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
            left += sample * angle.cos() * gain;
            right += sample * angle.sin() * gain;
        }

        (left, right)
    }
}
```

### Nya parametrar

```rust
pub enum OscillatorParam {
    // ... befintliga ...
    UnisonVoices(u8),                    // 1-7
    UnisonDetune(Cents),                 // 0-100 cent
    UnisonSpread(NormalizedValue),        // 0.0-1.0
    UnisonPhaseRandom(NormalizedValue),   // 0.0-1.0
}
```

### Stereo-output från Oscillator

Idag har oscillatorn en mono-utgång ("out"). Med unison behövs stereo:

```rust
// Nya output-portar
.port(PortDescriptor::audio_output("out_l", "Out L"))
.port(PortDescriptor::audio_output("out_r", "Out R"))
.port(PortDescriptor::audio_output("out", "Out"))  // Behåll mono (summa)
```

Filtret behöver sedan hantera stereo-input, eller så summeras unison till mono innan filter och stereo-spread appliceras efter amp (enklare).

**Enklaste approach:** Behåll mono signalkedja, applicera stereo-spread i `StereoOutput`-modulen:

```rust
// I StereoOutput: ta emot unison_spread-data via port
// Oscillatorn skickar pan-info som sideband
// StereoOutput applicerar stereo-imaging på det redan mixade mono-signalet
```

**Alternativ (rekommenderad):** Gör Oscillator stereo med `out_l`/`out_r`, och låt Filter + Amp hantera stereo internt. Mer jobb men bättre resultat.

### Note-on: fasrandomisering

```rust
impl Oscillator {
    fn note_on(&mut self, note: MidiNote, velocity: Velocity) {
        let random = self.unison.phase_random.as_f32();
        for i in 0..self.unison.voice_count as usize {
            if random > 0.0 {
                self.unison_phases[i] = Phase::new(fastrand::f32() * random);
            } else {
                self.unison_phases[i] = Phase::ZERO;
            }
        }
    }
}
```

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/oscillators.rs` | Unison-parametrar |
| synth_modules | `oscillator.rs` | `UnisonConfig`, unison-generering, stereo output |
| synth_modules | `math_oscillator.rs` | Samma unison-stöd (valfritt, kan vänta) |
| synth_engine | `voice.rs` | Ev. stereo-routing genom grafen |
| modular_synth | `gui/module_panel.rs` | Unison-kontroller i oscillator-panel |
| modular_synth | `patch.rs` | Serialisering av unison-parametrar |

### Uppskattad omfattning

- ~400 rader totalt (mestadels i oscillator.rs)

---

## 3. Waveshaper / Wavetable-position

### Vad och varför

MathOscillator har redan 18 algoritmer med `var_a`/`var_b`/`var_c`-kontroller, men de är diskreta algoritmer — man kan inte *morphas* smidigt mellan dem. En wavetable-oscillator med scanbar position ger:

- **Timbral rörelse** — koppla LFO/Envelope till position för ljud som utvecklas
- **Preset-vänligt** — ladda wavetables som definierar hela timbral paletten
- **Kompatibelt med mod matrix** — position = modulerbar destination

### Tvådelad implementation

**Del A: Waveshaper-modul** — en fristående modul som omformar en signal.
**Del B: Wavetable-oscillator** — en ny oscillatortyp med scanbar position.

### Del A: Waveshaper-modul

En modul som tar en signal in och formar om den med en valbar kurva:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveshaperCurve {
    /// tanh(x * drive) — mjuk kompression
    SoftClip,
    /// Asymmetrisk: positive/negative har olika kurvor
    Asymmetric,
    /// Viker tillbaka signalen vid ±threshold
    Fold,
    /// Chebyshev-polynom (harmonisk kontroll)
    Chebyshev,
    /// Sin(x * drive) — FM-liknande
    SineFold,
    /// Kvantisera till N nivåer
    Quantize,
}

pub struct Waveshaper {
    curve: WaveshaperCurve,
    drive: NormalizedValue,      // 0.0-1.0 → 1x-20x
    mix: NormalizedValue,        // Dry/wet
    bias: BipolarValue,          // DC offset före shaping
    symmetry: BipolarValue,      // Asymmetrisk distortion
    output_buffer: AudioBuffer,
}

impl PolyModule for Waveshaper {
    fn process(&mut self, inputs: InputPorts<'_>, outputs: &mut HashMap<String, AudioBuffer>, context: &ProcessContext) {
        let input = inputs.get(PortName::IN);
        let drive_cv = inputs.get(PortName::intern("drive_cv"));

        let samples = context.samples.as_usize();
        for i in 0..samples {
            let x = input.map_or(0.0, |b| b[i]);
            let drive = self.effective_drive(drive_cv.map(|b| b[i]));
            let biased = x + self.bias.as_f32();

            let shaped = match self.curve {
                WaveshaperCurve::SoftClip => (biased * drive).tanh(),
                WaveshaperCurve::Fold => {
                    // Wavefolder: viker signalen vid ±1.0
                    let folded = biased * drive;
                    fold_signal(folded)  // Triangelvåg-liknande folding
                }
                WaveshaperCurve::Chebyshev => {
                    // T_n(x) Chebyshev polynom av ordning drive
                    chebyshev(biased, drive as u32)
                }
                WaveshaperCurve::SineFold => {
                    (biased * drive * std::f32::consts::PI).sin()
                }
                WaveshaperCurve::Quantize => {
                    let levels = (drive * 255.0).max(2.0);
                    (biased * levels).round() / levels
                }
                WaveshaperCurve::Asymmetric => {
                    let sym = self.symmetry.as_f32();
                    if biased >= 0.0 {
                        (biased * drive * (1.0 + sym)).tanh()
                    } else {
                        (biased * drive * (1.0 - sym)).tanh()
                    }
                }
            };

            // Dry/wet mix
            let mix = self.mix.as_f32();
            self.output_buffer[i] = x * (1.0 - mix) + shaped * mix;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }
}
```

### Del B: Wavetable-oscillator

En ny oscillator som scannar genom en tabell av single-cycle waveforms:

```rust
/// En wavetable: en sekvens av single-cycle waveforms.
/// Varje frame är 2048 samples (standard).
pub struct Wavetable {
    /// Namn för UI
    name: String,
    /// Frames (varje frame = en fullständig vågcykel)
    frames: Vec<[f32; Self::FRAME_SIZE]>,
}

impl Wavetable {
    pub const FRAME_SIZE: usize = 2048;

    /// Hämta interpolerat sample vid given fas och position.
    /// position: 0.0 = första framen, 1.0 = sista framen
    /// phase: 0.0-1.0 position inom en cykel
    #[inline]
    pub fn sample(&self, position: f32, phase: f32) -> f32 {
        let num_frames = self.frames.len();
        if num_frames == 0 { return 0.0; }

        // Interpolera mellan frames
        let frame_f = position * (num_frames - 1) as f32;
        let frame_a = (frame_f as usize).min(num_frames - 1);
        let frame_b = (frame_a + 1).min(num_frames - 1);
        let frame_frac = frame_f.fract();

        // Interpolera inom frame (cubic)
        let sample_a = self.sample_frame(frame_a, phase);
        let sample_b = self.sample_frame(frame_b, phase);

        // Crossfade mellan frames
        sample_a * (1.0 - frame_frac) + sample_b * frame_frac
    }

    #[inline]
    fn sample_frame(&self, frame_idx: usize, phase: f32) -> f32 {
        let frame = &self.frames[frame_idx];
        let pos = phase * Self::FRAME_SIZE as f32;
        let idx = pos as usize;
        let frac = pos.fract();

        // Linjär interpolation (kan uppgraderas till cubic)
        let a = frame[idx % Self::FRAME_SIZE];
        let b = frame[(idx + 1) % Self::FRAME_SIZE];
        a + (b - a) * frac
    }
}

/// Wavetable-oscillator.
pub struct WavetableOscillator {
    frequency: Hertz,
    detune: Cents,
    octave: Semitones,
    level: Gain,

    /// Vilken wavetable som används (index i global lista)
    table_index: usize,
    /// Position i wavetable (0.0-1.0) — MODULERBAR
    position: NormalizedValue,
    position_mod_offset: f32,

    phase: Phase,
    sample_rate: SampleRate,
    output_buffer: AudioBuffer,
}
```

### Inbyggda wavetables

Generera matematiskt vid uppstart — inga externa filer behövs:

```rust
pub fn generate_builtin_wavetables() -> Vec<Wavetable> {
    vec![
        // "Basic" — Sine → Triangle → Saw → Square → Pulse
        generate_basic_morph(),

        // "Harmonics" — 1 harmonisk → 2 → 4 → 8 → 16 → 32
        generate_additive_sweep(),

        // "PWM" — Puls med varierande bredd: 50% → 10%
        generate_pwm_sweep(),

        // "Formant" — Vokalliknande former (a → e → i → o → u)
        generate_formant_table(),

        // "Digital" — Matematiska funktioner (sin → abs(sin) → sign(sin) → steps)
        generate_digital_table(),

        // "Warm" — Mjuka, analoga varianter (round saw, soft square, etc.)
        generate_warm_table(),
    ]
}

fn generate_basic_morph() -> Wavetable {
    let mut frames = Vec::with_capacity(64);

    // 64 frames: morphar gradvis mellan vågformer
    for i in 0..64 {
        let t = i as f32 / 63.0;
        let mut frame = [0.0f32; Wavetable::FRAME_SIZE];

        for s in 0..Wavetable::FRAME_SIZE {
            let phase = s as f32 / Wavetable::FRAME_SIZE as f32;
            let sine = (phase * TAU).sin();
            let tri = if phase < 0.5 { 4.0 * phase - 1.0 } else { 3.0 - 4.0 * phase };
            let saw = 2.0 * phase - 1.0;
            let square = if phase < 0.5 { 1.0 } else { -1.0 };

            // Morph: sine(0) → tri(0.33) → saw(0.66) → square(1.0)
            frame[s] = if t < 0.33 {
                let mix = t / 0.33;
                sine * (1.0 - mix) + tri * mix
            } else if t < 0.66 {
                let mix = (t - 0.33) / 0.33;
                tri * (1.0 - mix) + saw * mix
            } else {
                let mix = (t - 0.66) / 0.34;
                saw * (1.0 - mix) + square * mix
            };
        }

        frames.push(frame);
    }

    Wavetable { name: "Basic".into(), frames }
}
```

### ModMatrix-integration

```rust
// Ny ModDestination
pub enum ModDestination {
    // ... befintliga ...
    WavetablePosition(u8),  // Wavetable position (0.0-1.0)
    WaveshaperDrive(u8),    // Waveshaper drive
}
```

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/mod.rs` | `ModuleType::Waveshaper`, `ModuleType::WavetableOsc` |
| synth_core | `params/waveshaper.rs` | **NY** — `WaveshaperParam` |
| synth_core | `params/wavetable.rs` | **NY** — `WavetableOscParam` |
| synth_modules | `waveshaper.rs` | **NY** — Waveshaper-modul |
| synth_modules | `wavetable_osc.rs` | **NY** — WavetableOscillator |
| synth_modules | `wavetable.rs` | **NY** — Wavetable data + inbyggda tables |
| synth_engine | `graph.rs` | Registrera nya modultyper |
| modular_synth | `gui/module_panel.rs` | UI för waveshaper + wavetable (position-slider, table-väljare) |
| modular_synth | `patch.rs` | Serialisering |
| modular_synth | `gui/patch_bridge.rs` | Load/save för nya moduler |

### Uppskattad omfattning

- **Waveshaper:** ~250 rader (modul + params + UI)
- **Wavetable-oscillator:** ~400 rader (oscillator + inbyggda tables)
- **Wavetable data:** ~200 rader (generering av inbyggda tables)
- **Totalt:** ~850 rader

---

## 4. MSEG / Looping Envelopes

### Vad och varför

Nuvarande ADSR-envelope har 4 fasta steg med kurv-kontroll per steg. Det räcker för grundljud men inte för:

- **Evolverande pads** — behöver fler steg (attack → rise → dip → sustain → ...)
- **Rytmiska effekter** — en loopande envelope som skapar tremolo/gating
- **Komplexa transienter** — dubbelanfall (ghost note + main attack), percussiva former

### Multi-Stage Envelope Generator (MSEG)

En ny modultyp som ersätter inte, utan *kompletterar* ADSR:

```rust
/// Ett segment i MSEG.
#[derive(Debug, Clone, Copy)]
pub struct MsegSegment {
    /// Tid för segmentet
    pub duration: Seconds,
    /// Målnivå vid slutet av segmentet
    pub target_level: NormalizedValue,
    /// Kurvform (-1.0 = exponentiell, 0.0 = linjär, 1.0 = logaritmisk)
    pub curve: BipolarValue,
    /// Tempo-synkad tid (istället för absolut tid)
    pub tempo_sync: Option<BeatDivision>,
}

/// MSEG-konfiguration.
pub struct Mseg {
    /// Alla segment (max 16)
    segments: ArrayVec<MsegSegment, 16>,
    /// Vilka segment som loopar (-1 = ingen loop)
    loop_start: Option<u8>,
    loop_end: Option<u8>,
    /// Sustain-punkt (väntar på note-off, sedan fortsätter)
    sustain_point: Option<u8>,
    /// Release-segment (spelas efter note-off, oavsett loop)
    release_start: Option<u8>,

    // Runtime state
    current_segment: u8,
    time_in_segment: f32,
    level: f32,
    prev_level: f32,       // Nivå vid segmentstart
    stage: MsegStage,
    sample_rate: SampleRate,

    output_buffer: AudioBuffer,
    position_buffer: Arc<MsegPositionBuffer>,  // GUI-sync (lock-free)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsegStage {
    Idle,
    Playing,
    Sustaining,  // Väntar vid sustain-punkt
    Releasing,   // Spelar release-segmenten
    Done,
}
```

### Bearbetning

```rust
impl Mseg {
    fn process_sample(&mut self, context: &ProcessContext) -> f32 {
        if self.stage == MsegStage::Idle || self.stage == MsegStage::Done {
            return 0.0;
        }

        let seg_idx = self.current_segment as usize;
        if seg_idx >= self.segments.len() {
            self.stage = MsegStage::Done;
            return 0.0;
        }

        let seg = &self.segments[seg_idx];

        // Beräkna duration i samples
        let duration_secs = seg.tempo_sync
            .map(|div| div.to_duration(context.tempo).as_f32())
            .unwrap_or(seg.duration.as_f32())
            .max(0.001);

        // Normaliserad position i segmentet (0.0-1.0)
        let t = (self.time_in_segment / duration_secs).min(1.0);

        // Applicera kurva
        let curved_t = apply_curve(t, seg.curve);

        // Interpolera nivå
        let target = seg.target_level.as_f32();
        self.level = self.prev_level + (target - self.prev_level) * curved_t;

        // Avancera tid
        self.time_in_segment += 1.0 / self.sample_rate.as_f32();

        // Kolla om segmentet är slut
        if t >= 1.0 {
            self.advance_to_next_segment();
        }

        self.level
    }

    fn advance_to_next_segment(&mut self) {
        self.prev_level = self.level;
        self.time_in_segment = 0.0;
        let next = self.current_segment + 1;

        // Sustain-punkt: pausa och vänta på note-off
        if Some(self.current_segment) == self.sustain_point
           && self.stage != MsegStage::Releasing
        {
            self.stage = MsegStage::Sustaining;
            return;
        }

        // Loop: hoppa tillbaka
        if Some(self.current_segment) == self.loop_end
           && self.stage == MsegStage::Playing
        {
            if let Some(start) = self.loop_start {
                self.current_segment = start;
                return;
            }
        }

        self.current_segment = next;

        // Klart?
        if self.current_segment as usize >= self.segments.len() {
            self.stage = MsegStage::Done;
            self.level = 0.0;
        }
    }

    fn note_off(&mut self) {
        if let Some(release_start) = self.release_start {
            self.current_segment = release_start;
            self.prev_level = self.level;
            self.time_in_segment = 0.0;
            self.stage = MsegStage::Releasing;
        } else {
            // Ingen release-sektion: fade snabbt till 0
            self.stage = MsegStage::Done;
        }
    }
}

/// Applicera kurvform på linjär t (0.0-1.0).
fn apply_curve(t: f32, curve: BipolarValue) -> f32 {
    let c = curve.as_f32();
    if c.abs() < 0.01 {
        t  // Linjär
    } else if c < 0.0 {
        // Exponentiell (snabb start, långsam slutning)
        let exp = 1.0 + (-c) * 4.0;
        t.powf(exp)
    } else {
        // Logaritmisk (långsam start, snabb slutning)
        let exp = 1.0 / (1.0 + c * 4.0);
        t.powf(exp)
    }
}
```

### Preset-mallar

MSEG kan initieras med vanliga former:

```rust
impl Mseg {
    /// Skapa standard ADSR
    pub fn from_adsr(a: Seconds, d: Seconds, s: NormalizedValue, r: Seconds) -> Self {
        Self::new(vec![
            MsegSegment { duration: a, target_level: NormalizedValue::MAX, curve: BipolarValue::CENTER, tempo_sync: None },
            MsegSegment { duration: d, target_level: s, curve: BipolarValue::new(-0.3), tempo_sync: None },
        ])
        .with_sustain_point(1)
        .with_release(vec![
            MsegSegment { duration: r, target_level: NormalizedValue::MIN, curve: BipolarValue::new(-0.5), tempo_sync: None },
        ])
    }

    /// Tremolo: snabb loop mellan två nivåer
    pub fn tremolo(rate: BeatDivision) -> Self {
        Self::new(vec![
            MsegSegment { duration: Seconds::ZERO, target_level: NormalizedValue::MAX, curve: BipolarValue::CENTER, tempo_sync: Some(rate) },
            MsegSegment { duration: Seconds::ZERO, target_level: NormalizedValue::MIN, curve: BipolarValue::CENTER, tempo_sync: Some(rate) },
        ])
        .with_loop(0, 1)
    }

    /// Sidechain-pump: snabb duck, långsam återhämtning
    pub fn sidechain_pump() -> Self {
        Self::new(vec![
            MsegSegment { duration: Seconds::new(0.01), target_level: NormalizedValue::MIN, curve: BipolarValue::CENTER, tempo_sync: None },
            MsegSegment { duration: Seconds::new(0.3), target_level: NormalizedValue::MAX, curve: BipolarValue::new(-0.7), tempo_sync: None },
        ])
        .with_loop(0, 1)
    }
}
```

### GUI: MSEG-editor

En visuell editor med dragbara brytpunkter:

```
┌─ MSEG ─────────────────────────────────────────────┐
│                                                      │
│  1.0 ─────╱╲                                         │
│           ╱  ╲         ╭──── sustain ────╮           │
│  0.7 ────╱    ╲───────╱                  ╲           │
│         ╱              │     loop ↺       ╲          │
│  0.0 ──╱               ╰─────────────────╱──────    │
│     ○──────○──────○────○────────────○────○───○       │
│     A(50ms) D(200ms) S-hold  loop  rel(500ms)       │
│                                                      │
│  [ADSR] [Tremolo] [Sidechain] [Custom]  Segments: 6 │
└──────────────────────────────────────────────────────┘
```

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/mod.rs` | `ModuleType::Mseg` |
| synth_core | `params/mseg.rs` | **NY** — `MsegParam` |
| synth_modules | `mseg.rs` | **NY** — MSEG-modul |
| synth_engine | `graph.rs` | Registrera MSEG |
| modular_synth | `gui/mseg_editor.rs` | **NY** — visuell MSEG-editor |
| modular_synth | `gui/module_panel.rs` | MSEG-panel med inbäddad editor |
| modular_synth | `patch.rs` | Serialisering av MSEG-segment |

### Uppskattad omfattning

- **synth_core:** ~80 rader (typer + params)
- **synth_modules:** ~350 rader (MSEG processering + presets)
- **modular_synth:** ~400 rader (MSEG-editor GUI)
- **Totalt:** ~830 rader

---

## 5. Generativ sekvensering

### Vad och varför

Tre generativa moduler som producerar triggers och CV-mönster algoritmiskt:

1. **Euclidean Sequencer** — fördelar N slag jämnt över M steg
2. **Turing Machine** — slumpmässigt muterande binärt skiftregister
3. **Random Gates** — sannolikhetsbaserade triggers

Dessa lever *i voice graph* som modulationsmoduler (som LFO/Envelope) och skickar gates/CV till andra moduler.

### Del A: Euclidean Sequencer

Euclidean-algoritmen fördelar `k` pulser jämnt över `n` steg. Ger naturligt intressanta rytmer:

- (3, 8) = tresillo [x . . x . . x .] (kubansk/afrikansk)
- (5, 8) = cinquillo [x . x x . x x .] (rumba)
- (7, 16) = [x . x x . x . x x . x . x x . x]

```rust
pub struct EuclideanSequencer {
    /// Antal steg i mönstret (1-32)
    steps: u8,
    /// Antal slag (pulser) i mönstret (0-steps)
    hits: u8,
    /// Rotation/offset av mönstret (0-steps)
    rotation: u8,
    /// Beräknat mönster (bitfält, max 32 steg)
    pattern: u32,
    /// Nuvarande steg-position
    current_step: u8,

    /// Tempo-synkad stegfrekvens
    step_division: BeatDivision,

    /// Swing amount (0.0 = rakt, 1.0 = max swing)
    swing: NormalizedValue,

    /// Gate-längd (0.0-1.0 av steg-längd)
    gate_length: NormalizedValue,

    // Timing state
    phase: f64,           // Ackumulerad fas (0.0-1.0 per steg)
    gate_active: bool,
    gate_timer: f32,      // Kvarvarande gate-tid i samples

    // Output
    gate_buffer: AudioBuffer,    // 0.0 eller 1.0
    accent_buffer: AudioBuffer,  // Valfri accent-output

    sample_rate: SampleRate,
}

impl EuclideanSequencer {
    /// Björklund/Euclidean-algoritm.
    fn calculate_pattern(steps: u8, hits: u8, rotation: u8) -> u32 {
        if steps == 0 || hits == 0 { return 0; }
        if hits >= steps { return (1u32 << steps) - 1; }

        // Björklunds algoritm
        let mut pattern = vec![false; steps as usize];
        let mut counts = vec![0u32; steps as usize];
        let mut remainders = vec![0u32; steps as usize];

        let mut divisor = (steps - hits) as u32;
        remainders[0] = hits as u32;
        let mut level = 0;

        loop {
            counts[level] = divisor / remainders[level];
            let new_remainder = divisor % remainders[level];
            divisor = remainders[level];
            remainders[level + 1] = new_remainder;
            level += 1;
            if remainders[level] <= 1 { break; }
        }
        counts[level] = divisor;

        // Bygg mönster från counts
        Self::build_pattern(&counts, &remainders, level, &mut pattern, &mut 0);

        // Konvertera till bitfält med rotation
        let mut result = 0u32;
        for i in 0..steps as usize {
            let rotated = (i + rotation as usize) % steps as usize;
            if pattern[rotated] {
                result |= 1 << i;
            }
        }
        result
    }
}

impl PolyModule for EuclideanSequencer {
    fn process(&mut self, inputs: InputPorts<'_>, outputs: &mut HashMap<String, AudioBuffer>, context: &ProcessContext) {
        let samples = context.samples.as_usize();
        let beats_per_sample = context.tempo.beats_per_sample(self.sample_rate);
        let step_beats = self.step_division.as_f32() as f64;

        // Reset-ingång
        let reset = inputs.get(PortName::intern("reset"));

        for i in 0..samples {
            // Reset-trigger
            if let Some(r) = reset {
                if r[i] > 0.5 && (i == 0 || r[i - 1] <= 0.5) {
                    self.current_step = 0;
                    self.phase = 0.0;
                }
            }

            // Avancera fas
            let prev_phase = self.phase;
            self.phase += beats_per_sample as f64 / step_beats;

            // Nytt steg?
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.advance_step();
            }

            // Gate output
            if self.gate_active {
                self.gate_timer -= 1.0;
                if self.gate_timer <= 0.0 {
                    self.gate_active = false;
                }
            }

            self.gate_buffer[i] = if self.gate_active { 1.0 } else { 0.0 };
        }

        if let Some(out) = outputs.get_mut("gate") {
            out.copy_from(&self.gate_buffer);
        }
    }
}
```

### Del B: Turing Machine

Ett binärt skiftregister som muterar slumpmässigt:

```rust
pub struct TuringMachine {
    /// Registerets längd (2-16 steg)
    length: u8,
    /// Binärt register (bitfält)
    register: u16,
    /// Sannolikhet att en bit flippas (0.0 = låst, 0.5 = kaos, 1.0 = inverterat låst)
    probability: NormalizedValue,
    /// Stegfrekvens (tempo-synkad)
    step_division: BeatDivision,
    /// Skalkvantisering
    scale: Scale,
    /// Utgångsrange i semitoner
    range: f32,

    // Timing
    phase: f64,
    current_value: f32,   // Senaste CV-värde

    // Output
    cv_buffer: AudioBuffer,    // Pitch CV (0.0-1.0)
    gate_buffer: AudioBuffer,  // Gate (bit 0 av registret)

    sample_rate: SampleRate,
}

#[derive(Debug, Clone, Copy)]
pub enum Scale {
    Chromatic,
    Major,
    Minor,
    Pentatonic,
    Blues,
    Dorian,
    Mixolydian,
    WholeTone,
}

impl TuringMachine {
    fn advance_step(&mut self) {
        // Läs ut bit som ska skiftas ut
        let high_bit = (self.register >> (self.length - 1)) & 1;

        // Eventuellt flippa biten
        let flip = fastrand::f32() < self.probability.as_f32();
        let new_bit = if flip { 1 - high_bit } else { high_bit };

        // Skifta registret
        self.register = ((self.register << 1) | new_bit as u16) & ((1 << self.length) - 1);

        // Beräkna CV-värde från registret
        let raw = self.register as f32 / ((1 << self.length) - 1) as f32;

        // Kvantisera till skala
        let semitones = raw * self.range;
        let quantized = self.scale.quantize(semitones);
        self.current_value = quantized / self.range;  // Normalisera tillbaka
    }
}

impl Scale {
    /// Kvantisera ett semitone-värde till närmaste ton i skalan.
    fn quantize(&self, semitones: f32) -> f32 {
        let intervals: &[u8] = match self {
            Self::Chromatic => &[0,1,2,3,4,5,6,7,8,9,10,11],
            Self::Major => &[0,2,4,5,7,9,11],
            Self::Minor => &[0,2,3,5,7,8,10],
            Self::Pentatonic => &[0,2,4,7,9],
            Self::Blues => &[0,3,5,6,7,10],
            Self::Dorian => &[0,2,3,5,7,9,10],
            Self::Mixolydian => &[0,2,4,5,7,9,10],
            Self::WholeTone => &[0,2,4,6,8,10],
        };

        let octave = (semitones / 12.0).floor();
        let note = semitones - octave * 12.0;

        // Hitta närmaste intervall
        let closest = intervals.iter()
            .min_by_key(|&&i| ((i as f32 - note).abs() * 100.0) as i32)
            .copied()
            .unwrap_or(0);

        octave * 12.0 + closest as f32
    }
}
```

### Del C: Random Gates

Probabilistisk gate-generator:

```rust
pub struct RandomGates {
    /// Sannolikhet att en gate triggas (0.0-1.0)
    density: NormalizedValue,
    /// Stegfrekvens
    step_division: BeatDivision,
    /// Hur många olika mönster (1 = ensamt, 2-4 = polyrytm)
    channels: u8,
    /// Gate-längd
    gate_length: NormalizedValue,
    /// Seed för reproducerbarhet (0 = random)
    seed: u32,

    // State per kanal
    phase: f64,
    gates: [bool; 4],
    gate_timers: [f32; 4],
    rng: fastrand::Rng,

    // Outputs
    gate_buffers: [AudioBuffer; 4],

    sample_rate: SampleRate,
}
```

### Integration med instrumentet

Generativa moduler lever i voice graph som **modulationskällor**:

```
[Euclidean] ──gate──→ [Envelope] trigger
            ──accent─→ [Filter] cutoff_cv

[Turing Machine] ──cv──→ [Oscillator] pitch (via mod matrix)
                 ──gate→ [Envelope] trigger

[Random Gates] ──gate_1─→ [Amplifier] cv (tremolo)
               ──gate_2─→ [LFO] retrigger
```

**Viktigt:** Dessa moduler behöver `ProcessContext` med tempo och beat-position för korrekt timing. Det finns redan i `ProcessContext`.

### Clock-input

Alla tre moduler tar en extern clock via port:

```rust
// Gemensam port
.port(PortDescriptor::control_input("clock", "Clock"))
.port(PortDescriptor::control_input("reset", "Reset"))
.port(PortDescriptor::audio_output("gate", "Gate"))
```

Om ingen extern clock är kopplad, körs intern clock baserad på `context.tempo` och `step_division`.

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/mod.rs` | `ModuleType::Euclidean`, `ModuleType::TuringMachine`, `ModuleType::RandomGates` |
| synth_core | `params/generative.rs` | **NY** — params för alla tre |
| synth_core | `types/scale.rs` | **NY** — `Scale` enum med kvantisering |
| synth_modules | `euclidean.rs` | **NY** — Euclidean Sequencer |
| synth_modules | `turing_machine.rs` | **NY** — Turing Machine |
| synth_modules | `random_gates.rs` | **NY** — Random Gates |
| synth_engine | `graph.rs` | Registrera nya moduler |
| modular_synth | `gui/module_panel.rs` | UI med mönster-visualisering |
| modular_synth | `patch.rs` | Serialisering |
| modular_synth | `gui/patch_bridge.rs` | Load/save |

### Uppskattad omfattning

- **Euclidean:** ~300 rader (Björklund-algoritm + modul + visualisering)
- **Turing Machine:** ~250 rader (skiftregister + skalkvantisering)
- **Random Gates:** ~200 rader (enklaste modulen)
- **Gemensamt:** ~150 rader (Scale, params, registrering)
- **Totalt:** ~900 rader

---

## 6. Character Filters — Analog filtermodeller

### Vad och varför

Befintliga filter (SVF med 7 modes + LadderFilter) är funktionella men saknar den distinkta *karaktär* som definierar ikonisk analog hårdvara. Moog, Korg MS-20 och Oberheim har alla unika olinjäriteter — asymmetrisk saturation, feedback-distortion, resonans som "skriker" — som ger dem personlighet.

Målet är att utöka befintlig `Filter`-modul med tre nya filtermodeller som var och en emulerar en specifik analog topologi med autentiska olinjäriteter. Alla använder Zero-Delay Feedback (ZDF) via trapezoidal integration för stabil, musikalisk resonans vid snabb cutoff-modulation.

### Designbeslut

**Utöka befintlig Filter, inte ny modul.** Lägg till en `FilterModel`-parameter (liknande `FmMode` på oscillatorn). Alla modeller delar Cutoff, Resonance, Drive — men *beter sig* annorlunda.

**ZDF ja, ADAA nej.** Trapezoidal integration (redan delvis på plats i SVF) ger stabilt beteende. ADAA kräver analytiska integraler per saturations-funktion och ger marginell vinst vid 48kHz med mjuk saturation. Kodbasens TODO har "2x oversampling" som löser aliasing generellt.

**Skalär f32, ingen SIMD.** Kodbasen processar en röst åt gången genom `PolyModule::process()`. Det finns ingen infrastruktur för `f32x4` — att införa det kräver ändring av hela engine-arkitekturen.

**Befintlig denormal-hantering.** `FilterState::flush_denormals()` (nollställer < 1e-15) fungerar bra. Inget behov av brusinjektion.

### De fyra modellerna

```
┌────────────────────────────────────────────────────────────────┐
│ FilterModel::Standard                                          │
│ Befintlig SVF (7 modes). Ren, mångsidig, neutral karaktär.    │
│ Topology: 2-integrator SVF, trapezoidal.                       │
│ Redan implementerad — inga ändringar.                          │
└────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────┐
│ FilterModel::Fluid (Oberheim-inspirerad SVF med morph)         │
│ Samma SVF-topologi, men med:                                   │
│ • Pre-filter tanh saturation (analog OP-amp värme)             │
│ • Morph-parameter: LP ↔ BP ↔ HP ↔ Notch                       │
│   via constant-power crossfade (sin/cos interpolation)         │
│ Karaktär: Varm, musikalisk, mjukt mättad. "Creamy."           │
└────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────┐
│ FilterModel::Screamer (Korg MS-20 K-35 inspirerad)             │
│ Topology: Sallen-Key, HP + LP i serie (12dB/oct).              │
│ • Asymmetrisk diod-clipping i feedback-loopen                  │
│ • Resonansen mättar filtret inifrån                            │
│ • Vid max resonans: asymmetrisk självsvängning ("skriker")     │
│ Karaktär: Rå, aggressiv, oförutsägbar. "Nasty."               │
└────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────┐
│ FilterModel::Acid (Steiner-Parker inspirerad)                  │
│ Topology: Unik multimode med feedback-förstärkning.            │
│ • Variabel saturation som ändras med resonansmängden           │
│ • Resonansens gain-omfång beror på vilken mode (LP/BP/HP)      │
│ • Wavefolding-liknande distortion vid hög resonans             │
│ Karaktär: Squelchy, aggressiv, levande. "Acid."               │
└────────────────────────────────────────────────────────────────┘
```

### Nya typer (synth_core)

```rust
/// Filtermodell — väljer underliggande topologi och karaktär.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FilterModel {
    /// Standard SVF — ren, mångsidig, befintligt beteende
    #[default]
    Standard,
    /// Oberheim-inspirerad SVF med morph och pre-saturation
    Fluid,
    /// MS-20-inspirerad Sallen-Key med diod-clipping
    Screamer,
    /// Steiner-Parker-inspirerad multimode med variabel saturation
    Acid,
}

impl FilterModel {
    pub const ALL: [Self; 4] = [Self::Standard, Self::Fluid, Self::Screamer, Self::Acid];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Fluid => "Fluid",
            Self::Screamer => "Screamer",
            Self::Acid => "Acid",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fluid => "fluid",
            Self::Screamer => "screamer",
            Self::Acid => "acid",
        }
    }
}
```

### Nya parametrar (synth_core/params/filters.rs)

```rust
pub enum FilterParam {
    // ... befintliga ...
    Mode(FilterMode),
    Cutoff(Hertz),
    Resonance(NormalizedValue),
    KeyTracking(NormalizedValue),
    Drive(Gain),
    EnvAmount(BipolarValue),
    CutoffMod(BipolarValue),

    // NYA
    /// Filtermodell (Standard, Fluid, Screamer, Acid)
    Model(FilterModel),
    /// Morph-parameter för Fluid-modellen (LP→BP→HP→Notch)
    Morph(NormalizedValue),
}
```

`Model` och `Morph` behöver `name()`, `as_f32()`, `with_f32()` och default-helpers, precis som befintliga varianter.

### DSP-implementationer (synth_dsp/src/filters.rs)

#### Fluid — SVF med morph och pre-saturation

```rust
/// Fluid filter: Oberheim-inspirerad SVF med morph och ingångssaturation.
/// Använder befintlig SvfCoeffs men beräknar alla utgångar simultant.
pub struct FluidFilter {
    ic1eq: f32,
    ic2eq: f32,
}

impl FluidFilter {
    /// Process med morph-parameter.
    /// morph: 0.0=LP, 0.33=BP, 0.66=HP, 1.0=Notch
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        coeffs: &SvfCoeffs,
        drive: f32,
        morph: f32,
    ) -> f32 {
        // Pre-filter saturation: mjuk tanh för analog OP-amp värme
        let saturated = (input * drive).tanh() / drive.max(0.01);

        // SVF — beräkna alla utgångar simultant
        let v3 = saturated - self.ic2eq;
        let v1 = coeffs.a1 * self.ic1eq + coeffs.a2 * v3;
        let v2 = self.ic2eq + coeffs.a2 * self.ic1eq + coeffs.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        let lp = v2;
        let bp = v1;
        let hp = saturated - coeffs.k * v1 - v2;
        let notch = saturated - coeffs.k * v1;

        // Constant-power morph: sin/cos crossfade mellan utgångar
        // 4 zoner: LP(0.0) → BP(0.33) → HP(0.66) → Notch(1.0)
        let morph_scaled = morph * 3.0;
        let zone = morph_scaled.floor().min(2.0) as u8;
        let t = morph_scaled.fract();
        let angle = t * std::f32::consts::FRAC_PI_2;
        let (a, b) = match zone {
            0 => (lp, bp),
            1 => (bp, hp),
            _ => (hp, notch),
        };
        a * angle.cos() + b * angle.sin()
    }
}
```

#### Screamer — Sallen-Key med diod-clipping

```rust
/// Screamer filter: MS-20-inspirerad Sallen-Key med asymmetrisk diod-clipping.
/// HP och LP i serie (12dB/oct totalt), diod-clip i feedback.
pub struct ScreamerFilter {
    /// HP-sektionens state
    hp_s1: f32,
    hp_s2: f32,
    /// LP-sektionens state
    lp_s1: f32,
    lp_s2: f32,
}

impl ScreamerFilter {
    /// Asymmetrisk diod-clipping: hårdare knä än tanh, positiv/negativ asymmetri.
    #[inline]
    fn diode_clip(x: f32) -> f32 {
        // Mjuk diod-modell: asymmetrisk positive/negative clipping
        if x > 0.0 {
            // Positivt: genomsläppande med mjuk kompression
            1.0 - (-x * 0.8).exp()
        } else {
            // Negativt: hårdare clipping (simulerar diod-drop)
            -(1.0 - (x * 1.2).exp())
        }
    }

    /// Process en sample genom HP → LP kedjan med diod-feedback.
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        g: f32,        // tan(pi * fc / fs) — ZDF-koefficient
        resonance: f32, // 0.0-1.0
        drive: f32,     // 1.0-20.0
    ) -> f32 {
        // Feedback med diod-clipping (det som ger "skrik"-karaktären)
        let feedback = Self::diode_clip(self.lp_s2 * drive) * resonance * 4.0;

        // HP-sektion (trapezoidal integration)
        let hp_in = input - feedback;
        let hp_out = (hp_in - self.hp_s1 - self.hp_s2) / (1.0 + g + g * g);
        let hp_v1 = g * hp_out;
        self.hp_s1 += 2.0 * hp_v1;
        self.hp_s2 += 2.0 * g * (hp_out + hp_v1);

        // LP-sektion
        let lp_in = hp_out + hp_v1 + self.hp_s2; // BP-output in i LP
        let lp_out = (lp_in + self.lp_s1 * g + self.lp_s2) / (1.0 + g + g * g);
        let lp_v1 = g * (lp_in - lp_out);
        self.lp_s1 += 2.0 * lp_v1;
        self.lp_s2 = lp_out;

        lp_out
    }
}
```

#### Acid — Steiner-Parker med variabel saturation

```rust
/// Acid filter: Steiner-Parker-inspirerad med resonans-beroende saturation.
/// Unik egenskap: resonansens gain ändras beroende på vilken mode som är aktiv.
pub struct AcidFilter {
    s1: f32,
    s2: f32,
}

impl AcidFilter {
    /// Variabel saturation som ändras med resonansmängd.
    /// Låg resonans → mjuk tanh. Hög resonans → wavefolding-liknande.
    #[inline]
    fn variable_saturate(x: f32, resonance: f32) -> f32 {
        let blend = resonance * resonance; // Exponentiell kurva
        let soft = x.tanh();
        // Enkel wavefold: sin(x * pi/2) ger vikning vid ±1
        let fold = (x * std::f32::consts::FRAC_PI_2).sin();
        soft * (1.0 - blend) + fold * blend
    }

    /// Process en sample. filter_mode påverkar resonansens gain-omfång.
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        g: f32,
        resonance: f32,
        drive: f32,
        filter_mode: SvfFilterType, // LP, BP, HP
    ) -> f32 {
        // Resonansens gain beror på mode (Steiner-Parker-egenskap)
        let q_scale = match filter_mode {
            SvfFilterType::Lowpass => 4.0,   // Mest resonans i LP
            SvfFilterType::Bandpass => 3.0,  // Något mindre i BP
            SvfFilterType::Highpass => 2.5,  // Minst i HP
            _ => 3.5,
        };

        let feedback = Self::variable_saturate(self.s2 * drive, resonance)
            * resonance * q_scale;

        // ZDF 2-pole med saturation i feedback
        let v0 = input - feedback;
        let v1 = (g * v0 + self.s1) / (1.0 + g);
        self.s1 = 2.0 * v1 - self.s1;
        let v2 = (g * v1 + self.s2) / (1.0 + g);
        self.s2 = 2.0 * v2 - self.s2;

        // Välj utgång baserat på mode
        match filter_mode {
            SvfFilterType::Lowpass => v2,
            SvfFilterType::Highpass => v0 - v1 - v2,
            SvfFilterType::Bandpass => v1,
            _ => v2, // Fallback till LP
        }
    }
}
```

### Ändringar i Filter-modulen (synth_modules/src/filter.rs)

```rust
pub struct Filter {
    // ... befintliga parametrar (cutoff, resonance, drive, etc.) ...

    // NYA
    model: FilterModel,
    morph: NormalizedValue,  // Bara för Fluid

    // State per modell (alla pre-allokerade, bara en används åt gången)
    svf_ic1eq: f32,       // Standard
    svf_ic2eq: f32,
    fluid: FluidFilter,   // Fluid
    screamer: ScreamerFilter, // Screamer
    acid: AcidFilter,     // Acid

    // ... befintliga mod offsets, output buffer, etc. ...
}
```

Process-loopen väljer modell via match:

```rust
fn process(&mut self, /* ... */) {
    // ... befintlig cutoff/resonance/drive-beräkning ...
    // ... befintlig CV-input-hantering ...

    for i in 0..n_samples {
        let input = /* ... */;
        let g = cutoff.to_tan_coeff(self.sample_rate);

        let output = match self.model {
            FilterModel::Standard => {
                // Befintlig SVF-kod (oförändrad)
                self.svf_coeffs.process(input, &mut self.svf_ic1eq, &mut self.svf_ic2eq, svf_type)
            }
            FilterModel::Fluid => {
                let coeffs = SvfCoeffs::new(cutoff, resonance, self.sample_rate);
                self.fluid.process(input, &coeffs, drive, self.morph.as_f32())
            }
            FilterModel::Screamer => {
                self.screamer.process(input, g, resonance, drive)
            }
            FilterModel::Acid => {
                self.acid.process(input, g, resonance, drive, svf_type)
            }
        };

        // Denormal-hantering
        self.flush_all_states();

        self.output_buffer[i] = output;
    }
}
```

### Descriptor-uppdatering

```rust
// Ny parameter i descriptor():
.parameter(
    ParameterDescriptor::choice(
        Param::Filter(FilterParam::Model(FilterModel::Standard)),
        "Model",
        FilterModel::to_choices(),
    )
    .description("Filter model: Standard, Fluid, Screamer, Acid")
    .widget(WidgetHint::Dropdown),
)
.parameter(
    ParameterDescriptor::float(
        Param::Filter(FilterParam::Morph(NormalizedValue::ZERO)),
        "Morph",
    )
    .description("Fluid: LP→BP→HP→Notch crossfade")
    .range(0.0, 1.0)
    .default(0.0)
    .widget(WidgetHint::Knob),
)
```

### set_param / get_param / get_params

```rust
// I set_param:
FilterParam::Model(m) => {
    self.model = m;
    // Nollställ alla filter-states vid modellbyte
    self.reset_filter_states();
}
FilterParam::Morph(v) => self.morph = v,

// I get_param:
FilterParam::Model(_) => self.model.index() as f32,
FilterParam::Morph(_) => self.morph.as_f32(),

// I get_params: lägg till båda
```

### Realtidssäkerhet

- **Inga allokeringar:** Alla filter-states är fasta fält i structen. `FluidFilter`, `ScreamerFilter`, `AcidFilter` är Copy-typer med enbart `f32`-fält.
- **Ingen branching per sample (nästan):** Match på `FilterModel` sker en gång per sample — förutsägbar branch som CPU:ns branch predictor hanterar väl (modellen ändras inte under buffern).
- **Denormal-hantering:** `flush_denormals()` på alla aktiva state-variabler.
- **Koefficient-beräkning:** `SvfCoeffs::new()` (trigonometri) kan lyftas ut ur sample-loopen om cutoff inte moduleras per-sample. Vid CV-modulation beräknas den per sample (samma som befintlig kod).

### Bakåtkompatibilitet

- **Default = Standard:** Befintliga patchar som inte sätter `Model` får `FilterModel::Standard` och beter sig exakt som innan.
- **Morph ignoreras:** Om `Model != Fluid` har Morph-parametern ingen effekt.
- **Inga ändrade portar:** Samma in/ut-portar som innan.

### Implementeringsordning (inom denna feature)

1. **Fluid först** — närmast befintlig SVF, mest straight-forward. Validerar `FilterModel`-infrastrukturen.
2. **Screamer** — unik topologi (Sallen-Key), kräver egen ZDF-implementation.
3. **Acid** — mest komplex (variabel saturation, mode-beroende resonans). Implementeras sist.

### Berörda filer

| Crate | Fil | Ändring |
|-------|-----|---------|
| synth_core | `params/filters.rs` | `FilterModel` enum, `Model(FilterModel)` + `Morph(NormalizedValue)` varianter |
| synth_dsp | `filters.rs` | `FluidFilter`, `ScreamerFilter`, `AcidFilter` structs + DSP |
| synth_modules | `filter.rs` | `model`/`morph` fält, match i `process()`, uppdaterad `descriptor()` |
| modular_synth | `gui/module_panel.rs` | Model-dropdown, Morph-knob (synlig när Fluid) |
| modular_synth | `patch.rs` | Serialisering (bakåtkompatibel — saknad Model = Standard) |

### Uppskattad omfattning

- **synth_core:** ~80 rader (FilterModel enum + 2 nya FilterParam-varianter)
- **synth_dsp:** ~200 rader (3 filter-structs med process-metoder)
- **synth_modules:** ~100 rader (integration i filter.rs)
- **modular_synth:** ~30 rader (GUI + patch)
- **Totalt:** ~410 rader

---

## 7. Implementeringsordning

### Status (2026-02-13)

| # | Feature | Status | Version |
|---|---------|--------|---------|
| 1 | Mod Matrix | KLAR | 0.107.0–0.112.0 |
| 2 | Waveshaper | KLAR | 0.113.0 |
| 3 | Unison | KLAR | 0.114.0 |
| 4 | Character Filters | PLANERAD | — |
| 5 | MSEG | PLANERAD | — |
| 6 | Wavetable-oscillator | PLANERAD | — |
| 7 | Generativa moduler | PLANERAD | — |

### Rekommenderad ordning för resterande features

```
Nästa: Character Filters    ← Direkt ljudkvalitetsvinst, utökar befintlig modul
  │
Sedan: MSEG                 ← Stor kreativ vinst, modulerar Character Filters
  │
Sedan: Wavetable-oscillator ← Ny ljudkälla, kräver mest ny infrastruktur
  │
Sist:  Generativa moduler   ← Störst scope, beroende av mod matrix
```

### Prioritering och motivering

1. **Character Filters näst** — Ger omedelbar uppgradering av *varje* befintlig patch som använder filter. Tre distinkta analoga karaktärer (Fluid, Screamer, Acid) förvandlar synten från "bra filter" till "val av klassiska filtermodeller". Bygger på befintlig `Filter`-modul (~410 rader, inga nya filer). Snabbast att implementera av de resterande.

2. **MSEG efter det** — Kräver visuell editor (mest GUI-arbete) men ger enorm kreativ kraft. Med mod matrix + character filters redan på plats kan MSEG modulera nya parametrar (filter morph, model-specifik drive) för evolverande ljud.

3. **Wavetable-oscillator** — Ny ljudkälla med scanbar position. Mest ny infrastruktur (wavetable data-format, interpolation, inbyggda tables). Oberoende av andra features men ger mest värde *efter* att modulationskedjan (mod matrix + MSEG) redan finns.

4. **Generativa moduler sist** — Euclidean, Turing Machine, Random Gates. Störst scope (3 nya moduler), och mest nytta när hela modulationskedjan finns. Producerar gate/CV som routas genom matrisen till filter, oscillatorer och envelopes.

### Beroendegraf

```
  ┌──────────────┐
  │  Mod Matrix  │ ✓ KLAR
  └──────┬───────┘
         │
  ┌──────▼───────┐     ┌──────────┐
  │  Waveshaper  │ ✓   │  Unison  │ ✓
  └──────────────┘     └──────────┘

  ┌─────────────────────────────────────────────────┐
  │  Resterande (i prioritetsordning):               │
  │                                                   │
  │  1. Character Filters  (utökar Filter)            │
  │     │                                             │
  │  2. MSEG  (modulerar character filter params)     │
  │     │                                             │
  │  3. Wavetable Osc  (oberoende, ny ljudkälla)      │
  │     │                                             │
  │  4. Generativa moduler  (Euclidean, Turing, Gates)│
  └─────────────────────────────────────────────────┘
```

### Total omfattning (resterande)

| Feature | Rader (uppskattning) | Nya filer | Ändrade filer |
|---------|---------------------|-----------|--------------|
| Character Filters | ~410 | 0 | 4 |
| MSEG | ~830 | 2 | 5 |
| Wavetable-oscillator | ~600 | 3 | 4 |
| Generativa moduler | ~900 | 4 | 5 |
| **Totalt resterande** | **~2740** | **9** | **~13 unika** |

### Historisk versionering (genomförda)

| Version | Innehåll |
|---------|----------|
| 0.107.0 | Mod Matrix (8 slots, 10 källor, 11 destinationer) |
| 0.108.0–0.112.0 | Mod Matrix: smarta namn, grid-layout, enabled-checkbox |
| 0.113.0 | Waveshaper-modul (6 kurvor: Soft Clip, Fold, Chebyshev, etc.) |
| 0.114.0 | Intra-voice Unison i Oscillator (1-7 röster, detune, stereo spread) |
