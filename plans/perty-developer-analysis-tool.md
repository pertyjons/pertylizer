# Pertylizer developer analysis tool

> **Status:** proposed.
>
> **Planning baseline:** Pertylizer `33bf1162`, 2026-08-17.

## Objective

Build a repository-owned command-line tool, invoked as `cargo perty`, that turns
Pertylizer's architecture and development policies into deterministic,
machine-readable analysis.

The tool has two jobs, delivered as two releases:

1. Given a Git change set, explain which workspace packages, targets, checks,
   and project-specific risk areas it affects.
2. Starting at the audio callback, find forbidden or unresolved effects and
   report the complete call path that makes them reachable.

**The first useful release is job 1 only**, delivered by Phases 1 and 2 and
accepted by the criteria at the end of this document. Job 2 is the second
release: Phases 3 to 5, with its own acceptance criteria. The split is not a
preference — an incomplete real-time audit is worse than none, because a report
that says "no forbidden effects" while an unresolved edge hides one is a false
clearance, and Phases 3 and 4 are what make unresolved edges visible.

The tool is for developers, CI, and coding agents. Human-readable output is the
default, but every command must also have a versioned JSON representation so an
agent never has to infer status from colored prose.

## Why Pertylizer needs its own tool

The workspace currently has thirteen packages, while `default-members` selects
only `pertylizer`. The full repository gate correctly uses `--workspace` for
Clippy and tests, but that is intentionally more expensive than the checks
needed during each edit-and-test iteration. Cargo knows the dependency graph;
it does not know which Pertylizer risks a changed file represents.

Pertylizer also has policies that generic Rust tooling cannot enforce on its
own:

- the audio callback and everything it reaches must remain allocation-free,
  non-blocking, non-panicking, and free of system calls;
- domain values must use validated newtypes rather than raw primitives;
- persisted and protocol contracts are closed, validated, and deterministic;
- stable IDs must not be confused with indices or display order;
- MCP, project persistence, GUI state, and the V1/V2 engines have different
  verification needs.

`cargo`, Clippy, rust-analyzer, ast-grep, and Flowistry each cover part of this
space. The proposed tool composes their useful information with repository
policy; it does not replace them.

At the planning baseline there are more than one hundred functions named
`process` or `process_*` in the DSP, module, and engine crates. A name search
cannot determine which of them are reachable from the callback, whether a
method call resolves to an allocating implementation, or which trait
implementations may be selected dynamically. That is the central problem for
the real-time audit.

## Non-goals

- Do not build a general-purpose Rust static analyzer.
- Do not replace the mandatory pre-commit commands in `AGENTS.md`.
- Do not claim that a targeted check proves the whole workspace is valid.
- Do not put the tool or its analysis on the audio thread or in the shipped
  application.
- Do not add a second build system around Cargo.
- Do not execute Git commits, modify source files, install toolchains, or start
  external services.
- Do not make the initial release depend on a running Pertylizer or synth MCP
  server.
- Do not make `audit domain`, serialized-contract auditing, MCP conformance, or
  semantic context lookup prerequisites for the first useful release. They are
  follow-on capabilities described here so the initial contracts leave room
  for them.

## Product decisions

### Repository-owned CLI

Add a workspace package at `tools/perty` with package name `perty-tool` and a
binary named `perty`. Add this Cargo alias:

```toml
[alias]
perty = "run --quiet -p perty-tool --"
```

The package is a normal workspace member, so `cargo clippy --workspace
--all-targets`, `cargo test --workspace`, and `cargo doc --workspace --no-deps`
cover it. Do not add it to `default-members`; a bare `cargo run` must continue
to select the application. The alias always names the package explicitly.

**The alias does not work everywhere inside the repository, and the acceptance
criterion is scoped to say so.** `visualizer/` is a package of its own and is
`exclude`d from the workspace, so Cargo invoked from there selects *that*
workspace and `-p perty-tool` names a package it has never heard of. Two
dispositions, and this plan takes the first:

- **Scope the claim.** The alias works from the workspace Cargo selects for it —
  anywhere inside the workspace root except an excluded package's directory —
  and the Phase 1 exit criterion says exactly that rather than "any directory".
- **Install a binary** later, if invoking from excluded trees turns out to
  matter. A `perty` on the path discovers the repository root itself and does
  not depend on which workspace Cargo picked.

