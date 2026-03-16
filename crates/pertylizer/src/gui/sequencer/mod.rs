//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls, an arrangement timeline,
//! a piano roll with mouse interaction (draw, select, move, resize, delete notes),
//! and a GUI input source for sending `InputCommand`s to the sequencer engine.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use synth_core::{Bpm, MidiNote, NormalizedValue, Semitones};
use synth_engine::{EngineCommand, EngineHandle, RecordingState};
use synth_sequencer::{
    AutoInstrumentParam, AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration,
    InputCommand, InputSource, NoteId, NoteName, PatternId, PatternTick, Pitch, SeqInstrumentId,
    Song, Tick, TimeSignature, TrackId, Velocity,
};

use crate::gui::input::KEY_MAP;
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
// EDIT TYPES
// ============================================================================

/// Active editing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTool {
    Select,
    Draw,
}

/// Which part of a note was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitZone {
    /// Note body (for moving).
    Body,
    /// Right edge (for resizing).
    RightEdge,
}

/// Active drag operation (preview only — Song is mutated on release).
#[derive(Debug, Clone)]
enum DragState {
    /// Drag-move a note.
    MoveNote {
        note_id: NoteId,
        original_tick: PatternTick,
        original_pitch: Pitch,
        current_tick: PatternTick,
        current_pitch: Pitch,
        /// Offset from note start to where the mouse grabbed (in ticks).
        grab_offset_ticks: SeqDuration,
    },
    /// Drag-resize a note (right edge).
    ResizeNote {
        note_id: NoteId,
        original_end_tick: PatternTick,
        current_end_tick: PatternTick,
    },
    /// Draw a new note by dragging (Draw tool on empty space).
    DrawNote {
        start_tick: PatternTick,
        pitch: Pitch,
        current_end_tick: PatternTick,
    },
    /// Selection rectangle (lasso).
    SelectRect { start_pos: Pos2, current_pos: Pos2 },
    /// Drag-move an automation point.
    DragAutomationPoint {
        target: AutomationTarget,
        original_tick: PatternTick,
        original_value: NormalizedValue,
        current_tick: PatternTick,
        current_value: NormalizedValue,
    },
    /// Drag-edit a velocity bar.
    DragVelocity {
        /// Note being velocity-edited (None = painting across multiple).
        last_note_id: Option<NoteId>,
    },
    /// Drag-move a pattern placement in the arrangement.
    DragPlacement {
        pattern_id: PatternId,
        track_id: TrackId,
        start_tick: Tick,
        /// Current drag tick position (snapped to grid).
        current_tick: Tick,
        /// Current target track ID.
        current_track_id: TrackId,
        /// Offset from placement start to where mouse grabbed (in ticks).
        grab_offset_ticks: Tick,
    },
}

// ============================================================================
// CLIPBOARD
// ============================================================================

/// A note stored in the clipboard, with position relative to the selection origin.
#[derive(Debug, Clone)]
struct ClipboardNote {
    tick_offset: SeqDuration,
    pitch: Pitch,
    velocity: Velocity,
    duration: Option<SeqDuration>,
    instrument: SeqInstrumentId,
}

/// Piano roll clipboard for copy/paste operations.
#[derive(Debug, Clone, Default)]
struct Clipboard {
    notes: Vec<ClipboardNote>,
    /// Total width of the copied selection (max end - min start), in ticks.
    selection_width: SeqDuration,
}

// ============================================================================
// VIEW STATE
// ============================================================================

/// Piano roll view state (persists across frames).
pub struct SequencerViewState {
    /// Clipboard for copy/paste operations.
    clipboard: Clipboard,
    /// Default velocity for newly drawn/recorded notes (0.0-1.0).
    pub default_velocity: Velocity,
    /// Quantization strength for the quantize button (0.0-1.0).
    quantize_strength: NormalizedValue,
    /// Swing amount (0.0-1.0, 0.0 = no swing).
    swing_amount: NormalizedValue,
    /// Velocity scale factor for scale-velocities operation (1–200%).
    velocity_scale_pct: u32,
    /// Draw-mode grid resolution in ticks (0 = use pattern default).
    draw_grid_resolution: u32,
    /// Draw-mode note length preset in ticks (0 = drag-to-length).
    draw_note_length: u32,
    /// Step entry mode enabled.
    step_entry_mode: bool,
    /// Step entry cursor position.
    step_cursor_tick: PatternTick,
    /// Currently opened pattern (None = piano roll closed).
    opened_pattern: Option<PatternId>,
    /// Currently selected notes.
    selected_notes: HashSet<NoteId>,
    /// Active edit tool.
    edit_tool: EditTool,
    /// Active drag operation.
    drag: Option<DragState>,
    /// Currently selected automation lane (None = automation zone hidden).
    selected_automation: Option<AutomationTarget>,
    /// Track currently being renamed (inline text edit).
    editing_track_name: Option<(TrackId, String)>,
    /// Pattern currently being renamed (inline text edit).
    editing_pattern_name: Option<(PatternId, String)>,
    /// Repeat song (loop entire song).
    repeat_enabled: bool,
    /// Pattern repeat enabled (loop current pattern in piano roll).
    pattern_repeat: bool,
    /// Stored right-click position (captured at click time, before menu opens).
    context_menu_pos: Option<Pos2>,
    /// Track to highlight (set on right-click, cleared on primary click).
    highlighted_track: Option<TrackId>,
    /// Selected track for pattern follow mode.
    selected_track: Option<TrackId>,
    /// Arrangement timeline zoom level (1.0 = default).
    zoom_level: f32,
    /// Auto-scroll to follow playhead during playback.
    auto_follow_playhead: bool,
    /// Last scroll offset set by auto-follow (to detect manual scrolling).
    last_auto_scroll_offset: Option<f32>,
    /// Recording quantize grid in ticks (0=off, 960=1/4, 480=1/8, 240=1/16, 120=1/32).
    pub record_quantize: u32,
    /// Overdub mode: true = layer on existing notes, false = replace.
    pub overdub: bool,
    /// Live recording preview: completed notes.
    pub recording_preview_completed: Vec<synth_engine::recording::RecordedNote>,
    /// Live recording preview: held note starts (pitch, start_tick).
    pub recording_preview_held: Vec<(Pitch, PatternTick)>,
    /// Pattern length for preview rendering.
    pub recording_preview_pattern_length: SeqDuration,
    /// Selected instrument for new notes in the piano roll.
    pub selected_instrument: SeqInstrumentId,
    /// Instrument captured at recording arm time (used when flushing recorded notes).
    pub recording_instrument: SeqInstrumentId,
}

impl SequencerViewState {
    pub fn new() -> Self {
        Self {
            clipboard: Clipboard::default(),
            default_velocity: Velocity::new(0.8),
            quantize_strength: NormalizedValue::new(1.0),
            swing_amount: NormalizedValue::new(0.0),
            velocity_scale_pct: 100,
            draw_grid_resolution: 0,
            draw_note_length: 0,
            step_entry_mode: false,
            step_cursor_tick: PatternTick::ZERO,
            opened_pattern: None,
            selected_notes: HashSet::new(),
            edit_tool: EditTool::Draw,
            drag: None,
            selected_automation: None,
            editing_track_name: None,
            editing_pattern_name: None,
            repeat_enabled: false,
            pattern_repeat: true,
            context_menu_pos: None,
            highlighted_track: None,
            selected_track: None,
            zoom_level: 1.0,
            auto_follow_playhead: true,
            last_auto_scroll_offset: None,
            record_quantize: 0,
            overdub: true,
            recording_preview_completed: Vec::new(),
            recording_preview_held: Vec::new(),
            recording_preview_pattern_length: SeqDuration(0),
            selected_instrument: SeqInstrumentId::new(0),
            recording_instrument: SeqInstrumentId::new(0),
        }
    }
}

