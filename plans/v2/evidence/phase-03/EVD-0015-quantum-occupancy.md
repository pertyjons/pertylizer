# EVD-0015: Quantum Occupancy in 23 Real Projects, Over Six Sampled Rates

| Field | Value |
|---|---|
| ID | EVD-0015 |
| Status | Complete |
| Phase | 03 |
| Created | 2026-08-24 |
| Last reviewed | 2026-08-25 |
| Supersedes | — |
| Superseded by | — |
| Source revision | Repository inputs at `ee574fe3`; measurement artifact SHA-256 `5423cbcf5368d9d48c3fdc3b9248d4ac0ae03b772cf257262b9eb6697b31ca61` |
| Retention | Permanent |
| Related | ADR-0043; ADR-0032 clause 27; [ADR-0046](../../decisions/ADR-0046-destination-quantum-admission.md); [ADR-0044](../../decisions/ADR-0044-deferral-causal-order.md); [ADR-0045](../../decisions/ADR-0045-cross-control-causal-order.md); `HOST-INV-011`; `HOST-INV-021`; `max_events_per_quantum`; `max_note_expansion_per_tick` |
| Artifacts | [`evd_0015_quantum_occupancy.py`](evd_0015_quantum_occupancy.py), SHA-256 as recorded above |

## Question and falsifier

**Question.** Can `max_events_per_quantum` and an emission policy be chosen so that an over-full
quantum is **unreachable** rather than merely unlikely? If yes, ADR-0043's rejected Option A costs
nothing — deferral would guard against something that cannot happen, and ADR-0044 and ADR-0045
would dissolve with it.

**The falsifier, written before any number was collected.** "Unreachable" is falsified if any of:

- **F-a.** A real project's measured peak already exceeds 256.
- **F-b.** An identified burst source has no ceiling the engine **enforces** — a source whose peak
  is a function of user data with no bound the engine can hold it to.
- **F-c.** The transport-locate catch-up burst is proportional to the number of automated
  parameters with no stated upper bound.

**The acceptance rule, likewise written first.** Unreachability holds **only if all three**: **A-a**
the corpus peak is at most 128, so a project twice as dense as anything written still fits; **A-b**
every burst source has an enforced ceiling rather than a value it happens to have; **A-c** raising
the cap to cover the worst source still fits the callback's real-time budget. A corpus peak under
256 is explicitly **not** sufficient alone: the corpus is what has been written, not what can be
written.

**Result: `Not supported`.** A-b fails. **A-a holds only over the 23 projects this record measures**,
which is not the whole corpus — see *Inputs and controls*. A-c is not evaluated.

## Inputs and controls

**27 projects scanned** — every `.ptz` under `assets/examples/projects/` and
`corpus/v2-reference/projects/`. **23 measured.** Two contain no played events
(`all-modules-reference`, `YAMS AudioScript Wavefolder`) and two are **excluded because their
streams cannot be derived from stored notes**: `Expression & Note-Processor Demo` (9 processor
racks) and `Chrome & Graph Teaching Demo` (3 note graphs). **Their rendered streams are unknown, and a small stored-note count does not bound
them**: a rack or graph can expand one source note into up to 128 notes per tick, so a project with
24 stored notes can emit far more. An earlier revision of this record argued from the stored counts
that neither could carry the peak; that argument is withdrawn. **Every occupancy figure below
therefore covers 23 projects, not the corpus**, and A-a is accepted only over those 23.

**Rates are sampled, not exhausted.** `accepted_sample_rates` is an inclusive range from 8 000 to
192 000 Hz; this measurement takes **six points** in it — 8 000, 22 050, 44 100, 48 000, 96 000 and
192 000 — and reports the worst. It does not compute a result for every admissible rate, and an
earlier revision claimed it did. What is established about the endpoints is that a *lower* rate
makes a 64-frame quantum span more musical time, so 8 kHz is the worst of the six and 192 kHz the
mildest. An earlier revision used 48 kHz alone and called it conservative, which was backwards.

**Controls run before trusting the result.**

- **The tempo integrator is validated against hand-computable cases**: constant tempo (960 ticks at
  120 BPM / 48 kHz = 24 000 frames, exact), a step change (exact), and a linear ramp, whose closed
  form `K·(t1−t0)/(b1−b0)·ln(b1/b0)` matches numerical integration over 200 000 slices to within
  0.000000 frames.
- **The automation parser was checked rather than assumed.** Projects reporting zero automation
  genuinely contain none; `automation` is a list of `{target, points}` lanes. A silent parse miss
  would have understated every peak.

## Method

1. Expand `arrangement` into absolute song ticks at 960 PPQN, **honouring playback bounds**: a note
   or automation point stored at or beyond `Pattern.length` is never played; `length_override`
   clips or extends the placement; and `PlacementLoopMode::repeat` repeats the source until the
   placement ends while `clip` plays it once.
