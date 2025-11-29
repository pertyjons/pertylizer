# Instruktion: Nya Moduler och Förbättringar för Modular Synth

## Projektkontext

**Version:** 0.13.2  
**Språk:** Rust 2024 edition  
**Arkitektur:** Modulär synthesizer med `VoiceModule` trait för per-röst moduler och `EffectModule` trait för globala effekter.

Alla moduler implementerar:
- `Describable` trait för UI-introspection och metadata
- Typsäkra parametrar via `TypedParam`/`TypedValue` enums
- `AudioBuffer` för sample-data

---

## Del 1: Förbättringar av Befintliga Moduler

### 1.1 Through-Zero FM i Oscillator

**Fil:** `src/modules/oscillator.rs`

**Bakgrund:** Linear FM finns men Through-Zero FM ger stabilare sidband eftersom fasen kan gå bakåt genom noll.

**Implementation:**

```rust
// Utöka FmMode enum
pub enum FmMode {
    Exponential,
    Linear,
    ThroughZero,  // NYTT
}

// I generate_sample(), lägg till:
FmMode::ThroughZero => {
    // Through-zero: frekvensen kan bli negativ
    // Detta ger symmetriska sidband (DX7-stil)
    let instant_freq = base_freq + freq_mod * base_freq * self.tz_fm_index;
    // INGEN clamp - tillåt negativ frekvens
    instant_freq
}

// Fasuppdatering måste hantera negativ frekvens:
self.phase += freq / self.sample_rate;
// Använd rem_euclid för korrekt wrap-around även vid negativa värden
self.phase = self.phase.rem_euclid(1.0);
```

**Ny parameter:**
- `TzFmIndex` (0.0-10.0) - Modulationsdjup för TZ-FM

---

### 1.2 Brown/Red Noise i Oscillator

**Fil:** `src/modules/oscillator.rs`

**Bakgrund:** Pink noise (-3dB/oktav) finns. Brown noise (-6dB/oktav) är ännu mörkare och användbart för vind, åska, rumble.

**Implementation:**

```rust
// Lägg till i Waveform enum (i typed_params.rs):
BrownNoise,

// State i Oscillator struct:
brown_state: f32,

// I generate_sample():
Waveform::BrownNoise => {
    // Brown noise: integrerad white noise (random walk)
    let white = self.white_noise();
    // Läckande integrator för att förhindra DC-drift
    self.brown_state = self.brown_state * 0.998 + white * 0.02;
    // Skala för ungefär samma RMS som white noise
    self.brown_state * 3.5
}

// I reset():
self.brown_state = 0.0;
```

---

### 1.3 Velocity Curves i Envelope

**Fil:** `src/modules/envelope.rs`

**Bakgrund:** Velocity sensitivity är linjär. Musiker förväntar sig olika response-kurvor.

**Implementation:**

```rust
// Ny enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VelocityCurve {
    #[default]
    Linear,
    Exponential,  // Mjukare vid låg velocity
    Logarithmic,  // Hårdare vid låg velocity  
    SCurve,       // Mjuk i båda ändar
    Fixed,        // Ignorera velocity helt
}

// Ny parameter i struct:
velocity_curve: VelocityCurve,

// Ersätt velocity_scale-beräkningen:
fn apply_velocity_curve(&self, velocity: f32) -> f32 {
    if self.velocity_sensitivity == 0.0 {
        return 1.0;
    }
    
    let curved = match self.velocity_curve {
        VelocityCurve::Linear => velocity,
        VelocityCurve::Exponential => velocity * velocity,
        VelocityCurve::Logarithmic => velocity.sqrt(),
        VelocityCurve::SCurve => {
            // Smoothstep
            velocity * velocity * (3.0 - 2.0 * velocity)
        }
        VelocityCurve::Fixed => 1.0,
    };
    
    1.0 - self.velocity_sensitivity * (1.0 - curved)
}
```

---

### 1.4 Velocity till Attack Time

**Fil:** `src/modules/envelope.rs`

**Bakgrund:** Hårdare anslag bör kunna ge snabbare attack för mer expressivt spel.

**Implementation:**

```rust
// Ny parameter:
velocity_to_attack: f32,  // -1.0 till +1.0

// I process_sample(), modifiera attack-beräkningen:
EnvelopeStage::Attack => {
    // Skala attack-tid baserat på velocity
    // Negativt värde = snabbare attack vid hårt anslag
    let vel_scale = 1.0 - self.velocity_to_attack * self.velocity;
    let effective_attack = (self.attack * vel_scale).max(0.001);
    
    // Resten av attack-logiken använder effective_attack...
}
```

---

### 1.5 PolyBLEP vid Hard Sync

**Fil:** `src/modules/oscillator.rs`