Keep the tool independent of Pertylizer application crates. It may read Cargo
metadata, Git data, Rust source, and a checked-in policy file, but the shipped
application must not gain a dependency on development tooling.

### Honest completeness

Every analysis result has one of three completeness states:

- `complete`: all inputs needed by that analysis were available and resolved;
- `partial`: useful findings were produced, but at least one relevant edge,
  configuration, or source could not be analyzed;
- `failed`: the requested analysis could not establish a usable result.

An unresolved dynamic call is not silently ignored. A missing rust-analyzer is
not replaced by a regex pass reported as success. Human and JSON output must
name every reason an analysis is partial.

### Advice versus gates

`impact` recommends fast checks for the current iteration. Its output must
always state that the repository's complete pre-commit gate remains mandatory.

`audit` commands are policy gates: deny-level findings or incomplete analysis
produce a non-zero exit status. This fail-closed behavior matters most for
real-time safety, where a false clean result is worse than an explicit unknown.

### Stable structured output

All commands accept `--format human` and `--format json`. JSON uses a top-level
envelope such as:

```json
{
  "schema_version": 1,
  "command": "impact",
  "status": "success",
  "completeness": "complete",
  "diagnostics": [],
  "result": {}
}
```

The exact payload types are command-specific. Contracts use
`#[serde(deny_unknown_fields)]`; required fields have no defaults; numeric and
path values are validated before entering domain types. Collections are sorted
before serialization. Introduce newtypes such as `PackageName`, `RuleId`,
`RustSymbol`, `SourcePath`, `LineNumber`, and `AnalysisSchemaVersion` instead of
passing domain concepts as raw strings or integers.

JSON goes to stdout. Operational diagnostics and child-process output go to
stderr. The tool must never mix progress text into a JSON document.

Use stable exit meanings across commands:

| Exit | Meaning |
|---:|---|
| `0` | Command completed and no deny-level finding occurred |
| `1` | Analysis completed and found a policy violation, or a requested gate failed |
| `2` | Invalid invocation, configuration, Git range, or other boundary input |
| `3` | Analysis was incomplete where completeness is required |

Process exit codes are protocol values and should be wrapped and validated
inside the implementation even though the operating-system boundary ultimately
uses an integer.

## Command surface

The intended surface is:

```text
cargo perty impact [--worktree | --staged | --range <REVISION_RANGE>]
cargo perty audit rt [--changed] [--no-cache]
cargo perty gate commit
cargo perty gate push

# Later phases
cargo perty audit domain
cargo perty audit serde
cargo perty mcp check
cargo perty context <RUST_SYMBOL>
```

`--worktree` is the default for `impact` and includes tracked staged and
unstaged changes plus untracked files. Rename and deletion records must retain
both relevant paths. `--range` accepts a validated Git revision range and does
not interpolate it into a shell command.

**A range is analyzed against the workspace its endpoint had, not the one the
checkout has now.** The workspace model comes from `cargo metadata` on the
current tree, so a range ending anywhere else can attribute a changed path to a
package that has since moved, been renamed, or been removed — and the failure is
silent: the path simply matches nothing and the change is reported as affecting
nobody. Two dispositions, and the first release takes the first:

- **Refuse.** If the range's endpoint is not the checked-out commit, exit with a
  clear diagnostic naming both commits. Refusing is correct rather than
  conservative: a wrong impact report is acted on, and an absent one is not.
  **The endpoint matching `HEAD` is not sufficient by itself** — `cargo metadata`
  reads the working tree, so an edited root manifest or a package moved but not
  committed puts the model somewhere the endpoint never was. The check is
  therefore: endpoint equals `HEAD` **and** no uncommitted change touches a
  manifest, `.cargo/config.toml`, or a package's location. Anything else refuses
  with the offending path named.
- **Materialize.** Later, load `cargo metadata` from a temporary worktree at the
  endpoint and analyze against that model. This is what makes `--range` useful
  for reviewing someone else's branch, and it is deferred, not dropped.

All commands support `--format`; commands that emit source findings also
support `--deny warnings` when warning-level findings should fail automation.

## Architecture

