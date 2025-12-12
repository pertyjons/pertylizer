# Debug Tools - Användarguide

Debug-systemet tillhandahåller verktyg för att analysera audio-motorn offline, utan realtidsuppspelning. Aktiveras med `--features debug-tools`.

## Innehåll

1. [SignalProbe](#signalprobe) - Inspektera sample-värden
2. [GraphDebugger](#graphdebugger) - Analysera modulkopplingar
3. [SequencerDebugger](#sequencerdebugger) - Stega genom songs
4. [VoiceDebugger](#voicedebugger) - Spåra voice allocation
5. [Integrationstester](#integrationstester) - Testsvit och exempel

---

## SignalProbe

Spelar in och analyserar sample-värden vid specifika punkter i signal-grafen.

### Användningsområden

- Verifiera att en modul producerar signal
- Jämföra signaler före/efter processing
- Mäta RMS, peak, DC offset
- Hitta tysta moduler i kedjan

### API

```rust
use modular_synth::debug::{SignalProbe, ProbePoint, SignalStats};

// Skapa probe
let mut probe = SignalProbe::new();

// Lägg till punkter att övervaka
probe.add_probe(envelope_id, "out");
probe.add_probe(amplifier_id, "cv");

// Spela in under processing (anropas i process-loop)
probe.record(envelope_id, "out", &output_buffer);

// Hämta statistik
if let Some(stats) = probe.stats(envelope_id, "out") {
    println!("RMS: {:.3}", stats.rms);
    println!("Peak: {:.3}", stats.peak);
    println!("DC offset: {:.3}", stats.dc_offset);
    println!("Silent: {}", stats.is_silent);
}

// Jämför två signaler
let point_a = ProbePoint::new(osc_id, "out");
let point_b = ProbePoint::new(filter_id, "out");
if let Some(cmp) = probe.compare(&point_a, &point_b) {
    println!("Correlation: {:.3}", cmp.correlation);
    println!("Identical: {}", cmp.are_identical);
}

// Rensa för ny inspelning
probe.clear();
```

### SignalStats

| Fält | Typ | Beskrivning |
|------|-----|-------------|
| `min` | f32 | Minsta sample-värde |
| `max` | f32 | Största sample-värde |
| `rms` | f32 | Root Mean Square (genomsnittlig effekt) |
| `peak` | f32 | Högsta absolutvärde |
| `dc_offset` | f32 | Genomsnittligt värde (bör vara ~0) |
| `is_silent` | bool | True om alla samples är 0.0 |
| `sample_count` | usize | Antal analyserade samples |

### Tips

- Använd `with_max_samples(n)` för att begränsa minneanvändning
- `summary()` ger en översikt av alla probe-punkter med data
- `has_samples(id, port)` kollar snabbt om data finns

---

## GraphDebugger

Analyserar kopplingar i en ModuleGraph utan att processa audio.

### Användningsområden

- Verifiera att signal-path finns från A till B
- Hitta okopplade portar
- Diagnostisera vanliga fel (envelope utan gate, etc.)
- Generera läsbar beskrivning av grafen

### API

```rust
use modular_synth::debug::GraphDebugger;

let dbg = GraphDebugger::new(&voice.graph);

// Lista alla kopplingar
for conn in dbg.connections() {
    println!("{}.{} → {}.{}",
        conn.from_module, conn.from_port,
        conn.to_module, conn.to_port);
}

// Kolla om path finns
if dbg.path_exists(oscillator_id, output_id) {
    println!("Signal kan nå output!");
}

// Tracera path (för visualisering)
if let Some(path) = dbg.trace_path(osc_id, amp_id).complete {
    println!("Path: {}", path.describe());
}

// Hitta problem
let issues = dbg.diagnose();
for issue in issues {
    println!("⚠️  {}", issue);
}

// Läsbar beskrivning
println!("{}", dbg.describe());
```

### Diagnose-kontroller

`diagnose()` hittar följande vanliga problem:

| Problem | Beskrivning |
|---------|-------------|
| Envelope utan gate | Envelope-modul har ingen gate/trigger-input kopplad |
| Amplifier utan CV | Amplifier har ingen CV-input (blir tyst om base gain = 0) |
| SamplePlayer utan gate | SamplePlayer med envelope men ingen gate-koppling |
| Isolerad modul | Modul utan några kopplingar alls |

### Exempel: describe() output

```
=== Module Graph ===

Modules:
  osc-1 - Oscillator [source]
  env-1 - Envelope
  amp-1 - Amplifier
  out-1 - StereoOutput [sink]

Connections:
  osc-1.out → amp-1.in
  env-1.out → amp-1.cv
  amp-1.out → out-1.left
  amp-1.out → out-1.right

Unconnected ports:
  env-1.gate (input)
```

---

## SequencerDebugger

Stegar genom en Song tick-för-tick utan realtidsuppspelning.

### Användningsområden

- Förstå vilka events som genereras och när
- Verifiera NoteOn/NoteOff-ordning
- Debugga mono-per-track-beteende
- Testa sequencer-logik i unit tests

### API

```rust
use modular_synth::debug::{SequencerDebugger, EventSource};

// Skapa debugger från song
let mut dbg = SequencerDebugger::new(song);

// Stega till specifik tick
let events = dbg.step_to(Tick(96));
for event in &events {
    println!("Tick {}: {:?} ({:?})",
        event.tick.0, event.event, event.source);
}

// Stega ett tick i taget
let events = dbg.step();

// Hoppa till nästa event (skippa tomma ticks)
if let Some(events) = dbg.step_to_next_event() {
    // ...
}

// Snapshot av nuvarande state
let snap = dbg.snapshot();
println!("Position: tick {}", snap.current_tick.0);
println!("Tempo: {} BPM", snap.tempo.as_f32());
println!("Active notes: {}", snap.active_notes.len());

// Filtrera events
let note_ons = dbg.note_on_events();
let note_offs = dbg.note_off_events();
let for_instrument = dbg.events_for_instrument(SeqInstrumentId(0));

// Sammanfattning
println!("{}", dbg.summarize());

// Återställ
dbg.reset();
```

### EventSource

| Variant | Beskrivning |
|---------|-------------|
| `NoteStart` | Not startade vid denna tick |
| `NoteDuration` | Not slutade p.g.a. sin duration |
| `MonoPerTrack` | Not stoppades av ny not på samma track |
| `ManualStop` | Stop() anropades |
| `LoopPoint` | Loop-punkt nåddes |

### Exempel: Test av mono-per-track

```rust
#[test]
fn test_mono_replacement() {
    let song = create_song_with_two_notes_same_track();
    let mut dbg = SequencerDebugger::new(song);

    // Stega till andra noten (ska generera NoteOff + NoteOn)
    let events = dbg.step_to(Tick(96));

    // Verifiera ordning: NoteOff FÖRE NoteOn
    assert!(events[0].event.is_note_off());
    assert!(events[1].event.is_note_on());
    assert_eq!(events[0].source, EventSource::MonoPerTrack);
}
```

---

## VoiceDebugger

Spårar voice allocation, state-förändringar och stealing.

### Användningsområden

- Förstå hur voices allokeras
- Debugga voice stealing
- Verifiera att retrigger fungerar korrekt
- Analysera polyfoni-beteende

### API

```rust
use modular_synth::debug::{VoiceDebugger, ReleaseReason, VoiceInfo};

let mut dbg = VoiceDebugger::new();

// Spela in events (anropas från VoiceAllocator)
dbg.record_allocation(voice_id, note, velocity, Some(tick));
dbg.record_release(voice_id, ReleaseReason::NoteOff);
dbg.record_steal(voice_id, new_note);
dbg.record_state_change(voice_id, &old_state, &new_state);
dbg.record_retrigger(voice_id, old_note, new_note, velocity);

// Avancera tid
dbg.advance(256); // samples

// Hämta events
let all = dbg.events();
let for_voice = dbg.events_for_voice(0);
let allocations = dbg.allocations();
let releases = dbg.releases();
let steals = dbg.steals();

// Analys
let unique = dbg.unique_voices_used();
let was_reused = dbg.was_voice_reused(0);

// Snapshots
dbg.take_snapshot(&voice_infos);
if let Some(snap) = dbg.snapshot_at(sample_position) {
    println!("Active voices: {}", snap.active_count());
}

// Sammanfattning
println!("{}", dbg.summarize());
```

### ReleaseReason

| Variant | Beskrivning |
|---------|-------------|
| `NoteOff` | Explicit NoteOff-event |
| `NoteDurationEnd` | Notens duration tog slut |
| `MonoReplacement` | Ersatt av ny not på samma track |
| `AllNotesOff` | All notes off-kommando |
| `Panic` | Panic/emergency stop |
| `VoiceStolen` | Voice stal för ny not |

### VoiceEvent-typer

```rust
pub enum VoiceEvent {
    Allocated { voice_id, note, velocity, sample_position, tick },
    Released { voice_id, note, sample_position, reason },
    Stolen { voice_id, old_note, new_note, sample_position },
    StateChanged { voice_id, from_state, to_state, sample_position },
    Retriggered { voice_id, old_note, new_note, velocity, sample_position },
}
```

### Exempel: Verifiera voice reuse

```rust
#[test]
fn test_voice_not_stolen_unnecessarily() {
    let mut dbg = VoiceDebugger::new();

    // Spela två noter sekventiellt
    dbg.record_allocation(0, MidiNote::C4, 0.8, None);
    dbg.advance(10000);
    dbg.record_release(0, ReleaseReason::NoteOff);
    dbg.advance(1000);
    dbg.record_allocation(0, MidiNote::E4, 0.7, None);

    // Samma voice ska återanvändas, inte ny
    assert_eq!(dbg.unique_voices_used(), 1);
    assert!(dbg.was_voice_reused(0));
    assert_eq!(dbg.steals().len(), 0); // Ingen stealing
}
```

---

## Integration i tester

### Grundläggande mönster

```rust
#[cfg(feature = "debug-tools")]
#[test]
fn test_signal_flow() {
    use modular_synth::debug::{GraphDebugger, SignalProbe};

    // Setup
    let mut voice = create_test_voice();

    // 1. Verifiera kopplingar
    let graph_dbg = GraphDebugger::new(&voice.graph);
    assert!(graph_dbg.diagnose().is_empty(), "Graph has issues");

    // 2. Spela in signaler
    let mut probe = SignalProbe::new();
    probe.add_probe(env_id, "out");

    // 3. Trigga och processa
    voice.note_on(MidiNote::C4, Velocity::MAX);
    for _ in 0..10 {
        let mut output = AudioBuffer::new(256);
        voice.process(&mut output, &context);
        probe.record(env_id, "out", voice.get_output("out"));
    }

    // 4. Verifiera
    let stats = probe.stats(env_id, "out").unwrap();
    assert!(!stats.is_silent, "Envelope should produce signal");
    assert!(stats.max > 0.5, "Envelope should reach attack peak");
}
```

### Conditional compilation

```rust
// Endast kompilera om debug-tools är aktiverat
#[cfg(feature = "debug-tools")]
mod debug_tests {
    use super::*;
    use modular_synth::debug::*;

    #[test]
    fn my_debug_test() { ... }
}
```

---

## Performance

Debug-systemet är designat för offline-analys, inte realtid:

- **SignalProbe**: O(1) per sample, men allokerar minne
- **GraphDebugger**: Alla operationer är O(n) där n = antal moduler/connections
- **SequencerDebugger**: Stegar i "simulerad realtid" (snabbare än faktisk tid)
- **VoiceDebugger**: O(1) per event, växande historik

För realtidssäker debugging, använd atomics och lock-free strukturer istället.

---

## Integrationstester

Integrationstest-modulen (`src/debug/integration_tests.rs`) demonstrerar verklig användning av debug-verktygen med faktiska moduler och voice graphs.

### Köra testerna

```bash
# Kör alla debug-tester
cargo test --features debug-tools

# Kör specifikt test
cargo test --features debug-tools test_graph_debugger_path_verification

# Kör med output
cargo test --features debug-tools -- --nocapture
```

### GraphDebugger-tester

| Test | Beskrivning |
|------|-------------|
| `test_graph_debugger_path_verification` | Verifierar att `path_exists()` hittar signal-paths i grafen |
| `test_graph_debugger_diagnose_missing_cv` | Testar att `diagnose()` varnar för amplifier utan CV-input |
| `test_graph_debugger_diagnose_with_envelope` | Verifierar att CV-varning försvinner när envelope kopplas |
| `test_graph_debugger_connections_list` | Testar `connections()` returnerar korrekt antal kopplingar |
| `test_graph_debugger_describe` | Verifierar att `describe()` innehåller moduler och kopplingar |

### SignalProbe-tester

| Test | Beskrivning |
|------|-------------|
| `test_signal_probe_oscillator_output` | Verifierar att oscillator producerar signal med korrekt statistik |
| `test_signal_probe_envelope_attack` | Testar envelope-output under attack-fasen |
| `test_signal_probe_compare_before_after_filter` | Jämför signal före/efter filter med `compare()` |

### SequencerDebugger-tester

| Test | Beskrivning |
|------|-------------|
| `test_sequencer_debugger_step_to_first_note` | Testar `step_to()` för att fånga första noten |
| `test_sequencer_debugger_step_through_pattern` | Stegar genom ett helt pattern och verifierar alla noter |
| `test_sequencer_debugger_snapshot` | Verifierar `snapshot()` returnerar korrekt position |
| `test_sequencer_debugger_event_filtering` | Testar `events_for_instrument()` filtrering |

### VoiceDebugger-tester

| Test | Beskrivning |
|------|-------------|
| `test_voice_debugger_allocation_tracking` | Testar allocation/release-tracking |
| `test_voice_debugger_steal_detection` | Verifierar att voice stealing detekteras |
| `test_voice_debugger_summary` | Testar `summarize()` output |

### Kombinerat workflow-test

`test_complete_debug_workflow` demonstrerar ett komplett debugging-scenario:

```rust
#[test]
fn test_complete_debug_workflow() {
    // 1. Skapa voice graph
    let graph = create_voice_graph_with_envelope();

    // 2. Verifiera kopplingar med GraphDebugger
    let graph_dbg = GraphDebugger::new(&graph);
    assert!(graph_dbg.path_exists(osc_id, out_id));
    assert!(graph_dbg.path_exists(env_id, amp_id));

    // 3. Setup SignalProbe för inspelning
    let mut probe = SignalProbe::new();
    probe.add_probe(osc_id, "out");
    probe.add_probe(env_id, "out");

    // 4. Setup VoiceDebugger för allocation-tracking
    let mut voice_dbg = VoiceDebugger::new();
    voice_dbg.record_allocation(0, MidiNote::C4, 0.8, None);

    // 5. (Process audio och record samples)

    // 6. Verifiera resultat
    voice_dbg.record_release(0, ReleaseReason::NoteOff);
    assert_eq!(voice_dbg.unique_voices_used(), 1);
}
```

### Hjälpfunktioner

Testerna använder två hjälpfunktioner för att skapa testgrafer:

```rust
// Enkel graph: Oscillator → Amplifier → StereoOutput
fn create_simple_voice_graph() -> ModuleGraph;

// Graph med envelope: Oscillator → Amplifier ← Envelope → StereoOutput
fn create_voice_graph_with_envelope() -> ModuleGraph;
```

Dessa kan kopieras till egna tester som startpunkt för debugging.
