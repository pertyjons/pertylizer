# Implementation Plan: The Acoustic World Engine (AWE)

## 1. Vision

AWE ger syntetiserade ljud ett fysiskt **rum att existera i**. Till skillnad
från en vanlig convolution reverb är AWE en **realtids-rumsmotor** där alla
parametrar — rummets dimensioner, material, käll- och lyssnarposition — kan
moduleras kontinuerligt via AWE-interna LFO:er.

Rummet kan vara allt från ett litet studio till en 1 km lång stålpipeline,
en grotta, eller ett "omöjligt" rum där basen hör en katedral och diskanten
hör en garderob.

### Vad som skiljer AWE från en reverb

| Egenskap | Convolution Reverb | AWE |
|---|---|---|
| Källa | Statisk IR-fil | Realtidsberäknat från geometri |
| Modulation | Ingen (eller knäppig crossfade) | Alla parametrar i realtid |
| Rumsmoder | Ej modellerade | Analytiska, stämda till dimensioner |
| Tidiga reflektioner | Baked in IR | ISM — ändras med käll-/lyssnarposition |
| "Omöjliga rum" | Nej | Ja (freq warp, negativ absorption, portaler) |
| CPU vid idle | Konstant (convolution) | Nära noll (FDN + delay taps) |

---

## 2. Arkitektur

### 2.1 Egen Cargo-crate

AWE är en **självständig crate** (`synth_awe`) utan beroenden till
`synth_modules` eller `synth_engine`. Detta ger:

- **Isolerad testning**: Alla DSP-algoritmer testbara utan engine-kontext
- **Noll cirkulära beroenden**: synth_engine beror på synth_awe, inte tvärtom
- **Tydlig API-yta**: AweEngine + AweParam är allt som exponeras

```
Beroendegraf:

synth_core          (newtypes: Hertz, Gain, Phase, SampleRate, etc.)
    ↑
synth_dsp           (DelayLine, InterpolatedDelayLine, FdnCore, filters)
    ↑
synth_awe           (AweEngine, AweParam, ISM, RoomModeBank, Spatializer)
    ↑
synth_engine        (SynthEngine anropar AweEngine::process())
    ↑
modular_synth       (GUI: awe_view.rs)
```

**synth_awe beror enbart på synth_core + synth_dsp**. Inga andra beroenden.

### 2.2 Signalflöde i SynthEngine

```
SynthEngine::process()
│
├── 1. Instruments → voice_left/right → effect_chain.process()
│                                       (Delay, Reverb, Chorus, etc.)
│
├── 2. master_left/right (summa av alla instrument)
│
├── 3. master_effects.process() (EQ, Compressor, etc.)
│
├── 4. ★ AWE (eget engine-steg) ★
│      │
│      │   Stereo In
│      │   │
│      │   ▼
│      │   ┌──────────────────────────────────────────────────────┐
│      │   │                    AweEngine                         │
│      │   │                                                      │
│      │   │   ┌──────────────────┐                               │
│      │   │   │ Early Reflections│  Image Source Method (ISM)    │
│      │   │   │ (tapped interp.  │  1:a + 2:a ordningen          │
│      │   │   │  delay + filter) │  InterpolatedDelayLine        │
│      │   │   └────────┬─────────┘                               │
│      │   │            │                                         │
│      │   │   ┌────────▼─────────┐                               │
│      │   │   │ FDN Late Reverb  │  FdnCore (från synth_dsp)    │
│      │   │   │ (8-kanal Hadamard│  Per-kanal filter + mod.      │
│      │   │   │  med modulation) │  delays                       │
│      │   │   └────────┬─────────┘                               │
│      │   │            │                                         │
│      │   │   ┌────────▼─────────┐                               │
│      │   │   │ Room Mode Bank   │  3-6 comb-filter              │
│      │   │   │                  │  f_n = c/(2·dim)              │
│      │   │   └────────┬─────────┘                               │
│      │   │            │                                         │
│      │   │   ┌────────▼─────────┐                               │
│      │   │   │ Spatializer      │  Enbart på wet-signal         │
│      │   │   │ (ITD + ILD)      │  InterpolatedDelayLine        │
│      │   │   └────────┬─────────┘                               │
│      │   │            │                                         │
│      │   │   Interna LFO:er → modulerar parametrar per sample   │
│      │   │   Mix: Dry/Wet · Early/Late · Modes Amount           │
│      │   └────────────┼────────────────────────────────────────┘
│      │                ▼
│      │           Stereo Out
│      │
├── 5. Global visualizers (post-AWE: ser slutresultatet)
│      (Oscilloscope, LevelMeter, SpectrumAnalyzer)
│
└── 6. Final stereo output → audio backend
```

**Visualizers körs efter AWE** så att de visar den slutgiltiga signalen.

Idag ligger visualizers som `ChainSlot::Visualizer` i samma `Vec<ChainSlot>`
som effekter i `EffectChain`. De modifierar inte signalen — enbart
`write_interleaved()` + `update_levels_interleaved()` — men de processas
inuti `EffectChain::process()` som en del av master_effects, dvs **pre-AWE**.

**Lösning: Delad EffectChain-processing.**

`EffectChain` får en ny metod `process_visualizers()` som bara kör
visualizer-slots. `process()` hoppar över dem. Visualizer-anropet sker
sedan i `SynthEngine::process()` efter AWE-steget:

