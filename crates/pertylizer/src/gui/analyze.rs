//! Floating "Analyze" window — GUI surface for the `analyze_note` pipeline.
//!
//! Wraps [`render_note_to_buffer`] + [`analyze_rendered_buffer`] in a
//! re-runnable, A/B-comparable view. The render and analysis pass run on a
//! worker thread so the UI thread stays responsive.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use std::path::Path;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_file_dialog::{FileDialog, Filter};
use egui_remixicon::icons as ri;

use synth_core::types::{MidiNote, Velocity};
use synth_engine::InstrumentId;
use synth_mcp::error::McpBridgeError;
use synth_mcp::types::{AnalysisSignalMode, AnalyzeFlags, AnalyzeNoteResult, AnalyzeSpectrumPeak};

use crate::audio::preview::{RenderedNote, encode_buffer_as_wav, render_note_to_buffer};
use crate::gui::theme::theme;
use crate::mcp_bridge::analyze_rendered_buffer;
use crate::session::SynthSession;

/// Test-render parameters. Persisted across re-analyses.
#[derive(Debug, Clone)]
struct AnalyzeParams {
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
    /// Pitch reference. `None` falls back to `note`.
    expected_note: Option<MidiNote>,
}

impl Default for AnalyzeParams {
    fn default() -> Self {
        Self {
            note: MidiNote::C4,
            velocity: Velocity::from_midi(100),
            duration_ms: 1500,
            tail_ms: 1000,
            expected_note: None,
        }
    }
}

/// One render+analyze pass, plus derived values cached so per-frame
/// rendering doesn't have to scan the immutable buffer / spectrum / pitch
/// arrays on every paint.
struct AnalysisSnapshot {
    result: AnalyzeNoteResult,
    /// Shared so pinning is a refcount bump, not a buffer copy.
    rendered: Arc<RenderedNote>,
    rendered_at: Instant,
    /// Cached harmonic partial dB values (1×..=10×). Read by the
    /// harmonics pane every frame; computed once at construction.
    partials_db: [f32; 10],
    /// Cached peak of `result.rms_envelope` for the envelope plot's Y scale.
    rms_envelope_peak: f32,
    /// Cached non-zero pitch range for the pitch pane's Y scale.
    /// `None` when the snapshot has no pitched content.
    pitch_range_hz: Option<(f32, f32)>,
}

impl AnalysisSnapshot {
    fn new(result: AnalyzeNoteResult, rendered: Arc<RenderedNote>) -> Self {
        let partials_db = compute_partials_db(&result);
        let rms_envelope_peak = max_or(&result.rms_envelope, 1e-6);
        let pitch_range_hz = compute_pitch_range(&result);
        Self {
            result,
            rendered,
            rendered_at: Instant::now(),
            partials_db,
            rms_envelope_peak,
            pitch_range_hz,
        }
    }

    fn clone_for_pin(&self) -> Self {
        Self {
            result: self.result.clone(),
            rendered: Arc::clone(&self.rendered),
            rendered_at: self.rendered_at,
            partials_db: self.partials_db,
            rms_envelope_peak: self.rms_envelope_peak,
            pitch_range_hz: self.pitch_range_hz,
        }
    }
}

fn compute_partials_db(r: &AnalyzeNoteResult) -> [f32; 10] {
    let mut partials = [-80.0_f32; 10];
    if r.fundamental_hz <= 0.0 {
        return partials;
    }
    for (i, slot) in partials.iter_mut().enumerate() {
        let target = r.fundamental_hz * (i + 1) as f32;
        let tol = (target * 0.05).max(2.0);
        *slot = nearest_peak_db(&r.spectrum_sustain, target, tol).unwrap_or(-80.0);
    }
    partials
}

fn compute_pitch_range(r: &AnalyzeNoteResult) -> Option<(f32, f32)> {
    let mut min_hz = f32::INFINITY;
    let mut max_hz = 0.0_f32;
    for v in r.pitch_envelope.iter().copied() {
        if v > 0.0 {
            min_hz = min_hz.min(v);
            max_hz = max_hz.max(v);
        }
    }
    if r.expected_fundamental_hz > 0.0 {
        min_hz = min_hz.min(r.expected_fundamental_hz);
        max_hz = max_hz.max(r.expected_fundamental_hz);
    }
    if !min_hz.is_finite() || max_hz <= 0.0 {
        return None;
    }
    let pad = ((max_hz - min_hz) * 0.05).max(1.0);
    Some((min_hz - pad, max_hz + pad))
}

struct PendingRender {
    handle: JoinHandle<Result<AnalysisSnapshot, McpBridgeError>>,
    target: InstrumentId,
}

/// Channel selection for the waveform pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaveformMode {
    Stereo,
    Left,
    Right,
    Sum,
    MidSide,
}

impl WaveformMode {
    fn label(self) -> &'static str {
        match self {
            Self::Stereo => "L/R",
            Self::Left => "L",
            Self::Right => "R",
            Self::Sum => "Sum",
            Self::MidSide => "M/S",
        }
    }
}

/// Floating analyze window. Single instance, re-targeted by the host UI.
pub struct AnalyzeWindow {
    /// Visibility flag. Toggled via [`Self::open`], [`Self::close`] and
    /// [`Self::toggle_open`]; egui flips it back to false when the user
    /// hits the window close button.
    open: bool,

    current: Option<AnalysisSnapshot>,
    pinned: Option<AnalysisSnapshot>,

    params: AnalyzeParams,
    target: Option<InstrumentId>,
    target_name: String,

    pending: Option<PendingRender>,
    last_error: Option<String>,
    last_status: Option<String>,

    waveform_mode: WaveformMode,

    /// Inner height of the window from the previous frame. Captured after
    /// each `Window::show` so the body can size plots from it without
    /// feeding `ui.available_height()` back through egui's resize widget
    /// (which grows the window when content is taller than the desired
    /// size). `vscroll(true)` makes the inner ui's `available_height`
    /// unbounded, so we can't read the visible area from the inner ui.
    last_window_height: f32,

    /// File dialog for "Save WAV…". Owned by the window so it doesn't share
    /// state with the app-level export-WAV flow (different format domain:
    /// raw note buffer, not a project render).
    wav_dialog: FileDialog,
    /// WAV bytes ready to write once the dialog returns a path.
    pending_wav_bytes: Option<Vec<u8>>,
}

impl AnalyzeWindow {
    pub fn new() -> Self {
        Self {
            open: false,
            current: None,
            pinned: None,
            params: AnalyzeParams::default(),
            target: None,
            target_name: String::new(),
            pending: None,
            last_error: None,
            last_status: None,
            waveform_mode: WaveformMode::Stereo,
            wav_dialog: FileDialog::new().add_file_filter(
                "WAV files",
                Filter::new(|p: &Path| p.extension().is_some_and(|e| e == "wav")),
            ),
            pending_wav_bytes: None,
            last_window_height: 640.0,
        }
    }

