# Phase 1: Introduce the Experimental Sound Core V2 Crate

> **Completed historical record.** Current phase outcomes live in
> [`../ROADMAP.md`](../ROADMAP.md), active work in [`../NOW.md`](../NOW.md), and
> the accepted verdict in [`REV-P01`](../reviews/phase-01-exit-review.md).

| Field | Value |
|-------|-------|
| Status | Complete |
| Phase | 01 |
| Started | 2026-08-17 |
| Last updated | 2026-08-17 |
| Master plan | [`master-plan.md`](../master-plan.md#phase-1-introduce-the-experimental-sound-core-v2-crate) |
| Exit review | [`REV-P01`](../reviews/phase-01-exit-review.md), `Accepted` |

## Objective

Stand up an experimental `synth_engine_v2` crate that owns the render-core contracts Phase 0A accepted: the time and
quantum types, the host profile and its admission, a minimal compiler IR, and a renderer that splits any caller block
into the fixed internal quantum. Phase 1 renders offline from IR built in tests. It does not connect to projects, does
not compile V1 patches, and does not build the scheduler.

This section records the scope and gates used when the phase ran. The
[host-profile specification](../specs/spec-host-profile-and-render-limits.md)
remains the current contract for the profile, its admission, and its failure
behaviour; [`../ROADMAP.md`](../ROADMAP.md) and
[`../PROCESS.md`](../PROCESS.md) own the live workflow rules.

## Entry conditions

- Phase 0A closed at [`REV-P00A`](../reviews/phase-00a-exit-review.md), `Accepted`.
- The four render-core decisions below are `Accepted`.
- No Phase 3 mechanism is required: `HOST-INV-021` is deferred, and Phase 1's event input is the prevalidated bounded
  span the specification defines under *Deferred to Phase 3*.

## Required decisions

| ADR | Required status | Deadline or permitted deferral |
| --- | --- | --- |
| ADR-0001 | `Accepted` | Phase entry — quantum semantics, both carries, the priming fill, lateness |
| ADR-0037 | `Accepted` | Phase entry — `Q` = 64, **provisional**; re-measured at the Phase 2 exit gate |
| ADR-0032 | `Accepted` | Phase entry — the time types, the envelope, the epoch |
| ADR-0021 | `Accepted` | Phase entry — admission policy, the capability/limit split, the report |
| ADR-0038 | `Accepted` | **Its classification is consumed at entry** — it is why `event_egress_capacity` is one profile field rather than two, so P01-T002 and P01-T004 carry and report exactly one. Only its *runtime* egress conformance is Phase 5's |
| ADR-0002 | `Proposed` permitted | Phase 2. Phase 1 carries only the layouts it renders and invents no vocabulary — see deviation 4 |
| ADR-0009, ADR-0024, ADR-0027, ADR-0034 | `Proposed` permitted | Their capacities are carried and reported; their semantics are later phases' |
| ADR-0022, ADR-0028 | `Deferred` permitted | Phase 3 and Phase 4 entry gates |

Because ADR-0037 is provisional, no work in this phase may hand-unroll a kernel to `Q`, lay out a buffer around its
value, or assert a control rate in Hz.

## Tasks

| ID | Deliverable | Status | Dependencies | ADRs/specs |
|----|-------------|--------|--------------|------------|
| P01-T001 | Time, quantum, and epoch types | Complete | None | ADR-0032 clauses 1-8, 12, 16, 18, 26, 28; ADR-0001 clauses 1, 11 |
| P01-T002 | `HostProfile`, `HostCapabilities`, `RenderLimits` with validated construction | Complete | P01-T001 | Host-profile spec; `HOST-INV-001`..`005`, `016`..`018` |
| P01-T003 | Minimal compiler IR with stable typed IDs | Complete | P01-T001 | Master plan Phase 1 work list; layer boundaries |
| P01-T004 | Admission: `ResourceReport`, `CompileError`, `CompileWarning`, diagnostics | Complete | P01-T002, P01-T003 | `HOST-INV-006`, `007`, `015`; ADR-0001 clause 7 |
| P01-T005 | Renderer boundary and the quantum splitting contract | Complete | P01-T004 | ADR-0001 clauses 4-9, 11-14, 16; ADR-0032 clauses 17-21, 27, 28. **Not clause 22** — the pre-epoch clamp is the Phase 3 ingress mapper's |
| P01-T006 | Offline harness and the executable exit-gate evidence | Complete | P01-T005 | Master plan Phase 1 exit gate |
| P01-T007 | Formal exit review `REV-P01` | Complete | All applicable tasks | Working agreement, review protocol |

## Active task

None. The phase is closed; the next work is Phase 2's.

The task that closed it was **P01-T007 — the formal exit review `REV-P01`.**

- **Scope.** Evaluate the five master-plan gate bullets and the thirteen contract checks below against the crate as
  committed, and record the result as a review from the exit-review template. The phase may be marked `Complete` only
  after that review's outcome is `Accepted`.
- **Non-goals.** Anything the later phases own. Three things this phase deliberately did **not** build, each with a
  named owner: the deferral mechanism of `HOST-INV-021` and V2's ingress streams (Phase 3), the pre-epoch clamp and its
  counter's increment (Phase 3's ingress mapper), and graph validation, the buffer arena with liveness analysis, and
  prepared/mutable state separation (Phase 2).
