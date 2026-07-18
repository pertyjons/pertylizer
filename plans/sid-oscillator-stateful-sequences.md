# SID oscillator stateful waveform sequences

## Problem

The native `sid_oscillator` can sequence 16 waveform masks, but SID driver programs also change
frequency and amplitude at individual steps and sometimes depend on a known 23-bit noise LFSR
state. The current parameter surface has global modulatable `freq_reg`, `pw_reg`, and `level`,
plus waveform-only `seq_step_*` fields. It has no deterministic LFSR seed/reset control and no
per-step frequency or level, so imported SID programs cannot be reproduced as one editable,
free-running native program.

## Implementation

- Add an optional 23-bit `noise_seed` parameter and an explicit `noise_reset` trigger/sequence
  action. Preserve the existing free-running default when neither is supplied.
- Extend each active sequence step with optional `freq_reg`, `pw_reg`, and `level` overrides.
  An unset field inherits the current/global value; this keeps existing projects compatible.
- Apply all step fields atomically at the sequence boundary before rendering the next sample.
- Keep sequence phase across legato note changes and document exactly which note/reset events
  restart it.
- Expose the new fields through module type info, project serialization, automation discovery,
  MCP parameter tooling, and GUI sequence editing.

## Verification

- Golden tests pin the first noise samples for two seeds and a test-bit reset/recovery cycle.
- Sequence tests alternate noise/tonal masks with distinct frequency and level values and prove
  that every boundary is sample-atomic.
- Legato tests prove sequence and LFSR continuity; explicit reset tests prove deterministic
  restart.
- Old projects deserialize with the current behavior and render unchanged.
