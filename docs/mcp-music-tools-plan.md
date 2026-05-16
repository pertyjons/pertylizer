# MCP Music Tools — Plan

> **Date:** 2026-05-11 (last update 2026-05-16: §0 sweep — `true_peak`, `lufs_momentary_max`, `lufs_short_term_max`, `pre_master_peak` shipped; `TrackContribution` restructured to embed `MixBusMetrics` (§7.6 decision); §8.3 `pre_master_peak` closed. Earlier 2026-05-16 work: higher-level music-understanding catalogue + optional ML sidecar; §8.3 pan-law documentation, §8.5.1–§8.5.5 inference round-3 fixes — Pad-precedence-by-name, Bass-gate Atonal-by-name, Lead-precedence allows Polyphonic, drum-gate wider pitch-spread when name says Drums, extended name vocabulary; §7.1 `OfflineEngineSession` engine-reuse; §8.5.6.1/§8.5.6.2 round-4 inference follow-ups.)
> **Status:** **Tier 0 shipped in v0.276.0; Tier-1 items 4 and 5 shipped in v0.277.0; offline-render determinism fix in two rounds post-v0.277.0; §8.2 auto-inferred instrument profiles shipped in v0.278.0; commits 74d18da + 93c0786 closed the offline-render snapshot bug class, added `get_instrument_profiles`, and applied seven inference improvements that roughly doubled live-test accuracy on synth patches; inference round-3 (§8.5.1–§8.5.5) + §8.3 pan-law doc + §7.1 engine reuse shipped 2026-05-16; §0 deferred trio (`true_peak`, LUFS-S/M, `pre_master_peak`) + §7.6 embed-MixBusMetrics decision shipped 2026-05-16.** Remaining Tier-1+ pending.
> Post-ship live testing surfaced determinism, auto-categorization, and offline-render-state issues — see §8. §8.1, §8.2, §8.3 (doc + `pre_master_peak`), §8.4, and §8.5 are fully shipped end-to-end through the MCP bridge.
> **Scope:** New MCP tools that give an AI agent the ability to evaluate and shape music as a whole, not only individual sounds.

---

## 0. Status snapshot

### Shipped — v0.276.0 (Tier 0), v0.277.0 (Tier-1 first wave), v0.278.0 (auto-inferred profiles), and 74d18da + 93c0786 (offline-render fixes, `get_instrument_profiles`, inference round 2)

| Tool | Version | Notes |
|------|---------|-------|
| `analyze_harmony` | v0.276.0 + v0.277.0 + v0.278.0 + 74d18da | Walks notes by tick, identifies chord symbols (18 templates incl. m7/maj7/dom7/sus/dim/aug/power), infers key via Krumhansl-Schmuckler over 24 major/minor keys, returns in-key ratio, out-of-scale pitch classes, composite stability score. Pattern or arrangement scope. Returns chord events with `start_bar`/`start_beat` (1-indexed). Default grouping: 1 quarter-note for pattern scope, 1 bar for arrangement scope. Consecutive identical chord events are merged. **v0.277.0 added drum-track filtering** (manual `InstrumentCategory::Drums` + explicit `exclude_track_ids`). **v0.278.0 wired `analysis::instrument_profile` auto-inference** into the same filter — uncategorized percussion is now caught with confidence ≥ 0.6 and the warning text carries the signal trail. **74d18da** added the `has_oneshot_sampler` graph signal so sample-based percussion (Sampler + Output, no envelope) also fires drum-gate via the `oneshot-sampler` evidence. Manual category keeps priority via the `manual-override` signal. Arrangement scope only. |
| `analyze_mix_bus` | v0.276.0 + 74d18da | Renders N seconds (default 10, max 300) of the master bus offline from `start_tick` (default 0). Returns sample peak, RMS, crest factor, integrated LUFS (ITU-R BS.1770-4 K-weighted + 400 ms gating, 75 % overlap, abs −70 / rel −10 gates), 4-band frequency-balance RMS (sub/low/mid/high) windowed across the full buffer, stereo correlation, mid/side RMS, stereo width, mono-compatibility score, clipped-sample count. Includes `start_bar`/`end_bar`/`start_beat`/`end_beat`. **74d18da** threaded `SharedSampleLibrary` through the offline renderer so sampler-based instruments stop rendering silence — see §8.4. |
| `analyze_section` | v0.276.0 + v0.277.0 + 74d18da | Same metrics as `analyze_mix_bus` but takes explicit `[start_tick, end_tick)`. **v0.277.0 added per-track contribution breakdown**: `include_per_track` (default false) opts in to one extra soloed render per audible track that overlaps the section, returning `TrackContribution { track_id, track_name, instrument_id, peak, peak_dbfs, rms, rms_dbfs, lufs_integrated, energy_bands, clipped_samples, rms_share }`. Soloing is implemented by cloning the song, setting only the target track's `solo = true`, and rendering via `render_arrangement_to_buffer_with_song`. **74d18da** propagated samples per soloed render so per-track contributions on sample-based instruments report real metrics. |
| `get_instrument_profiles` | 93c0786 | Returns `Vec<InstrumentProfileResult>`, one entry per instrument routed to by at least one track. Each profile carries the inferred `role` plus a `confidence: 0.0..=1.0` and a `signals` trail listing every axis that contributed (`decision`, `name`, `envelope`, `graph`, `pattern`, `manual`). Same inference path that `analyze_harmony`'s `exclude_drums = true` uses — exposed directly so users can debug or override the classification. Manual `set_instrument_category` short-circuits the decision tree and reports as `manual-override` with confidence 1.0. Wire format uses snake_case strings for role/envelope_shape/etc. so the MCP type stays decoupled from pertylizer-internal enums. |
| `analyze_pattern` | 2026-05-16 | Pure symbolic single-pattern analyzer; no audio render. Reads a pattern's notes directly and reports `density` (notes per bar/beat, active ratio), `pitch` (low/high/range, mean, distinct count, duration-weighted pitch-class histogram), `velocity` (min/max/mean/std/range), `rhythm` (max/mean polyphony, monophonic flag, distinct onsets+durations, IOI mean+std, regularity score), and `repetition` (distinct bar signatures quantized to a 32nd-note grid, repetition score). `length_bars` and `notes_per_bar` use the song's default time signature. Notes that start past the pattern length are dropped with a warning. Implementation in `crates/pertylizer/src/analysis/pattern_analysis.rs`; bridge in `analyze_pattern_impl`. Closes Tier-1 #6. |

### Deferred / in-progress