2. Convert ticks to frames through the project's **tempo map**, integrating step changes and
   linear ramps at each admitted rate, then apply ADR-0032 clause 15's single
   round-half-away-from-zero conversion to integer `PlanPosition`.
3. Emit **two** events per played note — an edge at its start and one at `start + duration` —
   because `NoteEdge` is an edge and both consume a slot.
4. Emit one event per played automation point.
5. Bucket the rounded integer position by `frame // 64`; report the maximum bucket.

## Reproduction

```text
python3 -B plans/v2/evidence/phase-03/evd_0015_quantum_occupancy.py
```

## Results

**A-a holds over the 23 measured projects, with a factor of seven to spare.** It is not established
for the two excluded ones.

| Project | Peak events in one quantum | Notes | Automation points | Lanes | Peak polyphony |
|---|---:|---:|---:|---:|---:|
| `oxygene-dreams-2-fixed` | **36** | 2 601 | 466 | 13 | 25 |
| `Karu Sydän — Suomalainen Tango` | 29 | 1 668 | 217 | 10 | 22 |
| `Synth Pop a la Codex` | 26 | 1 932 | 0 | 0 | 16 |
| `Neuro F#m 174-extended` | 22 | 3 998 | 458 | 10 | 13 |
| `Nemesis_the_Warlock` | 16 | 6 514 | 25 893 | 34 | 4 |

The current artifact was rerun after the rounding correction; the reported project table, peak 36, release peak 14
and polyphony peak 25 were unchanged. The corpus peak is **36** against a cap of 256. Peak polyphony is **25** voices, the most releases
in one quantum is **14**, and distinct automation targets peak at **34** — so F-c does not fire on
this corpus either: a locate emitting one value per lane is an order of magnitude inside the cap.

**Occupancy came out identical at all six sampled rates**, and the mechanism is that these peaks
are dominated by events sharing **one tick** — chord width, plus automation quantized to the same
grid position — rather than by the width of the window.

**That is an observation over this corpus, not a law, and an earlier revision of this record stated
it as one.** The draft argued that every quantum at every admitted rate is narrower than the
spacing between musical positions, since a 64-frame quantum spans at most 15.1 ticks at 8 kHz while
a 64th note is 60 ticks. **The corpus refutes it**: `Karu Sydän — Suomalainen Tango` has
neighbouring event positions **5 ticks apart** (ticks 151 872 and 151 877, and two other such
pairs), and 63 of its 1 111 neighbouring gaps are under 60 ticks. Such a pair shares an 8 kHz
bucket and not a 48 kHz one. Rate-invariance held here because those close pairs do not fall at the
peak buckets — which is a fact about these projects, not a guarantee.

**A-b fails: `max_note_expansion_per_tick` cannot bind quantum occupancy by itself, and the two regimes are
unrelated.**

Expansion does not obey the grid the corpus does: one active pattern/placement emits at tick granularity and may fill
*every* tick. Its buffer bound is 128 `ExpandedNote`s per **tick**, and each expanded note contributes an onset. The
table multiplies only tick instants inside the current quantum by those 128 onsets. It deliberately does **not** call
that product a total-occupancy upper bound: releases produced earlier and simultaneous active placements can add to
it. The cap is 256 per **quantum**, and the number of tick instants a quantum intersects depends on rate, tempo, and
sub-frame phase:

| Rate | 93 BPM: instants (onset contribution) | 200 BPM: instants (onset contribution) |
|---:|---:|---:|
| 8 000 | 12 (1 536) | 26 (3 328) |
| 22 050 | 5 (640) | 10 (1 280) |
| 44 100 | 3 (384) | 5 (640) |
| 48 000 | 2 (256) | 5 (640) |
| 96 000 | 1 (128) | 3 (384) |
| 192 000 | 1 (128) | 2 (256) |

At the worst admitted rate the per-tick limit permits one active placement to contribute **thirteen times** the cap in
onsets alone, before releases or simultaneous placements are added. At 192 kHz the same calculation reaches exactly
the cap at 200 BPM and half of it at 93 BPM, from a per-tick limit whose value never changed.

This is F-b, not merely a claim that 128 is the wrong number. The enforced 128 ceiling is per pattern/placement, while
the current profile has no ceiling on the number of simultaneous active placements whose expansions can target the
same quantum. A finite whole-plan destination envelope does not exist until ADR-0046's admission rule constructs and
enforces one. Raising the renderer cap from the observed corpus peak therefore cannot establish unreachability.

**Re-denominating that limit per quantum is not the repair**, for two independent reasons:

- It would state a musical rule in DSP buffer units, which `HOST-INV-011` forbids — a budget whose
  meaning is a duration is evaluated in seconds at the prepared rate, never in frames. The same
  arpeggiator would otherwise produce different music at 44.1 kHz and 192 kHz.
