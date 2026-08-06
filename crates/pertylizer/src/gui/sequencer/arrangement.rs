//! Arrangement timeline view: track headers, placement miniatures,
//! the arrangement toolbar, and timeline interaction (drag/resize/context menu).
//!
//! The snapshot DTOs (`ArrangementData` et al.) live in the parent module so
//! both this view and `draw_sequencer_view` can read them.

use super::*;
use crate::app_services::{SongMutationService, TempoPointEdit};
use crate::gui::widgets::{expose, expose_selected, mute_toggle, solo_toggle, submenu_button};

const PLACEMENT_RESIZE_ZONE: f32 = 8.0;
const SECTION_RESIZE_ZONE: f32 = 7.0;

/// Apply one section edit and record the complete before/after section list as
/// a single undo step.
fn edit_sections(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
    edit: impl FnOnce(&mut synth_sequencer::Song),
) {
    let (old, new) = {
        let mut song_w = song.write();
        let old = song_w.sections().to_vec();
        edit(&mut song_w);
        let new = song_w.sections().to_vec();
        (old, new)
    };
    if old != new {
        undo_manager.push(crate::undo::UndoAction::SetArrangementSections { old, new });
    }
}

/// Record a pattern this view just created, together with the placement it was
/// dropped at, as one undo step.
///
/// [`UndoAction::AddPattern`](crate::undo::UndoAction::AddPattern) carries its
/// placements, so a create-and-place collapses into a single entry — undoing
/// takes both away, which is what the user made in one gesture. Call after the
/// write lock is released.
///
/// The arrangement view creates patterns from three places (double-click on
/// empty timeline, "New Pattern Here", "Duplicate Pattern") and every one of
/// them used to record nothing, while the *same* operations in the pattern view
/// did — so Ctrl+Z here undid the previous edit and left the new pattern
/// standing.
fn record_created_pattern(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
    pattern_id: PatternId,
) {
    let captured = {
        let song_r = song.read();
        song_r.pattern(pattern_id).cloned().map(|pattern| {
            let placements: Vec<_> = song_r
                .arrangement()
                .iter()
                .filter(|p| p.pattern_id == pattern_id)
                .cloned()
                .collect();
            (pattern, placements)
        })
    };
    if let Some((pattern, placements)) = captured {
        undo_manager.push(crate::undo::UndoAction::AddPattern {
            pattern,
            placements,
        });
    }
}

/// Record a placement of an *existing* pattern as one undo step.
///
/// The pattern itself is untouched, so this is an
/// [`InsertPlacement`](crate::undo::UndoAction::InsertPlacement) rather than an
/// `AddPattern` — undoing must take the clip off the timeline without deleting
/// the pattern every other placement still refers to. Looks the placement up
/// rather than reconstructing it, so whatever defaults `place_pattern` chose
/// are the ones restored.
fn record_placement(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
    pattern_id: PatternId,
    track_id: TrackId,
    start: Tick,
) {
    let placement = song
        .read()
        .arrangement()
        .iter()
        .find(|p| p.pattern_id == pattern_id && p.track_id == track_id && p.start == start)
        .cloned();
    if let Some(placement) = placement {
        undo_manager.push(crate::undo::UndoAction::InsertPlacement { placement });
    }
}

/// Collect arrangement data from song (short read-lock, then release).
pub(super) fn collect_arrangement_data(
    song: &Arc<synth_sequencer::SharedSong>,
) -> Option<ArrangementData> {
    let song = song.try_read()?;

    let tracks: Vec<TrackInfo> = song
        .tracks()
        .map(|t| TrackInfo {
            id: t.id,
            name: t.name.clone(),
            description: t.description.clone(),
            color: track_color_to_egui(t.color),
            track_color: t.color,
            volume: t.volume,
            pan: t.pan,
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

    let mut song_end_tick = song.calculate_length().0;
    let placements: Vec<PlacementInfo> = song
        .arrangement()
        .iter()
        .filter_map(|p| {
            let pattern = song.pattern(p.pattern_id)?;
            let end = p.end(pattern.length);
            if end.0 > song_end_tick {
                song_end_tick = end.0;
            }
            // Color and instrument from the track this placement belongs to.
            let track = song.track(p.track_id);
            let color = track
                .map(|t| track_color_to_egui(t.color))
                .unwrap_or(Color32::GRAY);
            let instrument = track.map(|t| t.instrument).unwrap_or_default();

            let effective_length = p.effective_length(pattern.length);

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

                // Bound the snapshot at what the draw loop could ever use:
                // a placement is at most effective-length beats × PIXELS_PER_BEAT ×
                // MAX_ZOOM pixels wide, and past MINIATURE_NOTES_PER_PIXEL
                // notes per pixel drawing is invisible. Decimate evenly past
                // that (notes are sorted by start tick) so a pathologically
                // dense pattern cannot blow up the per-frame snapshot cost.
                #[allow(clippy::cast_sign_loss)]
                let budget = (((effective_length.0 as f32
                    / synth_sequencer::TICKS_PER_QUARTER as f32)
                    * PIXELS_PER_BEAT
                    * MAX_ZOOM
                    * MINIATURE_NOTES_PER_PIXEL) as usize)
                    .max(1);
                let step = notes.len().div_ceil(budget).max(1);

                notes
                    .iter()
                    .step_by(step)
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
                instrument,
                pattern_length: pattern.length,
                effective_length,
                loop_mode: p.loop_mode,
                note_miniatures,
            })
        })
        .collect();

    let time_sig = song.default_time_signature;
    let tempo_changes: Vec<(u64, f32, bool)> = song
        .tempo_changes()
        .iter()
        .map(|tc| (tc.tick.0, tc.bpm.as_f32(), tc.ramp))
        .collect();

    Some(ArrangementData {
        tracks,
        placements,
        patterns,
        sections: song.sections().to_vec(),
        time_sig,
        song_end_tick,
        tempo_changes,
        default_tempo: song.default_tempo.as_f32(),
    })
}

