# Project Instructions for Pertylizer

## Scope and priority

These instructions apply to the entire repository. More specific instructions in
a nested `AGENTS.md` take precedence for files below that directory.

### Language

All code, comments, UI strings, documentation, and commit messages are in
**English**.

### Project phase and compatibility

Pertylizer is in active development. Backward compatibility is **not promised**,
but breaking an existing API, persisted format, manifest, wire contract, or
protocol requires the user's **explicit approval for that break**.

After approval, make the break cleanly; compatibility shims and migrations are
not required unless the user asks for them. For persisted and wire contracts,
prefer a format or protocol version change. Never silently give an existing
version or serialized field a different meaning. If an approved break remains
unversioned, document that decision and make old input fail clearly rather than
being misinterpreted.

---

## Required delivery workflow

### Before committing

First inspect `git status --short` and the relevant diffs. Preserve unrelated
changes already in the worktree, and do not stage them. "Clean" below means that
the checks report no warnings or errors and that the intended change contains no
unexplained files; it does not mean that the worktree has no uncommitted change.

Choose the gate from the change, rather than running unrelated work:

| Change | Required verification before commit |
|---|---|
| Mechanical documentation, status, links, indexes, or `scripts/check_v2_docs.py` | `git diff --check`; for Core V2, run the documentation gate below |
| Normative documentation, ADRs, specifications, evidence methods, or process rules | The documentation checks above plus one independent semantic review |
| Rust behavior | The core Rust gate below plus `codex review --uncommitted` |
| Features, dependencies, build configuration, release, or merge to `main` | The complete repository gate below plus `codex review --uncommitted` |

The Core V2 documentation gate is:

```bash
python3 -B scripts/check_v2_docs.py
python3 -B -m unittest scripts/test_check_v2_docs.py
```

The core Rust gate is:

```bash
python3 -B scripts/check_v2_docs.py
python3 -B -m unittest scripts/test_check_v2_docs.py
cargo fmt --check
cargo build --workspace
cargo clippy --workspace --all-targets
cargo test --workspace
cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure
cargo doc --workspace --no-deps
```

The complete repository gate adds configuration and MSRV coverage:

```bash
cargo check --workspace --all-targets --no-default-features
cargo check --workspace --all-targets --all-features
cargo +1.97.0 check --workspace
```

`--workspace` is required. The workspace has
`default-members = ["crates/pertylizer"]`, and the standalone
`synth_engine_v2` crate is not a dependency of that package. A bare Cargo command
therefore does not cover every workspace target. The explicit release-mode test
is also required because its `#[cfg(not(debug_assertions))]` branch is not
compiled by the ordinary development-profile test run.

The complete gate mirrors `.github/workflows/quality.yml`: the core gate plus
the three configuration/MSRV commands above. The MSRV command requires the
Rust 1.97 toolchain.

The only pre-approved Clippy exceptions are `too_many_lines` for large
`process()` functions, `cast_precision_loss` for `usize` to `f32` conversion in
audio code, and `cast_possible_truncation` when the value is proven to fit.
Every exception still needs a narrowly scoped allowance at the relevant site.

When the table requires a repository review, use a reader that has no memory of
what you meant:

```bash
codex review --uncommitted
```

