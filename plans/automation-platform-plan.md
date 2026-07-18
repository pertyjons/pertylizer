# Automation platform — track/global lanes, pitch, lane types

## Goal

Grow Pertylizer's automation system into a complete, general platform on a
dedicated branch from `main`, independent of the tracker importer:

- track- and global-scoped lanes become first-class: authorable from GUI and
  MCP, and relative to their hosting track by default;
- a track-pitch lane with per-voice application;
- curve-type editing and a stacked all-lanes view in the automation zone.

(Two ideas that started here have moved: live LFO modulation of lanes became the
**Mod Grid**, `plans/mod-grid-plan.md`; the per-voice track fader became
`plans/TODO.md` Tier B6. See A3 / A5 below.)

The tracker importer (`plans/tracker-import-daw-mapping-plan.md`) is the first
consumer and shrinks to a pure translator once this lands — but every feature
here stands on its own for ordinary song-making.

## Verified current state

- `AutomationTarget` = `Instrument` / `Track { track: TrackId, param }` /
  `Global(GlobalParam)` / `Module` (`synth_sequencer/src/automation.rs`).
  `TrackParam` = Volume/Pan/Mute; `GlobalParam` = MasterVolume.
- Lanes are pattern-local (`Pattern.automation`) and point-based:
  `AutomationPoint { tick, value: NormalizedValue, curve }` with `CurveType` =
  `Linear | Step | Exponential(i8) | SCurve`. **Step already exists.** There
  is no generator (LFO) segment and no Bezier.
- Track lanes are playback-only: the piano-roll lane picker offers only
  Instrument/Module submenus, and MCP `AutomationTargetInput`
  (`synth_mcp/src/server.rs:2719`) has only `Module`/`Instrument` variants.
- Track automation is applied per engine instrument, not per track:
  `update_track_controls` (`synth_engine.rs:2945`) composes `track_auto` per
  `TrackId` but writes `track_controls` keyed by the track's engine instrument
  id — shared instrument ⇒ the last track in `tracks()` order wins fader and
  sends (documented in the code, which names "channel-strip plan, Phase 8" as
  the fix).
- `SequencerEvent::NoteOn` carries no `TrackId`; voices don't know their
  track.
- Voice pitch is already continuously drivable per block via
  `set_voice_pitch` / `VoicePitch` (`voice.rs:986`).

## A1. Relative track lanes

Change `AutomationTarget::Track` to `{ track: Option<TrackId>, param }`.
`None` = "the track hosting the placement", resolved during the sequencer's
placement walk (the placement is in scope where lane values are collected,
`sequencer_engine.rs:862`); push the **resolved** concrete target into
`scratch_automation` so dedup (`last_automation_values`) and `track_auto`
stay unchanged downstream. `Some(id)` = deliberate cross-track automation
(e.g. song-spanning fades hosted on a dedicated automation track); it may
stay GUI-unexposed initially. Host-track lanes are inert in the
placement-less preview path, which is harmless. Update `display_name`
("This track" for `None`) and serialization.

Why: a pooled pattern should not hard-code which track it automates — an
unplaced pattern has no track yet, a placement moved to another track would
keep automating the old one, and two identical tracks could never share a
pattern once a Track lane exists. Relative lanes also unlock cross-track
pattern dedup (one pattern, several placements on different tracks).

## A2. `TrackParam::Pitch` + voice→track tagging

1. Add `TrackParam::Pitch` with a documented bipolar `NormalizedValue`
   encoding (0.5 = 0 semitones over a fixed ± range; pick the range so large
   tracker portamento spans fit — at least ±48 st).
2. Carry the pitch value through `TrackAutoOverride` / `track_auto` alongside
   volume/pan/mute.
3. Extend `SequencerEvent::NoteOn` with the source `TrackId` (from
   `placement.track_id`) and store it on `Voice`.
4. Apply the track's current pitch offset per block to the voices tagged with
   that track, through the existing `set_voice_pitch` / `VoicePitch.expr`
   path — no stored instrument parameters are touched. A second track sharing
   the instrument is unaffected (pitch is per-voice by construction).

## A3. Lane types — DROPPED (folded into the Mod Grid)

The lane model keeps its existing point-with-curve types (Linear / Step /
Exponential / SCurve — all already implemented). The two richer additions that
used to live here are removed from this plan:

- **LFO generator segments** — rather than baking an LFO into lane *data*, a
  live control-rate LFO drives the automation target space directly. That is
  the **Mod Grid** (`plans/mod-grid-plan.md`): pooled control-rate modulator
  graphs whose outputs compose additively onto lanes, covering the whole xmrs
  waveform/retrig feature set as live signal. The tracker importer maps tracker
  LFOs onto Mod Grid graphs, not baked segments.
- **Bezier curve type** — pure editing comfort with nothing downstream
  depending on it; dropped for now, revisit only if point editing needs it.

## A4. Authoring surfaces

