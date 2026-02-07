   
# Effekt-noggrannhetsanalys: Vår implementation vs FT2

Analys av alla tracker-effekter som kan påverka ljudet och orsaka felaktig uppspelning.
Jämfört mot **ft2-clone** (8bitbubsy), OpenMPT testfall och MilkyTracker-dokumentation.

**Datum:** 2026-02-07
**Version:** 0.92.0 (buggar 1.1-1.3 fixade i denna version)
**Filer analyserade:**
- `crates/synth_engine/src/tracker_effects.rs` — effektprocessor
- `crates/synth_sequencer/src/effects.rs` — EffectCommand-typer och waveform-sampling
- `crates/synth_engine/src/voice.rs` — pitch/volym-applicering på voices
- `crates/modular_synth/src/io/import/tracker.rs` — XM-import och effektkonvertering
- `crates/synth_engine/src/sequencer_engine.rs` — effekt→engine-integration

**FT2-referens:** `docs/references/ft2-effect-reference.md`

---

## Sammanfattning

| Kategori | Antal |
|----------|-------|
| Bekräftade buggar (ljudpåverkan) | 3 |
| Saknade effekter | 4 |
| Kända begränsningar (xmrs) | 2 |
| Mindre avvikelser | 4 |
| Korrekta implementationer | 15+ |

---

## 1. BEKRÄFTADE BUGGAR (påverkar ljud)

### 1.1 KRITISK: Tremolo-djup ~4x för svagt

**Plats:** `tracker_effects.rs` — `TremoloDepth::from_param()` (rad 319-320)

**Vår kod:**
```rust
pub fn from_param(param: u8) -> Self {
    Self(f32::from(param) / 64.0)
}
```
- Vår max depth (param=15): `15 / 64 = 0.234` (23.4% av full volym)

**FT2 (ft2-clone):**
```c
tmpTrem = (waveform_value * depth) >> 6;  // waveform_value: 0-255, depth: 0-15
// Max: (255 * 15) >> 6 = 59 volym-enheter av 64 = 92.2%
```
- FT2 max depth (param=15): `(255 × 15) / 64 / 64 = 0.934` (93.4% av full volym, trunkeras till 59/64 = 92.2%)

**Diskrepans:** Vår tremolo är **~4x för svag**. Beror på att formeln saknar waveform-peak-faktorn (255).

**Korrekt formel:**
```rust
pub fn from_param(param: u8) -> Self {
    // FT2: (waveform_peak * depth) >> 6, peak=255, volume_range=64
    // Vår waveform ger ±1.0 (peak=1.0), så: depth * 255/64 / 64
    Self((f32::from(param) * 255.0 / 64.0 / 64.0).clamp(0.0, 1.0))
}
```

**Effekt:** Tremolo-effekten (7xy) är nästan ohörbar. Sånger med tremolo för dramatisk volymvariation låter platt.

---

### 1.2 MEDEL: Vibrato-offset appliceras på tick 0

**Plats:** `tracker_effects.rs` — `current_modulation()` (rad ~1466-1478)

**Vår kod:** På tick 0 av en rad med vibrato beräknas vibrato-offset från föregående fas:
```rust
// current_modulation() - anropas på ALLA ticks inklusive tick 0
if self.vibrato_depth.as_f32() > 0.0 {
    let waveform_value = self.vibrato_waveform.sample(self.vibrato_phase.as_f32());
    let vibrato = waveform_value * self.vibrato_depth.as_f32();
    pitch_mod += PitchCents::new(vibrato);
}
```

**FT2 (ft2-clone):**
```c
// I getNewNote() (tick 0):
ch->outPeriod = ch->realPeriod;  // Vibrato-offset NOLLSTÄLLS på tick 0
// doVibrato() körs INTE på tick 0
```

**Diskrepans:** FT2 nollställer vibrato-offset på tick 0 av varje rad. Vår kod använder föregående ticks vibrato-fas, vilket ger ett litet pitch-fel vid början av varje rad.

**Effekt:** Subtil men hörbar — speciellt vid låga vibratohastigheter får varje rad en liten pitch-offset som inte ska vara där. Vid speed=6 körs vibrato 6 gånger istället för 5 per rad.

