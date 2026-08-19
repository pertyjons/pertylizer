# EVD-0012: What the render quantum costs on the real V2 path

| Field | Value |
|---|---|
| ID | EVD-0012 |
| Status | Complete |
| Phase | 2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-08-19 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `14a292bc` |
| Retention | Permanent |
| Conclusion | Inconclusive — rule 1′(c); escalated and decided by the user |
| Related | ADR-0037, ADR-0001, EVD-0002, EVD-0010, EVD-0011, P02-T010 |
| Artifacts | `EVD-0012-render-quantum-real-path.csv`, `EVD-0012-digests.csv`, `EVD-0012-discarded-invocation-granularity.csv`, `evd_0012_analyse.py`, `crates/synth_engine_v2/examples/quantum_cost.rs` |

## Question and falsifier

[ADR-0037](../../decisions/ADR-0037-render-quantum-value.md) accepted `Q` = 64
**provisionally**. Its evidence, [EVD-0002](../phase-00a/EVD-0002-render-quantum-cost-proxy.md),
could not measure the quantity that separates the options — per-quantum overhead
in the V2 compiled node model — because no V2 renderer existed. It measured V1's
cost against block size as a proxy, recorded that the proxy's direction of error
is unknown, and fired rule 1: accept 64 provisionally, with a **binding** Phase 2
re-measurement against real V2 nodes.

This is that re-measurement. The question is: **what does the render quantum cost
on the V2 path the crate now compiles, and does that confirm 64 or select another
value?**

Let `c(Q)` be the measured cost of rendering one second of audio through the V2
renderer at quantum `Q`, and `r(a, b) = c(a)/c(b) - 1`. These are EVD-0002's
quantities, measured on the real renderer rather than on a proxy.

### The rule table

ADR-0037's rules are evaluated **in order**, stopping at the first that applies.
Rules 2 to 5 are used exactly as accepted:

| # | Condition | Outcome |
|---|---|---|
| 2 | `r(64, 256) > 15%` and `r(128, 256) <= 15%` | **Select 128** — supersede ADR-0037 |
| 3 | `r(64, 256) > 15%` and `r(128, 256) > 15%` | **Escalate** — every candidate is expensive |
| 4 | `r(32, 64) <= 2%` | **Select 32** — the finer resolution and lower latency are free; supersede ADR-0037 |
| 5 | otherwise | **Confirm 64** — ADR-0037 becomes final |

### Rule 1 is not applicable, and this is the Phase 2 resolution rule that stands in its place

ADR-0037's rule 1 fires when a comparison falls within 5 percentage points of its
threshold, and its outcome is "accept 64 provisionally with the binding Phase 2
obligation" — unreachable from this measurement, because this *is* that
obligation and ADR-0037 states that Phase 2 may not close with the question still
open. The 5-point margin was attached to the proxy: it existed because the
proxy's transfer error had no established direction. Neither the outcome nor the
margin transfers to a direct measurement.

This record therefore states its own resolution rule, before collection, in
rule 1's first position. It does not amend ADR-0037; it supplies the uncertainty
rule that record's rule table needs when it is evaluated against a direct
measurement, which [`PROCESS.md`](../../PROCESS.md) requires an EVD to state
before collecting.

**Rule 1′ — unresolvable by this instrument.** A comparison the rule table uses
is unresolvable when any of the following holds. If any comparison the evaluation
*reaches* is unresolvable, rule 1′ fires.

- **(a) The margin is below the measured noise floor.** The noise floor `N` for a
  comparison is the **larger** of two measured quantities, computed **per shape,
  per variant and per comparison**:
  - `|r(64′, 64)|`'s median over sweeps, where `64′` is a **second, independently
    built `Q` = 64 arm**; and
  - that comparison's own **median absolute deviation across sweeps**. The same
    five binaries run thirty times have no true sweep-to-sweep variation, so the
    dispersion of a ratio across sweeps *is* what this instrument does to it.

  Two quantities rather than one, because neither alone bounds every comparison.
  The null is the only measurement that is *known* to have a true value of zero,
  so it is what says the instrument is unbiased at all — but the sweep order is a
  cyclic rotation, which holds every pair of arms at a **fixed position distance**:
  `64′` sits beside `64` in every sweep, as do `32` and `64`, and `128` and `256`,
  while `64` and `256` are three positions apart in every sweep. The null
  therefore bounds the distance-one comparisons and not `r(64, 256)`. The
  per-comparison dispersion has no such gap, because it is measured on the
  comparison itself at its own separation.

  Taking the larger of the two can only make a comparison **harder** to resolve,
  never easier. That is the direction a correction to an acceptance rule has to
  run in, but it is not a claim of no bias in general: a wider floor makes rule
  1′(a) — the inconclusive outcome — more reachable, so the amendment could in
  principle favour irresolution. On **this** collection it did not, and that is
  checkable rather than argued: every comparison the evaluation reaches clears
  the original null-only floor as well, and the outcome is decided by rule 1′(c),
  which no floor enters. It is the same source with the same
  constant, so its true ratio to `64` is zero, and whatever this instrument
  reports for it is what the instrument does to two identical programs measured
  in different processes from different builds — the exact boundary every other
  `r(a, b)` crosses. A comparison whose distance from its threshold is smaller
  than `N` is one this instrument cannot resolve.
