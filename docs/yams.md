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

- **Runs per voice, at control rate** (`sr`, typically a few hundred Hz — not
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

1. **Header** — zero or more `src` bindings, each aliasing a module address to
   an identifier.
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

`lfo-1.out`, `env-2.out`, `flt-1.cutoff`. The member is a port (`out`) or a
parameter (`cutoff`, `resonance`, …). A parameter read as a source reads the
**previous block's post-modulation value** (the 1-block latency is inherent to
the control loop).

> **Dangling sources are kept, not errored.** If a bound module is deleted or
> renamed, its register simply reads `0` — the routing stays installed and inert
> rather than being thrown away.

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
| `sr`        | control rate in Hz (device-dependent) |
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
> contract: output ports are ±1, a parameter maps through its descriptor range
> to `0..1` (or `-1..1` if bipolar), macros are already normalized. You must
> know each source's polarity — e.g. `flt-1.cutoff` is `0..1` but a pan is
> `-1..1`.

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
| Musical   | `semis(x)` → `2^(x/12)` ratio · `mtof(x)` → Hz, keyboard-tracking |

`sqrt`/`log` of non-positive inputs and `x/0` are clamped (NaN-free), so dead
branches can't poison state.

### Stateful — carry a per-voice register

| Function            | Meaning |
|---------------------|---------|
| `lag(x, t)`         | one-pole smoothing (e.g. mod-wheel glide) |
| `slew(x, up, down)` | slew limiter / portamento, separate rise/fall rates |
| `sah(x, trig)`      | sample-and-hold: latch `x` on a rising edge of `trig` |
| `accum(x)`          | integrator (running sum) |
| `delta(x)`          | change since previous block |
| `phasor(rate)`      | own ramp `0 → 1` at `rate` Hz |
| `edge(x)`           | rising-edge detector |
| `counter(trig)`     | event count |
| `rand([lo, hi])`    | seeded PRNG (latch per note via `gate_on`) |
| `white()`           | per-block seeded noise |

**Per-voice state.** Each voice gets its own register file. State resets on
note-on so a reused/stolen voice never leaks the previous note. **Exception:**
`rand`/`white` *re-seed* (not zero) from `hash(global_seed, voice_index)`, so
simultaneous voices stay decorrelated (stereo width) yet retriggers are
deterministic.

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

Use `set_mod_matrix_script` to install or clear a script on a slot:

| Field           | Meaning |
|-----------------|---------|
| `instrument_id` | instrument (0 = default) |
| `module_id`     | the Mod Matrix module, e.g. `mmx-1` |
| `slot`          | 1-based slot, `1..=16` |
| `source`        | YAMS source text; **empty string clears** the slot back to scalar |

A compile error comes back with diagnostics (all errors, not just the first).
Read back installed scripts with `get_mod_matrix_routings` — a slot with a
script reports its `script` text and its offset is the script's `out`, not
`amount × source`.

---

## Limits and diagnostics

Scripts compile to a flat bytecode VM with hard caps (a script exceeding any cap
is a *compile error* — never silently truncated; the routing is kept inert until
it compiles):

- 256 instructions, 64 registers (≤32 sources, ≤16 state, ≤16 scratch)
- 32 source bindings, 32 nesting depth, 4 KiB source text

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