---

### 1.3 MEDEL: Volume Slide prioritetsregel felaktig

**Plats:** `tracker_effects.rs` — `SlideRate::from_volume_slide()` (rad 282-283)

**Vår kod:**
```rust
pub fn from_volume_slide(up: u8, down: u8) -> Self {
    Self((f32::from(up) - f32::from(down)) / 64.0)
}
```
- Om `up=15, down=15` → `(15-15)/64 = 0.0` (ingen slide)

**FT2 (ft2-clone):**
```c
if ((param & 0xF0) == 0) {
    newVol -= param;       // Slide DOWN (nedre nibble)
} else {
    param >>= 4;
    newVol += param;       // Slide UP (övre nibble) — PRIORITET
}
```
- Om `up=15, down=15` → UP tar prioritet, slide UP med 15

**Diskrepans:** FT2 ger **övre nibble (UP) prioritet** när båda är icke-noll. Vår kod subtraherar dem, vilket nollställer effekten.

**Effekt:** Ovanligt i praktiken (moduler bör inte ha båda icke-noll), men `AFx`-parametrar tolkas fel: FT2 tolkar som "slide up 15", vi tolkar som "slide up (15-x)".

**Samma bugg gäller:** `PanningSlide` (`from_panning_slide`, rad 287-288) — FT2 ger right-nibble (övre) prioritet.

---

## 2. SAKNADE EFFEKTER

### 2.1 Fine Portamento (E1x/E2x) — inte implementerad

**Beskrivning:** Slides pitch upp/ner med finare precision, appliceras **en gång på tick 0** (till skillnad från 1xx/2xx som körs varje tick 1+).

**FT2 (ft2-clone):**
```c
// E1x: Fine Porta Up — körs BARA på tick 0
ch->realPeriod -= param * 4;
ch->outPeriod = ch->realPeriod;
```

**Nuvarande status:** xmrs verkar rapportera fine portamento som vanlig `TrackEffect::Portamento`. Vår kod konverterar detta till `PortamentoUp`/`PortamentoDown` som körs på tick 1+ — **fel beteende** (borde köras en gång på tick 0).

**FT2 har separata minnesvariabler:** `fPitchSlideUpSpeed` / `fPitchSlideDownSpeed` (separata från 1xx/2xx).

**Effekt:** Fine portamento-effekter (E1x/E2x) körs varje tick istället för en gång — ~6x för stark slide vid speed=6.

### 2.2 Extra Fine Portamento (X1x/X2x) — inte implementerad

**Beskrivning:** Ännu finare portamento, **utan `* 4` multiplikator**. Appliceras en gång på tick 0.

**FT2:**
```c
// X1x: Raw period units, NO multiplication
ch->realPeriod -= param;  // (jämfört med E1x: param * 4)
```

**Nuvarande status:** Inte identifierad i xmrs `TrackEffect`-typer. Troligen droppad helt.

**Effekt:** X1x/X2x-kommandon ignoreras. Ovanligt men används i finjusterade moduler.

### 2.3 Global Volume (Gxx) — per-kanal istället för master

**Plats:** `import/tracker.rs` rad 1076-1079

**Nuvarande kod:**
```rust
GlobalEffect::Volume(vol) => {
    // Global volume - convert to SetVolume for now
    let v = (vol * 64.0).min(64.0) as u8;
    Some(EffectCommand::SetVolume(v))
}
```

**FT2:** Global volume påverkar **alla kanaler** samtidigt via master-multiplikator:
```c
int32_t vol = song.globalVolume * ch->outVol * ch->fadeoutVol;
```

**Nuvarande beteende:** Gxx ändrar bara den aktuella kanalens volym. Moduler som använder Gxx för fade-in/fade-out av hela mixen påverkar bara en kanal.

### 2.4 Global Volume Slide (Hxy) — inte implementerad

Inte hanterad i import-koden. XM Hxy-effekt ignoreras helt.

### 2.5 BPM Slide — inte implementerad

`GlobalEffect::BpmSlide(_)` returnerar `None` i import-koden (rad 1111).

---