Read and report the findings before acting on them. A separate independent
design review and `codex review --uncommitted` are not both required when one
review can satisfy the declared scope and stopping rule. Follow the
[review and design protocol](#review-and-design-protocol), including its
self-audit after every repair. Re-run every gate command affected by a repair;
run the complete gate again when the repair changes features, dependencies,
build behavior, release behavior, or a cross-configuration contract.

Before the commit, inspect `git status --short`, `git diff`, and
`git diff --cached`. Stage only the intended paths:

```bash
git add -- <path>...
git diff --cached --check
git diff --cached
git commit -m "<short description of changes>"
```

Use `git add --all` only after verifying that every worktree change belongs in
the same commit.

### Review and design protocol

Reviews are load-bearing: they catch false behavioral claims, contradictions,
uncontrolled measurements, and contract holes that compilation cannot find.
They must remain independent and bounded.

1. **Review the design before building.** For a measurement, decision record,
   or anything carrying acceptance criteria, write the criteria first and ask:
   *what would make this wrong?*
2. **Do not ask the reviewer to author the frames.** A reader who wrote the
   constraints cannot independently review them.
   `plans/v2/PROCESS.md` requires a reader who did not author the durable
   change.
3. **A criterion must be falsifiable.** State the observable symmetry,
   threshold, artifact, or failure that can violate it.
4. **Verify factual claims while drafting.** Check them against the code or the
   command's actual output; do not infer facts that are cheap to inspect.
5. **Self-audit every repair before another read.** Search for renamed or
   renumbered references, recount changed totals, and reread for contradictions
   introduced by the repair.
6. **Scope rereads to the changed claim or clauses.** Do not trigger a broad
   reaudit when a focused independent read can settle the repair.
7. **State the stopping rule.** A false claim, internal contradiction, or
   contract hole that an implementer cannot fill blocks acceptance. A request
   for optional implementation detail does not.

Do not diagnose a `codex` process as hung from zero CPU usage alone: a healthy
client may sleep while waiting for network work. Over a bounded observation
period, inspect process state, elapsed time, wait channel, output progress, and
the `rchar` delta in `/proc/<pid>/io`. Interrupt or retry only when the process
has exceeded a reasonable deadline and the combined evidence shows no progress.

### Merging a branch to `main`

Work completed on a branch is always squash-merged so the feature lands on
`main` as one commit. Before switching branches, ensure all intended branch work
is committed and unrelated worktree changes are safe.

```bash
git switch main
git merge --squash <branch>
```

Inspect the staged squash, run the required gate and uncommitted review, then:

```bash
git commit -m "<summary of the whole branch>"
git branch -D <branch>
```

Delete the branch only after the squash commit succeeds. Never use a plain or
fast-forward merge for completed branch work.

### Creating a new version

1. **Document every commit since the previous version.** The boundary is the
   most recent release commit, not a tag. Enumerate the range with:
   ```bash
   git log "$(git log --grep='^Release v' --format=%H -n 1)"..HEAD --oneline
   ```
   Fold every listed commit into the new history entry; no commit may be omitted.
2. Add the newest entry at the top of `docs/history.md` as
   `## [x.y.z] - YYYY-MM-DD`.
3. Review `plans/TODO.md` and mark completed tasks.
4. Update `crates/pertylizer/Cargo.toml`, then run `cargo build --workspace` so
   `Cargo.lock` is synchronized.
5. If dependencies changed since the last release, regenerate attribution:
   ```bash
   cargo about generate --manifest-path crates/pertylizer/Cargo.toml \
     about.hbs -o THIRD-PARTY-LICENSES.md
   ```
   This requires `cargo-about` (`cargo install cargo-about --locked --features
   cli`). If generation rejects a license, add it to `about.toml` only after
   confirming it is redistributable. Commit the regenerated file.
6. Follow the full commit workflow and commit on `main` as
   `Release vX.Y.Z: <summary>`.
7. Verify that the Cargo version and `docs/history.md` section both equal the tag
   version. Create the tag and atomically push `main` and the tag:
   ```bash
   git tag vX.Y.Z
   git push --atomic origin main vX.Y.Z
   ```

Pushing a `v*` tag is the only release trigger in
`.github/workflows/build.yml`; ordinary commits and manual dispatch do not start
a release. The tag name is the release version's source of truth
(`v0.313.0` becomes `0.313.0`), and the workflow extracts release notes from the
matching `docs/history.md` section. The tag must therefore point at a commit
that already contains both the matching Cargo version and history entry.

### `docs/history.md` style

Keep each fix or change description to one or two sentences: what was broken or
added, followed by the essence of the fix. Do not include multi-paragraph design
rationale or test-list enumerations; the commit and code carry the detail.

---

## Engineering invariants

### Newtypes for domain concepts (critical)

Never use a raw primitive for a domain concept. Search the codebase first; a
suitable newtype likely exists.

```rust
// Wrong: raw primitive for a domain value.
fn set_frequency(hz: f32) { ... }

// Right: the unit and invariant are explicit.
fn set_frequency(freq: Hertz) { ... }
```

Raw primitives are acceptable for loop counters, intermediate arithmetic, and
FFI or serialization internals.

| Crate | Examples |
|---|---|
| `synth_core` | `Hertz`, `SampleRate`, `DeviceSampleRate`, `Cents`, `Semitones`, `MidiNote`, `MidiChannel`, `Velocity`, `Gain`, `Decibels`, `Seconds`, `Milliseconds`, `Bpm`, `NormalizedValue`, `NormalizedDelta`, `BipolarValue`, `BipolarDelta`, `Phase`, `SampleCount`, `BlockSize`, `PortName`, `Meters`, `MetersPerSecond`, `SampleOffset`, `Position3`, `InstrumentId`, `SampleId` |
| `synth_sequencer` | `PatternId`, `TrackId`, `NoteId`, `Tick`, `PatternTick`, `Duration`, `Pitch`, `TrackIndex`, `RowIndex`, `NoteGraphId`, `NoteModuleId` |
| `synth_engine` | `TransactionId`, `ClientId`, `MidiChannelSelection`, `ConnectionCount`, `ModuleId`; instrument IDs use `synth_core::InstrumentId` |

Newtype invariants:

- Fields are private unless an external representation requires otherwise.
- Construction validates the domain invariant. Use `Result` or `TryFrom` for
  fallible external input; invalid values must not enter domain logic.
- Clamp only when clamping is explicitly documented domain behavior. Do not
  silently replace invalid persisted, protocol, or user input with another
  value.
- Expose named accessors such as `as_f32()`. Implement only arithmetic meaningful
  for the unit; a newtype protects a concept rather than merely renaming a
  primitive.

### State changes and errors

- Never discard a `Result` or boolean success value from I/O, serialization,
  command sends, state reconstruction, or persistence.
- Propagate failure, surface a diagnostic, or explicitly document why failure is
  harmless. Best-effort behavior must not silently leave engine or project state
  at a default or partial value.

### Serialized contracts

- Breaking a serialized or protocol contract requires the explicit approval
  described under [project phase and compatibility](#project-phase-and-compatibility).
- Prefer a format or protocol version change for every approved semantic break.
  No backward reader or migration is required unless explicitly requested, but
  old data must never be silently reinterpreted as the new contract.
- Closed persisted schemas, manifests, receipts, and protocols use
  `#[serde(deny_unknown_fields)]` so misspelled fields cannot be ignored.
- Required fields do not use `#[serde(default)]`. Optionality is deliberate; when
  it matters, preserve missing versus explicit `null`.
- Add round-trip tests and rejection tests for unknown, missing, and invalid
  fields, including old-version rejection when an approved break has no
  migration.
- Persisted output is deterministic: use canonical ordering or sort collections
  before serialization and digesting.

### Numeric invariants

- Validate external and persisted numeric values at the boundary. Reject
  non-finite floats and out-of-domain values unless a documented saturating type
  owns that policy.
- Prefer `From` for infallible conversions and `TryFrom` for checked conversions.
  Avoid `as` casts at domain boundaries and keep units typed through calculations.
- Use `total_cmp` when floats require ordering. Do not use raw floating-point
  values as identity or map keys without a documented canonical representation.

### Stable identity

- Do not use collection position, vector index, display order, name, or module
  instance number as persistent identity unless the domain explicitly defines it
  that way.
- Keep stable IDs distinct from indices, counts, ordering positions, and sample
  or tick offsets through their dedicated newtypes. Reordering a collection must
  not change what persisted references identify.

### Code style

- Use `Self` in impl blocks, not the type name.
- Use `thiserror` for error types; do not manually implement both `Display` and
  `Error`.
- Do not use `.unwrap()` or `.expect()` in production code. Use `?`, `if let`, or
  an explicit safe default such as `unwrap_or` where the fallback is correct.
- Use `pub(crate)` for internal types and minimize public API surface.
- Add `#[must_use]` to newtypes and builder methods.
- Do not add `unsafe` code without discussing it with the user first.

### Real-time safety

In `process()` functions and other real-time-critical code:

- Do not allocate on the heap. This includes operations that may allocate even
  if capacity often happens to be available, such as `Vec::push`, plus
  `Box::new` and `String::clone`.
- Do not acquire blocking locks, including any `Mutex` or `RwLock` read/write
  guard.
- Do not panic. Avoid `unwrap`, `expect`, unchecked assumptions about indices,
  and indexing that has not been proven in bounds.
- Do not perform system calls, file I/O, or logging.
- Use preallocated buffers, atomics, and lock-free structures. An explicit safe
  fallback such as `unwrap_or(0.0)` is allowed when zero is the correct domain
  behavior.
- Use `for` loops for DSP sample processing. Keep iterator-based traversal
  outside the hot path.

---

## Architecture

### Thread model

- **Audio thread:** real-time and lock-free. Runs `SynthEngine::process()` and
  communicates through `EngineCommand` and `EngineEvent` ring buffers.
- **UI thread:** renders egui and holds `EngineHandle` for commands and shared
  atomic state.
- **Shared state:** `Arc<SharedSong>` keeps editable sequencer data behind an
  `RwLock` and publishes immutable `Arc<Song>` snapshots through `ArcSwap`. The
  audio thread reads snapshots lock-free. The UI thread uses `write()` for
  mutations. Collect snapshots before rendering, release locks, and then draw.

### GUI architecture

- Import icons as `egui_remixicon::icons as ri`.
- Panel order is TopPanel, SidePanel, `TopBottomPanel::bottom`, then CentralPanel.
- Patch-editor modules use `egui::Area` at `Order::Background`. The keyboard
  panel renders at `Order::Middle` for input priority.
- Collect pattern snapshots with `collect_arrangement_data` and
  `collect_piano_roll_data` before drawing to minimize lock hold time.

### Shared widgets

Small reusable widgets and composite controls belong in
`crates/pertylizer/src/gui/widgets/controls.rs`: themed labels, icon buttons,
toggles, drag/slider presets, and layout idioms such as `right_aligned_row`.

Add a helper even for a single current caller when the primitive is a reusable
idiom. Large view-specific composites remain with their view or chrome module;
for example, `ModuleFrame`, `draw_module_header`, and `draw_module_footer` stay
in `widgets/frame.rs` and call small controls helpers.

---

## Repository tools

### Search and structural rewrites

Use `rg` for file discovery, literal text, and regular-expression searches. Use
`ast-grep` when the question concerns a syntax shape such as calls, expressions,
fields, or Rust items independent of formatting.

Quote metavariables so the shell does not expand them:

```bash
ast-grep run --lang rust --pattern '$EXPR.unwrap()' crates
```

Use the full `ast-grep` name; its `sg` alias is deprecated and may collide with
`shadow-utils`. After a structural rewrite, inspect the complete diff. A
syntactic match does not establish domain correctness or real-time safety.

### Pertylizer MCP server

The repository's `synth` MCP server is available at
`http://127.0.0.1:9850/mcp`. Gemini CLI configuration is in
`.gemini/settings.json`; Claude Code configuration is in `.mcp.json`.

Representative tools:

- Discovery: `list_module_types`, `get_module_type_info`, `search_modules`,
  `list_port_types`.
- Instruments: `create_instrument`, `build_instrument`, `list_instruments`,
  `set_parameter`.
- Sequencer: `create_pattern`, `add_note`, `create_track`, `place_pattern`.
- Analysis: `analyze_harmony`, `analyze_mix_bus`, `analyze_section`.

Use `batch_execute` for independent operations that can safely be issued
together.

#### MCP feedback is mandatory

This feedback rule applies only to Pertylizer's own `synth` MCP server at the
endpoint above, not to egui inspection or unrelated MCP servers.

Whenever a `synth` call errors, returns an unexpected result, is awkward to use,
lacks a needed tool, or has a confusing or incomplete schema or description,
stop and report:

- the exact tool, arguments, and error or response;
- why it obstructed the task and any workaround required;
- a concrete improvement such as validation, clearer documentation, a missing
  field, an array variant, or a new tool.

Do not silently work around a `synth` MCP shortcoming; identifying these gaps is
part of the task.
