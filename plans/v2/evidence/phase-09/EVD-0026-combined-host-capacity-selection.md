# EVD-0026: Combined-host capacity selection

| Field | Value |
|---|---|
| ID | EVD-0026 |
| Status | Complete |
| Phase | 09 |
| Created | 2026-10-14 |
| Last reviewed | 2026-10-17 (reviewed method freeze and first stereo collection) |
| Supersedes | — |
| Superseded by | — |
| Source revision | `99c63395681434cec522ee902845b80fe2d5b055` (independently reviewed workload/method freeze and clean collection revision); historical collector audit `c8502a82`, custody repair `698ef30a` |
| Retention | Until phase exit |
| Related | ADR-0054, ADR-0075, EVD-0021, EVD-0024 |
| Artifacts | [Workload checksums](EVD-0026-workload-SHA256SUMS.txt), [compact results](EVD-0026-stereo-results.txt), [artifact checksums](EVD-0026-stereo-SHA256SUMS.txt); full trace retained externally below |

## Question and falsifier

Can one concrete mixed host admit and render the simultaneously legal compiled,
live, session and release workload while preserving bounded custody and the
callback budget, and do its measured high waters and declared bounds support a
complete reselection of all six producer shares, `release_hold_capacity`,
`max_events_per_quantum` and every live ingress depth, while the source fence
adds no more than 5 ms of live latency at every admitted sample rate?

Before measuring or selecting a profile, freeze the workload's legal demand
independently of that profile: the exact saved-project and synthetic-control
requests, two registered sources' advertised note and pedal entitlements,
activation and loop schedules, and each control's simultaneous live hold set.
The selected profile may refuse an offered workload only if that frozen demand
was already rejected by the preselection admission rules. It cannot redefine
which demand the evidence calls legal.

The stereo-candidate conclusion is **Not supported** if any real, enabled producer
or combination in that frozen legal demand is refused under the selected profile, a protected release
is lost, the fence adds more than 5 ms of live latency, a callback allocates or
blocks, a bound is undercounted, or the measured
callback exceeds the budget fixed before collection. It is **Inconclusive** if
an enabled producer, required common-host path or representative saved project
cannot execute, or if a positive share for a currently unavailable producer
lacks a conservative admitted bound. An unavailable authored-runtime or
renderer-internal producer is recorded as unavailable, never as measured zero;
ADR-0054 clause 2 requires its own high-water measurement before a consumer
enables it. It is **Supported** only after the controls below pass, every
currently enabled required path runs on one host, the selection is repeated
with the selected profile, and all observations remain within the bounds fixed
before collection. A component-only pass cannot support that conclusion.

## Inputs and controls

### Executable-path inventory before collection