**Bakgrund:** Sync finns men fas-reset sker abrupt vilket ger aliasing.

**Implementation:**

```rust
// I process(), vid sync detection:
if let Some(sync) = sync_input {
    let sync_val = sync[i];
    if sync_val > 0.5 && prev_sync <= 0.5 {
        // Beräkna exakt var i samplet övergången skedde
        let crossing = (0.5 - prev_sync) / (sync_val - prev_sync);
        
        // Applicera PolyBLEP-korrigering vid diskontinuiteten
        // Detta minskar aliasing vid sync-reset
        let blep = self.poly_blep(crossing, dt);
        
        self.phase = crossing * dt;  // Partiell fas-reset
        
        // Lägg till blep-korrigering till output
        // (implementation beror på vågform)
    }
    prev_sync = sync_val;
}
```

---

## Del 2: Nya Utility-Moduler

### 2.1 Ring Modulator

**Ny fil:** `src/modules/ring_mod.rs`

**Beskrivning:** Multiplicerar två signaler för metalliska, klockliknande ljud.

```rust
pub struct RingModulator {
    mix: f32,  // Dry/wet
    output_buffer: AudioBuffer,
}

impl RingModulator {
    pub fn new() -> Self {
        Self {
            mix: 1.0,
            output_buffer: AudioBuffer::new(256),
        }
    }
}

impl Describable for RingModulator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("ring_mod", "Ring Mod")
            .description("Ring modulator - multiplies two signals")
            .category(ModuleCategory::Utility)
            .tag("ring")
            .tag("modulation")
            .parameter(
                ParameterDescriptor::float(TypedParam::RingMod(RingModParam::Mix), "Mix")
                    .range(0.0, 1.0)
                    .default(1.0)
            )
            .port(PortDescriptor::audio_input("carrier", "Carrier"))
            .port(PortDescriptor::audio_input("modulator", "Mod"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl VoiceModule for RingModulator {
    fn process(&mut self, inputs: &HashMap<String, &AudioBuffer>, 
               outputs: &mut HashMap<String, AudioBuffer>, context: &ProcessContext) {
        self.output_buffer.resize(context.samples);
        
        let carrier = inputs.get("carrier");
        let modulator = inputs.get("modulator");
        
        for i in 0..context.samples {
            let c = carrier.map(|b| b[i]).unwrap_or(0.0);
            let m = modulator.map(|b| b[i]).unwrap_or(1.0);
            
            let ring = c * m;
            self.output_buffer[i] = c * (1.0 - self.mix) + ring * self.mix;
        }
        
        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }
    // ... övriga trait-metoder
}
```

---

### 2.2 Sample & Hold

**Ny fil:** `src/modules/sample_hold.rs`

**Beskrivning:** Samplar input vid trigger och håller värdet. Klassisk för slumpmässiga melodier.

```rust
pub struct SampleAndHold {
    held_value: f32,
    slew_rate: f32,      // 0 = instant, 1 = slow glide
    current_value: f32,  // För slew
    output_buffer: AudioBuffer,
}

impl SampleAndHold {
    pub fn new() -> Self {
        Self {
            held_value: 0.0,
            slew_rate: 0.0,
            current_value: 0.0,
            output_buffer: AudioBuffer::new(256),
        }
    }
}

impl Describable for SampleAndHold {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("sample_hold", "S&H")
            .description("Sample and Hold with optional slew")
            .category(ModuleCategory::Utility)
            .parameter(
                ParameterDescriptor::float(TypedParam::SampleHold(SampleHoldParam::Slew), "Slew")
                    .description("Glide time between values")
                    .range(0.0, 1.0)
                    .default(0.0)
            )
            .port(PortDescriptor::audio_input("in", "In").description("Signal to sample"))
            .port(PortDescriptor::gate_input("trig", "Trig").description("Sample trigger"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl VoiceModule for SampleAndHold {
    fn process(&mut self, inputs: &HashMap<String, &AudioBuffer>,
               outputs: &mut HashMap<String, AudioBuffer>, context: &ProcessContext) {
        self.output_buffer.resize(context.samples);
        
        let signal = inputs.get("in");
        let trigger = inputs.get("trig");
        
        let mut prev_trig = 0.0f32;
        let slew_coef = if self.slew_rate > 0.0 {
            (-1.0 / (self.slew_rate * context.sample_rate * 0.1)).exp()
        } else {
            0.0
        };
        
        for i in 0..context.samples {
            // Detect trigger rising edge
            if let Some(trig) = trigger {
                let t = trig[i];
                if t > 0.5 && prev_trig <= 0.5 {
                    self.held_value = signal.map(|b| b[i]).unwrap_or(0.0);
                }
                prev_trig = t;
            }
            
            // Apply slew
            if self.slew_rate > 0.0 {
                self.current_value = self.held_value + 
                    (self.current_value - self.held_value) * slew_coef;
            } else {
                self.current_value = self.held_value;
            }
            
            self.output_buffer[i] = self.current_value;
        }
        
        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }
    // ... övriga trait-metoder
}
```

