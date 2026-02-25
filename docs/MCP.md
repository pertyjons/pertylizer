# MCP-stöd (Model Context Protocol)

MCP låter AI-agenter (t.ex. Claude Code) inspektera och styra den körande synthen via ett standardiserat protokoll.

## Översikt

```
  Claude Code ←stdio→ synth-mcp-bridge ←TCP:9850→ modular-synth (GUI + ljud)
```

**Primärt läge:** GUI + MCP samtidigt. Synthen startar med GUI och ljud som vanligt, plus en MCP TCP-server på port 9850 i en bakgrundstråd.

**Sekundärt läge:** `--mcp` headless. Stdio direkt, ingen GUI. Ljud spelas fortfarande.

## Setup

### Bygg med MCP-stöd

```bash
cargo build --features mcp
```

### Kör med GUI + MCP

```bash
cargo run --features mcp
# → GUI startar, MCP lyssnar på 127.0.0.1:9850
```

### Kör headless (stdio MCP)

```bash
cargo run --features mcp -- --mcp
# → Inget GUI, MCP på stdin/stdout, ljud spelas
```

### Claude Code-konfiguration

Lägg till i `.claude/settings.json`:

```json
{
  "mcpServers": {
    "synth": {
      "command": "path/to/synth-mcp-bridge",
      "args": []
    }
  }
}
```

Alternativt med socat (utan bridge-binären):

```json
{
  "mcpServers": {
    "synth": {
      "command": "socat",
      "args": ["STDIO", "TCP:localhost:9850"]
    }
  }
}
```

## Verktyg

### Läsverktyg

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `list_instruments` | Alla instrument med grundinställningar | inga |
| `get_instrument_info` | Detaljer för ett specifikt instrument | `instrument_id` |
| `list_modules` | Moduler i röstgrafen med parametrar och portar | `instrument_id` |
| `get_module_info` | Fullständig info för en modul | `instrument_id`, `module_id` |
| `get_connections` | Alla kopplingar (kablar) i grafen | `instrument_id` |
| `get_parameter` | Enskilt parametervärde med display-format | `instrument_id`, `module_id`, `param_name` |
| `get_engine_status` | CPU, röster, meters, transport | inga |
| `get_graph_diagnostics` | Hitta problem i grafen | `instrument_id` |

### Instrumentverktyg

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `create_instrument` | Skapa nytt instrument | `name` |
| `delete_instrument` | Ta bort instrument (ej ID 0) | `instrument_id` |
| `rename_instrument` | Byt namn på instrument | `instrument_id`, `name` |
| `set_instrument_volume` | Ställ volym (0.0–2.0) | `instrument_id`, `volume` |
| `set_instrument_pan` | Ställ pan (-1.0–1.0) | `instrument_id`, `pan` |
| `set_instrument_mute` | Muta/avmuta | `instrument_id`, `muted` |
| `set_instrument_solo` | Solo/avsolo | `instrument_id`, `solo` |
| `set_instrument_midi_channel` | Ställ MIDI-kanal (1–16) | `instrument_id`, `channel` |
| `set_instrument_enabled` | Aktivera/avaktivera | `instrument_id`, `enabled` |

### Modulhantering

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `list_module_types` | Alla modultyper med portar och parametrar | inga |
| `add_module` | Lägg till modul i röstgrafen | `instrument_id`, `module_type` |
| `remove_module` | Ta bort modul | `instrument_id`, `module_id` |
| `connect` | Koppla ihop två modulportar | `instrument_id`, `from_module`, `from_port`, `to_module`, `to_port` |
| `disconnect` | Koppla bort två modulportar | `instrument_id`, `from_module`, `from_port`, `to_module`, `to_port` |
| `clear_graph` | Rensa hela röstgrafen | `instrument_id` |

### Parameterverktyg

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `set_parameter` | Ändra parametervärde | `instrument_id`, `module_id`, `param_name`, `value` |
| `note_on` | Spela en MIDI-not | `note`, `velocity`, `channel` (opt) |
| `note_off` | Stoppa en MIDI-not | `note`, `channel` (opt) |