```rust
// I EffectChain:
pub fn process(&mut self, mix_buffer: &mut AudioBuffer, context: &ProcessContext) {
    for slot in &mut self.slots {
        match slot {
            ChainSlot::Effect(effect_slot) => { /* befintlig effektlogik */ }
            ChainSlot::Visualizer(_) => { /* skip — hanteras separat */ }
        }
    }
}

pub fn process_visualizers(&mut self, mix_buffer: &AudioBuffer) {
    for slot in &mut self.slots {
        if let ChainSlot::Visualizer(viz) = slot {
            if viz.state.is_active() {
                viz.buffer.write_interleaved(mix_buffer.as_slice());
                viz.buffer.update_levels_interleaved(mix_buffer.as_slice());
            }
        }
    }
}

// I SynthEngine::process():
self.process_master_effects(&process_context);   // Enbart effekter
self.awe_engine.process(&mut self.mix_buffer);   // AWE
self.master_effects.process_visualizers(&self.mix_buffer);  // Post-AWE
```

**Ingen strukturändring av ChainSlot eller EffectChain::slots** — samma
unified Vec, bara separerade processeringspass.

### 2.3 AWE som eget engine-steg

AWE är **inte** en slot i `master_effects` (EffectChain). Det är ett eget
steg i `SynthEngine::process()` med egen state och egna kommandon:

```rust
// I SynthEngine:
awe_engine: AweEngine,  // Fixed global slot, alltid allokerad

// I process():
if self.awe_engine.enabled() {
    // mix_buffer är interleaved stereo: [L0, R0, L1, R1, ...]
    // Matchar AudioBuffer-formatet — ingen kopiering behövs.
    self.awe_engine.process(self.mix_buffer.as_mut_slice(), sample_rate);
}
```

**Process-signatur**: `AweEngine::process(&mut self, buffer: &mut [f32], sample_rate: f32)`

AWE processar interleaved stereo in-place, samma format som `AudioBuffer`.
Intern split till L/R sker bara för DSP-steg som kräver det (ISM, FDN) —
dessa arbetar sample-för-sample och behöver inte separata buffers.

**Varför inte i master_effects?**
- AWE har specialiserade parametrar (RoomShape, Material, positioner) som
  inte passar i `Param`-enumen. Att trycka in dem skapar massor av dead
  code (name/as_f32/with_f32/module_type-mappings i params/mod.rs).
- AWE har egen vy (`AppView::AcousticWorld`) med 2D-planritning och
  rikare interaktion (drag-and-drop) än `SetModuleParameter` erbjuder.
- Fixed global slot med bypass — ingen add/remove livscykel.

### 2.4 Param-routing: AweParam helt separat

`AweParam` definieras i `synth_awe`-craten och har **ingen koppling till
`Param`-enumen** i synth_core. Ingen `Param::Awe(AweParam)`, ingen
`ModuleType::AcousticWorld`. Routing sker enbart via:

```rust
// I EngineCommand (synth_engine):
SetAweParameter { param: synth_awe::AweParam },
SetAweEnabled { enabled: bool },
/// Batch-kommando: sätter alla AWE-parametrar atomiskt.
/// Används vid patch-load och drag-operationer för att undvika
/// ringbuffer-stress (16384 capacity, men 15+ parametrar per drag
/// frame × 60fps = hundratals kommandon/sekund).
SetAweState { state: synth_awe::AweSnapshot },
```

**Batch-strategi**: GUI throttlar drag-uppdateringar till ett
`SetAweState` per frame istället för individuella `SetAweParameter`.
`AweSnapshot` är en flat struct (Copy, ~80 bytes) med alla numeriska
parametrar — ingen heap-allokering. Vid patch-load: ett `SetAweState`
istället för 15+ `SetAweParameter`.

**Varför?**
- `Param` + `ModuleType` är designade för per-röst moduler med fasta
  mappings (name, as_f32, with_f32, module_type, prefix). AWE passar inte.
- Dubbel routing (`Param::Awe` + `EngineCommand::SetAweParameter`) ger
  förvirring och dead code.
- Ren separation: synth_core vet inget om AWE, synth_awe vet inget om
  voice-moduler. Varje crate har ett jobb.

### 2.5 Varför inte ray tracing + IR?

Den ursprungliga planen använde stochastisk ray tracing → IR → convolver.
Denna approach har tre kritiska problem:

1. **RT-säkerhet**: IR-swap kräver antingen lock-free allokering eller
   dubbel-convolver med crossfade. Nuvarande `PartitionedConvolver`
   allokerar vid rebuild — bryter mot RT-kravet.

2. **Ingen modulation**: Statisk IR innebär att alla parametrar fryser
   tills nästa beräkning. "Mouse up only" eliminerar kreativ potential.

3. **Oproportionell komplexitet**: CSG + ray tracing + IR-generering +
   crossfade-convolver ≈ 2000+ rader greenfield-kod. Resultatet: en
   fancy reverb.

**Den reviderade arkitekturen** (ISM + FDN + comb bank) ger:
- Alla parametrar modulerbara i realtid
- Noll allokeringar i audio-tråden
- Återanvänder extraherad FDN (FdnCore i synth_dsp)
- Inga nya externa dependencies
- ~1100 rader totalt

### 2.6 Plats i kodbasen