The current `mixed_source_bridge` rehearsal composes one armed compiled plan,
two live source rings, the mixed transfer, and raw capture in one callback. Its
compiled program is armed with one initial activation candidate at sample 64.
A common-host activation control carries later compiled transport through the
same host: control prepares and seals each seek or Start, the host carries it to
the mixed audio half's single offer slot, and the retired candidate comes back
for collection. Its seek, compiled Stop, Start after Stop and a seek cancelled by
Stop run beside two held live notes and raw capture of both sources. Foreign,
duplicate, uncollected-slot, stale and Stop-pending offers are refused and
returned whole; joined teardown names an uncollected retired candidate. Whole and
irregular callback partitions agree on audio, and raw capture matches a run
without activation. The control is
`later_compiled_activation_crosses_the_two_source_host_beside_live_notes_and_raw_capture`
in `crates/synth_engine_v2/tests/mixed_source_bridge.rs`; two later candidates
adopt and a third is cancelled, which is not an activation every few quanta, and
it records no timing. A loop control uses the same path: control seals a later
loop entry with its repeating pass, admitted against the arm profile, and the host
carries it through the same slot. Its 28-frame loop wraps twice in the quantum it
enters and six times in all; an ordinary later seek leaves it, while one live note
stays held throughout and another starts and ends inside the loop. Both receipts
name the loop entered or the pass retired, and the audio half reports the pass,
wrap count and wrap releases. Wherever the compiled note is silent after the
entry, a wrap or the exit, the output equals a run that stops the compiled
program at the entry, so each wrap lands on its sample and leaves the live voices
untouched. Whole and irregular partitions agree, raw capture matches a run
without the loop, and joined custody is complete. The control is
`a_later_loop_wraps_twice_in_one_quantum_and_exits_by_seek_beside_held_live_notes`
in the same file; it enters one loop once and records no timing. A separate
common-host control now runs compiled Stop and a sealed
stopped-owner reset through both source rings, the raw recorder and joined old/new
teardown; its whole and irregular callback partitions agree on audio, and raw
capture matches a direct-owner reference. The control is
`stopped_reset_preserves_two_source_raw_capture_and_partitioned_sound_custody`
in `crates/synth_engine_v2/tests/mixed_source_bridge.rs`. A host Panic control
carries one Panic through the bridge's own cell and the protected place beyond
the admitted queue bound, which the bridge arm now requires of both host queues.
At its stamp it ends the held, tied and sustained live notes of both sources and
the open compiled note; the output is silent until a later live note, a later
seek still adopts and replays the compiled note, and the Panic leaves raw capture
identical to a run without it. Later key-ups of the ended notes are consumed after
the Panic, and whole and irregular partitions agree. Further controls show a
Panic waiting for an unfenced peer and behind an earlier source head that waits
for command places, taking its place with every ordinary command place,
uncollected result and live hold charged, being refused at the caller boundary
while its cell or service quantum is occupied, and keeping its custody at joined
teardown. The controls are the six Panic tests in
`crates/synth_engine_v2/tests/mixed_source_bridge.rs`; they record no timing.
Item 2 step 2's precedence now holds in the rehearsal: a Panic stamped before
`B + Q` cancels a sealed reset when its compare-and-swap on the seal's one atomic
phase wins against audio's own decision at `B`. The wrapper then refuses the
successor as `PanicCancelled` and the old owner serves the Panic; a Panic that
loses, or is stamped later, waits for the commit, which sends it to the
successor, or the abandon, which sends it to the old owner. The bridge still
refuses to seal while a Panic is uncollected. Two further controls in the same
file show the cancelled run identical in sound, reports, raw capture and the old
owner's teardown to a run with no reset in four partitions, and a Panic accepted
after an applied reset ending only the successor's notes. Winning decides only
which owner the Panic reaches: a Panic accepted or pumped after audio has served
its quantum executes late, at the first sample the bridge can still stamp, and
its report keeps both its nominal and its execution sample. Successor plans now
activate repeatedly in the rehearsal. The reset wrapper forwards later Starts and
seeks to its active owner through the same binding check as a directly rendered
audio half. It refuses an offer as `ResetPending` while a successor waits for its
boundary, and refuses a reset as `ActivationUncollected` until the old owner's
offer slot is empty, so no candidate or uncollected receipt fades out with the old
owner. The control
`repeated_resets_carry_successor_activations_and_hand_off_live_obligations_in_every_partition`
in the same file commits two resets on the two-source host and joins each retired
owner while the host runs. The first successor starts, seeks beside a held live
note and stops; the second seeks while stopped. Each `Reset` group ends only its
own owner's live notes, and each reset tombstone is consumed once. Offers of an
earlier owner are refused whole, including one that supersedes the sequence the
successor holds in force. Sound, reports, raw capture and both retired teardowns
agree across four partitions and with a run that makes no refused offer, and raw
capture matches a direct-owner run with no reset or activation. Focused wrapper
tests in `crates/synth_engine_v2/src/stream/mixed/reset/transfer_tests.rs` cover
the forwarding and the `ResetPending`, `Faulted` and `ActivationUncollected`
refusals. None of them records timing. These activation, loop, reset and Panic
observations are controls, not the frozen common-host workload or a capacity
measurement. The authored-runtime and renderer-internal event producers are
unavailable in this first mixed shape,
not observed at zero occupancy. `MixedTargetAdmission` refuses authored sources
and requires one compiled and one live producer range; its compiled-writer
check also rejects shapes it cannot restore. The largest saved EVD-0021
project that reached a plan, `instrument-inserts`, now has a private mixed-lowering
control and a checked graph transcription used by this host, described below. The
synthetic two-instrument, two-send fixture has the survey's highest session
request, 64, and remains a separate admission-bound control rather than being
silently called a saved project. These controls alone do not measure
occupancies. The collection below uses explicit conservative bounds for unavailable
producer classes and runs the enabled workload and its transport controls together.

### Saved-project functional controls

The private lowering control loads the manifest-pinned `CORPUS-0005` bytes and
compares its complete `GraphIr` and compiled timeline with
`tests/support/instrument_inserts.rs`. At 44,100, 48,000, 96,000 and 192,000 Hz,
the transcription equals the actual stereo lowering, including the half-frame
release rounding at 44,100 Hz. With no live input, appending the four-note live
range preserves every rendered sample bit through the last compiled release
and a tail. This is equality with V2's ordinary lowering, not a new V1 parity verdict.

`tests/support/saved_project_host.rs`, included by `mixed_source_bridge`, uses
that same transcription through the existing concrete source rings, mixed
command/result queues, callback and raw recorder. Its full-timeline control
runs the original six compiled events at all four rates, with and without live
notes, and verifies after the last edge that Panic finds no compiled note left.
Its transport control fills the four-note live span from two sources, carries
their bends and sustain, seeks every four quanta, enters a 28-frame loop with
multiple wraps per quantum, leaves it, Stops, Starts and applies Panic. A
separate stopped-reset control carries held and sustained notes across joined
old/new custody and Starts the successor. Transport and reset controls run at
all four candidate rates. A separate stereo Panic control ends the complete
four-note live hold set (held and sustained) and both compiled notes at one
sample, then settles the old key-ups and lift. The loop control checks exactly
two wraps within the entry quantum, not merely over several quanta.
Each control compares whole, 64-frame, eight-frame
and irregular callbacks for sample bits, sound reports and raw receipts; the
transport and reset controls also compare raw capture with controls omitting
those operations. Every raw admission and sound disposition settles exactly once.

