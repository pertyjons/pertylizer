//! Sequencer GUI module.
//!
//! Provides the sequencer view and a GUI input source for sending
//! `InputCommand`s to the sequencer engine.

use std::sync::{Arc, RwLock};

use eframe::egui;
use synth_sequencer::{InputCommand, InputSource, Song};

use crate::gui::theme::theme;

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

/// Draw the sequencer stub view.
pub fn draw_sequencer_view(ctx: &egui::Context, song: &Arc<RwLock<Song>>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let t = theme();

        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(
                egui::RichText::new("Sequencer View")
                    .size(24.0)
                    .color(t.colors.accent_primary),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("(under construction)")
                    .size(14.0)
                    .color(t.colors.text_dim),
            );
            ui.add_space(20.0);

            // Show basic song info if available
            if let Ok(song) = song.try_read() {
                ui.group(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Song: {}", song.name))
                            .color(t.colors.text_primary),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "Tempo: {:.0} BPM",
                            song.default_tempo.as_f32()
                        ))
                        .color(t.colors.text_secondary),
                    );
                    ui.label(
                        egui::RichText::new(format!("Patterns: {}", song.pattern_count()))
                            .color(t.colors.text_secondary),
                    );
                    ui.label(
                        egui::RichText::new(format!("Tracks: {}", song.track_count()))
                            .color(t.colors.text_secondary),
                    );
                });
            }
        });
    });
}
