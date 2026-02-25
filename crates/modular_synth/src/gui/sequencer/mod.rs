//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls, an arrangement timeline,
//! and a GUI input source for sending `InputCommand`s to the sequencer engine.

use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use synth_core::Bpm;
use synth_engine::{EngineCommand, EngineHandle};
use synth_sequencer::{InputCommand, InputSource, Song, Tick, TimeSignature, TrackId};

use crate::gui::theme::theme;

// ============================================================================
// GUI INPUT SOURCE
// ============================================================================

/// GUI input source for the sequencer.
///
/// Commands are queued from the GUI thread and polled by the sequencer engine.
pub struct SequencerGuiInput {
    pending: Vec<InputCommand>,
    enabled: bool,
}

impl SequencerGuiInput {
    /// Create a new GUI input source (enabled by default).
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            enabled: true,
        }
    }
}

impl Default for SequencerGuiInput {
    fn default() -> Self {
        Self::new()
    }
}

impl InputSource for SequencerGuiInput {
    fn poll(&mut self) -> Vec<InputCommand> {
        std::mem::take(&mut self.pending)
    }

    fn name(&self) -> &str {
        "sequencer_gui"
    }

    fn is_active(&self) -> bool {
        !self.pending.is_empty()
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

// ============================================================================
// CONSTANTS
// ============================================================================

/// Width of the track header panel (left side).
const TRACK_HEADER_WIDTH: f32 = 150.0;
/// Height of each track row in the arrangement.
const TRACK_ROW_HEIGHT: f32 = 48.0;
/// Height of the timeline ruler at the top.
const RULER_HEIGHT: f32 = 24.0;
/// Pixels per beat at default zoom.
const PIXELS_PER_BEAT: f32 = 40.0;
/// Minimum number of bars to show (even if song is shorter).
const MIN_VISIBLE_BARS: u32 = 8;
/// Padding inside pattern placement boxes.
const PLACEMENT_PADDING: f32 = 2.0;

// ============================================================================
// TRANSPORT BAR
// ============================================================================

/// Draw the transport control bar.
///
/// Shows play/stop/pause buttons, position display (Bar:Beat:Tick), and tempo.
/// Returns true if playback is active (for repaint scheduling).
fn draw_transport_bar(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
) -> bool {
    let t = theme();
    let state = &handle.state;
    let is_playing = state.transport.is_playing();
    let current_ticks = state.transport.get_ticks();
    let current_tick = Tick(current_ticks);
    let tempo_f32 = state.transport.get_tempo();

    // Read time signature and song name from song (non-blocking)
    let (time_sig, song_name) = song
        .try_read()
        .map(|s| (s.time_signature_at(current_tick), s.name.clone()))
        .unwrap_or((TimeSignature::COMMON, String::new()));

    let (bar, beat, tick) = current_tick.to_bar_beat_tick(time_sig);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // Song name
        if !song_name.is_empty() {
            ui.label(RichText::new(&song_name).color(t.colors.accent_cyan));
            ui.separator();
        }

        // Go to start
        if ui
            .button(RichText::new("|<").color(t.colors.text_primary))
            .on_hover_text("Go to start")
            .clicked()
        {
            handle.send(EngineCommand::Seek { tick: Tick::ZERO });
        }

        // Play / Pause toggle
        if is_playing {
            if ui
                .button(RichText::new("||").color(t.colors.accent_yellow))
                .on_hover_text("Pause")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else if ui
            .button(RichText::new(" > ").color(t.colors.accent_green))
            .on_hover_text("Play")
            .clicked()
        {
            handle.send(EngineCommand::Play);
        }

        // Stop
        if ui
            .button(RichText::new("[]").color(if is_playing {
                t.colors.accent_red
            } else {
                t.colors.text_dim
            }))
            .on_hover_text("Stop")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
        }

        ui.separator();

        // Position display: Bar:Beat:Tick (1-based)
        let pos_text = format!("{:03}:{:02}:{:03}", bar + 1, beat + 1, tick);
        ui.label(
            RichText::new(pos_text)
                .family(egui::FontFamily::Monospace)
                .size(16.0)
                .color(if is_playing {
                    t.colors.accent_primary
                } else {
                    t.colors.text_primary
                }),
        );

        ui.separator();

        // Tempo
        ui.label(RichText::new("BPM").color(t.colors.text_dim));
        let mut tempo_val = tempo_f32;
        let tempo_response = ui.add(
            egui::DragValue::new(&mut tempo_val)
                .range(20.0..=300.0)
                .speed(0.5)
                .fixed_decimals(1),
        );
        if tempo_response.changed() {
            handle.send(EngineCommand::SetTempo(Bpm::new(tempo_val)));
        }

        ui.separator();

        // Time signature
        ui.label(
            RichText::new(format!("{}/{}", time_sig.numerator, time_sig.denominator))
                .color(t.colors.text_secondary),
        );

        ui.separator();

        // Status indicator
        if is_playing {
            ui.label(RichText::new("PLAYING").color(t.colors.meter_green));
        } else if current_ticks > 0 {
            ui.label(RichText::new("PAUSED").color(t.colors.accent_yellow));
        } else {
            ui.label(RichText::new("STOPPED").color(t.colors.text_dim));
        }
    });

