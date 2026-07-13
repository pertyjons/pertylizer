# Script modules as user-defined modules: fixed in/out ports + user knobs

Status: IMPLEMENTED — branch `feat/script-exposed-params` (both parts landed;
workspace green). In-app GUI eyeball still pending.

Origin: external architecture review (§3) — "let scripts expose parameters" —
evaluated against the code 2026-07-12. Extended the same day after discussion:
the review only asked for script-declared *inputs* (knobs); the symmetric move is
to drop the 8-slot rack in favour of **one program per module with a small,
fixed set of ports both ways** — 4 CV in, 4 CV out — plus user knobs. Together
these turn `ScriptModule` into a clean "write your own module in YAMS" node: a
normal graph node with cables in, cables out, and knobs on the faceplate.

Two parts, landed in order:

- **Part 1 — model change (fixed ports).** Replace `ScriptModule`'s 8-slot rack
  with a single program that reads up to **4 fixed CV inputs** (`in1`..`in4`) and
  writes up to **4 fixed CV outputs** (`out1`..`out4`). Cheap, all static, and it
  unifies `ScriptModule` with `AudioScriptModule` (already one-program-per-module
  with fixed stereo in/out).
- **Part 2 — user knobs.** Let a program declare named `param`s that render as
  knobs, persist, read inside the script, and become mod-matrix destinations +
  automation lanes. Shared by both script modules; rides on top of Part 1.

A second external review (DSP / systems, 2026-07-13) was validated against the
code and folded in: the `Copy`/`&'static str` constraints it flagged are already
satisfied by the `PortName` design (§3.B), and its one real gap — keeping the
cached descriptors in sync when a script's param set changes — is now specified
RT-safely in **§3.C**.

## 1. Current state (verified)

### 1a. `ScriptModule` is an 8-slot rack

`synth_modules/src/script_module.rs` is a rack of up to 8 **independent** YAMS
programs. Each slot holds its own script; each slot's `out` is a *static* output
port `out1`..`out8` (`SCRIPT_MODULE_OUTPUTS = 8`, `script_module.rs:28,90-99`).
All 8 ports always exist, even for empty slots. The rack's only unique powers:

- **in-module chaining** — slot 2 reads slot 1 via the mod-matrix source
  `scr-1.out1`, evaluated in slot order in one pass;
- reuse of the embedded `ScriptHost` (`synth_core/src/script/host.rs`, 16 slots
  wide — `SCRIPT_HOST_SLOTS = 16` — of which only the first 8 are exposed as ports).

The Voice drives it: it resolves each slot's address-based sources, then calls
`PolyModule::eval_script_slot`, which caches the slot's single value;
`process()` broadcasts each cached value across its output buffer.

### 1b. `AudioScriptModule` is already one-program-per-module

`synth_modules/src/audio_script.rs` holds **exactly one** `Arc<BoundScript>`
plus this voice's `RegisterFile`, with fixed stereo `in_l`/`in_r` → `out_l`/`out_r`
ports. It already writes **two** outputs via `out.left` / `out.right`. This is
the shape Part 1 moves `ScriptModule` toward.

### 1c. The VM already supports 4 dialect-defined outputs

`synth_core/src/script/eval.rs` captures up to `OUT_SLOTS = 4` outputs per
evaluation (`OutCapture`, `eval.rs:74-85`). `Op::StoreAudioOut(slot)` is a
**generic** multi-out store whose slot meaning is the dialect's choice
(`bytecode.rs:316` — audio: `0=left,1=right`; note_event: `0=pitch,1=vel,2=dur,
3=gate`). `eval_note` already returns all four captured slots; only the
control-rate `eval` still returns just the value-stack top (`eval.rs:161-167`).
**So 4 fixed control outputs need no VM widening — just a new dialect + a
multi-out control eval.**

### 1d. Neither script module exposes numeric params

