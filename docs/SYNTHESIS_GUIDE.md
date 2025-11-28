# Modulär Syntes - En Praktisk Guide

## Grundkonceptet

En modulär synthesizer är som ett byggsystem för ljud. Istället för en färdig signalväg
kan du koppla ihop moduler precis som du vill. Det finns tre grundläggande signaltyper:

```
AUDIO SIGNAL      Hörbara ljudvågor (20 Hz - 20 kHz)
                  Går genom: VCO → Filter → VCA → Output

CONTROL VOLTAGE   Långsamma signaler som styr parametrar (0-10V typiskt)
(CV)              Kommer från: LFO, Envelope, Keyboard, Sequencer
                  Styr: Pitch, Filter cutoff, Amplitude, etc.

GATE/TRIGGER      På/av-signaler (0V = av, 5-10V = på)
                  Kommer från: Keyboard, Sequencer
                  Startar: Envelopes, LFO sync
```

## Den klassiska signalkedjan

Detta är grunden för nästan all subtraktiv syntes:

```
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│   VCO   │────▶│   VCF   │────▶│   VCA   │────▶│ OUTPUT  │
│Oscillator│    │ Filter  │     │Amplifier│     │         │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
     │               │               │
     │ Pitch CV      │ Cutoff CV     │ Amplitude CV
     │               │               │
┌────┴────┐    ┌────┴────┐    ┌────┴────┐
│Keyboard │    │   LFO   │    │  ADSR   │
│ 1V/Oct  │    │ or ENV  │    │Envelope │
└─────────┘    └─────────┘    └─────────┘
```

### Vad varje modul gör:

**VCO (Voltage Controlled Oscillator)**
- Genererar råljudet - en kontinuerlig ton
- Olika vågformer ger olika karaktär:
  - **Sawtooth** `/|/|/|` - Rik på övertoner, brassig, klassisk synth-lead
  - **Square** `_|‾|_|` - Ihålig, klarinett-liknande, bra för bas
  - **Triangle** `/\/\/\` - Mjuk, flöjt-liknande, få övertoner
  - **Sine** `∿∿∿` - Ren ton, inga övertoner, sub-bas

**VCF (Voltage Controlled Filter)**
- Formar ljudet genom att ta bort frekvenser
- **Cutoff** - Var filtret "klipper" frekvenser
- **Resonance** - Förstärker frekvenser vid cutoff (kan självoscillera!)
- Filtertyper:
  - **Lowpass (LP)** - Släpper igenom låga frekvenser, dämpar höga (vanligast)
  - **Highpass (HP)** - Släpper igenom höga, dämpar låga
  - **Bandpass (BP)** - Släpper igenom ett band, dämpar resten

**VCA (Voltage Controlled Amplifier)**
- Styr volymen
- Utan VCA: konstant ljud
- Med envelope → VCA: ljudet har en "form" (attack, sustain, release)

**ADSR Envelope**
- Skapar en "kurva" över tid när du trycker på en tangent:
```
    │   /\
    │  /  \____
    │ /        \
    │/          \
    └────────────────▶ tid
      A  D  S   R

A = Attack  - Tid att nå maxnivå (0 = snabb, 2s = långsam)
D = Decay   - Tid att falla till sustain-nivå
S = Sustain - Nivå medan tangent hålls nere (0-100%)
R = Release - Tid att tystna efter tangent släpps
```

**LFO (Low Frequency Oscillator)**
- Samma som VCO men LÅNGSAM (0.1 - 20 Hz)
- Skapar cykliska rörelser/modulationer
- Typiska användningar:
  - LFO → VCO pitch = **Vibrato**
  - LFO → VCF cutoff = **Wah-wah / Filter sweep**
  - LFO → VCA = **Tremolo**

---

## Praktiska Patches

### 1. Basic Lead Sound
```
Kopplingar:
  Keyboard CV  ──▶ VCO Pitch
  Keyboard Gate ──▶ ADSR Gate
  VCO (Saw)    ──▶ VCF Input
  VCF          ──▶ VCA Input
  ADSR         ──▶ VCA CV

Inställningar:
  VCO: Sawtooth
  VCF: Cutoff 60%, Resonance 20%
  ADSR: A=0.01s, D=0.2s, S=70%, R=0.3s
```

**Resultat:** En klassisk synth-lead. Snabb attack, lite decay, 
håller tonen medan du spelar, släpper snabbt.

---

### 2. Plucky Bass
```
Kopplingar:
  Keyboard CV  ──▶ VCO Pitch
  Keyboard Gate ──▶ ADSR Gate
  VCO (Square) ──▶ VCF Input
  VCF          ──▶ VCA Input
  ADSR         ──▶ VCA CV
  ADSR         ──▶ VCF Cutoff CV  ← NYTT!

Inställningar:
  VCO: Square, -1 oktav
  VCF: Cutoff 30%, Resonance 40%, Env Amount 50%
  ADSR: A=0.001s, D=0.15s, S=0%, R=0.1s
