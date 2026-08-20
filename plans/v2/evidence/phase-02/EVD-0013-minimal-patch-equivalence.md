# EVD-0013: Musical equivalence of the minimal patch, V2 against V1

| Field | Value |
|---|---|
| ID | EVD-0013 |
| Status | Complete |
| Phase | 2 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-20 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `3acb7e6f` |
| Retention | Permanent |
| Conclusion | Not supported — E2a exceeded, as this record's falsifier defines it. No CORPUS-0001 claim is broken; see *Conclusion* |
| Related | ADR-0040 clause 4, ADR-0042, ADR-0001 clauses 14 and 17, ADR-0037, ADR-0041 clause 16, CORPUS-0001, EVD-0001, EVD-0012, EVD-0014, P02-T008 |
| Artifacts | `EVD-0013-thresholds.csv`, `evd_0013_oscillator_floor.py`, `crates/pertylizer/examples/evd_0013_equivalence.rs`, `corpus::fixtures::equivalence_probe` |

## Question and falsifier

The Phase 2 exit gate's third bullet asks whether the basic voice render is
**musically equivalent to V1, or the difference is documented and intentional**.
[ADR-0040](../../decisions/ADR-0040-v2-owns-its-dsp.md) clause 4 keeps both
branches, chooses neither in advance, and makes the measurement mandatory:
P02-T008 records **every** difference it finds, with a cause.

This record is that measurement.

### The three dispositions a difference can have

ADR-0040 clause 4 defines them, and this record assigns exactly one to each
difference it finds:

| Disposition | When it applies | What it needs |
|---|---|---|
| **Explained** | The corresponding preserve threshold is **still met** | A cause, traced to named code in both engines |
| **Failure** | A preserve threshold is **exceeded** | A blocking finding, whatever its cause, unless the claim is changed by a recorded decision |
| **Intentional** | The phase wanted the difference, and it is named in advance | A named disposition, not only an explanation of where it came from |

A difference with **no** disposition is a failure. Clause 4's control is that an
unexplained difference is a P02-T008 finding rather than a pass.

**A known cause does not convert a Failure into an Explained.** ADR-0040
clause 4 is explicit that a difference breaking a `preserve` claim "is a
failure — not a documented difference — unless that claim is itself changed by a
recorded decision", and this record has no authority over CORPUS-0001's claims.
The disposition is decided by **whether the threshold is met**, and the cause is
what the record then owes; the cause never decides the disposition.

### What would make the preferred conclusion wrong

The preferred conclusion is that every difference is Explained or Intentional.
It is wrong if **any** acceptance threshold below is exceeded, whether or not the
excess can be traced. Tracing it is what turns a failure into an actionable
finding; it is not what turns it into a pass.

### The acceptance thresholds, stated before collection

CORPUS-0001's claims are prose, and prose is not falsifiable. Each is given a
number here, before any render exists, and each number is justified by the
instrument's own resolution rather than chosen to be comfortable.

| Claim | Corresponds to | Threshold | Why this number |
|---|---|---|---|
| **E1** — fundamental frequency | CORPUS-0001-P1, `exact-parity` | `\|cents\|` ≤ 1.0 | Both arms are told the same frequency by construction; a cent is well inside what `PitchDifference`'s 50 ms window resolves, so anything above it is a tuning path difference rather than an estimator artifact |
| **E2a** — attack and release landmarks | CORPUS-0001-P2, `feature-parity` | Every `EnvelopeDifference::delta_ms` field ≤ 10.0 ms | `ENVELOPE_WINDOW_MS` is 10 ms, so one window **is** the landmark resolution. A threshold below it would measure the instrument's quantisation; one above it would let a real segment-length difference through |
| **E2b** — the decay endpoint and the sustain level | CORPUS-0001-P2, `feature-parity` | Decay endpoint within 10.0 ms; sustain level within 0.1 dB | See *E2 needs two measurements, not one* below |
| **E3a** — the filter's magnitude response | CORPUS-0001-P3, `feature-parity` | `\|delta_db\|` ≤ 1.0 in the fundamental's own band at **every** frequency of the declared sweep | See *E3 needs a sweep, because one sine occupies one band* below |
| **E3b** — whole-render band balance | CORPUS-0001-P3, `feature-parity` | `\|delta_db\|` ≤ 1.0 in every octave band whose **reference** energy is within 60 dB of the reference peak band | The floor is `ENVELOPE_FLOOR_DB`'s −60 dB reused as a coverage rule. Control **C3** decides whether what lies below it is the oscillator or something else. On this material it reaches one band, which is why E3a exists |
| **E4** — onset placement | CORPUS-0001-C1, `intentional-correction` | V2's note edge takes effect at its declared sample; V1's onsets move only in whole blocks of 256 samples | This is the gate's second branch. It is not a tolerance to stay inside but a **prediction to confirm**, and its two halves need two different instruments — see below |

**E3b's floor is a coverage rule, not a claim about audibility.** It says which
bands the threshold is evaluated over. Bands below it are still reported.

#### E3 needs a sweep, because one sine occupies one band

CORPUS-0001-P3 claims that "the static low-pass leaves the same spectral
balance: per-band energy tracks V1's across the whole render". CORPUS-0001 can
make that claim because its source is a **sawtooth**, which puts energy in every
band, so a filter that shaped them differently would show up as a different
per-band profile. This fixture's source is a **sine**, and a sine at 440 Hz puts
essentially all of its energy in one octave band.

This was not reasoned out in advance; **control C3 produced it** before any
render existed, which is the control doing the job it is there for. Running it
on the fixture's own signal:

- the fundamental's band, 315–630 Hz, sits at −7.75 dBFS;
- **every other band is at or below −82 dB**, which is more than 60 dB beneath
  it, so E3b's coverage rule admits exactly **one** band;
- and in the bands above the corner, the residual between the two oscillators is
  very nearly **equal to the reference** — V1's band energy there is essentially
  all of its difference from V2, because V2's `f64` sine leaves almost nothing
  above the fundamental, and the bounds there run to tens of dB.

A spectral-balance threshold that evaluates one band is not a spectral-balance
threshold. **E3a is what makes the claim testable**: rather than asking one
render to fill every band, it asks the filter for its magnitude response at one
frequency per band, one fixture per point.

| Point | MIDI note | Frequency | Fundamental's band | Relative to the 1 kHz corner |
|---|---|---|---|---|
| 1 | 45 | 110 Hz | 80–160 Hz | three octaves below |
| 2 | 57 | 220 Hz | 160–315 Hz | two below |
| 3 | 69 | 440 Hz | 315–630 Hz | one below |
| 4 | 81 | 880 Hz | 630–1 250 Hz | just below it, in the band holding it |
| 5 | 93 | 1 760 Hz | 1 250–2 500 Hz | one above |
| 6 | 105 | 3 520 Hz | 2 500–5 000 Hz | two above |

