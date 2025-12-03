//! Part Manager UI for managing multiple instrument parts.
//!
//! Provides a panel for visualizing and controlling synth parts (instruments),
//! including volume, pan, MIDI channel, and mute controls.

use eframe::egui::{self, RichText, Ui};
use crate::engine::{
    EngineHandle,
    EngineCommand,
    PartParam,
    part::{PartId, MidiChannel, SynthPart},
};
use crate::types::{Gain, BipolarValue};
use super::widgets::{Knob, colors};
use super::theme::theme;

/// GUI state for a single synth part.
///
/// This mirrors the engine's SynthPart state for display purposes.
/// Updates are sent to the engine via EngineCommands when values change.
#[derive(Debug, Clone)]
pub struct PartUiState {
    /// Unique identifier matching the engine's PartId.
    pub id: PartId,
    /// Display name for this part.
    pub name: String,
    /// MIDI channel this part responds to.
    pub channel: MidiChannel,
    /// Output volume (0.0 = mute, 1.0 = unity).
    pub volume: Gain,
    /// Stereo pan position (-1.0 = left, 0.0 = center, +1.0 = right).
    pub pan: BipolarValue,
    /// Whether this part is muted (uses volume = 0 for soft mute).
    pub muted: bool,
    /// Stored volume when muted (to restore on unmute).
    stored_volume: Gain,
}

impl Default for PartUiState {
    fn default() -> Self {
        Self {
            id: PartId::FIRST,
            name: "Part 1".to_string(),
            channel: MidiChannel::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            stored_volume: Gain::UNITY,
        }
    }
}

impl PartUiState {
    /// Create a new part with the given ID and name.
    pub fn new(id: PartId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            channel: MidiChannel::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            stored_volume: Gain::UNITY,
        }
    }

    /// Create a new part with a specific MIDI channel.
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

/// Result of part manager interactions.
#[derive(Debug, Default)]
pub struct PartManagerResult {
    /// The newly selected active part, if changed.
    pub active_part_changed: Option<PartId>,
}

