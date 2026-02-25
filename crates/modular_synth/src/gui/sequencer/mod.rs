//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls, an arrangement timeline,
//! a piano roll read-view, and a GUI input source for sending `InputCommand`s
//! to the sequencer engine.

use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use synth_core::Bpm;
use synth_engine::{EngineCommand, EngineHandle};
use synth_sequencer::{
    InputCommand, InputSource, NoteName, PatternId, Song, Tick, TimeSignature, TrackId,
};

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
// VIEW STATE
// ============================================================================

/// Piano roll view state (persists across frames).
pub struct SequencerViewState {
    /// Currently opened pattern (None = piano roll closed).
    opened_pattern: Option<PatternId>,
}

impl SequencerViewState {
    pub fn new() -> Self {
        Self {
            opened_pattern: None,
        }
    }
}

impl Default for SequencerViewState {
    fn default() -> Self {
        Self::new()
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

// Piano roll constants
/// Height of the piano roll bottom panel.
const PIANO_ROLL_HEIGHT: f32 = 300.0;
/// Width of the keyboard column.
const KEY_WIDTH: f32 = 40.0;
/// Pixels per semitone (row height).
const NOTE_ROW_HEIGHT: f32 = 12.0;
/// Height of the velocity zone at the bottom.
const VELOCITY_ZONE_HEIGHT: f32 = 40.0;
/// Horizontal zoom: pixels per beat in the piano roll.
const PR_PIXELS_PER_BEAT: f32 = 60.0;
/// Height of the piano roll toolbar.
const PR_TOOLBAR_HEIGHT: f32 = 24.0;

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
    pattern_id: PatternId,
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
                pattern_id: p.pattern_id,
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
/// Returns `Some(PatternId)` if a placement was double-clicked.
#[allow(clippy::too_many_lines)]
fn draw_arrangement(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    current_tick: u64,
) -> Option<PatternId> {
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
        return None;
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

    let mut double_clicked_pattern: Option<PatternId> = None;

    // Scrollable timeline
    let scroll_id = ui.id().with("seq_scroll");
    egui::ScrollArea::horizontal()
        .id_salt(scroll_id)
        .show(ui, |ui| {
            let total_size = Vec2::new(
                TRACK_HEADER_WIDTH + timeline_width,
                RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
            );
            let (response, painter) = ui.allocate_painter(total_size, Sense::click());
            let painter_rect = response.rect;

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

            // ── Pattern placements (build rects for double-click detection) ──
            let mut placement_rects: Vec<(Rect, PatternId)> = Vec::new();

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

                placement_rects.push((rect, placement.pattern_id));

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

            // ── Double-click detection on placements ──
            if response.double_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                for (rect, pattern_id) in &placement_rects {
                    if rect.contains(pos) {
                        double_clicked_pattern = Some(*pattern_id);
                        break;
                    }
                }
            }

            // ── Hover hint on placements ──
            if response.hovered()
                && let Some(pos) = ui.ctx().pointer_hover_pos()
            {
                let on_placement = placement_rects.iter().any(|(r, _)| r.contains(pos));
                if on_placement {
                    response.on_hover_text("Double-click to open piano roll");
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

    double_clicked_pattern
}

// ============================================================================
// PIANO ROLL
// ============================================================================

/// Snapshot of a note for piano roll rendering.
struct PianoRollNote {
    pitch: u8,
    start_tick: u32,
    end_tick: Option<u32>,
    velocity: f32,
}

/// Collected data for piano roll rendering.
struct PianoRollData {
    pattern_name: String,
    #[allow(dead_code)]
    pattern_id: PatternId,
    length_ticks: u32,
    ticks_per_row: u16,
    notes: Vec<PianoRollNote>,
    pitch_min: u8,
    pitch_max: u8,
}

/// Collect piano roll data from song (short read-lock, then release).
fn collect_piano_roll_data(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
) -> Option<PianoRollData> {
    let song = song.try_read().ok()?;
    let pattern = song.pattern(pattern_id)?;

    let mut pitch_min: u8 = 127;
    let mut pitch_max: u8 = 0;

    let notes: Vec<PianoRollNote> = pattern
        .notes()
        .iter()
        .map(|n| {
            let midi = n.pitch.as_midi();
            if midi < pitch_min {
                pitch_min = midi;
            }
            if midi > pitch_max {
                pitch_max = midi;
            }
            PianoRollNote {
                pitch: midi,
                start_tick: n.start.0,
                end_tick: n.end().map(|e| e.0),
                velocity: n.velocity.as_f32(),
            }
        })
        .collect();

    // Default range if no notes
    if pitch_min > pitch_max {
        pitch_min = 48; // C3
        pitch_max = 72; // C5
    }

    Some(PianoRollData {
        pattern_name: if pattern.name.is_empty() {
            format!("Pattern {}", pattern_id.0)
        } else {
            pattern.name.clone()
        },
        pattern_id,
        length_ticks: pattern.length.0,
        ticks_per_row: pattern.row_resolution.ticks_per_row,
        notes,
        pitch_min,
        pitch_max,
    })
}

/// Draw the piano roll in a bottom panel.
/// Returns false if the close button was clicked.
#[allow(clippy::too_many_lines)]
fn draw_piano_roll(ui: &mut egui::Ui, data: &PianoRollData, current_tick: u64) -> bool {
    let t = theme();
    let mut keep_open = true;

    // ── Toolbar ──
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new(&data.pattern_name)
                .size(14.0)
                .color(t.colors.accent_cyan),
        );
        ui.separator();
        ui.label(
            RichText::new(format!("{} notes", data.notes.len())).color(t.colors.text_secondary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("X").color(t.colors.accent_red))
                .on_hover_text("Close piano roll")
                .clicked()
            {
                keep_open = false;
            }
        });
    });

    ui.separator();

    // ── Pitch range with margin ──
    let margin = 6_u8;
    let view_pitch_min = data.pitch_min.saturating_sub(margin);
    let view_pitch_max = (data.pitch_max + margin).min(127);
    let pitch_range = view_pitch_max - view_pitch_min + 1;

    let grid_height = pitch_range as f32 * NOTE_ROW_HEIGHT;
    let total_content_height = grid_height + VELOCITY_ZONE_HEIGHT;

    // Timeline width based on pattern length
    let ticks_per_beat = synth_sequencer::TICKS_PER_QUARTER;
    let beats_in_pattern = if ticks_per_beat > 0 {
        data.length_ticks as f32 / ticks_per_beat as f32
    } else {
        4.0
    };
    let grid_width = (beats_in_pattern * PR_PIXELS_PER_BEAT).max(200.0);

    // ── Scrollable piano roll area ──
    egui::ScrollArea::both()
        .id_salt("piano_roll_scroll")
        .max_height(PIANO_ROLL_HEIGHT - PR_TOOLBAR_HEIGHT - 8.0)
        .show(ui, |ui| {
            let total_size = Vec2::new(KEY_WIDTH + grid_width, total_content_height);
            let (_, rect) = ui.allocate_space(total_size);
            let painter = ui.painter_at(rect);

            let origin = rect.min;
            let grid_x = origin.x + KEY_WIDTH;
            let grid_y = origin.y;

            // Helper: tick to x position
            let tick_to_x = |tick_val: u32| -> f32 {
                if ticks_per_beat == 0 {
                    return grid_x;
                }
                let beats = tick_val as f32 / ticks_per_beat as f32;
                grid_x + beats * PR_PIXELS_PER_BEAT
            };

            // Helper: pitch to y position (higher pitch = lower y, piano style)
            let pitch_to_y = |pitch: u8| -> f32 {
                let row = view_pitch_max.saturating_sub(pitch);
                grid_y + row as f32 * NOTE_ROW_HEIGHT
            };

            // ── Keyboard (left column) ──
            painter.rect_filled(
                Rect::from_min_size(origin, Vec2::new(KEY_WIDTH, grid_height)),
                0.0,
                t.colors.bg_dark,
            );

            for p in view_pitch_min..=view_pitch_max {
                let y = pitch_to_y(p);
                let note_name = NoteName::from_midi(p % 12);
                let is_black = note_name.is_black_key();

                // Key background
                let key_color = if is_black {
                    Color32::from_rgb(30, 30, 35)
                } else {
                    Color32::from_rgb(55, 58, 65)
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x, y),
                        Vec2::new(KEY_WIDTH, NOTE_ROW_HEIGHT),
                    ),
                    0.0,
                    key_color,
                );

                // Label on C notes
                if p % 12 == 0 {
                    let octave = (p / 12) as i8 - 1;
                    painter.text(
                        Pos2::new(origin.x + 4.0, y + 1.0),
                        egui::Align2::LEFT_TOP,
                        format!("C{octave}"),
                        egui::FontId::proportional(10.0),
                        t.colors.text_primary,
                    );
                }

                // Key border
                painter.line_segment(
                    [
                        Pos2::new(origin.x, y + NOTE_ROW_HEIGHT),
                        Pos2::new(origin.x + KEY_WIDTH, y + NOTE_ROW_HEIGHT),
                    ],
                    Stroke::new(0.5, t.colors.border.gamma_multiply(0.3)),
                );
            }

            // ── Note grid background ──
            for p in view_pitch_min..=view_pitch_max {
                let y = pitch_to_y(p);
                let note_name = NoteName::from_midi(p % 12);
                let is_black = note_name.is_black_key();
                let is_c = p % 12 == 0;

                let bg = if is_c {
                    Color32::from_rgba_premultiplied(50, 55, 65, 80)
                } else if is_black {
                    Color32::from_rgba_premultiplied(25, 27, 30, 80)
                } else {
                    Color32::from_rgba_premultiplied(35, 38, 42, 80)
                };

                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(grid_x, y),
                        Vec2::new(grid_width, NOTE_ROW_HEIGHT),
                    ),
                    0.0,
                    bg,
                );