- **(b) The variants disagree.** The two per-call-work variants below select
  different rules.
- **(c) The shapes disagree.** A bounding shape selects a different rule than the
  governing shape. The rule table assumes one number; if the shapes select
  differently, the answer is a property of the graph rather than of `Q`, and
  reading one shape's outcome as the answer would be choosing the shape after
  seeing the data. Selecting a *different number* is not disagreement; selecting
  a **different rule** is.

Rule 1′'s outcome is **escalate to the user with the figures**, not a default to
64. Selecting between values this instrument cannot separate is a trade of
control resolution and carry latency against CPU, which ADR-0037 lists among its
decision drivers and which `PROCESS.md` classifies as requiring an explicit
product choice.

### What would make the preferred conclusion wrong

The preferred conclusion — the one that costs least — is rule 5, confirm 64.
It is wrong if `r(32, 64) <= 2%`, since that makes 32's finer control resolution
and lower carry latency free; or if `r(64, 256) > 15%`, since that makes 64 the
expensive option the proxy could not rule out. Both are measured here directly.

## Inputs and controls

### The arms

`Q` ∈ {32, 64, **64′**, 128, 256}. The first, second, fourth and fifth are
ADR-0037's options A, B and C plus the reference denominator; `64′` is the null.

**256 is not a candidate** — 5.33 ms of control resolution and the same again in
carry latency — and is measured for one reason only: rules 2 and 3 are expressed
against `c(256)`, exactly as EVD-0002 measured it. This record does not propose
it.

`QUANTUM_FRAMES` is a compile-time constant that ADR-0001 clause 1 forbids
exposing as configuration, so an arm is a **separate build**: a git worktree at
the source revision with that one constant edited, and the same measurement file
copied into each. No configuration surface is added to the crate, and the
reproduction below states the edit. `64′` is a second worktree with the constant
left at 64, built independently, so it differs from `64` in nothing but its
build and its process.

### The two variants, and the cost this renderer does not yet pay

ADR-0001 clause 5 gives the renderer an **input** carry as well as an output one:
each call appends `N` input frames to it and each quantum consumes `Q` from it.
The renderer at this revision **allocates that buffer and never reads it**, because
no node in this phase consumes live input; `PreparedRenderer::input_carry` says
so at its declaration. That work is contracted and unimplemented, so a
measurement of the renderer as built is not a measurement of the renderer
ADR-0001 describes, and the omission is not neutral: the missing work is
per-frame rather than per-quantum, so it adds roughly the same amount to `c(Q)`
at every `Q` and therefore shrinks every ratio toward zero. It moves rule 4 and
rule 2 in opposite directions, so it cannot be argued away as conservative.

Each shape is therefore measured in **two variants**:

| Variant | What the timed loop does |
|---|---|
| `as-built` | The renderer exactly as it exists at the source revision |
| `clause-5` | The same, with the harness performing clause 5's input-carry procedure around every call |

`clause-5` is not invented work: it is the procedure clause 5 states, in the shape
the *output* carry already implements. It is an estimate of the missing cost
rather than the eventual implementation, and it is labelled as such.

Its specification, because the details decide whether it measures anything:

- **Its own buffer.** `PreparedRenderer::input_carry` is private and a harness
  cannot reach it, so the variant allocates its own of exactly clause 5's size,
  `maximum_block_size + Q` frames of the stream's channels, and does clause 5's
  bookkeeping over that. It simulates the cost; it does not simulate the state.
