//! Export WAV dialog for offline audio rendering.
//!
//! Shows sample rate, bit depth, duration, and tail time options.
//! Displays a progress bar while rendering runs in a background thread.

use std::path::PathBuf;

use egui::{Align, Layout, ProgressBar, RichText};

use crate::audio::export::{ExportConfig, ExportProgress, SharedSampleLibrary, start_export};
use crate::audio::wav_format::WavFormat;
use crate::gui::theme::theme;
use crate::gui::widgets::time_drag_value;
use crate::project::ProjectFile;

/// Persistent state for the export dialog.
pub struct ExportDialogState {
    /// Selected sample rate.
    pub sample_rate_index: usize,
    /// Selected sample format.
    pub format: WavFormat,
    /// Duration in seconds to render.
    pub duration_seconds: f64,
    /// Extra tail time for reverb/delay.
    pub tail_seconds: f32,
    /// Chosen export path (from file dialog).
    pub export_path: Option<PathBuf>,
    /// Active export progress (while rendering).
    pub progress: Option<ExportProgress>,
    /// Flag: the Export button was clicked, caller should provide project and call `begin_export`.
    pub wants_export: bool,
}

/// Available sample rates for export.
const SAMPLE_RATES: [u32; 3] = [44100, 48000, 96000];

/// Labels for sample rate dropdown.
const SAMPLE_RATE_LABELS: [&str; 3] = ["44100 Hz", "48000 Hz", "96000 Hz"];

impl Default for ExportDialogState {
    fn default() -> Self {
        Self {
            sample_rate_index: 1, // 48000 Hz
            format: WavFormat::Int24,
            duration_seconds: 60.0,
            tail_seconds: 2.0,
            export_path: None,
            progress: None,
            wants_export: false,
        }
    }
}

impl ExportDialogState {
    /// Set the duration from the song length (in seconds).
    pub fn set_duration_from_song(&mut self, song_seconds: f64) {
        if song_seconds > 0.0 {
            self.duration_seconds = song_seconds;
        }
    }

    /// Start the export with the given project snapshot.
    ///
    /// `sample_library` carries the audio buffers Sampler modules reference by
    /// id (the `ProjectFile` stores only the ids), so the export renders
    /// samplers instead of silence.
    ///
    /// Call this after checking `wants_export` is true.
    pub fn begin_export(&mut self, project: ProjectFile, sample_library: SharedSampleLibrary) {
        self.wants_export = false;
        if let Some(ref path) = self.export_path {
            let sample_rate = SAMPLE_RATES[self.sample_rate_index];
            let config = ExportConfig {
                path: path.clone(),
                sample_rate,
                format: self.format,
                duration_seconds: self.duration_seconds,
                tail_seconds: self.tail_seconds,
            };
            self.progress = Some(start_export(project, sample_library, config));
        }
    }
}

/// Result of showing the export dialog.
pub enum ExportDialogResult {
    /// Nothing happened.
    None,
    /// Export completed successfully — status message to display.
    Completed(String),
    /// Export failed — error message to display.
    Failed(String),
    /// Dialog was closed.
    Closed,
}