```
crates/
├── synth_core/                        # Oförändrad — vet inget om AWE
│
├── synth_dsp/src/
│   └── fdn.rs                         # FdnCore — extraherad från synth_modules
│
├── synth_awe/                         # ★ NY CRATE ★
│   ├── Cargo.toml                     # deps: synth_core, synth_dsp
│   ├── src/
│   │   ├── lib.rs                     # pub exports
│   │   ├── awe_engine.rs             # AweEngine struct + process()
│   │   ├── params.rs                  # AweParam enum (alla parametrar)
│   │   ├── early_reflections.rs       # ISM implementation
│   │   ├── room_modes.rs             # Comb-filter bank
│   │   ├── spatializer.rs            # ITD + ILD
│   │   ├── room.rs                    # RoomShape, Material, geometry
│   │   └── lfo.rs                     # AWE-interna LFO:er
│   └── tests/
│       └── integration.rs             # Crate-interna tester
│
├── synth_modules/src/effects/
│   └── reverb.rs                      # FdnReverb delegerar till synth_dsp::FdnCore
│
├── synth_engine/src/
│   ├── commands.rs                    # EngineCommand::SetAweParameter/SetAweEnabled
│   └── synth_engine.rs               # AWE-steg i process(), command-hantering
│
└── modular_synth/src/gui/
    ├── app/state.rs                   # AppView::AcousticWorld
    ├── awe_view.rs                    # 2D planritning + kontroller
    ├── egui_backend.rs                # View routing, AWE-toolbar
    └── patch_bridge.rs                # AweState load/save wiring
```

---

## 3. RT-säkerhet: Pre-allokering

### 3.1 Max-dimensioner

Alla delay-linjer allokeras vid skapande och **ändrar aldrig storlek**.
`DelayLine::resize()` allokerar via `Vec::resize` — detta får **aldrig**
anropas runtime.

Max-delay måste täcka:
- **Största preset**: Steel Pipeline = 200m, men ISM-ordning begränsas
- **Högsta sample rate**: 96 kHz
- **Marginal**: Beror på ISM-ordning per preset

**Nyckelinsikt: 1.0s max delay ≠ 2:a ordningens ISM för alla rum.**
200m pipeline med 2:a ordningen → 400m → 1.17s — överskrider 1.0s.

**Lösning: Dynamisk ISM-ordning.**
- Rum ≤ 85m (2:a ordningen ≤ 170m/343 = 0.50s): **2:a ordning** (18 taps)
- Rum 85–170m (1:a ordningen ≤ 340m/343 = 0.99s): **1:a ordning** (6 taps)
- Rum > 170m: **Enbart direkt + FDN** (inga ISM-taps, FDN-delays klampas)

Steel Pipeline preset kör 1:a ordningens ISM. "The Void" (100m) kör
enbart FDN — ISM-reflektioner i 100m³ rum är för glesa för att höras.

```
Max delay-tid: 1.0 sekund
  (täcker 343m avstånd, tillräckligt för 1:a ordningens ISM
   upp till ~170m och alla FDN delay-tider)

Max delay-samples: 1.0s × 96_000 Hz = 96_000 samples
Avrundning: 98_304 (96 × 1024, power-of-2-vänligt)

Minnesbudget (worst case @ 96kHz):
  ISM:         2 InterpolatedDelayLine × 98_304 × 4 bytes = 768 KB
  FDN:         8 kanaler × 98_304 × 4 bytes                = 3.0 MB
  Comb bank:   6 kanaler × 98_304 × 4 bytes                = 2.3 MB
  Spatializer: 2 InterpolatedDelayLine × 64 × 4 bytes     ≈ 0.5 KB
  ──────────────────────────────────────────────────────────────
  Totalt: ~6 MB (engångsallokering vid AweEngine::new())
```

Vid parameterändringar justeras bara `read_position` (varifrån i buffern
vi läser), inte bufferstorleken. Oanvända delar av buffern är nollor.

Presets som "The Void" (100×100×100m) kör utan ISM (enbart FDN + moder).
GUI klampar dimensioner så att vald ISM-ordning ryms inom 1.0s max delay.

### 3.2 InterpolatedDelayLine för ISM

ISM-reflektioner ger fraktionella delay-tider (t.ex. 3.7m / 343 m/s
× 44100 Hz = 475.3 samples). `DelayLine` stödjer bara heltal.

**Lösning**: Använd befintlig `InterpolatedDelayLine` (synth_dsp) som
har linjär interpolering. Denna finns redan och används för chorus.

```rust
struct EarlyReflections {
    taps: [EarlyTap; MAX_EARLY_TAPS],
    active_taps: usize,
    delay_line_left: InterpolatedDelayLine,   // fraktionell delay
    delay_line_right: InterpolatedDelayLine,
}
```

### 3.3 Sample-rate hantering

`SynthEngine::on_stream_start()` uppdaterar `self.sample_rate`,
`metering` och `sequencer` — men **inte** effekter eller moduler.
AWE:s delay-tider beror direkt på sample rate (meter → samples).

**Lösning**: AWE tar `sample_rate` som parameter i `process()`:

```rust
pub fn process(&mut self, buffer: &mut [f32], sample_rate: f32) {
    if (sample_rate - self.cached_sample_rate).abs() > 0.1 {
        self.cached_sample_rate = sample_rate;
        self.recalculate_all_delays();  // Alla tider → samples
    }
    // ... DSP
}
```

Inga nya hooks behövs. `SynthEngine` skickar `self.sample_rate` vid
varje `process()`-anrop. AWE cachear och omberäknar bara vid ändring.