Each point is compared **in its own fundamental band**, so the six points
together trace the filter's roll-off across six octave bands — which is the
per-band comparison P3 asks for, obtained from a source both engines have.

**The frequencies are octaves of A on purpose.** V1's oscillator takes its
frequency from the note, through `Frequency::from_midi`, which computes
`440.0 * 2.0f32.powf((n - 69) / 12)` in `f32`. At an exact octave the exponent
is an integer, the power is exact, and the result is exactly 110.0, 220.0,
880.0 and so on — so V2's sine can be built with the same literal and the two
arms are at the same frequency by construction rather than to within rounding.
At a non-octave note they would differ in the last places, and E1 would be
measuring `powf`.

#### E2 needs two measurements, not one

`EnvelopeDifference::delta_ms` carries exactly four fields — `peak_ms`,
`rise_to_90_ms`, `fall_to_50_ms` and `tail_end_ms`
(`crates/pertylizer/src/compare/metrics.rs:473`). With sustain at 0.700, the
envelope never falls to half its peak until the **release**, so `fall_to_50_ms`
lands somewhere on the release *curve* rather than on any of P2's four
landmarks — and **nothing reported measures the decay-to-sustain transition or
the sustain level at all**. E2a alone could therefore pass while the decay
segment or the sustain level differed materially, which is two of the four
landmarks CORPUS-0001-P2 names.

**That `fall_to_50_ms` is not a landmark was not noticed until after
collection**, and E2a's threshold was declared over all four of the metric's
fields. The consequence is recorded in *Conclusion* rather than repaired by
narrowing the threshold.

E2b closes that with a direct measurement over the rendered signal, in the
harness rather than in `pertylizer compare`:

Both are computed over the **10 ms RMS envelope** the comparison already uses —
`rms_envelope(mono, sample_rate, ENVELOPE_WINDOW_MS)`,
`crates/pertylizer/src/audio/analysis` — and not over raw samples. Naming the
estimator is not a formality: the render is a filtered sine, so its raw samples
cross every level near zero once per cycle, and "the first sample at which the
level reaches X" is not a measurement until something says what "the level"
means. Reusing the 10 ms envelope also puts E2b on E2a's scale rather than
inventing a second one.

- **The sustain level** is the mean of the envelope windows lying wholly inside
  sustain — from attack plus decay plus one window, to the window before the
  gate falls. Compared as a level ratio in dB. The 0.1 dB threshold is twenty
  times the +0.0029 dB the oscillator approximation contributes (control **C3**)
  and far below the smallest gain-staging error worth catching.
- **The decay endpoint** is the first envelope window after the peak whose level
  is within 0.5% of that sustain level. Its resolution is one window, which is
  why its threshold is 10 ms and not something finer.

Both are computed from the same WAV files `pertylizer compare` reads, by a
committed script, so a reader can recompute them.

#### E4 needs its own instrument, and its direction is the opposite of the obvious one

Two facts, both verified in code before this threshold was written:

- **V1 is early, not late.** `SynthEngine::process` calls
  `self.sequencer.process(sample_count, &mut self.sequencer_event_buffer)` to
  collect every event falling anywhere in the block, and then
  `route_sequencer_events(...)` (`crates/synth_engine/src/synth_engine.rs:4303`)
  delivers all of them to the instruments **before** the voices render that
  block. An event whose true position is sample `s` is therefore applied at the
  start of the block containing it — **early by `s mod 256`**, not late by
  `(-s) mod 256`. That is a statement about routing, given a position; **which**
  sample a given authored tick occupies is a separate question, and E4 is
  written so that it never has to be answered. CORPUS-0001-C1's prose says V1
  "dispatches sequencer events on block boundaries", which is true of both
  directions and does not settle this; the code does.
- **`pertylizer compare` cannot see it.** `onset_ms`
  (`crates/pertylizer/src/compare/metrics.rs:319`) locates the first 10 ms
  envelope window above a threshold, so its resolution is 10 ms. The largest
  displacement V1 can have is 255 samples, which at 44 100 Hz is **5.78 ms** —
  below one window. The offset can never exceed the grid, because both are
  fixed: `BUFFER_SIZE` is 256 and `ENVELOPE_WINDOW_MS` is 10.

Whatever measures E4 must therefore work in the **sample domain**, and must
never compare an index taken in one engine against an index taken in the other.
A first-crossing index does not locate the gate: the signal only becomes
detectable after the envelope has risen and the filter has responded, and that
latency differs between the two engines — which is precisely the difference the
rest of this record is measuring. Compared across arms it would contaminate the
answer.

**The two halves of E4 are established by two different means**, because only
one of them is a question about audio.

| Engine | Claim, checked exactly | Established by |
|---|---|---|
| **V2** | A note edge takes effect at its declared sample | `a_note_on_takes_effect_at_its_declared_sample` and its note-off counterpart in `crates/synth_engine_v2/tests/note_events.rs` — P02-T007's conformance check |
| **V1** | Over authored ticks spanning more than one block, every onset difference is a **multiple of 256**, and distinct ticks inside one block share an onset | This record's sample-domain measurement, differenced within V1's own family |

V2's half is the correction the phase claims; V1's half is the behaviour being
corrected. Neither requires the two engines to be asked for the same position,
which is what makes them checkable at all.

**V2's half is cited, not re-derived, and an onset detector could not derive
it.** `PROCESS.md` asks for "a named automated test or reproducible EVD", and the
test asserts the property on the renderer's output at the sample, with no
detector in the way: it gates a **constant** rather than an oscillator, so the
rendered signal is exactly 1.0 while held and exactly 0.0 otherwise, every frame
is asserted, and the first non-zero frame is asserted to be the declared sample —
one deliberately not a multiple of `Q`. What it establishes is the *renderer's*
event placement, which is a property of how events are applied and not of the
graph they are applied to, so it carries to this fixture. What it does not do is
observe this fixture's own audio, and the paragraph below is why that is a
feature rather than a gap. That is also the only instrument that can: V2's `Sine` is a
free-running phase accumulator started at plan sample 0 and is **not** reset by
the note edge (`crates/synth_engine_v2/src/node/kernels.rs:498`), so moving the
gate to a different sample changes the oscillator's phase at the gate, and with
it the delay from the gate to the first sample crossing any threshold. A
within-engine difference of first-crossing indices would have measured that
phase and called it scheduling. The same objection sinks a peak-relative
threshold, which is identical across two renders only if their peaks are — and
nothing had established that.

