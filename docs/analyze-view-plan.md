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

**Floating `egui::Window`** (resizable, can be moved around, multiple
instances if useful for A/B-ing across patches). Triggered from:

- A toolbar button on the patch editor ("🔍 Analyze")
- An instrument-list context menu ("Analyze instrument…")
- Keyboard shortcut (default Ctrl-A; bind via the existing shortcut layer)

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

| Pane               | Source field on `AnalyzeNoteResult`                                                                                                                     |
|--------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| Status chips       | `flags`, `peak_amplitude`, `rms_overall`, `dc_offset`, `pitch_error_cents`, `pitch_confidence`, `stereo_correlation`                                    |
| Waveform L/R       | `RenderedNote.samples` (the raw f32 buffer — needs to be returned alongside the result; see *Open questions*); `note_off_frame` for the boundary marker |
| Envelope           | `rms_envelope`, `envelope_window_ms`, `envelope_estimate` (overlay)                                                                                     |
| Pitch track        | `pitch_envelope`, `pitch_envelope_window_ms`, `expected_fundamental_hz`, `pitch_confidence`                                                             |
| Spectrum           | `spectrum_attack`, `spectrum_sustain`, `spectrum_release`, `attack_window_start_ms`, `sustain_window_start_ms`, `release_window_start_ms` (tooltip)     |
| Harmonics          | `harmonic_content`, `spectrum_sustain` for individual partial bars                                                                                      |
| Stereo             | `peak_left`, `peak_right`, `rms_left`, `rms_right`, `mid_rms`, `side_rms`, `stereo_width`, `stereo_correlation`. Goniometer reads the raw L/R buffer.   |
| Warnings strip     | `warnings`                                                                                                                                              |
| Bottom-right strip | `centroid_trend_hz_per_sec`, `trimmed_tail_windows`                                                                                                     |

## Interactions

- Hover over a plot → tooltip with exact value + timestamp/frequency.
- Click a status chip → pane highlights / scrolls to relevant view.
- Click "📌 Pin" → current analysis becomes "A". Next render becomes "B"
  and is overlaid in every plot with a contrasting colour. Numerics show
  `0.098 → 0.123 (+25%)` style deltas.
- Toggle waveform view: L only, R only, sum, M/S.
- Toggle spectrum legend entries on/off.
- Right-click on the analyze window → "Copy as JSON" (existing
  `serde_json::to_string_pretty(&result)`), "Save WAV" (re-encode the
  rendered buffer with `encode_buffer_as_wav`).

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

    /// Live re-analysis state — set when a render is in flight.
    in_flight: bool,

    /// Per-pane UI state (zoom, toggles, hover position, ...).
    waveform_state: WaveformPaneState,
    spectrum_state: SpectrumPaneState,
    // ... one struct per pane
}

struct AnalysisSnapshot {
    result: synth_mcp::types::AnalyzeNoteResult,
    raw_samples: Arc<Vec<f32>>, // shared with the goniometer/waveform plot
    sample_rate: u32,
    note_off_frame: u64,
    rendered_at: std::time::Instant,
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

1. **Schema plumbing.** Make sure the `AnalysisSnapshot` we keep around in the
   GUI has access to the raw stereo samples + `note_off_frame`. The MCP
   bridge currently returns only the analysis numerics — a sibling helper
   `analyze_rendered_note_with_buffer` (or returning a tuple) keeps the
   render buffer alive without re-rendering for plotting.
2. **`AnalyzeWindow` skeleton.** Empty window, header with test parameters,
   "Re-analyze" button wired to `render_note_to_buffer` +
   `analyze_rendered_note`. No plots yet.
3. **Status chips + headline numerics.** First useful surface — even a plain
   chip strip is enough to drive iteration.
4. **Time-domain panes.** Waveform (cheap to plot from the raw buffer),
   envelope (just plot `rms_envelope`), pitch track. ADSR overlay on
   envelope reads `envelope_estimate`.
5. **Frequency-domain panes.** Spectrum (overlaid attack/sustain/release),
   harmonics bar chart.
6. **Stereo pane.** Goniometer + L/R/M/S meters + width/correlation
   readouts.
7. **Pin + A/B compare.** Snapshot → second slot, plot/numeric diff
   rendering.
8. **Warnings strip + bottom row.** Centroid trend, trimmed tail count,
   warnings list.
9. **Polish.** Hover tooltips, click-through navigation, copy-as-JSON,
   save-WAV, keyboard shortcut.

Steps 1–3 deliver something useful on day one. Steps 4–6 are independent and
can be parallelized.

## Test strategy

Most of the analyze-view value is visual; gate on:

- **Snapshot tests on the data plumbing.** A test instrument with a known
  patch → run analysis → assert specific status chips fire and the snapshot
  fields are populated as expected. Reuses the integration tests in
  `crates/pertylizer/tests/preview_integration.rs`.
- **A/B-diff unit test.** Given two `AnalysisSnapshot`s, the diff helper
  produces stable, signed deltas for the headline numerics.
- **Render-buffer lifetime test.** `Arc<Vec<f32>>` ensures re-analyzing
  doesn't free the pinned buffer.

Skip pixel-level GUI tests. Egui's immediate-mode rendering doesn't snapshot
well, and the panes are fundamentally about looking-at, not asserting.

## Open questions

- **Should the analyze window be per-instrument or a single global window
  that re-targets?** Per-instrument lets users keep multiple analyses open
  for cross-patch A/B. Global is simpler and matches the patch-editor
  model. Recommendation: start global, add multi-instance later if asked.
- **Re-analyze on patch change?** Could auto-rerun whenever a patch
  parameter changes (debounced). Risk: ~50–100 ms render per change is
  fine, but blocks the audio thread momentarily for the snapshot read. Defer
  until users complain about the manual button.
- **Where does the test note default come from?** If the patch has a
  category (`bass`, `pad`), use the per-category defaults from
  `docs/patch-analysis-plan.md`. Otherwise C4. Should be remembered per
  patch.
- **Buffer lifetime.** Returning the raw `Vec<f32>` from
  `render_note_to_buffer` already wraps it in `RenderedNote`; the GUI just
  needs to keep the struct alive in `AnalysisSnapshot`. Worth wrapping in
  `Arc<Vec<f32>>` so pinning doesn't double-copy ~1 MB per second of audio.

## Cross-refs

- `crates/pertylizer/src/audio/preview.rs` — `render_note_to_buffer`,
  `RenderedNote`.
- `crates/pertylizer/src/audio/analysis.rs` — analysis primitives.
- `crates/pertylizer/src/mcp_bridge.rs:analyze_rendered_note` — the
  function the window will call.
- `crates/synth_mcp/src/types.rs:AnalyzeNoteResult` — the shape of the
  data the window renders.
- `docs/patch-analysis-plan.md` — methodology for per-category test
  parameters; reuse those defaults in the window's parameter row.