impl SequencerViewState {
    /// Get the effective grid resolution in ticks, falling back to pattern default.
    fn effective_grid(&self, fallback_ticks_per_row: u16) -> SeqDuration {
        SeqDuration(if self.draw_grid_resolution > 0 {
            self.draw_grid_resolution
        } else {
            fallback_ticks_per_row as u32
        })
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
/// Minimum zoom level for arrangement timeline.
const MIN_ZOOM: f32 = 0.25;
/// Maximum zoom level for arrangement timeline.
const MAX_ZOOM: f32 = 4.0;
/// Maximum number of note miniatures per placement.
const MAX_MINIATURE_NOTES: usize = 200;

// Piano roll constants
/// Minimum default height for the piano roll bottom panel.
const MIN_PIANO_ROLL_HEIGHT: f32 = 400.0;
/// Width of the keyboard column.
const KEY_WIDTH: f32 = 40.0;
/// Pixels per semitone (row height).
const NOTE_ROW_HEIGHT: f32 = 12.0;
/// Height of the velocity zone at the bottom.
const VELOCITY_ZONE_HEIGHT: f32 = 40.0;
/// Horizontal zoom: pixels per beat in the piano roll.
const PR_PIXELS_PER_BEAT: f32 = 60.0;
/// Resize grab zone width (pixels from right edge).
const RESIZE_GRAB_ZONE: f32 = 10.0;
/// Height of the automation zone (below velocity zone).
const AUTOMATION_ZONE_HEIGHT: f32 = 80.0;
/// Radius for automation point circles.
const AUTOMATION_POINT_RADIUS: f32 = 4.0;

/// Grid resolution options: (display label, ticks). Uses `Duration` constants.
const GRID_RESOLUTIONS: &[(&str, u32)] = &[
    ("Auto", 0),
    ("1/4", SeqDuration::QUARTER.0),
    ("1/8", SeqDuration::EIGHTH.0),
    ("1/16", SeqDuration::SIXTEENTH.0),
    ("1/32", SeqDuration::THIRTY_SECOND.0),
];
/// Hit radius for automation point click detection.
const AUTOMATION_HIT_RADIUS: f32 = 8.0;

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
    view_state: &mut SequencerViewState,
) -> bool {
    use egui_remixicon::icons as ri;
    let t = theme();
    let is_playing = handle.state.transport.is_playing();
    let current_ticks = handle.state.transport.get_ticks();
    let current_tick = Tick(current_ticks);
    let tempo_f32 = handle.state.transport.get_tempo().as_f32();
    let rec_state = RecordingState::from_u32(handle.state.transport.recording_state());
    let metro_on = handle.state.transport.is_metronome_on();

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
            .button(RichText::new(ri::SKIP_BACK_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Go to start")
            .clicked()
        {
            handle.send(EngineCommand::Seek { tick: Tick::ZERO });
        }

        // Play / Pause toggle
        if is_playing {
            if ui
                .button(RichText::new(ri::PAUSE_FILL).color(t.colors.accent_yellow))
                .on_hover_text("Pause")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else if ui
            .button(RichText::new(ri::PLAY_FILL).color(t.colors.accent_green))
            .on_hover_text("Play")
            .clicked()
        {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }

        // Stop (rewinds to beginning)
        if ui
            .button(RichText::new(ri::STOP_FILL).color(if is_playing {
                t.colors.accent_red
            } else {
                t.colors.text_dim
            }))
            .on_hover_text("Stop")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
            view_state.auto_follow_playhead = true;
        }

        // Record button
        let has_pattern = view_state.opened_pattern.is_some();
        let dim_red = Color32::from_rgb(120, 40, 40);
        let rec_color = match rec_state {
            RecordingState::Capturing => t.colors.accent_red,
            RecordingState::CountIn => {
                let blink = ((ui.input(|i| i.time) * 4.0) as u64).is_multiple_of(2);
                if blink {
                    t.colors.accent_red
                } else {
                    t.colors.text_dim
                }
            }
            RecordingState::Armed => {
                let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);
                if blink { t.colors.accent_red } else { dim_red }
            }
            RecordingState::Idle => {
                if has_pattern {
                    dim_red
                } else {
                    t.colors.text_dim
                }
            }
        };
        let rec_btn = ui.add_enabled(
            has_pattern,
            egui::Button::new(RichText::new(ri::RECORD_CIRCLE_FILL).color(rec_color)),
        );
        if rec_btn
            .on_hover_text(match rec_state {
                RecordingState::Idle => "Arm recording",
                _ => "Disarm recording",
            })
            .clicked()
        {
            if rec_state != RecordingState::Idle {
                handle.send(EngineCommand::DisarmRecord);
            } else if let Some(pattern_id) = view_state.opened_pattern {
                // Arm — look up placement bounds and time signature
                // Prefer placement on selected track, fall back to first placement
                let bounds = song.try_read().ok().and_then(|s| {
                    let mut best: Option<(Tick, SeqDuration, SeqDuration, TrackId)> = None;
                    for p in s.arrangement() {
                        if p.pattern_id == pattern_id {
                            let pat = s.pattern(pattern_id)?;
                            let tpb = SeqDuration(s.time_signature_at(p.start).ticks_per_bar());
                            let is_selected_track =
                                view_state.selected_track == Some(p.track_id);
                            if best.is_none() || is_selected_track {
                                best = Some((p.start, pat.length, tpb, p.track_id));
                            }
                            if is_selected_track {
                                break;
                            }
                        }
                    }
                    best
                });
                if let Some((region_start, pattern_length, ticks_per_bar, track_id)) = bounds {
                    // Capture instrument at arm time so it doesn't change during recording
                    view_state.recording_instrument = view_state.selected_instrument;
                    handle.send(EngineCommand::ArmRecord {
                        pattern_id,
                        track_id,
                        region_start,
                        pattern_length,
                        ticks_per_bar,
                        quantize_grid: SeqDuration(view_state.record_quantize),
                        overdub: view_state.overdub,
                    });
                }
            }
        }
        // Request repaint during blinking states
        if matches!(rec_state, RecordingState::Armed | RecordingState::CountIn) {
            ui.ctx().request_repaint();
        }

        // Metronome toggle
        let metro_color = if metro_on {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new("M").strong().color(metro_color))
            .on_hover_text(if metro_on {
                "Metronome off"
            } else {
                "Metronome on"
            })
            .clicked()
        {
            handle.send(EngineCommand::SetMetronome(!metro_on));
        }

        // Quantize button — cycles Off → 1/4 → 1/8 → 1/16 → 1/32
        let q_label = match view_state.record_quantize {
            960 => "Q:1/4",
            480 => "Q:1/8",
            240 => "Q:1/16",
            120 => "Q:1/32",
            _ => "Q",
        };
        let q_color = if view_state.record_quantize > 0 {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new(q_label).strong().color(q_color))
            .on_hover_text(match view_state.record_quantize {
                960 => "Quantize: 1/4 note (click to cycle)",
                480 => "Quantize: 1/8 note (click to cycle)",
                240 => "Quantize: 1/16 note (click to cycle)",
                120 => "Quantize: 1/32 note (click to cycle)",
                _ => "Quantize: Off (click to cycle)",
            })
            .clicked()
        {
            view_state.record_quantize = match view_state.record_quantize {
                0 => 960,
                960 => 480,
                480 => 240,
                240 => 120,
                _ => 0,
            };
        }

        // Overdub toggle
        let ovr_color = if view_state.overdub {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new("OVR").strong().color(ovr_color))
            .on_hover_text(if view_state.overdub {
                "Overdub on (click for replace)"
            } else {
                "Overdub off (click to layer)"
            })
            .clicked()
        {
            view_state.overdub = !view_state.overdub;
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

        // Song repeat toggle
        let repeat_icon = if view_state.repeat_enabled {
            RichText::new(ri::REPEAT_FILL).color(t.colors.accent_primary)
        } else {
            RichText::new(ri::REPEAT_LINE).color(t.colors.text_dim)
        };
        if ui
            .button(repeat_icon)
            .on_hover_text(if view_state.repeat_enabled {
                "Disable song repeat"
            } else {
                "Repeat song"
            })
            .clicked()
        {
            view_state.repeat_enabled = !view_state.repeat_enabled;
            handle.send(EngineCommand::SetRepeat {
                enabled: view_state.repeat_enabled,
            });
        }

        ui.separator();

        // Status indicator
        match rec_state {
            RecordingState::Capturing => {
                ui.label(RichText::new("REC").color(t.colors.accent_red).strong());
            }
            RecordingState::CountIn => {
                ui.label(
                    RichText::new("COUNT-IN")
                        .color(t.colors.accent_red)
                        .strong(),
                );
            }
            RecordingState::Armed => {
                ui.label(RichText::new("ARM").color(Color32::from_rgb(180, 60, 60)));
            }
            RecordingState::Idle => {
                if is_playing {
                    ui.label(RichText::new("PLAYING").color(t.colors.meter_green));
                } else if current_ticks > 0 {
                    ui.label(RichText::new("PAUSED").color(t.colors.accent_yellow));
                } else {
                    ui.label(RichText::new("STOPPED").color(t.colors.text_dim));
                }
            }
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
    instrument_id: Option<SeqInstrumentId>,
}

/// Snapshot of a pattern in the song (for pattern management UI).
struct PatternInfo {
    id: PatternId,
    name: String,
    length_ticks: u32,
}

/// A miniature note rectangle for pattern preview.
struct NoteMiniature {
    /// Horizontal position as fraction of pattern length (0.0–1.0).
    start_frac: f32,
    /// Width as fraction of pattern length.
    duration_frac: f32,
    /// Vertical position as fraction of pitch range (0.0 = lowest, 1.0 = highest).
    pitch_frac: f32,
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
    /// Length in beats (for tooltip).
    length_beats: f32,
    /// Note miniatures for preview drawing.
    note_miniatures: Vec<NoteMiniature>,
}

/// Collected song data for arrangement rendering.
struct ArrangementData {
    tracks: Vec<TrackInfo>,
    placements: Vec<PlacementInfo>,
    patterns: Vec<PatternInfo>,
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
            instrument_id: t.instrument,
        })
        .collect();

    let patterns: Vec<PatternInfo> = song
        .patterns()
        .map(|p| PatternInfo {
            id: p.id,
            name: p.name.clone(),
            length_ticks: p.length.0,
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

            let length_beats = pattern.length.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;

            // Build note miniatures for preview
            let notes = pattern.notes();
            let note_miniatures = if notes.is_empty() || pattern.length.0 == 0 {
                Vec::new()
            } else {
                let len = pattern.length.0 as f32;
                // Find pitch range for normalization
                let (min_pitch, max_pitch) = notes.iter().fold((127_u8, 0_u8), |(lo, hi), n| {
                    let p = n.pitch.as_midi();
                    (lo.min(p), hi.max(p))
                });
                let pitch_range = (max_pitch - min_pitch).max(1) as f32;

                notes
                    .iter()
                    .take(MAX_MINIATURE_NOTES)
                    .map(|n| {
                        let dur = n.duration.map_or(len * 0.02, |d| d.0 as f32);
                        NoteMiniature {
                            start_frac: n.start.0 as f32 / len,
                            duration_frac: dur / len,
                            pitch_frac: (n.pitch.as_midi() - min_pitch) as f32 / pitch_range,
                        }
                    })
                    .collect()
            };

            Some(PlacementInfo {
                pattern_id: p.pattern_id,
                track_id: p.track_id,
                start_tick: p.start.0,
                end_tick: end.0,
                pattern_name: pattern.name.clone(),
                note_count: pattern.notes().len(),
                color,
                length_beats,
                note_miniatures,
            })
        })
        .collect();

    let time_sig = song.default_time_signature;

    Some(ArrangementData {
        tracks,
        placements,
        patterns,
        time_sig,
        song_end_tick,
    })
}

/// Draw the arrangement view with track headers and timeline.
/// Returns `Some(PatternId)` if a placement was double-clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn draw_arrangement(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    current_tick: u64,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
) -> Option<PatternId> {
    use egui_remixicon::icons as ri;
    let t = theme();
    let track_count = data.tracks.len();

    // ── Empty state: show "Add Track" button ──
    if track_count == 0 {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Empty song")
                    .size(16.0)
                    .color(t.colors.text_dim),
            );
            ui.add_space(8.0);
            if ui
                .button(
                    RichText::new(format!("{} Add Track", ri::ADD_LINE))
                        .color(t.colors.accent_green),
                )
                .clicked()
                && let Ok(mut song_w) = song.write()
            {
                song_w.create_track("Track 1");
            }
        });
        return None;
    }

    // Calculate timeline extent
    let ticks_per_bar = data.time_sig.ticks_per_bar() as u64;
    let ticks_per_beat = data.time_sig.ticks_per_beat() as u64;
    let beats_per_bar = data.time_sig.numerator as u64;

    let pixels_per_beat = PIXELS_PER_BEAT * view_state.zoom_level;

    let song_bars = if ticks_per_bar > 0 {
        data.song_end_tick.div_ceil(ticks_per_bar) as u32
    } else {
        MIN_VISIBLE_BARS
    };
    let total_bars = song_bars.max(MIN_VISIBLE_BARS) + 2;
    let total_beats = total_bars as f32 * beats_per_bar as f32;
    let timeline_width = total_beats * pixels_per_beat;

    let mut double_clicked_pattern: Option<PatternId> = None;

    // ── Track header panel (left side, uses egui widgets) ──
    egui::SidePanel::left("seq_track_headers")
        .exact_width(TRACK_HEADER_WIDTH)
        .resizable(false)
        .show_inside(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            // Ruler corner placeholder
            ui.allocate_space(Vec2::new(TRACK_HEADER_WIDTH, RULER_HEIGHT));

            for (i, track) in data.tracks.iter().enumerate() {
                let is_selected = view_state.selected_track == Some(track.id);
                let is_highlighted = view_state.highlighted_track == Some(track.id);
                let bg = if is_selected {
                    Color32::from_rgba_premultiplied(80, 140, 220, 50)
                } else if is_highlighted {
                    Color32::from_rgba_premultiplied(80, 120, 200, 40)
                } else if i % 2 == 0 {
                    t.colors.bg_module
                } else {
                    t.colors.bg_panel
                };

                let frame = egui::Frame::new()
                    .fill(bg)
                    .stroke(if is_selected {
                        Stroke::new(1.0, t.colors.accent_cyan)
                    } else {
                        Stroke::NONE
                    })
                    .inner_margin(egui::Margin::symmetric(4, 2));

                frame.show(ui, |ui| {
                    ui.set_min_height(TRACK_ROW_HEIGHT - 4.0);
                    ui.set_max_height(TRACK_ROW_HEIGHT - 4.0);
                    ui.set_min_width(ui.available_width());

                    // Color indicator + name row
                    ui.horizontal(|ui| {
                        // Color indicator
                        let (color_rect, _) = ui.allocate_exact_size(
                            Vec2::new(4.0, TRACK_ROW_HEIGHT - 8.0),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(color_rect, 2.0, track.color);

                        ui.vertical(|ui| {
                            // Track name (editable on click)
                            if view_state.editing_track_name.as_ref().map(|(id, _)| *id)
                                == Some(track.id)
                            {
                                if let Some((_, ref mut name_buf)) = view_state.editing_track_name {
                                    let resp = ui.add(
                                        egui::TextEdit::singleline(name_buf)
                                            .desired_width(80.0)
                                            .font(egui::FontId::proportional(12.0)),
                                    );
                                    if resp.lost_focus()
                                        || ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        let new_name = name_buf.clone();
                                        let tid = track.id;
                                        if let Ok(mut song_w) = song.write()
                                            && let Some(t) = song_w.track_mut(tid)
                                        {
                                            t.name = new_name;
                                        }
                                        view_state.editing_track_name = None;
                                    } else {
                                        resp.request_focus();
                                    }
                                }
                            } else {
                                let name_resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(&track.name)
                                            .size(12.0)
                                            .color(t.colors.text_primary),
                                    )
                                    .sense(Sense::click()),
                                );
                                if name_resp.clicked() {
                                    view_state.selected_track = Some(track.id);
                                }
                                if name_resp.double_clicked() {
                                    view_state.editing_track_name =
                                        Some((track.id, track.name.clone()));
                                }

                                // Right-click context menu on track name
                                name_resp.context_menu(|ui| {
                                    if ui.button("Rename").clicked() {
                                        view_state.editing_track_name =
                                            Some((track.id, track.name.clone()));
                                        ui.close();
                                    }
                                    if ui
                                        .button(
                                            RichText::new("Delete Track")
                                                .color(t.colors.accent_red),
                                        )
                                        .clicked()
                                    {
                                        if let Ok(mut song_w) = song.write() {
                                            song_w.delete_track(track.id);
                                        }
                                        // close even if write fails
                                        ui.close();
                                    }
                                });
                            }

                            // Mute / Solo row
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                // Mute button
                                let m_color = if track.mute {
                                    t.colors.accent_red
                                } else {
                                    t.colors.text_dim
                                };
                                if ui
                                    .button(RichText::new("M").size(10.0).color(m_color))
                                    .on_hover_text("Mute")
                                    .clicked()
                                    && let Ok(mut song_w) = song.write()
                                    && let Some(trk) = song_w.track_mut(track.id)
                                {
                                    trk.toggle_mute();
                                }

                                // Solo button
                                let s_color = if track.solo {
                                    t.colors.accent_yellow
                                } else {
                                    t.colors.text_dim
                                };
                                if ui
                                    .button(RichText::new("S").size(10.0).color(s_color))
                                    .on_hover_text("Solo")
                                    .clicked()
                                    && let Ok(mut song_w) = song.write()
                                    && let Some(trk) = song_w.track_mut(track.id)
                                {
                                    trk.toggle_solo();
                                }
                            });
                        });
                    });
                });
            }

            // "+" button to add track
            ui.add_space(4.0);
            if ui
                .button(
                    RichText::new(format!("{} Add Track", ri::ADD_LINE))
                        .size(11.0)
                        .color(t.colors.accent_green),
                )
                .clicked()
                && let Ok(mut song_w) = song.write()
            {
                let count = song_w.track_count();
                song_w.create_track(format!("Track {}", count + 1));
            }
        });

    // ── Timeline area (right side, uses painter for performance) ──
    // The scroll area's actual ID = ui.make_persistent_id(Id::new(salt))
    let scroll_salt = "seq_scroll";
    let scroll_id = ui.make_persistent_id(egui::Id::new(scroll_salt));

    // Pre-set scroll offset for auto-follow before showing the scroll area
    if is_playing && view_state.auto_follow_playhead && ticks_per_beat > 0 {
        let playhead_beats = current_tick as f32 / ticks_per_beat as f32;
        let playhead_x_offset = playhead_beats * pixels_per_beat;
        let visible_width = ui.available_width();
        // Keep playhead at ~30% from the right edge
        let target_offset = (playhead_x_offset - visible_width * 0.7).max(0.0);
        // Write scroll state directly
        let mut scroll_state =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
        scroll_state.offset.x = target_offset;
        scroll_state.store(ui.ctx(), scroll_id);
        view_state.last_auto_scroll_offset = Some(target_offset);
    }

    let scroll_output = egui::ScrollArea::horizontal()
        .id_salt(scroll_salt)
        .show(ui, |ui| {
            let total_size = Vec2::new(
                timeline_width,
                RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
            );
            let (response, painter) = ui.allocate_painter(total_size, Sense::click_and_drag());
            let painter_rect = response.rect;

            let tl_x = painter_rect.min.x;
            let tl_y = painter_rect.min.y;

            let tick_to_x = |tick_val: u64| -> f32 {
                if ticks_per_beat == 0 {
                    return tl_x;
                }
                let beats = tick_val as f32 / ticks_per_beat as f32;
                tl_x + beats * pixels_per_beat
            };

            // Helper: x position to tick
            let x_to_tick = |x: f32| -> u64 {
                if ticks_per_beat == 0 {
                    return 0;
                }
                let beats = (x - tl_x) / pixels_per_beat;
                (beats * ticks_per_beat as f32).max(0.0) as u64
            };

            // Helper: y position to track row index
            let y_to_row = |y: f32| -> Option<usize> {
                let row_offset = y - (tl_y + RULER_HEIGHT);
                if row_offset < 0.0 || TRACK_ROW_HEIGHT <= 0.0 {
                    return None;
                }
                let idx = (row_offset / TRACK_ROW_HEIGHT) as usize;
                if idx < track_count { Some(idx) } else { None }
            };

            // ── Ruler (bar/beat numbers) ──
            let ruler_rect = Rect::from_min_size(
                Pos2::new(tl_x, tl_y),
                Vec2::new(timeline_width, RULER_HEIGHT),
            );
            painter.rect_filled(ruler_rect, 0.0, t.colors.bg_dark);

            for bar_idx in 0..total_bars {
                let bar_tick = bar_idx as u64 * ticks_per_bar;
                let x = tick_to_x(bar_tick);

                painter.text(
                    Pos2::new(x + 4.0, tl_y + 4.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}", bar_idx + 1),
                    egui::FontId::proportional(12.0),
                    t.colors.text_secondary,
                );

                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                painter.line_segment(
                    [Pos2::new(x, tl_y + RULER_HEIGHT), Pos2::new(x, line_bottom)],
                    Stroke::new(1.0, t.colors.border),
                );

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

            // ── Track row backgrounds ──
            for i in 0..track_count {
                let row_y = tl_y + RULER_HEIGHT + i as f32 * TRACK_ROW_HEIGHT;
                let is_highlighted = view_state.highlighted_track == Some(data.tracks[i].id);
                let bg = if is_highlighted {
                    Color32::from_rgba_premultiplied(80, 120, 200, 40)
                } else if i % 2 == 0 {
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
                // Row separator
                painter.line_segment(
                    [
                        Pos2::new(tl_x, row_y + TRACK_ROW_HEIGHT),
                        Pos2::new(tl_x + timeline_width, row_y + TRACK_ROW_HEIGHT),
                    ],
                    Stroke::new(0.5, t.colors.border),
                );
            }

            // ── Pattern placements ──
            let mut placement_rects: Vec<(Rect, PatternId, TrackId, u64)> = Vec::new();

            for placement in &data.placements {
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

                placement_rects.push((
                    rect,
                    placement.pattern_id,
                    placement.track_id,
                    placement.start_tick,
                ));

                let is_opened = view_state.opened_pattern == Some(placement.pattern_id);
                let fill_alpha = if is_opened { 140 } else { 100 };
                let fill = Color32::from_rgba_unmultiplied(
                    placement.color.r(),
                    placement.color.g(),
                    placement.color.b(),
                    fill_alpha,
                );
                painter.rect_filled(rect, 3.0, fill);
                let stroke = if is_opened {
                    Stroke::new(2.0, t.colors.accent_cyan)
                } else {
                    Stroke::new(1.0, placement.color)
                };
                painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);

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
                }

                // ── Note miniatures ──
                if !placement.note_miniatures.is_empty() {
                    let mini_top = rect.min.y + 18.0;
                    let mini_height = rect.max.y - mini_top - 2.0;
                    if mini_height > 4.0 {
                        let mini_width = rect.width() - 4.0;
                        let note_color = Color32::from_rgba_unmultiplied(
                            placement.color.r(),
                            placement.color.g(),
                            placement.color.b(),
                            180,
                        );
                        let clipped = painter.with_clip_rect(rect);
                        for mini in &placement.note_miniatures {
                            let nx = rect.min.x + 2.0 + mini.start_frac * mini_width;
                            let nw = (mini.duration_frac * mini_width).max(1.0);
                            let ny = mini_top + (1.0 - mini.pitch_frac) * (mini_height - 2.0);
                            clipped.rect_filled(
                                Rect::from_min_size(Pos2::new(nx, ny), Vec2::new(nw, 2.0)),
                                0.0,
                                note_color,
                            );
                        }
                    }
                }
            }

            // ── Double-click → open piano roll or create pattern ──
            if response.double_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut hit_placement = false;
                for (rect, pattern_id, _, _) in &placement_rects {
                    if rect.contains(pos) {
                        double_clicked_pattern = Some(*pattern_id);
                        hit_placement = true;
                        break;
                    }
                }
                // Double-click on empty area → create pattern + place + open
                if !hit_placement && let Some(row_idx) = y_to_row(pos.y) {
                    let target_track = data.tracks[row_idx].id;
                    let click_tick = x_to_tick(pos.x);
                    let bar_tick = if ticks_per_bar > 0 {
                        (click_tick / ticks_per_bar) * ticks_per_bar
                    } else {
                        click_tick
                    };
                    if let Ok(mut song_w) = song.write() {
                        let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                        song_w.place_pattern(new_pat_id, target_track, Tick(bar_tick));
                        double_clicked_pattern = Some(new_pat_id);
                    }
                }
            }

            // ── Hover hint + tooltip on placements ──
            if response.hovered()
                && let Some(pos) = ui.ctx().pointer_hover_pos()
            {
                let hovered_placement = data
                    .placements
                    .iter()
                    .zip(placement_rects.iter())
                    .find(|(_, (r, _, _, _))| r.contains(pos));

                if let Some((pl, _)) = hovered_placement {
                    ui.ctx().output_mut(|o| {
                        o.cursor_icon = CursorIcon::PointingHand;
                    });
                    // Tooltip with pattern info
                    let instr_name = data
                        .tracks
                        .iter()
                        .find(|t| t.id == pl.track_id)
                        .and_then(|t| t.instrument_id)
                        .and_then(|seq_id| {
                            instruments.iter().find(|inst| inst.id.0 == seq_id.0 as u64)
                        })
                        .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
                    let tip_name = pl.pattern_name.clone();
                    let tip_beats = pl.length_beats;
                    let tip_notes = pl.note_count;
                    response.clone().on_hover_ui(|ui: &mut egui::Ui| {
                        ui.label(
                            RichText::new(&tip_name)
                                .strong()
                                .color(t.colors.text_primary),
                        );
                        ui.label(format!("{tip_beats:.1} beats"));
                        ui.label(format!("{tip_notes} notes"));
                        ui.label(format!("Instrument: {instr_name}"));
                    });
                }
            }

            // ── Ctrl+scroll → timeline zoom ──
            if response.hovered() {
                let scroll_delta = ui.input(|i| {
                    if i.modifiers.ctrl || i.modifiers.command {
                        i.smooth_scroll_delta.y
                    } else {
                        0.0
                    }
                });
                if scroll_delta != 0.0 {
                    let factor = 1.0 + scroll_delta * 0.002;
                    view_state.zoom_level =
                        (view_state.zoom_level * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }

            // ── Primary click: ruler seek or clear highlight ──
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                if ruler_rect.contains(pos) {
                    // Click in ruler → seek to that position
                    let seek_tick = x_to_tick(pos.x);
                    handle.send(EngineCommand::Seek {
                        tick: Tick(seek_tick),
                    });
                    // Re-enable auto-follow on ruler click
                    view_state.auto_follow_playhead = true;
                } else {
                    view_state.highlighted_track = None;
                }
            }

            // ── Ruler hover: pointing hand cursor + indicator line ──
            if response.hovered()
                && let Some(pos) = ui.ctx().pointer_hover_pos()
                && ruler_rect.contains(pos)
            {
                ui.ctx().output_mut(|o| {
                    o.cursor_icon = CursorIcon::PointingHand;
                });
                // Draw subtle hover indicator line
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                painter.line_segment(
                    [Pos2::new(pos.x, tl_y), Pos2::new(pos.x, line_bottom)],
                    Stroke::new(1.0, t.colors.text_dim.gamma_multiply(0.4)),
                );
            }

            // ── Capture right-click position + set highlighted track ──
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                view_state.context_menu_pos = Some(pos);
                view_state.highlighted_track = y_to_row(pos.y).map(|i| data.tracks[i].id);
            }

            // ── Drag-to-move placements ──
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                // Check if dragging on an existing placement
                if let Some((_, pat_id, trk_id, start_tick)) =
                    placement_rects.iter().find(|(r, _, _, _)| r.contains(pos))
                {
                    let grab_tick = x_to_tick(pos.x);
                    view_state.drag = Some(DragState::DragPlacement {
                        pattern_id: *pat_id,
                        track_id: *trk_id,
                        start_tick: Tick(*start_tick),
                        current_tick: Tick(*start_tick),
                        current_track_id: *trk_id,
                        grab_offset_ticks: Tick(grab_tick.saturating_sub(*start_tick)),
                    });
                }
            }

            // ── Update drag state ──
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(DragState::DragPlacement {
                    current_tick,
                    current_track_id,
                    grab_offset_ticks,
                    ..
                }) = &mut view_state.drag
                && let Some(pos) = response.interact_pointer_pos()
            {
                let raw_tick = x_to_tick(pos.x).saturating_sub(grab_offset_ticks.0);
                // Snap to beat grid
                *current_tick = Tick(if ticks_per_beat > 0 {
                    (raw_tick / ticks_per_beat) * ticks_per_beat
                } else {
                    raw_tick
                });
                if let Some(row_idx) = y_to_row(pos.y) {
                    *current_track_id = data.tracks[row_idx].id;
                }
            }

            // ── Release drag → move placement ──
            if response.drag_stopped_by(egui::PointerButton::Primary)
                && let Some(DragState::DragPlacement {
                    pattern_id,
                    track_id,
                    start_tick,
                    current_tick,
                    current_track_id,
                    ..
                }) = view_state.drag.take()
            {
                // Only move if something changed
                if (current_tick != start_tick || current_track_id != track_id)
                    && let Ok(mut song_w) = song.write()
                {
                    song_w.move_placement(
                        pattern_id,
                        track_id,
                        start_tick,
                        current_track_id,
                        current_tick,
                    );
                }
            }

            // ── Draw drag ghost ──
            if let Some(DragState::DragPlacement {
                pattern_id,
                current_tick,
                current_track_id,
                ..
            }) = &view_state.drag
            {
                // Find original placement length
                if let Some(placement) =
                    data.placements.iter().find(|p| p.pattern_id == *pattern_id)
                {
                    let duration_ticks = placement.end_tick - placement.start_tick;
                    let ghost_x = tick_to_x(current_tick.0);
                    let ghost_end_x = tick_to_x(current_tick.0 + duration_ticks);
                    if let Some(row_idx) =
                        data.tracks.iter().position(|t| t.id == *current_track_id)
                    {
                        let ghost_y = tl_y
                            + RULER_HEIGHT
                            + row_idx as f32 * TRACK_ROW_HEIGHT
                            + PLACEMENT_PADDING;
                        let ghost_rect = Rect::from_min_size(
                            Pos2::new(ghost_x, ghost_y),
                            Vec2::new(
                                (ghost_end_x - ghost_x).max(4.0),
                                TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0,
                            ),
                        );
                        painter.rect_filled(
                            ghost_rect,
                            3.0,
                            Color32::from_rgba_unmultiplied(120, 180, 255, 60),
                        );
                        painter.rect_stroke(
                            ghost_rect,
                            3.0,
                            Stroke::new(1.5, Color32::from_rgb(120, 180, 255)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }

            // ── Right-click context menu on timeline ──
            // Use stored position from secondary_clicked, not current hover
            let ctx_pos = view_state.context_menu_pos;
            response.context_menu(|ui| {
                ui.set_min_width(180.0);
                let hover_pos = ctx_pos.unwrap_or(ui.min_rect().min);

                // Check if right-click is on an existing placement
                let clicked_placement = placement_rects
                    .iter()
                    .find(|(r, _, _, _)| r.contains(hover_pos));

                if let Some((_, pat_id, trk_id, start_tick)) = clicked_placement {
                    let pat_id = *pat_id;
                    let trk_id = *trk_id;
                    let start_tick = *start_tick;

                    if ui.button("Open in Piano Roll").clicked() {
                        double_clicked_pattern = Some(pat_id);
                        ui.close();
                    }
                    if ui.button("Rename Pattern").clicked() {
                        // Get current name
                        let current_name = data
                            .patterns
                            .iter()
                            .find(|p| p.id == pat_id)
                            .map_or_else(String::new, |p| p.name.clone());
                        view_state.editing_pattern_name = Some((pat_id, current_name));
                        ui.close();
                    }

                    // Pattern length editing
                    ui.menu_button("Set Length", |ui| {
                        for &(label, bars) in &[
                            ("1 bar", 1_u32),
                            ("2 bars", 2),
                            ("4 bars", 4),
                            ("8 bars", 8),
                            ("16 bars", 16),
                        ] {
                            if ui.button(label).clicked() {
                                let new_len = SeqDuration::WHOLE * bars;
                                if let Ok(mut song_w) = song.write()
                                    && let Some(pat) = song_w.pattern_mut(pat_id)
                                {
                                    pat.length = new_len;
                                }
                                ui.close();
                            }
                        }
                    });

                    if ui.button("Duplicate Pattern").clicked() {
                        if let Ok(mut song_w) = song.write()
                            && let Some(new_id) = song_w.duplicate_pattern(pat_id)
                        {
                            let pattern_length = song_w
                                .pattern(pat_id)
                                .map_or(SeqDuration::WHOLE, |p| p.length);
                            song_w.place_pattern(
                                new_id,
                                trk_id,
                                Tick(start_tick + pattern_length.0 as u64),
                            );
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Remove from Timeline").clicked() {
                        if let Ok(mut song_w) = song.write() {
                            song_w.remove_placement(pat_id, trk_id, Tick(start_tick));
                        }
                        ui.close();
                    }
                    if ui
                        .button(RichText::new("Delete Pattern").color(t.colors.accent_red))
                        .clicked()
                    {
                        if let Ok(mut song_w) = song.write() {
                            song_w.delete_pattern(pat_id);
                        }
                        ui.close();
                    }
                } else {
                    // Clicked on empty area — figure out track + tick
                    let row_offset = hover_pos.y - (tl_y + RULER_HEIGHT);
                    let row_idx = if TRACK_ROW_HEIGHT > 0.0 {
                        (row_offset / TRACK_ROW_HEIGHT) as usize
                    } else {
                        0
                    };
                    let click_tick = x_to_tick(hover_pos.x);
                    // Quantize to bar boundary
                    let bar_tick = if ticks_per_bar > 0 {
                        (click_tick / ticks_per_bar) * ticks_per_bar
                    } else {
                        click_tick
                    };

                    if row_idx < data.tracks.len() {
                        let target_track = data.tracks[row_idx].id;
                        ui.label(
                            RichText::new(format!(
                                "Bar {}",
                                if ticks_per_bar > 0 {
                                    bar_tick / ticks_per_bar + 1
                                } else {
                                    1
                                }
                            ))
                            .color(t.colors.text_dim),
                        );
                        ui.separator();

                        if ui
                            .button(format!("{} New Pattern Here", ri::ADD_LINE))
                            .clicked()
                        {
                            if let Ok(mut song_w) = song.write() {
                                let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                                song_w.place_pattern(new_pat_id, target_track, Tick(bar_tick));
                            }
                            ui.close();
                        }

                        // Place existing pattern submenu
                        if !data.patterns.is_empty() {
                            ui.menu_button("Place Existing Pattern", |ui| {
                                for pat in &data.patterns {
                                    let beats = pat.length_ticks as f32
                                        / synth_sequencer::TICKS_PER_QUARTER as f32;
                                    if ui
                                        .button(format!("{} ({:.0} beats)", pat.name, beats))
                                        .clicked()
                                    {
                                        if let Ok(mut song_w) = song.write() {
                                            song_w.place_pattern(
                                                pat.id,
                                                target_track,
                                                Tick(bar_tick),
                                            );
                                        }
                                        ui.close();
                                    }
                                }
                            });
                        }
                    }
                }
            });

            // ── Playhead ──
            if current_tick > 0 || data.song_end_tick > 0 {
                let playhead_x = tick_to_x(current_tick);
                let line_top = tl_y;
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;

                painter.line_segment(
                    [
                        Pos2::new(playhead_x, line_top),
                        Pos2::new(playhead_x, line_bottom),
                    ],
                    Stroke::new(2.0, t.colors.accent_primary),
                );

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

            // Ruler bottom border
            painter.line_segment(
                [
                    Pos2::new(tl_x, tl_y + RULER_HEIGHT),
                    Pos2::new(tl_x + timeline_width, tl_y + RULER_HEIGHT),
                ],
                Stroke::new(1.0, t.colors.border),
            );
        });

    // Detect manual scrolling to disable auto-follow
    if is_playing {
        let actual_offset = scroll_output.state.offset.x;
        if let Some(expected) = view_state.last_auto_scroll_offset {
            // If user scrolled manually (offset differs significantly from what we set)
            if (actual_offset - expected).abs() > 2.0 {
                view_state.auto_follow_playhead = false;
                view_state.last_auto_scroll_offset = None;
            }
        }
    }

    // Re-enable auto-follow when playback starts from stopped
    if !is_playing {
        view_state.last_auto_scroll_offset = None;
    }

    double_clicked_pattern
}

// ============================================================================
// PIANO ROLL
// ============================================================================

/// Snapshot of a note for piano roll rendering.
struct PianoRollNote {
    note_id: NoteId,
    pitch: Pitch,
    start_tick: PatternTick,
    end_tick: Option<PatternTick>,
    velocity: Velocity,
    instrument: SeqInstrumentId,
}

/// Snapshot of a single automation point for rendering.
struct AutomationPointSnapshot {
    tick: PatternTick,
    value: NormalizedValue,
    curve: CurveType,
}

/// Snapshot of an automation lane for rendering.
struct AutomationLaneSnapshot {
    target: AutomationTarget,
    points: Vec<AutomationPointSnapshot>,
}

/// Collected data for piano roll rendering.
struct PianoRollData {
    pattern_name: String,
    pattern_id: PatternId,
    length_ticks: SeqDuration,
    ticks_per_row: u16,
    notes: Vec<PianoRollNote>,
    pitch_min: Pitch,
    pitch_max: Pitch,
    automation_lanes: Vec<AutomationLaneSnapshot>,
}

/// Collect piano roll data from song (short read-lock, then release).
fn collect_piano_roll_data(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
) -> Option<PianoRollData> {
    let song = song.try_read().ok()?;
    let pattern = song.pattern(pattern_id)?;

    let mut pitch_min = Pitch::MAX;
    let mut pitch_max = Pitch::MIN;

    let notes: Vec<PianoRollNote> = pattern
        .notes()
        .iter()
        .map(|n| {
            if n.pitch < pitch_min {
                pitch_min = n.pitch;
            }
            if n.pitch > pitch_max {
                pitch_max = n.pitch;
            }
            PianoRollNote {
                note_id: n.id,
                pitch: n.pitch,
                start_tick: n.start,
                end_tick: n.end(),
                velocity: n.velocity,
                instrument: n.instrument,
            }
        })
        .collect();

    // Default range if no notes
    if pitch_min > pitch_max {
        pitch_min = Pitch::new(48).unwrap_or(Pitch::MIDDLE_C); // C3
        pitch_max = Pitch::new(72).unwrap_or(Pitch::MIDDLE_C); // C5
    }

    let automation_lanes: Vec<AutomationLaneSnapshot> = pattern
        .automation
        .iter()
        .map(|lane| AutomationLaneSnapshot {
            target: lane.target.clone(),
            points: lane
                .points()
                .iter()
                .map(|p| AutomationPointSnapshot {
                    tick: p.tick,
                    value: p.value,
                    curve: p.curve,
                })
                .collect(),
        })
        .collect();

    Some(PianoRollData {
        pattern_name: if pattern.name.is_empty() {
            format!("Pattern {}", pattern_id.0)
        } else {
            pattern.name.clone()
        },
        pattern_id,
        length_ticks: pattern.length,
        ticks_per_row: pattern.row_resolution.ticks_per_row.as_u16(),
        notes,
        pitch_min,
        pitch_max,
        automation_lanes,
    })
}

/// Find the note at the given position, returning its ID and which zone was hit.
fn note_at_pos(
    notes: &[PianoRollNote],
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    pitch_to_y: &dyn Fn(Pitch) -> f32,
    length_ticks: SeqDuration,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
) -> Option<(NoteId, HitZone)> {
    // Iterate in reverse so top-most (last drawn) notes are checked first
    for note in notes.iter().rev() {
        if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
            continue;
        }

        let y = pitch_to_y(note.pitch);
        let x_start = tick_to_x(note.start_tick);
        let x_end = match note.end_tick {
            Some(end) => tick_to_x(end),
            None => tick_to_x(length_ticks.as_pattern_tick()),
        };
        let note_width = (x_end - x_start).max(3.0);

        let note_rect = Rect::from_min_size(
            Pos2::new(x_start, y + 1.0),
            Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
        );

        // Expand tiny notes so they're easier to click
        let hit_rect = if note_width < RESIZE_GRAB_ZONE {
            note_rect.expand2(Vec2::new(2.0, 1.0))
        } else {
            note_rect
        };

        if hit_rect.contains(pos) {
            // Proportional grab zone: at most 30% of note width, so short notes remain movable
            let grab_zone = RESIZE_GRAB_ZONE.min(note_width * 0.3);
            let zone = if pos.x >= note_rect.max.x - grab_zone {
                HitZone::RightEdge
            } else {
                HitZone::Body
            };
            return Some((note.note_id, zone));
        }
    }
    None
}

/// Check if a note already exists at the given tick and pitch.
fn has_note_at(notes: &[PianoRollNote], tick: PatternTick, pitch: Pitch) -> bool {
    notes.iter().any(|n| {
        n.pitch == pitch
            && n.start_tick <= tick
            && n.end_tick.unwrap_or(PatternTick(u32::MAX)) > tick
    })
}

/// Quantize a tick value to the nearest row boundary (floor).
fn quantize_tick(tick: PatternTick, ticks_per_row: u16) -> PatternTick {
    if ticks_per_row == 0 {
        return tick;
    }
    let tpr = ticks_per_row as u32;
    PatternTick((tick.0 / tpr) * tpr)
}

/// Draw the piano roll in a bottom panel.
/// Returns false if the close button was clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn draw_piano_roll(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    playhead_tick: Option<PatternTick>,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) -> bool {
    let t = theme();
    let mut keep_open = true;

    // ── Toolbar ──
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Pattern name (editable via rename context menu)
        if view_state.editing_pattern_name.as_ref().map(|(id, _)| *id) == Some(data.pattern_id) {
            if let Some((_, ref mut name_buf)) = view_state.editing_pattern_name {
                let resp = ui.add(
                    egui::TextEdit::singleline(name_buf)
                        .desired_width(120.0)
                        .font(egui::FontId::proportional(14.0)),
                );
                if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let new_name = name_buf.clone();
                    let pid = data.pattern_id;
                    if let Ok(mut song_w) = song.write()
                        && let Some(pat) = song_w.pattern_mut(pid)
                    {
                        pat.name = new_name;
                    }
                    view_state.editing_pattern_name = None;
                } else {
                    resp.request_focus();
                }
            }
        } else {
            let name_resp = ui.add(
                egui::Label::new(
                    RichText::new(&data.pattern_name)
                        .size(14.0)
                        .color(t.colors.accent_cyan),
                )
                .sense(Sense::click()),
            );
            if name_resp.double_clicked() {
                view_state.editing_pattern_name =
                    Some((data.pattern_id, data.pattern_name.clone()));
            }
        }

        // Instrument selector for new notes
        {
            let selected_label = instruments
                .iter()
                .find(|inst| inst.id.0 == view_state.selected_instrument.0 as u64)
                .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
            egui::ComboBox::from_id_salt(ui.id().with("piano_roll_instrument"))
                .selected_text(RichText::new(&selected_label).size(12.0))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for inst in instruments {
                        let seq_id = SeqInstrumentId::new(inst.id.0 as u16);
                        let selected = view_state.selected_instrument == seq_id;
                        if ui.selectable_label(selected, &inst.name).clicked() {
                            view_state.selected_instrument = seq_id;
                        }
                    }
                });
        }
        ui.separator();

        // Mini-transport controls
        {
            use egui_remixicon::icons as ri;
            ui.spacing_mut().item_spacing.x = 2.0;

            if is_playing {
                // Pause button
                if ui
                    .button(
                        RichText::new(ri::PAUSE_FILL)
                            .size(12.0)
                            .color(t.colors.accent_yellow),
                    )
                    .on_hover_text("Pause")
                    .clicked()
                {
                    handle.send(EngineCommand::Pause);
                }
            } else {
                // Play pattern button
                if ui
                    .button(
                        RichText::new(ri::PLAY_FILL)
                            .size(12.0)
                            .color(t.colors.accent_green),
                    )
                    .on_hover_text(if view_state.pattern_repeat {
                        "Play pattern (loop)"
                    } else {
                        "Play from pattern"
                    })
                    .clicked()
                {
                    if view_state.pattern_repeat {
                        handle.send(EngineCommand::PlayPattern {
                            pattern_id: data.pattern_id,
                        });
                    } else {
                        handle.send(EngineCommand::PlayFromPattern {
                            pattern_id: data.pattern_id,
                        });
                    }
                }
            }

            // Stop button
            if ui
                .button(
                    RichText::new(ri::STOP_FILL)
                        .size(12.0)
                        .color(if is_playing {
                            t.colors.accent_red
                        } else {
                            t.colors.text_dim
                        }),
                )
                .on_hover_text("Stop")
                .clicked()
            {
                handle.send(EngineCommand::Stop);
            }

            // Pattern repeat toggle
            let pattern_repeat_icon = if view_state.pattern_repeat {
                RichText::new(ri::REPEAT_FILL)
                    .size(12.0)
                    .color(t.colors.accent_primary)
            } else {
                RichText::new(ri::REPEAT_LINE)
                    .size(12.0)
                    .color(t.colors.text_dim)
            };
            if ui
                .button(pattern_repeat_icon)
                .on_hover_text(if view_state.pattern_repeat {
                    "Disable pattern repeat"
                } else {
                    "Repeat pattern"
                })
                .clicked()
            {
                view_state.pattern_repeat = !view_state.pattern_repeat;
            }

            ui.spacing_mut().item_spacing.x = 8.0;
        }
        ui.separator();

        // Tool selector
        let select_label = if view_state.edit_tool == EditTool::Select {
            RichText::new("Select").color(t.colors.accent_primary)
        } else {
            RichText::new("Select").color(t.colors.text_secondary)
        };
        if ui.button(select_label).clicked() {
            view_state.edit_tool = EditTool::Select;
        }

        let draw_label = if view_state.edit_tool == EditTool::Draw {
            RichText::new("Draw").color(t.colors.accent_primary)
        } else {
            RichText::new("Draw").color(t.colors.text_secondary)
        };
        if ui.button(draw_label).clicked() {
            view_state.edit_tool = EditTool::Draw;
        }

        ui.separator();

        // Automation lane selector
        ui.label(RichText::new("Auto:").color(t.colors.text_dim));
        {
            // Build label for ComboBox
            let auto_label = view_state
                .selected_automation
                .as_ref()
                .map_or_else(|| "None".to_owned(), AutomationTarget::display_name);

            egui::ComboBox::from_id_salt("auto_lane_select")
                .selected_text(&auto_label)
                .width(110.0)
                .show_ui(ui, |ui| {
                    // "None" option to hide automation zone
                    if ui
                        .selectable_label(view_state.selected_automation.is_none(), "None")
                        .clicked()
                    {
                        view_state.selected_automation = None;
                    }

                    // All instrument params for selected instrument
                    for param in AutoInstrumentParam::ALL {
                        let target = AutomationTarget::Instrument {
                            instrument: view_state.selected_instrument,
                            param: *param,
                        };
                        let has_points = data
                            .automation_lanes
                            .iter()
                            .any(|l| l.target == target && !l.points.is_empty());
                        let label = if has_points {
                            format!("* {}", param.display_name())
                        } else {
                            param.display_name().to_owned()
                        };
                        let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            view_state.selected_automation = Some(target);
                        }
                    }
                });
        }

        ui.separator();

        // ── Grid resolution selector ──
        let grid_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_grid_resolution)
            .map_or("Grid", |(l, _)| l);
        egui::ComboBox::from_id_salt("grid_res")
            .selected_text(grid_label)
            .width(50.0)
            .show_ui(ui, |ui| {
                for &(label, ticks) in GRID_RESOLUTIONS {
                    if ui
                        .selectable_label(view_state.draw_grid_resolution == ticks, label)
                        .clicked()
                    {
                        view_state.draw_grid_resolution = ticks;
                    }
                }
            });

        // ── Note length preset selector ──
        let len_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_note_length)
            .map_or("L:Drag", |(l, _)| l);
        egui::ComboBox::from_id_salt("note_len")
            .selected_text(if view_state.draw_note_length == 0 {
                "L:Drag"
            } else {
                len_label
            })
            .width(55.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(view_state.draw_note_length == 0, "Drag")
                    .clicked()
                {
                    view_state.draw_note_length = 0;
                }
                for &(label, ticks) in &GRID_RESOLUTIONS[1..] {
                    if ui
                        .selectable_label(view_state.draw_note_length == ticks, label)
                        .clicked()
                    {
                        view_state.draw_note_length = ticks;
                    }
                }
            });

        // ── Default velocity ──
        ui.label(RichText::new("Vel:").color(t.colors.text_dim).size(10.0));
        let mut vel_pct = (view_state.default_velocity.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut vel_pct)
                    .range(1.0..=100.0)
                    .speed(1.0)
                    .suffix("%"),
            )
            .changed()
        {
            view_state.default_velocity = Velocity::new((vel_pct / 100.0).max(0.01));
        }

        // ── Quantize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("Q").color(t.colors.accent_primary)),
            )
            .on_hover_text(format!(
                "Quantize selected (strength {:.0}%)",
                view_state.quantize_strength.as_f32() * 100.0
            ))
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            if let Ok(mut song_w) = song.write()
                && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
            {
                pattern.quantize_selected(
                    &view_state.selected_notes,
                    grid,
                    view_state.quantize_strength,
                );
            }
        }

        // ── Quantize strength (small drag value) ──
        let mut q_str_pct = (view_state.quantize_strength.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut q_str_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix("%")
                    .prefix("Str:"),
            )
            .on_hover_text("Quantize strength")
            .changed()
        {
            view_state.quantize_strength = NormalizedValue::new(q_str_pct / 100.0);
        }

        // ── Humanize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("H").color(t.colors.text_secondary)),
            )
            .on_hover_text("Humanize selected notes (random timing/velocity)")
            .clicked()
            && let Ok(mut song_w) = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            pattern.humanize_notes(
                &view_state.selected_notes,
                SeqDuration(15),
                NormalizedValue::new(0.05),
            );
        }

        ui.separator();

        // ── Swing control ──
        ui.label(RichText::new("Sw:").color(t.colors.text_dim).size(10.0));
        let mut sw_pct = (view_state.swing_amount.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut sw_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix("%"),
            )
            .on_hover_text("Swing amount (offset even subdivisions)")
            .changed()
        {
            view_state.swing_amount = NormalizedValue::new(sw_pct / 100.0);
        }
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.swing_amount.as_f32() > 0.0,
                egui::Button::new(RichText::new("Apply").color(t.colors.text_secondary)),
            )
            .on_hover_text("Apply swing to selected notes")
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            if let Ok(mut song_w) = song.write()
                && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
            {
                pattern.apply_swing(&view_state.selected_notes, grid, view_state.swing_amount);
            }
        }

        // ── Scale velocities ──
        ui.add(
            egui::DragValue::new(&mut view_state.velocity_scale_pct)
                .range(1..=200_u32)
                .speed(1.0)
                .suffix("%")
                .prefix("Vel×"),
        )
        .on_hover_text("Velocity scale factor");
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.velocity_scale_pct != 100,
                egui::Button::new(RichText::new("Scale").color(t.colors.text_secondary)),
            )
            .on_hover_text("Scale velocities of selected notes")
            .clicked()
            && let Ok(mut song_w) = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            pattern.scale_velocities(
                &view_state.selected_notes,
                view_state.velocity_scale_pct as f32 / 100.0,
            );
            view_state.velocity_scale_pct = 100;
        }

        // ── Step entry toggle ──
        let step_color = if view_state.step_entry_mode {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new("STEP").color(step_color))
            .on_hover_text("Step entry mode")
            .clicked()
        {
            view_state.step_entry_mode = !view_state.step_entry_mode;
            if view_state.step_entry_mode {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }

        ui.separator();

        ui.label(
            RichText::new(format!("{} notes", data.notes.len())).color(t.colors.text_secondary),
        );

        if !view_state.selected_notes.is_empty() {
            ui.label(
                RichText::new(format!("{} selected", view_state.selected_notes.len()))
                    .color(t.colors.accent_yellow),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new(egui_remixicon::icons::CLOSE_LINE).color(t.colors.accent_red))
                .on_hover_text("Close piano roll")
                .clicked()
            {
                keep_open = false;
            }
        });
    });

    ui.separator();

    // ── Pitch range with margin ──
    let margin = 6;
    let view_pitch_min = data.pitch_min.saturating_sub(margin);
    let view_pitch_max = data.pitch_max.saturating_add(margin);
    let pitch_range = view_pitch_max.as_midi() - view_pitch_min.as_midi() + 1;

    let grid_height = pitch_range as f32 * NOTE_ROW_HEIGHT;
    let auto_height = if view_state.selected_automation.is_some() {
        AUTOMATION_ZONE_HEIGHT
    } else {
        0.0
    };
    let total_content_height = grid_height + VELOCITY_ZONE_HEIGHT + auto_height;

    // Timeline width: use max of pattern length and furthest note end
    let ticks_per_beat = synth_sequencer::TICKS_PER_QUARTER;
    let max_note_end = data
        .notes
        .iter()
        .filter_map(|n| n.end_tick)
        .max()
        .map_or(0, |t| t.0);
    let effective_ticks = data.length_ticks.0.max(max_note_end);
    let beats_in_pattern = if ticks_per_beat > 0 {
        effective_ticks as f32 / ticks_per_beat as f32
    } else {
        4.0
    };
    let grid_width = (beats_in_pattern * PR_PIXELS_PER_BEAT).max(200.0);

    // ── Scrollable piano roll area ──
    // Use all available height (panel is resizable via TopBottomPanel)
    let scroll_max_height = ui.available_height().max(100.0);

    // Auto-follow playhead in piano roll during playback
    let pr_scroll_salt = "piano_roll_scroll";
    let pr_scroll_id = ui.make_persistent_id(egui::Id::new(pr_scroll_salt));
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(pt) = playhead_tick
        && ticks_per_beat > 0
    {
        let playhead_beats = pt.0 as f32 / ticks_per_beat as f32;
        let playhead_x = KEY_WIDTH + playhead_beats * PR_PIXELS_PER_BEAT;
        let visible_width = ui.available_width();
        let target_offset = (playhead_x - visible_width * 0.5).max(0.0);

        if let Some(mut scroll_state) = egui::scroll_area::State::load(ui.ctx(), pr_scroll_id) {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), pr_scroll_id);
        }
    }

    egui::ScrollArea::both()
        .id_salt(pr_scroll_salt)
        .max_height(scroll_max_height)
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: false, // Don't steal drag events — we handle them for note editing
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            let total_size = Vec2::new(KEY_WIDTH + grid_width, total_content_height);

            // Use allocate_rect with click_and_drag sense for mouse interaction
            let alloc_rect = Rect::from_min_size(ui.cursor().min, total_size);
            let response = ui.allocate_rect(alloc_rect, Sense::click_and_drag());
            let rect = response.rect;
            let painter = ui.painter_at(rect);

            let origin = rect.min;
            let grid_x = origin.x + KEY_WIDTH;
            let grid_y = origin.y;

            // Helper: tick to x position
            let tick_to_x = |tick_val: PatternTick| -> f32 {
                if ticks_per_beat == 0 {
                    return grid_x;
                }
                let beats = tick_val.0 as f32 / ticks_per_beat as f32;
                grid_x + beats * PR_PIXELS_PER_BEAT
            };

            // Helper: pitch to y position (higher pitch = lower y, piano style)
            let pitch_to_y = |pitch: Pitch| -> f32 {
                let row = view_pitch_max.as_midi().saturating_sub(pitch.as_midi());
                grid_y + row as f32 * NOTE_ROW_HEIGHT
            };

            // Inverse: x to tick
            let x_to_tick = |x: f32| -> PatternTick {
                #[allow(clippy::cast_possible_truncation)]
                let tick = ((x - grid_x) / PR_PIXELS_PER_BEAT * ticks_per_beat as f32).max(0.0);
                PatternTick(tick as u32)
            };

            // Inverse: y to pitch (clamped to visible range)
            let y_to_pitch = |y: f32| -> Pitch {
                #[allow(clippy::cast_possible_truncation)]
                let row = ((y - grid_y) / NOTE_ROW_HEIGHT).floor().max(0.0) as u8;
                let midi = view_pitch_max
                    .as_midi()
                    .saturating_sub(row)
                    .clamp(view_pitch_min.as_midi(), view_pitch_max.as_midi());
                Pitch::new(midi).unwrap_or(Pitch::MIDDLE_C)
            };

            // Grid rect for checking if pointer is in the note grid area
            let grid_rect = Rect::from_min_size(
                Pos2::new(grid_x, grid_y),
                Vec2::new(grid_width, grid_height),
            );

            // ── Keyboard (left column) ──
            painter.rect_filled(
                Rect::from_min_size(origin, Vec2::new(KEY_WIDTH, grid_height)),
                0.0,
                t.colors.bg_dark,
            );

            for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
                let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
                let y = pitch_to_y(pitch);
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
            for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
                let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
                let y = pitch_to_y(pitch);
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
                let x = tick_to_x(PatternTick(beat_tick));
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
                let total_rows = data.length_ticks.0 / data.ticks_per_row as u32;
                for row in 0..total_rows {
                    let row_tick = row * data.ticks_per_row as u32;
                    // Skip if this aligns with a beat line (already drawn)
                    if row_tick.is_multiple_of(ticks_per_beat) {
                        continue;
                    }
                    let x = tick_to_x(PatternTick(row_tick));
                    painter.line_segment(
                        [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                        Stroke::new(0.3, t.colors.border.gamma_multiply(0.15)),
                    );
                }
            }

            // ── Notes ──
            let note_color = Color32::from_rgb(100, 180, 255);
            let selected_color = Color32::from_rgb(140, 210, 255);

            for note in &data.notes {
                if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                    continue;
                }

                // Skip notes that are being dragged (draw ghost instead)
                let is_being_moved = matches!(
                    &view_state.drag,
                    Some(DragState::MoveNote { note_id, .. }) if *note_id == note.note_id
                );
                if is_being_moved {
                    continue;
                }

                let y = pitch_to_y(note.pitch);
                let x_start = tick_to_x(note.start_tick);

                let x_end = match note.end_tick {
                    Some(end) => tick_to_x(end),
                    None => {
                        // Open-ended: draw to pattern end or at least a visible width
                        tick_to_x(data.length_ticks.as_pattern_tick()).min(x_start + grid_width)
                    }
                };

                // Apply resize preview if this note is being resized
                let x_end = match &view_state.drag {
                    Some(DragState::ResizeNote {
                        note_id,
                        current_end_tick,
                        ..
                    }) if *note_id == note.note_id => tick_to_x(*current_end_tick),
                    _ => x_end,
                };

                let note_width = (x_end - x_start).max(3.0);
                let alpha = (note.velocity.as_f32() * 200.0 + 55.0).min(255.0) as u8;

                let is_selected = view_state.selected_notes.contains(&note.note_id);
                let base_color = if is_selected {
                    selected_color
                } else {
                    note_color
                };

                let fill = Color32::from_rgba_unmultiplied(
                    base_color.r(),
                    base_color.g(),
                    base_color.b(),
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
                    Stroke::new(if is_selected { 1.5 } else { 0.5 }, base_color),
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
                            base_color.r(),
                            base_color.g(),
                            base_color.b(),
                            alpha / 3,
                        ),
                    );
                }
            }

            // ── Recording preview notes (orange) ──
            if !view_state.recording_preview_completed.is_empty()
                || !view_state.recording_preview_held.is_empty()
            {
                let preview_color = Color32::from_rgb(255, 160, 60);

                // Draw completed preview notes
                for note in &view_state.recording_preview_completed {
                    if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                        continue;
                    }
                    let y = pitch_to_y(note.pitch);
                    let x_start = tick_to_x(note.start);
                    let x_end = tick_to_x(note.start + note.duration);
                    let note_width = (x_end - x_start).max(3.0);
                    let alpha = 180_u8;

                    let preview_rect = Rect::from_min_size(
                        Pos2::new(x_start, y + 1.0),
                        Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
                    );
                    painter.rect_filled(
                        preview_rect,
                        2.0,
                        Color32::from_rgba_unmultiplied(
                            preview_color.r(),
                            preview_color.g(),
                            preview_color.b(),
                            alpha,
                        ),
                    );
                    painter.rect_stroke(
                        preview_rect,
                        2.0,
                        Stroke::new(0.5, preview_color),
                        egui::StrokeKind::Inside,
                    );
                }

                // Draw held notes extending from start to current playhead
                if !view_state.recording_preview_held.is_empty() {
                    // Compute playhead position within pattern
                    let playhead_in_pattern = playhead_tick.map_or(0, |pt| pt.0);

                    for (pitch, start_tick) in &view_state.recording_preview_held {
                        if *pitch < view_pitch_min || *pitch > view_pitch_max {
                            continue;
                        }
                        let y = pitch_to_y(*pitch);
                        let x_start = tick_to_x(*start_tick);
                        let end = if playhead_in_pattern >= start_tick.0 {
                            PatternTick(playhead_in_pattern)
                        } else {
                            // Note wraps around pattern — draw to end
                            data.length_ticks.as_pattern_tick()
                        };
                        let x_end = tick_to_x(end);
                        let note_width = (x_end - x_start).max(3.0);

                        let held_rect = Rect::from_min_size(
                            Pos2::new(x_start, y + 1.0),
                            Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
                        );
                        painter.rect_filled(
                            held_rect,
                            2.0,
                            Color32::from_rgba_unmultiplied(255, 160, 60, 140),
                        );
                        painter.rect_stroke(
                            held_rect,
                            2.0,
                            Stroke::new(0.5, preview_color),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                // Request repaint during recording for live updates
                ui.ctx().request_repaint();
            }

            // ── Ghost note for MoveNote drag ──
            if let Some(DragState::MoveNote {
                note_id,
                current_tick: drag_tick,
                current_pitch: drag_pitch,
                ..
            }) = &view_state.drag
            {
                // Find the original note data for velocity/duration
                if let Some(note) = data.notes.iter().find(|n| n.note_id == *note_id) {
                    let duration_ticks = note.end_tick.map_or(
                        data.length_ticks.as_pattern_tick() - note.start_tick,
                        |end| end - note.start_tick,
                    );
                    let y = pitch_to_y(*drag_pitch);
                    let x_start = tick_to_x(*drag_tick);
                    let x_end = tick_to_x(*drag_tick + duration_ticks);
                    let note_width = (x_end - x_start).max(3.0);

                    let ghost_rect = Rect::from_min_size(
                        Pos2::new(x_start, y + 1.0),
                        Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
                    );

                    // Semi-transparent ghost
                    painter.rect_filled(
                        ghost_rect,
                        2.0,
                        Color32::from_rgba_unmultiplied(140, 210, 255, 100),
                    );
                    painter.rect_stroke(
                        ghost_rect,
                        2.0,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(140, 210, 255, 180)),
                        egui::StrokeKind::Inside,
                    );
                }
            }

            // ── DrawNote preview ──
            if let Some(DragState::DrawNote {
                start_tick,
                pitch,
                current_end_tick,
            }) = &view_state.drag
            {
                let y = pitch_to_y(*pitch);
                let x_start = tick_to_x(*start_tick);
                let x_end = tick_to_x(*current_end_tick);
                let note_width = (x_end - x_start).max(3.0);

                let draw_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, NOTE_ROW_HEIGHT - 2.0),
                );
                painter.rect_filled(
                    draw_rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(100, 220, 140, 120),
                );
                painter.rect_stroke(
                    draw_rect,
                    2.0,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(100, 220, 140, 200)),
                    egui::StrokeKind::Inside,
                );
            }

            // ── Selection rectangle ──
            if let Some(DragState::SelectRect {
                start_pos,
                current_pos,
            }) = &view_state.drag
            {
                let sel_rect = Rect::from_two_pos(*start_pos, *current_pos);
                painter.rect_filled(
                    sel_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(100, 180, 255, 30),
                );
                painter.rect_stroke(
                    sel_rect,
                    0.0,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(100, 180, 255, 150)),
                    egui::StrokeKind::Inside,
                );
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
                let bar_height = note.velocity.as_f32() * (VELOCITY_ZONE_HEIGHT - 4.0);
                let bar_y = vel_y + VELOCITY_ZONE_HEIGHT - bar_height - 2.0;

                let is_selected = view_state.selected_notes.contains(&note.note_id);
                let vel_color = if is_selected {
                    Color32::from_rgb(140, 210, 255)
                } else {
                    velocity_color(note.velocity.as_f32())
                };
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x - 1.5, bar_y), Vec2::new(3.0, bar_height)),
                    1.0,
                    vel_color,
                );
            }

            // ── Automation zone (below velocity) ──
            let auto_y = vel_y + VELOCITY_ZONE_HEIGHT;
            if let Some(selected_target) = &view_state.selected_automation {
                draw_automation_zone(
                    &painter,
                    data,
                    view_state,
                    selected_target,
                    grid_x,
                    auto_y,
                    grid_width,
                    &tick_to_x,
                    &t,
                );
            }

            // ── Playhead (only if this pattern is actually playing) ──
            if let Some(pattern_tick) = playhead_tick {
                let playhead_x = tick_to_x(pattern_tick);

                if playhead_x >= grid_x && playhead_x <= grid_x + grid_width {
                    painter.line_segment(
                        [
                            Pos2::new(playhead_x, grid_y),
                            Pos2::new(playhead_x, grid_y + total_content_height),
                        ],
                        Stroke::new(1.5, t.colors.accent_primary),
                    );
                }
            }

            // ── Step cursor ──
            if view_state.step_entry_mode {
                let cursor_x = tick_to_x(view_state.step_cursor_tick);
                if cursor_x >= grid_x && cursor_x <= grid_x + grid_width {
                    painter.line_segment(
                        [
                            Pos2::new(cursor_x, grid_y),
                            Pos2::new(cursor_x, grid_y + grid_height),
                        ],
                        Stroke::new(2.0, Color32::from_rgb(255, 100, 255)),
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

            // Automation zone rect (for hit-testing)
            let auto_rect = if view_state.selected_automation.is_some() {
                Some(Rect::from_min_size(
                    Pos2::new(grid_x, auto_y),
                    Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
                ))
            } else {
                None
            };

            // ── Hover and cursor ──
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                // Check automation zone hover first
                if auto_rect.is_some_and(|r| r.contains(pos)) {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                } else if grid_rect.contains(pos) {
                    let vp_min = view_pitch_min;
                    let vp_max = view_pitch_max;
                    let hit = note_at_pos(
                        &data.notes,
                        pos,
                        &tick_to_x,
                        &pitch_to_y,
                        data.length_ticks,
                        vp_min,
                        vp_max,
                    );

                    match hit {
                        Some((_, HitZone::RightEdge)) => {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeEast);
                        }
                        Some((note_id, HitZone::Body)) => {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);

                            // Subtle hover highlight
                            if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                                let y = pitch_to_y(note.pitch);
                                let x_start = tick_to_x(note.start_tick);
                                let x_end = match note.end_tick {
                                    Some(end) => tick_to_x(end),
                                    None => tick_to_x(data.length_ticks.as_pattern_tick()),
                                };
                                let hover_rect = Rect::from_min_size(
                                    Pos2::new(x_start, y + 1.0),
                                    Vec2::new((x_end - x_start).max(3.0), NOTE_ROW_HEIGHT - 2.0),
                                );
                                painter.rect_stroke(
                                    hover_rect,
                                    2.0,
                                    Stroke::new(
                                        1.0,
                                        Color32::from_rgba_unmultiplied(255, 255, 255, 60),
                                    ),
                                    egui::StrokeKind::Outside,
                                );
                            }
                        }
                        None => {
                            if view_state.edit_tool == EditTool::Draw {
                                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                            }
                        }
                    }
                }
            }

            // ── Mouse interaction ──
            handle_piano_roll_interaction(
                &response,
                ui,
                data,
                song,
                view_state,
                handle,
                grid_rect,
                auto_rect,
                auto_y,
                &x_to_tick,
                &y_to_pitch,
                &tick_to_x,
                &pitch_to_y,
                view_pitch_min,
                view_pitch_max,
                undo_manager,
            );
        });

    // ── Keyboard shortcuts ──
    let ctx = ui.ctx();
    if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        view_state.selected_notes.clear();
        view_state.drag = None;
    }

    // ── Ctrl+A — select all notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
        view_state.selected_notes.clear();
        for note in &data.notes {
            view_state.selected_notes.insert(note.note_id);
        }
    }

    // ── Ctrl+C — copy selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
    }

    // ── Ctrl+X — cut selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::X))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+V — paste at playhead or start ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V))
        && !view_state.clipboard.notes.is_empty()
    {
        let paste_tick = playhead_tick.unwrap_or(PatternTick(0));
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+D — duplicate selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        // Find min tick of selection
        let min_tick = data
            .notes
            .iter()
            .filter(|n| view_state.selected_notes.contains(&n.note_id))
            .map(|n| n.start_tick)
            .min()
            .unwrap_or(PatternTick::ZERO);
        let paste_tick = min_tick + view_state.clipboard.selection_width;
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Arrow Up/Down — transpose selected notes ──
    if !view_state.selected_notes.is_empty() {
        let shift = ctx.input(|i| i.modifiers.shift);
        let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        if up || down {
            let semitones = match (up, shift) {
                (true, false) => Semitones::new(1.0),
                (true, true) => Semitones::new(12.0),
                (false, false) => Semitones::new(-1.0),
                (false, true) => Semitones::new(-12.0),
            };
            if let Ok(mut song_w) = song.write()
                && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
            {
                let mut composite = Vec::new();
                for note_id in &view_state.selected_notes {
                    if let Some(note) = pattern.note(*note_id) {
                        let old_pitch = note.pitch;
                        if pattern.transpose_note(*note_id, semitones)
                            && let Some(transposed) = pattern.note(*note_id)
                        {
                            composite.push(crate::undo::UndoAction::TransposeNote {
                                pattern_id: data.pattern_id,
                                note_id: *note_id,
                                old_pitch,
                                new_pitch: transposed.pitch,
                            });
                        }
                    }
                }
                if !composite.is_empty() {
                    undo_manager.push(crate::undo::UndoAction::Composite(composite));
                }
            }
        }
    }

    // ── Space — toggle play/pause ──
    if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
        if is_playing {
            handle.send(EngineCommand::Pause);
        } else {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }
    }

    // ── Step entry mode — keyboard piano inserts notes at cursor ──
    if view_state.step_entry_mode {
        let step_size = if view_state.draw_note_length > 0 {
            SeqDuration(view_state.draw_note_length)
        } else {
            view_state.effective_grid(data.ticks_per_row)
        };

        // Collect pressed key in a single input lock acquisition
        let pressed_note = ctx.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl {
                return None;
            }
            KEY_MAP
                .iter()
                .find(|(key, _)| i.key_pressed(*key))
                .map(|(_, note)| *note)
        });
        let mut inserted = false;
        if let Some(pitch_val) = pressed_note
            && let Some(pitch) = Pitch::new(pitch_val)
            && let Ok(mut song_w) = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            let note_id = pattern.add_note(
                view_state.step_cursor_tick,
                pitch,
                view_state.default_velocity,
                view_state.selected_instrument,
            );
            pattern.resize_note(note_id, step_size);
            if let Some(note) = pattern.note(note_id) {
                undo_manager.push(crate::undo::UndoAction::AddNote {
                    pattern_id: data.pattern_id,
                    note: note.into(),
                });
            }
            view_state.selected_notes.clear();
            view_state.selected_notes.insert(note_id);
            preview_note(handle, pitch, view_state.default_velocity);
            inserted = true;
        }
        if inserted {
            view_state.step_cursor_tick = view_state.step_cursor_tick + step_size;
            // Wrap around at pattern end
            if view_state.step_cursor_tick.0 >= data.length_ticks.0 {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }
    }

    keep_open
}

