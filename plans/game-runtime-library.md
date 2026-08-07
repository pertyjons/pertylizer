# Pertylizer Game Runtime Library

## Status

Proposed, and reviewed once against the code at `e546e2b5`. This plan
deliberately starts with a small, usable runtime instead of trying to deliver
the complete adaptive-audio vision in the first release.

Two things must be decided by a human before Phase 1 starts, because they change
what gets built rather than how:

- **Format compatibility policy** — see "Asset loading". Publishing a runtime is
  incompatible with `CLAUDE.md`'s "no backward compatibility required" for one
  artifact: project files embedded in shipped games.
- **Crate renaming scope** — whether the twelve `synth_*` crates get branded
  names before publication, or whether only the facade is branded.

## Objective

Publish a Rust library that lets games and other applications play Pertylizer
projects and trigger sound effects on macOS, Linux, Windows, Android, and iOS.
The library must use the exact same `SynthEngine`, sequencer, modules, sampler,
and DSP implementation as Pertylizer Studio. There must never be a second player
engine whose behavior has to be synchronized with the editor.

The first useful release should prove one promise:

> A host application can load a Pertylizer project, render its audio into a
> host-owned buffer, control transport and parameters, and trigger musical or
> sound-effect instruments through the same engine used by Pertylizer Studio.

## Guiding decisions

1. **One engine.** `synth_engine::SynthEngine` remains the sole real-time audio
   engine. The runtime is a control, asset-loading, and host-integration facade.
2. **Host-driven audio first.** The host supplies the output buffer from its own
   audio callback. A CPAL-owned stream is optional and is not a default feature.
3. **Prepare off the audio thread.** File access, archive decompression, JSON
   parsing, sample decoding/resampling, script compilation, graph construction,
   and allocation happen before playback or on a control thread.
4. **Small public surface.** Games depend on one facade crate. The existing
   lower-level crates remain implementation layers rather than the recommended
   user API.
5. **Bytes before paths.** Runtime assets must load from `&[u8]` or a reader so
   they work with Android APK assets, iOS application bundles, embedded assets,
   and game-engine asset systems.
6. **Type-safe API.** All domain values use existing or new newtypes, including
   sample rate, block size, cue IDs, music-state IDs, tempo scale, and scheduling
   offsets.
7. **The facade owns the engine's load and maintenance protocol.** Loading a
   project and reclaiming retired audio objects are not incidental details a
   host can be asked to reproduce; see "Engine realities the facade must absorb".

## Engine realities the facade must absorb

Three properties of the current engine shape the API more than anything else in
this plan. They are stated up front because every phase below depends on them.

### Loading is a live-engine protocol, not a parse

`crate::project_apply::apply_project` does not build a value and hand it over.
It *sends commands* into `SynthEngine`'s bounded ring while the engine is being
processed, and the load is complete only when
`synth_engine::CommandSync::processed() >= enqueued()`. `render/headless.rs`
already shows the full dance: spawn a loader thread, spin `engine.process()` on
a throwaway 44.1 kHz/256-frame stream until the loader finishes and the ring
drains, run `SETTLE_BLOCKS` more blocks so deferred work (voice teardown, graph
rebuilds) completes, and fail on a wall-clock deadline if it never settles.

Consequences for the facade:

- `RuntimeAsset::from_bytes` cannot be a pure parse that `build()` later
  installs. Either `build()` performs the whole drain internally on the calling
  (loading) thread before yielding the `AudioRenderer`, or the split has to be
  redrawn. The MVP should do the former: `build()` is blocking, allocating, and
  explicitly documented as a loading-screen operation.
- The 300-second wall-clock deadline and the `std::thread` spawn in
  `render/headless.rs` are a CLI's assumptions. The runtime needs a bounded,
  configurable settle budget, and it should be able to drain on the calling
  thread rather than requiring a second thread on platforms where spawning one
  during load is unwelcome.
