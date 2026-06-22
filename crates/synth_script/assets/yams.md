# YAMS — Yet Another Modulation Script

YAMS is Pertylizer's small control-rate expression language. A YAMS program is
what a **Mod Matrix slot** holds when its modulation amount is an *expression*
instead of a plain scalar. The program reads modulation sources (other modules'
outputs, MIDI macros, per-voice context), computes a single value, and assigns
it to `out`. That value is the **normalized-space additive offset** applied to
the slot's destination parameter — the exact same write channel a scalar amount
uses (summed across slots, run through the param curve, clamped).

In short: a scalar slot says *"this source, scaled by this knob."* A YAMS slot
says *"compute the offset however you like — from any combination of sources,
math, curves, and per-voice state."*

- **Runs per voice, at control rate** (`cr`, typically a few hundred Hz — not
  audio rate). One independent copy of the script's state per sounding voice.
- **Compiled offline, evaluated in real time.** The UI/MCP thread compiles the
  source text to a flat bytecode program; the audio thread only ever runs the
  bytecode over a pre-allocated register file (no allocation, no recursion).
- **Source text is the source of truth.** Scripts are persisted as canonical
  YAMS text and recompiled on load — human-readable and diff-friendly.

---

## Anatomy of a program

```yams
# Pitch vibrato whose depth opens up with the mod wheel.
src vib = lfo-1.out          # header: bind a module output to a clean name

out = vib * lerp(0.05, 0.4, mod_wheel)   # body: compute the offset
```

Every program has two parts:

1. **Header** — zero or more `src` bindings (each aliasing a module address to
   an identifier) and zero or more `arr` const-table declarations, in any order.
2. **Body** — zero or more `let` locals, then **exactly one** `out = …`.

The header and body are separated by a blank line in canonical form.

### Why the `src` header exists

Module addresses contain hyphens (`lfo-1`, `flt-2`), and a hyphen is the minus
operator. `lfo-1.out` in an expression would parse as `lfo − 1.out`. So module
addresses may **only** appear in `src` bindings, where a dedicated address
grammar allows the hyphen. The body then refers to the clean alias:

```yams
src lfo = lfo-1.out
src eg  = env-1.out

out = lfo * eg * 0.4
```

Macros and context variables are **predefined** — always in scope, never bound
with `src`. Only module/param addresses need a binding.

### Module addresses

```
module_ref = module_id "." member
module_id  = name [ "-" instance ]      # instance defaults to 1 if omitted
```

`lfo-1.out`, `env-2.out`. The member is usually an **output port** (`out`,
`out1`, …), but it may also be a **parameter** (`flt-1.cutoff`, `osc-1.detune`).

> **Parameter sources read the live, normalized value.** An address whose member
> is a *parameter* rather than an output port (`flt-1.cutoff`) reads the
> parameter's current value, normalized to `0..1` through its descriptor
> range+curve — so a 1000 Hz cutoff in a 20..20000 Hz log range arrives as a
> value you can mix with the ±1 of an LFO, not a raw frequency. This lets a
> script react to what the user/automation does to a knob. A member that is
> neither a port nor a parameter still reads `0` (disable-and-keep).

> **Dangling sources are kept, not errored.** If a bound module is deleted or
> renamed, its register simply reads `0` — the routing stays installed and inert
> rather than being thrown away.

### Arrays — const lookup tables

An `arr` declaration in the header defines a **read-only, compile-time-constant**
lookup table. The headline use is a per-parameter step sequencer on top of the
`beat` transport var:

```yams
arr seq = [0, 0.5, 1, 0.25, 0, 0.75, 0.5, 0]

out = seq[floor(beat) % 8]      # one step per beat, wrapping every 8
```

or a custom LFO / transfer shape indexed by a phasor:

```yams
arr shape = [0, 0.3, 0.8, 1, 0.8, 0.3, 0, -0.3]

out = shape[floor(phasor(2) * len(shape))]
```

Rules that keep arrays cheap and RT-safe:

- **An array is not a value.** It can only be *indexed* (`name[expr]`) or
  *measured* (`len(name)`); it cannot be assigned to a `let`, passed around, or
  used in arithmetic. "Everything is `f32`" still holds — `name[expr]` is an
  `f32`. (`out = seq + 1` is a compile error: "arrays cannot be used directly".)