/// Show the part manager panel.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `parts` - Mutable list of part UI states
/// * `active_part_id` - Currently selected part for keyboard input
/// * `handle` - Engine handle for sending commands
/// * `next_part_id` - Counter for generating new part IDs
pub fn show_part_manager(
    ui: &mut Ui,
    parts: &mut Vec<PartUiState>,
    active_part_id: &mut PartId,
    handle: &mut EngineHandle,
    next_part_id: &mut u64,
) -> PartManagerResult {
    let mut result = PartManagerResult::default();
    let t = theme();

    ui.vertical(|ui| {
        // Header
        ui.horizontal(|ui| {
            ui.label(RichText::new("INSTRUMENTS").color(colors::TEXT_SECONDARY).size(t.fonts.size_normal));
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Capture len before iterating to avoid borrow issues
        let can_remove = parts.len() > 1;
        let mut part_to_remove: Option<usize> = None;

        // Scrollable list of parts
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                for idx in 0..parts.len() {
                    let part_id = parts[idx].id;
                    let is_active = part_id == *active_part_id;

                    // Part row frame
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
                                    *active_part_id = part_id;
                                    result.active_part_changed = Some(part_id);
                                }

                                // Part name (editable)
                                ui.add(
                                    egui::TextEdit::singleline(&mut parts[idx].name)
                                        .desired_width(80.0)
                                        .font(egui::FontId::proportional(t.fonts.size_small))
                                );

                                ui.add_space(4.0);

                                // MIDI Channel dropdown
                                let channel = parts[idx].channel;
                                let channel_label = if channel.is_omni() {
                                    "Omni".to_string()
                                } else {
                                    format!("Ch{}", channel.as_one_indexed())
                                };

                                egui::ComboBox::from_id_salt(format!("ch_{}", part_id.as_u64()))
                                    .selected_text(RichText::new(&channel_label).size(t.fonts.size_small))
                                    .width(50.0)
                                    .show_ui(ui, |ui| {
                                        // Omni option
                                        if ui.selectable_label(channel.is_omni(), "Omni").clicked() {
                                            parts[idx].channel = MidiChannel::OMNI;
                                            handle.send(EngineCommand::SetPartMidiChannel {
                                                part_id,
                                                channel: MidiChannel::OMNI,
                                            });
                                        }
                                        // Channels 1-16
                                        for ch in 1..=16u8 {
                                            let midi_ch = MidiChannel::from_one_indexed(ch).unwrap();
                                            let is_selected = !channel.is_omni()
                                                && channel.as_one_indexed() == ch;
                                            if ui.selectable_label(is_selected, format!("Ch {}", ch)).clicked() {
                                                parts[idx].channel = midi_ch;
                                                handle.send(EngineCommand::SetPartMidiChannel {
                                                    part_id,
                                                    channel: midi_ch,
                                                });
                                            }
                                        }
                                    });

                                ui.add_space(4.0);

                                // Volume knob (small)
                                let muted = parts[idx].muted;
                                let mut vol = parts[idx].volume.as_f32();
                                let vol_response = Knob::new(&mut vol, 0.0, 1.0)
                                    .default(1.0)
                                    .label("Vol")
                                    .size(t.sizes.knob_size_small)
                                    .accent_color(if muted { colors::TEXT_DIM } else { colors::ACCENT_GREEN })
                                    .show(ui);

                                if vol_response.changed() && !muted {
                                    parts[idx].volume = Gain::new(vol);
                                    parts[idx].stored_volume = parts[idx].volume;
                                    handle.send(EngineCommand::SetPartParameter {
                                        part_id,
                                        param: PartParam::Volume(parts[idx].volume),
                                    });
                                }

                                // Pan knob (small)
                                let current_pan = parts[idx].pan.as_f32();
                                let mut pan = current_pan;
                                Knob::new(&mut pan, -1.0, 1.0)
                                    .default(0.0)
                                    .label("Pan")
                                    .size(t.sizes.knob_size_small)
                                    .accent_color(colors::ACCENT_CYAN)
                                    .show(ui);

                                if (pan - current_pan).abs() > f32::EPSILON {
                                    parts[idx].pan = BipolarValue::new(pan);
                                    handle.send(EngineCommand::SetPartParameter {
                                        part_id,
                                        param: PartParam::Pan(parts[idx].pan),
                                    });
                                }

                                // Mute button
                                let mute_text = "M";
                                let mute_color = if muted { colors::ACCENT_RED } else { colors::TEXT_DIM };
                                if ui.add(
                                    egui::Button::new(RichText::new(mute_text).color(mute_color).size(t.fonts.size_small))
                                        .min_size(egui::vec2(24.0, 24.0))
                                ).clicked() {
                                    let new_volume = parts[idx].toggle_mute();
                                    handle.send(EngineCommand::SetPartParameter {
                                        part_id,
                                        param: PartParam::Volume(new_volume),
                                    });
                                }

                                // Remove button (only if more than one part)
                                if can_remove {
                                    if ui.add(
                                        egui::Button::new(RichText::new("×").color(colors::TEXT_DIM).size(t.fonts.size_small))
                                            .min_size(egui::vec2(20.0, 24.0))
                                    ).on_hover_text("Remove instrument").clicked() {
                                        part_to_remove = Some(idx);
                                    }
                                }
                            });
                        });

                    ui.add_space(2.0);
                }
            });

        // Handle part removal (after iteration to avoid borrow issues)
        if let Some(idx) = part_to_remove {
            let removed_part = parts.remove(idx);
            handle.send(EngineCommand::RemovePart {
                part_id: removed_part.id,
            });

            // If we removed the active part, select the first one
            if removed_part.id == *active_part_id && !parts.is_empty() {
                *active_part_id = parts[0].id;
                result.active_part_changed = Some(*active_part_id);
            }
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Add new part button
        if ui.button(RichText::new("+ Add Instrument").size(t.fonts.size_small)).clicked() {
            let new_id = PartId::new(*next_part_id);
            *next_part_id += 1;

            let part_num = parts.len() + 1;
            let new_name = format!("Part {}", part_num);

            // Assign next available channel
            let new_channel = MidiChannel::from_one_indexed(part_num as u8)
                .unwrap_or(MidiChannel::CH1);

            // Create UI state
            let new_ui_part = PartUiState::new(new_id, &new_name)
                .with_channel(new_channel);

            // Create engine part (Box for real-time safety)
            let mut engine_part = SynthPart::new(new_id, &new_name);
            engine_part.set_midi_channel(new_channel);

            // Send to engine
            handle.send(EngineCommand::AddPart {
                part: Box::new(engine_part),
            });

            parts.push(new_ui_part);
        }
    });

    result
}
