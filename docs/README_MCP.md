# Pertylizer × MCP — AI Agent Integration Guide

Pertylizer ships with a built-in [Model Context Protocol](https://modelcontextprotocol.io) server that exposes
**~150 tools** for full remote control of the synth, sequencer, sample library, and Acoustic World Engine. Any
MCP-capable client — Claude Code, Claude Desktop, custom agents — can use it to build instruments, compose songs,
edit patterns, render audio, and analyze the result, all while the synth keeps running.

This guide covers how to start the server, how to connect from common clients, what tools exist, and the
AI-specific affordances built into Pertylizer.

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Connecting Claude Code](#connecting-claude-code)
3. [Connecting Claude Desktop](#connecting-claude-desktop)
4. [Tool Catalog](#tool-catalog)
5. [AI-Friendly Features](#ai-friendly-features)
6. [Architecture](#architecture)
7. [Example Workflows](#example-workflows)

---

## Quick Start

You can either **run a pre-built binary** or **compile from source** — both expose the same MCP server.

### Option A — Run the pre-built binary

Grab the latest `pertylizer` executable from the
[GitHub Releases](https://github.com/pertyjons/pertylizer/releases) page (or use whatever binary you've already
built). Invocation is identical to the `cargo run` examples below — just substitute the executable path:

```bash
./pertylizer                # GUI + HTTP MCP on port 9850
./pertylizer --headless     # stdio MCP, no GUI
./pertylizer --no-osc       # disable OSC telemetry to the visualizer
```

### Option B — Compile from source

Requires Rust 1.95+ (edition 2024). From the repo root:

```bash
cargo build --release       # produces target/release/pertylizer
cargo run --release          # build + run in one step
```

For development, plain `cargo run` (debug build) works too but is significantly slower at audio-heavy workloads.

---

The MCP server runs in two transport modes — pick whichever fits your client. All commands below use `cargo run`,
but the pre-built binary works identically (replace `cargo run --` with `./pertylizer`).

### Streamable HTTP (default, GUI mode)

```bash
cargo run
```

This starts the synth with the egui GUI **and** an HTTP MCP server on
`http://127.0.0.1:9850/mcp`. The audio engine, GUI, and MCP server share the same process — agents can edit a patch
while you hear the changes immediately.

The HTTP transport supports multiple concurrent clients via session tracking.

### Stdio (headless mode)

```bash
cargo run -- --headless
```

No GUI, no window. The MCP server speaks JSON-RPC over stdin/stdout. Audio still plays through the default output
device. Use this when launching Pertylizer as a child process from an MCP client (Claude Desktop's default
integration model).

### Disabling OSC telemetry

OSC telemetry to the visualizer is enabled by default. If you don't need the visualizer:

```bash
cargo run -- --no-osc
```

---

## Connecting Claude Code

### HTTP transport (recommended for GUI work)

Add Pertylizer as an MCP server with the CLI:

```bash
claude mcp add --transport http pertylizer http://127.0.0.1:9850/mcp
```

Start Pertylizer first (`cargo run`), then launch Claude Code in any project. The tools appear under the
`pertylizer` namespace.

### Stdio transport (headless)

```bash
claude mcp add pertylizer -- cargo run --manifest-path /path/to/pertylizer/Cargo.toml -- --headless
```

Claude Code will spawn Pertylizer as a child process whenever the MCP server is needed.

---

## Connecting Claude Desktop

Edit your Claude Desktop MCP config (typically `~/.config/Claude/claude_desktop_config.json` on Linux, or the
equivalent on macOS/Windows) and add:

```json
{
  "mcpServers": {
    "pertylizer": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/pertylizer/Cargo.toml", "--", "--headless"]
    }
  }
}
```

Restart Claude Desktop. The Pertylizer tools become available in any conversation.

For HTTP transport with Claude Desktop, use a streamable-HTTP MCP wrapper or run Pertylizer in GUI mode and use
Claude Code instead.

---

## Tool Catalog

Tools are grouped by purpose. Each tool returns typed JSON; arguments are validated and errors are returned as
structured messages so agents can recover.

### Discovery & Introspection

Read the current state of instruments, modules, ports, parameters, and the audio graph.

| Tool | Purpose |
|------|---------|
| `list_instruments` | All instruments with id, name, category |
| `get_instrument_info` | Modules, connections, parameter values for one instrument |
| `get_instrument_profiles` | Auto-inferred role (drums/bass/lead/pad/pluck/FX) with confidence + signal trail |
| `get_instrument_automation_targets` | Valid automation targets for an instrument: every automatable per-module parameter (ready-to-use target string, range, unit, response curve) plus the instrument macros |
| `list_modules` | All modules in an instrument; each parameter carries `type_id`, `unit`, `is_automatable`, and `response_curve` |
| `list_module_types` | All 67 module types with categories |
| `get_module_type_info` | Ports, parameters, defaults, and ranges for a module type |
| `search_modules` | Fuzzy search by name across all module types |
| `list_port_types` | All port types (Audio, Cv, Gate, Pitch, Clock, …) |
| `get_connections` | All cables in an instrument |
| `check_connection` | Whether a specific connection exists |
| `get_parameter` | Current value of one parameter |
| `get_graph_diagnostics` | Detect feedback loops, unreachable modules, missing outputs (one instrument) |
| `lint_project` | Project-wide load-lint: graph diagnostics over every instrument + error/warning/info totals |
| `get_project_schema` | Authoritative on-disk `.pertyproj` JSON Schema + format/build version (validate/diff project files without introspection drift) |
| `get_ui_snapshot` | Current GUI focus, selection, view |
| `get_engine_status` | Sample rate, transport state, BPM, voice count |

### Patch & Sound Design

Create instruments, wire modules together, set parameters.

| Tool | Purpose |
|------|---------|
| `create_instrument`, `delete_instrument`, `rename_instrument` | Lifecycle |
| `set_instrument_category` | Drums/Bass/Lead/Pad/Pluck/Keys/FX/Other |
| `add_module`, `remove_module` | Modify the graph |
| `connect`, `disconnect` | Cables (`connect` takes one or many port pairs) |
| `clear_graph`, `auto_layout` | Reset / tidy |
| `set_parameter` | One or many params atomically; value may be a number, a choice string (`"sawtooth"`), or a boolean |
| `build_instrument` | Single call: one or many full instruments from a JSON spec |
| `apply_example_patch`, `load_example_patch`, `list_example_patches` | Built-in patch library |

### Playback & Transport

Play notes and drive the sequencer.

| Tool | Purpose |
|------|---------|
| `note_on`, `note_off` | MIDI input from the agent |
| `seq_play`, `seq_stop`, `seq_seek` | Transport control |

### Song & Project

| Tool | Purpose |
|------|---------|
| `get_song_info`, `set_song_name`, `set_song_author` | Metadata |
| `set_song_tempo`, `set_song_time_signature` | Timing |
| `new_project`, `load_project`, `save_project` | Project I/O |
| `set_song` | Load a complete arrangement in one call |

### Patterns & Notes

Non-realtime composition.

| Tool | Purpose |
|------|---------|
| `list_patterns`, `create_pattern` | Pattern lifecycle (`create_pattern` takes one or many) |
| `delete_pattern`, `rename_pattern`, `duplicate_pattern` | |
| `set_pattern_length` | In ticks (960 PPQN) |
| `list_notes`, `add_note` | Note entry (`add_note` takes one or many) |
| `update_note`, `replace_notes`, `remove_note` | Edits (`update_note` takes one or many) |
| `clear_pattern` | Wipe a pattern |

**Per-note expression (`add_note`).** Each note in `add_note` accepts two
optional fields beyond pitch/start/duration/velocity:

- `legato` (bool) — tie into the next note without retriggering the envelope.
- `glide` — portamento/glissando *into* the note:
  - `from_semitones` (f32) — source as a semitone offset relative to the note
    (negative = below), **or** `from_pitch` (0–127) for an absolute source
    (takes precedence). Default `-2`.
  - `time_ms` (f32) — glide time, default `100`.
  - `interp` — `"continuous"` (smooth portamento, default) or `"stepped"`
    (chromatic glissando).
- `expression` — per-note shaping + vibrato:
  - `accent` (f32) — velocity multiplier (`1.0` = unchanged, `>1` louder).
  - `gate` (0–1) — note length as a fraction of its duration (staccato).
  - `ghost` (bool) — force a soft velocity.
  - `probability` (0–1) — chance the note plays (resolved at playback; preview
    always sounds it).
  - `vibrato` — `{ depth (semitones), rate (Hz), delay_ms (depth fade-in),
    shape: sine|triangle|square|saw }`.

### Tracks & Arrangement

| Tool | Purpose |
|------|---------|
| `list_tracks`, `create_track` | Track lifecycle (`create_track` takes one or many) |
| `rename_track`, `delete_track`, `set_track_instrument` | |
| `set_track_volume`, `set_track_pan`, `set_track_mute`, `set_track_solo` | Mixing |
| `list_arrangement`, `place_pattern`, `remove_placement` | Song arrangement (`place_pattern` takes one or many) |

### Instrument Mixing

| Tool | Purpose |
|------|---------|
| `set_instrument_volume`, `set_instrument_pan` | |
| `set_instrument_mute`, `set_instrument_solo`, `set_instrument_enabled` | |
| `set_instrument_midi_channel` | Multitimbral routing |

### Automation

Pattern-level breakpoint automation for instrument macros (Volume, Pan, Filter
Cutoff/Resonance, Attack/Decay/Sustain/Release) and any continuous, RT-safe
per-module parameter. Targets can be given as a structured `target` object or the
`module:<type>:<instance>:<param>` DSL string; the `Exponential` curve takes a
`curve_strength` (-127..=127). See [AI-Friendly Features](#ai-friendly-features).

| Tool | Purpose |
|------|---------|
| `get_instrument_automation_targets` | Discover the valid targets (per-module + macros) for an instrument before editing |
| `add_automation_points`, `remove_automation_points` | Edit |
| `list_automation_lanes`, `get_automation_points` | Read |
| `clear_automation_lane` | Wipe |

### Audio Analysis (the AI killer feature)

Offline-rendered, deterministic, quantitative feedback. These let an agent hear what it built.

| Tool | Returns |
|------|---------|
| `analyze_harmony` | Chord progression (18 templates), key inference (24 keys via Krumhansl–Schmuckler), in-key ratio, out-of-scale notes |
| `analyze_mix_bus` | LUFS-I, peak, RMS, crest factor, 4-band frequency balance, stereo correlation, mid/side energy, mono-compat score, clipped-sample count |
| `analyze_section` | Same as `analyze_mix_bus` plus per-track contribution breakdown (via soloing) |

All three are bit-exact reproducible across calls — `fastrand` is reseeded and `BTreeMap` is used for module
iteration.

### Samples & Sampler

| Tool | Purpose |
|------|---------|
| `list_samples`, `import_sample`, `delete_sample`, `rename_sample`, `duplicate_sample`, `export_sample` | Library |
| `get_sample_info` | Length, sample rate, channels, root note |
| `normalize_sample`, `reverse_sample`, `trim_sample_silence` | DSP |
| `set_sample_loop`, `set_sample_crop`, `set_sample_root_note` | Slicing & mapping |
| `assign_sample_to_module`, `get_sampler_state`, `set_sampler_parameter` | Sampler module |

### Acoustic World Engine

Spatial audio and room simulation.

| Tool | Purpose |
|------|---------|
| `get_awe_state`, `set_awe_enabled`, `set_awe_parameter` | Core |
| `set_awe_room_shape`, `set_awe_material`, `set_awe_lfo` | Room & modulation |
| `set_awe_preset`, `list_awe_presets` | Preset library |

### Audio Input

| Tool | Purpose |
|------|---------|
| `list_input_devices`, `get_input_state` | Live input routing |

### Batch & Utility

| Tool | Purpose |
|------|---------|
| `batch_execute` | Run up to 50 arbitrary tool calls in one request |
| `optimize_project` | Remove unused patterns, instruments, samples |

---

## AI-Friendly Features

Pertylizer's MCP layer is designed *for agents*, not as an afterthought.

### Auto-inference of instrument roles

`get_instrument_profiles` analyzes the module graph + parameters and returns a role per instrument:

```json
{
  "instrument_id": 3,
  "role": "Bass",
  "confidence": 0.87,
  "signal_trail": [
    "decision:bass-low-osc-frequency",
    "envelope:sustained",
    "graph:filter-after-osc"
  ]
}
```

Agents don't need to manually `set_instrument_category` — the synth tells them what each instrument *is*.

### Self-describing, validated automation

Automation is designed so an agent can pick the right target on the first try
instead of guessing a stringly-typed DSL:

- **Discover before you write.** `get_instrument_automation_targets(instrument_id)`
  returns every valid target for an instrument — the instrument macros plus each
  automatable per-module parameter — with a ready-to-use `target` string, `unit`,
  `min`/`max`, and `response_curve`. No need to reverse-engineer `type_id`s or
  instance indices from `list_modules`.

```json
// get_instrument_automation_targets(instrument_id=1) -> a flat array of targets
[
  { "target": "module:flt:4:cutoff", "kind": "module", "module_id": "flt-4",
    "param_id": "cutoff", "display_name": "Cutoff", "unit": "Hz",
    "min": 20.0, "max": 20000.0, "response_curve": "Logarithmic" },
  { "target": "FilterCutoff", "kind": "instrument", "display_name": "Filter Cutoff" }
]
```

Each entry's `target` string is ready to pass straight to the automation tools;
`kind` distinguishes per-`module` parameters from instrument-level macros.

- **Structured targets.** `add_automation_points` accepts a typed `target` object
  in addition to the `module:<type>:<instance>:<param>` / macro string. The
  structured form is a tagged union mirroring the engine's `AutomationTarget`, so
  there is no prose grammar to memorize:

```json
{
  "pattern_id": 0,
  "points": [
    { "target": { "module": { "module_type": "flt", "instance": 1, "param_id": "cutoff" } },
      "instrument_id": 1, "beat": 0.0, "value": 0.1, "curve": "Linear" },
    { "target": { "instrument": { "param": "FilterCutoff" } },
      "instrument_id": 1, "beat": 4.0, "value": 0.9,
      "curve": "Exponential", "curve_strength": -40 }
  ]
}
```

- **Instance validation.** Per-module targets are checked against the instrument's
  real graph and the automatable allowlist (continuous + RT-safe, no enum or
  structural params), so a target that points at a module instance that doesn't
  exist is rejected rather than creating a silently dead lane.
- **Curve strength.** The `Exponential` curve carries a `curve_strength`
  (-127..=127; negative = ease-in, positive = ease-out).
- **Response curves exposed.** Module/parameter listings include each parameter's
  `response_curve` (e.g. cutoff is `Logarithmic`) and `unit`, so an agent can
  convert a real value into the correct 0..1 lane value.

### Forgiving input vocabulary

The tools accept the shapes an agent naturally reaches for:

- **Module-type tokens** are case-insensitive and accept the short key (`"flt"`),
  the snake_case full name (`"ladder_filter"`), or the display name
  (`"Ladder Filter"`) — in `add_module`, `build_instrument`, and automation targets.
- **Parameter values** in `set_parameter` accept a number in the
  native range, a choice *string* (`"sawtooth"`, matched against the choice id or
  display name), or a boolean for on/off params — not just raw floats.
- **Canonical param keys.** Parameter listings expose the snake_case `type_id`
  (e.g. `"cutoff"`) next to the human-readable `name`, so the same key works for
  reading, automating, and addressing a parameter.

### Deterministic offline analysis

`analyze_section` and `analyze_mix_bus` render audio offline (separate `SynthEngine` instance, no shared state with
the live engine) and return quantitative metrics. Calls are bit-exact reproducible, so an agent can A/B compare
parameter changes by re-running analysis.

### Batch operations to save tokens

Most mutating tools accept either a single item or an array, so pass many items in
one call instead of one call per item:

- `add_note`, `update_note`, `replace_notes` — many notes at once
- `create_pattern`, `place_pattern`, `create_track` — arranger setup in one call
- `build_instrument` — one or many full patches in one call
- `set_parameter` — atomic multi-parameter changes
- `add_module` / `remove_module`, `add_return_effect` / `remove_return_effect`,
  `add_master_effect` / `remove_master_effect` — build or tear down chains in one call
- `remove_note`, `remove_placement`, `remove_track_send`, `remove_return_send`,
  `delete_instrument`, `delete_pattern`, `delete_track`, `delete_return_bus`,
  `delete_sample` — bulk removal by passing many IDs
- `set_song` — a whole arrangement in one call; `batch_execute` for cross-domain orchestration

### Structured errors

Tools return typed errors (`McpBridgeError`) with explicit causes — invalid MIDI note, missing instrument, port
mismatch on connect, etc. Agents can recover programmatically without parsing prose.

### Diagnostics

`get_graph_diagnostics` reports feedback loops, unreachable modules, modules with no audio path to the output, and
mod-matrix slot-source mismatches. Run it after any patch edit to catch broken graphs before listening.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  MCP client (Claude Code, Claude Desktop, custom)       │
└────────────────────────┬────────────────────────────────┘
                         │  JSON-RPC over HTTP or stdio
                         ▼
┌─────────────────────────────────────────────────────────┐
│  synth_mcp::server (rmcp + axum)                        │
│  • ~150 #[tool] handlers                                │
│  • Validates JSON params, serializes results            │
└────────────────────────┬────────────────────────────────┘
                         │  SynthBridge trait (primitive types only)
                         ▼
┌─────────────────────────────────────────────────────────┐
│  AppSynthBridge (in pertylizer crate)                   │
│  • Reads EngineState snapshots (Arc<RwLock<…>>)         │
│  • Sends EngineCommands via lock-free MPSC              │
└────────────────────────┬────────────────────────────────┘
                         │  Non-blocking channel send / snapshot read
                         ▼
┌─────────────────────────────────────────────────────────┐
│  Audio thread (synth_engine::SynthEngine::process)      │
│  • Real-time, lock-free, zero allocations               │
│  • Drains EngineCommand queue between blocks            │
└─────────────────────────────────────────────────────────┘
```

The bridge pattern keeps the audio thread fully decoupled from MCP. The audio thread **never blocks** on
JSON parsing, network I/O, or lock acquisition — MCP can be slow without affecting playback.

For offline analysis (`analyze_*`), a fresh `SynthEngine` is constructed from a snapshot of the current project and
rendered to a buffer. The live engine is untouched.

### Real-time safety guarantees

- All MCP→engine writes go through an MPSC `EngineCommand` queue (non-blocking send).
- All engine→MCP reads go through `EngineState` snapshots (non-blocking `RwLock` read).
- Heavy work (offline render, sample loading, project I/O) happens on the MCP thread, never on the audio thread.

---

## Example Workflows

### "Build me a techno kick"

1. Agent calls `create_instrument(name="Kick", category="Drums")`.
2. Agent calls `build_instrument` with a JSON spec: sine osc → fast pitch envelope → fast amp envelope → output.
3. Agent calls `note_on(instrument_id, note=36, velocity=100)` to audition.
4. Agent calls `analyze_section` after placing it in a pattern, checks LUFS-I and sub-band energy, adjusts gain.

### "Compose a 16-bar progression in C minor"

1. Agent calls `set_song_tempo(120)`, `set_song_time_signature(4, 4)`.
2. Agent calls `create_pattern` for verse/chorus.
3. Agent calls `add_note` with chord voicings.
4. Agent calls `analyze_harmony` to verify the progression matches C minor (24 keys auto-detected).
5. Agent calls `place_pattern` to arrange the song.

### "Mix the song"

1. Agent calls `analyze_mix_bus` for global metrics.
2. Agent calls `analyze_section` per section to find the loudest/dullest parts.
3. Agent calls `set_track_volume`, `set_track_pan`, `set_instrument_volume` to balance.
4. Agent re-runs `analyze_mix_bus` to verify LUFS-I target.

### "Inspect what the user just patched"

1. Agent calls `list_instruments` then `get_instrument_profiles`.
2. For each instrument, agent calls `get_instrument_info` to read modules + connections.
3. Agent calls `get_graph_diagnostics` to surface any broken cables or feedback loops.

---

## See Also

- [`README.md`](../README.md) — main project overview
- [`docs/history.md`](history.md) — version history including MCP tool additions
- [Model Context Protocol spec](https://modelcontextprotocol.io)