/// Arrangement view toolbar: zoom controls (out / 1x / in / fit), the snap
/// selector, and the follow-playhead toggle.
fn draw_arrangement_toolbar(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    view_state: &mut SequencerViewState,
) {
    let t = theme();
    let beats_per_bar = data.time_sig.numerator as u64;
    // Secondary controls row under the transport bar (shared toolbar styling).
    crate::gui::toolbar::secondary_row(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        caption(ui, "Zoom", CaptionTone::Dim);
        if ui.small_button("-").on_hover_text("Zoom out").clicked() {
            view_state.zoom_level = (view_state.zoom_level * 0.8).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        if ui.small_button("1x").on_hover_text("Reset zoom").clicked() {
            view_state.zoom_level = 1.0;
        }
        if ui.small_button("+").on_hover_text("Zoom in").clicked() {
            view_state.zoom_level = (view_state.zoom_level * 1.25).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        if ui
            .small_button("Fit")
            .on_hover_text("Fit song to view")
            .clicked()
        {
            // Approximate: scale zoom so the whole song fits the visible width
            let visible_w = ui.available_width().max(200.0);
            let song_beats = (data.song_end_tick as f32
                / synth_sequencer::TICKS_PER_QUARTER as f32)
                .max(beats_per_bar as f32 * MIN_VISIBLE_BARS as f32);
            let needed = visible_w / song_beats.max(1.0) / PIXELS_PER_BEAT;
            view_state.zoom_level = needed.clamp(MIN_ZOOM, MAX_ZOOM);
        }

        ui.separator();
        caption(ui, "Snap", CaptionTone::Dim);
        let snap_options = [
            ("Off", 0_u32),
            ("1/2 beat", synth_sequencer::TICKS_PER_QUARTER / 2),
            ("Beat", synth_sequencer::TICKS_PER_QUARTER),
            ("Bar", data.time_sig.ticks_per_bar()),
            ("2 bars", data.time_sig.ticks_per_bar() * 2),
        ];
        let snap_label = snap_options
            .iter()
            .find(|(_, ticks)| *ticks == view_state.arrangement_snap_ticks)
            .map_or("Custom", |(l, _)| l);
        egui::ComboBox::from_id_salt("arrangement_snap")
            .selected_text(snap_label)
            .width(80.0)
            .show_ui(ui, |ui| {
                for (label, ticks) in snap_options {
                    if ui
                        .selectable_label(view_state.arrangement_snap_ticks == ticks, label)
                        .clicked()
                    {
                        view_state.arrangement_snap_ticks = ticks;
                    }
                }
            });

        ui.separator();
        let follow_color = if view_state.auto_follow_playhead {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .small_button(RichText::new("Follow").color(follow_color))
            .on_hover_text("Auto-scroll to keep the playhead visible")
            .clicked()
        {
            if view_state.auto_follow_playhead {
                view_state.auto_follow_playhead = false;
            } else {
                // Bring an off-screen marker into view even while stopped.
                view_state.reveal_playhead();
            }
        }
        if ui
            .small_button("Go to playhead")
            .on_hover_text("Scroll the timeline to the current playhead position")
            .clicked()
        {
            // Re-arm follow (so it keeps tracking if playing) and force an
            // unconditional scroll so the jump always lands, even when stopped
            // and the marker is already roughly in view.
            view_state.reveal_playhead_force();
        }
    });
    ui.separator();
}

/// Bundle of the long-lived locals the arrangement view threads through: the
/// song snapshot, live engine handle, view state, undo manager and instrument
/// list. Mirrors [`super::piano_roll`]'s `PianoRollCtx` so extracted sub-
/// sections (the track-header panel, timeline painter, context menu) take one
/// `&mut ctx` instead of many positional parameters; each helper re-exposes the
/// fields under their original names so the moved bodies stay byte-for-byte
/// unchanged.
struct ArrangementCtx<'a> {
    data: &'a ArrangementData,
    song: &'a Arc<synth_sequencer::SharedSong>,
    handle: &'a mut EngineHandle,
    view_state: &'a mut SequencerViewState,
    undo_manager: &'a mut crate::undo::UndoManager,
    instruments: &'a [crate::gui::instrument_rack::InstrumentUiState],
}

/// Painter-local coordinate transforms for the arrangement timeline. Mirrors
/// [`super::piano_roll`]'s `PianoRollCoords`: the screen geometry (timeline
/// origin, zoom, snap unit) lives in one value so extracted painter / context-
/// menu helpers can take `&ArrangementCoords` instead of capturing a fistful of
/// closures. The closures in [`draw_arrangement`] delegate straight to these
/// methods, so every call site stays byte-for-byte unchanged.
struct ArrangementCoords {
    /// Left edge of the timeline (x of tick 0), in screen pixels.
    tl_x: f32,
    /// Top edge of the timeline (ruler top), in screen pixels.
    tl_y: f32,
    /// Ticks per beat (quarter note).
    ticks_per_beat: u64,
    /// Horizontal scale: screen pixels per beat.
    pixels_per_beat: f32,
    /// Number of track rows (clamps `y_to_row`).
    track_count: usize,
    /// Snap unit in ticks, shared by placement create / drag / resize / loop.
    snap_ticks: u64,
}

impl ArrangementCoords {
    /// Tick → x screen position.
    fn tick_to_x(&self, tick_val: u64) -> f32 {
        if self.ticks_per_beat == 0 {
            return self.tl_x;
        }
        let beats = tick_val as f32 / self.ticks_per_beat as f32;
        self.tl_x + beats * self.pixels_per_beat
    }

    /// x screen position → tick (clamped at 0).
    fn x_to_tick(&self, x: f32) -> u64 {
        if self.ticks_per_beat == 0 {
            return 0;
        }
        let beats = (x - self.tl_x) / self.pixels_per_beat;
        (beats * self.ticks_per_beat as f32).max(0.0) as u64
    }

    /// Snap a tick to the current arrangement snap unit.
    fn snap_tick(&self, tick: u64) -> u64 {
        snap_to_step(tick, self.snap_ticks)
    }

    /// y screen position → track row index, or `None` when above the first row
    /// or past the last track.
    fn y_to_row(&self, y: f32) -> Option<usize> {
        let row_offset = y - (self.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT);
        if row_offset < 0.0 || TRACK_ROW_HEIGHT <= 0.0 {
            return None;
        }
        let idx = (row_offset / TRACK_ROW_HEIGHT) as usize;
        if idx < self.track_count {
            Some(idx)
        } else {
            None
        }
    }
}

/// Write a new length to a pattern and push the matching `SetPatternLength`
/// undo action — but only if the length actually changed. Shared by the
/// "Set Length…" submenu's free-input Apply branch and its bar presets.
fn apply_pattern_length(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
    pat_id: PatternId,
    new_len: SeqDuration,
) {
    let mut applied: Option<SeqDuration> = None;
    {
        let mut song_w = song.write();
        if let Some(pat) = song_w.pattern_mut(pat_id)
            && pat.length != new_len
        {
            applied = Some(pat.length);
            pat.length = new_len;
        }
    }
    if let Some(old) = applied {
        undo_manager.push(crate::undo::UndoAction::SetPatternLength {
            pattern_id: pat_id,
            old_length: old,
            new_length: new_len,
        });
    }
}

/// Visible BPM window for the tempo lane's vertical axis, fitted to the map
/// (default tempo + all points) with padding so the curve fills the lane. Kept
/// to a usable minimum span and clamped to the valid `[20, 300]` tempo range.
fn tempo_lane_range(data: &ArrangementData) -> (f32, f32) {
    let mut lo = data.default_tempo;
    let mut hi = data.default_tempo;
    for &(_, bpm, _) in &data.tempo_changes {
        lo = lo.min(bpm);
        hi = hi.max(bpm);
    }
    let pad = ((hi - lo) * 0.2).max(15.0);
    lo = (lo - pad).max(20.0);
    hi = (hi + pad).min(300.0);
    if hi - lo < 40.0 {
        // Degenerate (few / equal tempos): centre a fixed span on the midpoint.
        let mid = ((lo + hi) * 0.5).clamp(40.0, 280.0);
        lo = mid - 20.0;
        hi = mid + 20.0;
    }
    (lo, hi)
}

/// Draw the arrangement view with track headers and timeline.
///
/// Returns `Some(PatternId)` if a placement was double-clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(super) fn draw_arrangement(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    current_tick: u64,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<synth_sequencer::SharedSong>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) -> Option<PatternId> {
    use egui_remixicon::icons as ri;
    let t = theme();
    let track_count = data.tracks.len();

    // Mirror the engine's loop region into view_state so MCP-set and
    // song-repeat loops surface as ruler markers without having to be
    // re-set via the right-click menu first.
    let (engine_loop_enabled, engine_loop_start, engine_loop_end) =
        handle.state.transport.loop_state();
    if engine_loop_enabled && engine_loop_end.0 > engine_loop_start.0 {
        view_state.loop_start_tick = Some(engine_loop_start);
        view_state.loop_end_tick = Some(engine_loop_end);
    }

    // ── Empty state: show "Add Track" button ──
    if track_count == 0 {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Empty song")
                    .size(16.0)
                    .color(t.colors.text_dim),
            );
            ui.add_space(t.spacing.md);
            if ui
                .button((
                    RichText::new(ri::ADD_LINE).color(t.colors.accent_green),
                    RichText::new("Add Track").color(t.colors.accent_green),
                ))
                .clicked()
            {
                let _ = song.write().create_track("Track 1");
            }
        });
        return None;
    }

    // Calculate timeline extent
    let ticks_per_bar = data.time_sig.ticks_per_bar() as u64;
    let ticks_per_beat = data.time_sig.ticks_per_beat() as u64;
    let beats_per_bar = data.time_sig.numerator as u64;

    let pixels_per_beat = PIXELS_PER_BEAT * view_state.zoom_level;

    draw_arrangement_toolbar(ui, data, view_state);

    let song_bars = if ticks_per_bar > 0 {
        data.song_end_tick.div_ceil(ticks_per_bar) as u32
    } else {
        MIN_VISIBLE_BARS
    };
    let total_bars = song_bars.max(MIN_VISIBLE_BARS) + 2;
    let total_beats = total_bars as f32 * beats_per_bar as f32;
    let timeline_width = total_beats * pixels_per_beat;

    let mut double_clicked_pattern: Option<PatternId> = None;

    // ── Shared scroll state ──
    // The right-side timeline owns the vertical scroll. The left header panel
    // mirrors the same y-offset so headers stay aligned with their rows.
    let scroll_salt = "seq_scroll";
    let scroll_id = super::scroll_state_id(ui, scroll_salt);
    let header_v_offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
        .map(|s| s.offset.y)
        .unwrap_or(0.0);

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_track_headers(&mut ctx, ui, header_v_offset);
    }

    // ── Timeline area (right side, uses painter for performance) ──
    // The scroll area's actual ID = ui.make_persistent_id(IdSalt::new(salt))

    // Pre-set scroll offset for auto-follow before showing the scroll area.
    // While playing we continuously keep the playhead ~30% from the right edge.
    // While stopped we only scroll inside the settle window right after a
    // transport action (◀◀/▶▶ jump, ruler seek, stop-to-cursor, Go to
    // playhead), and only if the marker landed off-screen — outside that
    // window manual scrolling is never fought.
    if view_state.auto_follow_playhead && ticks_per_beat > 0 {
        let playhead_x_offset = current_tick as f32 / ticks_per_beat as f32 * pixels_per_beat;
        let visible_width = ui.available_width();
        let mut scroll_state =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
        let current_offset = scroll_state.offset.x;

        let target_offset = if is_playing {
            // Continuous follow: keep the playhead ~30% from the right edge.
            Some((playhead_x_offset - visible_width * 0.7).max(0.0))
        } else if view_state.follow_settle_frames > 0 {
            // Settle window after a transport action: recenter the marker ~30%
            // from the left so the music ahead of it stays visible. An explicit
            // jump (`force_reveal`: Go to start / phrase ◀◀ ▶▶ / Go to playhead)
            // always recenters; a passive reveal (ruler seek / stop) only does
            // so when the marker is off-screen, so a click on a visible spot is
            // not yanked around. A small slack keeps it off the edge.
            let margin = pixels_per_beat;
            let off_screen = playhead_x_offset < current_offset + margin
                || playhead_x_offset > current_offset + visible_width - margin;
            (view_state.force_reveal || off_screen)
                .then(|| (playhead_x_offset - visible_width * 0.3).max(0.0))
        } else {
            // Stopped with no recent transport action: leave the user's
            // scroll position alone.
            None
        };

        if let Some(target_offset) = target_offset {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), scroll_id);
            view_state.last_auto_scroll_offset = Some(target_offset);
        }
    }

    // ── Pinned section and bar-number strips (mirror horizontal scroll) ──
    // The form lane and ruler are pinned; the tempo lane stays in the scrolling canvas
    // below and scrolls away with the tracks. Read the offset after auto-follow
    // has set it so the strip tracks playback with no frame of lag. Mirrors the
    // piano roll's pinned ruler (`draw_pr_ruler_strip`). Drawn after the left
    // header panel so it spans only the timeline width right of the headers.
    let ruler_offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
        .map(|s| s.offset)
        .unwrap_or_default();
    egui::Panel::top("seq_sections")
        .exact_size(SECTION_LANE_HEIGHT)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let mut ctx = ArrangementCtx {
                data,
                song,
                handle: &mut *handle,
                view_state: &mut *view_state,
                undo_manager: &mut *undo_manager,
                instruments,
            };
            draw_arrangement_section_strip(
                &mut ctx,
                ui,
                ruler_offset.x,
                ticks_per_bar,
                ticks_per_beat,
                pixels_per_beat,
            );
        });
    egui::Panel::top("seq_ruler")
        .exact_size(RULER_HEIGHT)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let mut ctx = ArrangementCtx {
                data,
                song,
                handle: &mut *handle,
                view_state: &mut *view_state,
                undo_manager: &mut *undo_manager,
                instruments,
            };
            draw_arrangement_ruler_strip(
                &mut ctx,
                ui,
                ruler_offset.x,
                current_tick,
                ticks_per_bar,
                ticks_per_beat,
                pixels_per_beat,
                total_bars,
            );
        });

    let scroll_output = egui::ScrollArea::both()
        .id_salt(scroll_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut ctx = ArrangementCtx {
                data,
                song,
                handle: &mut *handle,
                view_state: &mut *view_state,
                undo_manager: &mut *undo_manager,
                instruments,
            };
            draw_arrangement_timeline(
                &mut ctx,
                ui,
                current_tick,
                beats_per_bar,
                ticks_per_bar,
                ticks_per_beat,
                pixels_per_beat,
                track_count,
                timeline_width,
                total_bars,
                &mut double_clicked_pattern,
            );
        });

    // Detect manual scrolling to disable auto-follow.
    if is_playing
        && let Some(expected) = view_state.last_auto_scroll_offset
        && super::user_scrolled_away(&scroll_output, expected)
    {
        view_state.auto_follow_playhead = false;
        view_state.last_auto_scroll_offset = None;
    }

    // Re-enable auto-follow when playback starts from stopped
    if !is_playing {
        view_state.last_auto_scroll_offset = None;
    }

    double_clicked_pattern
}

/// Draw the arrangement timeline (ruler, grid, placements, loop/tempo markers,
/// playhead) and handle its pointer interaction — the body of the timeline
/// `ScrollArea`, split out of [`draw_arrangement`]. Builds its own
/// `ArrangementCoords` from the painter rect and delegates the right-click menu
/// to [`draw_arrangement_context_menu`]. Sets `*double_clicked_pattern` when a
/// placement is opened in the piano roll.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw_arrangement_timeline(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    current_tick: u64,
    beats_per_bar: u64,
    ticks_per_bar: u64,
    ticks_per_beat: u64,
    pixels_per_beat: f32,
    track_count: usize,
    timeline_width: f32,
    total_bars: u32,
    double_clicked_pattern: &mut Option<PatternId>,
) {
    let data = ctx.data;
    let song = ctx.song;
    let handle = &mut *ctx.handle;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();

    let total_size = Vec2::new(
        timeline_width,
        TEMPO_LANE_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
    );
    let (response, painter) = ui.allocate_painter(total_size, Sense::click_and_drag());
    // Expose the canvas container to AccessKit / the egui-inspection MCP. Per-clip
    // drivability is out of scope for v1 — this makes the canvas locatable.
    expose(
        &response,
        egui::WidgetType::Other,
        "arrangement canvas",
        None,
    );
    let painter_rect = response.rect;

    let tl_x = painter_rect.min.x;
    // The bar-number ruler is now pinned in its own top strip
    // (`draw_arrangement_ruler_strip`), so it is no longer part of this canvas.
    // Keep `tl_y` as a "virtual ruler top" (canvas top minus RULER_HEIGHT) so
    // every existing offset formula — `tracks_top`, `y_to_row`, the tempo lane's
    // `lane_top = tl_y + RULER_HEIGHT`, the tempo band background — still lands
    // correctly with no other edits: `tracks_top` resolves to
    // `painter_rect.min.y + TEMPO_LANE_HEIGHT`, so the tempo lane occupies the
    // top `TEMPO_LANE_HEIGHT` of the canvas and the tracks follow below it.
    let tl_y = painter_rect.min.y - RULER_HEIGHT;

    // Painter-local geometry. The closures below delegate to it so the
    // rest of this function's call sites stay unchanged; later painter
    // extractions can take `&coords` directly.
    let coords = ArrangementCoords {
        tl_x,
        tl_y,
        ticks_per_beat,
        pixels_per_beat,
        track_count,
        snap_ticks: view_state.arrangement_snap_ticks as u64,
    };

    // Top of the track content area: below both the ruler and the tempo lane.
    let tracks_top = tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;

    // ── Tempo lane band (between the ruler and the track rows) ──
    let lane_top = tl_y + RULER_HEIGHT;
    let lane_rect = Rect::from_min_max(
        Pos2::new(tl_x, lane_top),
        Pos2::new(tl_x + timeline_width, tracks_top),
    );
    painter.rect_filled(lane_rect, 0.0, t.colors.bg_panel.gamma_multiply(0.5));
    // Bottom border closing the lane (the ruler's own bottom border is drawn in
    // the pinned ruler strip).
    painter.line_segment(
        [
            Pos2::new(tl_x, tracks_top),
            Pos2::new(tl_x + timeline_width, tracks_top),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    draw_arrangement_grid_lines(
        &painter,
        &coords,
        total_bars,
        beats_per_bar,
        ticks_per_bar,
        ticks_per_beat,
        track_count,
    );

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_track_rows(&mut ctx, &painter, &coords, track_count, timeline_width);
    }

    let placement_rects = {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_placements(&mut ctx, &painter, &coords)
    };

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        handle_arrangement_pointer(
            &mut ctx,
            ui,
            &response,
            &painter,
            &coords,
            &placement_rects,
            double_clicked_pattern,
            tracks_top,
        );
    }

    // ── Right-click context menu on timeline ──
    // Use stored position from secondary_clicked, not current hover
    let ctx_pos = view_state.context_menu_pos;
    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        response.context_menu(|ui| {
            draw_arrangement_context_menu(
                &mut ctx,
                ui,
                &coords,
                ctx_pos,
                &placement_rects,
                double_clicked_pattern,
                ticks_per_bar,
            );
        });
    }

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_loop_markers(&mut ctx, &painter, &coords, track_count);
    }

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_tempo_lane(&mut ctx, ui, &painter, &coords, timeline_width);
    }

    {
        let mut ctx = ArrangementCtx {
            data,
            song,
            handle: &mut *handle,
            view_state: &mut *view_state,
            undo_manager: &mut *undo_manager,
            instruments,
        };
        draw_arrangement_playhead(&mut ctx, &painter, &coords, current_tick, track_count);
    }
}