The host callback now distinguishes interleaved stereo samples from frames;
the raw owner's separate silent mono output still uses the same frame count
and sample rate. The pinned graph itself **refuses mono output**:
`StereoMonitor` has a stereo output and the final `Output` follows the mono
profile, producing `LayoutMismatch`. No graph node or source byte was removed
to obtain a mono pass. On 2026-10-17 the maintainer explicitly approved a first
stereo-only measurement while keeping Phase 9's mono requirement open. The
candidate's envelope check refuses mono before compiling or opening any stream,
including a control graph that the general core accepts in mono. Other mixed
rehearsals keep their existing envelopes. A future mono/downmix path needs its
own law and fresh collection; no stereo result is transferable to it.

Rehearsal publication snapshots now expose the authoritative arbiter's four
executable class high waters and same-quantum external total. Authored-runtime
and Internal remain `None`, not measured zero. Snapshots survive joined
teardown; reset snapshots distinguish active and secondary owners so their
peaks cannot be silently summed or discarded. The allocation test reads these
snapshots during its callback window. Mutation-site observations additionally retain ingress/hold transients, required
and written timed controls, rejected writes, payload occupancy, seeded rows and
adoption-gate entries. Named storage and writable extents distinguish granted
buffers from observed peaks; they are explicit subsets, not total host memory.
A same-callback onset and release leaves both peaks visible after the store
has drained. A deliberately undersized control scratch records its rejected
write. Reset snapshots retain the old owner's release before its discarded
quantum drains it.

`tests/support/capacity_selection.rs` implements the ordered selection formula
with checked arithmetic. It requires explicit independently frozen bounds and
observations rather than inferring demand from a profile; queue demand separates
the ordinary accepted burst from simultaneous protected releases and adds them
once. `mixed_capacity_selection` tests rounding, changed observed peaks,
protected-release and compiled-span floors, unavailable versus zero, missing
bounds and overflow. These are arithmetic controls with invented test inputs,
not a workload freeze, measured capacities or an admitted selected profile.
The selector also rejects a combined sound hold entitlement beyond the mixed
rehearsal's one-release-group limit before any numeric selection. Its scheduled
window retains the default authored headroom when the compiled floor grows.

`sustained_host` adds a functional long-run control through the same graph and
host: exactly 1,024 warm-up and 10,000 exercise callbacks, fixed at 64 frames,
at each of the four rates in stereo. Both source prefixes advance each callback,
with `peer_wait = 64`; four live holds remain outstanding except at the
scripted release boundaries. Repeated seeks and 28-frame loop entries occur
every four quanta. After warm-up, the script exercises Stop/Start, a full-hold
Panic and a stopped reset; their old key-ups settle before successor notes and
later activations continue. The actual reset callback renders both owners,
and both owners' custody joins. Raw capture remains running, and an ordered
per-source admission/receipt comparison checks every message and frontier.
The ordinary test collects operation transcripts, not timing or capacity high waters.
The separate ignored collection entry point enables callback and resource observations
only with an explicitly specified reviewed freeze revision that matches clean
`HEAD`, an explicit collection flag, one CPU in its
process affinity, a recorded governor and matching workload checksums. It runs the
same attribution, fence, actual admission-boundary and protected-pressure
partition controls with each reference/selected configuration, then the complete
unchanged sustained script.

At source revision `c8502a82`, the host test ignored raw command outcomes and
could lose refused completions, packets and receipt errors or panic while holding
the raw owners. Commit `698ef30a` repaired that test collector: it retains exact
outcomes and returned packets, reports receipt errors and returns owning errors
from raw reunion and teardown. Its focused tests cover a foreign completion,
cancel recovery, outstanding credit recovery and joined Stop cancellation. This
repairs the historical custody gap; it does not measure the common-host workload
or establish the selected profile.

The compiled load is `instrument-inserts`, the saved EVD-0021 project with the
largest session request among saved projects that reached a plan, under the
same lowering policy. A mixed lowering must retain its compiled graph while
adding a separately admitted live producer range. The run records the project
identifier, source digest, plan admission report and exact executable graph;
the synthetic 64-request fixture remains a separate admission-bound control.
Two independent live sources use the concrete source rings and mixed bridge.
A separate source-custody control reaches the ring entitlements frozen before
selection, including at most 64 held notes and 16 pedals per source in the
current non-shipping source-ring test support. The common-host sound script
uses its separately declared live span and hold set; a ring stress packet that
exceeds that sound entitlement is not counted as accepted sound demand.
The program activates
every few quanta and uses an admitted interval that wraps at least twice in a
quantum. The fixture records the actual activation, wrap and source
prefix, so no absent operation is inferred from a synthetic charge. Separate
same-host controls schedule Stop, Panic and stopped reset with the frozen
simultaneous live hold set, and check their mass-release and return custody
across callback partitions. The selection must also cover their legal
coincidence with compiled activation and loop wraps, either in a single
measured run or by a conservative upper bound derived from the frozen
producer entitlements. A separate control's measured peak is never silently
added to another run's peak and called observed occupancy.

Run these controls before any selection run:

1. Empty sources and an empty compiled schedule report zero Compiled, Live
   and Release occupancy. Initial adoption still charges Session: the literal
   sixteen scoped restoration groups plus one mass-release operation, then
   zero in an idle quantum. A separate empty control delays adoption and
   verifies zero in all executable classes before it. Neither observation
   calls the raw companion's own Play operation zero.
2. A one-unit request in each executable producer must appear in exactly that
   share. Authored-runtime and renderer-internal producers must be reported as
   unavailable while no executable path exists; their positive shares need
   separate conservative admitted bounds.
