# Sampling & Audio Recording — Status & Backlog

> **Shipped** (through v0.262.0). The sampling feature is functional end-to-end:
> `synth_sampler` crate, WAV import/export, Sample view (waveform + crop/loop +
> toolbar), audio input recording, `Sampler` + `AudioInput` Rack modules, ZIP
> bundle project format, and the MCP tool surface.
>
> This file was trimmed from the original 1448-line design spec once the feature
> landed — the **full design document** (architecture, type definitions, signal-
> flow diagrams, security gates, and all 20 future-feature ideas) lives in git
> history (`git log --follow -- plans/sampling-plan.md`). Kept here: the shipped
> summary and the remaining backlog.

## Done

All **P0 (must-have)** items complete (52/52) plus most P1. Phases 1–8 of the
original plan landed: foundation (sample data + WAV I/O), Sample view GUI, audio
input recording, Sampler module, live audio input module, bundle format, MCP
integration, and polish. See `docs/history.md` (v0.251.0–v0.262.0).

| Priority | Done | Remaining |
|----------|------|-----------|
| P0 (must have)    | 52 | 0  |
| P1 (should have)  | 19 | 5  |
| P2 (nice to have) | 1  | 22 |
| P3 (future)       | 0  | 13 |

## Backlog — P1 (the real gaps)

These are the only "should-have" items still open; all relate to MCP recording/
monitoring being GUI-side state that the bridge doesn't yet reach:

- [ ] Engine-side sample cache (`HashMap<SampleId, Arc<[f32]>>`) +
  `EngineCommand::LoadSample`/`UnloadSample` (the `needs_sample_reload` flag exists
  but the engine doesn't poll it; `load_sample()` is called manually today).
- [ ] `pending_sample_ops` in `McpSharedState` — the channel GUI-side MCP ops need.
- [ ] Wire `list_input_devices` to real backend enumeration (currently `Ok(vec![])`).
- [ ] Wire `get_input_state` to real monitoring/recording state (currently static idle).
- [ ] MCP `set_input_device` / `start_monitoring` / `stop_monitoring` /
  `start_recording` / `stop_recording` — need the GUI-side op channel above.

## Backlog — P2 (nice to have)

- Sample view: draggable crop handles + loop markers (currently DragValue only),
  playback cursor during preview.
- Sampler module: mini waveform preview in the Rack panel, crossfade at loop
  points (field exists, DSP missing), cubic Hermite interpolation.
- Live input: "Live" indicator when active, one-per-instrument limit.
- Bundle: `sample_refs` field in `ProjectFile`, `.pertpatch` patch bundle format,
  sample deduplication, explicit v1→v2 migration path.
- MCP: `normalize_sample` `target_db` argument (always 0 dB now),
  `preview_sample`/`stop_preview`, server-instructions "Sampling" section.
- Recording: countdown/pre-roll, latency warning badge (>20 ms).
- Polish: sample preview in Rack (click to audition), drag-and-drop WAV from file
  manager, undo/redo for sample edits, sample-usage tracking + delete warning,
  performance testing on >10-min samples.

## Backlog — P3 (future / stretch)

Sinc-interpolation resampling, anti-alias LP filter for pitch-up, dedicated
recording thread, disk streaming for large files, multi-sample instruments, sample
slicing, timestretch, granular sampler (`GrainSource::Sample`), audio track in
sequencer, resample-engine-output, and the rest of the original §12 "future
feature ideas" list (zero-crossing snap, convert-to-wavetable, drum rack, …) —
all preserved in the git-history copy of this file.
