# Kodanalys: modular-synth v0.39.0

## Övergripande bedömning

Kodbasen är **välstrukturerad och följer projektets CLAUDE.md-standard väl**. Typsystemet med newtypes används konsekvent, och arkitekturen är tydlig. Nedan följer identifierade förbättringsområden och buggar.

---

## 🔴 Kritiska problem

### 1. Panic i Clone-implementation för EngineCommand
**Fil:** `src/engine/transactions.rs:541-553`

```rust
EngineCommand::AddInstrument { .. } => {
    panic!("AddInstrument cannot be cloned - instrument instances are unique")
}
```

**Problem:** En `Clone`-implementation som panikerar är ett antipattern och bryter kontraktet för `Clone`. Om någon kod omedvetet klonar denna typ kraschar programmet.

**Lösning:** 
- Använd `Arc<dyn ...>` istället för `Box<dyn ...>` för att tillåta billig kloning
- Eller implementera `Clone` endast för de varianter som kan klonas och separera de andra i en annan enum

---

### 2. Potentiell allokering i audio-kontext
**Fil:** `src/engine/graph.rs:135-162`

```rust
let mut outputs = HashMap::new();
outputs.insert(port.name.clone(), AudioBuffer::new(self.buffer_size));
```

**Problem:** `add_module()` skapar nya HashMaps och allokerar buffertar. Om detta anropas under audio-processing (t.ex. via hot-reload) kan det orsaka glitchar.

**Lösning:** Säkerställ att `add_module()` aldrig anropas från audio-tråden, eller pre-allokera output-buffers.

---

### 3. `std::sync::RwLock` i SequencerEngine
**Fil:** `src/engine/sequencer_engine.rs:49`

```rust
song: Arc<RwLock<Song>>,  // std::sync::RwLock, inte parking_lot!
```

**Problem:** Resten av projektet använder `parking_lot::RwLock` som är snabbare och har bättre fairness. Inkonsekvent användning kan dölja problem.

**Lösning:** Byt till `parking_lot::RwLock` för konsistens.

---

## 🟡 Medelallvarliga problem

### 4. TODOs i sequencer GUI
**Fil:** `src/gui/views/sequencer.rs:83-99`

Play/Stop/Record-knappar gör ingenting:
```rust
if ui.button(...).clicked() {
    // TODO: Implement play
}
```

**Lösning:** Koppla till `SequencerState`s `is_playing`, `is_recording` och skicka `EngineCommand`.

---

### 5. Hårdkodade värden i sequencer
**Fil:** `src/gui/views/sequencer.rs:211`

```rust
let num_tracks = 4; // TODO: Get from song/config
```

**Lösning:** Hämta från `state.song.tracks.len()` eller config.

---

### 6. EnvelopeEditor använder f32 internt
**Fil:** `src/gui/widgets/envelope.rs:103-106`

```rust
attack: &'a mut f32,
decay: &'a mut f32,
sustain: &'a mut f32,
release: &'a mut f32,
```

**Problem:** Widgeten tar `&mut f32` istället för typade värden, men returnerar `EnvelopeChanges` med rätt typer. Inkonsekvent.

**Lösning:** Ta `&'a mut Seconds` och `&'a mut NormalizedValue` som input för full typsäkerhet.

---

### 7. Dead code markerade men behålls
**Fil:** `src/engine/synth_engine.rs:432-450`

```rust
#[allow(dead_code)] // Useful for future bulk operations
fn rebuild_all_instrument_voices(&mut self) { ... }
```

**Problem:** Kod som inte används men behålls "för framtiden" ökar underhållsbördan.

**Lösning:** Ta bort eller använd. Kod kan alltid hämtas från git-historiken.

---

### 8. Inkonsekvent error-handling i MathOscillator
**Fil:** `src/modules/math_oscillator.rs:483`

```rust
self.frequency = Hertz::new(440.0 * freq_mult); // TODO: Use base frequency
```

**Lösning:** Använd korrekt basfrekvens från note_on.

---

## 🟢 Mindre förbättringar

### 9. GUI-filer är stora
- `egui_backend.rs`: 2529 rader
- `patch_editor.rs`: 1768 rader

**Rekommendation:** Bryt ut delar till separata moduler:
- `egui_backend.rs` → `menu.rs`, `transport.rs`, `dialogs.rs`
- `patch_editor.rs` → `module_window.rs`, `cable_drawing.rs`, `context_menu.rs`

---

### 10. String-allokeringar i GUI
55 ställen med `.to_string()` eller `format!()` i `egui_backend.rs`.

De flesta är OK (GUI är inte realtidskritiskt), men överväg:
- Cache formaterade strängar som uppdateras sällan (t.ex. instrument-namn)
- Använd `write!` till en återanvänd buffer för meter-värden

---

### 11. Duplicerad port-type konvertering
**Fil:** `src/gui/patch_editor.rs` och `src/gui/module_panel.rs`

`convert_port_type()` existerar i båda. Bör finnas på ett ställe.

---

### 12. Magic numbers i LFO
**Fil:** `src/modules/lfo.rs:176`

```rust
Hertz::new((base_rate.as_f32() * rate_mult).clamp(0.01, 50.0))
```

**Rekommendation:** Använd konstanter:
```rust
const LFO_MIN_RATE: Hertz = Hertz::new(0.01);
const LFO_MAX_RATE: Hertz = Hertz::new(50.0);
```

---

## ✅ Bra saker

1. **Typsystemet** används konsekvent (`Hertz`, `Seconds`, `NormalizedValue`, etc.)
2. **Separata output-buffers per modul** undviker aliasing i audio-tråden
3. **`parking_lot`** används för lås (förutom sequencer)
4. **EnvelopeChanges** returnerar typade värden trots interna f32
5. **ProcessContext** samlar all relevant information
6. **Tydlig CLAUDE.md** med kodstandard
7. **Bra teststruktur** med `#[allow(clippy::unwrap_used)]` i tester
8. **Phase-typen** har korrekta wrap-around-metoder

---

## Prioriterad åtgärdslista

| Prio | Problem | Åtgärd |
|------|---------|--------|
| 1 | Panic i Clone | Refaktorera EngineCommand |
| 2 | std::sync::RwLock | Byt till parking_lot |
| 3 | Sequencer TODOs | Implementera play/stop/record |
| 4 | EnvelopeEditor typer | Använd Seconds/NormalizedValue |
| 5 | Dead code | Ta bort oanvänd kod |
| 6 | GUI-filstorlek | Bryt ut till moduler |
| 7 | Magic numbers | Extrahera till konstanter |