---

### 2.3 Slew Limiter / Lag Processor

**Ny fil:** `src/modules/slew.rs`

**Beskrivning:** Begränsar hur snabbt en signal kan ändras. Använd för portamento, smoothing av CV, etc.

```rust
pub struct SlewLimiter {
    rise_time: f32,   // Sekunder för 0->1
    fall_time: f32,   // Sekunder för 1->0
    current: f32,
    output_buffer: AudioBuffer,
}

impl SlewLimiter {
    pub fn new() -> Self {
        Self {
            rise_time: 0.01,
            fall_time: 0.01,
            current: 0.0,
            output_buffer: AudioBuffer::new(256),
        }
    }
    
    #[inline]
    fn slew_sample(&mut self, target: f32, sample_rate: f32) -> f32 {
        let diff = target - self.current;
        
        let max_change = if diff > 0.0 {
            // Rising
            if self.rise_time > 0.0 {
                1.0 / (self.rise_time * sample_rate)
            } else {
                f32::MAX
            }
        } else {
            // Falling
            if self.fall_time > 0.0 {
                1.0 / (self.fall_time * sample_rate)
            } else {
                f32::MAX
            }
        };
        
        self.current += diff.clamp(-max_change, max_change);
        self.current
    }
}

impl Describable for SlewLimiter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("slew", "Slew")
            .description("Slew limiter / lag processor")
            .category(ModuleCategory::Utility)
            .parameter(
                ParameterDescriptor::float(TypedParam::Slew(SlewParam::Rise), "Rise")
                    .description("Rise time")
                    .range(0.0, 2.0)
                    .default(0.01)
                    .unit(ParameterUnit::Seconds)
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Slew(SlewParam::Fall), "Fall")
                    .description("Fall time")
                    .range(0.0, 2.0)
                    .default(0.01)
                    .unit(ParameterUnit::Seconds)
            )
            .port(PortDescriptor::audio_input("in", "In"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}
```

---

### 2.4 Quantizer

**Ny fil:** `src/modules/quantizer.rs`

**Beskrivning:** Kvantiserar CV till musikaliska skalor. Essentiellt för generativ musik.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scale {
    #[default]
    Chromatic,
    Major,
    Minor,
    Pentatonic,
    Blues,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    WholeTone,
    Diminished,
}

impl Scale {
    /// Returnerar vilka halvtoner som ingår i skalan (0-11)
    fn intervals(&self) -> &'static [u8] {
        match self {
            Scale::Chromatic => &[0,1,2,3,4,5,6,7,8,9,10,11],
            Scale::Major => &[0,2,4,5,7,9,11],
            Scale::Minor => &[0,2,3,5,7,8,10],
            Scale::Pentatonic => &[0,2,4,7,9],
            Scale::Blues => &[0,3,5,6,7,10],
            Scale::Dorian => &[0,2,3,5,7,9,10],
            Scale::Phrygian => &[0,1,3,5,7,8,10],
            Scale::Lydian => &[0,2,4,6,7,9,11],
            Scale::Mixolydian => &[0,2,4,5,7,9,10],
            Scale::WholeTone => &[0,2,4,6,8,10],
            Scale::Diminished => &[0,2,3,5,6,8,9,11],
        }
    }
}

pub struct Quantizer {
    scale: Scale,
    root_note: u8,  // 0-11 (C=0, C#=1, etc)
    output_buffer: AudioBuffer,
}

impl Quantizer {
    pub fn new() -> Self {
        Self {
            scale: Scale::Major,
            root_note: 0,
            output_buffer: AudioBuffer::new(256),
        }
    }
    
    /// Kvantisera ett CV-värde (1V/oktav, 0 = C4) till närmaste not i skalan
    fn quantize(&self, cv: f32) -> f32 {
        // Konvertera CV till MIDI-aktig representation
        let semitones = cv * 12.0;
        let octave = (semitones / 12.0).floor();
        let note_in_octave = ((semitones % 12.0) + 12.0) % 12.0;
        
        // Hitta närmaste not i skalan
        let intervals = self.scale.intervals();
        let mut best_note = intervals[0];
        let mut best_dist = f32::MAX;
        
        for &interval in intervals {
            let adjusted = (interval as i32 + self.root_note as i32) % 12;
            let dist = (note_in_octave - adjusted as f32).abs();
            let dist = dist.min(12.0 - dist);  // Wrap-around distance
            
            if dist < best_dist {
                best_dist = dist;
                best_note = adjusted as u8;
            }
        }
        
        // Konvertera tillbaka till CV
        (octave * 12.0 + best_note as f32) / 12.0
    }
}