```

**Resultat:** Kort, perkussiv bas. Filtret öppnar snabbt och 
stänger igen - ger "pluck"-karaktär.

**Varför det fungerar:** 
- Kort decay + 0% sustain = ljudet dör snabbt
- ADSR → Filter = filtret följer samma kurva som volymen
- Låg cutoff + hög resonance = "boing"-karaktär

---

### 3. Pad med vibrato
```
Kopplingar:
  Keyboard CV  ──▶ VCO Pitch
  Keyboard Gate ──▶ ADSR Gate
  LFO (Sine)   ──▶ VCO Pitch CV (liten mängd!)
  VCO (Saw)    ──▶ VCF Input
  VCF          ──▶ VCA Input
  ADSR         ──▶ VCA CV

Inställningar:
  VCO: Sawtooth
  LFO: Sine, Rate 5Hz, Amount 2-5%
  VCF: Cutoff 50%, Resonance 10%
  ADSR: A=0.8s, D=0.5s, S=80%, R=1.5s
```

**Resultat:** Mjuk pad med subtilt vibrato.

**Varför det fungerar:**
- Lång attack = ljudet "fadas in"
- LFO → Pitch (liten mängd) = naturligt vibrato
- Lång release = ljudet hänger kvar

---

### 4. Filter Sweep / Wah
```
Kopplingar:
  Keyboard CV  ──▶ VCO Pitch
  Keyboard Gate ──▶ ADSR Gate
  LFO (Tri)    ──▶ VCF Cutoff CV
  VCO (Saw)    ──▶ VCF Input
  VCF          ──▶ VCA Input
  ADSR         ──▶ VCA CV

Inställningar:
  VCO: Sawtooth
  LFO: Triangle, Rate 0.5Hz, Amount 50%
  VCF: Cutoff 40%, Resonance 60%
  ADSR: A=0.01s, D=0.1s, S=100%, R=0.3s
```

**Resultat:** Klassisk "wah-wah" effekt.

**Varför det fungerar:**
- LFO → Filter cutoff = cutoff rör sig upp och ner
- Hög resonance = förstärker rörelsen, mer dramatisk effekt
- Triangelvåg = mjuk, jämn rörelse

---

### 5. Aggressiv Bass (TB-303 stil)
```
Kopplingar:
  Keyboard CV   ──▶ VCO Pitch
  Keyboard Gate ──▶ ADSR1 Gate (VCA)
  Keyboard Gate ──▶ ADSR2 Gate (Filter)
  VCO (Saw)     ──▶ VCF Input
  VCF           ──▶ VCA Input
  ADSR1         ──▶ VCA CV
  ADSR2         ──▶ VCF Cutoff CV

Inställningar:
  VCO: Sawtooth
  VCF: Cutoff 20%, Resonance 80%, Env Amount 70%
  ADSR1 (VCA): A=0.001s, D=0.3s, S=0%, R=0.1s
  ADSR2 (Filter): A=0.001s, D=0.1s, S=0%, R=0.05s
```

**Resultat:** Squelchy acid-bass.

**Varför det fungerar:**
- Separata envelopes för VCA och Filter
- Filter-envelope är SNABBARE än VCA = "squelch"
- Hög resonance = självoscillation vid cutoff
- Låg cutoff + hög env amount = stort sweep

---

### 6. Kick Drum
```
Kopplingar:
  Trigger      ──▶ ADSR1 Gate (Pitch)
  Trigger      ──▶ ADSR2 Gate (VCA)
  ADSR1        ──▶ VCO Pitch CV (mycket!)
  VCO (Sine)   ──▶ VCA Input
  ADSR2        ──▶ VCA CV

Inställningar:
  VCO: Sine, bas-frekvens ~50Hz
  ADSR1 (Pitch): A=0.001s, D=0.05s, S=0%, R=0.01s, Amount=200%
  ADSR2 (VCA): A=0.001s, D=0.2s, S=0%, R=0.1s
```

**Resultat:** Klassisk synth-kick.

**Varför det fungerar:**
- Pitch-envelope ger "punch" - startar högt, faller snabbt
- Sine-våg = ren sub-bas
- Kort decay = tight kick

---

### 7. Snare Drum
```
Kopplingar:
  Trigger      ──▶ ADSR1 Gate (VCA-Tone)
  Trigger      ──▶ ADSR2 Gate (VCA-Noise)
  ADSR1        ──▶ VCO Pitch CV
  VCO (Tri)    ──▶ VCA1 Input
  Noise        ──▶ VCF Input (Highpass)
  VCF          ──▶ VCA2 Input
  VCA1 + VCA2  ──▶ Mixer ──▶ Output

Inställningar:
  VCO: Triangle, ~180Hz
  ADSR1 (Tone): A=0.001s, D=0.1s, S=0%, R=0.05s
  Noise → HPF: Cutoff 2kHz
  ADSR2 (Noise): A=0.001s, D=0.15s, S=0%, R=0.1s
  Mix: 30% tone, 70% noise