---

## 4. Datamodell

### 4.1 Rumsgeometri

```rust
/// Rum-form som styr ISM-beräkningar och FDN-parametrar.
pub enum RoomShape {
    /// Rektangulärt rum (L × W × H meter).
    Box { length: f32, width: f32, height: f32 },

    /// Cylindriskt rum (radie × höjd).
    Cylinder { radius: f32, length: f32 },

    /// L-format rum (två sammankopplade rektanglar).
    LShape {
        length_a: f32, width_a: f32,
        length_b: f32, width_b: f32,
        height: f32,
    },
}

impl RoomShape {
    /// Beräkna rummets volym (m³) — driver FDN reverb time.
    pub fn volume(&self) -> f32;

    /// Beräkna rummets yta (m²) — driver absorption.
    pub fn surface_area(&self) -> f32;

    /// Axiella rumsmoder: f_n = c / (2 · dimension).
    /// Returnerar max 6 moder i pre-allokerad array (inga heap-allokeringar).
    pub fn axial_modes(&self) -> ([f32; 6], usize);
}
```

### 4.2 Material

```rust
/// Akustiskt material med frekvensberoende absorption.
pub struct Material {
    pub name: &'static str,
    pub absorption_low: f32,   // < 500 Hz
    pub absorption_mid: f32,   // 500 Hz – 4 kHz
    pub absorption_high: f32,  // > 4 kHz
    pub diffusion: f32,        // 0.0 = spegel, 1.0 = helt diffus
}

// Presets (6 st)
const CONCRETE: Material = Material { name: "Concrete", absorption_low: 0.01, .. };
const WOOD:     Material = Material { name: "Wood",     absorption_low: 0.15, .. };
const GLASS:    Material = Material { name: "Glass",    absorption_low: 0.04, .. };
const METAL:    Material = Material { name: "Metal",    absorption_low: 0.01, .. };
const FABRIC:   Material = Material { name: "Fabric",   absorption_low: 0.10, .. };
const TILE:     Material = Material { name: "Tile",     absorption_low: 0.02, .. };
```

### 4.3 "Omöjliga rum"-parametrar

```rust
/// Parametrar som bryter fysikens lagar för kreativa effekter.
pub struct ImpossibleParams {
    /// Frekvensbaserad rumsstorlek: bas ser ett större rum.
    /// 0.0 = normalt, 1.0 = bas ser 4x större rum.
    pub freq_warp: f32,

    /// Negativ absorption: väggar förstärker ljud.
    /// 0.0 = fysisk, 1.0 = +6dB per reflektion (med intern limiter).
    pub resonance_boost: f32,

    /// Svans-stretch: reverb-tid oberoende av rum-storlek.
    /// 0.0 = fysisk, >0 = längre, <0 = kortare.
    pub tail_stretch: f32,
}
```

### 4.4 Persistence (patch-format)

**GUI-state är canonical.** AWE-vyns state serialiseras vid save och
deserialiseras vid load. `EngineState` innehåller **inget** AWE-data.

```rust
// I PatchSettings:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub awe: Option<AweState>,

#[derive(Serialize, Deserialize)]
pub struct AweState {
    pub enabled: bool,
    pub room_shape: RoomShapeState,
    pub material: String,         // preset-namn eller "custom"
    pub source_pos: (f32, f32),
    pub listener_pos: (f32, f32),
    pub dry_wet: f32,
    pub early_late_balance: f32,
    pub modes_amount: f32,
    pub impossible: ImpossibleState,
    pub lfos: Vec<LfoState>,     // 0-4 interna LFO:er
}
```

**patch_bridge.rs — explicit load/save wiring**:

```rust
// I load_patch() — GUI-state → EngineCommands:
if let Some(awe_state) = &patch.settings.awe {
    handle.send_blocking(EngineCommand::SetAweEnabled {
        enabled: awe_state.enabled,
    });
    // Varje parameter som eget command:
    handle.send_blocking(EngineCommand::SetAweParameter {
        param: AweParam::RoomShape(/* ... */),
    });
    handle.send_blocking(EngineCommand::SetAweParameter {
        param: AweParam::Material(/* ... */),
    });
    // ... alla parametrar
}

// I save_patch() — GUI-state → AweState:
// AWE-vyns state (i modular_synth) har alla värden.
// Serialiseras direkt till AweState utan att fråga engine.
patch.settings.awe = Some(awe_view_state.to_awe_state());
```

Nuvarande `load_patch()` hanterar `master_volume`, `glide_time` och
`octave_offset` (rad 119-127 i `patch_bridge.rs`). AWE-wiring läggs till
i samma funktion, efter befintlig parameter-laddning.

---

## 5. FDN-extraktion (prerequisite)

Befintlig FDN-implementation lever i `synth_modules/src/effects/reverb.rs`.
AWE i `synth_awe` kan inte importera `synth_modules`. Lösning: extrahera
FDN-kärnan till `synth_dsp`.

### 5.1 Vad som extraheras

Nuvarande FDN har mer komplexitet än bara delay + matrix:
- **Per-kanal one-pole LP-filter** (damping)
- **Modulerade delay-tider** (LFO chorus-effekt i feedback-loopen)
- **Hadamard-matris** (8×8 mixing)
- **Filter state** per kanal

Allt detta måste finnas i `FdnCore`.