- **From the first call, not from the timed loop.** The wrapper runs around the
  priming call, the note call and every settle call as well, so the occupancy the
  timed loop inherits is the one clause 5 actually produces. That is not zero: on
  a call of `N` frames the procedure appends `N` and consumes `Q` per quantum
  rendered, and the primed output carry means the first call renders none — so
  the steady occupancy entering each later call is `Q` frames, and the compaction
  each call performs moves `Q` frames rather than nothing. That remainder grows
  with `Q`, which is exactly why starting the wrapper at the timed loop would
  have measured a different and smaller thing.
- **Quanta counted before the call.** `quanta_needed_for` reads the carry, so the
  wrapper asks it before `render` rather than after.
- **Observed, so the optimizer cannot delete it.** The release build would be
  free to drop copies into a buffer nothing reads, so the carry is passed through
  `black_box` and one sample of it is folded into a value the harness prints.

Rule 1′(b) makes disagreement between the variants unresolvable rather than
letting either be picked afterwards.

### The shapes, and which one governs

| Shape | Graph | What it is for |
|---|---|---|
| **`voice-mono`** — **governing** | Envelope, sine, filter, amplifier, output, at a mono profile | The path Phase 2 exists to render, and ADR-0041 clause 16's first baseline fixture |
| `voice-stereo` — bound | The same graph at a stereo profile | Adds the widening operation and the wider boundary copy |
| `gain-chain` — bound | A sine into 32 gains into the output, mono | A deliberately dispatch-heavy shape: 33 kernel dispatches per quantum against one multiply per sample each |

The governing shape is named **before** collection, and the bounds exist to show
whether the outcome is a property of the graph rather than of `Q`.

**No claim is made that `gain-chain` is extremal in any direction.** Two earlier
drafts made one — first that it is the crate's worst case, then that a gain has
the least arithmetic among chainable kernels — and both were refused: `Silence`
and `Constant` fill without multiplying, a disconnected node is scheduled and
dispatched without being chained at all, and an `Amplifier` whose control input
is unpatched takes a zero-fill path with no multiply either. The shape's role
does not need a superlative: it exists to show whether the outcome moves when the
dispatch count rises against the per-sample arithmetic, which 33 dispatches of a
one-multiply kernel does whether or not something cheaper exists. The default
`max_nodes` is 16 384, so the count is far inside admission.

### Symmetry, stated before collecting

- **Every arm renders the same audio.** Same graph, same total frame count, same
  caller block size of 512 frames, same starting plan position; only the internal
  quantum differs. Per-sample work is therefore identical by construction and
  what remains is per-quantum overhead.
- **And it is checked, not assumed.** Each arm renders each shape offline through
  `render_offline` — which is latency-compensated, so its first output sample is
  plan sample 0 at every `Q` under ADR-0001 clause 9 — and prints a SHA-256 digest
  over the result. **The five digests must agree per shape before any timing
  figure is read.** If they do not, the arms are not rendering the same audio and
  no ratio between them means anything.

  The comparison is fixed so that it is one reproducible number: **49 152 frames**
  from plan sample 0, digested as every sample's IEEE-754 `f32` bit pattern in
  four little-endian bytes, in frame order with a frame's channels adjacent —
  ADR-0041 clause 16's encoding, so the two records' digests mean the same thing.
  The two voice shapes present the note at plan sample 0 and nothing after it.
  **`gain-chain` has no envelope and therefore no note**: it renders with no
  events at all, which makes its output `Q`-independent for the same reason and
  by a shorter argument.
- **ADR-0001 clause 17 is not violated by this.** It makes a render digest
  comparable only within one quantum value because *control response* moves with
  `Q`. These fixtures present at most one note, at plan sample 0, and no
  control-rate event at all, and since P02-T007 a note edge lands at its declared sample at every
  `Q`, so the clause's subject does not arise. The digest agreement is a property
  of these fixtures and is not claimed for plans that automate a control.
- **Identical source for the measurement**, identical build profile (`--release`),
  identical round and iteration counts, identical fixtures.

### The controls

Two, measuring different things, and only the first is a threshold:

- **The noise floor `N`** — the larger of the `64′` arm's median ratio and the
  comparison's own dispersion, both described above. It spans process and build
  boundaries, which is what every `r(a, b)` spans, and it is what rule 1′(a) is
  evaluated against.
