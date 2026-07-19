# SID oscillator per-step frequency and deterministic noise

## Goal

Extend the native `sid_oscillator` just enough to reproduce SID driver programs whose waveform
sequence changes pitch between steps or depends on a known noise state. Keep the work local to the
SID parameter model and oscillator implementation, and reuse the existing note, legato, TEST,
descriptor, serialization, automation, GUI, and MCP infrastructure.

## Existing behavior to preserve

- The oscillator accumulator and 23-bit noise LFSR free-run across ordinary note events.
- A non-legato note restarts the waveform sequence without resetting the accumulator or LFSR.
- A legato pitch change does not call module `note_on`, so sequence phase and LFSR state continue.
- A TEST rising edge resets the accumulator and reloads the LFSR deterministically.
- A sequence with no per-step frequency overrides follows the played note or global `freq_reg`
  exactly as it does today.

## Phase 1: per-step frequency

- Add 16 raw SID frequency-register parameters named `seq_step_freq_0` through
  `seq_step_freq_15`.
- Add a 16-bit `seq_freq_mask`. Bit `i` enables the frequency override for step `i`; a clear bit
  inherits the current played-note or authored global frequency.
- Store the values in fixed-size arrays and masks. Do not allocate or resize collections in the
  audio thread.
- Resolve waveform mask and frequency from the same sequence index and apply both atomically at
  the driver-frame boundary before rendering the next sample.
- Keep automation and modulation on the global `freq_reg` as the inherited frequency source.
  Per-step frequency fields are static program data and are not automatable or modulatable.
- Expose the new parameters through the normal module descriptor. Generic project serialization,
  module type information, MCP parameter tooling, and GUI parameter handling should consume that
  descriptor without separate SID-specific APIs.

## Phase 2: configurable noise seed

- Add a `noise_seed` parameter constrained to a valid non-zero 23-bit LFSR state.
- Use `noise_seed` instead of the fixed initial state when the module is reset or TEST produces a
  rising edge. Preserve the current seed as the default.
- Do not add a separate general-purpose `noise_reset` trigger: the existing TEST parameter and
  gate input already provide deterministic reset semantics.
- If a real imported driver requires reset at a sequence boundary, add a 16-bit
  `seq_noise_reset_mask`. Bit `i` reloads `noise_seed` when step `i` becomes active. Do not add this
  mask speculatively without a fixture that demonstrates the need.

## Explicitly deferred

- Per-step `pw_reg` and `level` overrides are deferred until an imported SID fixture requires
  them. Adding both would introduce 32 more visible parameters and complicate the editor and MCP
  surface without evidence that they are needed.
- No changes are planned for the sequencer, voice allocator, note graph, automation-lane model, or
  project container format.
- No custom SID sequence editor is required initially. Add one later only if the generic parameter
  UI proves impractical for real programs.

## Verification

- Existing SID renders remain unchanged when `seq_freq_mask` is zero and the default noise seed is
  used.
- Sequence tests alternate steps with distinct frequency registers and prove the waveform and
  frequency switch on the same sample boundary.
- Inheritance tests prove clear mask bits continue to follow played-note pitch, global
  `freq_reg`, automation overrides, and modulation offsets.
- Legato tests prove sequence phase and LFSR continuity while pitch changes.
- Retrigger tests prove non-legato notes restart only the waveform sequence.
- Golden tests pin the first noise samples for at least two seeds and after a TEST reset.
- Descriptor and project round-trip tests cover every new parameter and confirm that MCP/module
  type discovery exposes the fields through the generic path.

## Completion criteria

Phase 1 is independently shippable. Phase 2 is independently shippable after Phase 1 and does not
require the optional sequence reset mask unless a real import fixture demonstrates that behavior.