- More fundamentally, **a bound on production does not bound occupancy.** Three quantities differ:
  graph work performed during a quantum, notes the graph produces, and *final events targeting* a
  render quantum. Only the last is what the cap holds, and events produced in different quanta can
  converge on one destination — a note's release lands at `start + duration`, and nothing relates
  one note's duration to another's. Occupancy is
  `E_sequence + E_graph + E_automation + E_live + E_transport + E_internal`, and bounding one term
  proves nothing about the sum.

### The anchor phase does not change admission on this corpus

A compiled event carries a `PlanPosition`; its `SampleTime` exists only after anchoring, and
ADR-0032 clause 27 fixes that conversion as `time = anchor.time + (position - anchor.position)`.
`anchor.time` is an integer frame count, so an anchor shifts every event by a whole number of
frames and **the complete phase space is the `Q` integer offsets within one quantum**.
Prepared-profile plan admission runs before any anchor exists and must therefore hold for all `Q` of
them.

Swept at 8 kHz, the worst admitted rate:

| Project | Fixed phase | Worst phase | Δ |
|---|---:|---:|---:|
| `oxygene-dreams-2-fixed` | 36 | 36 | +0 |
| `Karu Sydän — Suomalainen Tango` | 29 | 29 | +0 |
| `Synth Pop a la Codex` | 26 | 26 | +0 |
| `Neuro F#m 174-extended` | 22 | 22 | +0 |
| `Nemesis_the_Warlock` | 16 | 16 | +0 |

**Every measured project is unchanged.** The reproduction artifact prints all 23 rows, not only the
busiest subset. The mechanism is the same one that makes occupancy rate-invariant here: these peaks
are events sharing exactly one tick, and no phase offset can split a same-tick cluster — every
member is displaced identically.

**A control establishes that the sweep can see a difference at all**, so the zero is a result and
not a blind instrument: frames `[60, 70, 130]` occupy one slot at phase 0 and two at phase 4, and
the sweep reports `fixed 1, worst 2`. Phase matters whenever events are close but not equal; it
cannot matter when they are equal.

**What this supports and what it does not.** It supports a plan-admission rule stated as the
worst case over every anchor phase: on this corpus that rule refuses nothing a fixed-phase rule
would admit, so the conservatism is free here. It does **not** establish that the two are equal in
general — they are equal only while peaks are same-tick clusters, and the corpus contains
neighbouring events as close as 5 ticks, which a wider quantum would merge.

## Limitations

- **Two projects with note expansion are excluded**, so the measured occupancy is the *authored*
  stream, not the rendered one, for those two. Measuring them needs the compiler and the Note Grid,
  which is Phase 3 and Phase 5 work.
- **Live ingress is not measured and cannot be**, because V2 has no ingress. It is bounded
  separately by the pre-renderer queue, which ADR-0021 already permits to drop, counted.
- **A-c is not evaluated.** Whether the callback budget admits a larger cap needs a benchmark of
  the render loop under load. Worth noting for whoever runs it: `max_events_per_quantum` is
  recorded as *Chosen*, replacing `LIMIT-0075`, an unbounded `Vec` — so 256 has no measurement
  behind it, and V1 runs with no cap at all.
- **Releases are not clipped to the placement.** A note whose release falls past the end of its
  placement is counted where it falls. This is the conservative direction for the question asked.
- **Track mute and solo are not honoured.** No corpus project sets either, so no reported number
  can be affected, but a project that did would be overcounted.
- **Three code paths are implemented and never exercised by this corpus**, so the method is not
  evidenced for them: placement looping never fires (the most repetitions any placement needs is
  one), no note carries a per-note track override, and no note has zero duration.
- **A withdrawn claim, with the remaining obligation kept.** An earlier revision asserted that
  `max_active_voices` = 512 by itself contradicts `max_events_per_quantum` = 256 because every voice
  must eventually release. A state capacity does not prove that those releases share a quantum, so
  the corpus conclusion does not use that argument. It also does not assume they cannot converge:
  ADR-0046 gives non-compiled releases separately admitted `EventCount` hold entitlements, maps their total to a
  release-share throughput guarantee, requires a bounded mass-release event, and requires Phase 3 to reselect 256
  from the measured partition before enabling the contract. `HeldNoteCount` is not converted into that entitlement:
  compiled and non-compiled held state consume different capacity paths, and plan admission reports which resource
  binds.

## Conclusion

**`Not supported`.** An over-full quantum cannot be made unreachable by choosing a larger
`max_events_per_quantum`. The limit intended to hold production back is denominated per tick while
occupancy is per quantum, their single-placement onset-contribution ratio varies thirteenfold across
the admitted rate endpoints, and in any case bounding production does not bound occupancy once events
may target future quanta.

Authored music is not the risk: 36 against 256, decided by chord width and invariant to rate.
Expansion is, and the two are governed by unrelated regimes.

Unreachability remains achievable, but as an architecture rather than a number: ADR-0046 partitions
the total among producer classes, admits compiled and authored-runtime envelopes, publishes through
one bounded arbiter, and gives future releases end-to-end holds. Those relations, not this corpus
peak, are what can make an over-full renderer quantum a defect rather than a load condition.