All §0 deferred items shipped 2026-05-16 alongside the §7.6 layout decision:

- **`true_peak` (inter-sample peak)** — ✅ shipped. 4× polyphase oversampling FIR (48-tap Hamming-windowed sinc, ITU-R BS.1770-4 Annex 2-compliant) lives in `mix_analysis::compute_true_peak_stereo`. `MixBusMetrics` now carries `true_peak` (linear) and `true_peak_dbtp` (dBTP). Regression test signal is an 11 025 Hz sine sampled at quadrature phases — sample peak ≈ 0.7071, true peak ≥ 0.99.
- **LUFS-S / LUFS-M (momentary / short-term)** — ✅ shipped. `compute_loudness` returns integrated + momentary-max + short-term-max in one pass over the existing 400 ms / 100 ms-hop K-weighted block decomposition. Short-term uses a 30-block sliding window (3 s, ITU-R-compliant); buffers < 3 s report -200.0 for that field. `MixBusMetrics` gained `lufs_momentary_max` and `lufs_short_term_max`.
- **Optional `pre_master_peak` on `TrackContribution`** — ✅ shipped. `analyze_mix_buffer` now also records per-channel sample peaks (`peak_left` / `peak_right`); the bridge reads each instrument's `(volume, pan)` from the `SynthSession` snapshot and analytically reverses the engine's constant-power `Gain::from_pan(pan) × volume` attenuation on the loud channel. No extra render pass — the single soloed render that produced `metrics` is enough. `pre_master_peak` is the patch's internal signal peak before any pan-law or volume scaling. Unit tests cover center / hard-pan / volume-drop / unknown-instrument / silence cases.
- **§7.6 layout decision** — went with embedding `MixBusMetrics` inside `TrackContribution` (the `metrics: MixBusMetrics` field), per CLAUDE.md's "active development — no backward compatibility required" stance. Eliminates the 9-field duplication permanently; adding any future metric only touches `MixBusMetrics`. Tests reading `tc.rms` → `tc.metrics.rms` were updated.

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

### 2.6 Higher-level music understanding (core / no heavy external dependency)

These tools should be implementable inside Pertylizer from existing song state, symbolic MIDI events, offline
renders, and the existing instrument-profile/mix-analysis infrastructure. Some may benefit from small DSP crates
(e.g. FFT helpers), but they should not require Python, ML models, or heavyweight third-party runtimes.

| Tool | Purpose |
|------|---------|
| `analyze_harmonic_function` | Build on `analyze_harmony` but return key-relative Roman numerals, functional roles (`tonic`, `predominant`, `dominant`, `modal_mixture`, `secondary_dominant`, `chromatic_passing`, etc.), cadence candidates, phrase-level tension/release, and warnings for progressions that are stable but harmonically static. Gives the AI feedback in the same language it uses to plan chord movement. |
| `analyze_drum_groove` | Drum-specific symbolic groove analysis: kick/snare backbeat strength, hat subdivision, ghost-note density, fill detection, syncopation score, accent grid, repeated-bar sameness, and common pattern hints (four-on-the-floor, breakbeat, half-time, tresillo/clave-like cells). Uses auto-inferred drum profiles by default. |
| `analyze_bass_drum_lock` | Compare bass notes/onsets against kick, snare, and chord roots. Return kick overlap, bass anticipations, offbeat drive, root/fifth usage, low-end note conflicts, sub gaps, and likely sidechain/arrangement issues. Particularly useful for electronic music where the groove depends on bass + kick relationship. |
| `analyze_masking_matrix` | Pairwise track masking report over a section. For each relevant track pair, return overlapping energy by perceptual/fixed bands, dominance, likely conflict ranges, and concrete mix hints (`pad masks lead around 1-4 kHz`, `kick/bass overlap too wide below 90 Hz`). Starts with soloed per-track renders and existing band metrics; can later grow Bark/ERB bands if needed. |
| `analyze_tension_curve` | Bar/section-level curve combining harmony, dissonance, density, register, loudness, brightness, spectral entropy, stereo width, and rhythmic activity. Flags shape problems like a chorus that does not lift, a build that peaks too early, or a drop that loses low-end energy. |
| `analyze_form_map` | Deterministic song-form/section map from bar-level features, arrangement clips, density, instrument activity, harmony changes, and self-similarity. Returns section candidates, phrase lengths, contrast scores, repetition/novelty peaks, and warnings for intros, builds, drops, or choruses that do not differ enough. This is the core version; ML/audio-reference structure models belong in the future sidecar bucket. |
| `find_motifs` / `analyze_hook_strength` | Detect repeated pitch/rhythm contours, transformed motifs, call-and-response, hook recurrence, and variation-vs-copy-paste balance. Returns motif IDs with bar/beat occurrences, transformation labels, and a compact "hook appears enough / too hidden / too repetitive" assessment. |
| `suggest_music_fixes` | Meta-analysis tool that consumes outputs from harmony, groove, mix, arrangement, and patch analyzers and returns ranked next actions with supporting evidence. It should not invent new measurements; it packages existing diagnostics into concrete edits an AI agent can act on. |

### 2.7 Future ideas — optional third-party / ML sidecar

These are powerful, but they should stay out of the deterministic core until there is a clear sidecar story
for model installation, licensing, runtime cost, cache invalidation, and reproducibility.

| Tool | Purpose |
|------|---------|
| `analyze_style_embedding` | Render a section and return semantic/style embeddings plus optional tag predictions (genre, mood, instrumentation, era, production style). Candidate backends: Essentia TensorFlow models, MERT, MuQ/MuLan, CLAP, MusicFM-like representation models. Useful for "this feels closer to synthwave than techno" feedback, but model choice and licensing matter. |
| `compare_style_to_reference` | Compare Pertylizer output against a reference WAV/text target using embeddings and tag deltas instead of only LUFS/frequency balance. This is analysis, not style transfer: it tells the AI what differs semantically (`reference is brighter, more percussive, less pad-heavy`). |
| `decompose_reference_to_stems` | Split an external reference WAV into drums/bass/vocals/other stems so `compare_to_reference` can compare the user's drums to reference drums, bass to reference bass, etc. Candidate backends: Demucs / HT-Demucs, Open-Unmix. Heavy runtime and licensing/deployment concerns make this a future optional tool. |
| `transcribe_reference_to_midi` | Convert reference audio into approximate MIDI/chord/bass/melody material for symbolic comparison. Candidate backends: Basic Pitch for lightweight pitch transcription, MT3-like models for multi-instrument transcription. Best treated as an optional import/reference-analysis path, not a core analyzer. |
| `analyze_audio_meter_map` | Audio-only beat/downbeat/tactus/timing analysis for imported reference audio. Candidate backends: Beat This!, BeatNet, madmom/librosa-style pipelines. The core `analyze_groove`/`analyze_drum_groove` path should stay symbolic for Pertylizer-authored songs. |

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
6. ✅ **`analyze_pattern`** — shipped 2026-05-16. Pure symbolic — reads a single pattern's notes directly,
   reports density (notes per bar/beat, active ratio), pitch shape (range, mean, distinct count,
   duration-weighted pitch-class histogram), velocity dynamics (min/max/mean/std/range), rhythmic
   structure (max/mean polyphony, distinct onsets/durations, IOI mean+std, regularity score), and
   bar-level repetition (distinct bar signatures quantized to a 32nd-note grid, repetition score).
   Implementation lives in `crates/pertylizer/src/analysis/pattern_analysis.rs`; bridge in
   `analyze_pattern_impl`.
