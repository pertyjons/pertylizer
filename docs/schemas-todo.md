# JSON Schema TODO

Tracking improvements to the four schemas under `/schemas/`:

- `project.schema.json` — full project save format
- `patch.schema.json` — single instrument patch
- `awe-preset.schema.json` — AWE preset
- `bundle-metadata.schema.json` — `metadata.json` inside `.zip` bundles

Schemas are auto-generated from Rust types via `schemars` derive; run
`cargo run -p pertylizer --bin gen_schemas` to regenerate.

Items below are grouped by category and ranked by priority within each group.
Each item lists effort (S/M/L), impact, and a recommendation.

## Status — 2026-05-21

**Completed:** A1, A2, B4, B5, B6, C1, D1, D2, F1 (Do-first block + polish α).

**Open:** B1, B2, B3, B7, C2, E1, E2.

See [Next iteration](#next-iteration) at the bottom for the recommended
next block of work.

---

## A. File-type identification

### ✅ A1. `file_type` should be a `const` string [S, high impact] — done 2026-05-21

Post-processing in `gen_schemas.rs::tighten_file_type` now stamps
`{"const": "project"}` / `{"const": "awe_preset"}` on the relevant schemas.

**Issue:** `ProjectFile.file_type` and `AwePresetFile.file_type` are typed as
plain `string` — any value passes validation. The schema doesn't enforce that
project files contain `"project"` or AWE preset files contain `"awe_preset"`.

**Impact:** Schema can't actually be used as a file-type discriminator.
Tooling that picks a schema based on `file_type` must duplicate the constants
externally.

**Fix:** Post-process the generated schemas in `gen_schemas.rs` to replace
the generic `string` with `{"const": "project"}` / `{"const": "awe_preset"}`.

**Recommendation:** Do.

### ✅ A2. Bundle schema doesn't document the ZIP layout [S, medium impact] — done 2026-05-21

`schemas/bundle.md` next to the schema files documents the ZIP archive
structure, sample naming convention, WAV format, and detection logic.

**Issue:** `bundle-metadata.schema.json` describes only the inner
`metadata.json` blob. A reader has no way to know the `.zip` must also contain
`project.json` and `samples/*.wav`, or that sample filenames must match
`samples/<sample_id>.wav`.

**Impact:** Anyone trying to author a bundle from scratch using only the
schema will produce something that fails to load.

**Fix:** Add a `$comment` block at the schema root describing the ZIP
structure, or split into a separate `bundle.md` next to the schemas.

**Recommendation:** Do. Lean toward an adjacent `bundle.md` since schema
comments aren't visible in most validation tooling.

---

## B. Module / parameter validation tightening

### ✅ B1. Parameter ranges (semantic vs normalized storage) [L, high impact] — done 2026-05-21

Resolved by introducing dedicated semantic newtypes (`BitDepth`,
`TimeScale`, `PulseWidth`, `DecayTrim`) in `synth_core::types` and
wrapping the four misused Param variants in them instead of
`NormalizedValue`. Each newtype's `as_f32()` returns the semantic
value directly, so the descriptor range matches what's saved. The
remaining open items in the migration block (B3, B7, plus optional
C2/E1/E2) carry on as their own work.

**Issue:** `ParameterDescriptor.range` reports semantic ranges (e.g.
`bit_depth: 1–16`), but some parameters are persisted as engine-internal
normalized 0..1 floats (`bit_depth: 0.466` ≈ 8 bits). The schema therefore
omits `minimum`/`maximum` entirely on numeric params — we couldn't trust
either form.

**Impact:** No numeric range validation. Out-of-range or NaN values pass.
JSON files are also less human-readable because of the normalized values.

**Fix:** Migrate the save format to always write semantic values. Adds
`Param::to_display_f32` / `from_display_f32`, version-bumps the save format
to "1.1", and migrates legacy files on load.

**Recommendation:** Do, as a separate dedicated PR. This is the root cause
for several other items below (B3, the JSON-readability problem). See the
schema/format discussion notes from 2026-05-21.

### B2. Parameter name typos aren't caught [M, medium impact]

**Issue:** Each module branch uses
`parameters.additionalProperties: {"$ref": "#/$defs/ParamValue"}` as a
forward-compatibility escape hatch. A typo like `"freq": 440` passes
validation instead of erroring as an unknown parameter.

**Impact:** Schema doesn't help authors catch typos. Easy to ship broken
patches that silently ignore parameters.

**Fix:** Two options:
- Set `additionalProperties: false` on `parameters`. Strict, but breaks
  forward-compat: any future parameter added to a module rejects old
  schemas.
- Keep the escape hatch but mirror known param names in a `propertyNames`
  pattern so unknown names get a warning, not an error. Requires a custom
  validator.

**Recommendation:** Do, with strict `additionalProperties: false`. Forward-
compat is handled by re-generating the schema with new module versions.
Authors who hit a stricter schema can run `gen_schemas` to update.

### B3. Numeric enums accept both string and number [M, medium impact] *(consumer feedback)*

**Issue:** Choice parameters (e.g. `oscillator.waveform`, `filter.type`,
`distortion.type`) are stored sometimes as strings (`"sawtooth"`) and
sometimes as numbers (`2.0` ≈ index into the choice list). The schema
accepts both via `anyOf: [string-enum, number]` but doesn't say which is
canonical or how the numeric mapping is defined.

**Impact:** Authors don't know which form to use. Tooling can't tell whether
`2` and `"sawtooth"` are interchangeable for diff purposes.

**Fix:** Same path as B1 — on the save side, always serialize choices as
their string ID via a dedicated `Param::to_display`/`from_display` so the
JSON only ever contains strings. Then the schema becomes `enum: [...]` of
strings only.

**Recommendation:** Do, bundled with B1. Both are fixed by the same
serialization migration.

### ✅ B4. `sample_id` object form accepted on every numeric parameter [S, low impact] — done 2026-05-21

`parameter_schema` in `gen_schemas.rs` now matches on
`Param::Sampler(SamplerParam::SampleSelect(_))` and emits the sample-id
object form only there. Side effect: numeric params lost their `anyOf`
wrapper, cutting overall schema size by ~41%.

**Issue:** The generator adds `{"sample_id": N}` as an alternative shape on
every numeric param, because some modules (sampler) really do store a sample
ID there. As written, you could put `{"sample_id": 42}` on `frequency` and
the schema accepts it.

**Impact:** Cosmetic — the engine would reject it on load. Doesn't actually
allow broken patches because the loader is stricter than the schema.

**Fix:** In `gen_schemas.rs`, inspect `param.id` (the `Param` variant); only
emit the sample-id branch when it wraps `Sampler::SampleSelect`.

**Recommendation:** Do. Small, mechanical, makes the schema honest.

### ✅ B5. `InstrumentId`, `AllocationMode`, `StealingStrategy` substituted [S, low impact] — done 2026-05-21

`schemars` dep added to `synth_engine`; `JsonSchema` derived on all three
types; `with`-substitutions in `pertylizer/src/patch.rs` removed.

**Issue:** Three foreign types from `synth_engine` are bypassed in the
schema via `#[schemars(with = "u64")]` / `with = "String"`. `AllocationMode`
and `StealingStrategy` are enums (Polyphonic/Mono/Legato/Unison;
Oldest/Quietest/etc.) — we lose enum-string validation on them.

**Impact:** The schema accepts any string for these fields instead of the
four-or-so valid variants.

**Fix:** Add `schemars` workspace dep to `synth_engine`'s Cargo.toml and
derive `JsonSchema` on the three types. Remove the `with` substitutions in
pertylizer's `patch.rs`.

**Recommendation:** Do.

### ✅ B6. `Note.instrument` convention not documented [S, medium impact] — done 2026-05-21 *(consumer feedback)*

Doc comment on `Note.instrument` in `synth_sequencer/src/note.rs` flows
through to the schema `description` automatically via `schemars`.

**Issue:** `Note.instrument` is typed as `SeqInstrumentId` (u16) with full
range. In practice, when a track binds the instrument, `Note.instrument`
should always be `0` — the track-level binding wins. The schema gives no
hint that the field is conventionally ignored.

**Impact:** Authors set values here thinking they have effect; they don't.

**Fix:** Either:
- Document the convention in the field's doc comment (which propagates to
  the schema description).
