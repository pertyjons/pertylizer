# Headless project render CLI

> **Status:** planned.
>
> **Planning baseline:** Pertylizer `3e25e679`, 2026-07-29.

## Goal

Add a stable, one-shot command that loads a saved Pertylizer project or bundle,
renders a deterministic arrangement window to WAV, emits a machine-readable
result, and exits. This is an adapter over the existing project loader and
offline render code, not a second rendering engine.

The initial consumer is `sid-analyzer sid-abtest`, but the protocol must remain
general enough for CI render tests and other offline producers.

## Current code and gap

- `crates/pertylizer/src/main.rs` accepts `--headless`, which starts the
  long-lived MCP server on stdio.
- `crates/pertylizer/src/mcp_bridge.rs` exposes the tested
  `render_to_wav_impl` path.
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
  --tap final-mix \
  --seed 0 \
  --tail-seconds 0 \
  --normalization none \
  --result-json render-result.json
```

Required taps are `final-mix` and a stable physical track/voice identifier.
Unsupported taps must fail before rendering. Duration may later gain an exact
tick/source-span alternative without changing version 1.

The JSON result records protocol version, Pertylizer revision, input and output
content digests, resolved sample rate/frame count, tap, seed, tail,
normalization, warnings, and the complete reproducible command. Human progress
goes to stderr; stdout remains usable for JSON when `--result-json` is omitted.

## Implementation

1. Extract the load-and-render orchestration used by `AppSynthBridge` into a
   library function whose inputs contain no MCP types.
2. Reuse `project_apply` and the same offline arrangement renderer used by
   `render_to_wav_impl`; do not reconstruct projects through public MCP calls.
3. Add typed errors for project/schema load, missing assets, unsupported tap,
   invalid time range, render failure, and output write failure.
4. Write WAV and JSON through temporary sibling files, then rename after both
   complete.
5. Include the renderer/build revision in successful output. A dirty build must
   say so rather than claim the commit is an exact identity.
6. Keep the MCP tool as an adapter over the same library operation so both
   entry points share defaults and validation.

## Tests

- A saved synthetic project renders byte-identically through the MCP adapter
  and one-shot command for the same configuration.
- Repeated invocations are byte-identical for a fixed seed.
- PCM16 and float32 output parse with the declared sample rate and frame count.
- Tail, normalization, and tap changes produce the expected output metadata.
- Missing bundle samples and unsupported taps return typed errors and no
  partial final output.
- Paths containing spaces round-trip in the emitted reproducible command.
- The command runs without an audio device or GUI.

## Exit gate

- `sid-abtest render` can invoke the installed Pertylizer command directly,
  without an MCP client or wrapper script.
- The command's version-1 arguments and JSON result are documented and covered
  by integration tests.
- MCP and CLI renders use the same load, validation, render, and WAV-writing
  implementation.