7. **`analyze_drum_groove`** — Drum-specific feel analysis is more actionable than a generic groove score:
   backbeat strength, hat subdivision, fills, ghost notes, and repeated-bar sameness are exactly the issues that
   make AI-written beats sound flat. Pure symbolic path, using the shipped instrument-profile inference.
8. **`analyze_bass_drum_lock`** — The kick/bass relationship carries a large share of perceived groove and
   low-end clarity in electronic music. This gives the AI concrete answers to "does the bass actually work with
   the beat?" without requiring external models.
9. **`analyze_masking_matrix`** — Extends the shipped per-track contribution work from "which track owns which
   band?" to "which track conflicts with which other track?". High mix utility once `analyze_section` can render
   per-track contributions deterministically.
10. **`analyze_harmonic_function`** — Chord labels and key fit are useful; functional roles, cadence candidates,
    and phrase-level tension are what let the AI reason about progression quality and direction.
11. **`analyze_instrument_range`** — Catches the bug class where a patch sounds great at C4 in `analyze_note` and
   falls apart at C6 or C2. Especially important once an AI is committing patches into a project without a human
   playing test notes.
12. **`render_section_to_wav`** — Even without immediate analysis tools layered on top, this unlocks the AI sending
   audio back to a human or feeding it to a separate model. Building block under `compare_to_reference`.
13. ✅ **True-peak + LUFS-S/M for `analyze_mix_bus`** — shipped 2026-05-16 alongside `pre_master_peak`. See §0 deferred section.

### Tier 2 — Quality-of-life and composition

14. **`analyze_tension_curve`** — Higher-level song-shape diagnostic built from existing analyzers. Useful for
    "the chorus doesn't lift" and "the build peaks too early" feedback; less foundational than the lower-level
    tools it consumes.
15. **`find_motifs` / `analyze_hook_strength`** — Helps verify whether a song has recognizable recurring ideas
    and whether variations are meaningfully related rather than random. Symbolic first, no audio rendering needed.
16. **`suggest_music_fixes`** — Meta-tool that ranks concrete next actions from existing diagnostics. High agent
    usefulness, but should come after enough analyzers exist for the suggestions to be grounded.
17. **`generate_chord`**, **`transpose_notes`**, **`quantize_notes_to_scale`**, **`quantize_notes_to_grid`** —
    Symbolic helpers that turn a 20-tool-call sequence into a 1-tool-call sequence. Not unlocking any new
    capability, but a large reduction in token-cost-per-musical-idea.
18. **`analyze_groove`** — Useful once the AI is past "write the right notes" and into "make it feel good".
    Less load-bearing than harmony analysis because timing problems are easier for humans to flag than harmonic ones.
    The new `analyze_drum_groove` should probably land first because it has clearer instrument semantics.
19. **`analyze_velocity_response`** — Patch-QA, narrower than `analyze_instrument_range`.
20. **`analyze_arrangement` / `analyze_form_map`** — Useful for long-form composition and section contrast. A
    deterministic first version can be built from bar-level features and self-similarity; heavier audio-structure
    models belong in the future sidecar category.

### Tier 3 — Specialized, build only when needed

21. **`compare_to_reference`** — Powerful for "make it sound like" prompts but requires the user to bring a
    reference. Build after `render_section_to_wav` is in place.
22. **`compare_patterns`**, **`compare_patches`**, **`humanize_notes`**, **`generate_variation`**,
    **`analyze_track`**, **`get_mix_meters`** — Each solves a narrower problem. Pick up as concrete user demand
    appears.
23. **Future optional ML / third-party tools** — `analyze_style_embedding`, `compare_style_to_reference`,
    `decompose_reference_to_stems`, `transcribe_reference_to_midi`, and `analyze_audio_meter_map`. These can be
    very powerful, but should live behind an explicit optional sidecar/backend because they introduce model
    downloads, licensing questions, runtime cost, and reproducibility concerns.

### What deliberately is **not** here

- Real-time spectrum streaming over MCP — high bandwidth, low value for an offline-reasoning agent.
  `analyze_mix_bus` over a rendered window covers the same questions.
- Style-transfer / "make it sound like genre X" generation — out of scope for this layer; belongs in a higher-level
  agent that *uses* these tools. Style *analysis* via embeddings is listed only as a future optional sidecar.
- Stem export for Pertylizer-authored tracks — covered by `analyze_track` + `render_section_to_wav` with a track
  solo. ML-based reference-audio stem separation is a separate future optional tool.

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
- **Dependency boundary.** The core MCP analyzer set should remain deterministic, local, and testable from the
  Rust project state wherever possible. Tools that require ML models, Python runtimes, large downloads, GPU/ONNX
  dependencies, or non-trivial third-party licensing should be exposed through an explicit optional sidecar/backend
  rather than becoming required core dependencies.

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
6. ✅ `analyze_pattern` — shipped 2026-05-16. Pure symbolic, no audio render.
7. `analyze_drum_groove` — pure symbolic drum feel diagnostics, using inferred drum profiles.
8. `analyze_bass_drum_lock` — symbolic groove + low-end relationship between kick, bass, and chord roots.
9. `analyze_masking_matrix` — pairwise per-track spectral masking on top of deterministic solo renders.
10. `analyze_harmonic_function` — Roman numerals, functional roles, cadence/tension analysis on top of harmony.
11. `analyze_instrument_range` — sweep on top of `analyze_note`.
12. `render_section_to_wav` — generalization of the renderer that writes out instead of analyzing.
13. ✅ True-peak + LUFS-S/M on `analyze_mix_bus` — shipped 2026-05-16.

