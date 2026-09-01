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

`synth_engine_v2` is an explicitly unstable experimental boundary until a
production dependency or supported external consumer is declared. The user's
2026-09-01 process-reset approval pre-approves clean breaks to its Rust API when
the diff is confined to that crate and repository-local development or test
consumers. Update every such consumer in the same change. This standing approval
does not cover persisted data, wire or protocol contracts, manifests, a shipping
dependency edge, an external consumer, or `unsafe` code; each still requires
specific approval.

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

Choose the smallest gate that covers the changed risk:

When more than one row applies, use the strongest applicable gate. The complete
repository gate includes the core and evidence gates; the core gate includes the
evidence gate.

| Change | Required verification before commit |
|---|---|
| Mechanical documentation, status, links, or indexes | `git diff --check`; for Core V2, run the fast documentation gate below |
| Documentation-checker behavior | The fast documentation gate plus author diff review; use the evidence gate when simulator selection, validation or dependency controls change |
| Normative documentation, ADRs, specifications, process rules, or evidence conclusions | The fast documentation gate plus one independent semantic review |
| Evidence method, harness, digest, phase exit, or Core V2 code in the EVD-0016 simulator dependency closure | The evidence gate below plus the review required by the change's other risk |
| Rust behavior confined to one package and outside the boundary-sensitive set below | The targeted Rust gate plus author review of the complete diff |
| Admission, scheduling, identity, concurrency, persistence, protocol, real-time boundaries, or production-facing APIs | The core Rust gate plus one independent uncommitted review |
| Features, dependencies, build configuration, phase exit, release, or merge to `main` | The complete repository gate plus one independent uncommitted review |

The fast Core V2 documentation gate is:

```bash
python3 -B scripts/check_v2_docs.py
python3 -B -m unittest scripts/test_check_v2_docs.py
```

It checks structure, links, registers, Python evidence controls, specification
coverage, and active-document style without compiling Rust.

The Core V2 evidence gate adds the deterministic EVD-0016 simulator:

```bash
python3 -B scripts/check_v2_docs.py --evidence
python3 -B -m unittest scripts/test_check_v2_docs.py
```

The quality workflow always runs the evidence gate. Run it locally when an
evidence method, harness, digest, phase exit, or any code in the simulator's
dependency closure changes. The simulator evaluates roughly 11.5 million
long-horizon observations; ordinary mechanical documentation does not need to
repeat it. The simulator must remain in `synth_engine_v2`'s
system-library-free dependency closure because CI runs it before installing
Linux system libraries.

The targeted Rust gate, valid only while every changed Rust path is inside one
package, is:

```bash
cargo fmt --check
cargo clippy -p <package> --all-targets
cargo test -p <package>
```

Use the core Rust gate instead when a diff crosses package boundaries or touches
admission, scheduling, identity, concurrency, persistence, a protocol, a
real-time boundary, or a production-facing API. A package-local change does not
become boundary-sensitive merely because it fixes ordinary UI, formatting,
diagnostic wording, or isolated non-real-time logic.

The core Rust gate is:

```bash
python3 -B scripts/check_v2_docs.py --evidence
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
cargo +1.98.0 check --workspace
```

`--workspace` is required. The workspace has
`default-members = ["crates/pertylizer"]`, and the standalone
`synth_engine_v2` crate is not a dependency of that package. A bare Cargo command
therefore does not cover every workspace target. The explicit release-mode test
is also required because its `#[cfg(not(debug_assertions))]` branch is not
compiled by the ordinary development-profile test run.

The complete gate mirrors `.github/workflows/quality.yml`: the core gate plus
the three configuration/MSRV commands above. The MSRV command requires the
Rust 1.98 toolchain.

The only pre-approved Clippy exceptions are `too_many_lines` for large
`process()` functions, `cast_precision_loss` for `usize` to `f32` conversion in
audio code, `cast_possible_truncation` when the value is proven to fit, and
`chunks_exact_to_as_chunks` inside an evidence harness whose recorded figures
were measured on the existing code shape, where the rewrite would change the
bounds checks the timed loop emits. Every exception still needs a narrowly
scoped allowance at the relevant site.

#### Who may perform an independent review

This governs every table row above that requires an independent review. A
targeted ordinary Rust change receives author diff review instead.
A reader qualifies by two properties rather than by being a named tool:

1. **It did not author the change.** A reader holding the session context in
   which the change was written has memory of what you meant and cannot
   independently review it.
2. **It is a different model family from the author.** Fresh context removes
   the memory; a different family removes the shared blind spot. Both are
   required, because an author has judged its own findings correct where only
   the other family caught the error.

Pick the reader you are not:

| Author | Reader | Command |
|---|---|---|
| Claude Code | Codex | `codex review --uncommitted` |
| Codex | Claude Code | the inline invocation below |
| Gemini CLI | Codex, or Claude Code | either of the above |

Claude Code's `/code-review` skill may fan the review out to subagents rather
than running it inline, which the no-delegation clause below forbids, so it must
not be used as the reader here. Invoke Claude Code directly instead: the tool
allowlist omits the subagent tool and `--disable-slash-commands` keeps the skill
from being reached, so neither is left to the reader's discretion.

