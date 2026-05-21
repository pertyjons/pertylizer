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

## Handover — start the save-format migration here (next session)

The next planned chunk is **option β**: the save-format migration that
bundles **B1 + B3 + B7** (and optionally **C2**, **E1**, **E2**) into a
single `"version": "1.0"` → `"1.1"` bump. This section is written so
the next session can pick up cold.

### What's already in place

- Generator + four schemas under `schemas/`, regenerated via
  `cargo run -p pertylizer --bin gen_schemas`.
- Drift + correctness tests in
  `crates/pertylizer/tests/schemas_validate_examples.rs` — these will
  catch any schema/format mismatch automatically.
- `project_load_snapshot` test (`crates/pertylizer/tests/`) snapshots
  the parsed structure of every example project; **expect this to
  fail after the migration and need regenerated fixtures.**
- `project_io_round_trip` test verifies serde round-trip — must keep
  passing for **both** v1.0 (legacy) and v1.1 (new) files after the
  migration.

### Start here

1. Read this file's sections **B1, B3, B7** plus the optional **C2,
   E1, E2**.
2. Scan `crates/synth_core/src/params/mod.rs` for the full `Param`
   enum — note which variants wrap `NormalizedValue` /
   `BipolarValue` (those need denormalization on save) vs which wrap
   semantic newtypes like `Hertz` / `Gain` / `Bpm` (already match
   descriptor range, identity transform).
3. Inspect `ParameterDescriptor.range` and `.response_curve` in
   `crates/synth_core/src/module_traits.rs` — the curve matters for
   non-linear params; a linear remap won't faithfully round-trip
   logarithmic frequency.
4. Pick one of the two migration shapes (see "Architecture choice"
   below) and confirm with the user before writing code.

### Architecture choice — confirm before coding

Two approaches; both work, tradeoffs differ:

**Option β1 — per-Param semantic methods.** Add
`Param::to_display_f32(&self, desc: &ParameterDescriptor) -> f32` and
`Param::from_display_f32(desc: &ParameterDescriptor, v: f32) -> Self`.
The descriptor's `range` + `response_curve` drives the mapping. Default
impl handles linear cases; per-variant overrides handle non-linear.
Pro: localized changes. Con: needs a giant match over Param.

**Option β2 — value-mapped at the `ParamValue` boundary.** Keep
`Param` as-is internally; do the denormalization in
`ParamValue::from_param` / `to_param` only. The descriptor is the
sole source of truth for the mapping. Pro: no Param changes; only one
serialization layer touched. Con: descriptor must be threaded through
every `from_param` / `to_param` call site.

Recommendation: **β1**. The descriptor's `range` and `response_curve`
are already metadata describing the semantic form; making
`Param::to_display_f32` consult them keeps the contract clean. β2's
threading is intrusive across the codebase.

### Implementation order

1. **Scaffolding.** Add `to_display_f32` / `from_display_f32` on
   `Param` with default linear implementations using
   `ParameterDescriptor::range` + `response_curve`. Keep
   `Param::as_f32` / `with_f32` unchanged — those are the engine's
   internal-form accessors and must stay normalized.
2. **Round-trip test.** Before flipping the save format: write a
   property test that loops over `ALL_MODULE_TYPES`, creates each
   module, sets every parameter to a known semantic value, calls
   `to_display_f32` → `from_display_f32`, asserts the engine's
   internal representation is preserved to within `f32::EPSILON *
   100`. Catches non-linear-curve bugs before they hit real files.
3. **Version constant.** Bump `default_version()` in
   `crates/pertylizer/src/patch.rs` from `"1.0"` to `"1.1"`. Add a
   `const FORMAT_VERSION: &str = "1.1"` somewhere accessible.
4. **Flip saves.** Change `ParamValue::from_param` to call
   `to_display_f32` and pass the descriptor. Change
   `ParamValue::to_param` to call `from_display_f32`.
5. **B3 — choice strings only.** In `from_param`, when the param has
   choices, emit `Choice(<id>)` (string) instead of `Float(<index>)`.
   Loaders already accept both.
6. **B7 — Track.pan to BipolarValue.** Migrate
   `SequencerTrack.pan: NormalizedValue` to `BipolarValue` in
   `synth_sequencer/src/track.rs`. On load, detect v1.0 files and
   remap `0..1` → `-1..1`.
7. **Load-side version gate.** In
   `crates/pertylizer/src/project.rs::ProjectFile::load` (and the
   patch equivalent), branch on the parsed `version` field. For
   `"1.0"`, run a migration step that:
   - Reads numeric params as normalized 0..1, looks up the
     descriptor, denormalizes to semantic.
   - Maps `Track.pan` from `[0, 1]` to `[-1, 1]`.
   - Tags the in-memory representation with `"1.1"` so a re-save
     emits the new form.
   For `"1.1"`, treat values as already semantic.
8. **Regenerate fixtures.** Run the `project_load_snapshot` tests
   with `INSTA_UPDATE=always` (or whatever your snapshot tooling
   uses) and review the diff. Every numeric value will change. Spot-
   check a handful for sanity.
9. **Regen schemas.** Now that descriptor range = JSON range,
   re-enable `minimum` / `maximum` in `gen_schemas.rs::parameter_schema`
   (currently dropped — see the comment block on that function).
10. **Update schemas-todo.md.** Mark B1, B3, B7 done; document any
    surprises.

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

### Optional satellite work — gate on time

- **C2** (legacy `Deserialize` policy) — since we're already bumping
  the version, this is the cheapest time to drop the legacy accept
  paths on `Position`, `CanvasSize`, `Author`. Eliminates the "schema
  describes serialize, not deserialize" gap. Worth doing if time
  allows.
- **E1** (`Map<u64, T>` → `Vec<T>` for patterns/tracks) — bigger
  format change but free under an already-versioned migration.
  Improves tooling interop. Discuss with the user before grabbing
  this — they may want it as its own beat.
- **E2** (`row_resolution` per-song, not per-pattern) — minor JSON
  cleanup. Defer unless the user explicitly asks for it.

### Known gotchas

- **Non-linear curves.** `ResponseCurve::Logarithmic`, `Exponential`,
  `SCurve`, `Squared` need their own mapping. A logarithmic
  frequency param maps internal 0..1 to a frequency range via
  something like `min * (max/min)^t`, not `min + (max-min)*t`. The
  round-trip test in step 2 will catch these — fix per-variant.
- **MCP tools** (`crates/synth_mcp/`) currently emit and accept
  values in whatever form `Param::as_f32` produces. Decide whether
  MCP should also flip to semantic values (recommended, for
  consistency with the new JSON) or stay normalized (less coupling).
  Coordinate with whatever MCP-facing docs exist.
- **`InstrumentState`'s manual `Deserialize`** in
  `crates/pertylizer/src/patch.rs` accepts default values for many
  fields. The migration shouldn't break this — but check.
- **Bundle ZIPs** carry an internal `project.json` that's just a
  ProjectFile, so they migrate transparently as long as the
  ProjectFile loader handles version gating.

---

## Priority summary (active)

**Save-format migration ("1.0" → "1.1") — next session:**

- B1: Semantic numeric ranges (the bit_depth issue)
- B3: Choice values always string in JSON
- B7: `Track.pan` to `BipolarValue`
- *(Optional in same migration: C2, E1, E2.)*

**Discuss before doing:**

- B2: Strict `additionalProperties: false` on `parameters`
- E1: `Map<u64, T>` → `Vec<T>` for patterns/tracks
- E2: `row_resolution` scope
