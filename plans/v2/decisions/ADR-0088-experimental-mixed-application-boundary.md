# ADR-0088: Experimental mixed application boundary

| Field | Value |
|---|---|
| ID | ADR-0088 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0022, ADR-0055, ADR-0056, ADR-0065, ADR-0078, ADR-0081, ADR-0087 |
| Supersedes | — |
| Amends | ADR-0056 assertion 6; ADR-0078 clause 1 and ADR-0081 application-route refusal, only for the exact feature forwarding below |
| Superseded by | — |

## Concrete approval scope

Add one non-default Cargo feature to `crates/pertylizer/Cargo.toml`:

```toml
v2-mixed-app = ["v2-lowering", "synth_engine_v2/mixed-transfer-rehearsal"]
```

Keep the current optional dependency declaration and default feature list.
Update `crates/synth_engine_v2/tests/crate_boundary.rs` to permit exactly this
additional feature forwarding rule, exclusively through the existing application
lowering subtree. No other production dependency, crate, module or feature gains
access. Existing `v2-lowering` compiled playback and its qualified partition
retain their behavior. This is an explicit exception to the existing no-forwarding
rule, not an inference from the unstable Rust API approval. It explicitly invokes
ADR-0056's two-feature revisit condition and amends its assertion 6. It also
amends ADR-0078 clause 1 and its application-route/forwarding falsifier for this
one named feature, including the retained application refusal in ADR-0081.
ADR-0078's hardware-input refusal and requirement that both bend tables cover
all sixteen channel scopes remain binding. This decision authorizes feature
forwarding only; it does not authorize hardware input or bypass mixed arming.
The `no_shipping_feature_forwards_the_mixed_transfer_rehearsal` boundary test
must permit exactly this exception across all workspace member features.
`only_the_lowering_feature_activates_the_optional_dependency` must pin the
activator set to exactly `v2-lowering` and `v2-mixed-app`, permit only
`v2-mixed-app` to forward to `v2-lowering`, and pin its exact value list above.
`v2-lowering` alone must still not enable the mixed rehearsal. Every other
shipping feature remains forbidden from forwarding into it.

The feature authorizes a separately selected experimental application route for
compiled/live playback, transport, loops and replacements. It is not selected
by the ordinary V2 toggle or enabled in a default build. It does not qualify a
production mixed profile, physical MIDI timestamps or hardware timing. Do not
select six-share limits from synthetic work or silently reuse EVD-0025's
compiled-only qualification for mixed playback. Canonical project revision,
persisted formats and MCP response contracts remain unchanged.

## Required implementation and checks after approval

A successor current specification must define the mixed application's owning
control/audio halves, live source admission, stop/seek/loop boundaries, state
migration and retirement. Existing rehearsal loop support does not lift the
compiled session's ADR-0055/0065 refusals. New laws need their own bounded
acceptance controls before the application route uses them.

Extend `.github/workflows/quality.yml` and the repository delivery instructions
with explicit check, Clippy and application integration tests for
`--no-default-features --features v2-mixed-app`, plus GUI integration coverage
with `--features v2-mixed-app`. Keep no-default, all-feature and MSRV workspace
coverage. The dependency boundary test must show that default builds have no
normal V2 dependency and that no unlisted feature forwards into V2. The full
repository gate and an independent final uncommitted review are required before
commit; approval of this proposal does not waive either.

## Falsifiers and stopping rule

Acceptance fails if mixed code becomes default, reaches an unauthorized module,
changes compiled-only behavior, claims unavailable producer capacity or physical
qualification, bypasses an existing loop/identity contract, or changes persisted
or protocol meaning. ADR-0022 constraints 1–3 remain binding: no implementation may invent a
mapping; no production path may consume `output_latency`, a host timestamp or
`stream_time` before ADR-0022 is accepted; a new GUI, recording or event-mapping
consumer would create an unwritten contract;
an untimestamped adapter must declare its `unmeasured` fallback. Refusal preserves
old sound and complete owning custody.
A missing producer, proof or platform bound blocks the relevant enablement;
optional implementation detail does not.

## Approval status

The maintainer explicitly approved the exact feature forwarding and associated
boundary/CI changes on 2026-10-18. This approval does not authorize new unsafe
implementation, qualify the mixed profile or lift the required implementation
contracts and verification gates above.