- Reloading or swapping a project at runtime repeats this protocol. It is a
  control-thread operation with a live renderer running elsewhere; the MVP
  should either forbid it or define it precisely, not leave it implied.

### Something must pump the control thread

Retired instruments, modules, and songs are handed back from the audio thread on
return rings (`DroppedItem`, `DeferredControlDrop`) and destroyed by
`EngineHandle::cleanup_dropped_modules`. `DeferredDropSlots` reserves a return
slot *before* a pointer-swap command is enqueued, so if nothing ever drains the
return rings, command submission starts failing rather than leaking silently.

The three-role split below therefore needs a fourth obligation: the `Controller`
must expose a maintenance call the game invokes from its update loop (naming it
`pump` or `update` is fine), or the runtime must own a control thread that does
it. This is MVP scope, not a nicety.

### Studio's live engine and the offline renderer are two engine instances

`render/headless.rs` keeps the loaded engine alive only so the offline renderer
can *snapshot its instruments and build its own `SynthEngine`*. "The same
engine" is true at the type and code level, not at the instance level. The
parity criterion has to be written against that reality.

## Architecture

```text
Pertylizer Studio ---------+
Rust game -----------------+--> runtime/project facade --> SynthEngine
Mobile application --------+                              /     |      \
Optional CPAL adapter ------+                        sequencer modules  DSP
```

Pertylizer Studio must adopt the extracted project-loading/runtime code. This is
what makes parity structural rather than relying only on duplicated integration
tests.

### Proposed packages

The MVP does not require publishing a large set of independently promoted APIs.
It adds one user-facing package and extracts only the shared code it needs:

- `pertylizer-runtime`: public facade for asset preparation, rendering, control,
  cues, and events.
- A small shared project/runtime layer, either inside `pertylizer-runtime` or in
  `pertylizer-project` if the dependency boundary is cleaner.
- The existing core, DSP, sampler, sequencer, module, script, and engine crates,
  renamed to branded package names before crates.io publication if necessary.
  Note the size of that "if necessary": the workspace uses `synth_*` snake_case
  names in twelve crates and every `use synth_core::…` in the tree. Renaming is
  mechanical but touches everything, so decide it once, in Phase 5, and prefer
  the cheapest option that works — publishing under the existing names, or
  renaming only the crates that actually reach crates.io.
- `synth_osc_protocol` is a **mandatory** dependency of `synth_engine` (with the
  `serde` feature), so it is part of the published set even though nothing about
  it is game-facing. Either publish it or fold its constants into `synth_core`.
- The Pertylizer GUI application remains an application package and should use
  `publish = false` if the `pertylizer` crates.io name is assigned to the public
  library facade. Confirm the name is actually available before planning around
  it.

All path dependencies that are published must also carry registry version
requirements, for example:

```toml
pertylizer-engine = { path = "../pertylizer-engine", version = "=0.1.0" }
```

Use lockstep versions for the Pertylizer crate family in the initial `0.x`
releases. Publish dependency leaves before the facade crate.

## MVP scope

### Asset loading

- Load a sample-free Pertylizer project from bytes.
- With the `bundle` feature, load the current ZIP-based project bundle and its
  embedded WAV samples from bytes.
- Validate the project format version and report typed errors.
- Rebuild derived note/mod graph state and compile scripts off the audio thread.
- Hydrate sampler modules through the same path used by Studio and offline
  rendering.
- Reject or strip GUI-only visualizer modules with an explicit warning.
  `SynthSession` already does this (`SessionError::VisualizerRequiresGui`), so
  the work is surfacing it as a warning rather than an error and proving the
  removal is audio-neutral — a visualizer that ever touched the signal would
  make headless output differ from Studio's.

The first release does not need a new game-specific file format. A `.ptygame`
export can be introduced later after actual runtime use reveals what deployment
metadata is needed.

`ProjectFile` already carries `file_type: "project"` and a `FORMAT_VERSION`
string bumped only on breaking changes, so version validation has something real
to check.