- Make the field `Option<SeqInstrumentId>` with `None` as the documented
  "track-bound" form.

**Recommendation:** Do the doc-comment fix now (S); evaluate the
`Option`-ification later if it keeps causing confusion.

### B7. Pan unit inconsistency between `Track.pan` and module `pan` parameters [M, medium impact] *(consumer feedback)*

**Issue:** `SequencerTrack.pan` uses `NormalizedValue` (0.0=left,
0.5=center, 1.0=right). Module-level pan parameters (e.g. amplifier) use
`BipolarValue` (-1.0=left, 0.0=center, 1.0=right). Same concept, two
conventions. This is a real semantic inconsistency, not just a schema
problem.

**Impact:** Confusion when reading saved files. Tooling that animates pan
needs to know which convention is in effect per field.

**Fix:** Migrate `SequencerTrack.pan` to `BipolarValue`. Behavior change for
saved tracks; migrate legacy files on load.

**Recommendation:** Do as a separate small migration. Versionable via the
same "1.0" → "1.1" bump as B1/B3.

---

## C. Documentation gaps

### ✅ C1. Many module parameters have no `description` [M, high impact] — done 2026-05-21 *(consumer feedback)*

325 `ParameterDescriptor::float`/`choice` call sites audited across 65
files in `synth_modules/` + `synth_engine/visualizers/`. All now have
`.description("...")`. Schema parameter description coverage is now
392/392 (100%).