    /// Update the target instrument. Called by the host UI each frame so
    /// the window automatically follows the active instrument.
    pub fn set_target(&mut self, id: InstrumentId, name: impl Into<String>) {
        let name = name.into();
        if self.target != Some(id) {
            self.current = None;
            self.last_error = None;
        }
        self.target = Some(id);
        self.target_name = name;
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn toggle_open(&mut self) {
        self.open = !self.open;
    }

    fn pin_current(&mut self) {
        if let Some(snap) = self.current.as_ref() {
            self.pinned = Some(snap.clone_for_pin());
        }
    }

    fn unpin(&mut self) {
        self.pinned = None;
    }

    fn copy_as_json(&mut self, ctx: &egui::Context) {
        let Some(snap) = self.current.as_ref() else {
            return;
        };
        match serde_json::to_string_pretty(&snap.result) {
            Ok(text) => {
                ctx.copy_text(text);
                self.last_status = Some("Copied analysis JSON to clipboard".to_string());
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(format!("JSON encode failed: {err}"));
            }
        }
    }

    fn begin_save_wav(&mut self) {
        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let bytes = match encode_buffer_as_wav(
            &snap.rendered.samples,
            snap.rendered.sample_rate,
            snap.rendered.channels,
        ) {
            Ok(b) => b,
            Err(err) => {
                self.last_error = Some(format!("WAV encode failed: {err}"));
                return;
            }
        };
        let default_name = wav_default_name(&self.target_name, snap.result.note_played);
        self.wav_dialog = FileDialog::new()
            .add_file_filter(
                "WAV files",
                Filter::new(|p: &Path| p.extension().is_some_and(|e| e == "wav")),
            )
            .default_file_name(&default_name);
        self.wav_dialog.save_file();
        self.pending_wav_bytes = Some(bytes);
    }

    /// Drive the WAV save dialog. Writes the pending bytes to the chosen
    /// path on completion, or releases them if the user cancelled.
    fn poll_wav_dialog(&mut self, ctx: &egui::Context) {
        if self.pending_wav_bytes.is_none() {
            return;
        }
        self.wav_dialog.update(ctx);
        match self.wav_dialog.state() {
            egui_file_dialog::DialogState::Picked(_) => {
                if let Some(path) = self.wav_dialog.take_picked() {
                    let path = if path.extension().is_some_and(|e| e == "wav") {
                        path
                    } else {
                        path.with_extension("wav")
                    };
                    let bytes = self.pending_wav_bytes.take().unwrap_or_default();
                    match std::fs::write(&path, &bytes) {
                        Ok(()) => {
                            self.last_status = Some(format!("Saved WAV: {}", path.display()));
                            self.last_error = None;
                        }
                        Err(err) => {
                            self.last_error = Some(format!("WAV write failed: {err}"));
                        }
                    }
                }
            }
            egui_file_dialog::DialogState::Cancelled | egui_file_dialog::DialogState::Closed => {
                // Dialog dismissed without picking a path — release the
                // multi-MB encoded buffer so it doesn't sit until the next
                // save attempt.
                self.pending_wav_bytes = None;
            }
            _ => {}
        }
    }

    fn spawn_render(&mut self, session: Arc<SynthSession>) {
        let Some(target) = self.target else { return };
        if self.pending.is_some() {
            return;
        }

        let params = self.params.clone();
        let handle = std::thread::spawn(move || -> Result<AnalysisSnapshot, McpBridgeError> {
            let rendered = render_note_to_buffer(
                session.as_ref(),
                target,
                params.note,
                params.velocity,
                params.duration_ms,
                params.tail_ms,
            )?;
            let result = analyze_rendered_buffer(
                &rendered,
                params.note.as_u8(),
                params.velocity.to_midi(),
                params.duration_ms,
                params.expected_note.map(MidiNote::as_u8),
            );
            Ok(AnalysisSnapshot::new(result, Arc::new(rendered)))
        });

        self.pending = Some(PendingRender { handle, target });
    }

    fn poll_pending(&mut self) {
        let finished = self
            .pending
            .as_ref()
            .is_some_and(|p| p.handle.is_finished());
        if !finished {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        match pending.handle.join() {
            Ok(Ok(snap)) => {
                // Stale-result drop: the user changed target while the
                // worker was running, so the snapshot doesn't describe the
                // current target.
                if Some(pending.target) == self.target {
                    self.current = Some(snap);
                    self.last_error = None;
                }
            }
            Ok(Err(err)) => {
                self.last_error = Some(format!("{err}"));
            }
            Err(_) => {
                self.last_error = Some("Render thread panicked".to_string());
            }
        }
    }

    /// Render the window. Call every frame from the host.
    pub fn show(&mut self, ctx: &egui::Context, session: &Arc<SynthSession>) {
        self.poll_pending();
        self.poll_wav_dialog(ctx);

        if !self.open {
            return;
        }

        let mut open = self.open;
        let title = if self.target_name.is_empty() {
            format!("{} Analyze", ri::FILE_SEARCH_LINE)
        } else {
            format!("{} Analyze: {}", ri::FILE_SEARCH_LINE, self.target_name)
        };

        let mut copy_json = false;
        let mut save_wav = false;

        // Build the per-frame palette/font snapshot once and pass it down.
        // Each `theme()` call grabs a parking_lot read lock; doing it once
        // saves 6+ acquisitions per frame and keeps colours coherent within
        // a frame even if the theme is swapped mid-render.
        let pt = PaneTheme::from_global();

        let response = egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .vscroll(true)
            .default_size([960.0, 640.0])
            .min_size([720.0, 360.0])
            .show(ctx, |ui| {
                self.render_header(ui, session, &mut copy_json, &mut save_wav);
                ui.separator();
                self.render_status_strip(ui, &pt);
                if let Some(err) = self.last_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                if let Some(msg) = self.last_status.as_ref() {
                    ui.colored_label(pt.expected, msg);
                }
                ui.separator();
                self.render_body_columns(ui, &pt);
                ui.separator();
                self.render_bottom_strip(ui);
            });
        if let Some(inner) = response {
            self.last_window_height = inner.response.rect.height();
        }
        self.open = open;

        if copy_json {
            self.copy_as_json(ctx);
        }
        if save_wav {
            self.begin_save_wav();
        }
    }

    fn render_header(
        &mut self,
        ui: &mut egui::Ui,
        session: &Arc<SynthSession>,
        copy_json: &mut bool,
        save_wav: &mut bool,
    ) {
        ui.horizontal(|ui| {
            ui.label("Note");
            let mut note_u8 = self.params.note.as_u8();
            if ui
                .add(egui::DragValue::new(&mut note_u8).range(0..=127))
                .on_hover_text(self.params.note.name())
                .changed()
            {
                self.params.note = MidiNote::new(note_u8);
            }
            ui.label(self.params.note.name());

            ui.separator();
            ui.label("Vel");
            let mut vel_midi = i32::from(self.params.velocity.to_midi());
            if ui
                .add(egui::DragValue::new(&mut vel_midi).range(1..=127))
                .changed()
            {
                self.params.velocity = Velocity::from_midi(vel_midi.clamp(1, 127) as u8);
            }

            ui.separator();
            ui.label("Dur (ms)");
            ui.add(egui::DragValue::new(&mut self.params.duration_ms).range(50..=10_000));

            ui.label("Tail (ms)");
            ui.add(egui::DragValue::new(&mut self.params.tail_ms).range(0..=10_000));

            ui.separator();
            ui.label("Expected");
            let mut has_expected = self.params.expected_note.is_some();
            if ui.checkbox(&mut has_expected, "").changed() {
                self.params.expected_note = if has_expected {
                    Some(self.params.note)
                } else {
                    None
                };
            }
            if let Some(exp) = self.params.expected_note.as_mut() {
                let mut exp_u8 = exp.as_u8();
                if ui
                    .add(egui::DragValue::new(&mut exp_u8).range(0..=127))
                    .changed()
                {
                    *exp = MidiNote::new(exp_u8);
                }
                ui.label(exp.name());
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // In a right-to-left layout, the *first* widget added is
                // anchored at the far right. Order here is therefore the
                // visual order from right to left: kebab → Re-analyze → Pin
                // → Unpin (when pinned).
                let has_result = self.current.is_some();
                ui.menu_button(ri::MORE_LINE.to_string(), |ui| {
                    if ui
                        .add_enabled(
                            has_result,
                            egui::Button::new(format!("{} Copy as JSON", ri::CLIPBOARD_LINE)),
                        )
                        .clicked()
                    {
                        *copy_json = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            has_result,
                            egui::Button::new(format!("{} Save WAV…", ri::DOWNLOAD_LINE)),
                        )
                        .clicked()
                    {
                        *save_wav = true;
                        ui.close();
                    }
                });

                let busy = self.pending.is_some();
                let can_run = !busy && self.target.is_some();
                let label = if busy {
                    format!("{} Rendering…", ri::REFRESH_LINE)
                } else {
                    format!("{} Re-analyze", ri::PLAY_LINE)
                };
                if ui
                    .add_enabled(can_run, egui::Button::new(label))
                    .on_hover_text("Render and analyze the active patch")
                    .clicked()
                {
                    self.spawn_render(Arc::clone(session));
                }

                let pin_label = if self.pinned.is_some() {
                    format!("{} Repin", ri::PUSHPIN_FILL)
                } else {
                    format!("{} Pin", ri::PUSHPIN_LINE)
                };
                if ui
                    .add_enabled(self.current.is_some(), egui::Button::new(pin_label))
                    .on_hover_text("Snapshot the current analysis as the A reference")
                    .clicked()
                {
                    self.pin_current();
                }

                if self.pinned.is_some()
                    && ui
                        .button(format!("{} Unpin", ri::UNPIN_LINE))
                        .on_hover_text("Clear the A reference")
                        .clicked()
                {
                    self.unpin();
                }
            });
        });
    }

    fn render_status_strip(&mut self, ui: &mut egui::Ui, pt: &PaneTheme) {
        let Some(snap) = self.current.as_ref() else {
            if self.pending.is_some() {
                ui.label("Rendering…");
            } else if self.target.is_none() {
                ui.label("No instrument selected.");
            } else {
                ui.label("Press Re-analyze to render and analyze the patch.");
            }
            return;
        };

        let r = &snap.result;
        let pin = self.pinned.as_ref().map(|p| &p.result);
        ui.horizontal_wrapped(|ui| {
            chip(ui, "PASS", overall_status(&r.flags), pt);
            ui.separator();
            metric(
                ui,
                "peak",
                &format!("{:.4}", r.peak_amplitude),
                pin.map(|p| (r.peak_amplitude, p.peak_amplitude, DeltaKind::Magnitude)),
                pt,
            );
            metric(
                ui,
                "rms",
                &format!("{:.4}", r.rms_overall),
                pin.map(|p| (r.rms_overall, p.rms_overall, DeltaKind::Magnitude)),
                pt,
            );
            metric(
                ui,
                "DC",
                &format!("{:+.4}", r.dc_offset),
                pin.map(|p| (r.dc_offset, p.dc_offset, DeltaKind::Signed4)),
                pt,
            );
            metric(
                ui,
                "pitch",
                &format!("{:+.1}¢", r.pitch_error_cents),
                pin.map(|p| (r.pitch_error_cents, p.pitch_error_cents, DeltaKind::Cents)),
                pt,
            );
            if let Some(conf) = r.pitch_confidence {
                ui.label(format!("(conf {:.0}%)", conf * 100.0));
            }
            metric(
                ui,
                "st-cor",
                &format!("{:.2}", r.stereo_correlation),
                pin.map(|p| {
                    (
                        r.stereo_correlation,
                        p.stereo_correlation,
                        DeltaKind::Signed2,
                    )
                }),
                pt,
            );
            if r.clipped_samples > 0 || pin.is_some_and(|p| p.clipped_samples > 0) {
                metric(
                    ui,
                    "clipped",
                    &format!("{}", r.clipped_samples),
                    pin.map(|p| {
                        (
                            r.clipped_samples as f32,
                            p.clipped_samples as f32,
                            DeltaKind::Integer,
                        )
                    }),
                    pt,
                );
            }
            ui.separator();

            // Quiet "OK" pills suppressed; the green PASS chip covers the
            // all-clean case.
            if r.flags.silent {
                chip(ui, "SILENT", ChipKind::Bad, pt);
            }
            if r.flags.clipping {
                chip(ui, "CLIPPING", ChipKind::Bad, pt);
            }
            if r.flags.has_dc_offset {
                chip(ui, "DC", ChipKind::Warn, pt);
            }
            if r.flags.low_output {
                chip(ui, "LOW OUT", ChipKind::Warn, pt);
            }
            if r.flags.off_pitch {
                chip(ui, "OFF PITCH", ChipKind::Bad, pt);
            }
        });
    }

    fn render_body_columns(&mut self, ui: &mut egui::Ui, pt: &PaneTheme) {
        // Plot heights derive from the previous frame's window inner height
        // (captured after `Window::show` returned). Using the *outer*
        // window size — instead of `ui.available_height()` inside the
        // `vscroll` ScrollArea — avoids the egui Resize feedback loop where
        // any overshoot of body content vs. window size grows
        // `desired_size` each frame. With vscroll on, the inner ui's
        // `available_height` is `f32::INFINITY` anyway, so we couldn't use
        // it even if we wanted to. Both columns receive the same plot_h,
        // which keeps left/right rows aligned in a 2-column grid.
        const WINDOW_CHROME: f32 = 200.0; // title bar + header + status + bottom strip
        const ROW_CHROME: f32 = 44.0; // pane title + footer
        const MIN_PLOT_H: f32 = 80.0;
        const ROW_GAP: f32 = 4.0;

        let body_h = (self.last_window_height - WINDOW_CHROME).max(3.0 * (MIN_PLOT_H + ROW_CHROME));
        let row_h = (body_h - 2.0 * ROW_GAP) / 3.0;
        let plot_h = (row_h - ROW_CHROME).max(MIN_PLOT_H);

        if self.current.is_none() {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), body_h), Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Run an analysis to populate the panes.",
                theme().fonts.normal(),
                pt.text_dim,
            );
            return;
        }

        ui.columns(2, |cols| {
            self.render_time_domain_panes(&mut cols[0], plot_h, pt);
            self.render_frequency_domain_panes(&mut cols[1], plot_h, pt);
        });
    }

    fn render_time_domain_panes(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        self.render_waveform_pane(ui, plot_h, pt);
        ui.add_space(4.0);
        self.render_envelope_pane(ui, plot_h, pt);
        ui.add_space(4.0);
        self.render_pitch_pane(ui, plot_h, pt);
    }

    fn render_frequency_domain_panes(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        self.render_spectrum_pane(ui, plot_h, pt);
        ui.add_space(4.0);
        self.render_harmonics_pane(ui, plot_h, pt);
        ui.add_space(4.0);
        self.render_stereo_pane(ui, plot_h, pt);
    }

    fn render_waveform_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Waveform", INFO_WAVEFORM);
            ui.separator();
            for mode in [
                WaveformMode::Stereo,
                WaveformMode::Left,
                WaveformMode::Right,
                WaveformMode::Sum,
                WaveformMode::MidSide,
            ] {
                ui.selectable_value(&mut self.waveform_mode, mode, mode.label());
            }
        });

        let (rect, response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let (Some(snap), pinned) = (self.current.as_ref(), self.pinned.as_ref()) else {
            return;
        };

        let mode = self.waveform_mode;
        if matches!(mode, WaveformMode::Stereo | WaveformMode::MidSide) {
            // Mid-line separator between the two stacked half-rects so the
            // viewer can tell L from R / M from S at a glance.
            let mid_y = rect.center().y;
            painter.line_segment(
                [
                    Pos2::new(rect.left(), mid_y),
                    Pos2::new(rect.right(), mid_y),
                ],
                Stroke::new(0.5, pt.axis),
            );
        }
        if let Some(pin) = pinned {
            draw_waveform(&painter, rect, &pin.rendered, mode, pt.pin, true);
        }
        draw_waveform(&painter, rect, &snap.rendered, mode, pt.current, false);

        // Note-off marker (anchored on the current snapshot's render).
        let total_frames =
            snap.rendered.samples.len() as u64 / u64::from(snap.rendered.channels.max(1));
        if total_frames > 0 && snap.rendered.note_off_frame < total_frames {
            let t = snap.rendered.note_off_frame as f32 / total_frames as f32;
            let x = rect.left() + t * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, pt.note_off),
            );
        }

        if let Some(pos) = response.hover_pos() {
            draw_crosshair(&painter, rect, pos.x, pt.crosshair);
            let tip = waveform_tooltip(rect, pos, &snap.rendered);
            response.on_hover_text(tip);
        }

        // Footer — keeps pane chrome height consistent with the other panes
        // so the left/right columns align in a row grid.
        let kind: String = match snap.rendered.channels {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            n => format!("{n} ch"),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{} · {} Hz · {:.3} s",
                kind, snap.rendered.sample_rate, snap.rendered.duration_seconds,
            ));
        });
    }

    fn render_envelope_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Envelope (RMS + ADSR estimate)", INFO_ENVELOPE);
        });

        let (rect, response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;

        // Y scale shared across current + pin (cached on each snapshot at
        // construction so deltas are meaningful without rescanning each
        // frame).
        let peak = self
            .pinned
            .as_ref()
            .map(|p| snap.rms_envelope_peak.max(p.rms_envelope_peak))
            .unwrap_or(snap.rms_envelope_peak);

        if let Some(pin) = self.pinned.as_ref() {
            draw_polyline_normalized(&painter, rect, &pin.result.rms_envelope, peak, pt.pin);
        }
        draw_polyline_normalized(&painter, rect, &r.rms_envelope, peak, pt.current);

        draw_adsr_markers(&painter, rect, snap, pt);

        if let Some(pos) = response.hover_pos() {
            draw_crosshair(&painter, rect, pos.x, pt.crosshair);
            let tip = envelope_tooltip(rect, pos, r);
            response.on_hover_text(tip);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "A {:.0} ms · D {:.0} ms · S {:.2} · R {:.0} ms",
                r.envelope_estimate.attack_ms,
                r.envelope_estimate.decay_ms,
                r.envelope_estimate.sustain_level,
                r.envelope_estimate.release_ms,
            ));
            ui.separator();
            ui.label(format!("window {:.0} ms", r.envelope_window_ms));
        });
    }

    fn render_pitch_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Pitch track", INFO_PITCH);
        });

        let (rect, response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;

        // Y range cached on each snapshot at construction. Combine with
        // the pinned snapshot's range so both traces share a scale.
        let range = match (
            snap.pitch_range_hz,
            self.pinned.as_ref().and_then(|p| p.pitch_range_hz),
        ) {
            (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        let Some((min_hz, max_hz)) = range else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "no pitched content",
                pt.font_small.clone(),
                pt.text_dim,
            );
            return;
        };

        if r.expected_fundamental_hz > 0.0 {
            let y = pitch_to_y(rect, r.expected_fundamental_hz, min_hz, max_hz);
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, pt.expected),
            );
        }

        if let Some(pin) = self.pinned.as_ref() {
            draw_pitch_polyline(
                &painter,
                rect,
                &pin.result.pitch_envelope,
                min_hz,
                max_hz,
                pt.pin,
            );
        }
        draw_pitch_polyline(
            &painter,
            rect,
            &r.pitch_envelope,
            min_hz,
            max_hz,
            pt.current,
        );

        if let Some(pos) = response.hover_pos() {
            draw_crosshair(&painter, rect, pos.x, pt.crosshair);
            let tip = pitch_tooltip(rect, pos, r);
            response.on_hover_text(tip);
        }

        // Per-channel scalars when stereo.
        let mut footer = String::new();
        if let (Some(l), Some(rh)) = (r.fundamental_left, r.fundamental_right) {
            footer.push_str(&format!("L {l:.1} Hz · R {rh:.1} Hz"));
            if let (Some(lc), Some(rc)) = (
                r.fundamental_left_confidence,
                r.fundamental_right_confidence,
            ) {
                footer.push_str(&format!(
                    " (conf L {:.0}%, R {:.0}%)",
                    lc * 100.0,
                    rc * 100.0
                ));
            }
        } else {
            footer.push_str(&format!("fundamental {:.2} Hz", r.fundamental_hz));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(footer);
            ui.separator();
            ui.label(format!("window {:.0} ms", r.pitch_envelope_window_ms));
            ui.separator();
            ui.label(format!("{:?}", r.analysis_signal_mode));
        });
    }

    fn render_spectrum_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Spectrum", INFO_SPECTRUM);
            ui.separator();
            if let Some(snap) = self.current.as_ref() {
                let mode = match snap.result.analysis_signal_mode {
                    AnalysisSignalMode::Mono => "mono",
                    AnalysisSignalMode::MaxAbsStereo => "stereo (max-abs)",
                };
                ui.label(RichText::new(mode).weak().small()).on_hover_text(
                    "Signal the spectrum was computed on. \
                         For stereo input the analysis runs on a synthetic \
                         max(|L|,|R|) signal so anti-phase content survives.",
                );
            }
            ui.separator();
            legend_swatch(ui, pt.spec_attack, "attack");
            legend_swatch(ui, pt.spec_sustain, "sustain");
            legend_swatch(ui, pt.spec_release, "release");
        });

        let (rect, response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;
        let nyquist = (r.sample_rate as f32 * 0.5).max(2_000.0);
        let axes = SpecAxes {
            log_min: 20f32.log10(),
            log_max: nyquist.log10(),
            db_min: -60.0,
            db_max: 0.0,
        };

        // Decade gridlines (100 Hz, 1 kHz, 10 kHz when in range).
        for &decade in &[100.0_f32, 1_000.0, 10_000.0] {
            if decade < 20.0 || decade > nyquist {
                continue;
            }
            let x = freq_to_x(rect, decade, axes.log_min, axes.log_max);
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(0.5, pt.axis),
            );
            painter.text(
                Pos2::new(x + 2.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format_freq(decade),
                pt.font_small.clone(),
                pt.text_dim,
            );
        }
        // -20/-40 dB gridlines.
        for &db in &[-20.0_f32, -40.0] {
            let y = db_to_y(rect, db, axes.db_min, axes.db_max);
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(0.5, pt.axis),
            );
            painter.text(
                Pos2::new(rect.left() + 2.0, y - 1.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{db:.0} dB"),
                pt.font_small.clone(),
                pt.text_dim,
            );
        }

        draw_spectrum_stems(&painter, rect, &r.spectrum_release, &axes, pt.spec_release);
        draw_spectrum_stems(&painter, rect, &r.spectrum_sustain, &axes, pt.spec_sustain);
        draw_spectrum_stems(&painter, rect, &r.spectrum_attack, &axes, pt.spec_attack);

        if let Some(pos) = response.hover_pos() {
            draw_crosshair(&painter, rect, pos.x, pt.crosshair);
            let tip = spectrum_tooltip(rect, pos, r, &axes);
            response.on_hover_text(tip);
        }

        ui.horizontal_wrapped(|ui| {
            if let Some(start) = r.attack_window_start_ms {
                ui.label(format!("attack @ {start:.0} ms"));
            }
            if let Some(start) = r.sustain_window_start_ms {
                ui.separator();
                ui.label(format!("sustain @ {start:.0} ms"));
            }
            if let Some(start) = r.release_window_start_ms {
                ui.separator();
                ui.label(format!("release @ {start:.0} ms"));
            }
        });
    }

    fn render_harmonics_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Harmonics (sustain)", INFO_HARMONICS);
        });

        let (rect, response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;

        if r.fundamental_hz <= 0.0 {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "no detectable fundamental",
                pt.font_small.clone(),
                pt.text_dim,
            );
            return;
        }

        // Cached at snapshot construction. Bars below the noise floor
        // (-80 dB) render as a thin tick so the user sees "this partial is
        // missing", not "data is gone".
        const N: usize = 10;
        let partials = &snap.partials_db;

        let db_min = -60.0_f32;
        let db_max = 0.0_f32;
        let bar_w = rect.width() / N as f32;

        // Y gridlines.
        for &db in &[-20.0_f32, -40.0] {
            let y = db_to_y(rect, db, db_min, db_max);
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(0.5, pt.axis),
            );
        }

        for (i, &db) in partials.iter().enumerate() {
            let x0 = rect.left() + i as f32 * bar_w + bar_w * 0.15;
            let x1 = rect.left() + (i + 1) as f32 * bar_w - bar_w * 0.15;
            let y_top = db_to_y(rect, db.clamp(db_min, db_max), db_min, db_max);
            let y_bot = rect.bottom();
            let bar =
                Rect::from_min_max(Pos2::new(x0, y_top.min(y_bot - 1.0)), Pos2::new(x1, y_bot));
            painter.rect_filled(bar, 1.0, pt.harmonic);
            painter.text(
                Pos2::new((x0 + x1) * 0.5, rect.bottom() + 1.0),
                egui::Align2::CENTER_TOP,
                PARTIAL_LABELS[i],
                pt.font_small.clone(),
                pt.text_dim,
            );
        }

        if let Some(pos) = response.hover_pos() {
            let i = (((pos.x - rect.left()) / bar_w) as usize).min(N - 1);
            // Highlight the hovered bar.
            let x0 = rect.left() + i as f32 * bar_w;
            let x1 = rect.left() + (i + 1) as f32 * bar_w;
            painter.rect_stroke(
                Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom())),
                1.0,
                Stroke::new(0.5, pt.crosshair),
                egui::StrokeKind::Inside,
            );
            let partial_hz = r.fundamental_hz * (i + 1) as f32;
            let db = partials[i];
            let tip = if db <= -79.0 {
                format!("{}× {} (no peak)", i + 1, format_freq_full(partial_hz))
            } else {
                format!("{}× {} · {db:.1} dB", i + 1, format_freq_full(partial_hz))
            };
            response.on_hover_text(tip);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(format!("THD {:.1} dB", r.harmonic_content.thd_db));
            ui.separator();
            ui.label(format!(
                "odd/even {:+.1} dB",
                r.harmonic_content.odd_even_ratio_db
            ));
            ui.separator();
            ui.label(format!("n {}", r.harmonic_content.n_harmonics));
            ui.separator();
            ui.label(format!("f₀ {:.1} Hz", r.fundamental_hz));
        });
    }

    fn render_stereo_pane(&mut self, ui: &mut egui::Ui, plot_h: f32, pt: &PaneTheme) {
        ui.horizontal(|ui| {
            pane_title_with_info(ui, "Stereo", INFO_STEREO);
        });

        let (rect, _response) = allocate_pane_rect(ui, plot_h);
        let painter = ui.painter_at(rect);
        paint_pane_background(&painter, rect, pt);

        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;

        // Split: square goniometer on the left, meters/readouts on the right.
        let goni_size = plot_h.min(rect.width() * 0.45);
        let goni_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 4.0, rect.top() + 4.0),
            Vec2::splat(goni_size - 8.0),
        );
        let info_rect = Rect::from_min_max(
            Pos2::new(goni_rect.right() + 8.0, rect.top() + 4.0),
            Pos2::new(rect.right() - 4.0, rect.bottom() - 4.0),
        );

        draw_goniometer(&painter, goni_rect, &snap.rendered, pt);
        draw_stereo_meters(&painter, info_rect, r, pt);

        ui.horizontal_wrapped(|ui| {
            if let Some(width) = r.stereo_width {
                ui.label(format!("width {:.0}%", width * 100.0));
                ui.separator();
            }
            ui.label(format!("corr {:+.2}", r.stereo_correlation));
            if let (Some(m), Some(s)) = (r.mid_rms, r.side_rms) {
                ui.separator();
                ui.label(format!("M {m:.4}"));
                ui.label(format!("S {s:.4}"));
            }
        });
    }

    fn render_bottom_strip(&mut self, ui: &mut egui::Ui) {
        let Some(snap) = self.current.as_ref() else {
            return;
        };
        let r = &snap.result;

        ui.horizontal_wrapped(|ui| {
            // Centroid trend — reads positive when the filter is opening
            // over the note, negative when closing.
            let trend = r.centroid_trend_hz_per_sec;
            let trend_label = if trend.abs() < 1.0 {
                "centroid stable".to_string()
            } else {
                format!("centroid {trend:+.0} Hz/s")
            };
            ui.label(trend_label).on_hover_text(
                "Linear-regression slope of centroid_envelope over the held-note region.\n\
                 Positive = filter opening, negative = closing.",
            );

            ui.separator();

            if let Some(trimmed) = r.trimmed_tail_windows {
                ui.label(format!("trimmed {trimmed} tail windows"))
                    .on_hover_text(
                        "Number of trailing centroid_envelope windows trimmed because\n\
                         they decayed to noise. The raw rms_envelope is not trimmed.",
                    );
                ui.separator();
            }

            // Warnings strip — collapsible, hidden when no warnings.
            if !r.warnings.is_empty() {
                let title = format!(
                    "{} {} warning{}",
                    ri::ERROR_WARNING_LINE,
                    r.warnings.len(),
                    if r.warnings.len() == 1 { "" } else { "s" }
                );
                ui.collapsing(title, |ui| {
                    for w in &r.warnings {
                        ui.label(w);
                    }
                });
            }
        });

        // Debug accordion (kept for now, not promised in the v1 plan but
        // useful while iterating). Collapsed by default; hidden inside a
        // muted DebuggingDetails-style entry so it doesn't fight the chrome.
        ui.collapsing(RichText::new("Raw analysis (debug)").small().weak(), |ui| {
            ui.label(format!(
                "note_played: {} ({})",
                MidiNote::new(r.note_played).name(),
                r.note_played
            ));
            ui.label(format!("sample_rate: {} Hz", r.sample_rate));
            ui.label(format!("duration: {:.3} s", r.duration_seconds));
            ui.label(format!("fundamental: {:.2} Hz", r.fundamental_hz));
            ui.label(format!(
                "expected fundamental: {:.2} Hz",
                r.expected_fundamental_hz
            ));
            ui.label(format!("clipped samples: {}", r.clipped_samples));
            ui.label(format!("signal mode: {:?}", r.analysis_signal_mode));
        });
    }
}

