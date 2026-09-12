# ADR-0061: Ordered compiled-session ownership

| Field | Value |
|---|---|
| ID | ADR-0061 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-11 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0050, ADR-0051, IO-INV-002 |
| Supersedes | — |
| Superseded by | — |

Current contract: [host I/O lifecycle](../specs/spec-host-io-lifecycle.md#ordered-compiled-transport)

## Decision boundary

The first audible session owns the point at which a prepared transport activation
enters the renderer. This is a real-time ownership boundary shared by the host,
scheduler and control-side identity minter. Its bounded serial implementation is
experimental; concurrent publication, capture coupling and physical adapters must
establish their additional contracts before consuming it.

## Decision

The renderer owns the only advancing engine clock. A stopped session keeps a frozen
`PlanPosition`; a playing session maps time through its current `StreamAnchor`.
Stopped rendering generates silent quanta without advancing DSP or reading the
old schedule. Existing output carry remains deliverable. The old renderer anchor
is not the stopped playhead.

Play always prepares an ADR-0050 activation off-thread from the frozen position at
the requested new engine time. Consequently crossing notes are cut on resume,
parameter catch-up follows ADR-0051, and later notes retain their compiled edges.
Idle retains dormant voice state; it neither redeems a live identity nor claims
live-note migration. The first session admits compiled playback only.

An accepted command reserves its identified outcome in the same bounded slot.
Play also retains its activation there, **before** scheduler offer. A following
Stop at the same time cancels that pre-offer value. Otherwise the callback offers
Play immediately before rendering the one quantum beginning at its requested
time. No action may cancel an already offered activation. The returned retired
value goes back into the same reserved slot; only control-side receipt collection
promotes or withdraws it and destroys resources. At most one Play remains
outstanding until collection. Play cannot consume the final slot reserved for
Stop, although Stop remains subject to time-order and producer-share admission.

The callback first validates the whole external buffer, drains carry, and then
handles each new quantum individually. Commands use aligned engine times, ordered
by time and submission order. Stop snapshots the playing position before that
quantum. Stop operations and Play release/catch-up spend the same publication
arbiter's session share. Admission conservatively includes Play's entire catch-up
cost even if a later same-time Stop would cancel it.

Loss closes admission without needing another callback. Unapplied commands become
retained cancellations; applied outcomes stay applied. A fenced, permanently closed
scheduler may transfer its pending exchange to the control owner for withdrawal.
Backend resource retirement waits for every command outcome to be collected.

## Alternatives and falsifier

Changing an affine renderer anchor while freezing its musical position would
confuse engine time with playhead time. Freezing the engine clock would also leave
no advancing timeline for later input and capture. The explicit stopped state
avoids both problems without changing the renderer's clock contract.

Offering a future activation immediately and later cancelling it contradicts
ADR-0050's infallible accepted offer. Keeping it pre-offer until the exact boundary
preserves that contract. This serial lane has no urgent insertion ahead of a
future accepted command; a concurrent owner must decide that policy explicitly.

Acceptance is falsified by differing samples or command outcomes for identical
commands under different callback partitions, an applied cancelled Play, a
retriggered crossing note, a lost accepted outcome, or any allocation/deallocation
in the callback. `tests/host_session.rs`, `src/host/session/tests.rs` and the
transitive render-loop purity checks exercise these properties. Source timing,
recording, live-note migration, concurrency and physical qualification are not
established by those tests and remain first-consumer gates.