```text
CLI boundary
    |
    +--> Git change collector --------+
    |                                 |
    +--> Cargo workspace model -------+--> Policy engine --> report model
    |                                 |                         |
    +--> Rust semantic provider ------+                         +--> human renderer
    |                                 |                         +--> JSON renderer
    +--> source/effect rules ---------+
    |
    +--> process runner --> gate results
```

Keep collection, policy, and rendering separate:

- collectors return typed facts and diagnostics, never formatted prose;
- policy code consumes facts without spawning processes;
- renderers consume one canonical report model;
- subprocess execution accepts argument arrays and explicit working
  directories, never shell fragments;
- tests can supply fake Git, Cargo, semantic, and process providers.

The tool may cache expensive semantic results under `target/perty/`. Cache keys
include the tool version, rustc version, rust-analyzer version, Cargo.lock
digest, feature set, policy digest, and source digests — **and every input that
can change the resolved call graph without changing any of those**:

- every manifest in the workspace, root and members, not only the lock file: a
  feature mapping, an optional dependency, a path dependency, or a
  `[target.'cfg(...)'.dependencies]` condition changes what compiles without
  moving `Cargo.lock` a byte;
- Cargo configuration that applies to the invocation — `.cargo/config.toml` at
  every level Cargo reads, including `[build]` flags and target selection;
- the toolchain channel and the target triple;
- the **effective cfg and build inputs** — `RUSTFLAGS` and its per-target
  equivalents, `--cfg` values, and any environment a build script reads. These
  change what rust-analyzer resolves while every other component of the key
  stays identical.

**Where an input cannot be captured, reuse is disabled rather than assumed
safe.** A cache key that silently omits an input is not a smaller key; it is a
wrong one, and the failure it produces is a clean-looking audit.

The failure this prevents is the dangerous kind: a *stale* semantic result is
not a wrong answer that looks wrong, it is a **falsely complete** real-time
audit. Cache data is disposable and is never a source of truth.

## `impact`: change-to-verification analysis

### Inputs

Collect changes with NUL-delimited Git plumbing so spaces, non-ASCII names,
renames, and deletions remain unambiguous. For worktree analysis, combine
porcelain-v2 status with the index and worktree diffs; a plain `git diff`
cannot see untracked files.

Load `cargo metadata --format-version 1 --no-deps` and construct:

- package and target ownership for every workspace path;
- normal, build, and development dependency edges, from each package's declared
  dependencies matched against the workspace members;
- reverse dependencies;
- feature declarations and optional dependency edges;
- test, example, benchmark, binary, build-script, and library targets;
- workspace members and default members.

**Patches and excluded projects are not in that output.** `--no-deps` returns
`resolve: null`, and no `cargo metadata` output carries a `patch` key or the
workspace's `exclude` list at all. This repository has both —
`exclude = ["visualizer"]` and a `[patch.crates-io]` section in the root
`Cargo.toml` — so a model built only from `cargo metadata` would miss that a
patched dependency changed, and would report a changed path under `visualizer/`
as belonging to no package. Two separate sources close that, and they are not
the same source:

- **the workspace root manifest**, for the `[patch]` tables and the `exclude`
  list — the latter being only a list of *paths*;
- **each excluded package's own manifest**, queried the same way as any other:
  the root's `exclude` entry gives a path and nothing else, so the package name,
  its targets, its features and its dependencies come from running
  `cargo metadata` in that directory. Without it there is no ownership model for
  those paths, only the knowledge that they are outside the workspace.

Neither is covered by the "Cargo has provided the identity" rule below, and both
are named as sources in the report's assumptions.

`--no-deps` also means there is **no resolved dependency graph and no resolved
feature set**. Everything above is built from *declarations*, which is what
impact analysis needs; any later analysis that needs resolution must say so and
pay for a second invocation with dependencies.

Never infer package identity from a directory name when Cargo has provided the
identity.

### Risk classification

Classify each changed path and, once semantic change detection exists, each
changed item into zero or more named risk areas:

- `audio_realtime`
- `dsp_behavior`
- `engine_state`
- `serialized_contract`
- `stable_identity`
- `mcp_protocol`
- `gui_state`
- `feature_or_dependency_graph`
- `build_or_ci`
- `documentation`
- `developer_tooling`
- `core_v2`

Risk classification is additive. A project-format type used by MCP can carry
both `serialized_contract` and `mcp_protocol`; selecting only one would hide
required verification.

