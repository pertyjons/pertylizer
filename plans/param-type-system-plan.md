# Parameter Type System — Full Restructuring Plan

> **Status:** Review-complete (3 review passes; all 6 decisions resolved, §14). Ready
> to implement — no code written yet.
> **Scope:** Maximal — a first-class value-kind model threaded through every layer
> (engine → descriptor → serialization → schema/MCP → GUI), plus newtype-derived
> *unit* (curve stays explicit — §14.6) to kill descriptor drift, and a shared
> parameter trait.
> **Supersedes:** `plans/TODO.md` §3.6 (integer marker), folds in §3.5 (MSEG knob
> awkwardness — partially), §4.2 (descriptor drift lint — made obsolete by Phase 2a).

> **Revision log — pass 1 (internal review).** (a) enum count fixed 42→**68** (67
> per-module + aggregate); (b) `ScalarParam` made genuinely drift-proof by *binding
> the value* and calling a method, not turbofishing the type name (§1.2/§1.3);
> (c) curve-derivation recognized as **behavioral, not cosmetic** and split out of
> the unit derivation (Phase 2 → 2a/2b, with curve defaulting to *not derived*);
> (d) bucket counts marked provisional — they don't yet reconcile `Reference` (Phase
> 0 produces the authoritative split); (e) Phase 1 kind/choices invariant corrected;
> (f) broken cross-refs fixed.
>
> **Revision log — pass 2 (external review).** (g) discrete params confirmed
> `.modulatable(false)` in `mseg.rs`/`amplifier.rs` → `is_automatable` tightening
> verified safe (risk downgraded, Phase 2b); (h) `impl_scalar_param_enum!` decl-macro
> added for the choice-enum impls (Phase 1); (i) the 5 original open decisions
> **resolved** (§14); (j) `SampleId` wire-shape normalized to the struct form (Phase
> 4); (k) integer MCP validation made explicit: round fractional *but reject
> out-of-range* + echo (Phase 5).
>
> **Revision log — pass 3 (external review of the split plan).** (l) the drift-proof
> bound-value dispatch and the 2a/2b split both endorsed; (m) decision §14.6
> **RESOLVED** — external review independently confirmed the internal recommendation:
> do **not** auto-derive `curve` (automation-recall regression + context-specific
> curves). All six decisions now resolved. (n) Added the §15 implementation
> checklist. The plan is review-complete.

---

## 0. Problem statement

Pertylizer has **378 parameters across 68 enums** (67 per-module `*Param` enums +
the aggregate `Param`, which has 67 variants — `crates/synth_core/src/params/mod.rs:669`;
counts from the Phase 0 audit, [`docs/param-kinds.md`](../docs/param-kinds.md)). Every
parameter carries a
*properly typed* value in the engine — `Frequency(Hertz)`, `SegmentCount(u8)`,
`LoopEnabled(bool)`, `Waveform(...)` — but **all type information is flattened to
`f32` at the `as_f32()` boundary** and never recovered downstream. The "kind" of a
value (continuous / integer / bool / enum / reference) is then *re-guessed*, badly
and inconsistently, by every layer below:

| Layer                            | How it guesses "kind" today                                                               | Failure mode                                                       |
|----------------------------------|-------------------------------------------------------------------------------------------|--------------------------------------------------------------------|
| Descriptor                       | `choices.is_some()` ⇒ enum; `step == Some(1.0)` ⇒ integer; `widget_hint == Toggle` ⇒ bool | three different proxies for one fact                               |
| Serialization (`patch.rs`)       | always `Float` except hand-special-cased `Choice`/`SampleId`                              | int/bool saved as `5.0`/`1.0`, files not self-describing           |
| Schema (`gen_schemas.rs`)        | always `"type": "number"`                                                                 | LLM/MCP client sends `4.0`, not `4`; no `integer`/`boolean`        |
| GUI (`knob.rs`, `param_grid.rs`) | `unit`/`step`/`widget_hint`                                                               | integer knobs show `"4.00"`; sliders ignore `step`; no scroll-step |

Additionally, `unit`, `response_curve`, `widget_hint` and `range` are **four
independent, hand-declared descriptor fields** that are *not* derived from the
parameter's newtype. A `Hertz` param can legally be declared with
`ParameterUnit::Milliseconds` and `ResponseCurve::Linear` — the drift that
`plans/TODO.md` §4.2 wants to lint against is structural, not accidental.

### The core finding

There are only **five value-kinds** in the entire engine. The newtypes
(`Hertz`, `Gain`, `Cents`, …) are **units within `Continuous`**, not separate
kinds. No further kinds are needed (see §1.4 for the proof against more).

```
Continuous  f32 within a range, with a response curve + unit   (newtype scalars + 2 raw f32)
Integer     discrete count / index (u8 / i32 / usize)
Bool        two-state on/off
Enum        finite named set (index into `choices`)
Reference   opaque id / address outside the value scale        (SampleId + mod-matrix addresses)
```

> **Authoritative counts — Phase 0 is DONE** ([`docs/param-kinds.md`](../docs/param-kinds.md)).
> Per-variant audit of all 67 enums (AST-verified): **378 parameter variants** —
> Continuous 307 (81.2 %), Enum 31 (8.2 %), Bool 20 (5.3 %), Integer 17 (4.5 %),
> Reference 3 (0.8 %). The earlier provisional tally (≈295/107/19/18/2 = 441) was
> wrong: it had no `Reference` bucket and **over-counted `Enum` (~107 → actually 31)**.
> The audit
> also surfaced that **integer-backed newtypes** (`VoiceCount`, `MidiNote`, `Octaves`,
> `StepCount`) are `Integer`, not `Continuous` (5 variants reclassified). `Reference`
> is exactly 3: `SampleSelect`, `SlotSource`, `SlotDestination`. See the doc for the
> full per-enum tables and the `ScalarParam::KIND` assignment list for Phase 1.

The fix is to make **kind a first-class, engine-derived fact** and have every
downstream layer *consume* it instead of re-deriving it.

---

## 1. Target architecture

### 1.1 `ParamKind` — the single classifier

A flat enum in `crates/synth_core/src/module_traits.rs`, next to `ParameterUnit`:

```rust
/// What kind of value a parameter holds. The single authoritative classifier,
/// derived from the engine variant's backing type — never hand-declared.
/// `Serialize` only: it is emitted into `descriptors.json` and onto the MCP wire,
/// but never read back from a file (descriptors are code, not persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ParamKind {
    /// f32 within `range`, shaped by `response_curve`, displayed with `unit`.
    Continuous,
    /// Discrete integer (count or index). `range` carries min/max; step is 1,
    /// curve is forced linear, display is decimal-free.
    Integer,
    /// Two-state. Rendered as a toggle; serialized as a JSON bool.
    Bool,
    /// Finite named set; the value is an index into `choices`.
    Enum,
    /// Opaque id / address that lives outside the numeric scale (sample id,
    /// mod-matrix address). Deliberately coarse — serialization and the picker
    /// widget stay variant-specific (Phase 4 / §7), since a sample-id reference
    /// and a mod-matrix address need different UI and wire shapes. `Reference`
    /// only flags "not a plain number" for GUI/schema; if a third reference type
    /// ever appears, revisit whether a `Reference(RefKind)` sub-tag is warranted.
    Reference,
}
```

**Design choice — flat classifier, not a payload-carrying enum.** The user's
sketch put `unit`/`curve` *inside* `Continuous { unit, curve }`. We deliberately
keep `unit`, `response_curve`, `range`, `choices`, `step` as the existing
`ParameterDescriptor` fields and add `kind` as **one additive field**. Reasons:

1. Minimal churn — every consumer that reads `descriptor.unit` keeps working.
2. `kind` is purely a *classifier*: it says which existing fields are meaningful
   and how to interpret the value. `unit` is a sub-attribute that is only
   meaningful for `Continuous`/`Integer`.
3. The invariant `kind == Enum ⇔ choices.is_some()` becomes a testable assertion
   rather than two representations of the same thing.

`ParameterDescriptor` gains exactly one field (`module_traits.rs:596`):