- **The in-process spread** — each invocation runs each shape and variant twice,
  the control first, and reports both. It is the narrower quantity: what this
  machine does to two measurements inside one process. It is reported as a
  diagnostic, so a run whose in-process spread is near `N` is visibly a run whose
  arms were noisy, but it is **not** the threshold. An earlier draft used it as
  one, which would have compared a within-process spread against a
  between-process ratio.

Both are medians over rounds rather than worst cases: a threshold taken from the
worst round is one chosen after seeing the data, which is the correction an
external review forced on EVD-0011.

### Environment

Release profile, `taskset -c 10,11` on a hybrid-core machine carrying a permanent
background load, so every arm is pinned to the same two cores rather than
compared across core types.

## Method

### One figure

The estimator is EVD-0010's and EVD-0011's, and matching it is the difference
between measuring the renderer and measuring the clock:

- a round times **one batch** of 2 000 `render` calls of 512 frames each and
  divides by the frames that batch rendered, so `Instant::now()` is read twice
  per round rather than twice per call;
- within a round the control runs first, and each control is compared to its arm
  within the round both were measured in;
- the **minimum** over 3 rounds is that arm's figure for that sweep — a slower
  round was slower for a reason outside the code under comparison.

Three rounds rather than the twenty-five an earlier draft specified, and thirty
sweeps rather than ten. *History* records why and does not describe the change as
free: a minimum over three observations is a different order statistic from a
minimum over twenty-five, not merely a noisier one.

Before timing, and identically in every arm:

1. prepare the plan, whose profile has a maximum block of 512 frames;
2. render one call of exactly `Q` frames with no events. The output carry is
   primed with `Q` frames of silence, so this call serves the priming and renders
   **no** quantum, which is why an event cannot be presented with it;
3. render one call of exactly `Q` frames carrying the note at `SampleTime` 0.
   This is the first call that renders, and the note lands at sample 0;
4. render further calls of exactly `Q` frames until the render clock reaches plan
   sample **49 152**. `Q`-frame calls rather than 512-frame ones, because the
   clock stands at `Q` after step 3 and a 512-frame call advances it by exactly
   512: the reachable set would be `Q + 512k`, which contains 49 152 at no
   candidate quantum. A `Q`-frame call advances it by `Q`, and 49 152 is a whole
   multiple of 32, 64, 128 and 256 alike — 1 536, 768, 384 and 192 quanta.

Every arm therefore enters the timed loop at the same plan sample, with an empty
output carry, and with the envelope long past its 10 ms attack and 100 ms decay
and held in sustain — so no arm times a different segment mixture than another.
The timed loop's 512-frame calls then run in a steady state at every quantum:
each renders `512 / Q` quanta, serves 512 frames, and leaves the carry empty.

### One sweep, and the reported ratios

A **sweep** is one pass over all five arms, each arm running every shape and
variant. Ratios are formed **within a sweep**, from that sweep's own figures:
`r_sweep(a, b) = c_sweep(a)/c_sweep(b) - 1`. The reported `r(a, b)` is the
**median over sweeps** of those paired ratios.

Pairing within a sweep rather than dividing two independently aggregated medians
is what makes the comparison survive drift: a machine that slows down over the
run slows both members of each pair together, and a ratio of two separately
pooled figures absorbs that drift instead of cancelling it.

**Thirty sweeps, with the arm order rotated by sweep index modulo five**, so each
of the five arms occupies each of the five positions exactly six times. Build
identity is thereby crossed with position rather than confounded with it. Nine
sweeps over five arms could not do this, and an earlier draft's four-arm rotation
over nine sweeps could not either.

`c(Q)` is reported as **milliseconds of elapsed render time per second of
rendered audio**. EVD-0002 reported milliseconds of CPU time; `Instant` is a wall
clock, so the two records' absolute figures are not the same quantity and are not
compared. Only the *shape* of the cost-versus-quantum curve is.

## Reproduction

Every command below runs from the **repository root**.

