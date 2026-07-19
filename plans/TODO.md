# TODO - Pertylizer

## ~~⭐ HIGHEST PRIORITY~~ — DONE: SpatialPanner (`spp`) motion smoothness + modulation reach (found live 2026-07-12)

**All 8 items shipped** (branch `feat/spp-cv-distance-mcp-gaps`, 2026-07-13). The new `spp`
positioned sound correctly from the start (verified live: `X = -0.95` → left RMS `0.137` /
right `0.080`, mirrored at `X = +0.95`); this batch closed the motion-smoothness and
modulation-reach gaps. Shipped: (1) ER per-sample smoothing + two direct-path artifact fixes
(head-shadow median discontinuity, ITD delay-seam click); (2) `x_cv`/`y_cv`/`z_cv` position CV
inputs; (5) `distance` param + `distance_cv`; (8) seam-safe `read_interpolated_newest` primitive;
(7) offline render preserves address-based mod-matrix routings (was rendering scripted spp motion
as mono); (3) MCP batch `set_parameter` now accepts string/address values so `slot_N_dest` can be
set to `spp-1.x` + description/discovery clarified. Design verdict (6) recorded below (keep ER
unified); ergonomics note (4) still open. Details per item below.

- [ ] **6. Design question — split the early reflections into their own module?** ER is
  currently *embedded* in `spp` (two parallel DSP halves — `spatializer.rs` direct binaural +
  `early_reflections.rs` ISM — already clean separate structs, summed in `mod.rs::process`).
  Splitting into a standalone ER module would let the reflections be routed independently (e.g.
  to their own reverb bus) — but both halves must share the *same* source position, so a split
  forces the user to wire and modulate `x`/`y`/`z` **twice** and keep them in sync. Verdict lean:
  **keep unified** for position-coherence; solve independent routing with the optional
  split `direct`/`er` **outputs** from #2 instead. The genuinely different architecture (bigger,
  separate effort) is a **shared, send-based room** rather than per-voice ER — physically a
  room is one shared space, and 6 delay taps × polyphony is a lot of duplication; but a shared
  send would need to carry each voice's position, which the current bus model doesn't. Record
  the decision here before merge.

---

## 0. ~~HIGHEST PRIORITY~~ — DONE: Envelope attack/decay/release now mean their nominal time

**Shipped (branch `feat/envelope-nominal-time`).** Option 1 (analog-style
overshoot) applied consistently to attack, decay, **and** release, plus compat
**A** (break the sound, no migration; `FORMAT_VERSION` bumped `"1.0"`→`"1.1"` as a
marker only — old files still load since `version` is never compared on load).