```rust
pub struct ParameterDescriptor {
    // ... existing fields ...
    pub step: Option<f32>,
    /// Value-kind classifier. Seeded from `id.kind()` by the constructors,
    /// so it can never drift from the engine's backing type.
    pub kind: ParamKind,
}
```

### 1.2 `ScalarParam` — kind/unit/curve derived from the value type

The strongly-typed core. A trait implemented **once per value type** that can
appear inside a `Param` variant. Because the kind comes from the *type*, it
cannot drift from the backing field:

```rust
/// Implemented by every value type a `Param` variant can carry. Provides the
/// type-derived metadata so descriptors never hand-declare kind/unit.
///
/// The provided methods (`scalar_kind`/`scalar_unit`/`scalar_curve`) are what the
/// per-enum `kind()` arms call on a **bound value** — see §1.3. Calling a method on
/// the bound field (not turbofishing the type name) is what makes the metadata
/// genuinely impossible to drift from the backing field: change the field type and
/// the method resolves to the new type automatically.
pub trait ScalarParam {
    const KIND: ParamKind;
    /// Natural display unit (overridable per descriptor for e.g. a
    /// NormalizedValue shown as Percent).
    const UNIT: ParameterUnit;
    /// Suggested response curve for this physical quantity. **Advisory only** —
    /// NOT auto-applied by the descriptor constructors (curve is behavioral; see
    /// the note below and Phase 2b). Used by the Phase 2b audit as the suggested
    /// value, not silently forced onto every param.
    const DEFAULT_CURVE: ResponseCurve;

    // Provided methods — call these on a bound value for drift-proof dispatch.
    #[inline]
    fn scalar_kind(&self) -> ParamKind { Self::KIND }
    #[inline]
    fn scalar_unit(&self) -> ParameterUnit { Self::UNIT }
    #[inline]
    fn scalar_curve(&self) -> ResponseCurve { Self::DEFAULT_CURVE }
}

impl ScalarParam for Hertz {
    const KIND: ParamKind = ParamKind::Continuous;
    const UNIT: ParameterUnit = ParameterUnit::Hertz;
    const DEFAULT_CURVE: ResponseCurve = ResponseCurve::Logarithmic;
}
impl ScalarParam for Decibels { /* Continuous, Decibels, Linear   */ }
impl ScalarParam for Seconds { /* Continuous, Seconds,  Exponential */ }
impl ScalarParam for Milliseconds { /* Continuous, Milliseconds, Exponential */ }
impl ScalarParam for Gain { /* Continuous, None,     Squared   */ }
impl ScalarParam for Cents { /* Continuous, Cents,    Linear    */ }
impl ScalarParam for Semitones { /* Continuous, Semitones,Linear    */ }
impl ScalarParam for NormalizedValue { /* Continuous, None, Linear  */ }
impl ScalarParam for BipolarValue { /* Continuous, None, Linear  */ }
impl ScalarParam for Phase { /* Continuous, None,     Linear    */ }
// integer-backed primitives:
impl ScalarParam for u8 {
    const KIND = Integer;
    const UNIT = None;
    const DEFAULT_CURVE = Linear;
}
impl ScalarParam for i32 { /* Integer, None, Linear */ }
impl ScalarParam for usize { /* Integer, None, Linear */ }
// boolean:
impl ScalarParam for bool { const KIND = Bool; /* ... */ }
// references:
impl ScalarParam for SampleId { const KIND = Reference; /* ... */ }
impl ScalarParam for Option<SrcAddr> { const KIND = Reference; /* ... */ }
impl ScalarParam for Option<DestAddr> { const KIND = Reference; /* ... */ }
// each module-local choice enum (Waveform, FilterMode, ...):
impl ScalarParam for Waveform { const KIND = Enum; /* ... */ }
```

> **Important — what is derived, and what is NOT.**
> - **`unit` IS derived** (Phase 2a) — it is *type-specific and cosmetic*: `Hertz` is
    > always "Hz". Deriving it removes hand-typing and kills the display-unit drift
    > §4.2 worried about. Overridable via `.unit()` (e.g. a `NormalizedValue` shown as
    > `Percent`).
> - **`curve` is NOT auto-derived** (see Phase 2b). A response curve is *behavioral*
    > (it reshapes `normalize`/`denormalize`, which knob feel, automation, and the
    > headless exporters all depend on) and is often *param-specific, not
    > type-specific* (two `Seconds` params — an envelope attack vs a delay time — may
    > legitimately want different curves). Auto-seeding `curve` from the type in
    > `float()` would **silently flip** every param that currently relies on the
    > `Linear` default to the type's curve — a behavioral change. `DEFAULT_CURVE` is
    > therefore advisory input to a Phase 2b audit, not an automatic override.
> - **`range` is NEVER derived** (see §1.4) — it is context-specific (`Hertz` has four
    > range presets). Stays explicit.

### 1.3 Per-enum and aggregate `kind()` and `unit()` (`default_curve()` in Phase 2b)

Each of the 67 module enums gains a `kind()` that **binds the carried value** and
calls `scalar_kind()` on it — so the kind follows the *actual field type*, not a
hand-named type. (Co-located with the existing `with_f32`,
e.g. `crates/synth_core/src/params/mseg.rs:78`):

```rust
impl MsegParam {
    pub fn kind(&self) -> ParamKind {
        match self {
            // Bind the value (`v`) and dispatch on it — NOT `u8::KIND`. If a field's
            // type changes (e.g. u8 → i32, or Seconds → Milliseconds), the kind
            // follows automatically; a turbofished `u8::KIND` would silently lie.
            Self::SegmentCount(v) | Self::SustainSegment(v)
            | Self::LoopStart(v) | Self::LoopEnd(v) => v.scalar_kind(), // u8 → Integer
            Self::LoopEnabled(v) => v.scalar_kind(), // bool → Bool
            Self::TimeScale(v) => v.scalar_kind(),
            Self::SegmentTime(_, v) => v.scalar_kind(), // value field, not the index
            Self::SegmentLevel(_, v) => v.scalar_kind(),
            Self::SegmentCurve(_, v) => v.scalar_kind(),
        }
    }
    // unit() follows the same shape via `v.scalar_unit()`. (No `default_curve()`
    // delegation is wired into the constructors — curve is not auto-derived; §1.2.)
}
```

> Two-field index+value variants (`SegmentTime(u8, Seconds)`,
> `SlotSource(u8, Option<SrcAddr>)`) bind the **value** field, ignoring the
> structural index — the index is not a parameter value.

The aggregate `Param` delegates exactly like the existing `same_kind`/`as_f32`
matches (`params/mod.rs:774`): `Param::kind()` and `Param::unit()` are giant
per-variant delegation matches added in Phase 1 (`Param::default_curve()` is added
only in Phase 2b, where the curve audit needs it — it is not used by the descriptor
constructors). **This match is what enforces completeness**: a new module enum that
forgets `kind()` is a compile error in `Param::kind()`. (This is also why the
`ModuleParam` trait in Phase 7 is *nice-to-have*, not load-bearing — the aggregate
match already forces presence.)

### 1.4 Why no more kinds — and why `range` stays explicit

- **Units are not kinds.** All 295 newtype params are `Continuous`; their newtype
  is the *unit*, an attribute. Adding a kind per newtype would be 20+ redundant
  kinds for one behavior.
- **`range` is context-specific, not type-specific.** Proof: `Hertz` ships **four**
  distinct range presets — `OSC_RANGE` (1–20 000), `LFO_RANGE` (0.01–50),
  `FILTER_RANGE` (20–20 000), `CROSSOVER_RANGE` (`crates/synth_core/src/types/frequency.rs:70-89`).
  A single per-newtype range would be wrong three times out of four. **Derivation
  must never touch `range`** — only `kind`/`unit`/`curve`.
- **`Reference` already covers the two opaque cases** (sample id, mod-matrix
  address). No separate `Address` kind is justified; the *serialization* of a
  Reference stays variant-specific (Phase 4 / §7), so the kind need only flag "not a plain
  number" for GUI/schema.

**Conclusion for the reviewer:** the five-kind taxonomy is complete. The audit in
Phase 0 is the falsification test — if any of the 441 params doesn't fit a kind, the
taxonomy is revised before Phase 1.