## 3. KÄNDA BEGRÄNSNINGAR (xmrs-biblioteket)

### 3.1 Kombinerade effekter 5xy/6xy med param=0

**Problem:** XM 5xy (TonePorta+VolSlide) och 6xy (Vibrato+VolSlide) med param 0 ska betyda "fortsätt båda effekterna". xmrs droppar `VolumeSlide(0,0)` vid uppdelning.

**Effekt:** Volume slide-delen av 500/600 stoppas (korrekt sedan v0.91.0) istället för att fortsätta. Tone portamento/vibrato-delen fortsätter korrekt.

### 3.2 Fine Portamento saknar fine-flagga

xmrs `TrackEffect::Portamento(speed)` har ingen `fine`-flagga (till skillnad från `VolumeSlide { speed, fine }`). Fine portamento (E1x/E2x) kan inte särskiljas från vanlig portamento (1xx/2xx).

---

## 4. MINDRE AVVIKELSER

### 4.1 Vibrato/Tremolo Ramp-waveform skiljer sig från FT2

**Vår kod:**
```rust
Self::Ramp => 2.0 * phase - 1.0,  // Linjär -1.0 → +1.0
```

**FT2:** Ramp-waveform använder `(table_index << 3)` med separat sign-check. Ger en sawtooth som går 0→+248 i positiva halvan, -255→-7 i negativa. Annorlunda form än vår linjära ramp.

**Effekt:** Subtil klangskillnad vid ramp-vibrato/tremolo (ovanligt; de flesta moduler använder sine).

### 4.2 FT2 Tremolo Ramp-bugg inte replikerad

**FT2-bugg:** I ramp-waveform för tremolo använder FT2 `ch->vibratoPos` istället för `ch->tremoloPos` för sign-kontroll. Vår kod använder korrekt `tremolo_phase`.

**Effekt:** Tremolo med ramp-waveform låter annorlunda från FT2 (bara för ramp, inte sine/square).

### 4.3 Set Panning (8xx) lätt asymmetrisk

**Vår kod:** `(pan / 127.5) - 1.0`
- pan=128: ger +0.0039 (inte exakt 0.0)

**FT2:** `ch->outPan = param` (0-255 direkt, center=128)

**Effekt:** Negligerbart — 0.39% offset från center.

### 4.4 Fas-wrapping använder if istället för modulo

**Plats:** `process_tick()` — vibrato/tremolo fas-avancering

```rust
self.vibrato_phase = Phase::new(if new_phase >= 1.0 {
    new_phase - 1.0
} else {
    new_phase
});
```

**Bättre:** `(new_phase % 1.0)` — mer robust om fas-increment skulle överstiga 1.0.

**Effekt:** Ingen i praktiken (max increment = 15/64 ≈ 0.234).

---

## 5. KORREKTA IMPLEMENTATIONER (verifierade mot FT2)

### 5.1 Vibrato (4xy) — djup och hastighet ✓

**Djup:**
- Vår: `depth * (255/32 * 1200/768)` cents ≈ `depth * 12.45` cents
- FT2: `(255 * depth) >> 5` period-enheter × 1200/768 cents/period
- Max (depth=15): 186.7 cents ≈ 1.87 semitoner ✓ MATCHAR

**Hastighet:**
- Vår: fas-increment = `speed / 64.0` per tick
- FT2: `vibratoSpeed = raw_speed * 4`, position avanceras med speed (uint8_t wraps vid 256)
- Full cykel vid speed=1: 64 ticks i båda ✓ MATCHAR

### 5.2 Portamento Up/Down (1xx/2xx) ✓

**Linjärt läge:** `speed * 4.0 * 1200/768` cents per tick ✓
**Amiga-läge:** Period-baserad slide med korrekt konvertering ✓
**Riktning:** Up = minska period (högre pitch), Down = öka period ✓

### 5.3 Tone Portamento (3xx) ✓

- Linjärt: glide i semiton-rymd med korrekt hastighet ✓
- Amiga: glide i period-rymd med `/4.0` korrektion ✓
- Target sätts från not, trigger undertrycks (early return i trigger_note) ✓
- `tone_porta_active` skiljer aktiv rad från minnesvärde (v0.91.0) ✓