impl Default for AnalyzeWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum ChipKind {
    Good,
    Warn,
    Bad,
}

fn overall_status(flags: &AnalyzeFlags) -> ChipKind {
    if flags.silent || flags.clipping || flags.off_pitch {
        ChipKind::Bad
    } else if flags.has_dc_offset || flags.low_output {
        ChipKind::Warn
    } else {
        ChipKind::Good
    }
}

fn chip(ui: &mut egui::Ui, text: &str, kind: ChipKind, pt: &PaneTheme) {
    let bg = match kind {
        ChipKind::Good => pt.chip_good,
        ChipKind::Warn => pt.chip_warn,
        ChipKind::Bad => pt.chip_bad,
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(pt.chip_fg).strong().small());
        });
}

fn wav_default_name(target_name: &str, note: u8) -> String {
    let safe: String = target_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let stem = if safe.is_empty() { "analyze" } else { &safe };
    let note_name = MidiNote::new(note).name();
    format!("{stem}_{note_name}.wav")
}

/// Per-metric delta semantics. Magnitudes use percent against the pinned
/// baseline; signed quantities show absolute deltas because percent flips
/// sign when the baseline crosses zero.
#[derive(Clone, Copy)]
enum DeltaKind {
    /// Percent change. Falls back to absolute when baseline is ~0 (avoids
    /// division by zero / nonsensical percents on near-zero RMS).
    Magnitude,
    /// Absolute delta with 2 decimals (e.g. stereo correlation).
    Signed2,
    /// Absolute delta with 4 decimals (e.g. DC offset).
    Signed4,
    /// Cents delta with 1 decimal (pitch_error_cents).
    Cents,
    /// Integer delta (counts like clipped_samples).
    Integer,
}