### 1.5 Relationship to the existing `ParamValue` — not a duplicate

`ParamKind` looks almost 1:1 with `ParamValue`
(`crates/pertylizer/src/patch.rs:367`):

| `ParamValue` variant | `ParamKind`  |
|----------------------|--------------|
| `Bool(bool)`         | `Bool`       |
| `Int(i32)`           | `Integer`    |
| `Float(f32)`         | `Continuous` |
| `Choice(String)`     | `Enum`       |
| `SampleId { .. }`    | `Reference`  |

`ParamKind` is essentially the **discriminant of `ParamValue` lifted into the
engine crate and refined**. They are intentionally *not* merged into one type, for
three reasons the reviewer should weigh:

1. **Dependency direction.** `ParamValue` lives in the `pertylizer` app crate
   (serialization). `ParamKind` must live in `synth_core` so the engine can
   *derive* it (`Param::kind()`) and descriptors (also in `synth_core`) can carry
   it. `synth_core` cannot depend on `pertylizer`, so it cannot reference
   `ParamValue`. `pertylizer` depends on `synth_core`, so it *can* reference both —
   the bridge is one-directional by construction.
2. **Classifier-of-a-slot vs value-instance.** `ParamKind` is a property of a
   *parameter* (a descriptor has one kind forever); `ParamValue` is a concrete
   runtime value. Schema generation and GUI widget selection need to ask "what kind
   is this parameter?" *without a value in hand* — `ParamValue` always carries a
   payload.
3. **`ParamValue` variants don't cleanly partition the kinds.** `Choice(String)` is
   used today for **both** real enums **and** mod-matrix addresses — it straddles
   `Enum` *and* `Reference`. `Float` holds both `Continuous` and legacy integers.
   So the discriminant is *coarser* than the kind model; `ParamKind` is the finer,
   correct partition.

**The coherent three-way story:** `Param` (synth_core) is the strongly-typed engine
value — the source of truth; `ParamKind` is its classifier (hence `Param::kind()`);
`ParamValue` is `Param`'s lossy serialized shadow, whose variants *should* align
with `ParamKind`. Phase 4 makes that alignment an enforced, tested invariant: the
`ParamValue` that `from_param` produces must match `descriptor.kind`. This also
exposes (and the kind model resolves) the latent smell that `ParamValue::Choice`
does double duty for enum-ids and mod-matrix addresses.

> **Possible future refinement (out of scope, note for reviewer):** once `kind` is
> authoritative, `ParamValue::Choice` could split into `Enum(String)` +
> `Reference(...)` to mirror the kinds exactly. That is a serialization-format
> change with a migration cost; left out of this plan deliberately.

### 1.6 The full parameter type family (workspace-wide audit)

A full sweep (all crates) found **far more than the obvious few**. `ParamKind` is
*not* a new competitor — it is the single classifier the rest should carry or align
to. The types fall into four groups.

**Group A — value-carrier shadows of `Param` (FOUR loosely-typed enums for one
concept).** Every boundary re-invented its own; none carries kind, two are missing
variants, and they disagree on serialization:

| Enum               | Location                  | Variants                                          | Notes                                                                                                                                                              |
|--------------------|---------------------------|---------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `ParamValue`       | `pertylizer/patch.rs:367` | Bool, Int, Float, Choice, **SampleId{sample_id}** | serialization (read+write); SampleId is a *struct* variant → `{"sample_id":N}`                                                                                     |
| `PatchParamValue`  | `synth_mcp/types.rs:950`  | Float, Int, Bool, Choice, **SampleId(u64)**       | MCP patch-resource view (Serialize-only); SampleId is a *tuple* variant → bare `N`. **Serializes SampleId differently from `ParamValue`** — a latent inconsistency |
| `BridgeParamValue` | `synth_mcp/bridge.rs:161` | Number, Choice, Bool                              | internal MCP apply — **no Int, no SampleId/Reference**                                                                                                             |
| `ParamValueInput`  | `synth_mcp/server.rs:364` | Number, Bool, Choice                              | tagged MCP tool *input* — **no Int, no Reference**                                                                                                                 |

> **Reviewer takeaway:** these four are the strongest collision with `ParamKind` —
> their variants *are* an ad-hoc kind enumeration, duplicated four times,
> inconsistently. Aligning all four to `ParamKind` (and fixing the SampleId
> serialization split + the missing-Int gap) is the cleanest payoff of this work.
> See Phase 4 (ParamValue/PatchParamValue) and Phase 5 (Bridge/Input).

**Group B — descriptor projections / DTOs (each should *carry* `kind`).** All mirror
`min`/`max`/`default`/`unit`/`choices` as bare `f32`/`String`; the f32-flattening
leaks all the way to the MCP wire:
 
| Type                           | Location                          | Role                                                                                              |
|--------------------------------|-----------------------------------|---------------------------------------------------------------------------------------------------|
| `ParameterDescriptor`          | `synth_core/module_traits.rs:596` | the spec — gains the authoritative `kind` (Phase 1)                                               |
| `ParameterInfo`                | `synth_mcp/types.rs:143`          | MCP: a param's current value (`get_parameter`); already carries `response_curve`/`is_automatable` |
| `ParamTypeInfo` + `ChoiceInfo` | `synth_mcp/types.rs:382/404`      | MCP: type spec (`get_module_type_info`); `ChoiceInfo.value:f32` re-exposes enum-as-index          |
| `ReturnEffectParamInfo`        | `synth_mcp/types.rs:790`          | MCP: return-bus effect param value                                                                |
| `PatchParamInfo`               | `synth_mcp/types.rs:940`          | MCP: name+`PatchParamValue` pair in a patch resource                                              |
| `AutomationTargetInfo`         | `synth_mcp/types.rs:829`          | MCP: a valid automation target (has `unit`/`min`/`max`/`response_curve`)                          |

> **Naming collision — `kind` is already taken on the wire.**
> `AutomationTargetInfo.kind: String` (`types.rs:834`) already uses the word "kind"
> for the *target class* (`"module"` vs `"instrument"`) — a different axis from
> `ParamKind`. When surfacing `ParamKind` on these DTOs, use a distinct field name
> (e.g. `value_kind`) to avoid confusing the two on the MCP wire.

**Group C — orthogonal but related (different axis, do NOT merge into `ParamKind`).**

- `WidgetHint` (`module_traits.rs:375`): *how to draw* it. Overlaps semantically
  (`Toggle`≈Bool, `Dropdown`≈Enum) but one kind maps to several widgets
  (a `Continuous` Hertz param → `Knob`/`FrequencySlider`/`Slider`). Phase 6 uses
  `kind` to *constrain* the hint (a `Bool` always a checkbox), keeping the hint as
  the chooser among compatible widgets. Layered, not duplicated.
- `ParameterUnit` (`module_traits.rs:506`): the *unit* — a sub-attribute of
  `Continuous`/`Integer`, derived from the newtype in Phase 2a.
- **Curve triplication** — `ResponseCurve` (`module_traits.rs:410`),
  `CurveType` (`synth_sequencer/automation.rs:96`), `CurveKind`
  (`synth_mcp/bridge.rs:2349`) are three near-identical curve enums for one concept.
  Orthogonal to `ParamKind` (shape, not kind) but the same "parallel enum" smell;
  flagged for a possible *separate* consolidation, out of scope here.

**Group D — parameter addressers in automation/modulation (consumers of `kind`).**

- `AutomationTarget` (`synth_sequencer/automation.rs:245`) + `AutoInstrumentParam`
  (315) / `TrackParam` (358, **`Mute` is a Bool target**) / `GlobalParam` (373).
  `AutomationPoint.value` is a `NormalizedValue` — automation *assumes* Continuous,
  so automating a Bool/Enum/Integer param via a normalized lane is semantically
  dubious. `kind` is exactly what should gate this — ties to the `is_automatable`
  tightening in Phase 2b.
- `SrcAddr`/`DestAddr`/`ModSource`/`ModDestination` (`mod_matrix.rs`) — the
  `Reference` addresses; already special-cased in serialization (Phase 4 / §7, §1.5).

### 1.7 Explicit out-of-scope: the AWE parameter universe

