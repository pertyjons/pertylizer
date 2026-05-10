# Analyze view plan

A dedicated GUI surface for the existing `analyze_note` MCP/audio pipeline,
so users can iterate on a patch and see what changed without going through
an LLM client. Reads from the same `AnalyzeNoteResult` returned by
`analyze_rendered_note` in `crates/pertylizer/src/mcp_bridge.rs`.

## Goals

- One-glance status: is this patch silent / clipping / DC-leaking / off-pitch
  / low-output? Visible without any drilling.
- Iteration speed: pin a "before" analysis, tweak the patch, hit re-analyze,
  see exactly what changed in numbers and plots side-by-side.
- All MCP-tool data on one surface: waveform, RMS envelope, spectrum
  snapshots, pitch track, stereo decomposition, harmonic structure, ADSR
  estimate, warnings.
- Fits on 1080p without scrolling. 4K should look spacious, not sparse.

Non-goals (for v1):

- Real-time meters during normal playback. The view is offline-render-driven.
- Editing patches from inside the analyze window. Just analysis.
- More than two pinned analyses at once. A vs B is enough.

## Surface choice

**Floating `egui::Window`** (resizable, movable). One window per session,
re-targeted when the user analyzes a different instrument — see *Open
questions* for the rationale. Triggered from:

- A toolbar button on the patch editor (Remix Icon `ri::SEARCH_EYE_LINE`
  or similar — never an emoji glyph; the project's UI uses
  `egui_remixicon::icons as ri` consistently)
- An instrument-list context menu ("Analyze instrument…")
- Keyboard shortcut. Avoid Ctrl-A (collides with "select all" in text
  fields and the piano-roll). Suggested: Ctrl-Shift-A, bound via the
  existing shortcut layer.

Rejected: modal dialog (blocks editing — analysis is iterative), bottom
panel (cramped at 1080p, harder to A/B with the patch graph visible).

## Layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Analyze: PWM E-Piano             Note [C4▾]  Vel [100] Dur [1500ms] Tail [1000] │
│                                                              [▶ Re-analyze] [📌 Pin] │
├──────────────────────────────────────────────────────────────────────────────┤
│  ●PASS │ peak 0.098 │ rms 0.014 │ DC -0.001 │ pitch -0.5¢ (conf 94%) │ st-cor 0.78 │
├───────────────────────────────┬──────────────────────────────────────────────┤
│ WAVEFORM (L / R)              │ SPECTRUM      attack━ sustain━ release━    │
│   stereo strip with note-off  │   overlaid attack/sustain/release peaks     │
│   marker; toggle L/R/sum      │   log-frequency x-axis                       │
│ ENVELOPE                      │ HARMONICS                                    │
│   rms_envelope + ADSR overlay │   bar chart 1×…10× fundamental               │
│   trimmed_tail_windows shaded │   THD, odd/even ratio, n_harmonics readouts │
│ PITCH TRACK                   │ STEREO                                       │
│   pitch_envelope + expected   │   goniometer + L/R/M/S meters + width/corr  │
│   confidence bar              │                                              │
├───────────────────────────────┴──────────────────────────────────────────────┤
│ ⚠ WARNINGS  (none)        │  CENTROID TREND +291 Hz/s · trimmed 12 windows  │
└──────────────────────────────────────────────────────────────────────────────┘
```

Three columns:

- **Header row.** Test parameters (note, velocity, duration_ms, tail_ms,
  optional expected_note dropdown). "Re-analyze" runs the offline render
  again. "Pin" snapshots the current result as the "A" reference; subsequent
  analyses show as "B" overlaid.
- **Status chips row.** Coloured pill per `flags.*` plus the headline
  numerics. Green = clean, orange = sub-threshold concern, red = flag set.
  Clicking a chip scrolls to / highlights the relevant pane.
- **Left column (time domain):** Waveform, Envelope, Pitch track.
- **Right column (frequency / structure):** Spectrum, Harmonics, Stereo.
- **Bottom strip:** Warnings (collapsible, hidden when `warnings.is_empty()`),
  centroid trend, trimmed_tail_windows note.

## Field-by-pane mapping

| Pane               | Source field on `AnalyzeNoteResult`                                                                                                                                                                |
|--------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Status chips       | `flags`, `peak_amplitude`, `rms_overall`, `dc_offset`, `pitch_error_cents`, `pitch_confidence`, `stereo_correlation`                                                                               |
| Waveform L/R       | `RenderedNote.samples` (held in the snapshot as `Arc<RenderedNote>`); `RenderedNote.note_off_frame` for the boundary marker                                                                        |
| Envelope           | `rms_envelope`, `envelope_window_ms`, `envelope_estimate` (overlay)                                                                                                                                |
| Pitch track        | `pitch_envelope`, `pitch_envelope_window_ms`, `expected_fundamental_hz`, `pitch_confidence`. For stereo input also overlay `fundamental_left` / `fundamental_right` with `*_confidence` as opacity |
| Spectrum           | `spectrum_attack`, `spectrum_sustain`, `spectrum_release`, `attack_window_start_ms`, `sustain_window_start_ms`, `release_window_start_ms` (tooltip). `analysis_signal_mode` shown as a small badge |
| Harmonics          | `harmonic_content`, `spectrum_sustain` for individual partial bars                                                                                                                                 |
| Stereo             | `peak_left`, `peak_right`, `rms_left`, `rms_right`, `mid_rms`, `side_rms`, `stereo_width`, `stereo_correlation`. Goniometer reads the raw L/R buffer (decimated — see *Open questions*).           |
| Warnings strip     | `warnings`                                                                                                                                                                                         |
| Bottom-right strip | `centroid_trend_hz_per_sec`, `trimmed_tail_windows`                                                                                                                                                |

## Interactions

- Hover over a plot → tooltip with exact value + timestamp/frequency.
- Click a status chip → pane highlights / scrolls to relevant view.
- Click "Pin" (Remix Icon `ri::PUSHPIN_LINE`) → current analysis becomes
  "A". Next render becomes "B" and is overlaid in every plot with a
  contrasting colour. Pressing Pin again while a pin exists *replaces*
  the pin with the current "B" snapshot (so you can iterate "compare
  against latest good"). A separate "Unpin" affordance clears it.
  Delta formatting depends on metric type:
    - **Magnitudes** (peak, rms, THD): `0.098 → 0.123 (+25%)`.
    - **Signed values** (`dc_offset`, `centroid_trend_hz_per_sec`): show
      absolute delta only (`-0.001 → +0.004 (Δ +0.005)`); percent is
      meaningless when the baseline crosses zero.
    - **Pitch error**: cents delta (`-0.5¢ → +12.3¢ (Δ +12.8¢)`).
    - **Counts** (`clipped_samples`, `n_harmonics`): integer delta.
- Toggle waveform view: L only, R only, sum, M/S.
- Toggle spectrum legend entries on/off.
- Right-click on the analyze window → "Copy as JSON" (existing
  `serde_json::to_string_pretty(&result)`), "Save WAV" (re-encode the
  rendered buffer with `encode_buffer_as_wav` and prompt the user with
  the same file dialog used by patch-export — first filesystem
  interaction from this window, so it must go through the existing
  dialog layer rather than a hard-coded path).

## Component sketch

```rust
// crates/pertylizer/src/gui/analyze.rs (new file)

