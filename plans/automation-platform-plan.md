# Automation platform — track/global lanes, pitch, lane types

## Goal

Grow Pertylizer's automation system into a complete, general platform on a
dedicated branch from `main`, independent of the tracker importer:

- track- and global-scoped lanes become first-class: authorable from GUI and
  MCP, and relative to their hosting track by default;
- a track-pitch lane with per-voice application;
- richer lane types (LFO generator segments; optional Bezier curves);
- (stretch) per-voice track faders for shared instruments.

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

## A3. Lane types

Two independent additions; the LFO segment is the load-bearing one.

### A3.1 LFO generator segments

Extend the lane model so a lane holds regions of two kinds: point runs (with
per-point curves, as today) or **generator segments**. An LFO segment spans
`[start, end)` and carries rate (Hz or tempo-synced), depth (in the lane's
units), waveform, a retrig flag, and phase. Engine: evaluate in the same
place lane point values are read today. GUI: draw + edit in the automation
zone. Save format + schema + MCP fields.

**Acceptance checklist = the xmrs feature set** (so the tracker importer can
map 1:1 instead of baking): waveforms sine / triangle / square / ramp /
random; retrig on/off; mid-song rate/depth changes (adjacent segments); depth
expressed in the target lane's units.

### A3.2 Bezier curve type (optional, last)

`CurveType::Bezier { .. }` with per-segment control handles (data model + GUI
handle editing + eval). Pure editing comfort — nothing downstream depends on
it; it must not block the branch.

## A4. Authoring surfaces

1. **GUI**: a Track submenu in the piano-roll lane picker, defaulting to
   "This track" (`None`), with explicit tracks listed for the cross-track
   case; a Global entry; automation-zone editing for the A3 lane types.
2. **MCP**: add `track { param, track_id? }` (omitted `track_id` = host
   track) and `global { param }` variants to `AutomationTargetInput`; accept
   the same variants in the target addressing used by
   `get/remove/clear/scale/offset/copy_automation_*` (today parameter name +
   `instrument_id` only); make `list_automation_lanes` report each lane's
   scope (instrument/module/track/global); expose the A3 segment fields.

## A5. (Stretch) Per-voice track fader — channel-strip Phase 8

With A2's voice tagging in place: apply the composed track volume/pan/mute as
a per-voice gain where velocity/expression already scale the voice, BEFORE
the instrument's shared effect chain, and re-key `track_controls` by
`TrackId`. The dry signal becomes fully track-correct; shared FX still react
to the sum (the same limitation as multitimbral racks in other DAWs), and
full isolation stays available by duplicating the instrument. Landing this
removes the tracker importer's clone-at-import workaround. Full write-up:
`plans/TODO.md` §5.7 (per-track accumulators for pre-FX sends and metering
ride the same infrastructure).

## Ordering

A1 → A2 → A4 can land incrementally; A3.1 is the largest design item and can
proceed in parallel after A1; A3.2 last; A5 any time after A2. Squash-merge
to `main` per project rules; then merge `main` into `feat/tracker-import` and
run the import plan.

## Acceptance criteria

- A Track lane created in the GUI on a pattern follows the placement's track,
  including after moving the placement to another track.
- Two placements of one pattern on different tracks each get the lane applied
  to their own track.
- A `TrackParam::Pitch` lane bends held voices on its own track only — a
  second track sharing the instrument is unaffected (pitch always; fader too
  if A5 landed).
- An LFO segment on a track-volume lane renders tremolo without baked points
  and survives save/load and MCP round-trips.
- Track and Global lanes are creatable, editable, and enumerable from both
  GUI and MCP.
