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

### B1. Parameter ranges (semantic vs normalized storage) [L, high impact]

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

**Polish:**

- A2: Bundle ZIP layout doc
- B4: `sample_id` only on Sampler params
- D2: Factor shared sub-schemas into `$defs`

---

## Next iteration

Two reasonable directions; pick one.

### Option α — Polish batch (recommended for a short iteration)

**Scope:** A2, B4, D2. All small, no design decisions, can ship in one PR.

- **A2** writes a `schemas/bundle.md` describing the ZIP archive layout
  next to the schema files.
- **B4** narrows the `sample_id` object alternative in `gen_schemas.rs`
  to fire only for `Param::Sampler(SamplerParam::SampleSelect(..))`
  variants. Cosmetic but makes the schema honest.
- **D2** factors repeated module-branch sub-schemas (id-pattern wrapper,
  sample-id branch, position $ref) into `$defs` references. Should cut
  the patch/project schemas roughly in half.

**Effort:** ~2–4 hours of focused work. No format changes, no migrations.
Test coverage already in place via the `schemas_validate_examples` tests.

### Option β — Save-format migration ("1.0" → "1.1") (recommended for biggest impact)

**Scope:** B1 + B3 + B7 bundled. Real format change with a version bump
and on-load migration of legacy files.

- **B1** adds `Param::to_display_f32` / `from_display_f32` and routes
  saves through the display form so numeric values match the
  descriptor range. JSON becomes human-readable (`"bit_depth": 8` not
  `0.466`) and the schema can emit real `minimum`/`maximum` constraints.
- **B3** rides on the same migration: choice parameters always
  serialize as their string ID, never as a numeric index.
- **B7** migrates `SequencerTrack.pan` from `NormalizedValue` (0..1) to
  `BipolarValue` (-1..1) so it matches the module-level pan convention.
- All three are gated on `version` in the file header. Legacy files
  with `"version": "1.0"` get converted at load time; new saves emit
  `"1.1"`.

**Effort:** 1–3 days. The big risks are non-linear parameter responses
(logarithmic frequency etc.) where round-trip conversion can introduce
small drift; snapshot tests will catch any audible regressions.

**Recommendation:** Start with **α** if we want a quick win and want to
commit the current state first. **β** if we're in for a longer session
and want to unlock real range validation as the next deliverable.

### Optional satellite work

- C2 (legacy `Deserialize` policy) — flag for the migration: if we're
  already bumping the format, we can also drop the legacy accept
  paths on `Position`, `CanvasSize`, `Author`. Eliminates the
  "schema describes serialize, not deserialize" gap.
- E1 / E2 — if we're already breaking the format for β, this is the
  cheapest time to also flip patterns/tracks to `Vec<T>` and lift
  `row_resolution` to song level. Each is a separate decision —
  ping me if you want a deeper write-up on either.