**Issue:** Parameters like `fm_amt`, `fm_mode`, `x_mod`, `cv_bipolar`,
`morph`, `model`, `key_track`, `env_amt` ship without descriptions in the
descriptor builder. The schema therefore emits them with empty descriptions.
Authors have to guess what they do.

**Impact:** Schema is much less useful as authoritative documentation.
Real-world consumer comment: "the example fields' own description prose is
often more valuable than the schema."

**Fix:** Audit every `ParameterDescriptor::float(...)` and
`ParameterDescriptor::choice(...)` call across `synth_modules` and add
`.description("...")` everywhere it's missing. Include the unit + the range
in human language ("0 = no FM, 1 = full FM").

**Recommendation:** Do. Mechanical, no design risk, biggest single
usability boost.

### C2. Manual `Deserialize` impls accept legacy forms not in the schema [M, low impact]

**Issue:** `Position`, `CanvasSize`, and `Author` have hand-written
`Deserialize` implementations that accept legacy shapes — `Position`
accepts both `{x, y}` and `[x, y]`; `Author` accepts both an object and a
bare string. The schema only describes the new (serialize-output) shape.

**Impact:** Legacy files that load fine would fail schema validation. The
schema describes "what we write," not "what we'll read."

**Fix:** Two options:
- Drop the legacy accept paths (breaks loading of old files unless we
  migrate first).
- Describe the legacy forms in the schema via `oneOf: [new-form,
  legacy-form]`.

**Recommendation:** Decide policy. If we plan to migrate everything to the
new form, drop legacy. Otherwise document.

---

## D. Schema structure / size

### ✅ D1. No `examples` are embedded anywhere [S, high impact] — done 2026-05-21 *(consumer feedback)*

Post-processing in `gen_schemas.rs::embed_examples` adds real-data
`examples` arrays to `Note`, `Pattern`, `RoomShape`, `Material`,
`AweLfoState`, `Author`, `ConnectionState`, plus the `oscillator`,
`filter`, and `reverb` module branches.

**Issue:** JSON Schema supports `examples: [...]` per `$def`. We have 16
real example files we could clip snippets from. Currently a reader has to
open `assets/examples/` to learn idioms.

**Impact:** Big usability win for a low-effort fix.

**Fix:** Pick representative snippets:
- One short oscillator-module block on `ModuleState` `oneOf` branches
- A 2-note pattern on `Pattern`
- A simple 2-track arrangement on `Song`
- A `Sphere` and a `Box` `RoomShape` example on `AweState`

Embed via `gen_schemas.rs` post-processing.

**Recommendation:** Do.

### ✅ D2. Module branches inline shared sub-schemas [M, medium impact] — done 2026-05-21 *(consumer feedback)*

`hoist_repeated_enums` post-processing in `gen_schemas.rs` walks the
generated `ModuleState` tree, detects `{type: string, enum: [...]}`
fragments that appear 3+ times, and hoists them into shared
`$defs.SharedEnumN` entries. The big win was the mod_matrix module —
16 slots × 2 enums (source + destination) — combined with B4 took
total schema size from 27,221 to 14,829 lines (-46%).

