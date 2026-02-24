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

## Verktyg (Fas 1)

| Verktyg | Typ | Beskrivning |
|---------|-----|-------------|
| `list_instruments` | Läs | Alla instrument med grundinställningar |
| `get_instrument_info` | Läs | Detaljer för ett specifikt instrument |
| `list_modules` | Läs | Moduler i röstgrafen med parametrar |
| `get_module_info` | Läs | Fullständig info för en modul |
| `get_connections` | Läs | Alla kopplingar (kablar) i grafen |
| `get_parameter` | Läs | Enskilt parametervärde |
| `get_engine_status` | Läs | CPU, röster, meters, transport |
| `get_graph_diagnostics` | Läs | Hitta problem i grafen |
| `set_parameter` | Skriv | Ändra parametervärde |
| `note_on` | Skriv | Spela en MIDI-not |
| `note_off` | Skriv | Stoppa en MIDI-not |

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

## Ej implementerat (framtida faser)

- `add_module` / `remove_module` — kräver modulfabrik
- `connect` / `disconnect` — topologiändring
- `save_patch` / `load_patch` — patch-I/O
- `add_instrument` / `remove_instrument`
- `set_tempo`, `play`, `stop` — transportkontroll
- MCP Resources (patch-filer, modulkatalog)
- MCP Prompts (guidade workflows)