The initial path rules live in a checked-in, strictly deserialized policy file.
Semantic rules may refine them but cannot remove a conservative path-based risk
without an explicit policy decision.

### Check selection

Return two distinct sets:

1. `suggested_iteration_checks`: the smallest high-confidence checks useful
   while developing;
2. `required_before_commit`: the exact repository gate from `AGENTS.md`.

Examples of conservative selection rules:

- A crate source change selects that package's tests and Clippy targets, plus a
  **compile check of every reverse dependant** and the tests of those with
  integration targets exercising it. The compile check is not optional: until
  semantic change detection exists there is no way to tell an internal edit from
  one that breaks a consumer's build, and a consumer with no integration target
  would otherwise not be built at all.
- A public domain type or shared trait change selects all reverse dependants,
  not only the defining package.
- `Cargo.toml`, `Cargo.lock`, feature declarations, build scripts, or
  `.cargo/config.toml` select the no-default-features and all-features checks.
- A dependency-graph change since the latest release boundary reports that
  third-party license attribution must be evaluated before a release. It does
  not regenerate the attribution itself.
- Audio callback roots or reachable DSP/module code select `cargo perty audit
  rt` — **once that command exists**. In the first release it is not
  implemented, so the recommendation is rendered as an unavailable check with
  the reason, never as a command to run. A suggested command that cannot be
  executed teaches a reader to ignore the suggestions.
- Serialized project or protocol types select their round-trip and rejection
  tests once those tests are registered.
- MCP server changes select `synth_mcp` tests and the future MCP conformance
  suite; they do not silently call the live synth MCP server.
- CI, workspace, or toolchain configuration changes select the full feature and
  MSRV matrix.

Every recommendation includes machine-readable reasons that point back to the
changed paths, dependency edges, and policy rules that selected it.

### Impact report

The report contains:

- normalized change records;
- directly changed packages and targets;
- transitively affected packages with dependency paths;
- risk classifications with evidence;
- suggested commands with reasons;
- mandatory commands that remain before commit or push;
- assumptions, unknowns, and completeness.

**Elapsed time and cache use are diagnostics, and they are not in the canonical
JSON.** They cannot be: the same run is required to produce byte-stable JSON,
and a wall-clock reading differs every time while cache state differs between a
cold and a warm run of identical inputs. They go to **stderr only**, where the
human renderer already writes, and they are not in the JSON document at all —
not even under an excluded key, because a key whose value differs between two
identical runs makes the serialized bytes differ whatever the determinism
contract says about digests. A consumer that wants timings reads stderr or times
the process itself.

Command arguments are arrays in JSON, not pre-quoted command strings. The human
renderer performs shell-safe display quoting solely for copying.

## `audit rt`: real-time effect analysis

### Safety property

For every configured audio-thread root and every call reachable under the
analyzed feature configurations, the tool must either:

1. prove that the known direct and transitive effects are permitted;
2. report a forbidden effect with a source path and call chain; or
3. report the unresolved edge that prevents a complete answer.

The tool does not claim proof about behavior hidden behind FFI, inline
assembly, dynamically loaded code, or unanalyzed dependencies. Those edges
require a reviewed policy declaration or remain incomplete.

### Root and effect policy

Store roots by qualified Rust symbol, never by line number, collection index,
or display name. Core V2 roots are added as they become executable; the audit
may analyze V1 and V2 simultaneously.

**Neither CPAL callback has such a symbol today, and that is a prerequisite
rather than a detail.** Both are anonymous closures in
`crates/pertylizer/src/audio/backends/cpal_backend.rs` — one passed to
`build_output_stream`, one to `build_input_stream` — so there is nothing to name
that a rename or an edit above it will not move. Consequences:

- **Extract both closure bodies into named items** — free functions or inherent
  methods — before the audit is rooted. It is a one-time source change the tool
  depends on, it belongs to Phase 3, and it is the only disposition that makes
  the roots stable.
- **The input callback is a root too, not an afterthought.** CPAL runs it on a
  real-time thread and it does real work: it walks `chunks_exact`, converts each
  frame, and pushes into two ring buffers. Rooting only the output side would
  let the audit report `complete` while never traversing the input conversion or
  the ring-buffer calls — the same false clearance as rooting too late, one
  callback over.
