# YAMS v1 — Grammar Spike

**YAMS** = *Yet Another Modulation Script*. The control-rate expression language for
the Control Script layer (Step 2 of `control-script-plan.md`). A YAMS program is what a
mod-matrix routing's **amount cell** holds when it is an expression instead of a scalar:

```rust
enum Amount {
    Scalar(BipolarValue),         // today
    Script(Arc<CompiledScript>),  // a compiled YAMS program
}
```

A program reads bound module sources + predefined macros, computes a single value, and
assigns it to `out`. That value is the **normalized-space additive offset** (~[-1, 1])
applied to the routing's destination param — same write channel as the scalar path
(summed across routings, applied through the param curve, clamped). See the Scaling
contract in `control-script-plan.md`.

## Design locks (this spike)

- **Header `src` bindings solve the hyphen problem.** Module addresses (`lfo-1.out`)
  cannot appear in expressions — `lfo-1` parses as `lfo − 1`. So they are aliased in a
  header, parsed by the *address* production (hyphens allowed), to a clean identifier the
  body uses. Each binding → one source register (the offline-resolved `SrcAddr`).
- **Macros / context vars are predefined identifiers** — always in scope, no binding
  needed. Only module/param addresses need `src`.
- **Grammar is rate- and context-agnostic.** Nothing here bakes in "sources are
  cross-module" or "out is a param offset" — those live in the binding header and the
  output target, the context-specific layers. The future audio-rate dialect (Domain A)
  reuses this exact grammar with a different binding source and output target.
- **The grammar is uniform across the tree→VM tiering.** Every function is just a `call`;
  whether the *evaluator* supports a given function (stateless tree vs stateful VM) is a
  runtime concern, not a grammar one. Promoting the evaluator never changes the grammar.
- **v1 = single output.** Exactly one `out = …`. The optional output name is reserved for
  multi-output (the VM tier / unified-panel direction).

## Lexical

```
comment    = "#" , { ? any char except newline ? } ;
ws         = ? spaces and tabs ? ;              (* insignificant *)
terminator = newline | ";" ;                    (* separates statements *)

ident      = letter , { letter | digit | "_" } ;
integer    = digit , { digit } ;
float      = digit , { digit } , [ "." , { digit } ] , [ exponent ] ;
exponent   = ( "e" | "E" ) , [ "+" | "-" ] , digit , { digit } ;
duration   = float , ( "ms" | "s" | "hz" ) ;    (* sugar; folds to a float at compile *)
number     = duration | float ;
```

Reserved words: `src`, `let`, `out`. Function and macro names are **not** reserved (they
are predefined identifiers, shadowable by a `let`/`src`, which the compiler may warn on).

## Grammar

Readable EBNF (`{ }` = zero-or-more, `[ ]` = optional, `|` = alt, `" "` = terminal,
juxtaposition = concatenation):

```
program     = { binding } , body ;

binding     = "src" , ident , "=" , address , terminator ;
address     = module_ref | macro_name ;
module_ref  = module_id , "." , port_name ;
module_id   = ident , "-" , integer ;          (* e.g. lfo-1, flt-2 *)
port_name   = ident ;                            (* out, cutoff, vel, … *)
macro_name  = ident ;                            (* velocity, mod_wheel, … *)

body        = { local } , output ;
local       = "let" , ident , "=" , expr , terminator ;
output      = "out" , [ ident ] , "=" , expr , [ terminator ] ;

expr        = ternary ;
ternary     = logic_or , [ "?" , expr , ":" , ternary ] ;       (* right assoc *)
logic_or    = logic_and , { "||" , logic_and } ;
logic_and   = equality , { "&&" , equality } ;
equality    = relational , { ( "==" | "!=" ) , relational } ;
relational  = additive , { ( "<" | ">" | "<=" | ">=" ) , additive } ;
additive    = multiplicative , { ( "+" | "-" ) , multiplicative } ;
multiplicative = unary , { ( "*" | "/" | "%" ) , unary } ;
unary       = ( "-" | "!" ) , unary | power ;
power       = primary , [ "^" , unary ] ;                       (* right assoc *)
primary     = number | call | ident | "(" , expr , ")" ;
call        = ident , "(" , [ arg_list ] , ")" ;
arg_list    = expr , { "," , expr } ;
```

