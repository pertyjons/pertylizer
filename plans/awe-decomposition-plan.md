# AWE Decomposition — From Monolith to Effect-Module Suite

Status: **design sketch** (no code yet, on hold). Supersedes the earlier incremental
"lapping fixes" direction. Decision driver: AWE is the one large bespoke
subsystem standing *outside* the project's effect-module graph, which is why its shape
picker is cosmetic, it can't be placed in an effect chain, and it can't be modulated by the
Mod Matrix / YAMS scripts. The fix is not "rewrite AWE as another monolith" but **dissolve it
into the `AudioEffect` ecosystem it should have belonged to.**

---

## 1. The two module categories (the architectural seam)

The engine already distinguishes these as **two traits** (`crates/synth_core/src/module_traits.rs`):

- **`PolyModule`** (`:1122`) — *per-voice* patch modules. Multi-port, note-aware
  (`note_on`/`note_off`/`is_release_done`), one instance per sounding voice. Oscillators,
  filters, envelopes. Wired in the patch editor graph.
- **`AudioEffect`** (`:1301`) — *"process the mixed output of all voices."* Single summed
  stream in/out, no polyphony, runs in an `EffectChain`. Everything in `effects/`.

`EffectChain` runs at three levels: per-instrument (post voice-mix), on return buses, and on
master (`master_effects: EffectChain`). **This is the home for the AWE reverb/room stages.**

AWE today is wrong precisely because it mixes a per-voice concern (the spatializer, via its
own `process_spatial` + `SpatialVoiceBank` side-path) and a mixed-stream concern (the
reverb tail) into one object bolted on the master bus outside the graph.

---

## 2. Current AWE signal chain (what we're decomposing)

From `awe_engine.rs:199-406`:

```
pre-delay → early reflections (ISM) → late reverb (FDN) → FDN modulation (chorus)
          → room modes → early/late mix → air absorption → spatializer
          → EQ → portal → dry/wet
```

Plus geometry that drives it: `RoomShape` (Box/Cylinder/Sphere/Dome/L-Shape/Tube) →
volume/surface-area/bounding-box → RT60, mode freqs, ER delays. **Shape only reaches RT60 +
mode frequencies today; ER and FDN see the bounding box only.**

---

## 3. Target module suite