#### Format stability is a policy decision, not an implementation detail

`CLAUDE.md` states the project is in active development with **no backward
compatibility required** — APIs and formats break freely. Publishing a runtime
contradicts that for one specific artifact: a shipped game embeds project bytes
and a pinned `pertylizer-runtime` version, and cannot re-export its assets when
Studio's format moves.

Pick one before `0.1.0` and write it into the crate's README:

1. **Loader compatibility.** The runtime accepts a documented range of
   `FORMAT_VERSION` values and Studio keeps loading old projects. Costs
   migration code; gives games a stable target.
2. **Lockstep only.** The runtime loads exactly the format its matching Studio
   version writes, and every Studio bump requires re-exporting game assets. Free
   today, unpleasant for anyone shipping.

Option 2 is defensible for a `0.x` beta but must be stated loudly rather than
discovered by a user. Either way, an unknown or out-of-range version must fail
with a typed error naming both versions, never load partially.

### Rendering and lifetime model

Construction returns three roles:

- `AudioRenderer`: owned exclusively by the audio thread and responsible for
  filling interleaved output buffers.
- `Controller`: clonable control-thread handle that submits bounded commands
  **and performs periodic maintenance** (see below).
- `EventReceiver`: game-thread receiver for transport, cue, warning, and error
  events.

The core entry point is host-driven:

```rust,ignore
// Blocking: parses, hydrates samples, compiles scripts, and drives the
// engine's load protocol to completion. Call it from a loading screen.
let asset = RuntimeAsset::from_bytes(PROJECT_BYTES)?;
let (mut renderer, controller, events) = Runtime::builder()
    .sample_rate(DeviceSampleRate::DVD_QUALITY)
    .max_block_size(BlockSize::B512)
    .build(asset)?;

controller.play()?;

// Game update thread, every frame:
controller.pump();          // drains events and reclaims retired audio objects

// Audio callback thread:
renderer.process_interleaved(output, callback_context);
```

`BlockSize` exposes `B64`/`B128`/`B256`/`B512`/`B1024`; there is no `MEDIUM`.
`DeviceSampleRate::DVD_QUALITY` does exist and is the right type — the engine
receives its rate through `AudioProcessor::on_stream_start(&StreamInfo)`, so the
builder's job is to synthesize a `StreamInfo` from the host's real callback
configuration rather than to invent a new configuration path.

`AudioProcessor::process(&mut [f32], &AudioCallbackContext)` is already
interleaved, so `process_interleaved` is a rename for clarity, not new
plumbing. The facade should still define its own method rather than re-export
the trait, whose `on_error`/`on_stream_start`/`on_stream_stop` hooks are backend
lifecycle concerns a game host does not implement.

The renderer must accept varying callback sizes up to the configured maximum.
It must not own a file system, window, async executor, or platform audio session.

#### `Controller::pump` is required, not optional

`pump` is what calls `EngineHandle::cleanup_dropped_modules`. If the game never
calls it, retired instruments and songs are never destroyed, the return rings
fill, and `DeferredDropSlots` refuses to reserve — at which point control
commands start failing rather than leaking silently. A runtime that lets a host
forget this has a designed-in failure mode.

Two acceptable designs; pick one in Phase 2 and document it:

- **Explicit:** `Controller::pump()` is part of the documented update-loop
  contract, and the runtime warns (via an event) when it has not been called for
  a long time.
- **Owned thread:** the runtime spawns a low-frequency maintenance thread.
  Simpler for hosts, but adds a thread the game did not ask for and is a poor
  fit for single-threaded/WASM-shaped hosts.

#### Events are game-shaped, not engine-shaped

`EngineEvent` is a GUI-facing enum: waveform data behind a `Vec<f32>`, envelope
stage bytes, CPU usage, parameter echoes. Do not re-export it. The facade
defines its own event enum covering transport, cue lifecycle, warnings, and
errors, and translates. This also keeps `EngineEvent` free to change without
breaking a published API.