impl Describable for Quantizer {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("quantizer", "Quantizer")
            .description("Quantize CV to musical scales")
            .category(ModuleCategory::Utility)
            .parameter(
                ParameterDescriptor::choice(TypedParam::Quantizer(QuantizerParam::Scale), "Scale",
                    Scale::to_choices())
            )
            .parameter(
                ParameterDescriptor::choice(TypedParam::Quantizer(QuantizerParam::Root), "Root",
                    note_name_choices())  // C, C#, D, etc.
            )
            .port(PortDescriptor::control_input("in", "In").description("CV input"))
            .port(PortDescriptor::control_output("out", "Out").description("Quantized CV"))
            .port(PortDescriptor::gate_output("trig", "Trig").description("Trigger on note change"))
    }
}
```

---

### 2.5 Clock Divider / Multiplier

**Ny fil:** `src/modules/clock_div.rs`

**Beskrivning:** Dela eller multiplicera clock/gate-signaler. Grundläggande för rytmisk variation.

```rust
pub struct ClockDivider {
    division: u8,      // 1-32
    multiply: u8,      // 1-8
    swing: f32,        // 0-1
    
    // State
    input_count: u32,
    output_phase: f32,
    last_input: f32,
    last_output_time: u64,
    
    output_buffer: AudioBuffer,
}

impl ClockDivider {
    pub fn new() -> Self {
        Self {
            division: 1,
            multiply: 1,
            swing: 0.0,
            input_count: 0,
            output_phase: 0.0,
            last_input: 0.0,
            last_output_time: 0,
            output_buffer: AudioBuffer::new(256),
        }
    }
}

impl Describable for ClockDivider {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("clock_div", "Clock Div")
            .description("Clock divider and multiplier with swing")
            .category(ModuleCategory::Utility)
            .parameter(
                ParameterDescriptor::int(TypedParam::ClockDiv(ClockDivParam::Division), "÷")
                    .description("Division ratio")
                    .range(1, 32)
                    .default(1)
            )
            .parameter(
                ParameterDescriptor::int(TypedParam::ClockDiv(ClockDivParam::Multiply), "×")
                    .description("Multiplication ratio")
                    .range(1, 8)
                    .default(1)
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::ClockDiv(ClockDivParam::Swing), "Swing")
                    .description("Swing amount")
                    .range(0.0, 1.0)
                    .default(0.0)
            )
            .port(PortDescriptor::gate_input("in", "In").description("Clock input"))
            .port(PortDescriptor::gate_input("reset", "Reset").description("Reset to downbeat"))
            .port(PortDescriptor::gate_output("out", "Out").description("Processed clock"))
    }
}
```

---

### 2.6 Noise Generator (Expanded)

**Ny fil:** `src/modules/noise.rs`

**Beskrivning:** Dedikerad noise-modul med fler varianter och färgning.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseType {
    #[default]
    White,
    Pink,       // -3dB/oktav
    Brown,      // -6dB/oktav (Brownian/Red)
    Blue,       // +3dB/oktav
    Violet,     // +6dB/oktav
    Grey,       // Perceptuellt platt (A-viktad)
    Velvet,     // Impulsivt brus (för dithering)
    Crackle,    // Vinyl-stil
}

pub struct NoiseGenerator {
    noise_type: NoiseType,
    level: f32,
    
    // State för olika brustyper
    state: u32,
    pink_rows: [f32; 16],
    pink_sum: f32,
    pink_index: u32,
    brown_state: f32,
    blue_state: f32,
    
    output_buffer: AudioBuffer,
}

impl NoiseGenerator {
    // Implementera alla brustyper...
    
    fn generate_sample(&mut self) -> f32 {
        match self.noise_type {
            NoiseType::White => self.white_noise(),
            NoiseType::Pink => self.pink_noise(),
            NoiseType::Brown => self.brown_noise(),
            NoiseType::Blue => self.blue_noise(),
            NoiseType::Violet => self.violet_noise(),
            NoiseType::Grey => self.grey_noise(),
            NoiseType::Velvet => self.velvet_noise(),
            NoiseType::Crackle => self.crackle_noise(),
        }
    }
    
    fn blue_noise(&mut self) -> f32 {
        // Blue = differentierad white noise (+3dB/oktav)
        let white = self.white_noise();
        let blue = white - self.blue_state;
        self.blue_state = white;
        blue * 0.7  // Normalisering
    }
    
    fn violet_noise(&mut self) -> f32 {
        // Violet = dubbel-differentierad (+6dB/oktav)
        let blue = self.blue_noise();
        let violet = blue - self.blue_state;
        violet * 0.5
    }
    
    fn crackle_noise(&mut self) -> f32 {
        // Slumpmässiga klick med varierande amplitud
        if (self.white_noise() + 1.0) * 0.5 > 0.997 {
            self.white_noise() * 0.8
        } else {
            0.0
        }
    }
}
```

