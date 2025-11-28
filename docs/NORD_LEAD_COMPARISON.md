# Jämförelse: Nord Lead 2X vs Modular-Synth 0.12.0

## Nord Lead 2X Specifikationer

Nord Lead 2X är en virtuell analog synth (VA) från Clavia, släppt 2003. 
Den anses vara en av de bästa VA-syntharna och har en mer modern arkitektur än Minimoog.

---

## Oscillatorer

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Antal oscillatorer | 2 per röst | 2 per röst | ✅ |
| Polyfoni | 20 röster | Konfigurerbart | ✅ |
| Multitimbral | 4 delar (slots) | 1 del | ⚠️ Saknas |
| Vågformer OSC 1 | Saw, Pulse, Triangle, Sine | Saw, Square, Tri, Sine, Noise | ✅ Liknande |
| Vågformer OSC 2 | Saw, Pulse, Triangle, Noise | Samma som OSC 1 | ✅ |
| Pulse Width | Variabel 0-99% | Fast 50% | ❌ SAKNAS |
| PWM (LFO → PW) | ✅ | ❌ | ❌ SAKNAS |
| OSC 2 Semitone tuning | ±60 semitoner | Cents-baserad | ✅ |
| OSC 2 Fine tune | ±50 cents | ✅ | ✅ |
| Oscillator Sync | OSC 2 → OSC 1 (hard sync) | ❌ | ❌ SAKNAS |
| FM (Frequency Mod) | OSC 2 → OSC 1 (linear FM) | ❌ | ❌ SAKNAS |
| Ring Modulator | ✅ | ❌ | ❌ SAKNAS |
| Noise med färgkontroll | ✅ (via tune-ratt) | ❌ (endast white) | ⚠️ Begränsat |

---

## Filter

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Filter typer | LP12, LP24, HP, BP, Notch | LP, HP, BP, Notch | ✅ |
| 12 dB/oct Low Pass | ✅ | ❌ (endast 24dB) | ⚠️ Saknas |
| 24 dB/oct Low Pass | ✅ | ✅ | ✅ |
| High Pass | ✅ | ✅ | ✅ |
| Band Pass | ✅ | ✅ | ✅ |
| Notch | ✅ | ✅ | ✅ |
| Cutoff | ✅ | ✅ | ✅ |
| Resonance | Ja, till självoscillation | ✅ | ✅ |
| Envelope Amount | ✅ | ✅ | ✅ |
| Velocity → Filter | ✅ Konfigurerbart | ❌ | ❌ SAKNAS |
| Keyboard Tracking | Off / Half / Full | ❌ | ❌ SAKNAS |
| Filter Distortion | ✅ (före filter) | ❌ (separat effekt) | ⚠️ Annorlunda |
| Filter ADSR | Full ADSR | Full ADSR | ✅ |

---

## Envelopes

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Filter Envelope | ADSR | ADSR | ✅ |
| Amplifier Envelope | ADSR | ADSR | ✅ |
| Modulation Envelope | AD (Attack-Decay) | ❌ | ❌ SAKNAS |
| Mod ENV → OSC 2 Pitch | ✅ | ❌ | ❌ SAKNAS |
| Mod ENV → FM Amount | ✅ | ❌ | ❌ SAKNAS |
| Mod ENV → Pulse Width | ✅ | ❌ | ❌ SAKNAS |
| Envelope curves | Linjär | Konfigurerbar kurva | ✅ Bättre! |

---

## LFO

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Antal LFOs | 2 (LFO 1 + LFO 2/Arp) | 1 | ⚠️ Saknar 1 |
| LFO 1 Waveforms | Tri, Saw, Pulse, Filtered Noise, Random | Sine, Tri, Saw, Square, S&H | ✅ Liknande |
| LFO 2 Waveforms | Triangle only | - | ⚠️ |
| Rate | Kontinuerlig | Kontinuerlig | ✅ |
| **LFO 1 Destinations:** | | | |
| → OSC 1+2 Pitch | ✅ | ❌ Inte kopplat | ❌ SAKNAS |
| → OSC 2 Pitch only | ✅ | ❌ | ❌ SAKNAS |
| → Filter Cutoff | ✅ | ⚠️ Fast amount | ⚠️ Begränsat |
| → Pulse Width | ✅ | ❌ | ❌ SAKNAS |
| → FM Amount | ✅ | ❌ | ❌ SAKNAS |
| **LFO 2 Destinations:** | | | |
| → OSC 1+2 Pitch | ✅ | - | ❌ |
| → Amplitude (tremolo) | ✅ | ❌ | ❌ SAKNAS |
| Amount control | Per destination | Global | ⚠️ |
| Sync to MIDI clock | ✅ | ❌ | ❌ SAKNAS |