**V1's half is a question about audio, and there the detector holds**, because
V1's oscillator *is* reset by the note: `Oscillator::note_on` re-seeds every
unison phase, and with `uni_phase` at 0.0 — asymmetry 3 — the seed is
`Phase::ZERO` (`crates/synth_modules/src/oscillator.rs:843`). Every note in V1's
family starts from the same phase, so the delay from gate to first crossing is
one constant that cancels in any difference between two of those renders, and
their peaks are equal because the renders differ only in where an identical note
sits. The harness reports, for each authored tick `t`, the first sample index
`x(t)` at which the render's absolute value exceeds 10⁻⁴ of that render's own
later peak, and E4 checks that every `x(t) - x(0)` is a multiple of 256 and that
distinct ticks inside one block share an `x`.

**No formula, and deliberately so.** An earlier draft predicted V1's displacement
as `s - (s mod 256)` for a note at sample `s`. But a note is authored at a
**tick**, and V1's tick-to-sample mapping is not `tick × samples_per_tick`:
`process_until_next_tick` (`crates/synth_engine/src/sequencer_engine.rs:573`)
advances a `tick_accumulator` in chunks rounded up to the next tick boundary and
collects a tick's events when the accumulator crosses it, so a formula written
against the nominal product can be wrong by a whole block — most easily at
exactly the offset that draft was choosing to maximise. Distinct authored
positions collapsing onto one onset is the observable the formula stood in for,
it cannot be wrong about the accumulator because it does not model it, and the
reported onsets trace the real mapping for a reader who wants it.

Both V1 checks hold only for a render that **begins at tick 0**, which is a
condition on the fixture and is stated as one: starting there makes
`earliest_active_note_start` coincide with the range start, so the pre-roll is
zero (`crates/pertylizer/src/audio/arrangement_render.rs:509`) and the callback
origin is fixed. Every `offset-<t>` fixture renders from tick 0.

## Inputs and controls

### The fixture, and why it is not CORPUS-0001 itself

CORPUS-0001 is a **sawtooth at four pitches**. V2 has no sawtooth among its six
node kinds, and P02-T007 deliberately gave `NoteEdge` neither pitch nor
velocity, because nothing in Phase 2 reads either. Rendering CORPUS-0001 in V2
is therefore not merely inconvenient; it is not expressible.

What "the equivalent minimal patch" means is defined **here, before any
comparison exists**, because defining it after seeing a comparison would be
choosing the fixture to fit the answer:

> The equivalent minimal patch is the V1 patch that mirrors V2's `voice-mono`
> graph node for node — sine into a two-pole low-pass into an amplifier gated by
> a four-segment envelope — at parameters chosen so that every quantity **both**
> engines have is equal, and rendered as one note whose pitch is the frequency
> V2's sine is built with.

It corresponds to CORPUS-0001 and inherits its **claim classes**, which is what
the exit gate's wording asks for. It does not become a manifest case: a manifest
entry pins V1 behaviour, that is P00A-T001's to decide, and a fixture authored
inside an implementation task has no business acquiring that authority.
`corpus::fixtures::polyphony_probe` is the precedent and says so at its
declaration — a generated probe that pins nothing, absent from `FIXTURES`, so it
is never written into the corpus directory or digested by the manifest test.

Three fixture families, each differing from `aligned` in exactly one thing:

| Fixture | Differs by | What it is for |
|---|---|---|
| **`aligned`** | — the reference fixture: MIDI 69, 440 Hz, note at plan sample 0 | E1, E2a, E2b and E3b. Sample 0 is a `BUFFER_SIZE` boundary in V1, so the onset difference E4 measures is **absent by construction** and cannot contaminate the DSP comparison |
| **`offset-<t>`** | The note's authored tick, over a family spanning more than one block | E4's V1 half alone. Every one renders from tick 0. V2's half needs no fixture — it is a named crate test |
| **`sweep-<n>`** | The note's pitch, one per E3a point | E3a. Six fixtures at MIDI 45, 57, 69, 81, 93 and 105; `sweep-69` is `aligned`, so the family is five additional renders rather than six |

### The arms

Both arms render **stereo at 44 100 Hz**. The rate is the corpus render rate,
and it matters because the filter's coefficients are a function of the stream's
sample rate in both engines. The channel count is not a detail either, and why
it is stereo rather than mono is asymmetry 5 below.

V2's graph is [ADR-0041](../../decisions/ADR-0041-interleaved-internal-channel-layout.md)
clause 16's first baseline fixture, which is also EVD-0012's governing shape and
`crates/synth_engine_v2/examples/quantum_cost.rs`'s `voice()`:

| Node | Parameters |
|---|---|
| `Envelope` | attack 0.010 s, decay 0.100 s, sustain 0.700, release 0.200 s |
| `Sine` | frequency 440.0 Hz, amplitude **0.5** |
| `Filter` | cutoff 1 000.0 Hz, resonance `Resonance::BUTTERWORTH` |
| `Amplifier` | control from the envelope, audio from the filter |
| `Output` | **stereo** profile; the mono source is written to both channels |

Rendered through `render_offline`. V1's arm is the same five stages, with
`ModuleType::Oscillator` at `waveform("sine")` in place of
`add_saw_into_filter`'s sawtooth, rendered with
`pertylizer render --sample-rate 44100 --bit-depth 32f`, the shipped offline
path the corpus digests are taken through.

### The five asymmetries closed before collection

Five quantities exist on the V1 side and would otherwise be read as engine
differences. Each is neutralised **in the fixture**, not in the analysis:

1. **The chain applies centre pan twice.** `Amplifier::process` writes
   `PortName::OUT` as `(left + right) * 0.5` with `Gain::from_pan` at centre, so
   the mono port carries `cos(π/4)` = 0.7071; the patch's connection is that
   mono port, and `StereoOutput` then applies `Gain::from_pan` again per
   channel. The chain gain is therefore **0.49999997** — `0x3effffff`, the `f32`
   product of the two rounded pan coefficients, not exactly one half — or
   −6.0206 dB, and it has nothing to do with either engine's DSP. V2's sine
   amplitude of 0.5 against V1's oscillator level of 1.0 is what cancels it to
   within 5 × 10⁻⁸, and control **C2** is what checks the cancellation rather
   than trusting this derivation.
2. **Velocity scales the envelope.** `velocity_sensitivity(v, s) = 1 - s(1 - v)`
   at the corpus instrument's `velocity_amp_sensitivity: MAX` reduces to `v`,
   and `Velocity::F` is 96/127 = 0.7559. The fixture uses `Velocity::FFF`, which
   is exactly 1.0. Setting the sensitivity to `MIN` would do the same thing and
   is not used: a velocity of full scale is the honest way to say "this
   comparison has no velocity in it", and it leaves the shipped default in place.
3. **The oscillator randomises its start phase.** `uni_phase` defaults to full
   randomisation seeded from the voice index, so the phase a note starts at is a
   function of which voice the allocator handed it.
   `silence_phase_randomization` sets it to 0.0, exactly as every corpus fixture
   authored since it existed does.