### Patchverktyg

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `list_example_patches` | Alla exempelpatchar grupperade per kategori | inga |
| `load_example_patch` | Ladda en exempelpatch | `name` |
| `get_ui_snapshot` | UI-layout (modulpositioner, storlekar) | `instrument_id` |

### Sequencer: Song & Patterns

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `get_song_info` | Låtinfo (namn, tempo, längd) | inga |
| `set_song_name` | Sätt låtnamn | `name` |
| `set_song_tempo` | Sätt tempo i BPM | `bpm` |
| `list_patterns` | Alla patterns i låten | inga |
| `create_pattern` | Skapa pattern | `name`, `length_beats` |
| `delete_pattern` | Ta bort pattern | `pattern_id` |

### Sequencer: Noter

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `list_notes` | Alla noter i ett pattern | `pattern_id` |
| `add_note` | Lägg till not | `pattern_id`, `pitch`, `start_beat`, `duration_beats`, `velocity` |
| `update_note` | Uppdatera not (delfält) | `pattern_id`, `note_id`, `pitch`?, `start_beat`?, `duration_beats`?, `velocity`? |
| `remove_note` | Ta bort not | `pattern_id`, `note_id` |

### Sequencer: Tracks & Arrangement

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `list_tracks` | Alla tracks | inga |
| `create_track` | Skapa track | `name`, `instrument_id`? |
| `list_arrangement` | Alla pattern-placeringar | inga |
| `place_pattern` | Placera pattern på track | `pattern_id`, `track_id`, `start_beat` |
| `remove_placement` | Ta bort placering | `pattern_id`, `track_id`, `start_beat` |

### Sequencer: Transport

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `seq_play` | Starta uppspelning | inga |
| `seq_stop` | Stoppa uppspelning | inga |
| `seq_seek` | Hoppa till position | `beat` |

### Batch-operationer (Sequencer)

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `add_notes` | Lägg till flera noter | `pattern_id`, `notes[]` |
| `update_notes` | Uppdatera flera noter | `pattern_id`, `updates[]` |
| `replace_notes` | Ersätt alla noter | `pattern_id`, `notes[]` |
| `clear_pattern` | Rensa alla noter | `pattern_id` |
| `create_patterns` | Skapa flera patterns (med inline-noter) | `patterns[]` |
| `create_tracks` | Skapa flera tracks | `tracks[]` |
| `place_patterns` | Placera flera patterns | `placements[]` |
| `set_song` | Bygg hel låt i ett anrop | `name`, `tempo`, `patterns[]`, `tracks[]`, `placements[]` |

### Modul-ID-format

Moduler identifieras med korta ID-strängar: `osc-1`, `flt-1`, `env-2`, `amp-1`, `mix-1`, `out-1`, `lfo-1`, `sub-1`, `nse-1`, `kbp-1`.

### Parameter display-format

Parametrar returneras med enheter: `"440.0 Hz"`, `"2.0 kHz"`, `"3.0 ms"`, `"1.50 s"`, `"0.65"` (nivåer).

## Exempelsession

```
→ list_instruments
  [{ id: 0, name: "Default", volume: 1.0, pan: 0.0, muted: false, solo: false, module_count: 14 }]

→ create_instrument(name="Bass")
  { id: 1, name: "Bass", volume: 1.0, module_count: 0 }

→ add_module(instrument_id=1, module_type="osc")
  "OK: Oscillator added as osc-3"

→ list_modules(instrument_id=1)
  [{ id: "osc-3", module_type: "Oscillator", parameters: [...] }]

→ list_modules(instrument_id=0)
  [{ id: "osc-1", ... }, { id: "flt-1", ... }, ...]   # Bara instrument 0:s moduler

→ set_parameter(instrument_id=0, module_id="flt-1", param_name="Cutoff", value=800.0)
  "OK"

→ note_on(note=60, velocity=100)
  "Note 60 on (vel=100, ch=1)"

→ get_engine_status()
  { cpu_usage: 0.005, voice_count: 1, instrument_count: 2, sample_rate: 48000, ... }

→ rename_instrument(instrument_id=1, name="Deep Bass")
  "OK"

→ delete_instrument(instrument_id=1)
  "OK"
```

## Arkitektur