**Issue:** Each of the 65+ module branches in `ModuleState.oneOf` is fully
inlined — the same `id` pattern wrapper, the same sample-id fallback, the
same `position: { $ref: ... }`. The schema is currently ~435 KB; with
sensible `$defs` factoring the common scaffolding it could drop to roughly
half.

**Impact:** Bigger downloads, slower tooling, harder to read.

**Fix:** Refactor `gen_schemas.rs::module_branch` to emit
`$defs.ModuleIdField`, `$defs.SampleIdValue`, `$defs.NumericParam`, etc.
References from each branch.

**Recommendation:** Do after B4 (otherwise we'd be deduplicating something
we want to remove).

---

## E. Format / design changes (deeper)

### E1. `Song.patterns` and `Song.tracks` use `Map<u64, T>` with stringified keys [L, high impact] *(consumer feedback)*

**Issue:** Patterns and tracks are stored as JSON objects keyed by ID
strings: `{"0": {...}, "1": {...}}`. This is an unusual choice that:
- Many JSON schema tools handle awkwardly (they expect arrays for
  collections).
- Loses ordering (BTreeMap iterates sorted by key, fine for u64 but not
  obvious from JSON).
- Makes diffs noisy because moving an item changes its key.

The natural representation would be a `Vec<Pattern>` where each `Pattern`
contains its `id` field, with `uniqueItems` constrained on `id`.

**Impact:** Save-format change. Affects every project file. Touches
`Song` serialization plus loaders.

**Fix:** Migrate `Song.patterns: BTreeMap<PatternId, Pattern>` to
`Song.patterns: Vec<Pattern>`; same for tracks. Version-bump the format.
Migrate legacy on load.

**Recommendation:** Discuss before doing. Worth it for tooling
interoperability, but it's a real save-format change. Bundle with B1/B3/B7
migration if we're already breaking the format.

### E2. `row_resolution` lives per-pattern but is effectively per-song [M, low impact] *(consumer feedback)*

**Issue:** Every `Pattern` carries its own `row_resolution: RowResolution`,
even though in practice the value rarely varies across patterns in a song.
Forces duplication of `{rows: 64, ticks_per_row: 240}` across all
patterns.

**Impact:** Noisy JSON; minor.

**Fix:** Move `row_resolution` to `Song`, drop from `Pattern`. Or make it
`Option<RowResolution>` on Pattern that overrides a song-level default.

**Recommendation:** Discuss. Behavior change for patterns that genuinely
differ from song default. Low priority — JSON noise is a real but minor
cost.

---

## F. Tooling / process

### ✅ F1. No CI validation that schemas match example files [S, medium impact] — done 2026-05-21

`crates/pertylizer/tests/schemas_validate_examples.rs` provides two
integration tests:

- `checked_in_schemas_match_generated` — drift detection. Invokes the
  `gen_schemas` binary and byte-compares output against `schemas/`. Fails
  if the developer forgot to regenerate after changing a type.
- `example_files_validate_against_schemas` — correctness. Loads each
  schema with `jsonschema` (Rust crate) and validates every example
  under `assets/examples/`. Catches the case where the schema is current
  with code but doesn't match existing saved files.

**Issue:** Schemas can drift from example files between commits and we
won't notice until someone runs the validator by hand. The
`project_io_round_trip` test validates serde round-trip but not schema
validity.

**Impact:** Schema can silently desync from the actual save format.

**Fix:** Add an integration test
(`crates/pertylizer/tests/schemas_validate_examples.rs`) that:
1. Runs the schema generator into a temp dir.
2. Loads each schema with `jsonschema` (Rust crate) or similar.
3. Validates each example file against the matching schema.

**Recommendation:** Do. One of the highest-ROI items — keeps the rest of
this TODO from regressing.

---

## Priority summary

**Done (2026-05-21):**

Do-first block:
- ✅ A1: `file_type` const
- ✅ B5: Schemars on `synth_engine` types
- ✅ B6: Document `Note.instrument` convention
- ✅ C1: Add descriptions to all parameter descriptors
- ✅ D1: Embed examples in schemas
- ✅ F1: CI validation test

Polish batch α:
- ✅ A2: Bundle ZIP layout doc
- ✅ B4: `sample_id` only on Sampler params
- ✅ D2: Factor shared sub-schemas into `$defs` (−46% total size)

**Do as a single save-format migration ("1.0" → "1.1"):**