```text
# Five worktrees, one per arm, differing only in that one constant.
for arm in 32 64 64b 128 256; do
  q=${arm%b}
  git worktree add /tmp/evd0012-$arm 14a292bc
  sed -i "s/^pub const QUANTUM_FRAMES: u32 = 64;/pub const QUANTUM_FRAMES: u32 = $q;/" \
    /tmp/evd0012-$arm/crates/synth_engine_v2/src/time.rs
  cp crates/synth_engine_v2/examples/quantum_cost.rs \
    /tmp/evd0012-$arm/crates/synth_engine_v2/examples/
  cargo build --release --manifest-path /tmp/evd0012-$arm/Cargo.toml \
    -p synth_engine_v2 --example quantum_cost
done

# One arm reports six figures — three shapes by two variants — plus its digests:
#   cost,<shape>,<variant>,<measured ms/s>,<control ms/s>,<in-process spread %>
taskset -c 10,11 /tmp/evd0012-64/target/release/examples/quantum_cost 3 2000

# Thirty sweeps; sweep k runs the five arms rotated by k mod 5, which puts each arm
# in each position six times. This is what produces the retained CSV's schema.
ARMS=(32 64 64b 128 256)
OUT=/tmp/evd0012-raw.csv
echo "sweep,position,arm,shape,variant,cost_ms_per_s,control_ms_per_s,in_process_spread_percent" > $OUT
for k in $(seq 0 29); do
  for pos in 0 1 2 3 4; do
    arm=${ARMS[$(( (pos + k) % 5 ))]}
    taskset -c 10,11 /tmp/evd0012-$arm/target/release/examples/quantum_cost 3 2000 \
      | awk -F, -v s=$k -v p=$pos -v a=$arm '/^cost,/ {print s","p","a","$2","$3","$4","$5","$6}' >> $OUT
  done
done

# The digest gate, which every timing figure is read behind.
{ echo "arm,quantum,shape,digest"
  for arm in 32 64 64b 128 256; do
    taskset -c 10,11 /tmp/evd0012-$arm/target/release/examples/quantum_cost 1 10 \
      | awk -F, -v a=$arm '/^quantum,/{q=$2} /^digest,/{print a","q","$2","$3}'
  done
} > EVD-0012-digests.csv

# The rule, applied mechanically rather than by eye. With no argument it reads the
# retained CSV beside it, which re-derives every figure below; pass a path to apply
# the same rule to a fresh collection.
python3 plans/v2/evidence/phase-02/evd_0012_analyse.py
python3 plans/v2/evidence/phase-02/evd_0012_analyse.py /tmp/evd0012-raw.csv
```

## History

Two things were changed after collection began. Both are recorded here because a
method a reader cannot check against the data is not a method.

### The sweep was re-collected at round granularity

The first collection ran **25 rounds per invocation**, so one arm occupied the
machine for two and a half minutes and one sweep took over twelve. Its `c(Q)`
figures came out **bimodal**: every arm sat at one of two levels about 11.5%
apart, which is the ratio of this processor's two sustained turbo bins, and an
arm measured during a hot stretch was measured at the lower one. Within-sweep
pairing does not help when the two arms of a pair are two minutes apart.

The collection was therefore repeated with **3 rounds per invocation and 30
sweeps**, so a sweep completes in about two seconds and the arms in it see the
same thermal state.

**This is not a free change, and the record does not claim it is.** A minimum
over three observations is a different order statistic from a minimum over
twenty-five, not merely a noisier one, and two arms with different variances are
not affected by that change equally. What can be said is narrower: the change
moves no threshold, it was made because the instrument was visibly bimodal rather
than because of anything the ratios showed, and **the discarded collection is
retained** as `EVD-0012-discarded-invocation-granularity.csv` so a reader can
check both claims rather than take them.

Applying the rule to that discarded collection reaches the **same overall
outcome** — rule 1′, escalate — and `gain-chain` selects rule 2 there too, at
+33.8% and +34.3% against the +35.3% and +34.7% measured here. It differs in
exactly the way the bimodality predicts: **two of the six cells that resolve here
are unresolvable there under rule 1′(a)** — `voice-mono`/`as-built`, whose
`r(32,64)` margin is 4.25 pp against a floor of 4.57 pp, and
`voice-stereo`/`clause-5`, whose margin is 7.35 pp against 7.84 pp. Those floors
are the same quantities that read 1.30 pp and 0.88 pp on the retained collection.
The recollection did not move the answer; it restored the resolution the
bimodality had destroyed.

The two-level behaviour is **not gone**, only much rarer; see *Limitations*.

### The noise floor was amended

**The noise floor was amended once, and this is the amendment.** Rule 1′(a)
originally defined `N` as the null arm's median ratio alone. The cyclic sweep
rotation, which is what balances arm against position, also fixes the *position
distance* between any two arms — so the null, which sits beside `64` in every
sweep, could only ever bound the comparisons whose arms are likewise adjacent.
`r(64, 256)`, which rules 2 and 3 are expressed over, is three positions apart in
every sweep and was therefore judged against a floor measured at a distance it
does not span.