- **Rooting at `AudioProcessor::process` instead is not an acceptable
  substitute**, and the reason is concrete rather than theoretical: the closure
  body calls `output_info.timestamp()` and `start_time.elapsed()` *before* it
  calls `process`. A wall-clock query is `Deny` in the table below, so a root
  placed after it would let the audit report a clean callback while skipping the
  one forbidden effect the callback actually contains.

**A configured root that does not resolve to exactly one item is an error, not a
warning.** Zero matches and several matches both make every "no forbidden
effects" result meaningless, and a real-time audit that can be silently rooted
at nothing is worse than no audit.

Classify direct effects at least as follows:

| Class | Examples | Default result |
|---|---|---|
| Heap growth/allocation | `Box::new`, `Vec::push`, `reserve`, `format!`, collecting into an allocating container | Deny |
| Blocking synchronization | `Mutex::lock`, `RwLock::read/write`, condition variables, blocking channels | Deny |
| System interaction | file/network I/O, process operations, sleeps, wall-clock queries | Deny |
| Diagnostics | logging, tracing, printing, panic hooks | Deny |
| Panic paths | `unwrap`, `expect`, explicit panic, and indexing whose bound is **proved violable** | Deny |
| Atomics and lock-free queues | approved atomic operations and reviewed ring-buffer APIs | Allow |
| Fixed-capacity operations | operations proven not to grow storage | Allow |
| Unknown/dynamic effect | unresolved call, escaping closure, unsupported macro, FFI | Incomplete |

**Indexing is only permitted when the bound is proved.** The bounds check exists
in the generated code whether or not the analyzer can decide it, so a rule that
denied only a *statically provable* panic would let every runtime-dependent
`slice[i]` vanish from a report that then called itself complete. Three
dispositions and no fourth, and the table above uses the same three:

- an **in-bounds proof** allows it;
- a **proved violation** denies it;
- **anything else is `Incomplete`** — not a pass, and not a deny either. It is a
  finding a reader must resolve, and a report containing one is not complete.

`Incomplete` rather than `Deny` for the unproved case is deliberate: the
codebase is full of indexing that is fine, and a rule that denied all of it
would be turned off within a week, which is the failure mode that ends with no
audit at all.

Method names alone are not enough. `clone`, `collect`, `lock`, and indexing have
different effects depending on the resolved type and implementation. Rules
that require type information must wait for semantic resolution instead of
guessing from text.

### Semantic provider

Use rust-analyzer as a subprocess through its LSP protocol rather than linking
unstable `ra_ap_*` implementation crates into the tool. The provider needs:

- definition and implementation resolution;
- outgoing call hierarchy;
- trait implementation expansion for dynamically dispatched module processing;
- macro expansion/source mapping where rust-analyzer exposes it;
- source ranges and qualified symbols;
- explicit diagnostics for unsupported or unresolved constructs.

Record the rust-analyzer and rustc versions in every report. The normal source
should be the rust-analyzer component corresponding to the repository
toolchain. If the executable is absent or incompatible, explain how to provide
it and exit incomplete; do not install it automatically.

Trait calls are especially important in `synth_modules`: a call through the
module-processing trait must expand to all implementations that the current
registry and feature configuration can select. Merely stopping at the trait
declaration would produce a false clean result.

Closures invoked synchronously inside a reachable function join its call graph.
An escaping closure, function pointer, or trait object whose target set cannot
be bounded is an unresolved edge. Calls into dependencies may be covered by a
reviewed effect summary; otherwise they remain unknown.

### Findings and exceptions

A finding contains:

- stable rule ID and severity;
- forbidden or unresolved effect;
- exact source location;
- root-to-effect call path;
- analyzed feature configuration;
- evidence used for classification;
- remediation text;
- whether an exception matched.

Exceptions live in policy, not source-code comments, and require:

- rule ID;
- qualified caller and callee or exact macro identity;
- source-evidence digest so an unrelated later call cannot inherit approval;
- concise English rationale;
- review date.

An unused or stale exception is itself a warning. Broad path-level exemptions,
wildcard callees, and “ignore all unknown calls” are not supported.

### Incremental mode

`audit rt --changed` uses `impact` to select roots whose reachable source or
policy changed. It is an iteration aid only. The full `audit rt` traverses all
roots and is the version suitable for CI.