- B1: Semantic numeric ranges (the bit_depth issue)
- B3: Choice values always string in JSON
- B7: `Track.pan` to `BipolarValue`
- *(Possibly bundle E1, E2 if going there.)*

**Discuss before doing:**

- B2: Strict `additionalProperties: false` on `parameters`
- C2: Legacy `Deserialize` form policy
- E1: `Map<u64, T>` → `Vec<T>` for patterns/tracks
- E2: `row_resolution` scope

---

## Handover — pick up here (next session)

B1 landed in commit `5532ab1` via dedicated semantic newtypes
(`BitDepth`, `TimeScale`, `PulseWidth`, `DecayTrim`) — *not* through
descriptor-aware display↔internal conversion. The β1/β2 architectural
debate is moot. The 4 misused `NormalizedValue`-wrapping params now
wrap their own semantic types; engine-side remap math is gone; JSON
saves the actual semantic value.

Remaining open: **B3, B7, C2, E1, E2, B2** plus task #10
(re-enable `minimum`/`maximum` in `gen_schemas.rs::parameter_schema`).
They can ship as one bundled `"1.0" → "1.1"` version-bump or split
several ways. The user previously asked for one commit; assessed
mid-session as too large for a single sitting, so was paused after B1.

### What's already in place

- Generator + four schemas under `schemas/`, regenerated via
  `cargo run -p pertylizer --bin gen_schemas`.
- Drift + correctness tests in
  `crates/pertylizer/tests/schemas_validate_examples.rs`.
- `project_load_snapshot` test (`crates/pertylizer/tests/`) snapshots
  the parsed structure of every example project — expect this to
  fail after E1/E2/B7 land and need regenerated fixtures.
- `project_io_round_trip` test verifies serde round-trip — must keep
  passing for **both** v1.0 (legacy) and v1.1 (new) files after the
  migration.
- `crates/synth_core/src/types/module_params.rs` — the 4 new semantic
  newtypes, ready to be extended if more misuse cases surface.

### Recommended order — bundle these as one "1.0" → "1.1" bump

1. **B3 — choice strings only.** In
   `crates/pertylizer/src/patch.rs`:
   - Thread `desc: &ParameterDescriptor` into
     `ParamValue::from_param(p, desc)`.
   - If `desc.choices.is_some()`, look up the option by
     `p.as_f32() as usize` and emit `ParamValue::Choice(id_string)`.
   - 5 call sites already have the descriptor available
     (project_apply.rs, session.rs, export.rs, patch_bridge.rs ×2);
     thread it through. `to_param(desc)` already accepts both forms.
2. **B7 — Track.pan to BipolarValue.** In
   `crates/synth_sequencer/src/track.rs:32`: change
   `pan: NormalizedValue` → `pan: BipolarValue`. Update:
   - `track.rs:52, :76, :212, :226` (CENTER, builder, tests).
   - `gui/sequencer/mod.rs:1418, :1438` (drop the `*2-1` conversion).
   - `mcp_bridge.rs:2282` (drop the `(pan+1)*0.5` normalization).
   - v1.0 load-side migration: remap `[0,1]` → `[-1,1]`.
3. **E1 — Map → Vec for `Song.patterns` / `Song.tracks`.** Confirmed
   safe; nothing depends on BTreeMap-specific behavior, and the
   existing parallel `track_order: Vec<TrackId>` goes away entirely.
   ~14 `Song` methods rewrite (`pattern(id)` → `iter().find(|p| p.id
   == id)`, etc.) plus ~60 external call sites in `mcp_bridge.rs`,
   `gui/sequencer/mod.rs`, `synth_mcp/server.rs`. Most return
   `impl Iterator` already so signatures don't change. Keep
   `next_pattern_id` / `next_track_id`.
4. **E2 — row_resolution to Song.** Move
   `Pattern.row_resolution` (`pattern.rs:115`) to
   `Song.row_resolution`. The 4 pattern methods that use it
   (`notes_at_row`, `quantize_notes`,
   `quantize_notes_with_strength`, plus `:223/:224/:273/:283`) get
   a `resolution: &RowResolution` arg. Callers fetch from the song.
5. **C2 — drop legacy Deserialize.** In `patch.rs`:
   - Drop manual impls at `:68` (Author), `:176` (Position),
     `:497` (CanvasSize). Replace with `#[derive(Deserialize)]`.
   - Migrate 2 example files with bare-string authors:
     `Sidechain Demo.json`, `Escape from Stockholm.json` →
     `{ "name": "..." }`. v1.0 load gate does the same.