4. **The resonance parameters are not the same quantity.** V1's filter takes a
   normalised resonance and forms `k = 2 - 2·res`; V2's takes a quality factor
   and forms `damping = 1/Q`. They are the same coefficient under two names, so
   the fixture uses the `res` that lands on V2's value **in `f32`**:

   `Resonance::BUTTERWORTH` is `FRAC_1_SQRT_2` rounded to `f32`, which is
   0.707106769084930…; its reciprocal rounded to `f32` is 1.414213538169861…,
   and `res` = 0.2928932309150696 reproduces that `k` **exactly** under V1's own
   `f32` arithmetic. This is verified arithmetic, not a value chosen by eye, and
   it is the reason CORPUS-0001's 0.30 is not reused: 0.30 is a different
   filter, by about 0.4% in `k`.
5. **The channel layouts are not the same.** V1's patch terminates in
   `StereoOutput`, which pans, limits, meters and writes two channels; V2's
   `Output` writes a mono source to however many channels the profile declares.
   Comparing a mono V2 render against a stereo V1 one would leave
   `SampleDifference` unavailable — it needs matching channel counts — and would
   let every other metric silently downmix, which is a difference the record
   would then never see. **V2 therefore renders at the stereo profile**, so both
   arms are two-channel and the per-channel gain identity in asymmetry 1 holds
   channel for channel: V1's chain applies 0.7071 twice per channel, and V2's
   duplication applies nothing, which is what the 0.5 sine amplitude cancels.

   What this does **not** neutralise is the *work* V1's output stage does and
   V2's does not — limiting, metering, interleaving. That is invisible here,
   because it does not change the samples on this fixture at unity master with
   no limiting engaged, but it is not invisible to
   [EVD-0014](EVD-0014-minimal-patch-cpu.md), which names it as a cost.

### Two differences that are **not** closed, and are the subject

These are the measurement, and neutralising them would be measuring nothing:

- **The oscillators are different functions, and they accumulate phase
  differently.** V1's `Waveform::Sine` is `fast_sin_turns`
  (`crates/synth_modules/src/math.rs:390`), a parabolic approximation with a
  correction term, driven by an `f32` phase advanced by an `f32` increment;
  V2's is `f64::sin` (`crates/synth_engine_v2/src/node/kernels.rs:519`) over an
  `f64` accumulator. **Both** the waveform and the accumulator differ, and
  control **C3** bounds both rather than only the first.
- **The envelopes are different shapes.** V1's segments are exponential with an
  overshoot target chosen so the curve *crosses* its endpoint at the authored
  time (`crates/synth_modules/src/envelope.rs:291`); V2's are linear ramps of an
  exact frame count. The **landmarks** should therefore agree while the shape
  between them does not, and E2 is a threshold on the landmarks precisely
  because that is the claim CORPUS-0001-P2 makes.

The filter is the third stage and is **not** in this list. Both engines run the
same topology-preserving state-variable form with the same recurrence and the
same coefficient expressions — `crates/synth_dsp/src/filters.rs:48` against
`crates/synth_engine_v2/src/node.rs:326`. Two things separate them, and both are
named rather than folded into "precision":

- **Arithmetic precision.** V1 forms its coefficients in `f32`, V2 in `f64`
  before rounding to `f32`.
- **Denormal handling, which is not the same mechanism in the two engines.**
  V2 flushes its filter state below 10⁻³⁰ once per quantum, in software
  (`crates/synth_engine_v2/src/node/kernels.rs:787`). V1's recurrence has no
  such step; instead `render_range_with_tail` installs a `DenormalGuard`
  (`crates/pertylizer/src/audio/arrangement_render.rs:505`) so the whole offline
  render runs with hardware FTZ/DAZ, flushing at the denormal boundary near
  10⁻³⁸. Both flush; at different thresholds and by different means, roughly
  −600 dB and −760 dB respectively.

That is a structural reading of four files rather than a measurement, and it is
stated here so the *prediction* is on record before the data: the filter should
contribute nothing above the last few units in the last place within the band
E3a and E3b evaluate, and if it contributes more, something in this reading is
wrong.

### The controls, run before the measurements they guard

The evidence README's rule is that a number produced without a control is an
artifact until proven otherwise, and that a control that fails is telling you
about your fixture. Three, in order:

- **C1 — determinism.** Each arm is rendered **twice** and compared against
  itself. `SampleDifference` must report exact equality. A fixture that does not
  reproduce makes every figure downstream of it meaningless, and EVD-0001
  already establishes that V1's corpus renders are bit-exact, so this checks
  that the fixture inherited that rather than making a new claim.
- **C2 — the gain null.** A reduced pair in which every intended difference is
  removed: the envelope's gate held long enough to reach sustain, the comparison
  window taken **inside** sustain, and the filter's corner placed above the band
  the fundamental occupies so the stage is near-transparent to it. The measured
  `LevelDifference` must be **0.00 dB within 0.05 dB** once the declared chain
  gain is accounted for. This catches a mis-derived gain staging, which would
  otherwise appear as a uniform spectral offset and be read as a filter
  difference.

  **0.05 dB is not zero, and the reason is stated rather than absorbed.** Two
  known contributions sit under it: the oscillator approximation's fundamental
  amplitude of 1.000333, which is +0.0029 dB, and the chain gain's departure
  from one half, which is −4 × 10⁻⁷ dB. The tolerance is more than an order of
  magnitude above their sum and far below the smallest gain-staging error worth
  catching, a factor of 0.7071 being 3 dB.
