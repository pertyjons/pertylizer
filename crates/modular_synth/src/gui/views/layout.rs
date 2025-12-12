//! Top bar and navigation layout components.
//!
//! Provides the main navigation UI including:
//! - Menu bar (File, Help)
//! - Transport controls (Play, Stop, BPM)
//! - View selector tabs (Rack, Sequencer, Mixer)
//! - Status indicators (CPU, voices, latency)

use eframe::egui::{self, Align, Layout, RichText, TopBottomPanel};

use crate::gui::app::state::{AppView, TopPanel};
use crate::gui::theme::theme;
use crate::io::MidiHandler;

/// Context for drawing the top bar, containing all necessary mutable references.
pub struct TopBarContext<'a> {
    /// Current active view.
    pub active_view: &'a mut AppView,
    /// Current top panel drawer state.
    pub active_top_panel: &'a mut TopPanel,
    /// Current patch name.
    pub patch_name: &'a str,
    /// CPU usage (0.0 - 1.0).
    pub cpu_usage: f32,
    /// Active voice count.
    pub voice_count: usize,
    /// Audio latency.
    pub latency: std::time::Duration,
    /// MIDI handler for port selection.
    pub midi_handler: &'a mut MidiHandler,
    /// Callback to show save patch dialog.
    pub show_save_patch: &'a mut bool,
    /// Callback to show load patch dialog.
    pub show_load_patch: &'a mut bool,
    /// Callback to show settings dialog.
    pub show_settings: &'a mut bool,
    /// Callback to show about dialog.
    pub show_about: &'a mut bool,
    /// Callback to show import song dialog.
    pub show_import_song: &'a mut bool,
    /// Save patch name buffer.
    pub patch_save_name: &'a mut String,
    /// Callback for new patch action.
    pub on_new_patch: Option<&'a mut dyn FnMut()>,
    /// Status message setter.
    pub set_status: Option<&'a mut dyn FnMut(String)>,
}

/// Draw the main top bar with menus, transport, and view selector.
#[allow(deprecated)] // egui::menu::bar is deprecated but MenuBar API differs slightly
pub fn draw_top_bar(ctx: &egui::Context, bar_ctx: &mut TopBarContext<'_>) {
    TopBottomPanel::top("top_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            // --- FILE MENU ---
            ui.menu_button("File", |ui| {
                if ui.button("New Patch").clicked() {
                    if let Some(callback) = &mut bar_ctx.on_new_patch {
                        callback();
                    }
                    if let Some(status) = &mut bar_ctx.set_status {
                        status("New patch created".to_string());
                    }
                    ui.close();
                }
                if ui.button("Load Patch...").clicked() {
                    *bar_ctx.show_load_patch = true;
                    ui.close();
                }
                if ui.button("Save Patch...").clicked() {
                    *bar_ctx.patch_save_name = bar_ctx.patch_name.to_string();
                    *bar_ctx.show_save_patch = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Import Song...").clicked() {
                    *bar_ctx.show_import_song = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Settings...").clicked() {
                    *bar_ctx.show_settings = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("About").clicked() {
                    *bar_ctx.show_about = true;
                    ui.close();
                }
            });

            ui.separator();

            // --- TOP PANEL TOGGLES ---
            let midi_label = if *bar_ctx.active_top_panel == TopPanel::Midi {
                RichText::new("MIDI").color(theme().colors.accent_primary)
            } else {
                RichText::new("MIDI").color(theme().colors.text_secondary)
            };
            if ui
                .selectable_label(*bar_ctx.active_top_panel == TopPanel::Midi, midi_label)
                .clicked()
            {
                *bar_ctx.active_top_panel = if *bar_ctx.active_top_panel == TopPanel::Midi {
                    TopPanel::None
                } else {
                    TopPanel::Midi
                };
            }

            let engine_label = if *bar_ctx.active_top_panel == TopPanel::Engine {
                RichText::new("Engine").color(theme().colors.accent_primary)
            } else {
                RichText::new("Engine").color(theme().colors.text_secondary)
            };
            if ui
                .selectable_label(*bar_ctx.active_top_panel == TopPanel::Engine, engine_label)
                .clicked()
            {
                *bar_ctx.active_top_panel = if *bar_ctx.active_top_panel == TopPanel::Engine {
                    TopPanel::None
                } else {
                    TopPanel::Engine
                };
            }

            // --- VIEW SELECTOR (right-aligned) ---
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Status indicators (rightmost)
                let cpu_color = if bar_ctx.cpu_usage > 0.8 {
                    theme().colors.meter_red
                } else if bar_ctx.cpu_usage > 0.5 {
                    theme().colors.meter_yellow
                } else {
                    theme().colors.meter_green
                };
                ui.label(
                    RichText::new(format!("CPU: {:>3.0}%", bar_ctx.cpu_usage * 100.0))
                        .color(cpu_color),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("Voices: {}", bar_ctx.voice_count))
                        .color(theme().colors.text_secondary),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "Latency: {:.1}ms",
                        bar_ctx.latency.as_secs_f64() * 1000.0
                    ))
                    .color(theme().colors.text_dim),
                );
                ui.separator();

                // MIDI port indicator
                draw_midi_selector(ui, bar_ctx.midi_handler);
                ui.separator();

                // Current patch name
                ui.label(
                    RichText::new(format!("Patch: {}", bar_ctx.patch_name))
                        .color(theme().colors.accent_cyan),
                );
                ui.separator();

                // View selector tabs
                for view in [AppView::Mixer, AppView::Sequencer, AppView::Rack] {
                    let is_selected = *bar_ctx.active_view == view;
                    let label = format!("{} {}", view.icon(), view.label());
                    let text = if is_selected {
                        RichText::new(label).color(theme().colors.accent_primary)
                    } else {
                        RichText::new(label).color(theme().colors.text_secondary)
                    };
                    if ui.selectable_label(is_selected, text).clicked() {
                        *bar_ctx.active_view = view;
                    }
                }
            });
        });
    });
}