pub struct AnalyzeWindow {
    /// Latest analysis. None until first run.
    current: Option<AnalysisSnapshot>,
    /// Pinned reference for A/B compare. None when no pin.
    pinned: Option<AnalysisSnapshot>,

    /// Test parameters, persisted across re-analyzes.
    params: AnalyzeParams,
    /// Which instrument to analyze (driven by the host UI).
    target: InstrumentId,

    /// Background-render handle when a re-analysis is in flight.
    /// `None` between renders.
    pending: Option<PendingRender>,

    /// Per-pane UI state (zoom, toggles, hover position, ...).
    waveform_state: WaveformPaneState,
    spectrum_state: SpectrumPaneState,
    // ... one struct per pane
}

struct AnalysisSnapshot {
    result: synth_mcp::types::AnalyzeNoteResult,
    /// Raw render output. `Arc` so pinning doesn't double-copy ~1 MB/s
    /// of audio and so panes can share the buffer cheaply.
    rendered: Arc<crate::audio::preview::RenderedNote>,
    rendered_at: std::time::Instant,
}

struct PendingRender {
    /// JoinHandle for the worker thread running `render_note_to_buffer`
    /// + `analyze_rendered_buffer`. Polled each frame; result drained
    /// into `current` when ready.
    handle: std::thread::JoinHandle<Result<AnalysisSnapshot, McpBridgeError>>,
}

struct AnalyzeParams {
    note: u8,
    velocity: u8,
    duration_ms: u32,
    tail_ms: u32,
    expected_note: Option<u8>,
}