- **C3 — the oscillator's difference floor.** The difference between the two
  oscillators can be bounded **without rendering either engine**, and it is
  bounded before collection so E3a's and E3b's band differences can be attributed
  rather than merely observed. `evd_0013_oscillator_floor.py` does it, and what it
  computes is fixed here because the review of an earlier draft found the first
  version bounding the wrong quantity:

  1. It reproduces `fast_sin_turns` in `f32` at every intermediate step, and it
     drives it from **V1's own phase accumulator** — an `f32` phase advanced by
     the `f32` increment for 440 Hz at 44 100 Hz and wrapped as `Phase` wraps it
     — over the fixture's whole length, rather than over one exact mathematical
     cycle. The accumulator is a second difference from V2's `f64` one, and a
     script that sampled a perfect cycle would omit it entirely.
  2. It compares that against the `f64` accumulator and `f64::sin` V2 uses, so
     the residual it reports is the **whole** oscillator difference: waveform
     approximation, phase drift, and the `f32` rounding of the output together.
  3. It integrates that residual's power into the **same octave bands**
     `SpectrumDifference` uses, rather than reporting per-harmonic figures. A
     band holds more than one harmonic, plus aliased images and whatever
     sidebands the drift produces, and a per-harmonic figure does not bound a
     band that contains several of them.
  4. It runs both signals through **the recurrence itself**, with V1's `f32`
     coefficients, rather than through an analogue prototype's magnitude
     response — so the attenuation it credits is the attenuation the render
     actually applies, and the residual it measures is the one the filter
     actually passes.
  5. It **gates both arms with the same envelope**, applied after the filter,
     where the amplifier sits. This is not decoration: multiplying by a
     time-varying envelope redistributes energy between octave bands, so an
     ungated residual is not a bound on the gated render E3a and E3b measure.
     Holding **one** envelope across both arms is what keeps the residual the
     oscillator's — the two engines' envelope *shapes* differ, and that
     difference is E2's subject and a separate cause.

  What the bound therefore does **not** cover is the difference between the two
  envelope shapes. A band difference exceeding it means "not the oscillator",
  not "no cause".

  Its output is the per-band bound E3a and E3b attribute to the oscillator. Run
  at the
  source revision on the fixture's own frequency, the parts that decide the
  thresholds are:

  | Quantity | Value |
  |---|---|
  | Maximum residual before the filter | 4.799 × 10⁻³ |
  | Maximum residual after the filter and the gate | 1.676 × 10⁻³ |
  | Bound in the fundamental's band, 315–630 Hz | **0.013 dB** |
  | Bound in every band below the fundamental's | ≤ 0.017 dB |
  | Bands at and above 1 250 Hz | 1.8 to 52.7 dB — the residual is the whole band |

  Three things follow, and all three are load-bearing:

  - **The accumulator is most of the difference.** 4.799 × 10⁻³ against the
    1.090 × 10⁻³ the waveform function alone contributes pointwise: the `f32`
    phase drift is roughly four times the approximation error, and a control
    that sampled one exact cycle would have reported the smaller number as the
    whole of it.
  - **The fundamental's band is comfortably inside E3a and E3b.** 0.013 dB
    against a 1.0 dB threshold, so a measured difference there is not the
    oscillator.
  - **Above the corner the oscillator is essentially the entire band.** V2's
    `f64` sine leaves almost nothing above the fundamental, so V1's band energy
    there is very nearly its whole difference from V2, and the bound is tens of
    dB. Those bands sit more than 60 dB below the peak and E3b's coverage rule
    excludes them — which is the fact that made E3a necessary.

## Method

1. Build the two V1 fixtures as `.ptz` projects through the same
   `pertylizer::corpus::fixtures` helpers the corpus uses, so the patch is
   authored the way a corpus fixture is authored rather than by hand, and
   outside `FIXTURES` for the reason `polyphony_probe` is.
2. Run **C1** on both arms. Stop if it fails.
3. Run **C2**. Stop if it fails; a failure here is a fixture defect and nothing
   after it can be interpreted.
4. Run **C3**, which needs no render.
5. Render `aligned` in both engines and run `pertylizer compare` with V1 as
   `--reference` and V2 as `--candidate`. Evaluate E1, E2a and E3b from the
   report, and E2b from the same WAV files by the committed script.
6. Render the five remaining `sweep-<n>` fixtures in both engines, compare each,
   and evaluate E3a in each point's own fundamental band. Rerun **C3** at each
   frequency, since the oscillator's bound is a function of it.
7. Render the `offset-<t>` family in **V1** and evaluate E4's V1 half in the
   sample domain. E4's V2 half needs no render: it is the crate test cited
   above, run by `cargo test -p synth_engine_v2`.
8. Assign every difference the reports contain exactly one disposition from the
   table above, with its cause named in code.

The comparison direction is fixed here and not chosen later: **V1 is the
reference**, because the claim classes are claims about preserving V1's
behaviour, and `delta` fields in the report are therefore V2 minus V1.

`pertylizer compare` has no tolerance flag and no pass/fail by design. Every
threshold in this record is applied to its output by a committed analysis
script, and the report is retained so a reader can apply a different threshold
to the same numbers.

## Reproduction

Every command from the repository root. `$DIR` is any empty directory.

```text
cargo build --release -p pertylizer --bin pertylizer
cargo build --release -p pertylizer --example evd_0013_equivalence
# The binary just built, not whatever is on PATH: the source revision this
# record states is only the one that produced these paths.
BIN=./target/release/pertylizer

# The V1 projects, built through the corpus helpers and outside `FIXTURES`.
cargo run --release -q -p pertylizer --example evd_0013_equivalence -- fixtures $DIR

# V1's arm, through the shipped offline path.
render() { "$BIN" render --input "$1" --output "$2" \
    --sample-rate 44100 --bit-depth 32f --seconds 2.0 --tail-seconds 1.0; }
for f in $DIR/v1/*.ptz; do n=$(basename "$f" .ptz)
  render "$f" "$DIR/wav/v1-$n.wav"
done
# Control C1 needs a *second* render of the two control arms. Without these two
# lines the `measure` step below has nothing to compare an arm against.
for n in aligned null; do render "$DIR/v1/$n.ptz" "$DIR/wav/v1-$n-b.wav"; done

# V2's arm, which writes its own `-b` repeats for the same two arms.
cargo run --release -q -p pertylizer --example evd_0013_equivalence -- v2 $DIR

# C1, C2, E2b, E4's V1 half, and the region attribution.
cargo run --release -q -p pertylizer --example evd_0013_equivalence -- measure $DIR

# C3, which renders nothing. Once per sweep point.
for hz in 110 220 440 880 1760 3520; do
  python3 plans/v2/evidence/phase-02/evd_0013_oscillator_floor.py $hz
done

# E1, E2a, E3a and E3b.
for n in aligned sweep-45 sweep-57 sweep-81 sweep-93 sweep-105; do
  "$BIN" compare --reference "$DIR/wav/v1-$n.wav" \
    --candidate "$DIR/wav/v2-$n.wav" --result-json "$DIR/wav/cmp-$n.json"
done

# E4's V2 half is a named crate test, not a render.
cargo test -p synth_engine_v2 --release --test note_events
```

## Results

The compact per-threshold table is `EVD-0013-thresholds.csv`.

### The controls, and the one that failed

**C1 passes.** Both engines' `aligned` and `null` arms are bit-identical to their
own second render, over all 132 300 frames.

**C2 failed on its first run, and the failure was the fixture.** It came back at
**+3.008 dB** — within 0.4% of one equal-power pan. The record's asymmetry 1 had
derived the V1 chain gain as **two** centre pans, the amplifier's and the stereo
output's, giving 0.49999997. There is a **third**: `SynthEngine::process` applies
the instrument fader's own `Gain::from_pan`
(`crates/synth_engine/src/synth_engine.rs:3835`), outside the voice's module
graph entirely. The `f32` product of the three is **0.353 553 354 740 142 8**,
`0x3eb504f2`.

