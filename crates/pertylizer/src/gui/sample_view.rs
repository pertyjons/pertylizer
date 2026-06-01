//! Sample view — browse, import, edit, and preview samples.
//!
//! Layout:
//! - Left sidebar: sample list with names and durations
//! - Center: waveform display with crop handles and loop markers
//! - Right/bottom: properties panel and toolbar

use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32};
use egui_remixicon::icons as ri;

use crate::audio::input::{AudioInputManager, InputState};
use crate::gui::theme::theme;
use synth_core::SampleCount;
use synth_sampler::{CropRegion, FrameIndex, LoopRegion, SampleId, SampleLibrary};

// ============================================================================
// PEAK CACHE
// ============================================================================

/// Downsampled peak data for efficient waveform rendering.
struct WaveformPeaks {
    /// (min, max) amplitude pairs per display column.
    peaks: Vec<(f32, f32)>,
    /// How many source frames each peak entry represents.
    frames_per_peak: usize,
    /// Sample ID this cache was built for.
    sample_id: SampleId,
    /// Number of channels (kept for future stereo display).
    _channels: usize,
}

/// Build peak cache from interleaved audio data (mono mixdown for display).
fn build_peaks(data: &[f32], channels: usize, frames_per_peak: usize) -> Vec<(f32, f32)> {
    if data.is_empty() || channels == 0 || frames_per_peak == 0 {
        return Vec::new();
    }
    let frame_count = data.len() / channels;
    let num_peaks = frame_count.div_ceil(frames_per_peak);
    let mut peaks = Vec::with_capacity(num_peaks);

    for p in 0..num_peaks {
        let start_frame = p * frames_per_peak;
        let end_frame = ((p + 1) * frames_per_peak).min(frame_count);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;

        for frame in start_frame..end_frame {
            // Mix channels to mono for display
            let mut sum = 0.0_f32;
            for ch in 0..channels {
                sum += data[frame * channels + ch];
            }
            let mono = sum / channels as f32;
            min_val = min_val.min(mono);
            max_val = max_val.max(mono);
        }

        if min_val > max_val {
            min_val = 0.0;
            max_val = 0.0;
        }
        peaks.push((min_val, max_val));
    }

    peaks
}

// ============================================================================
// VIEW STATE
// ============================================================================

/// State for the sample view, stored in `SynthApp`.
pub struct SampleViewState {
    /// Currently selected sample ID.
    pub selected_sample: Option<SampleId>,
    /// Horizontal zoom level (samples per pixel).
    zoom: f32,
    /// Horizontal scroll offset in frames.
    scroll_offset: f32,
    /// Cached waveform peaks.
    peak_cache: Option<WaveformPeaks>,
    /// Whether peak cache needs rebuilding.
    peaks_dirty: bool,
    /// Editing the sample name.
    editing_name: bool,
    /// Temporary name buffer for editing.
    name_buffer: String,
    /// Editing the sample description.
    editing_description: bool,
    /// Temporary description buffer for editing.
    description_buffer: String,
    /// Selected input device name.
    pub selected_input_device: Option<String>,
    /// Cached input device list (refreshed on demand, not every frame).
    pub cached_input_devices: Vec<synth_core::audio::DeviceInfo>,
    /// Whether the device list needs refreshing.
    pub devices_dirty: bool,
}

impl Default for SampleViewState {
    fn default() -> Self {
        Self {
            selected_sample: None,
            zoom: 1.0,
            scroll_offset: 0.0,
            peak_cache: None,
            peaks_dirty: true,
            editing_name: false,
            name_buffer: String::new(),
            editing_description: false,
            description_buffer: String::new(),
            selected_input_device: None,
            cached_input_devices: Vec::new(),
            devices_dirty: true,
        }
    }
}

impl SampleViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark peak cache as needing rebuild.
    pub fn invalidate_peaks(&mut self) {
        self.peaks_dirty = true;
    }
}

// ============================================================================
// VIEW ACTIONS
// ============================================================================

/// Actions that the sample view requests from the parent.
pub enum SampleViewAction {
    /// No action.
    None,
    /// Open file dialog to import a WAV file.
    ImportWav,
    /// Open file dialog to export the selected sample.
    ExportWav { name: String },
    /// Start monitoring input.
    StartMonitoring,
    /// Stop monitoring input.
    StopMonitoring,
    /// Start recording.
    StartRecording,
    /// Stop recording and create a sample.
    StopRecording,
}