    is_playing
}

// ============================================================================
// ARRANGEMENT VIEW
// ============================================================================

/// Snapshot of track info needed for rendering (avoids holding RwLock during paint).
struct TrackInfo {
    id: TrackId,
    name: String,
    color: Color32,
    mute: bool,
    solo: bool,
}

/// Snapshot of a pattern placement for rendering.
struct PlacementInfo {
    track_id: TrackId,
    start_tick: u64,
    end_tick: u64,
    pattern_name: String,
    note_count: usize,
    color: Color32,
}

/// Collected song data for arrangement rendering.
struct ArrangementData {
    tracks: Vec<TrackInfo>,
    placements: Vec<PlacementInfo>,
    time_sig: TimeSignature,
    song_end_tick: u64,
}

/// Collect arrangement data from song (short read-lock, then release).
fn collect_arrangement_data(song: &Arc<RwLock<Song>>) -> Option<ArrangementData> {
    let song = song.try_read().ok()?;

    let tracks: Vec<TrackInfo> = song
        .tracks()
        .map(|t| TrackInfo {
            id: t.id,
            name: t.name.clone(),
            color: track_color_to_egui(t.color),
            mute: t.mute,
            solo: t.solo,
        })
        .collect();

    let mut song_end_tick: u64 = 0;
    let placements: Vec<PlacementInfo> = song
        .arrangement()
        .iter()
        .filter_map(|p| {
            let pattern = song.pattern(p.pattern_id)?;
            let end = p.end(pattern.length);
            if end.0 > song_end_tick {
                song_end_tick = end.0;
            }
            // Color from the track this placement belongs to
            let color = song
                .track(p.track_id)
                .map(|t| track_color_to_egui(t.color))
                .unwrap_or(Color32::GRAY);

            Some(PlacementInfo {
                track_id: p.track_id,
                start_tick: p.start.0,
                end_tick: end.0,
                pattern_name: pattern.name.clone(),
                note_count: pattern.notes().len(),
                color,
            })
        })
        .collect();

    let time_sig = song.default_time_signature;

    Some(ArrangementData {
        tracks,
        placements,
        time_sig,
        song_end_tick,
    })
}