Both `get_params()` return `Vec::new()` (`script_module.rs:128`,
`audio_script.rs:210`): *"the scripts are installed via `set_script`."* A YAMS
program reads **bound sources** (any module output/param or macro, resolved
address-based — `synth_core/src/script/bound.rs`) but cannot **declare** a value
of its own. So a user who writes a waveshaper in `audio_script` and wants a
"Drive" knob has no knob in the UI, no `DestAddr` to point a routing at
(`scr-1.drive` doesn't exist), and no automation lane. `DestAddr`
(`synth_core/src/params/mod_matrix.rs:504`) is already `(module_type, instance,
param)` and can target *any* modulatable param — so the moment a script module
**has** a param named `drive`, the mod matrix can already address it. The missing
piece is letting a script declare the param and having the module surface it.

## 2. Part 1 — one program per module, 4 fixed CV in + 4 fixed CV out

### Decision

Replace the 8-slot rack with a single program per `ScriptModule` instance that
reads up to **4 fixed CV inputs** `in1`..`in4` (cables wired *into* the node) and
writes up to **4 fixed CV outputs** `out1`..`out4` (cables *out*). Need more, or
a second independent script? Add another `Script` module — extra instances are
cheap, in the modular spirit.

Rationale (over both today's rack and a *dynamic*-port variant):

- **Static ports = no churn.** All 8 ports (4 in + 4 out) always exist;
  connections, persistence, and the node descriptor never change when the script
  is edited. This is the whole reason to fix the count rather than let ports be
  dynamic: a disappearing port is a dangling **graph edge** (cable), strictly
  worse than a dangling automation lane. Fixed ceiling sidesteps it entirely.
- **Reusable scripts.** Reading `in1`..`in4` abstractly (instead of hardcoding a
  bound source address like `lfo-1.out` in the script text) makes the same
  program reusable across instances — you rewire what feeds each input port
  without editing the script. Bound sources (`src x = lfo-1.out`) still work for
  the cases where naming the address in-script is more convenient.
- **Subsumes in-module chaining.** One program that writes `out1`/`out2`/`out3`
  shares intermediate math as ordinary locals (compute a phase once → `out1 =
  sin(phase)`, `out2 = cos(phase)`), so the common "several related signals"
  case needs no cross-slot `scr-1.out1` plumbing. Chaining between *independent*
  scripts still works, via the graph like any other module-to-module CV.
- **Unifies the two script modules.** `ScriptModule` (block-rate, 4 CV in/out)
  and `AudioScriptModule` (audio-rate, stereo in/out) become the same shape: one
  immutable `Arc<BoundScript>` + per-voice `RegisterFile`. Part 2's param
  mechanism is then genuinely shared.
- **Matches the VM.** 4 == existing `OUT_SLOTS`; the capture buffer needs no
  change.

### A. Grammar (`synth_script`)

There is no free-standing "control dialect" today: dialect is decided by
`CompileOptions`'s `audio_rate` / `note_event` bools (**both false = the default
control-rate mono-`out` dialect**; the struct also carries an unrelated
`control_rate: f32`, `compile.rs:129`, so it is *not* just two bools), shared by
mod-matrix `scr` scripts
*and* today's `ScriptModule`. The Script-module ports + multi-out must **not**
leak into mod-matrix `scr` scripts (which have no ports and produce a single
offset), so add a **third mode** — a `control_ports: bool` flag (or refactor the
pair into a `Dialect` enum). Only that mode enables `in1`..`in4` and `out1`..`out4`;
mod-matrix `scr` stays single-`out`, no ports.

- **Outputs:** `out1` .. `out4` — **bare** reserved output targets, symmetric with
  the `in1`..`in4` inputs and matching **every numbered port/name in the
  codebase**, which use a bare suffix and never a dot (today's ScriptModule
  `out1`..`out8`; note_event `in1`..`in4`; letter variants use an underscore:
  `out_l`, `in_a`). They compile to `Op::StoreOut(0..3)` — rename the
  already-generic `Op::StoreAudioOut` → `Op::StoreOut` in passing, updating the
  audio and note_event compile sites (`compile.rs:450-453`). **Those keep their
  dotted grammar** (`out.left`, `out.pitch`) — the dot is only for *named*
  channels; *numbered* outputs are bare, like everything else numbered. Bare
  `out = expr` is sugar for `out1 = expr`. Add `out1`..`out4` as reserved output
  keywords + their `OutChannel` variants + the `channel_allowed` gate for the new
  mode (`compile.rs:497-505` — the gate is named `channel_allowed`, not `is_legal`).
- **Inputs:** `in1`..`in4` read the matching CV input port. **Note:** the tokens
  `in1`..`in4` *already exist* — but only as **note_event** modulation inputs.
  This spans **two** enums: the compiler-side `SourceInput::NoteInput(u8)`
  (`compile.rs:43`) and the runtime `ScriptInput::NoteInput(u8)`
  (`bound.rs:82-105`). The control-ports mode needs a **new variant on both**
  (e.g. `SourceInput::ControlIn(u8)` → `ScriptInput::ControlIn(u8)`), distinct
  from `NoteInput`, so the two dialects resolve the same token differently. The
  compiler assigns each a fixed source register — a block-rate `ControlBindings`
  analogous to `AudioScript`'s `AudioBindings` (`eval.rs:59-68`) — which the Voice
  fills each block from the module's wired input connections (see §2.C).
- Optional per-port **display label**: `out1 "pitch"`, `in1 "rate"` — cosmetic
  only, shown on the faceplate; it does **not** change the port id (`out1`/`in1`
  stays the connection identity), so labels never cause cable churn.
- Reading/writing beyond 4 (`in5`, `out5`) is a compile error naming the ceiling.

### B. Module rewrite (`script_module.rs`)

`ScriptModule` mirrors `AudioScript`:

```rust
pub struct ScriptModule {
    script: Option<Arc<BoundScript>>, // one program (was: ScriptHost, 8 slots)
    regs: RegisterFile,               // this voice's persistent state + PRNG
    voice_index: u32,
    sources: Vec<f32>,                // voice-resolved block constants + in1..in4 + params
    bindings: ControlBindings,        // which source regs read in1..in4
    params: ScriptParams,             // declared param decls + values (Part 2)
    outputs: [f32; 4],               // last eval, broadcast in process()
    in_names: [PortName; 4],         // interned in1..in4
    out_names: [PortName; 4],        // interned out1..out4
}
```

- `descriptor()` advertises 4 fixed `in1`..`in4` CV-input ports **and** 4 fixed
  `out1`..`out4` CV-output ports (was: 8 out, 0 in), each carrying its
  script-supplied label if any. Port names and addresses are unchanged from
  today's rack (`scr-1.out1`), so any legacy `scr-1.out1`..`out4` reference keeps
  resolving.
- `scripts()` returns a 1-element slice (like `AudioScript`) so the Voice's
  slot-0 source-resolution path is uniform.
- `set_script(0, ..)` installs the single program, resizes `sources`, recomputes
  `bindings` (which registers are `in1`..`in4`), and refreshes `params.decls`
  (Part 2). Slots 1..7 are gone.
- Eval: the Voice resolves the program's block-constant sources **and** the
  `in1`..`in4` port values, evaluates it **once** into 4 outputs (new
  `CompiledScript::eval_multi` → `[Option<f32>; 4]`, the control twin of
  `eval_note`; unwritten slot → 0), caches them; `process()` broadcasts each
  `outputs[i]` across its `out{i+1}` port buffer over the whole block.

### C. Voice + engine

- `voice.rs`: replace the per-slot `eval_script_slot` drive of the Script module
  with a single-program eval producing 4 cached outputs, reusing
  `resolve_script_sources` for the address-based block constants.
- **Input-port resolution is a graph-connection lookup, not the bound-source
  path.** A bound source (`src x = lfo-1.out`) names an address *in the script
  text*; an `inN` port is a **patch cable** in the graph. To resolve `in1`..`in4`,
  look up the module's **incoming connections** — the graph already maintains
  `incoming_map: module_id → Vec<(from_module, from_port, to_port)>`
  (`graph.rs:105`) — find the edge whose `to_port` is `in{N}`, then read that
  source's **previous-block first sample** via `get_module_output(from_module,
  from_port)[0]` (`voice.rs:1237-1240`), and write it into the `ControlIn(N)`
  source register. An unwired port reads `0.0`. This needs a small **public
  accessor** for incoming connections per module (`incoming_map` is private today;
  the `connections()` iterator at `graph.rs:336` is public but O(C)). One control
  block of latency, identical to every existing bound source — no new timing
  semantics.
- Deliver the resolved block constants + input-port values via `AudioScript`'s
  existing `set_audio_block_sources` path (or a control equivalent).
- `CompiledScript::eval` grows a sibling `eval_multi` (or `eval` returns the
  capture and single-value callers read slot 0) — the mod-matrix `scr`-script
  path keeps its single-value contract via bare `out`/slot 0.

### D. Migration

No backward compatibility is required (project phase). Saved patches with a
Script module carrying per-slot scripts load **best-effort**: keep slot 0's
script as the single program, drop slots 1..7 with a `tracing::warn`. Connections
to the now-removed `out5`..`out8` ports drop the same way (unknown-port → warn,
don't panic) — the same graceful-degrade path the mod-matrix already uses for
unknown addresses. The 4 new `in1`..`in4` ports are additive (nothing to migrate).

## 3. Part 2 — user knobs (script-declared params)

Goal: let a program declare named parameters (**dynamic count**, unlike the fixed
ports — a rich node wants many well-named knobs, and a dropped knob only orphans
an automation lane, never a cable) that (1) render as knobs in the node UI
(default / range / unit / tooltip), (2) persist in the patch, (3) are readable
inside the script by name (`out = in * drive`), (4) become mod-matrix
destinations (`scr-1.drive`) + automation lanes, and (5) are readable by **other**
scripts as bound sources (`scr-1.drive`) — see §3.E. Targets **both** script
modules via one mechanism (now genuinely shared after Part 1).

### A. Declaration syntax (`synth_script`)

Top-of-program declarations (keyword `param`):

```
param drive = 0.5                                  # identifier `drive`, default 0.5, 0..1
param cutoff = 1000 [20, 20000]                    # + [min, max]
param amt = 0.5 [0, 1] "Drive" "Drives the thing"  # + display label + tooltip
out = tanh(in1 * (1 + drive * 8))
```

The bareword (`drive`) is the **identifier** — used in the code *and* as the
persistence key (`type_id`). The two string literals are optional and positional:
`"label"` then `"tooltip"`. Chosen as **(label, tooltip)** rather than
(unit, tooltip) because `ParameterDescriptor.name` is a free `String` (a good home
for a display label) while `unit: ParameterUnit` is a **closed enum**
(`None/Hertz/Decibels/…`, `module_traits.rs:511`) that can't hold an arbitrary
string. If no label is given, the display name defaults to the identifier. Unit
is deferred to a later optional keyword (map a recognized token → the enum, else
`None`).

The compiler collects these into a `Vec<ScriptParamDecl { name, default, min,
max, label, tooltip }>` carried on the compiled program / `BoundScript` (`name`
interned to a `PortName` at compile time — see §3.B for why). Inside the body a
declared `param` name resolves to a **new `ScriptInput::LocalParam(PortName)`**
variant (parallel to the `ScriptInput::ControlIn(u8)` added for input ports in
§2.A), which the compiler pins to a fixed source register. The Voice fills that
register each block from the param's **effective value** (stored/automated base +
accumulated mod offset) — the same block-constant source mechanism `in1`..`in4`
and bound sources use (§2.C). No audio-thread allocation: the register mapping is
fixed at compile time.

### B. Per-instance param store — needs a new `Param` variant

This is the one part that is **not** free. To be a *real* param (descriptor +
save/load + mod-matrix + automation, all by one mechanism) a script knob must be
representable as a [`Param`](../crates/synth_core/src/params/mod.rs) — but `Param`
is a **`Copy` enum with one typed variant per module type and no generic /
string-keyed variant** (`params/mod.rs:715`). `String` is not `Copy`, so a knob's
*name* cannot live in `Param` as a `String`. The fix:

- Add a new variant `Param::Script(ScriptParam)` with
  `ScriptParam::Knob(PortName, f32)`. `PortName` is an interned `u32` handle and
  **is `Copy`** (`types/interned.rs:180`), so `Param` stays `Copy`, and the knob's
  name travels *as the interned handle*, not a `String`. Interning happens at
  script-compile / install time (off the audio thread — `PortName::intern` locks).
  The dynamic **count** is unbounded (each knob is its own interned name); only
  the `Param`-level *type* is fixed. `AudioScript` reuses the same variant.
- **The `ScriptParam` arms of `same_kind` / `with_f32` are load-bearing — specify
  them.** `same_kind` **must compare the interned `PortName`**: it drives
  descriptor-matching (`desc.parameters.iter().any(|d| p.same_kind(&d.id))`,
  `sid_oscillator.rs:2386`, plus the GUI/automation param lookups). If two knobs
  counted as the "same kind" regardless of name, a module with ≥2 script knobs
  would collapse them all onto the first descriptor entry. `with_f32` **must
  preserve the `PortName` and replace only the f32** (`Knob(name, _)` →
  `Knob(name, value)`) — this is exactly what makes the MCP/automation value path
  "free": the setter looks up the descriptor by `type_id`, takes `desc.id`
  (`= Param::Script(Knob(name, default))`), and calls `.with_f32(value)`. `as_f32`
  returns the stored f32; the `name` arm is `port.as_str()` (below).
- **`ModuleParam::name()` returns `&'static str` — and that is free here.** The
  trait requires `fn name(&self) -> &'static str` (`module_traits.rs:682`), which
  looks like a problem for a *dynamically*-named param. It isn't: the intern pool
  **leaks** every string it interns (`types/interned.rs:161`), so
  `PortName::as_str()` already returns `&'static str` (`types/interned.rs:324`).
  The `Param::Script` arm of
  `name()` is therefore just `port.as_str()` — no `Box::leak` at the call site, no
  global name table. (This is a second reason the knob name rides as a `PortName`
  rather than a `String`, on top of `Copy`.)
