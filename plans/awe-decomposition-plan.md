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

**Packaging (review §2): expose them as one combined `SpatialPanner` PolyModule, not two graph
nodes.** Two separate mono-in/stereo-out modules that both need the dry source and both sum to the
stereo bus mean ~6 cables per instrument for basic 3D placement — cable spaghetti for a fundamental
feature. A single `SpatialPanner` (mono in → pre-mixed Direct+ER stereo out) that *internally* keeps
the two DSP structs fully separate halves the module/cable count while preserving the clean DSP
split. This is exactly what the shipped code already does — `SpatialVoiceSlot` bundles
`early_reflections` + `spatializer` and `process_slot` drives both — so the combined module is the
natural port of the existing structure. Keep the internal split so the two stages remain
independently parameterised/modulatable; the merge is a GUI/packaging decision, not a DSP one.

**`SpatialPanner` implementation spec (review round 3):**

- **Ports** — input `"in"` (mono); outputs `"left"` / `"right"` mapped to `PortName::LEFT` /
  `PortName::RIGHT`. This is the primary stereo convention `voice.rs` looks for when extracting a
  voice's output (`voice.rs:981`: tries `LEFT`/`RIGHT`, then `out_l`/`out_r`, then mono `out`), so
  the module drops in as a voice end-stage with no special casing.
- **Modulatable params** — `x` / `y` / `z` (3D position), `diffusion`, `er_level` (early-reflection
  balance), `direct_level` (direct-path balance); all carried on the module's `ParamModOffsets` so the
  Mod Matrix / YAMS can drive them per-voice (§4a).
- **Parameter smoothing (zipper-noise guard) — mandatory.** Making position a per-voice modulation
  target means `x/y/z` can move fast (an envelope at note-on), and the spatializer recomputes
  `gain_left/right` + `shadow_coeff_left/right` in `update()` with **no smoothing today**
  (`spatializer.rs`), applying them as block-constant values in `process()` → audible zipper noise at
  block boundaries. The combined module must **ramp the ILD gains and head-shadow coefficients
  per-sample** across the block (linear ramp or a one-pole). ITD needs **no** smoothing — the
  interpolated delay (`read_interpolated`) already makes delay changes continuous (and yields a
  correct natural Doppler on movement).
- **Geometry at control rate only.** The heavy geometry math (`atan2`/`sin`/`sqrt` for the spatializer
  and the six ER mirror sources) must run **only at block start**, never inside the per-sample loop —
  which is already the structure (`update()` / `update_geometry()` are separate from `process()`).
  Make it an explicit invariant: `process()` reads pre-computed + ramped values; the inner loop does
  delay reads + one-poles + gain only. This is what keeps 32 voices affordable.

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

Spatializer and EarlyReflections run **in parallel** on the same dry mono voice signal — **not**
in series. Both take **mono in → stereo out** (the spatializer applies ITD/ILD/head-shadow to the
direct path; ER produces six panned mirror taps), and their stereo outputs are *summed*. Chaining
them serially (spatializer → ER) would force the spatializer's stereo back to mono at ER's input
and destroy the ITD/ILD phase. So:

```
                               ┌─► Spatializer      (direct: ITD / ILD)      ─┐
 per voice:  osc/filter/env ───┤                                             ├─► stereo sum ─► voice mix ─┐
 (mono)                        └─► EarlyReflections (6 panned wall taps)      ─┘                           │
                                                                                                          ▼
 buss (return/master):                            → Reverb (FDN tail) → ModalResonator → Convolver → EQ → out
```

This mirrors the **shipped** per-voice path exactly: `spatial_voice.rs::process_slot` feeds the
same `mono_sample` into both `early_reflections.process()` and `spatializer.process()` and sums
the two stereo results (the master-bus monolith path additionally runs a *global* spatializer on
the summed wet at the very end — `awe_engine.rs:330` — but that is the path we are retiring). Per-voice
ER + spatial position summed at the voice mix, then a **shared** late tail — the standard
pro-spatial-reverb structure; no special-case `process_spatial` side-path needed.

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

**Refinement (review §3): the macro must be engine-side and automatable, not UI-thread-only.** If
the "Room Size / Material" fan-out lives purely on the UI thread, a song-length room-size sweep
forces the user to draw three separate automation lanes (ER + Reverb + modes) and won't reproduce
in a headless / offline render where no GUI runs. Make the macro an **engine-side global parameter**
(e.g. `GlobalParam::RoomSize` / `RoomMaterial`) that the sequencer can automate on a single lane and
that the audio thread fans out to the active room modules per block. This keeps decision (A)'s DSP
independence intact — the macro only writes the same module params the user could set by hand — while
making the unified room coherent under automation. This is the same mechanism as the broader
"Macro controllers" backlog item (`TODO.md §1.3`); build the Room macro on that infrastructure
rather than a one-off UI fan-out.