```

**Resultat:** Snare med ton-kropp och brus-"snärp".

---

### 8. Ambient Drone
```
Kopplingar:
  Fixed CV     ──▶ VCO1 Pitch (låg ton)
  Fixed CV     ──▶ VCO2 Pitch (lite högre, slight detune)
  LFO1 (slow)  ──▶ VCO2 Pitch (mycket lite)
  LFO2         ──▶ VCF Cutoff
  VCO1 + VCO2  ──▶ Mixer ──▶ VCF
  VCF          ──▶ VCA
  Fixed CV     ──▶ VCA (konstant volym)

Inställningar:
  VCO1: Sawtooth, C2
  VCO2: Sawtooth, C2 + 5 cents detune
  LFO1: Sine, 0.1Hz, Amount 1%
  LFO2: Triangle, 0.3Hz, Amount 30%
  VCF: Cutoff 40%, Resonance 30%
```

**Resultat:** Svävande, levande drone.

**Varför det fungerar:**
- Två oscillatorer med slight detune = "beating" / chorus-effekt
- Väldigt långsam LFO på pitch = organisk drift
- LFO på filter = rörligt ljud utan att vara uppenbart

---

## Modulationsdestinationer - Vad kan styras?

| Destination      | Effekt när modulerad                    |
|------------------|-----------------------------------------|
| VCO Pitch        | Vibrato, sirener, FM-liknande ljud      |
| VCO Pulse Width  | PWM - rörlig, chorus-liknande klang     |
| VCF Cutoff       | Wah-wah, filter sweeps, brightness      |
| VCF Resonance    | Mer dramatiska filter-effekter          |
| VCA Amplitude    | Tremolo, gating, volume swells          |
| LFO Rate         | Accelererande/decelererande modulationer|
| ADSR Times       | Dynamiska envelopes                     |

---

## Vanliga misstag och lösningar

### "Jag hör inget ljud!"
1. Är VCA öppen? (Behöver CV eller fast nivå)
2. Är ADSR:en triggad? (Gate-signal behövs)
3. Är filter-cutoff för låg? (Höj cutoff)
4. Är output-nivån uppe?

### "Ljudet är för hårt/vasst"
1. Sänk filter cutoff
2. Sänk resonance
3. Byt till mjukare vågform (Saw → Triangle → Sine)
4. Lägg till lite attack på envelope

### "Ljudet är tråkigt/statiskt"
1. Lägg till LFO-modulation
2. Använd envelope på filter (inte bara VCA)
3. Lägg till en andra oscillator med detune
4. Öka resonance lite

### "Filtret låter inte"
1. Resonance på 0 ger subtil effekt - höj till 30-50%
2. Se till att det finns övertoner att filtrera (Saw/Square, inte Sine)
3. Modulera cutoff med envelope eller LFO för att höra rörelsen

---

## Avancerade tekniker

### Keyboard Tracking på Filter
```
Keyboard CV ──▶ VCO Pitch
Keyboard CV ──▶ VCF Cutoff (delvis)
```
Får filtret att öppna mer för höga noter. 
Annars låter höga noter "dova" jämfört med låga.

### Velocity till Filter
```
Velocity CV ──▶ VCF Cutoff CV
```
Hårdare anslag = ljusare ljud. Mer expressivt.

### Cross-modulation (FM)
```
VCO2 ──▶ VCO1 Pitch CV (audio rate!)
```
Skapar komplexa, metalliska ljud. 
**Varning:** Kan låta väldigt kaotiskt!

### Self-patching Filter
```
VCF Output ──▶ VCF Cutoff CV (via attenuator)
```
Filtret modulerar sig självt. Kaotiska, levande texturer.

---

## Quick Reference: Ljudtyper

| Ljudtyp       | VCO       | Filter           | ADSR              |
|---------------|-----------|------------------|-------------------|
| Lead          | Saw       | LP 60%, Res 20%  | Fast A, medium DR |
| Bass          | Square    | LP 30%, Res 40%  | Fast A, short D   |
| Pad           | Saw       | LP 50%, Res 10%  | Slow A, long R    |
| Pluck         | Saw       | LP 40%, Res 50%  | Fast A, short D   |
| Brass         | Saw       | LP 50%, Res 30%  | Medium A, sustain |
| Strings       | Saw×2     | LP 60%, Res 0%   | Slow A, long R    |
| Kick          | Sine      | Ingen            | Pitch env down    |
| Snare         | Noise+Tri | HP på noise      | Very short        |

---

## Nästa steg

1. **Börja enkelt** - En VCO → Filter → VCA → Output
2. **Lägg till modulation** - En sak i taget (LFO eller extra envelope)
3. **Experimentera** - "Fel" kopplingar ger ofta intressanta resultat
4. **Lyssna aktivt** - Vad händer när du vrider på en parameter?
5. **Spara patches** - Dokumentera inställningar du gillar

Lycka till med syntandet! 🎹