`synth_awe` has a **completely parallel** parameter system — `AweParam` (100+ typed
variants, `synth_awe/src/params.rs:103`), `AweLfoTarget`, `AweLfoState`,
`MaterialKind`, `RoomShapeKind` — that does **not** flow through `Param` /
`ParameterDescriptor` at all. This plan covers only the `synth_core` `Param`
modules. Applying the same kind model to AWE is a **future parallel application**,
explicitly out of scope; noted so the reviewer knows the boundary is deliberate, not
an oversight.

---

## 2. Phasing overview

Each phase is independently committable and must pass the full gate
(`cargo fmt --check && cargo build && cargo clippy --all-targets && cargo test`,
per `CLAUDE.md`). Dependency order:

| Phase | Title                                                                | Layer         | Risk           | Depends on |
|-------|----------------------------------------------------------------------|---------------|----------------|------------|
| 0     | Audit & taxonomy table ✅ **done** (`docs/param-kinds.md`)            | (doc)         | none           | —          |
| 1     | `ParamKind` + `ScalarParam` + `kind()` + descriptor field ✅ **done** | engine        | low            | 0          |
| 2a    | Derive `unit` from newtype; drift-assert test ✅ **done**             | engine        | low            | 1          |
| 2b    | `curve` audit (no silent auto-flip; behavioral) ✅ **done**          | engine        | medium         | 1          |
| 3     | Kind-aware display (decimal-free ints; bool) ✅ **done**             | core+GUI      | low            | 1          |
| 4     | Serialization emits `Int`/`Bool`; consistent int rounding ✅ **done** | serialization | low            | 1          |
| 5     | Schema `type`/`kind`; MCP validation by kind ✅ **done** (5a+5b)      | schema/MCP    | low            | 1          |
| 6     | GUI interaction polish (snap/scroll/text by kind)                    | GUI           | medium         | 3          |
| 7     | `ModuleParam` shared trait (recommended)                             | engine        | medium (churn) | 1          |
| 8     | `#[derive(ParamReflect)]` proc-macro — **skipped**                   | —             | —              | —          |

Phases 2–6 can proceed in parallel after Phase 1 lands, except Phase 6 depends on 3.

---

## 3. Phase 0 — Audit & taxonomy table ✅ DONE

**Deliverable (shipped):** [`docs/param-kinds.md`](../docs/param-kinds.md) — the
authoritative per-variant `module::variant → kind` table for all 378 params across
67 enums, with per-enum tables, totals, findings, and the `ScalarParam::KIND`
assignment list that Phase 1 implements against.

**Taxonomy confirmed complete:** every variant fits one of the five kinds; no sixth
kind needed. Headline results that change downstream phases:

- **Exact buckets:** Continuous 307 / Enum 31 / Bool 20 / Integer 17 / Reference 3.
  (`Enum` was over-counted ~107 in the provisional tally — it is **31**.)
- **Integer-backed newtypes are `Integer`, not `Continuous`** — `VoiceCount`,
  `MidiNote`, `Octaves`, `StepCount`. Their `ScalarParam` impls get `KIND = Integer`
  (finding #4 in the doc). Watch `Octaves` newtype (Integer) vs the `SubOscOctave`
  *enum* (Enum) — same display name, different kind.
- **`Reference` is exactly 3:** `SamplerParam::SampleSelect`,
  `ModMatrixParam::SlotSource`, `ModMatrixParam::SlotDestination`.
- Two-field `Variant(u8, T)` classified by **T** (the `u8` is a structural index).
- Two raw-`f32` params confirmed (`KineticParam::OutputVel/OutputAcc`) — give them
  newtypes later (separate task).
- **One verify carried into Phase 1:** `DistortionParam::BitDepth` — confirm whether
  its newtype is f32- or integer-backed (Continuous vs Integer).

---

## 4. Phase 1 — `ParamKind` foundation (engine-only, no behavior change)

**Goal:** introduce the classifier and derive it from the engine, with zero
observable behavior change. Nothing consumes `kind` yet.

**Steps:**

1. Add `ParamKind` enum (§1.1) to `module_traits.rs`.
2. Add the `ScalarParam` trait (§1.2). **Recommendation:** define all three
   constants + the provided `scalar_*` methods now (one pass over the value types),
   but *consume* only `KIND`/`scalar_kind()` in Phase 1 — `UNIT` is used in Phase 2a,
   `DEFAULT_CURVE` only by the Phase 2b audit. Implement for every value type:
   newtypes, `u8`/`i32`/`usize`, `bool`, `SampleId`, `Option<SrcAddr>`,
   `Option<DestAddr>`, and each choice enum. **Use a declarative macro for the many
   choice enums** (`Waveform`, `FilterMode`, `DelayMode`, …) — they all map to the
   same `Enum`/`None`/`Linear` triple, so a one-liner avoids dozens of identical
   blocks (per external review):
   ```rust
   macro_rules! impl_scalar_param_enum {
       ($($t:ty),* $(,)?) => { $(
           impl ScalarParam for $t {
               const KIND: ParamKind = ParamKind::Enum;
               const UNIT: ParameterUnit = ParameterUnit::None;
               const DEFAULT_CURVE: ResponseCurve = ResponseCurve::Linear;
           }
       )* };
   }
   impl_scalar_param_enum!(Waveform, FilterMode, FilterModel, DelayMode, /* … */);
   ```
   Covers the **~25 genuine choice enums** (the 36 non-`Param` enums in `params/`
   minus the ~8 port/address types: `ModuleType`, `Port`/`AudioPort`/`ControlPort`,
   `SrcAddr`, `ModSource`, `ModDestination`, `MacroSource`). **Do NOT feed the
   address types to this macro** — `Option<SrcAddr>`/`Option<DestAddr>` are
   `Reference`, not `Enum`, and get their own hand-written impls. (This is a
   *declarative* macro for trait impls — distinct from, and not in conflict with,
   the *proc-macro derive* evaluated and skipped in Phase 8.)
3. Add `kind()` (and `unit()`) to each of the **67** module enums, binding the
   value and calling `v.scalar_kind()` / `v.scalar_unit()` (§1.3) — never
   turbofishing the type name.
4. Add `Param::kind()` / `Param::unit()` aggregate delegation matches
   (`params/mod.rs`).
5. Add `kind: ParamKind` to `ParameterDescriptor`; seed it in **both** constructors:
    - `float()` (`module_traits.rs:654`): `kind: id.kind()`.
    - `choice()` (`module_traits.rs:675`): `kind: id.kind()` — note this yields
      `Enum` for real enums *and* `Reference` for mod-matrix slot source/dest,
      cleanly separating the two. `choices` stays the picker list.
6. **Tests (Phase 1 acceptance) — precise kind/choices invariants:**
    - `kind == Enum ⇒ choices.is_some()` (every enum carries its choice list).
    - `kind ∈ {Continuous, Integer, Bool} ⇒ choices.is_none()`.
    - `kind == Reference`: `choices` is *optional* — the mod-matrix address params
      carry a picker list, the `SampleId` reference carries none. (So the earlier
      "Enum ⇔ choices" biconditional is wrong: choices presence does **not** imply
      `Enum`.)
    - Assert every descriptor's `kind` equals its `id.kind()` (constructors wired).
    - A `ScalarParam` coverage check: every value type used in a `Param` variant has
      an impl. Enforced by compilation (a missing impl fails the `v.scalar_kind()`
      arm); a doc-test pins the intent.

**Behavior change:** none. `kind` is dead metadata until later phases.

**Why this is safe:** additive field with a derived default; `Param::kind()` match
forces completeness at compile time.

---

## 5. Phase 2 — Derive `unit` (2a) and audit `curve` (2b); kill drift

Split into two sub-phases because **unit derivation is cosmetic and safe, but curve
derivation is behavioral and must not happen silently** (review finding #2).

### Phase 2a — derive `unit` (cosmetic, low risk)

**Goal:** stop hand-declaring `unit`; surface and fix display-unit drift. This is
what makes `plans/TODO.md` §4.2 (drift lint) obsolete.

1. Seed `unit` from `id` in `float()`:
   ```rust
   pub fn float(type_id, id, name) -> Self {
       Self {
           kind: id.kind(),
           unit: id.unit(),          // was ParameterUnit::None — type-derived
           response_curve: ResponseCurve::Linear, // UNCHANGED in 2a (see 2b)
           range: ValueRange::UNIT,  // still explicit/overridable
           // ...
       }
   }
   ```
   The `.unit()` builder remains for genuine overrides (e.g. `NormalizedValue` →
   `Percent`).
2. **De-risking (do NOT mass-delete `.unit()` calls yet).** Add a test asserting,
   for every descriptor, `explicit_unit == id.unit()` *unless* the param is on a
   curated allow-list of intentional overrides. **Run it and fix the mismatches —
   these are real drift bugs** (e.g. a `Hertz` param wrongly tagged `Milliseconds`).
   The allow-list is the §4.2 "legitimate one-offs" list, now a test-time guarantee.
3. **Cleanup (separate mechanical commit — resolved §14.2):** delete the redundant
   `.unit()` calls equal to the derived default. The external review prefers doing
   this (not leaving dead builders to confuse future devs); keep it as its own commit
   *after* the validation test lands, so the PR stays readable. The test guarantees
   agreement either way.

### Phase 2b — `curve` audit (behavioral, do not auto-flip)

**The risk:** `float()` defaults `response_curve` to `Linear` today; many params
never call `.curve()` and rely on that. Auto-seeding `curve = id.default_curve()`
would **silently change** every such param to the type's curve (`Hertz`→Log,
`Seconds`→Exp, `Gain`→Squared), altering `normalize`/`denormalize` — which knob feel,
automation lanes, and the headless exporters all depend on. That is an audible
behavioral change, not a cleanup.

**Approach — audit, don't auto-apply:**

1. Add a test computing each param's *current effective curve* (explicit `.curve()`
   value, else `Linear`) and comparing it to `id.default_curve()` (the type's
   suggestion). Report every mismatch — **do not change behavior**.
