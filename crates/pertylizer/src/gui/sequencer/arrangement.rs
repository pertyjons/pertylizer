//! Arrangement timeline view: track headers, placement miniatures,
//! the arrangement toolbar, and timeline interaction (drag/resize/context menu).
//!
//! The snapshot DTOs (`ArrangementData` et al.) live in the parent module so
//! both this view and `draw_sequencer_view` can read them.

use super::*;

/// Collect arrangement data from song (short read-lock, then release).
pub(super) fn collect_arrangement_data(song: &Arc<RwLock<Song>>) -> Option<ArrangementData> {
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
            // Color and instrument from the track this placement belongs to.
            let track = song.track(p.track_id);
            let color = track
                .map(|t| track_color_to_egui(t.color))
                .unwrap_or(Color32::GRAY);
            let instrument = track.map(|t| t.instrument).unwrap_or_default();

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

                // Bound the snapshot at what the draw loop could ever use:
                // a placement is at most length_beats × PIXELS_PER_BEAT ×
                // MAX_ZOOM pixels wide, and past MINIATURE_NOTES_PER_PIXEL
                // notes per pixel drawing is invisible. Decimate evenly past
                // that (notes are sorted by start tick) so a pathologically
                // dense pattern cannot blow up the per-frame snapshot cost.
                #[allow(clippy::cast_sign_loss)]
                let budget =
                    ((length_beats * PIXELS_PER_BEAT * MAX_ZOOM * MINIATURE_NOTES_PER_PIXEL)
                        as usize)
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
                length_beats,
                note_miniatures,
            })
        })
        .collect();

    let time_sig = song.default_time_signature;
    let tempo_changes: Vec<(u64, f32)> = song
        .tempo_changes()
        .iter()
        .map(|tc| (tc.tick.0, tc.bpm.as_f32()))
        .collect();

    Some(ArrangementData {
        tracks,
        placements,
        patterns,
        time_sig,
        song_end_tick,
        tempo_changes,
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
            view_state.reveal_playhead();
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
    song: &'a Arc<RwLock<Song>>,
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
        let row_offset = y - (self.tl_y + RULER_HEIGHT);
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

    /// The ruler rectangle spanning the full timeline width.
    fn ruler_rect(&self, timeline_width: f32) -> Rect {
        Rect::from_min_size(
            Pos2::new(self.tl_x, self.tl_y),
            Vec2::new(timeline_width, RULER_HEIGHT),
        )
    }
}

/// Write a new length to a pattern and push the matching `SetPatternLength`
/// undo action — but only if the length actually changed. Shared by the
/// "Set Length…" submenu's free-input Apply branch and its bar presets.
fn apply_pattern_length(
    song: &Arc<RwLock<Song>>,
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
    song: &Arc<RwLock<Song>>,
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
    let scroll_id = ui.make_persistent_id(egui::Id::new(scroll_salt));
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
    // The scroll area's actual ID = ui.make_persistent_id(Id::new(salt))

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
            // Settle window: if the marker is off-screen, re-center it ~30%
            // from the left so the music ahead of it stays visible. A small
            // slack keeps it from hugging the edge.
            let margin = pixels_per_beat;
            let off_screen = playhead_x_offset < current_offset + margin
                || playhead_x_offset > current_offset + visible_width - margin;
            off_screen.then(|| (playhead_x_offset - visible_width * 0.3).max(0.0))
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
        RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
    );
    let (response, painter) = ui.allocate_painter(total_size, Sense::click_and_drag());
    let painter_rect = response.rect;

    let tl_x = painter_rect.min.x;
    let tl_y = painter_rect.min.y;

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

    let tick_to_x = |tick_val: u64| coords.tick_to_x(tick_val);
    // Helper: x position to tick
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    // Single snap unit shared by placement create, drag, resize, and
    // loop-region — see `snap_to_step` for the underlying math.
    let snap_tick = |tick: u64| coords.snap_tick(tick);
    // Helper: y position to track row index
    let y_to_row = |y: f32| coords.y_to_row(y);

    // ── Ruler (bar/beat numbers) ──
    let ruler_rect = coords.ruler_rect(timeline_width);
    draw_ruler_labels(
        &painter,
        &t,
        ruler_rect,
        total_bars,
        ticks_per_bar,
        tick_to_x,
    );

    // ── Full-height bar/beat grid lines ──
    for bar_idx in 0..total_bars {
        let bar_tick = bar_idx as u64 * ticks_per_bar;
        let x = tick_to_x(bar_tick);

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
        let row_y = tl_y + RULER_HEIGHT + TRACK_ROW_HEIGHT * 0.5;
        painter.text(
            Pos2::new(tl_x + 16.0, row_y),
            egui::Align2::LEFT_CENTER,
            "Double-click here to create a pattern",
            egui::FontId::proportional(13.0),
            t.colors.text_dim,
        );
    }

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
        let row_y = tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
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
                // Pixel budget: drawing more notes than the box has
                // horizontal pixels is invisible, so decimate evenly
                // (notes are sorted by start tick, so every Nth note
                // preserves the pattern's shape over its full length).
                #[allow(clippy::cast_sign_loss)]
                let budget = ((mini_width * MINIATURE_NOTES_PER_PIXEL) as usize).max(1);
                let step = placement.note_miniatures.len().div_ceil(budget).max(1);
                let note_color = Color32::from_rgba_unmultiplied(
                    inst_color.r(),
                    inst_color.g(),
                    inst_color.b(),
                    200,
                );
                for mini in placement.note_miniatures.iter().step_by(step) {
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
            {
                let mut song_w = song.write();
                let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                song_w.place_pattern(new_pat_id, target_track, Tick(placement_tick));
                *double_clicked_pattern = Some(new_pat_id);
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
            ui.output_mut(|o| {
                o.cursor_icon = CursorIcon::PointingHand;
            });
            // Tooltip with pattern info
            let instr_name = data
                .tracks
                .iter()
                .find(|t| t.id == pl.track_id)
                .map(|t| t.instrument_id)
                .and_then(|seq_id| instruments.iter().find(|inst| inst.id == seq_id.into()))
                .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
            let tip_name = pl.pattern_name.clone();
            let tip_beats = pl.length_beats;
            let tip_notes = pl.note_count;
            response.clone().on_hover_ui(|ui: &mut egui::Ui| {
                strong_label(ui, &tip_name, Some(t.colors.text_primary));
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
            view_state.zoom_level = (view_state.zoom_level * factor).clamp(MIN_ZOOM, MAX_ZOOM);
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
            view_state.reveal_playhead();
        } else {
            view_state.highlighted_track = None;
        }
    }

    // ── Ruler hover: pointing hand cursor + indicator line ──
    if response.hovered()
        && let Some(pos) = ui.ctx().pointer_hover_pos()
        && ruler_rect.contains(pos)
    {
        ui.output_mut(|o| {
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

    // ── Drag-to-move / resize placements ──
    const PLACEMENT_RESIZE_ZONE: f32 = 8.0;
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
        let ghost_y = tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
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
                let ghost_y =
                    tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
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
                ruler_rect,
                ctx_pos,
                &placement_rects,
                double_clicked_pattern,
                ticks_per_bar,
            );
        });
    }

    // ── Loop region markers (in ruler + faint band over rows) ──
    if let (Some(loop_start), Some(loop_end)) =
        (view_state.loop_start_tick, view_state.loop_end_tick)
        && loop_end.0 > loop_start.0
    {
        let x_a = tick_to_x(loop_start.0);
        let x_b = tick_to_x(loop_end.0);
        let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
        let band_fill =
            Color32::from_rgba_unmultiplied(LOOP_COLOR.r(), LOOP_COLOR.g(), LOOP_COLOR.b(), 24);

        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x_a, tl_y + RULER_HEIGHT),
                Pos2::new(x_b, line_bottom),
            ),
            0.0,
            band_fill,
        );

        for (x, dx) in [(x_a, 6.0), (x_b, -6.0)] {
            painter.line_segment(
                [Pos2::new(x, tl_y), Pos2::new(x, tl_y + RULER_HEIGHT)],
                Stroke::new(2.0, LOOP_COLOR),
            );
            painter.line_segment(
                [Pos2::new(x, tl_y + 4.0), Pos2::new(x + dx, tl_y + 4.0)],
                Stroke::new(2.0, LOOP_COLOR),
            );
        }
    }

    // ── Tempo change markers on the ruler ──
    if !data.tempo_changes.is_empty() {
        let tempo_color = TEMPO_MARKER;
        for (tick, bpm) in &data.tempo_changes {
            let x = tick_to_x(*tick);
            // Small flag: vertical tick + label "120.0".
            painter.line_segment(
                [Pos2::new(x, tl_y + 1.0), Pos2::new(x, tl_y + RULER_HEIGHT)],
                Stroke::new(1.5, tempo_color),
            );
            painter.text(
                Pos2::new(x + 2.0, tl_y + 12.0),
                egui::Align2::LEFT_TOP,
                format!("{bpm:.0}"),
                egui::FontId::proportional(9.0),
                tempo_color,
            );
        }
    }

    // ── Playhead ──
    // The play-start / return position (the "cursor") is tracked by the
    // engine and drives Play / Stop, but it is intentionally not drawn
    // as a separate marker: while paused it sits behind the playhead and
    // reads as a misaligned "ghost". Stop simply snaps the playhead back
    // to it, the standard DAW behavior.
    let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
    if current_tick > 0 || data.song_end_tick > 0 {
        let playhead_x = tick_to_x(current_tick);
        let line_top = tl_y;

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
    ruler_rect: Rect,
    ctx_pos: Option<Pos2>,
    placement_rects: &[(Rect, PatternId, TrackId, u64)],
    double_clicked_pattern: &mut Option<PatternId>,
    ticks_per_bar: u64,
) {
    use egui_remixicon::icons as ri;
    let data = ctx.data;
    let song = ctx.song;
    let handle = &mut *ctx.handle;
    let view_state = &mut *ctx.view_state;
    let undo_manager = &mut *ctx.undo_manager;
    let t = theme();
    let tl_y = coords.tl_y;
    let x_to_tick = |x: f32| coords.x_to_tick(x);
    let snap_tick = |tick: u64| coords.snap_tick(tick);

    ui.set_min_width(180.0);
    let hover_pos = ctx_pos.unwrap_or(ui.min_rect().min);

    // Right-click on the ruler shows loop-region commands.
    if ruler_rect.contains(hover_pos) {
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
        let existing_bpm: Option<f32> = data
            .tempo_changes
            .iter()
            .find(|(t, _)| *t == snapped)
            .map(|(_, b)| *b);
        let default_bpm = existing_bpm.unwrap_or_else(|| {
            // Seed from the song's tempo at this position so
            // dragging from a curve point feels stable.
            song.try_read()
                .map(|s| s.tempo_at(Tick(snapped)).as_f32())
                .unwrap_or(120.0)
        });
        ui.menu_button("Set tempo here…", |ui| {
            let mut bpm = default_bpm;
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut bpm)
                        .range(20.0..=300.0)
                        .speed(0.5)
                        .fixed_decimals(1)
                        .suffix(" BPM"),
                );
                if ui.button("Apply").clicked() {
                    let new_bpm = Bpm::new(bpm);
                    let old_bpm = existing_bpm.map(Bpm::new);
                    if old_bpm != Some(new_bpm) {
                        song.write().set_tempo_at(Tick(snapped), new_bpm);
                        undo_manager.push(crate::undo::UndoAction::SetTempo {
                            tick: Tick(snapped),
                            old_bpm,
                            new_bpm: Some(new_bpm),
                        });
                    }
                    ui.close();
                }
            });
            if let Some(existing) = existing_bpm {
                ui.separator();
                if danger_button(ui, "Remove tempo change here").clicked() {
                    if song.write().remove_tempo_change(Tick(snapped)) {
                        undo_manager.push(crate::undo::UndoAction::SetTempo {
                            tick: Tick(snapped),
                            old_bpm: Some(Bpm::new(existing)),
                            new_bpm: None,
                        });
                    }
                    ui.close();
                }
            }
        });
        return;
    }

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

        // Pattern length editing — free-input bars
        ui.menu_button("Set Length…", |ui| {
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
            {
                let mut song_w = song.write();
                if let Some(new_id) = song_w.duplicate_pattern(pat_id) {
                    let pattern_length = song_w
                        .pattern(pat_id)
                        .map_or(SeqDuration::WHOLE, |p| p.length);
                    song_w.place_pattern(
                        new_id,
                        trk_id,
                        Tick(start_tick + pattern_length.0 as u64),
                    );
                }
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
        // Clicked on empty area — figure out track + tick
        let row_offset = hover_pos.y - (tl_y + RULER_HEIGHT);
        let row_idx = if TRACK_ROW_HEIGHT > 0.0 {
            (row_offset / TRACK_ROW_HEIGHT) as usize
        } else {
            0
        };
        let click_tick = x_to_tick(hover_pos.x);
        let bar_tick = snap_tick(click_tick);

        if row_idx < data.tracks.len() {
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
                {
                    let mut song_w = song.write();
                    let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                    song_w.place_pattern(new_pat_id, target_track, Tick(bar_tick));
                }
                ui.close();
            }

            // Place existing pattern submenu
            if !data.patterns.is_empty() {
                ui.menu_button("Place Existing Pattern", |ui| {
                    for pat in &data.patterns {
                        let beats =
                            pat.length_ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                        if ui
                            .button(format!("{} ({:.0} beats)", pat.name, beats))
                            .clicked()
                        {
                            song.write()
                                .place_pattern(pat.id, target_track, Tick(bar_tick));
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

            // Ruler corner placeholder (pinned, outside the scroll area)
            ui.allocate_space(Vec2::new(TRACK_HEADER_WIDTH, RULER_HEIGHT));

            egui::ScrollArea::vertical()
                .id_salt("seq_track_headers_scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .scroll_source(egui::scroll_area::ScrollSource::NONE)
                .vertical_scroll_offset(header_v_offset)
                .show(ui, |ui| {
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
                                            } else if !resp.has_focus() {
                                                // Grab focus only on the first
                                                // frame so clicking elsewhere
                                                // commits naturally via
                                                // lost_focus.
                                                resp.request_focus();
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
                                        .find(|inst| inst.id == track.instrument_id.into())
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
                                            let Ok(seq_id) = SeqInstrumentId::try_from(inst.id)
                                            else {
                                                continue;
                                            };
                                            let selected = track.instrument_id == seq_id;
                                            if ui.selectable_label(selected, &inst.name).clicked()
                                                && let mut song_w = song.write()
                                                && let Some(trk) = song_w.track_mut(track.id)
                                            {
                                                trk.instrument = seq_id;
                                            }
                                        }
                                    });

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
                                            && let mut song_w = song.write()
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
                                                        let resp = ui.add(
                                                            egui::TextEdit::multiline(desc_buf)
                                                                .desired_rows(2)
                                                                .desired_width(f32::INFINITY)
                                                                .hint_text("Description"),
                                                        );
                                                        // Commit on every change so a popup
                                                        // dismissal (click-outside) can never
                                                        // strand an in-progress edit; lost_focus
                                                        // just ends the edit session.
                                                        if resp.changed() {
                                                            let new_desc = desc_buf.clone();
                                                            let mut song_w = song.write();
                                                            if let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                                && trk.description != new_desc
                                                            {
                                                                trk.description = new_desc;
                                                            }
                                                        }
                                                        if resp.lost_focus() {
                                                            view_state.editing_track_description =
                                                                None;
                                                        } else if !resp.has_focus() {
                                                            resp.request_focus();
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
