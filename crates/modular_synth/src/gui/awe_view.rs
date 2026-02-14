//! AWE (Acoustic World Engine) view — 2D floor plan with parameter controls.
//!
//! Displays:
//! - Room outline as a 2D floor plan (top-down)
//! - Draggable source and listener markers
//! - Material selector
//! - Sliders for dry/wet, early/late, modes amount, tail stretch
//! - Two LFO sections with rate/amount/target controls

use eframe::egui;

use crate::gui::app::state::AppView;
use crate::gui::theme::theme;
use synth_awe::params::{AweLfoState, AweLfoTarget};
use synth_awe::{AweParam, Material, RoomShape};
use synth_engine::{EngineCommand, EngineHandle};

/// UI state for the AWE view, stored in SynthApp.
pub struct AweUiState {
    // Room dimensions
    pub room_length: f32,
    pub room_width: f32,
    pub room_height: f32,

    // Positions (x, y only — z is fixed for floor plan)
    pub source_x: f32,
    pub source_y: f32,
    pub listener_x: f32,
    pub listener_y: f32,

    // Material index (0=Concrete, 1=Wood, 2=Glass, 3=Metal, 4=Fabric, 5=Tile)
    pub material_idx: usize,

    // Mix parameters
    pub dry_wet: f32,
    pub early_late: f32,
    pub modes_amount: f32,
    pub tail_stretch: f32,

    // LFO states
    pub lfo1: AweLfoState,
    pub lfo2: AweLfoState,

    // Drag state
    pub dragging_source: bool,
    pub dragging_listener: bool,
}

impl Default for AweUiState {
    fn default() -> Self {
        let snap = synth_awe::AweSnapshot::default();
        Self {
            room_length: 8.0,
            room_width: 5.0,
            room_height: 3.0,
            source_x: snap.source_pos[0],
            source_y: snap.source_pos[1],
            listener_x: snap.listener_pos[0],
            listener_y: snap.listener_pos[1],
            material_idx: 0,
            dry_wet: snap.dry_wet,
            early_late: snap.early_late_balance,
            modes_amount: snap.modes_amount,
            tail_stretch: snap.tail_stretch,
            lfo1: snap.lfo1,
            lfo2: snap.lfo2,
            dragging_source: false,
            dragging_listener: false,
        }
    }
}

const MATERIAL_NAMES: [&str; 6] = ["Concrete", "Wood", "Glass", "Metal", "Fabric", "Tile"];

fn material_from_index(idx: usize) -> Material {
    match idx {
        0 => Material::CONCRETE,
        1 => Material::WOOD,
        2 => Material::GLASS,
        3 => Material::METAL,
        4 => Material::FABRIC,
        5 => Material::TILE,
        _ => Material::CONCRETE,
    }
}

const LFO_TARGET_NAMES: [&str; 8] = [
    "Room Length",
    "Room Width",
    "Source X",
    "Source Y",
    "Listener X",
    "Listener Y",
    "Dry/Wet",
    "Freq Warp",
];

fn lfo_target_from_index(idx: usize) -> AweLfoTarget {
    match idx {
        0 => AweLfoTarget::RoomLength,
        1 => AweLfoTarget::RoomWidth,
        2 => AweLfoTarget::SourceX,
        3 => AweLfoTarget::SourceY,
        4 => AweLfoTarget::ListenerX,
        5 => AweLfoTarget::ListenerY,
        6 => AweLfoTarget::DryWet,
        7 => AweLfoTarget::FreqWarp,
        _ => AweLfoTarget::SourceX,
    }
}

fn lfo_target_to_index(target: AweLfoTarget) -> usize {
    match target {
        AweLfoTarget::RoomLength => 0,
        AweLfoTarget::RoomWidth => 1,
        AweLfoTarget::SourceX => 2,
        AweLfoTarget::SourceY => 3,
        AweLfoTarget::ListenerX => 4,
        AweLfoTarget::ListenerY => 5,
        AweLfoTarget::DryWet => 6,
        AweLfoTarget::FreqWarp => 7,
    }
}

/// Draw the AWE view.
#[allow(clippy::too_many_lines)]
pub fn draw_awe_view(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    awe_enabled: &mut bool,
    active_view: &mut AppView,
    ui_state: &mut AweUiState,
) {
    // Top toolbar
    egui::TopBottomPanel::top("awe_toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("← Rack").clicked() {
                *active_view = AppView::Rack;
            }

            ui.separator();

            let toggle_label = if *awe_enabled { "AWE: ON" } else { "AWE: OFF" };
            let toggle_color = if *awe_enabled {
                theme().colors.meter_green
            } else {
                theme().colors.text_dim
            };
            if ui
                .button(egui::RichText::new(toggle_label).color(toggle_color))
                .clicked()
            {
                *awe_enabled = !*awe_enabled;
                handle.send(EngineCommand::SetAweEnabled {
                    enabled: *awe_enabled,
                });
            }
        });
    });

    // Right side panel with controls
    egui::SidePanel::right("awe_controls")
        .min_width(220.0)
        .default_width(260.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                draw_controls(ui, handle, ui_state);
            });
        });

    // Central panel — 2D floor plan
    egui::CentralPanel::default().show(ctx, |ui| {
        draw_floor_plan(ui, handle, ui_state);
    });
}