// ============================================================================
// DRAW FUNCTION
// ============================================================================

/// Draw the complete sample view.
#[allow(clippy::too_many_lines)]
pub fn draw_sample_view(
    ui: &mut egui::Ui,
    library: &Arc<RwLock<SampleLibrary>>,
    state: &mut SampleViewState,
    audio_input: &mut AudioInputManager,
) -> SampleViewAction {
    let ctx = ui.ctx().clone();
    let mut action = SampleViewAction::None;
    let t = theme();

    // ---- Toolbar (top) ----
    egui::Panel::top("sample_toolbar").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            // Import
            if ui
                .button(
                    egui::RichText::new(format!("{} Import WAV", ri::FOLDER_OPEN_FILL))
                        .color(t.colors.text_primary),
                )
                .clicked()
            {
                action = SampleViewAction::ImportWav;
            }

            // Export
            let has_selection = state.selected_sample.is_some();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new(format!("{} Export WAV", ri::DOWNLOAD_2_FILL)).color(
                            if has_selection {
                                t.colors.text_primary
                            } else {
                                t.colors.text_dim
                            },
                        ),
                    ),
                )
                .clicked()
                && let Some(id) = state.selected_sample
                && let Ok(lib) = library.read()
                && let Some(meta) = lib.get_meta(id)
            {
                action = SampleViewAction::ExportWav {
                    name: meta.name.clone(),
                };
            }

            ui.separator();

            // Zoom controls
            if ui
                .button(egui::RichText::new(ri::ZOOM_IN_LINE).color(t.colors.text_secondary))
                .clicked()
            {
                state.zoom = (state.zoom * 0.5).max(0.01);
                state.peaks_dirty = true;
            }
            if ui
                .button(egui::RichText::new(ri::ZOOM_OUT_LINE).color(t.colors.text_secondary))
                .clicked()
            {
                state.zoom = (state.zoom * 2.0).min(1000.0);
                state.peaks_dirty = true;
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new("Fit All").color(t.colors.text_secondary),
                    ),
                )
                .clicked()
            {
                state.scroll_offset = 0.0;
                state.zoom = 1.0;
                state.peaks_dirty = true;
            }

            ui.separator();

            // Normalize
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new("Normalize").color(t.colors.text_secondary),
                    ),
                )
                .clicked()
                && let Some(id) = state.selected_sample
            {
                normalize_sample(library, id);
                state.peaks_dirty = true;
            }

            // Reverse
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new("Reverse").color(t.colors.text_secondary),
                    ),
                )
                .clicked()
                && let Some(id) = state.selected_sample
            {
                reverse_sample(library, id);
                state.peaks_dirty = true;
            }

            // Auto-trim
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new("Auto-trim").color(t.colors.text_secondary),
                    ),
                )
                .clicked()
                && let Some(id) = state.selected_sample
            {
                auto_trim_sample(library, id);
                state.peaks_dirty = true;
            }

            // Delete
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(
                        egui::RichText::new(format!("{} Delete", ri::DELETE_BIN_FILL))
                            .color(t.colors.meter_red),
                    ),
                )
                .clicked()
                && let Some(id) = state.selected_sample
            {
                if let Ok(mut lib) = library.write() {
                    lib.remove(id);
                }
                state.selected_sample = None;
                state.peak_cache = None;
                state.peaks_dirty = true;
            }
        });
    });

    // ---- Sample list (left sidebar) ----
    egui::Panel::left("sample_list_panel")
        .default_size(180.0)
        .min_size(140.0)
        .show_inside(ui, |ui| {
            ui.heading(
                egui::RichText::new(format!("{} Samples", ri::MUSIC_FILL))
                    .color(t.colors.text_primary),
            );
            ui.separator();

            // Sample list
            if let Ok(lib) = library.read() {
                let metas = lib.list();
                if metas.is_empty() {
                    ui.label(
                        egui::RichText::new("No samples loaded")
                            .color(t.colors.text_dim)
                            .italics(),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .content_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            for meta in &metas {
                                let is_selected = state.selected_sample == Some(meta.id);
                                let duration = meta.duration_seconds();

                                let label = format!(
                                    "{}\n{:.1}s  {}",
                                    meta.name,
                                    duration,
                                    match meta.channels {
                                        synth_core::ChannelCount::Mono => "Mono",
                                        synth_core::ChannelCount::Stereo => "Stereo",
                                        synth_core::ChannelCount::Multi(n) =>
                                        // borrow checker: can't return &str for format,
                                        // so use a static match for common cases
                                            if n <= 2 {
                                                "Stereo"
                                            } else {
                                                "Multi"
                                            },
                                    }
                                );

                                let text_color = if is_selected {
                                    t.colors.text_primary
                                } else {
                                    t.colors.text_secondary
                                };

                                let resp = ui.selectable_label(
                                    is_selected,
                                    egui::RichText::new(label).color(text_color),
                                );

                                if resp.clicked() && !is_selected {
                                    state.selected_sample = Some(meta.id);
                                    state.peaks_dirty = true;
                                    state.scroll_offset = 0.0;
                                    state.zoom = 1.0;
                                    state.editing_name = false;
                                    // Drop any in-progress description edit so its
                                    // buffer can't commit onto the newly selected sample.
                                    state.editing_description = false;
                                }
                            }
                        });

                    ui.separator();

                    // Memory usage
                    let total_bytes: usize = metas
                        .iter()
                        .map(|m| {
                            m.frame_count.as_usize()
                                * m.channels.count() as usize
                                * std::mem::size_of::<f32>()
                        })
                        .sum();
                    let mb = total_bytes as f64 / (1024.0 * 1024.0);
                    ui.label(
                        egui::RichText::new(format!("{} samples — {:.1} MB", metas.len(), mb))
                            .color(t.colors.text_dim)
                            .small(),
                    );
                }
            }

            ui.add_space(t.spacing.md);
            if ui
                .button(
                    egui::RichText::new(format!("{} Import", ri::ADD_LINE))
                        .color(t.colors.accent_primary),
                )
                .clicked()
            {
                action = SampleViewAction::ImportWav;
            }
        });

    // ---- Input monitor bar (bottom) ----
    audio_input.drain_gui_buffer();

    egui::Panel::bottom("sample_input_bar")
        .min_size(28.0)
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Input device selector
                ui.label(egui::RichText::new("Input:").color(t.colors.text_secondary));
                let current_name = state.selected_input_device.as_deref().unwrap_or("Default");
                egui::ComboBox::from_id_salt("input_device_selector")
                    .selected_text(current_name)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(state.selected_input_device.is_none(), "Default")
                            .clicked()
                        {
                            state.selected_input_device = None;
                        }
                        for dev in &state.cached_input_devices {
                            let selected =
                                state.selected_input_device.as_deref() == Some(&dev.name);
                            if ui.selectable_label(selected, &dev.name).clicked() {
                                state.selected_input_device = Some(dev.name.clone());
                            }
                        }
                    });

                ui.separator();

                // Level meter
                let peak = audio_input.peak_level();
                let meter_width = 80.0;
                let meter_height = 12.0;
                let (meter_rect, _) = ui.allocate_exact_size(
                    egui::vec2(meter_width, meter_height),
                    egui::Sense::hover(),
                );
                let painter = ui.painter_at(meter_rect);
                painter.rect_filled(meter_rect, 2.0, t.colors.bg_panel);
                let fill_w = (peak.min(1.0) * meter_width).max(0.0);
                if fill_w > 0.0 {
                    let fill_color = if peak > 0.9 {
                        t.colors.meter_red
                    } else if peak > 0.6 {
                        t.colors.meter_yellow
                    } else {
                        t.colors.meter_green
                    };
                    let fill_rect = egui::Rect::from_min_size(
                        meter_rect.left_top(),
                        egui::vec2(fill_w, meter_height),
                    );
                    painter.rect_filled(fill_rect, 2.0, fill_color);
                }

                ui.separator();

                // Monitor toggle
                let input_state = audio_input.state();
                let is_monitoring = input_state != InputState::Idle;
                let monitor_label = if is_monitoring {
                    format!("{} Monitor: ON", ri::MIC_FILL)
                } else {
                    format!("{} Monitor", ri::MIC_LINE)
                };
                let monitor_color = if is_monitoring {
                    t.colors.meter_green
                } else {
                    t.colors.text_dim
                };
                if ui
                    .button(egui::RichText::new(monitor_label).color(monitor_color))
                    .clicked()
                {
                    if is_monitoring {
                        action = SampleViewAction::StopMonitoring;
                    } else {
                        action = SampleViewAction::StartMonitoring;
                    }
                }

                ui.separator();

                // Record button
                let is_recording = input_state == InputState::Recording;
                let rec_label = if is_recording {
                    format!("{} Stop", ri::STOP_FILL)
                } else {
                    format!("{} Rec", ri::RECORD_CIRCLE_FILL)
                };
                let rec_color = if is_recording {
                    t.colors.meter_red
                } else if is_monitoring {
                    t.colors.text_primary
                } else {
                    t.colors.text_dim
                };
                if ui
                    .add_enabled(
                        is_monitoring,
                        egui::Button::new(egui::RichText::new(rec_label).color(rec_color)),
                    )
                    .clicked()
                {
                    if is_recording {
                        action = SampleViewAction::StopRecording;
                    } else {
                        action = SampleViewAction::StartRecording;
                    }
                }

                // Recording timer
                if is_recording {
                    let secs = audio_input.recorded_seconds();
                    let mins = (secs / 60.0) as u32;
                    let remaining = secs - f64::from(mins) * 60.0;
                    ui.label(
                        egui::RichText::new(format!("{mins}:{remaining:04.1}"))
                            .color(t.colors.meter_red)
                            .monospace(),
                    );
                    ctx.request_repaint();
                }
            });
        });

    // ---- Central panel: waveform + properties ----
    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(id) = state.selected_sample {
            if let Ok(lib) = library.read() {
                if let Some(sample) = lib.get(id) {
                    // Rebuild peak cache if needed
                    let width = ui.available_width().max(1.0) as usize;
                    rebuild_peaks_if_needed(state, sample, width);

                    // Waveform display
                    draw_waveform(ui, state, sample);

                    ui.add_space(t.spacing.md);
                    ui.separator();
                    ui.add_space(t.spacing.xs);

                    // Properties panel
                    let meta = sample.meta.clone();
                    drop(lib); // Release read lock before writing
                    draw_properties(ui, state, library, id, &meta);
                } else {
                    state.selected_sample = None;
                }
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Select or import a sample to begin editing")
                        .color(t.colors.text_dim),
                );
            });
        }
    });

    action
}