#### Do not route per-frame control through `SharedSong`

`SharedSong::write()` clones the whole `Song` to publish a new snapshot. That is
correct for editing and wrong for a game changing an intensity parameter every
frame. Live control must go through the engine command ring; `SharedSong`
mutation is a load-time and structural-change path only.

### Transport and live control

- Play, pause, stop, rewind, seek, and loop. All of these already exist as
  `EngineCommand` variants (`Play`, `Pause`, `Stop`, `Rewind`, `Seek { tick }`,
  `SetLoop`, `SetTempo(Bpm)`), so this layer is a typed facade over commands
  that work, not new engine behavior.
- Read current tick, playback state, peak levels, and active voice count.
- Set master volume and instrument volume/pan/mute/solo.
- Set module parameters through the existing typed parameter system.
- Send note on/off, pitch bend, mod wheel, aftertouch, and control changes.
- Introduce a runtime `TempoScale` or equivalent override rather than rewriting
  the authored song tempo map. The authored tempo map remains the musical source
  of truth; the runtime scale controls gameplay pacing on top of it. The map is
  real — `Song` holds `default_tempo` plus a `Vec<TempoChange>` with optional
  linear ramps — so the scale must compose with ramp segments, not just with a
  single BPM.

The tempo override must initially document its sample behavior: synthesized
notes retain pitch, while tempo-synchronized sample loops require later
time-stretch support to retain pitch.

### Sound-effect cues

The MVP cue layer maps stable names to preloaded Pertylizer instruments and
triggers them through the normal engine voice path. A cue can therefore be a
sampled sound, a procedural patch, or a layered synth/sample instrument.

Initial cue support:

- `CueId` and `CueInstanceId`.
- Trigger a cue with velocity, gain, pan, pitch, and optional destination bus.
- Stop one instance or all instances in a cue group.
- Allow overlap or restart behavior.
- Apply a configured per-cue voice limit and deterministic voice stealing.
- Report started, finished, rejected, and stolen events.

Commands from a game update currently reach the next audio callback. Precise
within-buffer timing is a follow-up engine feature needing a **new** newtype —
an absolute engine-frame timestamp, or a within-block offset. Do not reach for
the existing `synth_core::SampleOffset`: it is an `f32` delay-line offset used
for the spatial panner's ITD and early reflections, and reusing it for
scheduling would conflate two unrelated quantities in the type that is supposed
to keep them apart. No raw frame counters should be exposed as domain values.

## Dependency budget

Users should normally declare only one dependency:

```toml
[dependencies]
pertylizer-runtime = "0.1"
```

The current `synth_engine` graph uses twelve direct third-party dependency
families across its internal engine crates and resolves to exactly 46 external
package nodes on the current Linux development target (`cargo tree -p
synth_engine --edges normal`), including transitive libraries and proc-macros.
Bundle support currently adds approximately fourteen new ZIP transitive nodes.
The exact count varies by target and enabled features.

Worth knowing where the mass is: the `realfft`/`rustfft` family alone accounts
for roughly eight nodes (`num-complex`, `num-integer`, `num-traits`,
`primal-check`, `strength_reduce`, `transpose`, …), and `schemars` pulls
`schemars_derive`, `serde_derive_internals`, `dyn-clone`, `ref-cast`, and
`serde_json`. Gating `schemars` is therefore the single largest reduction
available — another reason to treat it as its own step rather than a footnote.

After a small feature-boundary cleanup, the default host-driven runtime should
use approximately these ten direct third-party dependencies across the complete
Pertylizer runtime stack:

| Dependency | Purpose |
|---|---|
| `serde` | Serializable project and engine types |
| `serde_json` | Pertylizer project decoding |
| `thiserror` | Typed errors |
| `ringbuf` | Bounded real-time command/event channels |
| `arc-swap` | Lock-free song snapshots |
| `parking_lot` | Control-thread synchronization |
| `arrayvec` | Fixed-capacity real-time storage |
| `static_assertions` | Compile-time thread/layout invariants |
| `realfft` | Spectral modules and effects |
| `strum` | Exhaustive module-type enumeration |

Optional features add:

- `bundle`: `zip` and `hound` for current Pertylizer bundles and WAV samples.
- `cpal`: `cpal` plus platform-specific audio backend dependencies.
- `schema`: `schemars`, for authoring/schema tools rather than game playback.
- `authoring`: `fastrand`, for destructive/random humanization operations that
  are not needed to play prepared projects.

To reach this boundary:

- Make `schemars` derives conditional or isolate schema-bearing wire types so
  game builds do not compile schema generation by default. **This is the
  expensive one, not a small cleanup.** `JsonSchema` derives are spread over
  roughly two dozen files in `synth_core`, `synth_sequencer`, and
  `synth_engine`, plus `ProjectFile` itself, and each needs a
  `cfg_attr(feature = "schema", …)` or a wire-type split. Budget it as its own
  step with its own review; the existing CI `cargo check --workspace
  --all-targets --no-default-features` is the natural gate.
- Make WAV file I/O optional in `synth_sampler`; its in-memory sample/player
  types must not require `hound` when a host supplies decoded PCM. This one is
  genuinely small: `hound` is confined to `synth_sampler/src/wav.rs` and one
  `SamplerError` variant.
- Keep `fastrand` out of the runtime default. Verified: the sequencer's only
  `fastrand` use is `Pattern::humanize_notes`, a destructive editing operation
  on a note selection. Playback-time humanization is `NoteProcessor::Humanize`,
  which is seeded and deterministic. Feature-gating `fastrand` therefore costs
  nothing in playback fidelity.
- Do not include GUI, MCP, Tokio, Axum, OSC, MIDI, Clap, Rayon, file dialogs,
  image support, or application configuration in the facade's default graph.

Do not replace `ringbuf`, `arc-swap`, `parking_lot`, or `realfft` merely to lower
the numerical dependency count. They implement substantial correctness or DSP
behavior, and replacing them would create more critical code to maintain.

## Platform strategy

### Default: host-owned callback

Host-driven rendering is the portable contract on every platform. A Rust game
engine, native application, or foreign-language wrapper can call the renderer
from its existing audio callback. This avoids competing audio devices and lets
the host own mobile permissions, audio-session categories, interruptions,
background/resume behavior, and route changes.

### Optional CPAL adapter

Provide a separate optional adapter for applications that want Pertylizer to own
the audio stream. It may be used by Studio, examples, and simple standalone
programs. It is not required for the core runtime and must not be a default
feature.

### Verification targets

CI should compile and test, where executable runners exist, at least:

- Linux x86-64.
- Windows x86-64 MSVC.
- macOS x86-64 and ARM64.
- Android ARM64 and emulator x86-64.
- iOS ARM64 and simulator.

This is new CI, not a config tweak. Today `quality.yml` is a single
`ubuntu-latest` job with no target matrix, and `build.yml` covers the three
desktop platforms but runs **only on pushed `v*` tags**. A runtime target matrix
needs its own workflow, plus Android NDK and Xcode toolchain setup, and it will
dominate CI time. Add it in Phase 5 alongside packaging — earlier phases can
rely on local cross-compilation checks.

Cross-compilation is only the first gate. Real Android and iOS smoke tests must
cover interruption, background/resume, route changes, headphones/Bluetooth,
and callback-size changes before mobile support is described as production
ready.

Also note that the workspace release profile sets `panic = "abort"`. Profiles do
not propagate to downstream consumers, so a game chooses its own — but the
runtime must not rely on unwinding to contain a fault in the audio callback.
The existing "no `unwrap`/`expect` in production code" rule is the actual
defence, and it applies with more force here than in Studio: a panic in a game's
audio callback takes the game down.

