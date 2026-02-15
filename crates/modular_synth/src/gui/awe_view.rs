//! AWE (Acoustic World Engine) view — 2D floor plan with parameter controls.
//!
//! Displays:
//! - Room outline as a 2D floor plan (top-down)
//! - Draggable source and listener markers
//! - Material selector
//! - Sliders for dry/wet, early/late, modes amount, tail stretch
//! - Four LFO sections with rate/amount/target controls

use eframe::egui;

use crate::gui::app::state::AppView;
use crate::gui::theme::theme;
use synth_awe::params::{AweLfoState, AweLfoTarget};
use synth_awe::presets::awe_presets;
use synth_awe::{
    AweParam, AwePreset, Material, Meters, NotePositionMapping, RoomShape, StretchFactor,
};
use synth_core::{BipolarValue, Hertz, MidiNote, NormalizedValue};
use synth_engine::{EngineCommand, EngineHandle};

/// Room shape kind for the UI selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomShapeKind {
    Box,
    Cylinder,
    LShape,
    Sphere,
    Dome,
    Tube,
}

const SHAPE_NAMES: [&str; 6] = ["Box", "Cylinder", "L-Shape", "Sphere", "Dome", "Tube"];

/// UI state for the AWE view, stored in SynthApp.
pub struct AweUiState {
    // Room shape kind
    pub shape_kind: RoomShapeKind,

    // Box dimensions
    pub room_length: f32,
    pub room_width: f32,
    pub room_height: f32,

    // Cylinder dimensions
    pub cyl_radius: f32,
    pub cyl_length: f32,

    // L-shape dimensions
    pub l_length_a: f32,
    pub l_width_a: f32,
    pub l_length_b: f32,
    pub l_width_b: f32,
    pub l_height: f32,

    // Sphere dimensions
    pub sphere_radius: f32,

    // Dome dimensions
    pub dome_radius: f32,

    // Tube dimensions
    pub tube_radius: f32,
    pub tube_length: f32,

    // Positions (x, y only — z is fixed for floor plan)
    pub source_x: f32,
    pub source_y: f32,
    pub listener_x: f32,
    pub listener_y: f32,

    // Material index (0=Concrete, 1=Wood, 2=Glass, 3=Metal, 4=Fabric, 5=Tile, 6=Marble,
    // 7=Ice, 8=Carpet, 9=Water, 10=Void, 11=Prism, 12=Plasma, 13=Membrane, 14=Nanogel)
    pub material_idx: usize,
    /// Material diffusion override (0.0 - 1.0).
    pub material_diffusion: f32,

    // Mix parameters
    pub dry_wet: f32,
    pub early_late: f32,
    pub modes_amount: f32,
    pub tail_stretch: f32,
    pub freq_warp: f32,
    pub resonance_boost: f32,
    pub portal_amount: f32,

    // LFO states
    pub lfo1: AweLfoState,
    pub lfo2: AweLfoState,
    pub lfo3: AweLfoState,
    pub lfo4: AweLfoState,

    // Per-voice spatial
    pub spatial_enabled: bool,
    pub note_mapping_idx: usize,

    // Preset selection (None = custom / manual)
    pub selected_preset: Option<usize>,

    // Drag state
    pub dragging_source: bool,
    pub dragging_listener: bool,
}