1. **GUI**: a Track submenu in the piano-roll lane picker, defaulting to
   "This track" (`None`), with explicit tracks listed for the cross-track
   case; a Global entry.
2. **Curve-type editing in the automation zone.** A selector for which point
   type is being edited — Linear / Step / Exponential (with strength) /
   SCurve — both as the tool default applied to newly created points and as a
   per-point change on existing (selected) points, e.g. via the point's context
   menu. The lane already renders each segment with its true shape
   (`gui/sequencer/automation.rs` samples `curve.interpolate` per pixel), so
   drawn output stays correct; add per-point type **indication** on top — a
   hover tooltip naming the type and/or a distinct handle glyph per type — so
   the type is readable at a glance in the lane.
3. **Stacked lane display — show ALL lanes.** Today the piano roll shows
   exactly two zones: `VEL` (velocity bars) and ONE automation zone drawn
   only for the currently selected lane (`selected_automation`; "None" hides
   it — `piano_roll.rs:2156`, `draw_automation_zone`). Replace with: every
   lane of the pattern gets its own zone, stacked vertically below the
   velocity zone. The picker dropdown marks which lane is selected; clicking
   a zone selects it, and every zone is directly editable. Each zone draws a
   dimmed caption under its curve naming the target — e.g.
   "Instrument – Attack", "Module – Oscillator 1 – Rate", "Track – Volume
   (this track)" — a human-readable long form of `display_name`. Where a
   color is cheaply available, tint the curve with its owner's color (module
   / track / instrument color); otherwise keep the current accent. Manage
   height: the zones join the existing scrollable content height
   (`total_content_height`), with a per-lane collapse and/or a cap + scroll
   once a pattern carries many lanes.
4. **MCP**: add `track { param, track_id? }` (omitted `track_id` = host
   track) and `global { param }` variants to `AutomationTargetInput`; accept
   the same variants in the target addressing used by
   `get/remove/clear/scale/offset/copy_automation_*` (today parameter name +
   `instrument_id` only); make `list_automation_lanes` report each lane's
   scope (instrument/module/track/global). Point-writing tools already accept
   `curve`/`curve_strength` per point.

## A5. Per-voice track fader — MOVED to `plans/TODO.md` (Tier B, B6)

The per-voice track fader (channel-strip "Phase 8") is a stretch/later item
that only *depends* on A2's voice tagging — it doesn't block this branch. It
now lives in `plans/TODO.md` §5 Tier B as **B6**, to be picked up when a
shared-instrument project actually needs independent track faders.

## A6. GUI follow-ups (deferred polish, noticed during implementation) — DONE

Small, non-blocking items surfaced by the code review of the A4 GUI work.
**Implemented in commit `617f4bd6`:**

1. ✅ **Share the velocity-set logic with the drag path.** The velocity-zone
   click and drag paths now share `velocity_zone_rect` /
   `velocity_from_pos_y` / `nearest_velocity_note` helpers in `piano_roll.rs`,
   removing the duplicated nearest-note + clamp code. The drag path also now
   picks the nearest note by start-tick (where the bar is drawn), matching the
   click path and bar rendering.
2. ✅ **Re-pick the auto-selected instrument when a placement changes, not only
   on pattern open.** `draw_pattern_instrument_transport` now keys on
   `(pattern, first-routed-instrument)` (`last_auto_instrument`) and re-fires
   when either changes, so placing/moving the pattern onto a track after it is
   open refreshes the pick. `arrangement()` is an ordered `Vec`, so the key is
   stable frame-to-frame and a manual pick still sticks while the placement is
   unchanged.
3. ✅ **Allow dragging points in any stacked lane, not just the focused one.**
   `DragAutomationPoint` now carries the band's `zone_y`, captured at drag
   start after resolving which stacked zone the press landed in — so the value
   math stays correct regardless of focus, and dragging a point in any lane
   also focuses it.

## Ordering

A1 → A2 → A4 land incrementally (all done); A6 (loose polish) is done. A3 is
dropped (folded into the Mod Grid) and A5 is moved to `plans/TODO.md` B6.
Squash-merge to `main` per project rules; then merge `main` into
`feat/tracker-import` and run the import plan.

## Acceptance criteria

- A Track lane created in the GUI on a pattern follows the placement's track,
  including after moving the placement to another track.
- Two placements of one pattern on different tracks each get the lane applied
  to their own track.
- A `TrackParam::Pitch` lane bends held voices on its own track only — a
  second track sharing the instrument is unaffected (pitch always; fader too
  once B6 lands).
- Track and Global lanes are creatable, editable, and enumerable from both
  GUI and MCP.
- The curve type of any automation point can be chosen for new points and
  changed on existing points in the GUI, and is visually identifiable in the
  lane.
- All of a pattern's lanes are visible at once as stacked zones, each with a
  dimmed caption naming its target; the selected lane is marked in the picker
  dropdown.