The prediction and the measurement agree to 1.3 × 10⁻⁵: three pans give a
V1/V2 ratio of 0.707107, the oscillator approximation's fundamental gain of
1.000333 raises it to 0.707342, and the measurement reads **0.707332**.

With V2's sine amplitude set to the corrected chain gain, **C2 passes at
−0.00276 dB** — and the residual is the oscillator's +0.0029 dB and nothing
else, which is the quantity C3 predicted before any render existed.

**C3's bounds, per sweep point**, in the fundamental's own band:

| Point | Reference | Residual | Bound |
|---|---|---|---|
| 110 Hz | −7.59 dB | −66.67 dB | 0.0097 dB |
| 220 Hz | −7.60 dB | −62.69 dB | 0.0153 dB |
| 440 Hz | −7.75 dB | −64.04 dB | 0.0133 dB |
| 880 Hz | −9.63 dB | −57.42 dB | 0.0355 dB |
| 1 760 Hz | −17.90 dB | −76.03 dB | 0.0108 dB |
| 3 520 Hz | −29.82 dB | −90.10 dB | 0.0084 dB |

### The thresholds

| Threshold | Result | Limit | Outcome |
|---|---|---|---|
| **E1** — fundamental frequency | worst 0.0036 cents over six points | 1.0 cent | **Pass** |
| **E2a** — attack and release landmarks | `rise_to_90_ms` at 0 everywhere; `peak_ms` at 0 except +10 at the 110 Hz point; `tail_end_ms` at +10; **`fall_to_50_ms` at +20 at four of six points** | 10.0 ms | **Exceeded** |
| **E2b** — decay endpoint and sustain level | sustain −0.00274 dB; decay endpoint +0.0 ms | 0.1 dB / 10.0 ms | **Pass** |
| **E3a** — the filter's magnitude response | **+0.068 dB at every one of the six points** | 1.0 dB | **Pass** |
| **E3b** — whole-render band balance | +0.068 dB, in the **one** band the coverage rule reaches | 1.0 dB | **Pass** |
| **E4** — onset placement | both halves confirmed; see below | exact | **Pass** |

**E3b reaches exactly one band, which C3 predicted before any render existed.**
The fundamental's band sits at −18.04 dBFS and every other band at or below
−93.88 dB, so the −60 dB coverage rule admits one. That is the deficiency E3a
was added for, and E3a is what makes the claim testable.

**E3a's constancy is the result, not its size.** The filter's magnitude response
agrees to +0.068 dB across six octave bands, from three octaves below the corner
to two above — which is what CORPUS-0001-P3 asks for and what the two engines
running the same recurrence with the same coefficients predicts.

**E4 passes on both halves.** V2's is P02-T007's conformance check, which passes
along with the six other cases in `tests/note_events.rs`. V1's is a step function
with three risers over 25 authored ticks, every difference an exact multiple of
256:

| Authored ticks | Onset frame | Difference from tick 0 |
|---|---|---|
| 0–10 | 4 | 0 |
| 11–21 | 260 | 256 |
| 22–24 | 516 | 512 |

Eleven distinct authored positions collapsing onto one onset is the observable
the earlier drafts' formula was standing in for, and it is exactly what
CORPUS-0001-C1 describes.

### Where the differences are, and what each one is

Every difference the comparison found, attributed. The attribution is a
measurement rather than an argument: the render was split into its segments and
each compared on its own. It is the `measure` subcommand's last section, so a
reader regenerates it with the same command that produces the controls rather
than by hand.

| Region | V2 against V1 | Cause |
|---|---|---|
| Sustain | **−0.0029 dB** | The oscillator approximation's fundamental gain, and nothing else. C3 predicted +0.0029 dB before collection |
| Attack and decay | +0.186 dB | The segment shapes: V1 exponential with overshoot targeting, V2 linear |
| Release, shared window | +1.333 dB | The curve **and** the scheduling difference together — V1's note-off lands 102 frames early, so this window compares V1's release already under way against V2's at its start |
| **Release, each from its own gate** | **+1.137 dB** | The curve alone: a linear release holds more energy than an exponential one. The remaining 0.196 dB is E4's subject, not the envelope's |
| Whole render | +0.063 dB | The regions above, weighted by their share of the window |

**The +0.068 dB E3a and E3b measure is the envelope, not the oscillator and not
the filter.** Three things say so together: it exceeds C3's oscillator bound at
every point (0.0084 to 0.0355 dB); it is **frequency-independent**, the same
+0.068 dB in all six bands across six octaves, which no filter difference can
be; and the region split puts it in the release, where V2 carries 1.33 dB more
energy. A broadband gain applied by a shared envelope is the only shape that
fits all three.

**The release figure is quoted from the own-gate row, not the shared window.**
Comparing both arms over one window starting at the nominal note-off would fold
E4's 102-frame scheduling difference into a number this record then attributes to
the envelope — 0.196 dB of it, as the two rows show. The harness reports both so
the separation is visible rather than asserted.

The bands above the corner show V1 with energy V2 does not have — −94.41 against
−116.22 dB at 1 250 Hz — which is `fast_sin_turns`'s harmonics, exactly as C3
described. Every one of those bands is more than 60 dB below the peak and
outside E3b's coverage rule.

## History

**The two records' methods were reviewed together before any data existed, and
nine blocking findings came back.** Five of them landed here; the other four are
recorded in [EVD-0014](EVD-0014-minimal-patch-cpu.md)'s *History*. They are recorded because a method a reader cannot check against
its own corrections is not a method, and because two of them changed what this
record measures rather than how it says it.

The five that changed this record:

- **E4 predicted the wrong direction and could not have been measured.** The
  draft said V1's onset was *late* by `(-s) mod 256`; the code routes a block's
  events before that block's voices render, so it is *early* by `s mod 256`. And
  the instrument it named quantises onsets to 10 ms windows while the largest
  effect available is 5.78 ms. Both are now stated, and E4 has a sample-domain
  instrument of its own.
- **E2 did not measure two of the four landmarks it claimed.** With sustain at
  0.700, `fall_to_50_ms` lands in the release, and nothing reported measured the
  decay endpoint or the sustain level. E2b was added for them.
- **C3 bounded the wrong quantity.** It evaluated `fast_sin_turns` over one
  exact mathematical cycle, which omits V1's `f32` phase accumulator entirely,
  and it reported per-harmonic figures against an analogue filter response —
  neither of which bounds an octave band containing several harmonics, their
  aliases and the drift's sidebands. The four points under **C3** are the
  rewrite.
- **A fifth asymmetry was missing: the channel layout.** V2 rendered mono
  against a stereo V1, which would have disabled `SampleDifference` and let
  every other metric downmix silently. V2 now renders stereo.
- **The falsifier contradicted the disposition table.** One said a threshold
  breach with a known cause was Explained; the other said it was a Failure. The
  table is what ADR-0040 clause 4 says, and the falsifier now matches it.