/// Handle mouse clicks and drags in the piano roll.
#[allow(clippy::too_many_arguments)]
fn handle_piano_roll_interaction(
    response: &egui::Response,
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    handle: &mut EngineHandle,
    grid_rect: Rect,
    auto_rect: Option<Rect>,
    auto_y: f32,
    x_to_tick: &dyn Fn(f32) -> PatternTick,
    y_to_pitch: &dyn Fn(f32) -> Pitch,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    pitch_to_y: &dyn Fn(Pitch) -> f32,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
    undo_manager: &mut crate::undo::UndoManager,
) {
    let shift_held = ui.ctx().input(|i| i.modifiers.shift);

    // ── Automation click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        let raw_tick = x_to_tick(pos.x);
        let tick = quantize_tick(raw_tick, data.ticks_per_row);
        let value = (1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0);

        if let Ok(mut song_w) = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            let lane = pattern.get_or_create_automation(target);
            lane.add_point(AutomationPoint::new(tick, NormalizedValue::new(value)));
        }
    }

    // ── Automation right-click (delete point) ──
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        // Find the lane snapshot to hit-test against
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let point_tick = lane.points[idx].tick;
            if let Ok(mut song_w) = song.write()
                && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                && let Some(auto_lane) = pattern.automation.iter_mut().find(|l| l.target == target)
            {
                auto_lane.remove_point(point_tick);
            }
        }
    }

    // ── Click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
        );

        match hit {
            Some((note_id, _)) => {
                // Clicked on a note — select it and preview its sound
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    preview_note(handle, note.pitch, note.velocity);
                }
                if shift_held {
                    // Toggle in selection
                    if !view_state.selected_notes.remove(&note_id) {
                        view_state.selected_notes.insert(note_id);
                    }
                } else {
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            None => {
                // Clicked on empty space
                if view_state.edit_tool == EditTool::Draw {
                    // Add a new note (only if no note already exists here)
                    let raw_tick = x_to_tick(pos.x);
                    let tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch_val = y_to_pitch(pos.y);

                    if !has_note_at(&data.notes, tick, pitch_val)
                        && let Ok(mut song_w) = song.write()
                        && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    {
                        let duration = if view_state.draw_note_length > 0 {
                            SeqDuration(view_state.draw_note_length)
                        } else {
                            SeqDuration((data.ticks_per_row as u32).max(1))
                        };
                        let note_id = pattern.add_note(
                            tick,
                            pitch_val,
                            view_state.default_velocity,
                            view_state.selected_instrument,
                        );
                        pattern.resize_note(note_id, duration);
                        if let Some(note) = pattern.note(note_id) {
                            undo_manager.push(crate::undo::UndoAction::AddNote {
                                pattern_id: data.pattern_id,
                                note: note.into(),
                            });
                        }
                        view_state.selected_notes.clear();
                        view_state.selected_notes.insert(note_id);
                        preview_note(handle, pitch_val, view_state.default_velocity);
                    }
                } else if !shift_held {
                    // Select tool on empty space — clear selection
                    view_state.selected_notes.clear();
                }
            }
        }
    }

    // ── Automation drag start ──
    if response.drag_started()
        && let Some(pos) = ui.ctx().input(|i| i.pointer.press_origin())
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let pt = &lane.points[idx];
            view_state.drag = Some(DragState::DragAutomationPoint {
                target,
                original_tick: pt.tick,
                original_value: pt.value,
                current_tick: pt.tick,
                current_value: pt.value,
            });
        }
    }

    // ── Velocity drag start ──
    if response.drag_started()
        && let Some(pos) = ui.ctx().input(|i| i.pointer.press_origin())
    {
        let vel_y = grid_rect.max.y;
        let vel_rect = Rect::from_min_size(
            Pos2::new(grid_rect.min.x, vel_y),
            Vec2::new(grid_rect.width(), VELOCITY_ZONE_HEIGHT),
        );
        if vel_rect.contains(pos) {
            view_state.drag = Some(DragState::DragVelocity { last_note_id: None });
        }
    }

    // ── Drag start ──
    // Use press_origin (where the click started) for hit-testing, not current pointer pos.
    // Without this, dragging straight up/down misses the note because the pointer
    // has already left the note rect by the time the drag threshold is reached.
    if response.drag_started()
        && let Some(pos) = ui.ctx().input(|i| i.pointer.press_origin())
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
        );

        match hit {
            Some((note_id, HitZone::Body)) => {
                // Start moving the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    // If note has no explicit duration, lock it before move
                    // so the visual length is preserved
                    if note.end_tick.is_none() {
                        let implied_dur = (data.length_ticks.as_pattern_tick() - note.start_tick)
                            .max(SeqDuration(1));
                        if let Ok(mut song_w) = song.write()
                            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                        {
                            pattern.resize_note(note_id, implied_dur);
                        }
                    }
                    // Calculate where on the note the user grabbed
                    let grab_tick = x_to_tick(pos.x);
                    let grab_offset = grab_tick - note.start_tick;
                    view_state.drag = Some(DragState::MoveNote {
                        note_id,
                        original_tick: note.start_tick,
                        original_pitch: note.pitch,
                        current_tick: note.start_tick,
                        current_pitch: note.pitch,
                        grab_offset_ticks: grab_offset,
                    });
                    // Ensure note is selected
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                    view_state.selected_notes.insert(note_id);
                }
            }
            Some((note_id, HitZone::RightEdge)) => {
                // Start resizing the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    let end_tick = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    view_state.drag = Some(DragState::ResizeNote {
                        note_id,
                        original_end_tick: end_tick,
                        current_end_tick: end_tick,
                    });
                }
            }
            None => match view_state.edit_tool {
                EditTool::Select => {
                    // Start selection rectangle
                    view_state.drag = Some(DragState::SelectRect {
                        start_pos: pos,
                        current_pos: pos,
                    });
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                }
                EditTool::Draw => {
                    // Start drawing a new note by dragging (only if position is free)
                    let raw_tick = x_to_tick(pos.x);
                    let start_tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch = y_to_pitch(pos.y);
                    if !has_note_at(&data.notes, start_tick, pitch) {
                        let end_tick =
                            PatternTick(start_tick.0 + (data.ticks_per_row as u32).max(1));
                        view_state.drag = Some(DragState::DrawNote {
                            start_tick,
                            pitch,
                            current_end_tick: end_tick,
                        });
                    }
                }
            },
        }
    }

    // ── Drag update ──
    // Use pointer_latest_pos for smooth tracking during drags
    if response.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        match &mut view_state.drag {
            Some(DragState::MoveNote {
                current_tick,
                current_pitch,
                grab_offset_ticks,
                ..
            }) => {
                let raw_tick = PatternTick(x_to_tick(pos.x).0.saturating_sub(grab_offset_ticks.0));
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_pitch = y_to_pitch(pos.y);
            }
            Some(DragState::ResizeNote {
                current_end_tick, ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                *current_end_tick = PatternTick(quantized.0.max(1));
            }
            Some(DragState::DrawNote {
                start_tick,
                current_end_tick,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                // End tick must be at least one row past start
                *current_end_tick = PatternTick(
                    quantized
                        .0
                        .max(start_tick.0 + (data.ticks_per_row as u32).max(1)),
                );
            }
            Some(DragState::SelectRect { current_pos, .. }) => {
                *current_pos = pos;
            }
            Some(DragState::DragAutomationPoint {
                current_tick,
                current_value,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_value = NormalizedValue::new(
                    (1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0),
                );
            }
            Some(DragState::DragVelocity { .. }) => {
                // Velocity painting handled in real-time below
            }
            Some(DragState::DragPlacement { .. }) | None => {}
        }
    }

    // ── Velocity drag: apply velocity change in real-time ──
    if let Some(DragState::DragVelocity { last_note_id }) = &mut view_state.drag
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        // vel_y is grid_rect.max.y, VELOCITY_ZONE_HEIGHT below
        let vel_y = grid_rect.max.y;
        let new_vel = (1.0 - (pos.y - vel_y) / VELOCITY_ZONE_HEIGHT).clamp(0.01, 1.0);
        // Find note at this x position
        let click_tick = x_to_tick(pos.x);
        let nearest_note = data.notes.iter().min_by_key(|n| {
            let mid = n.start_tick.0 + n.end_tick.map_or(0, |e| (e.0 - n.start_tick.0) / 2);
            (mid as i64 - click_tick.0 as i64).unsigned_abs()
        });
        if let Some(note) = nearest_note {
            let note_x = tick_to_x(note.start_tick);
            if (note_x - pos.x).abs() < 15.0 {
                *last_note_id = Some(note.note_id);
                if let Ok(mut song_w) = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    pattern.set_note_velocity(note.note_id, Velocity::new(new_vel));
                }
            }
        }
    }

    // ── Drag end ──
    if response.drag_stopped()
        && let Some(drag) = view_state.drag.take()
    {
        match drag {
            DragState::MoveNote {
                note_id,
                original_tick,
                original_pitch,
                current_tick,
                current_pitch,
                ..
            } => {
                // Apply move to song
                if (current_tick != original_tick || current_pitch != original_pitch)
                    && let Ok(mut song_w) = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let mut composite = Vec::new();
                    if current_tick != original_tick {
                        pattern.move_note(note_id, current_tick);
                        composite.push(crate::undo::UndoAction::MoveNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_start: original_tick,
                            new_start: current_tick,
                        });
                    }
                    if current_pitch != original_pitch {
                        #[allow(clippy::cast_precision_loss)]
                        let delta =
                            current_pitch.as_midi() as f32 - original_pitch.as_midi() as f32;
                        pattern.transpose_note(note_id, Semitones::new(delta));
                        composite.push(crate::undo::UndoAction::TransposeNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_pitch: original_pitch,
                            new_pitch: current_pitch,
                        });
                    }
                    if !composite.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::Composite(composite));
                    }
                }
            }
            DragState::ResizeNote {
                note_id,
                original_end_tick,
                current_end_tick,
                ..
            } => {
                // Apply resize to song
                if current_end_tick != original_end_tick
                    && let Some(note) = data.notes.iter().find(|n| n.note_id == note_id)
                {
                    let new_duration = (current_end_tick - note.start_tick).max(SeqDuration(1));
                    let old_duration = note.end_tick.map(|e| e - note.start_tick);
                    if let Ok(mut song_w) = song.write()
                        && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    {
                        pattern.resize_note(note_id, new_duration);
                        undo_manager.push(crate::undo::UndoAction::ResizeNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_duration: old_duration.map(|d| SeqDuration(d.0)),
                            new_duration: Some(new_duration),
                        });
                    }
                }
            }
            DragState::DrawNote {
                start_tick,
                pitch,
                current_end_tick,
            } => {
                // Create the note with the dragged duration (only if no duplicate)
                let duration = (current_end_tick - start_tick).max(SeqDuration(1));
                if !has_note_at(&data.notes, start_tick, pitch)
                    && let Ok(mut song_w) = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let note_id = pattern.add_note(
                        start_tick,
                        pitch,
                        view_state.default_velocity,
                        view_state.selected_instrument,
                    );
                    pattern.resize_note(note_id, duration);
                    if let Some(added_note) = pattern.note(note_id) {
                        undo_manager.push(crate::undo::UndoAction::AddNote {
                            pattern_id: data.pattern_id,
                            note: added_note.into(),
                        });
                    }
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            DragState::SelectRect {
                start_pos,
                current_pos,
                ..
            } => {
                // Resolve selection rectangle to notes
                let sel_rect = Rect::from_two_pos(start_pos, current_pos);
                let tick_start = x_to_tick(sel_rect.min.x);
                let tick_end = x_to_tick(sel_rect.max.x);
                let pitch_top = y_to_pitch(sel_rect.min.y);
                let pitch_bottom = y_to_pitch(sel_rect.max.y);
                let p_min = pitch_bottom.min(pitch_top);
                let p_max = pitch_bottom.max(pitch_top);

                for note in &data.notes {
                    let note_end = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    if note.start_tick < tick_end
                        && note_end > tick_start
                        && note.pitch >= p_min
                        && note.pitch <= p_max
                    {
                        view_state.selected_notes.insert(note.note_id);
                    }
                }
            }
            DragState::DragAutomationPoint {
                target,
                original_tick,
                original_value,
                current_tick,
                current_value,
            } => {
                // Apply automation point move
                if (current_tick != original_tick
                    || (current_value.as_f32() - original_value.as_f32()).abs() > f32::EPSILON)
                    && let Ok(mut song_w) = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let lane = pattern.get_or_create_automation(target);
                    lane.remove_point(original_tick);
                    lane.add_point(AutomationPoint::new(current_tick, current_value));
                }
            }
            // DragVelocity — already applied in real-time, nothing to finalize
            DragState::DragVelocity { .. } => {}
            // DragPlacement is handled in the arrangement view, not here
            DragState::DragPlacement { .. } => {}
        }
    }
}