3. One admitted onset under saturated ordinary traffic must retain its release
   through source ring, stage, command, result and ingress custody.
4. A one-quantum callback and irregular subdivisions of the same admitted
   prefix must agree on identities, sound outcomes and successful audio.
5. For a frozen request of `N` units at a boundary that promises admission,
   lowering the corresponding share or queue depth to `N - 1` must produce a
   named preparation or caller-boundary refusal. The control does not subtract
   one from a rounded selected capacity or assume every observed occupancy is
   itself an admission check. A conservative `B` is an upper bound, so `B - 1`
   need not be a failing admission limit. Actual promises are checked separately:
   compiled dyad admission, saved Session restoration, live/release profile
   relations, four holds versus three, `2G + 1 = 9` versus eight ingress
   places, `C + 1` versus `C` host queue places and two versus one stage cells.
   Raw input, command and physical ring boundaries retain their precise named
   refusal or owned backpressure result; backpressure is not called refusal.

### Frozen candidate envelope and selection rule

The candidate host admits exactly 44,100, 48,000, 96,000 and 192,000 Hz,
stereo only, fixed 64-frame callbacks, and a two-source sound fence with
`peer_wait = 64` frames. Each rate uses a single-rate profile. This is a
**candidate envelope fixed before collection**; its admitted and measured result
is recorded below. The engine's wider
8,000–192,000 Hz range remains available to other hosts; this host must refuse
rates, layouts or callback sizes outside its envelope before opening a mixed stream.
The irregular subdivisions in control 4 exercise renderer equivalence, but
they are outside this candidate host's timing envelope. The host publishes
each source's frontier and pumps the bridge at least once per callback. With
a healthy peer publishing every 64 frames, the strict `boundary > bound + W`
fence check does not fence it. A silent peer can first be fenced at the next
64-frame boundary after `bound + W`; the worst wait is fewer than `W + 64`
frames. One further 64-frame callback handoff gives a conservative 192-frame
end-to-end added-delay bound, about 4.36 ms at 44,100 Hz, below 5 ms (220.5
frames). The fence control must test both a healthy peer at the maximum
publication cadence and a silent peer across the last observable boundary
before the fence and the first one after it, at every admitted rate. A missed
publication, an extra callback handoff or a measured delay above 5 ms rejects
this envelope. The method does not move the rate floor or latency limit after
collection.

The frozen source-ring **custody** demand is two independent connections, each
entitled to 64 held keys and 16 pressed channel pedals and their protected
releases. Sound demand is a separate script: its simultaneous held keys and
sustain obligations must fit the mixed plan's declared live span and release
holds before any profile is selected. The harness must pin that script and
its preselection admission result. The initial sound script declares four
simultaneous live release holds in total (`G = 4`), split between the two
sources, within the rehearsal's one-release-group limit of eight. The selector
refuses a larger-than-supported combined entitlement before collection. The
selected global hold capacity is storage headroom, not permission to increase
the frozen per-producer sound entitlement. Raw ring entitlement alone does not make
64 simultaneous sound notes or 16 sustained sound holds legal. The bridge now
carries each source's effective pedal-down and lift for sound, with a protected
lift cell at the stage and a reserved lift place at the merger; the
`mixed_source_bridge` sustain tests exercise hold redemption, scope isolation,
stage and command-ring saturation, refused downs and a stopped reset. Those are
functional controls only: a sound pedal in the frozen script counts as
exercised only when the timed common-host run carries it.

The timing run uses release mode on one recorded CPU and toolchain, with a
recorded clock source, one fixed CPU affinity and the unchanged `performance`
governor policy. CPU frequency may vary under that policy. It runs 1,024
warm-up callbacks followed by 10,000 timed callbacks of the sustained script
for each admitted rate in stereo. The sustained script contains every required
timed path: activation, multi-wrap, Stop/Start, full-hold Panic, stopped reset
and simultaneous ordinary-pressure/protected-release traffic. Attribution,
admission-boundary, fence and partition controls run untimed before each
configuration; they establish their own functional claims. All 11,024
callbacks count for allocation, custody and high-water checks. The duration
estimator excludes the first 1,024; it retains the maximum and nearest-rank
99.9th percentile of the following 10,000, plus the timer's empty-callback
floor. The fixed budget is **25% of the 64-frame callback period** at the
actual sample rate; the maximum raw duration, without subtracting the timer
floor, must stay within it. Every required path above must have retained
post-warm-up timed callbacks; a missing path is Inconclusive. A reference-run
deadline overrun is Inconclusive and stops selection; an overrun in the
selected-profile rerun is Not supported. A
deliberately delayed callback must exceed the budget in the positive timing
control. No physical round-trip claim is made without loopback.