/// Draw the arrangement view with track headers and timeline.
#[allow(clippy::too_many_lines)]
fn draw_arrangement(ui: &mut egui::Ui, data: &ArrangementData, current_tick: u64) {
    let t = theme();
    let track_count = data.tracks.len();

    if track_count == 0 {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Empty song")
                    .size(16.0)
                    .color(t.colors.text_dim),
            );
            ui.add_space(4.0);
            ui.label(RichText::new("Use MCP to add tracks and patterns").color(t.colors.text_dim));
        });
        return;
    }

    // Calculate timeline extent
    let ticks_per_bar = data.time_sig.ticks_per_bar() as u64;
    let ticks_per_beat = data.time_sig.ticks_per_beat() as u64;
    let beats_per_bar = data.time_sig.numerator as u64;

    // Song end in bars (at least MIN_VISIBLE_BARS)
    let song_bars = if ticks_per_bar > 0 {
        data.song_end_tick.div_ceil(ticks_per_bar) as u32
    } else {
        MIN_VISIBLE_BARS
    };
    let total_bars = song_bars.max(MIN_VISIBLE_BARS) + 2; // extra padding
    let total_beats = total_bars as f32 * beats_per_bar as f32;
    let timeline_width = total_beats * PIXELS_PER_BEAT;

    // Scrollable timeline
    let scroll_id = ui.id().with("seq_scroll");
    egui::ScrollArea::horizontal()
        .id_salt(scroll_id)
        .show(ui, |ui| {
            let (_, painter_rect) = ui.allocate_space(Vec2::new(
                TRACK_HEADER_WIDTH + timeline_width,
                RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
            ));
            let painter = ui.painter_at(painter_rect);

            let origin = painter_rect.min;
            let tl_x = origin.x + TRACK_HEADER_WIDTH;
            let tl_y = origin.y;

            // Helper: tick to x position
            let tick_to_x = |tick_val: u64| -> f32 {
                if ticks_per_beat == 0 {
                    return tl_x;
                }
                let beats = tick_val as f32 / ticks_per_beat as f32;
                tl_x + beats * PIXELS_PER_BEAT
            };

            // ── Track headers (drawn on top of scroll, at fixed offset) ──
            // Background for header column
            painter.rect_filled(
                Rect::from_min_size(
                    origin,
                    Vec2::new(
                        TRACK_HEADER_WIDTH,
                        RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
                    ),
                ),
                0.0,
                t.colors.bg_panel,
            );

            // Ruler corner
            painter.rect_filled(
                Rect::from_min_size(origin, Vec2::new(TRACK_HEADER_WIDTH, RULER_HEIGHT)),
                0.0,
                t.colors.bg_dark,
            );

            for (i, track) in data.tracks.iter().enumerate() {
                let row_y = tl_y + RULER_HEIGHT + i as f32 * TRACK_ROW_HEIGHT;
                let row_rect = Rect::from_min_size(
                    Pos2::new(origin.x, row_y),
                    Vec2::new(TRACK_HEADER_WIDTH, TRACK_ROW_HEIGHT),
                );

                // Row background (alternating)
                let bg = if i % 2 == 0 {
                    t.colors.bg_module
                } else {
                    t.colors.bg_panel
                };
                painter.rect_filled(row_rect, 0.0, bg);

                // Track color indicator
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x + 2.0, row_y + 2.0),
                        Vec2::new(4.0, TRACK_ROW_HEIGHT - 4.0),
                    ),
                    2.0,
                    track.color,
                );

                // Track name
                painter.text(
                    Pos2::new(origin.x + 12.0, row_y + 8.0),
                    egui::Align2::LEFT_TOP,
                    &track.name,
                    egui::FontId::proportional(13.0),
                    t.colors.text_primary,
                );

                // Mute/Solo indicators
                let mut indicator_x = origin.x + 12.0;
                let indicator_y = row_y + 26.0;
                if track.mute {
                    painter.text(
                        Pos2::new(indicator_x, indicator_y),
                        egui::Align2::LEFT_TOP,
                        "M",
                        egui::FontId::proportional(11.0),
                        t.colors.accent_red,
                    );
                    indicator_x += 16.0;
                }
                if track.solo {
                    painter.text(
                        Pos2::new(indicator_x, indicator_y),
                        egui::Align2::LEFT_TOP,
                        "S",
                        egui::FontId::proportional(11.0),
                        t.colors.accent_yellow,
                    );
                }

                // Row separator
                painter.line_segment(
                    [
                        Pos2::new(origin.x, row_y + TRACK_ROW_HEIGHT),
                        Pos2::new(
                            origin.x + TRACK_HEADER_WIDTH + timeline_width,
                            row_y + TRACK_ROW_HEIGHT,
                        ),
                    ],
                    Stroke::new(0.5, t.colors.border),
                );
            }

            // ── Ruler (bar/beat numbers) ──
            let ruler_rect = Rect::from_min_size(
                Pos2::new(tl_x, tl_y),
                Vec2::new(timeline_width, RULER_HEIGHT),
            );
            painter.rect_filled(ruler_rect, 0.0, t.colors.bg_dark);

            for bar_idx in 0..total_bars {
                let bar_tick = bar_idx as u64 * ticks_per_bar;
                let x = tick_to_x(bar_tick);

                // Bar number
                painter.text(
                    Pos2::new(x + 4.0, tl_y + 4.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}", bar_idx + 1),
                    egui::FontId::proportional(12.0),
                    t.colors.text_secondary,
                );

                // Bar line (strong)
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                painter.line_segment(
                    [Pos2::new(x, tl_y + RULER_HEIGHT), Pos2::new(x, line_bottom)],
                    Stroke::new(1.0, t.colors.border),
                );

                // Beat lines (subtle)
                for beat in 1..beats_per_bar {
                    let beat_tick = bar_tick + beat * ticks_per_beat;
                    let bx = tick_to_x(beat_tick);
                    painter.line_segment(
                        [
                            Pos2::new(bx, tl_y + RULER_HEIGHT),
                            Pos2::new(bx, line_bottom),
                        ],
                        Stroke::new(0.5, t.colors.border.gamma_multiply(0.4)),
                    );
                }
            }

            // ── Track row backgrounds for timeline area ──
            for i in 0..track_count {
                let row_y = tl_y + RULER_HEIGHT + i as f32 * TRACK_ROW_HEIGHT;
                let bg = if i % 2 == 0 {
                    Color32::from_rgba_premultiplied(40, 42, 46, 80)
                } else {
                    Color32::TRANSPARENT
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(tl_x, row_y),
                        Vec2::new(timeline_width, TRACK_ROW_HEIGHT),
                    ),
                    0.0,
                    bg,
                );
            }

            // ── Pattern placements ──
            for placement in &data.placements {
                // Find track row index
                let Some(row_idx) = data.tracks.iter().position(|t| t.id == placement.track_id)
                else {
                    continue;
                };

                let x_start = tick_to_x(placement.start_tick);
                let x_end = tick_to_x(placement.end_tick);
                let row_y =
                    tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
                let height = TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0;

                let rect = Rect::from_min_size(
                    Pos2::new(x_start, row_y),
                    Vec2::new((x_end - x_start).max(4.0), height),
                );

                // Fill with track color (semi-transparent)
                let fill = Color32::from_rgba_unmultiplied(
                    placement.color.r(),
                    placement.color.g(),
                    placement.color.b(),
                    100,
                );
                painter.rect_filled(rect, 3.0, fill);

                // Border
                painter.rect_stroke(
                    rect,
                    3.0,
                    Stroke::new(1.0, placement.color),
                    egui::StrokeKind::Inside,
                );

                // Pattern name (clipped to box)
                let text_clip = Rect::from_min_max(
                    Pos2::new(rect.min.x + 4.0, rect.min.y),
                    Pos2::new(rect.max.x - 2.0, rect.max.y),
                );
                if text_clip.width() > 20.0 {
                    painter.with_clip_rect(text_clip).text(
                        Pos2::new(rect.min.x + 4.0, rect.min.y + 4.0),
                        egui::Align2::LEFT_TOP,
                        &placement.pattern_name,
                        egui::FontId::proportional(11.0),
                        t.colors.text_primary,
                    );

                    // Note count
                    if text_clip.width() > 50.0 {
                        painter.with_clip_rect(text_clip).text(
                            Pos2::new(rect.min.x + 4.0, rect.min.y + 18.0),
                            egui::Align2::LEFT_TOP,
                            format!("{} notes", placement.note_count),
                            egui::FontId::proportional(9.0),
                            t.colors.text_dim,
                        );
                    }
                }
            }

            // ── Playhead ──
            if current_tick > 0 || data.song_end_tick > 0 {
                let playhead_x = tick_to_x(current_tick);
                let line_top = tl_y;
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;

                // Playhead line
                painter.line_segment(
                    [
                        Pos2::new(playhead_x, line_top),
                        Pos2::new(playhead_x, line_bottom),
                    ],
                    Stroke::new(2.0, t.colors.accent_primary),
                );

                // Playhead triangle in ruler
                let tri_size = 6.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(playhead_x - tri_size, line_top),
                        Pos2::new(playhead_x + tri_size, line_top),
                        Pos2::new(playhead_x, line_top + RULER_HEIGHT * 0.6),
                    ],
                    t.colors.accent_primary,
                    Stroke::NONE,
                ));
            }

            // ── Header/timeline separator ──
            painter.line_segment(
                [
                    Pos2::new(tl_x, tl_y),
                    Pos2::new(
                        tl_x,
                        tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
                    ),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // Ruler bottom border
            painter.line_segment(
                [
                    Pos2::new(origin.x, tl_y + RULER_HEIGHT),
                    Pos2::new(
                        origin.x + TRACK_HEADER_WIDTH + timeline_width,
                        tl_y + RULER_HEIGHT,
                    ),
                ],
                Stroke::new(1.0, t.colors.border),
            );
        });
}

// ============================================================================
// SEQUENCER VIEW (MAIN ENTRY)
// ============================================================================

/// Draw the full sequencer view (transport + arrangement).
pub fn draw_sequencer_view(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
) {
    // Transport bar at the top
    let is_playing = egui::TopBottomPanel::top("sequencer_transport")
        .show(ctx, |ui| draw_transport_bar(ui, handle, song))
        .inner;

    // Request repaint during playback for smooth position updates
    if is_playing {
        ctx.request_repaint();
    }

    // Read current playhead position (atomic, lock-free)
    let current_tick = handle.state.transport.get_ticks();

    // Collect song data (short read-lock, then release before rendering)
    let arrangement_data = collect_arrangement_data(song);

    // Main content: arrangement view
    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(data) = &arrangement_data {
            draw_arrangement(ui, data, current_tick);
        } else {
            ui.label(RichText::new("Song locked...").color(theme().colors.text_dim));
        }
    });
}

/// Convert a sequencer track color to an egui Color32.
fn track_color_to_egui(color: synth_sequencer::TrackColor) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}
