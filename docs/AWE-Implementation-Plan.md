# Implementation Plan: The Acoustic World Engine (AWE)

## 1. Vision

AWE ger syntetiserade ljud ett fysiskt **rum att existera i**. Till skillnad
från en vanlig convolution reverb är AWE en **realtids-rumsmotor** där alla
parametrar — rummets dimensioner, material, käll- och lyssnarposition — kan
moduleras kontinuerligt via LFO:er, envelopes och mod matrix.

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

### 2.1 Signalflöde

```
Stereo In (master bus)
│
▼
┌─────────────────────────────────────────────────────────┐
│                    AWE Master Effect                     │
│                                                          │
│   ┌────────────────────┐                                 │
│   │  Early Reflections │  Image Source Method (ISM)      │
│   │  (tapped delay     │  1:a + 2:a ordningen            │
│   │   + per-tap filter)│  Delay-tider från geometri      │
│   │                    │  Gain/filter från material       │
│   └────────┬───────────┘                                 │
│            │                                             │
│   ┌────────▼───────────┐                                 │
│   │  FDN Late Reverb   │  8-kanal Hadamard FDN           │
│   │  (befintlig impl.) │  Delay-tider ∝ rummets volym    │
│   │                    │  Damping från material           │
│   │                    │  LFO-modulerade delays           │
│   └────────┬───────────┘                                 │
│            │                                             │
│   ┌────────▼───────────┐                                 │
│   │  Room Mode Bank    │  3-6 comb-filter                │
│   │                    │  f_n = c/(2·L), c/(2·W), c/(2·H)│
│   │                    │  Ger rummets "ton"               │
│   └────────┬───────────┘                                 │
│            │                                             │
│   ┌────────▼───────────┐                                 │
│   │  Spatializer       │  Lyssnare-position →             │
│   │  (ITD + ILD)       │  Inter-aural time delay          │
│   │                    │  + frekvensber. level diff.       │
│   └────────┬───────────┘                                 │
│            │                                             │
│   Mix: Dry/Wet · Early/Late · Modes Amount               │
└────────────┼─────────────────────────────────────────────┘
             ▼
        Stereo Out
```

### 2.2 Varför inte ray tracing + IR?

Den ursprungliga planen använde stochastisk ray tracing → IR → convolver.
Denna approach har tre kritiska problem:

1. **RT-säkerhet**: IR-swap kräver antingen lock-free allokering eller
   dubbel-convolver med crossfade. Nuvarande `PartitionedConvolver`
   allokerar vid rebuild — bryter mot RT-kravet (CLAUDE.md, ARCHITECTURE.md).

2. **Ingen modulation**: Statisk IR innebär att alla parametrar fryser
   tills nästa beräkning. "Mouse up only" eliminerar kreativ potential.

3. **Oproportionell komplexitet**: CSG + ray tracing + IR-generering +
   crossfade-convolver ≈ 2000+ rader greenfield-kod med noll befintliga
   dependencies. Resultatet: en fancy reverb.

**Den reviderade arkitekturen** (ISM + FDN + comb bank) ger:
- Alla parametrar modulerbara i realtid
- Noll allokeringar i audio-tråden
- Återanvänder befintlig FDN-implementation (v0.120.0)
- Inga nya dependencies
- ~900 rader totalt

### 2.3 Plats i kodbasen

```
crates/
├── synth_core/src/params/
│   └── awe.rs                  # AweParam enum (alla AWE-parametrar)
│
├── synth_dsp/src/
│   └── room_acoustics.rs       # ISM, room modes, spatializer (ren DSP)
│
├── synth_engine/src/
│   ├── awe_engine.rs           # AweEngine struct (AudioEffect impl)
│   └── commands.rs             # EffectType::AcousticWorld
│
└── modular_synth/src/
    ├── gui/
    │   └── awe_view.rs         # 2D planritning + kontroller
    └── patch.rs                # PatchSettings::awe: Option<AweState>
```

### 2.4 Integration med befintlig master-bus

```
SynthEngine::process()
│
├── Instruments → voice_left/right → effect_chain.process()
│                                     (Delay, Reverb, Chorus, etc.)
│
├── master_left/right (summa av alla instrument)
│
├── Global visualizers (Oscilloscope, LevelMeter, SpectrumAnalyzer)
│
└── ★ AWE master effect (ny slot, efter global effects) ★
    │
    └── Final stereo output → audio backend
```

AWE sitter som **sista steget** före audio-output. Det är *inte* en del av
per-instrument `EffectChain` — det är en global master-effekt med egen state.

---

## 3. Datamodell

### 3.1 Rumsgeometri

