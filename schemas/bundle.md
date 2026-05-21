# Bundle archive format

Pertylizer projects with embedded samples are saved as `.zip` (or
`.pertylizer`) archives — not plain JSON. The schema in
[`bundle-metadata.schema.json`](./bundle-metadata.schema.json) describes
only the `metadata.json` blob inside the archive; this file documents
the surrounding ZIP layout.

## Archive structure

```text
my_project.pertylizer  (ZIP, DEFLATE-compressed)
├── project.json       — full ProjectFile (same shape as project.schema.json)
├── metadata.json      — BundleMetadata (see bundle-metadata.schema.json)
└── samples/
    ├── 1.wav          — sample data, 32-bit float WAV
    ├── 2.wav
    └── ...
```

## File rules

- **`project.json`** — required. Identical to the standalone project
  format documented by `project.schema.json`. Must parse against that
  schema.
- **`metadata.json`** — optional but strongly recommended. Carries the
  sample-library metadata that isn't part of the WAV header (display
  names, root notes, loop / crop regions). When absent the loader
  falls back to defaults derived from the WAV file alone.
- **`samples/<id>.wav`** — one file per sample in the library. Files
  must:
  - Live under `samples/` (single directory, no nesting).
  - Be named `<sample_id>.wav` where `<sample_id>` matches the `id`
    field of the corresponding `BundleSampleEntry` in `metadata.json`.
  - Be 32-bit float WAV. The loader also accepts 16-bit and 24-bit
    integer WAV for ingestion, but the canonical save format is 32-bit
    float to preserve the engine's internal precision.

## Sample ID convention

Sample IDs are `u64` values assigned by the engine when a sample is
imported. They never change for the lifetime of a project — patches
referring to a sample (via the `sample_select` parameter on `Sampler`
modules) use this ID. When a project is saved as a bundle, sample
files are named after their ID so the loader can match WAV files back
to metadata entries.

## Detection

`pertylizer::project::load_file` distinguishes bundles from plain JSON
project files by the leading bytes:

- `PK\x03\x04` (ZIP magic) → bundle, load via
  [`bundle::load_bundle`](../crates/pertylizer/src/bundle.rs).
- Otherwise → parse as JSON and dispatch on `file_type`
  (`"project"` / `"awe_preset"`) or fall back to plain patch format.

## Recommended extension

`.zip` works but `.pertylizer` is preferred for user-facing saves so
the file association picks up the right application icon and OS-level
preview handler.

## See also

- [`bundle-metadata.schema.json`](./bundle-metadata.schema.json) —
  schema for the inner metadata blob.
- [`project.schema.json`](./project.schema.json) — schema for
  `project.json`, identical to the standalone-project format.
- `crates/pertylizer/src/bundle.rs` — reference implementation of
  `save_bundle` / `load_bundle`.
