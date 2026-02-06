# Analys: Tracker-uppspelning - Arkitekturella problem

**Datum:** 2026-02-06
**Kontext:** Efter v0.80.0 och v0.81.0 (totalt 24+ buggfixar) kvarstår uppspelningsproblem med tracker-filer (XM/MOD/S3M). Denna analys granskar hela kedjan från tracker-kommandon till ljudutgång.

---

## Sammanfattning

Problemen ligger inte i enskilda effektberäkningar (de är mestadels korrekta) utan i **hur synth-motorn konsumerar sequencer-händelser**. Det finns ett arkitekturellt grundproblem plus flera sekundära buggar.

---

## Kritiska synpunkter (granskade)

Synpunkter som lagts till efter initial analys, granskade mot källkoden 2026-02-06.

### 1. Antagandet om fast bufferstorlek (512)

> *Exemplet använder 512 samples. Maxfelet är inte "±buffer_size/2" utan upp till `buffer_size - 1` samples.*

**Berättigad.** Maxfelet är `buffer_size - 1` samples, inte `±buffer_size/2`. Om en event ska ske vid sista samplet i bufferten men appliceras vid första, är felet hela bufferten minus ett sample. Vid 512 samples/48kHz = upp till ~10.6ms, vid 2048 samples = ~42.6ms. Exemplen i analysen är korrigerade nedan.

### 2. Tick-positioner vs sample-offsets är oklara

> *Analysen antar att sequencer_engine redan räknar fram exakta sample-offsets. Om den bara genererar tick-index måste steget "tick → sample" designas först.*

**Berättigad.** Verifierat: `SequencerEvent` bär fältet `tick: Tick` som är en **absolut song-position** (`Tick(u64)`, definierad i `time.rs:11-15`), dokumenterad som "Absolute position on the song timeline". Det finns **inga sample-offsets** i event-strukturen. Sequencern beräknar inte var i bufferten en event ska appliceras — den genererar bara absoluta tick-index. Exemplet "tick vid sample 256" var illustrativt, inte baserat på faktisk data. En process-per-tick-lösning skulle **eliminera behovet** av sample-offsets helt, eftersom events appliceras vid tick-gränser och rendering sker mellan dem.

### 3. SetSpeed-exemplet blandar begrepp

> *Scenariot använder "Tick 240" som om tick är en global räknare. Om `current_tick` redan är modulo kan exemplet vara missvisande.*

**Delvis berättigad — men buggen är bekräftad.** `current_tick` ÄR en absolut global räknare (`sequencer_engine.rs:63`, `Tick(u64)`). Den räknas upp med 1 per iteration (`rad 368`). Buggen är reell men mekanismen är rad-överhoppning, inte "för tidig row-advance". Exakt spårning:

- Tick 240: `pattern_tick = 240`, `240 % 240 = 0` → rad 1 processas, SetSpeed(3) ändrar `ticks_per_row` till 120
- Tick 241-359: Inget av dessa är delbart med 120 förutom tick 360
- Tick 360: `pattern_tick = 360`, `360 % 120 = 0` → rad-gräns. `row_idx = 360 / 120 = 3`
- **Rad 2 hoppas över helt** — den borde ha processats men `row_idx` beräknas retroaktivt med nya `ticks_per_row`

I riktiga XM-trackers tar SetSpeed (Fxx) effekt omedelbart men **nuvarande rad slutförs med gammal timing**. Nästa rad börjar med ny speed. Vår implementation omtolkar retroaktivt alla radpositioner med det nya värdet. Scenariot i sektion 3 nedan är korrigerat.

### 4. `effect_tick_accumulator` kan vara avsiktligt kontinuerlig

> *Att nollställa vid PatternJump/Break är inte självklart korrekt; det måste styras av målspelarens semantik.*

**Helt berättigad — den ursprungliga analysen var felaktig.** Verifierat: `effect_tick_accumulator` är en **ren klockdelare** (`sequencer_engine.rs:93-95`) som räknar song-ticks (0-40) för att avfyra `process_tick()` vid tracker-tick-intervall. Den har ingen musikalisk semantik. De faktiska effektfaserna (vibrato, tremolo) lagras separat i `ChannelEffectState` och nollställs korrekt av `trigger_note()` vid ny not (`tracker_effects.rs:1143, 1149`). Att INTE nollställa klockdelaren vid pattern-hopp är **korrekt beteende**. ~~Sektion 4 nedan är borttagen.~~