**Fan-out site (review round 3): do it once per block at the top of `SynthEngine::process()`.** The
macro writes params in modules that live in two places — the master/return `EffectChain` and inside
each active voice — so the audio thread should: (1) read the automated macro value once per block;
(2) remap + write it to the global `Reverb` / `ModalResonator` params on the master bus; (3) push it
down through `Instrument` to the active voices' `SpatialPanner` params. Because every write is a plain
per-block parameter set on owned modules, this is lock-free with **no shared `Arc`/`ProcessContext`
pointer** — which is exactly why option (B) was rejected. (Per-voice `SpatialPanner` targets reached
this way still also honor their own Mod-Matrix/YAMS modulation; the macro just sets the base value.)

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
- **Phase 4 — Room macro.** Coherence via an **engine-side automatable** "Room" macro (§4
  refinement): a global param (`GlobalParam::RoomSize`/`RoomMaterial`) the sequencer automates on one
  lane and the audio thread fans out to the active room modules per block (works headless/offline),
  built on the `TODO.md §1.3` macro-controller infrastructure; plus a reusable `Room3DWidget` that
  *reads* module params on the UI thread (DSP stays UI-free).
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

### Resolved by the second review round (DSP/topology)

Cross-checked against the shipped `synth_awe` code:

7. **Voice-graph signal flow** — ✅ **fixed: it's parallel, not serial.** The §3a diagram now shows
   mono → (Spatializer ∥ EarlyReflections) → stereo sum, matching `spatial_voice.rs::process_slot`
   (both stages fed the same dry `mono_sample`). The original serial diagram was an error — it would
   have remixed the spatializer's stereo to mono at ER's input and killed ITD/ILD.
8. **Module packaging** — ✅ expose the two per-voice stages as **one combined `SpatialPanner`**
   PolyModule (mono in → Direct+ER stereo out) keeping the DSP structs internally separate — halves
   cables, mirrors `SpatialVoiceSlot`. GUI/packaging choice, not a DSP change (§3).
9. **Room macro** — ✅ sharpened to an **engine-side automatable** global param (single sequencer
   lane, audio-thread fan-out, headless-safe), not a UI-thread-only fan-out; built on the existing
   macro-controller backlog (§4 refinement, Phase 4).
10. **Bus-level ER** — ✅ kept as a **profile-gated CPU fallback** (global-position approximation),
    promoted to a committed deliverable only if Phase-1 profiling shows per-voice ER exceeds budget
    at high polyphony (§ Still open).
11. **Air absorption one-pole + Reverb evolution** — ✅ review concurs with the existing decisions
    (§7.3/§7.4); no change.

### Resolved by the third review round (implementation detail)

All confirmed against `synth_awe`; folded into the §3 `SpatialPanner` spec and §4:

12. **Zipper-noise smoothing** — ✅ `SpatialPanner` must ramp ILD gains + head-shadow coefficients
    per-sample (none today); ITD already smooth via the interpolated delay (correct Doppler).
13. **Control-rate geometry** — ✅ made explicit: `atan2`/`sin`/`sqrt` for spatializer + 6 ER mirrors
    run only at block start (already the `update()` vs `process()` split); inner loop reads ramped
    values only.
14. **Port schema** — ✅ defined: `in` (mono) → `left`/`right` (`PortName::LEFT`/`RIGHT`, the
    convention `voice.rs:981` extracts); modulatable `x`/`y`/`z`/`diffusion`/`er_level`/`direct_level`.
15. **Macro fan-out site** — ✅ once per block at the top of `SynthEngine::process()`, remapping to
    master `Reverb`/`ModalResonator` and pushing down to active voices' `SpatialPanner` — lock-free,
    no shared pointer.