- **Verification.** The repository quality gate plus `cargo test -p synth_engine_v2`, which is 99 tests across six
  binaries.

## Deliverables and verification

| Task | Output/revision | Verification/evidence | Result |
|------|-----------------|-----------------------|--------|
| P01-T001 | `synth_engine_v2::time` | Unit tests: a refused out-of-range offset, signed subtraction in both directions, refused clock and plan-position advance at exhaustion, strictly increasing epochs, forward-only anchoring | Complete |
| P01-T002 | `synth_engine_v2::{quantities, profile}` | Unit tests: a zero capacity refused per type, a `NaN` cost ratio refused from both constructors, an inverted and an over-ceiling rate range refused, both range endpoints admitted, the horizon floor at eight block sizes including one above a second's worth of frames, the slot floor refused and its legal direction accepted | Complete |
| P01-T003 | `synth_engine_v2::ir` | Unit tests: a duplicate identity, a dangling edge, and an edge out of the output node each refused; aggregates naming their dominant node, port, and program; the cost model reproducing both figures EVD-0003 states | Complete |
| P01-T004 | `synth_engine_v2::{report, diagnostics, compile, plan}` | `tests/admission.rs`: 28 refusal cases checked against the admission-checked set itself, every field reported on both paths, the advisory budget warning and never refusing, the admitting-rule partition, the script aggregate reported without a threshold | Complete |
| P01-T005 | `synth_engine_v2::render`, hot path in `render/hot.rs` | `tests/render_contract.rs` (22 cases) and `src/tests/render_allocation.rs` (3) | Complete |
| P01-T006 | `synth_engine_v2::offline`, `tests/crate_boundary.rs`, `tests/render_loop_purity.rs` | The gate table below | Complete |
| P01-T007 | [`REV-P01`](../reviews/phase-01-exit-review.md) | The five gate bullets and fifteen contract rows, plus the repository quality gate | Complete |

## Exit gate mapping

The master plan's five gate bullets, each against the check that decides it. A gate with no executable check is not a
gate; the working agreement's phase-gate rule is what this table exists to satisfy.