// ============================================================================
// WAVEFORM RENDERING
// ============================================================================

fn rebuild_peaks_if_needed(
    state: &mut SampleViewState,
    sample: &synth_sampler::Sample,
    display_width: usize,
) {
    let needs_rebuild = state.peaks_dirty
        || state
            .peak_cache
            .as_ref()
            .is_none_or(|c| c.sample_id != sample.meta.id);

    if !needs_rebuild {
        return;
    }

    let channels = sample.meta.channels.count() as usize;
    let frame_count = sample.meta.frame_count.as_usize();

    if frame_count == 0 || display_width == 0 {
        state.peak_cache = None;
        state.peaks_dirty = false;
        return;
    }

    // Compute frames per peak based on zoom
    let visible_frames = (frame_count as f32 * state.zoom).max(1.0) as usize;
    let frames_per_peak = (visible_frames / display_width).max(1);

    let peaks = build_peaks(&sample.data, channels, frames_per_peak);

    state.peak_cache = Some(WaveformPeaks {
        peaks,
        frames_per_peak,
        sample_id: sample.meta.id,
        _channels: channels,
    });
    state.peaks_dirty = false;
}

fn draw_waveform(ui: &mut egui::Ui, state: &mut SampleViewState, sample: &synth_sampler::Sample) {
    let t = theme();
    let available = ui.available_size();
    let height = (available.y * 0.55).clamp(100.0, 400.0);
    let width = available.x;

    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, 4.0, t.colors.bg_panel);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, t.colors.border),
        egui::StrokeKind::Inside,
    );

    // Center line
    let center_y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left(), center_y),
            egui::pos2(rect.right(), center_y),
        ],
        egui::Stroke::new(0.5, t.colors.border),
    );

    // Draw waveform from peaks
    if let Some(cache) = &state.peak_cache {
        let frame_count = sample.meta.frame_count.as_usize();
        let scroll_peak = (state.scroll_offset as usize)
            .checked_div(cache.frames_per_peak)
            .unwrap_or(0);
        let visible_peaks = (width as usize).min(cache.peaks.len().saturating_sub(scroll_peak));
        let half_h = height / 2.0 - 4.0;

        let waveform_color = t.colors.accent_primary;

        for i in 0..visible_peaks {
            let peak_idx = scroll_peak + i;
            if peak_idx >= cache.peaks.len() {
                break;
            }
            let (min_val, max_val) = cache.peaks[peak_idx];
            let x = rect.left() + i as f32;
            let y_min = center_y - max_val * half_h;
            let y_max = center_y - min_val * half_h;

            painter.line_segment(
                [egui::pos2(x, y_min), egui::pos2(x, y_max.max(y_min + 0.5))],
                egui::Stroke::new(1.0, waveform_color),
            );
        }

        // Compute frame-to-pixel mapping accounting for zoom and scroll
        let visible_frames = (frame_count as f32 * state.zoom).max(1.0);
        let frames_per_px = visible_frames / width;
        let scroll_frames = state.scroll_offset;

        /// Convert a frame index to an x pixel coordinate.
        fn frame_to_x(frame: usize, rect_left: f32, scroll: f32, fpp: f32) -> f32 {
            rect_left + (frame as f32 - scroll) / fpp
        }

        // Draw crop region overlay
        if let Some(crop) = sample.meta.crop {
            // Dim area before crop start
            if crop.start.0 > 0 {
                let crop_x = frame_to_x(crop.start.0, rect.left(), scroll_frames, frames_per_px);
                if crop_x > rect.left() {
                    let dim_rect = egui::Rect::from_min_max(
                        rect.left_top(),
                        egui::pos2(crop_x.min(rect.right()), rect.bottom()),
                    );
                    painter.rect_filled(dim_rect, 0.0, Color32::from_black_alpha(100));
                }
            }

            // Dim area after crop end
            if crop.end.0 < frame_count {
                let crop_x = frame_to_x(crop.end.0, rect.left(), scroll_frames, frames_per_px);
                if crop_x < rect.right() {
                    let dim_rect = egui::Rect::from_min_max(
                        egui::pos2(crop_x.max(rect.left()), rect.top()),
                        rect.right_bottom(),
                    );
                    painter.rect_filled(dim_rect, 0.0, Color32::from_black_alpha(100));
                }
            }
        }

        // Draw loop region markers
        if let Some(loop_region) = sample.meta.loop_region {
            let loop_start_x = frame_to_x(
                loop_region.start.0,
                rect.left(),
                scroll_frames,
                frames_per_px,
            );
            let loop_end_x =
                frame_to_x(loop_region.end.0, rect.left(), scroll_frames, frames_per_px);

            // Loop region overlay (guard against inverted rect when zoomed out of view)
            let clamped_start = loop_start_x.max(rect.left());
            let clamped_end = loop_end_x.min(rect.right());
            if clamped_start < clamped_end {
                let loop_rect = egui::Rect::from_min_max(
                    egui::pos2(clamped_start, rect.top()),
                    egui::pos2(clamped_end, rect.bottom()),
                );
                painter.rect_filled(
                    loop_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(100, 200, 100, 30),
                );
            }

            // Loop start/end lines (only if visible)
            let loop_color = t.colors.accent_green;
            if loop_start_x >= rect.left() && loop_start_x <= rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(loop_start_x, rect.top()),
                        egui::pos2(loop_start_x, rect.bottom()),
                    ],
                    egui::Stroke::new(2.0, loop_color),
                );
            }
            if loop_end_x >= rect.left() && loop_end_x <= rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(loop_end_x, rect.top()),
                        egui::pos2(loop_end_x, rect.bottom()),
                    ],
                    egui::Stroke::new(2.0, loop_color),
                );
            }
        }
    }

    // Handle scroll wheel for zoom
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            let factor = if scroll > 0.0 { 0.9 } else { 1.1 };
            state.zoom = (state.zoom * factor).clamp(0.01, 1000.0);
            state.peaks_dirty = true;
        }
    }

    // Handle drag for scroll
    if response.dragged() {
        state.scroll_offset = (state.scroll_offset - response.drag_delta().x * state.zoom).max(0.0);
        state.peaks_dirty = true;
    }
}