### Still open
- **Cheap buss-level `EarlyReflections` variant as a CPU fallback (review §4 — promote on
  profiling).** Per-voice ER is the correct default (it's what preserves per-note 3D position), but
  the cost scales hard: at 32 voices that's 32 × 6 = 192 delay reads + ~384 one-pole filters *per
  sample*, which can saturate weaker CPUs on a big pad. Offer an opt-in where a `SpatialPanner` with
  its internal ER disabled sends its position-baked stereo to a **shared bus `EarlyReflections`**.
  Be honest about the trade-off: a bus ER runs one mirror-tap set on the *summed* signal, so it's a
  **global-position approximation**, not per-voice distinct reflections — cheaper, lower fidelity.
  Decision: **profile per-voice ER in Phase 1** (AWE already proves the per-voice path in
  `SpatialVoicePool`); if it exceeds budget at target polyphony, promote the bus-level variant to a
  committed Phase-1/2 deliverable rather than leaving it deferred.

---

## 8. Pedagogical AWE GUI Proposal & Design Review

### 8.1. Core Architectural Layout & Navigation
The new AWE interface is structured into three main visual zones inside the central panel and sidebar, visually mapping to the physical propagation stages of sound:

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│  Top Toolbar: Preset Manager & Room Macro (Size, Material, Temp)                       │
├───────────────────────────────────────┬────────────────────────────────────────────────┤
│                                       │  Interactive Control Panel                     │
│  Acoustic Signal Flow & Navigation    │  (Dynamic based on selected signal stage)      │
│  ┌──────────┐   ┌─────────┐   ┌─────┐ │                                                │
│  │Panner/ER │──►│Resonator│──►│ FDN │ │  [Sliders / Knobs]                             │
│  └──────────┘   └─────────┘   └─────┘ │  - Material Absorption Coefficients            │
│                                       │  - ITD / ILD Head-Shadow Coeffs                │
├───────────────────────────────────────┤  - Pre-delay & Feedback Matrices               │
│                                       │                                                │
│  Layered 3D Acoustic Visualizer       ├────────────────────────────────────────────────┤
│                                       │  Modulation & Automation                       │
│  [Direct Path] [ISM Rays] [Modes] [FDN]│  - LFOs / Macro Mod Matrix                     │
│                                       │  - Target mappings for active stage            │
└───────────────────────────────────────┴────────────────────────────────────────────────┘
```

The top **Acoustic Signal Flow** diagram serves as the primary navigation bar. Clicking on a stage highlights that block, updates the 3D visualizer to focus on that phenomenon, and updates the sidebar to show parameters for that specific DSP module.

### 8.2. Layered 3D Acoustic Visualizer
The visualizer represents the room in an isometric 3D cutaway. Rather than displaying everything at once, users toggle different **Visualization Layers** to isolate and study specific acoustic concepts:

1. **Direct Path (Spatializer):**
   * **ITD (Interaural Time Difference):** Visualizes the wavefront expanding from the source and hitting the left/right ears at slightly different times.
   * **ILD (Interaural Level Difference) & Head Shadow:** Draws a shaded "acoustic shadow" region behind the listener's head. When the source is in the shadow, high frequencies are visibly damped.
   * **Distance Attenuation:** Shows the signal fading as it travels from source to listener.
2. **Early Reflections (Image Source Method):**
   * **Mirror Sources:** Draws the 6 virtual mirror sources outside the room boundaries.
   * **Rays:** Renders dotted lines showing reflection paths from the source, bouncing off walls, and reaching the listener. Ray colors change to indicate material absorption.
3. **Room Modes (Modal Resonator):**
   * **Pressure Grid Heatmap:** Displays a 2D or 3D pressure grid representing the wave patterns of selected room modes.
   * **Mode Frequency Explorer:** Displays a spectrum graph of axial modes. Users select individual mode frequencies (e.g., the (1, 0, 0) fundamental) to overlay its standing wave nodes/antinodes on the floor.
4. **Late Reverb (FDN Tail):**
   * **Particle Cloud:** A particle emitter shoots out particles that bounce around walls, losing kinetic energy based on RT60 decay.
   * **Chorus Modulation:** Particles sway/pulsate gently, mapped to the modulation rate and depth.

### 8.3. Multi-Source "Orchestra / Stage" View
To support positioning multiple instruments in a single shared virtual room (simulating a band or orchestra on stage):
1. **Instrument/Track Mapping:** The UI scans the project's instrument list for any instrument containing a `SpatialPanner` in its patch editor graph.
2. **Visual Representation:** renders labeled, color-coded source markers for each active instrument (e.g., `"Violin 1"`, `"Cello"`). Active notes trigger a brief visual glow/pulsing around the marker, mapped to voice amplitude.
3. **Interactive Mixing:** Dragging any instrument's marker updates its specific `SpatialPanner` spatial coordinates (`x` and `y`) in real-time.
4. **Focus Mode:** Isolates reflection rays and wave animations for the currently selected track, keeping other instruments as faint, non-intrusive background indicators.

### 8.4. Custom Acoustic Patch View (AWE Room Graph)
To maximize code reuse and utilize the existing node-based patch editor system, we can implement the **AWE Room Graph** as a specialized canvas mode of the [PatchEditor](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor/canvas.rs):
1. **Reusing Modules:** Standard panners, resonators, and reverb modules automatically inherit rendering from `node.rs`.
2. **Reusing Cables:** Users connect panners and room effect nodes using the standard cable rendering logic.
3. **The Voice-to-Bus Boundary Constraint:**
   * **The Reality:** You **cannot** connect a voice-level output port directly to a master-level input port using a patch cable because they execute at different granularities and thread scopes in [SynthEngine::process()](file:///home/per/github/pertylizer/crates/synth_engine/src/lib.rs).
   * **Solution:** Represent the **Voice Mix summing point** as a visual separator in the room graph rather than drawing physical cables across it:
     ```
     VOICE GRAPH LAYER (Polyphonic)              MIX BUS LAYER (Stereo Sum)
     ┌─────────────────────────────────┐        ┌─────────────────────────┐
     │ ┌─────────────────┐             │        │ ┌───────────┐   ┌─────┐ │
     │ │  SpatialPanner  │──► [L/R] ───┼─┐    ┌─┼─► ModalRes  │──►│Reverb │
     │ └─────────────────┘             │ │    │ │ └───────────┘   └─────┘ │
     └─────────────────────────────────┘ │    │ └─────────────────────────┘
                                         ▼    │
                              ┌─────────────────┐
                              │ Voice Mix / Sum │
                              └─────────────────┘
     ```
4. **Locked Nodes & Inserts:** Keep the core routing (SpatialPanner -> Resonator -> Reverb -> Out) locked to prevent accidental deletion, but allow advanced users to insert creative effects directly into the paths.

