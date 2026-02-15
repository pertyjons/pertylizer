# AWE Implementation Review

## Status Overview
### Resolved
- None confirmed resolved since the last review (no code changes detected in this update round).

### New
- High: AWE LFO modulation is additive (drifts) rather than oscillating around a base value; each block adds the LFO output to the snapshot, which can walk parameters to extremes and forces geometry recalculation every block. (crates/synth_awe/src/awe_engine.rs:615)

### Still Open
- High: RT-safety broken by FDN resizing on the audio thread; `FdnCore::set_delay_times` calls `FdnChannel::resize` which can allocate, and this is triggered from `AweEngine::recalculate_geometry` inside the process loop. (crates/synth_dsp/src/fdn.rs:82, crates/synth_dsp/src/fdn.rs:275, crates/synth_awe/src/awe_engine.rs:683)
- High: Instrument visualizers appear broken; `EffectChain::process` skips visualizers and only the master chain runs `process_visualizers`, so per-instrument visualizers no longer receive data. (crates/synth_engine/src/effect_chain.rs:272, crates/synth_engine/src/instrument.rs:1081)
- Medium: AWE persistence is incomplete; save only writes enabled/spatial/note_mapping with defaults for everything else, and load applies snapshot + flags but not room/material. Settings are lost across save/load. (crates/modular_synth/src/gui/patch_bridge.rs:129, crates/modular_synth/src/gui/patch_bridge.rs:1032, crates/synth_awe/src/params.rs:191)
- Medium: Early reflections and room modes max delays are far smaller than the plan’s large-room goals; big rooms/pipelines will clamp reflections and modes. (crates/synth_awe/src/early_reflections.rs:14, crates/synth_awe/src/room_modes.rs:14)
- Medium: LFO modulation of RoomLength/RoomWidth forces `RoomShape::Box`, silently replacing Cylinder/L-Shape when those targets are used. (crates/synth_awe/src/awe_engine.rs:615)
- Medium: Sample-rate changes do not mark AWE geometry dirty; `on_stream_start` updates SR but AWE geometry/taps won’t be recomputed until another param change. (crates/synth_engine/src/synth_engine.rs:1745, crates/synth_awe/src/awe_engine.rs:683)
- Low: Per-voice spatial capture truncates blocks larger than 4096 samples (VOICE_BUFFER_SIZE), which can cause inconsistent spatial behavior with large callbacks. (crates/synth_awe/src/spatial_voice.rs:16)

## Questions / Assumptions
- Should per-instrument visualizers continue to work, or is only master visualization intended?
- Is full AWE state round-trip required (room/material/snapshot), or is partial persistence acceptable?
- Are large-room/pipeline presets meant to be accurate in v1, or is clamping acceptable?

## Testing Gaps / Risks
- No tests verify AWE save/load round-trip.
- No tests ensure instrument visualizers still update.
- No RT-safety tests for audio-thread allocations in AWE/FDN.

## Summary
AWE is integrated and feature-rich, but RT-safety is still violated via FDN resizing and LFO drift causes parameter runaway. Per-instrument visualizers appear broken, persistence is incomplete, and large-room goals are not met by current delay caps. Fix these before expanding features.