---

## Modulation & Control

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Pitch Stick/Wheel | ✅ Konfigurerbar range | ❌ | ❌ SAKNAS |
| Mod Wheel | ✅ | ❌ | ❌ SAKNAS |
| Mod Wheel Destinations | Filter, LFO1, OSC2, FM | - | ❌ SAKNAS |
| Velocity | Full implementation | Ej använd | ❌ SAKNAS |
| Velocity → Filter Amount | ✅ | ❌ | ❌ |
| Velocity → Amp | ✅ | ❌ | ❌ |
| **Velocity Morph** | Vilken parameter som helst | ❌ | ❌ SAKNAS |
| Aftertouch | ✅ | ❌ | ❌ SAKNAS |
| Expression Pedal | ✅ Assignable | ❌ | ❌ SAKNAS |

---

## Arpeggiator

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Arpeggiator | ✅ | ❌ | ❌ SAKNAS |
| Patterns | Up, Down, Up/Down, Random | - | ❌ |
| Range | 1-4 oktaver | - | ❌ |
| Hold | ✅ | - | ❌ |
| Echo/Repeat | 1-8 repeats | - | ❌ |
| Sync to MIDI | ✅ | - | ❌ |

---

## Voice Modes

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Poly | ✅ | ✅ | ✅ |
| Mono | ✅ | ✅ | ✅ |
| Legato | ✅ | ✅ | ✅ |
| Unison Mono | ✅ (stacked voices) | ✅ | ✅ |
| Unison Poly | ✅ | ⚠️ Begränsat | ⚠️ |
| Portamento/Glide | ✅ | ✅ | ✅ |
| Auto-portamento | ✅ (endast legato) | ❌ | ❌ |
| Unison Detune | Konfigurerbart | ✅ | ✅ |

---

## Effects (Nord Lead 2X saknar inbyggda effekter!)

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Delay | ❌ | ✅ | ✅ Bättre! |
| Reverb | ❌ | ✅ | ✅ Bättre! |
| Chorus | ❌ | ✅ | ✅ Bättre! |
| Distortion | ❌ (endast filter dist) | ✅ | ✅ Bättre! |
| Phaser | ❌ | ❌ | - |
| Flanger | ❌ | ❌ | - |

---

## Övriga funktioner

| Feature | Nord Lead 2X | Vår synth | Status |
|---------|--------------|-----------|--------|
| Patch Memory | 990 programs | JSON-filer | ✅ |
| Performances | 400 | ❌ | ❌ SAKNAS |
| 4 Slots/Layers | ✅ | ❌ | ❌ SAKNAS |
| Keyboard Split | ✅ | ❌ | ❌ SAKNAS |
| Drum Kits | 10 kits | ❌ | ❌ |
| MIDI | Full implementation | ❌ | ❌ SAKNAS |
| 4 Audio Outputs | ✅ | Stereo | ⚠️ |
| 24-bit 96kHz DAC | ✅ | f32 processing | ✅ |

---

## Sammanfattning: Vad SAKNAS för Nord Lead 2X-paritet

### 🔴 Kritiska funktioner som saknas

1. **Oscillator Sync**
   - Nord: OSC 2 låses till OSC 1 - klassiska sync-leads
   - Vår synth: Saknas helt

2. **FM Synthesis**
   - Nord: OSC 2 → OSC 1 frekvensmodulation
   - Vår synth: Saknas helt

3. **Pulse Width + PWM**
   - Nord: Variabel pulsbredd + LFO-modulation
   - Vår synth: Fast square wave

4. **LFO → Pitch koppling**
   - Nord: LFO 1 och 2 kan modulera pitch (vibrato)
   - Vår synth: LFO finns men EJ KOPPLAD

5. **LFO → Amplitude (Tremolo)**
   - Nord: LFO 2 kan modulera amp
   - Vår synth: Saknas

6. **Modulation Envelope**
   - Nord: Separat AD-envelope för OSC2/FM/PW
   - Vår synth: Saknas