                // Horizontal pitch row separator
                painter.line_segment(
                    [
                        Pos2::new(grid_x, y + NOTE_ROW_HEIGHT),
                        Pos2::new(grid_x + grid_width, y + NOTE_ROW_HEIGHT),
                    ],
                    Stroke::new(
                        if is_c { 0.8 } else { 0.3 },
                        t.colors.border.gamma_multiply(if is_c { 0.6 } else { 0.2 }),
                    ),
                );
            }

            // ── Vertical beat/sub-beat lines ──
            let beats_total = beats_in_pattern.ceil() as u32;
            let beats_per_bar_val = 4_u32; // Default 4/4, could be derived from time sig
            for beat_idx in 0..=beats_total {
                let beat_tick = beat_idx * ticks_per_beat;
                let x = tick_to_x(beat_tick);
                let is_bar_line = beat_idx % beats_per_bar_val == 0;

                painter.line_segment(
                    [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                    Stroke::new(
                        if is_bar_line { 1.0 } else { 0.5 },
                        t.colors
                            .border
                            .gamma_multiply(if is_bar_line { 0.8 } else { 0.3 }),
                    ),
                );

                // Sub-beat lines (based on ticks_per_row)
                if data.ticks_per_row > 0 && !is_bar_line {
                    // Already drawing at beat level; skip sub-beats for readability
                }
            }

            // Sub-beat grid lines based on row resolution
            if data.ticks_per_row > 0 {
                let total_rows = data.length_ticks / data.ticks_per_row as u32;
                for row in 0..total_rows {
                    let row_tick = row * data.ticks_per_row as u32;
                    // Skip if this aligns with a beat line (already drawn)
                    if row_tick.is_multiple_of(ticks_per_beat) {
                        continue;
                    }
                    let x = tick_to_x(row_tick);
                    painter.line_segment(
                        [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                        Stroke::new(0.3, t.colors.border.gamma_multiply(0.15)),
                    );
                }
            }

            // ── Notes ──
            let note_color = Color32::from_rgb(100, 180, 255);
            for note in &data.notes {
                if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                    continue;
                }

                let y = pitch_to_y(note.pitch);
                let x_start = tick_to_x(note.start_tick);

                let x_end = match note.end_tick {
                    Some(end) => tick_to_x(end),
                    None => {
                        // Open-ended: draw to pattern end or at least a visible width
                        tick_to_x(data.length_ticks).min(x_start + grid_width)
                    }
                };

                let note_width = (x_end - x_start).max(3.0);
                let alpha = (note.velocity * 200.0 + 55.0).min(255.0) as u8;

                let fill = Color32::from_rgba_unmultiplied(
                    note_color.r(),
                    note_color.g(),
                    note_color.b(),
                    alpha,
                );

                let note_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
                );

                painter.rect_filled(note_rect, 2.0, fill);
                painter.rect_stroke(
                    note_rect,
                    2.0,
                    Stroke::new(0.5, note_color),
                    egui::StrokeKind::Inside,
                );

                // Open-ended indicator (gradient fade-out at right edge)
                if note.end_tick.is_none() && note_width > 8.0 {
                    let fade_rect = Rect::from_min_max(
                        Pos2::new(note_rect.max.x - 6.0, note_rect.min.y),
                        note_rect.max,
                    );
                    painter.rect_filled(
                        fade_rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(
                            note_color.r(),
                            note_color.g(),
                            note_color.b(),
                            alpha / 3,
                        ),
                    );
                }
            }

            // ── Velocity bars (below grid) ──
            let vel_y = grid_y + grid_height;
            // Background
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(grid_x, vel_y),
                    Vec2::new(grid_width, VELOCITY_ZONE_HEIGHT),
                ),
                0.0,
                Color32::from_rgba_premultiplied(20, 22, 26, 200),
            );

            // Separator line
            painter.line_segment(
                [
                    Pos2::new(grid_x, vel_y),
                    Pos2::new(grid_x + grid_width, vel_y),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // Velocity label
            painter.text(
                Pos2::new(origin.x + 2.0, vel_y + 2.0),
                egui::Align2::LEFT_TOP,
                "VEL",
                egui::FontId::proportional(9.0),
                t.colors.text_dim,
            );

            // Velocity bars
            for note in &data.notes {
                let x = tick_to_x(note.start_tick);
                let bar_height = note.velocity * (VELOCITY_ZONE_HEIGHT - 4.0);
                let bar_y = vel_y + VELOCITY_ZONE_HEIGHT - bar_height - 2.0;

                let vel_color = velocity_color(note.velocity);
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x - 1.5, bar_y), Vec2::new(3.0, bar_height)),
                    1.0,
                    vel_color,
                );
            }

            // ── Playhead ──
            // Convert song tick to pattern-relative tick if applicable
            if current_tick > 0 && data.length_ticks > 0 {
                // Show playhead if it's within pattern range
                // (simplified: just show the raw position modulo pattern length)
                #[allow(clippy::cast_possible_truncation)]
                let pattern_tick = (current_tick % data.length_ticks as u64) as u32;
                let playhead_x = tick_to_x(pattern_tick);

                if playhead_x >= grid_x && playhead_x <= grid_x + grid_width {
                    painter.line_segment(
                        [
                            Pos2::new(playhead_x, grid_y),
                            Pos2::new(playhead_x, vel_y + VELOCITY_ZONE_HEIGHT),
                        ],
                        Stroke::new(1.5, t.colors.accent_primary),
                    );
                }
            }

            // ── Keyboard / grid separator ──
            painter.line_segment(
                [
                    Pos2::new(grid_x, grid_y),
                    Pos2::new(grid_x, grid_y + total_content_height),
                ],
                Stroke::new(1.0, t.colors.border),
            );
        });

    keep_open
}