- This ripples into the exhaustive `match Param` arms. The bulk are the **~7
  hand-written per-variant methods on `Param` in `params/mod.rs`** (`same_kind:824`,
  `name:985`, `as_f32:1065`, `with_f32:1145`, plus `kind` / `unit` /
  `default_curve`) — **not** `params/module_param.rs`, which is only the delegating
  `impl_module_param!` macro (`Param` is already in its list, so no edit there).
  serde is derived (free); the remaining exhaustive matches (`voice.rs`, the GUI
  param grid `gui/widgets/param_grid.rs`, …) the compiler flags. Bounded, but real
  work — budget for it.

```rust
// Fixed knob-slot ceiling (§6 decision). Lives in a shared crate (synth_core,
// beside OUT_SLOTS) so the `synth_script` compiler can enforce it at compile time
// (a 33rd `param` errors, like `in5`/`out5`) and the module can size its arrays.
const SCRIPT_MAX_PARAMS: usize = 32;

struct ScriptParams {
    // `decls` ride *inside* the immutable Arc<BoundScript> (self.script) — read,
    // never copied onto the audio thread. The owned mutable per-instance state is
    // fixed arrays that never resize, so set_script never allocates on the audio
    // thread: `len` active knobs; per slot its interned name, current value, and
    // this block's accumulated mod-offset (cleared each block).
    len: usize,                            // active knobs: slots 0..len
    names: [PortName; SCRIPT_MAX_PARAMS],  // slot → interned param name
    values: [f32; SCRIPT_MAX_PARAMS],      // slot → current knob value
    offsets: [f32; SCRIPT_MAX_PARAMS],     // slot → mod-offset (per block)
}
```