6. **B2 — strict `additionalProperties: false`.** In
   `gen_schemas.rs::module_branch` around line 416, flip
   `"additionalProperties": { "$ref": "#/$defs/ParamValue" }` →
   `false`. `param_props` already enumerates all known param
   `type_id`s. If any example has stray param names the
   `example_files_validate_against_schemas` test will surface them.
7. **Re-enable min/max in `gen_schemas::parameter_schema`** (lines
   ~477–485). The `as_f32` contract is now semantic so
   `desc.range.min`/`max` can land in JSON directly.
8. **Version bump.** `default_version()` in `patch.rs` from `"1.0"`
   → `"1.1"`; add `const FORMAT_VERSION: &str = "1.1"`.
9. **Load-side v1.0 → v1.1 migration.** In
   `ProjectFile::load`, branch on `version`. For `"1.0"`:
   - The 4 misused-param backfill: detect the four `type_id`s
     (`bit_depth`, `time_scale`, `pulse_width`, `decay_trim`) and
     remap normalized-form values via the previous formulas:
     `bit_depth: 1.0 + v*15.0`, `time_scale: piecewise` (see
     `mseg.rs` history before commit `5532ab1`),
     `pulse_width: clamp to [0.01, 0.99]`,
     `decay_trim: clamp to [0.1, 1.0]`.
   - Choice params: convert numeric indices to id strings.
   - `Track.pan`: `[0,1]` → `[-1,1]`.
   - `Song.patterns`/`tracks`: object-with-string-keys → array
     sorted by id.
   - `Pattern.row_resolution` → `Song.row_resolution` (take from
     first pattern; warn if patterns differ).
   - Author bare-string → `{ "name": "..." }`.
   - Position `[x,y]` → `{x,y}`; CanvasSize `[w,h]` →
     `{width,height}`.
   - Stamp `"version": "1.1"` so re-save emits the new form.
10. **Regenerate fixtures / examples.** Re-save each example
    project (load v1.0, save v1.1); `INSTA_UPDATE=always cargo
    test snapshot` to update `project_load_snapshot` fixtures.
    `cargo run -p pertylizer --bin gen_schemas` to regenerate
    schemas with min/max + strict additionalProperties.
11. **Update schemas-todo.md.** Mark B2, B3, B7, C2, E1, E2 done.

### Tests to run as you go

```bash
# Per-step sanity:
cargo build
cargo clippy --all-targets

# Round-trip and format integrity:
cargo test -p pertylizer round_trip
cargo test -p pertylizer snapshot

# Schema consistency:
cargo test -p pertylizer --test schemas_validate_examples

# Full suite at the end:
cargo fmt --check && cargo test
```

### Known gotchas

- **MCP tools** (`crates/synth_mcp/`) currently emit and accept
  values in whatever form `Param::as_f32` produces. After B1 most
  params already match the new semantic JSON form — but
  double-check the 4 newly retyped params are consistent in MCP
  tool descriptions.
- **`InstrumentState`'s manual `Deserialize`** in
  `crates/pertylizer/src/patch.rs` accepts default values for many
  fields. The migration shouldn't break this — but check.
- **Bundle ZIPs** carry an internal `project.json` that's just a
  ProjectFile, so they migrate transparently as long as the
  ProjectFile loader handles version gating.
- **E1's `track_order` removal** also drops the
  `repair_track_order` helper and the `reorder_track` mutation
  logic — `Song.tracks: Vec<_>` natural order IS display order.
- **E2 behavior change**: patterns that genuinely set a non-default
  `row_resolution` lose that override. None in the current example
  set, but flag if found in the user's local saves.

---

## Priority summary (active)

**Save-format migration ("1.0" → "1.1") — next session:**

- B3: Choice values always string in JSON
- B7: `Track.pan` to `BipolarValue`
- C2: Drop legacy `Deserialize` paths (Position, CanvasSize, Author)
- E1: `Map<u64, T>` → `Vec<T>` for patterns/tracks
- E2: `row_resolution` Song-level (no per-pattern override)
- B2: Strict `additionalProperties: false` on `parameters`
- Schema re-enable: `minimum`/`maximum` from semantic ranges
