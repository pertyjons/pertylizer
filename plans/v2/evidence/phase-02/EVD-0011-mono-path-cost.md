# EVD-0011: What the mono path costs after the arena became interleaved

| Field | Value |
|---|---|
| ID | EVD-0011 |
| Status | Complete |
| Phase | 2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-08-19 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `a00c253e` against `9992955b` |
| Retention | Until phase exit |
| Related | ADR-0041, EVD-0010, P02-T013 |
| Artifacts | `EVD-0011-mono-path-cost.csv`, `crates/synth_engine_v2/examples/mono_path_cost.rs` |

## Question and falsifier

ADR-0041 clause 3 promises that a mono signal's **storage** is unchanged and
refuses to promise that a mono path *costs* the same afterwards: EVD-0010
measured the storage question with the kernels of the time, and the conversion
changes the ABI around them — a step now carries a layout, `bind` resolves a
recorded region instead of multiplying a slot index by the quantum, and the
kernel is handed a channel count. The record carries that as a named risk whose
control is this re-measurement.

The question is therefore: **does rendering the minimal voice path at a mono
profile cost more after the conversion than before it?**

The falsifier is a difference outside the control spread, and the rule is stated
in all three directions rather than only the one the risk feared:

- **inside** the control spread — the paired median's magnitude is no greater
  than the median per-round separation between two identical measurements — the
  mono path is unchanged as far as this instrument can see;
- **outside it and positive** — a regression, to fix or to record under the risk
  clause, not a rounding error to absorb;
- **outside it and negative** — an improvement, reported as the net figure it is.
  A comparison of two builds cannot say which change produced it, so any reading
  of the mechanism is a hypothesis until an arm isolates one.

The threshold is the **median** of the per-round control separations, not the
worst. An outlier round is what the minimum estimator already discards, and a
threshold taken from the worst round is a threshold chosen after seeing the
data. The first version of this record did exactly that, and an external review
refused it; see *History*.

## Inputs and controls

- **The plan**: the minimal voice path — envelope, sine, filter, amplifier,
  output — at a mono harness profile, 48 kHz, `Q` = 64, maximum block 64 so one
  `render` call is one quantum. It is clause 16's first fixture, which is the
  path the phase renders.
- **The two builds**: `9992955b`, the planar commit that carries clause 16's
  baselines, and `a00c253e`, the interleaved conversion. The **same** measurement
  file runs against both — it uses only the public compile/prepare/render path,
  so neither build gets a bespoke harness.
- **The control**: each invocation runs the arm twice, alternately, and reports
  both. The two are the same code measuring the same build, so what separates
  them is what this machine does to any two identical runs.
- **Environment**: release profile, `taskset -c 10,11` on a hybrid-core machine
  with a permanent background load, so the two builds are pinned to the same two
  cores rather than compared across core types.
- **Order**: the two builds alternate which runs first, pair by pair. Running one
  build first every time confounds it with warm-up, frequency scaling and
  short-term drift — for a sub-percent margin that is not a detail, and the
  duplicate arm inside one invocation does not control it because both halves sit
  in the same position.
- **Symmetry**, stated before collection: identical source for the measurement,
  identical profile, identical fixture, identical iteration counts, and the two
  builds run **alternately** rather than one after the other, so a machine that
  drifts over the run drifts through both.

## Method

Ten pairs, each twenty-five rounds of fifty thousand rendered quanta — EVD-0010's
own counts. Odd pairs run the planar binary first, even pairs the interleaved
one, so build identity is crossed with position rather than confounded with it.

The estimator is EVD-0010's, and matching it is the difference between measuring
the renderer and measuring the clock. A round times **one batch** of iterations
and divides by the count, so `Instant::now()` is read twice per round rather than
twice per quantum; at a few hundred nanoseconds per quantum, a per-call timer is
a measurable fraction of the figure. Within a round the **control runs first**,
and each control is compared to its arm within the round both were measured in.
The **minimum** over rounds is one run's figure, and the **median** over the ten
runs of each build is the reported one: a minimum cannot be inflated by a scheduler, and a
median cannot be moved by one unlucky run.

Paired differences are reported beside the medians because a paired comparison
survives a machine-wide drift that an unpaired one absorbs.

The renderer is prepared once outside the timed loop and its gate opened before
timing, so no iteration times an allocation or a silent voice. The first `render`
call returns the primed carry and renders no quantum, so the gate goes on the
second — the first that renders.

## Reproduction

```text
git worktree add <dir> 9992955b
cp crates/synth_engine_v2/examples/mono_path_cost.rs <dir>/crates/synth_engine_v2/examples/
cargo build --release -p synth_engine_v2 --example mono_path_cost          # in both trees
taskset -c 10,11 <binary> 25 50000    # ten pairs, alternating which build runs first
```

## Results

