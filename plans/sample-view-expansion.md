# Sample Workstation View

**Status:** Draft implementation plan

**Date:** 2026-08-05

**Scope:** Sample library, waveform editor, audition, recording, analysis,
stereo playback, and multi-sample instruments

## 1. Objective

Turn the existing Sample view into a first-class workstation that feels like the
other Pertylizer views and supports the complete lifecycle of sampled audio:

1. Import or record audio.
2. Inspect and audition it immediately.
3. Select regions and edit them safely.
4. Set crop, loop, root-note, and playback metadata visually.
5. Analyze pitch, spectrum, dynamics, transients, and stereo properties.
6. Use mono or stereo assets without losing channel information.
7. Build playable multi-octave instruments from multiple sample zones.
8. Save, reload, undo, and recover every operation reliably.

The plan treats **waveform rendering** and **audio rendering** as separate quality
problems. Improving only the painted waveform would leave the sampler's playback
quality and stereo behaviour unchanged.

## 2. Current Baseline and Confirmed Gaps

The current implementation already provides a useful foundation:

- A project-owned `SampleLibrary` with immutable `Arc<[f32]>` audio buffers.
- WAV import/export and bundle persistence.
- Mono and stereo metadata plus stereo audio-input capture.
- Root note, crop, loop-region, and loop-crossfade metadata.
- Normalize, reverse, and automatic silence trim commands.
- Input-device selection, monitoring, recording, and an input peak meter.
- A pitch-tracking sampler module with one-shot, sustain, loop, reverse, and
  ping-pong modes.
- Existing offline spectrum, spectrogram, pitch, transient, loudness, and stereo
  analysis primitives used by the MCP analysis tools.

The gaps that drive this plan are:

- The Sample view mixes all channels to mono for waveform display and has no
  channel-separated stereo lanes.
- The waveform uses one simple zoom-dependent peak cache. It has no time ruler,
  sample-level line rendering, overview strip, selection, playhead, draggable
  crop/loop handles, or pointer-centred zoom.
- The view has no audition/play action despite being described as a preview view.
- The sampler internally reads stereo but currently collapses it to a mono `out`
  port, so stereo import and recording do not survive patch playback end to end.
- `LoadSampleData` does not carry the asset sample rate, crop region, loop region,
  or loop crossfade into the sampler. Stored metadata therefore cannot reliably
  control live/offline playback.
- Import resampling and pitched playback both use linear interpolation. This is
  inexpensive but audibly weak for large pitch shifts and does not sufficiently
  suppress pitch-up aliasing.
- Loop crossfade is stored and editable but is not applied by `SamplePlayer`.
- Recording is accumulated when the GUI drains its ring buffer. A sufficiently
  long UI stall can overflow that buffer and lose captured frames.
- Edits are applied directly without sample undo/redo, bounded edit history, or
  confirmation for irreversible operations.
- Existing sample analysis is surfaced through the MCP bridge, not as a reusable
  application service and Sample-view panel.
- A single sample can be pitched from its root note, but there is no multi-sample
  keymap, velocity layer, round robin, or automatic multi-octave mapping.
- Files with more than two channels have ambiguous playback semantics. The player
  clamps its channel count while retaining the original interleaved layout; import
  must explicitly reject or convert such files instead.

## 3. Product Decisions

These decisions should be made explicit before implementation so the UI, engine,
and project format do not evolve in different directions.

### 3.1 Samples and sample maps are different concepts

- A **sample asset** is one mono or stereo recording with metadata and immutable
  audio data.
- A **sample map** is a playable instrument definition containing zones that
  reference sample assets.
- Editing an asset updates every map and sampler that references its stable
  `SampleId`; it must not create hidden copies.
- Add domain newtypes such as `SampleMapId`, `SampleZoneId`, and any missing range
  types. Do not represent domain identifiers or ranges with raw primitives.

### 3.2 Preserve native audio and convert at playback

- Preserve the imported/recorded asset's native sample rate and channel layout.
- Pass the source sample rate into every live and offline player.
- Playback speed is `source_rate / engine_rate * pitch_ratio`; changing the audio
  device or offline render rate must not alter pitch or duration.
- Support mono and stereo assets as the required contract. Reject files with more
  than two channels with a clear error until an explicit channel-mapping dialog
  exists.

### 3.3 Editing model

- Crop, loop, root note, markers, and mapping are non-destructive metadata edits.
- DSP commands produce a new immutable audio buffer and retain the prior `Arc` for
  undo. Do not mutate a buffer shared with the audio thread.
- Long operations run on a worker from a source snapshot and apply only if the
  sample revision still matches. A stale result must never overwrite newer work.
