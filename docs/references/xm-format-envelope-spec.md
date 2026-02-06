# XM Format Specification: Envelope & Instrument Reference

Sammanställt från "The Complete XM module format specification v0.81" av Matti "ccr" Hamalainen (TNSP, 2000-2001) samt ft2-clone av 8bitbubsy.

**Källor:**
- https://gist.github.com/loveemu/737ace92f08b439a416adc829ae2aa76
- https://github.com/8bitbubsy/ft2-clone
- https://ftp.modland.com/pub/documents/format_documentation/FastTracker%202%20v2.04%20(.xm).html

---

## Instrument Header (relevanta fält)

| Offset | Storlek | Typ | Fält |
|--------|---------|-----|------|
| +129 | 48 | byte[48] | Volume envelope points (12 punkter x 4 bytes) |
| +177 | 48 | byte[48] | Panning envelope points (12 punkter x 4 bytes) |
| +225 | 1 | byte | Antal volume envelope-punkter (max 12) |
| +226 | 1 | byte | Antal panning envelope-punkter (max 12) |
| +227 | 1 | byte | Volume sustain point (index) |
| +228 | 1 | byte | Volume loop start (index) |
| +229 | 1 | byte | Volume loop end (index) |
| +230 | 1 | byte | Panning sustain point (index) |
| +231 | 1 | byte | Panning loop start (index) |
| +232 | 1 | byte | Panning loop end (index) |
| +233 | 1 | byte | Volume type (bitflaggor) |
| +234 | 1 | byte | Panning type (bitflaggor) |
| +239 | 2 | word | Volume fadeout (0..4095) |

### Envelope Type Flags (offset +233/+234)

| Bit | Betydelse |
|-----|-----------|
| 0 | Envelope on/off (1 = aktiverad) |
| 1 | Sustain aktiverad |
| 2 | Loop aktiverad |

---

## Envelope Point Format

Varje envelope-punkt = 4 bytes:

| Offset | Storlek | Typ | Fält |
|--------|---------|-----|------|
| 0 | 2 | word (u16) | Frame-nummer (X, 0..65535, FT2 begränsar till 0..255) |
| 2 | 2 | word (u16) | Värde (Y, 0..64 för volume, 0..64 för panning) |

- **Max 12 punkter** per envelope (XM-format)
- **Första punktens frame MÅSTE vara 0**
- **Linjär interpolering** mellan punkter

### Interpoleringsalgoritm

```
point_distance = next_point.frame - current_point.frame
value_delta = next_point.value - current_point.value
frame_offset = current_frame - current_point.frame
output = current_point.value + ((value_delta * frame_offset) / point_distance)
```

---

## Fadeout-mekanism

### Algoritm (FT2)

```c
// Vid note trigger:
fadeoutVol = 32768;  // Startvärde

// Varje tick EFTER key-off:
fadeoutVol -= instrument.fadeout;  // LINJÄR subtraktion
if (fadeoutVol < 0) fadeoutVol = 0;

// I volymberäkningen:
FinalVol = (FadeOutVol / 32768) * (EnvelopeVol / 64) * (GlobalVol / 64) * (Vol / 64);
```

### Viktiga regler

1. **Fadeout är LINJÄR** - en fast mängd subtraheras varje tick, INTE multiplikativ/exponentiell
2. **FadeoutVol startar vid 32768** (normaliserat: 1.0)
3. **Fadeout processas INTE om volume envelope är inaktiverad** - fadeoutVol förblir 32768
4. **Fadeout startar vid key-off** och fortsätter parallellt med envelope-avancering
5. **Fadeout-värde 0..4095** i XM-filen, 0 = ingen fadeout
6. **Tid till tystnad** = 32768 / fadeout ticks

### Exempel

| Fadeout-värde | Ticks till tystnad | Tid vid 125 BPM (50 Hz) |
|---------------|-------------------|--------------------------|
| 4096 | 8 | 160 ms |
| 2048 | 16 | 320 ms |
| 1024 | 32 | 640 ms |
| 512 | 64 | 1.28 s |
| 256 | 128 | 2.56 s |
| 128 | 256 | 5.12 s |

---

## Sustain Point

- Sustain point = ett **index** i envelope-punkt-arrayen (inte en frame-position)
- När envelope-positionen når sustain-punkten, **pausar envelopet** vid den punktens värde
- Envelopet **återupptas** efter key-off (note release)
- **Sustain har prioritet över loop** - om sustain-punkten = loop end och noten släpps, slutar loopningen

### FT2-algoritm (från ft2_replayer.c)

```c
// Sustain check (bara om noten fortfarande hålls):
if ((ins->volEnvFlags & ENV_SUSTAIN) && !ch->keyOff) {
    if (envPos-1 == ins->volEnvSustain) {
        envPos--;
        ch->fVolEnvDelta = 0.0f;  // Stoppa interpolering
    }
}
```

---

## Loop Region

- Loop start och loop end = **index** i punkt-arrayen
- När envelope-positionen passerar loop end, hoppar den till loop start
- **Loopning slutar** om sustain-punkten = loop end OCH noten har släppts

### FT2-algoritm

```c
if (ins->volEnvFlags & ENV_LOOP) {
    envPos--;
    if (envPos == ins->volEnvLoopEnd) {
        if (!(ins->volEnvFlags & ENV_SUSTAIN) ||
            envPos != ins->volEnvSustain || !ch->keyOff) {
            envPos = ins->volEnvLoopStart;
            ch->volEnvTick = ins->volEnvPoints[envPos][0];
        }
    }
    envPos++;
}
```

---

## Envelope-processing timing

- Envelopes processas **en gång per tick** (inte per rad, inte per sample)
- Tick-rate = `BPM * 2 / 5` Hz (50 Hz vid 125 BPM)
- Envelope-positionen inkrementeras **FÖRE** evaluering (vanlig källa till off-by-one-buggar)
- Vid BPM-ändring ändras tick-rate, vilket påverkar envelope-hastighet

---

## Viktiga edge cases

1. **Envelope position inkrementeras före evaluering** (`EnvLoops.xm` testfall)
2. **NoteOff utan volume envelope** → volymen sätts till 0 omedelbart
3. **Instrument-nummer efter NoteOff** → återställer NoteOff-status
4. **Portamento med key-off** → KeyOff/fadeout-flaggor ska INTE återställas
5. **NoteDelay + NoteOff** → envelopes retriggras
6. **Lxx (set env position)** → FT2-bugg: sätter panning-position bara om volume sustain-flagga är satt
7. **Retrigger + fadeout** → fadeout kvarstår trots retrigger om key-off + instrument + retrigger på samma rad
