# Pertylizer

A modular synthesizer with 35 voice modules, 21 effects, a sequencer, an
acoustic room engine, and MCP integration for driving it from an AI CLI.

This package contains everything you need to run Pertylizer and its 3D
visualizer, plus example projects and ready-made AI-CLI configs.

## Contents

```
pertylizer            The synthesizer (GUI app)
pertylizer-visualizer 3D OSC-driven visualizer
examples/             Example projects, patches, and AWE presets
pertylizer.toml       Editable config (MCP / OSC ports) — read at startup
mcp/                  AI-CLI MCP configs + setup guide (see mcp/README.md)
.mcp.json             Project-local Claude Code config (works in this folder)
.gemini/              Project-local Gemini config
.ai/                  Scratch directory used by the app (backups, exports)
```

## Running

### Linux
```
chmod +x pertylizer pertylizer-visualizer    # first time only
./pertylizer
```

### Windows
Double-click `pertylizer.exe`. (SmartScreen may warn on first launch —
choose *More info → Run anyway*.)

### macOS
Double-click `Pertylizer.app`. The app is **unsigned**, so the first time
macOS will block it: **right-click the app → Open → Open**. After that it
launches normally. Run the visualizer the same way (or from a terminal).

## Examples

Open Pertylizer and load a project from `examples/projects`, a patch from
`examples/patches`, or an AWE preset from `examples/awe`.

## Configuration (`pertylizer.toml`)

Ports are read from `pertylizer.toml` next to the executable. Edit it to change
the MCP HTTP port (default `9850`) or the OSC telemetry port (default `9000`).
The visualizer reads the same file, so both stay in sync. If you change the MCP
port, update the matching AI-CLI config — see `mcp/README.md`.

## Driving it from an AI CLI

Start Pertylizer, then run your AI CLI (e.g. `claude` or `gemini`) **from this
directory** — the bundled `.mcp.json` / `.gemini` configs connect to the
`synth` MCP server automatically. Full details and other clients (Antigravity,
Codex) are in `mcp/README.md`.
