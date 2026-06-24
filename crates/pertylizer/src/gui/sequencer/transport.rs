//! Transport control bar and the shared timeline-ruler strip.

use super::*;

// ============================================================================
// SHARED TIMELINE RULER
// ============================================================================

/// Draw the shared timeline-ruler strip: a dark background with a running
/// per-bar number (1, 2, 3, …). Used by both the arrangement ruler and the
/// piano-roll ruler so the two "top bars" look identical. Callers overlay
/// their own grid lines, loop/tempo markers, playhead and bottom border.
pub(super) fn draw_ruler_labels(
    painter: &egui::Painter,
    t: &crate::gui::theme::Theme,
    ruler_rect: Rect,
    total_bars: u32,
    ticks_per_bar: u64,
    tick_to_x: impl Fn(u64) -> f32,
) {
    painter.rect_filled(ruler_rect, 0.0, t.colors.bg_dark);
    for bar_idx in 0..total_bars {
        let x = tick_to_x(u64::from(bar_idx) * ticks_per_bar);
        painter.text(
            Pos2::new(x + 4.0, ruler_rect.min.y + 4.0),
            egui::Align2::LEFT_TOP,
            format!("{}", bar_idx + 1),
            egui::FontId::proportional(12.0),
            t.colors.text_secondary,
        );
    }
}

// ============================================================================
// TRANSPORT BAR
// ============================================================================