The amendment was made **after one and a half sweeps of raw per-arm figures had
been seen and before any aggregate ratio was computed**, which is stated here
rather than left to be inferred. It takes the *larger* of the two floors, so it
can only widen the band in which a comparison is unresolvable; a rule change that
can only make the answer harder to reach is not one that can have been chosen to
reach a preferred answer. The raw data supports both computations, so no
collection was repeated.

## Results

Raw per-sweep figures are in `EVD-0012-render-quantum-real-path.csv`; the rule is
applied by `evd_0012_analyse.py` rather than by eye.

### The gate

**All fifteen digests agree** — five arms, three shapes, one digest each over
49 152 frames from plan sample 0. They are recorded in `EVD-0012-digests.csv`,
one row per arm and shape, so the gate is checkable rather than asserted: fifteen
rows carry three distinct values.

| Shape | Digest, identical at `Q` = 32, 64, 64′, 128 and 256 |
|---|---|
| `voice-mono` | `0fe495aee2793cc6f3cbd3f16b84935320b9866183e6d4c68bdcbea7a1cdc748` |
| `voice-stereo` | `e954b97df4c61c7109d347f02150f53e5f71aeb738ee0aeb2bff80cafd9ef1e2` |
| `gain-chain` | `acf121773d70debd7da91b39a0c5056a66df4e731f2a1119d52d8bfada4b734f` |

The four quanta render bit-identical audio, so the ratios below are ratios
between one program at four quanta rather than between four programs.

### `c(Q)`, in milliseconds of elapsed render time per second of rendered audio

Medians over 30 sweeps, `as-built` variant:

| Shape | `c(32)` | `c(64)` | `c(64′)` | `c(128)` | `c(256)` |
|---|---|---|---|---|---|
| `voice-mono` | 0.5494 | 0.5213 | 0.5272 | 0.4965 | 0.4841 |
| `voice-stereo` | 0.6224 | 0.5739 | 0.5746 | 0.5439 | 0.5269 |
| `gain-chain` | 0.7967 | 0.5669 | 0.5648 | 0.4536 | 0.4193 |

### The ratios the rule table is over

Median of the per-sweep paired ratios, with the noise floor `N` each margin is
judged against:

| Shape | Variant | `r(64,256)` | `r(128,256)` | `r(32,64)` | null `r(64′,64)` | Rule |
|---|---|---|---|---|---|---|
| **`voice-mono`** | `as-built` | +6.14% | +2.15% | +5.52% | +0.17% | **5 — confirm 64** |
| **`voice-mono`** | `clause-5` | +5.89% | +1.91% | +5.31% | +0.09% | **5 — confirm 64** |
| `voice-stereo` | `as-built` | +7.72% | +2.45% | +8.34% | +0.01% | 5 — confirm 64 |
| `voice-stereo` | `clause-5` | +7.78% | +2.22% | +8.00% | −0.02% | 5 — confirm 64 |
| `gain-chain` | `as-built` | +35.26% | +9.04% | +41.51% | +0.04% | **2 — select 128** |
| `gain-chain` | `clause-5` | +34.71% | +8.77% | +41.39% | +0.01% | **2 — select 128** |

The null arm reports between −0.02% and +0.17% everywhere, which is what a
comparison whose true value is zero should report and is the evidence that the
five builds differ in nothing but their quantum.

**Rule 1′(a) does not fire.** Every margin the evaluation reaches clears its noise
floor, by multiples running from **2.70× to 16.8×**. The narrowest is the
governing shape's `r(32,64)` in the `as-built` variant — 3.52 pp from its
threshold against `N` = 1.30 pp — and the widest is the same shape's `r(64,256)`
in the `clause-5` variant, 9.11 pp against `N` = 0.54 pp.

**Rule 1′(b) does not fire.** The two variants select the same rule in every
shape, and no comparison differs between them by more than **0.55 pp** — that
worst case being `gain-chain`'s `r(64,256)`, whose margin is 20 pp. Clause 5's
input carry costs about 0.4% of a render: real, and far too small to move any
threshold.

**Rule 1′(c) fires.** `gain-chain` selects rule 2 where the governing shape
selects rule 5.

### The split is not an estimator artifact

The rule outcome is identical under four estimators — the declared median of
per-sweep ratios, the ratio of medians, the mean of ratios, and the ratio of
per-arm minima — for every shape and variant. `voice-mono` selects rule 5 under
all four; `gain-chain` selects rule 2 under all four.

