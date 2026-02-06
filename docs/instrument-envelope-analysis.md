# Analys: Instrument-import och Envelope-rendering

**Datum:** 2026-02-06
**Kontext:** Kritisk granskning av hur instrument, envelopes och ADSR skapas vid import av XM/S3M/MOD-filer, samt hur de renderas under uppspelning. Verifierad mot FastTracker 2-specifikationen, ft2-clone källkod, och OpenMPT testfall.

**Referenser:** Se `docs/references/` för detaljerade specifikationer.

---

## Sammanfattning

Instrument-importen fungerar korrekt i sina grunddrag — samples extraheras, envelope-punkter bevaras, och multi-sample keymaps hanteras. Men envelope-**renderingen** har flera allvarliga avvikelser från FT2-specifikationen som påverkar hur importerade XM-filer låter. Det allvarligaste problemet är att fadeout implementerats som exponentiell decay istället för den linjära subtraktion som FT2 använder, samt att fadeout och envelope-avancering inte körs parallellt.

---

## Dataflöde: Fil → Ljud

```
XM/MOD/S3M-fil
    ↓ xmrs crate (parsing)
xmrs::Module
    ↓ tracker.rs: extract_instruments(), extract_samples()
ImportedInstrument + Arc<Sample>
    ↓ egui_backend.rs: setup (rad 1880-1937)
    ├─ envelope_points finns → MultiPointEnvelope
    └─ inga punkter → ADSR Envelope (legacy)
    ↓ Uppspelning
    ├─ Gate ON → trigger() → Playing
    ├─ Sustain point → Sustaining
    ├─ Gate OFF → release() → Releasing → Fadeout
    └─ Fadeout level < 0.001 → Idle
```

---

## 1. KRITISKT: Fadeout är exponentiell istället för linjär

**Filer:** `tracker.rs:249-255`, `multi_point_envelope.rs:306-327`

### Specifikation (FT2)

FT2 använder **linjär subtraktion** för fadeout:

```c
// ft2_replayer.c - VARJE TICK efter key-off:
ch->fadeoutVol -= ch->fadeoutSpeed;   // Linjär subtraktion
if (ch->fadeoutVol <= 0) ch->fadeoutVol = 0;

// Slutlig volym:
FinalVol = (fadeoutVol / 32768) * (envVol / 64) * (globalVol / 64) * (outVol / 64);
```

- `fadeoutVol` startar vid **32768** (normaliserat 1.0)
- Subtraheras med `fadeoutSpeed` (0..4095) **varje tick**
- Ticks till tystnad = `32768 / fadeoutSpeed`

### Vår implementation

```rust
// FadeoutRate::to_decay_per_tick (tracker.rs:249-255)
pub fn to_decay_per_tick(self) -> f32 {
    if self.0 == 0 { 1.0 }
    else { 1.0 - (self.0 as f32 / 65536.0) }  // Multiplikativ faktor
}

// MultiPointEnvelope::process_sample (multi_point_envelope.rs:306-312)
let decay_per_sample = self.fadeout_rate.to_decay_per_tick()
    .powf(1.0 / self.samples_per_frame);
self.fadeout_level *= decay_per_sample;   // EXPONENTIELL decay
```

### Konkret jämförelse (fadeout=4096, 125 BPM)

| Tick | FT2 (linjär) | Vår kod (exponentiell) |
|------|--------------|----------------------|
| 0 | 1.000 | 1.000 |
| 1 | 0.875 | 0.938 |
| 2 | 0.750 | 0.879 |
| 4 | 0.500 | 0.774 |
| 8 | **0.000** (tyst) | 0.599 |
| 16 | — | 0.359 |
| 32 | — | 0.129 |
| 107 | — | **0.001** (tyst) |

**FT2 når tystnad på 8 ticks (160ms). Vår kod behöver ~107 ticks (2.14s) — 13x längre.**

### Dessutom: Fel divisor