Revised after a professional DAW review (its conclusions are folded into this plan —
see §7 Decisions; the standalone review doc was never committed to the repo). The decisive change
vs. the first draft: **early reflections and the spatializer are per-voice (`PolyModule`)**, not
buss effects — once voices are mixed the per-note 3D positions are gone, so buss-level ER would
collapse every voice to one source (a regression from AWE's `process_spatial`).

| AWE stage | Becomes | Trait | Status |
|---|---|---|---|
| Early reflections (ISM) | **`EarlyReflections`** | **`PolyModule`** (per-voice) | NEW — port `synth_awe::early_reflections`; position from `note_on` + internal `NoteMapping` param. Optional buss-level variant deferred (§3a) |
| Late reverb (FDN) + modulation + pre-delay | **evolve `Reverb`** (`effects/reverb.rs`) | `AudioEffect` | **MOSTLY EXISTS** — already an 8-ch `FdnCore` + Hadamard mixing + pre-delay + an internal ER feed (`reverb.rs:332`). Extend, don't build a parallel module |
| Room modes | **`ModalResonator`** | `AudioEffect` | **EXISTS** (`effects/modal_resonator.rs`) — 16 biquad bandpass bank; add optional room-dimension auto-tune |
| Convolution / real IR space | **`Convolver`** | `AudioEffect` | **EXISTS** (`effects/convolver.rs`) — partitioned FFT, Plate/Room/Spring/Hall; later: custom IR load |
| Air absorption | **param/one-pole inside** `EarlyReflections` + `Reverb` | — | NOT a module — graph in/out overhead exceeds the DSP cost (review §2.2) |
| Spatializer (per-voice 3D position: pan/ITD/ILD) | **`Spatializer`** | **`PolyModule`** | NEW — per-voice on the dry signal before mix; the only inherently per-voice DSP |
| Portal / Freq Warp / Diffusion | **creative params inside** `Reverb` | — | keep as coloration knobs, do not split out (review §4.2) |
| EQ | existing `Eq`/`TiltEq` | — | reuse, drop AWE's inline EQ |

So the genuinely new DSP modules are just **`EarlyReflections`** (PolyModule) and **`Spatializer`**
(PolyModule); the late tail is an *evolution of the existing `Reverb`*, and modes/convolution
already ship.

### 4a. Modulation domain — voice vs bus (pitfalls review §1.2)

A hard constraint that shapes which params live where: **the Mod Matrix / YAMS engine is
per-voice (`PolyModule`)**; once voices are mixed, a bus `AudioEffect` cannot be driven by a
voice-level source (a note envelope, per-voice LFO). Therefore:

- **Per-voice modules** (`EarlyReflections`, `Spatializer`) — full Mod-Matrix/YAMS modulation.
- **Bus modules** (`Reverb`, `ModalResonator`, `Convolver`) — driven by **sequencer automation**
  (the `ParamId::Module` lane already targets module params) or global modulators, **not** the
  per-voice matrix. The "Room macro" (§4) is a control-level fan-out, which fits this exactly.

This also right-sizes the dispatch concern (review §2.1): the `Box<dyn AudioEffect>` chain is
already the universal path for all 25 effects, so 4–5 bus nodes add negligible per-block cost vs.
per-sample DSP. Keep the FDN tail (pre-delay + FDN + chorus) bundled in one module — already the
plan — and don't over-split.

### 4b. Backward compatibility (pitfalls review §1.1) — a choice, not a blocker

AWE state is serialized in both project files (`project.rs: global.awe`, `awe_preset`) and
**patches** (`patch.rs`). Per CLAUDE.md ("no backward compatibility required — break APIs freely")
a migration layer is **optional**, not mandatory. Two levels, user's call:

- **Break freely** — old files load without the room effect; cheapest, allowed by policy.
- **One-time migration** — on load, if `global.awe` is present+active, synthesize the equivalent
  module chain (Spatializer+ER per voice → Reverb→ModalResonator) on the master/return bus with
  mapped params. Worth it only to preserve the user's existing projects (Commando, etc.).

Either way, **preset parity** (§5.8) is independent and recommended: ship module-chain presets
matching the old room presets so factory sounds survive.

### 3a. Signal topology (explicit)

```
 per voice (patch graph):   osc/filter/env → Spatializer → EarlyReflections ─┐
                                                                              ├─ voice mix →
 buss (return/master):      → Reverb (FDN tail) → ModalResonator → Convolver → EQ → out
```

Per-voice early reflections + spatial position, summed at the voice mix, then a **shared** late
tail. This is the standard pro-spatial-reverb structure and it falls out naturally from the
trait seam — no special-case `process_spatial` side-path needed.

---

## 4. The coherence decision (the one real design fork)

The monolith's single virtue: one room definition (8×5×3 m concrete) drives ER + FDN + modes
*consistently*. Independent modules lose that single source of truth. Three ways to restore it:

- **(A) Independent effects — no shared room.** Each module has its own params; user dials
  each (or a preset wires them together). Maximally composable, **zero new core plumbing**,
  fully consistent with the existing arch. Cost: no unified "room", no 3D viz tied to one
  truth, redundant geometry if you want them to agree.
- **(B) Shared Room context.** A `RoomGeometry` type read by several effects via a new
  `ProcessContext` field (`room_context: Option<&Arc<RoomGeometry>>`). RT-safe (Arc, no
  alloc/lock) but **cross-cutting**: touches the core `AudioEffect`/`ProcessContext` contract
  and `EffectChain` plumbing. Preserves coherence + a single-truth 3D picture.
- **(C) Room "controller" module via the existing Mod Matrix.** A `Room` module owns geometry
  and *drives sibling modules' params through Mod Matrix routings / a macro* — coherence with
  **no new core plumbing**, and it reuses the dynamic Mod Matrix + YAMS we just shipped. Caveat:
  modulation scope is per-instrument today; a master-bus "room" would need the matrix to reach
  bus effects.

**Decision (confirmed by review §2.3):** **(A) DSP-independent modules + coherence solved at the
control/UI layer.** A single "Room Size / Material" macro in the GUI (and in the saved project)
pushes parameter updates to the active `EarlyReflections`, `Reverb`, and `ModalResonator` so the
user experiences one unified room, while the DSP nodes stay 100 % independent and clean. This is
the reviewer's sharpening of option (C): linking at the parameter layer, **not** a shared audio
`ProcessContext`. **(B) is rejected** — passing shared pointers across the audio graph adds
synchronization risk for no DSP benefit. Build the macro after the modules exist.

---

## 5. Per-module registration checklist (reference)

Adding one `AudioEffect` touches (verified against `ModalResonator`):

1. `crates/synth_core/src/params/effects.rs` — new `XParam` enum (+ `same_kind`/`name`/`as_f32`/`with_f32`/`Default`).
2. `crates/synth_core/src/params/mod.rs` — `mod x;` + re-export; `Param::X(XParam)` variant + propagate through `same_kind` etc.; append `ModuleType::X` (append-only for save/load `Ord` stability); add to `is_effect_module()`.
3. `crates/synth_modules/src/effects/x.rs` — `Describable` (descriptor: params, ranges, ports, units, widgets) + `AudioEffect` impl (`process`/`set_param`/`get_param`/`get_params`/`module_type`/`reset`/`set_mix`/`get_mix`; opt: `set_sample_rate`/`set_sidechain_input`/`tail_samples`).
4. `crates/synth_modules/src/effects/mod.rs` — `pub mod x;` + `pub use x::X;`.
5. `crates/pertylizer/src/module_factory.rs` — `create_effect()` match arm + `ALL_MODULE_TYPES` entry.
6. MCP `list_module_types` + serialization: **automatic** (factory-driven + serde) provided type_ids/variant names stay stable and `ModuleType` is append-only.

Modulation: a `ParamModOffsets` field returned from `mod_offsets_mut()` makes every `modulatable`
param Mod-Matrix/YAMS-drivable — **but only for per-voice (`PolyModule`) modules** (see §4a). Hold
it on `EarlyReflections`/`Spatializer`; the bus effects modulate via automation instead.

Two checklist items the pitfalls review flagged, both mandatory:

7. **Regenerate schemas.** New params/`ModuleType` change the JSON schemas; run
   `cargo run -p pertylizer --bin gen_schemas` and commit — the `schemas_validate_examples` drift
   test fails the build otherwise.
8. **Preset parity.** AWE ships built-in presets (`synth_awe/src/presets.rs`: Concrete/Fabric/…).
   Define equivalent module-chain presets so the factory rooms still sound right (ties to §4b).

---

## 6. Phased migration (incremental, AWE stays alive in parallel)

AWE keeps working until the suite covers it; no big-bang removal.

- **Phase 0 — late tail.** Evolve the existing `Reverb` (`effects/reverb.rs`) to full AWE-tail
  parity: port the FDN-chorus modulation and the air-absorption + Portal/Freq-Warp/Diffusion
  coloration params from `awe_engine.rs`, reusing its parameter ramping (`awe_engine.rs:259-262`).
  Lowest-risk, highest-coverage piece. A/B against AWE's wet on a return bus.
- **Phase 1 — early reflections (PolyModule).** Port ISM into a per-voice `EarlyReflections` with
  an internal `NoteMapping` (note→position) param; fold air absorption in as a one-pole. Pre-allocate
  all 6 delay lines at max size in `new()`; ramp/cross-fade delay changes to avoid clicks. This is
  where shape-aware geometry can finally be honest (non-box mirrors) since it's now isolated.
- **Phase 2 — modes + convolution.** Adopt `ModalResonator` (add room-dimension auto-tune) and
  `Convolver` into an "acoustic space" category; document them as the resonance + IR-space pieces.
- **Phase 3 — spatializer (PolyModule).** Build the per-voice `Spatializer` (pan/ITD/ILD on the
  dry signal), retiring AWE's bespoke `process_spatial` side-path from `SynthEngine` — the single
  biggest core-audio simplification.
- **Phase 4 — Room macro.** Control/UI-level coherence (§4): one "Room" macro that fans out
  parameter updates to the active modules; reusable `Room3DWidget` that *reads* module params on
  the UI thread (DSP stays UI-free).
- **Phase 5 — retire AWE.** Ship a preset chain (Spatializer + EarlyReflections per voice → Reverb
  → ModalResonator) reproducing a useful AWE-equivalent, then remove the `synth_awe` special-case
  wiring, the `SetAweParameter` command path, and `awe_view.rs`.

Each phase is independently shippable and reviewable (fits the `/loop` step → `/code-review --fix`
→ commit cadence).

---

## 7. Decisions (resolved by the DAW review)

1. **Coherence** — ✅ (A) DSP-independent + a control/UI-level "Room" macro. (B) rejected.
2. **Portal / Freq Warp / Diffusion** — ✅ keep as creative coloration params inside `Reverb`.
3. **Air absorption** — ✅ a one-pole param folded into `EarlyReflections` + `Reverb`; no module.
4. **Late tail** — ✅ evolve the existing `Reverb` (already an 8-ch `FdnCore` + Hadamard + ER feed
   + pre-delay); no parallel `FdnReverb`.
5. **Early reflections** — ✅ per-voice `PolyModule` (position via `note_on` + internal `NoteMapping`);
   optional buss-level variant deferred.
6. **3D visualization** — ✅ a reusable `Room3DWidget` that reads module params on the UI thread;
   retire the bespoke `awe_view.rs` render. DSP stays UI-free.

### Still open
- Whether to also ship a cheap **buss-level `EarlyReflections`** variant (global position) for
  CPU-light use, alongside the per-voice one. Lean: defer until asked.
- Per-voice ER CPU budget at high polyphony (6 delay lines + 12 filters × up to 32 voices). AWE
  already proves this in `SpatialVoicePool`, but profile in Phase 1.