impl Default for AweUiState {
    fn default() -> Self {
        let snap = synth_awe::AweSnapshot::default();
        let default_material = material_from_index(0);
        Self {
            shape_kind: RoomShapeKind::Box,
            room_length: 8.0,
            room_width: 5.0,
            room_height: 3.0,
            cyl_radius: 1.0,
            cyl_length: 20.0,
            l_length_a: 8.0,
            l_width_a: 5.0,
            l_length_b: 6.0,
            l_width_b: 4.0,
            l_height: 3.0,
            sphere_radius: 5.0,
            dome_radius: 6.0,
            tube_radius: 1.5,
            tube_length: 30.0,
            source_x: snap.source_pos.x().as_f32(),
            source_y: snap.source_pos.y().as_f32(),
            listener_x: snap.listener_pos.x().as_f32(),
            listener_y: snap.listener_pos.y().as_f32(),
            material_idx: 0,
            material_diffusion: default_material.diffusion.as_f32(),
            dry_wet: snap.dry_wet.as_f32(),
            early_late: snap.early_late_balance.as_f32(),
            modes_amount: snap.modes_amount.as_f32(),
            tail_stretch: snap.tail_stretch.as_f32(),
            freq_warp: snap.freq_warp.as_f32(),
            resonance_boost: snap.resonance_boost.as_f32(),
            portal_amount: snap.portal_amount.as_f32(),
            lfo1: snap.lfo1,
            lfo2: snap.lfo2,
            lfo3: snap.lfo3,
            lfo4: snap.lfo4,
            spatial_enabled: false,
            note_mapping_idx: 0,
            selected_preset: None,
            dragging_source: false,
            dragging_listener: false,
        }
    }
}

impl AweUiState {
    /// Build a `RoomShape` from the current UI state.
    pub fn current_room_shape(&self) -> RoomShape {
        match self.shape_kind {
            RoomShapeKind::Box => RoomShape::Box {
                length: Meters::new(self.room_length),
                width: Meters::new(self.room_width),
                height: Meters::new(self.room_height),
            },
            RoomShapeKind::Cylinder => RoomShape::Cylinder {
                radius: Meters::new(self.cyl_radius),
                length: Meters::new(self.cyl_length),
            },
            RoomShapeKind::LShape => RoomShape::LShape {
                length_a: Meters::new(self.l_length_a),
                width_a: Meters::new(self.l_width_a),
                length_b: Meters::new(self.l_length_b),
                width_b: Meters::new(self.l_width_b),
                height: Meters::new(self.l_height),
            },
            RoomShapeKind::Sphere => RoomShape::Sphere {
                radius: Meters::new(self.sphere_radius),
            },
            RoomShapeKind::Dome => RoomShape::Dome {
                radius: Meters::new(self.dome_radius),
            },
            RoomShapeKind::Tube => RoomShape::Tube {
                radius: Meters::new(self.tube_radius),
                length: Meters::new(self.tube_length),
            },
        }
    }

    /// Build a `Material` from the current UI state.
    pub fn current_material(&self) -> Material {
        let mut material = material_from_index(self.material_idx);
        material.diffusion = NormalizedValue::new(self.material_diffusion);
        material
    }

    /// Effective room length for floor plan display.
    fn effective_length(&self) -> f32 {
        self.current_room_shape().length().as_f32()
    }

    /// Effective room width for floor plan display.
    fn effective_width(&self) -> f32 {
        self.current_room_shape().width().as_f32()
    }

    /// Build a full serializable `AweState` from the current UI state.
    #[must_use]
    pub fn to_awe_state(&self, enabled: bool) -> synth_awe::AweState {
        let room = self.current_room_shape();
        let half_height = room.height().as_f32() * 0.5;
        synth_awe::AweState {
            enabled,
            room,
            material: self.current_material(),
            spatial_enabled: self.spatial_enabled,
            note_mapping: mapping_from_index(self.note_mapping_idx),
            snapshot: synth_awe::AweSnapshot {
                dry_wet: NormalizedValue::new(self.dry_wet),
                early_late_balance: NormalizedValue::new(self.early_late),
                modes_amount: NormalizedValue::new(self.modes_amount),
                freq_warp: BipolarValue::new(self.freq_warp),
                resonance_boost: NormalizedValue::new(self.resonance_boost),
                tail_stretch: StretchFactor::new(self.tail_stretch),
                portal_amount: NormalizedValue::new(self.portal_amount),
                source_pos: [self.source_x, self.source_y, half_height].into(),
                listener_pos: [self.listener_x, self.listener_y, half_height].into(),
                spatial_enabled: self.spatial_enabled,
                note_mapping: mapping_from_index(self.note_mapping_idx),
                lfo1: self.lfo1,
                lfo2: self.lfo2,
                lfo3: self.lfo3,
                lfo4: self.lfo4,
            },
        }
    }