2. For each mismatch, make a **deliberate** decision: (a) the param genuinely should
   change feel → accept and add an explicit `.curve()` so it's intentional and
   documented; or (b) keep current feel → leave the explicit/`Linear` curve. Either
   way the curve stays **explicit in the descriptor**, never silently derived.
3. **Recommendation:** do *not* wire `default_curve()` into `float()` at all. Keep
   `curve` an explicit descriptor field; `DEFAULT_CURVE` exists only as the audit's
   suggested value and as documentation of the "natural" curve per quantity. (See
   Open Decision §14.6.)

**Note on `is_automatable`** (`module_traits.rs:780`): today
`modulatable && choices.is_none()`. With kind available, tighten to
`modulatable && kind == ParamKind::Continuous` — integer/bool/reference params are
structural and can't be ramped. **Verified safe (external review + grep):** the
discrete params (`segments`/`sustain_seg`/`loop_start`/`loop_end` in `mseg.rs`,
`mute` in `amplifier.rs`, etc.) are *already* `.modulatable(false)`, so tightening
drops nothing currently automatable. **Caveat:** `modulatable` also gates mod-matrix
*destination* eligibility elsewhere; this change touches only `is_automatable()`, so
the two may now diverge (intentional). Still ship the before/after `is_automatable`
set as a regression test.

---

## 6. Phase 3 — Kind-aware display

**Goal:** integer knobs show `"4"`, not `"4.00"`; bools never show as a knob value.
Fixes `plans/TODO.md` §3.6 display and part of §3.5.

**Steps:**

1. Add one centralized formatter in `synth_core` that takes kind into account:
   ```rust
   // module_traits.rs — single source of truth for value→string.
   pub fn format_value(kind: ParamKind, unit: ParameterUnit, value: f32) -> String {
       match kind {
           ParamKind::Integer => format!("{:.0}{}", value.round(), unit.suffix()),
           ParamKind::Bool    => if value > 0.5 { "On".into() } else { "Off".into() },
           _                  => unit.format(value),
       }
   }
   ```
   Keep `ParameterUnit::format` unit-only (still used where kind is irrelevant).
   > **Caveat:** the `Integer` arm uses `unit.suffix()`, which bypasses the value
   > *transforms* in `unit.format` — `Percent` (×100) and `Hertz` (≥1000 → kHz). For
   > a hypothetical integer+`Percent` or integer+`Hertz` param this would mis-display
   > (e.g. "1000 Hz" not "1.00 kHz"). Integer params are almost all unitless counts,
   > so this is an edge case — but if an integer param ever takes a scaling unit,
   > round first then run through `unit.format` instead of `suffix()`.
2. Route `ParameterDescriptor::format` (`module_traits.rs:798`) through it:
   choice → choice name (unchanged), else `format_value(self.kind, self.unit, value)`.
3. `Knob`: carry `kind` (set in `from_descriptor`, `knob.rs:41`); change
   `format_value` (`knob.rs:88`) to use the centralized formatter.
4. **Integer step defaults to 1 from kind.** In `Knob::from_descriptor`, when
   `descriptor.kind == Integer && descriptor.step.is_none()`, use an effective step
   of `1.0`. Then **drop the 4 `.step(1.0)` calls in `mseg.rs`** (`407/422/451/465`)
   — integers snap for free. `step` survives only for genuine fractional
   quantization (0.25/0.5).

**Verify:** in-app eyeball of MSEG (`Segments`/`Sustain Seg`/`Loop Start`/`Loop End`)
showing whole numbers and snapping; bool params reading On/Off in the tooltip.

---

## 7. Phase 4 — Serialization emits typed values

**Goal:** self-describing project files; int/bool round-trip as `Int`/`Bool`.

**Key fact (confirms small scope):** the **read** path already handles all variants
— `ParamValue::to_f32`/`to_param` (`patch.rs:378/404`) and the builder helpers
`param_i`/`param_b` (`patch.rs:1009/1017`). Only the **write** path
(`from_param`, `patch.rs:439`) never produces `Int`/`Bool`.

**Steps:**

1. In `from_param`, after the existing SampleId / mod-matrix / choice special-cases,
   branch on `desc.kind` (or `p.kind()`):
   ```rust
   match p.kind() {
       ParamKind::Integer => Self::Int(p.as_f32().round() as i32),
       ParamKind::Bool    => Self::Bool(p.as_f32() > 0.5),
       _                  => Self::Float(p.as_f32()),
   }
   ```
   (SampleId/Choice already returned earlier; Reference for mod-matrix already
   returned as the address `Choice`.)
2. **Fix the truncation inconsistency.** Integer `with_f32` does `value as u8`
   (truncates: 3.9 → 3) while the knob `snapped()` rounds. Make integer `with_f32`
   round (`value.round() as u8`) so snapping, validation, and engine application
   agree. Audit all integer-backed `with_f32` arms; one helper
   (`fn round_to<T>(f32) -> T`) keeps it uniform.
3. **Migration:** old projects storing `5.0`/`1.0` for int/bool params still load
   (untagged `Float` → `to_f32` → `with_f32`). New saves reformat once to `5`/`true`
   — values identical, a one-time cosmetic diff. `CLAUDE.md` declares no
   backward-compat requirement; note the reformat in `docs/history.md`.
4. **`untagged` ordering check.** `ParamValue` order is `Bool, Int, Float, Choice,
   SampleId` (`patch.rs:367`). serde_json parses JSON `5` → `Int`, `5.0` → `Float`
   (fraction disqualifies `i32`), `true` → `Bool`. A JSON `5` reaching a Continuous
   param deserializes as `Int(5)` → `to_f32` = 5.0 — harmless. Add round-trip tests
   covering: int param saves `Int` and reloads; bool saves `Bool`; legacy `Float`
   for an int param still loads to the correct engine value.
5. **Alignment invariant (the `ParamValue` ↔ `ParamKind` bridge, §1.5).** Add a
   test asserting that for every descriptor, the `ParamValue` returned by
   `from_param(&desc.id, &desc)` has the variant dictated by `desc.kind`
   (`Continuous→Float`, `Integer→Int`, `Bool→Bool`, `Enum→Choice`,
   `Reference→Choice`/`SampleId`). This is the one place the two types are wired
   together and guarantees they can never silently disagree.