| Gate | Deciding check | State |
|------|----------------|-------|
| An empty plan and a constant/sine source render deterministically | `an_empty_plan_renders_silence_deterministically`, `a_constant_source_renders_deterministically`, `a_sine_source_renders_deterministically_and_audibly` — each byte-comparing two renders, and the sine asserting audibility rather than accepting silence | **Passes** |
| Varying caller block sizes up to the maximum are split into the fixed quantum | `varying_caller_block_sizes_produce_the_same_audio` over nine partitions including 1, 7, 63, 65 and the maximum, plus `a_maximum_block_below_one_quantum_is_admitted_and_renders_the_same_audio` | **Passes** |
| A plan over its profile is refused before rendering with an attributable diagnostic, and nothing is clipped to fit | `every_limit_a_plan_can_exceed_has_a_refusal_case` proves the 28 cases *are* the admission-checked set; `each_refusal_names_its_field_both_amounts_and_the_responsible_object` runs them | **Passes** |
| The render loop takes no lock, allocates nothing, performs no I/O, and logs nothing | Counting-allocator test that arms **before the first render call after preparation** and stays armed across subsequent ones, so a kernel that allocates lazily on first use fails rather than being warmed away; plus a source-level purity check over `render/hot.rs`, which is its own file so the region needs no exceptions. `render_allocation.rs` arms the counting allocator **before the first call after preparation**; `render_loop_purity.rs` bans locks, I/O, logging, panicking accessors, and allocating constructs, with a control test asserting it is reading the render loop | **Passes** |
| The crate can be deleted without affecting V1 behavior or public APIs | `crate_boundary.rs`: no workspace crate names it, its own `[dependencies]` are within a four-name allowlist, and the workspace lists it — otherwise the tests would not run at all | **Passes** |

## Contract checks owed by this phase

The five bullets above are the master plan's. These are the rows the accepted decisions and the `Current` specification
assign to Phase 1 by name. They are not additional gates; they are what the gate bullets are worth nothing without, and
each names the task that owns it.

| Check | Source | Owner | State |
|-------|--------|-------|-------|
| A prepared plan renders after its source profile is dropped, and the renderer holds no profile reference | `HOST-INV-001`, `HOST-INV-002` | P01-T005 | **Passes** — `a_prepared_plan_renders_after_its_profile_is_dropped`. The guarantee is structural: no renderer field holds a profile |
| The build fails if `Q` outgrows `QuantumOffset::MAX`, and no profile field carries a quantum | `HOST-INV-004` | P01-T001, P01-T002 | **Passes** — and the assertion had to be repaired to bite: with `Q` and the offset both `u16` it was a tautology, so `Q` is a `u32` and the comparison is a real constraint |
| Every profile field is admitted by its rule: the capability half against the closed set, each limit field against exactly one of the three grounds | `HOST-INV-005` | P01-T002 | **Passes** — `every_field_is_admitted_by_exactly_one_rule`, over an exhaustive match so a new field must be classified |
| Every compile — succeeding or failing — returns a report whose every field has requested, available, and a dominant contributor | `HOST-INV-006` | P01-T004 | **Passes** — 42 rows on both paths, and `no_row_compares_mismatched_units` |
| One refusal case per render limit a plan can exceed, each naming the field, both amounts, and the responsible object, with the plan unchanged | `HOST-INV-007` | P01-T004 | **Passes** — 28 cases. Writing them corrected the predicate: an earlier form excluded only eight fields, and six of the rest compare a value against itself, so no plan could exceed them and the row was unsatisfiable |
| A plan over the advisory cost budget compiles and warns; no advisory field can produce a `CompileError` | `HOST-INV-015` | P01-T004 | **Passes** — both directions, including a budget at `f32::MIN_POSITIVE` |
| A profile with `forward_event_horizon < maximum_block_size + Q` fails construction naming both fields, and the default satisfies it at every admissible `maximum_block_size` | `HOST-INV-016` | P01-T002 | **Passes** — eight block sizes including 1, 63, and 1 000 000 |
| A `sample_rate` below or above `accepted_sample_rates` fails construction naming both fields, and each inclusive endpoint is accepted | `HOST-INV-016` | P01-T002 | **Passes** |
| Every quantity field refuses an out-of-domain value, the two kind fields are closed enums, and `HeldNoteCount` does not convert to `VoiceCount` | `HOST-INV-018` | P01-T002 | **Passes** behaviourally: one zero-capacity case per type, `NaN` refused by both float constructors. The non-convertibility is a property of an absent `impl` and is enforced by not writing one; no runtime test can see it |
| The carry latency is a named contributor in the report | ADR-0001 follow-up | P01-T004 | **Passes** — `LatencyContributor::RenderQuantumCarry`, asserted present and equal to `Q` |
| An impulse at plan sample 0 lands at output sample 0 on the offline path | ADR-0001 follow-up and risk control | P01-T006 | **Passes** — plus impulses at six interior positions and a range anchored at plan sample 900, and a test that live output is the offline output delayed by exactly one quantum |
| Clock exhaustion is a terminal stream fault: output silence, `needs_reprepare` published, a counted diagnostic, nothing allocated — and no panic on the audio thread | ADR-0032 clause 28 | P01-T005, P01-T006 | **Implemented; the fault path is exercised through the oversized callback, which shares it exactly.** Reaching exhaustion itself needs 2^64 frames — three million years at 192 kHz — so it has no test of its own, and the arithmetic that refuses is tested directly at P01-T001 |
| All four ingress counters are published: stale-epoch, out-of-horizon, pre-epoch-clamp, and arrival-stamp | ADR-0032 follow-up, assigned to Phase 1 by name | P01-T004 carries all four; P01-T005 increments the three the renderer can observe | **Passes** |
| An event whose epoch is not the renderer's is discarded and counted; one beyond `forward_event_horizon` is rejected and counted; an `Arrival`-stamped one is counted; a late event is clamped forward and counted | ADR-0032 clauses 20, 21, 19; ADR-0001 clause 16 | P01-T005 | **Passes** — four tests, and the late case asserts the pre-epoch counter stays at zero so the two are not conflated |
| The **pre-epoch clamp** is the ingress mapper's, so Phase 1 publishes the counter and never increments it: the envelope's `time` is an unsigned `SampleTime`, so a pre-zero stamp is unrepresentable by the time it reaches the renderer, and ADR-0032 assigns the mapper and both pre-epoch tests to Phase 3 | ADR-0032 clause 22 and its follow-up table | Phase 3 | Deferred, deliberately |
| A per-quantum event count over `max_events_per_quantum` is rejected before renderer state or output is mutated | Specification, *Deferred to Phase 3* rule 2 | P01-T005 | **Passes** — the clock, the output, and every counter are asserted unchanged after the refusal |