- `descriptor()` appends one `ParameterDescriptor` (note: the real type name is
  `ParameterDescriptor`, `module_traits.rs:719`) per declared param —
  `type_id = name`, `name = label` (or the identifier), `description = tooltip`,
  `range` from `[min,max]`+default, `id = Param::Script(Knob(interned_name, val))`,
  `modulatable = true` — **derived from the installed script**, per-instance.
- `set_param` / `get_param` / `get_params` read/write `values` by the interned
  `PortName`. Save/load already serialises `desc.parameters` into the module's
  `parameters` map keyed by `type_id` (`resave_examples.rs:78`), so persistence
  works once these are real params — **but with a load-order constraint**: the
  script must be **installed before** its param values are applied, or the
  descriptor won't yet list the params to match them to. Sequence `set_script`
  ahead of the parameter restore in the loader.
- `set_script` swaps in the new `Arc<BoundScript>` (which carries the new decls),
  then **remaps the fixed `names`/`values` arrays in place** from the new decls:
  for each new slot keep the old value if a knob of the same name survived
  (editing a script doesn't reset unrelated knobs), else take the decl default. It
  is a bounded ≤`SCRIPT_MAX_PARAMS`² name scan over fixed arrays — no heap, RT-safe.
  `offsets` are zeroed (they re-accumulate next block). Because the arrays are
  fixed-size, nothing is resized or reallocated — so unlike the descriptor, the
  knob store needs **no** off-thread build and **no** command payload (§3.C).

### C. Keeping the cached descriptors in sync on recompile (RT-safe)

**Scope — this fires on a *script-text* edit, never on a param *value* change.**
Turning a knob, automating it, modulating it from another script, or reading
`scr-1.drive` from another script are all **value-level** operations on the
already-compiled program: they take the ordinary `set_param` / mod-offset / source
paths (`EngineCommand::SetModuleParameter`, `synth_engine.rs:1110`) and **never
recompile or rebuild the descriptor** — exactly like turning a filter's cutoff
never rebuilds the filter. The refresh below is triggered *only* by re-applying
edited **script source** (`EngineCommand::SetModScript`, `synth_engine.rs:1118`),
a human-speed action, and *only* because that is the one moment the *set* of
declared params can change. If the edit didn't change the param set (e.g. tweaking
the formula), the rebuilt descriptor is identical and the swap is a harmless no-op.

