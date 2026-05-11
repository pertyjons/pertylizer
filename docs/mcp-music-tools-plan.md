# MCP Music Tools — Plan

> **Date:** 2026-05-11
> **Status:** Proposal
> **Scope:** New MCP tools that give an AI agent the ability to evaluate and shape music as a whole, not only individual sounds.

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

### Tier 0 — Biggest impact, build first

1. **`analyze_mix_bus`** — Without this, every mix decision the AI makes is a guess. The single tool that converts
   Pertylizer from "AI can build sounds" into "AI can hear its own music". LUFS + per-band energy + true peak +
   stereo correlation covers ~80 % of mix-bus debugging.
2. **`analyze_section`** — `analyze_mix_bus` over an arbitrary arrangement range, *with* per-track contribution
   breakdown. This is what lets the AI answer "why is the chorus muddy?" by pointing at the actual offending track.
3. **`analyze_harmony`** — Currently the AI generates chord progressions by writing MIDI notes and has no feedback
   on whether the result is in a key, harmonically static, or accidentally dissonant. Chord-symbol + key inference
   converts notes back into the language the AI plans in.

### Tier 1 — High impact, build next

4. **`analyze_pattern`** — Cheap, no audio rendering. Density, range, velocity variance, repetition factor. Lets
   the AI verify "is this pattern interesting?" without rendering, and is a prerequisite for sane
   `generate_variation` heuristics.
5. **`analyze_instrument_range`** — Catches the bug class where a patch sounds great at C4 in `analyze_note` and
   falls apart at C6 or C2. Especially important once an AI is committing patches into a project without a human
   playing test notes.
6. **`render_section_to_wav`** — Even without immediate analysis tools layered on top, this unlocks the AI sending
   audio back to a human or feeding it to a separate model. Building block under `compare_to_reference`.

### Tier 2 — Quality-of-life and composition

7. **`generate_chord`**, **`transpose_notes`**, **`quantize_notes_to_scale`**, **`quantize_notes_to_grid`** —
   Symbolic helpers that turn a 20-tool-call sequence into a 1-tool-call sequence. Not unlocking any new
   capability, but a large reduction in token-cost-per-musical-idea.
8. **`analyze_groove`** — Useful once the AI is past "write the right notes" and into "make it feel good".
   Less load-bearing than harmony analysis because timing problems are easier for humans to flag than harmonic ones.
9. **`analyze_velocity_response`** — Patch-QA, narrower than `analyze_instrument_range`.

### Tier 3 — Specialized, build only when needed

10. **`analyze_arrangement`** — Useful for long-form composition; less critical for the typical 1-4 minute project.
11. **`compare_to_reference`** — Powerful for "make it sound like" prompts but requires the user to bring a
    reference. Build after `render_section_to_wav` is in place.
12. **`compare_patterns`**, **`compare_patches`**, **`humanize_notes`**, **`generate_variation`**,
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

## 5. Suggested implementation order (concrete)

1. `analyze_mix_bus` — reuses `analyze_note`'s analysis primitives (FFT, banding, peak/RMS) over a longer buffer.
   Smallest delta to ship.
2. `analyze_section` — adds offline arrangement rendering and per-track soloed sub-renders.
3. `analyze_harmony` + `analyze_pattern` — pure symbolic analysis on sequencer data, no audio path. Can ship in
   parallel with §2 since they touch different modules.
4. `analyze_instrument_range` — wraps `analyze_note` in a sweep loop with a compact per-note summary.
5. `render_section_to_wav` — generalization of §2 that writes the buffer out instead of analyzing it.
6. Symbolic helpers (Tier 2 §7) — landed together as one PR; they share validation and scale/key infrastructure.
7. Remaining Tier 2 / Tier 3 tools picked up by demand.