- The project-wide undo manager owns edit transactions; the Sample view does not
  create an isolated undo universe.
- Bound audio history by retained bytes as well as action count. Warn before an
  operation that cannot fit the configured undo budget.

### 3.4 UI consistency

- Reuse `list_panel`, `toolbar::top`, themed controls from `widgets/controls.rs`,
  Remix icons, status/error presentation, and the normal panel ordering.
- Keep the waveform editor as a view-specific composite. Only small reusable
  controls belong in `widgets/controls.rs`.
- Use the same computer-keyboard/on-screen-keyboard audition conventions as the
  Rack and piano roll, routed through the global focus-safe shortcut dispatcher.

## 4. Target User Experience

```text
┌ Sample library ┐ ┌ Toolbar: tools | undo | audition | analyze | record ┐
│ Search/filter  │ ├──────────────────────────────────────────────────────┤
│ Samples        │ │ Time ruler + overview/minimap                       │
│ Sample maps    │ │ L waveform / mono waveform                          │
│ Import/record  │ │ R waveform (stereo only)                            │
│ Usage/status   │ │ Selection, markers, crop/loop handles, playhead     │
└────────────────┘ ├──────────────────────────────────────────────────────┤
                   │ Properties | Edit | Analyze | Mapping | Recording   │
                   ├──────────────────────────────────────────────────────┤
                   │ Optional audition keyboard / multi-sample key zones │
                   └──────────────────────────────────────────────────────┘
```

### Required interactions

- Single click places the audition cursor; drag creates or adjusts a selection.
- Shift-click extends the selection; Escape clears it.
- Mouse wheel scrolls horizontally and the command modifier zooms around the
  pointer. `Fit`, `Selection`, `1:1`, and zoom history are available in the toolbar.
- Crop and loop markers have labelled, draggable handles with zero-crossing snap.
- Space starts/stops audition when the waveform owns focus; global song transport
  remains available through the central shortcut policy without collisions.
- The playhead is engine-driven and remains accurate while the UI frame rate varies.
- Edits operate on the selected range when present and on the audible crop/full
  sample only when the command explicitly says so.
- Commands are available through toolbar groups and a waveform context menu, with
  disabled-state explanations and visible shortcuts.
- The lower inspector is collapsible/resizable so the waveform remains useful on
  smaller windows.

## 5. Architecture

### 5.1 Sample document and snapshots

Extend the library API instead of allowing GUI code to edit `Sample` fields
directly:

- Give each asset its own content revision in addition to the library revision.
- Add an owned `SampleSnapshot` containing ID, revision, metadata, and cloned
  audio `Arc` for background work.
- Add validated library operations for replacing data, metadata, and complete
  sample state. They return whether a semantic change occurred.
- Add invariant checks for interleaved length, frame count, channel count, crop,
  loop bounds, crossfade length, root note, and zone references.
- Publish one change event after a committed operation so dirty-state, waveform
  caches, analysis caches, sampler reloads, and project autosave all observe the
  same mutation.

### 5.2 Playback specification

Replace the partial `LoadSampleData` payload with a complete immutable playback
specification containing:

- Audio buffer and stable `SampleId`/revision.
- Source sample rate and channel count.
- Frame count and validated audible crop.
- Optional loop region and crossfade.
- Root note and any asset-level tuning offset.

The GUI, project hydration, live engine, offline renderers, preview player, and MCP
preview must all build this specification through one shared function.

When an asset changes, reload every live sampler referencing it at a safe command
boundary. Existing voices may finish on the old `Arc`; new notes must use the new
revision. This avoids mutating data underneath the audio thread.

### 5.3 Edit command service

Define a UI-independent `SampleEditCommand` layer used by GUI actions and future
MCP tools. Initial commands:

- Set/clear crop and loop; set crossfade and root note.
- Crop/bake selection, cut, copy, paste, delete, and insert silence.
- Normalize peak to a chosen `Decibels` target; apply gain.
- Reverse; fade in; fade out; crossfade selection.
- Remove DC offset; trim silence with configurable threshold and padding.
- Snap selection/crop/loop edges to appropriate zero crossings.
- Stereo operations: swap L/R, invert a channel, extract L or R, downmix with an
  explicit law, and convert dual mono to stereo.
- Resample with an explicit target `DeviceSampleRate` for export/baking only.

Each command validates its frame range, runs outside the library write lock,
returns an old/new transaction for undo, and reports a structured error rather
than silently doing nothing.

### 5.4 Background jobs

Use a small application-owned worker queue for peak generation, analysis, and
offline edits:

- Jobs receive immutable snapshots and cancellation tokens.
- Results include the source sample ID and revision.
- Only the newest matching result is installed.
- Progress and cancellation appear in the Sample toolbar/status area.
- No FFT, full-buffer scan, allocation, file I/O, or lock acquisition is performed
  on the audio thread.

## 6. Implementation Phases

### Phase 1 — Playback correctness and real stereo

- [ ] Carry source rate, crop, loop, and crossfade through the shared playback
  specification and every live/offline hydration path.
- [ ] Correct playback-rate math for source rate versus engine rate.
- [ ] Give the sampler `out_l` and `out_r` ports and preserve channel separation
  through the voice graph, instrument output, effects, preview, and export.
- [ ] Duplicate mono to L/R without applying unintended pan or level changes.
- [ ] Implement loop crossfade for forward and ping-pong playback, with crossfade
  clamped to a valid fraction of the loop.
- [ ] Replace linear pitch interpolation with a measured higher-quality strategy.
  A staged implementation is acceptable: cubic Hermite first, followed by a
  band-limited/polyphase or mipmapped path for pitch-up alias suppression.
- [ ] Preallocate all player scratch state from the configured maximum block size;
  no grow-on-demand allocation may occur in `process()`.
- [ ] Reject or explicitly downmix imports with more than two channels.

**Acceptance:** A stereo impulse remains isolated to its original channel; a
44.1-kHz asset plays at the same pitch/duration in 44.1, 48, and 96-kHz engines;
crop and loop metadata sound the same live and offline; multi-octave pitch tests
meet defined alias-energy and pitch-error thresholds.

### Phase 2 — Waveform canvas and visual quality

- [ ] Replace the single peak cache with a per-channel, multi-resolution min/max
  pyramid keyed by sample ID and revision.
- [ ] Build/cache peaks asynchronously and render only the visible level/range.
- [ ] Draw separate L/R lanes for stereo, one lane for mono, per-lane zero lines,
  clipping indicators, a time/sample ruler, and an overview strip.
- [ ] Switch to a sample/polyline rendering mode at high zoom so transients and
  zero crossings are inspectable instead of drawing one min/max bar forever.
- [ ] Add stable frame↔screen transforms shared by waveform, ruler, selection,
  handles, markers, playhead, and hit testing.
- [ ] Add pointer-centred zoom, bounded pan, fit-all, fit-selection, 1:1 zoom,
  overview navigation, and resize-safe cache selection.
- [ ] Add selection, cursor, crop/loop handles, marker labels, edge auto-scroll,
  zero-crossing snap, and clear hover/drag affordances.
- [ ] Expose meaningful AccessKit nodes/actions for handles, markers, selection,
  playhead status, and the waveform toolbar rather than only one canvas node.

**Acceptance:** Long stereo samples do not stall normal UI interaction; resize and
zoom never desynchronize overlays from audio; left/right polarity differences are
visible; exact sample values and zero crossings can be inspected at high zoom.

### Phase 3 — Audition transport

- [ ] Add a dedicated engine-owned sample preview voice with Play/Pause/Stop,
  restart, loop, selection-only audition, and click-to-start.
- [ ] Route preview through the normal output device and master safety limiting;
  do not open a second audio stream.
- [ ] Publish playhead frame and playback state through atomics/events for the UI.
- [ ] Add audition gain, mono-check, L-only/R-only, and loop-follow-editor options.
- [ ] Stop or safely replace audition when selection, sample, project, or audio
  device changes.
- [ ] Support MIDI/computer-keyboard audition from the asset root note and show an
  on-screen keyboard using the same interaction rules as other views.
- [ ] Prevent focused text fields, modal dialogs, and global command shortcuts from
  triggering preview notes.

**Acceptance:** Audition starts promptly, stops without tails or stuck state,
tracks the visible playhead, respects crop/loop/selection, preserves stereo, and
does not disturb song transport or active instrument notes.

### Phase 4 — Safe editing and undo/redo

- [ ] Introduce the `SampleEditCommand` service and route all existing Normalize,
  Reverse, Auto-trim, metadata, rename, import, and delete operations through it.
- [ ] Add the initial edit commands listed in §5.3 with selection-aware labels.
- [ ] Integrate sample actions with project-wide undo/redo and dirty-state
  baselines from TODO §0.1/§0.3.
- [ ] Coalesce handle drags and continuous gain changes into one transaction.
- [ ] Add a bounded audio-history budget and visible warning/confirmation when an
  edit cannot be made undoable.
- [ ] Update crop/loop/marker positions correctly after destructive changes such
  as reverse, delete, paste, and bake crop.
