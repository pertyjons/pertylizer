# MCP Improvements Plan

Tracking doc for the batch of MCP tool improvements requested 2026-05-30 (from
real-world usage feedback during a SID-import / mixing session). Each item has a
status marker so nothing gets lost across sessions.

**Status legend:** ✅ done · 🔨 in progress · ⬜ not started · ♻️ already exists
(no work) · ⏸️ deferred

The original feedback had ~15 distinct asks (two overlapping passes). They are
consolidated and de-duplicated below, grouped by effort.

---

## ✅ Done (v0.304.0, uncommitted)

### insert_module_between  ✅
Splice a module into an existing audio cable in one call (`source → new → dest`).
Anchor model instead of a numeric index (voice graph is a DAG):
`after`/`before` (module id), `after_type`/`before_type` (module type, robust
across instruments), explicit cable, or default = before the output module.
Rejects audio-less modules; restores original cable on wiring failure.
- `synth_mcp`: `types.rs` (`InsertModuleResult`), `bridge.rs` (`InsertAnchor` +
  default trait method + helper fns + 4 unit tests), `server.rs` (param, handler,
  dispatch).

### validate_instrument_audio  ✅
Renders one test note and returns a compact go/no-go verdict (is_audible, peak/RMS,
clipping, fundamental, DC offset, warnings). Wraps the bridge's existing
`analyze_note` (which was not previously exposed as a tool).
- `synth_mcp`: `types.rs` (`ValidateInstrumentAudioResult`), `server.rs`
  (`distill_audio_validation` helper + param + handler + dispatch).

### scale_automation_lane / offset_automation_lane  ✅
In-place value transform on a whole lane (`(v-pivot)*scale+pivot` / `v+offset`,
clamped), tick + curve preserved. Backed by one bridge method
`transform_automation_lane` (in-place, not the lossy get/add round-trip — keeps
curve_strength).

### copy_automation_lane  ✅
Copy a lane to another pattern/target, optional scale/offset, merge or replace.
Bridge method `copy_automation_lane`.

### get_automation_summary  ✅
Project-wide read-only overview, group_by instrument / target / pattern. Default
trait method composing `list_patterns` + `list_automation_lanes`.
- `synth_mcp`: `types.rs` (3 summary structs), `bridge.rs` (default method).

### AWE pre_delay alias  ✅
`set_awe_parameter` accepts `pre_delay_ms` (the `get_awe_state` field name) as an
alias for `pre_delay`. `mcp_bridge.rs` set_awe_parameter + server.rs param doc.

---

## ♻️ Already exists (verify usage, maybe small extension)

### analyze_track_contributions  ♻️ / ⬜ (thin wrapper)
`analyze_section` already supports `include_per_track: true` → per-track RMS/LUFS/
peak + `pre_master_peak` + `rms_share`, soloed & parallelized.
- **Remaining (optional):** expose the same per-track mode on `analyze_mix_bus`
  so a full-song breakdown doesn't require passing an explicit tick range.

### batch_execute continue_on_error  ♻️
Already present as `stop_on_error` (inverse) with per-step `succeeded/failed/
results[]` summary. (`dry_run` + `rollback` are still missing — see Large.)

---

## ⬜ Quick wins (low risk, high value)

All five quick-wins landed in v0.304.0 (see **Done** above): AWE `pre_delay`
alias, `validate_instrument_audio`, `scale_/offset_/copy_automation_lane`,
`get_automation_summary`.

---

## ✅ Medium (self-contained features) — landed in v0.305.0

### auto_gain_stage  ✅
Measures integrated LUFS + true peak through the master chain and sets the master
fader toward `target_lufs` without breaching `true_peak_ceiling`. Single render —
the fader is post-effects so loudness/peak scale linearly; reports measured vs.
predicted and `limited_by`. Adjusts the master fader only (inherently preserves
track balance); per-track gain staging not implemented.
- `mcp_bridge.rs` `auto_gain_stage_impl` + trait method; `types.rs`
  `AutoGainStageResult`.

### create_chord_progression_pattern  ✅
Creates a pattern and fills it with a voiced progression (`chords`,
`beats_per_chord`, `octave`, `voicing`, `velocity`). Default trait method
composing `create_pattern` + `generate_chord` + `add_notes`. Uses
`beats_per_chord` (not bars) to avoid a time-signature dependency.

### analyze_mix_bus master-volume bug + signal_chain reporting  ✅
**Confirmed real bug:** the offline renderer never applied `master_volume`, so
`set_master_volume` had no effect on `analyze_mix_bus` / `analyze_section`. Fixed
in `OfflineEngineSession::new_with_scope` (sends the live master volume to the
offline engine). Regression test `master_volume_scales_offline_render`. Both
results now carry a `signal_chain` string describing exactly what was measured.

---

## ⏸️ Large / trickier (scope carefully, design before coding)

### rebuild_instrument_preserve_automation  ⏸️
Automation is pattern-scoped via target strings (`module:flt:1:cutoff`); a rebuild
changes instance numbers → orphaned lanes. Needs snapshot + remap logic.

### analyze_return_busses / analyze_master_chain  ⏸️
Per-return-bus and per-master-effect breakdown (peak/RMS/LUFS/width, gain
reduction, masking). Today only A/B via scope flags. Needs incremental-chain
rendering (measure before/after each effect).

### batch_execute dry_run + rollback_on_error  ⏸️
`dry_run` = validate params without executing (needs a per-tool validate path).
`rollback` = transaction support (engine has `TransactionId` — investigate).

### compare_mix_before_after  ⏸️
Store a baseline metric snapshot, then show delta (LUFS, true peak, width, masking
top pairs). Needs baseline state held somewhere.

### suggest_patch_changes (applicable)  ⏸️
Concrete per-instrument parameter suggestions toward a goal (e.g. "modern_sid",
reduce_masking). Fuzzy + large; `suggest_music_fixes` partly covers it.
**Recommended: defer** until the rest lands.

---

## Suggested order

1. **Quick wins** (this list, 5 items) — one commit series.
2. **Medium** — auto_gain_stage, chord progression, analyze_mix_bus verification.
3. **Large** — design sketch each (data model, remap, transactions) before coding.