Raw per-pair figures, with each run's position, are in
`EVD-0011-mono-path-cost.csv`.

| Quantity | Value |
|---|---|
| Planar, median of ten | **687.34 ns** per quantum |
| Interleaved, median of ten | **686.23 ns** per quantum |
| Difference of medians | −1.11 ns, −0.16% |
| Paired difference, median of ten pairs | **−2.52 ns**, −0.37% |
| Paired difference, range | −13.20 ns to **+1.22 ns**; eight of ten negative |
| Paired median, planar first | −2.09 ns |
| Paired median, interleaved first | −2.95 ns |
| Order effect: second position minus first | +0.46 ns |
| Control spread, median | 0.18% |
| Control spread, worst | 0.50% |

**The direction survives counterbalancing**: both orders give a negative paired
median, and they differ by less than a nanosecond, so what the earlier
planar-first runs measured was not the order. The pooled position difference is
+0.46 ns, which is 0.07% — **below** the median control spread, so this method
cannot say an order effect exists at all. That is not a licence to have skipped
the counterbalancing: an effect too small to measure directly can still be the
whole of a sub-percent margin, which is why the design cancels it rather than
arguing it away.

**The magnitude does not hold still.** Three independent runs of this instrument
gave −5.98, −3.61 and −2.52 nanoseconds as its method tightened: batch timing
first, then EVD-0010's control ratio, then counterbalanced order. Each correction
moved the figure toward zero, which is the signature of a measurement shedding
its own artifacts rather than converging on a value. The paired median is now
about twice the median control spread, and two of ten pairs point the other way.

So the result is asymmetric, and the record states it that way: **no regression**
is well supported; **an improvement** is supported in direction and not in
magnitude.

## Limitations

- **One machine, one profile, one plan.** The figure is the minimal voice path at
  `Q` = 64, mono, on the cores named above. It says nothing about a larger graph,
  another quantum, or another machine.
- **It measures the net effect of the whole conversion, and cannot attribute it.**
  Every change between the two commits is in both builds at once: the recorded
  regions, the borrow order, the kernel ABI, the widening and the boundary copy.
  Nothing here isolates any of them, so this record does not say *why* the mono
  path got faster — see *Hypothesis* below for the reading that has to be tested
  rather than assumed.
- **The margin is close to the floor.** At roughly twice the median control
  spread, with two pairs of the opposite sign, this is near the smallest
  difference the method can distinguish from its own noise. A future comparison
  of the same shape should not be expected to resolve anything smaller.
- **It measures the renderer, not the compiler.** Preparation and admission are
  outside the timed loop by construction, so a conversion that made compilation
  slower would not appear here.
- **It is not EVD-0010's comparison.** That record compares two layouts inside one
  build; this one compares one build against another. Their figures answer
  different questions and are not comparable.

## Hypothesis, untested

The likely mechanism is the boundary: a mono output that walked frames with index
arithmetic became one `copy_from_slice`, and EVD-0010 measured *its* indexed
stereo carry write at 45.8 ns per quantum against a frame-strided form. That is a
different comparison — stereo, and against an iterator rather than a memcpy — so
it is a reason to suspect the boundary rather than a measurement of it.

Testing it needs an arm this record does not have: a build that changes the
boundary operation and nothing else. Until one exists, the ABI's cost and the
boundary's saving are both unmeasured, and this record claims neither. What it
claims is the net figure above.

## Conclusion

**Supported**, in the sense the risk asked about: there is no regression on the
mono path. The paired difference is negative under both orders and outside the
control spread, which is the rule's verdict for an improvement — but at 0.37%
against a 0.18% spread, with two of ten pairs positive and a magnitude that
halved as the method tightened, the honest claim is a direction rather than a
figure. The mono path did not get slower; it may have got a few nanoseconds
faster.

ADR-0041 clause 3's refusal to promise the mono path's cost was the right call,
and the reason is now visible: the answer was not zero in either direction, and
guessing it would have been guessing.

## History

The first version of this record measured 622 ns and reported +0.32%, and an
external review refused it on two grounds, both correct.

Its instrument timed **each quantum separately** and took the fastest single
sample, where EVD-0010 times a whole batch. At this scale that measures the clock
alongside the renderer and then selects the luckiest quantum — which is why its
figures were both lower and noisier than these, and why its margin was inside the
noise.

Its acceptance rule then said "inside the control spread" without saying which
spread, and the result was accepted against the **worst** of nine rather than the
median — a threshold chosen after seeing the data. That is the defect this
project has recorded before, in ADR-0002's "materially larger" revisit
condition, and the rule above now names its aggregation before it is applied.

A third review round found the remaining confound: every pair ran the planar
build first, so build identity travelled with position. The design above crosses
them, and the run reported here is the counterbalanced one. Its margin is the
smallest of the three, which is the point of having corrected it.