A script's param set is **per-installed-script**, so every cache that was filled
*once* from the descriptor at module-add time goes stale the moment a script is
edited to add, rename, or drop a `param`. Three such caches exist, and if any is
left stale the knob silently misbehaves:

1. **Session registry descriptor** (`session.rs:135`,
   `registry: Mutex<HashMap<(InstrumentId, ModuleId), ModuleDescriptor>>`) — feeds
   the GUI knob grid, MCP param discovery, and the **save path** (a param absent
   here is dropped on save). Lives on the **UI/MCP thread**.
2. **Graph-node descriptor** (`graph.rs:66`, `GraphNode.descriptor`, cached per
   voice-graph) — read on the **audio thread** by `resolve_param_source`
   (`voice.rs:1265`) for cross-script reads (§3.E) and mod-destination resolution.
   A new param absent here → `scr-1.drive` resolves to `0.0`.
3. **Module-internal knob store** (the fixed `ScriptParams` arrays inside
   `Box<dyn PolyModule>`, §3.B) — the `SCRIPT_MAX_PARAMS`-slot `names`/`values`/
   `offsets` arrays that back `set_param`/`get_param`/`set_mod_offset` alloc-free.
   A param missing a slot → the mod matrix has nowhere to write. **Handled in
   place, not by swap:** the arrays are pre-sized at module-add and `set_script`
   reindexes them from the new decls on the audio thread without allocating (§3.B).
   So this cache needs no off-thread build and no command payload — only the two
   **descriptor** caches (1)+(2) below do.

(Output *buffers* are **not** in this list either: the 4 `out1`..`out4` ports are
fixed, so `GraphNode.outputs` — built from `descriptor.ports` — never changes on a
script edit. Only the param list moves.)