## Implementation phases

### Phase 1: Extract the reusable loading path

Start from what already holds: `render/headless.rs` loads projects through
`project_apply::load_file_into_engine`, the same function Studio uses, so
"one loading path" is already true *in code*. What is not true is that the path
is *reusable outside the application crate* — it lives in `crates/pertylizer`
next to the GUI, and it is path-based throughout.

1. Add the runtime facade crate to the workspace.
2. Move `ProjectFile`, `patch`, `module_factory`, `SynthSession`,
   `mod_grid_build`, sample hydration, `bundle`, and `project_apply` out of
   `crates/pertylizer` into a crate with no GUI/MCP dependency. This is roughly
   7 800 lines across eight files. The good news from a survey of those files:
   the only `crate::gui::` references in `project_apply.rs` are inside its
   `#[cfg(test)]` module, so the production code is already GUI-free and the
   move is mostly mechanical — the test module is what needs rehoming.
3. Replace the path-based entry points with byte/reader ones and keep thin
   path-based wrappers for Studio and the CLI. `project::load_file` currently
   does `fs::read_to_string` + `serde_json::from_str`, and `bundle::load_bundle`
   takes a `&Path` and sniffs ZIP by reading the file — both need a bytes form,
   including format sniffing from a byte slice rather than from a file.
4. Generalize the load-drain loop from `render/headless.rs` into a reusable,
   configurable operation (settle budget as a parameter, no hardcoded 300 s
   deadline, no mandatory thread spawn), and make the `render` command use it.

Exit criterion: Studio, `pertylizer render`, and a headless test in the new
crate all load the same project from the same reusable function — one of them
from bytes — and reach equivalent engine snapshots.

Risk to watch: this is a large move inside a crate that also holds the GUI, and
the `#[cfg(test)]` blocks are the part most likely to fight back. Keep it a pure
move plus re-exports in one commit, with behavior changes in separate ones.

### Phase 2: Host-driven player MVP

1. Implement `RuntimeAsset`, `RuntimeBuilder`, `AudioRenderer`, `Controller`, and
   `EventReceiver`, with `build()` owning the engine load protocol end to end.
2. Decide and implement the maintenance contract (`Controller::pump` or an owned
   maintenance thread) so `cleanup_dropped_modules` always runs.
3. Expose transport, parameter, note, and meter operations.
4. Define the runtime's own event enum and translate `EngineEvent` into it.
5. Define bounded queue/backpressure errors; never silently lose a requested
   game action. `synth_engine::CommandSync::dropped` already makes a dropped
   command observable — build the typed failures on top of it rather than
   inventing a parallel mechanism.
6. Add a minimal example that renders through a host-owned callback/buffer.

Exit criterion: a sample-free Pertylizer song plays without GUI, MCP, MIDI,
CPAL, or file-system access in the audio path, and a test that never calls the
maintenance entry point fails loudly rather than degrading quietly.

### Phase 3: Samples and cues

1. Add optional bundle/WAV decoding and preloading.
2. Add named cue definitions and cue-to-instrument resolution.
3. Add overlap/restart policy, voice limits, and lifecycle events.
4. Demonstrate one procedural shot and one sampled explosion.

Exit criterion: music and overlapping effects render through one `SynthEngine`
without audio-thread allocation.

### Phase 4: Tempo override and first adaptive controls

1. Add a runtime tempo scale with bounded/smoothed changes.
2. Add named game parameters that map to existing instrument/module controls.
3. Add track/layer fades suitable for simple exploration/combat intensity.
4. Emit beat, bar, and arrangement-section events.

Exit criterion: a host can raise intensity, change tempo, and enable a musical
layer while playback continues.

### Phase 5: Packaging and public beta

1. Rename/reserve branded crate package names as needed, after confirming which
   names are actually free on crates.io.