### Why the shapes disagree

`c(Q)` is `A · (rate / Q) + B`: a fixed cost `A` paid once per quantum and a cost
`B` paid per frame. Fitting that from each sweep's `c(64)` and `c(256)`, and
taking the median:

| Shape | `A` ns per quantum | `B` ns per frame | `A/B` in frames |
|---|---|---|---|
| `voice-mono` | 55.0 | 9.87 | 5.6 |
| `voice-stereo` | 74.8 | 10.68 | 7.0 |
| `gain-chain` | 257.5 | 7.59 | 33.9 |

The fit is not circular: it is built from the 64 and 256 points, and the 128
point is held out. Under the declared estimator — a fit per sweep, medians of the
coefficients — it predicts that held-out point to **−0.39%, −0.60% and +1.56%**
for `voice-mono`, `voice-stereo` and `gain-chain` in the `as-built` variant, and
to −1.19%, +0.09% and +2.05% in `clause-5`. An earlier draft reported 0.0% for
`voice-mono`; that figure came from fitting the pooled endpoint medians, which is
the ratio-of-medians estimator and not the declared one, and it is withdrawn.

`r(64, 256)` is `3x / (256 + x)` where `x` is `A/B` in frames, so it reaches 15%
at `x` = 13.5 frames. The two voice shapes sit at 5.6 and 7.0; `gain-chain` sits
at 33.9. That is the whole disagreement.

Solving `A = A_fixed + n · A_op` across `voice-mono`'s 5 scheduled operations and
`gain-chain`'s 34 gives **≈ 20 ns of fixed per-quantum cost plus ≈ 7.0 ns per
scheduled operation**.

**That is a two-point fit and the third shape does not obey it.** Six operations
would predict `A` ≈ 62 ns for `voice-stereo`, and its fitted `A` is 74.8 ns — so
per-operation cost is not uniform across kinds, which is unsurprising when one of
the operations is a widening copy over a doubled region. The decomposition is an
order-of-magnitude guide to *why* the shapes disagree, not a renderer law, and
nothing below rests on its precision.

Read as such a guide: for a graph of `n` similar nodes `A/B` tends to `7.0 / b`,
where `b` is one node's cost per frame, so the 15% crossover sits near **`b` ≈
0.52 ns per frame per node, about 2.5 cycles per sample at this processor's
4.8 GHz**. The measured shapes sit either side of it by a wide margin — the
figures are averages over each shape's whole operation list, including its
oscillator and its output write, rather than measurements of one kernel:

- `voice-mono` averages 1.97 ns per frame per operation, about 9.5 cycles per
  sample;
- `gain-chain` averages 0.22 ns, about 1.1 cycles, because one multiply per
  sample vectorizes eight wide while its dispatch does not.

## Limitations

- **One machine, one processor, one build profile.** Every figure is from a
  13th-generation Core i7 at 4.8 GHz with two cores pinned. The per-operation
  dispatch cost and the per-sample kernel cost would both move on another
  processor, and `A/B` is their ratio.
- **The two-level frequency behaviour is reduced, not removed.** About a third of
  the 30 sweeps still had their two arms at different turbo bins, which is why
  the ratio of medians reads 1.5 pp higher than the declared median of ratios for
  `voice-mono`. The declared estimator was chosen before collection and is robust
  to a minority of contaminated sweeps; the four-estimator agreement above is
  what shows the outcome does not rest on that choice. No sweep was excluded —
  a filter chosen after seeing which sweeps were contaminated would be a filter
  chosen after seeing the data.
- **It measures elapsed time, not CPU time.** EVD-0002 reported CPU
  milliseconds, so the two records' absolute figures are not the same quantity.
  Only the shape of the cost-versus-quantum curve is compared.
- **`clause-5` is an estimate of a cost that is not implemented.** It is clause
  5's procedure over a buffer of clause 5's size, not the eventual
  implementation, and it is measured because omitting a contracted per-frame cost
  moves rules 2 and 4 in opposite directions. It turns out to cost about 0.4%,
  which is far below every margin here — but that is a result, not something the
  method assumed.
- **The node catalog is this phase's.** Six kernels exist. A conclusion about how
  much work a node does per sample is a conclusion about these six, and Phase 5's
  declarative node API will add kinds whose per-sample cost nobody has measured.
