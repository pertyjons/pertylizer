//! Instrument Rack UI for managing multiple instruments.
//!
//! Provides a panel for visualizing and controlling instruments (instruments),
//! including volume, pan, MIDI channel, and mute controls.

use super::patch_editor::PatchEditor;
use super::theme::theme;
use super::widgets::{Knob, colors};
use crate::engine::{
    EngineCommand, EngineHandle, InstrumentParam,
    instrument::{Instrument, InstrumentId, MidiChannel},
};
use crate::types::{BipolarValue, Gain};
use eframe::egui::{self, RichText, Ui};

/// GUI state for a single instrument.
///
/// This mirrors the engine's Instrument state for display purposes.
/// Updates are sent to the engine via EngineCommands when values change.
/// Each instrument owns its own PatchEditor for independent visual graphs.
#[derive(Clone)]
pub struct InstrumentUiState {
    /// Unique identifier matching the engine's InstrumentId.
    pub id: InstrumentId,
    /// Display name for this instrument.
    pub name: String,
    /// MIDI channel this instrument responds to.
    pub channel: MidiChannel,
    /// Output volume (0.0 = mute, 1.0 = unity).
    pub volume: Gain,
    /// Stereo pan position (-1.0 = left, 0.0 = center, +1.0 = right).
    pub pan: BipolarValue,
    /// Whether this instrument is muted (uses volume = 0 for soft mute).
    pub muted: bool,
    /// Stored volume when muted (to restore on unmute).
    stored_volume: Gain,
    /// The patch editor for this instrument's visual module graph.
    pub patch_editor: PatchEditor,
}

impl Default for InstrumentUiState {
    fn default() -> Self {
        Self {
            id: InstrumentId::FIRST,
            name: "Instrument 1".to_string(),
            channel: MidiChannel::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            stored_volume: Gain::UNITY,
            patch_editor: PatchEditor::new(),
        }
    }
}

impl InstrumentUiState {
    /// Create a new instrument with the given ID and name.
    pub fn new(id: InstrumentId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            channel: MidiChannel::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            stored_volume: Gain::UNITY,
            patch_editor: PatchEditor::new(),
        }
    }

    /// Create a new instrument with a specific MIDI channel.
    pub fn with_channel(mut self, channel: MidiChannel) -> Self {
        self.channel = channel;
        self
    }

    /// Toggle mute state (soft mute via volume).
    pub fn toggle_mute(&mut self) -> Gain {
        if self.muted {
            // Unmute: restore previous volume
            self.muted = false;
            self.volume = self.stored_volume;
        } else {
            // Mute: store current volume and set to 0
            self.muted = true;
            self.stored_volume = self.volume;
            self.volume = Gain::MUTE;
        }
        self.volume
    }
}

/// Result of instrument rack interactions.
#[derive(Debug, Default)]
pub struct InstrumentRackResult {
    /// The newly selected active instrument, if changed.
    pub active_instrument_changed: Option<InstrumentId>,
}