### 5. NoteOn/Modulation-ordning kan vara icke-problem

> *Om `note_on()` inte "låser in" volym/pan finns ingen glitch.*

**Helt berättigad — den ursprungliga analysen var felaktig.** Verifierat: `note_on()` (`voice.rs:414-440`) rör **inte** fälten `tracker_volume`, `tracker_panning` eller `tracker_pitch_cents`. Dessa fält läses först under `process_audio()` (`voice.rs:626-644`). Eftersom både NoteOn och Modulation appliceras före rendering, har Modulation-värdena redan skrivits korrekt när `process_audio()` körs. Det finns ingen glitch. ~~Sektion 5 nedan är borttagen.~~

### 6. "Process-per-tick löser alla problem" är för starkt

> *Det fixar timing, men inte ignorerade effekter, per-kanal state, eller tempo-drift p.g.a. icke-heltaliga samples per tick.*

**Berättigad.** Process-per-tick löser timing-problemen (grundproblemet + SetSpeed-buggen + modulation-granularitet) men inte:
- Tyst ignorerade effekter (Tremor, Panbrello, etc.)
- Tempo-drift: antal samples per tick är sällan heltal (t.ex. 48000/(125*24/60) ≈ 960 exakt vid 125 BPM, men vid 130 BPM ≈ 923.08). En fractional sample-ackumulator krävs.
- Eventuella per-kanal state-problem som inte är timing-relaterade

Rekommendationen nedan är nyanserad.

### 7. Prestanda-överhead är en hypotes

> *I en nodgraf med per-anrop setup kan overhead bli betydande. Bör mätas.*

**Berättigad.** Varje `process_voices()`-anrop har fast overhead för att iterera instruments, voices, och modulgrafen. Vid typiskt 2-5 ticks per buffer (speed 6, 125 BPM, 512 samples) multipliceras denna overhead 2-5x. Om overhead per anrop är ~10μs och buffer-budget ~10ms (512@48kHz) är det ~50μs = 0.5% — troligen hanterbart men **bör mätas med profiler** innan beslut. Modulgrafer med många noder kan ha högre fast overhead.

---

## 1. GRUNDPROBLEMET: Ingen sample-accuracy i event-hanteringen

**Filer:** `synth_engine.rs:1660-1833`, `sequencer_engine.rs:321-384`

### Nuvarande flöde

```
1. sequencer.process(512 samples)  → genererar ALLA events för hela bufferten
2. for event in events             → applicerar ALLA events OMEDELBART (rad 1675)
3. process_voices()                → renderar 512 samples med de statiska värdena
```

### Problemet

Sequencern genererar events med exakta tick-positioner (`sequencer_engine.rs:354`), men synth-motorn **ignorerar tick-tidsstämplarna helt**. Alla events appliceras vid sample 0 i bufferten, oavsett var i bufferten de egentligen ska ske.

### Konkret exempel

Vid 48kHz, 512 samples/buffer, 125 BPM:
- En buffer ≈ 10.7ms
- En tracker-tick (vid speed 6) ≈ 20ms
- Sequencern genererar events med absoluta `Tick(u64)`-positioner, men synth-motorn konverterar aldrig dessa till sample-offsets inom bufferten
- Om 2-3 song-ticks infaller i samma buffer → alla events kollapsar till buffer-start
- Vid lägre speed eller högre tempo kan fler tracker-ticks hamna i samma buffer, och bara sista modulationsvärdet gäller
- **Maximal timing-jitter:** `buffer_size - 1` samples = upp till ~10.6ms vid 512 samples, ~42.6ms vid 2048 samples

### Effekt på uppspelning