- [ ] Refresh every referencing sampler after commit, undo, and redo.
- [ ] Add explicit `Apply`/`Cancel` previews for expensive or parameterized DSP
  commands; never hold a library lock while the preview is calculated.

**Acceptance:** Every Sample-view mutation marks the project dirty, can be undone
and redone unless explicitly confirmed otherwise, survives project save/reload,
and produces identical live and offline sampler state.

### Phase 5 — Recording workstation

- [ ] Move durable capture away from GUI-frame draining. The audio callback may
  write only to a bounded lock-free queue; a dedicated capture worker drains it
  into chunked memory or a temporary file outside the audio thread.
- [ ] Detect and report queue overruns/dropouts with the affected frame count.
- [ ] Add input channel selection: mono L, mono R, or stereo pair. Persist the
  user's device/channel choice in application settings, not project song data.
- [ ] Draw independent L/R meters with peak hold, clipping latch, and reset.
- [ ] Show a live decimated waveform, elapsed time, recorded frames, sample rate,
  channel mode, and available recording capacity.
- [ ] Add optional count-in, threshold-triggered start, pre-roll capture, and
  stop-after-silence. Keep these disabled by default.
- [ ] Stop recording into a take-review state with Audition, Rename, Trim,
  Normalize, Save, Retake, and Discard; do not immediately commit an unnamed asset.
- [ ] Recover finalized temporary takes after a crash and clean discarded/expired
  capture files safely.

**Acceptance:** Recording remains gap-free during deliberate multi-second UI
stalls, mono/stereo channel choices are correct, monitoring does not feed back by
default, and a take is not added to the project until the user accepts it.

### Phase 6 — Sample analysis

- [ ] Extract a reusable sample-analysis service from the MCP bridge so GUI and
  MCP call the same implementation without the GUI depending on MCP types.
- [ ] Analyze either the selection, audible crop, or full asset at native rate.
- [ ] Provide a quick summary: sample/sample-true peak, RMS, LUFS where valid,
  crest factor, clipping count, DC offset, silence bounds, and duration.
- [ ] Provide pitch analysis: detected fundamental, confidence, nearest MIDI note,
  cents error, and a non-destructive “Set root note/tuning” suggestion.
- [ ] Provide time/frequency views: spectrum, spectrogram, energy bands, centroid,
  transient/onset markers, and amplitude envelope.
- [ ] Preserve stereo information in analysis: per-channel peak/RMS/DC, channel
  balance, correlation, mid/side energy, width, and mono-compatibility warnings.
- [ ] Cache results by sample revision plus selected range/options; run in the
  background with progress/cancel and reject stale results.
- [ ] Convert analysis findings into opt-in actions such as Trim silence, Add
  slices at transients, Set root note, and Snap loop. Analysis must never mutate
  the sample automatically.

**Acceptance:** GUI and MCP report equivalent results for the same sample/window;
stereo anti-phase and one-sided audio are diagnosed correctly; changing the sample
invalidates stale results; analysis never blocks playback or UI rendering.

### Phase 7 — Multi-octave sample maps

- [ ] Add project-owned `SampleMap` and `SampleZone` models with stable newtype
  IDs. Each zone references a `SampleId` and contains root note, inclusive key
  range, velocity range, gain, pan, fine tuning, optional round-robin group, and
  deterministic selection priority.
- [ ] Validate that zones reference existing assets and use ordered ranges. Deleting
  an in-use asset must remain blocked or offer an explicit reference-repair flow.
- [ ] Add a Mapping inspector with a piano keyboard, draggable horizontal key
  zones, velocity-layer lanes, root markers, audition, duplicate, and overlap
  diagnostics.
- [ ] Support multi-file drag/drop and auto-map roots parsed from filenames
  (`C3`, `F#4`, MIDI numbers), embedded WAV metadata when available, or detected
  pitch with confidence. Require review when detection is ambiguous.
- [ ] Provide “spread roots” mapping that derives non-overlapping key boundaries
  halfway between adjacent roots and covers the requested octave range.
- [ ] Resolve a note RT-safely from a precompiled immutable zone table. Preload all
  referenced `Arc` buffers outside the audio thread; zone selection may not lock,
  allocate, hash dynamically, or scan an unbounded collection in `process()`.
- [ ] Extend the sampler selector to choose either one asset or a sample map, and
  hydrate live/offline engines through the same compiled playback specification.
- [ ] Preserve per-zone stereo and source sample rates. Crossfade between adjacent
  key/velocity zones is optional after deterministic hard switching is correct.

**Acceptance:** A map spanning at least five octaves selects the expected source
and pitch ratio at every MIDI note, velocity layers and round robins are
deterministic, no zone lookup allocates on the audio thread, and bundled project
round-trips retain every mapping and referenced asset.

