# MCP Music Tools — Plan

> **Date:** 2026-05-11 (updated 2026-05-11 after Tier-1 partial ship, post-ship live-test pass, and two rounds of §8.1 determinism fix)
> **Status:** **Tier 0 shipped in v0.276.0; Tier-1 items 4 and 5 shipped in v0.277.0; offline-render determinism fix landed in two rounds post-v0.277.0; §8.2 auto-inferred instrument profiles shipped in v0.278.0**; remaining Tier-1+ pending.
> Post-ship live testing surfaced determinism + auto-categorization issues — see §8. §8.1 and §8.2 fully fixed end-to-end through the MCP bridge; §8.3 still open.
> **Scope:** New MCP tools that give an AI agent the ability to evaluate and shape music as a whole, not only individual sounds.

---

## 0. Status snapshot

### Shipped — v0.276.0 (Tier 0), v0.277.0 (Tier-1 first wave), and v0.278.0 (auto-inferred instrument profiles)

| Tool | Version | Notes |
|------|---------|-------|
| `analyze_harmony` | v0.276.0 + v0.277.0 + v0.278.0 | Walks notes by tick, identifies chord symbols (18 templates incl. m7/maj7/dom7/sus/dim/aug/power), infers key via Krumhansl-Schmuckler over 24 major/minor keys, returns in-key ratio, out-of-scale pitch classes, composite stability score. Pattern or arrangement scope. Returns chord events with `start_bar`/`start_beat` (1-indexed). Default grouping: 1 quarter-note for pattern scope, 1 bar for arrangement scope. Consecutive identical chord events are merged. **v0.277.0 added drum-track filtering** (manual `InstrumentCategory::Drums` + explicit `exclude_track_ids`). **v0.278.0 wired `analysis::instrument_profile` auto-inference** into the same filter — uncategorized percussion (Noise + percussive envelope + atonal pitch use) is now caught with confidence ≥ 0.6 and the warning text carries the signal trail (`[drums conf=X.XX; …]`). Manual category keeps priority via the `manual-override` signal. Arrangement scope only. |
| `analyze_mix_bus` | v0.276.0 | Renders N seconds (default 10, max 300) of the master bus offline from `start_tick` (default 0). Returns sample peak, RMS, crest factor, integrated LUFS (ITU-R BS.1770-4 K-weighted + 400 ms gating, 75 % overlap, abs −70 / rel −10 gates), 4-band frequency-balance RMS (sub/low/mid/high) windowed across the full buffer, stereo correlation, mid/side RMS, stereo width, mono-compatibility score, clipped-sample count. Includes `start_bar`/`end_bar`/`start_beat`/`end_beat`. |
| `analyze_section` | v0.276.0 + v0.277.0 | Same metrics as `analyze_mix_bus` but takes explicit `[start_tick, end_tick)`. **v0.277.0 added per-track contribution breakdown**: `include_per_track` (default false) opts in to one extra soloed render per audible track that overlaps the section, returning `TrackContribution { track_id, track_name, instrument_id, peak, peak_dbfs, rms, rms_dbfs, lufs_integrated, energy_bands, clipped_samples, rms_share }`. Soloing is implemented by cloning the song, setting only the target track's `solo = true`, and rendering via `render_arrangement_to_buffer_with_song`. |

### Deferred / in-progress

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

4. ✅ **Per-track contribution breakdown for `analyze_section`** — shipped in v0.277.0. Implements approach (a):
   N+1 renders (master + each audible track soloed). Returns one `TrackContribution` per audible track in
   `AnalyzeSectionResult.per_track`. Renderer factored into `render_arrangement_to_buffer_with_song` so the per-track
   variants run against song clones with overridden solo flags, not the live shared instance.
5. ✅ **Drum-track filtering on `analyze_harmony`** — shipped in v0.277.0. Two new parameters:
   `exclude_drums` (default true) honours `InstrumentCategory::Drums`; `exclude_track_ids` is an explicit drop list.
   Both apply to arrangement scope only; pattern scope warns when filters are passed.
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

**Tier 1:**

