# Jämförelse: Minimoog Model D vs Modular-Synth 0.12.0

## Minimoog Model D Specifikationer

Baserat på officiell Moog-dokumentation:

### Oscillatorer
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Antal oscillatorer | 3 VCO | 2 VCO | ⚠️ Saknar 1 |
| Frekvensomfång | 0.1 Hz - 20 kHz | ~20 Hz - 20 kHz | ⚠️ Saknar LFO-range |
| Range-switch (oktav) | 6 lägen (LO, 32', 16', 8', 4', 2') | Kontinuerlig | ✅ Annorlunda men OK |
| Vågformer OSC 1&2 | Triangle, Tri-Saw, Saw, Square, Wide Pulse, Narrow Pulse | Saw, Square, Tri, Sine, Noise | ⚠️ Saknar PWM-varianter |
| Vågformer OSC 3 | + Reverse Saw | Samma som OSC1 | ✅ OK |
| Oscillator 3 som LFO | Ja (LO-range + kbd tracking off) | Separat LFO-modul | ✅ Bättre! |
| Tune-kontroll | Per oscillator | Per oscillator | ✅ |
| Detune | Via tune-kontroll | Cents-baserad | ✅ |

### Noise Generator
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| White noise | ✅ | ✅ (som vågform) | ✅ |
| Pink noise | ✅ | ❌ | ❌ Saknas |
| Separat modul | ✅ Ja, i mixer | ❌ Del av oscillator | ⚠️ Begränsat |

### Mixer
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| OSC 1 level | ✅ | ✅ | ✅ |
| OSC 2 level | ✅ | ✅ | ✅ |
| OSC 3 level | ✅ | ❌ (bara 2 osc) | ❌ |
| Noise level | ✅ Separat | ❌ | ❌ |
| External input | ✅ | ❌ | ❌ Saknas |
| Feedback loop | ✅ (Output → Input) | ❌ | ❌ Saknas |
| Overload-lampa | ✅ | ❌ | ❌ |

### Filter (VCF)
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Filtertyp | 24 dB/oct Ladder LP | State-variable + Ladder | ✅ Bättre! |
| Cutoff range | 10 Hz - 20 kHz | Liknande | ✅ |
| Resonance/Emphasis | Ja, till självoscillation | Ja | ✅ |
| Keyboard tracking | 0%, 33%, 67%, 100% | ❌ | ❌ SAKNAS |
| Filter envelope amount | ✅ | ✅ | ✅ |
| Filter FM från OSC 3 | ✅ (via mod wheel) | ❌ | ❌ SAKNAS |
| Filtertyper | Endast LP | LP, HP, BP, Notch | ✅ Bättre! |

### Contour Generators (Envelopes)
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Antal envelopes | 2 (Filter + Amp) | 2 | ✅ |
| Stages | ADS + D (decay=release) | ADSR (separat R) | ✅ Bättre! |
| Attack range | 1ms - 10s | Liknande | ✅ |
| Decay range | 1ms - 10s | Liknande | ✅ |
| Sustain | 0-100% | 0-100% | ✅ |
| "Decay" switch | On/Off (release) | Alltid på | ✅ |

### Modulation
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Mod sources | OSC 3, Noise, Filter ENV, LFO | LFO | ⚠️ Färre val |
| Mod mix | Blandning av 2 källor | En källa | ❌ Begränsat |
| Mod wheel | Styr mod amount i realtid | ❌ | ❌ SAKNAS |
| Mod → Pitch (OSC 1&2) | ✅ | ❌ Inte kopplat | ❌ SAKNAS |
| Mod → Filter cutoff | ✅ | ⚠️ Fast amount | ⚠️ Begränsat |
| LFO Rate | 0.05 - 200 Hz | Liknande | ✅ |
| LFO Waveforms | Triangle, Square | Sine, Tri, Saw, Square, S&H | ✅ Bättre! |

### Controllers
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Pitch wheel | ✅ ±7 semitoner | ❌ | ❌ SAKNAS |
| Mod wheel | ✅ | ❌ | ❌ SAKNAS |
| Glide (portamento) | ✅ On/Off + Time | ✅ Time | ✅ |
| Glide mode | Linear/Exponential | Exponential | ✅ |

### Output
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Master volume | ✅ | ✅ | ✅ |
| Headphone out | ✅ | Via system | ✅ |
| Main out | ✅ High/Low | ✅ | ✅ |
| A-440 reference | ✅ | ❌ | ❌ |

### Övrigt
| Feature | Minimoog | Vår synth | Status |
|---------|----------|-----------|--------|
| Velocity sensitivity | ✅ (nya modellen) | ❌ Används inte | ❌ SAKNAS |
| Aftertouch | ✅ (nya modellen) | ❌ | ❌ SAKNAS |
| Note priority | Low/High/Last | Konfigurerbart | ✅ |
| Polyfoni | Mono | Poly/Mono/Legato/Unison | ✅ Bättre! |
| MIDI | ✅ | ❌ (endast keyboard) | ❌ SAKNAS |

---

## Sammanfattning: Vad SAKNAS för Minimoog-paritet

### 🔴 Kritiska funktioner som saknas

1. **Modulation Wheel**
   - Minimoog: Mod wheel styr hur mycket modulation som appliceras i realtid
   - Vår synth: Ingen wheel, ingen realtidskontroll

2. **Pitch Wheel / Pitch Bend**
   - Minimoog: ±7 semitoner böjning
   - Vår synth: Saknas helt

3. **Keyboard Tracking till Filter**
   - Minimoog: 0/33/67/100% tracking så höga noter har öppnare filter
   - Vår synth: Saknas - alla noter har samma cutoff

4. **Modulation → Oscillator Pitch**
   - Minimoog: OSC 3 / LFO kan modulera pitch på OSC 1 & 2 (vibrato)
   - Vår synth: LFO finns men är INTE KOPPLAD till pitch

5. **Velocity → Filter/Amp**
   - Minimoog (ny): Velocity påverkar filter och/eller amp
   - Vår synth: Velocity sparas men används inte

6. **Tredje Oscillator**
   - Minimoog: 3 oscillatorer (eller 2 + LFO)
   - Vår synth: Endast 2 oscillatorer

### 🟡 Funktioner som delvis saknas

7. **Pink Noise**
   - Minimoog: White + Pink noise
   - Vår synth: Endast white noise (som oscillator-vågform)

8. **Noise som separat mixerkanal**
   - Minimoog: Noise har egen volym i mixer
   - Vår synth: Noise är en vågform, ersätter oscillatorn

9. **External Audio Input**
   - Minimoog: Kan processa extern audio genom filter/VCA
   - Vår synth: Saknas

10. **Feedback Loop**
    - Minimoog: Output kan routas tillbaka till input för distortion
    - Vår synth: Saknas

11. **Pulse Width Modulation (PWM)**
    - Minimoog: Flera pulse-bredder + modulation
    - Vår synth: Fast square wave, ingen PWM

### 🟢 Saker vi har som Minimoog SAKNAR

- **Polyfoni** - Minimoog är mono, vi har full poly
- **Fler filtertyper** - Vi har LP, HP, BP, Notch
- **Separat Release** - Minimoog har decay=release
- **State-variable filter** - Utöver ladder
- **Fler LFO-vågformer** - Inkl. Sample & Hold
- **Effekter** - Delay, Reverb, Chorus, Distortion
- **Unison mode** - Med detune spread

---

## Prioriterad Implementation

### Fas 1: Grundläggande Minimoog-funktionalitet
```
1. [ ] Pitch bend kontroll (±12 semitoner konfigurerbart)
2. [ ] Mod wheel → Amount kontroll
3. [ ] LFO → Oscillator pitch (vibrato) - KOPPLA IHOP!
4. [ ] Keyboard tracking för filter (0-100%)
5. [ ] Velocity → Filter cutoff amount
6. [ ] Velocity → Amp level
```

### Fas 2: Utökad funktionalitet
```
7. [ ] Tredje oscillator
8. [ ] Pink noise generator
9. [ ] Noise som separat mixer-kanal
10. [ ] Pulse Width parameter på square wave
11. [ ] PWM (LFO/Envelope → Pulse Width)
```

### Fas 3: Avancerat
```
12. [ ] External audio input
13. [ ] Feedback loop
14. [ ] Filter envelope → Pitch (för kicks)
15. [ ] Oscillator sync (OSC2 → OSC1)
16. [ ] MIDI input
```

---

## Kodexempel: Implementera saknade funktioner

### 1. Keyboard Tracking för Filter

```rust
// I Filter-modulen, lägg till:
pub struct Filter {
    // ... existing fields ...
    keyboard_tracking: f32,  // 0.0 - 1.0 (0%, 33%, 67%, 100%)
    current_note: u8,
}

impl Filter {
    pub fn set_keyboard_tracking(&mut self, amount: f32) {
        self.keyboard_tracking = amount.clamp(0.0, 1.0);
    }
    
    fn calculate_cutoff(&self, base_cutoff: f32) -> f32 {
        // Middle C (60) som referens
        let semitones_from_middle_c = self.current_note as f32 - 60.0;
        let octaves = semitones_from_middle_c / 12.0;
        
        // Keyboard tracking: varje oktav fördubblar/halverar cutoff
        let tracking_multiplier = 2.0_f32.powf(octaves * self.keyboard_tracking);
        
        (base_cutoff * tracking_multiplier).clamp(20.0, 20000.0)
    }
}
```

### 2. LFO → Pitch modulation

```rust
// I Voice::process(), lägg till:
fn process(&mut self, output: &mut AudioBuffer, context: &ProcessContext) {
    // ... existing code ...
    
    // Hämta LFO-värde
    let lfo_value = self.get_module("lfo")
        .map(|lfo| lfo.get_output_sample())
        .unwrap_or(0.0);
    
    // Applicera pitch modulation (i cents)
    let pitch_mod_cents = lfo_value * self.pitch_mod_amount * 100.0; // ±100 cents max
    let pitch_mod_ratio = 2.0_f32.powf(pitch_mod_cents / 1200.0);
    
    let modulated_freq = self.base_frequency * pitch_mod_ratio;
    self.set_oscillator_frequency(modulated_freq);
}
```

### 3. Velocity → Filter

```rust
// I Voice::note_on(), lägg till:
pub fn note_on(&mut self, note: u8, velocity: f32, time: u64) {
    // ... existing code ...
    
    // Velocity → Filter cutoff modulation
    if let Some(filter) = self.get_module_mut("filter") {
        let vel_mod = velocity * self.velocity_to_filter_amount;
        filter.set_parameter("cutoff_mod", vel_mod);
    }
}
```

---

## Slutsats

**Vår synth har grunderna** men saknar den **expressivitet** som gör Minimoog så spelbar:
- Pitch/Mod wheels för realtidskontroll
- Keyboard tracking för naturligt ljud över hela registret
- Velocity-respons för dynamiskt spel
- Modulation-routing (LFO → pitch) för vibrato

De **tekniska bitarna finns redan** i koden (LFO, Envelope, Filter) - 
de behöver bara **kopplas ihop** och **exponeras i GUI**.