Koden använder `65536.0` som divisor, men FT2 använder `32768` som normaliseringsvärde för fadeoutVol. Om vi behåller exponentiell modell (fel i sig) borde divisorn åtminstone vara `32768`.

### Rekommendation

Implementera linjär fadeout:

```rust
// Korrekt linjär fadeout per tick:
let fadeout_per_tick = self.fadeout_rate.as_u16() as f32 / 32768.0;
let fadeout_per_sample = fadeout_per_tick / self.samples_per_frame;
self.fadeout_level = (self.fadeout_level - fadeout_per_sample).max(0.0);
```

---

## 2. KRITISKT: Fadeout och envelope körs inte parallellt

**Fil:** `multi_point_envelope.rs:220-231, 247-329`

### Specifikation (FT2)

I FT2 sker detta **PARALLELLT** efter key-off:
1. Envelope-positionen fortsätter avancera (förbi sustain-punkten)
2. Fadeout subtraheras **SAMTIDIGT** varje tick
3. Slutgiltig volym = `envelope_value * fadeout_level`

### Vår implementation

```rust
pub fn release(&mut self) {
    match self.stage {
        MultiPointStage::Sustaining => {
            self.stage = MultiPointStage::Releasing;  // Fortsätt envelope
        }
        MultiPointStage::Playing => {
            self.stage = MultiPointStage::Fadeout;     // Hoppa DIREKT till fadeout
        }
        _ => {}
    }
}
```

Problem:
- **Playing → Fadeout:** Om noten släpps medan envelope spelar (innan sustain), hoppar koden direkt till Fadeout **utan att köra envelope-resten**. I FT2 fortsätter envelopet OCH fadeout körs parallellt.
- **Releasing → Fadeout:** Fadeout börjar inte förrän hela envelopet spelat klart. I FT2 startar fadeout omedelbart vid key-off.

### Korrekt flöde

```
FT2 efter key-off:
  VARJE TICK:
    1. Avancera envelope-position (förbi sustain, respektera loop om ej sustain=loopEnd)
    2. Beräkna envelope-värde via interpolering
    3. Subtrahera fadeout: fadeoutVol -= fadeoutSpeed
    4. output = envelope_value * (fadeoutVol / 32768)
```

### Rekommendation

Ta bort `Fadeout` som separat stage. Istället:
- Håll en `released: bool`-flagga
- När `released = true`: avancera envelope UTAN sustain-paus + applicera fadeout parallellt
- Output = `interpolated_value * fadeout_level`

---

## 3. ALLVARLIGT: Panning envelope ignoreras helt

**Fil:** `tracker.rs:264-269`

### Specifikation

XM-filer har **två separata envelopes** per instrument:
- Volume envelope (0..64 → amplitud)
- Panning envelope (0..64 → panorering, 32=center)

Båda har samma punkt/sustain/loop-struktur.

### Vår implementation

Koden extraherar **bara volume envelope**:

```rust
let envelope_points: Vec<(u16, f32)> = instr
    .volume_envelope
    .point
    .iter()
    .map(|p| (p.frame as u16, p.value))
    .collect();
```

`instr.panning_envelope` läses aldrig. Inga panning-envelope-punkter sparas i `ImportedInstrument`. Ingen panning envelope-modul skapas.

### Konsekvens

XM-filer med panorerings-automation via panning envelope (vanligt i professionella moduler) kommer att spelas med statisk panorering. Alla dynamiska panneringseffekter som designades av musikern försvinner.

### Rekommendation

Lägg till `panning_envelope_points`, `panning_envelope_sustain`, `panning_envelope_loop` i `ImportedInstrument` och skapa en andra `MultiPointEnvelope`-instans kopplad till Amplifier-modulens panorering.

---

## 4. ALLVARLIGT: ADSR-konverteringen ger felaktiga release-tider

**Fil:** `tracker.rs:425-430`

