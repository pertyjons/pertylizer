# Debug: Tracker Playback (XM/MOD/S3M)

Guide for att debugga tracker-uppspelningsproblem. Uppdateras löpande.

## Snabbguide: Debug-verktyg

### 1. `debug_playback` - Tick-för-tick uppspelningslogg (PRIMÄRT VERKTYG)

```bash
cargo run --example debug_playback -- <fil> [position] [kanaler]
```

**Exempel:**
```bash
# Position 19, kanaler 26,27,28
cargo run --example debug_playback -- /home/per/Musik/joli_untouched.xm 19 26,27,28

# Filtrera specifika rader med grep
cargo run --example debug_playback -- fil.xm 19 26 2>&1 | grep "row 08\|row 09"
```

**Output-format:**
```
--- Tick 244800 (row 08, tracker tick 0) ---
  NoteOn  voice=26 pitch=C-6 vel=1.000 inst=27 effects=[PortamentoDown(5)]
  Mod     T26 vol=0.6406 pitch=-31.25ct pan=+0.004 [TRIG]
```

- `Tick NNNN`: Absolut song tick
- `row XX`: Hexadecimal radnummer i pattern
- `tracker tick N`: Tick inom raden (0 = rad-start, 1-4 = effekt-ticks vid speed=5)
- `pitch=+X.XXct`: Pitch-offset i cent (100ct = 1 halvton)
- `[TRIG]`: Not triggades denna tick
- `[CUT]`: Not klipptes

**Viktigt om tick-labelling:** Debug-scriptet beräknar `tracker_tick = tick_in_row / 40`. Första `process_tick()` sker 39 song ticks efter rad-start, men labellas fortfarande "tracker tick 0" (39/40=0 heltalsdivision). Effekter appliceras vid "tick 0 second" (den andra emissionen med samma label).

### 2. `analyze_tracker` - Intern representation

```bash
cargo run --example analyze_tracker -- fil.xm
```

Visar Cell/EffectCommand efter import. Jämför med `analyze_tracker_raw` för att verifiera import.

### 3. `analyze_tracker_raw` - Rå xmrs-data

```bash
cargo run --example analyze_tracker_raw -- fil.xm
```

Visar xmrs PatternSlot/TrackUnit FÖRE import. Användbart för att verifiera att importen inte tappar eller felkonverterar effekter.

### 4. `analyze_xm_detailed` - Instrument/sample-info

```bash
cargo run --example analyze_xm_detailed -- fil.xm
```

Visar instrument, samples, envelopes, loop-inställningar, volym, finetune.

### 5. GUI Debug-knapp

Klicka **Debug** i Sequencer-vyn. Skriver pattern-grid, mute-status, instrument defaults, effektsammanfattning till konsolen.

---

## Arkitektur: Pitch-system

### Dubbla pitch-variabler (KRITISKT att förstå)

```
ChannelEffectState:
  current_pitch (Semitones)  - Basnot/ton-portamento target
  pitch_offset  (PitchCents) - Portamento up/down, vibrato, arpeggio

current_modulation() returnerar:
  pitch_cents     = pitch_offset + fine_tune + vibrato + arpeggio
  tone_porta_pitch = Some(current_pitch) om tone porta aktiv
```

I FT2 finns bara EN periodvariabel. Vår kod har TVÅ. Detta orsakar buggar när effekter interagerar:

- **PortamentoUp/Down (1xx/2xx)**: Modifierar `pitch_offset` (via `apply_amiga_portamento()`)
- **TonePortamento (3xx/5xx)**: Modifierar `current_pitch` (via `apply_amiga_tone_portamento()`)
- **Vibrato (4xx)**: Beräknas i `current_modulation()`, modifierar varken pitch_offset eller current_pitch

### Voice-pitch beräkning

```
I Voice::process_audio():
  1. base_freq = tone_porta_pitch ? semitones_to_hz(tone_porta) : glide.get_frequency()
  2. freq = base_freq * 2^(pitch_bend / 12) * 2^(pitch_cents / 100 / 12)
```

---

## Arkitektur: Tick-system

### Song ticks vs Tracker ticks

```
Song tick:    Intern klocka, ~24 ticks per beat (TICKS_PER_QUARTER=960, vid 120 BPM)
Tracker tick: En FT2-tick. 40 song ticks = 1 tracker tick.
Row:          speed * 40 song ticks (speed=5 -> 200 song ticks per rad)
```

### Processerings-ordning per rad

```
1. collect_events_at_tick() -> process_row_start() [tick 0]
   - Sätter upp effekt-state (portamento direction, vibrato depth, etc.)
   - Emittar tick-0 modulation (volym, pitch, panning)
   - Triggar not om applicable

2. process_tick() x (speed-1) [ticks 1 till speed-1]
   - Applicerar kontinuerliga effekter (portamento, vibrato, volume slide, etc.)
   - Emittar modulation-events

3. Nästa rad: tillbaka till steg 1
```

**VIKTIGT:** `process_tick()` anropas av SequencerEngine var 40:e song tick via `effect_tick_accumulator`. Ackumulatorn synkar INTE med rad-gränser utan kör kontinuerligt. Det totala antalet anrop per rad = speed, men tick 0 hanteras av `process_row_start()`, så `process_tick()` skippar effekter om `tick_in_row >= speed` (fixat i v0.96.0).

---

## Vanliga bugg-mönster

### 1. Effekt appliceras på tick 0 (ska bara vara tick 1+)