**Viktig detalj**: Nuvarande FDN i `reverb.rs` använder **inte**
`DelayLine` från synth_dsp. Den har egna cirkulära `Vec<f32>`-buffertar
per kanal med manuell linjär interpolation (`read_interpolated()`).
FDN-modulation kräver fraktionell läsning — `DelayLine` stödjer
bara heltal. `InterpolatedDelayLine` skulle fungera men matchar inte
nuvarande implementation.

**Beslut**: FdnCore behåller nuvarande mönster med egen `FdnChannel`-struct
per kanal (cirkulär buffer + write_index + interpolation). Detta är en
ren extraktion, inte en omskrivning:

```rust
// Nytt: crates/synth_dsp/src/fdn.rs

/// Per-kanal state: cirkulär buffer med interpolerad läsning.
/// Matchar befintlig FdnChannel i reverb.rs — ren flytt.
pub(crate) struct FdnChannel {
    buffer: Vec<f32>,              // pre-allokerad cirkulär buffer
    write_index: usize,
    delay_samples: usize,          // bas-delay (heltal)
    lowpass_state: FilterState,    // one-pole LP (damping)
    highpass_state: FilterState,   // one-pole HP (low-cut)
    lfo_phase: f32,                // per-kanal LFO fas
    lfo_rate: f32,                 // per-kanal LFO hastighet
}

impl FdnChannel {
    /// Linjär interpolation vid fraktionell delay-position.
    #[inline]
    fn read_interpolated(&self, delay_frac: f32) -> f32;

    /// Beräkna modulerad delay (bas + LFO × mod_depth).
    #[inline]
    fn modulated_delay(&self, diffusion: f32) -> f32;
}

/// FDN-kärna: 8-kanal feedback delay network med Hadamard-matris.
/// Ren DSP utan modul-boilerplate.
pub struct FdnCore {
    channels: [FdnChannel; 8],        // per-kanal state inkl. interpolation
    feedback_matrix: [[f32; 8]; 8],    // Hadamard (hårdkodad)
    feedback_gains: [f32; 8],
    damping_coeffs: [f32; 8],
}

impl FdnCore {
    /// Skapa ny FDN med kanaler pre-allokerade till max_delay_samples.
    pub fn new(max_delay_samples: usize) -> Self;

    /// Processa ett stereo-sample-par. Noll allokeringar.
    /// Intern LFO-modulation + interpolerad läsning per kanal.
    pub fn process_sample(&mut self, left: f32, right: f32,
                          sample_rate_recip: f32) -> (f32, f32);

    /// Uppdatera delay-tider (ändrar bara delay_samples, INTE bufferstorlek).
    pub fn set_delay_times(&mut self, times: &[usize; 8]);

    /// Uppdatera damping (one-pole LP koefficienter per kanal).
    pub fn set_damping(&mut self, coeffs: &[f32; 8]);

    /// Uppdatera feedback gains.
    pub fn set_feedback(&mut self, gains: &[f32; 8]);

    /// Uppdatera LFO modulation (chorus-effekt i delays).
    pub fn set_modulation(&mut self, rate: f32, depths: &[f32; 8]);

    /// Rensa alla buffertar och filter states.
    pub fn clear(&mut self);
}
```

### 5.2 Vad som blir kvar i synth_modules

```rust
// synth_modules/src/effects/reverb.rs:
pub struct FdnReverb {
    core: synth_dsp::FdnCore,  // delegerar all DSP
    // UI-parametrar (room_size, damping, etc.) och param-konvertering
}

impl AudioEffect for FdnReverb {
    fn process(...) {
        // Konvertera UI-parametrar → FdnCore-parametrar
        self.core.process_sample(left, right)
    }
}
```

**Vinst**: Både `FdnReverb` (synth_modules) och `AweEngine` (synth_awe)
kan använda `FdnCore` utan cirkulärt beroende.

---

## 6. GUI: 2D Planritning

### 6.1 AppView och navigation

```rust
pub enum AppView {
    #[default]
    Rack,
    AcousticWorld,  // Ny
}
```

Befintlig toolbar i `egui_backend.rs` är villkorad på `AppView::Rack`.
AWE-vyn har en **separat toolbar-funktion** — inte modifiering av
befintlig rack-toolbar:

```rust
// I egui_backend.rs render loop:
match self.app_view {
    AppView::Rack => {
        self.draw_rack_toolbar(ui);
        self.draw_rack_view(ui);
    }
    AppView::AcousticWorld => {
        self.draw_awe_toolbar(ui);  // Helt separat funktion
        self.draw_awe_view(ui);
    }
}
```