**Tier 2 / Tier 3** — picked up by demand. Keep optional ML/reference-audio tools behind a sidecar boundary.

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

### 7.1 Reuse one `SynthEngine` across the N per-track renders — ✅ shipped 2026-05-16

`render_per_track_contributions` (`crates/pertylizer/src/mcp_bridge.rs`) previously called
`render_arrangement_to_buffer_with_song` once per audible track. Each call spun up a brand-new offline
`SynthEngine` and replayed every instrument's full module graph, connections, parameters, effect chain, bypass
states, and volume/pan into it via `send_blocking`. For an N-track section that work was repeated N times even
though only the per-track `solo` flags differ between iterations.

**Shipped form.** New `pub struct OfflineEngineSession` in
`crates/pertylizer/src/audio/arrangement_render.rs` owns the offline engine + handle and exposes
`OfflineEngineSession::render_range(song, start, end) -> Result<RenderedArrangement, _>`. Setup
(snapshot live instruments, build engine, load every instrument's voice graph + samples,
`on_stream_start`) runs once in `OfflineEngineSession::new`; `render_range` then handles the per-call
work (reseed `fastrand`, `Stop`, drain residual voice/effect state, `SetSong`, warm-up on first call
only, `Play`, `Seek`, frame loop, `Stop`). `render_arrangement_to_buffer_with_song` is now a 4-line
wrapper around the same session API so single-render callers (`analyze_mix_bus`, `analyze_section`
without `include_per_track`) keep identical semantics.

**Voice-bleed drain.** `Stop` flips active envelopes into release but does not advance them; without
extra processing, the next render's first sample inherits whatever amplitude the released voice
still holds. `VOICE_DRAIN_BLOCKS = 64` (≈ 372 ms at 44.1 kHz) silent `engine.process` calls are run
between consecutive `render_range` calls — empirically the minimum to bring the dual-oscillator
regression patch back to bit-exact fresh state. Skipped on the first call where the engine has no
prior voices.

**Determinism preserved.** `fastrand::seed(OFFLINE_RENDER_SEED)` is called at the start of every
`render_range` call (not in `new`), so consecutive renders consume the same RNG bytes for note-on
phase randomization regardless of how many renders preceded them. Combined with the §8.1 Round-2
`BTreeMap` ordering fix, repeated renders on a reused session are bit-exact for the test corpus.

**Test coverage added in `tests/arrangement_render_determinism.rs`:**

- `session_reuse_matches_fresh_engine_dual_osc` — one fresh-engine render and one session render
  produce the same bytes for the same range. Anchors the §7.1 wrapper.
- `session_render_range_is_bit_exact_across_three_calls` — three back-to-back `render_range` calls
  on the same session and same range produce bit-exact identical buffers. Anchors the voice-bleed
  drain.
- `session_render_range_is_bit_exact_for_noise_patch` — repeats the above for the noise patch,
  which consumes `fastrand` every sample.

This is the single biggest win for `analyze_section` with `include_per_track = true` on real songs:
an N-track section previously paid the full engine + module-graph load N times; with the session,
it pays it once.

### 7.2 Parallelize the per-track renders with rayon — medium impact

The N renders in `render_per_track_contributions` are independent (each its own engine + soloed song clone, no
shared mutable state). Today they run serially in a `for` loop. With §7.1 landed, the natural next step is
`par_iter` over `targets`, scaled to `min(N, num_cpus)`, with each worker holding its own
`OfflineEngineSession`. Watch peak memory — each worker holds one session plus a `Song` snapshot — but for
8–16 tracks on modern hardware this trades RAM for near-linear wall-clock improvement.

Note: the current per-track loop mutates a single shared `song_arc`'s solo flags between iterations.
Parallelization needs a per-worker song clone (or a non-mutating solo-override mechanism in the renderer) to
keep the workers independent — otherwise two workers fight over the same `solo` flag.

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

### 7.6 `TrackContribution` field drift vs. `MixBusMetrics` — ✅ shipped 2026-05-16 (Option 1)

`TrackContribution` previously duplicated 9 of `MixBusMetrics`'s numeric fields. The §0 work on `true_peak` +
LUFS-S/M forced the decision: every metric would have needed to land in two places.

Took **Option 1**: embed `MixBusMetrics` inside `TrackContribution`. CLAUDE.md's "active development — no
backward compatibility required" stance made this the right call: the response shape changes (clients read
`tc.metrics.peak` instead of `tc.peak`), but the duplication is eliminated permanently and adding any future
metric only touches `MixBusMetrics`. The pan-law-compensated `pre_master_peak` + `pre_master_peak_dbfs` and the
existing identity/share fields (`track_id`, `track_name`, `instrument_id`, `rms_share`) stay at the outer
level — they don't belong in `MixBusMetrics`.

Wire-format consequence: any existing client reading `per_track[i].peak` / `per_track[i].rms` etc. directly
must update to `per_track[i].metrics.peak` / `per_track[i].metrics.rms`. Internal tests
(`analyze_section_per_track_breakdown_emits_one_entry_per_track` and friends) were updated in the same edit
pass.

Bonus side-effect: `TrackContribution` now exposes per-track `stereo_correlation`, `mid_rms`, `side_rms`,
`stereo_width`, `mono_compat`, `crest_factor_db` for free — useful for "is this lead actually wide?" or
"does this pad have a problematic L-R imbalance in isolation?" debugging that the previous flat layout
omitted.

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

**Shipped form (v0.278.0, 8.2b path).** New module `pertylizer::analysis::instrument_profile` (~600 lines
incl. tests). `InstrumentProfile` exposes five independent axes —
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

#### 8.2c Inference round 2 — eight rule fixes after multi-song live testing — ✅ shipped in 74d18da + 93c0786

Once `get_instrument_profiles` (added in `93c0786`) exposed the inference output directly through MCP, four
test songs were run against it. The shipped 8.2b rules classified samples correctly but had clear failure
modes on synth patches: 33 % accuracy across 61 synth instruments, with 18 % of instruments dropped to
`Role::Unknown` and the rest split between near-correct and clearly-wrong classifications. Seven changes to
`classify_role` (plus one new graph signal) raised accuracy to ~64 % across the same instruments. The four
test songs were retained as the regression suite for any further inference work.