---

## Del 3: Nya Syntes-Moduler

### 3.1 Wavetable Oscillator

**Ny fil:** `src/modules/wavetable.rs`

**Beskrivning:** Oscillator som morphar mellan flera vågformer lagrade i en tabell.

```rust
const WAVETABLE_SIZE: usize = 2048;
const MAX_WAVETABLES: usize = 256;

pub struct WavetableOscillator {
    // Wavetable data
    tables: Vec<[f32; WAVETABLE_SIZE]>,
    num_tables: usize,
    
    // Parameters
    frequency: f32,
    position: f32,     // 0-1, morphar mellan tabeller
    detune: f32,
    level: f32,
    
    // State
    phase: f32,
    sample_rate: f32,
    
    output_buffer: AudioBuffer,
}

impl WavetableOscillator {
    pub fn new() -> Self {
        let mut wt = Self {
            tables: Vec::with_capacity(MAX_WAVETABLES),
            num_tables: 0,
            frequency: 440.0,
            position: 0.0,
            detune: 0.0,
            level: 1.0,
            phase: 0.0,
            sample_rate: 48000.0,
            output_buffer: AudioBuffer::new(256),
        };
        
        // Initiera med basic waveforms
        wt.init_basic_tables();
        wt
    }
    
    fn init_basic_tables(&mut self) {
        // Tabell 0: Sine
        let mut sine = [0.0f32; WAVETABLE_SIZE];
        for i in 0..WAVETABLE_SIZE {
            sine[i] = (i as f32 / WAVETABLE_SIZE as f32 * TAU).sin();
        }
        self.tables.push(sine);
        
        // Tabell 1: Triangle
        // Tabell 2: Saw
        // Tabell 3: Square
        // ... etc med harmoniskt innehåll
        
        self.num_tables = self.tables.len();
    }
    
    #[inline]
    fn read_interpolated(&self, phase: f32, table_idx: f32) -> f32 {
        // Bilineär interpolation mellan tabeller och samples
        let table_lo = (table_idx as usize).min(self.num_tables - 1);
        let table_hi = (table_lo + 1).min(self.num_tables - 1);
        let table_frac = table_idx.fract();
        
        let sample_pos = phase * WAVETABLE_SIZE as f32;
        let sample_lo = sample_pos as usize % WAVETABLE_SIZE;
        let sample_hi = (sample_lo + 1) % WAVETABLE_SIZE;
        let sample_frac = sample_pos.fract();
        
        // Interpolera inom varje tabell
        let val_lo = self.tables[table_lo][sample_lo] * (1.0 - sample_frac)
                   + self.tables[table_lo][sample_hi] * sample_frac;
        let val_hi = self.tables[table_hi][sample_lo] * (1.0 - sample_frac)
                   + self.tables[table_hi][sample_hi] * sample_frac;
        
        // Interpolera mellan tabeller
        val_lo * (1.0 - table_frac) + val_hi * table_frac
    }
}

impl Describable for WavetableOscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("wavetable", "Wavetable")
            .description("Wavetable oscillator with morphing")
            .category(ModuleCategory::Oscillator)
            .parameter(
                ParameterDescriptor::float(TypedParam::Wavetable(WavetableParam::Position), "Position")
                    .description("Morph position through wavetable")
                    .range(0.0, 1.0)
                    .default(0.0)
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Wavetable(WavetableParam::Frequency), "Freq")
                    .range(20.0, 20000.0)
                    .default(440.0)
                    .unit(ParameterUnit::Hertz)
                    .curve(ResponseCurve::Logarithmic)
            )
            .port(PortDescriptor::control_input("pos_cv", "Pos CV"))
            .port(PortDescriptor::control_input("fm", "FM"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}
```

---

### 3.2 Sub-Oscillator

**Ny fil:** `src/modules/sub_osc.rs`

**Beskrivning:** Enkel sub-oscillator en eller två oktaver under, för extra bas-fundament.

```rust
pub struct SubOscillator {
    // Parameters
    octave: i8,       // -1 eller -2
    waveform: SubWaveform,
    level: f32,
    
    // State
    phase: f32,
    master_freq: f32,
    sample_rate: f32,
    
    output_buffer: AudioBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubWaveform {
    #[default]
    Square,
    Sine,
    Pulse25,   // 25% pulse
}

impl SubOscillator {
    // Track master oscillator frequency och generera sub
    fn generate_sample(&mut self) -> f32 {
        let freq = self.master_freq * 2.0f32.powi(self.octave as i32);
        let dt = freq / self.sample_rate;
        
        let sample = match self.waveform {
            SubWaveform::Square => if self.phase < 0.5 { 1.0 } else { -1.0 },
            SubWaveform::Sine => (self.phase * TAU).sin(),
            SubWaveform::Pulse25 => if self.phase < 0.25 { 1.0 } else { -1.0 },
        };
        
        self.phase = (self.phase + dt).rem_euclid(1.0);
        sample * self.level
    }
}
```

