//! Rack view - Instrument rack and patch editor.
//!
//! This is the main synthesizer view showing:
//! - Left panel: Instrument rack (list of instruments)
//! - Center: Patch editor (visual modular patching)
//! - Right panel: Meters and master effects

use eframe::egui;

use crate::engine::EngineHandle;
use crate::engine::instrument::InstrumentId;
use crate::gui::instrument_rack::{InstrumentUiState, show_instrument_rack};

/// Context for the rack view.
pub struct RackContext<'a> {
    /// List of instruments.
    pub instruments: &'a mut Vec<InstrumentUiState>,
    /// Currently active instrument ID.
    pub active_instrument_id: &'a mut InstrumentId,
    /// Engine handle for sending commands.
    pub handle: &'a mut EngineHandle,
    /// Next instrument ID counter.
    pub next_instrument_id: &'a mut u64,
}

/// Draw the instrument rack side panel.
pub fn draw_instrument_rack(ctx: &egui::Context, rack_ctx: &mut RackContext<'_>) {
    egui::SidePanel::left("instrument_rack_panel")
        .min_width(320.0)
        .max_width(400.0)
        .show(ctx, |ui| {
            show_instrument_rack(
                ui,
                rack_ctx.instruments,
                rack_ctx.active_instrument_id,
                rack_ctx.handle,
                rack_ctx.next_instrument_id,
            );
        });
}

/// Draw placeholder content for when no instrument is selected.
pub fn draw_empty_state(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("Select or create an instrument to begin patching");
    });
}