Implementation: a single `Envelope::overshoot_target(start, dest)` aims the
one-pole a fixed fraction `k = e⁻¹/(1−e⁻¹) ≈ 0.582` of the span *past* the
destination, so each stage *crosses* its destination at exactly `t = τ = nominal
time`. `target_level: NormalizedValue` (couldn't hold >1 or <0) was replaced by a
`stage_start_level: f32` captured at each stage entry; the asymptote itself is a
documented raw `f32` (deliberately outside [0,1], never serialized). The decay
"sustain modulated above current → glide up" case is preserved via a direction
guard. Regression test `stages_complete_in_nominal_time` locks all three stages
to ±4 ms of nominal (cleanly separated from the old ~7× behavior). `velocity_pad`
and the other built-ins authored *to the number* (e.g. `attack=0.15` = "150 ms")
are now correct as-is — no blind retune done (needs an in-app ear check).

**Remaining (needs the running app):** A/B the built-ins by ear and retune any
that now sound off; the change is measurable-correct but not yet audibly reviewed.

<details><summary>Original investigation (kept for reference)</summary>

**The `Attack`/`Decay`/`Release` parameters do not mean what their descriptions
say.** `Attack` is documented as *"Attack time (silence to peak)"*
(`crates/synth_modules/src/envelope.rs:344`), but the stage is a one-pole
exponential glide toward `target_level = MAX (1.0)` that only advances to Decay
once `level >= 0.999` (`envelope.rs:240-259`), with time-constant `τ = Attack`
seconds (`to_exp_coeff` in `crates/synth_core/src/types/time.rs`,
`coef = exp(-1/(τ·fs))`). A one-pole toward 1.0 reaches:

- **90 % at `t = ln(10)·τ ≈ 2.3 × Attack`**
- **99.9 % (stage completes) at `t = ln(1000)·τ ≈ 6.9 × Attack`**

So a "20 ms" attack takes **~138 ms** to reach the peak. Decay/Release use the
same one-pole-to-threshold pattern (~6× nominal), though perceptually they land
near nominal because "most of the way" is reached sooner. The amp is **not**
involved — it reads `cv` raw per sample, no smoothing on the cv input
(`crates/synth_modules/src/amplifier.rs:194-204`); the de-zipper there is only on
its own `Level` knob.

**Verified live (2026-07-05)** via `compare_envelopes`/`analyze_note` on a
saw-pad with Attack=0.02: `attack_ms` reads a stable ~50 ms across window sizes
(the analyzer's 90 % threshold ≈ 2.3·τ = 46 ms), and the raw `rms_envelope`
peaks at ~125 ms (≈ 6.9·τ = 138 ms) — both match the math exactly. The
`envelope_estimate` hardening (`d043187c`) is correct and honest; this is a
genuine **envelope DSP / parameter-semantics** issue, not an analysis artifact.

Evidence the params are authored *to the number*: 68/69 built-in patches set
attack, and `patches/velocity_pad.rs:82` sets `.param_f("attack", 0.15)` with the
comment *"Slow attack (150ms) for pad-like swell"* — the author expected
0.15 = 150 ms but gets ~1 s to peak. So a fix makes such patches **more** correct.

### Chosen direction — Option 1: overshoot target (analog-style)

- [ ] **Make Attack/Decay/Release mean their nominal time.** Aim the attack
  glide at a target **> 1.0** (≈ 1.58, since `1.58·(1−e⁻¹) ≈ 0.999`) so the curve
  crosses the completion threshold at ≈ one time-constant = the nominal Attack.
  Apply the same fix consistently to Decay and Release (else attack is accurate
  while decay/release stay ~6× time-constants). Keeps the natural exponential
  feel while the number becomes meaningful.
  - **Implementation note:** `target_level` is a `NormalizedValue`, which clamps
    to [0, 1] — the attack overshoot target needs a raw `f32` (or a dedicated
    "attack target" constant), so a small refactor of the attack branch in
    `envelope.rs`, not a one-liner. Decide the exact overshoot from the completion
    threshold used.
  - **Spot-check by ear:** A/B `velocity_pad` against its own "150 ms" intent,
    then retune the handful of built-in patches that sound wrong.

### Loading older projects / instruments (compat)

- **Files still load — no format break.** Attack is stored as a plain number
  (seconds) in module params; Option 1 changes DSP behavior, not the schema, so
  `.pertyproj` and patch files parse unchanged (`ProjectFile::FORMAT_VERSION`
  stays parseable).
- **But the SOUND changes:** every envelope with a non-tiny attack plays ~7×
  faster to peak. Impact scales with attack length — plucks (attack≈0) unaffected;
  pads/swells (0.15–0.5 s) dramatically snappier.
- **Decision needed — how to treat old user content:**
  - **A (recommended): break the sound, no migration.** Aligned with the
    project's "no backward compatibility required" stance; the built-in patches
    (authored to the number) get *more* correct. Bump `FORMAT_VERSION` "1.0"→"1.1"
    purely as a marker; retune the few built-ins that sound off. User projects
    with long deliberate attacks need a manual re-tune.
  - **B: behavior-preserving migration.** On load of a "1.0" file, scale every
    Envelope module's Attack (and Decay/Release if fixed) by the conversion factor
    (~6.9×), gated on `ProjectFile.version` so "1.1" saves aren't re-scaled.
    Precedent exists: `upgrade_legacy_mod_matrix` (`patch.rs:549`) and
    `resolve_stereo_out_port` run transforms on load. **Caveats:** the migration
    preserves only one point on the curve (the completion time) not the whole
    shape — old is a long-tailed exponential, new is overshoot-truncated (~2.7×
    for the 90 % point vs ~6.9× for completion), so it's approximate; and it also
    "un-fixes" patches that were authored to the number (like `velocity_pad`).
- **Suggested first step:** a small spike — fix only the attack curve and A/B
  `velocity_pad` against its "150 ms" intent before committing to the full
  three-stage change + the A-vs-B migration decision.

</details>

---

## 1. Sequencer & Arrangement

### 1.1 Tempo automation

**Done.** The **tempo map** (position-specific tempo + accelerando/ritardando ramps)
shipped in full: MCP tools (`set_tempo_at` / `remove_tempo_at` / `get_tempo_map`, each
with a `ramp` flag) + the map in `get_song_info`; ramp interpolation in `tempo_at` and
ramp-aware `tick_to_seconds` / `seconds_to_tick`; ramp-aware undo (`SetTempo` +
`MoveTempo`); and a draggable GUI tempo lane in the arrangement — curve + handles with
drag/add/remove, hover glow, a dynamic BPM axis (frozen during a drag), and the global
default drawn/labelled distinct from map points. (Not to be confused with the generic
`AutomationTarget::Global(Tempo)` lane, removed for good 2026-06-01 — a tempo-map point
can't be a per-block lane value; that dead code is not coming back.)

### 1.2 Section markers

- [ ] Verse, chorus, bridge labels in the arrangement

### 1.5 Pattern-loop presentation and controls

**Playback looping shipped:** placement positions now wrap through type-safe
`PatternTick::looping_at`; crossing notes keep their absolute NoteOff and retrigger on the
next pass, while automation restarts in pattern space each pass.
- [ ] **Mini-note visualization should mirror the loop.** `NoteMiniature.start_frac` is currently
  fraction-of-pattern-length. For loop-within semantics the rendering in
  `gui/sequencer/arrangement.rs` (mini-note loop, near the `inst_color_cache` use) should repeat the miniature
  across the placement's `effective_length / pattern.length` iterations, so the user sees what they hear.
- [ ] **Add a toggle on `PatternPlacement`** (`loop_mode: PlacementLoopMode { Clip, Repeat }`, default
  `Repeat` to match DAW expectations). Surface in the placement context menu and in the right-edge
  resize-grab tooltip so the user can choose per placement. Migration of older songs: default existing
  placements to `Clip` so behaviour is preserved, or `Repeat` if we accept a one-time semantic change.

### 1.6 Persist the transport loop region across save/load

---

## 2. Sound Design — Expanded Capabilities

### 2.1 Sample & wavetable import

- [ ] Sample import — load .wav files as oscillator source or in granular synth
- [ ] Wavetable import — load custom wavetables (Serum format, single-cycle .wav)

### 2.2 Alternative tunings

- [ ] **Support tunings other than 12-TET.** Today the pitch path hardcodes 12-tone equal
  temperament when converting `MidiNote` → `Hertz`. Route that conversion through a pluggable
  tuning table so the synth can play just intonation (pure integer ratios like `3/2`, `5/4`),
  microtonal systems (19/22/31-EDO, quarter-tones), and arbitrary historical/non-Western scales.
- [ ] **Load Scala `.scl` files** as the import format — the de facto standard for sharing
  tunings (scale steps given in cents or as frequency ratios). Parsing a `.scl` file fills the
  tuning table from the previous item.

### 2.3 Expression & articulation

**Remaining open work from the retired note-expression roadmap:**

- [ ] **Phase D residual — automate master/return effect params.** A `Filter` can be
  placed on the master or a return bus and set today (`set_master_effect_parameter` /
  `set_return_effect_parameter`), but its cutoff **cannot be swept by an automation
  lane**: `AutomationTarget::Module` resolves only against instrument-owned modules
  (`synth_engine.rs:~3293`, `instruments.iter_mut().find(...)`). Add a target variant
  (e.g. `AutomationTarget::{MasterEffect, ReturnEffect}` keyed by slot + `param_id`)
  dispatched through the same override layer as A2. Delivers exact *shared* SID-style
  filter sweeps. **S** task — build only when a tune genuinely needs a shared (not
  per-instrument) automated sweep; per-instrument sweeps are already covered by A2.

### 2.4 Polyphony settings

**Done.** The feature — **unison detune + spread controls** for the voice-allocator's
global `AllocationMode::Unison` — shipped earlier: detune end-to-end (`268441f9`)
and per-voice stereo spread (`eac9b020`). The remaining **MCP surface** is now
also shipped: `get_instrument_info` reads the whole allocator config
(`allocation_mode`, `stealing_strategy`, `unison_detune`, `unison_spread`,
`max_voices`), and a dedicated array tool `set_allocator_config` sets any subset
as a group. `max_voices` is stored RT-safely (no live `resize()`) and applies on
the next voice-graph reconstruct/load; the other four are live. `Display`/`FromStr`
on the enums make the string round-trip authoritative. End-to-end round-trip tests
in `mcp_allocator_config.rs` drive the real engine.

### 2.6 YAMS scripting follow-ups

- [ ] **Per-sample pitch binding for `note_hz` in AudioScript.** The `note_hz`
  context var is currently *block-constant* — resolved once per block by the
  voice (`ScriptCtx`), same as the oscillator's own `set_voice_pitch`. So an
  audio-rate `phasor(note_hz)` does not follow intra-block pitch bend / glide at
  per-sample resolution; fast portamento steps once per block. A future
  per-sample pitch binding (analogous to how the audio-in registers are injected
  each sample in `eval_block`, via `AudioBindings`) would make scripted
  oscillators track fast portamento faithfully. Small-to-medium; only matters for
  audible fast glides.
  **Investigated 2026-07-03 (deferred until someone actually needs it):** the
  `AudioBindings`/`eval_block` half is trivial (add
  `note_hz: Option<ScriptRegister>`, a
  `bindings_for` arm, and a per-sample `set_source`). The real work is that
  **there is no per-sample pitch signal in the engine at all** — the whole voice
  pitch pipeline is block-rate: `glide.update` runs once per block
  (`instrument.rs:~1277`) and `process_audio` delivers a single scalar
  `set_voice_pitch(freq)` (`voice.rs:~973`). Pragmatic fix: have the voice expose
  this block's start→end frequency (remember the previous block's `note_hz`) via a
  small new module method, and **lerp per sample inside `eval_block`** — glide/bend
  are piecewise-linear, so a per-block linear ramp reconstructs the trajectory
  exactly. Bundle the per-sample inputs into a struct rather than growing
  `eval_block`'s already-`too_many_arguments` signature.
### 2.7 Script-exposed params follow-ups
*(IMPLEMENTED on branch `feat/script-exposed-params` — both parts landed, workspace
green (dev + release), NOT merged/eyeballed. The `Script`
module became a one-program **4 CV-in (`in1..in4`) / 4 CV-out (`out1..out4`)** node, and both
`Script` and `AudioScript` gained user-declared `param` knobs — real descriptor params:
GUI faceplate + mod-matrix dest + automation + save + cross-script `scr-1.drive` reads.)*

- [ ] **In-app GUI eyeball (pending verification, not a bug).** The 4-in/4-out faceplate
  ports, the ƒx editor's live control-ports status, and the declared-knob rendering are
  wired + unit/integration-tested but never clicked through in the running app. Confirm: a
  `param drive = 0.5` shows a **Drive** knob that changes the sound; the ƒx popup lists the
  declared params; rewiring a cable into `in1` changes the read without editing the script;
  editing a live script to add a `param` makes the knob appear with no audio glitch.
- [ ] **Cross-script reads don't see automation overrides.** `resolve_param_source`
  (`voice.rs`) reads another script's knob (`scr-1.drive`) as its *stored base* value via
  `get_param` — the transient sequencer-automation override (and the per-block mod-offset,
  deliberately) are excluded. So one script reading another's *automated* knob sees the
  un-automated value. Minor v1 limitation; if it matters, route the read through the knob
  store's effective-minus-offset value. **S**.
- [ ] **Optional per-CV-port display labels (`in1 "rate"` / `out1 "pitch"`).** Deferred from
  plan §2.A — the `param` string label/tooltip shipped, but the cosmetic per-port faceplate
  labels did not; ports show bare `in1..in4` / `out1..out4`. Needs a small header-declaration
  grammar addition (a bare `in1 "label"` statement) + carrying the label onto the port
  descriptor (it must NOT change the port id, so no cable churn). **S–M**, purely cosmetic.
- [ ] **Confirm `rebuild_instrument_preserve_automation` + a removed `param`.** Plan §6 open
  question: editing a live script to *remove* a knob a lane was bound to should drop the
  orphaned automation lane and warn, not panic. The store degrades (the knob vanishes from the
  descriptor + the fixed arrays reindex), but the rebuild/automation interaction wasn't
  specifically exercised — worth an in-app check.
- [ ] **Built-in knob smoothing (`smooth()` / slew) for audio-rate params.** Plan §6: a
  declared `param` is block-constant, so under fast automation/mod it *steps* at each block
  boundary — audible on a steep `audio_script` knob (filter cutoff, gain). v1 leaves click-free
  knobs to user-side per-sample smoothing in the script (`s = s + (drive - s) * 0.005` via a
  `state` cell); a built-in `smooth(x, coeff)` helper (or a `param … smooth` modifier) would
  remove the boilerplate. Defer unless the manual one-pole proves too fiddly.
- [ ] **Unit keyword for `param` metadata.** v1 carries default + `[min,max]` + `"label"` +
  `"tooltip"`; the `ParameterUnit` (Hz/dB/…), bipolar-vs-unipolar, and response curve are
  deferred (default linear/unipolar). A later optional `param … unit hz` keyword maps a
  recognized token → the `ParameterUnit` enum, else `None`. **S**.

### 2.8 Per-oscillator glide (portamento)

**Shipped (branch `feat/per-oscillator-glide`, squash-merged 2026-07-13).** All 8
pitched oscillators (`oscillator`, `sub_osc`, `wavetable_osc`, `math_oscillator`,
`additive_osc`, `granular_osc`, `fractal_osc`, `sid_oscillator`) gained a
`glide_time` param: when `> 0` the oscillator runs its own portamento (shared
`PitchGlide` in synth_dsp / `OscGlide` in synth_modules) toward the raw note
target and re-applies bend/vibrato on top, overriding the voice-level glide for
itself; `= 0` is bit-identical to before. `VoicePitch` (synth_core) decomposes the
per-block pitch broadcast into `played` / `note_target` / `expr` / `note`. Gate
green; final full-branch review clean. Plan doc deleted with the merge.

- [ ] **In-app eyeball (pending — not a bug).** Two oscillators in one voice: set
  `glide_time > 0` on one, confirm it audibly portamentos between notes while the
  other jumps; confirm the **Glide** knob renders and changes the sound; confirm
  no-glide patches sound unchanged and that pitch-bend/vibrato still track on a
  gliding oscillator.
- [ ] **Extend the opt-in param to the other pitch-tracking sources.** `voice_synth`,
  `vocal_tract`, `fof`, `ring_mod`, `padsynth`, `sampler` took the `VoicePitch`
  signature change but not the `glide_time` param (plan §5 "candidates"). Each is a
  small `OscGlide` adoption if wanted. **S each.**
- [ ] **(Optional) Make `glide_time` modulatable/automatable.** It's deliberately
  `.modulatable(false)` on every module because `OscGlide` doesn't read
  `ParamModOffsets`/automation overrides for it — marking it modulatable without
  that would be a silent-drop bug (`is_automatable()` also gates on `modulatable`).
  To let an LFO/automation sweep the glide time, have `OscGlide` consume the mod
  offset for `glide_time`. **S–M.**
- [ ] **(Optional) Per-note glide + stepped glissando, per-oscillator.** The
  per-note glide (tracker import, `GlideState::start_from`) and stepped/glissando
  voice glides stay voice-level; a per-osc glide is always continuous. Mirror them
  per-oscillator only if a use case appears.

### 2.9 Mod Grid follow-ups

**Shipped (branch `feat/mod-grid`, 11 commits, gate green dev + release, final
full-branch review clean — NOT merged, NOT eyeballed in-app).** A third node view
beside the patch editor and Note Grid: a pool of control-rate modulator graphs
(`ModGraph`, `ModGraphScope::{Global, Track}`) whose outputs write additive
block-constant offsets into the automation target space. Landed: data model +
`Song` pool + `mod_grid_generation` counter; engine control-rate pre-pass
(`process_mod_grid`) running before instruments with **track (volume/pan/pitch) +
global (master volume)** write paths composed additively over lanes; cheap value
sources **Macro / Transport / Audio-tap** (`ModSource` enum, tap follows an
instrument/master RMS through a one-pole so it ducks directly); app-side builder
(`mod_grid_build`) + GUI generation-watch sync shipping the runtime via
`EngineCommand::SetModGrid` (old runtime freed off-thread); MCP tool family
(pool/nodes/cables/`list_mod_targets` provenance); the GUI node view (pool +
Scene canvas, named descriptor ports, module→module cables, scope toggle +
per-track assignment, `UndoAction::SetModGraph`); persistence + offline-render
wiring; and a lane provenance chip + jump + `＋ Mod Grid` quick-assign. Also folded in the fix-first
`clear_mod_offsets`-unconditional bug (an offset writer without a Mod Matrix
module used to latch offsets forever).

- [ ] **Robustness follow-up: surface orphaned tracks (track→instrument that doesn't
  exist).** Such a track silently drops **all** track-level control (fader, mute,
  automation lanes, *and* Mod Grid), yet its notes still play (at unity) — a confusing
  silent trap (it cost a long debugging session above). Options: a `lint_project`
  warning, a GUI badge on the track header, and/or repointing/clearing the reference on
  load. General track-management, not Mod-Grid-specific. **S–M.**
**Follow-ups shipped 2026-07-18** (branch `feat/mod-grid`, one commit each, gate
green + per-step reviews). The six deferred items are done:

- [ ] **Optional: fuller §6 exit-gate walk-through.** Not blocking (the GUI was
  eyeballed above). If you want an exhaustive live pass: `LFO → Track 2 volume` with
  no lane; a Track graph on two tracks; `LFO → Pitch` host-only; an Audio-tap duck;
  routing-removal returns to base; a seeded render matches live; `Macro → LFO.rate_cv`;
  a live `MidiCc`; sustain hold+release.
- [ ] **Cache the Module-target snapshot (7.1 refinement).** `egui_backend` rebuilds
  the per-instrument `module_target_groups` for **every** instrument every frame the
  Mod Grid view is open (a registry lock + descriptor clone each). Fine for a handful
  of instruments (egui repaints on demand), but cache it on a structure-generation
  bump — or build only for the target's selected instrument — for large projects. **S.**
- [ ] **Per-graph CPU attribution (7.6 refinement).** The header CPU is total across
  all running instances (reuses the per-stage timing). Per-graph cost needs separate
  instrumentation (time each `ModGridInstance` and map to its `ModGraphId`, exposed to
  the GUI). **M.**
- [ ] **Soft cap on Mod Grid assignments.** Once per-graph CPU attribution is available,
  use the measured instance cost to decide whether track-scoped graphs need a soft
  assignment limit. Warn in the GUI when the estimated aggregate cost exceeds the
  budget; do not impose a hard limit unless real projects demonstrate a need. **S–M.**
- [ ] **Per-voice decorrelation for RandomGates / TuringMachine (optional).**
  DriftGenerator now decorrelates per voice via `set_voice_index`; RandomGates and
  TuringMachine still run in lockstep across a patch's voices (they don't override it).
  Add `set_voice_index` to them too if per-voice variation is wanted — but it may be
  intentional for rhythmic modules (a consistent gate pattern), so decide per module. **S.**
- [ ] **Full sustain-pedal is CC64-specific.** The pedal path only handles CC64; other
  hold-type pedals (sostenuto CC66, soft CC67) are not modelled. Low priority. **S.**
- [ ] **CPU metering uses `Instant::now()` every audio callback (review finding 5).**
  The mod-grid stage timing follows the *pre-existing* per-stage pattern (voices /
  module-graph / master-fx already do 4 clock reads per callback). `Instant::now()`
  isn't guaranteed to be a syscall-free vDSO read on every platform, which the RT rules
  frown on, and the reads happen even when the CPU tooltip is hidden. Consider making the
  whole metering opt-in/off-by-default or sampling it sparsely — a cross-cutting change to
  the existing metering subsystem, not mod-grid-specific. **S–M.**

---

## 3. UI & Visual Polish

### 3.1 Improve module knobs

- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 3.2 Redesign instrument list

- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 3.3 Module Groups — Phase 2–3

- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

### 3.4 Mod Matrix routing visibility

**Done.** Header badges and MCP surfacing shipped in v0.289.0
(`get_mod_matrix_routings`, virtual `"matrix"` port on `list_modules`). The
script-source markers then shipped in full: `ModRole` was replaced by a
multi-kind `ModMarkers` set, and `PatchAnalysis` now extracts sources read *from
inside a script* — Mod Matrix slot expressions, `ScriptModule` (`"scr"`), and
`AudioScript` (`"asc"`) — not just scalar `slot_addrs`, each tagged with its
consumer kind (per-slot compile cache; disabled Mod Matrix slots emit nothing).
Three source kinds are distinguished by icon+colour: Mod Matrix `↗` purple,
Script `ƒx` teal, AudioScript `ƒx` yellow, plus the Mod Matrix destination `↙`
purple. Markers render on param labels/knobs, output-port corners (glyph inside
the fixed 20×20 box), the module footer badge, and the macro rail — each kind in
its **own fixed corner** (knobs push the glyph just outside the circle, grown
vertically inward so it clears the label), each glyph with its own hover tooltip.
Shipped alongside: GUI patch load/save now install/capture per-slot control
scripts (`patch_bridge::load_module` + `create_patch_from_editor`), which the GUI
paths had been silently dropping. No "what feeds what" tooltip yet — a possible
future refinement, but not tracked as open work.

### 3.5 MSEG UI overhaul (problematic — needs review)

- [ ] **The MSEG module UI is very problematic and must be reworked.** MSEG is a multi-segment
  envelope (up to 16 segments, each with time/level/curve, plus loop start/end), but it currently has
  **no graphical editor** — the only UI is the generic descriptor-driven knob grid. Consequences:
    1. **The actual envelope shape is not editable in the GUI.** The 48 per-segment params
       (`seg{0..15}_{time,level,curve}`) are deliberately `WidgetHint::Hidden` (added so the shape
       round-trips through save/load and is MCP-settable — see the State Sync work), so the only way
       to draw/shape the envelope today is per-id via MCP `set_parameter`. There is no way to do it by
       hand in the app.
    2. **The visible knobs are awkward.** `Segments`/`Sustain Seg`/`Loop Start`/`Loop End` are integer
       knobs (now `.step(1.0)`-snapped) and `Time Scale` is a multiplier — a grid of knobs is a poor fit
       for what is fundamentally a *curve*.
       Fix direction: build a proper **graphical multi-segment envelope editor** (drag segment
       nodes for time/level, drag handles for per-segment curve, visible sustain + loop-region markers),
       rendered via a custom widget (`WidgetHint::EnvelopeEditor` already exists as a hint). The Hidden
       segment params can stay as the persistence/MCP backing; the editor just reads/writes them. Also
       consider an array-style MCP tool (`set_mseg_segments`) so the shape can be set in one call instead of
       ~50 individual `set_parameter`s. Review the whole MSEG UX as part of this.

### 3.6 `ModuleParam` single-definition cleanup (MAYBE — aesthetics only, future)

- [ ] **Collapse the inherent-vs-trait duplication for the param method set — purely for
  "one definition" tidiness, low priority.** Phase 7 of the param-type-system work
  (`plans/param-type-system-plan.md` §10, shipped) added the `ModuleParam` trait via a
  delegation macro: each of the 67 `*Param` enums + `Param` `impl ModuleParam` by
  *forwarding* to the existing inherent methods (`fn as_f32(&self) { Self::as_f32(self) }`).
  So the bodies live in the inherent impls and the trait is a thin forwarding layer — there
  is a small amount of duplication (the ~470 macro-generated one-liners). The "pure" form
  would make `ModuleParam` the **single** definition and delete the inherent methods.
    - **Why it's only a maybe:** the literal version means the trait must be in scope at the
      **~2489 call sites** of `.as_f32()`/`.with_f32()`/`.same_kind()` across the workspace
      (via a `synth_core::prelude` glob in dozens of files). That is a large, sprawling,
      purely-cosmetic diff with **zero functional/correctness gain** — the aggregate `Param`
      match + the macro already force the full contract on every enum (a missing method is a
      compile error today). YAGNI: nothing currently needs it.
    - **If we ever do it:** **own branch + own session** (it touches most files in the
      workspace). Mechanism: move each method body into `impl ModuleParam for X`, delete the
      inherent method, and add `use synth_core::prelude::*` where the compiler flags missing
      trait scope. Let the compiler drive the call-site fixes; gate per crate.

### 3.7 Unified list-panel follow-ups (deferred from code review)

Surfaced during the shared left-list-panel work (`feat/uniform-list-panels`,
2026-06-24, `gui/list_panel.rs` + Instruments/Patterns/Samples panels). None are
correctness bugs (those were fixed in that branch); these are the cleanup/
efficiency/altitude items deliberately left out of that change.

- [ ] **Cache sample-usage instead of recomputing every frame.** The Sample view
  rebuilds `used_sample_ids` on every repaint by calling
  `self.session.state().shared_graph.get_all_modules()` (which clones *every*
  module snapshot incl. its full `parameters` vec) and scanning for
  `Param::Sampler(SamplerParam::SampleSelect(..))` — see the sample-view call site
  in `gui/egui_backend.rs` (the `used_sample_ids` block just before
  `draw_sample_view`). Only runs while the Sample tab is open, but it allocates +
  walks the whole graph ~60×/sec. Fix: cache the id set and invalidate on a
  graph-version change (`shared_graph.version()`), or expose a lighter query that
  yields just the referenced `SampleId`s without cloning snapshots.
- [ ] **Generalize the per-panel scaffolding (altitude).** `list_panel::row`/
  `header`/`search_box` centralize the row visuals, but the three call sites
  (`render_instruments_panel` in `gui/egui_backend.rs`, `draw_browser_row` in
  `gui/pattern_view.rs`, and the sample loop in `gui/sample_view.rs`) still repeat
  the same surrounding boilerplate: build the used/unused tooltip string, dispatch
  `clicked()`/`double_clicked()`, apply the search-needle filter, and render the
  empty-state placeholder. A higher-altitude helper taking
  `(selected, used, name, tip, kebab) -> RowOutcome { clicked, double_clicked }`
  would remove the repetition the first pass left behind.
- [ ] **Drop the redundant `select` flag in the sample row loop**
  (`gui/sample_view.rs`). It is only ever read in `if select || rename` and
  `rename` already implies selection; the selection assignment can test the row
  response (and `rename`) directly. Pure cleanup, no behavior change.
### 3.8 Shared widget helpers follow-ups (evaluating Phase 2 residual)

Residual after the shared-widget-helpers work landed — these are the remaining areas to polish the GUI helpers layer:

- [ ] **Global FileDialog memory across kinds.** Refactor `ensure_dialog`
  in [dialogs.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/dialogs.rs) to reuse a single global
  `FileDialog` instance across all kinds (Open/Save Patch, templates, etc.) rather than rebuilding it when
  `file_dialog_kind` changes. Update its `config_mut().file_filters` dynamically on every open. This enables directory
  memory and highlighting (`retain_selected_entry`) to survive switching between Open and Save actions.
- [ ] **Address inline toggle button variations.** Several inline toggle styles (e.g. M/S muting/soloing badges,
  custom-colored selections) still bypass `toggle_button_colored`. Create a flexible `toggle_badge` or
  `selectable_toggle` helper to cover these and keep sizes consistent (preventing drift).
- [ ] **Perform a visual eyeball check on normalized captions.** Verify that the normalized size shift (~9px to 10px
  `size_small`) for the 24 migrated `.small()` labels does not cause visual clipping or alignment issues in tight
  spaces (especially grid cells
  in [tracker.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/tracker.rs) and Vol/Pan knob
  rows in [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs)).

### 3.9 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an
  egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor
  `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
    * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on
      crates.io and its fork was dropped.
    * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the
      `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build.
      Watch: https://github.com/get200/egui-remixicon

### 3.10 Review the mixer view layout

- [ ] **Give the mixer view (`gui/mixer_view.rs`) a proper layout pass.** The module-header
  consolidation (2026-07-01) shared `draw_module_header`'s right-alignment across the mixer, switched
  its strips to the shared `icon_button`, and sized channel strips / return columns off the
  `ModuleWidth` buckets (`Small` 192 / `Medium` 256) instead of hardcoded 108/200 — which fixed the
  header title/icon overlap but was a spot fix, not a considered layout. Still worth reviewing: overall
  strip proportions and spacing at the new widths, sends/pan/meter/fader arrangement inside a strip, the
  master strip, and how it all reads next to the patch editor. Vertical scrolling was just added
  (`ScrollArea::both`); confirm it behaves with tall strips and many channels.

---

## 4. AI & Automation

### 4.1 MCP & AI Interaction

---

## 5. Architectural & Performance Hardening

> **Ground rule: nothing here gets optimized before it is a *measured* problem.**
> Readable, well-structured code wins over speculative micro-optimization. The
> long catalogue of speculative cycle-shaving items (SIMD/alignment, `rem_euclid`
> tricks, mmap, dashmap, PGO, reciprocal-mult, etc.) was **removed on 2026-06-30**
> — it traded clarity for unmeasured gains. What remains is split into two tiers:
>
> - **Tier A = cheap wins** that raise safety / readability / diagnostics or fix a
    > *known* bug. Do them whenever; no trigger needed. Ordered cheapest-first.
> - **Tier B = real problems that need a trigger.** Each is a genuine
    > correctness/RT-safety issue, but architectural enough that it should be driven
    > by an *actually observed symptom*, not done pre-emptively. Ordered by impact.

### Tier A — cheap quality/safety wins (do whenever, cheapest first)

These are not performance bets; they make the code safer, clearer, or more
debuggable at low cost.

#### A1. Thread diagnostics: named background threads

#### A2. Code quality: standardise on to_radians / to_degrees

#### A3. Invariant checking: debug_assert in new_unchecked constructors

#### A4. DSP: prevent CPU denormal spikes via FTZ/DAZ

#### A5. UX: custom panic hook for desktop crash diagnostics

#### A6. Compile-time safety: static assertions for lock-free structs

#### A7. Real-time safety: automated allocation testing with assert-no-alloc

### Tier B — real problems, trigger-based (do when the symptom appears)

Principled correctness/RT-safety issues, not guesses — but each is architectural
enough to be driven by an actual observed symptom. Ordered by likely impact.

#### B1. Architectural: RCU / arc-swap to remove RwLock<Song> read locks on the audio thread

- [ ] **Replace `RwLock<Song>` with an RCU/double-buffering pattern.** The audio
  thread uses `try_read()` on `Arc<RwLock<Song>>`. When the UI takes a write lock
  (e.g. a large project mutation), `try_read()` fails and the audio thread skips
  blocks / plays silence — an **audible dropout during heavy editing**, not a perf
  nicety. Evaluate RCU pointers (`arc-swap`) or double-buffered pointer swaps for
  lock-free, contention-free reads. **Trigger: do it if you hear dropouts while
  editing big projects.**

#### B2. Reproducibility & RT safety: replace fastrand on the audio thread

- [ ] **Replace `fastrand` usage on the audio thread.** `synth_core/src/hash.rs`
  already states the audio path should never call an RNG — to keep renders
  *deterministic/reproducible* and avoid TLS lookups. Yet `noise.rs`,
  `drift_generator.rs`, and `oscillator.rs` call `fastrand::f32()`. Refactor them
  onto the deterministic SplitMix64 helpers in `synth_core::hash`. A
  correctness/reproducibility fix, not a speed bet.

#### B3. Real-time safety: metadata deallocation on the audio thread

- [ ] **Stop deallocating metadata on the audio thread.** Commands like
  `RenameInstrument` / `SetInstrumentDescription` move `String`s by value to the
  audio thread; dropping them heap-deallocates there — a direct violation of the
  project's own RT rules. Evaluate separating metadata from the audio engine's
  structs (keep it in GUI state / shared graph; the audio thread holds only numeric
  IDs), or return old metadata to the UI thread via a queue for disposal. Best
  folded into the next change to the instrument-state model.

#### B4. Real-time safety: replace HashMap usage on the audio thread

- [ ] **Remove `HashMap` lookups/updates from the audio thread.**
  `last_automation_values`, `track_auto`, `prev_instrument_outputs`, and
  `track_controls` use `std::collections::HashMap` (SipHash 1-3, worst-case O(n) on
  collisions) in `synth_engine.rs` / `sequencer_engine.rs` — a latency-jitter risk.
  Evaluate flat arrays (`[Option<T>; MAX]`) or linear search over small stack
  arrays, which are cache-friendly with deterministic WCET, and can read more
  cleanly. **Trigger: when jitter is actually measured.**

#### B5. DSP: parameter smoothing for CV/cutoff changes

- [ ] **Add parameter smoothing to hot paths.** Sudden block-by-block parameter
  jumps cause audible clicks / "zipper noise". A lightweight smoother (1-pole
  lowpass or sample-rate linear ramp) in oscillators, amplifiers, and filters
  guarantees smooth transitions. An **audible-quality** fix (effectively a small
  feature). **Trigger: when you hear clicks on cutoff/CV moves.**

#### B6. Per-track pre-FX sends and metering for shared instruments

- [ ] **Add per-track accumulators for pre-FX sends and metering.** Track
  volume/pan/mute now resolves through a generation-marked `TrackId` table and is
  applied per voice before the shared instrument effect chain, so shared-instrument
  dry playback is track-correct. Pre-FX sends and meters still aggregate at the
  instrument channel; split those taps per track if a project needs independent
  routing or meters for tracks sharing one instrument. **Trigger: when such a
  project needs separate pre-FX sends or meters.**

---

## 6. Future features (harvested from retired plan docs)

> **Consolidated 2026-07-13.** The standalone design/status docs that used to live
> under `plans/` were folded in here. Their full text is recoverable from git
> history; only their remaining open work is captured below.

### 6.1 Deferred design work

- [ ] **Cable layering, gradients, and focus mode.** Render cables and flow
  particles transparently in front of module faceplates, colour cross-domain
  cables with a source→destination gradient, and dim cables unrelated to the
  current selection. Keep the existing orthogonal routing and telemetry model;
  this is a scoped visual pass touching `theme.rs`, `cable.rs`, and `wiring.rs`.
### 6.2 Note Grid — deferred earned-escalation
*(from `plans/note-grid.md`; Note Grid shipped + squash-merged to main 2026-07-13 @`65f12900`. Full plan in git history.)*

- [ ] **DAG (branch/merge) escalation** — relax the linear-stream validation; add
  `KeyZoneSplitter`/`VelocitySplitter`/`RoundRobin` + merge semantics
  (connection-sorted concat under the buffer cap). Data model is already
  graph-shaped (no serde change); the hard part is **held-pitch resolution through a
  *branched* upstream** (`expand_pitch` in `note_processor.rs` today assumes one
  linear upstream chain). Value is low (single terminal output, no cross-instrument
  routing) — **only if real usage shows branching is needed**, not scheduled.
- [ ] **Track scope** (`SequencerTrack::note_graph`) — graph over the merged stream
  of all placements on a track: tails across pattern boundaries + the future
  live-input path (Ableton/Bitwig model). Costs: cross-placement look-back source
  material + a freeze-semantics answer.
- [ ] **`NoteScriptGenerator`** (YAMS `note_event` + `emit`) — statement-only 1-to-N
  generation (`emit(pitch,vel,dur[,delay])`, `MAX_SCRIPT_EMITS=16`), purity via the
  same bounded look-back idiom as Delay/Ratchet. Real language-surface work
  (parser/compiler/VM/`yamsfmt`/docs); may need a `StreamOnset`-style anti-stall cap.
- [ ] **`MicrotonalTuner` note-graph module** — per-note detune field + event
  plumbing. Related to §2.2 (alternative tunings) but delivered as a Note Grid node.
- [ ] **Misc later**: per-reference overrides, per-scope `Vec` of graphs,
  cross-track routing, cable telemetry, per-node tracker taps.

### 6.3 SID oscillator — open fidelity follow-ups
*(from `plans/sid-oscillator-module.md`; the `sid` module shipped to main @`d0d872f3`. Full spec + expert reviews in git history.)*

- [ ] **Oversampled ring/sync bus (ring-mod HF fidelity).** Ring sideband
  *positions* are exact but broadband `compare_spectra` distance holds ~16.9 dB vs
  reSID — pinned to **host-rate ring-edge jitter (~22.7 µs)**: the neighbour's `msb`
  is read once per host sample, outside the oversample loop. Fix needs the source
  `sid` to expose its MSB at the 4× rate (or the sub-sample crossing fraction) — a
  cross-module `msb`-port contract change, out of scope for a local
  `sid_oscillator.rs` edit. The one-sided PolyBLEP fold-flip already shipped (keep
  it). *(Same item as the ring note under §6.6.)*
- [ ] **Golden reSID A/B acceptance re-run.** Re-run the §11 reSID matrix (the
  sid-analyzer harness) as the acceptance gate for the shipped option-C combine /
  ring / `DcBlock` changes.

### 6.4 AccessKit / egui-inspection — deferred
*(from `plans/accesskit-custom-widgets.md`; shipped to main @`c7372dae`, container-level exposure across all views. Full inventory in git history.)*

- [ ] **Per-element canvas drivability.** v1 exposed the big canvases (piano roll,
  tracker, arrangement, keyboard, sample) only at *container* level. Making
  individual notes / tracker cells / keys / clips clickable+queryable via MCP needs a
  per-element `ui.interact(sub_rect, …)` per view — a larger, view-specific effort.
- [ ] **Cables as AccessKit nodes.** Cables are pure paint (no `Response`); v1
  encodes topology on the port labels. A cable-as-node pass (via the `expose_painted`
  escape hatch + AccessKit relations) is optional follow-up.

### 6.5 Sampling & recording backlog
*(from `plans/sampling-plan.md`; feature shipped through v0.262.0. Full P1/P2/P3 backlog + RT-review notes in git history. Related: §2.1.)*

- [ ] **P2 — sample UX/DSP.** Draggable crop/loop handles + preview playback cursor;
  zero-crossing snap (match slope *sign*, not just proximity); loop crossfade
  (static/baked default, keep the original alongside; dynamic dual-read only if loop
  points are modulated); cubic-Hermite interpolation (+ oversample-in-RAM for *short*
  samples only); mini waveform in the Rack; sample-usage tracking; undo/redo for edits.
- [ ] **P3 — stretch.** Sinc resampling, mipmaps for pitch-up anti-alias, disk
  streaming for large files, multi-sample zones (pull the `SampleZone` data model
  earlier to avoid a voice/GUI rewrite), slicing, timestretch, granular
  `GrainSource::Sample`, audio track in the sequencer.

### 6.6 Pertylizer MCP gaps
*(from `plans/pertylizer-mcp-feedback.md`; the live running log continues in the sid-analyzer session memory. Only Pertylizer-side open items harvested.)*

*(MCP convention audit 2026-07-19 — findings on the tools added 17–19 July that aren't
covered elsewhere; mod-grid-specific items live in §2.9.)*

---

## Maybe later

### Graph-level feedback edges (mostly redundant with Script — only build for audio-rate/UX)

- [ ] **Graph-level feedback loops (allow cycles via a one-block delay).** Was
  `plans/graph-feedback-loops.md` (deleted 2026-07-13; full text in git history). The
  proposal: stop rejecting a cycle-closing cable, tag it `is_feedback`, exclude it from the
  topo sort, and read the source's *previous* block (z⁻ᵇˡᵒᶜᵏ). **Largely redundant** — the
  block-latency feedback it wants already exists for the script path: a `Script` (`scr`) or
  `AudioScript` (`asc`) reads any module output as an **address-based source**
  (`src fb = flt-1.out`), which resolves via `Voice::resolve_source` to that module's
  **previous block's** value (`voice.rs:1277`, `buf[0]`) and does **not** go through
  `validate_connection`/`would_create_cycle` (the cycle check only guards `Connection` cable
  edges). So `flt-1.out → osc-1.fm_amt` (the plan's exit-gate example) is expressible today
  as: `scr-1` reads `src fb = flt-1.out`, writes `out1 = fb * amount`, cable
  `scr-1.out1 → osc-1.fm_amt` (a forward edge, no cycle). `AudioScript` additionally gives
  **sample-accurate** in-module feedback via `state` cells — strictly better than one-block
  latency for loops that fit in one module. **Only build the graph-edge feature if we
  actually want** (a) **audio-buffer-rate** feedback wrapped around *existing* modules
  without reimplementing their DSP in script (the address source yields one control-rate
  scalar/block, not an audio buffer), or (b) the **turnkey "drag a back-edge → feedback
  cable" UX** with a visually distinct feedback arc. Otherwise
  document the script recipe as the supported way to do control/CV-rate feedback.