| # | Change | Cascade position | Effect |
|---|--------|------------------|--------|
| 1 | Pad-gate relaxed to `(Sustained \|\| Evolving) && (Polyphonic \|\| Chordal) && (Tonal \|\| Mixed)` | was step 3, now step 5 | Real pad patches (texture: polyphonic, 2-4 notes; sustained envelope, attack < 500 ms) fire pad-gate. Previously required `Evolving && Chordal` (≥ 5 simultaneous notes) and so missed every monophonic-or-polyphonic pad in the corpus. |
| 2a | New `envelope_shape: Unknown` fallback after every primary gate | new step 9 | Patches without a single dominant Envelope module previously fell to `Role::Unknown` with confidence 0.0. Now soft-classify from name + pitch + texture (Pad / Lead / Fx) with confidence 0.4. |
| 2b | Sustained-polyphonic-tonal mid/high register caught by relaxed Pad-gate | step 5 | Closes the "polyphonic Lead/Brass/Strings → Unknown" gap from §8.5.1 below. (Imperfect for instruments named Lead; tracked in §8.5.) |
| 3 | Drum-gate envelope accepts `Plucked` when register is sub/bass | step 1 | Synth-kick envelopes that are slightly longer than the Percussive threshold (sustain ≈ 0, release ~300 ms) now fire drum-gate. New signal `plucked-bass` in the trail. |
| 4 | Drum-gate pitch accepts `pitch_spread ≤ 5` semitones for pitched percussion | step 1 | Synth-kicks with pitch envelopes that sweep 2-5 semitones across hits (Mixed/Tonal pitch role) now fire drum-gate. New signal `narrow-pitch-spread` in the trail. |
| 5 | Drum-gate requires short envelope alongside `has_noise_source` | step 1 | A noise source with a sustained or evolving envelope is a sweep, not a drum. Falls through to FX-gate now. |
| 6 | Lead-by-name-precedence step before Bass-gate | new step 3 | When `name_hint == Some(Lead)` and the envelope/texture matches, fire Lead-gate even if register is sub/bass. A "Sub Lead" patch should be a Lead, not a Bass. |
| 7 | Pluck-gate fires before Bass-gate when envelope is `Plucked` and texture is `Monophonic` | new step 2 | A plucked-envelope monophonic patch in the bass register is more pluck-like than bass-like. Fixes "Pluck Synth" mis-classified as bass. |