`P_ref` is the reference profile, constructed before collection from the
default 64-frame stereo harness profile at each admitted rate. Its six event shares,
release-hold capacity, event cap and physical host queue depths in the inventory
are each scaled
eight-fold with checked arithmetic; the event cap is reset to the sum of the
scaled shares. Other limits stay at their default values, except for the raw source-action
queue byte budget and the scheduled window. The reference raw source-action
byte budget is also eight-fold: 2,097,152 bytes instead of 262,144. The
scheduled window is raised to preserve the default 4,096-event authored
headroom in addition to any raised compiled-share callback floor. The core
profile's legacy `command_queue_capacity` and `event_egress_capacity` fields
remain at their defaults: compile reporting reads them, and this host uses
its separately constructed physical command/result rings. They are not
silently counted as scaled or measured queues.
The harness must prove `P_ref` admits the frozen script; a refusal makes the
method Inconclusive. The script's legal demand is its explicit plan and packet
sequence under profile-independent structural rules, not what `P_ref` happens
to admit. A runtime publication/share fault under `P_ref` also makes the
method Inconclusive: the ledger records only successful charges, so a faulted
high water is a censored lower bound and cannot drive selection. A runtime
fault in the selected-profile rerun is Not supported. Neither run may silently
increase a limit or discard its faulted prefix.
Every bound below comes from that script, its source entitlements or
an explicitly named existing default entitlement; none is recursively derived
from `P_ref` or the selected profile.

A precollection preparation control rejected the first eight-fold raw queue
under its unchanged source-action byte budget: `Session::ByteBudget` requested
294,992 bytes against 262,144 available. No callback-capacity or timing data
was collected. The explicit reference-only byte-budget amendment above is a
method repair independently reviewed and frozen before collection.
The selected host retains the original 262,144-byte budget; any later
preparation refusal remains Inconclusive and cannot silently raise it.

Let `round32(x)` mean the least positive multiple of 32 at or above `x`, with
checked arithmetic. Let `H` be a class's same-run high water and `B` its
independently frozen worst-case legal coincidence. For an unavailable class,
`H` is **undefined**, never zero: use only `B`. The authored-runtime and
renderer-internal classes have no executable mixed-host path today. Their
provisional `B` values preserve the default profile's admitted entitlements of
48 and 16 respectively, selecting 96 and 32. The former has a working
exclusive authored consumer; the latter can pass plan admission but has no
working mixed-host publisher. Neither class is qualified for this host by
those reservations, and a new consumer reopens ADR-0054's measurement gate.

Select in this fixed order, with all inputs frozen before collection:

1. For every live ingress queue, choose its depth as
   `round32(2 × max(observed occupancy, frozen accepted burst, 1))`. For mixed
   command and result queues, the final depth is additionally the least
   multiple of 32 satisfying `depth >= C + 1`: the ordinary admitted
   performance depth plus the protected Panic place. This relation is an
   explicit part of selection, not a post-selection capacity increase. For the
   mixed performance store, also include `2G + 1`, where `G` is the frozen
   simultaneous sound hold entitlement; call the selected depth `C`.
2. Choose `release_hold_capacity` as
   `round32(2 × max(observed holds, G, 1))`.
3. Choose the compiled, session and any other executable share as
   `round32(2 × max(H, B, 1))`. Choose the live share as
   `round32(2 × max(H_live, B_live, C, 1))`. Choose the release share as
   `round32(2 × max(H_release, B_release, C + ceil(S / 8),
   release_hold_capacity, 1))`, where `S` is the compiled span and the
   `C + ceil(S / 8)` term is the existing Panic admission relation. For the
   two unavailable shares, use their `B` values above without an `H`.
4. Set `max_events_per_quantum` to the checked sum of the six shares. Keep
   the default scheduled-window limit or raise it to the selected compiled
   share's callback floor **plus the full default authored headroom of 4,096**,
   whichever is greater. This preserves ADR-0046's independent authored-future
   entitlement even though that publisher is unavailable in the mixed host.
   Keep the other default
   limits. Construct the profile and admit the plan; any remaining inequality
   failure is Inconclusive and requires a reviewed method repair before new
   collection, never a silent post-measurement increase.

The source-ring physical 128-cell depth and its 16 ordinary-cell limit are
fixed by the source contract, not inferred from occupancy. The selected
transfer, stage and raw queue depths must likewise be computed from their
frozen accepted bursts and protected releases before a rerun; any such queue
without an executable selection rule leaves the method Inconclusive. No
producer borrows another share. The workload script, digest, each `B`, every
queue bound and the executable selection function must be committed and
independently reviewed **before** collection.

### Independent demand inventory

The literal workload is `tests/support/sustained_host.rs`, its saved graph is
`tests/support/instrument_inserts.rs`, and `frozen_capacity.rs` binds the
profile and queue inventory. The workload checksum list must be committed
before collection. These bounds are authored before observing that workload:

| Producer | `B` | Derivation |
|---|---:|---|
| Compiled | 6 | At most three starts of the two-note dyad in a 64-frame quantum of the 28-frame loop |
| Authored-runtime | 48 | Preserve the existing default entitlement; unavailable, no `H` |
| Live | 96 | Sixteen ordinary source commands per source, each at most two entries for its two held keys, plus two effective lifts and host Panic, fit below 96 |
| Session | 64 | Saved scope has sixteen restoration groups and two compiled notes: three continuing wraps cost `3 × (16 + 2 + 1) = 57`; the separate synthetic admission control requires 64 |
| Internal | 16 | Preserve the existing default entitlement; unavailable, no `H` |
| Release | 8 | Four live key releases and at most one compiled release group fit below 8 |

An entry quantum costs `1 + 16 + 2 × (16 + 2 + 1) = 55`
Session units. Stop, Panic and stopped reset do not authorize a larger note or
source-command entitlement. Their measured owner peaks remain separate;
the bounds above cover combinations without calling a sum of peaks observed.