/// Show the instrument rack panel.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `instruments` - Mutable list of instrument UI states
/// * `active_instrument_id` - Currently selected instrument for keyboard input
/// * `handle` - Engine handle for sending commands
/// * `next_instrument_id` - Counter for generating new instrument IDs
pub fn show_instrument_rack(
    ui: &mut Ui,
    instruments: &mut Vec<InstrumentUiState>,
    active_instrument_id: &mut InstrumentId,
    handle: &mut EngineHandle,
    next_instrument_id: &mut u64,
) -> InstrumentRackResult {
    let mut result = InstrumentRackResult::default();
    let t = theme();

    ui.vertical(|ui| {
        // Header
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("INSTRUMENTS")
                    .color(colors::TEXT_SECONDARY)
                    .size(t.fonts.size_normal),
            );
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Capture len before iterating to avoid borrow issues
        let can_remove_instrument = instruments.len() > 1;
        let mut instrument_to_remove: Option<usize> = None;

        // Scrollable list of instruments
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                for idx in 0..instruments.len() {
                    let instrument_id = instruments[idx].id;
                    let is_active = instrument_id == *active_instrument_id;

                    // Instrument row frame
                    let frame_color = if is_active {
                        colors::ACCENT_ORANGE.gamma_multiply(0.2)
                    } else {
                        colors::BG_WIDGET
                    };

                    egui::Frame::new()
                        .fill(frame_color)
                        .inner_margin(6.0)
                        .corner_radius(4.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Selection indicator / radio button
                                let response = ui.selectable_label(is_active, "");
                                if response.clicked() {
                                    *active_instrument_id = instrument_id;
                                    result.active_instrument_changed = Some(instrument_id);
                                }

                                // Instrument name (editable)
                                ui.add(
                                    egui::TextEdit::singleline(&mut instruments[idx].name)
                                        .desired_width(80.0)
                                        .font(egui::FontId::proportional(t.fonts.size_small)),
                                );

                                ui.add_space(4.0);

                                // MIDI Channel dropdown
                                let channel = instruments[idx].channel;
                                let channel_label = if channel.is_omni() {
                                    "Omni".to_string()
                                } else {
                                    format!("Ch {}", channel.as_one_indexed())
                                };

                                egui::ComboBox::from_id_salt(format!(
                                    "ch_{}",
                                    instrument_id.as_u64()
                                ))
                                .selected_text(
                                    RichText::new(&channel_label).size(t.fonts.size_small),
                                )
                                .width(50.0)
                                .show_ui(ui, |ui| {
                                    // Omni option
                                    if ui.selectable_label(channel.is_omni(), "Omni").clicked() {
                                        instruments[idx].channel = MidiChannel::OMNI;
                                        handle.send(EngineCommand::SetInstrumentMidiChannel {
                                            instrument_id,
                                            channel: MidiChannel::OMNI,
                                        });
                                    }
                                    // Channels 1-16
                                    for ch in 1..=16u8 {
                                        let midi_ch = MidiChannel::from_one_indexed(ch).unwrap();
                                        let is_selected =
                                            !channel.is_omni() && channel.as_one_indexed() == ch;
                                        if ui
                                            .selectable_label(is_selected, format!("Ch {}", ch))
                                            .clicked()
                                        {
                                            instruments[idx].channel = midi_ch;
                                            handle.send(EngineCommand::SetInstrumentMidiChannel {
                                                instrument_id,
                                                channel: midi_ch,
                                            });
                                        }
                                    }
                                });

                                ui.add_space(4.0);

                                // Volume knob (small)
                                let muted = instruments[idx].muted;
                                let mut vol = instruments[idx].volume.as_f32();
                                let vol_response = Knob::new(&mut vol, 0.0, 1.0)
                                    .default(1.0)
                                    .label("Vol")
                                    .size(t.sizes.knob_size_small)
                                    .accent_color(if muted {
                                        colors::TEXT_DIM
                                    } else {
                                        colors::ACCENT_GREEN
                                    })
                                    .show(ui);

                                if vol_response.changed() && !muted {
                                    instruments[idx].volume = Gain::new(vol);
                                    instruments[idx].stored_volume = instruments[idx].volume;
                                    handle.send(EngineCommand::SetInstrumentParameter {
                                        instrument_id,
                                        param: InstrumentParam::Volume(instruments[idx].volume),
                                    });
                                }

                                // Pan knob (small)
                                let current_pan = instruments[idx].pan.as_f32();
                                let mut pan = current_pan;
                                Knob::new(&mut pan, -1.0, 1.0)
                                    .default(0.0)
                                    .label("Pan")
                                    .size(t.sizes.knob_size_small)
                                    .accent_color(colors::ACCENT_CYAN)
                                    .show(ui);

                                if (pan - current_pan).abs() > f32::EPSILON {
                                    instruments[idx].pan = BipolarValue::new(pan);
                                    handle.send(EngineCommand::SetInstrumentParameter {
                                        instrument_id,
                                        param: InstrumentParam::Pan(instruments[idx].pan),
                                    });
                                }

                                // Mute button
                                let mute_text = "M";
                                let mute_color = if muted {
                                    colors::ACCENT_RED
                                } else {
                                    colors::TEXT_DIM
                                };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(mute_text)
                                                .color(mute_color)
                                                .size(t.fonts.size_small),
                                        )
                                        .min_size(egui::vec2(24.0, 24.0)),
                                    )
                                    .clicked()
                                {
                                    let new_volume = instruments[idx].toggle_mute();
                                    handle.send(EngineCommand::SetInstrumentParameter {
                                        instrument_id,
                                        param: InstrumentParam::Volume(new_volume),
                                    });
                                }

                                // Remove button (only if more than one instrument)
                                if can_remove_instrument
                                    && ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("×")
                                                    .color(colors::TEXT_DIM)
                                                    .size(t.fonts.size_small),
                                            )
                                            .min_size(egui::vec2(20.0, 24.0)),
                                        )
                                        .on_hover_text("Remove instrument")
                                        .clicked()
                                {
                                    instrument_to_remove = Some(idx);
                                }
                            });
                        });

                    ui.add_space(2.0);
                }
            });

        // Handle instrument removal (after iteration to avoid borrow issues)
        if let Some(idx) = instrument_to_remove {
            let removed_instrument = instruments.remove(idx);
            handle.send(EngineCommand::RemoveInstrument {
                instrument_id: removed_instrument.id,
            });

            // If we removed the active instrument, select the first one
            if removed_instrument.id == *active_instrument_id && !instruments.is_empty() {
                *active_instrument_id = instruments[0].id;
                result.active_instrument_changed = Some(*active_instrument_id);
            }
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Add new instrument button
        if ui
            .button(RichText::new("+ Add Instrument").size(t.fonts.size_small))
            .clicked()
        {
            let new_id = InstrumentId::new(*next_instrument_id);
            *next_instrument_id += 1;

            let instrument_num = instruments.len() + 1;
            let new_name = format!("Instrument {}", instrument_num);

            // Assign next available channel
            let new_channel =
                MidiChannel::from_one_indexed(instrument_num as u8).unwrap_or(MidiChannel::CH1);

            // Create UI state
            let new_ui_instrument =
                InstrumentUiState::new(new_id, &new_name).with_channel(new_channel);

            // Create engine instrument (Box for real-time safety)
            let mut engine_instrument = Instrument::new(new_id, &new_name);
            engine_instrument.set_midi_channel(new_channel);

            // Send to engine
            handle.send(EngineCommand::AddInstrument {
                instrument: Box::new(engine_instrument),
            });

            instruments.push(new_ui_instrument);
        }
    });

    result
}
