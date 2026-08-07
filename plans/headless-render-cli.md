# Headless project render CLI

> **Status:** implemented and squash-merged to `main`, 2026-08-06. The
> remaining open work lives in `plans/TODO.md` §5.6; this document is kept as
> the record of *why* the version-1 contract looks the way it does.
>
> **Planning baseline:** Pertylizer `3e25e679`, 2026-07-29.
>
> **Revised 2026-08-06** at `2d21a0cb`: the `--tap` enum is replaced by track
> mute/solo. See [Selecting what is rendered](#selecting-what-is-rendered) for
> why, and [Argument parsing](#argument-parsing) for the `clap` decision.
>
> **Implemented 2026-08-06.** Decisions taken during the work, beyond what is
> written below:
>
> - **`--seed` and `--normalization` are not in version 1** (see below).
> - **The render is always the full chain** — master and return effects
>   included — because the command's job is to produce the file the project
>   sounds like. The analyzers default the other way, and for the opposite
>   reason. There is no flag to change it in version 1.
> - **`--seconds` is capped at 300 s**, the offline renderer's existing ceiling.
>   The renderer would clamp a longer range and warn; the command refuses, so a
>   harness never compares a truncated render against a full-length reference.
> - **Digests are SHA-256**, via a new `sha2` dependency. Together with `clap`
>   that makes two new crates for `THIRD-PARTY-LICENSES.md` to pick up at the
>   next release.
> - **The reproducible command is an argv array, not a shell string** — it
>   round-trips spaces and quotes with no quoting rules to get wrong.

## Goal

Add a stable, one-shot command that loads a saved Pertylizer project or bundle,
renders a deterministic arrangement window to WAV, emits a machine-readable
result, and exits. This is an adapter over the existing project loader and
offline render code, not a second rendering engine.

The initial consumer is `sid-analyzer sid-abtest`, but the protocol must remain
general enough for CI render tests and other offline producers.

## Current code and gap

- `crates/pertylizer/src/main.rs` accepts `--headless`, which starts the
  long-lived MCP server on stdio. Arguments are matched by hand
  (`args.iter().any(|a| a == "--headless")`); there is no argument parser.
- `crates/pertylizer/src/mcp_bridge/analysis_impl.rs` exposes the tested
  `render_to_wav_impl` path. Its signature is MCP-coloured — `&McpSharedState`
  in, `McpBridgeError` out, `synth_mcp::AnalysisScope` for scope — even though
  what the render actually needs from the shared state is the song and the time
  window.
- `crates/pertylizer/tests/render_to_wav.rs` verifies WAV creation and tails.
- Project loading and headless application live in `project_apply.rs` and are
  exercised by `tests/mcp_project_load.rs`.

There is no supported process contract for load-render-exit. Requiring an
external regression runner to implement MCP initialization, tool discovery,
session mutation, error decoding, and shutdown adds state and failure modes
that are unrelated to rendering.

## Command contract

Add a subcommand or separate small binary with the following version-1
contract:

```text
pertylizer render \
  --protocol-version 1 \
  --input project.ptz \
  --output render.wav \
  --sample-rate 44100 \
  --seconds 10 \
  --tail-seconds 0 \
  --result-json render-result.json
```

with an optional mix selection, either flag repeatable:

```text
  --solo-track <id|name> ...
  --mute-track <id|name> ...
```

Duration may later gain an exact tick/source-span alternative without changing
version 1.

`--seed` and `--normalization` are deliberately **not** part of version 1. No
global render seed exists — seeds live per note-processor in the project and are
already deterministic, so a given project renders byte-identically — and no
normalization stage exists in the render path, where it would actively harm the
A/B budget's `maximum_peak_error` / `maximum_rms_error`. Adding either as an
optional flag later does not break version 1.

The JSON result records protocol version, Pertylizer revision, input and output
content digests, resolved sample rate/frame count, the effective mix selection,
tail, warnings, and the complete reproducible command.
Human progress goes to stderr; stdout remains usable for JSON when
`--result-json` is omitted.

## Selecting what is rendered

Version 1 originally specified a `--tap` enum taking `final-mix` or "a stable
physical track/voice identifier". That is dropped in favour of the primitive the
application already has, for three reasons.

**A SID voice is not one instrument.** In an exported SID project one voice
spreads across several instruments — `Nemesis_the_Warlock.json` carries
`V1 drum (drop)` and `V1 Lead`, `V2 triangle flt` and `V2 triangle flt rng` —
because the tune reassigns the voice over time. So `voice-2` would have had to
name a *set*, which the renderer's single `instrument_id` cannot express, and
keying on the `V<N> ` name prefix would break the moment an instrument is
renamed.

**Mute/solo is what the reference side already does.** `sid-abtest` isolates a
voice on the sidplayfp side by muting the other two (`-u<other>`). Mute/solo is
therefore the symmetric primitive; a bespoke tap enum is a second vocabulary for
the same idea.

**It is already implemented and already exercised.** `SequencerTrack` carries
`mute` and `solo` (`track.rs:100`, `:104`) with the usual DAW semantics via
`song.any_solo()` + `track.is_audible(any_solo)`, both are serialized per track,
and the offline arrangement renderer honours them (`arrangement_render.rs:616`).
`analyze_section` already performs per-track soloed renders through that path,
including the subtle parts: reverb/delay/compressor tails must not bleed from
one soloed render into the next, and a reused session has to be reset between
them (`arrangement_render.rs:322`, `:521`). The command reuses that, and must
not reimplement it.

The consumer keeps the mapping, which is the right place for it: `sid-analyzer`
generated the project and knows which tracks it wrote for which voice.

### Identifiers

`TrackId` is canonical. It is a `u16` serialized as the track's `id` field, it
is stable under renaming *and* reordering, and the producer of the file already
holds it.

A name is accepted as a convenience but must resolve **unambiguously** — zero
matches or more than one is an error before rendering, because `create_track`
does not enforce unique names.

Display index is not accepted: it shifts when tracks are reordered, so the same
command would render different audio from the same file.

### The flags are the whole mix state

On load, `mute` and `solo` are cleared on **every** track, and then exactly what
the flags say is applied.

| Command | Result |
|---|---|
| neither flag | the true full mix, whatever the file had saved |
| `--solo-track 3` | track 3 only |
| `--mute-track 3` | everything except track 3 |

Saved flags are deliberately not honoured. The JSON result must be a function of
the command plus the input digest; if the file's saved solo leaked in, two
projects with identical audio content but different saved mix state would render
differently under the same command, and the receipt could not explain why.

Because that override is invisible otherwise, a project whose stored flags were
non-default emits a **warning** in the result — someone asking why the render
differs from what they hear in the app finds the answer in the receipt.

Solo and mute may be combined; the semantics are inherited from
`is_audible(any_solo)` rather than restated, so the command sounds like the
application. There is deliberately no `--respect-saved-mix` escape hatch in
version 1: nothing asks for it, and it would reintroduce exactly the
irreproducibility this rule removes.

The command sets these flags on the in-memory project only. It renders and
exits; it must never write the input file back.

## Argument parsing

Adopt **`clap` 4.6.5** (latest at time of writing; MSRV 1.85 against this
workspace's 1.97) as a workspace dependency, with the derive API.

`main.rs` matches argument strings by hand today, which is tolerable for one
boolean flag and unpleasant for ten flags with values, two of them repeatable.
Hand-rolling would also mean hand-rolling `--help`, arity and validation errors,
and the quoting behaviour the reproducible-command field depends on.

### Move the whole existing surface onto it, not just the new command

The migration is part of this work, not a follow-up. Leaving two argument
dialects in one binary is worse than either one alone: the rules for what counts
as a valid invocation would depend on which flag you happened to use.

Today's surface is small, which is exactly why this is cheap to do now:

| Flag | Gate |
|---|---|
| `--headless` | `feature = "mcp"` |
| `--no-osc` | `feature = "osc"` |
| `-h` / `--help` | always |

All three move to a `clap` derive struct, with `#[cfg(feature = ...)]` on the
gated fields so the help text keeps matching the build. Three things go away
with them:

- **The hand-written `print_help`**, which is feature-gated in three separate
  places and has to be kept in step with the matching by hand. `clap` derives it
  from the same struct that parses, so they cannot drift.
- **The second `env::args()` collection.** `main` collects the arguments and
  `run_gui` collects them again to look for `--no-osc`; parsing once and passing
  the parsed value down removes the duplicate.
- **Silent acceptance of unknown arguments.** Nothing today rejects them, so
  `pertylizer --headles` starts the GUI without a word, and any misspelled flag
  is indistinguishable from an unsupported one. That matters more once an
  external harness drives the binary: a typo in a generated command line must
  fail loudly, not render the wrong thing successfully.

`render` becomes a subcommand of the same parser. Running with no subcommand
keeps today's behaviour — launch the GUI — so existing invocations and desktop
launchers are unaffected.

A new dependency means `THIRD-PARTY-LICENSES.md` must be regenerated at the next
release (`cargo about`, see the `new version` flow in `CLAUDE.md`). The same
applies to `sha2`, added for the receipt's content digests.

## Implementation

1. Extract the load-and-render orchestration used by `AppSynthBridge` into a
   library function whose inputs contain no MCP types.
2. Reuse `project_apply` and the same offline arrangement renderer used by
   `render_to_wav_impl`; do not reconstruct projects through public MCP calls.
   Per-track renders go through the existing soloed-render path so the
   tail-isolation and session-reset behaviour is shared, not duplicated.
3. Resolve and validate the mix selection **before** rendering: unknown track
   id, an unresolvable or ambiguous name, invalid time range. A ten-second
   render must not run before the arguments are known to be good.
4. Add typed errors for project/schema load, missing assets, unresolved track
   selection, invalid time range, render failure, and output write failure.
5. Write WAV and JSON through temporary sibling files, then rename after both
   complete. `io/atomic.rs` already does exactly this and is reused rather than
   reimplemented.
6. Include the renderer/build revision in successful output. A dirty build must
   say so rather than claim the commit is an exact identity.
7. Keep the MCP tool as an adapter over the same library operation so both
   entry points share defaults and validation.

## Tests

- A saved synthetic project renders byte-identically through the MCP adapter
  and one-shot command for the same configuration.
- Repeated invocations of the same configuration are byte-identical.
- float32 output parses with the declared sample rate and frame count.
- Tail changes produce the expected output metadata.
- `--solo-track` and `--mute-track` produce the expected audible set, by id and
  by name, and combine per `is_audible`.
- A project saved with a soloed track renders the **full** mix when no flag is
  given, and reports the override as a warning.
- An unknown track id, and a name matching zero or several tracks, return typed
  errors before any rendering and leave no partial final output.
- Missing bundle samples return a typed error and no partial final output.
- The input file is byte-identical after a render that changed the mix
  selection.
- Paths containing spaces round-trip in the emitted reproducible command.
- The command runs without an audio device or GUI.
- The migrated flags still behave: `--headless` starts the MCP server, `--no-osc`
  disables telemetry, no subcommand launches the GUI, and an unknown flag exits
  non-zero with a message instead of being ignored.

## Exit gate

- [ ] `sid-abtest render` can invoke the installed Pertylizer command directly,
  without an MCP client or wrapper script. Note that `sid-abtest.rs` currently
  emits `--tap final-mix` / `--tap voice-N`; it moves to `--solo-track` with the
  track ids its own exporter wrote. **Open — the change is in the `sid-analyzer`
  repository, not this one.**
- [x] The command's version-1 arguments and JSON result are documented and
  covered by integration tests (`crates/pertylizer/tests/render_command.rs`,
  plus the parser tests in `main.rs`).
- [x] MCP and CLI renders use the same load, validation, render, and
  WAV-writing implementation (`crates/pertylizer/src/render/`), asserted
  byte-for-byte by `the_mcp_tool_renders_the_same_bytes`.
- [x] The binary has one argument parser. No hand-rolled matching or
  hand-written help text remains in `main.rs`.

### Not covered

The plan asks for a test that *missing bundle samples* return a typed error. A
truncated bundle is covered instead; a structurally valid bundle whose sample
entries are absent is not, because it was not clear that the loader treats that
as an error rather than a warning. Carried to `plans/TODO.md` §5.6 along with
the other open follow-ups.

## What shipped

`crates/pertylizer/src/render/` holds the core both entry points use:

| File | Contents |
|---|---|
| `wav.rs` | `TickWindow` (cannot hold an empty range), `tick_window_from_seconds`, `render_window_to_wav` |
| `mix.rs` | `TrackSelector` / `MixSelection` / `apply_mix_selection` |
| `headless.rs` | `load_project_file`, `path_identity` |
| `receipt.rs` | `RenderReceipt` and its parts |
| `command.rs` | `RenderCommand` / `run_render_command` — validate, load, mix, render, write |

`mcp_bridge/analysis_impl.rs`'s `render_to_wav_with_tail_impl` is now an adapter
over the same code, and `write_interleaved_wav` writes through
`io::atomic`, so the MCP tool's WAV write became crash-safe as a side effect.

Two review passes over the finished branch found twelve real defects between
them — the mix-inverting solo+mute overlap, `--output` able to be `--input`,
unbounded tail and window×rate allocations, a collision guard that compared
paths `canonicalize` could not resolve, and a silence warning that never fired.
All are fixed and covered by tests.
