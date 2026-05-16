# MCP Music Tools — Plan

> **Date:** 2026-05-11 (last update 2026-05-17: Group D from §8.6 — `analyze_tension_curve`, `suggest_music_fixes` — shipped in v0.287.0. Earlier 2026-05-17: Group C from §8.6 — `analyze_arrangement`, `analyze_form_map`, `find_motifs`, `analyze_hook_strength` — shipped in v0.286.0. Earlier 2026-05-17: Group B from §8.6 — `generate_chord`, `transpose_notes`, `quantize_notes_to_scale`, `quantize_notes_to_grid` — shipped in v0.285.0. Previous update 2026-05-16: §0 sweep — `true_peak`, `lufs_momentary_max`, `lufs_short_term_max`, `pre_master_peak` shipped; `TrackContribution` restructured to embed `MixBusMetrics` (§7.6 decision); §8.3 `pre_master_peak` closed. Earlier 2026-05-16 work: higher-level music-understanding catalogue + optional ML sidecar; §8.3 pan-law documentation, §8.5.1–§8.5.5 inference round-3 fixes — Pad-precedence-by-name, Bass-gate Atonal-by-name, Lead-precedence allows Polyphonic, drum-gate wider pitch-spread when name says Drums, extended name vocabulary; §7.1 `OfflineEngineSession` engine-reuse; §8.5.6.1/§8.5.6.2 round-4 inference follow-ups.)
> **Status:** **Tier 0 shipped in v0.276.0; Tier-1 items 4 and 5 shipped in v0.277.0; offline-render determinism fix in two rounds post-v0.277.0; §8.2 auto-inferred instrument profiles shipped in v0.278.0; commits 74d18da + 93c0786 closed the offline-render snapshot bug class, added `get_instrument_profiles`, and applied seven inference improvements that roughly doubled live-test accuracy on synth patches; inference round-3 (§8.5.1–§8.5.5) + §8.3 pan-law doc + §7.1 engine reuse shipped 2026-05-16; §0 deferred trio (`true_peak`, LUFS-S/M, `pre_master_peak`) + §7.6 embed-MixBusMetrics decision shipped 2026-05-16.** Remaining Tier-1+ pending.
> Post-ship live testing surfaced determinism, auto-categorization, and offline-render-state issues — see §8. §8.1, §8.2, §8.3 (doc + `pre_master_peak`), §8.4, and §8.5 are fully shipped end-to-end through the MCP bridge.
> **Scope:** New MCP tools that give an AI agent the ability to evaluate and shape music as a whole, not only individual sounds.

---

## 0. Status snapshot

### Shipped — v0.276.0 (Tier 0), v0.277.0 (Tier-1 first wave), v0.278.0 (auto-inferred profiles), 74d18da + 93c0786 (offline-render fixes, `get_instrument_profiles`, inference round 2), v0.283.0 (drum-groove + bass-drum-lock + harmonic-function), v0.284.0 (Group A: `analyze_instrument_range` + `analyze_velocity_response`), v0.285.0 (Group B: `generate_chord` + `transpose_notes` + `quantize_notes_to_scale` + `quantize_notes_to_grid`), v0.286.0 (Group C: `analyze_arrangement` + `analyze_form_map` + `find_motifs` + `analyze_hook_strength`), and v0.287.0 (Group D: `analyze_tension_curve` + `suggest_music_fixes`)