```rust
/// Rum-form som styr ISM-beräkningar och FDN-parametrar.
pub enum RoomShape {
    /// Rektangulärt rum (L × W × H meter).
    /// Enklast att beräkna ISM för (spegelpunkter).
    Box { length: f32, width: f32, height: f32 },

    /// Cylindriskt rum (radie × höjd).
    /// Pipeline/tunnel-mode vid hög aspect ratio.
    Cylinder { radius: f32, length: f32 },

    /// L-format rum (två sammankopplade rektanglar).
    /// Ger intressanta tidiga reflektioner med "hörn".
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
    pub fn axial_modes(&self) -> Vec<f32>;
}
```

### 3.2 Material

```rust
/// Akustiskt material med frekvensberoende absorption.
pub struct Material {
    pub name: &'static str,
    /// Absorption per frekvensband (0.0 = total reflektion, 1.0 = total absorption).
    pub absorption_low: f32,   // < 500 Hz
    pub absorption_mid: f32,   // 500 Hz – 4 kHz
    pub absorption_high: f32,  // > 4 kHz
    /// Diffusion (0.0 = spegelreflektion, 1.0 = helt diffus).
    pub diffusion: f32,
}

// Presets
const CONCRETE:  Material = Material { absorption_low: 0.01, absorption_mid: 0.02, absorption_high: 0.02, diffusion: 0.1, .. };
const WOOD:      Material = Material { absorption_low: 0.15, absorption_mid: 0.10, absorption_high: 0.07, diffusion: 0.3, .. };
const GLASS:     Material = Material { absorption_low: 0.04, absorption_mid: 0.03, absorption_high: 0.02, diffusion: 0.05, .. };
const METAL:     Material = Material { absorption_low: 0.01, absorption_mid: 0.01, absorption_high: 0.02, diffusion: 0.05, .. };
const FABRIC:    Material = Material { absorption_low: 0.10, absorption_mid: 0.40, absorption_high: 0.70, diffusion: 0.8, .. };
const TILE:      Material = Material { absorption_low: 0.02, absorption_mid: 0.02, absorption_high: 0.03, diffusion: 0.1, .. };
```

### 3.3 "Omöjliga rum"-parametrar

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

### 3.4 Persistence (patch-format)

```rust
// I PatchSettings:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub awe: Option<AweState>,

#[derive(Serialize, Deserialize)]
pub struct AweState {
    pub enabled: bool,
    pub room_shape: RoomShapeState,  // enum med dimensioner
    pub material: String,            // preset-namn eller "custom"
    pub source_pos: (f32, f32),      // x, y i rum-koordinater (0.0-1.0)
    pub listener_pos: (f32, f32),
    pub dry_wet: f32,
    pub early_late_balance: f32,
    pub modes_amount: f32,
    pub impossible: ImpossibleState,
}
```

---

## 4. GUI: 2D Planritning

### 4.1 Varför 2D?

- **Trivial i egui**: Rektanglar, cirklar, drag-and-drop — allt finns.
- **Visuellt tillräcklig**: Visar käll-/lyssnar-positioner, rumsform, väggar.
- **Konsekvent**: Matchar det befintliga node-baserade UI:t.
- **3D kan läggas till senare** utan att ändra datamodell eller DSP.

En 3D-vy i egui kräver custom rendering pipeline, kameramatematik och
hit-testing — månader av arbete för marginellt mervärde i v1.

### 4.2 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  AWE Toolbar: [Shape ▼] [Material ▼] [Presets ▼] [← Rack]  │
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
├─────────────────────────────────────────┴───────────────────┤
│  Status: Room 8.0×5.0×3.0m · Concrete · RT60: 1.2s          │
└─────────────────────────────────────────────────────────────┘
```

### 4.3 Visuell feedback

- **Rummets konturer** ritas som tjock linje. Proportionerna matchar
  verkliga dimensioner (skalas till tillgängligt utrymme).
- **Källa** (◉) och **lyssnare** (▲) kan dras med musen.
- **Första reflektionerna** ritas som tunna linjer från källa → vägg →
  lyssnare (uppdateras i realtid vid drag). Ger visuell feedback på ISM.
- **Rumsmoder** kan visualiseras som färgade ståendevågor (valfritt).

---

## 5. DSP-detaljer

### 5.1 Image Source Method (ISM) — Tidiga reflektioner

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

Varje spegelpunkt ger en **tapped delay** med:
- Delay-tid (avstånd / ljudhastighet)
- Gain (1/r avståndsdämpning × material absorption)
- One-pole filter (materialberoende HF-dämpning)

Implementation: pre-allokerad array av `EarlyTap`:

```rust
const MAX_EARLY_TAPS: usize = 24;

