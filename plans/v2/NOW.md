# Core V2: Current Work

Last updated: 2026-09-09

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md),
and Phase 4's durable record is [REV-P04](reviews/phase-04-exit-review.md)
together with its section in the [master plan](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path).
Phase 5's durable record is [REV-P05](reviews/phase-05-exit-review.md) with its section in the
[master plan](master-plan.md#phase-5-declarative-node-and-parameter-api); its slice table is
archived in [`archive/phase-05/`](archive/phase-05/INDEX.md). Phase 6's durable record is
[REV-P06](reviews/phase-06-exit-review.md) with its section in the
[master plan](master-plan.md#phase-6-polyphony-and-instrument-runtime); its slice table is archived
in [`archive/phase-06/`](archive/phase-06/INDEX.md). Phase 7's durable record is
[REV-P07](reviews/phase-07-exit-review.md) with its section in the
[master plan](master-plan.md#phase-7-yams-mod-grid-and-unified-modulation); its slice table is
archived in [`archive/phase-07/`](archive/phase-07/INDEX.md).

## Phase 4 — closed and merged

[REV-P04](reviews/phase-04-exit-review.md) is **Accepted**: saved projects lower and render
through V2 from their own pinned bytes, at their own pitches and velocities. Its gate and the
roadmap outcome were amended on 2026-09-02 under `PROCESS.md`'s phase-exit rule, so the phase
delivers the V2 side of the headless comparison path rather than the join between the two
paths.
[ADR-0057](decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) owns that decision;
the two obligations it carries are active state and stay below. The branch was squash-merged
to `main` on 2026-09-04 after thirteen independent reads, the last two over the whole squash.

## Phase 4 residual obligations

Phase 4 is complete. Its exit review accepted these two residuals; `P04-R002` and `P04-R003` are
discharged rather than carried.

| ID | Residual | Pull-forward rule |
|---|---|---|
| P04-R001 | V1 applies one saved velocity twice, under two independent sensitivities; V2 applied it once as one scale on the envelope | **Closed** by `P06-S004` under [ADR-0059](decisions/ADR-0059-velocity-composition.md): a voice has two velocity destinations, each with its own sensitivity and V1's formula, the lowerer carries `vel_sens` and `velocity_amp_sensitivity` to them, and a placed note no longer names velocity as unrepresented. `LOWER-INV-003`'s general rule stands: nothing may issue a parity verdict over a **lowered** outcome that is not `Faithful`, and a placed note is still `UnsupportedScope` through Phase 8's marks |
| P04-R004 | [ADR-0028](decisions/ADR-0028-long-running-job-contract.md) is `Deferred`: a *revisioned* job contract needs Phase 10A's canonical revision and Phase 10B's job capture | All three standing constraints hold until acceptance in Phase 10B. Constraint 3 refuses streaming, progress, cancellation, multi-project A/B and a shared render request/result as **task selections**, so that work does not proceed under another name |

## Phase 5 — closed

[REV-P05](reviews/phase-05-exit-review.md) is **Accepted** on `1b1252f4`: ten node kinds are
declared once, discovery and validation derive from the same declaration, every parameter
composes in one slot under a declared law and ramps as a declared segment, a declared tap is
the plan's only observation point, and the host's bounded lossy subscription over it changes
no sample. Its gate was corrected on 2026-09-05 — the legacy adapter withdrawn, not deferred —
and the phase exits with two residuals, below. Nine slices, each merged on one independent read
at the user's standing decision; the exit review's first draft was rejected by an independent
read for an unevidenced observation bullet, and the ninth slice is what evidenced it.

## Phase 5 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P05-R002 | The exit gate's observation bullet claimed that observation changes no semantic project digest, and that digest is defined only in Phase 10D; the clause is carried, not claimed. ADR-0027 clause 2 already keeps every observation field out of the serialized project. | Phase 10D, when it defines the semantic project digest: its digest test holds that opening, closing or saturating an observation changes no digest, before the digest is used for round-trip or migration checks. Fails closed: no digest exists to misreport |
| P05-R001 | The smoothing policy of a lowered level, owned by the first lowering that maps V1's *amplifier* level onto a V2 parameter, or first writes a V2 amplitude dynamically. **Half decided by `P08-S001`**: the mix channel's fader is the first dynamic write to a V2 amplitude and declares `Smoothing::None`, because V1 applies the instrument fader as one gain per block in `mix_channel_busses` and never ramps it (`SOUND-INV-031`). The amplifier-level half still binds the first slice that maps V1's amplifier level. | Every declared `Smoothing` is `None`, so a write is a step. That is V1 parity for the one quantum-rate control V2 has: the lowerer maps V1's *oscillator* level onto the V2 amplitude as a static base (`lowering/graph.rs`), and V1's oscillator applies that level unsmoothed (`synth_modules/src/oscillator.rs`, `effective_level`). The control V1 does de-zipper — a linear ramp per block landing exactly on the target — is its *amplifier* level (`synth_modules/src/amplifier.rs`), which the lowerer refuses unless unity because V2's amplifier has no level of its own. The decision the user took on 2026-09-05 is therefore: no declared policy changes now; the parameter that first receives V1's amplifier level, or the first dynamic write to a V2 amplitude, decides its `Smoothing` against V1's per-block ramp with an A/B to measure. Fails closed: nothing is silently ramped. The mechanism is built and mutation-verified (`P05-S007b`), so the decision is a one-line declaration change. An independent read corrected this row's first form, which named the oscillator's own lowering as the trigger — a point already passed |

## Phase 6 — closed

[REV-P06](reviews/phase-06-exit-review.md) is **Accepted** on `e7e2d0a9`: a voice scope is one
prepared shape and `N` instances of state, stealing under three decided policies ends the taken
voice and starts the new note at a precise displaced sample on the compiled path and at the
live boundary, a bend follows its occurrence through a steal, velocity composes as V1 composes
it, one zone of a prepared sample map plays through a declared trigger, one prepared tuning is
held to every path, and a polyphonic render under pressure is the same bits run to run, under
every host partition, offline and live. Its gate was corrected on 2026-09-06 — the seed clause
and the live-note-across-recompilation clause carried, not claimed — and the phase exits with
two residuals, below. Eight slices, each merged on one independent read at the user's standing
decision. The slice table is archived at `archive/phase-06/`.

## Phase 6 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P06-R001 | The exit gate's first bullet claimed determinism for a fixed **project seed**, and no seed existed in V2 at Phase 6's exit; the event-stream half was held by bits on every path and the seed half carried, not claimed | **Closed** by `P07-S005` and `P07-S008` under [ADR-0008](decisions/ADR-0008-yams-state-identity-and-seeds.md): the seed enters as the project seed, the node, the instance and the stable script-state identity, never plan order or revision, and `tests/determinism.rs` holds Phase 6's pressure fixture — seeded per voice in the Control and Audio domains — to the same bits for one seed under fresh compilation, every partition, offline and simulated live, with a changed seed changing the samples (`fixed_project_seed_survives_stealing_recompile_partitions_offline_and_live`) |
| P06-R002 | The exit gate's seventh bullet claimed that note identity routes expression across a **plan recompilation**; the runtime resolves a stale identity as foreign to its new table (ADR-0047 clause 8, `an_identity_from_another_table_is_not_an_orphan`) and routes nothing, and a live note surviving a recompilation has no consumer before Phase 9's live host | Binds Phase 9, with ADR-0050 clause 8's redemption of a live note across an activation, to route or refuse such a note by a stated rule rather than by the table's refusal alone |

## Phase 7 — closed

[REV-P07](reviews/phase-07-exit-review.md) is **Accepted** on the squash that adds it: a
modulation edge feeds any declared parameter's modulation layer from a control-domain output
and the LFO is its first source, the filter's and the envelope's fields are addressable
controls, a placed pattern's instrument automation lowers onto the override layer as V1 emits
it with simultaneous absolute writers refused by name, V1's Mod Matrix and Mod Grid lower onto
that edge at V1's scale and its macros onto declared controllers, three YAMS domains run on one
off-thread compiler — Control as a scheduled node with stable local keys and identity-seeded
state, Audio per sample and allocation-free at the profile's maxima, Note as one finite
exclusively owned source measured (EVD-0020) before it was enabled — and the exit's evidence
holds every contributor to one slot under one law and a render to its event stream and project
seed alone, which closes `P06-R001`. Nine slices, each merged on one independent read; the
squashes of the last four were read whole. The slice table is archived at `archive/phase-07/`.
ADR-0060 owns the bounded Note source and its exclusive stream ownership.

## Phase 7 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P07-R001 | Saved Note Grid graphs and note-processor racks remain refused. The finite standalone authored stream's previous-quantum inputs and explicit cuts are not a saved-rack fidelity claim. | Phase 10A's canonical note-processing work item owns the model and lowering. Before any saved Note Grid/rack consumer, define its ordering, timing, identity, source bounds and fidelity dispositions, then test its saved inputs through the lowerer. Phase 10A cannot exit with this refusal unassigned. |

Owed onward, not residuals: ADR-0008's hot-edit clauses to Phase 9's plan-swap decisions
(ADR-0009, ADR-0010); ADR-0054 clause 3's simultaneous six-share reselection to Phase 9; the
saved sources, targets and lane classes `P07-S002b` and `P07-S003` refuse by name, and a saved
YAMS script, to the slice that first lowers each; and ADR-0012's product conflict policy to
Phase 10. `P05-R001` was not triggered: the phase's combined target is the filter's cutoff, its
seeded modulation targets the pitch, and an Audio program multiplying a signal is not a write to
an amplitude parameter.

## Active streams

### Phase 8 — active since 2026-09-09

Activated by selection, as the Phase 7 exit said it would be. Its entry prerequisite is met:
Phase 7 is `Complete` under [REV-P07](reviews/phase-07-exit-review.md), which is the one phase
`ROADMAP.md` names this one as depending on. No record is drafted at entry. Under `PROCESS.md`'s
decision-timing rule the phase's open decisions bind the slices that need them, not the entry:

- **ADR-0033**, the graph feedback rule, is a register row with no record. V2 refuses a cycle at
  compilation (`SOUND-INV-007`) and the lowerer refuses a V1 feedback cable naming this phase. An
  **acyclic** sidechain is a dependency edge scheduled in current-quantum order and needs no
  record; the record is drafted before the first slice that admits a cyclic path through an
  explicit delay (`P08-S006` below), and until then such a path stays refused by name.
- **ADR-0034**, track, source and channel ownership, stays `Proposed` for Phase 0B/10A: it owns
  the *persisted* relationship between a track, an instrument, a patch and a strip. This phase
  defines the **compiled** channel, bus and send as plan concepts in the render contract, keyed by
  `ChannelId` and `BusId` and never by `InstrumentId`, and states what `max_sends_per_channel`
  counts (per channel, as `LIMIT-0024` counts it). Nothing is persisted and a reversal is a
  rebuild, so that is a current-spec rule, not a decision. If a slice needs the meaning of the
  profile field to change, ADR-0034's Sound Core half is drafted before it.
- **ADR-0022** keeps the physical mapping and hardware latency ownership at Phase 9's exit. The
  latency this phase makes visible is the **plan's**: a node's declared latency, a path's sum and
  the compensation the compiler inserts, reported in frames of plan time and independent of the
  host's block size.
- **ADR-0012** stays `Proposed` for Phase 10. A track lane over the fader, pan or mute takes the
  strict form `P07-S002b` applied: two absolute writers on one target at one sample are refused
  at compilation by name.
- **ADR-0027** is accepted and binds: a meter is a declared tap, not an engine read. The open
  question of whether `max_mix_channels` and `max_observation_taps` should be coupled is answered
  by the slice that declares the channel meters (`P08-S007`).

What V2 has at the phase's start: one `Output` node per plan at global scope, whose `PlanOp` copies
**one** source into the stream's layout, widening mono to stereo by duplication and refusing a
stereo-to-mono edge (`SOUND-INV-014`); `Mono` and `Stereo` are the only layouts (`SOUND-INV-009`);
two cables into one input are illegal fan-in, so the **only** summing that exists is the
compiler-inserted voice sum — one copy and `N − 1` accumulates, where the steal fade is applied
(`SOUND-INV-025`, ADR-0058); no node pans, trims or limits — V2's amplifier has no level of its
own and does not pan; no node declares a latency or a tail, and the plan's `added_latency` is
the constant `Q`; the profile carries `max_mix_channels` (256), `max_buses` (64) and
`max_sends_per_channel` (16), reported against a plan declaration a builder states and no
compiled object yet fills, as Phase 7's capacities were until its slices declared usage; and
no effect kind exists — the lowerer maps the oscillator, envelope, filter, amplifier, LFO, Mod
Matrix and terminating node, and every V1 effect module is refused as an unmapped kind.
The one-slot composition Phase 7 built is what a fader, a pan or a send level becomes: a
declared control in the same slot type, reached by the same edge, lane, controller and script
producers, so this phase declares parameters and builds no new writer.

What V1 does per block, read at its own boundary rather than copied: a track's volume, pan and
audibility are applied **per voice** before the instrument's shared effect chain, so an
instrument shared by two tracks carries two gains inside one signal; the instrument's fader
and pan — V1's constant-power law, with the Mod Grid's offsets added — are applied once per
instrument after that chain and summed linearly into the master mix, with sends tapped pre- or
post-fader into return buses, a channel's resolved sends dropped past the sixteenth; each
return runs its effect chain, **soft-clips** its own output and feeds bus-to-bus sends in a
Kahn order **recomputed every block**, which is exactly what this phase's fifth exit bullet
forbids; the master effect chain runs on the mix, the master volume is applied and the output
is **hard-clamped** to full scale; a sidechain reads the source's **previous callback**, so its
latency is the host's block size, which the second exit bullet forbids; and the terminating
`StereoOutput` module pans, soft-limits at −0.3 dB and meters. EVD-0013 measured three centre
pans in that chain at `0x3eb504f2`, which is the figure a lowered channel has to reconcile
against, not reproduce blindly. The lowerer today names this phase for: an instrument volume or
pan other than neutral, a muted instrument, a sidechain source, a master volume other than
unity, a per-placement gain, a track volume or pan, a track lane over the fader, pan or mute,
an instrument volume or pan lane, a master volume lane, a send into a return bus, a second
output module, two cables into one input, a feedback cable, the amplifier's pan stage, the
terminating node's pan, limiter and metering stages, and a Mod Matrix or Mod Grid route to a
track or master target or an audio tap. An instrument soloed **elsewhere** is the lowering
spec's open question and needs the first whole-project lowering. Instrument **oversampling** is
refused under a "Phase 5" label that phase never claimed — the master plan places oversampling
islands here — so `P08-S003` decides whether it lowers in this phase and corrects the label
either way, as Phase 7 did for the note-processing label. Inventoried in V1: the mixing tools
(`CAP-0004`), the mixer view (`CAP-0033`), the return-bus and effect-chain identities
(`IDN-0005`, `IDN-0012`, `IDN-0020`, `IDN-0021`) and the send cap (`LIMIT-0024`).

Not built by design, under the phase's own YAGNI: layouts beyond `Mono` and `Stereo` — the
contract's general rule is stated where a layout is declared and only stereo is compiled, since
V1 is stereo; a stereo-to-mono summing law, unless a slice's corpus case needs V1's offline
`(L + R) / 2`; and a sample-rate island — `P08-S003` decided against one in this phase's slices,
because no corpus case oversamples and V1's island is the voice sum alone, so the mark stays
reported under a Phase 8 label and the phase's exit names it a residual if no case arrives.

| Task | State | Current boundary |
|---|---|---|
| P08-S001 — the mix channel and explicit summing | **Merged** 2026-09-09 (`7e19e521`); `SOUND-INV-031` built; one independent read (codex, `gpt-6-astra`) found two P2s — instance-slot contiguity under an inserted widening, and an over-charged inserted-record count refusing a plan whose own report fit — both repaired, focused reread clean; 15 mutations caught; EVD-0013 and `quantum_cost` digests reproduce; merged on the one read at the user's standing decision | A **channel** in the IR and the plan (`IrNodeKind::Channel`, `ExecutionScope::Channel`), keyed by a `ChannelId` the compiler mints in ascending node identity and carries with its plan, with `CompiledPlan::channels()` recording the three slots: a fader (linear amplitude, decibel law, quantum-rate, `Smoothing::None`), a pan (bipolar law, quantum-rate) and a mute (thresholded boolean, sample-positioned), placed between the voice sum and the output; and a **`Mix`** kind whose one input port **declares fan-in** (`PortSpec::summing`), so every cable into it is a scheduled sum — the node's own step seeds its output with the first cable in ascending source identity and the voice sum's accumulate adds each further one, linear in float, unclamped — while fan-in into any other port is still refused. Both kinds carry stereo on every port; a mono cable into either is widened by the same scheduled conversion the output receives, on the validator's record of the edge. `Output` takes the channel. The lowerer maps the instrument's volume, pan and mute onto the channel (`ChannelStrip`, `identity::CHANNEL`), the fader held to V1's own `Gain::MIXER_RANGE` and a value outside it refused by name and value; the muted-instrument refusal and the volume and pan marks are gone. The amplifier's own pan stage and the terminating node's pan, limiter and metering keep their `Phase 8` marks: EVD-0013's three-pan figure is one instrument-fader pan, carried here, times two stages this slice does not lower. Admission counts compiled channels against `max_mix_channels` from the plan's nodes; `PlanDeclarations::mix_channels` is removed. **Delivered against the check:** a constant through a channel renders `input × (V1 coefficient × fader)` per side bit for bit against `synth_core::Gain::from_pan`; unity and centre is `cos(π/4)` per side and not a pass-through; a plan with no channel renders as before (`layout_baseline`, EVD-0013 and `quantum_cost` digests — see the commit); two sources through one sum are the sum of each alone exactly, a sum above full scale is preserved, and a three-cable sum whose float result depends on order is identity-ordered both ways; a mute mid-quantum silences from its sample and its release restores from its own; an override write, a held-LFO edge and a script edge compose in the fader's slot under the decibel law, with the write-alone render as the control; two channels under a profile of one are refused by name; same bits under whole, 256, 64 and irregular partitions; the purity scan covers both kernels; the lowerer's half is held by four tests over the corpus patch. `P05-R001` decided for the channel fader: `None`, because V1 applies the instrument fader as one gain per block in `mix_channel_busses` and never ramps it; the amplifier module's ramped level stays refused unless unity, so that half still binds the slice that maps it |
| P08-S002 — a whole project through one plan | **Merged** 2026-09-09 (`f7f7946d`); `SOUND-INV-032` built, `SOUND-INV-021`'s binding generalised to islands; one independent read (codex, `gpt-6-astra`) found two P2s and its focused reread a third — an ordering that aborted a note whose edges round to one sample, lanes dropped as inert where the one-instrument plan lacked the stage, and a concurrency counted in ticks where two ticks round to one sample — all repaired, the last reread clean; 25 mutations caught; EVD-0013 and `quantum_cost` digests reproduce; merged on the one read at the user's standing decision | Every instrument the project holds lowers into one plan: its voice patch in the one voice scope as its own **island** — `SOUND-INV-021` now binds a note to the nodes its played node is cabled to, which is what lets two instruments share the scope; the compiler had refused a second playable node there — then V1's stages in V1's order: the velocity scaler and the playing track's **balance** per voice (`IrNodeKind::Balance`, V1's `sqrt(1 ∓ pan) × level` law, unity at centre, in the voice scope where V1 applies it, possible because the voice sum's seed now copies a stereo instance output verbatim rather than as twice as many mono frames — a latent V2 bug a probe of a two-voice stereo script showed), the instrument's channel and V1's channel-stage **soft clipper** (`IrNodeKind::SoftClip`, V1's 0.8 knee — a stage `SOUND-INV-031` had omitted) on the voice sum; every channel into one master `Mix`, the master volume as a **trim** (`IrNodeKind::Trim`, held to V1's `0..=2`), V1's output **hard clamp** (`IrNodeKind::HardClamp`) and one output. The two saturation stages are the **output policy**: `OutputPolicy::Parity` places both, `Headroom` neither. The plan declares one compiled producer of the project's peak simultaneous notes, a tie counted as an overlap, every island instantiated per note. Solo across instruments mutes the unsoloed channels, closing the lowering spec's solo-elsewhere question. The track's fader, pan and audibility lower where one track plays the instrument or several at equal controls; differing controls, and a track lane on a shared instrument, are refused by name until ADR-0034. Track lanes over fader, pan and mute, the instrument's volume and pan lanes and the master volume lane lower to override writes, restored at the song's end only where V1's stop clears them. The per-placement gain is **inert in V1** — read by nothing that renders, measured — so it lowers to nothing. Addresses are per instrument (`InstrumentSlot`), which also ends the `P08-S001` collision between the channel and the velocity macro at `0xFFFF_0001`. **Delivered against the check:** two instruments on one patch at two faders, two notes at once, render the sum of each alone bit for bit (first exit bullet); a soloed instrument renders as itself alone; parity holds a sum at exactly full scale where headroom peaks past it; the corpus's `shared-instrument-tracks` is refused by name; the chain, scopes and one output are pinned; half the master is half every sample; every V2 stage is held to V1's law bit for bit at two channels and under four partitions. **Not built:** a per-instrument voice group (every island is instantiated once per simultaneous note across the project; bounded, idle instances on a sparse project); the engine's default `session_event_share` (24) admits one lowered instrument's catch-up addresses and not two, so a whole project needs the roomier partition ADR-0054 reselects — the slice's tests scale the events group eight-fold; three or more instruments saved out of identity order are marked, since V1 sums the master in list order and V2 in identity order |
| P08-S003 — inserts: the first native effects with latency and tail | **Built** 2026-09-09 on `feat/v2-phase8-s003`; `SOUND-INV-033` built; codex (`gpt-6-astra`) consulted on the design frame before building and found seven points — the naive −60 dB tail rule falsified at a short dark loop, the feedback and time domains not held at runtime, the history counted twice, `NONE` untrue for the filter and envelope, the dyad falsifier not separating P1 from P2, a steal's reset reaching the shared insert, and order entries compared by spelling — every one shaped the build below; merge record follows | The declaration gains **latency**, **tail** and **history** (`NodeTiming`, per kind from its authored values and the rate): every kind declares zero latency until `P08-S005` compensates one; the tail is `Some(0)` for a per-frame kind or a source, the kind's rule for the two inserts, and honestly `None` for the filter, the envelope, the sampler and the scripts, which have not stated one; the history is what the renderer allocates per scheduled step at the kind's output width — one slab with a per-record index beside the ramps, handed to the kernel as `NodeIo::history`, charged to `mutable_state_bytes` from the same figure. The plan carries every node's timing and its longest stated tail, and the report carries the same. Two kinds with V1's law, held bit for bit against V1's own modules run over the same input: `Distortion` (V1's soft-clip mode; drive, tone, mix; tail = its tone filter's 60 dB time) and `Delay` (V1's mono mode; two times and a feedback in V1's own domains `[0.001, 2]` s and `[0, 0.95]` as **parameter units** — `DelayTime`, `DelayFeedback` — so the slot holds every write to them; mix and tone; history = one 2 s line at stereo width). The delay's tail is V2's own rule, **checked not proved**: the slowest mode of the loop's positive majorant by bisection plus one traversal, held at five named points including the one that falsified the naive count; a mutation run showed the traversal margin needed and a filter-decay term not. The lowerer reads `patch.settings.effect_chain_order` (`Patch` and `PatchSettings` now destructured exhaustively; `master_volume` and `octave_offset` measured inert offline) and places the chain as instrument-scope nodes between the balance and the channel; an omitted, unknown, repeated (by parsed identity) or non-effect entry is refused naming the problem (`CORPUS-0005-C1`), a distortion mode other than soft clip, a delay mode other than mono and a tempo-synced time are refused naming the parameter, and every other effect type stays an unsupported type. **Two notes of different keys held at once on one instrument now lower** — the overlap refusal was Phase 4's, before per-note instantiation; two of one key stay refused by name, and more notes held at once than V1's voice count are refused by name where V1 steals — which is what lets `CORPUS-0005`'s dyad lower. The one independent read (codex, `gpt-6-astra`) found two P2s in that lifted rule — the voice limit missing and the same-key scan quadratic — both repaired in one sweep; merged on the one read at the user's standing decision. **Oversampling decided:** not a rate island in this phase's slices (no corpus case; V1's island is the voice sum alone); the label now says Phase 8. **Delivered against the check:** the corpus insert case lowers and renders, and its dyad through the distortion alone, the delay alone and no insert measures P1, P2 and the null control as the corpus isolates them; the reversed order is another signal (P4); the delay rings past the last release (P3). **Not built, recorded:** the corpus case is refused by name under the engine's **default** event partition — 29 catch-up addresses against a session share of 24 — so it renders only under the roomier partition `P08-S002`'s tests select, and the default-profile survey still counts three eligible projects until ADR-0054's reselection; the tails of the filter, envelope, sampler and scripts are owed to their first reader; a compiled release that names its occurrence would lift the same-key overlap refusal |
| P08-S004 — sends, return buses and the bus graph | Not started | A **bus** keyed by `BusId` with its own inserts, fader, pan and mute; a **send** from a channel or a bus into a bus, pre- or post-fader, with its level a declared control; return-to-return routing as the same bus graph as channels and groups, compiled **once** into dependency order so no block sorts anything, which is the fifth exit bullet; a cyclic send refused at compilation naming the closing edge. Admission counts buses against `max_buses` and a channel's sends against `max_sends_per_channel`, refusing where V1 dropped (`LIMIT-0024`). V1's soft-clip on a return's output is an explicit node with V1's law or a named refusal, decided here. The send mark and the Mod Matrix and Mod Grid marks for a track, master or bus target come off as each becomes addressable |
| P08-S005 — path latency and compensation | Not started | The compiler sums declared latency along every path to the output, inserts the compensation the declared policy asks for as scheduled delay operations, and reports node, path and compensated latency in the plan's diagnostics; the plan's `added_latency` becomes `Q` plus the compensated path. Two paths of unequal latency into one `Mix` arrive aligned; a policy that declines compensation reports the skew instead. Fourth exit bullet. Sample-identical under partitions; digests unchanged for a plan whose nodes all declare zero |
| P08-S006 — sidechains and feedback | Not started | An acyclic sidechain is an audio edge into an effect's declared sidechain port, scheduled in current-quantum dependency order, with its latency reported in plan frames and independent of the host's block — V1's previous-callback read is an intentional difference with its own corpus category, as `CORPUS-0003-C1` is for modulation timing. A cyclic path through an explicit delay needs ADR-0033 first; until it is accepted the cycle stays refused by name. Second exit bullet |
| P08-S007 — meters as taps, and the terminating node's stages | Not started | Channel, bus and master meters as declared taps under ADR-0027, read through the host subscription Phase 5 built, with the `max_mix_channels`-versus-`max_observation_taps` question answered in the profile spec; V1's terminating limiter as an explicit node with V1's law, selectable as the output policy; the master effect chain as the master bus's inserts. The terminating node's remaining marks come off |
| P08-S008 — parity, refusal behavior and the RT guard | Not started | The exit's evidence: every corpus project whose routing this phase carries renders through its own channels, buses and sends and is compared against V1 under the comparison categories the phase declared, an intentional difference named rather than hidden (`LOWER-INV-003` still refuses a verdict over any remaining mark); a bounded refusal for every route the phase does not carry; and the channel and bus processing held allocation-free under the render-loop purity scan and the allocation test, which is the sixth exit bullet |

Inherited before it builds: `P05-R001` (a lowered level's smoothing policy, `P08-S001`). Owed
to this phase by Phase 7 and taken by name above: the track, master and bus targets and the
audio tap `P07-S003` refuses, and the track and master lanes `P07-S002b` refuses. `P07-R001`
is Phase 10A's, `P06-R002` is Phase 9's, `P05-R002` is Phase 10D's and `P04-R004` is Phase
10B's; none binds this phase.

### Phase 0B — active in parallel

Phase 0B remains `Active, parallel`; Phase 10 still waits for its exit.

| Task | State | Current boundary |
|---|---|---|
| P00B-T001 | Complete | Closed 2026-08-29; 64 state entries are `Classified` and coverage-gated |
| P00B-T002 | Paused | Resume by assigning reachability and migration dispositions in the capability inventory |
| P00B-T003 | Active | Fill `Proposed V2 newtype/rule` for all 31 identity entries; this is the selected Phase 0B slice |
| P00B-T004–T007, P00B-T009 | Not started | Follow the frozen Phase 0B decomposition |
| P00B-T008 | Not started | Re-scope the former all-ADR task under `PROCESS.md` decision timing |

## Phase 3 residual obligations

Phase 3 is complete. Its exit review accepted these bounded residuals:

| ID | Residual | Pull-forward rule |
|---|---|---|
| P03-R001 | Sample-exact runtime loop wrap and per-pass note identity remain undecided in [ADR-0052](decisions/ADR-0052-loop-wrap-note-identity.md) | [ADR-0055](decisions/ADR-0055-refuse-unimplemented-loop-playback.md) refuses loop playback meanwhile. Resolve before any V2 loop consumer; Phase 9 cannot exit without it |
| P03-R002 | Current producer shares, event cap, release holds and live-ingress depth remain provisional | [ADR-0054](decisions/ADR-0054-staged-producer-capacity-calibration.md) measures each first real authored/internal producer and requires complete reselection before production live ingress |
| P03-R003 | Note events carry identity but not typed pitch and velocity | **Closed.** A note-on carries a validated key and velocity, resolves the key through the plan's prepared tuning, expands to the control writes its scope declares, and a saved note's own magnitudes reach it. Phase 6 still owns the full composition law, which the work list is explicit this does not decide |
| P03-R004 | Numeric note-index and generation widths are safe by checked bounds and fail-closed exhaustion, but not endurance-qualified against a real live workload | Validate the widths before a production live adapter; generation exhaustion retires and reports instead of aliasing |

## Later-owned work

- Phase 6 owned `P04-R001`'s composition law and `SOUND-INV-021`'s **bend** clause; both are
  built (`P06-S003`, `P06-S004`).
- Phase 9 owns ADR-0022 acceptance against retained platform/adapter evidence,
  P03-R001 before loop playback or phase exit, P03-R004 before production live
  ingress, and ADR-0050 clause 8's release-hold redemption and activation-time
  minter ownership before activation can coexist with live ingress.
- ADR-0051's shared-gate ownership law is required before two producers can
  drive one scalar gate through activation/catch-up behavior.
- Phase 10A owns the canonical project revision `P04-R004` waits for and the note-processing
  work item `P07-R001` binds; Phase 10B owns ADR-0028's acceptance and the revision-pinned
  job service.
- Phase 10E owns ADR-0039 and `LIMIT-0017`.
- Phase 0B still gates Phase 10.

## Current blockers

Nothing blocks the selected slice. Residuals bind later work by name: `P05-R001` — a lowered
level's smoothing policy — binds `P08-S001`, the first slice that writes a V2 amplitude
dynamically; `P07-R001` binds Phase 10A's note-processing work item; `P06-R002` binds Phase 9's
live host; `P05-R002` binds Phase 10D's digest; and Phase 3's residuals block only their named
consumers. `P04-R004` binds the first shared render surface, which is Phase 10B's. `P07-S001`'s
rule that a modulation source declares no input port and no sample-positioned control still
keeps the envelope from being a source; the slice that lowers V1's envelope matrix source owns
the collection split that lifts it, and the slice that gives the LFO a lowered target owes the
refusal that names the slot a modulation cycle would close. ADR-0033 is drafted before
`P08-S006`, not before the selected slice, because `P08-S001` sums and pans and admits no cycle.

Two streams are active: Phase 8, with `P08-S003` built and awaiting its merge, and Phase 0B,
with `P00B-T003` as its selected slice.

Next action: **review and merge `P08-S003`**, then build `P08-S004` — sends, return buses
and the bus graph — on a branch off `main`; its boundary is in the table above. Three facts
`P08-S003` leaves for it: the engine's default `session_event_share` (24) now falls short of
**one** instrument with two inserts (29 catch-up addresses), so ADR-0054's reselection is
the next slice's first obligation rather than a whole-project nicety; every island is still
instantiated once per simultaneous note across the project, which a per-instrument voice
group would tighten; and a bus's inserts take the same declaration and the same
instrument-scope shape, with the bus in `ExecutionScope::Bus`. Reload remains owned by
Phase 9.
