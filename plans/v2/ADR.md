# Core V2 Decision Index

This is the compact status index for durable Core V2 decisions. Individual ADRs own rationale and evidence; current
specifications own implementation semantics. Do not copy review history or measurement results into this file.

Next free identifier: `ADR-0058`.

## Status vocabulary

- `Proposed` — not yet authoritative;
- `Accepted` — approved durable choice;
- `Rejected` — considered and not selected;
- `Deferred` — postponed to a named evidence point or gate;
- `Superseded` — replaced by a successor.

Only an accepted ADR can change a current specification. Code follows the current specification rather than
reconstructing a contract from this index.

`Phase` names relevant ownership or consumer phases; it is not automatically an acceptance deadline. In particular,
an entry that includes `0B` does not block Phase 0B merely because of that label. Decision timing follows the readiness
test in [`PROCESS.md`](PROCESS.md#decision-timing-and-readiness).

## Decision classes

New ADRs use the durable-decision test in [`PROCESS.md`](PROCESS.md#durable-decision-or-evidence). Internal experimental
types, layouts, algorithms, and values are ordinary implementation choices unless they cross one of those durable
boundaries.

Existing ADR classifications and records remain historical; this workflow change does not retroactively reclassify or
rewrite them.

### The reversibility test

Historical records link this anchor and cite the numbered tests. Under the
former workflow, an entry was `Reversible` only when all four held:

1. **Value, not shape.** It selected a number, default, or threshold rather
   than a type, field, contract, ownership boundary, or error behavior.
2. **Nothing persisted it.** No serialized field, format, protocol, or public
   API carried or was sized by it.
3. **Reversal was a rebuild.** Changing it required recompilation and
   re-measurement, not a superseding contract, migration, or broad re-review.
4. **A named revisit point existed.** A phase gate or explicit condition kept
   the provisional value from being forgotten.

No entry currently uses the reversible table. New cheap internal choices use
the durable-decision test in `PROCESS.md` and normally do not need an ADR.

## Register

| ID       | Topic                                       | Status   | Phase                 | Record                                                             | Note                                     |
|----------|---------------------------------------------|----------|-----------------------|--------------------------------------------------------------------|------------------------------------------|
| ADR-0001 | Render quantum semantics and splitting      | Accepted | 0A/1                  | [ADR](decisions/ADR-0001-internal-render-quantum.md)               | Clauses 12, 14 and 16 superseded by ADR-0043; clause 12's second sentence and the rest stand |
| ADR-0002 | Internal channel layout                     | Superseded | 2                   | [ADR](decisions/ADR-0002-internal-channel-layout.md)               | Superseded in full by ADR-0041, 2026-08-19 |
| ADR-0003 | Event segmentation API                      | Proposed | 3                     | —                                                                  | —                                        |
| ADR-0004 | Native node representation                  | Accepted | 2/5                   | [ADR](decisions/ADR-0004-native-node-representation.md)            | Acceptance rule B failed; record redrafted under rule C |
| ADR-0005 | Buffer liveness strategy                    | Accepted | 2                     | [ADR](decisions/ADR-0005-buffer-liveness-strategy.md)              | Clauses 1, 2, 4, 5, 7 and 8 superseded or refined by ADR-0041; 3, 6 and 9 stand |
| ADR-0006 | Parameter ramp representation               | Proposed | 3/5                   | —                                                                  | —                                        |
| ADR-0007 | Parameter modulation laws                   | Proposed | 5                     | —                                                                  | —                                        |
| ADR-0008 | YAMS state identity and reload policy       | Proposed | 7                     | —                                                                  | —                                        |
| ADR-0009 | Plan-swap crossfade and latency             | Proposed | 9                     | —                                                                  | —                                        |
| ADR-0010 | Compatible node-state migration surface     | Proposed | 9                     | —                                                                  | —                                        |
| ADR-0011 | Shared V1 instrument conversion             | Proposed | 10                    | —                                                                  | —                                        |
| ADR-0012 | Automation conflict policy                  | Proposed | 10                    | —                                                                  | —                                        |
| ADR-0013 | Project/session/settings boundary cases     | Proposed | 0B/10A                | —                                                                  | —                                        |
| ADR-0014 | Persistent ID generation and encoding       | Proposed | 0B/10A                | [ADR](decisions/ADR-0014-persistent-id-generation-and-encoding.md) | —                                        |
| ADR-0015 | History representation                      | Proposed | 10C                   | —                                                                  | —                                        |
| ADR-0016 | Known-version unknown-field policy          | Proposed | 0B/10D                | —                                                                  | —                                        |
| ADR-0017 | Asset identity and external references      | Proposed | 0B/10D                | —                                                                  | —                                        |
| ADR-0018 | Editor metadata persistence scope           | Proposed | 0B/10A                | —                                                                  | —                                        |
| ADR-0019 | Remote mutation history semantics           | Proposed | 10B/10C               | —                                                                  | —                                        |
| ADR-0020 | Final crate boundaries and names            | Proposed | After vertical slices | —                                                                  | —                                        |
| ADR-0021 | Host profile and admission policy           | Accepted | 0A/1                  | [ADR](decisions/ADR-0021-host-profile-and-admission-policy.md)     | —                                        |
| ADR-0022 | Hardware time mapping and latency ownership | Deferred | 0A/9                  | [ADR](decisions/ADR-0022-hardware-time-mapping.md)                 | Phase 3 consumes already mapped `SampleTime`; physical mapping and latency ownership gate Phase 9 exit |
| ADR-0023 | Same-sample event ordering                  | Accepted | 3                     | [ADR](decisions/ADR-0023-same-sample-event-ordering.md)            | Same-sample order is the **declared drain order** of one publication pass, with the **producer** as the unit of order: session and transport, compiled, authored runtime in plan declaration order, live ingress in queue order; each producer drained in one contiguous block in its own emission order; a renderer-internal emission applies after every external event at that position. Ranking by ADR-0046 capacity class is recorded as **refuted** — a class partitions who pays, not what happens first, and it splits one live producer's note pair across `Live` and `Release`. ADR-0051 clause 6's missing gate-ownership law is named as the coupled boundary that bounds what may be built on this record; a narrower reading of it was tried and withdrawn. Creates `SOUND-INV-020` on acceptance |
| ADR-0024 | Recording take and commit semantics         | Proposed | 0B/9/10B              | —                                                                  | —                                        |
| ADR-0025 | Tuning representation and ownership         | Accepted | 0B/4/6/10A            | [ADR](decisions/ADR-0025-tuning-representation-and-ownership.md)   | **Option B**: the event contract is pre-tuning — a note names a validated key identity, the plan carries `PreparedTuning`, and every pitch-producing node resolves through it. A per-note bend is a continuous offset in cents applied after resolution. Accepted with the velocity clause. The payload landed on 2026-09-02: `P03-R003` is closed and `P04-R001`'s payload half with it. What remains of `P04-R001` is V1's velocity composition, which this record and `SOUND-INV-021` put on Phase 6 and which Phase 4 exits carrying as a named residual |
| ADR-0026 | Minimum SampleMap and SampleZone model      | Proposed | 6/10A/10D             | —                                                                  | —                                        |
| ADR-0027 | Observation and analyzer ownership          | Accepted | 0B/5/9/10E            | [ADR](decisions/ADR-0027-observation-and-analyzer-ownership.md)    | Accepted 2026-09-04 with option 3, the master plan's split ownership: a persisted analyzer node owns authored intent only, a compiler-declared tap is the only subscribable point, the host owns bounded lossy subscriptions admitted by the profile, analysis runs on workers, one versioned telemetry facade serves GUI, OSC and the visualizer. Creates `SOUND-INV-022` (the tap, a compiler artifact present whether or not read) and `HOST-INV-023` (the host-owned subscription). Phase 9 verifies the live subscription contract; Phase 10E migrates OSC under it |
| ADR-0028 | Long-running job contract                   | Deferred | 0A/4/10B              | [ADR](decisions/ADR-0028-long-running-job-contract.md)             | Initial Phase 4 is limited to pure lowering and one bounded in-process smoke render. Accept before shared render orchestration, multi-project A/B, streaming, progress, cancellation or a frontend surface. **The Phase 4 deadline is withdrawn by `P04-R004`**: a *revisioned* contract needs Phase 10A's canonical revision and Phase 10B's job capture, so acceptance moves to Phase 10B. The workflow analysis it required is done and recorded in the ADR. Phase 4 exited on 2026-09-02 carrying `P04-R004` as a named residual, with its gate bullet rewritten to ask that no streaming, progress, cancellation or shared render surface be built here; constraint 3 still refuses that work as a task selection |
| ADR-0029 | Host configuration and remote authorization | Proposed | 0B/10E                | —                                                                  | —                                        |
| ADR-0030 | Public facade and compatibility surface     | Proposed | Before 10E            | —                                                                  | —                                        |
| ADR-0031 | Supported build and release matrix          | Proposed | 0B/12                 | —                                                                  | —                                        |
| ADR-0032 | Sample-time and event-timestamp model       | Accepted | 0A/3                  | [ADR](decisions/ADR-0032-sample-time-and-event-timestamps.md)      | Clause 16 superseded by ADR-0043. **Clauses 18 and 21 amended by ADR-0053**, which adds a fourth provenance for a deterministic simulated ingress producer and binds the forward horizon to it; the rest stands |
| ADR-0033 | Graph feedback and delay-boundary rule      | Proposed | 2/3                   | —                                                                  | —                                        |
| ADR-0034 | Track, source, and channel ownership        | Proposed | 0B/10A                | —                                                                  | —                                        |
| ADR-0035 | Transaction and concurrency semantics       | Proposed | 0B/10B                | —                                                                  | —                                        |
| ADR-0036 | Audio device and input lifecycle            | Proposed | 0B/9                  | —                                                                  | —                                        |
| ADR-0037 | Render quantum frame count                  | Accepted | 0A/1                  | [ADR](decisions/ADR-0037-render-quantum-value.md)                  | `Q` = 64, final after EVD-0012's escalation |
| ADR-0038 | Engine-egress queue classification          | Accepted | 0A/1                  | [ADR](decisions/ADR-0038-engine-egress-queue-classification.md)    | —                                        |
| ADR-0039 | Initial multi-client hub omission           | Proposed | 0A/10E                | [ADR](decisions/ADR-0039-multi-client-hub-delivery-contract.md)    | —                                        |
| ADR-0040 | V2 owns its DSP                             | Accepted | 2                     | [ADR](decisions/ADR-0040-v2-owns-its-dsp.md)                       | Accepted with ADR-0041 as its clause 7 requires |
| ADR-0041 | Interleaved internal channel layout         | Accepted | 2                     | [ADR](decisions/ADR-0041-interleaved-internal-channel-layout.md)   | Supersedes ADR-0002 in full; P02-T013 converts the crate |
| ADR-0042 | Envelope segment shape                      | Accepted | 2                     | [ADR](decisions/ADR-0042-envelope-segment-shape.md)                | EVD-0013's envelope shape difference; `CORPUS-0001-C2` added 2026-08-20. CORPUS-0001-P2 untouched |
| ADR-0043 | Event deferral and the late clamp           | Accepted | 3                     | [ADR](decisions/ADR-0043-event-deferral-and-late-clamp.md)         | Capacity-deferral half superseded by ADR-0046. The preserving late clamp, immutable stamp and control-response rule remain accepted |
| ADR-0044 | Deferral-induced causal order               | Superseded | 3                   | [ADR](decisions/ADR-0044-deferral-causal-order.md)                 | Dissolved by ADR-0046 rather than answered: the selective `+Q` capacity movement that created the same-control hazard no longer exists |
| ADR-0045 | Cross-control causal order under deferral   | Superseded | 3                   | [ADR](decisions/ADR-0045-cross-control-causal-order.md)            | Dissolved by ADR-0046 rather than answered: the selective `+Q` capacity movement that created the cross-control hazard no longer exists |
| ADR-0046 | Destination-quantum admission               | Accepted | 3                     | [ADR](decisions/ADR-0046-destination-quantum-admission.md)         | Fixed producer shares, plan-time envelopes, one publication arbiter and no renderer movement for capacity. ADR-0054 supersedes only its Phase 3 numeric-selection deadline; ADR-0055 supersedes runtime loop adoption while playback is unsupported |
| ADR-0047 | Note identity in the event contract          | Accepted | 3                     | [ADR](decisions/ADR-0047-note-identity-in-the-event-contract.md)   | Makes ADR-0046 clause 3's orphan-release sentence implementable. `SOUND-INV-017` is its contract; `HOST-INV-009` gained a third drop cause. Decides identity only; the 2026-09-01 phase-boundary correction carries pitch and velocity as P03-R003 to the first Phase 4 saved-note consumer. Clause 7's generation-advance timing is **amended by ADR-0050** for the transport-activation case |
| ADR-0048 | Note obligation across an identity-table rebuild | Proposed | 9                 | —                                                                  | Split out of ADR-0047 by its fifth review round. ADR-0047 clause 8 refuses the rebuild meanwhile; ADR-0009's plan swap is the coupled topic |
| ADR-0049 | Tempo-ramp law under clause 15               | Accepted | 3                     | [ADR](decisions/ADR-0049-tempo-ramp-law.md)                         | Fills the hole ADR-0032 clause 15 names rather than amending it: a ramp interpolates the **period**, so elapsed time is a quadratic the four operations express and no near-flat branch exists. Creates `SOUND-INV-019`. V1's declaration is unchanged, so Phase 4's lowering translates nothing — but the timing differs, and the reference corpus **does** contain a ramp, which makes it an intentional V1-to-V2 difference Phase 4's A/B owes a comparison category. The inverse conversion is deliberately not defined |
| ADR-0050 | Transport activation                         | Accepted | 3                     | [ADR](decisions/ADR-0050-transport-activation.md)                   | Settles six coupled questions as one contract: a quantum-granular half-open activation point, the kept carry, the atomic state set, notes ended at the boundary, a monotone activation sequence, and the locate catch-up batch. Creates `SOUND-INV-018` and `HOST-INV-022`, and **amends ADR-0047 clause 7** for the activation case, which `SOUND-INV-017` records. Scoped to a stream whose note producers are compiled; clause 8 names the obligations that bounds, **raised to three by ADR-0051**. Clause 7's gate case is **amended by ADR-0051**. Sample-exact seek and loop are explicitly not claimed; ADR-0055 supersedes clause 3's recorded-but-unenforced loop behavior with fail-closed refusal |
| ADR-0051 | What a locate owes a gate                    | Accepted | 3                     | [ADR](decisions/ADR-0051-locate-catch-up-gate-exception.md)         | **Amends ADR-0050 clauses 7 and 8.** A locate computes the last pre-destination write for every prepared target, then substitutes `ZERO` for every physical `(node, control)` gate held open by an in-scope note contract at the destination. Taking clause 7 literally makes the boundary's release-then-restore pair a **rising edge** on an edge-triggered control, restarting an envelope no note contract stands behind. The batch's size is unchanged, so `HOST-INV-022`'s bound holds. Adds clause 8's third obligation: one scalar gate reached by two producers has no ownership law |
| ADR-0052 | Where a loop wrap's note identity comes from | Proposed | 3                     | [ADR](decisions/ADR-0052-loop-wrap-note-identity.md)                | **Deliberately undecided, and the record says why.** Three designs went to independent design consultation and all three were refuted; what the rounds established is a coupled boundary. A quantum-granular wrap truncates or overruns the pass whenever the loop's length is not a whole number of quanta — 30 of 281 integer tempi at 48 kHz give one — so the wrap is where ADR-0050 clause 1's deferred sample-exactness stops being optional. Carries fifteen verified constraints, four closed options and one left open with its cost stated, so the next attempt does not rediscover them. The loop's **admission** is built and is unaffected |
| ADR-0053 | What a simulated ingress producer stamps    | Accepted | 3                     | [ADR](decisions/ADR-0053-simulated-ingress-provenance.md)           | **Amends ADR-0032 clause 18** from three provenances to four. All three existing values are wrong for a deterministic simulated producer: `Compiled` is horizon-exempt and would make the exit gate's equivalence fixture vacuous, `Hardware` is EVD-0016's F11 defect without the clock bridge, and `Arrival` understates an exact timestamp while moving a fallback counter. `Simulated` is ingress, so the horizon binds. Updates `HOST-INV-013`'s enumeration. The variant lands with the producer that stamps it; the API break is approved |
| ADR-0054 | Staged producer-capacity calibration        | Accepted | 3/5/7/9               | [ADR](decisions/ADR-0054-staged-producer-capacity-calibration.md)    | Supersedes ADR-0046's Phase 3 numeric-selection deadline. Provisional values may exercise compiled, session and simulated ingress; each first real authored/internal producer measures before use, and Phase 9 reselects the complete partition before a production live adapter can be enabled |
| ADR-0055 | Refuse unimplemented loop playback          | Accepted | 3/9                   | [ADR](decisions/ADR-0055-refuse-unimplemented-loop-playback.md)      | Supersedes ADR-0050 clause 3's recorded-but-unenforced loop behavior and ADR-0046 clause 4's runtime-adoption sentence while the guard stands. A loop candidate still passes off-thread density and polyphony admission, but the runtime offer refuses it with the interval named until ADR-0052's sample-exact successor exists |
| ADR-0056 | V1-to-V2 consumer boundary                  | Accepted | 4                     | [ADR](decisions/ADR-0056-v1-to-v2-consumer-boundary.md)            | Phase 4's lowerer is the first non-harness consumer, so the Phase 1 gate's deletability claim needs a boundary rather than a deletion. A non-default `v2-lowering` feature carries the one optional edge; `crate_boundary` gains a check that enabling it adds exactly one dependent |
| ADR-0057 | Refuse a parity verdict over a placed note  | Accepted | 4/6                   | [ADR](decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) | V1 applies one saved velocity twice and V2 applies it once, so every lowering that places a note is `UnsupportedScope` and no parity verdict may read it. The obligation is `P04-R001`, owned by Phase 6 with the composition law. Phase 4's exit gate and `ROADMAP.md`'s Phase 4 outcome are amended not to claim the deferred behaviour, and the gate's project count becomes the measured eligible set |

### Reversible decisions

| ID | Topic | Value | Status | Revisit at |
|----|-------|-------|--------|------------|
| —  | —     | —     | —      | *(none)*   |

## Maintenance

For a durable decision:

1. allocate the next ID and create the ADR from the lean template;
2. link the evidence or current code that verifies its factual premises;
3. obtain one independent semantic review;
4. on acceptance, update this row and the affected current specification;
5. update `NOW.md` only when active work changes.

Historical phase records, measurements, reviews, and superseded ADR prose are not status consumers and are not rewritten
to agree with the present.