- **Elements are constants.** Each element must fold at compile time — literals,
  negatives, `pi`, and constant arithmetic (`[-0.3, 1 + 1, pi]`) are all fine; a
  source or macro is not.
- **The index is floored, then clamped.** `name[i]` reads element
  `clamp(floor(i), 0, len-1)`. Flooring gives equal-width steps (the correct rule
  for a sequencer); the clamp makes an out-of-range index safe — a negative or
  `NaN` index reads element `0`, a too-large one reads the last element. Indexing
  never wraps and never interpolates; for sequencer *wrap*, write it explicitly
  (`seq[floor(beat) % len(seq)]`), and for interpolation use `table_lin` (below).
  A *constant* index the compiler can prove is out of range (e.g. `seq[8]` on a
  length-8 table) is a **compile warning** — the runtime still clamps safely, but
  it flags the likely off-by-one. Dynamic indices are left to the runtime clamp.
- **`len(name)`** folds to the element count at compile time, so
  `i % len(seq)` costs nothing extra.
- **`table_lin(arr, pos)`** is the interpolated cousin of `arr[i]`: it lerps
  between neighbours (`pos` in `0..len-1`, clamped), so a smooth-LFO shape table
  reads without the zipper noise of stepped indexing.
- **`scale_snap(x, arr)`** snaps a value in semitones to the nearest member of a
  scale table, octave-aware (a pitch class near the octave boundary snaps across
  it). With `arr maj = [0, 2, 4, 5, 7, 9, 11]`, `out = semis(scale_snap(x * 24,
  maj))` turns a continuous source into musical scale-degree steps instead of a
  glissando. Both take an array name (not a value) as one argument.

Caps: at most 16 arrays, and at most 256 elements total across all of them.
Exceeding either is a compile error (never a silent truncation). An empty array
(`[]`) is also an error.

---

## Types and truthiness

Everything is `f32`. There is no separate boolean type:

- `bool = (x != 0)` — any non-zero value is "true".
- Comparisons yield exactly `1.0` or `0.0`.
- `!x` is `x == 0 ? 1 : 0`.
- `&&` / `||` operate on the same convention.

## Evaluation model — eager, no short-circuiting

**This is the most important thing to understand about YAMS, and it differs
from C/Rust on purpose.**

> Every node evaluates every block. `?:`, `&&`, and `||` are **value
> selectors**, not control flow.

```yams
out = velocity > 0.8 ? lag(x, 50ms) : 0
```

Here `lag(x, 50ms)` is evaluated **every block**, even when velocity ≤ 0.8 — its
per-voice smoothing state keeps advancing. The ternary only chooses *which
already-computed value* becomes the result.

Consequences:

- **Stateful functions never stall.** `lag`, `slew`, `sah`, `phasor`, `accum`,
  PRNG — all of their registers tick continuously regardless of which branch is
  "live". No frozen state, no jumps when a branch reactivates, no dependence on
  evaluation order.
- **Cost is worst-case by construction**, which is what makes the instruction
  cap exact. The extra cost of "untaken" branches is negligible at control rate.
- A dead branch that would divide by zero or produce NaN is harmless — YAMS
  arithmetic is NaN-free (safe division, safe `log`/`sqrt`), the result clamps
  to a safe default, and it's discarded anyway.

---

## Operators

Listed low → high precedence. Comparison operators are **non-associative** —
`a < b < c` is a *syntax error* (parenthesize instead), not silent
`(a < b) < c`.

| Level | Operators        | Assoc | Notes |
|------:|------------------|-------|-------|
| 1     | `?:`             | right | ternary select |
| 2     | `\|\|`           | left  | eager OR |
| 3     | `&&`             | left  | eager AND |
| 4     | `==` `!=`        | none  | cannot chain |
| 5     | `<` `>` `<=` `>=`| none  | cannot chain |
| 6     | `+` `-`          | left  | |
| 7     | `*` `/` `%`      | left  | `/` by 0 → 0 (NaN-free) |
| 8     | unary `-` `!`    | right | |
| 9     | `^`              | right | power |
| 10    | call / atom / `( )` | —  | |

---

## Predefined identifiers