| Tool | Version | Notes |
|------|---------|-------|
| `analyze_harmony` | v0.276.0 + v0.277.0 + v0.278.0 + 74d18da | Walks notes by tick, identifies chord symbols (18 templates incl. m7/maj7/dom7/sus/dim/aug/power), infers key via Krumhansl-Schmuckler over 24 major/minor keys, returns in-key ratio, out-of-scale pitch classes, composite stability score. Pattern or arrangement scope. Returns chord events with `start_bar`/`start_beat` (1-indexed). Default grouping: 1 quarter-note for pattern scope, 1 bar for arrangement scope. Consecutive identical chord events are merged. **v0.277.0 added drum-track filtering** (manual `InstrumentCategory::Drums` + explicit `exclude_track_ids`). **v0.278.0 wired `analysis::instrument_profile` auto-inference** into the same filter — uncategorized percussion is now caught with confidence ≥ 0.6 and the warning text carries the signal trail. **74d18da** added the `has_oneshot_sampler` graph signal so sample-based percussion (Sampler + Output, no envelope) also fires drum-gate via the `oneshot-sampler` evidence. Manual category keeps priority via the `manual-override` signal. Arrangement scope only. |
| `analyze_mix_bus` | v0.276.0 + 74d18da | Renders N seconds (default 10, max 300) of the master bus offline from `start_tick` (default 0). Returns sample peak, RMS, crest factor, integrated LUFS (ITU-R BS.1770-4 K-weighted + 400 ms gating, 75 % overlap, abs −70 / rel −10 gates), 4-band frequency-balance RMS (sub/low/mid/high) windowed across the full buffer, stereo correlation, mid/side RMS, stereo width, mono-compatibility score, clipped-sample count. Includes `start_bar`/`end_bar`/`start_beat`/`end_beat`. **74d18da** threaded `SharedSampleLibrary` through the offline renderer so sampler-based instruments stop rendering silence — see §8.4. |
| `analyze_section` | v0.276.0 + v0.277.0 + 74d18da | Same metrics as `analyze_mix_bus` but takes explicit `[start_tick, end_tick)`. **v0.277.0 added per-track contribution breakdown**: `include_per_track` (default false) opts in to one extra soloed render per audible track that overlaps the section, returning `TrackContribution { track_id, track_name, instrument_id, peak, peak_dbfs, rms, rms_dbfs, lufs_integrated, energy_bands, clipped_samples, rms_share }`. Soloing is implemented by cloning the song, setting only the target track's `solo = true`, and rendering via `render_arrangement_to_buffer_with_song`. **74d18da** propagated samples per soloed render so per-track contributions on sample-based instruments report real metrics. |
| `get_instrument_profiles` | 93c0786 | Returns `Vec<InstrumentProfileResult>`, one entry per instrument routed to by at least one track. Each profile carries the inferred `role` plus a `confidence: 0.0..=1.0` and a `signals` trail listing every axis that contributed (`decision`, `name`, `envelope`, `graph`, `pattern`, `manual`). Same inference path that `analyze_harmony`'s `exclude_drums = true` uses — exposed directly so users can debug or override the classification. Manual `set_instrument_category` short-circuits the decision tree and reports as `manual-override` with confidence 1.0. Wire format uses snake_case strings for role/envelope_shape/etc. so the MCP type stays decoupled from pertylizer-internal enums. |
| `analyze_pattern` | 2026-05-16 | Pure symbolic single-pattern analyzer; no audio render. Reads a pattern's notes directly and reports `density` (notes per bar/beat, active ratio), `pitch` (low/high/range, mean, distinct count, duration-weighted pitch-class histogram), `velocity` (min/max/mean/std/range), `rhythm` (max/mean polyphony, monophonic flag, distinct onsets+durations, IOI mean+std, regularity score), and `repetition` (distinct bar signatures quantized to a 32nd-note grid, repetition score). `length_bars` and `notes_per_bar` use the song's default time signature. Notes that start past the pattern length are dropped with a warning. Implementation in `crates/pertylizer/src/analysis/pattern_analysis.rs`; bridge in `analyze_pattern_impl`. Closes Tier-1 #6. |
| `analyze_drum_groove` | v0.283.0 | Pure-symbolic drum-feel diagnostics. Pattern or arrangement scope. In arrangement scope identifies drum tracks via `infer_all_profiles` (Role::Drums, confidence ≥ 0.6) and pulls notes from them; pattern scope treats every note as a drum hit. Classifies each note via the GM drum map (kick / snare / closed-hat / open-hat / tom / cymbal / clap / other — unknown notes still flow through as `other` for custom maps). Reports `composition` (per-component counts), `backbeat` (snare hits on beats 2/4 within a 16th-note tolerance — strength + matched + off-backbeat counts), `hat` (subdivision in `quarter`/`8th`/`16th`/`triplet_8th`/`triplet_16th`/`irregular`/`none`, density per beat, hat count), `ghost_notes` (snare hits below half the loudest snare velocity, only counted when both quiet and loud snares are present), `fills` (bar density exceeding 2× scope mean), `repetition` (distinct bar signatures on a 32nd-note grid). 13 unit tests. Implementation in `crates/pertylizer/src/analysis/drum_groove.rs`; bridge in `analyze_drum_groove_impl`. Closes Tier-1 #7. |
| `analyze_bass_drum_lock` | v0.283.0 | Pure-symbolic kick/bass relationship. Identifies drum tracks (Role::Drums, conf ≥ 0.6) and bass tracks (Role::Bass, conf ≥ 0.6) via the same `infer_all_profiles` path. Aligns kick onsets (GM MIDI 35/36) against bass-note onsets within `onset_tolerance_ticks` (default 120 = ±1/32-note at 960 PPQN, clamped to [30, 960]). Returns `alignment` (matched / kick-only / bass-only counts plus `lock_score = matched/kicks` and `coverage_score = matched/bass`) and `bass_pitch` (most common pitch class on matched onsets + its share + distinct PC counts on-kick vs total + mean bass MIDI). Pattern scope splits a combined rhythm-section pattern into kicks (GM kick MIDI) and bass (everything else). 11 unit tests. Implementation in `crates/pertylizer/src/analysis/bass_drum_lock.rs`; bridge in `analyze_bass_drum_lock_impl`. Closes Tier-1 #8. |
| `analyze_masking_matrix` | 2026-05-16 (post-v0.283.0) | Pairwise per-track spectral masking on top of the §7.2 parallel solo-render pipeline. Each pair carries 4 `BandOverlap` entries (sub 0-100 / low 100-500 / mid 500-2000 / high 2000+ Hz) with linear RMS for each side, `overlap_energy = min(a, b)`, and `dominance_db = 20·log10(max/min)` clamped to 200.0 for silent-vs-non-silent. Pair-level `conflict_score ∈ [0, 1]` = `sum(overlap) / sum(max)` across bands. Pairs returned sorted by descending `conflict_score`. Optional `dominant_track_id` is set when the worst-overlap band shows ≥ 6 dB margin and `hint` is a human-readable string (`"Pad(2) masks Lead(3) in mid (500-2000 Hz)"` or `"…/… compete in …"` when dominance is small). Implementation: `analyze_masking_matrix_impl` + helpers in `crates/pertylizer/src/mcp_bridge.rs`; types `BandOverlap` / `MaskingPair` / `AnalyzeMaskingMatrixResult` in `crates/synth_mcp/src/types.rs`; bridge trait method + server handler. 4 integration tests cover pair shape, determinism across calls, inverted-range rejection, single-track empty-pair case. Closes Tier-1 #9. |
| `analyze_harmonic_function` | v0.283.0 | Tonal-function annotation on top of `analyze_harmony`. Same scope params (pattern_id or arrangement range, `grouping_ticks`, `exclude_drums`, `exclude_track_ids`). Reuses `analyze_song_harmony` end-to-end so chord identification, key inference, and drum exclusion stay in lock-step. For every chord event: scale degree (1..=7 for diatonic), Roman numeral with quality decoration (`I`, `V7`, `ii7`, `vii°`, `ø7`, `°7`, plus `bII`/`bIII`/`bVI`/`bVII` for chromatic), function bucket (`tonic`/`subdominant`/`dominant`/`other`/`chromatic`, simplified Riemannian), and tension (function base + 0.15 for dom7 + 0.2 for dim/half-dim). Detects cadences on consecutive pairs: Authentic (V → I), Plagal (IV → I), Half (anything → V), Deceptive (V → vi). Returns the chord stream, cadence index list, function distribution (T/S/D/Other/Chromatic counts), and tension stats (mean/peak/trough/std-dev). Leading-tone in minor (interval 11) labeled as Dominant when the chord has major or dominant-7 quality (covers borrowed `V` from harmonic minor). 12 unit tests. Implementation in `crates/pertylizer/src/analysis/harmonic_function.rs`; bridge in `analyze_harmonic_function_impl`. Closes Tier-1 #10. |
| `analyze_instrument_range` | v0.284.0 | Patch-QA sweep across a MIDI note range. One offline render per step via the existing `analyze_note` path (`step_semitones` defaults to 12 — one note per octave; cheaper than the obvious one-per-semitone sweep). Returns per-step (`note`, `note_played`, `expected_hz`, `fundamental_hz`, `pitch_error_cents`, `pitch_confidence`, `peak_amplitude`, `rms_overall`, `centroid_hz`, `clipped_samples`, plus boolean `silent` / `likely_aliased` / `pitch_lost`) and a cross-step `issues` summary (`silent_notes`, `aliased_notes` — centroid > Nyquist/2 + confidence < 0.3, `pitch_lost_notes` — fundamental more than an octave off, `clipping_notes`, `level_spread_db`). Catches the bug class where a patch sounds great at C4 in `analyze_note` and falls apart at C6 (aliasing) or C2 (energy loss). Implementation: `analyze_instrument_range_impl` in `crates/pertylizer/src/mcp_bridge.rs`; pure helpers in `crates/pertylizer/src/analysis/patch_sweep.rs`. Closes Tier-1 #11. |
| `analyze_velocity_response` | v0.284.0 | Velocity sweep at a fixed MIDI note. Same render path as `analyze_instrument_range`, but the note is held and velocity walks `[velocity_low, velocity_high]` in steps of `velocity_step` (default 16). Returns per-velocity (`peak_amplitude`, `rms_overall`, `centroid_hz`, `clipped_samples`) and cross-step diagnostics (`amplitude_range_db`, `non_monotonic_amplitude_steps`, `non_monotonic_centroid_steps`, `velocity_unresponsive` — flagged when `amplitude_range_db < 3.0` dB). Confirms a patch actually responds to velocity in a musical way (rising amplitude, brighter filter at higher velocity) instead of being effectively velocity-deaf — common surprise on patches with the wrong envelope → amp routing. Implementation: `analyze_velocity_response_impl` in `crates/pertylizer/src/mcp_bridge.rs`; shares helpers with `analyze_instrument_range`. Closes Tier-2 #19. |
| `generate_chord` | v0.285.0 | Pure-symbolic chord-symbol → MIDI notes. Parses `"Cm7"` / `"F#maj7"` / `"Bbsus4"` / `"G7sus4"` / `"Dm7b5"` / `"C5"` against the same suffix table the identifier emits (round-trip stable) plus synonyms `min`/`minor`, `maj`/`major`, `dim`, `aug`, `ø`. `octave` defaults to 4 (middle-C octave). Voicings: `close` (default), `drop2` (drop the 2nd-highest), `drop3` (drop the 3rd-highest), `open` (drop2+drop3 combined). Voicings that need more notes than the chord has fall back to `drop2` with a warning. Notes clamped to 0..=127 with a warning so callers can retry one octave lower. Returns parsed `root_pitch_class`, `quality`, `suffix`, applied `voicing`, and the MIDI note list. Does not touch the song — pair with `add_notes` to place. |
| `transpose_notes` | v0.285.0 | Shifts every note in `pattern_id` by a signed semitone delta. Notes whose new pitch would leave 0..=127 stay in place, counted in `notes_out_of_range`. When both `scale_tonic` (0..12) and `scale_name` are set, off-scale results snap to the nearest in-scale pitch via `tie_break` (`up`/`down`/`nearest`, default `up`); partial constraint emits a warning and proceeds without snapping. 13 scale templates supported (major, minor, harmonic_minor, melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, pentatonic_major, pentatonic_minor, blues, chromatic). Returns `notes_transposed` / `notes_out_of_range` / `notes_snapped_to_scale` plus the echoed-back scale name (lets callers detect the major-fallback on unknown input). |
| `quantize_notes_to_scale` | v0.285.0 | Snaps every off-scale pitch in `pattern_id` to its nearest in-scale neighbour (search radius ±6 semitones — a 12-pitch-class scale always contains a member within a tritone of any input). Same scale templates + tie_break semantics as `transpose_notes`. Returns `notes_already_in_scale`, `notes_moved`, `mean_correction_semitones`, `max_correction_semitones`. |
| `quantize_notes_to_grid` | v0.285.0 | Snaps note start ticks in `pattern_id` to a `grid_ticks` grid (240 = sixteenth at 960 PPQN, 480 = eighth, 960 = quarter) with optional `strength` (0..1, default 1.0 — full snap), `swing` (0..1, even-indexed grid positions stay, odd push back by up to half-grid), and `humanize_ticks` (max ±jitter per note, seeded with `humanize_seed` for reproducibility — same seed + same notes + same options → byte-identical output). `grid_ticks == 0` returns early with a warning; final tick clamped to `pattern_length - 1` so swing/jitter never push notes past the pattern end. Returns `notes_moved`, `mean_delta_ticks`, `max_delta_ticks`, plus all input options echoed back. |
| `analyze_tension_curve` | v0.287.0 | Bar-level tension diagnostic built from existing analyzers. Per-bar rows carry `harmonic_tension` (mean per-chord tension from `harmonic_function` over chord windows that touch the bar), `dissonance` (fraction of in-bar chord-window-ticks out of key), `density_score` (note count / 16, clamped), `register_score` ((mean MIDI − 36) / 60, clamped), `rhythmic_activity` (distinct 16th-note onset cells / 16), `mean_velocity`, and (in audio mode) `loudness_score` (LUFS-M mapped from `[-50, -10]` dB → `[0, 1]`, RMS fallback for very short bars), `brightness` ((mid + high) / total energy), `band_entropy` (Shannon entropy across the 4 fixed bands / ln 4), `stereo_width_score`. Composite blend: 35 % harmonic + 15 % dissonance + 20 % density + 10 % register + 20 % rhythm in symbolic mode; audio-augmented blend keeps 60 % of the symbolic score and adds 20 % loudness + 12 % brightness + 8 % entropy. Returns the cluster-derived `sections` from `analyze_arrangement` so callers can map bars to A/B/A'. Warnings cover chorus-doesn't-lift (reprise > 0.10 below its first appearance), build-peaks-too-early (section peak bar before midpoint and last bar > 0.10 below peak), drop-loses-low-end (section-start bar's sub-band energy falls > 50 % vs. the prior bar at high tension), monotone-tension (std-dev < 0.05 across 4+ bars). `include_audio` defaults to true in arrangement scope and false in pattern scope; audio mode does one offline render and slices the buffer per bar. Implementation in `crates/pertylizer/src/analysis/tension_curve.rs`; bridge in `analyze_tension_curve_impl`. Closes Tier-2 #14. |
| `suggest_music_fixes` | v0.287.0 | Meta-analysis tool. The bridge runs harmony, harmonic-function, mix-bus, masking, drum-groove, bass-drum-lock, form, hook, and tension-curve analyzers for the requested scope, hands the results into a category-grouped rule engine, sorts suggestions by descending severity, and truncates to `max_suggestions` (default 15, clamped to `[1, 50]`). `categories` filters to a subset of `harmony` / `mix` / `groove` / `arrangement` / `composition` / `patch` — empty/null runs everything. `include_audio` (default true) gates the mix-bus, masking, and audio-augmented tension-curve checks; false skips the offline renders for a faster symbolic-only pass. Each `FixSuggestion` carries a stable rule `id` (`harmony.no_key_inferred`, `mix.heavy_masking_pair`, `groove.weak_backbeat`, …) so callers can suppress specific rules in a follow-up call without parsing titles, plus `severity ∈ [0, 1]`, a short `title`, a multi-sentence `detail`, and an `evidence` list referencing the supporting measurements. `rules_clean` lists rule IDs that ran but found nothing — confirms what was checked. No new measurements: each rule reads exactly one underlying analyzer's output. Implementation in `crates/pertylizer/src/analysis/suggest_fixes.rs`; bridge in `suggest_music_fixes_impl`. Closes Tier-2 #16. |

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
7. ✅ **`analyze_drum_groove`** — shipped 2026-05-16. Drum-specific feel analysis is more actionable than a generic groove score: backbeat strength, hat subdivision, fills, ghost notes, and repeated-bar sameness — the exact dimensions that make AI-written beats sound flat. Pure symbolic, built on the instrument-profile inference.
8. ✅ **`analyze_bass_drum_lock`** — shipped 2026-05-16. The kick/bass relationship carries a large share of perceived groove and low-end clarity. Gives concrete answers to "does the bass actually work with the beat?" via onset alignment with a configurable tolerance, plus a bass-pitch-stability summary on matched onsets.
9. ✅ **`analyze_masking_matrix`** — shipped 2026-05-16. Pairwise per-track spectral overlap on top of the
   existing per-track soloed renders: each pair carries the 4-band overlap energy + dB dominance, an overall
   `conflict_score ∈ [0, 1]`, a `dominant_track_id` when one side leads the worst-overlap band by ≥ 6 dB, and
   a textual hint such as `"Pad(2) masks Lead(3) in mid (500-2000 Hz)"`. Pairs sorted by descending conflict
   score so the most contested combination is index 0. No extra audio renders beyond the per-track set —
   pair matrix is computed in-memory and O(N²). Implementation: helpers + `analyze_masking_matrix_impl` in
   `crates/pertylizer/src/mcp_bridge.rs`; types in `crates/synth_mcp/src/types.rs`; bridge trait + server
   handler in `crates/synth_mcp/src/{bridge,server}.rs`. Closes Tier-1 #9.
10. ✅ **`analyze_harmonic_function`** — shipped 2026-05-16. Roman numerals + simplified Riemannian function buckets + per-chord tension + cadence detection (authentic/plagal/half/deceptive). Built on `analyze_harmony` so chord ID + key inference + drum exclusion stay in lock-step.
11. ✅ **`analyze_instrument_range`** — shipped in v0.284.0. Sweeps an instrument across a MIDI range, runs the
   existing `analyze_note` path per step, and aggregates per-step entries into a cross-step `issues` summary
   (`silent_notes`, `aliased_notes`, `pitch_lost_notes`, `clipping_notes`, `level_spread_db`). One render per step,
   `step_semitones` defaults to 12 — keeps cost manageable on full-keyboard sweeps. Implementation +
   `analyze_velocity_response` share `crates/pertylizer/src/analysis/patch_sweep.rs`.
12. **`render_section_to_wav`** — Even without immediate analysis tools layered on top, this unlocks the AI sending
   audio back to a human or feeding it to a separate model. Building block under `compare_to_reference`.
13. ✅ **True-peak + LUFS-S/M for `analyze_mix_bus`** — shipped 2026-05-16 alongside `pre_master_peak`. See §0 deferred section.

### Tier 2 — Quality-of-life and composition

14. ✅ **`analyze_tension_curve`** — shipped in v0.287.0 as Group D from §8.6. Per-bar rows over harmonic
    tension + dissonance + density + register + rhythmic activity + (audio mode) loudness/brightness/band-
    entropy/stereo-width. Section labels from `analyze_arrangement`. Warnings: chorus-doesn't-lift,
    build-peaks-too-early, drop-loses-low-end, monotone-tension. `include_audio` defaults to true in
    arrangement scope.
15. ✅ **`find_motifs`** + **`analyze_hook_strength`** — shipped in v0.286.0 as Group C from §8.6. Pitch-interval
    n-gram motif search (transposition-invariant, lengths 3..=6 by default, hard cap 12); hook score blends
    longest motif length, repeat count, and coverage ratio.
16. ✅ **`suggest_music_fixes`** — shipped in v0.287.0 as Group D from §8.6. Runs harmony, harmonic-function,
    mix-bus, masking, drum-groove, bass-drum-lock, form, hook, and tension-curve analyzers for the scope,
    feeds them into a category-grouped rule engine, returns ranked suggestions with stable rule IDs.
    Categories filterable; `rules_clean` reports rules that found nothing.
17. ✅ **`generate_chord`**, **`transpose_notes`**, **`quantize_notes_to_scale`**, **`quantize_notes_to_grid`** —
    Symbolic helpers that turn a 20-tool-call sequence into a 1-tool-call sequence. Not unlocking any new
    capability, but a large reduction in token-cost-per-musical-idea. **Shipped 2026-05-17 (v0.285.0) as
    Group B from §8.6.**
18. **`analyze_groove`** — Useful once the AI is past "write the right notes" and into "make it feel good".
    Less load-bearing than harmony analysis because timing problems are easier for humans to flag than harmonic ones.
    The new `analyze_drum_groove` should probably land first because it has clearer instrument semantics.
19. ✅ **`analyze_velocity_response`** — shipped in v0.284.0 alongside `analyze_instrument_range` as Group A from
    §8.6. Holds one MIDI note and sweeps velocity. Returns per-velocity amplitude/centroid plus
    `amplitude_range_db`, `non_monotonic_amplitude_steps`, `non_monotonic_centroid_steps`, and a
    `velocity_unresponsive` flag (< 3 dB spread across the sweep).
20. ✅ **`analyze_arrangement`** + **`analyze_form_map`** — shipped in v0.286.0 as Group C from §8.6. Per-bar
    feature row + cosine self-similarity + adjacent-merge clustering with first-appearance section labels
    (primes mark soft matches). `analyze_form_map` adds the run-length-compressed form string.

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
7. ✅ `analyze_drum_groove` — shipped 2026-05-16. Pure symbolic drum-feel diagnostics built on `infer_all_profiles`. Reports backbeat strength, hat subdivision, ghost notes, fill candidates, bar repetition; classifies hits via the General MIDI drum map (kick/snare/hat/tom/cymbal/clap/other).
8. ✅ `analyze_bass_drum_lock` — shipped 2026-05-16. Symbolic kick/bass-onset alignment with configurable tolerance (default ±1/32-note). Returns lock_score / coverage_score / kick-only / bass-only counts plus bass-pitch stability (most common PC on matched onsets).
9. ✅ `analyze_masking_matrix` — shipped 2026-05-16. Pairwise per-track spectral masking on top of the
   parallel per-track solo renders. No extra audio renders beyond the per-track set; pair matrix is O(N²)
   and computed in-memory from `MixBusMetrics.energy_bands`. Produces per-pair textual hints when one side
   dominates by > 6 dB on the worst-overlap band.
10. ✅ `analyze_harmonic_function` — shipped 2026-05-16. Roman numerals + Tonic/Subdominant/Dominant/Other/Chromatic function buckets + per-chord tension (T 0.0 / S 0.3 / D 0.7, +0.15 for dom7, +0.2 for dim/half-dim) + cadence detection (authentic/plagal/half/deceptive) on top of `analyze_harmony`.
11. ✅ `analyze_instrument_range` — shipped 2026-05-16 (v0.284.0). Sweep on top of `analyze_note`.
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

### 7.2 Parallelize the per-track renders with rayon — ✅ shipped 2026-05-16

The N renders in `render_per_track_contributions` are independent (each its own engine + soloed song clone, no
shared mutable state). Now run on rayon's thread pool via `par_iter` over `targets`. Each worker builds its own
`OfflineEngineSession` and per-target `Song` clone (with the target's solo flag set up-front via
`Song::set_solo_only` — see §7.4), so workers never share mutable state.

**Determinism preserved.** The §8.1 Round-2 fixes (`BTreeMap` ordering in `synth_engine::graph` + `fastrand`
reseed per `render_range`) make N independent sessions produce bit-exact output equal to the prior serial
path. Verified by `analyze_section_per_track_is_bit_exact_across_calls` (new) — two back-to-back
`analyze_section` calls with `include_per_track = true` return bit-exact peak / RMS / LUFS / pre_master_peak /
clipped_samples for every track.

**Setup-warning capture.** Engine-level setup warnings (per-instrument load failures, etc.) are identical for
every worker because they describe the live session state, not the target solo flag. They're captured once
via `OnceLock` and emitted alongside the result — no duplicate warnings, no synchronization cost beyond a
single atomic compare-and-swap.

**Trade-off accepted.** §7.1's "build session once, reuse for N renders" win is partly undone here: each
worker builds its own session per target (no within-worker reuse). With N targets on K cores you pay N setup
costs total instead of 1, but render time dominates setup for any non-trivial section, so the parallel
speedup more than compensates. If profiling on big sections shows setup-cost dominance, switch to
`par_iter().map_init` for per-worker session reuse.

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

### 7.4 `Song::tracks_mut` / `Song::set_solo_only` helpers in `synth_sequencer` — ✅ shipped 2026-05-16

Two new helpers on `Song` (`crates/synth_sequencer/src/song.rs`):

- `pub fn tracks_mut(&mut self) -> impl Iterator<Item = &mut SequencerTrack>` — mutable iteration in
  `BTreeMap` (TrackId) order. Display-order mutation goes through `track_order()` + `track_mut(id)` to
  avoid the borrow conflict that prevents borrowing `track_order` while holding a mutable borrow of
  `tracks`.
- `pub fn set_solo_only(&mut self, target: TrackId)` — atomic "make this the only soloed track". Sets
  `solo = true` on `target` and `false` on every other track in one pass. No-op for `target` if it doesn't
  exist; other tracks are still cleared.

The per-track contribution loop's clear-then-flip dance collapsed into a single `song.set_solo_only(target)`
per iteration. Both helpers also unblock §7.2 (parallel renders): each worker `set_solo_only`s its own song
clone exactly once, so workers never share mutable state. Three unit tests in `song::tests` cover both
helpers and the unknown-target edge case.

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

### 7.7 Engine reuse across patch-sweep steps — deferred from v0.284.0

`analyze_instrument_range_impl` and `analyze_velocity_response_impl`
(`crates/pertylizer/src/mcp_bridge.rs`) call `analyze_rendered_note` once per
swept value. Each call goes through `audio::preview::render_note_to_buffer`,
which spins up a brand-new offline `SynthEngine` and reloads the instrument's
module graph + sample data. For a 60-note semitone-step sweep that's 60 fresh
engines; for the default 8-step velocity sweep that's 8. Same shape as the
per-track contribution path before §7.1 introduced `OfflineEngineSession`.

The win is real (engine setup dominates the wall-clock cost for short notes),
but the change is sizable — needs an `OfflineNoteSession`-style wrapper that
takes a `SynthSession` + `SharedSampleLibrary` + `InstrumentId` at
construction, builds the engine + loads the patch + samples once, then exposes
`render(note, velocity, duration_ms, tail_ms) -> RenderedNote` for the N calls.
Voice-bleed drain between renders (same problem as §7.1) needs to be
reproduced. Determinism tests would mirror
`tests/arrangement_render_determinism.rs::session_render_range_is_bit_exact_across_three_calls`.

Until that lands, sweeps pay the full engine build N times. After §7.1 shipped
for arrangement renders, rayon parallelism followed in §7.2 — same sequence
applies here: ship session-reuse first, then `par_iter` over the sweep target
vector to add a 2-4× speedup on top.

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

### 8.6 Next-session priorities (added 2026-05-16)

With every §8 item closed and Tier-1 items 4–10 plus 13 shipped, the next concrete work splits into natural
groups. Order chosen to maximize shared infrastructure per PR and to land the highest-leverage items first.

**Group A — Patch-QA sweeps (Tier-1 #11 + Tier-2 #19)** — ✅ shipped 2026-05-16 (v0.284.0)

`analyze_instrument_range` and `analyze_velocity_response` are both `analyze_note`-driven sweeps with the same
plumbing: loop over a parameter (MIDI note or velocity), call the existing single-note render path per step,
collect per-step metrics, return a curve plus stability flags. Shared result-struct shape (per-step entries +
overall warnings), shared aliasing/non-monotonic detection helpers, shared "render N notes against one
instrument" infrastructure. Landed in `crates/pertylizer/src/analysis/patch_sweep.rs` with the per-step
extraction + cross-step issue detection as pure helpers; the render loops live in the impl functions in
`crates/pertylizer/src/mcp_bridge.rs`. 10 in-module unit tests + 5 integration tests against a real
`SynthEngine` and a sustaining saw patch.

**Group B — Symbolic composition helpers (Tier-2 #17 — four tools)** — ✅ shipped 2026-05-17 (v0.285.0)

`generate_chord`, `transpose_notes`, `quantize_notes_to_scale`, `quantize_notes_to_grid`. All purely symbolic,
no audio render, share a scale/interval theory module (chord templates, scale-degree tables, voicing rules —
landed in `crates/pertylizer/src/composition/`, reusing `crate::harmony::{CHORD_TEMPLATES, SCALES,
scale_by_name}` so identification and generation stay round-trip stable). Big token-cost-per-musical-idea
reduction for the AI agent — turns 20-call note-edit sequences into 1-call operations. 28 in-module unit
tests + 10 integration tests through the bridge.

**Group C — Form & motifs (Tier-2 #15 + #20)** — ✅ shipped 2026-05-17 (v0.286.0)

`find_motifs` / `analyze_hook_strength` and `analyze_arrangement` / `analyze_form_map` shipped together,
sharing the bar-level feature matrix (`analysis/bar_features.rs` — duration-weighted pitch-class
histogram + density + active-track set per bar, plus cosine self-similarity and adjacent-merge section
clustering with prime-label soft matches). Form tools live in `analysis/form.rs` (section summaries +
run-length-compressed form string); motif tools live in `analysis/motifs.rs` (pitch-interval n-grams,
transposition-invariant by construction, prefix-suppression so the longest informative motif wins).
`analyze_hook_strength` reduces the motif catalogue to a single `[0, 1]` score blending longest-motif
length, repeat count, and coverage ratio. All four tools take optional pattern_id (pattern scope) or
arrangement_start/end_tick (arrangement scope), reusing the harmony tools' resolve_arrangement_range +
HarmonyScope. `exclude_drums` defaults to true via `infer_all_profiles`. 17 in-module unit tests + 5
bridge integration tests in `tests/form_motifs_integration.rs`.

**Group D — Meta-analysis (Tier-2 #14 + #16)** — ✅ shipped 2026-05-17 (v0.287.0)

`analyze_tension_curve` and `suggest_music_fixes` shipped together. Tension curve lives in
`crates/pertylizer/src/analysis/tension_curve.rs` as a pure synthesis layer: takes bar features +
chord-tension spans + optional per-bar `BarAudio`, returns per-bar `composite_tension` + summary +
warnings. Bridge runs harmony + harmonic_function to get chord-tension spans and (when
`include_audio = true` — default in arrangement scope) does one offline render and slices the buffer
per bar via `analyze_mix_buffer`. `suggest_music_fixes` lives in
`crates/pertylizer/src/analysis/suggest_fixes.rs` as a category-grouped rule engine over
`SuggestionInputs { harmony, mix_bus, masking, drum_groove, bass_drum_lock, form_map, hook,
tension_curve }`; the bridge populates each slot by calling the corresponding `*_impl` for the
scope, then aggregates. 21 in-module unit tests (9 tension_curve + 6 suggest_fixes + 6 integration
suite) + 6 bridge integration tests in `tests/group_d_integration.rs`.

**Standalone / lower priority**

- **Tier-1 #12 `render_section_to_wav`** — wait until the reference-comparison workflow comes up. It's the
  prerequisite for Tier-3 `compare_to_reference`, so pick up when that becomes relevant rather than
  speculatively.
- **Tier-2 #18 `analyze_groove`** — the drum-specific `analyze_drum_groove` already covers the highest-value
  groove diagnostic. Generic-track groove is lower urgency.

**Recommended landing order**

1. ✅ **Group A** — smallest scope, clear design path via `analyze_note`, catches a concrete real bug class
   (patches that work at C4 and fall apart at C6 or C2). One PR. **Shipped 2026-05-16 (v0.284.0).**
2. ✅ **Group B** — low risk, high token-saving impact for the agent. One PR. **Shipped 2026-05-17 (v0.285.0).**
3. ✅ **Group C** — auto-detect section boundaries from self-similarity, pitch-interval n-gram motifs.
   One PR. **Shipped 2026-05-17 (v0.286.0).**
4. ✅ **Group D** — tension curve + suggest_music_fixes. One PR. **Shipped 2026-05-17 (v0.287.0).**

### 8.7 Cross-reference

§8.1 is fully fixed (Round 1 + Round 2); §4's "deterministic output for a given project state" claim holds
bit-exact end-to-end through the MCP bridge, not just inside the determinism unit tests. §8.2 (auto-inferred
profiles) shipped in v0.278.0, with §8.2c (round-2 inference improvements) and §8.4 (offline-render
snapshot propagation) landing together in commits 74d18da + 93c0786 — the previously implicit "manual
`set_instrument_category` required" workflow is now fully optional. §8.3 is fully closed: the mandatory
doc fix (pan-law attenuation in per-track peak/RMS) and the optional `pre_master_peak` field both shipped
2026-05-16. All five §8.5 round-3 inference follow-ups (§8.5.1–§8.5.5) shipped 2026-05-16. §8.5.6.1
(Plucked-Mono-Lead misclassification) and §8.5.6.2 (0.60-threshold margin for name-conflict drums) — both
surfaced by the 2026-05-16 end-to-end verification on the "Neuro F#m 174" project — also shipped 2026-05-16.
Every §8 item is now closed; Tier-1 items 6 (`analyze_pattern`), 7 (`analyze_drum_groove`), 8
(`analyze_bass_drum_lock`), 9 (`analyze_masking_matrix`), and 10 (`analyze_harmonic_function`) all
shipped 2026-05-16 — alongside §7.2 (rayon-parallel per-track renders) and §7.4 (`Song::tracks_mut`
+ `set_solo_only` helpers). Tier-1 item 11 (`analyze_instrument_range`) + Tier-2 #19
(`analyze_velocity_response`) — Group A from §8.6 — shipped 2026-05-16 (v0.284.0). Tier-2 #17 — Group B
from §8.6, four symbolic composition helpers (`generate_chord`, `transpose_notes`,
`quantize_notes_to_scale`, `quantize_notes_to_grid`) — shipped 2026-05-17 (v0.285.0). Tier-1 item 12
(`render_section_to_wav`) is the only remaining Tier-1 work — pick up when reference audio comes
into the workflow (it unlocks `compare_to_reference` from Tier 3). Group D from §8.6 (`analyze_
tension_curve` + `suggest_music_fixes`) shipped 2026-05-17 (v0.287.0), closing Tier-2 #14 and #16.
All §8.6 groups (A/B/C/D) are now shipped; remaining Tier-2 work is item #18 `analyze_groove`
(deferred — `analyze_drum_groove` already covers the highest-value case).
