# MCP Music Tools — Plan

> **Date:** 2026-05-11 (updated 2026-05-11 after Tier 0 ship)
> **Status:** **Tier 0 shipped in v0.276.0**; Tier 1+ pending.
> **Scope:** New MCP tools that give an AI agent the ability to evaluate and shape music as a whole, not only individual sounds.

---

## 0. Status snapshot

### Shipped — v0.276.0 (Tier 0)

| Tool | Version | Notes |
|------|---------|-------|
| `analyze_harmony` | v0.276.0 | Walks notes by tick, identifies chord symbols (18 templates incl. m7/maj7/dom7/sus/dim/aug/power), infers key via Krumhansl-Schmuckler over 24 major/minor keys, returns in-key ratio, out-of-scale pitch classes, composite stability score. Pattern or arrangement scope. Returns chord events with `start_bar`/`start_beat` (1-indexed). Default grouping: 1 quarter-note for pattern scope, 1 bar for arrangement scope. Consecutive identical chord events are merged. |
| `analyze_mix_bus` | v0.276.0 | Renders N seconds (default 10, max 300) of the master bus offline from `start_tick` (default 0). Returns sample peak, RMS, crest factor, integrated LUFS (ITU-R BS.1770-4 K-weighted + 400 ms gating, 75 % overlap, abs −70 / rel −10 gates), 4-band frequency-balance RMS (sub/low/mid/high) windowed across the full buffer, stereo correlation, mid/side RMS, stereo width, mono-compatibility score, clipped-sample count. Includes `start_bar`/`end_bar`/`start_beat`/`end_beat`. |
| `analyze_section` | v0.276.0 | Same metrics as `analyze_mix_bus` but takes explicit `[start_tick, end_tick)`. Per-track contribution breakdown deferred (see §6). |

### Deferred / in-progress

- **Per-track contribution breakdown for `analyze_section`** — Tier-0 ships master-only. Deliberate scoping decision to ship a complete and tested master path first; per-track stems land as a follow-up.
- **`true_peak` (inter-sample peak)** — currently reports sample peak; inter-sample peak would catch over-shoots that emerge after DA conversion. Important for mixes that already clip on sample-peak (the test song clipped at 22 % of samples in chorus).
- **LUFS-S / LUFS-M (momentary / short-term)** — only integrated LUFS shipped. Momentary LUFS would help locate the hottest spot inside a section.

### Tier 0 follow-up commits

- `aa48cde` — `analyze_harmony` + plan
- `ef2f767` — `analyze_mix_bus` + `analyze_section` + offline arrangement renderer (v0.276.0)
- `869308e` — integration test for the offline arrangement renderer
- `cf34b3b` — cleanup pass: bug fixes, dedupe, tighten visibility
- `41d8bf6` — live-testing follow-ups: fix Seek-overrun, fix `analyze_harmony` token bloat, add bar/beat positions, fix single-note chord mislabeling

---

## 1. Motivation

`analyze_note` lets an AI hear and reason about a single patch in isolation. That covers the **sound design** loop:
build a patch → render one note → read metrics → adjust.

It does **not** cover the **music-making** loop. As soon as the AI has more than one instrument playing together, or
writes a pattern of more than a few notes, it is blind to:

- Whether the mix is balanced, muddy, masking itself, or clipping.
- Whether the chords it wrote are harmonically coherent, in a key, or moving anywhere.
- Whether the rhythm has groove, variation, or is robotically uniform.
- Whether a patch holds up across its full playing range, not only at C4.
- Whether two sections of the song actually differ in energy / density / timbre.

The tools below close that gap. The order in §3 is the recommended build order — biggest blind spot first.

---

## 2. Proposed Tools (catalogue)

All tools follow the `analyze_note` style: synchronous render-and-analyze on the audio thread snapshot, return
quantitative metrics as a typed struct, no WAV roundtrip required.

### 2.1 Mix-level audio analysis