---

### 3.3 Granular Processor

**Ny fil:** `src/modules/granular.rs`

**Beskrivning:** Granulär syntes - bryter upp ljud i små "korn" för texturella effekter.

```rust
const MAX_GRAINS: usize = 64;
const GRAIN_BUFFER_SIZE: usize = 48000 * 4;  // 4 sekunder vid 48kHz

struct Grain {
    active: bool,
    buffer_pos: f32,
    phase: f32,          // 0-1 inom grain
    duration: f32,       // I samples
    pitch_ratio: f32,
    pan: f32,
    amplitude: f32,
}

pub struct GranularProcessor {
    // Circular buffer för input
    buffer: Vec<f32>,
    write_pos: usize,
    
    // Grain pool
    grains: [Grain; MAX_GRAINS],
    
    // Parameters
    grain_size: f32,     // ms
    density: f32,        // grains per second
    position: f32,       // 0-1 i buffern
    position_random: f32,
    pitch: f32,          // Ratio
    pitch_random: f32,
    spray: f32,          // Timing randomization
    
    // State
    sample_rate: f32,
    samples_until_next: f32,
    
    output_buffer_l: AudioBuffer,
    output_buffer_r: AudioBuffer,
}

impl GranularProcessor {
    fn spawn_grain(&mut self) {
        // Hitta ledig grain slot
        if let Some(grain) = self.grains.iter_mut().find(|g| !g.active) {
            grain.active = true;
            grain.phase = 0.0;
            grain.duration = self.grain_size * self.sample_rate / 1000.0;
            
            // Randomisera position
            let pos_offset = (random() - 0.5) * self.position_random;
            grain.buffer_pos = ((self.position + pos_offset) * GRAIN_BUFFER_SIZE as f32)
                .rem_euclid(GRAIN_BUFFER_SIZE as f32);
            
            // Randomisera pitch
            let pitch_offset = (random() - 0.5) * self.pitch_random;
            grain.pitch_ratio = self.pitch * (1.0 + pitch_offset);
            
            // Randomisera pan
            grain.pan = random();
            grain.amplitude = 1.0;
        }
    }
    
    fn process_grain(&mut self, grain: &mut Grain) -> (f32, f32) {
        if !grain.active {
            return (0.0, 0.0);
        }
        
        // Hann window envelope
        let env = 0.5 * (1.0 - (grain.phase * TAU).cos());
        
        // Läs från buffer med interpolation
        let pos = grain.buffer_pos as usize % GRAIN_BUFFER_SIZE;
        let frac = grain.buffer_pos.fract();
        let sample = self.buffer[pos] * (1.0 - frac) 
                   + self.buffer[(pos + 1) % GRAIN_BUFFER_SIZE] * frac;
        
        // Avancera
        grain.buffer_pos += grain.pitch_ratio;
        grain.phase += 1.0 / grain.duration;
        
        if grain.phase >= 1.0 {
            grain.active = false;
        }
        
        let out = sample * env * grain.amplitude;
        let left = out * (1.0 - grain.pan).sqrt();
        let right = out * grain.pan.sqrt();
        
        (left, right)
    }
}
```

---

## Del 4: Nya Effekt-Moduler

### 4.1 Chorus

**Ny fil:** `src/effects/chorus.rs`

**Beskrivning:** Klassisk chorus-effekt med flera LFO-modulerade delay-linjer.