These share one namespace with your `src`/`let` names. They are **reserved** —
binding a name that collides with any of them (or with the keywords `src`,
`let`, `out`) is a compile error. So `sin` always means sine; there is no silent
shadowing.

### Macros (per-voice MIDI, already normalized)

| Name         | Meaning |
|--------------|---------|
| `velocity`   | note-on velocity, `0..1` |
| `mod_wheel`  | mod wheel (CC1), `0..1` |
| `aftertouch` | channel aftertouch, `0..1` |
| `pitch_bend` | pitch bend, `-1..1` |
| `note`       | MIDI note number / 127 |
| `poly_at`    | polyphonic aftertouch, `0..1` |

### Context (per-voice, filled each block)

| Name        | Meaning |
|-------------|---------|
| `gate`      | `1` while the note is held, else `0` |
| `gate_on`   | `1` for the single block of note-on, else `0` |
| `age`       | seconds since note-on |
| `cr`        | **control** rate in Hz — `sample_rate / block_size`, ~hundreds of Hz (device-dependent). **Not** the audio sample rate; a 48 kHz device yields `cr ≈ 750`, not 48000 |
| `beat`      | absolute transport position in beats (grows unbounded; `sin(beat * tau)` is a tempo-locked sine) |
| `bar_phase` | phase within the current bar, `0..1` (4/4); wraps every bar |
| `tempo`     | transport tempo in BPM (`tempo / 60` is beats per second) |
| `playing`   | `1` while the transport is running, else `0` |

The transport vars (`beat`, `bar_phase`, `tempo`, `playing`) are global —
every voice sees the same value each block — so they drive tempo-synced
modulation: `out = (sin(bar_phase * tau) * 0.5 + 0.5) * playing` is a bar-locked
ramp that mutes when the transport stops.

### Constants

`pi`, `tau` (= 2π), `e`.

> **Source polarity matters.** Bound sources arrive normalized per the scaling
> contract: output ports are ±1 and macros are already normalized. You must know
> each source's polarity — e.g. an `lfo-1.out` is `-1..1` but an `env-1.out` is
> `0..1`. Parameter sources (`flt-1.cutoff`) arrive in `0..1`, mapped through the
> param's descriptor range+curve — see *Module addresses* above.

---

## Function catalog

All functions are written `name(args)`. Two tiers, distinguished only by whether
they carry per-voice state.

### Stateless — pure functions of this block's inputs

| Domain    | Functions |
|-----------|-----------|
| Range     | `abs(x)` · `sign(x)` · `min(a,b)` · `max(a,b)` · `clamp(x,lo,hi)` |
| Rounding  | `floor(x)` · `ceil(x)` · `round(x)` · `trunc(x)` · `quantize(x,step)` |
| Power/exp | `pow(x,e)` · `sqrt(x)` · `exp(x)` · `log(x)` |
| Trig      | `sin(x)` · `cos(x)` · `tan(x)` · `atan(x)` · `atan2(y,x)` |
| Interp    | `lerp(a,b,t)` · `mix(a,b,t)` *(alias of `lerp`)* · `smoothstep(a,b,x)` |
| Curves    | `sigmoid(x)` · `gauss(x)` |
| Musical   | `semis(x)` → `2^(x/12)` ratio · `mtof(m)` → Hz from a **raw MIDI note** (`mtof(69)` = 440 Hz; for the normalized `note` macro use `mtof(note * 127)`) · `scale_snap(x, arr)` → snap semitones to a scale (octave-aware) |
| Polarity  | `unipolar(x)` → `x*0.5+0.5` (±1 → 0..1) · `bipolar(x)` → `x*2-1` (0..1 → ±1) |
| Arrays    | `name[i]` index a const table (floor + clamp) · `len(name)` → element count (folds at compile time) · `table_lin(arr, pos)` → linear-interpolated lookup |