/// Draw and edit the pinned song-form lane above the bar ruler.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw_arrangement_section_strip(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    offset_x: f32,
    ticks_per_bar: u64,
    ticks_per_beat: u64,
    pixels_per_beat: f32,
) {
    let t = theme();
    let area = ui.max_rect();
    let coords = ArrangementCoords {
        tl_x: area.left() - offset_x,
        tl_y: area.top(),
        ticks_per_beat,
        pixels_per_beat,
        track_count: ctx.data.tracks.len(),
        snap_ticks: ctx.view_state.arrangement_snap_ticks as u64,
    };
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(area, 0.0, t.colors.bg_panel);
    painter.line_segment(
        [area.left_bottom(), area.right_bottom()],
        Stroke::new(1.0, t.colors.border),
    );
    let mut section_rects = Vec::with_capacity(ctx.data.sections.len());
    for section in &ctx.data.sections {
        let left = coords.tick_to_x(section.start.0);
        let right = coords.tick_to_x(section.end().0).max(left + 2.0);
        let rect = Rect::from_min_max(
            Pos2::new(left, area.top() + 2.0),
            Pos2::new(right, area.bottom() - 2.0),
        );
        section_rects.push(rect);

        let color = Color32::from_rgb(section.color.red, section.color.green, section.color.blue);
        let selected = ctx.view_state.selected_section == Some(section.id);
        painter.rect_filled(
            rect,
            3.0,
            color.gamma_multiply(if selected { 0.9 } else { 0.68 }),
        );
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(
                if selected { 2.0 } else { 1.0 },
                if selected {
                    Color32::WHITE
                } else {
                    color.gamma_multiply(1.25)
                },
            ),
            egui::StrokeKind::Inside,
        );

        let resize_rect = Rect::from_min_max(
            Pos2::new(
                (rect.right() - SECTION_RESIZE_ZONE).max(rect.left()),
                rect.top(),
            ),
            rect.right_bottom(),
        );
        let body_rect = Rect::from_min_max(
            rect.left_top(),
            Pos2::new(resize_rect.left().max(rect.left()), rect.bottom()),
        );
        let body = ui
            .interact(
                body_rect,
                ui.id().with(("arr_section_body", section.id.0)),
                Sense::click_and_drag(),
            )
            .on_hover_text(format!(
                "{} · {}\nDrag to move · Right-click to edit",
                section.name,
                section.kind.display_name()
            ));
        let resize = ui
            .interact(
                resize_rect,
                ui.id().with(("arr_section_resize", section.id.0)),
                Sense::click_and_drag(),
            )
            .on_hover_text("Drag to resize section");
        expose(
            &body,
            egui::WidgetType::Button,
            format!("{} section {}", section.kind.display_name(), section.name),
            None,
        );

        if body.clicked() || resize.clicked() {
            ctx.view_state.selected_section = Some(section.id);
        }
        if body.hovered() {
            ui.output_mut(|output| output.cursor_icon = CursorIcon::Grab);
        }
        if resize.hovered() || resize.dragged() {
            ui.output_mut(|output| output.cursor_icon = CursorIcon::ResizeHorizontal);
        }

        let label = if section.name.trim().is_empty() {
            section.kind.display_name()
        } else {
            section.name.as_str()
        };
        painter.with_clip_rect(rect.shrink(2.0)).text(
            Pos2::new(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );

        if body.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = ui.input(|input| input.pointer.press_origin())
        {
            let grab_tick = coords.x_to_tick(pos.x);
            ctx.view_state.drag = Some(DragState::MoveSection {
                section_id: section.id,
                original_start: section.start,
                current_start: section.start,
                grab_offset_ticks: Tick(grab_tick.saturating_sub(section.start.0)),
            });
        }
        if body.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = body.interact_pointer_pos()
            && let Some(DragState::MoveSection {
                section_id,
                current_start,
                grab_offset_ticks,
                ..
            }) = &mut ctx.view_state.drag
            && *section_id == section.id
        {
            let raw_start = coords.x_to_tick(pos.x).saturating_sub(grab_offset_ticks.0);
            *current_start = Tick(coords.snap_tick(raw_start));
        }
        if let Some(DragState::MoveSection {
            section_id,
            current_start,
            ..
        }) = &ctx.view_state.drag
            && *section_id == section.id
        {
            let moved_start = current_start.0;
            let ghost = Rect::from_min_max(
                Pos2::new(coords.tick_to_x(moved_start), rect.top()),
                Pos2::new(
                    coords.tick_to_x(moved_start.saturating_add(u64::from(section.length.0))),
                    rect.bottom(),
                ),
            );
            painter.rect_stroke(
                ghost,
                3.0,
                Stroke::new(2.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
        if body.drag_stopped_by(egui::PointerButton::Primary) {
            match ctx.view_state.drag.take() {
                Some(DragState::MoveSection {
                    section_id,
                    original_start,
                    current_start,
                    ..
                }) if section_id == section.id => {
                    if current_start != original_start {
                        let mut moved = section.clone();
                        moved.start = current_start;
                        edit_sections(ctx.song, ctx.undo_manager, |song| {
                            song.set_section(moved);
                        });
                    }
                }
                other => ctx.view_state.drag = other,
            }
        }

        if resize.drag_started_by(egui::PointerButton::Primary) {
            ctx.view_state.drag = Some(DragState::ResizeSection {
                section_id: section.id,
                start: section.start,
                original_length: section.length,
                current_length: section.length,
            });
        }
        if resize.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = resize.interact_pointer_pos()
            && let Some(DragState::ResizeSection {
                section_id,
                start,
                current_length,
                ..
            }) = &mut ctx.view_state.drag
            && *section_id == section.id
        {
            let minimum = section.start.0.saturating_add(coords.snap_ticks.max(1));
            let end = coords.snap_tick(coords.x_to_tick(pos.x)).max(minimum);
            *current_length =
                SeqDuration(u32::try_from(end.saturating_sub(start.0)).unwrap_or(u32::MAX));
        }
        if let Some(DragState::ResizeSection {
            section_id,
            current_length,
            ..
        }) = &ctx.view_state.drag
            && *section_id == section.id
        {
            let ghost = Rect::from_min_max(
                rect.left_top(),
                Pos2::new(
                    coords.tick_to_x(section.start.0.saturating_add(u64::from(current_length.0))),
                    rect.bottom(),
                ),
            );
            painter.rect_stroke(
                ghost,
                3.0,
                Stroke::new(2.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
        if resize.drag_stopped_by(egui::PointerButton::Primary) {
            match ctx.view_state.drag.take() {
                Some(DragState::ResizeSection {
                    section_id,
                    original_length,
                    current_length,
                    ..
                }) if section_id == section.id => {
                    if current_length != original_length {
                        let mut resized = section.clone();
                        resized.length = current_length;
                        edit_sections(ctx.song, ctx.undo_manager, |song| {
                            song.set_section(resized);
                        });
                    }
                }
                other => ctx.view_state.drag = other,
            }
        }

        egui::Popup::context_menu(&body)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                draw_section_context_menu(ctx, ui, section, ticks_per_bar);
            });
    }

    // Read empty-lane double-clicks directly instead of placing one large
    // background Response over the strip. An overlapping background Response
    // can capture the primary pointer before the section body/right-edge
    // Responses, which makes their drag gestures inert.
    let empty_lane_double_click = ui.input(|input| {
        input
            .pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    if empty_lane_double_click
        && let Some(pos) = ui.input(|input| input.pointer.interact_pos())
        && area.contains(pos)
        && !section_rects.iter().any(|rect| rect.contains(pos))
    {
        let start = coords.snap_tick(coords.x_to_tick(pos.x));
        let length = u32::try_from(ticks_per_bar.saturating_mul(4)).unwrap_or(u32::MAX);
        edit_sections(ctx.song, ctx.undo_manager, |song| {
            let _ = song.create_section(
                "Section",
                SectionKind::Custom,
                Tick(start),
                SeqDuration(length.max(1)),
            );
        });
    }

    if ctx.data.sections.is_empty() {
        painter.text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            "Double-click to add a section",
            egui::FontId::proportional(11.0),
            t.colors.text_dim,
        );
    }
}

/// Context menu for one arrangement section.
fn draw_section_context_menu(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    section: &synth_sequencer::ArrangementSection,
    ticks_per_bar: u64,
) {
    if ctx
        .view_state
        .editing_section_name
        .as_ref()
        .is_none_or(|(id, _)| *id != section.id)
    {
        ctx.view_state.editing_section_name = Some((section.id, section.name.clone()));
    }
    if ctx
        .view_state
        .editing_section_color
        .as_ref()
        .is_none_or(|(id, _)| *id != section.id)
    {
        ctx.view_state.editing_section_color = Some((
            section.id,
            [section.color.red, section.color.green, section.color.blue],
        ));
    }

    ui.label(RichText::new("Section").strong());
    if let Some((_, draft)) = &mut ctx.view_state.editing_section_name {
        let edit = ui.text_edit_singleline(draft);
        let apply_with_enter =
            edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.button("Apply name").clicked() || apply_with_enter {
            let name = draft.trim().to_owned();
            let mut renamed = section.clone();
            renamed.name = name;
            edit_sections(ctx.song, ctx.undo_manager, |song| {
                song.set_section(renamed);
            });
            ui.close();
        }
    }

    submenu_button(ui, format!("Type: {}", section.kind.display_name()), |ui| {
        const KINDS: [SectionKind; 9] = [
            SectionKind::Intro,
            SectionKind::Verse,
            SectionKind::PreChorus,
            SectionKind::Chorus,
            SectionKind::Bridge,
            SectionKind::Break,
            SectionKind::Solo,
            SectionKind::Outro,
            SectionKind::Custom,
        ];
        for kind in KINDS {
            if ui
                .selectable_label(section.kind == kind, kind.display_name())
                .clicked()
            {
                let mut changed = section.clone();
                changed.kind = kind;
                changed.color = kind.default_color();
                edit_sections(ctx.song, ctx.undo_manager, |song| {
                    song.set_section(changed);
                });
                ui.close();
            }
        }
    });

    if let Some((_, draft)) = &mut ctx.view_state.editing_section_color {
        ui.horizontal(|ui| {
            ui.label("Color");
            ui.color_edit_button_srgb(draft);
            if ui.button("Apply").clicked() {
                let mut changed = section.clone();
                changed.color = synth_sequencer::SectionColor::new(draft[0], draft[1], draft[2]);
                edit_sections(ctx.song, ctx.undo_manager, |song| {
                    song.set_section(changed);
                });
                ui.close();
            }
        });
    }

    ui.separator();
    if ui.button("Duplicate after").clicked() {
        let section = section.clone();
        edit_sections(ctx.song, ctx.undo_manager, |song| {
            let id = song.create_section(
                section.name.clone(),
                section.kind,
                section.end(),
                section.length,
            );
            if let Some(mut duplicate) = song.sections().iter().find(|item| item.id == id).cloned()
            {
                duplicate.color = section.color;
                song.set_section(duplicate);
            }
        });
        ui.close();
    }
    if ui.button("Set to 4 bars").clicked() {
        let mut changed = section.clone();
        changed.length =
            SeqDuration(u32::try_from(ticks_per_bar.saturating_mul(4)).unwrap_or(u32::MAX));
        edit_sections(ctx.song, ctx.undo_manager, |song| {
            song.set_section(changed);
        });
        ui.close();
    }
    ui.separator();
    if ui
        .button(RichText::new("Delete section").color(theme().colors.accent_red))
        .clicked()
    {
        let id = section.id;
        edit_sections(ctx.song, ctx.undo_manager, |song| {
            song.remove_section(id);
        });
        if ctx.view_state.selected_section == Some(id) {
            ctx.view_state.selected_section = None;
        }
        ui.close();
    }
}

/// Draw the pinned bar-number ruler strip and own all ruler interaction: seek
/// on click, pointing-hand hover, loop brackets, the playhead triangle, and the
/// loop/tempo right-click menu. Hosted by the fixed [`egui::Panel::top`] in
/// [`draw_arrangement`] so it stays put while the tracks (and the tempo lane)
/// scroll away. `offset_x` mirrors the timeline's horizontal scroll so the bar
/// numbers track the song. Mirrors the piano roll's `draw_pr_ruler_strip`.
#[allow(clippy::too_many_arguments)]
fn draw_arrangement_ruler_strip(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    offset_x: f32,
    current_tick: u64,
    ticks_per_bar: u64,
    ticks_per_beat: u64,
    pixels_per_beat: f32,
    total_bars: u32,
) {
    let t = theme();
    let area = ui.max_rect();
    // `tl_x = area.left() - offset_x` is the content origin's screen x (the same
    // value the scrolling canvas sees as `painter_rect.min.x`), so ticks map to
    // the same screen positions here as in the timeline below.
    let coords = ArrangementCoords {
        tl_x: area.left() - offset_x,
        tl_y: area.top(),
        ticks_per_beat,
        pixels_per_beat,
        track_count: ctx.data.tracks.len(),
        snap_ticks: ctx.view_state.arrangement_snap_ticks as u64,
    };
    let painter = ui.painter().with_clip_rect(area);
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);
    let x_to_tick = |x: f32| coords.x_to_tick(x);

    // `draw_ruler_labels` fills the visible strip background before painting the
    // labels at the offset tick positions.
    draw_ruler_labels(&painter, &t, area, total_bars, ticks_per_bar, tick_to_x);

    // ── Seek on click + pointing-hand hover with a strip-height indicator ──
    let resp = ui.interact(area, ui.id().with("arr_ruler_seek"), egui::Sense::click());
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        ctx.handle.send(EngineCommand::Seek {
            tick: Tick(x_to_tick(pos.x)),
        });
        ctx.view_state.reveal_playhead();
    }
    if resp.hovered()
        && let Some(pos) = ui.ctx().pointer_hover_pos()
        && area.contains(pos)
    {
        ui.output_mut(|o| {
            o.cursor_icon = CursorIcon::PointingHand;
        });
        // Strip-height indicator only — a full-height line across the tracks is
        // not reachable from the pinned strip.
        painter.line_segment(
            [
                Pos2::new(pos.x, area.top()),
                Pos2::new(pos.x, area.bottom()),
            ],
            Stroke::new(1.0, t.colors.text_dim.gamma_multiply(0.4)),
        );
    }

    // ── Loop-region brackets (the faint band over the rows stays in the canvas) ──
    if let (Some(loop_start), Some(loop_end)) =
        (ctx.view_state.loop_start_tick, ctx.view_state.loop_end_tick)
        && loop_end.0 > loop_start.0
    {
        for (x, dx) in [
            (tick_to_x(loop_start.0), 6.0),
            (tick_to_x(loop_end.0), -6.0),
        ] {
            painter.line_segment(
                [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
                Stroke::new(2.0, LOOP_COLOR),
            );
            painter.line_segment(
                [
                    Pos2::new(x, area.top() + 4.0),
                    Pos2::new(x + dx, area.top() + 4.0),
                ],
                Stroke::new(2.0, LOOP_COLOR),
            );
        }
    }

    // ── Playhead triangle (the vertical line stays in the canvas) ──
    if current_tick > 0 || ctx.data.song_end_tick > 0 {
        let playhead_x = tick_to_x(current_tick);
        if playhead_x >= area.left() && playhead_x <= area.right() {
            let tri_size = 6.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(playhead_x - tri_size, area.top()),
                    Pos2::new(playhead_x + tri_size, area.top()),
                    Pos2::new(playhead_x, area.top() + RULER_HEIGHT * 0.6),
                ],
                t.colors.accent_primary,
                Stroke::NONE,
            ));
        }
    }

    // Ruler bottom border.
    painter.line_segment(
        [
            Pos2::new(area.left(), area.bottom()),
            Pos2::new(area.right(), area.bottom()),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // ── Right-click: loop-region + tempo-map commands ──
    let ctx_pos_id = resp.id.with("ruler_ctx_pos");
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        ui.memory_mut(|m| m.data.insert_temp(ctx_pos_id, pos));
    }
    {
        let mut ctx2 = ArrangementCtx {
            data: ctx.data,
            song: ctx.song,
            handle: &mut *ctx.handle,
            view_state: &mut *ctx.view_state,
            undo_manager: &mut *ctx.undo_manager,
            instruments: ctx.instruments,
        };
        resp.context_menu(|ui| {
            let pos = ui
                .memory(|m| m.data.get_temp::<Pos2>(ctx_pos_id))
                .unwrap_or_else(|| ui.min_rect().min);
            draw_ruler_context_menu(&mut ctx2, ui, &coords, pos);
        });
    }
}

/// The ruler's right-click menu: loop-region commands (set start/end, clear) and
/// the "Tempo point here…" editor (add/edit BPM, ramp, remove). Split verbatim
/// out of [`draw_arrangement_context_menu`]'s former ruler branch; now attached
/// to the pinned ruler strip. Computes the target tick from `hover_pos` via the
/// strip's `coords`.
fn draw_ruler_context_menu(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    coords: &ArrangementCoords,
    hover_pos: Pos2,
) {
    let data = ctx.data;
    let song = ctx.song;
    let handle = &mut *ctx.handle;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let t = theme();
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    let snap_tick = |tick: u64| coords.snap_tick(tick);

    ui.set_min_width(180.0);

    let tick = Tick(x_to_tick(hover_pos.x));
    let snapped = snap_tick(tick.0);
    if ui.button("Set loop start here").clicked() {
        view_state.loop_start_tick = Some(Tick(snapped));
        if let Some(end) = view_state.loop_end_tick
            && end.0 > snapped
        {
            handle.send(EngineCommand::SetLoop {
                start: Tick(snapped),
                end,
                enabled: true,
            });
        }
        ui.close();
    }
    if ui.button("Set loop end here").clicked() {
        view_state.loop_end_tick = Some(Tick(snapped));
        if let Some(start) = view_state.loop_start_tick
            && snapped > start.0
        {
            handle.send(EngineCommand::SetLoop {
                start,
                end: Tick(snapped),
                enabled: true,
            });
        }
        ui.close();
    }
    if (view_state.loop_start_tick.is_some() || view_state.loop_end_tick.is_some())
        && danger_button(ui, "Clear loop").clicked()
    {
        view_state.loop_start_tick = None;
        view_state.loop_end_tick = None;
        handle.send(EngineCommand::SetLoop {
            start: Tick::ZERO,
            end: Tick::ZERO,
            enabled: false,
        });
        ui.close();
    }

    ui.separator();

    // Tempo automation at the clicked (snapped) tick.
    let existing: Option<(f32, bool)> = data
        .tempo_changes
        .iter()
        .find(|(t, _, _)| *t == snapped)
        .map(|(_, b, r)| (*b, *r));
    let default_bpm = existing.map_or_else(
        || {
            // Seed from the song's tempo at this position so editing an
            // existing curve point feels stable.
            song.try_read()
                .map(|s| s.tempo_at(Tick(snapped)).as_f32())
                .unwrap_or(120.0)
        },
        |(b, _)| b,
    );
    submenu_button(ui, "Tempo point here…", |ui| {
        // A position-specific point in the tempo *map*, distinct from the
        // song's global default (set via the transport tempo field).
        ui.label(RichText::new("Tempo-map point (not the song default)").color(t.colors.text_dim));
        ui.separator();
        // Live-apply BPM edit: the song is the buffer (like the knobs and the
        // ramp toggle). A per-frame `let mut bpm = default` local resets every
        // frame, so a mouse drag could never accumulate — instead seed from the
        // song's current value and write straight back on change. Undo is
        // coalesced to one entry per gesture (drag or keyboard edit); the two
        // gesture kinds are disjoint, so exactly one end event fires each time.
        // `default_bpm` already resolves to the existing point's bpm when one
        // exists, else the song's tempo at this tick.
        let mut bpm = default_bpm;
        // A newly-created point (drag with no existing change) is a step; when a
        // point exists we preserve its ramp flag while editing the bpm.
        let ramp = existing.is_some_and(|(_, r)| r);
        let resp = ui
            .horizontal(|ui| {
                ui.label("Tempo");
                ui.add(
                    egui::DragValue::new(&mut bpm)
                        .range(20.0..=300.0)
                        .speed(0.5)
                        .fixed_decimals(1)
                        .suffix(" BPM"),
                )
            })
            .inner;

        let undo_id = ui.id().with(("tempo_edit_old", snapped));
        if resp.drag_started() || resp.gained_focus() {
            // Snapshot the pre-edit state so the whole gesture is one undo.
            let old = existing.map(|(b, r)| (Bpm::new(b), r));
            ui.memory_mut(|m| m.data.insert_temp(undo_id, old));
        }
        if resp.changed() {
            SongMutationService::new(song).set_tempo_point(TempoPointEdit::new(
                Tick(snapped),
                Bpm::new(bpm),
                ramp,
            ));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            let old: Option<(Bpm, bool)> = ui
                .memory(|m| m.data.get_temp::<Option<(Bpm, bool)>>(undo_id))
                .flatten();
            let new = Some((Bpm::new(bpm), ramp));
            if old != new {
                undo_manager.push(crate::undo::UndoAction::SetTempo {
                    tick: Tick(snapped),
                    old,
                    new,
                });
            }
        }
        // Ramp toggle for an existing point. It applies immediately (the song
        // is the source of truth, re-read each frame), so no per-frame local
        // state that would fail to persist across frames.
        if let Some((existing_bpm, existing_ramp)) = existing {
            let mut ramp = existing_ramp;
            let resp = ui.checkbox(&mut ramp, "Ramp to next").on_hover_text(
                "Ramp linearly toward the next tempo point (accelerando/ritardando) instead of a step change.",
            );
            if resp.changed() {
                let bpm = Bpm::new(existing_bpm);
                SongMutationService::new(song).set_tempo_point(TempoPointEdit::new(
                    Tick(snapped),
                    bpm,
                    ramp,
                ));
                undo_manager.push(crate::undo::UndoAction::SetTempo {
                    tick: Tick(snapped),
                    old: Some((bpm, existing_ramp)),
                    new: Some((bpm, ramp)),
                });
            }
            ui.separator();
            if danger_button(ui, "Remove tempo change here").clicked() {
                if SongMutationService::new(song).remove_tempo_point(Tick(snapped)) {
                    undo_manager.push(crate::undo::UndoAction::SetTempo {
                        tick: Tick(snapped),
                        old: Some((Bpm::new(existing_bpm), existing_ramp)),
                        new: None,
                    });
                }
                ui.close();
            }
        }
    });
}

/// Draw the full-height bar/beat grid lines behind the arrangement track
/// rows. Split out of [`draw_arrangement_timeline`].
fn draw_arrangement_grid_lines(
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    total_bars: u32,
    beats_per_bar: u64,
    ticks_per_bar: u64,
    ticks_per_beat: u64,
    track_count: usize,
) {
    let t = theme();
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);

    // ── Full-height bar/beat grid lines ──
    for bar_idx in 0..total_bars {
        let bar_tick = bar_idx as u64 * ticks_per_bar;
        let x = tick_to_x(bar_tick);

        let line_bottom = tracks_top + track_count as f32 * TRACK_ROW_HEIGHT;
        painter.line_segment(
            [Pos2::new(x, tracks_top), Pos2::new(x, line_bottom)],
            Stroke::new(1.0, t.colors.border),
        );

        for beat in 1..beats_per_bar {
            let beat_tick = bar_tick + beat * ticks_per_beat;
            let bx = tick_to_x(beat_tick);
            painter.line_segment(
                [Pos2::new(bx, tracks_top), Pos2::new(bx, line_bottom)],
                Stroke::new(0.5, t.colors.border.gamma_multiply(0.4)),
            );
        }
    }
}

/// Draw the arrangement track-row backgrounds (highlight + zebra striping)
/// and the "double-click to create a pattern" discoverability hint. Split
/// out of [`draw_arrangement_timeline`].
fn draw_arrangement_track_rows(
    ctx: &mut ArrangementCtx<'_>,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    track_count: usize,
    timeline_width: f32,
) {
    let data = ctx.data;
    let view_state = &mut *ctx.view_state;
    let t = theme();
    let tl_x = coords.tl_x;
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;

    // ── Track row backgrounds ──
    for i in 0..track_count {
        let row_y = tracks_top + i as f32 * TRACK_ROW_HEIGHT;
        let is_highlighted = view_state.highlighted_track == Some(data.tracks[i].id);
        let bg = if is_highlighted {
            TRACK_HIGHLIGHT_FILL
        } else if i % 2 == 0 {
            TRACK_ROW_BG_EVEN
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

    // ── Discoverability hint when no placements exist yet ──
    if data.placements.is_empty() && !data.tracks.is_empty() {
        let row_y = tracks_top + TRACK_ROW_HEIGHT * 0.5;
        painter.text(
            Pos2::new(tl_x + 16.0, row_y),
            egui::Align2::LEFT_CENTER,
            "Double-click here to create a pattern",
            egui::FontId::proportional(13.0),
            t.colors.text_dim,
        );
    }
}

/// Draw all pattern placements (body fill, name label, note miniatures) and
/// return their hit-test rectangles for pointer interaction. Split out of
/// [`draw_arrangement_timeline`].
fn draw_arrangement_placements(
    ctx: &mut ArrangementCtx<'_>,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
) -> Vec<(Rect, PatternId, TrackId, u64)> {
    let data = ctx.data;
    let view_state = &mut *ctx.view_state;
    let instruments = ctx.instruments;
    let t = theme();
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);

    // ── Pattern placements ──
    let mut placement_rects: Vec<(Rect, PatternId, TrackId, u64)> = Vec::new();
    // One-shot per-frame colour cache so mini-note rendering is O(1)
    // per note instead of O(notes × instruments) with a hex parse.
    let inst_color_cache = build_instrument_colour_cache(instruments);

    for placement in &data.placements {
        let Some(row_idx) = data.tracks.iter().position(|t| t.id == placement.track_id) else {
            continue;
        };

        let x_start = tick_to_x(placement.start_tick);
        let x_end = tick_to_x(placement.end_tick);
        let row_y = tracks_top + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
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

        // Placement body uses the TRACK colour — uniform per track
        // so all placements on one track share the same row identity.
        let track_color = placement.color;

        let is_opened = view_state.opened_pattern == Some(placement.pattern_id);
        let fill_alpha = if is_opened { 140 } else { 100 };
        let fill = Color32::from_rgba_unmultiplied(
            track_color.r(),
            track_color.g(),
            track_color.b(),
            fill_alpha,
        );
        painter.rect_filled(rect, 3.0, fill);
        let stroke = if is_opened {
            Stroke::new(2.0, t.colors.accent_cyan)
        } else {
            Stroke::new(1.0, track_color)
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

        if !placement.note_miniatures.is_empty() {
            let mini_top = rect.min.y + 18.0;
            let mini_height = rect.max.y - mini_top - 2.0;
            if mini_height > 4.0 {
                let mini_width = rect.width() - 4.0;
                let fallback = MINIATURE_FALLBACK;
                let clipped = painter.with_clip_rect(rect);
                // All notes in a placement play the placement's track instrument.
                let inst_color =
                    cached_instrument_color(&inst_color_cache, placement.instrument, fallback);
                let repeat_count = if placement.loop_mode == PlacementLoopMode::Repeat {
                    placement
                        .effective_length
                        .0
                        .div_ceil(placement.pattern_length.0.max(1))
                } else {
                    1
                };
                let cycle_width = mini_width * placement.pattern_length.0 as f32
                    / placement.effective_length.0.max(1) as f32;
                // Pixel budget: drawing more notes than the box has horizontal
                // pixels is invisible. Walk the conceptual repeated note list
                // by index so even enormous placements draw bounded work while
                // preserving the miniature across their full length.
                #[allow(clippy::cast_sign_loss)]
                let budget = ((mini_width * MINIATURE_NOTES_PER_PIXEL) as usize).max(1);
                let candidate_count = placement
                    .note_miniatures
                    .len()
                    .saturating_mul(repeat_count as usize);
                let step = candidate_count.div_ceil(budget).max(1);
                let note_color = Color32::from_rgba_unmultiplied(
                    inst_color.r(),
                    inst_color.g(),
                    inst_color.b(),
                    200,
                );
                for candidate in (0..candidate_count).step_by(step) {
                    let cycle = candidate / placement.note_miniatures.len();
                    let mini =
                        &placement.note_miniatures[candidate % placement.note_miniatures.len()];
                    let nx = rect.min.x + 2.0 + (cycle as f32 + mini.start_frac) * cycle_width;
                    let nw = (mini.duration_frac * cycle_width).max(1.0);
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

    placement_rects
}

/// Handle pointer interaction over the arrangement track canvas: double-click
/// to open/create a pattern, hover tooltip, Ctrl+scroll zoom, primary-click
/// highlight clear, right-click capture, and placement drag/resize with live
/// ghosts. Ruler seek/hover live in the pinned strip
/// ([`draw_arrangement_ruler_strip`]). Split out of [`draw_arrangement_timeline`].
#[allow(clippy::too_many_arguments)]
fn handle_arrangement_pointer(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    placement_rects: &[(Rect, PatternId, TrackId, u64)],
    double_clicked_pattern: &mut Option<PatternId>,
    tracks_top: f32,
) {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    let snap_tick = |tick: u64| coords.snap_tick(tick);
    let y_to_row = |y: f32| coords.y_to_row(y);

    // ── Double-click → open piano roll or create pattern ──
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let mut hit_placement = false;
        for (rect, pattern_id, _, _) in placement_rects {
            if rect.contains(pos) {
                *double_clicked_pattern = Some(*pattern_id);
                hit_placement = true;
                break;
            }
        }
        // Double-click on empty area → create pattern + place + open
        if !hit_placement && let Some(row_idx) = y_to_row(pos.y) {
            let target_track = data.tracks[row_idx].id;
            let click_tick = x_to_tick(pos.x);
            let placement_tick = snap_tick(click_tick);
            let new_pat_id = {
                let mut song_w = song.write();
                let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                song_w.place_pattern(new_pat_id, target_track, Tick(placement_tick));
                *double_clicked_pattern = Some(new_pat_id);
                new_pat_id
            };
            record_created_pattern(song, undo_manager, new_pat_id);
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

        if let Some((pl, (rect, _, _, _))) = hovered_placement {
            let resize_hover = pos.x >= rect.max.x - PLACEMENT_RESIZE_ZONE;
            ui.output_mut(|o| {
                o.cursor_icon = if resize_hover {
                    CursorIcon::ResizeHorizontal
                } else {
                    CursorIcon::PointingHand
                };
            });
            // Tooltip with pattern info
            let instr_name = data
                .tracks
                .iter()
                .find(|t| t.id == pl.track_id)
                .map(|t| t.instrument_id)
                .and_then(|seq_id| instruments.iter().find(|inst| inst.id == seq_id))
                .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
            let tip_name = pl.pattern_name.clone();
            let tip_beats =
                pl.effective_length.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
            let source_beats =
                pl.pattern_length.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
            let tip_notes = pl.note_count;
            let loop_mode = pl.loop_mode;
            response.clone().on_hover_ui(|ui: &mut egui::Ui| {
                strong_label(ui, &tip_name, Some(t.colors.text_primary));
                ui.label(format!(
                    "{tip_beats:.1} beats ({source_beats:.1}-beat source)"
                ));
                ui.label(format!("Playback: {}", loop_mode.display_name()));
                ui.label(format!("{tip_notes} notes"));
                ui.label(format!("Instrument: {instr_name}"));
                if resize_hover {
                    ui.separator();
                    ui.label(format!(
                        "Drag to resize in {} mode; right-click to change",
                        loop_mode.display_name()
                    ));
                }
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
            view_state.zoom_level = (view_state.zoom_level * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    // ── Primary click on the canvas: clear the track highlight ──
    // (Ruler seek + hover indicator now live on the pinned ruler strip,
    // `draw_arrangement_ruler_strip`.)
    if response.clicked() {
        view_state.highlighted_track = None;
    }

    // ── Capture right-click position + set highlighted track ──
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        view_state.context_menu_pos = Some(pos);
        view_state.highlighted_track = y_to_row(pos.y).map(|i| data.tracks[i].id);
    }

    // ── Drag-to-move / resize placements ──
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some((rect, pat_id, trk_id, start_tick)) =
            placement_rects.iter().find(|(r, _, _, _)| r.contains(pos))
    {
        // Right-edge grab → resize. Body grab → move.
        if pos.x >= rect.max.x - PLACEMENT_RESIZE_ZONE {
            if let Some(p) = data.placements.iter().find(|p| {
                p.pattern_id == *pat_id && p.track_id == *trk_id && p.start_tick == *start_tick
            }) {
                let cur_len = SeqDuration((p.end_tick - p.start_tick) as u32);
                view_state.drag = Some(DragState::ResizePlacement {
                    pattern_id: *pat_id,
                    track_id: *trk_id,
                    start_tick: Tick(*start_tick),
                    original_length: cur_len,
                    current_length: cur_len,
                });
            }
        } else {
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
        && let Some(pos) = response.interact_pointer_pos()
    {
        match &mut view_state.drag {
            Some(DragState::DragPlacement {
                current_tick,
                current_track_id,
                grab_offset_ticks,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x).saturating_sub(grab_offset_ticks.0);
                *current_tick = Tick(snap_tick(raw_tick));
                if let Some(row_idx) = y_to_row(pos.y) {
                    *current_track_id = data.tracks[row_idx].id;
                }
            }
            Some(DragState::ResizePlacement {
                start_tick,
                current_length,
                ..
            }) => {
                let raw_end = x_to_tick(pos.x).max(start_tick.0 + 1);
                let snapped_end = snap_tick(raw_end);
                let end = snapped_end.max(start_tick.0 + 1);
                *current_length = SeqDuration((end - start_tick.0).max(1) as u32);
            }
            _ => {}
        }
    }
    // ── Release drag → move placement / commit resize ──
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        match view_state.drag.take() {
            Some(DragState::DragPlacement {
                pattern_id,
                track_id,
                start_tick,
                current_tick,
                current_track_id,
                ..
            }) => {
                if current_tick != start_tick || current_track_id != track_id {
                    let moved = song.write().move_placement(
                        pattern_id,
                        track_id,
                        start_tick,
                        current_track_id,
                        current_tick,
                    );
                    if moved {
                        undo_manager.push(crate::undo::UndoAction::MovePlacement {
                            pattern_id,
                            old_track_id: track_id,
                            old_start: start_tick,
                            new_track_id: current_track_id,
                            new_start: current_tick,
                        });
                    }
                }
            }
            Some(DragState::ResizePlacement {
                pattern_id,
                track_id,
                start_tick,
                original_length,
                current_length,
            }) => {
                if current_length != original_length {
                    // Resolve the underlying pattern's native length so
                    // we can decide whether the override clears (== pattern.length).
                    let pattern_len = song
                        .read()
                        .pattern(pattern_id)
                        .map(|p| p.length)
                        .unwrap_or(current_length);
                    let new_override = if current_length == pattern_len {
                        None
                    } else {
                        Some(current_length)
                    };
                    // Old override is the original length unless it
                    // matched the pattern's native length too.
                    let old_override = if original_length == pattern_len {
                        None
                    } else {
                        Some(original_length)
                    };
                    song.write().set_placement_length(
                        pattern_id,
                        track_id,
                        start_tick,
                        new_override,
                    );
                    undo_manager.push(crate::undo::UndoAction::SetPlacementLength {
                        pattern_id,
                        track_id,
                        start: start_tick,
                        old_length: old_override,
                        new_length: new_override,
                    });
                }
            }
            other => view_state.drag = other,
        }
    }

    // ── Draw resize ghost ──
    if let Some(DragState::ResizePlacement {
        track_id,
        start_tick,
        current_length,
        ..
    }) = &view_state.drag
        && let Some(row_idx) = data.tracks.iter().position(|t| t.id == *track_id)
    {
        let ghost_x = tick_to_x(start_tick.0);
        let ghost_end_x = tick_to_x(start_tick.0 + current_length.0 as u64);
        let ghost_y = tracks_top + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
        let ghost_rect = Rect::from_min_size(
            Pos2::new(ghost_x, ghost_y),
            Vec2::new(
                (ghost_end_x - ghost_x).max(4.0),
                TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0,
            ),
        );
        painter.rect_stroke(
            ghost_rect,
            3.0,
            Stroke::new(2.0, RESIZE_GHOST_STROKE),
            egui::StrokeKind::Outside,
        );
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
        if let Some(placement) = data.placements.iter().find(|p| p.pattern_id == *pattern_id) {
            let duration_ticks = placement.end_tick - placement.start_tick;
            let ghost_x = tick_to_x(current_tick.0);
            let ghost_end_x = tick_to_x(current_tick.0 + duration_ticks);
            if let Some(row_idx) = data.tracks.iter().position(|t| t.id == *current_track_id) {
                let ghost_y = tracks_top + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
                let ghost_rect = Rect::from_min_size(
                    Pos2::new(ghost_x, ghost_y),
                    Vec2::new(
                        (ghost_end_x - ghost_x).max(4.0),
                        TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0,
                    ),
                );
                painter.rect_filled(ghost_rect, 3.0, DRAG_GHOST_FILL);
                painter.rect_stroke(
                    ghost_rect,
                    3.0,
                    Stroke::new(1.5, DRAG_GHOST_STROKE),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

/// Draw the loop-region markers: the ruler brackets plus the faint band over
/// the track rows. Split out of [`draw_arrangement_timeline`].
fn draw_arrangement_loop_markers(
    ctx: &mut ArrangementCtx<'_>,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    track_count: usize,
) {
    let view_state = &mut *ctx.view_state;
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);

    // ── Loop region: faint band over the track rows. The ruler brackets are
    // drawn in the pinned ruler strip (`draw_arrangement_ruler_strip`). ──
    if let (Some(loop_start), Some(loop_end)) =
        (view_state.loop_start_tick, view_state.loop_end_tick)
        && loop_end.0 > loop_start.0
    {
        let x_a = tick_to_x(loop_start.0);
        let x_b = tick_to_x(loop_end.0);
        let line_bottom = tracks_top + track_count as f32 * TRACK_ROW_HEIGHT;
        let band_fill =
            Color32::from_rgba_unmultiplied(LOOP_COLOR.r(), LOOP_COLOR.g(), LOOP_COLOR.b(), 24);

        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x_a, tracks_top), Pos2::new(x_b, line_bottom)),
            0.0,
            band_fill,
        );
    }
}

/// Draw the tempo lane: the piecewise default/map curve, BPM axis labels, and
/// the interactive per-point handles (double/right-click to add, drag to move,
/// context menu to ramp/remove). Split out of [`draw_arrangement_timeline`].
#[allow(clippy::too_many_lines)]
fn draw_arrangement_tempo_lane(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    timeline_width: f32,
) {
    let data = ctx.data;
    let song = ctx.song;
    let undo_manager = &mut *ctx.undo_manager;
    let t = theme();
    let tl_x = coords.tl_x;
    let tl_y = coords.tl_y;
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;
    let lane_top = tl_y + RULER_HEIGHT;
    let lane_rect = Rect::from_min_max(
        Pos2::new(tl_x, lane_top),
        Pos2::new(tl_x + timeline_width, tracks_top),
    );
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    let snap_tick = |tick: u64| coords.snap_tick(tick);

    let tempo_color = TEMPO_MARKER;
    let lane_pad = 8.0;
    let y_hi = lane_top + lane_pad; // top edge → axis max
    let y_lo = tracks_top - lane_pad; // bottom edge → axis min

    // Dynamic BPM axis fitted to the map. Freeze it while a handle is being
    // dragged so vertical drag stays 1:1 (a live-rescaling axis makes the
    // point slip under the cursor); recompute + re-store when idle.
    let axis_id = ui.id().with("tempo_lane_axis");
    let dragging = ui.ctx().dragged_id().is_some_and(|d| {
        (0..data.tempo_changes.len()).any(|i| d == ui.id().with(("tempo_handle", i)))
    });
    let (bpm_min, bpm_max) = if dragging {
        ui.memory(|m| m.data.get_temp::<(f32, f32)>(axis_id))
            .unwrap_or_else(|| tempo_lane_range(data))
    } else {
        let range = tempo_lane_range(data);
        ui.memory_mut(|m| m.data.insert_temp(axis_id, range));
        range
    };
    let bpm_to_y = |bpm: f32| {
        let f = ((bpm - bpm_min) / (bpm_max - bpm_min)).clamp(0.0, 1.0);
        y_lo + f * (y_hi - y_lo)
    };
    let right_x = tl_x + timeline_width;

    // Leading segment governed by the *global default* tempo (up to the first
    // map point, or the whole lane when there are none). Drawn dim + labelled
    // so it reads as the default, distinct from the bright map points.
    let default_y = bpm_to_y(data.default_tempo);
    let dim = tempo_color.gamma_multiply(0.4);
    let first_x = data
        .tempo_changes
        .first()
        .map_or(right_x, |(t, _, _)| tick_to_x(*t));
    // Only when there is an actual default region — i.e. the first point is
    // past tick 0 (or there are no points). A first point at tick 0 governs
    // from the start, so drawing/labelling a default here would be misleading.
    if first_x > tl_x {
        painter.line_segment(
            [Pos2::new(tl_x, default_y), Pos2::new(first_x, default_y)],
            Stroke::new(1.0, dim),
        );
        if let Some((_, b0, _)) = data.tempo_changes.first() {
            // Step from the default level to the first point.
            painter.line_segment(
                [
                    Pos2::new(first_x, default_y),
                    Pos2::new(first_x, bpm_to_y(*b0)),
                ],
                Stroke::new(1.0, dim),
            );
        }
        painter.text(
            Pos2::new(tl_x + 3.0, default_y + 1.0),
            egui::Align2::LEFT_TOP,
            "default",
            egui::FontId::proportional(8.0),
            tempo_color.gamma_multiply(0.7),
        );
    }

    // Map segments: each point draws its outgoing segment to the next point
    // (step = flat-then-jump, ramp = sloped), the last one holding to the
    // right edge.
    for i in 0..data.tempo_changes.len() {
        let (_, bpm, ramp) = data.tempo_changes[i];
        let x = tick_to_x(data.tempo_changes[i].0);
        let y = bpm_to_y(bpm);
        let (next_x, next_y) = data
            .tempo_changes
            .get(i + 1)
            .map_or((right_x, y), |n| (tick_to_x(n.0), bpm_to_y(n.1)));
        if ramp {
            painter.line_segment(
                [Pos2::new(x, y), Pos2::new(next_x, next_y)],
                Stroke::new(1.5, tempo_color),
            );
        } else {
            painter.line_segment(
                [Pos2::new(x, y), Pos2::new(next_x, y)],
                Stroke::new(1.5, tempo_color),
            );
            if data.tempo_changes.get(i + 1).is_some() {
                painter.line_segment(
                    [Pos2::new(next_x, y), Pos2::new(next_x, next_y)],
                    Stroke::new(1.0, tempo_color.gamma_multiply(0.5)),
                );
            }
        }
    }

    // Axis scale labels at the lane's left edge (the padded extremes are
    // always clear of the curve), so the dynamic BPM range is legible.
    for (bpm, y, anchor) in [
        (bpm_max, y_hi, egui::Align2::LEFT_TOP),
        (bpm_min, y_lo, egui::Align2::LEFT_BOTTOM),
    ] {
        painter.text(
            Pos2::new(tl_x + 2.0, y),
            anchor,
            format!("{bpm:.0}"),
            egui::FontId::proportional(8.0),
            t.colors.text_dim,
        );
    }

    // ── Interaction (handles are drawn here so hover/drag glow reflects the
    // live response state) ──
    let y_to_bpm = |y: f32| {
        let f = ((y - y_lo) / (y_hi - y_lo)).clamp(0.0, 1.0);
        bpm_min + f * (bpm_max - bpm_min)
    };

    // Empty-lane background: double-click or right-click to add a point at the
    // clicked position. `lane_bg` sits above the canvas response, so it owns
    // both gestures here (the per-point handles, added later, sit above it).
    let lane_bg = ui.interact(lane_rect, ui.id().with("tempo_lane_bg"), Sense::click());
    // Expose the tempo lane background so the MCP can locate it (and target
    // add-point clicks) by name.
    expose(&lane_bg, egui::WidgetType::Panel, "tempo lane", None);
    let add_point = |song: &Arc<synth_sequencer::SharedSong>,
                     undo_manager: &mut crate::undo::UndoManager,
                     pos: Pos2| {
        let tick = snap_tick(x_to_tick(pos.x));
        if !data.tempo_changes.iter().any(|(t, _, _)| *t == tick) {
            let bpm = Bpm::new(y_to_bpm(pos.y).clamp(20.0, 300.0));
            SongMutationService::new(song).set_tempo_point(TempoPointEdit::new(
                Tick(tick),
                bpm,
                false,
            ));
            undo_manager.push(crate::undo::UndoAction::SetTempo {
                tick: Tick(tick),
                old: None,
                new: Some((bpm, false)),
            });
        }
    };
    if lane_bg.double_clicked()
        && let Some(pos) = lane_bg.interact_pointer_pos()
    {
        add_point(song, undo_manager, pos);
    }
    // Right-click → "Add tempo point" at the summoning position. Capture the
    // exact click so the menu (which persists across frames) knows where it
    // was opened.
    let ctx_pos_id = lane_bg.id.with("ctx_pos");
    if lane_bg.secondary_clicked()
        && let Some(p) = lane_bg.interact_pointer_pos()
    {
        ui.memory_mut(|m| m.data.insert_temp(ctx_pos_id, p));
    }
    lane_bg.context_menu(|ui| {
        let pos = ui
            .memory(|m| m.data.get_temp::<Pos2>(ctx_pos_id))
            .unwrap_or_else(|| ui.min_rect().min);
        let snapped = snap_tick(x_to_tick(pos.x));
        let existing = data
            .tempo_changes
            .iter()
            .find(|(t, _, _)| *t == snapped)
            .map(|(_, b, r)| (*b, *r));
        // A point already sits on this snapped tick → edit it; otherwise add.
        if let Some((eb, er)) = existing {
            let mut r = er;
            if ui.checkbox(&mut r, "Ramp to next").changed() {
                SongMutationService::new(song).set_tempo_point(TempoPointEdit::new(
                    Tick(snapped),
                    Bpm::new(eb),
                    r,
                ));
                undo_manager.push(crate::undo::UndoAction::SetTempo {
                    tick: Tick(snapped),
                    old: Some((Bpm::new(eb), er)),
                    new: Some((Bpm::new(eb), r)),
                });
                ui.close();
            }
            if danger_button(ui, "Remove tempo point").clicked() {
                if SongMutationService::new(song).remove_tempo_point(Tick(snapped)) {
                    undo_manager.push(crate::undo::UndoAction::SetTempo {
                        tick: Tick(snapped),
                        old: Some((Bpm::new(eb), er)),
                        new: None,
                    });
                }
                ui.close();
            }
        } else {
            let bpm_val = y_to_bpm(pos.y).clamp(20.0, 300.0);
            if ui
                .button(format!("Add tempo point · {bpm_val:.0} BPM"))
                .clicked()
            {
                add_point(song, undo_manager, pos);
                ui.close();
            }
        }
    });

    // Per-point handles. Clamping each point between its neighbours keeps the
    // list order — and thus the index-keyed interaction id — stable across a
    // drag. Drag moves tick+bpm (ramp preserved); right-click removes / toggles.
    let n = data.tempo_changes.len();
    for i in 0..n {
        let (tick, bpm, ramp) = data.tempo_changes[i];
        let center = Pos2::new(tick_to_x(tick), bpm_to_y(bpm));
        let handle_rect = Rect::from_center_size(center, Vec2::splat(12.0));
        let id = ui.id().with(("tempo_handle", i));
        let resp = ui.interact(handle_rect, id, Sense::click_and_drag());
        // Expose each tempo point with its live BPM so the MCP can read/target it.
        expose(
            &resp,
            egui::WidgetType::Slider,
            format!("tempo point {i}"),
            Some(f64::from(bpm)),
        );

        if resp.drag_started() {
            ui.memory_mut(|m| m.data.insert_temp(id.with("old"), (tick, bpm, ramp)));
        }
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let lo = if i > 0 {
                data.tempo_changes[i - 1].0 + 1
            } else {
                0
            };
            let hi = if i + 1 < n {
                data.tempo_changes[i + 1].0.saturating_sub(1)
            } else {
                u64::MAX
            };
            // When neighbours are too close to leave any gap (`lo > hi`),
            // there is no room to move horizontally — keep the tick and drag
            // only the BPM. (`u64::clamp` would panic on `lo > hi`.)
            let new_tick = if lo <= hi {
                snap_tick(x_to_tick(pos.x)).clamp(lo, hi)
            } else {
                tick
            };
            let new_bpm = Bpm::new(y_to_bpm(pos.y).clamp(20.0, 300.0));
            SongMutationService::new(song).move_tempo_point(
                Tick(tick),
                TempoPointEdit::new(Tick(new_tick), new_bpm, ramp),
            );
            ui.memory_mut(|m| {
                m.data
                    .insert_temp(id.with("cur"), (new_tick, new_bpm.as_f32(), ramp));
            });
        }
        if resp.drag_stopped() {
            let old = ui.memory(|m| m.data.get_temp::<(u64, f32, bool)>(id.with("old")));
            let cur = ui.memory(|m| m.data.get_temp::<(u64, f32, bool)>(id.with("cur")));
            // Clear both so a later drag of whatever point next occupies this
            // index can't read a stale `cur` (the id is keyed by index).
            ui.memory_mut(|m| {
                m.data.remove::<(u64, f32, bool)>(id.with("old"));
                m.data.remove::<(u64, f32, bool)>(id.with("cur"));
            });
            if let (Some(o), Some(c)) = (old, cur)
                && o != c
            {
                undo_manager.push(crate::undo::UndoAction::MoveTempo {
                    old: (Tick(o.0), Bpm::new(o.1), o.2),
                    new: (Tick(c.0), Bpm::new(c.1), c.2),
                });
            }
        }

        resp.context_menu(|ui| {
            let mut r = ramp;
            if ui
                .checkbox(&mut r, "Ramp to next")
                .on_hover_text("Ramp linearly toward the next point instead of a step change.")
                .changed()
            {
                SongMutationService::new(song).set_tempo_point(TempoPointEdit::new(
                    Tick(tick),
                    Bpm::new(bpm),
                    r,
                ));
                undo_manager.push(crate::undo::UndoAction::SetTempo {
                    tick: Tick(tick),
                    old: Some((Bpm::new(bpm), ramp)),
                    new: Some((Bpm::new(bpm), r)),
                });
                ui.close();
            }
            if danger_button(ui, "Remove").clicked() {
                if SongMutationService::new(song).remove_tempo_point(Tick(tick)) {
                    undo_manager.push(crate::undo::UndoAction::SetTempo {
                        tick: Tick(tick),
                        old: Some((Bpm::new(bpm), ramp)),
                        new: None,
                    });
                }
                ui.close();
            }
        });

        // Draw the handle, lit up when hovered or dragged so it's clear which
        // point is grab/draggable. egui has no blur, so the glow is a couple
        // of concentric translucent rings.
        let hot = resp.hovered() || resp.dragged();
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if hot {
            for (r, a) in [(12.0, 22), (9.0, 40), (6.0, 70)] {
                painter.circle_filled(
                    center,
                    r,
                    Color32::from_rgba_unmultiplied(
                        TEMPO_MARKER.r(),
                        TEMPO_MARKER.g(),
                        TEMPO_MARKER.b(),
                        a,
                    ),
                );
            }
        }
        let radius = if hot { 5.0 } else { 3.5 };
        painter.circle_filled(center, radius, tempo_color);
        if ramp {
            painter.circle_stroke(center, radius + 2.0, Stroke::new(1.0, tempo_color));
        }
        painter.text(
            center + Vec2::new(7.0, -5.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{bpm:.0}"),
            egui::FontId::proportional(9.0),
            tempo_color,
        );
    }
}

/// Draw the playhead: a vertical line spanning the ruler and track rows with a
/// triangle head in the ruler. Split out of [`draw_arrangement_timeline`].
fn draw_arrangement_playhead(
    ctx: &mut ArrangementCtx<'_>,
    painter: &egui::Painter,
    coords: &ArrangementCoords,
    current_tick: u64,
    track_count: usize,
) {
    let data = ctx.data;
    let t = theme();
    let tl_y = coords.tl_y;
    let tracks_top = coords.tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT;
    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);

    // ── Playhead ──
    // The play-start / return position (the "cursor") is tracked by the
    // engine and drives Play / Stop, but it is intentionally not drawn
    // as a separate marker: while paused it sits behind the playhead and
    // reads as a misaligned "ghost". Stop simply snaps the playhead back
    // to it, the standard DAW behavior.
    let line_bottom = tracks_top + track_count as f32 * TRACK_ROW_HEIGHT;
    if current_tick > 0 || data.song_end_tick > 0 {
        let playhead_x = tick_to_x(current_tick);
        // Content top (tempo lane + tracks). The triangle head is drawn in the
        // pinned ruler strip; the virtual-ruler region above the canvas is not
        // painted here.
        let line_top = tl_y + RULER_HEIGHT;

        painter.line_segment(
            [
                Pos2::new(playhead_x, line_top),
                Pos2::new(playhead_x, line_bottom),
            ],
            Stroke::new(2.0, t.colors.accent_primary),
        );
    }
}

/// Right-click context menu for the arrangement timeline (ruler loop/tempo
/// commands, per-placement actions, and empty-area pattern creation). Split
/// out of [`draw_arrangement`]; takes `&ArrangementCoords` for the geometry
/// plus the live `EngineHandle` and the painter-local hit-test state.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw_arrangement_context_menu(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    coords: &ArrangementCoords,
    ctx_pos: Option<Pos2>,
    placement_rects: &[(Rect, PatternId, TrackId, u64)],
    double_clicked_pattern: &mut Option<PatternId>,
    ticks_per_bar: u64,
) {
    use egui_remixicon::icons as ri;
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let t = theme();
    let tl_y = coords.tl_y;
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    let snap_tick = |tick: u64| coords.snap_tick(tick);

    ui.set_min_width(180.0);
    let hover_pos = ctx_pos.unwrap_or(ui.min_rect().min);

    // The ruler's loop/tempo commands now live on the pinned ruler strip
    // (`draw_ruler_context_menu`); this menu handles the canvas below it —
    // per-placement actions and empty-area pattern creation.

    // Check if right-click is on an existing placement
    let clicked_placement = placement_rects
        .iter()
        .find(|(r, _, _, _)| r.contains(hover_pos));

    if let Some((_, pat_id, trk_id, start_tick)) = clicked_placement {
        let pat_id = *pat_id;
        let trk_id = *trk_id;
        let start_tick = *start_tick;

        if ui.button("Open in Piano Roll").clicked() {
            *double_clicked_pattern = Some(pat_id);
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
            // The inline rename editor lives in the piano-roll
            // toolbar — open it so the user can actually type.
            *double_clicked_pattern = Some(pat_id);
            ui.close();
        }

        let current_loop_mode = data
            .placements
            .iter()
            .find(|placement| {
                placement.pattern_id == pat_id
                    && placement.track_id == trk_id
                    && placement.start_tick == start_tick
            })
            .map_or(PlacementLoopMode::Repeat, |placement| placement.loop_mode);
        submenu_button(
            ui,
            format!("Playback: {}", current_loop_mode.display_name()),
            |ui| {
                for loop_mode in [PlacementLoopMode::Repeat, PlacementLoopMode::Clip] {
                    if ui
                        .selectable_label(current_loop_mode == loop_mode, loop_mode.display_name())
                        .clicked()
                    {
                        if loop_mode != current_loop_mode
                            && song.write().set_placement_loop_mode(
                                pat_id,
                                trk_id,
                                Tick(start_tick),
                                loop_mode,
                            )
                        {
                            undo_manager.push(crate::undo::UndoAction::SetPlacementLoopMode {
                                pattern_id: pat_id,
                                track_id: trk_id,
                                start: Tick(start_tick),
                                old_mode: current_loop_mode,
                                new_mode: loop_mode,
                            });
                        }
                        ui.close();
                    }
                }
            },
        );

        // Pattern length editing — free-input bars
        submenu_button(ui, "Set Length…", |ui| {
            let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
            let current_len = data
                .patterns
                .iter()
                .find(|p| p.id == pat_id)
                .map_or(ticks_per_bar, |p| p.length_ticks);
            let mut bars = (current_len / ticks_per_bar).max(1) as i32;
            ui.horizontal(|ui| {
                unit_drag_value(ui, &mut bars, 1..=64, 0.1, " bars");
                if ui.button("Apply").clicked() {
                    let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                    apply_pattern_length(song, undo_manager, pat_id, new_len);
                    ui.close();
                }
            });
            ui.separator();
            for bars_preset in [1_u32, 2, 4, 8, 16] {
                if ui.button(format!("{bars_preset} bar(s)")).clicked() {
                    let new_len = SeqDuration(bars_preset * ticks_per_bar);
                    apply_pattern_length(song, undo_manager, pat_id, new_len);
                    ui.close();
                }
            }
        });

        if ui.button("Duplicate Pattern").clicked() {
            let duplicated = {
                let mut song_w = song.write();
                song_w.duplicate_pattern(pat_id).inspect(|new_id| {
                    let pattern_length = song_w
                        .pattern(pat_id)
                        .map_or(SeqDuration::WHOLE, |p| p.length);
                    song_w.place_pattern(
                        *new_id,
                        trk_id,
                        Tick(start_tick + pattern_length.0 as u64),
                    );
                })
            };
            if let Some(new_id) = duplicated {
                record_created_pattern(song, undo_manager, new_id);
            }
            ui.close();
        }
        ui.separator();
        if ui.button("Remove from Timeline").clicked() {
            let mut captured: Option<synth_sequencer::PatternPlacement> = None;
            {
                let mut song_w = song.write();
                if let Some(p) = song_w
                    .arrangement()
                    .iter()
                    .find(|p| {
                        p.pattern_id == pat_id && p.track_id == trk_id && p.start.0 == start_tick
                    })
                    .cloned()
                {
                    song_w.remove_placement(pat_id, trk_id, Tick(start_tick));
                    captured = Some(p);
                }
            }
            if let Some(p) = captured {
                undo_manager.push(crate::undo::UndoAction::RemovePlacement { placement: p });
            }
            ui.close();
        }
        if danger_button(ui, "Delete Pattern").clicked() {
            let mut captured: Option<(
                synth_sequencer::Pattern,
                Vec<synth_sequencer::PatternPlacement>,
            )> = None;
            {
                let mut song_w = song.write();
                let placements: Vec<_> = song_w
                    .arrangement()
                    .iter()
                    .filter(|p| p.pattern_id == pat_id)
                    .cloned()
                    .collect();
                if let Some(deleted) = song_w.delete_pattern(pat_id) {
                    captured = Some((deleted, placements));
                }
            }
            if let Some((pat, plcs)) = captured {
                undo_manager.push(crate::undo::UndoAction::DeletePattern {
                    pattern: pat,
                    placements: plcs,
                });
            }
            ui.close();
        }
    } else {
        // Clicked on empty area — figure out track + tick. A negative offset
        // means the click landed in the tempo lane (between ruler and tracks);
        // guard it so a lane right-click doesn't fall through to a track-0
        // "New Pattern Here" menu (matches `y_to_row`'s `< 0` rejection).
        let row_offset = hover_pos.y - (tl_y + RULER_HEIGHT + TEMPO_LANE_HEIGHT);
        let row_idx = if TRACK_ROW_HEIGHT > 0.0 {
            (row_offset / TRACK_ROW_HEIGHT) as usize
        } else {
            0
        };
        let click_tick = x_to_tick(hover_pos.x);
        let bar_tick = snap_tick(click_tick);

        if row_offset >= 0.0 && row_idx < data.tracks.len() {
            let target_track = data.tracks[row_idx].id;
            ui.label(
                RichText::new(format!(
                    "Bar {}",
                    bar_tick.checked_div(ticks_per_bar).map_or(1, |q| q + 1)
                ))
                .color(t.colors.text_dim),
            );
            ui.separator();

            if ui.button((ri::ADD_LINE, "New Pattern Here")).clicked() {
                let new_pat_id = {
                    let mut song_w = song.write();
                    let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                    song_w.place_pattern(new_pat_id, target_track, Tick(bar_tick));
                    new_pat_id
                };
                record_created_pattern(song, undo_manager, new_pat_id);
                ui.close();
            }

            // Place existing pattern submenu
            if !data.patterns.is_empty() {
                submenu_button(ui, "Place Existing Pattern", |ui| {
                    for pat in &data.patterns {
                        let beats =
                            pat.length_ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                        if ui
                            .button(format!("{} ({:.0} beats)", pat.name, beats))
                            .clicked()
                        {
                            let placed =
                                song.write()
                                    .place_pattern(pat.id, target_track, Tick(bar_tick));
                            if placed {
                                record_placement(
                                    song,
                                    undo_manager,
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
}

fn draw_arrangement_track_headers(
    ctx: &mut ArrangementCtx<'_>,
    ui: &mut egui::Ui,
    header_v_offset: f32,
) {
    use egui_remixicon::icons as ri;
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();

    // ── Track header panel (left side, uses egui widgets) ──
    egui::Panel::left("seq_track_headers")
        .exact_size(TRACK_HEADER_WIDTH)
        .resizable(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            // Pinned form-lane label and ruler corner, matching the two fixed
            // strips above the right-side timeline.
            let (section_label_rect, _) = ui.allocate_exact_size(
                Vec2::new(TRACK_HEADER_WIDTH, SECTION_LANE_HEIGHT),
                Sense::hover(),
            );
            ui.painter()
                .rect_filled(section_label_rect, 0.0, theme().colors.bg_panel);
            ui.painter().text(
                section_label_rect.left_center() + Vec2::new(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "Sections",
                egui::FontId::proportional(11.0),
                theme().colors.text_secondary,
            );
            ui.allocate_space(Vec2::new(TRACK_HEADER_WIDTH, RULER_HEIGHT));

            egui::ScrollArea::vertical()
                .id_salt("seq_track_headers_scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .scroll_source(egui::scroll_area::ScrollSource::NONE)
                .vertical_scroll_offset(header_v_offset)
                .show(ui, |ui| {
                    // Tempo-lane label — first scroll item, so it stays aligned
                    // with the timeline's tempo lane as both scroll together.
                    let (lane_rect, _) = ui.allocate_exact_size(
                        Vec2::new(TRACK_HEADER_WIDTH, TEMPO_LANE_HEIGHT),
                        Sense::hover(),
                    );
                    ui.painter().text(
                        lane_rect.left_center() + Vec2::new(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        "Tempo",
                        egui::FontId::proportional(11.0),
                        theme().colors.text_secondary,
                    );

                    for (i, track) in data.tracks.iter().enumerate() {
                        let is_selected = view_state.selected_track == Some(track.id);
                        let is_highlighted = view_state.highlighted_track == Some(track.id);
                        let bg = if is_selected {
                            TRACK_HEADER_SELECTED_FILL
                        } else if is_highlighted {
                            TRACK_HIGHLIGHT_FILL
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
                                        if let Some((_, ref mut name_buf)) =
                                            view_state.editing_track_name
                                        {
                                            let edit =
                                                inline_editable_text(ui, name_buf, false, |te| {
                                                    te.desired_width(80.0)
                                                        .font(egui::FontId::proportional(12.0))
                                                });
                                            if edit.ended {
                                                let new_name = name_buf.clone();
                                                let tid = track.id;
                                                let old_name = track.name.clone();
                                                let mut applied = false;
                                                {
                                                    let mut song_w = song.write();
                                                    if let Some(t) = song_w.track_mut(tid)
                                                        && t.name != new_name
                                                    {
                                                        t.name = new_name.clone();
                                                        applied = true;
                                                    }
                                                }
                                                if applied {
                                                    undo_manager.push(
                                                        crate::undo::UndoAction::RenameTrack {
                                                            track_id: tid,
                                                            old_name,
                                                            new_name,
                                                        },
                                                    );
                                                }
                                                view_state.editing_track_name = None;
                                            }
                                        }
                                    } else {
                                        let name_resp = clickable_label(
                                            ui,
                                            RichText::new(&track.name)
                                                .size(12.0)
                                                .color(t.colors.text_primary),
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
                                            if danger_button(ui, "Delete Track").clicked() {
                                                let mut captured: Option<(
                                                    synth_sequencer::SequencerTrack,
                                                    usize,
                                                    Vec<synth_sequencer::PatternPlacement>,
                                                )> = None;
                                                {
                                                    let mut song_w = song.write();
                                                    let idx = song_w
                                                        .tracks()
                                                        .position(|t| t.id == track.id)
                                                        .unwrap_or(0);
                                                    let placements: Vec<_> = song_w
                                                        .arrangement()
                                                        .iter()
                                                        .filter(|p| p.track_id == track.id)
                                                        .cloned()
                                                        .collect();
                                                    if let Some(deleted) =
                                                        song_w.delete_track(track.id)
                                                    {
                                                        captured = Some((deleted, idx, placements));
                                                    }
                                                }
                                                if let Some((trk, idx, plcs)) = captured {
                                                    undo_manager.push(
                                                        crate::undo::UndoAction::DeleteTrack {
                                                            track: trk,
                                                            track_index: idx,
                                                            placements: plcs,
                                                        },
                                                    );
                                                }
                                                ui.close();
                                            }
                                        });
                                    }

                                    // Instrument selector row
                                    let inst_label = instruments
                                        .iter()
                                        .find(|inst| inst.id == track.instrument_id)
                                        .map_or_else(
                                            || "— (none) —".to_owned(),
                                            |inst| inst.name.clone(),
                                        );
                                    egui::ComboBox::from_id_salt(
                                        ui.id().with(("track_instr", track.id.0)),
                                    )
                                    .selected_text(RichText::new(&inst_label).size(11.0))
                                    .width(116.0)
                                    .show_ui(ui, |ui| {
                                        for inst in instruments {
                                            let seq_id = inst.id;
                                            let selected = track.instrument_id == seq_id;
                                            if ui.selectable_label(selected, &inst.name).clicked()
                                                && let mut song_w = song.write()
                                                && let Some(trk) = song_w.track_mut(track.id)
                                            {
                                                trk.instrument = seq_id;
                                            }
                                        }
                                    });

                                    // Mute / Solo row — shared icon toggles, so
                                    // these match the mixer strip's mute/solo.
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        if mute_toggle(ui, track.mute).clicked()
                                            && let mut song_w = song.write()
                                            && let Some(trk) = song_w.track_mut(track.id)
                                        {
                                            trk.toggle_mute();
                                        }

                                        if solo_toggle(ui, track.solo).clicked()
                                            && let mut song_w = song.write()
                                            && let Some(trk) = song_w.track_mut(track.id)
                                        {
                                            trk.toggle_solo();
                                        }

                                        // Reorder buttons (move up / down)
                                        let track_count_local = data.tracks.len();
                                        let up_enabled = i > 0;
                                        let down_enabled = i + 1 < track_count_local;
                                        if ui
                                            .add_enabled(
                                                up_enabled,
                                                egui::Button::new(
                                                    RichText::new(ri::ARROW_UP_S_LINE)
                                                        .size(10.0)
                                                        .color(t.colors.text_secondary),
                                                ),
                                            )
                                            .on_hover_text("Move track up")
                                            .clicked()
                                            && i > 0
                                        {
                                            song.write().reorder_track(i, i - 1);
                                        }
                                        if ui
                                            .add_enabled(
                                                down_enabled,
                                                egui::Button::new(
                                                    RichText::new(ri::ARROW_DOWN_S_LINE)
                                                        .size(10.0)
                                                        .color(t.colors.text_secondary),
                                                ),
                                            )
                                            .on_hover_text("Move track down")
                                            .clicked()
                                        {
                                            song.write().reorder_track(i, i + 1);
                                        }

                                        // Properties button (volume / pan / colour)
                                        let prop_btn = ui
                                            .button(
                                                RichText::new(ri::MORE_FILL)
                                                    .size(10.0)
                                                    .color(t.colors.text_secondary),
                                            )
                                            .on_hover_text(
                                                "Track properties (volume, pan, colour)",
                                            );
                                        egui::Popup::from_toggle_button_response(&prop_btn).show(
                                            |ui| {
                                                ui.set_min_width(220.0);
                                                strong_label(ui, "Track properties", None);
                                                ui.add_space(t.spacing.xs);

                                                // Volume
                                                let mut vol = track.volume.as_f32();
                                                labeled_row(
                                                    ui,
                                                    RichText::new("Vol").color(t.colors.text_dim),
                                                    |ui| {
                                                        if ui
                                                            .add(
                                                                egui::Slider::new(
                                                                    &mut vol,
                                                                    0.0..=1.0,
                                                                )
                                                                .show_value(true)
                                                                .fixed_decimals(2),
                                                            )
                                                            .changed()
                                                            && let mut song_w = song.write()
                                                            && let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                        {
                                                            trk.volume = NormalizedValue::new(vol);
                                                        }
                                                    },
                                                );

                                                let mut pan_bi = track.pan.as_f32();
                                                labeled_row(
                                                    ui,
                                                    RichText::new("Pan").color(t.colors.text_dim),
                                                    |ui| {
                                                        if ui
                                                            .add(
                                                                egui::Slider::new(
                                                                    &mut pan_bi,
                                                                    -1.0..=1.0,
                                                                )
                                                                .show_value(true)
                                                                .fixed_decimals(2),
                                                            )
                                                            .changed()
                                                            && let mut song_w = song.write()
                                                            && let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                        {
                                                            trk.pan = BipolarValue::new(pan_bi);
                                                        }
                                                    },
                                                );

                                                // Description (utility metadata, no undo).
                                                // Buffered so edits survive the per-frame
                                                // snapshot rebuild; committed on lost focus.
                                                ui.add_space(t.spacing.xs);
                                                ui.label(
                                                    RichText::new("Description")
                                                        .color(t.colors.text_dim),
                                                );
                                                let editing_this = view_state
                                                    .editing_track_description
                                                    .as_ref()
                                                    .map(|(id, _)| *id)
                                                    == Some(track.id);
                                                if editing_this {
                                                    if let Some((_, ref mut desc_buf)) =
                                                        view_state.editing_track_description
                                                    {
                                                        // Commit on every change so a popup
                                                        // dismissal (click-outside) can never
                                                        // strand an in-progress edit; `ended`
                                                        // just closes the edit session.
                                                        let edit = inline_editable_text(
                                                            ui,
                                                            desc_buf,
                                                            true,
                                                            |te| {
                                                                te.desired_rows(2)
                                                                    .desired_width(f32::INFINITY)
                                                                    .hint_text("Description")
                                                            },
                                                        );
                                                        if edit.response.changed() {
                                                            let new_desc = desc_buf.clone();
                                                            let mut song_w = song.write();
                                                            if let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                                && trk.description != new_desc
                                                            {
                                                                trk.description = new_desc;
                                                            }
                                                        }
                                                        if edit.ended {
                                                            view_state.editing_track_description =
                                                                None;
                                                        }
                                                    }
                                                } else {
                                                    let desc_text = if track.description.is_empty()
                                                    {
                                                        RichText::new("(click to add)")
                                                            .italics()
                                                            .color(t.colors.text_dim)
                                                    } else {
                                                        RichText::new(&track.description)
                                                            .color(t.colors.text_secondary)
                                                    };
                                                    if clickable_label(ui, desc_text).clicked() {
                                                        view_state.editing_track_description = Some(
                                                            (track.id, track.description.clone()),
                                                        );
                                                    }
                                                }

                                                ui.add_space(t.spacing.xs);
                                                ui.label(
                                                    RichText::new("Colour")
                                                        .color(t.colors.text_dim),
                                                );
                                                ui.horizontal_wrapped(|ui| {
                                                    for preset in
                                                        synth_sequencer::TrackColor::presets()
                                                    {
                                                        let c = Color32::from_rgb(
                                                            preset.r, preset.g, preset.b,
                                                        );
                                                        let selected = track.track_color == *preset;
                                                        let stroke = if selected {
                                                            Stroke::new(2.0, t.colors.accent_cyan)
                                                        } else {
                                                            Stroke::NONE
                                                        };
                                                        let (rect, resp) = ui.allocate_exact_size(
                                                            Vec2::new(18.0, 18.0),
                                                            Sense::click(),
                                                        );
                                                        // Expose each swatch so the MCP can recolor a
                                                        // track by picking a color by name (hex) and
                                                        // read which is currently selected.
                                                        expose_selected(
                                                            &resp,
                                                            egui::WidgetType::ColorButton,
                                                            format!(
                                                                "track color #{:02X}{:02X}{:02X}",
                                                                preset.r, preset.g, preset.b
                                                            ),
                                                            selected,
                                                        );
                                                        ui.painter().rect_filled(rect, 3.0, c);
                                                        ui.painter().rect_stroke(
                                                            rect,
                                                            3.0,
                                                            stroke,
                                                            egui::StrokeKind::Inside,
                                                        );
                                                        if resp.clicked()
                                                            && let mut song_w = song.write()
                                                            && let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                        {
                                                            trk.color = *preset;
                                                        }
                                                    }
                                                });
                                            },
                                        );
                                    });
                                });
                            });
                        });
                    }

                    // "+" button to add track
                    ui.add_space(t.spacing.xs);
                    if ui
                        .button((
                            RichText::new(ri::ADD_LINE)
                                .size(11.0)
                                .color(t.colors.accent_green),
                            RichText::new("Add Track")
                                .size(11.0)
                                .color(t.colors.accent_green),
                        ))
                        .clicked()
                    {
                        let mut song_w = song.write();
                        let count = song_w.track_count();
                        let _ = song_w.create_track(format!("Track {}", count + 1));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 80.0))),
            events,
            ..Default::default()
        }
    }

    fn pointer_button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn draw_section_test_frame(
        egui_ctx: &egui::Context,
        input: egui::RawInput,
        song: &Arc<synth_sequencer::SharedSong>,
        handle: &mut EngineHandle,
        view_state: &mut SequencerViewState,
        undo_manager: &mut crate::undo::UndoManager,
    ) {
        let data = collect_arrangement_data(song).expect("arrangement snapshot");
        let _ = egui_ctx.run_ui(input, |ui| {
            let mut arrangement_ctx = ArrangementCtx {
                data: &data,
                song,
                handle,
                view_state,
                undo_manager,
                instruments: &[],
            };
            draw_arrangement_section_strip(&mut arrangement_ctx, ui, 0.0, 3_840, 960, 40.0);
        });
    }

    #[test]
    fn section_body_drag_and_right_edge_resize_commit_on_release() {
        let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Drag test")));
        let section_id =
            song.write()
                .create_section("Verse", SectionKind::Verse, Tick(0), SeqDuration(15_360));
        let (_engine, mut handle) = synth_engine::SynthEngine::new();
        let mut view_state = SequencerViewState::new();
        let mut undo_manager = crate::undo::UndoManager::new();
        let egui_ctx = egui::Context::default();

        let body_start = Pos2::new(100.0, 14.0);
        let body_end = Pos2::new(220.0, 14.0);
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![egui::Event::PointerMoved(body_start)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![pointer_button(body_start, true)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![egui::Event::PointerMoved(body_end)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![pointer_button(body_end, false)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );

        let moved = song
            .read()
            .sections()
            .iter()
            .find(|section| section.id == section_id)
            .cloned()
            .expect("moved section");
        assert_eq!(moved.start, Tick(2_880));

        let resize_start = Pos2::new(755.0, 14.0);
        let resize_end = Pos2::new(720.0, 14.0);
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![egui::Event::PointerMoved(resize_start)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![pointer_button(resize_start, true)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![egui::Event::PointerMoved(resize_end)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );
        draw_section_test_frame(
            &egui_ctx,
            pointer_input(vec![pointer_button(resize_end, false)]),
            &song,
            &mut handle,
            &mut view_state,
            &mut undo_manager,
        );

        let resized = song
            .read()
            .sections()
            .iter()
            .find(|section| section.id == section_id)
            .cloned()
            .expect("resized section");
        assert_eq!(resized.length, SeqDuration(14_400));
        assert!(undo_manager.can_undo());
    }
}