/// Draw the MIDI port selector dropdown.
fn draw_midi_selector(ui: &mut egui::Ui, midi_handler: &mut MidiHandler) {
    let midi_label = if midi_handler.is_connected() {
        let port_name = midi_handler.port_name().unwrap_or("Unknown");
        let short_name = if port_name.len() > 20 {
            format!("{}...", &port_name[..17])
        } else {
            port_name.to_string()
        };
        RichText::new(format!("{} ", short_name)).color(theme().colors.meter_green)
    } else {
        RichText::new("MIDI ").color(theme().colors.text_dim)
    };

    ui.menu_button(midi_label, |ui| {
        ui.set_min_width(250.0);
        let ports = MidiHandler::list_ports();
        if ports.is_empty() {
            ui.label(RichText::new("No MIDI ports available").color(theme().colors.text_dim));
        } else {
            for port in &ports {
                let is_current = midi_handler.port_name() == Some(port.as_str());
                let label = if is_current {
                    RichText::new(port.clone()).color(theme().colors.meter_green)
                } else {
                    RichText::new(format!("  {}", port))
                };
                if ui.button(label).clicked() {
                    if let Err(e) = midi_handler.connect_to(port) {
                        eprintln!("MIDI connection error: {}", e);
                    }
                    ui.close();
                }
            }
        }
        ui.separator();
        if ui.button("Refresh").clicked() {
            ui.close();
        }
    });
}

/// Draw the expandable top drawer based on current panel state.
pub fn draw_top_drawer(ctx: &egui::Context, panel: TopPanel, midi_handler: &mut MidiHandler) {
    if panel == TopPanel::None {
        return;
    }

    TopBottomPanel::top("top_drawer")
        .resizable(true)
        .min_height(60.0)
        .max_height(200.0)
        .show(ctx, |ui| match panel {
            TopPanel::Midi => draw_midi_panel(ui, midi_handler),
            TopPanel::Engine => draw_engine_panel(ui),
            TopPanel::None => {}
        });
}

/// Draw MIDI settings panel content.
fn draw_midi_panel(ui: &mut egui::Ui, midi_handler: &mut MidiHandler) {
    ui.heading("MIDI Settings");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Input Port:");
        let ports = MidiHandler::list_ports();
        let current = midi_handler.port_name().unwrap_or("None");

        egui::ComboBox::from_id_salt("midi_port_combo")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for port in &ports {
                    if ui
                        .selectable_label(midi_handler.port_name() == Some(port.as_str()), port)
                        .clicked()
                    {
                        let _ = midi_handler.connect_to(port);
                    }
                }
            });

        if ui.button("Refresh").clicked() {
            // Ports will be refreshed on next combo box open
        }
    });

    if midi_handler.is_connected() {
        ui.label(
            RichText::new(format!(
                "Connected to: {}",
                midi_handler.port_name().unwrap_or("Unknown")
            ))
            .color(theme().colors.meter_green),
        );
    } else {
        ui.label(RichText::new("Not connected").color(theme().colors.text_dim));
    }
}

/// Draw engine/audio settings panel content.
fn draw_engine_panel(ui: &mut egui::Ui) {
    ui.heading("Engine Settings");
    ui.separator();

    ui.label("Audio buffer size, sample rate, and other engine settings will go here.");
    ui.label(RichText::new("(Coming soon)").color(theme().colors.text_dim));
}