| Queue | Frozen accepted demand before rounding | Additional admission relation |
|---|---|---|
| Mixed performance store | 64 ordinary entries plus seven protected entries: four key-ups, two lifts and host Panic | At least `2G + 1`, with `G = 4` |
| Mixed command and result, each owner | 32 ordinary commands plus seven protected places | At least `C + 1` after rounding |
| Per-source sound stage | Sixteen ordinary cells plus two key reservations and one pedal-lift reservation | No producer borrows the other source's cells |
| Per-source raw input | Preserve the existing 136-cell admitted entitlement | Startup-inclusive pressure control and frontier guard; at least seven cells |
| Raw packet and completion rings | Preserve each existing 64-cell entitlement | Both directions selected separately |
| Raw session command store | Preserve the existing four-command entitlement | Observe actual storage after every enqueue |
| Raw source-action store | Preserve the existing 256-action entitlement | Observe actual storage after every enqueue |

The pressure envelope preserves two sounding holds per source and sixteen
ordinary source cells per source. At callback 4,003 each source fills those
sixteen ordinary cells with bends and appends its two protected key-ups.
Control stages the packets before offering the next frontiers; no retry or
extra audio callback is inserted. The loop entered at callback 4,000 wraps
three times in this actual pressure callback. Callback 4,004 replenishes the
four holds. This legal coincidence is timed in the common host, rather than
inferred by adding separate peaks. The same pressure prefix separately agrees
under 64, eight-frame and irregular subdivisions for audio, identities, raw
links and settled outcomes.

Every observed producer, queue and hold peak is compared with the independently
frozen bound before numeric selection and in the selected rerun. A `B + 1`
observation is a named frozen-bound failure even when `P_ref` has spare capacity;
it yields Not supported rather than being silently absorbed by `max(H, B)`.
Unavailable classes must remain undefined. Tests deliberately inject each
bound violation into a configuration with ample capacity and require refusal.

The raw bounds deliberately preserve the previous raw domain alongside the
four-note sound script. Startup Play and two start
frontiers can remain alongside the next six source packets: the lower
startup controls require nine packet/completion places, eight source actions
and one command. Input pressure includes the retained start frontier,
three-cell source prefix and two protected key releases; a further frontier
place gives the seven-cell guard. Existing raw backpressure remains part of
its custody contract; this does not promise to execute both complete 128-cell
source rings in one callback. Ring-only custody tests do not prove raw
admission, and the separate raw key-entitlement control must also pass.

## Method

On each destination quantum, retain the six independently charged high waters,
release holds, ingress occupancy, timed-control entries, payload bytes,
restoration and seed storage, and outstanding source, command and result cells.
Retain the maxima from the **same** run; do not add maxima from separate arms as
though they coincided. All owners in a reset script must use the same profile.
For each class, `H` is the maximum of that class's owner-local high waters from
this run; each owner has its own arbiter and independently checked share. Keep
every owner's identity and observations so a retired peak cannot disappear.
The external total is likewise the largest owner-local same-quantum total,
not a summed host-wide event peak. During a fade both owners consume CPU and
prepared storage: callback timing includes both, and the host's memory bound
must cover the simultaneous two-owner allocation. Per-owner event maxima do
not stand in for that combined CPU or memory bound.
For any legal combination not run together, derive an
explicit conservative bound from the independently frozen entitlements and
select at least the larger of that bound and the measured same-run maximum.
Record callback duration and its declared deadline for
every callback, including activation, multi-wrap and reset transitions. Audit
every source packet and accepted identity at joined teardown. For every
admitted rate, first bound the worst possible fence delay analytically from
`peer_wait`, the maximum admitted frontier-publication cadence and the callback
handoff cadence; refuse a configuration whose sum exceeds 5 ms at that rate.
Then replay a packet with a healthy peer publishing at the declared 64-frame
cadence, and with a silent peer through the adjacent quantum boundaries on
either side of the strict fence threshold. Compare each with a direct,
unfenced mapped-timestamp control. Include the first packet after each
frontier publication. Fail if a healthy peer is fenced or any measured added
delay exceeds 5 ms. The present rehearsal's `peer_wait = 4_096` frames is already too large
for this criterion at 48 kHz; it is a custody fixture, not a candidate
production setting.

The capacity host is a deterministic serial interleaving of source, control
and audio ownership. Between calls the entire callback is returned to host
maintenance; the reset wrapper is constructed and later unwrapped there.
Those `Box` allocations and frees belong to preparation/maintenance, not to an
unobserved part of a concurrent audio callback. During reset, `render` itself
serves both owners and is timed in full. This experiment does not qualify a
continuous device callback or prove thread scheduling behavior.

The test-only callback observer prepares output outside its measured closure.
Inside its allocation scope, monotonic `Instant` readings bracket the complete
`HostCallback::render`, including mixed sound, raw admission/render and raw
completion publication. All 11,024 callbacks require `count_total == 0`,
`count_current == 0` and `bytes_current == 0`; checking only allocations would
miss a free. The positive allocator control detects a transient allocation
and a free of an externally prepared buffer. Timer-scope bookkeeping is outside
the duration; clock overhead remains included and is never subtracted.