- **`gain-chain` is not a bound**, in either direction. It is a shape with a high
  dispatch-to-arithmetic ratio, not the highest one this crate can express.
- **The rule table's `c(256)` denominator is not a candidate value.** Nothing here
  proposes `Q` = 256; it is the reference EVD-0002 used and is measured only so
  rules 2 and 3 can be evaluated as accepted.

## Conclusion

**Inconclusive, by rule 1′(c), and the irresolution is a finding rather than
noise.**

On the path Phase 2 exists to render, `Q` = 64 is comfortably the right value:
rule 5 fires on the governing shape under both variants and all four estimators,
with `r(64, 256)` at about 6% against a 15% threshold and `r(32, 64)` at about
5.5% against a 2% one. Moving to 128 would save 3.6% of elapsed render time for twice the
control interval and twice the carry latency; moving to 32 would cost 5.5% for
half of each. The stereo voice agrees.

On a graph whose nodes do about one cycle of arithmetic per sample, rule 2 fires
instead: `Q` = 64 costs 35% over 256 where 128 costs 9%. The instrument can see
both results clearly — every margin the evaluation reaches is between 2.7 and
16.8 times its noise floor — so
the disagreement is not something more measurement resolves. `Q` is one constant
and the answer depends on the graph.

### The direct trade between the three candidates

The rule table is expressed against `c(256)`, which is not a candidate. A reader
choosing between 32, 64 and 128 needs the comparisons between *those*, so they
are stated here under the same estimator:

| Shape | `r(32,64)` | `r(64,128)` | `r(32,128)` | What 64 → 128 **saves** |
|---|---|---|---|---|
| `voice-mono` | +5.52% | +3.78% | +9.17% | 3.64% |
| `voice-stereo` | +8.34% | +4.69% | +13.71% | 4.48% |
| `gain-chain` | +41.51% | +23.10% | +73.82% | 18.77% |

The `as-built` variant; `clause-5` differs by at most 0.55 pp in any cell. The
last column is not the third-from-last restated: `r(64,128)` is what 64 costs
*over* 128, while the saving from moving to 128 is measured against 64 and is
therefore smaller — 3.64% rather than 3.78%, and 18.77% rather than 23.10%.

Against which, at 48 kHz, `Q` buys control resolution and costs carry latency in
equal measure — these are arithmetic, not measurements:

| `Q` | Control interval | Carry latency (ADR-0001 clause 7) |
|---|---|---|
| 32 | 0.67 ms | 0.67 ms |
| 64 | 1.33 ms | 1.33 ms |
| 128 | 2.67 ms | 2.67 ms |

So on the voice path, halving `Q` to 32 costs 5.5% of elapsed render time and halves both
figures; doubling it to 128 saves 3.6% and doubles them. On a graph of trivial
nodes the same two moves cost 41.5% and save 18.8%.

What this record therefore establishes, and what it does not:

- **Established.** `Q` = 64 is not expensive on either voice shape — `r(64,256)`
  is 6% and 8% against a 15% threshold — and `Q` = 32 is not free on any shape
  measured, by 5.3 to 41.5 points against a 2% threshold. A graph whose scheduled
  operations average about a cycle of arithmetic per sample pays 35% for `Q` = 64
  over `Q` = 256, and one averaging about ten cycles pays 6%.
- **Not established.** Where Pertylizer's real graphs sit between those two. The
  `A/B` decomposition suggests the boundary is near 2.5 cycles per sample per
  operation, but it is a two-shape fit that the third shape does not obey, and
  every per-operation figure here is an average over a whole shape rather than a
  measurement of one kernel. Nothing here measures an individual kernel's cost,
  and Phase 5's declarative node API will add kinds nobody has measured at all.

The choice therefore returned to the user as ADR-0037's driver list frames it — a
trade of control resolution and carry latency against CPU on graphs that do not
exist yet — which `PROCESS.md` classifies as requiring an explicit product
choice.

### The escalation, and what was decided

The figures above were put to the user on 2026-08-19 with all four courses
available: confirm 64, supersede with 128, supersede with 32, or measure a
realistic large graph before deciding. **The user confirmed `Q` = 64**, on the
reading that the governing shape is the one that resembles an instrument — every
node kind a synthesizer actually has sits far above the crossover — while a chain
of bare multiplies is a shape nobody authors.

ADR-0037 therefore becomes final rather than provisional, and its restriction on
treating 64 as settled is lifted. This record supplies the evidence; ADR-0037
carries the decision.