| Tool                | Purpose                                                                                              |
|---------------------|------------------------------------------------------------------------------------------------------|
| `analyze_mix_bus`   | Render N seconds of the master bus during arrangement playback; return LUFS-I/S/M, true peak, RMS, crest factor, frequency balance in bands (sub/low/lo-mid/hi-mid/high/air), stereo correlation, mid/side energy, mono-compatibility score. |
| `analyze_section`   | Render an arrangement range `[start_tick, end_tick]` offline; return the same metrics as `analyze_mix_bus` plus a per-track contribution breakdown (which track owns how much of each band's energy). Lets the AI spot masking and unmixed elements. |
| `analyze_track`     | Solo-render a single track over a range; same metrics, plus dynamic range and silence ratio. Enables A/B against the full mix to detect which track dominates. |
| `render_section_to_wav` | Render `[start_tick, end_tick]` to a WAV file the AI can fetch (or get base64 back). Escape hatch when built-in metrics aren't enough and the AI wants to do its own DSP — and a building block for the comparison tools below. |
| `get_mix_meters`    | Cheap live read of current peak / RMS / LUFS-S during real-time playback. Useful during interactive sessions where AI is tweaking knobs and wants a meter, not a full render. |

### 2.2 Note / composition analysis

| Tool                  | Purpose                                                                                          |
|-----------------------|--------------------------------------------------------------------------------------------------|
| `analyze_harmony`     | Walk a pattern (or arrangement range) tick-by-tick, group simultaneous notes, return a chord progression with chord-symbol labels (`Cm7`, `F7sus4`), inferred key/scale, and a "harmonic stability" score. Flags out-of-scale notes. |
| `analyze_pattern`     | Per-pattern stats: note count, polyphony histogram, pitch range, average velocity, velocity variance, rhythmic density (notes/beat), swing estimate, voice-leading distance between consecutive chords, repetition factor. |
| `analyze_groove`      | Micro-timing analysis on a pattern: deviation from grid in ms, swing percentage on each subdivision, velocity grouping per beat position. Detects mechanical vs. humanized timing. |
| `analyze_arrangement` | Whole-song shape: per-section (or per-bar) energy curve, instrument-activity matrix, key changes, tempo changes, average density. Returns a "section map" the AI can use to spot a missing bridge or an over-long intro. |
| `compare_patterns`    | Similarity score between two patterns: rhythmic similarity, pitch-contour similarity, harmonic similarity. Lets the AI verify "this variation is actually different" or "these two patterns are too samey". |

### 2.3 Patch behavior across range

| Tool                       | Purpose                                                                                  |
|----------------------------|------------------------------------------------------------------------------------------|
| `analyze_instrument_range` | Sweep an instrument across a MIDI note range (configurable step), run a lightweight version of `analyze_note` at each step. Returns per-note: fundamental, peak amplitude, brightness (centroid), and stability flags. Surfaces aliasing in the top octaves, energy loss in the bass, envelope blow-ups at velocity 127, and patches that simply stop tracking pitch above a certain note. |
| `analyze_velocity_response`| Hold one note, sweep velocity 1..127, report amplitude curve, brightness curve, and any non-monotonic behavior. Confirms that the patch *responds* to velocity at all and how musically. |

### 2.4 Composition helpers (symbolic, not audio)

These don't analyze — they let the AI manipulate notes at a level above "one note at a time", which is currently
the only granularity available for note edits.

| Tool                       | Purpose                                                                                  |
|----------------------------|------------------------------------------------------------------------------------------|
| `generate_chord`           | Symbol → notes: `("Cm7", octave: 4, voicing: "drop2") → [C4, Eb4, G4, Bb4]`. Saves the AI from re-deriving intervals every time. |
| `transpose_notes`          | Transpose a set of notes (or a whole pattern) by semitones, optionally constrained to a scale. |
| `quantize_notes_to_scale`  | Snap pitches in a pattern to the nearest scale degree of a given key/scale. Cleans up generated material that drifted out of key. |
| `quantize_notes_to_grid`   | Snap timings to a grid with optional swing and humanization amount. |
| `humanize_notes`           | Add bounded random offsets to timing and velocity with configurable amounts. Useful when AI-written patterns sound mechanical. |
| `generate_variation`       | Produce N variations of a pattern with controllable mutation rate (note drop / add / shift / velocity jitter). |

### 2.5 Reference / A-B

| Tool                  | Purpose                                                                                       |
|-----------------------|-----------------------------------------------------------------------------------------------|
| `compare_to_reference`| Render a section, run the same DSP analysis on a reference WAV path, and return a diff report: frequency-balance delta per band, LUFS delta, stereo-width delta. Closes the "make it sound like X" loop. Builds on `render_section_to_wav`. |
| `compare_patches`     | Run `analyze_note` on two patches with the same note/velocity and return a structured diff (brightness, harmonic richness, envelope shape, stereo width). Faster than the AI eyeballing two `AnalyzeNoteResult` blobs side by side. |

---

## 3. Prioritization for AI utility

Sorted by how much each tool removes a current blind spot, weighted by how often the AI hits that blind spot.

### Tier 0 — Biggest impact, build first  ✅ **Shipped in v0.276.0**

1. ✅ **`analyze_mix_bus`** — Without this, every mix decision the AI makes is a guess. The single tool that converts
   Pertylizer from "AI can build sounds" into "AI can hear its own music". LUFS + per-band energy + sample peak +
   stereo correlation covers ~80 % of mix-bus debugging. *(True peak deferred — see §0.)*
2. ✅ **`analyze_section`** — `analyze_mix_bus` over an arbitrary arrangement range. *(Per-track contribution
   breakdown deferred — see §0.)*
3. ✅ **`analyze_harmony`** — Currently the AI generates chord progressions by writing MIDI notes and has no feedback
   on whether the result is in a key, harmonically static, or accidentally dissonant. Chord-symbol + key inference
   converts notes back into the language the AI plans in.

### Tier 1 — High impact, build next

4. **Per-track contribution breakdown for `analyze_section`** — deferred from Tier 0. Live testing shows
   `analyze_mix_bus` reports clipping and high LUFS, but the AI can't tell *which* track is the offender. Two
   sub-options: (a) N+1 renders (master + each track soloed) — simple, correct, O(N) slower; (b) engine change to
   tap per-instrument output pre-master — fast, invasive. Recommendation: ship (a) first.
5. **`exclude_drums_from_harmony`** (or a `drum_track_ids` parameter on `analyze_harmony`) — live testing surfaced
   that drum/percussion MIDI pitches pollute chord identification. The "Tung Synthpop" test song reports `F#m7b5`
   on bar 1 because the hi-hat plays MIDI 42 (F#2) on top of an Am pad; the analyzer correctly identifies the
   pitch set but it's musically wrong. Either honor an instrument category flag or expose a parameter to skip
   percussion tracks.
6. **`analyze_pattern`** — Cheap, no audio rendering. Density, range, velocity variance, repetition factor. Lets
   the AI verify "is this pattern interesting?" without rendering, and is a prerequisite for sane
   `generate_variation` heuristics.
7. **`analyze_instrument_range`** — Catches the bug class where a patch sounds great at C4 in `analyze_note` and
   falls apart at C6 or C2. Especially important once an AI is committing patches into a project without a human
   playing test notes.
8. **`render_section_to_wav`** — Even without immediate analysis tools layered on top, this unlocks the AI sending
   audio back to a human or feeding it to a separate model. Building block under `compare_to_reference`.
9. **True-peak + LUFS-S/M for `analyze_mix_bus`** — sample-peak misses inter-sample peaks; LUFS-S/M locate the
   hottest moments. Cheap additions on top of the already-shipped LUFS-I pipeline.

### Tier 2 — Quality-of-life and composition

10. **`generate_chord`**, **`transpose_notes`**, **`quantize_notes_to_scale`**, **`quantize_notes_to_grid`** —
    Symbolic helpers that turn a 20-tool-call sequence into a 1-tool-call sequence. Not unlocking any new
    capability, but a large reduction in token-cost-per-musical-idea.
11. **`analyze_groove`** — Useful once the AI is past "write the right notes" and into "make it feel good".
    Less load-bearing than harmony analysis because timing problems are easier for humans to flag than harmonic ones.
12. **`analyze_velocity_response`** — Patch-QA, narrower than `analyze_instrument_range`.

### Tier 3 — Specialized, build only when needed

13. **`analyze_arrangement`** — Useful for long-form composition; less critical for the typical 1-4 minute project.
14. **`compare_to_reference`** — Powerful for "make it sound like" prompts but requires the user to bring a
    reference. Build after `render_section_to_wav` is in place.
15. **`compare_patterns`**, **`compare_patches`**, **`humanize_notes`**, **`generate_variation`**,
    **`analyze_track`**, **`get_mix_meters`** — Each solves a narrower problem. Pick up as concrete user demand
    appears.

### What deliberately is **not** here

- Real-time spectrum streaming over MCP — high bandwidth, low value for an offline-reasoning agent.
  `analyze_mix_bus` over a rendered window covers the same questions.
- Style-transfer / "make it sound like genre X" — out of scope for this layer; belongs in a higher-level agent
  that *uses* these tools.
- Stem export — covered by `analyze_track` + `render_section_to_wav` with a track solo.

---

## 4. Cross-cutting design notes

- **Render path.** Tier-0 and Tier-1 audio tools render offline from an engine snapshot, same as `analyze_note`.
  No real-time playback dependency, deterministic output for a given project state.
- **Range addressing.** Arrangement-range tools take `[start_tick, end_tick]` (already the sequencer's native
  unit). Pattern-only tools take `pattern_id`.
- **Return shapes.** All analysis tools return one typed struct per call, with `#[serde(skip_serializing_if =
  "Option::is_none")]` on optional fields, mirroring `AnalyzeNoteResult`. No streaming, no chunking.
- **Naming.** `analyze_*` for read-only analysis returning metrics. `generate_*` / `quantize_*` / `transpose_*` /
  `humanize_*` for symbolic note manipulation. `render_*` for tools that produce audio output as a file/buffer.
- **Newtypes.** All durations as `Milliseconds` / `Seconds`, all frequencies as `Hertz`, all gains as `Decibels` /
  `Gain`, all pitch deltas as `Cents` / `Semitones`, ticks as `Tick`. No raw `f32` for domain values in public
  signatures.
- **Performance budget.** Each Tier-0 call should complete in well under a second for a 4-bar section at 44.1 kHz.
  `analyze_section` over a full 4-minute song is allowed to take a few seconds.

---

## 5. Implementation order — actual vs. planned

**Tier 0 (shipped):**

1. ✅ `analyze_harmony` — landed first as the symbolic / no-audio path. Smallest delta to ship.
2. ✅ Offline arrangement renderer (`crates/pertylizer/src/audio/arrangement_render.rs`) — the new infrastructure
   that both audio tools sit on. Drives `SequencerEngine` + `SynthEngine` over a tick range, captures the master
   bus.
3. ✅ `analyze_mix_bus` + `analyze_section` — share the renderer and the mix-bus analyzer
   (`crates/pertylizer/src/audio/mix_analysis.rs`: LUFS-I, peak/RMS/crest, banded energy, stereo correlation,
   mid/side, mono-compat, clip count).

**Tier 1 (next):**

4. Per-track contribution breakdown for `analyze_section` (deferred from Tier 0).
5. Drum-track filtering on `analyze_harmony` (surfaced by live testing).
6. `analyze_pattern` — pure symbolic, fast win.
7. `analyze_instrument_range` — sweep on top of `analyze_note`.
8. `render_section_to_wav` — generalization of the renderer that writes out instead of analyzing.
9. True-peak + LUFS-S/M on `analyze_mix_bus`.

**Tier 2 / Tier 3** — picked up by demand.

---

## 6. Lessons from live testing

Validating Tier 0 against a real 60-bar synthpop arrangement surfaced a handful of issues. Fixes for #1–#4 shipped
in `41d8bf6`; #5 is a deliberate Tier-1 follow-up.

1. **Seek-overrun in the offline renderer** — `EngineCommand::Seek` issued before `EngineCommand::Play` was
   silently undone, because `SequencerEngine::play` resets `current_tick = 0` on the Stopped → Playing transition.
   All `analyze_section` calls effectively rendered from tick 0 regardless of the requested `start_tick`, making
   section comparisons useless. Fix: send Play *before* Seek so the reset happens first and Seek overrides it.
2. **`analyze_harmony` token bloat** — Full-arrangement analysis at the default 1-quarter-note grouping produced
   77 KB of JSON and overran the MCP response-size limit. Fix: default to 1-bar grouping for arrangement scope
   (pattern scope keeps quarter-note), and merge consecutive `HarmonyChordEvent`s with the same chord symbol +
   `in_key` flag into single spans. `stats.chord_event_count` still reports the raw per-window count.
3. **Tick-based output is hard to read** — `start_tick: 76800` is meaningless without mental conversion. Fix:
   every chord event and every audio-section result now also carries `start_bar` / `start_beat` (1-indexed) via
   `Tick::to_bar_beat_tick` and `Song::time_signature_at`.
4. **Single-note windows mislabeled as `X(oct)` / `interval_octave`** — the catch-all 1-pitch-class template
   matched any monophonic note as if it were a chord, and also caught 2-note non-chordal dyads (e.g. G+A reported
   as `G(oct)`). Fix: dropped the `interval_octave` template; `identify_chord` returns `None` for inputs with
   fewer than 2 distinct pitch classes. `in_key` no longer short-circuits on `chord = None`, so a single note
   still gets checked against the inferred key.
5. **Drum tracks pollute harmonic analysis** — bar 1 of the test song reports `F#m7b5` because the hi-hat plays
   MIDI 42 (F#2) on top of an Am pad. The pitch set is correct; the musical interpretation isn't. Tier-1 work:
   add a way to exclude percussion tracks from chord identification (see §3 Tier 1, item 5).
6. **The test song clips at ~22 % of samples in the chorus** and reads **−5 LUFS-I** — 9 dB hotter than streaming
   norms (−14 LUFS for Spotify). Not a tool bug, but evidence that the tools are pointing at real and useful
   problems.
