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
- **The grammar is uniform over the function set.** Every function is just a `call`;
  whether a function is stateless or carries a per-voice register is a compiler/evaluator
  concern, not a grammar one. Adding functions never changes the grammar.
- **v1 = single output.** Exactly one `out = …`. The optional output name is reserved for
  multi-output (the unified-panel direction).

## Lexical

```
comment    = "#" , { ? any char except newline ? } ;
ws         = ? spaces and tabs ? ;              (* insignificant *)
separator  = ( newline | ";" ) , { newline | ";" } ;  (* one+; consumes blank lines *)

ident      = letter , { letter | digit | "_" } ;
integer    = digit , { digit } ;
float      = digit , { digit } , [ "." , { digit } ] , [ exponent ] ;
exponent   = ( "e" | "E" ) , [ "+" | "-" ] , digit , { digit } ;
duration   = float , ( "ms" | "s" ) ;           (* time sugar; folds to SECONDS at compile *)
number     = duration | float ;
```

Reserved words: `src`, `let`, `out`. Function/macro/context/constant names are predefined
identifiers and **cannot be shadowed** by a `let`/`src` (compile error — see Semantic
constraints).

## Grammar

Readable EBNF (`{ }` = zero-or-more, `[ ]` = optional, `|` = alt, `" "` = terminal,
juxtaposition = concatenation):

