# ADR-0080: Explicit terminal mono folding

| Field | Value |
|---|---|
| ID | ADR-0080 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-17 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0041, ADR-0054, ADR-0057, ADR-0077, EVD-0026 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Phase 9 now needs to render the saved stereo master into a mono rehearsal sink.
ADR-0041 deliberately left this sound law undecided. This record fills that
explicit-node law; it changes neither implicit edge conversion nor the
application's stereo-only V2 admission under ADR-0077. No saved project,
manifest, wire contract or existing node identity changes.

## Verified premises

The pinned `instrument-inserts` graph renders through stereo instruments,
inserts, master stages and `StereoMonitor`. Connecting that graph directly to
a mono `Output` refuses with `LayoutMismatch`. Removing its stereo stages
would change the workload EVD-0026 pinned. The first stereo experiment is
complete on `99c63395`; its results do not qualify a mono workload.

Existing sampler, delay and amplifier folds sum f32 inputs before halving.
That arithmetic can overflow for large finite inputs. Halving f32 inputs
before summing avoids overflow but loses odd subnormal dual-mono values.
Neither supplies the full-range terminal law chosen here.

## Product choice

On 2026-10-17 the user selected the recommended terminal half-sum,
`M = (L + R) / 2`, after the stereo master. The wider intermediate arithmetic
below implements that law while preserving finite range extremes.

## Decision

1. `StereoToMono` is an explicit stateless node with one fixed Stereo audio
   input and one fixed Mono audio output. Validation never inserts it. A
   stereo cable directly into a mono port continues to refuse.
2. For each frame, compute `(f64::from(left) + f64::from(right)) * 0.5`, then
   round once to f32. There is no clamp, compensation gain, control, history
   or latency. For finite f32 inputs the mean is finite and bounded by the
   input extrema. Dual-mono preserves the input, including minimum subnormals
   and maximum finite values; opposite values cancel; swapping channels does
   not change the result. A single active channel contributes half its value,
   rounded to f32. This is a half-sum, not a constant-power sum.
3. The wider intermediate is deliberate: this new terminal operation preserves
   both finite range extremes. Existing sampler, delay and amplifier arithmetic
   stays unchanged under its own laws; this record does not silently alter them.
4. Project lowering inserts the named `MASTER_MONO` node only when its requested
   profile is Mono. It follows `MASTER_METER` and precedes `MASTER_OUTPUT`.
   Instruments, inserts, master processing and the meter remain stereo. The
   meter observes pre-fold stereo audio; it is not the emitted mono waveform.
5. This is an ordinary declared node, visible in the IR, scheduled operations,
   prepared records and arena/resource accounting, not another implicit
   `Conversion`. Its differing layouts prohibit in-place reuse. Its kind and
   master identities are appended/reserved without renumbering existing kinds.
6. Lowering emits `MonoOutputFold` as an `Unrepresented` diagnostic naming the
   project. The plan can render, but its derived fidelity refuses a V1 stereo
   parity verdict. `OutputPolicy` continues to select limiting, not a layout.
7. Application mono-device admission, production mixed refusal and physical or
   concurrent qualification remain unchanged. The new path is for lowering,
   offline rendering and the experimental rehearsal. Mono capacity/timing
   requires a separately frozen method and fresh collection before acceptance.

## Alternatives

- Sum f32 channels before halving: preserves small dual-mono values but can
  create infinity for finite same-sign input near f32 maximum.
- Halve f32 channels before adding: preserves large values but rounds away or
  changes odd subnormal dual-mono values.
- Constant-power sum: raises identical channels by about 3 dB and changes
  coherent mono gain. It is not selected for this terminal conversion.
- Remove stereo stages or silently narrow a compiler edge: changes the saved
  graph or hides the conversion, contradicting the existing refusal contract.

## Falsifier and stopping rule

Literal kernel vectors must cover asymmetric frames, dual-mono, opposite signs,
signed zero, odd subnormals, maximum finite values and output above full scale.
They must detect channel/frame errors, clamping, hidden gain, transient
allocation and freeing. The graph must schedule one non-aliasing fold; an
otherwise identical graph without it must still refuse its narrowing edge.
Mono rendering must equal the specified fold of the equivalent stereo render
and remain bit-identical under regular and irregular host partitions. The
saved-project lowering must retain the stereo graph, mark parity ineligible
and reproduce the checked mono transcription at all four candidate rates.

A false sound claim, changed stereo behavior, hidden narrowing, ownership hole
or unsupported parity verdict blocks acceptance. Optional implementation detail
does not. Full verification and an independent uncommitted review precede the
implementation commit. Acceptance includes the user's half-sum choice and the
independent implementation/method review.
