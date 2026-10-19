# Phase 9 real producer load controls

This delivery covers software mechanisms and a reproducible calibration tool.
It is not Phase 9 acceptance, representative production qualification or
ADR-0054's simultaneous six-share selection. The periodic consumers are
mechanism fixtures, not representative data-dependent musical workloads.

## Scope and falsifiers

[ADR-0093](../decisions/ADR-0093-real-mixed-producer-load-consumers.md) defines
parameter-only global Trim programs and node-linked periodic global Delay
clears. Both are real executable consumers. The application does not declare
or enable them. Unavailable producers remain unavailable in observations;
observed idle producers may report zero.

The [producer controls](../../../crates/synth_engine_v2/src/stream/mixed/transfer/continuation/tests/producer_tests.rs)
exercise six real classes in one quantum, including Session operation charges,
protected releases and an actual external-plus-internal total. They test
multi-target contiguity and stable program order, maximum authored and internal
occupancy, suppression of the arena's audible effect, actual-emitter cap+1
faults, stopped/seek phase, compatible continuation, ordinary-owner refusal,
first-sample initialization and carry-only observation. Callback allocation
counting includes allocation and deallocation, starting with the first callback.

The [cold influence control](../../../crates/synth_engine_v2/src/producer_programs.rs)
first compiles a valid global-to-voice audio path without a producer, then
requires the actual compiler to refuse that same path as a producer target.
The [merge control](../../../crates/synth_engine_v2/src/render/producer_tests.rs)
checks an earlier internal control ahead of a later external control while
preserving external-first equal-offset ties. It is a merge fixture, not an
admitted external parameter writer to a producer-owned Delay.

The existing mixed/software controls continue to cover live overload,
protected releases, identity exhaustion, stalled readers, continuation/reset
custody and terminal callback faults. This slice adds no issued runtime note
identity: parameter programs retain no holds or future events, and periodic
internal actions use no note identity or serial cursor.

## Calibration command

Run from the repository root, in the environment being characterized:

```bash
CARGO_BUILD_JOBS=2 cargo test -p synth_engine_v2 --release \
  --no-default-features --features mixed-transfer-rehearsal --lib \
  calibrate_real_six_producer_software_load -- --ignored --nocapture
```

The opt-in runner reports OS, architecture, available parallelism and compiler;
at each of four sample rates it runs 4096 whole 64-frame callbacks. Its fixture
SHA-256 covers the actual fixture source, program source, prepared native node
parameters and complete host profile. It records median, p99 and maximum
host-callback duration, admitted maximum internal clear sample writes, class
peaks, actual external peak and actual combined
peak. Collection runs outside callback timing. The ordinary advisory CPU prediction excludes periodic full-line clears; the
compiler warns about that exclusion and the runner reports their checked work
bound. The reported maximum is an
observation, not a worst-case execution bound. Build mode follows the command;
keep the command and environment with every output.

The callback fixture includes the real compiled/live/Session/Release burst and
continuing actual authored/internal producers. All six peaks must be nonzero;
a missing class fails the run. Class peaks need not coincide; the deterministic
same-quantum control supplies that separate proof. No profile or ingress depth
is selected from these measurements. Before normal downstream enabling,
ADR-0054 still requires the new consumers' occupancy evidence and the internal
retain-or-reselect record; production selection needs representative workloads.

## Verification

The complete repository gate and explicit mixed/headless/GUI checks passed,
as did the deterministic evidence gate and 41 documentation-checker tests.
Independent Claude review and the focused repair reread accepted the declared
scope. The repairs make continuation initialization falsifiable, exercise real
producer reset, separate fault attribution and accepted occupancy, and expose
the uncalibrated full-history clear work.

The opt-in release runner passed on Linux x86_64 with 24 available parallel
threads and rustc 1.99.0. The recorded command is the command above. These are
local observations under concurrent development load, not a controlled
production measurement. Measurements were repeated while repairing correctness and closing source
coverage. The table records the last run after those repairs, with a fixture
fingerprint covering the test sources. No timing threshold or fastest-run selection was used.

| Rate | Fixture SHA-256 | Callback ns p50 / p99 / max | Admitted history sample writes/Q |
|---|---|---|---|
| 44.1 kHz | `28da68bfaf01750816116ad61fbc0d8e9f4fe7c4fc4d7781074cfefb7faa7548` | 33803 / 38852 / 290656 | 176400 |
| 48 kHz | `6db02c696e001fef2715b2bb62f908ce5cf68f41d835aadd1f625bc606bdf3ab` | 35732 / 44392 / 255428 | 192000 |
| 96 kHz | `d815c0c2d6daad0debca7d4f2100a8b36f0ae1bf9ae57ef8121a2770e0cb73f8` | 58526 / 75182 / 530655 | 384000 |
| 192 kHz | `59f490da4c6379d5d62a7a2fd45b21c364b18e1c47dc8b34c0c326dbbed82697` | 120819 / 134716 / 982169 | 768000 |

Each rate ran 4096 whole 64-frame callbacks. Every rate observed class peaks
Compiled 2, AuthoredRuntime 3, Live 1, Session 10, Internal 1 and Release 1.
The actual same-quantum external peak was 17 and combined peak 18. These
numbers are observed fixture occupancy, not selected capacities. In particular,
the observed maxima do not establish callback deadlines. Both shares and the
CPU coefficient for periodic clears retain their qualification gates.