```rust
const MAX_CHORUS_VOICES: usize = 4;
const MAX_DELAY_MS: f32 = 40.0;

pub struct Chorus {
    // Parameters
    rate: f32,           // LFO rate
    depth: f32,          // Modulation depth
    mix: f32,
    voices: usize,       // 1-4
    spread: f32,         // Stereo spread
    
    // State per voice
    delay_lines: [Vec<f32>; MAX_CHORUS_VOICES],
    write_pos: usize,
    lfo_phases: [f32; MAX_CHORUS_VOICES],
    
    sample_rate: f32,
}

impl Chorus {
    fn process_sample(&mut self, input: f32) -> (f32, f32) {
        let mut left = input * (1.0 - self.mix);
        let mut right = input * (1.0 - self.mix);
        
        for i in 0..self.voices {
            // LFO för denna röst (fasförskjutna)
            let lfo = (self.lfo_phases[i] * TAU).sin();
            self.lfo_phases[i] = (self.lfo_phases[i] 
                + self.rate / self.sample_rate).rem_euclid(1.0);
            
            // Delay time med modulation
            let base_delay = 7.0 + (i as f32 * 3.0);  // ms
            let mod_delay = base_delay + lfo * self.depth * 5.0;
            let delay_samples = mod_delay * self.sample_rate / 1000.0;
            
            // Läs från delay line med interpolation
            let read_pos = (self.write_pos as f32 - delay_samples)
                .rem_euclid(self.delay_lines[i].len() as f32);
            let sample = self.read_interpolated(i, read_pos);
            
            // Stereo spread
            let pan = (i as f32 / (self.voices - 1).max(1) as f32) * self.spread;
            left += sample * (1.0 - pan) * self.mix / self.voices as f32;
            right += sample * pan * self.mix / self.voices as f32;
        }
        
        // Skriv till delay lines
        for i in 0..self.voices {
            self.delay_lines[i][self.write_pos] = input;
        }
        self.write_pos = (self.write_pos + 1) % self.delay_lines[0].len();
        
        (left, right)
    }
}
```

---

### 4.2 Stereo Widener

**Ny fil:** `src/effects/stereo_width.rs`

**Beskrivning:** Kontrollera stereobredd från mono till extra-bred.

```rust
pub struct StereoWidener {
    width: f32,      // 0 = mono, 1 = normal, 2 = extra wide
    bass_mono: f32,  // Frekvens under vilken bas blir mono
    
    // State för bass mono
    lp_state_l: f32,
    lp_state_r: f32,
    
    sample_rate: f32,
}

impl StereoWidener {
    fn process_sample(&mut self, left: f32, right: f32) -> (f32, f32) {
        // Mid-Side processing
        let mid = (left + right) * 0.5;
        let side = (left - right) * 0.5;
        
        // Justera width
        let side_scaled = side * self.width;
        
        // Konvertera tillbaka
        let mut out_l = mid + side_scaled;
        let mut out_r = mid - side_scaled;
        
        // Bass mono: lowpass för att få bas, mono:ifiera den
        if self.bass_mono > 20.0 {
            let coef = (TAU * self.bass_mono / self.sample_rate).min(1.0);
            
            self.lp_state_l += (left - self.lp_state_l) * coef;
            self.lp_state_r += (right - self.lp_state_r) * coef;
            
            let bass_mono = (self.lp_state_l + self.lp_state_r) * 0.5;
            
            // Subtrahera original-bas, lägg till mono-bas
            out_l = out_l - self.lp_state_l + bass_mono;
            out_r = out_r - self.lp_state_r + bass_mono;
        }
        
        (out_l, out_r)
    }
}
```

---

### 4.3 Bit Crusher (utökad)

**Finns i distortion.rs men bör utökas till egen modul med fler features.**

**Ny fil:** `src/effects/bitcrusher.rs`

```rust
pub struct BitCrusher {
    bit_depth: f32,      // 1-16
    sample_rate_div: f32, // Sample rate reduction (1 = ingen, 32 = max)
    dither: bool,
    jitter: f32,         // Sample timing randomization
    
    // State
    held_sample: f32,
    samples_since_update: f32,
    noise_state: u32,
}

impl BitCrusher {
    fn process_sample(&mut self, input: f32) -> f32 {
        // Sample rate reduction med jitter
        self.samples_since_update += 1.0;
        
        let update_threshold = self.sample_rate_div 
            + self.jitter * (self.noise() * 2.0 - 1.0);
        
        if self.samples_since_update >= update_threshold {
            self.samples_since_update = 0.0;
            
            // Bit depth reduction
            let levels = 2.0f32.powf(self.bit_depth);
            let mut quantized = (input * levels * 0.5).round() / (levels * 0.5);
            
            // Optional dithering
            if self.dither {
                let dither_amount = 1.0 / levels;
                quantized += (self.noise() - 0.5) * dither_amount;
            }
            
            self.held_sample = quantized;
        }
        
        self.held_sample
    }
}
```

---

## Del 5: Modulationshantering

### 5.1 Mod Matrix

**Ny fil:** `src/engine/mod_matrix.rs`

**Beskrivning:** Central modulationsmatris för att routa CV-källor till destinationer.