## `gate`: one faithful quality entry point

`cargo perty gate commit` runs the repository's existing commands without
changing their arguments or weakening failures:

```text
cargo fmt --check
cargo build
cargo clippy --workspace --all-targets
cargo test --workspace
cargo doc --workspace --no-deps
```

`cargo perty gate push` runs the commit gate and then the feature/dependency
checks documented in `AGENTS.md`:

```text
cargo check --workspace --all-targets --no-default-features
cargo check --workspace --all-targets --all-features
cargo +1.97 check --workspace
```

The runner streams output, stops after a failed step by default, and reports
the exact failed command and exit status. An opt-in `--keep-going` may collect
independent failures. It must not run `codex review`, stage files, commit, tag,
push, regenerate licenses, or decide that a release is ready.

**Because it does not run `codex review --uncommitted`, it may not report a
clean result as the pre-commit gate.** That review is part of the procedure in
`AGENTS.md`, and it is the step that catches the class the build gate cannot see
— an unverified claim, a contradiction, a measurement without a control. A
success from this command therefore reports the review as an **outstanding
external step**, by name, in both renderers. A tool that lets a developer
believe the gate passed when a mandatory step was skipped is worse than no
runner.

The gate is deliberately a later phase. `impact` and `audit rt` contain the new
value; wrapping commands alone does not justify a project tool.

## Follow-on audits

### Domain boundary audit

`cargo perty audit domain` identifies public or cross-crate fields, parameters,
and return values where raw primitives appear to represent domain concepts. It
should combine syntax, resolved types, naming evidence, and an explicit catalog
of existing Pertylizer newtypes. Findings begin as warnings because intent
cannot always be inferred statically.

High-confidence rules include a raw numeric type paired with unit-bearing names
such as frequency, sample rate, MIDI note, gain, duration, tick, ID, index, or
position at a domain boundary. Loop counters and local intermediate arithmetic
are outside its scope.

### Serialized-contract audit

`cargo perty audit serde` starts from registered persisted and protocol root
types and checks that their reachable closed structs deny unknown fields,
required fields are not accidentally defaulted, numeric boundary types validate
their input, and deterministic collection policy is declared. It reports
missing round-trip, unknown-field, missing-field, and invalid-value tests when a
contract changes.

Do not flag every internal `Serialize` derive. The policy must identify which
types are contracts; otherwise the audit would turn a precise repository rule
into noise.

### MCP conformance

`cargo perty mcp check` eventually launches an isolated test instance, discovers
the synth MCP catalog, validates schemas and annotations, exercises registered
read-only and transactional probes, and verifies that mutations report their
effect and revision consistently. It must use a disposable project and never
connect to an arbitrary already-running user session by default.

Any issue found while this command itself uses the synth MCP server remains
subject to the repository's MCP-feedback rule: the exact call and unexpected
response must be surfaced rather than hidden inside a summary.

### Semantic context

`cargo perty context <RUST_SYMBOL>` may later expose the same semantic provider
to humans and agents: definitions, implementations, callers, callees, owning
package, nearby tests, feature gates, and relevant architecture policy. This is
useful only after the real-time audit has proven that the semantic layer is
accurate enough to trust.

## Policy contract

Place the checked-in policy beside the tool, for example
`tools/perty/perty-policy.toml`. Its first version contains:

- policy format version;
- workspace-relative real-time root symbols;
- dependency effect summaries;
- narrowly scoped reviewed exceptions;
- path-to-risk classifications;
- registered persisted/protocol roots in later phases;
- registered targeted tests where Cargo metadata alone is insufficient.

Deserialize through closed structs with `deny_unknown_fields`. Missing,
unknown, duplicate, or invalid values are hard errors. Paths are normalized but
must remain inside the workspace. Symbols must resolve uniquely. Sort policy
entries when generating diagnostics so filesystem or hash-map order cannot
change output.

There is no automatic “update baseline” command in the initial release.
Changing policy is a normal reviewed source edit. This prevents a failing audit
from being silenced by regenerating a large opaque snapshot.

## Implementation phases

### Phase 0 — measurements and executable specification

- Record representative timings for package tests, the full gate, Cargo
  metadata, and rust-analyzer workspace loading.