Side fix: the default Lead-gate (step 7) now requires `pitch_role != Atonal` — an atonal monophonic signal
is a sweep/FX, not a melodic lead. Without this, the noise-sweep relaxation (#5) above leaked from Drums to
Lead instead of falling through to FX.

The drum-gate now uses a new `has_oneshot_sampler` graph signal (a Sampler module whose `PlayMode` parameter
is `OneShot`) as percussive evidence on equal footing with `has_noise_source` and `EnvelopeShape::Percussive`.
This makes sample-based percussion (Sampler + Output, no envelope, no oscillator) — the dominant shape in
ZIP-bundle projects — fire drum-gate without manual categorization.

**Accuracy lift across the test corpus (synth patches only):**

| Song | Instruments | Pre-fix accuracy | Post-fix accuracy |
|------|-------------|------------------|-------------------|
| Oxygene Dreams (80s Techno) | 16 | 38 % (6/16) | 56 % (9/16) |
| Neon Horizon (Extended) | 13 | 31 % (4/13) | 62 % (8/13) |
| Neon Horizon | 7 | n/a | 100 % (7/7) |
| Oxygène Dreams (9-instr) | 9 | n/a | 78 % (7/9) |
| Oxygène Dreams (20-instr) | 20 | 30 % (6/20) | 60 % (12/20) |
| **Synth corpus total** | **65** | **~33 %** | **~64 %** |

`get_instrument_profiles` is the canonical way to inspect this: each `InstrumentProfileResult` carries the
inferred role, a 0.0..=1.0 confidence, and a signal trail listing every axis that contributed (`decision`,
`name`, `envelope`, `graph`, `pattern`, plus `manual` when `set_instrument_category` was used). Manual
override is still authoritative — it reports as `manual-override` with confidence 1.0 and short-circuits the
decision tree.

### 8.3 Per-track contribution: clarify pan-law output, optionally add `pre_master_peak` — ✅ fully shipped 2026-05-16 (doc fix + `pre_master_peak`)

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

**Shipped 2026-05-16 (doc).** `TrackContribution`'s doc comment in
`crates/synth_mcp/src/types.rs` and the `analyze_section` MCP tool description in
`crates/synth_mcp/src/server.rs` now spell out that per-track peak/RMS include
constant-power pan-law attenuation (≈ 0.7071 / -3 dB on each channel for a
center-panned source), and direct users to `analyze_note` for the unattenuated
internal-signal peak.

**Shipped 2026-05-16 (`pre_master_peak`).** `analyze_mix_buffer` records the
per-channel sample peaks (`peak_left` / `peak_right`) on its existing frame
loop. The bridge then reads each target instrument's `(volume: Gain, pan:
BipolarValue)` from `SynthSession::state().instrument_snapshots` and reverses
the engine's mix-stage attenuation analytically:
`pre_master_peak = max(peak_left / (volume × gL), peak_right / (volume × gR))`
where `(gL, gR) = Gain::from_pan(pan)` is the same constant-power pan-law the
realtime engine applies. No second render. With §7.1's engine reuse the
per-track loop now does exactly one render per audible track — halving the
wall-clock cost of `include_per_track = true` vs. the earlier two-render
draft, and removing the `.expect()` pan/volume restore dance that the
two-render path needed. `TrackContribution.pre_master_peak` (linear) and
`pre_master_peak_dbfs` join the existing identity / share fields; the dBFS
field clamps to -200.0 for silent tracks. Unit tests in
`mcp_bridge::pre_master_peak_tests` cover center pan, hard pan (no
divide-by-zero on the silent channel), volume drop, unknown instrument
(falls back to raw channel peak), and silence (`-200.0` floor). The original
`analyze_section_per_track_pre_master_peak_compensates_for_pan_law` integration
test still passes — `pre_master_peak / metrics.peak ≈ √2` for default-pan
tracks falls out naturally from the analytical reversal.

### 8.4 Offline-render snapshot propagation bug class — HIGH ✅ **Fixed in 74d18da**

Running the analyzers against a sample-based project (`echoing (003)`, 9 sample instruments) revealed a
class of bugs where read paths (`analyze_note`, `analyze_mix_bus`, `analyze_section`, `get_sampler_state`,
`get_engine_status`) consulted an engine snapshot that the realtime audio thread never wrote into. Audible
playback worked but offline analysis returned -200 dBFS across every metric — the sampler modules in the
fresh offline `SynthEngine` had their `SampleSelect` IDs set but no audio buffer attached.

Four instances were fixed together:

1. **Offline renderers didn't load sample data.** `render_note_to_buffer` and
   `render_arrangement_to_buffer{,_with_song}` create a fresh `SynthEngine` and rebuild the module graph via
   `SetModuleParameter` commands, but never sent `EngineCommand::LoadSampleData`. Fixed by threading
   `SharedSampleLibrary` through the renderer call chain and adding a `load_sample_data_for_samplers`
   helper that walks sampler modules and dispatches `LoadSampleData` after the graph is built. A new shared
   `load_sample_data_command` helper now constructs the `LoadSampleData` payload for both the live engine
   (`egui_backend::send_loaded_sample_data`) and the offline path.
2. **`load_project_data` didn't sync tempo to the engine transport.** The Song's `default_tempo` flowed to
   the sequencer (so playback was correct), but `EngineCommand::SetTempo` was never sent. `get_engine_status`
   reported the engine default (120 BPM) for every loaded project. Fixed by sending `SetTempo` alongside the
   other restored global state in `load_project_data`.
3. **`get_sampler_state` read sample id via a lossy f32 path.** `SamplerParam::SampleSelect::as_f32()`
   returns 0.0 ("not meaningful as f32" — sample ids don't fit a slider widget), so reading the parameter
   through the f32 indirection always reported 0 regardless of the real assignment. Fixed by pattern-matching
   the typed `Param::Sampler(SamplerParam::SampleSelect(SampleId))` directly off the snapshot. `PlayMode` and
   `Direction` are now also matched as typed enums instead of being round-tripped through u8.
4. **Drum-gate didn't fire for sample-based percussion.** Sample-only patches (Sampler + Output, no envelope,
   no noise) failed both branches of the gate. Added the `has_oneshot_sampler` graph signal so a Sampler in
   `OneShot` PlayMode counts as percussive evidence — see §8.2c #3 for the cascade-level details.

**Lesson.** Whenever an analyzer/read path reports unexpected silence, defaults, or stale values, check
whether the live audio thread has state (sample data, tempo, mod-matrix slot, sampler params, etc.) that
needs an explicit propagation command and isn't being sent on the offline-side `SynthEngine::new()`. The
realtime engine has it; the snapshot consulted by readers may not. Particularly suspect anything where the
value flows through `Param::as_f32()` — that conversion is lossy for several variants.

### 8.5 Inference round-3 follow-ups (live testing 2026-05-12) — ✅ all five fixes shipped 2026-05-16

After the round-2 improvements (§8.2c), four songs were re-run through `get_instrument_profiles`. Accuracy
went from ~33 % to ~64 %, but five concrete failure modes persisted. All five are now closed; each
sub-section below carries a ✅ marker and a brief note on the shipped fix.

#### 8.5.1 Bass-gate dominates name-priority — ✅ shipped 2026-05-16

Pad/Lead/Strings/etc. patches that play in the bass register get classified as `Bass` even when the name
clearly says otherwise. The Bass-gate fires at cascade position 4, before Pad (5) and the default Lead (7);
when the patch's pitch is `Tonal`/`Mixed` and register is `Sub`/`Bass`, Bass-gate wins regardless of name.

Concrete examples from the test corpus:

| Song | Instrument | Got | Expected (from name) |
|------|------------|-----|----------------------|
| Oxygène Dreams (20-instr) | Fractal Pad | bass 0.55 | Pad |
| Oxygène Dreams (20-instr) | String Ensemble | bass 0.55 | Pad |
| Oxygène Dreams (20-instr) | Unison Supersaw | bass 0.90 | Lead |
| Neon Horizon (Extended) | Pad | bass 0.55 | Pad |

The fix that already exists for Lead (the Lead-by-name-precedence gate at cascade position 3) covers
monophonic Lead patches in the bass register, but doesn't cover Pad (because Pad-gate requires polyphonic+
texture, so the cascade order between Lead-precedence and Bass-gate doesn't help). The cleanest fix is a
generalized "name-priority for exact vocab match": if the name hint is `Some(Role::X)` and that role's gate
*could* fire on a relaxed register requirement, prefer it over a register-based Bass-gate. The conservative
form is `name_hint == Some(Pad) && pad_gate_without_register_check matches → Pad with confidence 0.6`.

**Shipped.** New cascade step 4 (Pad-precedence-by-name) sits between Lead-precedence (step 3) and Bass-gate
(now step 5). Fires when `name_hint == Some(Pad)` AND `envelope ∈ {Sustained, Evolving}` AND `texture ∈
{Polyphonic, Chordal}` AND `pitch_role ∈ {Tonal, Mixed}` — same shape as the relaxed Pad-gate, no register
check. Counter-test ensures a monophonic patch named "Pad" still falls through to Bass-gate.

#### 8.5.2 Atonal patches in mid register fall through Bass-gate to FX or Lead — ✅ shipped 2026-05-16

Bass-gate requires `pitch_role in [Tonal, Mixed]`. A sub-bass that plays one note repeatedly is `Atonal`
and so escapes Bass-gate even when the register is sub/bass.

| Song | Instrument | Got | Expected |
|------|------------|-----|----------|
| Oxygene Dreams (80s Techno) | Sub Bass | fx 0.30 | Bass |
| Neon Horizon (Extended) | Sub Bass | lead 0.55 | Bass |

Fix candidates: (a) relax Bass-gate to accept Atonal when `name_hint == Some(Bass)` and register is
sub/bass; (b) generalize 8.5.1's name-priority rule to bass-by-name. (a) is simpler and risks no new
mis-classifications because the bass-by-name combination is unambiguous.

**Shipped (approach a).** Bass-gate's pitch check is now `Tonal | Mixed | (Atonal && name_hint == Some(Bass))`.
Register + texture requirements unchanged, so the unrelaxed bass criteria still gate FX/Lead candidates that
just happen to play low.

#### 8.5.3 Tom-style tonal percussion fails drum-gate — ✅ shipped 2026-05-16

Tom patches play 1-3 distinct tom-tuned notes (a small but non-zero pitch spread). Fix §8.2c #4 set the
`pitch_spread ≤ 5` threshold for the relaxed drum-gate, but real Tom patterns can spread further (e.g. 2-3
toms tuned to ~8 semitones apart). With pitch spread > 5 and pitch_role = Tonal, the drum-gate doesn't
fire; the Pluck-before-Bass step (§8.2c #7) then captures Tom as `pluck`.

| Song | Instrument | Got | Expected |
|------|------------|-----|----------|
| Neon Horizon (Extended) | Tom | pluck 0.40 | Drums |

Fix candidates: (a) relax the pitch-spread threshold to ~10-12 semitones when name_hint says Drums; (b) add
a "tonal-narrow-percussive" branch with a more permissive spread when envelope is Plucked + percussive-like
register; (c) name-hint-priority — if name says Drums and envelope is short, classify as Drums even when
pitch is wider.

Note that fix #6 (Lead-by-name-precedence) already establishes the pattern of "name says X, fire X's gate
before the cascade decides". (c) is the same shape applied to Drums.

**Shipped (approach a).** `DRUM_PITCH_SPREAD_LIMIT_NAMED = 12` is applied when `name_hint == Some(Drums)`;
the strict `DRUM_PITCH_SPREAD_LIMIT = 5` still applies otherwise. The `narrow-pitch-spread` signal trail
also uses the relaxed limit when relevant, so the named-Drums case still emits the same evidence string.

#### 8.5.4 Polyphonic Lead patches get caught by relaxed Pad-gate — ✅ shipped 2026-05-16

After §8.2c #1, Pad-gate accepts polyphonic+sustained+tonal+mid. Lead patches that happen to be
polyphonic (sustained chord-stab leads, brass-style polyphonic leads) now hit Pad-gate before the
monophonic-only Lead-gate. With name_hint = Lead, the name-conflict policy kicks in: role stays Pad,
confidence drops by 0.2.

| Song | Instrument | Got | Expected |
|------|------------|-----|----------|
| Neon Horizon (Extended) | Lead | pad 0.40 | Lead |
| Oxygène Dreams (20-instr) | Glitch Pad | fx 0.30 | Pad (atonal pad — separate issue) |

The Lead → pad case is "less wrong than before" — previously this exact pattern fell to Unknown 0.0,
which gave the user no information at all. Pad 0.40 with name-conflict trail at least tells the user the
inference is uncertain. Fix would relax Lead-gate to accept Polyphonic when name_hint == Lead (same
pattern as §8.2c #6 / 8.5.1).

**Shipped.** Lead-precedence-by-name (cascade step 3) now accepts `texture ∈ {Monophonic, Polyphonic}`
instead of `Monophonic` only. Polyphonic chord-stab leads named "Lead" fire Lead-precedence before the
relaxed Pad-gate (now step 6) gets a chance. The atonal-Pad case for "Glitch Pad" remains a separate
issue — fix #1 still requires `pitch_role ∈ {Tonal, Mixed}`.

#### 8.5.5 Name vocabulary is too narrow — ✅ shipped 2026-05-16

`role_from_name` matches a small fixed vocabulary; many real instrument names get no hint at all and rely
purely on the decision tree. The 49-instrument corpus showed these names produce no name_hint:

- `Tom` ← actually in the vocab, but tokenizes as the whole instrument name so `has("tom")` does match;
  not a vocab gap. (Listed here as a near-miss to remember the tokenizer's word-match semantics.)
- `Brass`, `Strings/String Ensemble` (only `strings`/`string` is in the vocab — "ensemble" isn't)
- `Arp`, `Arp Main`, `Arp High`, `Crystal Arp` (no `arp` token)
- `Stab`, `Punchy Stab` (no `stab` token — stabs are typically Pluck/Keys)
- `Supersaw`, `Unison Supersaw` (no `supersaw` token — supersaws are leads/pads)
- `Chime`, `Digital Chime` (no `chime` token — chimes are Pluck/Keys)
- `Shimmer`, `Ethereal Shimmer Pad` (Pad token catches it; `shimmer` alone wouldn't)
- `Glitch`, `Glitch Pad` (same; pad token catches it)

Extension: add `brass` → Lead, `arp` → Lead (or new `Arp` role), `stab`/`stabs` → Pluck, `supersaw` → Lead,
`chime`/`chimes`/`bell`/`bells` → Pluck. The vocabulary is intentionally word-match (not substring) to
avoid false hits like `bassoon` → bass; extension just adds tokens.

**Shipped.** Lead vocabulary extended with `brass`, `arp`, `supersaw`. Pluck vocabulary extended with
`stab`, `stabs`, `chime`, `chimes`, `bell`, `bells`. Tests cover both the positive matches and the
word-match-not-substring invariants (`Stable Drone` ≠ Stab, `Doorbell` ≠ Bell). Tokens for `brass` /
`arp` / `supersaw` in particular unlock §8.5.1 / §8.5.4 fixes for instruments whose names previously
produced no name_hint.

#### Suggested fix order

8.5.1 (name-priority for Pad) → 8.5.2 (Atonal bass-by-name) → 8.5.5 (vocabulary) → 8.5.3 (Tom pitch spread)
→ 8.5.4 (Polyphonic Lead). The first two cover the majority of remaining mis-classifications by themselves;
vocabulary is cheap and reduces name-conflict cases; 8.5.3 and 8.5.4 are narrower long-tail fixes.

**Shipped order matches the suggested order.** All five landed together on 2026-05-16 inside the same
edit pass on `classify_role` and `role_from_name`.

### 8.5 round-4 follow-ups (live testing 2026-05-16) — ✅ both shipped 2026-05-16

End-to-end verification of the round-3 fixes against the "Neuro F#m 174" project (13 instruments, all
manually categorized → temporarily uncategorized to exercise the inference path) confirmed 11 of 13
auto-classifications correct. The two remaining failure modes are documented below as the next batch of
inference work; each has a concrete example from that verification run.

#### 8.5.6.1 Plucked + Monophonic + `name_hint == Lead` misclassifies as Pluck — ✅ shipped 2026-05-16

The cascade order is:

1. Drums
2. Pluck (`envelope == Plucked && texture == Monophonic`)
3. Lead-precedence-by-name (`name_hint == Some(Lead) && lead_envelope_ok && texture ∈ {Mono, Poly}`)
4. Pad-precedence-by-name
5. Bass
6. Pad (relaxed)
…

§8.5.4 widened Lead-precedence (step 3) to accept Polyphonic so polyphonic chord-stab leads no longer
fall into the relaxed Pad-gate. It did not address the case where a Plucked + Monophonic patch named
"Lead" reaches step 2 (Pluck) before step 3 (Lead-precedence) — Pluck-gate fires, then
`apply_name_override` records a `name-conflict` and shaves 0.2 off the confidence.

Example from the test corpus (Neuro F#m 174 with categories cleared):

| Instrument | Envelope | Texture | name_hint | Got | Expected |
|------------|----------|---------|-----------|-----|----------|
| Arp Lead | Plucked | Monophonic | Some(Lead) | pluck 0.40 (name-conflict) | lead |

By contrast, **Arp Echo** in the same project (Plucked + Polyphonic + name=Lead, via the `arp` token
added in §8.5.5) correctly resolves to `lead 0.90` via Lead-precedence — only the monophonic case is
broken.

Fix candidates:

(a) **Reorder cascade** — move Lead-precedence-by-name BEFORE Pluck-gate. Risk: a "Pluck Lead"-style
patch (envelope=Plucked, name says Lead but the user intended a plucked patch) would now resolve to
Lead instead of Pluck. Probably acceptable since the user named it Lead.

(b) **Guard Pluck-gate against `name_hint == Some(Lead)`** — skip Pluck-gate when the name explicitly
says Lead, letting Lead-precedence catch it. Narrower fix, lower blast radius. Mirrors the existing
pattern of "name-priority gates" introduced in §8.2c #6 and §8.5.1.

Recommendation: **(b)** — Pluck-gate keeps firing for every other plucked monophonic patch, but yields
to a user-stated Lead intent. Same logical shape as the existing Lead-precedence-by-name; just
expressed as a guard on Pluck-gate instead of a new gate.

**Shipped (approach b).** Pluck-gate (cascade step 2) now requires `name_hint != Some(Role::Lead)`,
so a Plucked + Monophonic patch named "Lead" falls through to Lead-precedence (step 3) and resolves
as `lead` with the existing `lead-gate` signal trail. Counter-test ensures that name=Pluck with the
same shape still fires Pluck-gate.

#### 8.5.6.2 `name-conflict` penalty parks Impact-style patches at the 0.60 auto-exclude threshold — ✅ shipped 2026-05-16

`analyze_harmony` defaults to excluding tracks classified as `Drums` with `confidence ≥ 0.6`. When the
decision tree fires Drums-gate via DSP signals (plucked-bass envelope + atonal pitch + sub register)
but the name vocabulary points elsewhere, `apply_name_override` applies a -0.2 `name-conflict`
penalty. Drums-gate's base confidence + one DSP-axis bonus = 0.80; after the penalty, 0.60 exactly.
That's at the threshold — passes today's `>= 0.6` comparison, but a future tightening to `> 0.6`
would silently break drum exclusion for these patches.

Example from the test corpus:

| Instrument | Got | Threshold margin |
|------------|-----|------------------|
| Impact | drums 0.60 (drums-gate, plucked-bass, name-conflict — name says Fx via `impact` → FX vocab) | 0 |

Fix candidates:

(a) **Soften the `name-conflict` penalty when DSP signals are strong** — e.g. reduce the penalty from
0.2 to 0.1 when the decision-tree signals are unambiguous (drums-gate fired with both an envelope and
a pitch_role signal, not just one). Keeps the penalty in place for genuinely ambiguous cases but lets
high-confidence DSP signals dominate weakly-correlated name hints.

(b) **Move "impact" out of the FX vocabulary** — "impact" patches in EDM/dnb are typically sub-thumps
(percussion), not FX risers. Reclassify `impact` as either Drums or remove it from the FX vocab so it
produces `name_hint = None`. Concrete impact: only this token; trivial to ship; matches the genre
usage in the test corpus.

(c) **Raise the bonus baseline for Drums-gate** so `plucked-bass + atonal` produces conf 0.85 pre-
penalty instead of 0.80 — pushes name-conflict drums to 0.65 instead of 0.60, comfortably above any
future strict-threshold change. Minimal blast radius.

Recommendation: **(b) + (c)** — (b) is genre-correct and removes the conflict at the root; (c) gives
the auto-exclude threshold a safety margin for all name-conflict drum cases, not just impacts.

**Shipped (approach b + c).**

- (b) `impact` token moved from the FX vocabulary to the Drums vocabulary in `role_from_name`.
  Patches named "Impact", "Sub Impact", "Drop Impact" etc. now produce `name_hint = Some(Drums)`,
  so name + DSP signals agree and `apply_name_override` clamps confidence to ≥ 0.85 instead of
  applying the conflict penalty.
- (c) Drums-gate confidence base bumped from 0.6 to 0.65 (other gates still use 0.6 — Drums was
  already an outlier with its 0.2 bonus increment vs. 0.15 elsewhere). DSP-driven drum
  classifications with a `name-conflict` penalty now land at 0.65 instead of 0.60 — comfortably
  above any future strict-threshold change in analyze_harmony.

Counter-test asserts `drums-gate + name-conflict` produces `confidence > 0.6`.

#### Suggested round-4 fix order

8.5.6.1 (Pluck-gate guard on `name_hint == Lead`) → 8.5.6.2 (impact vocab move + Drums-gate bonus
bump). 8.5.6.1 is a real mis-classification visible in `get_instrument_profiles`; 8.5.6.2 is a margin
issue that only becomes a bug if the threshold tightens, so lower priority. Both are small additions
to `classify_role` / `role_from_name` — same edit-pass shape as round-3.

**Shipped order matches the suggested order.** Both fixes landed together on 2026-05-16 inside the
same edit pass on `classify_role` and `role_from_name`. Verified end-to-end against the
"Neuro F#m 174" project: Arp Lead now resolves as `lead 0.90` (previously `pluck 0.40` with
name-conflict), Impact resolves as `drums 1.00` (previously `drums 0.60` at the threshold edge).

### 8.6 Cross-reference

§8.1 is fully fixed (Round 1 + Round 2); §4's "deterministic output for a given project state" claim holds
bit-exact end-to-end through the MCP bridge, not just inside the determinism unit tests. §8.2 (auto-inferred
profiles) shipped in v0.278.0, with §8.2c (round-2 inference improvements) and §8.4 (offline-render
snapshot propagation) landing together in commits 74d18da + 93c0786 — the previously implicit "manual
`set_instrument_category` required" workflow is now fully optional. §8.3 is fully closed: the mandatory
doc fix (pan-law attenuation in per-track peak/RMS) and the optional `pre_master_peak` field both shipped
2026-05-16. All five §8.5 round-3 inference follow-ups (§8.5.1–§8.5.5) shipped 2026-05-16. §8.5.6.1
(Plucked-Mono-Lead misclassification) and §8.5.6.2 (0.60-threshold margin for name-conflict drums) — both
surfaced by the 2026-05-16 end-to-end verification on the "Neuro F#m 174" project — also shipped 2026-05-16.
Every §8 item is now closed; Tier-1 item 6 (`analyze_pattern`) shipped 2026-05-16. The next blind
spot to address is Tier-1 item 7 (`analyze_drum_groove`).