```
program     = { separator } , { binding } , body , { separator } ;

binding     = "src" , ident , "=" , address , separator ;
address     = module_ref ;                       (* macros are predefined, not bound here *)
module_ref  = module_id , "." , member_name ;
module_id   = ident , [ "-" , integer ] ;        (* lfo-1, flt-2; omitted instance ⇒ 1 *)
member_name = ident ;                            (* port or param: out, cutoff, vel, … *)

body        = { local } , output ;
local       = "let" , ident , "=" , expr , separator ;
output      = "out" , [ ident ] , "=" , expr ;

expr        = ternary ;
ternary     = logic_or , [ "?" , expr , ":" , ternary ] ;       (* right assoc *)
logic_or    = logic_and , { "||" , logic_and } ;
logic_and   = equality , { "&&" , equality } ;
equality    = relational , [ ( "==" | "!=" ) , relational ] ;   (* non-assoc: no chaining *)
relational  = additive , [ ( "<" | ">" | "<=" | ">=" ) , additive ] ; (* non-assoc *)
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
| 4 | `== !=` | none |
| 5 | `< > <= >=` | none |
| 6 | `+ -` | left |
| 7 | `* / %` | left |
| 8 | unary `- !` | right |
| 9 | `^` | right |
| 10 | call / atom / `( )` | — |

**Comparison operators are non-associative.** Chaining at one level — `a < b < c`,
`a == b == c` — is a **syntax error**, not silent `(a < b) < c` (which would compare a
`0/1` boolean against `c`, a classic logic-bug footgun). The parser should emit a helpful
"comparison operators cannot be chained; parenthesize" message. Mixed precedence levels are
still fine and unambiguous (`a < b == c` → `(a < b) == c`); only same-level chaining is
rejected.

## Predefined identifiers

**Macros** (per-voice, ~normalized): `velocity` `mod_wheel` `aftertouch` `pitch_bend`
`note` (= MIDI note / 127) `poly_at`.

**Context** (per-voice): `gate` (1 while held) · `gate_on` (1 the block of note-on) ·
`age` (seconds since note-on).

**Constants**: `pi` `tau` `e` `sr` (control-rate, Hz).

## Function catalog

Grammar treats all of these as `call`. Tier = which evaluator runs them.

**Stateless — pure functions of this block's inputs (no register):**

| Domain | Functions |
|---|---|
| Range | `abs(x)` `sign(x)` `min(a,b)` `max(a,b)` `clamp(x,lo,hi)` |
| Rounding | `floor(x)` `ceil(x)` `round(x)` `trunc(x)` `quantize(x,step)` |
| Power/exp | `pow(x,e)` `sqrt(x)` `exp(x)` `log(x)` |
| Trig | `sin(x)` `cos(x)` `tan(x)` `atan(x)` `atan2(y,x)` |
| Interp | `lerp(a,b,t)` `mix(a,b,t)` `smoothstep(a,b,x)` |
| Curves | `sigmoid(x)` `gauss(x)` |
| Musical | `semis(x)` → `2^(x/12)` ratio · `mtof(x)` → `440·2^((x·127−69)/12)` Hz (keyboard-tracking) |

**Stateful — needs a per-voice register:**

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

**Coefficient caching.** For `lag`/`slew`, the smoothing coefficient
`α = 1 − e^(−1/(sr·t))` is transcendental. When `t` is a **literal constant** (`50ms`) the
compiler precomputes `α` offline, so the audio thread runs only a multiply-accumulate
`y += α·(x − y)`. When `t` is an **expression**, `α` is computed per block at runtime (a
documented cost; a fast `exp` polynomial approximation is the fallback if it shows up hot).

**PRNG voice isolation.** `rand`/`white` registers are **per voice**, seeded
`hash(global_seed, voice_index)` — voice index (not pitch/Note ID) guarantees
simultaneously-sounding voices are decorrelated, preserving stereo width. These registers
**re-seed** (not zero) on note-on/voice-steal, so retriggers are deterministic without
collapsing the per-voice decorrelation.

## Evaluation model (locked)

**YAMS is fully eager — there is no short-circuiting anywhere. Every node evaluates every
block.** This is a deliberate divergence from C/Rust semantics, justified by control-rate
determinism, and must be surfaced loudly to authors.

- `?:`, `&&`, `||` are **value selectors, not control flow** — a mux over already-computed
  operands, never a lazy gate. `a ? lag(x, 50ms) : 0` *always* evaluates `lag(...)` (its
  per-voice state keeps ticking); the ternary only chooses which finished value becomes the
  result.
- **Stateful functions therefore never stall.** All state (`lag`, `slew`, `sah`, `accum`,
  `phasor`, PRNG) advances continuously regardless of which branch is "live" — no frozen
  registers, no jumps on reactivation, no traversal-order dependence.
- **Cost is worst-case by construction**, which makes the instruction cap exact rather than
  data-dependent. The extra cost of evaluating untaken branches is negligible at
  control-rate. A dead branch that would divide by zero / produce NaN is harmless — it
  clamps to a safe default (below) and is then discarded.

## Semantic constraints (not enforced by grammar)

- Exactly **one** `out` statement; its value is the offset. Optional name reserved for
  multi-output.
- **Single namespace; built-ins are reserved.** Function names, macros, context vars,
  constants, and keywords cannot be bound by `let` or `src` — shadowing is a **compile
  error** (e.g. `let sin = 2.0` → *"'sin' is a built-in function, choose another name"*).
  So `sin(x)` always means sine; there is no Lisp-2 function/variable split and no silent
  shadowing footgun. All user-introduced names (`src` aliases + `let`s) share one namespace.
- Every identifier must resolve to a `src` binding, a `let`, a predefined
  macro/context var/constant, or a function name — else compile error.
- **Unit literals fold to SI base at compile time:** `ms` → seconds (×0.001), `s` →
  seconds (×1). Time is the only unit dimension in v1 (`hz` was dropped — a frequency
  literal cannot fold to seconds without silently changing the number; reintroduce only
  with an explicit raw-Hz meaning if a consumer ever needs it).
- RT contract: compile offline to an immutable `Arc`; **flat bytecode VM** over a
  pre-allocated voice-local register file (no AST-walk recursion → O(1) stack); hard
  instruction cap (overflow clamps + counts, never reallocates). Mirror `note_processor.rs`.
- **NaN poisoning is fatal to state — sanitize in two layers.** Clamping only the final
  `out` is insufficient: NaN is contagious and permanent — feed it into `lag`/`accum` once
  and that register stays `NaN` forever.
  1. *Prevent at the source:* NaN-free arithmetic — safe division (`x/0 → 0`), safe
     `log`/`sqrt` of non-positive inputs.
  2. *Belt-and-suspenders:* a `safe_clamp` (`NaN → 0.0`) on **every write to a state
     register**, unconditionally. (`f32::clamp` panics on NaN bounds — use the custom
     `safe_clamp`, never `std`.) Transient ALU results need no per-op sanitize; they are
     clamped at `out`. Per-op sanitize of everything is acceptable if simpler — the
     control-rate cost is negligible.

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

## Canonical formatting (`yamsfmt`)

YAMS has **one** canonical form. Like `rustfmt`/`gofmt` there is **no config and no
opt-out** — you never think about layout. The formatter walks the **AST** and prints the
canonical form; it never changes meaning. (A lossless-CST formatter was considered and
**rejected** — see decision #11.)

**Mandatory, not advisory.** The *persisted* form is always canonical: the GUI editor
formats on commit/blur, the MCP author path stores the formatted text, and there is no
unformatted state on disk. A parse error suppresses formatting (the editor keeps the raw
text + an error marker until it parses). The formatter is **idempotent** —
`fmt(fmt(x)) == fmt(x)` — and lives in the non-RT compiler crate (UI/MCP thread only),
exposed as a pure `format(&str) -> Result<String, ParseError>` and an MCP `format_yams`.

### Rules

- **Indent** 4 spaces, spaces only, never tabs. LF endings, exactly one trailing newline,
  no trailing whitespace on any line.
- **One statement per line.** `;` terminators are removed; multiple statements on a line
  are split.
- **Layout:** all `src` bindings first (author order preserved — not sorted), then
  **exactly one** blank line, then the body (`let`s, then the single `out`). Runs of blank
  lines collapse to one; no leading/trailing blank line in a block. A single blank line
  inside the body is preserved as a grouping affordance.
- **No alignment** (the `rustfmt` stance, not `gofmt`'s): one space each side of `=`, never
  padded to align adjacent lines — so renaming never reflows neighbours.
- **Spacing:** binary operators ` x + y ` one space each side (`+ - * / % ^ == != < > <= >=
  && ||`); unary tight (`-x`, `!c`); ternary ` c ? a : b `; calls `f(a, b)` — no space
  before `(`, no padding inside, `, ` between args; no padding inside grouping `(a + b)`.
- **Parentheses:** author grouping is preserved (intent / readability). Only parens around
  a single atom are dropped: `(x)`, `((x))` → `x`. Precedence-redundant parens around
  compound expressions are kept.
- **Numbers** canonicalize to a single spelling: leading zero required (`.5` → `0.5`),
  insignificant trailing zeros trimmed (`1.50` → `1.5`, `1.0` → `1`), bare integers
  unchanged, lowercase exponent/units (`1E3` → `1e3`, `50MS` → `50ms`).
- **Comments:** `#` then exactly one space. An own-line comment takes the following
  statement's indent; a trailing comment sits two spaces after the code. Text preserved
  verbatim (surrounding whitespace trimmed).

### Wrapping (width = 80)

A statement that fits on 80 columns stays on one line. Otherwise:

- **`out = <expr>`** → break after `=`, expr indented +4; then break at the **lowest-
  precedence** top-level operators, operator **leading** each continuation line, all
  continuations at the same +4 hanging indent. Descend into a subexpression only if it
  still overflows.
- **Calls** → one argument per line indented +4, **trailing comma**, `)` back at the
  statement indent.
- **Ternary** → break before `?` and `:`, each leading its line, indented +4.

```
# binary chain
out =
    env * lerp(0.2, 1.0, velocity)
    + lfo * 0.1
    + sah(white(), gate_on) * 0.3

# call that overflows
out = clamp(
    env * depth + lfo * 0.1,
    0,
    1,
)

# ternary
out =
    velocity > 0.8
    ? lfo * fullDepth
    : lfo * 0.2
```

## Open questions / decisions

Gap analysis before implementation. Recommendations written in; **LOCKED** = decided
(recommendation adopted), **OPEN** = needs a call. Numbering matches the review.

### Engine / data-model seam

- **[LOCKED] 1 — The script subsumes `amount`; the routing owns the destination.** A
  Script routing's offset is the script's `out` directly — no separate attenuverter
  multiply (depth is written in the script, `out = … * 0.5`). The `DestAddr` stays on the
  routing; the script *computes the value*, the routing *owns the address*. v1: one `out`
  → the routing's single destination. Multi-output (a script owning several named dests)
  is the future VM/unified-panel direction. The `Amount` enum at the top of this doc is the
  **in-memory** model; persistence is #2.
- **[LOCKED] 2 — Persistence + schema.** Do **not** swap `amount: BipolarValue` for an
  `Amount` enum in the save format — it hits the same descriptor-driven schema +
  example-project validation wall that sank the first S1.1b spike. Instead add a separate
  optional `script: Option<String>` field on the routing (when present it wins over
  `amount`), persisting the script as **canonical YAMS source text, compiled on load** (not
  serialized bytecode) — human-readable, forward-compatible, consistent with "no
  unformatted persisted state." **Multiline is a non-issue:** `serde_json` escapes newlines
  to `\n` automatically (lossless round-trip; pretty-print keeps the string inline). The
  only cost is that a script is one long JSON line → whole-line git diffs; switch to a
  JSON array-of-lines (`["src …", "out …"]`, joined on load) *only if* long scripts in the
  repo-tracked example projects make line-level diffs worth the join/split.
- **[LOCKED] 3 — Dangling `src` bindings = disable-and-keep.** A source whose module was
  deleted/renamed (ID-based addressing does not rebind) compiles to a zero-reading
  register; the routing is kept, not errored away — consistent with Step 1's destination
  disable-and-keep (`apply_mod_offset_addr` no-ops on an absent module).
- **[LOCKED] 4 — Per-voice state ownership.** One shared immutable `Arc<CompiledScript>`
  (never cloned onto the `Copy` routing) + a **per-voice mutable register file** sized from
  the script's register count. State registers (`lag`/`slew`/`sah`/`phasor`) **reset on
  note-on** so a stolen/reused voice never leaks the previous note's state. **Exception:
  PRNG registers re-seed, not zero** — seeded `hash(global_seed, voice_index)` so
  simultaneous voices are decorrelated (stereo width) yet retriggers stay deterministic.

### Language semantics

- **[LOCKED] 5 — Type model.** Everything is `f32`. Truthiness: `bool = (x != 0)`.
  Comparisons yield exactly `1.0` / `0.0`; `!x` = `x == 0 ? 1 : 0`; `&&`/`||` operate on the
  same convention (eager, per the evaluation model).
- **[LOCKED] 6 — Source polarity + read timing.** Bound sources present **normalized** per
  the plan's scaling contract (output ports ±1; params via descriptor range → `0..1`,
  bipolar → `-1..1`; macros already normalized) — so authors must know each source's
  polarity (`cutoff` is `0..1`, `pan` is `-1..1`). A param read as a source reads the
  **previous block's effective (post-mod) value**, not the base — that is the point of the
  1-block latency.
