//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls and a GUI input source
//! for sending `InputCommand`s to the sequencer engine.

use std::sync::{Arc, RwLock};

use eframe::egui::{self, RichText};
use synth_core::Bpm;
use synth_engine::{EngineCommand, EngineHandle};
use synth_sequencer::{InputCommand, InputSource, Song, Tick, TimeSignature};

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

        // Transport buttons
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

        // Position display: Bar:Beat:Tick (1-based for user display)
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

        // Tempo display with editable DragValue
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

        // Time signature display
        ui.label(
            RichText::new(format!("{}/{}", time_sig.numerator, time_sig.denominator))
                .color(t.colors.text_secondary),
        );

        ui.separator();

        // Playing indicator
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
// SEQUENCER VIEW
// ============================================================================

/// Draw the full sequencer view (transport + song info).
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

    // Main content area
    egui::CentralPanel::default().show(ctx, |ui| {
        let t = theme();

        // Song overview
        if let Ok(song) = song.try_read() {
            ui.add_space(12.0);

            // Song info header
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Song: {}", song.name))
                        .size(18.0)
                        .color(t.colors.accent_primary),
                );
            });

            ui.add_space(8.0);

            // Tracks and patterns summary
            let track_count = song.track_count();
            let pattern_count = song.pattern_count();
            let arrangement_count = song.arrangement().len();

            if track_count == 0 && pattern_count == 0 {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Empty song")
                            .size(16.0)
                            .color(t.colors.text_dim),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Use MCP to add tracks and patterns")
                            .color(t.colors.text_dim),
                    );
                });
            } else {
                // Stats row
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Tracks: {track_count}"))
                            .color(t.colors.text_secondary),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Patterns: {pattern_count}"))
                            .color(t.colors.text_secondary),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Placements: {arrangement_count}"))
                            .color(t.colors.text_secondary),
                    );
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // Track list
                for track in song.tracks() {
                    ui.horizontal(|ui| {
                        let color = track_color_to_egui(track.color);
                        ui.colored_label(color, &track.name);

                        if track.mute {
                            ui.label(RichText::new("[M]").color(t.colors.text_dim));
                        }
                        if track.solo {
                            ui.label(RichText::new("[S]").color(t.colors.accent_yellow));
                        }

                        if let Some(inst_id) = track.instrument {
                            ui.label(
                                RichText::new(format!("Inst: {}", inst_id.0))
                                    .color(t.colors.text_dim),
                            );
                        }
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // Pattern list
                ui.label(RichText::new("Patterns").color(t.colors.text_secondary));
                for pattern in song.patterns() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("[{:?}]", pattern.id)).color(t.colors.text_dim),
                        );
                        ui.label(RichText::new(&pattern.name).color(t.colors.text_primary));
                        ui.label(
                            RichText::new(format!(
                                "{} notes, len: {}",
                                pattern.notes().len(),
                                pattern.length.0
                            ))
                            .color(t.colors.text_dim),
                        );
                    });
                }
            }
        } else {
            ui.label(RichText::new("Song locked...").color(t.colors.text_dim));
        }
    });
}

/// Convert a sequencer track color to an egui Color32.
fn track_color_to_egui(color: synth_sequencer::TrackColor) -> egui::Color32 {
    egui::Color32::from_rgb(color.r, color.g, color.b)
}