`sqrt`/`log` of non-positive inputs and `x/0` are clamped (NaN-free), so dead
branches can't poison state. Array indexing and `len` operate on `arr` tables
declared in the header — see [Arrays](#arrays--const-lookup-tables).

### Stateful — carry a per-voice register

| Function            | Meaning |
|---------------------|---------|
| `lag(x, t)`         | one-pole (exponential) smoothing toward `x`, time constant `t` — decelerates near the target (e.g. mod-wheel glide) |
| `slew(x, up, down)` | **linear** slew limiter / portamento — constant rate, `up`/`down` in **units per second** (separate rise/fall) |
| `sah(x, trig)`      | sample-and-hold: latch `x` on a rising edge of `trig` |
| `accum(x)`          | integrator (running sum) |
| `accum(x, reset)`   | integrator that zeroes on a rising edge of `reset` |
| `delta(x)`          | change since previous block |
| `phasor(rate)`      | own ramp `0 → 1` at `rate` Hz (free-running) |
| `phasor(rate, sync)`| ramp that resets to 0 on a rising edge of `sync` (phase-align to note-on / a clock) |
| `edge(x)`           | rising-edge detector |
| `counter(trig)`     | event count |
| `pulse(div)`        | trigger at the start of every `div`-th beat (beats 0, div, 2·div, …). `div` must be an integer ≥ 2: a *constant* is checked at compile time, but a *dynamic* `div` is accepted unchecked and **must stay an integer ≥ 2 at runtime** — a value < 2 (e.g. 1) makes `% div` stick at 0 and the trigger freezes |
| `rand([lo, hi])`    | seeded PRNG (latch per note via `gate_on`) |
| `rand_smooth(rate)` | smooth random LFO: band-limited wander in `[0, 1)`, new target every `1/rate` s. Unipolar — wrap in `bipolar(…)` for ±1 (e.g. pitch drift) |
| `white()`           | per-block seeded noise |

**Per-voice state.** Each voice gets its own register file. State resets on
note-on so a reused/stolen voice never leaks the previous note. **Exception:**
`rand`/`white`/`rand_smooth` *re-seed* (not zero) from
`hash(global_seed, voice_index)`, so simultaneous voices stay decorrelated
(stereo width) yet retriggers are deterministic.

**State footprint.** Most stateful ops use one register cell; the cap is 16. The
synced overloads `phasor(rate, sync)` and `accum(x, reset)` use **two** (the
value plus the previous block's trigger, for edge detection), and
`rand_smooth(rate)` uses **three** (phase + two segment endpoints). The synced
form is what lets a custom LFO phase-align to note-on (`phasor(r, gate_on)`) or to
the bar.

**Coefficient caching.** For `lag`/`slew`, when the time argument is a *literal*
(`50ms`) the smoothing coefficient is precomputed at compile time and the audio
thread runs a single multiply-accumulate. A time *expression* costs a per-block
coefficient computation.

---

## Numbers, units, comments

- **Numbers:** `440`, `0.5`, `1.5e3`. Leading zero required (`0.5`, not `.5`).
- **Duration literals:** `50ms`, `1.5s`. These fold to **seconds** at compile
  time (`ms` × 0.001). Time is the only unit dimension. Use them anywhere a time
  argument is expected: `lag(mod_wheel, 50ms)`.
- **Comments:** `#` to end of line.

```yams
# own-line comment
out = lag(velocity, 50ms)   # trailing comment
```

---

## Canonical formatting (`yamsfmt`)

YAMS has exactly **one** canonical form — like `rustfmt`/`gofmt`, no config, no
opt-out. The persisted form is always canonical: the GUI editor formats on
commit, and the MCP author path stores formatted text. A parse error suppresses
formatting (the editor keeps your raw text + an error marker until it parses).
The formatter is idempotent: `fmt(fmt(x)) == fmt(x)`.

The rules you'll notice:

- 4-space indent, LF endings, one trailing newline, no trailing whitespace.
- One statement per line; `;` terminators removed.
- All `src` bindings first (author order kept, **not** sorted), then exactly one
  blank line, then the body (`let`s, then the single `out`).
- One space around binary operators (`a + b`); unary tight (`-x`, `!c`);
  ternary `c ? a : b`; calls `f(a, b)` with no space before `(`.
- Numbers canonicalize: `.5` → `0.5`, `1.50` → `1.5`, `1.0` → `1`, `1E3` →
  `1e3`, `50MS` → `50ms`.
- The formatter **never changes meaning** — it won't fold `2 * 3`; constant
  folding is the compiler's job.

---

## Examples

All of these parse and compile. The first batch is taken from the bundled
**"YAMS Script Demo"** example project.

```yams
# Velocity → filter, smoothed (lag) so it glides instead of stepping.
out = lag(velocity, 50ms)
```

```yams
# Pitch vibrato (LFO → detune); the mod wheel opens the depth.
src vib = lfo-1.out

out = vib * lerp(0.05, 0.4, mod_wheel)
```

```yams
# LFO gated by an envelope so the modulation tapers on release.
src lfo = lfo-1.out
src eg  = env-1.out

out = lfo * eg * 0.4
```

```yams
# A built-in 3 Hz resonance wobble (phasor + sin), strong only on hard hits.
let lfo2 = sin(phasor(3) * tau) * 0.5

out = velocity > 0.7 ? lfo2 : lfo2 * 0.2
```

```yams
# Slow auto-pan that widens as the note ages (smoothstep ramps it in over 1.5 s).
out = sin(phasor(0.3) * tau) * smoothstep(0, 1.5, age)
```

```yams
# Asymmetric slew (slow rise, fast fall) animating a filter drive.
src lfo = lfo-1.out

out = slew(lfo, 2, 8) * 0.4
```

```yams
# White noise on oscillator level — subtle amplitude shimmer / analog instability.
out = white() * 0.08
```

More patterns:

```yams
# Cross-source multiply — LFO depth scales with velocity (impossible with a
# scalar amount, which can only reference one source).
src lfo = lfo-1.out

out = lfo * velocity
```

```yams
# Intermediate locals + clamping a composite offset.
src lfo = lfo-1.out
src env = env-2.out

let depth = lerp(0.2, 1.0, velocity)
out = clamp(env * depth + lfo * 0.1, 0, 1)
```

```yams
# Stateful: smoothed mod wheel plus a per-note random offset latched at note-on.
out = lag(mod_wheel, 50ms) + sah(white(), gate_on) * 0.3
```

---

## Authoring scripts

### In the GUI

Open a Mod Matrix module, pick a slot's source and destination, then open the
**ƒx** expression editor on that slot. Type YAMS, and the live compile status
shows errors with line/column spans. The text is formatted and committed on
apply.

### Over MCP

Use `set_mod_matrix_script` to install or clear a script on a slot. Despite the
name, the host may be a **Mod Matrix** (`mmx-N`) *or* a **Script module**
(`scr-N`) — on a Mod Matrix the script's `out` is the slot's modulation offset;
on a Script module it is the value of that slot's `outN` output port:

| Field           | Meaning |
|-----------------|---------|
| `instrument_id` | instrument (0 = default) |
| `module_id`     | the host module — a Mod Matrix (`mmx-1`) or Script module (`scr-1`) |
| `slot`          | 1-based slot — Mod Matrix `1..=16`, Script module `1..=8` (drives `out1`..`out8`) |
| `source`        | YAMS source text; **empty string clears** the slot back to scalar |

A compile error comes back with diagnostics (all errors, not just the first).
Read back installed scripts with `get_mod_matrix_routings` (Mod Matrix — a slot
with a script reports its `script` text and its offset is the script's `out`, not
`amount × source`) or with `get_module_info` (a Script module exposes a `scripts`
array of `{slot, output_port, source}`). `get_yams_reference` returns this
document over MCP.

---

## Limits and diagnostics

Scripts compile to a flat bytecode VM with hard caps (a script exceeding any cap
is a *compile error* — never silently truncated; the routing is kept inert until
it compiles):

- 256 instructions, 64 registers (≤32 sources, ≤16 state, ≤16 scratch)
- 32 source bindings, 32 nesting depth, 4 KiB source text
- 16 arrays, 256 array elements total (across all `arr` declarations)

Diagnostics carry text spans and **all** errors are reported in one pass (better
for the editor and for tooling). Distinctions:

- An unknown **function** is an error.
- An unknown **module** (dangling `src`) is *not* — it's a zero-reading register
  (disable-and-keep).
- Binding a reserved name (`let sin = …`) is a compile error.

---

## Reference

- Toolchain crate (lexer/parser/compiler/`yamsfmt`): `crates/synth_script`
- Real-time bytecode + evaluator: `crates/synth_core/src/script`
- Runnable demo: `cargo run -p synth_script --example demo`
- Bundled example project: `assets/examples/projects/YAMS Script Demo.json`