/// Draw the transport control bar.
///
/// Shows play/stop/pause buttons, position display (Bar:Beat:Tick), and tempo.
/// Returns true if playback is active (for repaint scheduling).
pub(super) fn draw_transport_bar(
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
    let rec_state = handle.state.transport.recording_state();
    let metro_on = handle.state.transport.is_metronome_on();

    // Read time signature from the song (non-blocking).
    let time_sig = song
        .try_read()
        .map(|s| s.time_signature_at(current_tick))
        .unwrap_or(TimeSignature::COMMON);

    // Phrase boundaries are the sorted, de-duplicated start and end ticks of
    // every placement (plus the song start) — the musical anchors the ◀◀/▶▶
    // buttons jump between, so navigation follows the music even when the tune
    // is not aligned to the 4/4 bar grid. They are only needed when those
    // buttons are actually clicked, so build them lazily here rather than
    // allocating + sorting on every frame.
    let phrase_boundaries = || -> Vec<u64> {
        song.try_read()
            .map(|s| {
                let mut boundaries: Vec<u64> = vec![0];
                for p in s.arrangement().iter() {
                    boundaries.push(p.start.0);
                    if let Some(pat) = s.pattern(p.pattern_id) {
                        boundaries.push(p.end(pat.length).0);
                    }
                }
                boundaries.sort_unstable();
                boundaries.dedup();
                boundaries
            })
            .unwrap_or_else(|| vec![0])
    };

    let (bar, beat, tick) = current_tick.to_bar_beat_tick(time_sig);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // Go to start
        if ui
            .button(RichText::new(ri::SKIP_BACK_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Go to start")
            .clicked()
        {
            handle.send(EngineCommand::Seek { tick: Tick::ZERO });
        }

        // Previous phrase — jump to the previous placement boundary (or, with
        // Shift, the previous 4/4 bar line). Boundary stepping follows the
        // music; bar stepping is the rigid grid for aligned songs.
        let ticks_per_bar = u64::from(time_sig.ticks_per_bar());
        let shift = ui.input(|i| i.modifiers.shift);
        if ui
            .button(RichText::new(ri::REWIND_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Previous phrase (Shift: previous bar)")
            .clicked()
        {
            let prev = if shift && ticks_per_bar > 0 {
                // On a bar line → a full bar back; otherwise snap to this bar.
                if current_ticks.is_multiple_of(ticks_per_bar) {
                    current_ticks.saturating_sub(ticks_per_bar)
                } else {
                    (current_ticks / ticks_per_bar) * ticks_per_bar
                }
            } else {
                // Last boundary strictly before the playhead (sorted ascending).
                phrase_boundaries()
                    .iter()
                    .copied()
                    .rfind(|&b| b < current_ticks)
                    .unwrap_or(0)
            };
            handle.send(EngineCommand::Seek { tick: Tick(prev) });
            view_state.reveal_playhead();
        }

        // Play / Pause toggle. Play starts from the cursor (or resumes in
        // place after a pause); Pause freezes the playhead where it is.
        if is_playing {
            if ui
                .button(RichText::new(ri::PAUSE_FILL).color(t.colors.accent_yellow))
                .on_hover_text("Pause — hold position (Play resumes here)")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else if ui
            .button(RichText::new(ri::PLAY_FILL).color(t.colors.accent_green))
            .on_hover_text("Play — from the cursor")
            .clicked()
        {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }

        // Next phrase — jump to the next placement boundary (or, with Shift,
        // the next 4/4 bar line).
        if ui
            .button(RichText::new(ri::SPEED_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Next phrase (Shift: next bar)")
            .clicked()
        {
            let next = if shift && ticks_per_bar > 0 {
                (current_ticks / ticks_per_bar + 1) * ticks_per_bar
            } else {
                // First boundary strictly after the playhead; stay put if none.
                phrase_boundaries()
                    .iter()
                    .copied()
                    .find(|&b| b > current_ticks)
                    .unwrap_or(current_ticks)
            };
            handle.send(EngineCommand::Seek { tick: Tick(next) });
            view_state.reveal_playhead();
        }

        // Stop returns the playhead to the cursor; a second press once it is
        // already at the cursor rewinds to the start. Disabled only when
        // stopped at the very beginning (nothing to return to or rewind).
        let stop_enabled = is_playing || current_ticks > 0;
        if ui
            .add_enabled(
                stop_enabled,
                egui::Button::new(RichText::new(ri::STOP_FILL).color(if is_playing {
                    t.colors.accent_red
                } else {
                    t.colors.text_primary
                })),
            )
            .on_hover_text("Stop — return to cursor (again: to start)")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
            view_state.reveal_playhead();
        }

        // Record button
        let has_pattern = view_state.opened_pattern.is_some();
        let dim_red = DIM_RED;
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
                RecordingState::Idle => {
                    if has_pattern {
                        "Arm recording"
                    } else {
                        "Open a pattern in the piano roll to arm recording"
                    }
                }
                _ => "Disarm recording",
            })
            .clicked()
        {
            if rec_state != RecordingState::Idle {
                handle.send(EngineCommand::DisarmRecord);
            } else if let Some(pattern_id) = view_state.opened_pattern {
                arm_recording_for_pattern(handle, song, view_state, pattern_id);
            }
        }
        // Request repaint during blinking states
        if matches!(rec_state, RecordingState::Armed | RecordingState::CountIn) {
            ui.request_repaint();
        }

        // Metronome toggle
        if toggle_button(ui, "M", metro_on)
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
        if toggle_button(ui, q_label, view_state.record_quantize > 0)
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
        if toggle_button(ui, "OVR", view_state.overdub)
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
        let mut tempo_val = tempo_f32;
        let tempo_response = ui.add(
            egui::DragValue::new(&mut tempo_val)
                .range(20.0..=300.0)
                .speed(0.5)
                .fixed_decimals(1)
                .suffix(" BPM"),
        );
        if tempo_response.changed() {
            handle.send(EngineCommand::SetTempo(Bpm::new(tempo_val)));
        }

        // Tap tempo
        if ui
            .button(RichText::new("TAP").size(10.0))
            .on_hover_text("Tap to set tempo (average of last 4 clicks)")
            .clicked()
        {
            let now = ui.input(|i| i.time);
            // Clicks older than 2 s are treated as the start of a new tap series.
            view_state.tap_tempo_times.retain(|t| now - *t < 2.0);
            view_state.tap_tempo_times.push(now);
            if view_state.tap_tempo_times.len() > 4 {
                let drop = view_state.tap_tempo_times.len() - 4;
                view_state.tap_tempo_times.drain(0..drop);
            }
            if view_state.tap_tempo_times.len() >= 2 {
                let first = view_state.tap_tempo_times[0];
                let last = *view_state.tap_tempo_times.last().unwrap_or(&first);
                let intervals = view_state.tap_tempo_times.len() as f64 - 1.0;
                let avg_interval = (last - first) / intervals;
                if avg_interval > 0.0 {
                    let bpm = (60.0 / avg_interval) as f32;
                    let clamped = bpm.clamp(20.0, 300.0);
                    handle.send(EngineCommand::SetTempo(Bpm::new(clamped)));
                }
            }
        }

        ui.separator();

        // Time signature — click to edit
        let ts_btn = ui
            .add(
                egui::Button::new(
                    RichText::new(format!("{}/{}", time_sig.numerator, time_sig.denominator))
                        .color(t.colors.text_secondary),
                )
                .frame(false),
            )
            .on_hover_text("Click to change time signature");
        egui::Popup::from_toggle_button_response(&ts_btn).show(|ui| {
            ui.set_min_width(180.0);
            ui.label(RichText::new("Time signature").strong());
            ui.add_space(t.spacing.xs);
            let mut num = time_sig.numerator as i32;
            let mut den = time_sig.denominator as i32;
            let mut changed = false;
            ui.horizontal(|ui| {
                if ui
                    .add(egui::DragValue::new(&mut num).range(1..=32).speed(0.1))
                    .changed()
                {
                    changed = true;
                }
                ui.label("/");
                egui::ComboBox::from_id_salt("ts_den")
                    .selected_text(format!("{den}"))
                    .width(56.0)
                    .show_ui(ui, |ui| {
                        for &allowed in &[1_i32, 2, 4, 8, 16, 32] {
                            if ui
                                .selectable_label(den == allowed, format!("{allowed}"))
                                .clicked()
                            {
                                den = allowed;
                                changed = true;
                            }
                        }
                    });
            });
            if changed
                && let Ok(num_u8) = u8::try_from(num.clamp(1, 32))
                && let Ok(den_u8) = u8::try_from(den.clamp(1, 32))
            {
                let new_sig = TimeSignature::new(num_u8, den_u8);
                song.write().default_time_signature = new_sig;
            }
        });

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

        // Without a status badge, a stale loop region quietly clips
        // playback — only the right-click menu reveals it exists.
        let (loop_enabled, loop_start, loop_end) = handle.state.transport.loop_state();
        if loop_enabled && loop_end.0 > loop_start.0 {
            let (s_bar, _, _) = loop_start.to_bar_beat_tick(time_sig);
            let (e_bar, _, _) = loop_end.to_bar_beat_tick(time_sig);
            let badge = format!("LOOP {}–{}", s_bar + 1, e_bar + 1);
            let resp = ui
                .add(
                    egui::Button::new(RichText::new(badge).color(LOOP_COLOR).strong()).frame(false),
                )
                .on_hover_text("Transport loop active — click to clear.");
            if resp.clicked() {
                handle.send(EngineCommand::SetLoop {
                    start: Tick::ZERO,
                    end: Tick::ZERO,
                    enabled: false,
                });
                view_state.loop_start_tick = None;
                view_state.loop_end_tick = None;
            }
            ui.separator();
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
                ui.label(RichText::new("ARM").color(ARM_RED));
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