### Operator precedence (low → high)

| Level | Operators | Assoc |
|------|-----------|-------|
| 1 | `?:` | right |
| 2 | `\|\|` | left |
| 3 | `&&` | left |
| 4 | `== !=` | left |
| 5 | `< > <= >=` | left |
| 6 | `+ -` | left |
| 7 | `* / %` | left |
| 8 | unary `- !` | right |
| 9 | `^` | right |
| 10 | call / atom / `( )` | — |

## Predefined identifiers

**Macros** (per-voice, ~normalized): `velocity` `mod_wheel` `aftertouch` `pitch_bend`
`note` (= MIDI note / 127) `poly_at`.

**Context** (per-voice): `gate` (1 while held) · `gate_on` (1 the block of note-on) ·
`age` (seconds since note-on).

**Constants**: `pi` `tau` `e` `sr` (control-rate, Hz).

## Function catalog

Grammar treats all of these as `call`. Tier = which evaluator runs them.

**Stateless — expression tree (S2.1 first cut):**

| Domain | Functions |
|---|---|
| Range | `abs(x)` `sign(x)` `min(a,b)` `max(a,b)` `clamp(x,lo,hi)` |
| Rounding | `floor(x)` `ceil(x)` `round(x)` `trunc(x)` `quantize(x,step)` |
| Power/exp | `pow(x,e)` `sqrt(x)` `exp(x)` `log(x)` |
| Trig | `sin(x)` `cos(x)` `tan(x)` `atan(x)` `atan2(y,x)` |
| Interp | `lerp(a,b,t)` `mix(a,b,t)` `smoothstep(a,b,x)` |
| Curves | `sigmoid(x)` `gauss(x)` |
| Musical | `semis(x)` → `2^(x/12)` ratio |

**Stateful — needs a per-voice register, promotes to the VM tier:**

| Function | Meaning |
|---|---|
| `lag(x, t)` | one-pole smoothing (mod-wheel glide) |
| `slew(x, up, down)` | slew limiter / portamento |
| `sah(x, trig)` | sample-and-hold |
| `accum(x [, reset])` | integrator |
| `delta(x)` | change since previous block |
| `phasor(rate)` | own ramp 0→1 |
| `counter(trig)` / `edge(x)` | event count / rising-edge detect |
| `rand([lo, hi])` | seeded PRNG (latch per note via `gate_on`) |
| `white()` | per-block seeded noise |

## Semantic constraints (not enforced by grammar)

- Exactly **one** `out` statement; its value is the offset. Optional name reserved for
  multi-output.
- All identifiers in the body must resolve to a `src` binding, a `let`, a predefined
  macro/context var/constant, or a function name — else compile error.
- RT contract: compile offline to an immutable `Arc`; fixed register file; hard
  instruction cap (overflow clamps + counts, never reallocates). Mirror `note_processor.rs`.
- Division by zero, NaN/Inf → clamp to a safe default at eval, never panic.

## Worked examples (all parse under this grammar)

```
# the demo, today's two additive routings as one program
src lfo = lfo-1.out
out = lfo * 0.45 + velocity * 0.6
```

```
# cross-source multiply — LFO depth scales with velocity (impossible pre-Step-2)
src lfo = lfo-1.out
out = lfo * velocity
```

```
# conditional — hard strike opens, soft closes
src lfo = lfo-1.out
out = velocity > 0.8 ? lfo : lfo * 0.2
```

```
# intermediate locals + a curve
src lfo = lfo-1.out
src env = env-2.out
let depth = lerp(0.2, 1.0, velocity)
out = clamp(env * depth + lfo * 0.1, 0, 1)
```

```
# stateful — smoothed mod wheel, and a per-note random offset
out = lag(mod_wheel, 50ms) + sah(white(), gate_on) * 0.3
```