```
synth_mcp (crate)              modular_synth
┌─────────────────────┐       ┌──────────────────────┐
│ SynthBridge trait    │◄──────│ AppSynthBridge impl  │
│ MCP-server (rmcp)   │       │ Läser SharedGraphState│
│ 56 tool-definitioner│       │ Skickar EngineCommand │
└──────┬──────────────┘       └──────┬───────────────┘
       │ TCP :9850                   │ ring buffer
       │                              ▼
       │                       SynthEngine (audio thread)
       ▼
  stdio-bridge                 GUI (egui) + Ljud
  (tunn binär)
       │
       │ stdio (JSON-RPC)
       ▼
  Claude Code
```

**SynthBridge** — trait som abstraherar bort synth_engine.
**AppSynthBridge** — impl som läser `EngineState.shared_graph` (RwLock) och skickar kommandon via `CommandSender` (lock-free ring buffer).
**SynthMcpServer** — rmcp `ServerHandler` som delegerar till bridge.

## Kända begränsningar

- Inga kända begränsningar för multi-instrument

## MCP Resources & Prompts (planerad)

| Feature | Beskrivning |
|---------|-------------|
| **Patch-resurser** | Exponera sparade patchar som MCP Resources — agenten kan browse:a och ladda |
| **Modulkatalog** | Resource med alla modultyper, deras portar och parametrar |
| **"Design a sound"-prompt** | Guidad workflow: beskriv ett ljud → agenten bygger patchen |
| **"Debug my patch"-prompt** | Agenten inspekterar grafen och föreslår fixar |

## Kreativa idéer och användningsfall

### AI som ljuddesigner
En agent kan bygga hela synth-patchar från naturligt språk: *"Skapa en mörk ambient pad med långsam filtermodulering"*. Med Fas 2-verktygen kan agenten skapa moduler, koppla dem, ställa in parametrar, spela testtoner och iterera tills ljudet stämmer.

### AI-driven live-performance
Agenten spelar musik i realtid via `note_on`/`note_off` med timing — som Bach-menuetten som redan testats. Kan utökas med:
- **Algoritmisk komposition** — generera melodier baserat på skalor, ackordföljder, kontrapunkt
- **Interaktiv improvisation** — agenten reagerar på vad användaren spelar via MIDI-input
- **Generativ ambient** — oändliga evolverande ljudlandskap med parametersweeps

### Diagnostik och pedagogik
- **"Varför låter det konstigt?"** — agenten inspekterar signalkedjan, hittar disconnected moduler, felaktiga routings, extrema parametervärden
- **Synth-tutor** — agenten förklarar hur patchen fungerar, vad varje modul gör, varför parametrar är inställda som de är
- **A/B-jämförelse** — agenten sparar parametrar, ändrar en sak, låter användaren lyssna, återställer

### Automatiserad testing
- **Regressionstest** — ladda patch, spela noter, verifiera att engine-status ser rätt ut
- **Stresstest** — spela 128 noter samtidigt, övervaka CPU-användning
- **Parameter-sweep** — systematiskt testa extremvärden för alla parametrar

### Parameter-automation via MCP
- **Morfning mellan presets** — agenten interpolerar parametrar mjukt över tid
- **LFO-liknande automation** — agenten sveper parametrar sinusformigt (långsammare än realtid, men coolt för demonstrationer)
- **Reaktiv ljuddesign** — koppla externa datakällor (väder, aktiekurser, sensor-data) till synth-parametrar

### Multi-agent-arkitektur
- **Kompositör + Ljuddesigner** — en agent skriver musiken, en annan designar ljuden
- **Critic-agent** — en agent lyssnar (via meters/status) och ger feedback: "för mycket brus", "filtret dämpar för mycket"
- **Ensemble** — flera agenter styr varsin MIDI-kanal för samspel

### Integration med andra verktyg
- **DAW-bridge** — MCP-agenten kan styra synthen inifrån en DAW-kontext
- **Notation → ljud** — agenten läser MusicXML/MIDI-filer och spelar dem live
- **Text-to-music** — kombinera med en LLM som genererar noter från textbeskrivningar, MCP-agenten spelar dem