/// Draw the 2D floor plan of the room.
fn draw_floor_plan(ui: &mut egui::Ui, handle: &mut EngineHandle, state: &mut AweUiState) {
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = response.rect;

    let t = theme();
    let bg_color = t.colors.bg_dark;
    let wall_color = t.colors.text_dim;
    let source_color = t.colors.accent_orange;
    let listener_color = t.colors.accent_cyan;

    // Fill background
    painter.rect_filled(rect, 0.0, bg_color);

    // Calculate room-to-screen transform
    // Leave margin for labels
    let margin = 30.0;
    let draw_rect = rect.shrink(margin);

    let room_w = state.room_length.max(0.5);
    let room_h = state.room_width.max(0.5);

    let scale_x = draw_rect.width() / room_w;
    let scale_y = draw_rect.height() / room_h;
    let scale = scale_x.min(scale_y);

    // Center the room in the draw area
    let room_screen_w = room_w * scale;
    let room_screen_h = room_h * scale;
    let room_origin = egui::pos2(
        draw_rect.center().x - room_screen_w * 0.5,
        draw_rect.center().y - room_screen_h * 0.5,
    );

    let room_screen_rect =
        egui::Rect::from_min_size(room_origin, egui::vec2(room_screen_w, room_screen_h));

    // Conversion closures
    let room_to_screen = |rx: f32, ry: f32| -> egui::Pos2 {
        egui::pos2(
            room_origin.x + rx * scale,
            room_origin.y + (room_h - ry) * scale, // flip Y: room Y up, screen Y down
        )
    };

    let screen_to_room = |sx: f32, sy: f32| -> (f32, f32) {
        let rx = (sx - room_origin.x) / scale;
        let ry = room_h - (sy - room_origin.y) / scale;
        (rx, ry)
    };

    // Draw room walls
    painter.rect_stroke(
        room_screen_rect,
        0.0,
        egui::Stroke::new(2.0, wall_color),
        egui::StrokeKind::Outside,
    );

    // Dimension labels
    let dim_label_color = t.colors.text_dim;
    painter.text(
        egui::pos2(room_screen_rect.center().x, room_screen_rect.max.y + 14.0),
        egui::Align2::CENTER_TOP,
        format!("{:.1}m", state.room_length),
        egui::FontId::proportional(12.0),
        dim_label_color,
    );
    painter.text(
        egui::pos2(room_screen_rect.min.x - 14.0, room_screen_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.1}m", state.room_width),
        egui::FontId::proportional(12.0),
        dim_label_color,
    );

    // Draw source marker
    let source_pos = room_to_screen(state.source_x, state.source_y);
    let source_radius = 10.0;
    painter.circle_filled(source_pos, source_radius, source_color);
    painter.text(
        egui::pos2(source_pos.x, source_pos.y - source_radius - 4.0),
        egui::Align2::CENTER_BOTTOM,
        "S",
        egui::FontId::proportional(12.0),
        source_color,
    );

    // Draw listener marker
    let listener_pos = room_to_screen(state.listener_x, state.listener_y);
    let listener_radius = 10.0;
    painter.circle_filled(listener_pos, listener_radius, listener_color);
    painter.text(
        egui::pos2(listener_pos.x, listener_pos.y - listener_radius - 4.0),
        egui::Align2::CENTER_BOTTOM,
        "L",
        egui::FontId::proportional(12.0),
        listener_color,
    );

    // Draw dashed line between source and listener
    let line_color = egui::Color32::from_rgba_premultiplied(100, 100, 100, 128);
    painter.line_segment(
        [source_pos, listener_pos],
        egui::Stroke::new(1.0, line_color),
    );

    // Handle dragging
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if let Some(pos) = pointer
        && response.drag_started()
    {
        let dist_source = source_pos.distance(pos);
        let dist_listener = listener_pos.distance(pos);
        if dist_source < 20.0 && dist_source < dist_listener {
            state.dragging_source = true;
        } else if dist_listener < 20.0 {
            state.dragging_listener = true;
        }
    }

    if response.dragged()
        && let Some(pos) = pointer
    {
        let (rx, ry) = screen_to_room(pos.x, pos.y);
        let rx = rx.clamp(0.1, state.room_length - 0.1);
        let ry = ry.clamp(0.1, state.room_width - 0.1);

        if state.dragging_source {
            state.source_x = rx;
            state.source_y = ry;
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::SourcePos([rx, ry, state.room_height * 0.5]),
            });
        } else if state.dragging_listener {
            state.listener_x = rx;
            state.listener_y = ry;
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ListenerPos([rx, ry, state.room_height * 0.5]),
            });
        }
    }

    if response.drag_stopped() {
        state.dragging_source = false;
        state.dragging_listener = false;
    }
}