| Effekt | Förväntat beteende | Faktiskt beteende |
|--------|-------------------|-------------------|
| **Vibrato** | Smidig sinusvåg per tick | Staircase-trappsteg per buffer |
| **Arpeggio** | 3 toner växlar varje tick | Kollapsar om 2+ ticks i samma buffer |
| **Retrigger (Rxx)** | Exakt tick-interval | Kvantiseras till buffer-gränser |
| **Note Delay (EDx)** | Fördröjer not inom rad | Kan inte fördröja inom en buffer |
| **Portamento** | Smidig pitch-slide per tick | Uppdateras i ~10ms-hopp |
| **Volume slide** | Stegvis per tick | Sista värdet per buffer gäller |

### Varför det är ett arkitekturproblem

En riktig tracker (eller tracker-emulator som libxm/libopenmpt) bearbetar **en tick åt gången** och renderar exakt rätt antal samples per tick. Vår arkitektur bearbetar **en buffer åt gången** (synth-sättet), vilket ger timing-jitter upp till `buffer_size - 1` samples (hela bufferten minus ett sample i värsta fall).

### Bevis i koden

```rust
// synth_engine.rs:1675 - Alla events processas i en loop FÖRE rendering
for event in &self.sequencer_event_buffer {
    match event {
        SequencerEvent::NoteOn { .. } => {
            // Appliceras omedelbart - ingen sample-offset
            target.allocator_mut().note_on_fixed_voice(*v_idx, note, vel);
        }
        SequencerEvent::Modulation { .. } => {
            // Skriver över värden - bara sista per buffer gäller
            voice.tracker_pitch_cents = *pitch_cents;
            voice.tracker_volume = *volume;
            voice.tracker_panning = *panning;
        }
    }
}

// SEDAN renderas alla samples med de statiska värdena
self.process_voices(&process_context);
```

---

## 2. Modulation sätts som fast värde för hela bufferten

**Fil:** `voice.rs:570-644`

Även om sequencern genererar flera modulations-events per buffer, skrivs `tracker_pitch_cents`, `tracker_volume`, `tracker_panning` över av varje event (`synth_engine.rs:1791-1793`). Bara det **sista** värdet gäller för hela bufferten.

```rust
// voice.rs:573 - Pitch sätts EN GÅNG innan process()
let tracker_semitones = Semitones::new(self.tracker_pitch_cents.as_f32() / 100.0);
self.graph.set_oscillator_frequency(freq);

// voice.rs:641-643 - Volume/pan appliceras med samma värde på ALLA samples
for i in 0..samples.as_usize() {
    left_out[i] *= amp_scale * pan_l;   // Samma amp_scale hela bufferten
    right_out[i] *= amp_scale * pan_r;
}
```

---

## 3. SetSpeed orsakar rad-överhoppning

**Fil:** `sequencer_engine.rs:710-717, 398, 968-972`

När `Fxx` (SetSpeed) exekveras ändras `current_ticks_per_row` omedelbart (`rad 715-716`). Rad-index beräknas sedan retroaktivt med `row_idx = pattern_tick / ticks_per_row` (`rad 972`), vilket omtolkar hela pattern-tidslinjen med det nya värdet.

**Exakt spårning (pattern startande vid tick 0, initial speed 6):**
- Tick 240: `pattern_tick = 240`, `240 % 240 = 0` → rad 1 processas, SetSpeed(3) ändrar `ticks_per_row` till 120
- Tick 241-359: Inget av dessa är delbart med 120 (nästa gräns vid 360)
- Tick 360: `pattern_tick = 360`, `360 % 120 = 0` → rad-gräns. `row_idx = 360 / 120 = 3`
- **Rad 2 hoppas helt över** — den existerar i patternet men processas aldrig

**Grundorsak:** `row_idx = pattern_tick / ticks_per_row` (`rad 972`) omtolkar retroaktivt alla tick-positioner med nya `ticks_per_row`. I riktiga XM-trackers tar SetSpeed effekt omedelbart men nuvarande rad slutförs med den gamla timingen, och rad-index räknas inkrementellt (inte via division).

---

## 4. Pattern Delay kan orsaka dubbel-processning

**Fil:** `sequencer_engine.rs:980-988`

Row-dedupliceringslogiken (`last_row_index`) uppdateras inte under pattern delay. När delayen löper ut kan samma rad processas igen.