**RT-safety — the descriptor is built off-thread and swapped; defer-drop the old.**
Only caches (1)+(2) are `ModuleDescriptor` values whose rebuild allocates
(Strings/Vecs), so they can't be rebuilt in the audio-thread `set_script`. The
reviewer's suggested `node.descriptor = node.module.descriptor()` is right in
intent but wrong in placement: `set_script` runs on the **audio thread**
(`handle_set_mod_script`, `synth_engine.rs:2220`, applied to the template graph
**and every live voice**), and `descriptor()` allocates. So build the fresh
descriptor **off-thread** — in `session.rs::set_mod_script`, right where the
`BoundScript` is already compiled — and carry it into the engine alongside the
script. On the audio thread the handler only **swaps** (`mem::replace`) the new
descriptor into each graph node and ships the replaced ones to a trash channel for
deferred drop on the main thread — mirroring the existing `script_trash`
deferred-drop for replaced `Arc<BoundScript>` (`synth_engine.rs:2265`). **The
existing trash ring is typed `HeapRb::<Arc<BoundScript>>` (`synth_engine.rs:600`),
so a `ModuleDescriptor` cannot be pushed into it** — "reuse the `script_trash`
path" means mirror the *mechanism*, not share the ring. Add a **second**
deferred-drop ring typed `ModuleDescriptor` (or a shared trash enum, or box the
descriptor in an `Arc` and reuse a generic ring). Concretely:

- Extend `EngineCommand::SetModScript` (or add a sibling command) to carry the
  prebuilt `ModuleDescriptor` next to `script: Option<Arc<BoundScript>>`. The
  descriptor is voice-independent, so build it once off-thread and clone it per
  target (template + each voice node) off-thread — never rebuilt in `process()`.
  (The knob store carries **nothing**: `set_script` remaps its fixed arrays in
  place, §3.B.)
- `session.rs::set_mod_script` / `clear_mod_script` additionally
  **`register_descriptor`** the fresh descriptor into the session registry
  (`session.rs:918`) after a successful compile, closing cache (1).
- The audio-thread handler swaps cache (2) per node, trashing the old descriptor;
  cache (3) is remapped in place by `set_script` — no trash, no payload.

**Transient syntax errors already preserve the last-working knobs.**
`set_mod_script` compiles **before** it sends (`session.rs:1086`): a compile error
returns `Err` and no command is sent, so the engine keeps the previous script,
its params, and every mod/automation binding intact while the user is mid-edit.
Only a **successful** edit that intentionally removes a `param` breaks that param's
bindings — and that degrades gracefully (drop the orphaned automation lane + warn,
per §6), never panics.

**Mod-matrix destination + automation** (once the caches above are fresh):

- The resolver walks a module's modulatable params to apply an offset
  (`PolyModule::set_mod_offset`; cf. `oscillator.rs:807` and the generic
  `ParamModOffsets` store). Give the script modules `set_mod_offset(name, value)`
  adding the offset into the effective value of the matching declared param
  (base + accumulated offset, cleared each block like every other module).
- **RT-safe target matching — match by a cached `&'static str`, not
  `PortName == &str`.** `set_mod_offset` runs on the **audio thread**, once per
  active routing per block, with a `&str` target: the graph already resolves it via
  `node.module.set_mod_offset(addr.param.as_str(), value)` (`graph.rs:521`), which
  pays *one* `PortName::as_str()` read-lock on the intern pool. Inside the module,
  matching that `&str` against the store's interned `names` via the `PortName ==
  &str` impl (`types/interned.rs:333`) would take **another** read-lock *per stored
  knob per block* (every `as_str()` locks). Avoid it: have each `ScriptParamDecl`
  cache its resolved `&'static str` name (from `as_str()` at compile time,
  off-thread) and match `target == name_str` — plain lock-free `str` equality,
  exactly how the generic `ParamModOffsets` store matches its owned `String`
  type_ids today. (Passing the `PortName` through the trait for a lock-free `u32`
  compare is cleaner still, but breaks the existing `&str` `set_mod_offset`
  convention — out of scope for v1.)
- `DestAddr::parse("scr-1.drive")` already works structurally; add the declared
  params to whatever enumerates a module's modulatable targets (the descriptor
  param list feeds this) so the picker and `get_instrument_automation_targets`
  show them.

### D. GUI

The node view renders module params generically from the descriptor, so declared
params appear automatically once (B) is done. Verify the ƒx/script editor popup
(`7dbbd75`) shows a live list of the params the current script declares, and that
the knobs sit on the node faceplate.

### E. Read by other scripts (free)

Because a declared param becomes a **real module param in the descriptor**, other
scripts can read it as a bound source `scr-1.drive` with **no extra work** — the
Voice's existing `resolve_param_source` (`voice.rs:1260`) already reads any
module param by name and normalizes it through its descriptor range+curve to
`0..1`, the same path that reads `flt-1.cutoff`. So a Script module doubles as a
shareable "macro knob": one script exposes `param drive` on its faceplate, any
other script (or Mod Matrix source) reads `scr-1.drive`. Value semantics match
every other param source — it reads the **stored/automated** value (0..1
normalized), *not* the transient per-block mod-matrix offset (which is applied
and cleared separately), so there is no surprising double-count. One control
block of latency, like all bound sources.

## 4. Real-time safety