```bash
set -o pipefail
{ set -e
  git status --short --untracked-files=all
  printf '\n=== staged ===\n';   git diff --cached
  printf '\n=== unstaged ===\n'; git diff
} | claude -p \
  "Review this uncommitted change against CLAUDE.md's invariants. The status
   block lists untracked files; open them yourself. Report defects only. Do
   not fix anything." \
  --tools "Read,Grep,Glob" --disable-slash-commands \
  --strict-mcp-config --mcp-config '{"mcpServers":{}}'
```

Check the exit status. Five details are load-bearing:

- **Staged and unstaged go separately.** `git diff` alone omits the index, and
  `git diff HEAD` alone can render as empty when a staged edit is changed back
  in the worktree — the commit would then be reviewed without its own content.
  Emitting both is the same rule this file already applies before a commit. The
  status block with `--untracked-files=all` covers the third case, new files,
  which no diff carries.
- **MCP is switched off.** `--tools` restricts only the built-in set; the
  servers in `.mcp.json` still load, and `.claude/settings.local.json`
  preapproves mutating `synth` calls such as `save_project`. `--strict-mcp-config`
  with an empty `--mcp-config` is what removes them. Verified: the reader then
  reports no `mcp__` tool at all.
- **The allowlist omits the shell.** `Bash` is not read-only here:
  `.claude/settings.local.json` preauthorizes `git add` and `git commit`, so a
  reader holding `Bash` can write. `Read`, `Grep` and `Glob` are enough to open
  the untracked files the status block names.
- **No `--permission-mode plan`.** The reader then needs a tool the allowlist
  withholds and cannot finish. The allowlist is already the write barrier, and
  omitting the subagent tool is what enforces the no-delegation clause below.
- **`set -o pipefail` and `set -e`.** Without both, a failure while collecting
  the change set is masked by the reader's own success, and an empty or partial
  review passes the gate. Measured: the guarded pipeline exits non-zero when
  collection fails and zero when it succeeds.

`CLAUDE.md`, `AGENTS.md`, and `GEMINI.md` are the same file. This rule
therefore reaches every agent that reads it, including one invoked as a reader,
so two clauses bound it:

- **The reader performs the review itself and does not delegate it.** An
  invoked reader must not start a third agent to review on its behalf. Doing so
  returns the review to the author's family and can recurse between the two.
- **If no qualifying reader is available**, property 1 still binds: run the
  review with a reader of the author's own family. That is an approved gate
  exception, not an independent review, and it satisfies the table's
  requirement only as a waiver. Name the waiver and why no qualifying reader
  was available in the commit message. The waiver is unavailable for a merge to
  `main` and for a release; those stop until a qualifying reader is available.

Read and report the findings before acting on them. One independent review
covers one declared high-risk scope; a separate design and uncommitted review
are not both required when the same read can inspect the final artifact. Follow
the [review and design protocol](#review-and-design-protocol). Re-run every gate
command affected by a repair; run the complete gate again when the repair
changes features, dependencies, build behavior, release behavior, or a
cross-configuration contract.

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

1. **Frame the design before building.** For a measurement, durable decision,
   or phase acceptance criterion, write the falsifier and ask *what would make
   this wrong?* An independent design consultation is required only when the
   method has an unresolved asymmetry or the choice crosses a safety boundary.
2. **Do not ask the reviewer to author the frames.** A reader who wrote the
   constraints cannot independently review them.
   `plans/v2/PROCESS.md` requires a reader who did not author the durable
   change; that is property 1 and it is the floor. Property 2 above is this
   file's additional requirement and applies to every review it gates.
3. **A criterion must be falsifiable.** State the observable symmetry,
   threshold, artifact, or failure that can violate it.
4. **Verify factual claims while drafting.** Check them against the code or the
   command's actual output; do not infer facts that are cheap to inspect.
5. **Self-audit every repair.** Search for renamed or renumbered references,
   recount changed totals, and reread for contradictions introduced by the
   repair.
6. **Reread only boundary-sensitive repairs.** A focused independent reread is
   required when a repair changes the reviewed conclusion, contract, evidence
   method, safety boundary, or code in admission, scheduling, identity,
   concurrency, persistence, protocol, or the real-time path. Other repairs use
   author self-audit and affected tests. A reviewer may explicitly mark a
   finding as requiring a focused reread. Never restart a broad review merely
   because a repair is semantic.
7. **State the stopping rule.** A false claim, internal contradiction, or
   contract hole that an implementer cannot fill blocks acceptance. A request
   for optional implementation detail does not.

Do not diagnose a reader process (`codex`, `claude -p`) as hung from zero CPU
usage alone: a healthy client may sleep while waiting for network work. Over a
bounded observation period, inspect process state, elapsed time, wait channel,
output progress, and the `rchar` delta in `/proc/<pid>/io`. Interrupt or retry
only when the process has exceeded a reasonable deadline and the combined
evidence shows no progress.

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
