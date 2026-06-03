# Pertylizer

A modular synthesizer with 69 module types (including 25 effects), a sequencer,
an acoustic room engine, and MCP integration for driving it from an AI CLI.

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

## Working with AI (MCP)

Once your AI CLI is connected (see above), you don't call tools by hand — you describe what you
want in plain language and the agent picks from Pertylizer's 180+ MCP tools. Keep Pertylizer open
while you work: every change is applied to the live engine, so you hear it immediately.

The agent can also *listen back* to what it changed. Tools like `analyze_mix_bus`,
`analyze_section`, `analyze_harmony` and `analyze_masking_matrix` render the song offline and return
hard numbers — LUFS loudness, true-peak, per-band energy, inferred key, spectral conflicts — so the
agent works from measurements, not guesswork.

### Example: tighten up a muddy mix

Open one of the example projects (`examples/projects`), then talk to the agent:

1. **"Load *Neon Horizon* and tell me what's in it."**
   The agent calls `load_project`, then `get_song_info`, `list_tracks` and `get_instrument_profiles`
   to map the song and auto-detect each track's role (drums / bass / lead / pad …).

2. **"How does the mix look, and which track is causing problems?"**
   `analyze_mix_bus` (with `include_per_track`) reports integrated loudness, true-peak, the 4-band
   balance and a per-track breakdown — so the agent can say e.g. *"the master clips at +0.8 dBTP and
   the pad owns most of the low-mids."*

3. **"The chorus sounds muddy — what's masking what?"**
   `analyze_section` compares the chorus range to the verse, and `analyze_masking_matrix` finds the
   worst spectral collisions (e.g. *"Pad(2) masks Lead(3) in the 500–2000 Hz band"*).

4. **"Check the harmony too."**
   `analyze_harmony` infers the key and flags out-of-scale notes; `suggest_music_fixes` rolls the
   harmony, mix, groove and arrangement findings into one ranked to-do list.

5. **"Fix it: clean up the low-mids, give the lead room, and tame the peak."**
   Now the agent *changes* things — `set_track_volume` / `set_instrument_pan` to rebalance, an EQ on
   the pad (`add_module`) to carve out the band the lead needs, a master limiter
   (`add_master_effect`), and `auto_gain_stage` to hit a sensible loudness without breaching the
   true-peak ceiling. Shared reverb/delay goes on a send via `create_return_bus` + `set_track_send`.

6. **"Did that actually help?"**
   The agent wraps the change in `compare_mix_before_after`: capture a baseline, apply the fixes,
   re-render, and read back the deltas (loudness, true-peak, dynamics, stereo width) — a concrete
   *before → after*, not a vibe.

### Example: shape the performance

Mix balance is only half of it — the agent can also play the parts more musically:

1. **"Give the lead a smooth portamento and a little vibrato."**
   Note expression lives on the notes themselves, so the agent re-emits the lead pattern with
   `replace_notes`, each note carrying a `glide` (`from_semitones: -2`, `time_ms: 80`,
   `interp: "continuous"` for smooth portamento — or `"stepped"` for a chromatic glissando) and an
   `expression.vibrato` block (depth, rate, delay). Setting `legato: true` ties a phrase together so
   the glide doesn't retrigger the envelope. (`add_note` takes the same fields when writing new
   parts; `update_note` only moves pitch/timing/velocity.)

2. **"Open the filter through the chorus and swell the delay at the end."**
   The agent calls `get_instrument_automation_targets` to discover the exact target strings for that
   instrument's modules — *including the effects in its chain* — then `add_automation_points` to draw
   the lanes: e.g. `module:flt:1:cutoff` ramping up across the chorus and `module:delay:1:mix`
   swelling toward the end. Effect parameters automate exactly like any other module parameter; use
   an `Exponential` curve with a `curve_strength` for a non-linear sweep.

It's all one conversation: you steer, the agent measures, edits and verifies, and you hear each step
live in Pertylizer.
