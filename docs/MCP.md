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

### Skrivverktyg

| Verktyg | Beskrivning | Parametrar |
|---------|-------------|------------|
| `set_parameter` | Ändra parametervärde | `instrument_id`, `module_id`, `param_name`, `value` |
| `note_on` | Spela en MIDI-not | `note`, `velocity`, `channel` (opt) |
| `note_off` | Stoppa en MIDI-not | `note`, `channel` (opt) |

### Modul-ID-format

Moduler identifieras med korta ID-strängar: `osc-1`, `flt-1`, `env-2`, `amp-1`, `mix-1`, `out-1`, `lfo-1`, `sub-1`, `nse-1`, `kbp-1`.

### Parameter display-format

Parametrar returneras med enheter: `"440.0 Hz"`, `"2.0 kHz"`, `"3.0 ms"`, `"1.50 s"`, `"0.65"` (nivåer).

## Exempelsession

```
→ list_instruments
  [{ id: 0, name: "Default", module_count: 14 }]

→ list_modules(instrument_id=0)
  [{ id: "osc-1", module_type: "Oscillator", parameters: [...] },
   { id: "flt-1", module_type: "Filter", ... }, ...]

→ get_parameter(instrument_id=0, module_id="flt-1", param_name="Cutoff")
  { name: "Cutoff", value: 2000.0, display: "2.0 kHz" }

→ set_parameter(instrument_id=0, module_id="flt-1", param_name="Cutoff", value=800.0)
  "OK"

→ note_on(note=60, velocity=100)
  "Note 60 on (vel=100, ch=1)"

→ get_engine_status()
  { cpu_usage: 0.005, voice_count: 1, sample_rate: 48000, ... }

→ get_graph_diagnostics(instrument_id=0)
  [{ severity: "Warning", module_id: "lfo-1", message: "Module lfo-1 (LFO) has no connections" }]
```

## Arkitektur

```
synth_mcp (crate)              modular_synth
┌─────────────────────┐       ┌──────────────────────┐
│ SynthBridge trait    │◄──────│ AppSynthBridge impl  │
│ MCP-server (rmcp)   │       │ Läser SharedGraphState│
│ 11 tool-definitioner│       │ Skickar EngineCommand │
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

**SynthBridge** — trait med 11 metoder som abstraherar bort synth_engine.
**AppSynthBridge** — impl som läser `EngineState.shared_graph` (RwLock) och skickar kommandon via `CommandSender` (lock-free ring buffer).
**SynthMcpServer** — rmcp `ServerHandler` som delegerar till bridge.

## Kända begränsningar

- Bara ett instrument (ID 0) exponeras — multi-instrument kräver ytterligare shared state

## Fas 2: Topologiändring (planerad)

Verktyg som ändrar grafen — kräver modulfabrik och topologikommandon:

| Verktyg | Beskrivning |
|---------|-------------|
| `add_module` | Skapa ny modul (typ, namn) |
| `remove_module` | Ta bort modul och dess kopplingar |
| `connect` | Koppla en port till en annan |
| `disconnect` | Koppla bort en kabel |
| `list_module_types` | Lista tillgängliga modultyper |
| `save_patch` / `load_patch` | Spara/ladda patch-filer |
| `set_tempo` / `play` / `stop` | Transportkontroll |

## Fas 3: MCP Resources & Prompts

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