// ============================================================================
// PROPERTIES PANEL
// ============================================================================

fn draw_properties(
    ui: &mut egui::Ui,
    state: &mut SampleViewState,
    library: &Arc<RwLock<SampleLibrary>>,
    id: SampleId,
    meta: &synth_sampler::SampleMeta,
) {
    let t = theme();

    ui.heading(
        egui::RichText::new(format!("{} Properties", ri::SETTINGS_3_FILL))
            .color(t.colors.text_primary),
    );

    egui::Grid::new("sample_properties")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            // Name
            ui.label(egui::RichText::new("Name:").color(t.colors.text_secondary));
            if state.editing_name {
                let resp = ui.text_edit_singleline(&mut state.name_buffer);
                if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !state.name_buffer.is_empty()
                        && let Ok(mut lib) = library.write()
                        && let Some(m) = lib.get_meta(id)
                    {
                        let mut updated = m.clone();
                        updated.name = state.name_buffer.clone();
                        lib.update_meta(id, updated);
                    }
                    state.editing_name = false;
                }
            } else {
                let resp = ui.label(egui::RichText::new(&meta.name).color(t.colors.text_primary));
                if resp.double_clicked() {
                    state.editing_name = true;
                    state.name_buffer = meta.name.clone();
                }
            }
            ui.end_row();

            // Description
            ui.label(egui::RichText::new("Description:").color(t.colors.text_secondary));
            if state.editing_description {
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut state.description_buffer)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                // Commit on every change so dismissing the panel can't strand
                // the buffer; lost_focus just ends the edit session.
                if resp.changed()
                    && let Ok(mut lib) = library.write()
                    && let Some(m) = lib.get_meta(id)
                {
                    let mut updated = m.clone();
                    updated.description = state.description_buffer.clone();
                    lib.update_meta(id, updated);
                }
                if resp.lost_focus() {
                    state.editing_description = false;
                }
            } else {
                let text = if meta.description.is_empty() {
                    egui::RichText::new("(double-click to add)").color(t.colors.text_dim)
                } else {
                    egui::RichText::new(&meta.description).color(t.colors.text_primary)
                };
                let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                if resp.double_clicked() {
                    state.editing_description = true;
                    state.description_buffer = meta.description.clone();
                }
            }
            ui.end_row();

            // Duration
            ui.label(egui::RichText::new("Duration:").color(t.colors.text_secondary));
            ui.label(
                egui::RichText::new(format!("{:.2}s", meta.duration_seconds()))
                    .color(t.colors.text_primary),
            );
            ui.end_row();

            // Sample Rate
            ui.label(egui::RichText::new("Sample Rate:").color(t.colors.text_secondary));
            ui.label(
                egui::RichText::new(format!("{} Hz", meta.sample_rate.0))
                    .color(t.colors.text_primary),
            );
            ui.end_row();

            // Channels
            ui.label(egui::RichText::new("Channels:").color(t.colors.text_secondary));
            ui.label(
                egui::RichText::new(match meta.channels {
                    synth_core::ChannelCount::Mono => "Mono".to_string(),
                    synth_core::ChannelCount::Stereo => "Stereo".to_string(),
                    synth_core::ChannelCount::Multi(n) => format!("{n} ch"),
                })
                .color(t.colors.text_primary),
            );
            ui.end_row();

            // Frames
            ui.label(egui::RichText::new("Frames:").color(t.colors.text_secondary));
            ui.label(
                egui::RichText::new(format!("{}", meta.frame_count.as_usize()))
                    .color(t.colors.text_primary),
            );
            ui.end_row();

            // Root Note
            ui.label(egui::RichText::new("Root Note:").color(t.colors.text_secondary));
            {
                let current = meta.root_note.map_or(60_u8, |n| n.0);
                let note_names = [
                    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
                ];
                let octave = (current as i32 / 12) - 1;
                let note_name = note_names[(current % 12) as usize];
                let label = format!("{note_name}{octave}");

                let mut new_val = current;
                ui.horizontal(|ui| {
                    if ui.small_button("-").clicked() && new_val > 0 {
                        new_val -= 1;
                    }
                    ui.label(
                        egui::RichText::new(label)
                            .color(t.colors.text_primary)
                            .monospace(),
                    );
                    if ui.small_button("+").clicked() && new_val < 127 {
                        new_val += 1;
                    }
                });

                if new_val != current
                    && let Ok(mut lib) = library.write()
                    && let Some(m) = lib.get_meta(id)
                {
                    let mut updated = m.clone();
                    updated.root_note = Some(synth_core::MidiNote(new_val));
                    lib.update_meta(id, updated);
                }
            }
            ui.end_row();

            // Loop toggle
            ui.label(egui::RichText::new("Loop:").color(t.colors.text_secondary));
            {
                let has_loop = meta.loop_region.is_some();
                let mut loop_enabled = has_loop;
                if ui.checkbox(&mut loop_enabled, "").changed() {
                    if let Ok(mut lib) = library.write() {
                        if loop_enabled {
                            let frame_count = meta.frame_count.as_usize();
                            let region = LoopRegion {
                                start: FrameIndex::new(0),
                                end: FrameIndex::new(frame_count),
                                crossfade: SampleCount::new(64),
                            };
                            lib.update_loop(id, Some(region));
                        } else {
                            lib.update_loop(id, None);
                        }
                    }
                    state.peaks_dirty = true;
                }
            }
            ui.end_row();

            // Loop start/end (if loop is enabled)
            if let Some(loop_region) = meta.loop_region {
                ui.label(egui::RichText::new("Loop Start:").color(t.colors.text_secondary));
                {
                    let mut val = loop_region.start.0 as f32;
                    let max = loop_region.end.0.saturating_sub(1) as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut val)
                                .range(0.0..=max)
                                .speed(100.0)
                                .suffix(" frames"),
                        )
                        .changed()
                    {
                        if let Ok(mut lib) = library.write() {
                            let mut region = loop_region;
                            region.start = FrameIndex::new(val as usize);
                            lib.update_loop(id, Some(region));
                        }
                        state.peaks_dirty = true;
                    }
                }
                ui.end_row();

                ui.label(egui::RichText::new("Loop End:").color(t.colors.text_secondary));
                {
                    let mut val = loop_region.end.0 as f32;
                    let max = meta.frame_count.as_usize() as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut val)
                                .range((loop_region.start.0 + 1) as f32..=max)
                                .speed(100.0)
                                .suffix(" frames"),
                        )
                        .changed()
                    {
                        if let Ok(mut lib) = library.write() {
                            let mut region = loop_region;
                            region.end = FrameIndex::new(val as usize);
                            lib.update_loop(id, Some(region));
                        }
                        state.peaks_dirty = true;
                    }
                }
                ui.end_row();

                ui.label(egui::RichText::new("Crossfade:").color(t.colors.text_secondary));
                {
                    let mut val = loop_region.crossfade.as_usize() as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut val)
                                .range(0.0..=4096.0)
                                .speed(10.0)
                                .suffix(" frames"),
                        )
                        .changed()
                        && let Ok(mut lib) = library.write()
                    {
                        let mut region = loop_region;
                        region.crossfade = SampleCount::new(val as usize);
                        lib.update_loop(id, Some(region));
                    }
                }
                ui.end_row();
            }

            // Crop
            ui.label(egui::RichText::new("Crop:").color(t.colors.text_secondary));
            {
                let has_crop = meta.crop.is_some();
                let mut crop_enabled = has_crop;
                if ui.checkbox(&mut crop_enabled, "").changed() {
                    if let Ok(mut lib) = library.write() {
                        if crop_enabled {
                            let frame_count = meta.frame_count.as_usize();
                            let region = CropRegion {
                                start: FrameIndex::new(0),
                                end: FrameIndex::new(frame_count),
                            };
                            lib.update_crop(id, Some(region));
                        } else {
                            lib.update_crop(id, None);
                        }
                    }
                    state.peaks_dirty = true;
                }
            }
            ui.end_row();

            if let Some(crop) = meta.crop {
                ui.label(egui::RichText::new("Crop Start:").color(t.colors.text_secondary));
                {
                    let mut val = crop.start.0 as f32;
                    let max = crop.end.0.saturating_sub(1) as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut val)
                                .range(0.0..=max)
                                .speed(100.0)
                                .suffix(" frames"),
                        )
                        .changed()
                    {
                        if let Ok(mut lib) = library.write() {
                            lib.update_crop(
                                id,
                                Some(CropRegion {
                                    start: FrameIndex::new(val as usize),
                                    end: crop.end,
                                }),
                            );
                        }
                        state.peaks_dirty = true;
                    }
                }
                ui.end_row();

                ui.label(egui::RichText::new("Crop End:").color(t.colors.text_secondary));
                {
                    let mut val = crop.end.0 as f32;
                    let max = meta.frame_count.as_usize() as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut val)
                                .range((crop.start.0 + 1) as f32..=max)
                                .speed(100.0)
                                .suffix(" frames"),
                        )
                        .changed()
                    {
                        if let Ok(mut lib) = library.write() {
                            lib.update_crop(
                                id,
                                Some(CropRegion {
                                    start: crop.start,
                                    end: FrameIndex::new(val as usize),
                                }),
                            );
                        }
                        state.peaks_dirty = true;
                    }
                }
                ui.end_row();
            }
        });
}