fn format_delta(curr: f32, pin: f32, kind: DeltaKind) -> String {
    let d = curr - pin;
    match kind {
        DeltaKind::Magnitude => {
            if pin.abs() < 1e-9 {
                format!("Δ {d:+.4}")
            } else {
                format!("{:+.1}%", d / pin.abs() * 100.0)
            }
        }
        DeltaKind::Signed2 => format!("Δ {d:+.2}"),
        DeltaKind::Signed4 => format!("Δ {d:+.4}"),
        DeltaKind::Cents => format!("Δ {d:+.1}¢"),
        #[allow(clippy::cast_possible_truncation)]
        DeltaKind::Integer => format!("Δ {:+}", d as i64),
    }
}

fn metric(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    delta: Option<(f32, f32, DeltaKind)>,
    pt: &PaneTheme,
) {
    ui.label(format!("{label} {value}"));
    if let Some((curr, pin, kind)) = delta {
        let txt = format_delta(curr, pin, kind);
        ui.label(RichText::new(txt).small().color(pt.delta_label));
    }
}

/// Pre-built labels for partials 1×–10× — referenced under each harmonics
/// bar every frame; static so the labels don't allocate per repaint.
const PARTIAL_LABELS: [&str; 10] = ["1×", "2×", "3×", "4×", "5×", "6×", "7×", "8×", "9×", "10×"];