- **Part 1 in/out** are block-constant control signals. Each `in1`..`in4` is
  resolved once per block (previous-block value of the wired source, off the hot
  loop); each `out1`..`out4` is one eval per block into 4 cached floats,
  broadcast across the buffer (identical to today's rack).
- **Part 2 params** are block-constant: resolved into the script's param
  registers once per block (off the hot loop), exactly like macros. Mod-offsets
  accumulate into a pre-allocated store, cleared per block.
- **The install path stays off the audio thread.** The only allocation Part 2
  introduces (rebuilding the `ModuleDescriptor` when the param set changes) happens
  in `session.rs` after the off-thread compile; the audio thread only swaps the
  pre-built descriptor in and defers the drop of the old (§3.C). The knob store is
  fixed-size (`SCRIPT_MAX_PARAMS`), so `set_script` remaps it in place with no
  allocation. `process()` and the per-block param resolution never allocate.
- **Zipper noise (documented behaviour, not a bug).** A param value is
  block-constant, so under fast modulation or automation it **steps** at each
  block boundary — and in `audio_script` that step is held across the whole block
  of per-sample evaluation, which can click on a steep param (a filter cutoff, a
  gain). v1 does **not** smooth params implicitly; document that an `audio_script`
  user who wants a click-free knob smooths it *inside* the script, per-sample (a
  one-pole: `s = s + (drive - s) * 0.005; out.left = in_l * s`, using a `state`
  cell), and treat a built-in `smooth()`/slew helper as a possible later addition
  (§6). **Both the ports *and* the knobs on the block-rate `ScriptModule` are
  block-constant** — its `in1`..`in4` are the *previous* block's value held flat
  across this block (§2.C), so they do **not** give audio-rate tracking either.
  Sample-accurate tracking exists only on the audio-rate `AudioScript` module, via
  its per-sample `in_l` / `in_r` inputs (not `in1`..`in4`); reach for that when a
  signal must track *within* the block rather than a block-constant knob.

No allocation, no locking in `process()` for any part.

## 5. Files to touch

- `crates/synth_core/src/params/` — **new `Param::Script(ScriptParam)` variant**
  (`mod.rs`) with `ScriptParam::Knob(PortName, f32)`; add the ~7 per-variant arms in
  `mod.rs`'s hand-written methods (`same_kind` / `name` / `as_f32` / `with_f32` /
  `kind` / `unit` / `default_curve`) — **not** `module_param.rs` (delegating macro
  only) — plus every other exhaustive `match Param` the compiler flags (`voice.rs`,
  GUI param grid). `same_kind` compares the `PortName`; `with_f32` keeps it.
- `crates/synth_core/src/script/` — `bytecode.rs` (rename `StoreAudioOut` →
  generic `StoreOut`), `eval.rs` (add `eval_multi` control multi-out + a
  block-rate `ControlBindings` for `in1`..`in4`), plus the param-register
  plumbing into `bound.rs`/`eval.rs` (Part 2).
- `crates/synth_script/` — grammar + compiler: new `control_ports` mode/dialect
  (bare `out1`..`out4` → `OutChannel::Out(u8)`; `in1`..`in4` →
  `ScriptInput::ControlIn(u8)`, distinct from the note_event `NoteInput`; optional
  port labels), then `param` decls carried as `ScriptParamDecl`s (name interned,
  label, tooltip) mapped to registers. Touches `ast.rs`, `parser.rs`, `compile.rs`,
  `fmt.rs`, `symbols.rs`.
- `crates/synth_engine/src/graph.rs` — small **public accessor** for a module's
  incoming connections (back the `incoming_map`) so the Voice can resolve
  `in1`..`in4`.
- `crates/synth_modules/src/script_module.rs` — full rewrite to the one-program /
  4-in / 4-out shape (mirror `audio_script.rs`); then the fixed-`SCRIPT_MAX_PARAMS`
  knob store (`names`/`values`/`offsets` arrays, in-place remap on `set_script`) +
  dynamic `descriptor()` params + `set/get_param` + `set_mod_offset` +
  `Param::Script`-arm of `name()` → `port.as_str()`. `audio_script.rs` gets only
  the Part 2 param additions.
- `crates/synth_engine/src/commands.rs` — extend `EngineCommand::SetModScript`
  (or add a sibling) to carry the **prebuilt** `ModuleDescriptor` next to the
  `Arc<BoundScript>` (§3.C), so the audio thread swaps instead of rebuilds. (The
  knob store carries nothing — it is remapped in place.)
- `crates/synth_engine/src/synth_engine.rs` — `handle_set_mod_script` swaps the
  cached descriptor into the template + every voice node and ships the replaced
  ones to a **new** deferred-drop trash channel typed `ModuleDescriptor` (mirror
  the existing `Arc<BoundScript>` `script_trash` path, `synth_engine.rs:2265,600` —
  the descriptor can't share that ring's element type).
- `crates/pertylizer/src/session.rs` — in `set_mod_script` / `clear_mod_script`,
  build the fresh descriptor off-thread after the compile and `register_descriptor`
  it into the session registry (§3.C, cache 1).