### Nuvarande kod

```rust
let release_secs = if volume_fadeout > 0.001 {
    (1.0 / volume_fadeout).clamp(0.01, 10.0)
} else {
    0.3
};
```

### Problem

`volume_fadeout` från xmrs ligger i spannet 0..65535 (raw XM-värde, eventuellt normaliserat). Om det är raw:

| Fadeout-värde | `1.0 / fadeout` | Faktisk FT2-tid |
|---------------|----------------|-----------------|
| 4096 | 0.000244 s | 0.16 s |
| 1024 | 0.000977 s | 0.64 s |
| 256 | 0.003906 s | 2.56 s |

ADSR-releasen blir **hundratals gånger kortare** än den borde vara. Alla fadeout-tider under 1024 mappas till `0.01s` (clamp-minimum) — praktiskt taget omedelbar release.

### Mildring

Denna bugg påverkar bara den legacy ADSR-pathen (instrument utan envelope-punkter). MultiPointEnvelope-pathen används för instrument med explicita envelope-punkter. Men MOD- och S3M-filer som saknar envelopes men har fadeout-liknande beteende kan påverkas.

---

## 5. ALLVARLIGT: Envelope tick-rate uppdateras inte vid BPM-ändring

**Fil:** `multi_point_envelope.rs:113, 190-193`

### Nuvarande kod

```rust
// Standardvärde (aldrig ändrat under uppspelning)
tick_rate: 50.0, // Default: 125 BPM * 2 / 5 = 50 Hz
```

### Problem

1. **Vid import:** `MultiPointEnvelope` skapas med default tick_rate=50.0, oavsett songens faktiska BPM. Om songen har BPM=150 borde tick_rate vara 60.0 Hz.

2. **Under uppspelning:** Om BPM ändras via Fxx-effekt (SetTempo), uppdateras inte MultiPointEnvelope-modulens tick_rate. Envelopes fortsätter med den ursprungliga hastigheten.

### Konsekvens

- Vid BPM=150: envelopes spelas 16.7% för långsamt
- Vid BPM=100: envelopes spelas 20% för snabbt
- BPM-ändringar mid-song ignoreras helt av envelopes

### Rekommendation

Tick-rate bör sättas korrekt vid import (baserat på songens `default_bpm`) och uppdateras dynamiskt vid BPM-ändringar under uppspelning.

---

## 6. SIGNIFIKANT: Sustain/loop-interaktion avviker från FT2

**Fil:** `multi_point_envelope.rs:258-281`

### FT2-beteende

```c
// Loop kontrolleras EFTER sustain:
if (envPos == ins->volEnvLoopEnd) {
    if (!(ins->volEnvFlags & ENV_SUSTAIN) ||
        envPos != ins->volEnvSustain || !ch->keyOff)
    {
        envPos = ins->volEnvLoopStart;  // Loopa
    }
    // ELSE: sustain = loopEnd OCH noten släppt → LOOPA INTE
}
```

Specifikt: om sustain-punkt = loop end och noten har släppts, ska loopen **inte** exekveras. Envelopet fortsätter istället förbi loop end.

### Vår implementation

```rust
// Sustain och loop kontrolleras separat utan interaktion:
if self.stage == MultiPointStage::Playing {
    // Sustain-kontroll (oberoende)
    if let Some(sustain_idx) = self.sustain_point { ... }

    // Loop-kontroll (oberoende, ingen sustain-check)
    if let (Some(start_idx), Some(end_idx)) = (self.loop_start, self.loop_end) { ... }
}
```

Koden kontrollerar sustain och loop **oberoende** utan FT2:s interaktionslogik. Dessutom kontrolleras loop bara i `Playing`-stage, inte i `Releasing`-stage.

### Konsekvens

Instrument med sustain-punkt vid loop end kommer att bete sig felaktigt efter key-off — de fortsätter loopa istället för att avancera förbi loop end.

---

