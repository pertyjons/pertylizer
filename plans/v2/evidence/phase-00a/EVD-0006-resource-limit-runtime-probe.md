# EVD-0006: Resource-Limit Runtime Probe

| Field | Value |
|-------|-------|
| ID | EVD-0006 |
| Status | Complete |
| Phase | 00A |
| Created | 2026-08-15 |
| Last reviewed | 2026-08-15 |
| Retention | Permanent |
| Related | P00A-T004, ADR-0021, LIMIT-0001, LIMIT-0020, LIMIT-0067 |
| Superseded by | — |

## Question or hypothesis

Do executable observations of the three runtime axes named by ADR-0021 — an audio callback above 4 096 frames, a
meter slot above 128, and a legacy rack above 32 stages — agree with the resource inventory's classifications and V2
rules, or do they reveal a failure outside the inventory or ADR-0021 taxonomy?

The probe does not ask whether every possible V1 limit has been found. It bounds three named axes that the earlier
search and use-site audit could not exercise.

## Acceptance criteria

These criteria were fixed before the successful run:

- exactly three tests whose names start with `resource_limit_probe_` run under one workspace command;
- each test crosses one named boundary and asserts the observed V1 result rather than a desired V2 result;
- each observation maps to an existing `LIMIT` row and failure class, or it creates a new row and triggers ADR-0021's
  revisit condition;
- the probe passes only after the inventory records any newly observed behaviour.

The first debug run failed the third criterion. The oversized callback grew a voice buffer on the audio thread and
then hit a debug assertion in a later fixed effect buffer. The original test observed only the panic, so independent
review correctly rejected its claim that allocation happened only in release. A first correction counted allocations
while catching the panic, but review found that panic reporting itself allocates and could satisfy that assertion. The
probe now records the voice buffer's length and capacity before the call and inspects both after unwinding: capacity has
grown before the later panic. Release additionally uses the allocation counter after the assertion is compiled out.
The inventory and result cover both build modes.

## Source and environment

- Source revision under test: V1 at `54cd6d3f`; the probe-only test diff and inventory correction are retained with
  this record.
- Platform and architecture: Linux x86-64.
- Rust/tool versions: workspace toolchain.
- Audio/sample configuration: 48 kHz, stereo, default voice graph for the callback axis.
- Feature flags: default; both debug and release profiles for the oversized-callback axis.
- Relevant host or device simulation: direct `AudioCallbackContext`; no physical device.

## Inputs

- audio callback sizes 4 096 and 4 097 frames, with the crossing case asserted;
- meter indices 127 and 128, asserting the last valid slot and the first omitted slot;
- a deserialized-equivalent legacy processor rack containing 33 stages.

## Method

The tests exercise production methods at one value past each boundary:

1. construct and warm a standard `SynthEngine`, process the 4 096-frame boundary control, then process one 4 097-frame
   callback in debug and release;
2. publish meters at indices 127 and 128, then query both keys after setting the visible count to 128;
3. migrate a 33-stage legacy rack and inspect both the resulting graph and the cleared source rack.

The existing in-range tests are controls: steady-state engine callbacks through 1 024 frames do not allocate, meter
slots inside the live count are readable, and ordinary rack migration preserves its processors. The probe tests add
only the boundary crossings.

## Commands

```text
cargo test --workspace resource_limit_probe_ -- --nocapture
cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure -- --nocapture
```

## Results

| Axis | Observed V1 behaviour | Inventory mapping |
|------|-----------------------|-------------------|
| 4 097-frame callback | Debug performs at least one audio-thread allocation while growing a voice buffer, then panics because 8 194 interleaved samples exceed an 8 192-sample fixed effect buffer. Release removes that assertion, retains the allocation, and continues | `LIMIT-0001`; same platform-capability class, corrected ordered and build-mode-specific overflow description |
| Meter index 128 | `publish` ignores the slot and the key remains invisible | `LIMIT-0020`; configurable safety budget |
| 33-stage rack | Migration creates 32 graph nodes, clears all 33 source stages, and silently loses the final stage | `LIMIT-0067`; configurable safety budget |

Successful runs: the workspace command ran three named tests with zero failures, and the release command independently
passed the oversized axis. Debug verifies the post-unwind voice-buffer length and increased capacity without counting
panic machinery; release verifies both increased capacity and an allocation event.

## Interpretation

All three axes now map to existing inventory rows and ADR-0021 classes. The callback result strengthens
`LIMIT-0001`: failure is build-mode and topology dependent and includes audio-thread allocation in both probed build
modes, a later debug-only panic, and input truncation on another path, so V2's terminal stream-fault rule remains the
correct disposition. No observation
requires a new failure class or a new `LIMIT` row, so ADR-0021's revisit condition is not triggered.

## Limitations

- This is three targeted boundary probes, not evidence that no unnamed truncation exists elsewhere.
- The callback case uses the standard default graph; another topology can reach a different `LIMIT-0001` failure first.
- The tests characterize Linux workspace behaviour and do not exercise a physical audio host.
- The rack is constructed through the crate-internal persisted representation because no current product authoring
  path can create a 33-stage legacy rack.

## Conclusion

**Supported for the three named runtime axes.** Every probe observation is classified and none triggers ADR-0021's
taxonomy revisit condition. Together with the inventory's proposed rule and diagnostic for every discovered row, this
satisfies P00A-T004's bounded master-plan gate. `LIMIT-0017` remains `Investigating` for its stricter inventory lifecycle
and ADR-0039's Phase 10E decision; that row is outside the probe's three axes. Later findings follow the working
agreement's new-finding policy rather than reopening Phase 0A automatically.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Probe tests | `crates/synth_engine/src/synth_engine/rt_alloc_guard.rs`, `crates/synth_engine/src/state.rs`, `crates/synth_sequencer/src/song.rs` | Permanent; reproduce with the command above |