**Symptom:** Effekten är ~25% starkare per rad.
**Kontroll:** I `process_tick()`, alla kontinuerliga effekter ska ha `tick.as_u8() > 0`.
**Historik:** Vibrato-offset på tick 0 fixat i v0.92.0. Extra process_tick-anrop fixat i v0.96.0.

### 2. pitch_offset läcker mellan effekter

**Symptom:** Not börjar falskt efter effektbyte (t.ex. PortamentoUp -> TonePortamento).
**Orsak:** `pitch_offset` ackumuleras av PortamentoUp/Down men absorberas inte av TonePortamento.
**Fix:** I `process_row_start()`, absorbera pitch_offset in i current_pitch när TonePortamento börjar (v0.95.0).

### 3. Effekt-target försenat en rad

**Symptom:** Första raden med TonePortamento ger ingen tonändring.
**Orsak:** `last_porta_target` uppdaterades EFTER effekt-processering i importkoden.
**Fix:** Beräkna pitch och uppdatera `last_porta_target` FÖRE effektloopen (v0.94.0).

### 4. Amiga-periodberäkning felaktig

**Symptom:** Portamento-hastighet i cent matchar inte FT2.
**Formel:**
```
Period = 7680 - (semitones - 12) * 64
Semitones = (7680 - period) / 64 + 12
PortamentoUp: period -= speed * 4     (speed=raw param)
PortamentoDown: period += speed * 4
TonePortamento: period ±= speed       (UTAN *4 multiplikator!)
```

---

## Debug-process (steg-för-steg)

### 1. Identifiera problemet

- Vilka kanaler? (T26, T27, T28...)
- Vilket instrument? (inst=0x27...)
- Vilka positioner i arrangementet? (pos 17-20...)
- Vilken typ av problem? (Falskt/surt, för snabbt/långsamt, klick, tyst...)

### 2. Kör debug_playback

```bash
cargo run --example debug_playback -- fil.xm <position> <kanaler>
```

### 3. Analysera output

Saker att leta efter:
- **Pitch drift:** Ökar/minskar pitch_offset mer än förväntat per rad?
- **Oväntad pitch på tick 0:** Borde pitch vara 0.00ct vid rad-start men är det inte?
- **Antal effekt-steg per rad:** Räkna hur många gånger pitch ändras. Bör vara (speed-1) gånger.
- **Volym:** Stämmer volym-nivåer? Volume slide korrekt hastighet?
- **[TRIG] vid rätt tidpunkt:** Triggras noter korrekt?

### 4. Jämför med referens

- **FT2-clone** eller **MilkyTracker** för auditiv referens
- `analyze_tracker` vs `analyze_tracker_raw` för import-verifiering
- FT2-specifikation i `docs/references/`

### 5. Räkna förväntat vs faktiskt

**Portamento-beräkning (Amiga mode):**
```
speed_periods = raw_param * 4
period = 7680 - (note_semitones - 12) * 64
new_period = period ± speed_periods
new_semitones = (7680 - new_period) / 64 + 12
pitch_offset_cents = (new_semitones - base_note) * 100
```

**Per rad (speed N):**
```
Antal effekt-ticks = N - 1
Total pitch-ändring = pitch_per_tick * (N - 1)
```

---

## Nyckel-filer

| Fil | Innehåll |
|-----|----------|
| `crates/synth_engine/src/tracker_effects.rs` | Effekt-processering (process_row_start, process_tick, apply_amiga_portamento, etc.) |
| `crates/synth_engine/src/sequencer_engine.rs` | Sequencer-main loop (tick-ackumulator, event-emittering) |
| `crates/synth_engine/src/voice.rs` | Voice pitch-beräkning (base_freq, pitch_bend, tracker modulation) |
| `crates/modular_synth/src/io/import/tracker.rs` | XM/MOD import (process_track_unit_to_cell, convert_track_effect_for_tracker) |
| `crates/synth_modules/src/sample_player.rs` | Sample playback rate (base_speed * 2^(pitch_mod/12)) |

---

## Kända fixade buggar (referens)

| Version | Problem | Orsak |
|---------|---------|-------|
| 0.96.0 | 25% för mycket effekt per rad | process_tick() anropades speed gånger istället för speed-1 |
| 0.95.0 | pitch_offset läcker till TonePortamento | Separata pitch-variabler inte synkade vid effektbyte |
| 0.94.0 | TonePortamento target en rad försenad | last_porta_target uppdaterades efter effektloop |
| 0.92.0 | Vibrato på tick 0, volume slide prioritet | FT2-beteende ej korrekt implementerat |
| 0.91.0 | Kontinuerliga effekter läcker mellan rader | Effekter inte nollställda vid ny rad |
| 0.90.0 | Portamento ~41x för snabb, inverterad | Amiga period-konvertering felaktig |

---

## XM Effekt-referens (vanliga)

| Hex | Namn | Tick 0 | Ticks 1+ | Modifierar |
|-----|------|--------|----------|------------|
| 1xx | PortamentoUp | Nej | period -= x*4 | pitch_offset |
| 2xx | PortamentoDown | Nej | period += x*4 | pitch_offset |
| 3xx | TonePortamento | Sätt target | period -> target ±x | current_pitch |
| 4xy | Vibrato | Nej | pitch oscillation | beräknas i current_modulation |
| 5xx | TonePorta+VolSlide | Sätt target | porta + vol slide | current_pitch + volume |
| 6xx | Vibrato+VolSlide | Nej | vibrato + vol slide | pitch + volume |
| Axy | VolumeSlide | Nej | vol ± slide rate | volume |
| EBx | FineVolSlide Down | vol -= x | Nej | volume |
| EAx | FineVolSlide Up | vol += x | Nej | volume |