## 7. SIGNIFIKANT: Envelope-positionering off-by-one

**Fil:** `multi_point_envelope.rs:252-253`

### FT2-beteende

FT2 inkrementerar envelope-tick **FÖRE** evaluering:

```c
ch->volEnvTick++;  // Inkrementera FÖRST
if (ch->volEnvTick == ins->volEnvPoints[envPos][0]) { ... }
```

### Vår implementation

```rust
// Avancera frame position
self.current_frame += 1.0 / self.samples_per_frame;  // Inkrementera
// Sedan evaluera
let value = self.interpolate_at_frame(self.current_frame);
```

Ordningen ser likadan ut (inkrementera före evaluering), men:
- FT2 inkrementerar med **1 tick** (heltals-steg)
- Vår kod inkrementerar med `1.0 / samples_per_frame` (bråkdels-steg per sample)

Det innebär att vid start (`current_frame = 0.0`), första samplet redan avancerar till `1/960 ≈ 0.001` — medan FT2 börjar vid tick 0 och interpolerar från exakt punkt 0. Effekten är liten men kan orsaka hörbara skillnader vid korta envelopes.

---

## 8. NOTERBART: NoteOff utan envelope har inget särskilt beteende

### FT2-beteende

Om ett instrument **saknar volume envelope** (disabled):
- NoteOff → volymen sätts till **0 omedelbart**
- Fadeout processas **inte**

### Vår implementation

Om `volume_envelope.enabled = false` skapas ingen Envelope-modul alls:

```rust
// egui_backend.rs:1880
let envelope_amplifier = if inst_meta.volume_envelope.enabled {
    // Skapa envelope + amplifier
} else {
    None  // Ingen envelope → direkt koppling till output
};
```

Utan envelope-modul finns inget sätt att tysta samplet vid NoteOff. Samplet fortsätter spela tills det naturligt slutar (eller dess loop fortsätter oändligt).

### Konsekvens

Instrument utan envelopes men med loopad sample kommer att spela oändligt efter NoteOff — de kan aldrig tystas. Detta påverkar särskilt MOD-filer där inga instrument har envelopes.

### Rekommendation

Alla instrument bör ha åtminstone en minimal "gate envelope" — 1.0 vid gate high, 0.0 vid gate low — även utan explicit XM volume envelope.

---

## 9. NOTERBART: MOD- och S3M-format hanteras som XM

### MOD-format

MOD-filer (ProTracker) har **inga envelopes alls**. Volym kontrolleras helt via:
- Sample default volume (per instrument)
- Volymkolumnen i patterns (per rad)
- Volymeffekter (slides, etc.)

xmrs konverterar MOD till sitt generiska `Module`-format. Envelope-fälten blir tomma/inaktiverade, vilket är korrekt. Men `volume_fadeout` kan eventuellt få ett standardvärde som sedan feltolkas.

### S3M-format

S3M (Scream Tracker 3) har begränsat volymhantering:
- Instruments = enstaka samples med default volume
- Inga punkt-baserade envelopes
- Volymen hanteras via effektkolumnen

Liknande problem som MOD — xmrs abstraherar bort formatskillnader men vår import-kod antar XM-semantik.

---

## 10. NOTERBART: Envelope-effekter ignoreras

**Fil:** `tracker.rs:961-968`

Följande XM-effekter som påverkar envelopes direkt ignoreras vid import:

| Effekt | Funktion | Konsekvens |
|--------|----------|------------|
| `InstrumentVolumeEnvelope` | Slå av/på volume envelope | Kan inte stänga av envelope mid-song |
| `InstrumentVolumeEnvelopePosition` | Hoppa till position i envelope | Lxx-effekten fungerar inte |
| `InstrumentPanningEnvelope` | Slå av/på panning envelope | — |
| `InstrumentPanningEnvelopePosition` | Hoppa till position | — |
| `InstrumentPitchEnvelope` | Slå av/på pitch envelope | — |