struct EarlyTap {
    delay_samples: f32,       // fraktionell delay
    gain_left: f32,           // inkl. pan + absorption
    gain_right: f32,
    damping_coeff: f32,       // one-pole LP per tap
    filter_state: f32,        // filter state (RT-safe)
}

struct EarlyReflections {
    taps: [EarlyTap; MAX_EARLY_TAPS],
    active_taps: usize,
    delay_line_left: DelayLine,   // befintlig synth_dsp::DelayLine
    delay_line_right: DelayLine,
}
```

### 5.2 FDN Late Reverb — geometridrivet

Befintlig 8-kanal FDN (v0.120.0) med parametrar härledda från geometri:

```
Rumsvolym V = L × W × H (m³)
Rumsyta   S = 2(LW + LH + WH) (m²)
Medelabsorption ᾱ = Σ(α_i · S_i) / S

Sabines formel: RT60 = 0.161 · V / (ᾱ · S)

FDN delay-tider: proportionella mot rummets dimensioner
  d₁ = 2L/c, d₂ = 2W/c, d₃ = 2H/c, d₄ = √(L²+W²)/c, ...

FDN damping: härledd från material absorption per band
FDN feedback: härledd från RT60
```

### 5.3 Room Mode Bank

Axiella moder i ett rektangulärt rum:

```
f_x(n) = n · c / (2·L)    (längdmoder)
f_y(n) = n · c / (2·W)    (breddmoder)
f_z(n) = n · c / (2·H)    (höjdmoder)

Exempel: L=8m → f₁ = 343/(2·8) = 21.4 Hz
         W=5m → f₁ = 343/(2·5) = 34.3 Hz
         H=3m → f₁ = 343/(2·3) = 57.2 Hz
```

Implementeras som comb-filter bank (3-6 filter), en per grundmod:

```rust
struct RoomModeBank {
    modes: [CombFilter; 6],
    active_modes: usize,
    amount: f32,              // 0.0 = inga moder, 1.0 = full
}