```rust
pub struct ModulationSlot {
    pub source: ModSource,
    pub destination: ModDestination,
    pub amount: f32,
    pub bipolar: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    None,
    Lfo1, Lfo2,
    Env1, Env2,
    ModWheel,
    Aftertouch,
    Velocity,
    KeyTrack,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModDestination {
    None,
    Osc1Pitch, Osc2Pitch,
    Osc1PW, Osc2PW,
    FilterCutoff,
    FilterResonance,
    Amp,
    Pan,
    Lfo1Rate, Lfo2Rate,
    Lfo1Depth, Lfo2Depth,
}

pub struct ModMatrix {
    slots: Vec<ModulationSlot>,
    max_slots: usize,
}

impl ModMatrix {
    pub fn new(max_slots: usize) -> Self {
        Self {
            slots: Vec::with_capacity(max_slots),
            max_slots,
        }
    }
    
    /// Beräkna total modulation för en destination
    pub fn get_modulation(&self, dest: ModDestination, sources: &ModSourceValues) -> f32 {
        let mut total = 0.0;
        
        for slot in &self.slots {
            if slot.destination == dest {
                let source_val = sources.get(slot.source);
                let mod_val = if slot.bipolar {
                    source_val * slot.amount
                } else {
                    source_val.max(0.0) * slot.amount
                };
                total += mod_val;
            }
        }
        
        total
    }
}
```

---

## Del 6: Filstruktur

Efter implementation:

```
src/
├── modules/
│   ├── mod.rs
│   ├── core.rs
│   ├── oscillator.rs       # Uppdaterad med TZ-FM, brown noise
│   ├── wavetable.rs        # NY
│   ├── sub_osc.rs          # NY
│   ├── math_oscillator.rs
│   ├── filter.rs
│   ├── envelope.rs         # Uppdaterad med velocity curves
│   ├── lfo.rs
│   ├── amplifier.rs
│   ├── noise.rs            # NY - expanded noise
│   ├── ring_mod.rs         # NY
│   ├── sample_hold.rs      # NY
│   ├── slew.rs             # NY
│   ├── quantizer.rs        # NY
│   ├── clock_div.rs        # NY
│   ├── granular.rs         # NY
│   └── output.rs
├── effects/
│   ├── mod.rs
│   ├── chorus.rs           # NY
│   ├── stereo_width.rs     # NY
│   ├── bitcrusher.rs       # NY (utbruten från distortion)
│   ├── delay.rs
│   ├── reverb.rs
│   ├── distortion.rs
│   ├── compressor.rs
│   ├── eq.rs
│   ├── flanger.rs
│   └── phaser.rs
└── engine/
    ├── mod_matrix.rs       # NY
    └── ...
```

---

## Del 7: Implementation-prioritering

### Prioritet 1 (Kärnfunktionalitet)
1. ✅ Through-Zero FM
2. ✅ Velocity Curves
3. ✅ Ring Modulator
4. ✅ Sample & Hold

### Prioritet 2 (Kreativa verktyg)
5. Quantizer
6. Slew Limiter
7. Clock Divider
8. Wavetable Oscillator

### Prioritet 3 (Effekter och polish)
9. Chorus
10. Stereo Widener
11. Expanded Noise Generator
12. Granular Processor

### Prioritet 4 (Avancerat)
13. Mod Matrix
14. Sub-Oscillator
15. Utökad BitCrusher

---

## Del 8: TypedParam-tillägg

Lägg till i `src/engine/typed_params.rs`:

```rust
// Nya parameter-enums
pub enum RingModParam { Mix }
pub enum SampleHoldParam { Slew }
pub enum SlewParam { Rise, Fall }
pub enum QuantizerParam { Scale, Root }
pub enum ClockDivParam { Division, Multiply, Swing }
pub enum WavetableParam { Position, Frequency, Detune, Level }
pub enum NoiseParam { Type, Level }
pub enum GranularParam { Size, Density, Position, PositionRandom, Pitch, PitchRandom, Spray, Mix }
pub enum ChorusParam { Rate, Depth, Mix, Voices, Spread }
pub enum StereoWidthParam { Width, BassMono }
pub enum BitCrusherParam { BitDepth, SampleRateDiv, Dither, Jitter }

// Uppdatera TypedParam enum
pub enum TypedParam {
    // ... existing ...
    RingMod(RingModParam),
    SampleHold(SampleHoldParam),
    Slew(SlewParam),
    Quantizer(QuantizerParam),
    ClockDiv(ClockDivParam),
    Wavetable(WavetableParam),
    Noise(NoiseParam),
    Granular(GranularParam),
    Chorus(ChorusParam),
    StereoWidth(StereoWidthParam),
    BitCrusher(BitCrusherParam),
}
```

---

## Sammanfattning

Denna instruktion täcker:

1. **5 förbättringar** av befintliga moduler
2. **6 nya utility-moduler** (Ring Mod, S&H, Slew, Quantizer, Clock Div, Noise)
3. **3 nya syntes-moduler** (Wavetable, Sub-Osc, Granular)
4. **3 nya effekt-moduler** (Chorus, Stereo Width, BitCrusher)
5. **1 nytt system** (Mod Matrix)

Total: ~15 nya/förbättrade komponenter som ger en komplett modulär synthesizer.