A separate outer allocation scope starts before graph construction, compilation,
queue preparation and host arm, and ends after joined sound and raw teardown.
Its requested-heap peak includes all source/control/raw storage, simultaneous
old and successor owners, every activation preparation, compiler temporaries
and the test's retained trace/output buffers. Keeping the latter is deliberately
conservative. The independently fixed whole-experiment upper bound is
**512 MiB of simultaneously live requested heap**. It is not a process RSS,
allocator-metadata, stack or per-plan prepared-memory claim. Nested callback
allocation scopes may overstate the outer peak by their own allocation peaks;
they never make it an exact resident-memory figure. Any such callback allocation
already invalidates the run. Named per-owner snapshots remain separately
reported and are never summed to claim this whole-host upper bound.

Required/written timed controls, payload entries, seeded rows and adoption-gate
entries must fit the independently prepared writable extents. Any rejected
control write or unrepresentable observation invalidates the run. The default
plan memory budgets remain independent admission checks; the larger experimental
host budget does not change them or admit additional demand.

Implement the fixed selection function above and verify its inputs against the
frozen script before collection. First run the controls, then collect the
common-host workload, apply the selection, and rerun controls and workload with
the selected profile. A failed or unavailable control stops numeric selection
and yields the corresponding conclusion above.

## Reproduction

Functional readiness controls:

```bash
cargo test -p pertylizer --features v2-lowering --lib lowering::render::evd_0026::
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --test mixed_source_bridge saved_project
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --test mixed_capacity_selection
```

The opt-in collector is compiled by the ordinary gate and ignored there. Before
running it, commit the independently reviewed method and checksums, prebuild
release mode, and stop competing build/review tasks. Choose the lowest-numbered
CPU in the process's allowed affinity and retain that CPU and the
`performance` governor through collection. Record the toolchain, CPU model and run command
with the raw output. The collector verifies the supplied freeze revision equals `HEAD`, the affinity
is one CPU, and the `performance` governor is unchanged at the end; it does not change machine policy.

```bash
CARGO_BUILD_JOBS=2 cargo test -p synth_engine_v2 --release --no-default-features --features mixed-transfer-rehearsal --test mixed_source_bridge --no-run
# Pin to the selected allowed CPU, replacing <cpu> with its recorded number.
PERTYLIZER_EVD_0026_COLLECT=1 PERTYLIZER_EVD_0026_FREEZE_REVISION=<reviewed-sha> taskset -c <cpu> cargo test -p synth_engine_v2 --release --no-default-features --features mixed-transfer-rehearsal --test mixed_source_bridge saved_project_host::sustained_host::capacity_collection::collect_stereo_reference_then_selected_at_all_four_rates -- --ignored --exact --nocapture --test-threads=1
```

The selected rerun must reproduce the selected capacities under the same fixed
rule. A different configuration is Not supported; the harness never replaces
the running profile with a silently recomputed one.

The run retains every raw callback duration, both owners' publication/resource
snapshots, all queue maxima, whole-experiment heap upper bound, required-path
maxima, timer floor, maximum and nearest-rank p99.9. A failed prefix stays a
failed artifact; it is not silently replaced by a later run. The first collection
below completed successfully; there was no failed prefix or measurement retry.

## Results

The first collection ran at 2026-10-17 12:10:33 UTC on clean revision
`99c63395681434cec522ee902845b80fe2d5b055`. The workload, its checksums,
independent bounds and executable selector were committed and independently
reviewed before collection. The last two diagnostic-only wording repairs received
author review and affected checks after the focused independent read; they did
not change the method, workload, measurements or successful-run behavior.
All eight runs completed: reference, then selected, at each of the four rates. The collector exited zero with no failed prefix,
configuration increase, CPU change or retry.

The host was an Intel Core i7-13700H, Linux 7.2.11-200.fc44.x86_64 with
glibc 2.43, Rust/Cargo 1.99.1 and LLVM 23.1.1. Affinity stayed on logical CPU 0,
the lowest initially allowed CPU; the governor stayed `performance` and the
clock source was `tsc`. The release binary was prebuilt and competing build/review
work had finished. This is one deterministic serial-host collection, without
physical devices or a continuous callback thread.

Each run retained 11,024 callback durations and checked every callback for
allocations and frees. The timing estimator used exactly the last 10,000;
there are 88,192 callback observations in total and 80,000 post-warm-up durations.
The following raw maxima and nearest-rank p99.9 values were independently
recomputed from the retained duration rows. Clock overhead was not subtracted;
the observed empty-callback floor was 9 ns for each run.

| Rate (Hz) | Reference max (µs) | Reference p99.9 (µs) | Selected max (µs) | Selected p99.9 (µs) | Budget (µs) |
|---|---|---|---|---|---|
| 44,100 | 68.039 | 33.832 | 64.324 | 28.930 | 362.811 |
| 48,000 | 74.460 | 33.927 | 60.143 | 30.390 | 333.333 |
| 96,000 | 68.008 | 35.061 | 57.942 | 30.384 | 166.666 |
| 192,000 | 74.828 | 34.713 | 56.934 | 30.556 | 83.333 |