/// Draw the right-side control panel.
#[allow(clippy::too_many_lines)]
fn draw_controls(ui: &mut egui::Ui, handle: &mut EngineHandle, state: &mut AweUiState) {
    let t = theme();

    // --- Room Dimensions ---
    ui.heading(
        egui::RichText::new("Room")
            .color(t.colors.accent_cyan)
            .size(16.0),
    );
    ui.add_space(4.0);

    let mut room_changed = false;

    ui.horizontal(|ui| {
        ui.label("Length:");
        if ui
            .add(egui::Slider::new(&mut state.room_length, 2.0..=30.0).suffix("m"))
            .changed()
        {
            room_changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Width:");
        if ui
            .add(egui::Slider::new(&mut state.room_width, 2.0..=20.0).suffix("m"))
            .changed()
        {
            room_changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Height:");
        if ui
            .add(egui::Slider::new(&mut state.room_height, 2.0..=10.0).suffix("m"))
            .changed()
        {
            room_changed = true;
        }
    });

    if room_changed {
        let shape = RoomShape::Box {
            length: state.room_length,
            width: state.room_width,
            height: state.room_height,
        };
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::RoomShape(shape),
        });
        // Clamp positions to new room
        state.source_x = state.source_x.clamp(0.1, state.room_length - 0.1);
        state.source_y = state.source_y.clamp(0.1, state.room_width - 0.1);
        state.listener_x = state.listener_x.clamp(0.1, state.room_length - 0.1);
        state.listener_y = state.listener_y.clamp(0.1, state.room_width - 0.1);
    }

    ui.separator();

    // --- Material ---
    ui.heading(
        egui::RichText::new("Material")
            .color(t.colors.accent_cyan)
            .size(16.0),
    );
    ui.add_space(4.0);

    let prev_material = state.material_idx;
    egui::ComboBox::from_label("Wall")
        .selected_text(MATERIAL_NAMES[state.material_idx])
        .show_ui(ui, |ui| {
            for (i, name) in MATERIAL_NAMES.iter().enumerate() {
                ui.selectable_value(&mut state.material_idx, i, *name);
            }
        });

    if state.material_idx != prev_material {
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::Material(material_from_index(state.material_idx)),
        });
    }

    ui.separator();

    // --- Mix Parameters ---
    ui.heading(
        egui::RichText::new("Mix")
            .color(t.colors.accent_cyan)
            .size(16.0),
    );
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Dry/Wet:");
        if ui
            .add(egui::Slider::new(&mut state.dry_wet, 0.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::DryWet(state.dry_wet),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Early/Late:");
        if ui
            .add(egui::Slider::new(&mut state.early_late, 0.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::EarlyLateBalance(state.early_late),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Modes:");
        if ui
            .add(egui::Slider::new(&mut state.modes_amount, 0.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ModesAmount(state.modes_amount),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Tail:");
        if ui
            .add(egui::Slider::new(&mut state.tail_stretch, 0.5..=4.0).suffix("x"))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::TailStretch(state.tail_stretch),
            });
        }
    });

    ui.separator();

    // --- LFO 1 ---
    draw_lfo_section(ui, handle, "LFO 1", &mut state.lfo1, true);

    ui.separator();

    // --- LFO 2 ---
    draw_lfo_section(ui, handle, "LFO 2", &mut state.lfo2, false);
}

/// Draw a single LFO section.
fn draw_lfo_section(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    label: &str,
    lfo_state: &mut AweLfoState,
    is_lfo1: bool,
) {
    let t = theme();
    ui.heading(
        egui::RichText::new(label)
            .color(t.colors.accent_purple)
            .size(14.0),
    );
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        ui.label("Rate:");
        if ui
            .add(
                egui::Slider::new(&mut lfo_state.rate, 0.01..=20.0)
                    .suffix(" Hz")
                    .logarithmic(true),
            )
            .changed()
        {
            let param = if is_lfo1 {
                AweParam::Lfo1Rate(lfo_state.rate)
            } else {
                AweParam::Lfo2Rate(lfo_state.rate)
            };
            handle.send(EngineCommand::SetAweParameter { param });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Amount:");
        if ui
            .add(egui::Slider::new(&mut lfo_state.amount, 0.0..=1.0))
            .changed()
        {
            let param = if is_lfo1 {
                AweParam::Lfo1Amount(lfo_state.amount)
            } else {
                AweParam::Lfo2Amount(lfo_state.amount)
            };
            handle.send(EngineCommand::SetAweParameter { param });
        }
    });

    let mut target_idx = lfo_target_to_index(lfo_state.target);
    let prev_target = target_idx;
    egui::ComboBox::from_label(if is_lfo1 { "Target 1" } else { "Target 2" })
        .selected_text(LFO_TARGET_NAMES[target_idx])
        .show_ui(ui, |ui| {
            for (i, name) in LFO_TARGET_NAMES.iter().enumerate() {
                ui.selectable_value(&mut target_idx, i, *name);
            }
        });

    if target_idx != prev_target {
        lfo_state.target = lfo_target_from_index(target_idx);
        let param = if is_lfo1 {
            AweParam::Lfo1Target(lfo_state.target)
        } else {
            AweParam::Lfo2Target(lfo_state.target)
        };
        handle.send(EngineCommand::SetAweParameter { param });
    }
}
