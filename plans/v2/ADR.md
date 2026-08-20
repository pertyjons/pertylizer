# Core V2 Decision Index

This is the compact status index for durable Core V2 decisions. Individual ADRs own rationale and evidence; current
specifications own implementation semantics. Do not copy review history or measurement results into this file.

Next free identifier: `ADR-0043`.

## Status vocabulary

- `Proposed` — not yet authoritative;
- `Accepted` — approved durable choice;
- `Rejected` — considered and not selected;
- `Deferred` — postponed to a named evidence point or gate;
- `Superseded` — replaced by a successor.

Only an accepted ADR can change a current specification. Code follows the current specification rather than
reconstructing a contract from this index.

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
| ADR-0001 | Render quantum semantics and splitting      | Accepted | 0A/1                  | [ADR](decisions/ADR-0001-internal-render-quantum.md)               | —                                        |
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
| ADR-0022 | Hardware time mapping and latency ownership | Deferred | 0A/3/9                | [ADR](decisions/ADR-0022-hardware-time-mapping.md)                 | —                                        |
| ADR-0023 | Same-sample session event ordering          | Proposed | 3                     | —                                                                  | —                                        |
| ADR-0024 | Recording take and commit semantics         | Proposed | 0B/9/10B              | —                                                                  | —                                        |
| ADR-0025 | Tuning representation and ownership         | Proposed | 0B/6/10A              | —                                                                  | —                                        |
| ADR-0026 | Minimum SampleMap and SampleZone model      | Proposed | 6/10A/10D             | —                                                                  | —                                        |
| ADR-0027 | Observation and analyzer ownership          | Proposed | 0B/5/9                | —                                                                  | —                                        |
| ADR-0028 | Long-running job contract                   | Deferred | 0A/4/10B              | [ADR](decisions/ADR-0028-long-running-job-contract.md)             | —                                        |
| ADR-0029 | Host configuration and remote authorization | Proposed | 0B/10E                | —                                                                  | —                                        |
| ADR-0030 | Public facade and compatibility surface     | Proposed | Before 10E            | —                                                                  | —                                        |
| ADR-0031 | Supported build and release matrix          | Proposed | 0B/12                 | —                                                                  | —                                        |
| ADR-0032 | Sample-time and event-timestamp model       | Accepted | 0A/3                  | [ADR](decisions/ADR-0032-sample-time-and-event-timestamps.md)      | —                                        |
| ADR-0033 | Graph feedback and delay-boundary rule      | Proposed | 2/3                   | —                                                                  | —                                        |
| ADR-0034 | Track, source, and channel ownership        | Proposed | 0B/10A                | —                                                                  | —                                        |
| ADR-0035 | Transaction and concurrency semantics       | Proposed | 0B/10B                | —                                                                  | —                                        |
| ADR-0036 | Audio device and input lifecycle            | Proposed | 0B/9                  | —                                                                  | —                                        |
| ADR-0037 | Render quantum frame count                  | Accepted | 0A/1                  | [ADR](decisions/ADR-0037-render-quantum-value.md)                  | `Q` = 64, final after EVD-0012's escalation |
| ADR-0038 | Engine-egress queue classification          | Accepted | 0A/1                  | [ADR](decisions/ADR-0038-engine-egress-queue-classification.md)    | —                                        |
| ADR-0039 | Initial multi-client hub omission           | Proposed | 0A/10E                | [ADR](decisions/ADR-0039-multi-client-hub-delivery-contract.md)    | —                                        |
| ADR-0040 | V2 owns its DSP                             | Accepted | 2                     | [ADR](decisions/ADR-0040-v2-owns-its-dsp.md)                       | Accepted with ADR-0041 as its clause 7 requires |
| ADR-0041 | Interleaved internal channel layout         | Accepted | 2                     | [ADR](decisions/ADR-0041-interleaved-internal-channel-layout.md)   | Supersedes ADR-0002 in full; P02-T013 converts the crate |
| ADR-0042 | Envelope segment shape                      | Proposed | 2                     | [ADR](decisions/ADR-0042-envelope-segment-shape.md)                | EVD-0013's envelope shape difference; adds CORPUS-0001-C2. CORPUS-0001-P2 is **not** amended |

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