## Defects the review found, and what now guards them

Seven findings from the independent review of the implementation, five at P1. Each is
recorded with the check that would now catch it, because a fix without one is a fix that
comes back.

| Defect | Guard |
|--------|-------|
| A control event inside a call's **final** quantum was never applied: the loop applied at each boundary and returned with the rest unapplied, and the next call cleared the scratch. Automation became a function of how the host partitioned its callbacks — the defect ADR-0001 exists to remove, and invisible to a test that renders in one call | `an_event_inside_a_calls_final_quantum_takes_effect_in_the_next_call` |
| The audio thread's work was a function of what the producer sent, not of a declared capacity: an event skipped for a stale epoch or a distant stamp never reached the per-quantum tally, so a million of them would each be examined | `a_span_larger_than_any_call_can_admit_is_refused_before_it_is_scanned`, and the check runs before any classification |
| `render_offline` took stamped events while issuing the epoch itself, so no caller could produce a matching stamp and every offline event was silently discarded as stale | `OfflineEvent` carries no epoch; `an_offline_event_is_stamped_with_the_epoch_preparation_issued` |
| The scratch budget counted the audio buffers and both carries but not the event scratch, so a raised `max_events_per_quantum` was reported as fitting and then allocated past the budget at preparation | `the_scratch_budget_counts_what_preparation_actually_allocates`, and the figure now comes from the module that allocates it |
| The IR took raw `f32` for oscillator controls and for an event's value. A `NaN` frequency poisons a phase accumulator permanently — every later sample is `NaN` and no later event recovers it | `Frequency`, `Amplitude`, and `ParameterValue` refuse non-finite at construction; `the_dsp_control_types_refuse_what_would_poison_a_phase` |
| `SampleTime::difference` wrapped: a forward position at the top of the range read as one frame in the *past*. ADR-0032 clause 3 calls an unrepresentable difference a fault, and the wrapping form made it a sign flip | `difference` returns a `Result`; `a_difference_too_large_to_represent_is_a_fault_rather_than_a_sign_flip` covers both boundaries and `i64::MIN`'s asymmetric magnitude |
| An edge into any output port but the first, or a second output node, compiled and rendered silence with nothing said | `an_output_port_this_phase_does_not_render_is_refused_rather_than_dropped`, `a_second_output_node_is_refused_rather_than_ignored` |