- `crates/synth_engine/src/voice.rs` — single-program eval for the Script module
  (resolve `in1`..`in4` from wired connections via the graph accessor + block
  constants → 4 cached outputs); fill script param registers per block; resolve
  `scr-N.<param>` mod destinations. Also the **loader ordering** (`set_script`
  before param restore).
- MCP: ensure `get_module_info` / automation-target listing surface the dynamic
  params; `set_parameter` already dispatches by name. Note the new port layout
  (4 in / 4 out, was 0 in / 8 out) in the module description.
- GUI: node faceplate + script editor render the 4 in / 4 out ports and the
  declared knobs.

## 6. Open questions

- **Port count.** 4 out matches `OUT_SLOTS` (no VM widening); 4 in mirrors it.
  Confirm 4 is the right ceiling both ways, or whether the common case is ≤2
  (could start narrower and widen later — but widening a static port set is
  itself a mild compat break).
- **Per-instance descriptors (Part 2).** The cache-sync mechanism is specified in
  §3.C (rebuild off-thread → swap → defer-drop, across the session registry,
  graph-node descriptor, and `ParamModOffsets`). The remaining open point is the
  *automation* interaction: confirm `rebuild_instrument_preserve_automation` and
  the automation lanes degrade gracefully when a script edit **removes** a param a
  lane was bound to (drop lane + warn, don't panic).
- **Knob-store capacity — DECIDED: fixed cap `SCRIPT_MAX_PARAMS = 32`.** A "param
  slot" is the audio-thread storage where **one knob** rests — its interned name,
  current value, and per-block mod-offset. This is a **separate** ceiling from the
  4 CV in / 4 CV out cable ports (Part 1): it bounds *how many knobs one script
  module may declare*, not cables. Chosen fixed (array-backed, `= 32`) over an
  unbounded `Vec`: the arrays are allocated once at module-add and never resize, so
  `set_script` remaps them in place with no audio-thread allocation and the command
  needs no knob-store payload — only the descriptor is built off-thread and swapped
  (§3.B/§3.C). The descriptor still lists **only the declared knobs** (never 32
  empty ones), so the cap is invisible in the GUI/save/MCP surface; a script
  realistically never needs >32 knobs. Declaring a 33rd `param` is a compile error
  naming the ceiling (like `in5`/`out5`). Revisit only if a real patch wants more.
- **Built-in param smoothing.** v1 leaves click-free knobs to user-side smoothing
  in the script (§4). A built-in `smooth(x, coeff)` / slew helper (or a
  `param … smooth` modifier) would remove the boilerplate; defer unless the manual
  one-pole proves too fiddly in practice.
- **Param metadata scope.** v1 carries `default` + optional `[min,max]` +
  optional `"label"` + optional `"tooltip"` (positional strings → `name` /
  `description`). Unit (a `ParameterUnit` enum keyword), bipolar-vs-unipolar, and
  response curve are deferred (default linear, unipolar unless the range spans
  negative-to-positive).
- **Audio-script `first_sample`.** Declared params are block-constant; document
  that they update per block, not per sample (fine for knobs, not for audio-rate
  cross-mod — use a real input port for that).

## 7. Exit gate

Part 1 (model change):

- A `Script` module shows **4 CV-in** (`in1`..`in4`) and **4 CV-out**
  (`out1`..`out4`) ports; a program with `out1 = in1 * in2` /
  `out2 = in3 * 0.5` feeds outputs from one script; unwritten outputs and
  unwired inputs read/emit 0.
- A cable wired `lfo-1.out → scr-1.in1` is read as `in1` inside the script
  (previous-block value), and rewiring it changes the input without editing the
  script text.
- One program's shared locals feed multiple outputs (no cross-slot plumbing).
- Loading an old multi-slot patch keeps slot 0 and warns on the rest without
  panicking; cables to removed ports drop gracefully.

Part 2 (user knobs):

- A YAMS script with `param drive = 0.5` shows a **Drive** knob on the node.
- Turning it changes the sound; the value saves/reloads with the patch.
- A mod-matrix routing `lfo-1.out → scr-1.drive` modulates it; an automation lane
  can target it.
- Another script reads `src x = scr-1.drive` and gets the knob's `0..1` value.
- A param declared with `"Drive" "tooltip text"` shows **Drive** as the label and
  the tooltip on hover.
- Both `script` and `audio_script` support params via the same mechanism (one
  shared `Param::Script` variant).
- `Param` still derives `Copy` (the knob name rides as an interned `PortName`).
- Editing a **live** script (voice playing) to add a `param` makes the knob
  appear in the UI, become a mod-matrix/automation target, and save — with no
  audio glitch and no reload (caches refresh via the off-thread build + swap of
  §3.C); a **syntax error** mid-edit keeps the previous knobs and bindings intact.
- Editing the script to rename/remove a param preserves surviving knob values and
  drops orphaned automation without panicking.

Both: workspace green (`build` / `clippy --all-targets` / `test` / `fmt --check`).