7. **Velocity-system**
   - Nord: Full velocity → valfri parameter ("Velocity Morph")
   - Vår synth: Velocity sparas men används inte

8. **Keyboard Tracking (Filter)**
   - Nord: Off / Half / Full
   - Vår synth: Saknas

9. **Ring Modulator**
   - Nord: OSC 1 × OSC 2
   - Vår synth: Saknas

10. **Arpeggiator**
    - Nord: Full arpeggiator med patterns
    - Vår synth: Saknas

### 🟡 Delvis implementerat

| Funktion | Status |
|----------|--------|
| Noise | Som vågform, inte separat med färgkontroll |
| LFO → Filter | Fast amount, ingen wheel-kontroll |
| 12 dB filter | Endast 24 dB implementerat |

### 🟢 Saker vi har som Nord Lead 2X SAKNAR

- **Inbyggda effekter** - Delay, Reverb, Chorus, Distortion
- **Fler LFO-vågformer** - S&H, Sine
- **Konfigurerbar envelope-kurva** - Linjär/Exponentiell
- **State-Variable Filter** - Utöver standard
- **Ladder Filter emulation** - Moog-stil
- **Granular synthesis** (planerat)
- **Sample playback** (planerat)

---

## Prioriterad Implementation för Nord Lead-paritet

### Fas 1: Grundläggande modulation (samma som Minimoog)
```
1. [ ] LFO → Oscillator pitch (vibrato)
2. [ ] LFO → Amplitude (tremolo)
3. [ ] Keyboard tracking för filter
4. [ ] Velocity → Filter + Amp
5. [ ] Pitch bend kontroll
6. [ ] Mod wheel kontroll
```

### Fas 2: Oscillator-funktioner
```
7. [ ] Pulse Width parameter (0-100%)
8. [ ] PWM: LFO → Pulse Width
9. [ ] Oscillator Sync (OSC2 → OSC1)
10. [ ] FM: OSC2 → OSC1 pitch
11. [ ] Ring Modulator
```

### Fas 3: Extra modulation
```
12. [ ] Modulation Envelope (AD)
13. [ ] Mod ENV → OSC2 pitch
14. [ ] Mod ENV → Pulse Width
15. [ ] Mod ENV → FM Amount
16. [ ] Noise med färgkontroll
```

### Fas 4: Performance
```
17. [ ] Arpeggiator
18. [ ] Multi-timbral (4 slots)
19. [ ] Keyboard split
20. [ ] MIDI implementation
```

---

## Jämförelse: Nord Lead 2X vs Minimoog vs Vår Synth

| Feature | Minimoog | Nord Lead 2X | Vår Synth |
|---------|----------|--------------|-----------|
| Oscillatorer | 3 | 2 | 2 |
| Polyfoni | Mono | 20 | Poly |
| OSC Sync | Nej* | Ja | Nej |
| FM | Nej | Ja | Nej |
| Ring Mod | Nej | Ja | Nej |
| PWM | Nej* | Ja | Nej |
| LFO→Pitch | Ja | Ja | **NEJ** |
| LFO→Filter | Ja | Ja | Delvis |
| LFO→Amp | Nej | Ja | **NEJ** |
| Keyboard Track | Ja | Ja | **NEJ** |
| Velocity | Ja** | Ja | **NEJ** |
| Mod Wheel | Ja | Ja | **NEJ** |
| Arpeggiator | Nej | Ja | Nej |
| Effekter | Nej | Nej | **JA** |
| Filter typer | LP | LP12,LP24,HP,BP | LP,HP,BP,Notch |

*Kan modifieras
**Nya modellen

---

## Slutsats

Nord Lead 2X har en **mer modern arkitektur** än Minimoog:
- Oscillator sync, FM, Ring Mod för mer komplexa ljud
- PWM för fetare chorus-liknande effekter
- Två LFO:s med fler destinations
- Modulation Envelope för pitch/timbral kontroll
- Full velocity implementation

**Vår synth har effekter** som Nord saknar, men saknar nästan all **modulation och expressivitet**.

De **absolut viktigaste** funktionerna att lägga till (gemensamma för båda):

1. **LFO → Pitch** - vibrato
2. **LFO → Amp** - tremolo  
3. **Keyboard tracking** - naturligt ljud
4. **Velocity routing** - dynamiskt spel
5. **Pitch/Mod wheel** - realtidskontroll

Dessa fem funktioner skulle göra synthen **spelbar och uttrycksfull**.