6. **Align the MCP patch-resource shadow `PatchParamValue`** (`synth_mcp/types.rs:950`,
   Group A). It mirrors `ParamValue` but serializes `SampleId` as a *tuple* variant
   (bare `N`) vs `ParamValue`'s *struct* variant (`{"sample_id":N}`) — a latent wire
   inconsistency (§1.6). **Resolved (external review, Risk 1):** normalize both to the
   **struct form** `{"sample_id": N}` — it is self-describing and can't collide with a
   plain numeric value under `untagged` (a bare `N` would). Note this is a wire-shape
   change for the MCP patch resource (Serialize-only, no migration). Emit `Int`/`Bool`
   from the same kind decision and share one conversion helper so the two shadows
   can't drift again.
7. **Reference values never round-trip through `f32`** (external review, Risk 2). A
   `SampleId` is `u64`; the lossy `f32` path (`to_f32`) is GUI-display only. Project
   save/load already bypasses it via the `SampleSelect`→`SampleId` special-case in
   `from_param`/`to_param` — keep that invariant and add a test asserting a `u64`
   sample id ≥ 2²⁴ survives a save/load round-trip bit-exact.

**Scope boundary — what this phase does and does NOT make type-safe.** Phase 4 makes
the **write** side type-consistent and self-describing (the saved JSON carries the
right type per param, guaranteed by the alignment-invariant test, §1.5). It does
**not** make the **read** side strictly type-safe: `ParamValue` is `#[serde(untagged)]`
and `to_param`/`with_f32` *coerce* (`5.0`→int, out-of-range→clamp, fractional→round).
A hand-edited or corrupt project file with a wrong-typed value is silently coerced,
not rejected. This leniency is **intentional** here — it is what lets old projects
that stored integers as `5.0` still load. Net: project/instrument JSON becomes
*type-consistent and self-describing on save*, and *type-aware* via the schema, but
*leniently coerced on load* — not strictly validated. Strict load validation is a
separate, deferred item (§16).

---

## 8. Phase 5 — Schema & MCP validation by kind

**Goal:** `descriptors.json` advertises real JSON types; MCP boundary validates by
kind. Fixes `plans/TODO.md` §3.6 layer-2.

**Steps:**

1. `gen_schemas::parameter_descriptor` (`bin/gen_schemas.rs:117`): for numeric
   params emit `"type"` from kind — `Integer → "integer"`, `Bool → "boolean"`,
   `Continuous → "number"` — and a `"kind"` string for completeness. Enum keeps
   `choices`; Reference emits its existing shape (SampleId object; mod-matrix
   address as string with the picker list). Regenerate `schemas/descriptors.json`.
2. `validate_f32` (`module_traits.rs:844`): make it kind-aware (rename or add a
   `validate(kind, value)` variant). **Resolved policy (external review, §14.1 +
   Risk 3) — two distinct axes, don't conflate them:**
    - **Fractional → lenient:** for `Integer`, *round* to nearest (so `4.3`/`4.0001`
      from an automation/LFO sweep is accepted, not rejected), and **echo the applied
      rounded value** in the response so the client knows what took effect.
    - **Out-of-range → reject:** after rounding, range-check and **reject** an
      out-of-bounds value (e.g. `20` for `1..=16`) with a clean `OutOfRange` error at
      the MCP boundary — *before* `with_f32`'s clamp is ever hit. The `with_f32` clamp
      stays only as a defensive net, never the primary mechanism.
    - `Bool`: accept any finite (maps `>0.5`). `Continuous`/`Enum`: unchanged
      (range / choice-index check). Non-finite rejected as today.
3. MCP input carriers (Group A): `BridgeParamValue` (`bridge.rs:161`) and the tagged
   `ParamValueInput` (`server.rs:364`) both **lack `Int` and any Reference variant**
   — clients can't send a typed integer. Route their conversion
   (`param_value_to_bridge`, `resolve_param_value`) through the new kind-aware
   validation, and add an `Int` path so an integer param accepts a typed int.
4. Extend **all** discovery DTOs (Group B, §1.6) to carry the kind. **Use a field
   name that does not collide with the existing `AutomationTargetInfo.kind: String`**
   (target class) — e.g. `value_kind`:
    - `ParamTypeInfo` (`types.rs:382`), built at `mcp_bridge.rs:12501` for
      `get_module_type_info` — add `value_kind`; consider dropping
      `ChoiceInfo.value: f32` (`types.rs:406`) now Enum is explicit.
    - `ParameterInfo` (`types.rs:143`) for `get_parameter` — add `value_kind` so a
      client reading a value knows to send `4` not `4.0`.
    - `ReturnEffectParamInfo` (`types.rs:790`) and `PatchParamInfo` (`types.rs:940`)
      — same treatment for return-bus effects and patch resources.
      Keep the f32 fields for compatibility but let `value_kind` be the authoritative
      type signal alongside the new JSON `"type"` in `descriptors.json`.
5. **MCP feedback hook (per `CLAUDE.md`):** while wiring this, note any tool whose
   schema still can't express integer/bool cleanly and report it.

**Tests:** schema snapshot for one integer, one bool, one enum, one reference param;
MCP `set_parameter` rejects/rounds a fractional integer per the chosen policy.

---

## 9. Phase 6 — GUI interaction polish (the "more typed knobs")

**Goal:** the graphical controls behave correctly per kind, not just display
correctly. Directly answers the request to improve `Knob` et al.

**Steps (each small, in `crates/pertylizer/src/gui/widgets/`):**

1. **Integer slider parity** (`param_grid.rs:196` slider branch): when
   `param.kind == Integer`, snap to integers and use `.min_decimals(0).max_decimals(0)`.
   Today only the `Knob` honors `step`; the `Slider` path ignores it entirely.
2. **Scroll-wheel stepping** (`knob.rs` `show`, currently drag-only `knob.rs:142`):
   add a scroll handler. `Integer` ⇒ ±1 per notch; `Continuous` ⇒ a curve-aware
   fine step; respect `step` when set.
3. **Bool dispatch hardening** (`param_grid.rs:104` `render_group`): a `Bool`-kind
   param renders as a checkbox regardless of `widget_hint` (defensive — prevents a
   bool accidentally drawn as a 0–1 slider). Keep `widget_hint` as the primary
   chooser for non-bool widgets.
4. **Kind-validated text entry (new, optional within phase):** if/when a
   double-click-to-type entry is added to `Knob`, validate by kind — integers reject
   `.5`, bools accept only 0/1/on/off. Scaffold the validation hook even if the text
   widget lands later.
5. **Integer drag sensitivity:** for `Integer` kind, scale drag sensitivity so one
   step ≈ a comfortable pixel distance (avoid skipping values on coarse ranges or
   needing pixel-perfect drags on 1–16 ranges).

**Verify:** in-app eyeball — integer slider snaps + shows no decimals; scroll steps
an integer knob by 1; a bool always a checkbox.

---

## 10. Phase 7 — `ModuleParam` shared trait (RECOMMENDED — resolved §14.3)

**Decision (external review):** land it. A uniform trait is valuable for generic
code, test harnesses, and serialization helpers; the churn is mechanical and
contained by a prelude.

**Goal:** formalize the per-enum method set so every module enum provably
implements the full contract, and enable generics over parameters.

```rust
pub trait ModuleParam: Copy + Sized {
    fn as_f32(&self) -> f32;
    fn with_f32(&self, v: f32) -> Self;
    fn same_kind(&self, other: &Self) -> bool;
    fn name(&self) -> &'static str;
    fn kind(&self) -> ParamKind;
    fn unit(&self) -> ParameterUnit;
    fn default_curve(&self) -> ResponseCurve;
}
```