impl AnalyzeWindow {
    pub fn show(&mut self, ctx: &egui::Context, session: &SynthSession) { /* ... */ }
    fn run_analysis(&mut self, session: &SynthSession) { /* ... */ }
    fn render_status_chips(&mut self, ui: &mut egui::Ui) { /* ... */ }
    fn render_waveform_pane(&mut self, ui: &mut egui::Ui) { /* ... */ }
    // ... one render_* method per pane
}
```

## Implementation phases

No "schema plumbing" step is needed — `crates/pertylizer/src/mcp_bridge.rs`
already exposes `pub fn analyze_rendered_buffer(rendered: &RenderedNote, …)`
(split out from `analyze_rendered_note` for testing). The GUI calls
`render_note_to_buffer` directly, then `analyze_rendered_buffer` on the
returned `RenderedNote`, and stores both in `AnalysisSnapshot`.

1. **`AnalyzeWindow` skeleton.** Empty window, header with test parameters,
   "Re-analyze" button that spawns a worker thread (`render_note_to_buffer`
   + `analyze_rendered_buffer`) and polls the join handle each frame. No
   plots yet.
2. **Status chips + headline numerics.** First useful surface — even a plain
   chip strip is enough to drive iteration.
3. **Time-domain panes.** Waveform (cheap to plot from the raw buffer),
   envelope (just plot `rms_envelope`), pitch track. ADSR overlay on
   envelope reads `envelope_estimate`. On stereo input, overlay
   `fundamental_left`/`fundamental_right` on the pitch pane.
4. **Frequency-domain panes.** Spectrum (overlaid attack/sustain/release,
   `analysis_signal_mode` shown as badge), harmonics bar chart.
5. **Stereo pane.** Goniometer (decimated to ≤4 k points) + L/R/M/S meters
   + width/correlation readouts.
6. **Pin + A/B compare.** Snapshot → second slot, per-metric-type diff
   rendering (see *Interactions*).
7. **Warnings strip + bottom row.** Centroid trend, trimmed tail count,
   warnings list.
8. **Polish.** Hover tooltips, click-through navigation, copy-as-JSON,
   save-WAV, keyboard shortcut.

Steps 1–2 deliver something useful on day one. Steps 3–5 are independent
and can be parallelized.

## Test strategy

Most of the analyze-view value is visual; gate on:

- **Snapshot tests on the data plumbing.** A test instrument with a known
  patch → run analysis → assert specific status chips fire and the snapshot
  fields are populated as expected. Reuses the integration tests in
  `crates/pertylizer/tests/preview_integration.rs`.
- **A/B-diff unit test.** Given two `AnalysisSnapshot`s, the diff helper
  produces correct deltas for each metric category — including the edge
  cases that motivated the typed format: a `dc_offset` baseline that
  crosses zero, a `pitch_error_cents` that flips sign, an integer
  `clipped_samples` going from 0 → N (no division by zero).
- **UI-thread responsiveness.** A smoke test that calling the GUI's
  "re-analyze" entry point returns immediately and that the worker-thread
  join is non-blocking on subsequent frames.

Skip pixel-level GUI tests (egui's immediate-mode rendering doesn't
snapshot well) and skip a dedicated `Arc` lifetime test (testing the
language, not the code).

## Open questions

- **Re-analyze on patch change?** Could auto-rerun whenever a patch
  parameter changes (debounced). The worker-thread render doesn't block
  the live audio thread (offline engine), b ut it does churn — defer until
  users ask for it.
- **Goniometer decimation strategy.** A 2.5 s stereo render is ~110 k
  frames; plotting every sample is wasteful and unreadable. Sub-sample to
  ≤4 k points with peak-hold per bucket so anti-phase spikes survive.
  Specifics TBD when the pane lands.
- **Worker-thread cancellation.** If the user changes patches mid-render,
  the in-flight analysis is stale. Cheapest path: just let it finish and
  drop the result if `target` no longer matches when the join completes.

## Resolved (was open in earlier draft)

- **Per-instrument vs global window.** Decided: single global window that
  re-targets when the user picks a different instrument. Matches the
  patch-editor model. Multi-instance can be revisited if cross-patch A/B
  becomes a real workflow rather than a hypothetical one.
- **Test-note defaults.** `docs/patch-analysis-plan.md` was deleted (see
  `git status` at the time of writing); per-category defaults to fold in
  here later if the analyze view ever needs them. For v1: default to C4,
  velocity 100, 1500 ms duration, 1000 ms tail. Remembered per patch via
  the existing project-state layer.
- **Buffer lifetime.** Snapshot holds `Arc<RenderedNote>` directly — the
  whole struct, not separate `samples` / `note_off_frame` fields. Avoids
  divergent copies, keeps pinning to a refcount bump, and surfaces
  `effective_note` / `warnings` / `duration_seconds` to panes that want
  them without an extra plumbing pass.
- **Schema plumbing.** `analyze_rendered_buffer` already exists in the
  bridge; no new helper required.

## Cross-refs

- `crates/pertylizer/src/audio/preview.rs` — `render_note_to_buffer`,
  `RenderedNote`, `encode_buffer_as_wav`.
- `crates/pertylizer/src/audio/analysis.rs` — analysis primitives.
- `crates/pertylizer/src/mcp_bridge.rs` —
  `analyze_rendered_note` (full pipeline) and the public, doc-hidden
  `analyze_rendered_buffer(rendered, note, velocity, duration_ms,
  expected_note)` that the GUI uses to avoid double-rendering.
- `crates/synth_mcp/src/types.rs:AnalyzeNoteResult` — the shape of the
  data the window renders. Note `analysis_signal_mode`,
  `fundamental_left/right`, and `*_confidence` fields, which v1 should
  surface (badge on spectrum/pitch panes, overlay on pitch track).