Every required timed path executed after warm-up, including full ordinary
pressure with protected releases and three actual loop wraps, and the reset
callback serving both owners. Their separate maxima are retained in the compact
results. The lint-grade `render_loop_purity` gate covered the render region,
including the new counter mutations and seed walk. Source review of the serial host
callback found no blocking lock or wait; its queue operations are nonblocking.
This supports the no-blocking condition at source level and does not qualify
device-thread scheduling. All callback allocator checks passed; every renderer
reported zero rejected timed-control writes and usage within its prepared writable extents.
All sound/raw owners joined with complete custody, exact release links and
ordered source message/frontier receipts.

Componentwise resource high waters were identical in all eight runs. Mixed
publication and ingress observations are owner-local; shared source stage/raw
queues have host-level peaks. Reset owners' maxima are never summed:

| Resource | Observed high water | Frozen bound | Selected capacity |
|---|---|---|---|
| Compiled | 6 | 6 | 32 |
| Authored-runtime | Undefined | 48 | 96 |
| Live | 64 | 96 | 320 |
| Session | 57 | 64 | 128 |
| Internal | Undefined | 16 | 32 |
| Release | 4 | 8 | 352 |
| Live release holds | 4 | 4 | 32 |
| Performance ingress | 68 | 71 | 160 |
| Command ring | 36 | 39 | 192 |
| Result ring | 36 | 39 | 192 |
| Stage, per source | 18 | 19 | 64 |
| Raw input, per source | 20 | 136 | 288 |
| Raw packet ring | 38 | 64 | 128 |
| Raw completion ring | 38 | 64 | 128 |
| Raw session commands | 1 | 4 | 32 |
| Raw source actions | 40 | 256 | 512 |

The selected event cap is the six-share sum, **960**; the scheduled-event
window is **4,192**, preserving the default authored headroom. The selected
raw action byte budget remains **262,144**. The same fixed rule reproduced
each selected configuration after its full rerun; no running profile was
replaced. Unavailable classes remain undefined observations and provisional
reservations, not qualified publishers.

The outer requested-heap peak ranged from 29,532,253 to 31,898,653 bytes for
reference runs and 27,213,917 to 29,580,317 bytes for selected runs. Every peak
is below the independently frozen 536,870,912-byte bound. These scopes include
preparation, simultaneous reset owners, temporary compilation and retained test
buffers; they are conservative requested-heap observations, not RSS or per-plan
memory claims. Named owner storage and writable extents remain in the full trace.

Attribution, actual admission boundaries, protected-pressure partition controls
and raw 64-key custody passed before each configuration. The fence control
passed for both sources and all rates: a healthy peer was not fenced, equality
at the strict threshold did not fence, and the silent-peer onset executed
127 frames later than its known-prefix control. The predeclared conservative
192-frame bound is below 5 ms even at 44,100 Hz. The physical source-ring
128-cell depth and its 16 ordinary places remain unchanged.

The [compact results](EVD-0026-stereo-results.txt) contain per-run heap peaks,
path maxima, all selected capacities and the raw trace's digest/location.
The complete 2,790,300-byte trace is retained as
`/home/per/pertylizer-evidence/phase-09/EVD-0026-99c63395-stereo-run.txt`,
SHA-256 `9ed1d10a812de896075bc0c32abdf89287356d592076dd79c2d3232e6e0a283d`.
It preserves the exact environment/command, all callback durations, both owners'
resource/publication snapshots and queue peaks. The trace is external under the
evidence artifact policy; preserve it when moving this work to another machine.
Reproduction uses the command above on the clean freeze revision, with CPU 0
and `PERTYLIZER_EVD_0026_FREEZE_REVISION=99c63395681434cec522ee902845b80fe2d5b055`.
A fresh run can reproduce the controls and method, not identical timing samples.

The complete repository gate plus an additional saved-project lowering control
passed before collection (20 commands total); the additional command was
`cargo test -p pertylizer --features v2-lowering --lib lowering::render::evd_0026::`.
Affected host tests (87 passed, collector ignored), Clippy, formatting and the
evidence/documentation checks passed after the review repairs. Claude reviewed
the uncommitted method/instrumentation and focused repairs independently; the
final diagnostic wording used author review as described above. A fresh
independent Claude read found no blocker in the retained results and this
scoped conclusion.

## Limitations

The result covers the pinned `instrument-inserts` graph and frozen G4 sound
script in this serial stereo rehearsal, at 64 frames and the four declared
rates. The source-ring/raw controls cover their separate custody entitlements;
they do not authorize 64 simultaneous sound notes per source. It is one local
collection and does not establish a worst-case execution-time guarantee.

Mono remains refused and open under the user's explicit stereo-first decision.
A downmix law and fresh mono evidence are still required. Authored-runtime and
renderer-internal mixed publishers remain unavailable and must be calibrated
before enabling them. Continuous callbacks, concurrent worker scheduling,
physical input/output timing and round-trip latency require their own evidence.
The wider project population, other block sizes and production adapters are
outside this result. Production mixed refusals remain in force.

## Conclusion

**Supported for the frozen serial stereo candidate.** All four enabled producer
classes, required host paths, independent demand bounds, queue/hold selection,
allocator checks, fence controls and raw callback budgets passed for the
reference and selected profiles at all four rates. Reselection was stable.

This completes this bounded EVD-0026 stereo experiment. It does not close
Phase 9, qualify the unavailable producer classes or open ADR-0054's
production-live gate. Mono and physical/concurrent qualification remain open.