---

## Jämförelsetabell: FT2 vs Vår implementation

| Aspekt | FT2 (korrekt) | Vår implementation | Status |
|--------|---------------|-------------------|--------|
| Fadeout-modell | Linjär subtraktion | Exponentiell multiplikation | **FEL** |
| Fadeout startpunkt | fadeoutVol = 32768 | Divisor 65536 | **FEL** |
| Fadeout + envelope | Parallellt | Sekventiellt (envelope sedan fadeout) | **FEL** |
| Panning envelope | Full support | Ignoreras helt | **SAKNAS** |
| Volume envelope punkter | 12 punkter, 0..64 värde | 25 punkter, 0.0-1.0 (IT-limit) | OK* |
| Sustain point | Pausar vid punkt, prioritet över loop | Pausar, oberoende av loop | **AVVIKER** |
| Loop region | Interagerar med sustain | Oberoende av sustain | **AVVIKER** |
| Envelope tick rate | BPM * 2 / 5, dynamisk | Fast 50 Hz | **AVVIKER** |
| Linjär interpolering | Ja | Ja | OK |
| NoteOff utan envelope | Volym → 0 direkt | Ingen effekt | **SAKNAS** |
| ADSR release-beräkning | N/A (FT2 har inte ADSR) | `1/fadeout` (felaktig) | **FEL** |
| Envelope-effekter (Lxx etc.) | Full support | Ignoreras | **SAKNAS** |
| Envelope pre-evaluering | Tick++ före evaluering | Frame++ före evaluering | OK |

\* IT-limit (25 punkter) är en superset av XM (12 punkter) — kompatibelt men inte exakt.

---

## Prioriterad åtgärdslista

### Prio 1 — Felaktig rendering (påverkar ljud direkt)

1. **Implementera linjär fadeout** — Byt ut exponentiell multiplikation mot linjär subtraktion med korrekt divisor (32768)
2. **Kör fadeout parallellt med envelope** — Ta bort Fadeout som separat stage; applicera fadeout som multiplikator under Releasing-stage
3. **Fixa sustain/loop-interaktion** — Sustain vid loop end + key-off → stoppa loopning

### Prio 2 — Saknad funktionalitet

4. **Implementera panning envelope** — Extrahera och rendera panning envelope på samma sätt som volume envelope
5. **Gate envelope för instrument utan volume envelope** — Säkerställ att NoteOff kan tysta instrument även utan XM volume envelope
6. **Dynamisk tick_rate baserad på BPM** — Sätt korrekt tick_rate vid import och uppdatera vid BPM-ändring

### Prio 3 — Förbättringar

7. **Fixa ADSR release-beräkning** — Korrekt konvertering från fadeout-värde till release-tid i sekunder
8. **Stöd för envelope-effekter (Lxx etc.)** — Möjliggör runtime-kontroll av envelope-position och on/off
9. **Korrekt hantering av MOD/S3M-specifika beteenden** — Separera format-specifik logik vid import

---

## Appendix: Verifierade korrektheter

Dessa delar av implementationen är korrekta och väl genomförda:

1. **Sample-extraktion** — Korrekt hantering av 8/16-bit, mono/stereo, delta-encoded data
2. **Root note-beräkning** — Korrekt Amiga/Linear frequency mapping med finetune
3. **Loop-typer** — Forward och ping-pong loops hanteras korrekt
4. **Multi-sample keymap** — MIDI note → sample mapping korrekt implementerad
5. **Linjär interpolering** — `interpolate_at_frame()` implementerar korrekt linjär interpolering mellan punkter
6. **ArrayVec för envelope-punkter** — Undviker heap-allokering i audio thread
7. **Gate edge detection** — Korrekt rising/falling edge detection med `prev_gate`
8. **Velocity sensitivity** — Korrekt formel: `1.0 - sensitivity * (1.0 - velocity)`