### 6.2 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  AWE: [Shape ▼] [Material ▼] [Presets ▼]         [← Rack]  │
├─────────────────────────────────────────┬───────────────────┤
│                                         │                   │
│           2D Plan View                  │   Parameters      │
│                                         │                   │
│   ┌─────────────────────────────┐       │   Room            │
│   │              ·  ·  ·  ·     │       │   ├ Length  [===]  │
│   │         ·                ·  │       │   ├ Width   [===]  │
│   │       ·    ◉ Source        ·│       │   └ Height  [===]  │
│   │       ·                   · │       │                   │
│   │         ·               ·   │       │   Material        │
│   │           · ▲ Listener·     │       │   ├ Absorb  [===]  │
│   │              ·  ·  ·        │       │   └ Diffuse [===]  │
│   └─────────────────────────────┘       │                   │
│                                         │   Mix             │
│   [Drag source/listener to move]        │   ├ Dry/Wet [===]  │
│                                         │   ├ Early   [===]  │
│                                         │   └ Modes   [===]  │
│                                         │                   │
│                                         │   Impossible      │
│                                         │   ├ FreqWarp[===]  │
│                                         │   ├ Resonate[===]  │
│                                         │   └ Stretch [===]  │
│                                         │                   │
│                                         │   LFO 1-2         │
│                                         │   ├ Rate    [===]  │
│                                         │   ├ Amount  [===]  │
│                                         │   └ Target  [▼]   │
│                                         │                   │
├─────────────────────────────────────────┴───────────────────┤
│  Status: Room 8.0×5.0×3.0m · Concrete · RT60: 1.2s          │
└─────────────────────────────────────────────────────────────┘
```

### 6.3 Visuell feedback

- **Rummets konturer** ritas som tjock linje, proportionellt skalade.
- **Källa** (◉) och **lyssnare** (▲) kan dras med musen.
- **Första reflektionerna** ritas som tunna linjer källa → vägg → lyssnare.
- **Rumsmoder** kan visualiseras som färgade ståendevågor (valfritt).

---

## 7. DSP-detaljer

### 7.1 Image Source Method (ISM) — Tidiga reflektioner

ISM beräknar reflektioner genom att spegla källan i varje vägg:

```
                Vägg
                 │
    Källa ◉      │      ◉' (speglad källa)
           \     │     /
            \    │    /
             \   │   /
              \  │  /
               \ │ /
                \│/
           ▲ Lyssnare

    Reflektionens avstånd = |Lyssnare - Speglad källa|
    Delay = avstånd / 343 m/s
    Gain = 1 / avstånd · (1 - absorption)
```

**Rektangulärt rum, 1:a ordningen**: 6 spegelpunkter (en per vägg).
**2:a ordningen**: ~18 spegelpunkter (vägg → vägg).

```rust
const MAX_EARLY_TAPS: usize = 24;

struct EarlyTap {
    delay_samples: f32,       // fraktionell delay (läses med interpolering)
    gain_left: f32,           // inkl. pan + absorption
    gain_right: f32,
    damping_coeff: f32,       // one-pole LP per tap
    filter_state: f32,
}

struct EarlyReflections {
    taps: [EarlyTap; MAX_EARLY_TAPS],
    active_taps: usize,
    delay_line_left: InterpolatedDelayLine,   // fraktionell delay
    delay_line_right: InterpolatedDelayLine,  // pre-allokerad vid max
}
```

### 7.2 FDN Late Reverb — geometridrivet

`FdnCore` (se §5) med parametrar härledda från geometri:

```
Rumsvolym V = L × W × H (m³)
Rumsyta   S = 2(LW + LH + WH) (m²)
Medelabsorption ᾱ = Σ(α_i · S_i) / S

Sabines formel: RT60 = 0.161 · V / (ᾱ · S)

FDN delay-tider: proportionella mot rummets dimensioner
  d₁ = 2L/c, d₂ = 2W/c, d₃ = 2H/c, d₄ = √(L²+W²)/c, ...

FDN damping: härledd från material absorption per band
FDN feedback: härledd från RT60
FDN modulation: intern LFO för chorus-effekt (ger densitet)
```

### 7.3 Room Mode Bank

Axiella moder i ett rektangulärt rum:

```
f_x(n) = n · c / (2·L)    (längdmoder)
f_y(n) = n · c / (2·W)    (breddmoder)
f_z(n) = n · c / (2·H)    (höjdmoder)
```

Implementeras som comb-filter bank (3-6 filter):

```rust
struct RoomModeBank {
    modes: [CombFilter; 6],
    active_modes: usize,
    amount: f32,
}

struct CombFilter {
    delay_line: DelayLine,    // pre-allokerad vid max-storlek
    feedback: f32,
    damping: f32,
}
```

### 7.4 Spatializer

Appliceras **enbart på wet-signalen**. Dry passerar ospatialiserad.

```rust
struct Spatializer {
    itd_delay_left: InterpolatedDelayLine,   // fraktionell ITD
    itd_delay_right: InterpolatedDelayLine,
    gain_left: f32,
    gain_right: f32,
    shadow_filter: f32,
}
```

### 7.5 AWE-interna LFO:er

Kodbasens mod matrix (`ModDestination`) stödjer enbart **per-röst** mål.
AWE har **egna interna LFO:er** som modulerar AWE-parametrar direkt:

```rust
struct AweLfo {
    phase: Phase,
    rate: Hertz,          // 0.01 - 20 Hz
    amount: f32,
    target: AweLfoTarget,
}