**Steps:** move the 67 enums' inherent methods into `impl ModuleParam for X`; impl
for the aggregate `Param` too (giant delegation matches stay — an enum-of-enums
can't blanket). **Churn mitigation (external review):** export the trait from a
`synth_core::prelude` that the workspace already imports, so the thousands of
existing `param.as_f32()` call sites resolve without per-file `use` edits. (If a
prelude is undesirable, the fallback is to keep thin inherent methods that delegate
to the trait — but the prelude is cleaner.)

**Cost/benefit (still worth stating):**

- **Benefit:** uniformity, generic code over `impl ModuleParam`, one place to read
  the contract.
- **Cost:** mechanical call-site churn, contained by the prelude.
- **Note:** the aggregate `Param::kind()`/`as_f32()` delegation matches *already*
  force every enum to implement these (missing method = compile error), so the trait
  adds ergonomics rather than correctness — but the review judges the ergonomics
  worth it. Lands after the functional phases (1–6) so it never blocks them.

---

## 11. Phase 8 — `#[derive(ParamReflect)]` proc-macro (DECIDED: SKIP — §14.4)

**Decision (external review):** **skip.** A proc-macro (`syn`/`quote`/`darling`) adds
real compile-time overhead and *hides* compiler errors behind generated code. The
`kind()`/`unit()` delegation matches are simple, type-derived, fast to compile, and
explicit. The cheap *declarative* macro for choice-enum impls (Phase 1) already
removes the only genuinely repetitive boilerplate; a proc-macro derive is not worth
its cost. Section kept for the record / future reconsideration only.

**Original goal (not pursued):** auto-generate `kind()`/`unit()`/`default_curve()`/
`name()`/`same_kind()` — and `as_f32()`/`with_f32()` where there is no custom clamp —
from each enum's variant types, eliminating the mechanical per-variant matches.

**Reality check (do an evaluation spike before committing):**

- `name()` needs a display string per variant → `#[param(name = "Segments")]` attr.
- `with_f32()` has per-variant clamps (`mseg.rs:80` `clamp(1, 16)`, `.min(15)`) and
  index-encoded two-field variants (`SegmentTime(u8, Seconds)`) → needs
  `#[param(clamp = "1..=16")]` / `#[param(index)]` attributes. If the attribute
  surface grows unwieldy, **stop** — keep `with_f32` hand-written and only derive
  the metadata methods (`kind`/`unit`/`curve`/`name`).
- `kind()`/`unit()`/`default_curve()` are pure type→const lookups and derive cleanly.

**Recommendation:** spike the metadata-only derive (kind/unit/curve/name); defer
deriving `with_f32`/`as_f32` unless the clamp attributes stay clean. Document the
decision; this phase is genuinely optional and the lowest priority.

---

## 12. Cross-cutting concerns (review checklist)

- **Real-time safety:** `ParamKind` is `Copy`; `kind()`/`unit()`/`default_curve()`
  are match-and-return-Copy, no allocation, callable anywhere including the audio
  thread. No `process()` hot-path change. ✔ per `CLAUDE.md` RT rules.
- **Newtype discipline:** `ScalarParam` reinforces the newtype pattern; the 2 raw
  `f32` params should get newtypes (tracked separately, not a blocker).
- **`type_id` stability:** untouched; `kind` is orthogonal to the JSON key. ✔
- **File backward-load:** old `Float`-encoded int/bool params still load (read path
  unchanged). New saves reformat once. No data loss. ✔
- **Mod-matrix Reference:** serialization stays the existing variant-specific
  special-case (address string / SampleId object); `kind == Reference` is advisory
  for GUI/schema only. ✔
- **`is_automatable` tightening** (Phase 2b) must not silently drop a currently
  automatable param — guard with a before/after test.
- **Drift lint obsolescence:** Phase 2a's assert-equal unit test replaces
  `plans/TODO.md` §4.2; remove §4.2 from TODO when Phase 2a lands.
- **Gate per phase:** `cargo fmt --check && cargo build && cargo clippy --all-targets
  && cargo test` green before each commit.

## 13. What this plan closes in `plans/TODO.md`

- **§3.6** (integer/value-kind marker) — fully, generalized to all kinds.
- **§4.2** (descriptor drift lint) — obsolete; Phase 2a unit derivation + assert-test
  guarantees it structurally.
- **§3.5** (MSEG knob awkwardness) — partially (integer display/snapping fixed; the
  graphical envelope editor remains a separate UI task).

## 14. Decisions (all resolved)

**Resolved by external review pass 1 (decisions 1–5):**

1. **Integer MCP policy** → **Round (lenient) with echo** *and* reject out-of-range.
   Round fractional input (don't break sweeps), echo the applied value; still reject
   values outside `[min,max]` (Phase 5 step 2).
2. **Phase 2a cleanup** → **Yes, as a separate mechanical commit** after the
   validation test lands (Phase 2a step 3).
3. **Phase 7 `ModuleParam` trait** → **Land it**, with the trait exported from a
   prelude to contain call-site churn (Phase 7).
4. **Phase 8 proc-macro** → **Skip** (Phase 8). The cheap declarative macro in Phase
   1 covers the only real boilerplate.
5. **Bool display string** → **`On`/`Off`** (Phase 3).

**Resolved by external review pass 2 (decision 6):**

6. **Curve derivation (Phase 2b)** → **RESOLVED: do NOT auto-derive `curve`.** Keep
   `ResponseCurve::Linear` as the `float()` fallback; keep every param's curve
   *explicit*; use `DEFAULT_CURVE` only as the audit baseline. Confirmed by external
   review pass 2, which independently flagged the same two hazards: (1) automation/
   recall regressions — existing normalized `[0,1]` automation points would map to
   different physical values if a curve silently flipped (e.g. a `Hertz` base freq
   Linear→Log); (2) curves are context-specific (a `Seconds` envelope attack may want
   Exponential while a `Seconds` delay-time wants Linear). The internal recommendation
   and external review now agree — **all six decisions resolved.**

---

## 15. Implementation checklist

Ordered task list distilled from external review pass 2. Each box is a commit-sized
unit; the full gate must be green before each commit.

**Step 1 — Foundation (Phase 1):**

- [x] Define `ParamKind` in `synth_core/src/module_traits.rs` (`Serialize` only). *(done)*
- [x] Implement `ScalarParam` per the `KIND` assignment list in
  [`docs/param-kinds.md`](../docs/param-kinds.md): f32-newtypes → Continuous;
  primitives + **integer-backed newtypes** (`VoiceCount`, `MidiNote`, `Octaves`,
  `StepCount`) → Integer; `bool` → Bool; references (`SampleId`,
  `Option<SrcAddr>`, `Option<DestAddr>`) → Reference. **`BitDepth` verified
  `pub f32` → Continuous.** Impls live in `params/scalar_impls.rs`; a coverage
  check confirmed all 56 carried value-types have an impl. *(done)*
- [x] Add `impl_scalar_param_enum!` and apply it to the ~25 choice enums (NOT the
  address/Reference types — see Phase 1). *(done — `scalar_enum!` over 29 enums)*
- [x] Add `kind()` + `unit()` to the 67 module enums and aggregate `Param`,
  dispatching via **bound values** (`v.scalar_kind()` / `v.scalar_unit()`).
  *(done — generated `params/kind_impls.rs`, arms grouped by value-type; exhaustive
  matches guarantee completeness; two-field variants bind the value field.)*
- [x] Add the `kind` field to `ParameterDescriptor`; populate in `float()` and
  `choice()` from `id.kind()`. *(done — `ParamKind` also derives `Deserialize`
  since `ParameterDescriptor` does; never read from disk.)*
- [x] Acceptance tests: the precise kind/choices invariants (§4 Phase 1 step 6).
  *(done — synth_core sanity test + an all-descriptors sweep in
  `module_factory.rs` over `ModuleType::all()`; `kind == id.kind()` enforced for
  every param.)*

> **Phase 1 follow-up (deferred):** the sweep surfaced **two enum-typed params built
> with `float()` instead of `choice()`** → `kind == Enum` but no choice list:
> `SpectralBlur/fft_size` (should mirror PhaseVocoder's `choice()` over
> `FftSizeOption::ALL` — a copy-paste miss) and `KeyboardPanner/invert` (`Polarity`,
> a 0/1 knob). Converting them changes widget + serialization behavior (Phase 3/4
> territory), so they are **allow-listed** in the sweep test for now. Fix when
> touching those modules' UI/serialization.

**Step 2 — Unit derivation + curve audit (Phase 2a / 2b):**

- [x] Seed `unit` in `float()` from `id.unit()`. *(done)*
- [x] Unit-drift test (`descriptor_unit_matches_type_unless_allow_listed` in
  `module_factory.rs`). Surfaced 30 mismatches; triaged: 27 are benign
  `Percent`-over-unitless-`Continuous` presentation (one principled allow rule),
  and **2 real fixes at the source** — `BeatDivision`'s type-unit set to `Beats`,
  and `WavetableOsc/octave`'s stale `.unit(Semitones)` removed so it derives
  `Octaves` (`st`→`oct`). Regenerated `schemas/*.json`. *(done)*
- [ ] *(Phase 2a cleanup — separate commit)* delete now-redundant `.unit()` calls
  equal to the derived default (e.g. `sync_division`'s `.unit(Beats)`).
- [x] Curve **audit** (one-shot): `default_curve()` added to all 67 enums + `Param`
  (advisory; **not** wired into `float()` per §14.6). Result: **397 match, 87 would
  flip** (38 `Linear→Exponential` time, 24 `→Squared` gain, 15 `→Logarithmic` freq,
  +10 explicit-vs-suggested) — concretely vindicates §14.6 (auto-deriving curve would
  silently change 87 params' feel/automation). **No curve changed**; added a small
  `default_curve()` correctness test. *(done)*
- [x] Tighten `is_automatable` to `modulatable && kind == Continuous` + regression
  test. **Correction to the "drops nothing" assumption:** it drops **10**
  non-continuous params that were `modulatable(true)` with no choices —
  `Chorus/voices`, `EnsembleChorus/voices`, `ModalResonator/{base_note,modes}`,
  `KeyboardPanner/center` (Integer); `Compressor/sidechain`,
  `{GranularFx,PhaseVocoder,SpectralBlur}/freeze` (Bool); `SpectralBlur/fft_size`
  (Enum). They lose *sequencer-lane* eligibility but keep **mod-matrix** eligibility
  (`modulatable` unchanged). An existing lane on one becomes a no-op at apply
  (`instrument.rs:1159`); lane data persists. Acceptable per no-backward-compat;
  relates to deferred §16 load diagnostics. *(done)*

**Step 3 — Display + interaction (Phase 3 / 6):**

- [x] Centralized kind-aware formatter — `ParamKind::format(unit, value)`
  (`On`/`Off` for bool; decimal-free rounded integers; else the unit formatter).
  `ParameterDescriptor::format` and `Knob` both route through it. *(done; the
  `Slider` is Phase 6)*
- [x] `Knob::from_descriptor`: carries `kind`; defaults `step` to `1.0` when
  `kind == Integer && step.is_none()`; dropped the four `.step(1.0)` in `mseg.rs`
  (snapping now comes from kind; the Knob is the only GUI `descriptor.step`
  consumer). Regenerated `schemas/descriptors.json` (step omitted — the proper
  integer signal lands in Phase 5). Added a `ParamKind::format` acceptance test.
  *(done)*
- [ ] *(Phase 6)* `param_grid.rs` slider: integer kind → `min_decimals(0)`/
  `max_decimals(0)` + snapping; scroll-wheel stepping on `Knob`; bool always a
  checkbox.

**Step 4 — Serialization + schema/MCP (Phase 4 / 5):**

- [x] `from_param`: emits `Int`/`Bool` by kind (after the existing SampleId /
  mod-matrix / choice special-cases). All-descriptors **alignment-invariant** sweep
  (`param_value_variant_matches_kind`) confirms the `ParamValue` variant matches
  `kind` for every param (enum-as-float → `Float` is encoded, see §1.5). *(done)*
- [x] Integer `with_f32` **rounds** instead of truncating — 14 integer-kind sites
  (`mseg`/`generative`/`effects`/`physical`); `UnisonVoices`/`Octaves`/`CenterNote`
  already rounded. Enum `from_index` casts left truncating (Enum kind, integral
  indices — a separate consistency follow-up, out of Phase 4 scope). `SampleId`
  stays off the `f32` path; **u64 ≥ 2³³ round-trip test** added. *(done)*
- [x] Normalized `PatchParamValue::SampleId` to the struct shape `{"sample_id": N}`
  (matches `ParamValue`); updated its one construction site. *(done)*
- [x] *(Phase 5a)* `gen_schemas` emits `"type": integer|boolean|number` + a
  `value_kind` classifier for every param (`kind_id`/`json_type` helpers);
  `descriptors.json` regenerated. `validate_f32` is now **kind-aware**: rounds
  integers (lenient) then range-checks the rounded value, accepts any finite for
  bools, range-checks Continuous/Enum as before — returning the validated value.
  Both MCP `set_parameter` paths **capture** it, so a `4.3` integer is applied *and*
  echoed as `4`. Kind-aware `validate_f32` test added. *(done)*
- [x] *(Phase 5b)* Extended the Group-B DTOs (`ParameterInfo`, `ParamTypeInfo`,
  `ReturnEffectParamInfo`, `PatchParamInfo`) with `value_kind: Option<ParamKind>`
  (`ParamKind` now serializes lowercase, matching `descriptors.json`). Populated at
  all 7 construction sites from the descriptor (`PatchParamInfo` stays `None` — its
  `PatchParamValue` variant already conveys the type, and a per-param descriptor
  lookup would rebuild a module). MCP display routes through the kind-aware
  formatter. *(done)*
- [ ] *(Phase 5b — deferred, low value)* typed `Int` path on the MCP input carriers
  (`BridgeParamValue`, `ParamValueInput`): functionally redundant — integers already
  round-trip via `Number(f64)` → resolve → kind-aware `validate` (rounds). Add only
  if a client needs to send a *typed* integer.

**Step 5 — Trait (Phase 7):**

- [ ] Introduce `ModuleParam`; move the 67 enums' methods onto it; export via a
  `synth_core::prelude`. (Phase 8 proc-macro: skipped.)

---

## 16. Future work (deferred, not in this plan)

### 16.1 Strict load-time validation of project/instrument JSON

**Why deferred.** The plan deliberately leaves the **read** path lenient (§7 scope
boundary): `ParamValue` is `#[serde(untagged)]` and `to_param`/`with_f32` coerce
(`5.0`→int, out-of-range→clamp, fractional→round). That leniency is what lets old
projects (integers stored as `5.0`, values that predate a tightened range) keep
loading. So after the main plan, project/instrument files are *type-consistent and
self-describing on save* and *type-aware* via the schema — but a hand-edited or
corrupt file with a wrong-typed/out-of-range value is silently coerced, not rejected.
They are **not strictly type-safe on load**.

**The future step.** Add a validation pass on project/instrument load that, for each
`(type_id, ParamValue)` against its descriptor, checks:

- the `ParamValue` variant matches `descriptor.kind` (the same alignment invariant
  the *write* side already enforces in test — applied to *incoming* data);
- `Integer` values are integral and in range; `Continuous` in range; `Enum` resolves
  to a real choice id; `Reference` parses to a valid address/id.

**Key design decision (reject vs warn) — must be made when this is built.** Strict
*rejection* conflicts with the intentional backward-load leniency. So this is not a
free tightening; pick a policy:

- **Reject** — fail the load (or the offending param) on any mismatch. Safest, but
  breaks old/edited files and loses the graceful-degradation Pertylizer has today.
- **Warn + coerce (recommended default)** — keep loading (coerce as now) but collect
  a structured diagnostics report ("param X in module Y: value 5.5 rounded to 6";
  "value 20 clamped to 16") surfaced to the user / MCP caller / a `get_load_warnings`
  tool. Gets the visibility of strict validation without sacrificing recall.
- **Strict mode opt-in** — warn+coerce by default, with a strict flag (per-load or a
  setting) that promotes warnings to errors for tooling/CI that wants airtight files.

**Scope/cost.** Touches the project load path (`project_apply.rs` / `project.rs`),
reuses `descriptor.kind` + `validate` from Phases 1/5, and needs a diagnostics
channel. Independent of the main plan — can land any time after Phase 5. Until then,
the honest statement is: *engine and MCP boundary are validated/type-safe; JSON files
are type-consistent on save and coerced (not rejected) on load.*