/// Find automation point at the given position (within hit radius).
fn automation_point_at_pos(
    lane: &AutomationLaneSnapshot,
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    auto_y: f32,
) -> Option<usize> {
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    for (i, pt) in lane.points.iter().enumerate() {
        let px = tick_to_x(pt.tick);
        let py = value_to_y(pt.value.as_f32());
        let dist = ((pos.x - px).powi(2) + (pos.y - py).powi(2)).sqrt();
        if dist <= AUTOMATION_HIT_RADIUS {
            return Some(i);
        }
    }
    None
}

/// Draw the automation zone below the velocity zone.
#[allow(clippy::too_many_arguments)]
fn draw_automation_zone(
    painter: &egui::Painter,
    data: &PianoRollData,
    view_state: &SequencerViewState,
    selected_target: &AutomationTarget,
    grid_x: f32,
    auto_y: f32,
    grid_width: f32,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    t: &crate::gui::theme::Theme,
) {
    let auto_color = Color32::from_rgb(255, 160, 50); // Orange

    // Background
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(grid_x, auto_y),
            Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
        ),
        0.0,
        Color32::from_rgba_premultiplied(18, 20, 24, 220),
    );

    // Separator line
    painter.line_segment(
        [
            Pos2::new(grid_x, auto_y),
            Pos2::new(grid_x + grid_width, auto_y),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // Label
    painter.text(
        Pos2::new(grid_x - KEY_WIDTH + 2.0, auto_y + 2.0),
        egui::Align2::LEFT_TOP,
        "AUTO",
        egui::FontId::proportional(9.0),
        t.colors.text_dim,
    );

    // Reference lines (25%, 50%, 75%)
    for frac in [0.25, 0.5, 0.75] {
        let ry = auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - frac);
        painter.line_segment(
            [Pos2::new(grid_x, ry), Pos2::new(grid_x + grid_width, ry)],
            Stroke::new(0.3, t.colors.border.gamma_multiply(0.3)),
        );
    }

    // Coordinate helpers
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    // Find the lane matching the selected target
    let lane = data
        .automation_lanes
        .iter()
        .find(|l| l.target == *selected_target);

    if let Some(lane) = lane {
        let points = &lane.points;

        if !points.is_empty() {
            // Draw flat extension before first point
            if let Some(first) = points.first() {
                let first_x = tick_to_x(first.tick);
                if first_x > grid_x {
                    let y = value_to_y(first.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(grid_x, y), Pos2::new(first_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw curves between consecutive points
            for [from, to] in points.array_windows() {
                let x_start = tick_to_x(from.tick);
                let x_end = tick_to_x(to.tick);
                let pixel_width = (x_end - x_start).max(1.0);

                // Sample the curve pixel by pixel
                let steps = (pixel_width as u32).max(2);
                let mut prev_pos = Pos2::new(x_start, value_to_y(from.value.as_f32()));

                for step in 1..=steps {
                    #[allow(clippy::cast_precision_loss)]
                    let frac = step as f32 / steps as f32;
                    let x = x_start + frac * (x_end - x_start);
                    let val =
                        from.curve
                            .interpolate(from.value, to.value, NormalizedValue::new(frac));
                    let y = value_to_y(val.as_f32());
                    let cur_pos = Pos2::new(x, y);

                    painter.line_segment([prev_pos, cur_pos], Stroke::new(1.5, auto_color));
                    prev_pos = cur_pos;
                }
            }

            // Draw flat extension after last point
            if let Some(last) = points.last() {
                let last_x = tick_to_x(last.tick);
                let grid_end_x = grid_x + grid_width;
                if last_x < grid_end_x {
                    let y = value_to_y(last.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(last_x, y), Pos2::new(grid_end_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw points
            for pt in points {
                let px = tick_to_x(pt.tick);
                let py = value_to_y(pt.value.as_f32());
                painter.circle_filled(Pos2::new(px, py), AUTOMATION_POINT_RADIUS, auto_color);
                painter.circle_stroke(
                    Pos2::new(px, py),
                    AUTOMATION_POINT_RADIUS,
                    Stroke::new(1.0, Color32::WHITE),
                );
            }
        }
    }

    // Draw drag preview ghost point
    if let Some(DragState::DragAutomationPoint {
        current_tick,
        current_value,
        target,
        ..
    }) = &view_state.drag
        && target == selected_target
    {
        let px = tick_to_x(*current_tick);
        let py = value_to_y(current_value.as_f32());
        painter.circle_filled(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            Color32::from_rgba_unmultiplied(255, 160, 50, 120),
        );
        painter.circle_stroke(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            Stroke::new(1.0, Color32::WHITE),
        );
    }
}

/// Delete all selected notes from the pattern.
/// Play a short note preview (instant note-on + note-off).
fn preview_note(handle: &mut EngineHandle, pitch: Pitch, velocity: synth_core::Velocity) {
    handle.note_on(MidiNote::new(pitch.as_midi()), velocity);
    handle.note_off(MidiNote::new(pitch.as_midi()));
}

fn delete_selected_notes(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    selected: &mut HashSet<NoteId>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    if selected.is_empty() {
        return;
    }
    if let Ok(mut song_w) = song.write()
        && let Some(pattern) = song_w.pattern_mut(pattern_id)
    {
        let mut composite = Vec::new();
        for note_id in selected.iter() {
            if let Some(note) = pattern.note(*note_id) {
                composite.push(crate::undo::UndoAction::RemoveNote {
                    pattern_id,
                    note: note.into(),
                });
            }
            pattern.remove_note(*note_id);
        }
        if !composite.is_empty() {
            undo_manager.push(crate::undo::UndoAction::Composite(composite));
        }
    }
    selected.clear();
}

/// Copy selected notes into the clipboard.
fn copy_selected_notes(
    data: &PianoRollData,
    selected: &HashSet<NoteId>,
    clipboard: &mut Clipboard,
) {
    clipboard.notes.clear();
    clipboard.selection_width = SeqDuration(0);

    let selected_notes: Vec<&PianoRollNote> = data
        .notes
        .iter()
        .filter(|n| selected.contains(&n.note_id))
        .collect();

    if selected_notes.is_empty() {
        return;
    }

    let min_tick = selected_notes
        .iter()
        .map(|n| n.start_tick)
        .min()
        .unwrap_or(PatternTick::ZERO);

    let max_end = selected_notes
        .iter()
        .map(|n| {
            n.end_tick
                .unwrap_or(PatternTick(n.start_tick.0 + data.ticks_per_row as u32))
        })
        .max()
        .unwrap_or(min_tick);

    clipboard.selection_width = max_end - min_tick;

    for note in &selected_notes {
        clipboard.notes.push(ClipboardNote {
            tick_offset: note.start_tick - min_tick,
            pitch: note.pitch,
            velocity: note.velocity,
            duration: note.end_tick.map(|e| e - note.start_tick),
            instrument: note.instrument,
        });
    }
}

/// Paste clipboard notes at a given tick position into the pattern.
fn paste_clipboard_notes(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    clipboard: &Clipboard,
    paste_tick: PatternTick,
    selected: &mut HashSet<NoteId>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    if clipboard.notes.is_empty() {
        return;
    }

    selected.clear();

    if let Ok(mut song_w) = song.write()
        && let Some(pattern) = song_w.pattern_mut(pattern_id)
    {
        let mut composite = Vec::new();
        for cn in &clipboard.notes {
            let tick = paste_tick + cn.tick_offset;
            let note_id = pattern.add_note(tick, cn.pitch, cn.velocity, cn.instrument);
            if let Some(dur) = cn.duration {
                pattern.resize_note(note_id, dur);
            }
            if let Some(note) = pattern.note(note_id) {
                composite.push(crate::undo::UndoAction::AddNote {
                    pattern_id,
                    note: note.into(),
                });
            }
            selected.insert(note_id);
        }
        if !composite.is_empty() {
            undo_manager.push(crate::undo::UndoAction::Composite(composite));
        }
    }
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
pub(crate) fn draw_sequencer_view(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) {
    // Transport bar at the top
    let is_playing = egui::TopBottomPanel::top("sequencer_transport")
        .show(ctx, |ui| draw_transport_bar(ui, handle, song, view_state))
        .inner;

    // Request repaint during playback for smooth position updates
    if is_playing {
        ctx.request_repaint();
    }

    // Read current playhead position (atomic, lock-free)
    let current_tick = handle.state.transport.get_ticks();

    // Collect song data (short read-lock, then release before rendering)
    let arrangement_data = collect_arrangement_data(song);

    // Pattern follow: when playing with a selected track, auto-switch to
    // whichever pattern is currently playing on that track.
    let playing_pattern_on_selected_track = if let Some(track_id) = view_state.selected_track
        && let Some(ad) = &arrangement_data
    {
        ad.placements.iter().find_map(|p| {
            if p.track_id == track_id
                && current_tick >= p.start_tick
                && current_tick < p.end_tick
            {
                Some(p.pattern_id)
            } else {
                None
            }
        })
    } else {
        None
    };

    if is_playing
        && let Some(pattern_id) = playing_pattern_on_selected_track
        && view_state.opened_pattern != Some(pattern_id)
    {
        view_state.opened_pattern = Some(pattern_id);
        view_state.selected_notes.clear();
        view_state.drag = None;
    }

    // Piano roll bottom panel (if a pattern is open)
    if let Some(pattern_id) = view_state.opened_pattern {
        let piano_roll_data = collect_piano_roll_data(song, pattern_id);

        // Calculate pattern-relative playhead tick (only if this pattern is
        // actually playing right now in the arrangement).
        let pattern_playhead_tick: Option<PatternTick> = arrangement_data.as_ref().and_then(|ad| {
            ad.placements.iter().find_map(|p| {
                if p.pattern_id == pattern_id
                    && current_tick >= p.start_tick
                    && current_tick < p.end_tick
                {
                    #[allow(clippy::cast_possible_truncation)]
                    Some(PatternTick((current_tick - p.start_tick) as u32))
                } else {
                    None
                }
            })
        });

        // Use ~50% of available height for piano roll, with generous max
        let available_height = ctx.available_rect().height();
        let default_height = (available_height * 0.5).max(MIN_PIANO_ROLL_HEIGHT);

        egui::TopBottomPanel::bottom("piano_roll")
            .resizable(true)
            .default_height(default_height)
            .min_height(150.0)
            .max_height(available_height - 100.0)
            .show(ctx, |ui| {
                if let Some(data) = &piano_roll_data {
                    if !draw_piano_roll(
                        ui,
                        data,
                        pattern_playhead_tick,
                        is_playing,
                        handle,
                        song,
                        view_state,
                        instruments,
                        undo_manager,
                    ) {
                        view_state.opened_pattern = None;
                        view_state.selected_notes.clear();
                        view_state.drag = None;
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
            if let Some(pattern_id) = draw_arrangement(
                ui,
                data,
                current_tick,
                is_playing,
                handle,
                song,
                view_state,
                instruments,
            ) {
                // Clear selection when switching patterns
                if view_state.opened_pattern != Some(pattern_id) {
                    view_state.selected_notes.clear();
                    view_state.drag = None;
                }
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