Two non-blocking corrections were also taken: the chain gain is 0.49999997
rather than exactly 0.5, and the filters are separated by denormal handling as
well as by precision.

**Then the rewritten control found a sixth defect the review had not.** With C3
fixed and run — still before any render existed — its own output showed that the
fixture's spectrum has **one** band inside E3b's coverage rule and that every
above the corner is 100% oscillator residual. A spectral-balance threshold
evaluating one band would have closed the gate's third bullet on almost nothing,
and the deficiency was a property of the fixture rather than of the engines:
CORPUS-0001 uses a sawtooth precisely because a sawtooth fills the bands, and a
sine does not. **E3a**, the six-point frequency sweep, is the repair, and this is
what the evidence README means by a control telling you about your fixture.

**A focused re-read of the repairs returned three more, all on this record's
measurement definitions, and all taken.** They are why E2b, E4 and C3 read as
they do:

- **E2b said "the envelope" while the method retained only samples.** A filtered
  sine crosses every level near zero once per cycle, so "the first sample at
  which the level reaches X" was not yet a measurement. Both of E2b's quantities
  are now defined over the 10 ms RMS envelope `pertylizer compare` already uses.
- **E4's detector could not establish what E4 claims.** A first-crossing index
  lags the gate by the envelope's rise and the filter's response, differently in
  each engine — so comparing the two arms' indices would have measured the DSP
  difference and called it a scheduling one. E4 now differences **within** each
  engine, across renders of the same fixture, where that latency cancels
  exactly.
- **C3 bounded an ungated residual against a gated measurement.** The amplifier
  sits after the filter, and multiplying by a time-varying envelope moves
  residual energy between bands. Both arms now carry the same gate.

One non-blocking correction came with them and is taken: the script's Hann
window was the periodic form, and `crates/synth_dsp/src/spectral.rs:38` uses the
symmetric one.

**E4 then failed two further passes, and both failures are why it now reads as
it does.** It is the only threshold in this record that took four attempts, and
each attempt failed for a different reason:

1. The original predicted V1 was *late*. It is early: a block's events are routed
   before that block's voices render.
2. The second predicted V1's displacement as `s - (s mod 256)` from a note's
   nominal sample position. But a note is authored at a **tick**, and
   `process_until_next_tick` advances a `tick_accumulator` in chunks rounded up
   to the next tick boundary rather than mapping a tick to
   `tick × samples_per_tick` — so the formula could be wrong by a whole block,
   most easily at the very offset the fixture was choosing to maximise. E4 no
   longer contains a formula.
3. The third moved V2's half to a within-engine difference of first-crossing
   indices, on the argument that a constant DSP latency cancels. **It does not
   cancel in V2**, whose `Sine` free-runs from plan sample 0 and is not reset by
   the note edge: moving the gate changes the oscillator's phase at the gate and
   therefore the delay to the first crossing. The same objection sinks a
   peak-relative threshold across renders whose peaks were never shown equal.

The resolution is that V2's half was never an audio question. P02-T007 already
proved it with a named automated test, and E4 cites that test. V1's half stays
in the sample domain, where the detector *does* hold, because V1's oscillator
**is** reset by the note. Both halves are now swept, which also retired this
record's "confirmed at one offset" limitation.

**One finding was checked and not accepted.** The review held that the V1 arm of
EVD-0014's narrow pair would digest silence, because `StereoOutput::process`
populates only `OUT_L` and `OUT_R`. It does not: the module declares an `out`
audio output (`crates/synth_modules/src/output.rs:155`) and `process` writes it
as the mono downmix of the two channels (`:333`). The same finding's other half
was correct and is taken — the type is `ModuleGraph`, not `VoiceGraph` — and the
deeper point it made about that control is taken in EVD-0014.

### What collection itself found

**Control C2 failed on its first run, and it was right to.** The record's
asymmetry 1 derived V1's chain gain as two equal-power centre pans and set V2's
sine amplitude to 0.5; the gain null came back at +3.008 dB, within 0.4% of one
pan. There is a **third** pan, applied by the instrument fader in
`SynthEngine::process` and therefore outside the voice's module graph
altogether. The derivation, the code and the measurement now agree to
1.3 × 10⁻⁵, and asymmetry 1 says three rather than two.

Nothing downstream of C2 was read until it passed, which is the whole reason the
control runs first. Had it been skipped, a 3 dB level error would have sat under
every spectral figure in this record and would have looked like a filter
difference.

**And the third pan changed EVD-0014 as well as this record.** Two of the three
pans are inside the voice's module graph and the fader's is not, so that
record's `voice-dsp` pair cancels two and its `whole-render` pair cancels three
— two amplitudes, not one.

### What the repository review found after collection

Three findings landed on this record, all repaired without recollecting:

- **The reproduction did not produce the renders control C1 reads.** Its loop
  rendered each project once, while C1 compares an arm against a second render
  into `-b.wav` — so the documented `measure` step would have failed on a clean
  directory. The collection had those renders; the *recipe* did not.
- **The region attribution had no retained artifact.** Its +0.186 dB and
  +1.333 dB are verified premises of ADR-0042, and they were computed by hand.
  They are now the `measure` subcommand's last section.
- **`peak_ms` is not at zero everywhere**, as an earlier draft of the results
  said: the retained CSV has it at one window at the 110 Hz sweep point. The
  same claim had been copied into ADR-0042 and is corrected there too.

A second pass, on the repairs, found two more:

- **The release attribution folded in the scheduling difference.** V1 applies a
  note-off at the start of the block containing it, so a shared window compared
  V1's release 102 frames in against V2's at its start. The harness now reports
  the release from **each engine's own gate** as well, and the difference this
  record attributes to the envelope fell from 1.333 dB to **1.137 dB**. ADR-0042
  quotes the own-gate figure.
- **This record misread the claim it was operationalising.** An earlier
  conclusion, and an earlier draft of ADR-0042, said `CORPUS-0001-P2` would be
  *narrowed* to claim landmarks. It already claims landmarks, and always has. So
  no `preserve` claim is broken, ADR-0040 clause 4's failure branch does not
  apply, and what E2a's exceedance actually shows is that E2a was a stricter test
  than P2 asks for. The threshold is unchanged; the reading of what it was
  measuring against is corrected.

A third pass caught the correction going one step too far. Having established
that no `preserve` claim was broken, this record briefly concluded
**`Supported`** — which contradicts its own falsifier, since that falsifier
turns on a *declared threshold* being exceeded and E2a still is. The conclusion
is `Not supported`, and the fact that no CORPUS-0001 claim is broken sits beside
it rather than replacing it. The distinction is the whole difference between
correcting a false premise and explaining a result away, and it took an outside
reader to see that the first had turned into the second.

## Limitations