struct CombFilter {
    delay_line: DelayLine,    // delay = 1/f_mode samples
    feedback: f32,            // ~0.95-0.99
    damping: f32,             // one-pole LP i feedback
}
```

### 5.4 Spatializer

Källa och lyssnare har 2D-positioner i rummet. Skillnaden i avstånd
till vänster/höger öra ger:

- **ITD** (Interaural Time Difference): ~0-0.7 ms delay mellan kanalerna
- **ILD** (Interaural Level Difference): ~0-6 dB gain-skillnad + HF-skugga

```rust
struct Spatializer {
    itd_delay_left: InterpolatedDelayLine,
    itd_delay_right: InterpolatedDelayLine,
    gain_left: f32,
    gain_right: f32,
    shadow_filter: f32,  // one-pole LP för HF-skugga
}
```

---

## 6. Implementeringsfaser

### Fas 0: RT-plumbing & infrastruktur

**Mål**: En tom AWE-effekt existerar i master-kedjan, har en dedikerad vy,
sparas och laddas. Ljud passerar oförändrat.

**Acceptance criteria**:
- [ ] `AppView::AcousticWorld` fungerar med toolbar-navigation
- [ ] AWE-vy renderas (tom, med "Coming soon" placeholder)
- [ ] `AweState` sparas i patch-fil och laddas tillbaka
- [ ] `AweEngine` processar audio (pass-through)
- [ ] Alla 4 obligatoriska kontroller passerar (build, clippy, test, fmt)

**Ändringar**:

| Fil | Ändring |
|-----|---------|
| `synth_core/src/params/awe.rs` | Ny. `AweParam` enum med alla parametrar |
| `synth_core/src/params/mod.rs` | `ModuleType::AcousticWorld`, `Param::Awe(AweParam)` |
| `synth_engine/src/awe_engine.rs` | Ny. `AweEngine` struct, `AudioEffect` impl (pass-through) |
| `synth_engine/src/commands.rs` | `EffectType::AcousticWorld` |
| `synth_engine/src/synth_engine.rs` | AWE-slot efter master effects |
| `modular_synth/src/gui/app/state.rs` | `AppView::AcousticWorld` |
| `modular_synth/src/gui/awe_view.rs` | Ny. Placeholder-vy |
| `modular_synth/src/gui/egui_backend.rs` | Toolbar-knapp, view routing |
| `modular_synth/src/patch.rs` | `PatchSettings::awe: Option<AweState>` |

**Uppskattning**: ~200 rader, 9 filer.

---

### Fas 1: Parametriskt rum

**Mål**: Spelbar rumseffekt med tidiga reflektioner, reverb-svans och
rumsmoder. 2D-vy med drag-and-drop.

**Acceptance criteria**:
- [ ] Rektangulärt rum med ISM tidiga reflektioner (6 taps)
- [ ] FDN-reverb med geometridrivna parametrar
- [ ] 3 axiella rumsmoder som comb-filter
- [ ] 2D planritning med dragbar källa + lyssnare
- [ ] 6 material-presets
- [ ] Dry/wet, early/late balance, modes amount
- [ ] Parametrar uppdateras i realtid (ingen IR-rebuild)
- [ ] Alla kontroller passerar

**Nya filer**:

| Fil | Innehåll | ~Rader |
|-----|----------|--------|
| `synth_dsp/src/room_acoustics.rs` | ISM, EarlyReflections, RoomModeBank, Spatializer | ~250 |
| `synth_core/src/params/awe.rs` | Full parameteruppsättning | ~80 |
| `synth_engine/src/awe_engine.rs` | Komplett DSP-pipeline | ~200 |
| `modular_synth/src/gui/awe_view.rs` | 2D-vy med interaktion | ~150 |

**Ändrade filer**: ~6 (samma som Fas 0 + FDN-integration)

**Uppskattning**: ~400 rader nya, ~100 ändrade.

---

### Fas 2: Avancerad geometri & kreativa features

**Mål**: Icke-rektangulära rum, "omöjliga" parametrar, mod matrix.

**Acceptance criteria**:
- [ ] Cylinder-rum (pipeline/tunnel mode)
- [ ] L-format rum
- [ ] Freq Warp: frekvensberoende rumsstorlek
- [ ] Resonance Boost: negativ absorption med intern limiter
- [ ] Tail Stretch: oberoende reverb-tid
- [ ] AWE-parametrar som ModDestination i mod matrix
- [ ] Akustisk portal (öppning till andra rum med andra material)

**Uppskattning**: ~300 rader.

---

### Fas 3: Per-röst spatialisering (framtid)

**Mål**: Varje polyfon röst placeras på unik position i rummet.

- Note-to-position mapping (tonhöjd → x-position, velocity → y)
- Per-röst ISM-beräkning (kräver AWE per instrument, inte bara master)
- Valfri 3D-vy

Detta kräver arkitekturförändring (AWE som per-instrument effekt) och
bör inte påbörjas förrän Fas 1 är stabil.

---

## 7. Risker & mitigeringar

| Risk | Konsekvens | Mitigering |
|------|-----------|------------|
| FDN-parametrar ger click vid snabb ändring | Hörbart artefakt | Smooth-rampa alla parametrar (~5ms) |
| Comb-filter resonans blåser upp | Distorsion/feedback | Intern limiter + max feedback 0.99 |
| ISM 2:a ordningen för CPU-tung | Stuttering | Begränsa till 1:a ordningen dynamiskt |
| "Omöjliga rum" låter illa | Oanvändbart | Bra defaults, subtila ranges, presets |
| 2D-vy otillräcklig | Användare vill ha 3D | Datamodellen är 3D-redo, vy kan bytas |

---

## 8. Presets

Presets genererar en komplett `AweState` med ett klick:

| Preset | Shape | Material | Storlek | Karaktär |
|--------|-------|----------|---------|----------|
| Studio | Box | Wood | 6×4×3m | Tight, kontrollerad |
| Concert Hall | Box | Concrete | 30×20×12m | Stor, öppen |
| Steel Pipeline | Cylinder | Metal | r=1m, L=200m | Metallic, lång svans |
| Bathroom | Box | Tile | 3×2×2.5m | Bright, fluttery |
| Cave | L-Shape | Concrete | 15×8+10×6, H=5m | Diffus, mystisk |
| The Void | Box | Custom | 100×100×100m | Enormt, mörkt |

---

## 9. Sammanfattning

```
Fas 0:  ~200 rader    Infrastruktur, pass-through, vy-routing, persistence
Fas 1:  ~400 rader    Spelbar rumseffekt med ISM + FDN + moder + 2D-vy
Fas 2:  ~300 rader    Avancerade former, omöjliga rum, mod matrix
Fas 3:  Framtida      Per-röst spatialisering, ev. 3D-vy
        ─────────
Totalt: ~900 rader (Fas 0-2), noll nya dependencies
```

Hela arkitekturen bygger på **realtids-DSP** istället för offline
IR-generering + convolution. Detta ger modulerbarhet, RT-säkerhet och
drastiskt lägre komplexitet — samtidigt som det akustiska resultatet
kan vara *bättre* än ray-traced convolution tack vare analytiska
rumsmoder och "omöjliga" parametrar.