A second pass over the fixes found four more, two at P1. Both P1s were reachable through
the API as it stood:

| Defect | Guard |
|--------|-------|
| A zero capacity could reach admission: the group constructors take already-built newtypes, so `measured(0)` bypassed `limit`'s own check, and a plan would then be refused against a budget of nothing | Every group constructor validates, and names the **field** rather than the type; `a_zero_capacity_is_refused_by_the_group_constructor_and_named_by_field` covers one per group |
| A negative frequency — explicitly legal, and meaning a backwards phase — broke the phase invariant: wrapping only at 1.0 let the phase fall below zero and grow without bound, feeding `sin` ever-larger arguments and losing precision to range reduction instead of staying periodic | `a_negative_frequency_stays_periodic_and_is_the_positive_render_inverted`, which is exact rather than approximate because `sin(-x) == -sin(x)` |
| The script-work aggregate saturated at `u32::MAX`: instructions per program times evaluations per quantum is a product of two separately admissible `u32`s, and a trillion instructions were reported as four billion | `ScriptWorkPerQuantum` is `u64`-backed; `script_work_survives_a_product_that_would_overflow_a_u32` |
| `render_offline` took a raw `usize` length, so a caller could pass a sample or byte count without a type error | It takes a `FrameCount` and converts once, internally |

## Deviations

Recorded rather than absorbed. Each names the authoritative document it changes and the reason.

1. **`HOST-INV-005`'s three grounds are scoped to the `RenderLimits` half** (host-profile specification). As written the
   invariant applies them to every profile field and its conformance test "fails on a field in none", yet three
   capability fields — `sample_rate`, `channel_layout`, and `source` — match none of the three, and
   `maximum_block_size` would match ground 1 through `LIMIT-0001` while also being a queried capability, which breaks
   *exactly one*. The capability half is a closed enumerated set fixed by ADR-0021 part 4 and ADR-0032 clause 12; the
   three grounds govern the budget half. Their ledger entries stay as **provenance**, which is the treatment the
   specification already gives `max_held_notes` and `max_events_per_quantum`.
2. **`RenderConfig` carries the profile and not a second `sample_rate`** (master plan Phase 1 work list). The plan's
   sketch predates the `Current` specification, which puts the rate in `HostCapabilities`; carrying both would give one
   stream two rates and make `HOST-INV-001`'s "no field is read from a global" unenforceable at the boundary that
   matters. The plan is updated in the same change, as ADR-0001's own follow-up table did for `quantum`.
3. **The mix-channel capacity is `MixChannelCount`, not `ChannelCount`.** `synth_core::ChannelCount` already exists in
   this workspace and means a channel *layout* — `Mono`, `Stereo`, `Multi(n)`. Reusing the name for a count of mix
   channels is the hazard ADR-0032 clause 5 refused for `SampleOffset`: two unrelated meanings one import away. The
   host-profile specification's newtype table and `max_mix_channels` row are updated in this change, so the contract and
   the implementation say the same thing.
4. **`ChannelLayout` carries `Mono` and `Stereo`, which is a refusal to invent the vocabulary rather than a decision
   about it.** ADR-0002 is `Proposed` and owns what a layout may be. Adding a `Multi(n)` variant now would be the claim —
   it asserts that a layout *is* a channel count, when ADR-0002 may just as well define speaker roles or a layout set —
   and it would then require a failure policy for a value no accepted decision defines. Carrying only what this phase
   renders makes no such claim and needs no policy: an unrenderable layout is not constructible, so there is nothing to
   refuse. This is the specification's requirement that Phase 1 "must not claim multichannel rendering merely because it
   can carry the value", met by not carrying it. Phase 9 queries a real device and is the first phase with somewhere to
   put a multichannel value; it adds the variant together with ADR-0002.
