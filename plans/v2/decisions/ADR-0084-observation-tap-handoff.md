# ADR-0084: Observation tap handoff

| Field | Value |
|---|---|
| ID | ADR-0084 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0027, ADR-0074, ADR-0082, ADR-0083 |
| Supersedes | — |
| Superseded by | — |

## Boundary

The maintainer selected observer handoff as part of structural plan editing.
This decides the bounded store primitive and its stopped/running session consumers.
It does not enable observation in the application, change sound admission or
define a cross-thread subscription command protocol.

## Decision

Prepare a mapping off-thread from one admitted store's subscriptions to the
successor plan by authored node and output-port identity. Plan-relative tap
indices never establish identity. The host must establish the graph's identity
continuity; lowered application addresses require its authoritative lineage
fence, including coalesced deletion/recreation. Matching channel count and tap
data are required; a rate change refuses handoff. Missing taps end explicitly.
An ended handle never revives after same-ID recreation: a fresh subscription
has a fresh store-local index.

Each descriptor binds the issuing store, source and destination plan, checked
store generation and complete subscription configuration. At the actual sound
transition's quantum boundary, preflight every binding before mutation. A
foreign, stale, replayed or changed-configuration descriptor leaves the store
untouched. Generations advance without wrapping, even if the host reinstalls
an earlier immutable plan. The descriptor is borrowed at installation and
returned for off-thread destruction; installation allocates or frees nothing.

Handoff discards unread outgoing frames and adds them to the existing loss
count. Ring owners and cumulative cursors remain. Every read labels its frames
with the actual store plan, generation and ended state. Late outgoing-plan
pushes remain ignored. No read can combine outgoing and successor samples
without a visible boundary. Existing dropped-frame counts survive the handoff.

Observation metadata uses its own explicit byte allowance. Observation refusal
cannot refuse a sound edit: the coordinated consumer ends observation instead. Preparation failure never changes the current store. The host
must coordinate the descriptor with the sound transition, not apply it on a
speculative offer. A failed sound install leaves the old observation binding.
The running-session consumer pairs `begin_running_replacement_observed` with
`render_running_replacement_observed` for every transition fragment. It applies
the descriptor only after successful reset or compatible continuation at B.
Sound refusal leaves the store untouched. Missing, refused or stale observation
preparation ends the old store; its labelled outgoing tail remains readable.
A receipt distinguishes handoff, observation refusal and interrupted handoff,
and identifies whether sound reset or continued when handoff succeeded. Same-plan fresh-owner resets
still advance the observation generation.

The stopped consumer calls `install_replacement_observed` with its prepared
mapping. Both stopped reset and compatible DSP migration use the same post-sound
handoff operation as running replacement. All sound checks and migration
transfer complete before observation changes. Refused sound installation and
cold cancellation leave the store unchanged. Missing or refused observation
preparation still permits sound installation and ends the old labelled tail.
The receipt records the stopped reset/compatible kind on successful handoff.

The stopped sound acknowledgement remains immediate; this adds no fade or
first-render acknowledgement. Subsequent `render_observed` calls use the same
store. Idle stopped quanta advance no DSP and produce no tap samples; after
Play the first actually rendered successor quantum is observed exactly once.
A same-plan reset still advances observation generation and restarts DSP under
its existing reset law; compatible migration retains its eligible warm history.
The stopped `HandedOff` receipt is final at installation; it cannot detect a
later host render bypass or a different store supplied after Play.
An envelope with observation attached cannot use the legacy stopped install:
it returns whole for retry through the observed entry, without spending its
sound attempt or rebinding observation.

The control owner prepares from an immutable, independently budgeted
`ObservationConfiguration` before audio takes the store. It can predict a
successor configuration without borrowing audio's rings. Installation checks
all row geometry as well as store, plan and generation; stale predictions
cannot authorize replay. Metadata remains in the owning replacement envelope
until off-thread collection or cancellation.

During a reset fade, taps report the successor's signal before fade gain. The
outgoing scratch owner has no store. The successor's first quantum is observed
once, including when callback fragments later consume its carry. A render fault
ends the installed observation generation. Observation errors neither choose
sound admission nor change callback output. Hosts must use the paired observed
entries and the same store throughout a transition. An attached envelope cannot
begin through the legacy entry; it is returned whole for correct retry. During
running transitions, bypassing observed rendering after handoff, or supplying a
different store, records `Detached` without claiming that the inaccessible
original store was ended.
The supplied wrong store is ended; the host must end or reclaim the original.
These APIs do not enforce the complete call sequence through ownership types.
Legacy unobserved stopped and running entries retain their explicit gap policy:
end the old store before offering and bind a fresh one after installation,
including after refusal or cancellation. The application still uses that policy;
these core entry points do not activate application telemetry.

## Falsifiers and stopping rule

Wrong-tap samples after a reordered table, unlabelled mixed-plan samples, revived
ended handles, lost loss counts, allocation/destruction during installation,
partial mutation on refusal, replay after cycling plans or generation wrap
blocks acceptance. Optional implementation detail does not.

`observe::handoff::tests` exercises stable-address remapping, loss accounting,
late old pushes, removed/recreated taps, foreign and changed descriptors,
metadata-budget refusal, replay and generation exhaustion. Existing observation
controls continue to require identical audio with no, draining or stalled
subscribers. Running replacement controls compare successor taps against an
ordinary successor across four sample rates and callback partitions, saturate
a stalled ring, retain the old tail on observation refusal, leave the store
unchanged on sound refusal and end it on render interruption. A structural
continuation control retains actual held-voice sound and later releases while
observing it. Stopped controls compare reset and migrated sound with unobserved
installation across four rates, callback partitions and stalled ring pressure.
They compare compatible history with the unedited warm owner and reset with a
fresh ordinary owner, including same-plan-ID installation. Refused migration,
carry, cancellation and legacy-entry retry do not alter observation. Cross-thread
subscriber lifecycle remains a separate consumer; Phase 9 is not declared complete.