enum AweLfoTarget {
    RoomLength, RoomWidth,
    SourceX, SourceY,
    ListenerX, ListenerY,
    DryWet, FreqWarp,
}
```

**Kontroll-rate, inte sample-rate.**

LFO:erna körs **inte per sample**. Geometriberäkningar (ISM tap-positioner,
rumsmoder, FDN delay-tider) är för dyra för per-sample uppdatering.

Istället: **kontroll-rate uppdatering** i `AweEngine::process()`:

1. LFO-faser inkrementeras per block (en gång per `process()`-anrop)
2. Modulerade parametrar beräknas en gång per block → nya mål-värden
3. DSP-parametrar (delay-tider, gains) rampas smootht mot mål (~5ms ramp)
4. ISM tap-omberäkning sker enbart om modulerade parametrar ändrats

```rust
// I AweEngine::process():
fn process(&mut self, buffer: &mut [f32], sample_rate: f32) {
    let num_samples = buffer.len() / 2;

    // 1. Kontroll-rate: uppdatera LFO:er och beräkna nya mål
    self.update_lfos(num_samples, sample_rate);
    self.update_geometry_if_changed();  // ISM, modes — bara vid ändring

    // 2. Per-sample DSP med rampade parametrar
    for i in 0..num_samples {
        self.ramp_parameters();  // Smooth 5ms ramp mot mål
        // ... ISM, FDN, modes, spatializer per sample
    }
}
```

Typisk blockstorlek: 256–1024 samples. Kontroll-rate uppdatering kostar
~1% av per-sample, eliminerar CPU-spikar vid snabb LFO-modulation.

---

## 8. Implementeringsfaser

### Fas 0: Crate + infrastruktur

**Mål**: `synth_awe`-crate existerar, AweEngine processar pass-through,
har en dedikerad vy, sparas och laddas. FdnCore extraherad.

**Prerequisite**: Extrahera `FdnCore` till `synth_dsp` (se §5).

**Acceptance criteria**:
- [ ] `synth_awe` crate skapad med Cargo.toml (deps: synth_core, synth_dsp)
- [ ] `FdnCore` extraherad till `synth_dsp/src/fdn.rs`, FdnReverb delegerar
- [ ] `AweEngine::new()` pre-allokerar alla delay-linjer (6 MB @ 96kHz)
- [ ] `AweEngine::process()` pass-through (output = input)
- [ ] `EngineCommand::SetAweParameter` och `SetAweEnabled` fungerar
- [ ] `AppView::AcousticWorld` med separat toolbar
- [ ] AWE-vy renderas (placeholder med rum-parametrar)
- [ ] `AweState` sparas/laddas via explicit wiring i `patch_bridge.rs`
- [ ] Visualizers processar **efter** AWE i `SynthEngine::process()`
- [ ] Alla 4 obligatoriska kontroller passerar (build, clippy, test, fmt)

**Filer**:

| Fil | Ändring |
|-----|---------|
| `synth_awe/Cargo.toml` | **Ny crate.** deps: synth_core, synth_dsp |
| `synth_awe/src/lib.rs` | Pub exports |
| `synth_awe/src/awe_engine.rs` | AweEngine (pass-through) |
| `synth_awe/src/params.rs` | AweParam enum |
| `synth_awe/src/room.rs` | RoomShape, Material (datastrukturer) |
| `Cargo.toml` (workspace) | Lägg till synth_awe member |
| `synth_dsp/src/fdn.rs` | **Ny.** FdnCore extraherad |
| `synth_dsp/src/lib.rs` | `pub mod fdn;` |
| `synth_modules/src/effects/reverb.rs` | FdnReverb delegerar till FdnCore |
| `synth_engine/Cargo.toml` | dep: synth_awe |
| `synth_engine/src/commands.rs` | SetAweParameter, SetAweEnabled |
| `synth_engine/src/synth_engine.rs` | AWE-steg + command-hantering + visualizer post-AWE |
| `synth_engine/src/effect_chain.rs` | Ny `process_visualizers()`, skip viz i `process()` |
| `modular_synth/Cargo.toml` | dep: synth_awe |
| `modular_synth/src/gui/app/state.rs` | AppView::AcousticWorld |
| `modular_synth/src/gui/awe_view.rs` | **Ny.** Placeholder-vy |
| `modular_synth/src/gui/egui_backend.rs` | View routing, AWE-toolbar |
| `modular_synth/src/gui/patch_bridge.rs` | AweState load/save |

**Uppskattning**: ~550 rader, 18 filer.

(Inkluderar FDN-extraktion med interpolerad per-kanal läsning,
EffectChain-split för visualizers, och AweSnapshot batch-struct.
Mer än initiala 400-uppskattningen pga dessa tre refaktoreringar.)

---

### Fas 1: Parametriskt rum

**Mål**: Spelbar rumseffekt med ISM + FDN + moder + 2D-vy + LFO:er.

**Acceptance criteria**:
- [ ] Rektangulärt rum med ISM tidiga reflektioner (6 taps, InterpolatedDelayLine)
- [ ] FDN-reverb med geometridrivna parametrar (via FdnCore)
- [ ] 3 axiella rumsmoder som comb-filter
- [ ] 2D planritning med dragbar källa + lyssnare
- [ ] Spatializer enbart på wet-signal
- [ ] 6 material-presets
- [ ] Dry/wet, early/late balance, modes amount
- [ ] 2 interna LFO:er med target-väljare
- [ ] Parametrar uppdateras i realtid
- [ ] Alla kontroller passerar

**Nya/ändrade filer i synth_awe**:

| Fil | Innehåll | ~Rader |
|-----|----------|--------|
| `synth_awe/src/early_reflections.rs` | ISM + InterpolatedDelayLine | ~120 |
| `synth_awe/src/room_modes.rs` | Comb-filter bank | ~60 |
| `synth_awe/src/spatializer.rs` | ITD + ILD | ~50 |
| `synth_awe/src/lfo.rs` | AWE-interna LFO:er | ~40 |
| `synth_awe/src/awe_engine.rs` | Komplett DSP-pipeline (uppdatera) | ~200 |
| `synth_awe/src/params.rs` | Full parameteruppsättning (uppdatera) | ~80 |
| `modular_synth/src/gui/awe_view.rs` | 2D-vy med interaktion (uppdatera) | ~200 |

**Uppskattning**: ~500 rader nya, ~100 ändrade.

---

### Fas 2: Avancerad geometri & kreativa features

**Mål**: Icke-rektangulära rum, "omöjliga" parametrar, fler LFO:er.

**Acceptance criteria**:
- [ ] Cylinder-rum (pipeline/tunnel mode)
- [ ] L-format rum
- [ ] Freq Warp, Resonance Boost, Tail Stretch
- [ ] 4 interna LFO:er med utökade targets
- [ ] Akustisk portal

**Uppskattning**: ~300 rader.

---

### Fas 3: Per-röst spatialisering (framtid)

- Note-to-position mapping
- Per-röst ISM
- Valfri 3D-vy

Kräver arkitekturförändring — bör inte påbörjas förrän Fas 1 är stabil.

---

## 9. Risker & mitigeringar

| Risk | Konsekvens | Mitigering |
|------|-----------|------------|
| FDN-parametrar ger click vid snabb ändring | Artefakt | Smooth-rampa ~5ms per block |
| Comb-filter resonans blåser upp | Feedback | Intern limiter + max feedback 0.99 |
| ISM 2:a ordningen för CPU-tung | Stuttering | Dynamisk ordning baserat på rumsstorlek (§3.1) |
| "Omöjliga rum" låter illa | Oanvändbart | Bra defaults, subtila ranges, presets |
| FDN-extraktion bryter FdnReverb | Regression | Befintliga tester passerar |
| Pre-allokerat minne ~6MB oanvänt | Minnesslöseri | Försumbart |
| DelayLine::resize anropas av misstag | RT-brott | FdnCore har egna buffertar, aldrig resize |
| Fraktionell delay ger artefakter | Tick/click | Linjär interpolation i FdnChannel + InterpolatedDelayLine |
| Visualizers visar pre-AWE signal | Förvirrande | Delad EffectChain: process() vs process_visualizers() |
| Ringbuffer-overflow vid drag/automation | Desync | SetAweState batch-kommando, 1 per frame |
| Sample rate ändras runtime | Felaktiga delay-tider | AWE tar sample_rate som process()-param, cachear (§3.3) |
| LFO modulerar geometri per-sample | CPU-spike | Kontroll-rate uppdatering per block + 5ms ramp (§7.5) |
| 200m pipeline + 2:a ordningen > 1.0s | Klampade reflektioner | Dynamisk ISM-ordning per preset (§3.1) |

---

## 10. Presets

| Preset | Shape | Material | Storlek | Karaktär |
|--------|-------|----------|---------|----------|
| Studio | Box | Wood | 6×4×3m | Tight, kontrollerad |
| Concert Hall | Box | Concrete | 30×20×12m | Stor, öppen |
| Steel Pipeline | Cylinder | Metal | r=1m, L=200m | Metallic, lång svans |
| Bathroom | Box | Tile | 3×2×2.5m | Bright, fluttery |
| Cave | L-Shape | Concrete | 15×8+10×6, H=5m | Diffus, mystisk |
| The Void | Box | Custom | 100×100×100m | Enormt, mörkt |

---

## 11. Sammanfattning

```
Fas 0:  ~550 rader    synth_awe crate, FDN-extraktion, EffectChain-split,
                       infrastruktur, AppView, persistence