// ============================================================================
// SAMPLE OPERATIONS
// ============================================================================

/// Normalize sample peak level to 0 dB.
fn normalize_sample(library: &Arc<RwLock<SampleLibrary>>, id: SampleId) {
    if let Ok(lib) = library.read()
        && let Some(sample) = lib.get(id)
    {
        let peak = sample.data.iter().fold(0.0_f32, |acc, &s| acc.max(s.abs()));
        if peak > 0.0 && (peak - 1.0).abs() > 1e-6 {
            let gain = 1.0 / peak;
            let normalized: Arc<[f32]> = sample
                .data
                .iter()
                .map(|&s| s * gain)
                .collect::<Vec<_>>()
                .into();
            drop(lib);
            if let Ok(mut lib) = library.write() {
                lib.replace_data(id, normalized);
            }
        }
    }
}

/// Reverse the sample data.
fn reverse_sample(library: &Arc<RwLock<SampleLibrary>>, id: SampleId) {
    if let Ok(lib) = library.read()
        && let Some(sample) = lib.get(id)
    {
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();
        let mut reversed = vec![0.0_f32; sample.data.len()];

        for frame in 0..frame_count {
            let src_frame = frame_count - 1 - frame;
            for ch in 0..channels {
                reversed[frame * channels + ch] = sample.data[src_frame * channels + ch];
            }
        }

        let data: Arc<[f32]> = reversed.into();
        drop(lib);
        if let Ok(mut lib) = library.write() {
            lib.replace_data(id, data);
        }
    }
}

/// Auto-trim: set crop to first/last audible frame (threshold: -40 dB).
fn auto_trim_sample(library: &Arc<RwLock<SampleLibrary>>, id: SampleId) {
    let threshold = 0.01_f32; // ~ -40 dB

    if let Ok(lib) = library.read()
        && let Some(sample) = lib.get(id)
    {
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();

        // Find first audible frame
        let mut start = 0;
        for frame in 0..frame_count {
            let mut max_val = 0.0_f32;
            for ch in 0..channels {
                max_val = max_val.max(sample.data[frame * channels + ch].abs());
            }
            if max_val > threshold {
                start = frame;
                break;
            }
        }

        // Find last audible frame
        let mut end = frame_count;
        for frame in (0..frame_count).rev() {
            let mut max_val = 0.0_f32;
            for ch in 0..channels {
                max_val = max_val.max(sample.data[frame * channels + ch].abs());
            }
            if max_val > threshold {
                end = frame + 1;
                break;
            }
        }

        if start < end {
            let crop = CropRegion {
                start: FrameIndex::new(start),
                end: FrameIndex::new(end),
            };
            drop(lib);
            if let Ok(mut lib) = library.write() {
                lib.update_crop(id, Some(crop));
            }
        }
    }
}