    /// Restore UI state from a loaded `AweState`.
    pub fn restore_from(&mut self, state: &synth_awe::AweState) {
        // Room shape
        match state.room {
            RoomShape::Box {
                length,
                width,
                height,
            } => {
                self.shape_kind = RoomShapeKind::Box;
                self.room_length = length.as_f32();
                self.room_width = width.as_f32();
                self.room_height = height.as_f32();
            }
            RoomShape::Cylinder { radius, length } => {
                self.shape_kind = RoomShapeKind::Cylinder;
                self.cyl_radius = radius.as_f32();
                self.cyl_length = length.as_f32();
            }
            RoomShape::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => {
                self.shape_kind = RoomShapeKind::LShape;
                self.l_length_a = length_a.as_f32();
                self.l_width_a = width_a.as_f32();
                self.l_length_b = length_b.as_f32();
                self.l_width_b = width_b.as_f32();
                self.l_height = height.as_f32();
            }
            RoomShape::Sphere { radius } => {
                self.shape_kind = RoomShapeKind::Sphere;
                self.sphere_radius = radius.as_f32();
            }
            RoomShape::Dome { radius } => {
                self.shape_kind = RoomShapeKind::Dome;
                self.dome_radius = radius.as_f32();
            }
            RoomShape::Tube { radius, length } => {
                self.shape_kind = RoomShapeKind::Tube;
                self.tube_radius = radius.as_f32();
                self.tube_length = length.as_f32();
            }
        }

        // Material
        self.material_idx = material_to_index(state.material);
        self.material_diffusion = state.material.diffusion.as_f32();

        // Spatial
        self.spatial_enabled = state.spatial_enabled;
        self.note_mapping_idx = mapping_to_index(state.note_mapping);

        // Snapshot params
        let snap = &state.snapshot;
        self.dry_wet = snap.dry_wet.as_f32();
        self.early_late = snap.early_late_balance.as_f32();
        self.modes_amount = snap.modes_amount.as_f32();
        self.freq_warp = snap.freq_warp.as_f32();
        self.resonance_boost = snap.resonance_boost.as_f32();
        self.tail_stretch = snap.tail_stretch.as_f32();
        self.portal_amount = snap.portal_amount.as_f32();

        // Positions
        self.source_x = snap.source_pos.x().as_f32();
        self.source_y = snap.source_pos.y().as_f32();
        self.listener_x = snap.listener_pos.x().as_f32();
        self.listener_y = snap.listener_pos.y().as_f32();

        // LFOs
        self.lfo1 = snap.lfo1;
        self.lfo2 = snap.lfo2;
        self.lfo3 = snap.lfo3;
        self.lfo4 = snap.lfo4;
    }
}

const MATERIAL_NAMES: [&str; 15] = [
    "Concrete", "Wood", "Glass", "Metal", "Fabric", "Tile", "Marmor", "Is", "Matta", "Vatten",
    "Tomrum", "Prisma", "Plasma", "Membran", "Nanogel",
];

fn material_from_index(idx: usize) -> Material {
    match idx {
        0 => Material::CONCRETE,
        1 => Material::WOOD,
        2 => Material::GLASS,
        3 => Material::METAL,
        4 => Material::FABRIC,
        5 => Material::TILE,
        6 => Material::MARBLE,
        7 => Material::ICE,
        8 => Material::CARPET,
        9 => Material::WATER,
        10 => Material::VOID,
        11 => Material::PRISM,
        12 => Material::PLASMA,
        13 => Material::MEMBRANE,
        14 => Material::NANOGEL,
        _ => Material::CONCRETE,
    }
}