// === Plot palette & helpers ===

/// Snapshot of the colors and fonts used by the analyze panes, drawn from
/// the global theme. Built once per render pass to avoid per-call lock
/// contention and to keep individual draw helpers themable from one place.
#[derive(Clone)]
struct PaneTheme {
    /// Current snapshot trace.
    current: Color32,
    /// Pinned (A reference) trace.
    pin: Color32,
    /// Vertical note-off marker on the waveform.
    note_off: Color32,
    /// Reference line for `expected_fundamental_hz`.
    expected: Color32,
    /// Axes / gridlines / dim labels drawn on plots.
    axis: Color32,
    /// ADSR markers on the envelope plot.
    adsr: Color32,
    /// Spectrum region colours.
    spec_attack: Color32,
    spec_sustain: Color32,
    spec_release: Color32,
    /// Harmonics bar fill.
    harmonic: Color32,
    /// Goniometer dots.
    gonio: Color32,
    /// Meter row background and fills.
    meter_bg: Color32,
    meter_fill: Color32,
    meter_fill_side: Color32,
    /// Hover crosshair (semi-transparent).
    crosshair: Color32,
    /// Background of every plot rect.
    pane_bg: Color32,
    /// Border of every plot rect.
    pane_border: Color32,
    /// Bright text drawn on plots (axis labels etc.).
    text_dim: Color32,
    /// Font for axis labels and tooltips inside plots.
    font_small: egui::FontId,
    /// Status-chip colours (Good/Warn/Bad backgrounds and shared fg).
    chip_good: Color32,
    chip_warn: Color32,
    chip_bad: Color32,
    chip_fg: Color32,
    /// Color used for A/B delta labels next to status-strip metrics.
    delta_label: Color32,
}