4. ✅ Per-track contribution breakdown for `analyze_section` — v0.277.0.
5. ✅ Drum-track filtering on `analyze_harmony` — v0.277.0.
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
   MIDI 42 (F#2) on top of an Am pad. The pitch set is correct; the musical interpretation isn't. ✅ Fixed in
   v0.277.0 for *manually-tagged* drum tracks: `analyze_harmony` defaults to `exclude_drums = true`, dropping
   notes from `InstrumentCategory::Drums` tracks. ✅ Fully fixed in v0.278.0 via `analysis::instrument_profile`
   auto-inference: a drum patch no longer needs to be tagged via `set_instrument_category` for the filter to
   fire. Name vocabulary, voice-graph contents, envelope shape, and pitch-role statistics together resolve
   uncategorized percussion to `Role::Drums` with confidence ≥ 0.6, and the warning text now names *why* a
   track was auto-excluded. `exclude_track_ids` remains as a manual escape hatch.
6. **The test song clips at ~22 % of samples in the chorus** and reads **−5 LUFS-I** — 9 dB hotter than streaming
   norms (−14 LUFS for Spotify). Not a tool bug, but evidence that the tools are pointing at real and useful
   problems.

---

## 7. Deferred technical follow-ups from the v0.277.0 ship

Surfaced by the post-ship simplify pass. None are blockers; ordered by expected impact. Pick up alongside the next
Tier-1 item that touches the same code.

### 7.1 Reuse one `SynthEngine` across the N per-track renders — high impact

`render_per_track_contributions` (`crates/pertylizer/src/mcp_bridge.rs`) currently calls
`render_arrangement_to_buffer_with_song` once per audible track. Each call spins up a brand-new offline
`SynthEngine` and replays every instrument's full module graph, connections, parameters, effect chain, bypass
states, and volume/pan into it via `send_blocking`. For an N-track section that work is repeated N times even
though only the per-track `solo` flags differ between iterations.

Plan: factor the engine construction + instrument-load out of `render_arrangement_to_buffer_with_song` into a
`OfflineEngineSession` that owns the engine and exposes `render_range(song_arc, start, end) -> RenderedArrangement`.
The per-track loop builds it once and calls `render_range` N times. Between iterations, send a `Stop` (to flush
note/voice state) and a fresh `SetSong` / `Seek` / `Play`. The sequencer will pick up the updated solo flags on the
next tick.

This is the single biggest win for `analyze_section` with `include_per_track = true` on real songs.

### 7.2 Parallelize the per-track renders with rayon — medium impact

The N renders in `render_per_track_contributions` are independent (each its own engine + soloed song clone, no
shared mutable state). Today they run serially in a `for` loop. After §7.1 lands, swap to `par_iter` over
`targets`, scaled to `min(N, num_cpus)`. Watch peak memory — each worker holds one `OfflineEngineSession` plus a
`Song` snapshot — but for 8–16 tracks on modern hardware this trades RAM for near-linear wall-clock improvement.

Cleanest order: do §7.1 first so each rayon worker reuses its own engine within its slice.

### 7.3 `HarmonyScope` enum to fix `analyze_song_harmony` argument sprawl — medium impact

`analyze_song_harmony` now takes 8 arguments and carries `#[allow(clippy::too_many_arguments)]`. Two of those
arguments (`exclude_drums`, `exclude_track_ids`) are only meaningful in arrangement scope; pattern scope
currently emits a runtime warning if they're passed. Both the lint allow and the runtime warning are symptoms of
mixing two scope-shaped variants into one flat parameter list.

Plan: introduce

```rust
enum HarmonyScope {
    Pattern { pattern_id: PatternId },
    Arrangement {
        start: Option<u64>,
        end: Option<u64>,
        exclude_drums: bool,
        exclude_track_ids: HashSet<TrackId>,
    },
}
```

at the bridge boundary. The MCP server's `AnalyzeHarmonyParam` flat struct (which the JSON-schema layer requires)
maps into the enum inside the bridge. The runtime "ignored in pattern scope" warning becomes a compile-time
impossibility, and the `#[allow]` lint disappears. Touches `synth_mcp::bridge`, the bridge impl, and the
arrangement-vs-pattern branch in `analyze_song_harmony`.

### 7.4 `Song::tracks_mut` / `Song::set_solo_only` helpers in `synth_sequencer` — low impact

The current solo-flag handling clears all flags via a Vec-of-IDs loop, then per render flips one flag and flips it
back. Two upstream helpers would shrink the call site and become reusable elsewhere in the project:

- `pub fn tracks_mut(&mut self) -> impl Iterator<Item = &mut SequencerTrack>` — symmetric with the existing
  `tracks()` at `crates/synth_sequencer/src/song.rs:228`.
- `pub fn set_solo_only(&mut self, target: TrackId)` — atomic "make this the only soloed track".

Both are small additions to `Song`; the per-track contribution loop becomes one line per iteration. Bundle with
§7.1 since both touch the same loop.

### 7.5 `Arc<RwLock<Song>>` constructor helper — low impact, project-wide

Grep finds ~9 sites that wrap a `Song` in `Arc::new(parking_lot::RwLock::new(...))` verbatim
(`crates/pertylizer/src/audio/export.rs:214`, `main.rs:83`, `mcp_shared.rs:56`, sequencer tests, the per-track
render loop, etc.). A `synth_sequencer::shared_song(Song) -> Arc<RwLock<Song>>` constructor would deduplicate
the pattern. Strictly cosmetic; not on any hot path. Pick up as a drive-by when next touching one of those sites.

### 7.6 `TrackContribution` field drift vs. `MixBusMetrics` — keep an eye on it

`TrackContribution` duplicates 9 of `MixBusMetrics`'s numeric fields. When §0's deferred items land (`true_peak`,
LUFS-S/M, etc.), the new fields will need to be added in two places. Two options when that happens:

1. Embed a `MixBusMetrics` inside `TrackContribution` and add only the identity/share fields (`track_id`,
   `track_name`, `instrument_id`, `rms_share`) at the outer level. Cleanest, but changes the response shape — MCP
   clients keyed on the current flat layout would need to update.
2. Keep the flat layout and add a small `#[cfg(test)]` test that asserts both structs declare the same superset of
   audio-metric fields, so drift fails CI.

Decision point comes with the `true_peak` work in §0; flag here so it's remembered.

---

## 8. Issues surfaced by live testing on 2026-05-11 (post-v0.277.0)

Re-ran the full tool suite against the "Tung Synthpop" arrangement after the v0.277.0 ship. Three new findings,
ordered by impact.

### 8.1 Offline render is not deterministic — HIGH ✅ **Fixed end-to-end through the MCP bridge**

Took two rounds. Round 1 (the `fastrand`-reseed fix) closed the bulk of the gap but missed a deeper architectural
source of nondeterminism; Round 2 caught the rest. The full story is preserved here because each round revealed
the next, and the residual symptoms after Round 1 are exactly what someone hunting a similar bug would see.

#### Round 1: `fastrand` thread-local RNG state

Four consecutive `analyze_section` calls on the same `[start_tick, end_tick)` against the same song-state had
produced:

| Call | RMS dBFS | LUFS-I | clipped samples |
|------|----------|--------|-----------------|
| 1 (master only) | -3.377 | -4.447 | 184 348 |
| 2 (master inside per-track call) | -3.669 | -4.889 | 167 015 |
| 3 (master only) | -3.492 | -4.816 | 181 642 |
| 4 (master only) | -3.404 | -4.520 | 183 770 |

LUFS-I spread of **0.44 dB**, clipped-samples spread of **10 %**, and RMS spread of **~0.3 dB** were large enough
to mask real 0.5 dB mix changes — exactly the decisions the tools are sold on.

**Root cause.** Several DSP modules pull from `fastrand::f32()` during processing:

- `Oscillator::note_on` randomizes phase by `fastrand::f32() * unison_phase_random` (default 1.0).
- `Noise`, `MathOscillator` noise mode, `MechanicalNoise`, `LFO` S&H, `DriftGenerator` pull every sample.

`fastrand`'s free functions use a thread-local RNG seeded once per thread at first use. Two consecutive offline
renders on the same MCP-bridge thread therefore started from whatever RNG state the previous render had left
behind, so phase + noise contributions drifted between calls.

**Fix.** Reseed `fastrand`'s thread-local RNG with a fixed constant at the start of every offline render entry
point — `render_arrangement_to_buffer_with_song` (covers `analyze_mix_bus`, `analyze_section`, and the per-track
contribution loop) and `render_note_to_buffer` (covers `analyze_note`). The shared seed lives as
`pub(crate) const OFFLINE_RENDER_SEED` in `arrangement_render.rs`. The live audio thread is unaffected: it has its
own thread-local RNG.

**Secondary bug surfaced.** With renders made (mostly) deterministic,
`voice_module_bypass_replicated_in_offline_render` revealed that `ModuleGraph::clone_structure()` did not copy the
`bypassed: HashSet`. Voices rebuilt from a template after a module was bypassed silently un-bypassed that module.
The previous test only passed because random phase noise between renders exceeded the 1e-3 threshold the
assertion compared against. Fixed by cloning the `bypassed` set inside `clone_structure`. This also tightens the
live engine, where re-allocations after a bypass had carried the same latent bug.

**Coverage.** `crates/pertylizer/tests/arrangement_render_determinism.rs` asserts bit-exact equality of repeated
renders for a sustained sawtooth patch and a noise-source patch (which hammers `fastrand` every sample), plus
downstream `analyze_mix_buffer` LUFS-I / RMS / clipped-samples readouts.

#### Round 2: HashMap iteration order across fresh `SynthEngine` instances

Re-running `analyze_section` through the MCP bridge after Round 1 still produced non-bit-exact results, just at
a much smaller magnitude — ~**0.10 dB LUFS-I spread, ~500 sample clipped-count spread** across three
back-to-back calls on the same range. The Round 1 test suite passed because it reused the same `SynthSession`
across renders; the bridge does not.

**Why the test path passed but the bridge did not.** Each `analyze_section` MCP call lands in
`render_arrangement_to_buffer_with_song`, which builds a **fresh** `SynthEngine` per call and loads every
instrument's voice graph into it. The render is then run on that throwaway engine. Two such engines — built
moments apart on the same thread, from the same `Patch` snapshots — are not equivalent. The reason: every
`ModuleGraph::new()` constructs a `HashMap<ModuleId, GraphNode>` whose hasher is seeded by `RandomState`, and
`RandomState`'s seed is itself drawn from a per-process RNG that advances on each construction. Iteration over
`self.nodes.values_mut()` (used by `note_on`, `note_off`, `reset`, `gather_mod_source_values`,
`clone_structure`, and the topo-sort seed set) therefore visits modules in a different order on each fresh
graph.

That mattered specifically because `Oscillator::note_on` calls `fastrand::f32()` for unison phase
randomization. With two oscillators in a voice, "osc-1 then osc-2" vs. "osc-2 then osc-1" consumes the same two
RNG values but assigns them to different oscillators — so the two voices produce different summed waveforms.
Round 1's reseed gave each render the same starting RNG state, but consumption order varied per fresh graph,
so phases drifted.

The same story affected:

- **Topological sort.** `topo_in_degree: HashMap<ModuleId, usize>` was iterated to find the Kahn-seed set of
  zero-in-degree modules. Different iteration order → different `processing_order` for any graph where two or
  more modules tied on in-degree (typical: every patch has multiple in-degree-0 sources — Oscillator, Envelope,
  LFO, …).
- **Per-port input summation.** `incoming_map` is built from `&self.connections` (a `HashSet`) into a `Vec<…>`
  per destination module. The Vec inherits the HashSet's iteration order. When two outputs feed the same input
  port (e.g. two oscillators → `amp-1.in`), `process_module` sums them in Vec order, and floating-point
  addition is non-associative — different orders yielded last-ULP-different sums each frame, drifting
  downstream filter state.

**Fix.** Replace `nodes`, `topo_in_degree`, and `topo_adjacency` in `crates/synth_engine/src/graph.rs` with
`BTreeMap` keyed by `ModuleId`. Sort each adjacency Vec and each `incoming_map` Vec at the end of
`calculate_processing_order` so any remaining `HashSet<Connection>` iteration order is normalized before it
reaches DSP. Required deriving `PartialOrd, Ord` on `ModuleId`, `ModuleType`, and `PortName` (all small `Copy`
types — comparison is free). Hot-path impact assessed and dismissed: `nodes.get(&id)` becomes O(log n) for
n ≤ ~16 modules per voice — roughly four cache-friendly comparisons vs. one hash, indistinguishable in
practice. `calculate_processing_order` runs on topology change only, never per frame, so the new sorts are free.

The `Ord` derive on `ModuleType` makes its source variant order load-bearing for determinism: reordering or
inserting variants now changes `ModuleId` ordering, which would silently change audio output for existing
patches that use the affected types. Doc-commented at the enum declaration to make the constraint visible.

**Live verification.** Four consecutive `analyze_section` calls on `[76800, 92160)` after the Round 2 fix:

| Call | RMS | LUFS-I | clipped samples |
|------|-----|--------|-----------------|
| 1 (master only) | 0.6583361 | -4.899891 | 170 663 |
| 2 (master only) | 0.6583361 | -4.899891 | 170 663 |
| 3 (master only) | 0.6583361 | -4.899891 | 170 663 |
| 4 (master inside per-track call, N+1 renders) | 0.6583361 | -4.899891 | 170 663 |

**Bit-exact.** Including the N+1-render variant where each per-track contribution kicks off another fresh
`SynthEngine` — the `BTreeMap` ordering survives the loop, and the master render hasn't shifted by a single
sample.

**Coverage.** Added `offline_render_is_bit_exact_for_dual_oscillator_patch` to the determinism test suite. Two
oscillators with phase randomization is the smallest patch that exposes `note_on` iteration-order dependence —
the existing single-Oscillator `sustain_patch` could not have caught the Round 2 regression.

**Consequence for v0.277.0 per-track work.** `rms_share` is exact across repeated `analyze_section` calls with
`include_per_track = true`. Master ≈ per-track sum is bit-exact-reproducible (modulo the inherent solo-flag
accounting; no longer modulo RNG drift or HashMap iteration drift).

### 8.2 Drop the manual `InstrumentCategory` requirement — auto-infer it instead — ✅ shipped in v0.278.0

The v0.277.0 default `exclude_drums: true` on `analyze_harmony` is a silent no-op when no instruments have
`category == Drums` — which is the realistic case, since `set_instrument_category` is barely used. On the test
song every instrument is `Uncategorized`, so the default call still produces the `F#m7b5` bug it was supposed
to fix. The user has no warning indicating the filter was inert.

**Shipped form (v0.278.0, 8.2b path).** New module `pertylizer::analysis::instrument_profile` (~600 lines incl.
tests; planned in `docs/instrument-profile-plan.md`). `InstrumentProfile` exposes five independent axes —
`role`, `envelope_shape`, `pitch_role`, `register`, `texture` — plus a `RoleInference` carrying the confidence
score and the signal trail that produced it. `analyze_song_harmony` now calls `infer_all_profiles` and drops
tracks classified as `Drums` with confidence ≥ 0.6; the warning text spells out *why*, e.g.
`"Excluded 1 track(s) from harmony analysis: Track 5(1) [drums conf=1.00; decision:drums-gate, envelope:percussive, graph:noise-no-osc]"`.
Manual `set_instrument_category` keeps priority (`manual-override` signal, confidence 1.0). No new MCP surface
— the module is internal until the first external consumer arrives.

The original analysis below is preserved for context.

Two paths, ordered by ambition:

**8.2a Minimum fix:** when `exclude_drums = true` and no `Drums`-categorized tracks exist, push a warning:
`"exclude_drums=true had no effect — no instruments are categorized as Drums. Use set_instrument_category or
pass exclude_track_ids."` Cheap, transparent, but still requires manual action to actually fix the analysis.

**8.2b Better fix: `infer_instrument_category(instrument_id) -> Category` with a layered fallback.** Live testing
verified the signals are strong enough to do this without any audio rendering:

| Layer | Signal | Cost | Example from test song |
|-------|--------|------|------------------------|
| 1. Name match | "kick", "snare", "hat", "drum", "perc", "cymbal", "tom", "clap" on instrument *or* track name | free | "Kick", "Snare", "Hat" → instant Drums |
| 2. Module graph | `Noise` source + no `Oscillator` → percussion noise | free | Hi-Hat: 5-module graph `Noise → Env → Flt → Amp → Out`, no oscillator |
| 3. Envelope shape | sustain ≈ 0 AND decay + release < 200 ms | free | Hat ADSR: A=1 ms, D=50 ms, S=0 %, R=30 ms |
| 4. Pattern form | `distinct_pitches ≤ 2 AND max_duration < 1 beat` across all placements | free | Hat pattern: 32 notes all MIDI 42, duration 0.25 beats |
| 5. analyze_note signal | low `pitch_confidence` + high centroid + sustain ≈ 0 | one render | Hat: confidence 0.20, centroid 13.6 kHz, RMS dies in 150 ms |

Layers 1–4 are free (no rendering). Layer 5 exists for borderline cases but isn't needed in practice. The test
song's three drum tracks (Kick, Snare, Hat) get picked up by layer 1 alone — name match.

Concrete shape:

```rust
pub fn infer_instrument_category(
    instrument: &InstrumentSnapshot,
    track: Option<&SequencerTrack>,
    patterns_used: &[&Pattern],
) -> (InstrumentCategory, f32 /* confidence */);
```

`analyze_harmony`'s default behaviour changes from "filter tracks where `category == Drums`" to "filter tracks
where `infer_instrument_category` returns `Drums` with confidence ≥ threshold". The warning then names the
auto-detected drum tracks transparently, e.g. `"Auto-excluded 3 track(s) inferred as drums: Kick(0) [name], Snare(1) [name], Hat(2) [name]"`.

This makes the manual `set_instrument_category` workflow optional — useful for overrides, not mandatory for the
default analysis to work. It also unblocks other future tools (`analyze_groove` could auto-pick drum tracks;
`analyze_pattern` could classify pattern shape).

Recommendation: ship 8.2b. It's a small standalone module (~150 lines), fully testable, and removes a sharp
edge in the harmony tool's UX.

### 8.3 Per-track contribution: clarify pan-law output, optionally add `pre_master_peak` — LOW/MED

Per-track renders on center-panned tracks report `peak = 0.7071068` (= 1/√2), which is the constant-power pan-law
attenuation kicking in for L = R = 1.0 inputs. That's correct but confusing: a user reading "Kick peak -3 dBFS"
might think the kick is hot to -3, when in fact the kick's internal signal is at 0 dBFS and the -3 dB is the
pan-law cost of being center.

Two parts:

1. **Documentation:** add a sentence to the `analyze_section` tool description and `TrackContribution` doc:
   `"Track peak/RMS reflect the track's contribution to the master mix, including pan-law attenuation
   (-3 dB at center pan). For internal patch peak, run analyze_note against the instrument directly."`
2. **Optional new field `pre_master_peak: f32`:** what the track's peak would be if it were the only audible
   track *and* not pan-attenuated — i.e. the actual internal-signal peak the patch is producing. Lets a user
   see "this kick is clipping internally before any pan/sum even happens". Cheap to compute (we already have
   the soloed render; just don't apply the pan/track-volume in the analysis step, or run it on an
   unpanned-copy).

Live testing showed Kick + Sub Bass + Aggro Bass + Stab all reported exactly 0.7071 — the user can't tell
whether any of them is hot internally without correlating with `analyze_note`. The doc fix is mandatory;
`pre_master_peak` is a quality-of-life add.

### 8.4 Cross-reference

§8.1 is now fully fixed (Round 1 + Round 2); §4's "deterministic output for a given project state" claim holds
bit-exact end-to-end through the MCP bridge, not just inside the determinism unit tests. §8.2 shipped in
v0.278.0: the v0.277.0 `analyze_harmony` doc's implicit "manual `set_instrument_category` required" is now
optional — the inference path resolves uncategorized percussion on its own. §8.3 is a doc-only update plus an
optional additive field.