### 5.4 Volume Slide (Axy) ✓

- Per-tick applicering (tick 1+) ✓
- Effekt-minne (A00 = fortsätt) ✓
- Volym clampas till 0.0-1.0 ✓
- ⚠ Prioritetsbugg (se 1.3)

### 5.5 Fine Volume Slide (EAx/EBx) ✓

- Appliceras en gång på tick 0 ✓
- Effekt-minne ✓
- Inte applicerad på tick 1+ ✓

### 5.6 Arpeggio (0xy) ✓

- FT2-ordning: base → y → x (reversed från ProTracker) ✓
- Modulo-3 tick cycling ✓
- Offset i semitoner × 100 cents ✓

### 5.7 Set Volume (Cxx) ✓

- Range 0-64 → 0.0-1.0 ✓

### 5.8 Set Panning (8xx) ✓

- Range 0-255 → -1.0 till 1.0 ✓ (med minimal avrundning, se 4.3)

### 5.9 Note Cut (ECx) ✓

- Volume → 0 vid specifik tick ✓
- Cleared efter utlösning ✓

### 5.10 Note Delay (EDx) ✓

- Trigger fördröjd till specifik tick ✓
- Cleared efter utlösning ✓

### 5.11 Retrigger (E9x) ✓

- Tick-baserad omtriggering ✓
- Volymändring per trigger ✓

### 5.12 Sample Offset (9xx) ✓

- Offset × 256 → normaliserad position ✓
- xmrs-konvertering korrekt ✓

### 5.13 Waveform-sampling (Sine/Square) ✓

- Sine: `sin(phase × 2π)` ✓
- Square: +1.0 för 0-50%, -1.0 för 50-100% ✓
- Random: xorshift32 per tick (FT2-beteende) ✓

### 5.14 Amiga Period-konvertering ✓

- `period = 7680 - (semitones - 12) × 64` ✓
- Invers: `semitones = (7680 - period) / 64 + 12` ✓

### 5.15 XM Import-pipeline ✓

- Portamento speed-konvertering (÷4 reversal) ✓ (fixad v0.90.0)
- Vibrato speed/depth recovery (×64/×16) ✓
- Volume/panning-konvertering ✓
- Note pitch +12 offset (tracker → MIDI) ✓
- Sample finetune baked into sample rate ✓
- Loop info preserved exakt ✓

---

## 6. PRIORITERAD ÅTGÄRDSLISTA

### Hög prioritet (märkbar ljudpåverkan)
1. **Fix tremolo-djup** — `TremoloDepth::from_param` ska ge ~4x starkare effekt
2. **Fix vibrato tick 0** — nollställ vibrato-offset på tick 0 (inte använda föregående fas)
3. **Fix volume slide-prioritet** — övre nibble (UP) ska ta prioritet

### Medel prioritet (korrekthet)
4. **Implementera Fine Portamento** — ny `EffectCommand::FinePortamento{Up,Down}` som körs en gång på tick 0
5. **Implementera Global Volume** — `GlobalCommand::SetGlobalVolume` som multiplicerar alla kanalers volym

### Låg prioritet (fullständighet)
6. **Fix ramp-waveform** — matcha FT2:s sawtooth-form
7. **Implementera Extra Fine Portamento (X1x/X2x)**
8. **Implementera Global Volume Slide (Hxy)**
9. **Implementera BPM Slide**

---

## 7. VERIFIERINGSMETOD

Denna analys baseras på:

1. **ft2-clone källkod** (8bitbubsy) — direkt C-port av FT2-replayern
   - `ft2_replayer.c`: effektprocessering, tick-hantering
   - `ft2_tables.c`: vibrato-sinetabell, arpeggio-tabell

2. **OpenMPT testfall** — systematiska edge case-tester
   - https://wiki.openmpt.org/Development:_Test_Cases/XM

3. **MilkyTracker dokumentation** — effektbeskrivningar och beteende

4. **Kodjämförelse** — rad-för-rad analys av alla effekt-handlers i vår kod

Se `docs/references/ft2-effect-reference.md` för fullständig FT2-effektreferens med exakta formler och C-kod.