5. **The accepted sample-rate range becomes a `RenderLimits` field, in a new `stream` group.** The specification listed
   it in the capability table — the half defined as *queried, never chosen* — while `LIMIT-0004` classifies it as a
   configurable budget owned by `HostProfile` whose rule refuses an out-of-range job *at admission*, and
   `HOST-INV-005` ground 1 reserves participation in admission to limits. A capability is what a plan is prepared
   against; a limit is what admission refuses on. Two earlier revisions of this deviation got it wrong in both
   directions — leaving it a capability, then demoting it to a constructor constant, which would have changed a settled
   classification without a successor decision — and independent review caught both. The capability set is therefore the
   four fields the specification's types sketch always had.
6. **`ResourceReport` rows carry one enum over the *typed* amounts.** `HOST-INV-006` requires a row for every field,
   which means the rows must be enumerable — but enumerability does not require erasing the units. An earlier revision
   of this deviation proposed a generic value-plus-unit pair, which would let a requested count be paired with the wrong
   unit at runtime and drop the guarantee `HOST-INV-018` exists for. One enum whose variants carry `NodeCount`,
   `PreparedBytes`, `FrameCount` and the rest gives both: a single row type, and no amount without its unit.
7. **Quantity newtypes get two constructors, and they enforce different invariants — neither is unchecked.** A profile
   capacity of zero admits nothing, so the *limit* constructor rejects zero as well as anything outside the type's domain.
   A *measured* amount of zero is ordinary, so the measurement constructor permits zero — but still rejects an invalid
   representation, which for a float means non-finite: `CostRatio` must not be able to carry `NaN` into the report. This
   keeps `HOST-INV-018`'s fallible construction where the domain invariant lives, on the same footing as
   `HostCapabilities`' two constructors, where which one ran is itself the guarantee.
8. **The DSP control values are newtypes too, not only the profile's fields.** `HOST-INV-018`
   governs the profile; the same rule applies with more force inside the renderer, where a
   non-finite value cannot be refused at all — the loop can neither allocate a diagnostic
   nor clamp without hiding the fault. `Frequency`, `Amplitude`, and `ParameterValue` are
   validated at the boundary they cross, which the review found the IR was not doing.
9. **V2 defines its own checked `SampleRate`.** The specification's newtype table said "existing `synth_core` newtype",
   whose `new` clamps `NaN`, zero, and negative to `1.0` — so a `HOST-INV-018` fallible constructor is unavailable and an
   invalid input is indistinguishable from a genuine 1 Hz value. ADR-0021 part 3 already replaces V1's clamping
   `VoiceCount` on exactly this ground, and the specification is updated to apply it here too. V1 keeps its type until it
   retires, and the conversion is **one-way**: V2 to `synth_core` is infallible and provided, because the permitted
   `synth_dsp` kernels take that type and the value has already been validated; the reverse does not exist, because a
   clamped `1.0` is indistinguishable from one hertz and a fallible signature would advertise a guarantee it cannot hold.

## Exit readiness

Status: **Complete.** [`REV-P01`](../reviews/phase-01-exit-review.md) is `Accepted`: every gate bullet has an executable
check that decides it, every contract row assigned to this phase by name passes or carries a stated and bounded limit,
and the repository quality gate is green. Eleven review findings were fixed rather than deferred.

## Next actions

1. Start Phase 2 — the minimal compiled voice graph — whose exit gate also owns the binding `Q` re-measurement.
2. Keep Phase 0B moving independently; it gates Phase 10, not this phase.
3. Carry ADR-0037's provisional `Q` into the Phase 2 re-measurement rather than treating 64 as settled. Nothing in the
   crate is tuned to 64: no kernel is unrolled to it, no buffer is laid out around it, and no test asserts a control
   rate in hertz.
