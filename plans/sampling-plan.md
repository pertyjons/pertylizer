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
| P1 (should have)  | 19 | 6  |
| P2 (nice to have) | 1  | 24 |
| P3 (future)       | 0  | 11 |

## Backlog — P1 (the real gaps)

These are the only "should-have" items still open; all relate to MCP recording/
monitoring being GUI-side state that the bridge doesn't yet reach:

- [ ] Engine-side sample cache (`HashMap<SampleId, Arc<[f32]>>`) +
  `EngineCommand::LoadSample`/`UnloadSample` (the `needs_sample_reload` flag exists
  but the engine doesn't poll it; `load_sample()` is called manually today).
  **RT rule (review 2026-06-22):** the `HashMap` must live on the control side —
  the `LoadSample` command carries the already-resolved `Arc<[f32]>`, so the audio
  thread never hashes/looks up by `SampleId`. The module already holds its data
  directly (`Sampler.sample_data: Option<Arc<[f32]>>`) and only `Arc::clone`s it at
  `note_on`, so no map access is on the hot path; this rule just keeps the *cache*
  resolution off the audio thread too.
- [ ] **Sample-data trash ring for `UnloadSample` / sample-replace** (review
  2026-06-22). `Sampler::unload_sample` / `load_sample` overwrite the module's
  `Arc<[f32]>` (and per-voice `SamplePlayer` clones) during the audio-thread command
  drain; if the cache/library already dropped its ref, that decrement is the *last*
  one and `free()` runs on the audio thread. Route replaced sample `Arc`s through a
  deferred-drop ring exactly like the existing `automation_trash` / `script_trash`
  channels. Low-probability today (the module keeps its own ref, so the player-clone
  drop is normally a non-last decrement), but the correct pattern when unload lands.
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
  - *Crossfade (review 2026-06-22):* default to a **static/baked** crossfade —
    the control thread bakes the fade into a modified buffer in RAM when loop points
    change (free on the audio thread). Add **dynamic** dual-read crossfade (reads
    both loop end and start, ~2× cost during the fade) only later, for when loop
    points are modulated.
    - *Round-2 caveats:* baking is **destructive** — it rewrites the loop region in
      the play buffer, so the **original sample must be kept alongside the baked
      copy** (≈2× RAM for that sample) to restore when looping is turned off or a
      loop point moves. It also **architecturally precludes modulated loop points**
      (an LFO/envelope on loop start/end can't re-bake 48 k times/s). Acceptable for
      v1, but if loop-point modulation is ever planned, `playback.rs` must be
      prepared for dual-read up front — record this as a known lock-in.
  - *Interpolation (review 2026-06-22):* Hermite (4-point cubic) is the cheap RT
    quality step and the **baseline for all sample lengths**. The
    **pre-oversample-in-RAM** trick (e.g. 8× on load → cheap linear interp ≈ sinc
    quality, no per-voice CPU) must be **conditional on sample length** — 8× turns a
    5-min stereo 48 kHz file (~115 MB) into ~1 GB RAM. Only oversample short samples
    (e.g. < ~10 s, drum/one-shot range); longer files fall back to plain RT Hermite
    (or P3 disk streaming). Sinc stays P3. (For pitch-*up* aliasing specifically,
    mipmaps are the preferred tool — see the P3 anti-alias note.)
- Zero-crossing snap for crop/loop points (moved up from P3, review 2026-06-22).
  Pure control-thread / GUI work (a small search in the sample Vec for the nearest
  zero crossing) — cheap, big UX win, and removes most loop clicks before crossfade
  DSP exists.
  - *Round-2 detail:* match the **slope sign**, not just proximity to 0.0. If loop
    start snaps to a rising zero-crossing and loop end to a falling one, the seam
    phase-inverts → audible click / bass loss even though the amplitude was zero.
    Require both points to cross zero in the **same direction** (e.g. both rising).
- **Recording-drain thread (moved up from P3, review round 2, 2026-06-22).** Drain
  the input ring buffer on a dedicated low-priority thread instead of the egui GUI
  thread. Today's GUI-thread drain (~60 fps) is fragile: a blocking file dialog, a
  large project save (disk write on the main thread), or the OS deprioritizing a
  minimized window can each stall the main thread > 5 s and overflow the ~5.5 s
  ring → dropped recording. It also means recording **only works while the GUI
  renders** — headless / MCP-only operation can't record at all. The fix is small
  (a thread with a loop + short `sleep` draining the ring into the record buffer/
  file) and makes recording immune to GUI stalls and headless-capable. The audio
  thread is already safe (see notes below); this is purely about who drains.
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

Sinc-interpolation resampling, anti-alias filtering at extreme pitch, disk
streaming for large files, multi-sample instruments, sample slicing, timestretch,
granular sampler (`GrainSource::Sample`), audio track in sequencer,
resample-engine-output, and the rest of the original §12 "future feature ideas"
list (convert-to-wavetable, drum rack, …) — all preserved in the git-history copy
of this file. (The recording-drain thread, formerly here, moved up to P2.)

Clarifications from the 2026-06-22 review:

- **Anti-alias at pitch (was "LP filter for pitch-up") — prefer mipmaps.** Pitch-*up*
  aliases near-Nyquist content; pitch-*down* causes imaging that wants better
  interpolation too. Round-2 DSP verdict: **mipmapping is the better route** than a
  per-voice pitch-controlled LP. A real-time per-voice filter has to be steep
  (4-pole+) to actually suppress aliasing above Nyquist and gets expensive across
  many voices; mipmaps (a few octave-band-limited copies, texture-style) cost only
  ≈ +33 % RAM (1 + ½ + ¼ + … ≈ 1.33×) and allow a **perfect offline linear-phase
  brickwall** filter at load time — cleanest and cheapest for pitch-up. Keep the
  2-pole-LP idea only as a fallback if mipmap RAM/precompute is ever unwanted.
- **"Dedicated recording thread" → recording-*drain* thread, not an audio-thread
  fix.** Audio-input recording is *already* RT-safe: the cpal input callback only
  writes to SPSC ring buffers (`audio/input.rs`), the file write happens on stop
  (control thread), and note recording (`recording.rs`) is pre-allocated /
  spare-Vec-swap with no audio-thread alloc. The residual gap is that the ring is
  drained on the **GUI thread at ~60 fps**, so a GUI stall (blocking file dialog,
  big project save, minimized window) can overflow the ~5.5 s ring and drop audio,
  and headless / MCP-only operation can't record at all. **Round-2 update: moved up
  to P2** — it's a small, high-value robustness fix (a dedicated drain thread), not
  the audio-thread hazard the first review assumed. See the P2 item above.
- **Multi-sample instruments — pull the zone *data model* earlier.** Building the
  `Sampler` around one `sample_id` will force a voice/GUI rewrite when keyzones +
  velocity layers arrive. Introducing a `SampleZone { sample_id, min/max_note,
  min/max_velocity }` array now (single full-range zone in the first GUI) avoids
  that structural churn. Note the usual file-format-migration argument is **weaker
  here** — the project runs no-backward-compatibility (CLAUDE.md), so a format break
  is cheap; the real payoff is avoiding the engine/voice rewrite, so treat this as
  optional risk-reduction, not a blocker.

## Architecture & RT review notes (2026-06-22)

A DSP/RT review of this plan was cross-checked against the shipped code. Recorded
so the already-handled points are not re-raised:

- **Audio-thread allocation/free is already controlled.** `Sampler.render_buffer`
  is pre-allocated to 16384 (8192 stereo frames) and only grows past a host block
  larger than that (never in practice); the audio-input path is ring-buffered (see
  above). A purist follow-up: do any block-size-driven resize on a control-thread
  buffer-size callback rather than lazily in `process()`.
- **`fmod` in the loop wrap.** `playback.rs::check_and_wrap_position` uses `%` on
  `f64`, but only when crossing a loop boundary, not per sample, and for normal
  speeds the overshoot is < `loop_len` so the modulo is a no-op. A
  `while pos >= end { pos -= loop_len }` is marginally cheaper; micro-opt, low
  priority (modulo only earns its keep at extreme speeds on tiny loops).
- **Headless / device ownership.** `AudioInputManager` lives in the `pertylizer`
  app crate, which is why the MCP bridge can't reach device enumeration /
  monitoring state — exactly the open P1 items. For true headless (MCP-only) input,
  device/host ownership needs to be reachable without the egui layer; fold this
  rationale into those P1 items when they're built.