/// Show the export WAV dialog window.
pub fn show_export_dialog(
    ctx: &egui::Context,
    state: &mut ExportDialogState,
    open: &mut bool,
) -> ExportDialogResult {
    let mut result = ExportDialogResult::None;

    egui::Window::new("Export WAV")
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_width(380.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // If we have an active export, show progress
            if let Some(ref progress) = state.progress {
                show_progress_ui(ui, progress);

                if progress.is_completed() {
                    if let Some(err) = progress.error_message() {
                        result = ExportDialogResult::Failed(err);
                    } else if let Some(ref path) = state.export_path {
                        result = ExportDialogResult::Completed(format!(
                            "Exported WAV: {}",
                            path.display()
                        ));
                    }
                    state.progress = None;
                }
                return;
            }

            // --- Settings ---
            ui.heading("Export Settings");
            ui.add_space(theme().spacing.xs);

            egui::Grid::new("export_settings_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    // File path
                    ui.label("Output:");
                    ui.horizontal(|ui| {
                        let path_text = state
                            .export_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "(no file selected)".to_string());
                        ui.label(RichText::new(truncate_path(&path_text, 40)).monospace());
                    });
                    ui.end_row();

                    // Sample rate
                    ui.label("Sample Rate:");
                    egui::ComboBox::from_id_salt("export_sample_rate")
                        .selected_text(SAMPLE_RATE_LABELS[state.sample_rate_index])
                        .show_ui(ui, |ui| {
                            for (i, label) in SAMPLE_RATE_LABELS.iter().enumerate() {
                                ui.selectable_value(&mut state.sample_rate_index, i, *label);
                            }
                        });
                    ui.end_row();

                    // Bit depth
                    ui.label("Bit Depth:");
                    egui::ComboBox::from_id_salt("export_bit_depth")
                        .selected_text(state.format.label())
                        .show_ui(ui, |ui| {
                            for format in WavFormat::ALL {
                                ui.selectable_value(&mut state.format, format, format.label());
                            }
                        });
                    ui.end_row();

                    // Duration
                    ui.label("Duration:");
                    ui.horizontal(|ui| {
                        let mut secs = state.duration_seconds as f32;
                        time_drag_value(ui, &mut secs, 0.1..=3600.0, 0.5);
                        state.duration_seconds = f64::from(secs);
                    });
                    ui.end_row();

                    // Tail
                    ui.label("Tail:");
                    time_drag_value(ui, &mut state.tail_seconds, 0.0..=30.0, 0.1);
                    ui.end_row();
                });

            ui.add_space(theme().spacing.md);

            // Estimated file info
            let total_secs = state.duration_seconds + f64::from(state.tail_seconds);
            let sample_rate = SAMPLE_RATES[state.sample_rate_index];
            let total_frames = (total_secs * f64::from(sample_rate)) as u64;
            let bytes_per_sample = u64::from(state.format.bytes_per_sample());
            let estimated_size = total_frames * 2 * bytes_per_sample; // stereo
            ui.label(
                RichText::new(format!(
                    "Estimated size: {:.1} MB ({:.1} sec total)",
                    estimated_size as f64 / (1024.0 * 1024.0),
                    total_secs,
                ))
                .weak(),
            );

            ui.add_space(theme().spacing.md);
            ui.separator();
            ui.add_space(theme().spacing.xs);

            // Buttons
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                let can_export = state.export_path.is_some();

                if ui
                    .add_enabled(can_export, egui::Button::new("Export"))
                    .clicked()
                {
                    state.wants_export = true;
                }

                if ui.button("Cancel").clicked() {
                    result = ExportDialogResult::Closed;
                }
            });
        });

    result
}

/// Show progress bar and cancel button during export.
fn show_progress_ui(ui: &mut egui::Ui, progress: &ExportProgress) {
    ui.heading("Exporting...");
    ui.add_space(theme().spacing.md);

    let fraction = progress.fraction();
    ui.add(
        ProgressBar::new(fraction)
            .text(format!("{:.1}%", fraction * 100.0))
            .animate(true),
    );

    let rendered = progress
        .frames_rendered
        .load(std::sync::atomic::Ordering::Relaxed);
    let total = progress.total_frames;
    let rendered_secs = rendered as f64 / 48000.0; // approximate
    let total_secs = total as f64 / 48000.0;
    ui.label(
        RichText::new(format!(
            "{:.1} / {:.1} sec rendered",
            rendered_secs, total_secs,
        ))
        .weak(),
    );

    ui.add_space(theme().spacing.md);

    if ui.button("Cancel").clicked() {
        progress.cancel();
    }

    // Request repaint during export to keep progress updating
    ui.request_repaint();
}

/// Truncate a path string for display.
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}