fn material_to_index(mat: Material) -> usize {
    const MATERIALS: [Material; 15] = [
        Material::CONCRETE,
        Material::WOOD,
        Material::GLASS,
        Material::METAL,
        Material::FABRIC,
        Material::TILE,
        Material::MARBLE,
        Material::ICE,
        Material::CARPET,
        Material::WATER,
        Material::VOID,
        Material::PRISM,
        Material::PLASMA,
        Material::MEMBRANE,
        Material::NANOGEL,
    ];
    let mut best_idx = 0;
    let mut best_score = f32::MAX;
    for (idx, m) in MATERIALS.iter().enumerate() {
        let score = (m.absorption_low.as_f32() - mat.absorption_low.as_f32()).abs()
            + (m.absorption_mid.as_f32() - mat.absorption_mid.as_f32()).abs()
            + (m.absorption_high.as_f32() - mat.absorption_high.as_f32()).abs()
            + (m.diffusion.as_f32() - mat.diffusion.as_f32()).abs() * 0.25;
        if score < best_score {
            best_score = score;
            best_idx = idx;
        }
    }
    best_idx
}

const LFO_TARGET_NAMES: [&str; 13] = [
    "Room Length",
    "Room Width",
    "Source X",
    "Source Y",
    "Listener X",
    "Listener Y",
    "Dry/Wet",
    "Freq Warp",
    "Early/Late",
    "Modes Amount",
    "Res Boost",
    "Tail Stretch",
    "Portal",
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
        8 => AweLfoTarget::EarlyLate,
        9 => AweLfoTarget::ModesAmount,
        10 => AweLfoTarget::ResonanceBoost,
        11 => AweLfoTarget::TailStretch,
        12 => AweLfoTarget::PortalAmount,
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
        AweLfoTarget::EarlyLate => 8,
        AweLfoTarget::ModesAmount => 9,
        AweLfoTarget::ResonanceBoost => 10,
        AweLfoTarget::TailStretch => 11,
        AweLfoTarget::PortalAmount => 12,
    }
}