2. Add versioned path dependencies, README files, repository metadata, examples,
   feature documentation, and an explicit verified MSRV. The workspace declares
   `rust-version = "1.97"` and `edition = "2024"`; "verified" means a CI job
   actually builds the published set at that toolchain, as `quality.yml` already
   does for the workspace.
3. Settle the format-compatibility policy (see "Asset loading") and state it in
   the runtime crate's README.
4. Add the cross-target verification workflow.
5. Refresh third-party license attribution. Publishing a library redistributes
   its dependency graph, so `THIRD-PARTY-LICENSES.md` becomes a shipping
   obligation rather than an internal courtesy; regenerate it for the runtime's
   default feature set, not just for the application's.
6. Run `cargo package` and `cargo publish --dry-run` for every public crate in
   dependency order.
7. Publish lockstep `0.1.0` crates and a small integration example.

## Deferred capabilities

These are valuable but explicitly not required for the first release:

- Quantized music-state transitions at beat/bar/section boundaries.
- Stingers, fills, pickups, transition segments, and musical outros.
- A `.ptygame` deployment format and asset compiler.
- Tempo-aware, pitch-preserving sample-loop time stretching.
- Multiple output buses and host-side stem access.
- 3D emitters, listener poses, distance attenuation, and occlusion.
- Dialogue-aware ducking and priority policies.
- Streaming long music/sample assets instead of preloading them.
- Unity, Godot, Unreal, C ABI, Swift, Kotlin, or JNI wrappers. Any FFI work must
  be isolated in a separate crate and discussed before introducing `unsafe`.

## Verification and acceptance criteria

The project-wide formatting, build, lint, and workspace test commands remain
mandatory. Runtime work also needs:

1. **Engine parity:** the same prepared project, input events, sample rate, and
   block sequence produce identical output in Studio's engine path, offline
   rendering, and `pertylizer-runtime`, or a documented tight tolerance where a
   target's floating-point behavior prevents byte equality. Scope this honestly:
   these are three *instances* of `SynthEngine` built from the same snapshots,
   not one shared instance, so the test must configure all three identically
   (sample rate, block size, master volume, mute/solo state) before comparing.
   Anything that legitimately differs — a stripped visualizer, a GUI-only
   default — must be listed and justified as audio-neutral, not silently
   absorbed into the tolerance.
2. **Real-time safety:** an allocation guard covers the facade, cue dispatch,
   tempo override, and event emission during `process_interleaved`. Note that
   the existing guard cannot simply be pointed at the new crate: it is a
   `#[cfg(test)]` module inside `synth_engine` that installs a
   `#[global_allocator]`, and only one of those can exist per test binary. Move
   the harness into a shared test-support crate (or accept a deliberate copy)
   before claiming coverage.
3. **Variable buffers:** render correctly at common sample rates and all callback
   sizes up to the configured maximum, including irregular sizes.
4. **Backpressure:** command/event queue saturation is deterministic and
   observable; control calls return typed failures rather than silently dropping
   work.
5. **Stress:** repeated overlapping effects respect voice limits without leaks,
   panics, unbounded work, or audio-thread destruction.
6. **Asset isolation:** loading and replacing assets performs no file I/O,
   decompression, parsing, compilation, resampling, or heap allocation on the
   audio thread.
7. **Feature isolation:** a default runtime build contains no GUI, MCP, network,
   MIDI, OSC, command-line, or platform audio-backend dependencies.
8. **Maintenance contract:** a long-running test that repeatedly installs and
   retires instruments while rendering neither exhausts `DeferredDropSlots` nor
   destroys anything on the audio thread — and the same test with the
   maintenance call removed fails.
9. **Format handling:** a project written by a newer or unknown `FORMAT_VERSION`
   fails with a typed error naming both versions, and never loads partially.

The MVP is complete when a small Rust host can embed a Pertylizer project, play
its arrangement, alter tempo and parameters, trigger procedural and sampled
effects, and render everything through the same `SynthEngine` that Studio uses.