impl PaneTheme {
    fn from_global() -> Self {
        let t = theme();
        let c = &t.colors;
        let secondary = c.text_secondary;
        Self {
            current: c.accent_cyan,
            pin: c.accent_orange,
            note_off: c.accent_yellow,
            expected: c.accent_green,
            axis: c.text_dim,
            adsr: c.accent_purple,
            spec_attack: c.accent_yellow,
            spec_sustain: c.accent_cyan,
            spec_release: c.accent_purple,
            harmonic: c.accent_cyan,
            gonio: c.accent_cyan,
            meter_bg: c.bg_dark,
            meter_fill: c.accent_cyan,
            meter_fill_side: c.accent_orange,
            crosshair: Color32::from_rgba_premultiplied(
                secondary.r(),
                secondary.g(),
                secondary.b(),
                180,
            ),
            pane_bg: c.bg_dark,
            pane_border: c.border,
            text_dim: c.text_dim,
            font_small: t.fonts.small(),
            chip_good: c.accent_green.linear_multiply(0.35),
            chip_warn: c.accent_orange.linear_multiply(0.35),
            chip_bad: c.accent_red.linear_multiply(0.35),
            chip_fg: c.text_primary,
            delta_label: c.text_secondary,
        }
    }
}

fn allocate_pane_rect(ui: &mut egui::Ui, height: f32) -> (Rect, egui::Response) {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover())
}

/// Draw a vertical crosshair at the hover position, clipped to the pane.
fn draw_crosshair(painter: &egui::Painter, rect: Rect, x: f32, color: Color32) {
    if x < rect.left() || x > rect.right() {
        return;
    }
    painter.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        Stroke::new(0.5, color),
    );
}

fn paint_pane_background(painter: &egui::Painter, rect: Rect, pt: &PaneTheme) {
    painter.rect_filled(rect, 2.0, pt.pane_bg);
    // Center / zero line.
    let mid = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)],
        Stroke::new(0.5, pt.axis),
    );
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, pt.pane_border),
        egui::StrokeKind::Inside,
    );
}

fn max_or(values: &[f32], floor: f32) -> f32 {
    values
        .iter()
        .copied()
        .fold(floor, |acc, v| if v > acc { v } else { acc })
}

/// Sample the chosen channel mode at frame `i`. Returns the value in [-1, 1].
fn waveform_sample(
    samples: &[f32],
    channels: u16,
    mode: WaveformMode,
    i: usize,
) -> (f32, Option<f32>) {
    let ch = channels.max(1) as usize;
    let base = i * ch;
    if base + ch > samples.len() {
        return (0.0, None);
    }
    let l = samples[base];
    let r = if ch >= 2 { samples[base + 1] } else { l };
    match mode {
        WaveformMode::Stereo => (l, Some(r)),
        WaveformMode::Left => (l, None),
        WaveformMode::Right => (r, None),
        WaveformMode::Sum => ((l + r) * 0.5, None),
        WaveformMode::MidSide => (((l + r) * 0.5), Some((l - r) * 0.5)),
    }
}

/// Draw the waveform into `rect`, decimated to one min/max bar per pixel
/// column. In Stereo / M-S mode the rect is split into two stacked sub-rects.
fn draw_waveform(
    painter: &egui::Painter,
    rect: Rect,
    rendered: &RenderedNote,
    mode: WaveformMode,
    color: Color32,
    is_pinned: bool,
) {
    let channels = rendered.channels.max(1);
    let total_frames = rendered.samples.len() / channels as usize;
    if total_frames == 0 || rect.width() < 2.0 {
        return;
    }

    let split = matches!(mode, WaveformMode::Stereo | WaveformMode::MidSide);
    let (top_rect, bot_rect) = if split {
        let h = rect.height() * 0.5;
        (
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), h)),
            Rect::from_min_size(
                Pos2::new(rect.left(), rect.top() + h),
                Vec2::new(rect.width(), h),
            ),
        )
    } else {
        (rect, rect)
    };

    let buckets = rect.width().floor() as usize;
    if buckets == 0 {
        return;
    }
    let step = (total_frames as f32 / buckets as f32).max(1.0);

    // Pinned overlay uses a thinner, semi-transparent stroke so the current
    // overlays clearly on top.
    let stroke = if is_pinned {
        Stroke::new(1.0, color.linear_multiply(0.7))
    } else {
        Stroke::new(1.0, color)
    };

    for px in 0..buckets {
        let from = (px as f32 * step) as usize;
        let to = ((px as f32 + 1.0) * step) as usize;
        let to = to.min(total_frames);
        if from >= to {
            continue;
        }
        let (mut min_a, mut max_a) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_b, mut max_b) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut has_b = false;
        for i in from..to {
            let (a, b) = waveform_sample(&rendered.samples, channels, mode, i);
            if a < min_a {
                min_a = a;
            }
            if a > max_a {
                max_a = a;
            }
            if let Some(bv) = b {
                has_b = true;
                if bv < min_b {
                    min_b = bv;
                }
                if bv > max_b {
                    max_b = bv;
                }
            }
        }
        let x = top_rect.left() + px as f32 + 0.5;
        let to_y = |sub: Rect, v: f32| sub.center().y - v.clamp(-1.0, 1.0) * sub.height() * 0.5;
        if min_a.is_finite() && max_a.is_finite() {
            painter.line_segment(
                [
                    Pos2::new(x, to_y(top_rect, min_a)),
                    Pos2::new(x, to_y(top_rect, max_a)),
                ],
                stroke,
            );
        }
        if has_b && min_b.is_finite() && max_b.is_finite() {
            painter.line_segment(
                [
                    Pos2::new(x, to_y(bot_rect, min_b)),
                    Pos2::new(x, to_y(bot_rect, max_b)),
                ],
                stroke,
            );
        }
    }
}

/// Plot a non-negative series mapped to [0, peak] across the rect. Negative
/// or NaN entries are clamped to zero. Used for `rms_envelope`.
fn draw_polyline_normalized(
    painter: &egui::Painter,
    rect: Rect,
    values: &[f32],
    peak: f32,
    color: Color32,
) {
    if values.len() < 2 || peak <= 0.0 {
        return;
    }
    let dx = rect.width() / (values.len() - 1).max(1) as f32;
    let mut points = Vec::with_capacity(values.len());
    for (i, v) in values.iter().enumerate() {
        let x = rect.left() + i as f32 * dx;
        let v_norm = (v.max(0.0) / peak).clamp(0.0, 1.0);
        let y = rect.bottom() - v_norm * rect.height();
        points.push(Pos2::new(x, y));
    }
    painter.add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

fn pitch_to_y(rect: Rect, hz: f32, min_hz: f32, max_hz: f32) -> f32 {
    let span = (max_hz - min_hz).max(1e-6);
    let t = ((hz - min_hz) / span).clamp(0.0, 1.0);
    rect.bottom() - t * rect.height()
}

/// Pitch polyline. Skips zero entries (silent windows) so dropouts don't
/// pull the line down to the rect floor.
fn draw_pitch_polyline(
    painter: &egui::Painter,
    rect: Rect,
    values: &[f32],
    min_hz: f32,
    max_hz: f32,
    color: Color32,
) {
    if values.len() < 2 {
        return;
    }
    let dx = rect.width() / (values.len() - 1).max(1) as f32;
    let mut segment: Vec<Pos2> = Vec::new();
    for (i, hz) in values.iter().copied().enumerate() {
        if hz > 0.0 {
            let x = rect.left() + i as f32 * dx;
            segment.push(Pos2::new(x, pitch_to_y(rect, hz, min_hz, max_hz)));
        } else if !segment.is_empty() {
            painter.add(egui::Shape::line(segment.clone(), Stroke::new(1.5, color)));
            segment.clear();
        }
    }
    if segment.len() >= 2 {
        painter.add(egui::Shape::line(segment, Stroke::new(1.5, color)));
    }
}

fn format_freq_full(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.2} kHz", hz / 1_000.0)
    } else {
        format!("{hz:.1} Hz")
    }
}