### Phase 8 — Persistence, import/export, and integration polish

- [ ] Persist new metadata, markers, maps, zones, and accepted recorded takes in
  plain/bundled projects; include them in autosave/recovery snapshots.
- [ ] Keep bundle sample storage lossless and channel-correct. Avoid rewriting
  unchanged large audio assets when an atomic/autosave implementation can reuse
  them safely.
- [ ] Export full asset, audible crop, or selection at 16-bit, 24-bit, or float;
  add dither for integer reduction and preserve channel count/sample rate choices.
- [ ] Import multiple WAV files in one operation with per-file error reporting.
  Read standard root/loop metadata where the decoder exposes it.
- [ ] Add file drag/drop, reveal source, duplicate asset/map, usage navigation,
  missing-reference diagnostics, and replacement/relink workflows.
- [ ] Put user-facing sample commands behind shared application actions so menus,
  shortcuts, GUI, and future MCP edit tools use identical validation.
- [ ] Add contextual help and a compact shortcut sheet matching the other views.
- [ ] Ensure responsive layout at the minimum window size and test keyboard-only
  navigation, screen-reader labels, high-DPI scaling, and long localized values.

AIFF/FLAC import, SFZ/SF2 import, disk-streamed playback, slicing directly into an
audio timeline, time-stretch/warp, granular resynthesis, spectral repair, and pitch
correction are useful follow-ups but are not required to complete this plan.

## 7. Test Strategy

### Unit tests

- Frame/screen coordinate round-trips at every zoom level and viewport edge.
- Peak-pyramid min/max preservation per channel and invalidation by revision.
- Every edit command on mono/stereo buffers, empty selections, one-frame ranges,
  and boundary-adjacent crop/loop regions.
- Undo/redo state, metadata relocation, and retained-byte history limits.
- Source-rate conversion, pitch ratio, interpolation, reverse, loop wrapping,
  ping-pong, crossfade, and release behaviour.
- Sample-map key/velocity boundary selection, overlaps, gaps, root ratios, and
  round-robin determinism.
- Analysis cache keys, cancellation, stale-result rejection, and channel metrics.

### Integration tests

- Import → edit → undo/redo → save bundle → reload → audition.
- Record mono L, mono R, and stereo with the null/test backend, including a forced
  GUI stall and overrun reporting.
- Modify an asset already referenced by several live sampler modules and verify
  that new notes, offline preview, arrangement export, and project reload agree.
- Play a known stereo file through a sampler patch and prove channel isolation at
  the master output.
- Play known tones over several octaves at multiple engine rates and measure pitch
  error, duration, alias energy, and discontinuities at loops/zones.
- Compare GUI sample-analysis results with the existing MCP sample-analysis path.

### Manual acceptance matrix

- Small/large files; mono/stereo; 44.1/48/96 kHz; short percussion and sustained
  loops; quiet, clipped, DC-offset, anti-phase, and one-sided material.
- Mouse, keyboard, text-field focus, modal dialogs, and global transport.
- Minimum window size, high DPI, theme variants, project close/reopen, crash
  recovery, audio-device restart, and selection changes during background work.

## 8. Delivery Order and Gates

The phases should land in this order:

1. Playback specification, sample-rate correctness, and stereo engine path.
2. Multi-resolution waveform canvas and selection model.
3. Audition transport.
4. Edit command service plus undo/dirty integration.
5. Robust recording and take review.
6. Shared analysis service and Analyze inspector.
7. Multi-sample maps and multi-octave playback.
8. Import/export, persistence, accessibility, and workflow polish.

Do not start multi-sample mapping before a single stereo asset plays correctly at
all engine rates. Do not expose destructive editing before undo and dirty-state
integration is operational. Do not call the view complete while recorded or edited
audio can differ between audition, live sampler playback, offline rendering, and a
reloaded project.

## 9. Definition of Done

The expanded Sample view is complete when a user can:

- Import or record mono/stereo audio without channel or timing loss.
- See an accurate, responsive, channel-separated waveform.
- Select, audition, loop, edit, analyze, undo, redo, save, and recover the asset.
- Hear the same crop, loop, pitch, stereo image, and edit state everywhere.
- Build and play a persisted multi-octave sample map from multiple recordings.
- Perform the entire primary workflow without unexplained disabled controls,
  conflicting shortcuts, UI stalls, stuck playback, or silent data loss.

All workspace formatting, build, clippy, and test gates must pass with zero
warnings. Audio-thread processing must remain lock-free, allocation-free,
panic-free, and free of file I/O or logging.