- **[LOCKED] 7 — `yamsfmt` is not an optimizer.** The formatter never changes meaning, so it
  **preserves** author arithmetic (`2 * 3` stays `2 * 3`); constant folding is the
  compiler's job, internal to `CompiledScript`.

### Robustness / limits

- **[LOCKED] 8 — Concrete caps.** The compile target is a **flat bytecode VM** (not an
  AST-walk) so eval is a `for` loop over a pre-allocated register file — O(1) stack, no
  recursion. Caps (raisable later for free — scripts recompile from source, so only
  *lowering* a cap is breaking): **256 instructions, 64 registers** (≤32 sources, ≤16 state,
  ≤16 scratch), **32 source bindings, 32 nesting depth, 4 KiB source**. With no loops/
  recursion, bytecode length *is* the exact instruction count, so the cap is a **compile-time
  length gate** — a script that exceeds any cap is a compile error (routing disable-and-keep,
  source kept), never silently truncated.
- **[LOCKED] 9 — Diagnostics model.** Errors carry **spans** (the lexer/parser tag every token
  and AST node with a text range). **Report all errors, not first-only** — better for an editor and
  for LLM authoring (fix everything in one pass). Cheap here: statements are
  `separator`-separated and independent, so parse recovery = "skip to the next separator and
  continue"; semantic errors (unknown identifier, builtin-shadowing, arity mismatch) are
  collected by walking the whole AST. Compile fails if any error exists, but the full list is
  surfaced. A non-compiling script is **not** data loss — the source text is kept and the
  routing is inert until it compiles (same disable-and-keep policy as #3). Distinction: an
  unknown *function* is an error; an unknown *module* (dangling `src`) is not — it is a
  zero-register per #3.

### Architecture / process

- **[LOCKED] 10 — Crate placement.** `synth_script` (non-RT: parser + `yamsfmt` + compiler;
  depended on by GUI/MCP) produces a `CompiledScript` that lives in `synth_core` (RT type:
  flat bytecode + evaluator + register file; depended on by `synth_engine`/voice). Keeps the
  parser's heavy deps out of the audio crate. The pipeline is **source → tokens → typed AST →
  bytecode** (no CST — see #11); the RT evaluator only ever sees the bytecode.
- **[REJECTED 2026-06-15] 11 — Parser tech (was: lossless CST).** The original plan was a
  **lossless CST** (`rowan`, the library behind rust-analyzer) so `yamsfmt` preserves
  comments/trivia and editor highlighting comes nearly free. **Decided against it:** it would
  add the first third-party dependency to the deliberately dep-light `synth_script` plus a full
  parser rewrite, and the shipped **hand-written recursive-descent parser + AST-based `yamsfmt`**
  is good enough — it already handles comments and is idempotent. Known AST-formatter limits we
  accept: minimal (not author-preserved) parentheses, no 80-column wrapping, and comments
  re-indented to their following statement. **Test strategy kept:** golden-file `yamsfmt`
  idempotency (`fmt(fmt(x)) == fmt(x)`) + parser round-trip, plus **numeric snapshot tests of
  the evaluator** (guarding the `analyze_*` "offline reader sees state the engine never wrote"
  bug class).
- **[RESOLVED 2026-06-15] 12 — Pitch destinations.** *The original framing was wrong:* the
  write side (`apply_mod_offset_addr`) does **no** scaling — it forwards the normalized value
  to `module.set_mod_offset(param, value)`, and **each module hard-codes the per-target
  interpretation** (filter `cutoff` = `value × 48` semitones, etc.). So a descriptor
  `mod_scale` hint read "on the write side" never fit. The *actual* gap was that the
  oscillator's `set_mod_offset` only implemented `pitch`/`level` and **silently dropped**
  everything else — so a routing the picker offers (`osc-N.detune` / `osc-N.frequency`) looked
  valid but did nothing. **Fixed (option B, `d81697f`+`218b3f1`):** `detune` → ±1 semitone
  (its ±100¢ knob range = fine vibrato), `frequency` → ±12 semitones (coarse octave sweep),
  both folding into the existing `mod_offset_pitch` accumulator; legacy `osc-N.pitch` still
  works. Per-target scale lives as a documented `const` in the module (the de-facto `mod_scale`
  hint, no descriptor field needed). **Broader follow-up (NOT done):** other modules likely
  drop unimplemented `set_mod_offset` targets too — S1.1's "any modulatable param is a
  destination" needs a per-module audit (see plan).