Fas 1:  ~500 rader    ISM + FDN + moder + spatializer + 2D-vy + LFO:er
Fas 2:  ~300 rader    Avancerade former, omöjliga rum, fler LFO:er
Fas 3:  Framtida      Per-röst spatialisering, ev. 3D-vy
        ─────────
Totalt: ~1350 rader (Fas 0-2), noll nya externa dependencies
```

### Nyckeldesignbeslut

| Beslut | Val | Motivering |
|--------|-----|-----------|
| Crate | Egen `synth_awe` | Isolerad testning, noll cirkulära deps |
| Param-routing | Ren AweParam + AweSnapshot batch | Undvik dead code, undvik ringbuffer-stress |
| Placering | Eget engine-steg | Inte slot i master_effects |
| Livscykel | Fixed global med bypass | Inget add/remove |
| Process-signatur | `process(&mut [f32], f32)` interleaved | Matchar AudioBuffer, ingen kopiering |
| Modulation | AWE-interna LFO:er, kontroll-rate | Per block, ej per sample — undvik CPU-spike |
| RT-säkerhet | Pre-allokera allt, 1.0s max delay | Dynamisk ISM-ordning per rumsstorlek |
| ISM delay-typ | InterpolatedDelayLine | Fraktionella delay-tider krävs |
| FDN delay-typ | Egna FdnChannel med linjär interpolation | Matchar befintlig reverb — ren extraktion |
| FDN | FdnCore i synth_dsp | Undvik cirkulärt beroende |
| Spatialisering | Enbart wet-signal | Bevara originalstereobild |
| Visualizers | Delad EffectChain: process() + process_visualizers() | Post-AWE utan strukturändring |
| Persistence | GUI-state canonical, SetAweState batch | Inget AWE i EngineState |
| Sample rate | Parameter i process(), cachad omberäkning | Inga nya hooks behövs |
| GUI | Separat AWE-toolbar | Rör inte befintlig rack-toolbar |