/// Map velocity (0.0-1.0) to a color (green → yellow → red).
fn velocity_color(vel: f32) -> Color32 {
    if vel < 0.5 {
        let t = vel * 2.0;
        Color32::from_rgb(
            (80.0 + t * 175.0) as u8,
            (180.0 + t * 40.0) as u8,
            (80.0 - t * 40.0) as u8,
        )
    } else {
        let t = (vel - 0.5) * 2.0;
        Color32::from_rgb(
            (255.0) as u8,
            (220.0 - t * 140.0) as u8,
            (40.0 - t * 40.0) as u8,
        )
    }
}

// ============================================================================
// SEQUENCER VIEW (MAIN ENTRY)
// ============================================================================

/// Draw the full sequencer view (transport + arrangement + piano roll).
pub fn draw_sequencer_view(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
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

    // Piano roll bottom panel (if a pattern is open)
    if let Some(pattern_id) = view_state.opened_pattern {
        let piano_roll_data = collect_piano_roll_data(song, pattern_id);

        egui::TopBottomPanel::bottom("piano_roll")
            .resizable(true)
            .default_height(PIANO_ROLL_HEIGHT)
            .min_height(150.0)
            .max_height(600.0)
            .show(ctx, |ui| {
                if let Some(data) = &piano_roll_data {
                    if !draw_piano_roll(ui, data, current_tick) {
                        view_state.opened_pattern = None;
                    }
                } else {
                    // Pattern no longer exists
                    ui.label(RichText::new("Pattern not found").color(theme().colors.text_dim));
                    view_state.opened_pattern = None;
                }
            });
    }

    // Main content: arrangement view
    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(data) = &arrangement_data {
            if let Some(pattern_id) = draw_arrangement(ui, data, current_tick) {
                view_state.opened_pattern = Some(pattern_id);
            }
        } else {
            ui.label(RichText::new("Song locked...").color(theme().colors.text_dim));
        }
    });
}

/// Convert a sequencer track color to an egui Color32.
fn track_color_to_egui(color: synth_sequencer::TrackColor) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}