fn format_time(secs: f32) -> String {
    if secs >= 1.0 {
        format!("{secs:.3} s")
    } else {
        format!("{:.1} ms", secs * 1000.0)
    }
}

fn waveform_tooltip(rect: Rect, pos: Pos2, rendered: &RenderedNote) -> String {
    let channels = rendered.channels.max(1) as usize;
    let total_frames = rendered.samples.len() / channels;
    if total_frames == 0 || rect.width() < 1.0 {
        return String::new();
    }
    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let frame = (t * total_frames as f32) as usize;
    let secs = frame as f32 / rendered.sample_rate.max(1) as f32;
    let mut tip = format!("t = {}", format_time(secs));
    let base = (frame * channels).min(rendered.samples.len().saturating_sub(channels));
    if channels >= 2 && base + 1 < rendered.samples.len() {
        let l = rendered.samples[base];
        let r = rendered.samples[base + 1];
        tip.push_str(&format!("\nL = {l:+.4}\nR = {r:+.4}"));
    } else if !rendered.samples.is_empty() {
        tip.push_str(&format!("\n= {:+.4}", rendered.samples[base]));
    }
    if frame == rendered.note_off_frame as usize {
        tip.push_str("\n(note-off)");
    }
    tip
}

fn envelope_tooltip(rect: Rect, pos: Pos2, r: &AnalyzeNoteResult) -> String {
    if r.rms_envelope.is_empty() || rect.width() < 1.0 {
        return String::new();
    }
    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let i = ((t * r.rms_envelope.len() as f32) as usize).min(r.rms_envelope.len() - 1);
    let secs = i as f32 * r.envelope_window_ms / 1000.0;
    let v = r.rms_envelope[i];
    format!("t = {}\nrms = {:.4}", format_time(secs), v)
}

fn pitch_tooltip(rect: Rect, pos: Pos2, r: &AnalyzeNoteResult) -> String {
    if r.pitch_envelope.is_empty() || rect.width() < 1.0 {
        return String::new();
    }
    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let i = ((t * r.pitch_envelope.len() as f32) as usize).min(r.pitch_envelope.len() - 1);
    let secs = i as f32 * r.pitch_envelope_window_ms / 1000.0;
    let hz = r.pitch_envelope[i];
    if hz <= 0.0 {
        format!("t = {}\n(silent)", format_time(secs))
    } else if r.expected_fundamental_hz > 0.0 {
        let cents = synth_core::types::Hertz::new(r.expected_fundamental_hz)
            .cents_between(synth_core::types::Hertz::new(hz));
        format!(
            "t = {}\n{} ({:+.1}¢)",
            format_time(secs),
            format_freq_full(hz),
            cents.as_f32(),
        )
    } else {
        format!("t = {}\n{}", format_time(secs), format_freq_full(hz))
    }
}

fn spectrum_tooltip(rect: Rect, pos: Pos2, r: &AnalyzeNoteResult, axes: &SpecAxes) -> String {
    if rect.width() < 1.0 {
        return String::new();
    }
    let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let log_hz = axes.log_min + t * (axes.log_max - axes.log_min);
    let hz = 10f32.powf(log_hz);
    let mut tip = format!("@ {}", format_freq_full(hz));
    let tol_hz = (hz * 0.05).max(2.0);
    if let Some(db) = nearest_peak_db(&r.spectrum_attack, hz, tol_hz) {
        tip.push_str(&format!("\nattack:  {db:+.1} dB"));
    }
    if let Some(db) = nearest_peak_db(&r.spectrum_sustain, hz, tol_hz) {
        tip.push_str(&format!("\nsustain: {db:+.1} dB"));
    }
    if let Some(db) = nearest_peak_db(&r.spectrum_release, hz, tol_hz) {
        tip.push_str(&format!("\nrelease: {db:+.1} dB"));
    }
    tip
}

fn legend_swatch(ui: &mut egui::Ui, color: Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
    ui.painter().rect_filled(rect, 1.0, color);
    ui.label(RichText::new(label).small());
}

// === Pane info-popup texts ===

const INFO_WAVEFORM: &str = "\
Raw rendered audio over time.

• Each pixel column shows the min/max sample range for that time slice — \
transients survive the decimation.
• L/R and M-S split the rect into top + bottom halves; Sum and the \
single-channel modes use the full rect.
• The yellow vertical line marks the note-off frame.
• Hover for exact timestamp and L/R values.";

const INFO_ENVELOPE: &str = "\
RMS amplitude per non-overlapping window — the patch's loudness contour \
over time.

ADSR markers (purple verticals) overlay the envelope estimate derived \
from the RMS curve:
• A — time of the RMS peak
• D — when the signal has decayed to ~120 % of sustain level
• off — note-off
• R — when RMS falls below 5 % of peak after note-off

Footer shows the numeric ADSR estimate and the RMS analysis window.";

const INFO_PITCH: &str = "\
Detected fundamental frequency per analysis window.

• The green horizontal line is the expected fundamental for the played \
note.
• Gaps in the trace are silent windows where pitch detection failed.
• Hover shows the cents offset from the expected fundamental.
• For stereo input, pitch is detected on a synthetic max(|L|,|R|) signal \
so anti-phase content still gets a fundamental — per-channel L and R \
fundamentals are shown in the footer with confidence percentages.";

const INFO_SPECTRUM: &str = "\
Top spectrum peaks at three time regions of the note:
• attack (yellow) — early region (~50 ms in)
• sustain (cyan) — sustained mid-region
• release (purple) — release tail

Y axis: dB relative to the loudest peak in each window. \
X axis: log-frequency from 20 Hz to Nyquist. The mode badge in the \
title indicates whether analysis ran on the mono channel directly or on \
a synthetic max(|L|,|R|) stereo signal.

Hover near a peak to see its dB across all three regions.";

const INFO_HARMONICS: &str = "\
Magnitude of partials 1× through 10× the detected fundamental, read from \
the sustain spectrum.

Each bar's height is the dB of the nearest spectrum peak within ±5 % of \
the partial frequency. A bar pinned to the floor means the partial is \
absent.

Footer:
• THD — total harmonic distortion (more negative = cleaner).
• odd/even — power ratio of odd vs even harmonics. Positive = \
odd-dominant (square / clean clipping), negative = even-dominant \
(asymmetric clipping, tube saturation).
• n — count of harmonics above the noise floor.
• f₀ — detected fundamental.";

const INFO_STEREO: &str = "\
Stereo image of the rendered note.

Goniometer (left): each frame plotted with L and R rotated 45°. A \
vertical streak means mono content (L = R). A horizontal streak means \
anti-phase content (L = −R). Decimated to ≤4 k points so transients \
survive.

Meters (right): peak L, peak R, mid RMS, side RMS. Mid (M) = (L+R)/2 — \
energy in the center. Side (S) = (L−R)/2 — energy in the stereo \
difference.

Footer:
• width = side / (mid + side). 0 = mono, ~0.5 = typical stereo, 1 = \
anti-phase / fully decorrelated.
• corr — Pearson correlation between L and R (-1 … +1). +1 = mono routed \
to both sides, lower = true stereo.";

/// Render a pane title with an inline ⓘ info button. Clicking the button
/// toggles a popup with `description`. Caller adds any further chrome
/// (mode toggles, badges, legends) after this in the same `ui.horizontal`.
fn pane_title_with_info(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).strong());
    let info_btn = ui.add(egui::Button::new(ri::INFORMATION_LINE).small());
    egui::Popup::from_toggle_button_response(&info_btn).show(|ui| {
        ui.set_max_width(360.0);
        ui.label(description);
    });
}