```rust
// Problemet: last_row_index uppdateras INTE under delay
if self.pattern_delay_rows_remaining > 0 {
    self.pattern_delay_rows_remaining -= 1;
    return;  // Returnerar INNAN last_row_index sätts
}
self.last_row_index = Some(row_idx);  // Nås aldrig under delay
```

---

## Jämförelse: Tracker-motor vs Synth-motor

| Aspekt | Riktig tracker | Vår synth-motor |
|--------|---------------|-----------------|
| Process-enhet | Per tick (~2.6ms) | Per buffer (~10ms) |
| Event-timing | Sample-exakt | Buffer-start |
| Modulation | Uppdateras per tick | Sista värdet per buffer |
| Arpeggio | 3 toner per 3 ticks | Kan kollapsa till 1 |
| Retrigger | Exakt tick-interval | Kvantiserad |
| Antal samples/enhet | `sample_rate * 60 / (bpm * 24)` | Buffer size (512) |

---

## Rekommenderad lösning: Process-per-tick

Istället för att processa hela bufferten på en gång, dela upp den i segment per tick:

```
process(buffer_size):
    remaining = buffer_size
    buffer_offset = 0

    while remaining > 0:
        // Beräkna samples till nästa tick
        samples_to_next_tick = calculate_samples_to_next_tick()
        chunk_size = min(samples_to_next_tick, remaining)

        // Rendera ljud för detta segment
        render_voices(buffer[offset..offset+chunk_size])

        // Avancera till nästa tick och applicera events
        advance_tick()
        apply_events_at_current_tick()

        buffer_offset += chunk_size
        remaining -= chunk_size
```

### Vad detta löser

1. **Sample-accuracy:** Events appliceras exakt vid rätt sample-position
2. **Korrekt modulation:** Varje tick-segment har rätt pitch/volume/pan-värden
3. **Arpeggio fungerar:** Varje ton får sitt eget segment
4. **Retrigger korrekt:** Retrigger sker vid exakt tick, inte buffer-gräns
5. **SetSpeed-bug försvinner:** Row-boundaries beräknas tick-för-tick (ingen retroaktiv omtolkning)

### Vad detta INTE löser

- Tyst ignorerade effekter (Tremor, Panbrello, etc.) — kräver implementation
- Tempo-drift vid icke-heltaliga samples per tick — kräver fractional sample-ackumulator
- Eventuella per-kanal state-problem som inte är timing-relaterade

### Referens

Alla seriösa tracker-emulatorer använder denna approach:
- **libopenmpt:** Renderar per-tick-segment
- **XMPlay:** Tick-baserad rendering
- **MilkyTracker:** Tick-baserad intern rendering

### Överväganden

- Innebär att `process_voices()` anropas flera gånger per buffer (en gång per tick-segment)
- Varje anrop renderar färre samples (typiskt 40-200 istället för 512)
- **Prestanda-overhead bör mätas:** Varje `process_voices()`-anrop har fast overhead (iterera instruments, voices, modulgraf). Vid 2-5 anrop per buffer multipliceras overheaden. Troligen hanterbart (~0.5% extra CPU) men bör verifieras med profiler, särskilt för modulgrafer med många noder.
- Kräver en **fractional sample-ackumulator** för att hantera att samples per tick sällan är heltal (t.ex. vid 130 BPM ≈ 923.08 samples per tick)
- Kan begränsas till tracker-läge (synth-läge behöver inte denna precision)

---

## Ignorerade tracker-effekter (ej mappade vid import)

Dessa effekter ignoreras tyst vid import (`tracker.rs:961-974, 1032`):

### Track-effekter (9 st)
- `InstrumentVolumeEnvelope` / `InstrumentVolumeEnvelopePosition`
- `InstrumentPanningEnvelope` / `InstrumentPanningEnvelopePosition`
- `InstrumentPitchEnvelope`
- `InstrumentNewNoteAction`
- `InstrumentSurround`
- `Panbrello` / `PanbrelloWaveform`
- `NoteOff` (XM-specifik key-off)
- `Tremor`

### Globala effekter (2 st)
- `BpmSlide`
- `MidiMacro`

Dessa är avancerade XM/S3M-funktioner. Tyst ignorering är rimligt men kan orsaka förvirring vid komplexa moduler.