Stated in advance, because a limitation discovered after the data is a
limitation that had a chance to be chosen:

- **One patch, one note, one pitch.** The fixture is the smallest graph both
  engines can express. It says nothing about a graph with two oscillators,
  modulation, or polyphony, and V2 has none of those.
- **No velocity, no pitch on the note.** `NoteEdge` carries neither, so the
  fixture works around their absence by fixing both. Where they live is
  [`NOW.md`](../../NOW.md)'s open question for Phase 3, and this record does not
  answer it.
- **E4 establishes V1's quantisation, not its mapping.** It shows that V1's
  onsets fall on block boundaries and that distinct authored positions collapse
  onto one, which is what CORPUS-0001-C1 claims. It deliberately does not model
  `tick_accumulator`'s arithmetic, so it says nothing about *which* block a
  given tick lands in.
- **`pertylizer compare`'s metrics are the instrument for E1, E2a, E3a and
  E3b.** Landmark resolution is 10 ms, pitch resolution is one 50 ms window, and
  the bands are octave-wide. A difference finer than those is invisible to them
  whatever its cause, which is why E2b and E4 are measured outside it.
- **E3a traces the filter's response at six points, not continuously.** Six
  octave-spaced tones say where the roll-off is at those six frequencies. A
  difference confined between two of them — a ripple, a notch — is not visible,
  and neither engine's filter is expected to have one, which is a prediction
  rather than a measurement.
- **The two engines' sines are the only source either can produce**, so nothing
  here tests the filter against broadband material at all. That is the
  difference between this fixture and CORPUS-0001, and it is the price of a
  vocabulary of six node kinds.
- **C3 is a model, not a render.** It bounds the oscillator difference by
  reproducing both accumulators and both waveform functions outside either
  engine. If either reproduction is wrong the bound is wrong, and the only
  defence offered is that it is committed, short, and reruns without the
  engines.
- **Nothing here says V2 sounds better.** ADR-0040 is explicit that "it sounds
  the same as V1" is no longer available as evidence of correctness, and the
  converse is equally unavailable: this record measures a gap and attributes it,
  and each kernel's own checks are what argue it is right.

## Conclusion

**Not supported.** This record's falsifier says the preferred conclusion is
wrong if **any** declared threshold is exceeded, whatever the cause. **E2a is
exceeded**, so it is wrong, and no later finding retracts that.

Two further facts sit beside it, and neither erases it:

- **No CORPUS-0001 claim is broken.** `CORPUS-0001-P2` claims landmark parity
  and V2 delivers it, so ADR-0040 clause 4's *failure* branch does not apply and
  the Phase 2 gate's third bullet is not blocked by this record.
- **E2a was a stricter test than the claim it operationalised**, because one of
  the four fields it bounded is not a landmark. That is a defect in **this
  record**, found after collection, and it is recorded rather than repaired by
  moving the number.

Five of the six thresholds pass, four by one to two orders of magnitude.

### The dispositions

| Difference | Size | Disposition |
|---|---|---|
| Fundamental frequency | ≤ 0.0036 cents | **Explained** — both arms are told the same frequency, and the six sweep points are exact octaves of A so `Frequency::from_midi` is exact |
| Sustain level | −0.0029 dB | **Explained** — `fast_sin_turns`'s fundamental gain of 1.000333, predicted by C3 before collection |
| Filter magnitude response | +0.068 dB, constant over six octave bands | **Explained** — the residual is the envelope's, not the filter's; the two engines run the same recurrence with the same coefficients |
| Harmonics above the corner | up to 21.8 dB, all more than 60 dB below the peak | **Explained** — `fast_sin_turns` against `f64::sin`, bounded by C3 and outside E3b's coverage rule |
| Onset placement | V1 quantised to 256 samples, V2 exact | **Intentional** — CORPUS-0001-C1, the correction Phase 3's sample-accurate scheduling exists to make, confirmed on both halves |
| **Envelope segment shape** | **release +1.137 dB from each engine's own gate; `fall_to_50_ms` +20 ms against a 10 ms threshold** | **Intentional** on [ADR-0042](../../decisions/ADR-0042-envelope-segment-shape.md)'s acceptance. Not a Failure: `CORPUS-0001-P2` claims landmarks, and every landmark is met — see below |

### The one exceeded threshold, and what it turned out to mean

V1's envelope segments are exponential, aimed past their endpoint so the curve
*crosses* it at the authored time; V2's are linear ramps of an exact frame
count. Both therefore **arrive** at the same place at the same time — which is
why every landmark is at zero or one window — but the paths between them differ,
and the release carries **1.137 dB** more energy in V2, measured from each
engine's own gate.

`fall_to_50_ms` is the field that sees it. With sustain at 0.700 the envelope
first reaches half its peak during the *release*, so that field is a probe of the
release **curve**, and it reads +20 ms at four of the six sweep points.

**E2a is not rewritten, and it stays exceeded.** Narrowing it to the three
fields that pass, after seeing which three they are, is the failure mode this
repository has recorded before, and this record does not do it.

**What review established instead is a fact about the claim, not about the
threshold.** `CORPUS-0001-P2` reads, and has always read, "the amplitude
envelope's attack, decay, sustain and release **landmarks** stay within
measurement tolerance of V1's". E2a applied its 10 ms landmark tolerance to all
four fields of `EnvelopeDifference::delta_ms`, one of which is not a landmark on
this fixture. **E2a is therefore a stricter test than P2 asks for**, and P2
itself is met: every landmark is within one window and the sustain level within
0.0027 dB.

So three things are true at once, and the record keeps all three rather than
letting the convenient one stand alone:

- **E2a is exceeded, and this record is therefore `Not supported`.** That is
  what its own falsifier says, and the falsifier was written before collection.
- **No `preserve` claim is broken**, so ADR-0040 clause 4's failure branch does
  not apply and the gate's third bullet is not blocked by this record.
- **E2a was over-broad**, a defect in EVD-0013 rather than in either engine.
  Fixing it would mean re-declaring a threshold after seeing which of its fields
  failed, so it is not fixed; a successor record measuring P2's landmarks
  directly would be the honest way to close it.

### The shape difference, and where its disposition went

V2's linear segments remain a real, measured difference from V1's exponential
ones, and a difference the phase wants: V2's exact segment durations are a
property P02-T005 built deliberately, after an accumulated increment was
measured arriving tens of samples early.

The figures were put to the user on 2026-08-20 and the decision is
[**ADR-0042**](../../decisions/ADR-0042-envelope-segment-shape.md): the shape is
intentional, `CORPUS-0001-P2` is **not** amended because what it claims is met,
and the shape becomes `CORPUS-0001-C2`, an intentional correction alongside C1.

Until ADR-0042 is accepted the shape difference has no named disposition, which
is the only thing in this record still open.