fn freq_to_x(rect: Rect, hz: f32, log_min: f32, log_max: f32) -> f32 {
    let log_hz = hz.max(1.0).log10();
    let span = (log_max - log_min).max(1e-6);
    let t = ((log_hz - log_min) / span).clamp(0.0, 1.0);
    rect.left() + t * rect.width()
}

fn db_to_y(rect: Rect, db: f32, db_min: f32, db_max: f32) -> f32 {
    let span = (db_max - db_min).max(1e-6);
    let t = ((db - db_min) / span).clamp(0.0, 1.0);
    rect.bottom() - t * rect.height()
}

fn format_freq(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.0}k", hz / 1_000.0)
    } else {
        format!("{hz:.0}")
    }
}

/// Axis ranges shared by spectrum + harmonics plots.
struct SpecAxes {
    log_min: f32,
    log_max: f32,
    db_min: f32,
    db_max: f32,
}

fn draw_spectrum_stems(
    painter: &egui::Painter,
    rect: Rect,
    peaks: &[AnalyzeSpectrumPeak],
    axes: &SpecAxes,
    color: Color32,
) {
    let stroke = Stroke::new(1.0, color);
    for peak in peaks {
        if peak.freq_hz <= 0.0 {
            continue;
        }
        let x = freq_to_x(rect, peak.freq_hz, axes.log_min, axes.log_max);
        let y = db_to_y(
            rect,
            peak.magnitude_db.clamp(axes.db_min, axes.db_max),
            axes.db_min,
            axes.db_max,
        );
        painter.line_segment([Pos2::new(x, rect.bottom()), Pos2::new(x, y)], stroke);
        painter.circle_filled(Pos2::new(x, y), 2.0, color);
    }
}

fn nearest_peak_db(peaks: &[AnalyzeSpectrumPeak], target_hz: f32, tol_hz: f32) -> Option<f32> {
    let mut best: Option<(f32, f32)> = None; // (|delta|, db)
    for p in peaks {
        if p.freq_hz <= 0.0 {
            continue;
        }
        let delta = (p.freq_hz - target_hz).abs();
        if delta > tol_hz {
            continue;
        }
        match best {
            Some((bd, _)) if bd <= delta => {}
            _ => best = Some((delta, p.magnitude_db)),
        }
    }
    best.map(|(_, db)| db)
}

/// Goniometer scatter — L on +45° axis, R on -45°. Mono = vertical streak;
/// anti-phase = horizontal streak. Decimates the buffer to ≤4 k frames so
/// the plot stays cheap to render. Plots scaled into a square inscribed in
/// `rect`; values |L|, |R| up to 1 reach the diamond's corners.
fn draw_goniometer(painter: &egui::Painter, rect: Rect, rendered: &RenderedNote, pt: &PaneTheme) {
    // Border (square frame + diagonal guides).
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, pt.axis),
        egui::StrokeKind::Inside,
    );
    let center = rect.center();
    let half = rect.height().min(rect.width()) * 0.5;
    // Diamond outline (the L/R axes when rotated 45°).
    let diamond_stroke = Stroke::new(0.5, pt.axis);
    painter.line_segment(
        [
            Pos2::new(center.x, center.y - half),
            Pos2::new(center.x + half, center.y),
        ],
        diamond_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x + half, center.y),
            Pos2::new(center.x, center.y + half),
        ],
        diamond_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x, center.y + half),
            Pos2::new(center.x - half, center.y),
        ],
        diamond_stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x - half, center.y),
            Pos2::new(center.x, center.y - half),
        ],
        diamond_stroke,
    );

    let channels = rendered.channels.max(1) as usize;
    if channels < 2 {
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            "mono",
            pt.font_small.clone(),
            pt.text_dim,
        );
        return;
    }
    let total_frames = rendered.samples.len() / channels;
    if total_frames == 0 {
        return;
    }

    // Decimate: pick at most ~4000 sample points, evenly stepping through
    // the buffer. For 110 k frames that's a ~28× decimation — fine for a
    // visual scatter and well under the plan's 4 k budget.
    const TARGET_POINTS: usize = 4_000;
    let step = total_frames.div_ceil(TARGET_POINTS).max(1);
    let scale = half * std::f32::consts::FRAC_1_SQRT_2;
    let dot_color = pt.gonio.linear_multiply(0.5);

    for i in (0..total_frames).step_by(step) {
        let base = i * channels;
        if base + 1 >= rendered.samples.len() {
            break;
        }
        let l = rendered.samples[base].clamp(-1.0, 1.0);
        let r = rendered.samples[base + 1].clamp(-1.0, 1.0);
        // Rotate 45° so the L axis points up-right and R points up-left.
        let x = (l - r) * scale;
        let y = -(l + r) * scale; // negate — egui Y grows downward.
        painter.circle_filled(Pos2::new(center.x + x, center.y + y), 1.0, dot_color);
    }
}

fn draw_stereo_meters(painter: &egui::Painter, rect: Rect, r: &AnalyzeNoteResult, pt: &PaneTheme) {
    let row_h = (rect.height() / 4.0).clamp(8.0, 22.0);
    let label_w = 16.0;
    let value_w = 56.0;
    let bar_x0 = rect.left() + label_w + 4.0;
    let bar_x1 = rect.right() - value_w - 4.0;

    let rows: [(&str, Option<f32>, bool); 4] = [
        ("L", r.peak_left, false),
        ("R", r.peak_right, false),
        ("M", r.mid_rms, false),
        ("S", r.side_rms, true),
    ];

    for (i, (label, value, is_side)) in rows.into_iter().enumerate() {
        let row_top = rect.top() + i as f32 * row_h;
        let row_rect = Rect::from_min_max(
            Pos2::new(rect.left(), row_top),
            Pos2::new(rect.right(), row_top + row_h),
        );
        painter.text(
            Pos2::new(rect.left(), row_rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            pt.font_small.clone(),
            pt.text_dim,
        );
        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x0, row_rect.center().y - row_h * 0.3),
            Pos2::new(bar_x1, row_rect.center().y + row_h * 0.3),
        );
        painter.rect_filled(bar_rect, 1.0, pt.meter_bg);
        if let Some(v) = value {
            let t = v.clamp(0.0, 1.0);
            let fill_rect = Rect::from_min_max(
                bar_rect.min,
                Pos2::new(bar_rect.left() + bar_rect.width() * t, bar_rect.bottom()),
            );
            let color = if is_side {
                pt.meter_fill_side
            } else {
                pt.meter_fill
            };
            painter.rect_filled(fill_rect, 1.0, color);
            painter.text(
                Pos2::new(rect.right(), row_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{v:.4}"),
                pt.font_small.clone(),
                pt.text_dim,
            );
        } else {
            painter.text(
                Pos2::new(rect.right(), row_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "—",
                pt.font_small.clone(),
                pt.text_dim,
            );
        }
    }
}

/// Draw the four ADSR markers on top of the envelope plot:
///   • attack peak (vertical line at attack_ms)
///   • decay end (vertical line at attack_ms + decay_ms)
///   • note-off (vertical line at the actual note-off time)
///   • release end (vertical line at note-off + release_ms)
/// All marker times are anchored against the total render duration so they
/// stay aligned with the rms_envelope polyline.
fn draw_adsr_markers(painter: &egui::Painter, rect: Rect, snap: &AnalysisSnapshot, pt: &PaneTheme) {
    let r = &snap.result;
    let total_secs = r.duration_seconds.max(1e-6);
    let attack_t = (r.envelope_estimate.attack_ms / 1000.0) / total_secs;
    let decay_end_t =
        ((r.envelope_estimate.attack_ms + r.envelope_estimate.decay_ms) / 1000.0) / total_secs;
    let note_off_t = (snap.rendered.note_off_frame as f32
        / snap.rendered.sample_rate.max(1) as f32)
        / total_secs;
    let release_end_t = note_off_t + (r.envelope_estimate.release_ms / 1000.0) / total_secs;

    for (t, label) in [
        (attack_t, "A"),
        (decay_end_t, "D"),
        (note_off_t, "off"),
        (release_end_t, "R"),
    ] {
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let x = rect.left() + t * rect.width();
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(0.75, pt.adsr),
        );
        painter.text(
            Pos2::new(x + 2.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            label,
            pt.font_small.clone(),
            pt.adsr,
        );
    }
}