fn is_extreme_preset(preset: &AwePreset) -> bool {
    preset.name.starts_with("EXT:")
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

            ui.separator();

            // Preset selector
            let presets = awe_presets();
            let mut standard_indices = Vec::new();
            let mut extreme_indices = Vec::new();
            for (i, preset) in presets.iter().enumerate() {
                if is_extreme_preset(preset) {
                    extreme_indices.push(i);
                } else {
                    standard_indices.push(i);
                }
            }
            let preset_label = ui_state
                .selected_preset
                .and_then(|i| presets.get(i))
                .map_or("-- Preset --", |p| p.name);
            let mut new_preset = ui_state.selected_preset;
            egui::ComboBox::from_id_salt("awe_preset")
                .selected_text(preset_label)
                .show_ui(ui, |ui| {
                    if !standard_indices.is_empty() {
                        ui.label("Standard");
                        for i in &standard_indices {
                            if let Some(preset) = presets.get(*i) {
                                let selected = ui_state.selected_preset == Some(*i);
                                if ui
                                    .selectable_label(selected, preset.name)
                                    .on_hover_text(preset.description)
                                    .clicked()
                                {
                                    new_preset = Some(*i);
                                }
                            }
                        }
                    }
                    if !extreme_indices.is_empty() {
                        ui.separator();
                        ui.label("Extreme");
                        for i in &extreme_indices {
                            if let Some(preset) = presets.get(*i) {
                                let selected = ui_state.selected_preset == Some(*i);
                                if ui
                                    .selectable_label(selected, preset.name)
                                    .on_hover_text(preset.description)
                                    .clicked()
                                {
                                    new_preset = Some(*i);
                                }
                            }
                        }
                    }
                });
            if new_preset != ui_state.selected_preset
                && let Some(idx) = new_preset
                && let Some(preset) = presets.get(idx)
            {
                ui_state.selected_preset = Some(idx);
                ui_state.restore_from(&preset.state);
                // Send all state to engine
                *awe_enabled = preset.state.enabled;
                handle.send(EngineCommand::SetAweEnabled {
                    enabled: preset.state.enabled,
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::RoomShape(preset.state.room),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Material(preset.state.material),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::DryWet(preset.state.snapshot.dry_wet),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::EarlyLateBalance(preset.state.snapshot.early_late_balance),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::ModesAmount(preset.state.snapshot.modes_amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::FreqWarp(preset.state.snapshot.freq_warp),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::ResonanceBoost(preset.state.snapshot.resonance_boost),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::TailStretch(preset.state.snapshot.tail_stretch),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::PortalAmount(preset.state.snapshot.portal_amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::SourcePos(preset.state.snapshot.source_pos),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::ListenerPos(preset.state.snapshot.listener_pos),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::SpatialEnabled(preset.state.spatial_enabled),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::NoteMapping(preset.state.note_mapping),
                });
                // LFOs
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo1Rate(preset.state.snapshot.lfo1.rate),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo1Amount(preset.state.snapshot.lfo1.amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo1Target(preset.state.snapshot.lfo1.target),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo2Rate(preset.state.snapshot.lfo2.rate),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo2Amount(preset.state.snapshot.lfo2.amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo2Target(preset.state.snapshot.lfo2.target),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo3Rate(preset.state.snapshot.lfo3.rate),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo3Amount(preset.state.snapshot.lfo3.amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo3Target(preset.state.snapshot.lfo3.target),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo4Rate(preset.state.snapshot.lfo4.rate),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo4Amount(preset.state.snapshot.lfo4.amount),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Lfo4Target(preset.state.snapshot.lfo4.target),
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

    let room_w = state.effective_length().max(0.5);
    let room_h = state.effective_width().max(0.5);

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
        format!("{:.1}m", room_w),
        egui::FontId::proportional(12.0),
        dim_label_color,
    );
    painter.text(
        egui::pos2(room_screen_rect.min.x - 14.0, room_screen_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.1}m", room_h),
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

    // Draw per-voice mapping pattern (dim dots showing where notes would go)
    let mapping = mapping_from_index(state.note_mapping_idx);
    if state.spatial_enabled && mapping != NotePositionMapping::Off {
        let dot_color = egui::Color32::from_rgba_premultiplied(180, 120, 255, 60);
        // Show positions for C notes across the range
        for octave in 1..=7u8 {
            let note = MidiNote::new(octave * 12); // C1=12, C2=24, ...C7=84
            let eff_height = state.current_room_shape().height();
            let pos = mapping.position_for_note(
                note,
                Meters::new(room_w),
                Meters::new(room_h),
                eff_height,
            );
            let screen_pos = room_to_screen(pos.x().as_f32(), pos.y().as_f32());
            painter.circle_filled(screen_pos, 4.0, dot_color);
        }
    }

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
        let rx = rx.clamp(0.1, room_w - 0.1);
        let ry = ry.clamp(0.1, room_h - 0.1);

        let half_h = state.current_room_shape().height().as_f32() * 0.5;
        if state.dragging_source {
            state.source_x = rx;
            state.source_y = ry;
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::SourcePos([rx, ry, half_h].into()),
            });
        } else if state.dragging_listener {
            state.listener_x = rx;
            state.listener_y = ry;
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ListenerPos([rx, ry, half_h].into()),
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

    // --- Room Shape & Dimensions ---
    ui.heading(
        egui::RichText::new("Room")
            .color(t.colors.accent_cyan)
            .size(16.0),
    );
    ui.add_space(4.0);

    // Shape selector
    let prev_shape = state.shape_kind;
    let shape_idx = match state.shape_kind {
        RoomShapeKind::Box => 0,
        RoomShapeKind::Cylinder => 1,
        RoomShapeKind::LShape => 2,
        RoomShapeKind::Sphere => 3,
        RoomShapeKind::Dome => 4,
        RoomShapeKind::Tube => 5,
    };
    let mut new_shape_idx = shape_idx;
    egui::ComboBox::from_label("Shape")
        .selected_text(SHAPE_NAMES[shape_idx])
        .show_ui(ui, |ui| {
            for (i, name) in SHAPE_NAMES.iter().enumerate() {
                ui.selectable_value(&mut new_shape_idx, i, *name);
            }
        });
    if new_shape_idx != shape_idx {
        state.shape_kind = match new_shape_idx {
            0 => RoomShapeKind::Box,
            1 => RoomShapeKind::Cylinder,
            2 => RoomShapeKind::LShape,
            3 => RoomShapeKind::Sphere,
            4 => RoomShapeKind::Dome,
            _ => RoomShapeKind::Tube,
        };
        state.selected_preset = None;
    }

    let mut room_changed = prev_shape != state.shape_kind;

    // Dimension sliders adapted to shape type
    match state.shape_kind {
        RoomShapeKind::Box => {
            ui.horizontal(|ui| {
                ui.label("Length:");
                if ui
                    .add(egui::Slider::new(&mut state.room_length, 2.0..=100.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Width:");
                if ui
                    .add(egui::Slider::new(&mut state.room_width, 2.0..=100.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Height:");
                if ui
                    .add(egui::Slider::new(&mut state.room_height, 2.0..=20.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
        RoomShapeKind::Cylinder => {
            ui.horizontal(|ui| {
                ui.label("Radius:");
                if ui
                    .add(egui::Slider::new(&mut state.cyl_radius, 0.5..=10.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Length:");
                if ui
                    .add(egui::Slider::new(&mut state.cyl_length, 2.0..=200.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
        RoomShapeKind::LShape => {
            ui.horizontal(|ui| {
                ui.label("Len A:");
                if ui
                    .add(egui::Slider::new(&mut state.l_length_a, 2.0..=30.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Wid A:");
                if ui
                    .add(egui::Slider::new(&mut state.l_width_a, 2.0..=20.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Len B:");
                if ui
                    .add(egui::Slider::new(&mut state.l_length_b, 2.0..=30.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Wid B:");
                if ui
                    .add(egui::Slider::new(&mut state.l_width_b, 2.0..=20.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Height:");
                if ui
                    .add(egui::Slider::new(&mut state.l_height, 2.0..=10.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
        RoomShapeKind::Sphere => {
            ui.horizontal(|ui| {
                ui.label("Radius:");
                if ui
                    .add(egui::Slider::new(&mut state.sphere_radius, 1.0..=20.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
        RoomShapeKind::Dome => {
            ui.horizontal(|ui| {
                ui.label("Radius:");
                if ui
                    .add(egui::Slider::new(&mut state.dome_radius, 1.0..=20.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
        RoomShapeKind::Tube => {
            ui.horizontal(|ui| {
                ui.label("Radius:");
                if ui
                    .add(egui::Slider::new(&mut state.tube_radius, 0.5..=10.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Length:");
                if ui
                    .add(egui::Slider::new(&mut state.tube_length, 2.0..=200.0).suffix("m"))
                    .changed()
                {
                    room_changed = true;
                }
            });
        }
    }

    if room_changed {
        state.selected_preset = None;
        let shape = state.current_room_shape();
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::RoomShape(shape),
        });
        // Clamp positions to new effective room dimensions
        let eff_len = state.effective_length();
        let eff_wid = state.effective_width();
        state.source_x = state.source_x.clamp(0.1, eff_len - 0.1);
        state.source_y = state.source_y.clamp(0.1, eff_wid - 0.1);
        state.listener_x = state.listener_x.clamp(0.1, eff_len - 0.1);
        state.listener_y = state.listener_y.clamp(0.1, eff_wid - 0.1);
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
    let prev_diffusion = state.material_diffusion;
    egui::ComboBox::from_label("Wall")
        .selected_text(MATERIAL_NAMES[state.material_idx])
        .show_ui(ui, |ui| {
            for (i, name) in MATERIAL_NAMES.iter().enumerate() {
                ui.selectable_value(&mut state.material_idx, i, *name);
            }
        });

    if state.material_idx != prev_material {
        state.selected_preset = None;
        let base = material_from_index(state.material_idx);
        state.material_diffusion = base.diffusion.as_f32();
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::Material(state.current_material()),
        });
    }

    ui.horizontal(|ui| {
        ui.label("Diffusion:");
        if ui
            .add(egui::Slider::new(&mut state.material_diffusion, 0.0..=1.0))
            .changed()
        {
            state.selected_preset = None;
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::Material(state.current_material()),
            });
        }
    });

    if state.material_diffusion != prev_diffusion && state.material_idx == prev_material {
        state.selected_preset = None;
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
                param: AweParam::DryWet(NormalizedValue::new(state.dry_wet)),
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
                param: AweParam::EarlyLateBalance(NormalizedValue::new(state.early_late)),
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
                param: AweParam::ModesAmount(NormalizedValue::new(state.modes_amount)),
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
                param: AweParam::TailStretch(StretchFactor::new(state.tail_stretch)),
            });
        }
    });

    ui.separator();

    // --- Impossible Parameters ---
    ui.heading(
        egui::RichText::new("Impossible")
            .color(t.colors.accent_orange)
            .size(16.0),
    );
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Freq Warp:");
        if ui
            .add(egui::Slider::new(&mut state.freq_warp, -1.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::FreqWarp(BipolarValue::new(state.freq_warp)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Resonance:");
        if ui
            .add(egui::Slider::new(&mut state.resonance_boost, 0.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ResonanceBoost(NormalizedValue::new(state.resonance_boost)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Portal:");
        if ui
            .add(egui::Slider::new(&mut state.portal_amount, 0.0..=1.0))
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::PortalAmount(NormalizedValue::new(state.portal_amount)),
            });
        }
    });

    ui.separator();

    // --- Spatial ---
    draw_spatial_section(ui, handle, state);

    ui.separator();

    // --- LFO 1 ---
    draw_lfo_section(ui, handle, "LFO 1", &mut state.lfo1, 1);

    ui.separator();

    // --- LFO 2 ---
    draw_lfo_section(ui, handle, "LFO 2", &mut state.lfo2, 2);

    ui.separator();

    // --- LFO 3 ---
    draw_lfo_section(ui, handle, "LFO 3", &mut state.lfo3, 3);

    ui.separator();

    // --- LFO 4 ---
    draw_lfo_section(ui, handle, "LFO 4", &mut state.lfo4, 4);
}

const MAPPING_NAMES: [&str; 4] = ["Off", "Linear X", "Linear Y", "Circular"];

pub fn mapping_from_index(idx: usize) -> NotePositionMapping {
    match idx {
        0 => NotePositionMapping::Off,
        1 => NotePositionMapping::LinearX,
        2 => NotePositionMapping::LinearY,
        3 => NotePositionMapping::Circular,
        _ => NotePositionMapping::Off,
    }
}

pub fn mapping_to_index(mapping: NotePositionMapping) -> usize {
    match mapping {
        NotePositionMapping::Off => 0,
        NotePositionMapping::LinearX => 1,
        NotePositionMapping::LinearY => 2,
        NotePositionMapping::Circular => 3,
    }
}

/// Draw the per-voice spatial section.
fn draw_spatial_section(ui: &mut egui::Ui, handle: &mut EngineHandle, state: &mut AweUiState) {
    let t = theme();
    ui.heading(
        egui::RichText::new("Spatial")
            .color(t.colors.accent_purple)
            .size(16.0),
    );
    ui.add_space(4.0);

    // Toggle
    let prev_enabled = state.spatial_enabled;
    ui.horizontal(|ui| {
        ui.label("Per-voice:");
        let label = if state.spatial_enabled { "ON" } else { "OFF" };
        let color = if state.spatial_enabled {
            t.colors.meter_green
        } else {
            t.colors.text_dim
        };
        if ui.button(egui::RichText::new(label).color(color)).clicked() {
            state.spatial_enabled = !state.spatial_enabled;
        }
    });
    if state.spatial_enabled != prev_enabled {
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::SpatialEnabled(state.spatial_enabled),
        });
    }

    // Mapping selector
    let prev_mapping = state.note_mapping_idx;
    egui::ComboBox::from_label("Mapping")
        .selected_text(MAPPING_NAMES[state.note_mapping_idx])
        .show_ui(ui, |ui| {
            for (i, name) in MAPPING_NAMES.iter().enumerate() {
                ui.selectable_value(&mut state.note_mapping_idx, i, *name);
            }
        });

    if state.note_mapping_idx != prev_mapping {
        handle.send(EngineCommand::SetAweParameter {
            param: AweParam::NoteMapping(mapping_from_index(state.note_mapping_idx)),
        });
    }
}

/// Draw a single LFO section.
///
/// `lfo_index` is 1-based (1, 2, 3, or 4).
fn draw_lfo_section(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    label: &str,
    lfo_state: &mut AweLfoState,
    lfo_index: u8,
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
        let mut rate_f = lfo_state.rate.as_f32();
        if ui
            .add(
                egui::Slider::new(&mut rate_f, 0.01..=20.0)
                    .suffix(" Hz")
                    .logarithmic(true),
            )
            .changed()
        {
            lfo_state.rate = Hertz::new(rate_f);
            let param = match lfo_index {
                1 => AweParam::Lfo1Rate(lfo_state.rate),
                2 => AweParam::Lfo2Rate(lfo_state.rate),
                3 => AweParam::Lfo3Rate(lfo_state.rate),
                _ => AweParam::Lfo4Rate(lfo_state.rate),
            };
            handle.send(EngineCommand::SetAweParameter { param });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Amount:");
        let mut amount_f = lfo_state.amount.as_f32();
        if ui
            .add(egui::Slider::new(&mut amount_f, 0.0..=1.0))
            .changed()
        {
            lfo_state.amount = NormalizedValue::new(amount_f);
            let param = match lfo_index {
                1 => AweParam::Lfo1Amount(lfo_state.amount),
                2 => AweParam::Lfo2Amount(lfo_state.amount),
                3 => AweParam::Lfo3Amount(lfo_state.amount),
                _ => AweParam::Lfo4Amount(lfo_state.amount),
            };
            handle.send(EngineCommand::SetAweParameter { param });
        }
    });

    let mut target_idx = lfo_target_to_index(lfo_state.target);
    let prev_target = target_idx;
    let combo_label = match lfo_index {
        1 => "Target 1",
        2 => "Target 2",
        3 => "Target 3",
        _ => "Target 4",
    };
    egui::ComboBox::from_label(combo_label)
        .selected_text(LFO_TARGET_NAMES[target_idx])
        .show_ui(ui, |ui| {
            for (i, name) in LFO_TARGET_NAMES.iter().enumerate() {
                ui.selectable_value(&mut target_idx, i, *name);
            }
        });

    if target_idx != prev_target {
        lfo_state.target = lfo_target_from_index(target_idx);
        let param = match lfo_index {
            1 => AweParam::Lfo1Target(lfo_state.target),
            2 => AweParam::Lfo2Target(lfo_state.target),
            3 => AweParam::Lfo3Target(lfo_state.target),
            _ => AweParam::Lfo4Target(lfo_state.target),
        };
        handle.send(EngineCommand::SetAweParameter { param });
    }
}