- Identify the actual CPAL callback and engine-processing roots for V1 and the
  currently executable V2 path.
- Hand-trace several known-safe and intentionally unsafe fixture call chains.
- Define JSON schemas, exit behavior, ordering, and completeness semantics
  before implementing collectors.
- Add fixture expectations for paths with spaces, renames, untracked files,
  detached HEAD, and an unavailable upstream branch.

Exit: the tool's claims and failure behavior are testable without relying on
prose in this plan.

### Phase 1 — CLI and report foundation

- Add `tools/perty`, the Cargo alias, typed errors, output envelope, and human
  and JSON renderers.
- Add provider traits for Git, Cargo metadata, semantic queries, and process
  execution.
- Implement strict policy loading and workspace-root discovery.
- Establish deterministic ordering, stdout/stderr separation, and exit-code
  tests.

Exit: `cargo perty --help` works from any directory of the workspace it belongs
to — excluded packages such as `visualizer/` select their own workspace and are
outside the claim — and a no-op diagnostic command produces byte-stable JSON.

### Phase 2 — useful `impact`

- Implement worktree, staged, and revision-range collection.
- Build the workspace/reverse-dependency/target model from Cargo metadata.
- Implement path risk classification and conservative check selection.
- Render dependency paths and reasons for every recommendation.
- Dogfood it against a sample of historical commits touching DSP, GUI, MCP,
  manifests, documentation, and Core V2.

Exit: historical-change tests show no false “unaffected” result for the chosen
sample, and untracked or renamed files cannot disappear from the report.

### Phase 3 — direct real-time rules

- **Extract both CPAL callback bodies — output and input — into named items**, so
  the audit has root symbols that exist. Nothing else in this phase works
  without it, and leaving the input closure anonymous either fails root
  resolution or drops its conversion and ring-buffer path from the result.
- Register and resolve real-time roots, failing on any root that does not
  resolve to exactly one item.
- Parse reachable root bodies and implement high-confidence direct rules for
  explicit allocation, locks, I/O, diagnostics, and panic constructs.
- Report all calls that still need semantic resolution as unknown.
- Add the effect-policy and exception model without permitting broad waivers.

Exit: the command already catches direct violations, but reports `partial`
until semantic traversal is implemented. It must not yet be used as a clean CI
gate.

### Phase 4 — semantic call graph and transitive effects

- Implement the rust-analyzer LSP client with bounded startup and request
  timeouts.
- Resolve calls, trait implementations, closures, macro locations, and source
  ranges.
- Propagate effect summaries to a fixed point and retain a predecessor path for
  each root/finding pair.
- Add feature-configuration analysis, dependency summaries, and cache keys.
- Compare selected results manually with compiler output and source inspection.

Exit: all configured roots are either complete or explicitly incomplete, a
fixture violation several calls deep reports the whole path, and dynamically
dispatched module implementations cannot be omitted silently.

### Phase 5 — adopt the real-time gate

- Run the full audit on the current repository and classify every finding
  before changing enforcement.
- Fix genuine violations separately from tool implementation changes.
- Review and report proposed exceptions individually.
- Measure cold and warm runtimes and choose whether the full audit belongs in
  every push workflow or a dedicated quality job.
- Update `AGENTS.md` and CI only after the audit is complete and stable.

Exit: `cargo perty audit rt` is fail-closed, has an owned policy baseline, and
passes in CI without undisclosed unknown edges.

### Phase 6 — gate runner and optional audits

- Add `gate commit` and `gate push` as faithful orchestration.
- Prioritize `audit domain` or `audit serde` from measured review misses rather
  than implementing both automatically.
- Build MCP conformance only when an isolated server fixture exists.
- Expose `context` only after the semantic API is stable.

Exit: each added command has a demonstrated repository failure class it catches
and does not merely duplicate an existing command.

## Verification strategy

### Unit tests

- NUL-delimited Git parsing, including rename pairs and unusual paths.
- Cargo dependency and reverse-dependency construction.
- Risk-rule composition and command selection.
- Strict policy and JSON contract acceptance/rejection.
- Deterministic sorting and rendering.
- Effect propagation, cycles, multiple roots, and shortest explanatory paths.
- Exception matching, stale evidence, and unused exceptions.

### Fixture repositories

Keep small Rust workspaces under the tool's test fixtures. Cover:

- normal, development, build, optional, and target-specific dependencies;
- default, no-default, and all-feature configurations;
- trait dispatch with multiple implementations;
- generic methods, closures, function pointers, macros, and async boundaries;
- direct and transitive allocation, locks, logging, I/O, and panics;
- approved atomics, fixed-capacity buffers, and lock-free queues;
- unresolved FFI or dynamic targets producing `partial`, never `complete`;
- a policy exception that stops matching after its evidence changes.

Integration tests create disposable Git repositories in temporary directories.
They must not mutate the Pertylizer checkout or depend on the user's global Git
configuration.

### Repository dogfooding

Before enforcement, run `impact` over representative historical commits and
compare its recommendations with the checks that exposed actual failures. Run
the RT audit against reviewed callback paths and seed temporary fixture-only
violations to prove each rule fires.

Do not add a large snapshot declaring the current repository clean. Tests need
positive controls: at least one known violation for every deny rule and at
least one similar permitted operation that must remain clean.

### Performance budgets

Measure rather than guess final limits. Initial targets are:

- warm `impact --worktree`: fast enough for every edit cycle;
- cold `impact`: dominated by Cargo metadata, not compilation;
- warm `audit rt --changed`: suitable during implementation;
- full cold RT audit: suitable for CI even if it is too slow for every edit.

Reports include timings, but elapsed time never changes findings or ordering.

## Rollout and maintenance

- Land each phase as a coherent change with the full repository gate and
  uncommitted review required by `AGENTS.md`.
- Keep the tool advisory until its own report says `complete` on the relevant
  analysis.
- Introduce warning-level CI before deny-level CI for new heuristic rules.
- Treat rust-analyzer upgrades as analyzer changes: run fixture and repository
  comparison tests before accepting changed results.
- Version the JSON and policy formats independently. Active development permits
  breaking them, but a version change must be explicit rather than silently
  reinterpreting stored output.
- Delete obsolete rules when architecture changes; do not preserve V1 roots
  after V1 is removed merely for compatibility.

## Risks and mitigations

### False confidence from an incomplete call graph

This is the primary risk. Fail closed, expose completeness, expand trait
implementations, and make unresolved calls visible. Do not label a source-only
scan as semantic analysis.

### Excessive false positives

Start with direct high-confidence rules, keep heuristic domain findings at
warning level, show evidence and paths, and permit only narrow reviewed
exceptions. A noisy rule does not become useful by forcing a large allowlist.

### rust-analyzer protocol or behavior drift

Use standard LSP requests where possible, isolate the provider behind a trait,
record versions, maintain semantic fixtures, and treat unsupported behavior as
incomplete. Avoid linking rust-analyzer implementation crates.

### Tool becomes another source of architecture truth

The application's types and code remain authoritative. Policy records roots,
effects, and verification mapping only; it must not reproduce module catalogs,
project schemas, or dependency declarations that can be discovered from code
and Cargo metadata.

### The tool slows the existing default workflow

Keep it out of `default-members`, cache only disposable semantic data, provide
changed-only iteration modes, and defer CI enforcement until timings are
measured. The existing bare `cargo run` behavior must not change.

## Acceptance criteria for the first useful release

Phases 1 and 2 constitute the first useful release when:

- `cargo perty impact` correctly includes staged, unstaged, untracked, renamed,
  and deleted files;
- every affected package and recommended check has an inspectable reason;
- the report distinguishes direct changes from reverse-dependency impact;
- manifest and feature changes select the feature matrix;
- human and JSON outputs represent the same typed report;
- output is deterministic and JSON is strictly versioned;
- malformed policy or Git input fails clearly without partial defaults;
- the output repeats that targeted checks do not replace the pre-commit gate;
- workspace tests include the tool even though it is not a default member.

**The second release is the real-time audit**, delivered by Phases 3 to 5, and
it is production-ready only when it additionally shows that:

- every configured audio root resolves uniquely;
- direct and transitive forbidden effects report a complete call path;
- trait-dispatched module implementations are included;
- unresolved calls make the analysis incomplete and non-zero;
- exceptions are narrow, justified, evidence-bound, and reported;
- positive and negative controls exist for every deny rule;
- the current Pertylizer callback graph has been manually reviewed against the
  tool's result before CI enforcement.
