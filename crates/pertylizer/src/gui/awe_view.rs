//! AWE (Acoustic World Engine) view — isometric 3D cutaway with parameter controls.
//!
//! Displays:
//! - Room as isometric 3D cutaway (back wall, right wall, floor visible)
//! - Draggable source and listener markers on the floor plane
//! - Expanding sound rings from source
//! - Animated reflection paths (marching ants)
//! - Material selector
//! - Sliders for dry/wet, early/late, modes amount, tail stretch
//! - Four LFO sections with rate/amount/target controls

use std::path::PathBuf;

use eframe::egui;

use crate::gui::theme::theme;
use crate::patch::AwePresetFile;
use synth_awe::params::{AweLfoState, AweLfoTarget};
use synth_awe::presets::awe_presets;
use synth_awe::{
    AweParam, AwePreset, Material, Meters, NotePositionMapping, RoomShape, StretchFactor,
};
use synth_core::{BipolarValue, Hertz, MidiNote, Milliseconds, NormalizedValue};
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

    // FDN modulation (chorus)
    pub modulation_depth: f32,
    pub modulation_rate: f32,

    // Mix parameters
    pub dry_wet: f32,
    pub early_late: f32,
    pub modes_amount: f32,
    pub tail_stretch: f32,
    pub freq_warp: f32,
    pub resonance_boost: f32,
    pub portal_amount: f32,
    pub pre_delay: f32,
    pub air_absorption: f32,
    pub width: f32,
    pub high_cut: f32,
    pub low_cut: f32,
    pub temperature: f32,

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

    /// Name of the currently active preset (empty = no preset loaded).
    pub current_preset_name: String,
    /// Description of the currently active preset.
    pub current_preset_description: String,
    /// Tags of the currently active preset.
    pub current_preset_tags: Vec<String>,

    /// Editable free-text description for the AWE state.
    pub description: String,
    /// True while the user is typing — guards against the MCP-shared pull
    /// clobbering keystrokes mid-edit.
    pub description_edit_in_progress: bool,

    // Drag state
    pub dragging_source: bool,
    pub dragging_listener: bool,

    // User presets loaded from disk
    pub user_presets: Vec<UserAwePreset>,
    /// Whether user presets have been scanned from disk.
    pub user_presets_loaded: bool,
}

/// A user-saved AWE preset loaded from disk.
pub struct UserAwePreset {
    /// Preset file data.
    pub preset: AwePresetFile,
    /// Path on disk.
    pub path: PathBuf,
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
            modulation_depth: snap.modulation_depth.as_f32(),
            modulation_rate: snap.modulation_rate.as_f32(),
            dry_wet: snap.dry_wet.as_f32(),
            early_late: snap.early_late_balance.as_f32(),
            modes_amount: snap.modes_amount.as_f32(),
            tail_stretch: snap.tail_stretch.as_f32(),
            freq_warp: snap.freq_warp.as_f32(),
            resonance_boost: snap.resonance_boost.as_f32(),
            portal_amount: snap.portal_amount.as_f32(),
            pre_delay: snap.pre_delay.as_f32(),
            air_absorption: snap.air_absorption.as_f32(),
            width: snap.width.as_f32(),
            high_cut: snap.high_cut.as_f32(),
            low_cut: snap.low_cut.as_f32(),
            temperature: snap.temperature.as_f32(),
            lfo1: snap.lfo1,
            lfo2: snap.lfo2,
            lfo3: snap.lfo3,
            lfo4: snap.lfo4,
            spatial_enabled: false,
            note_mapping_idx: 0,
            selected_preset: None,
            current_preset_name: String::new(),
            current_preset_description: String::new(),
            current_preset_tags: Vec::new(),
            description: String::new(),
            description_edit_in_progress: false,
            dragging_source: false,
            dragging_listener: false,
            user_presets: Vec::new(),
            user_presets_loaded: false,
        }
    }
}

impl AweUiState {
    /// Scan a directory for user AWE presets (`.json` files with `file_type: "awe_preset"`).
    pub fn scan_user_presets(&mut self, dir: &std::path::Path) {
        self.user_presets.clear();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && let Ok(preset) = AwePresetFile::load(&path)
                    && preset.file_type == "awe_preset"
                {
                    self.user_presets.push(UserAwePreset { preset, path });
                }
            }
        }
        self.user_presets
            .sort_by(|a, b| a.preset.name.cmp(&b.preset.name));
        self.user_presets_loaded = true;
    }

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
                pre_delay: Milliseconds::new(self.pre_delay),
                modulation_depth: NormalizedValue::new(self.modulation_depth),
                modulation_rate: Hertz::new(self.modulation_rate),
                air_absorption: NormalizedValue::new(self.air_absorption),
                width: NormalizedValue::new(self.width),
                high_cut: Hertz::new(self.high_cut),
                low_cut: Hertz::new(self.low_cut),
                temperature: synth_awe::Celsius::new(self.temperature),
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
        self.pre_delay = snap.pre_delay.as_f32();
        self.modulation_depth = snap.modulation_depth.as_f32();
        self.modulation_rate = snap.modulation_rate.as_f32();
        self.air_absorption = snap.air_absorption.as_f32();
        self.width = snap.width.as_f32();
        self.high_cut = snap.high_cut.as_f32();
        self.low_cut = snap.low_cut.as_f32();
        self.temperature = snap.temperature.as_f32();

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
    "Concrete", "Wood", "Glass", "Metal", "Fabric", "Tile", "Marble", "Ice", "Carpet", "Water",
    "Void", "Prism", "Plasma", "Membrane", "Nanogel",
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

const LFO_TARGET_NAMES: [&str; 21] = [
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
    "Pre-delay",
    "Mod Depth",
    "Mod Rate",
    "Air Absorb",
    "Width",
    "High Cut",
    "Low Cut",
    "Temperature",
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
        13 => AweLfoTarget::PreDelay,
        14 => AweLfoTarget::ModulationDepth,
        15 => AweLfoTarget::ModulationRate,
        16 => AweLfoTarget::AirAbsorption,
        17 => AweLfoTarget::Width,
        18 => AweLfoTarget::HighCut,
        19 => AweLfoTarget::LowCut,
        20 => AweLfoTarget::Temperature,
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
        AweLfoTarget::PreDelay => 13,
        AweLfoTarget::ModulationDepth => 14,
        AweLfoTarget::ModulationRate => 15,
        AweLfoTarget::AirAbsorption => 16,
        AweLfoTarget::Width => 17,
        AweLfoTarget::HighCut => 18,
        AweLfoTarget::LowCut => 19,
        AweLfoTarget::Temperature => 20,
    }
}

fn is_extreme_preset(preset: &AwePreset) -> bool {
    preset.name.starts_with("EXT:")
}

/// Apply an AWE preset: update UI state and send all parameters to the engine.
pub fn apply_awe_preset(
    preset_idx: usize,
    preset: &AwePreset,
    handle: &mut EngineHandle,
    awe_enabled: &mut bool,
    ui_state: &mut AweUiState,
) {
    ui_state.selected_preset = Some(preset_idx);
    ui_state.current_preset_name = preset.name.to_string();
    ui_state.current_preset_description = preset.description.to_string();
    ui_state.current_preset_tags.clear();
    ui_state.restore_from(&preset.state);
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
    handle.send(EngineCommand::SetAweState {
        snapshot: preset.state.snapshot,
    });
    handle.send(EngineCommand::SetAweParameter {
        param: AweParam::SpatialEnabled(preset.state.spatial_enabled),
    });
    handle.send(EngineCommand::SetAweParameter {
        param: AweParam::NoteMapping(preset.state.note_mapping),
    });
}

/// Action requested by the AWE view toolbar.
pub enum AweViewAction {
    /// No action.
    None,
    /// User wants to save the current AWE config as a preset.
    SavePreset,
    /// User wants to open an AWE preset file.
    LoadPreset,
}

/// Draw the AWE view.
#[allow(clippy::too_many_lines)]
pub fn draw_awe_view(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    awe_enabled: &mut bool,
    ui_state: &mut AweUiState,
) -> AweViewAction {
    let mut action = AweViewAction::None;
    // Top toolbar
    egui::Panel::top("awe_toolbar").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
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
            let mut apply_user_preset_idx: Option<usize> = None;
            egui::ComboBox::from_id_salt("awe_preset")
                .selected_text(preset_label)
                .show_ui(ui, |ui| {
                    // User presets
                    if !ui_state.user_presets.is_empty() {
                        ui.label("User");
                        for (i, user_preset) in ui_state.user_presets.iter().enumerate() {
                            let hover = if user_preset.preset.description.is_empty() {
                                user_preset.path.display().to_string()
                            } else {
                                user_preset.preset.description.clone()
                            };
                            if ui
                                .selectable_label(false, &user_preset.preset.name)
                                .on_hover_text(hover)
                                .clicked()
                            {
                                apply_user_preset_idx = Some(i);
                            }
                        }
                        ui.separator();
                    }
                    // Built-in standard presets
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
                apply_awe_preset(idx, preset, handle, awe_enabled, ui_state);
            }
            if let Some(idx) = apply_user_preset_idx {
                let user_preset = &ui_state.user_presets[idx].preset;
                let state = user_preset.state.clone();
                let name = user_preset.name.clone();
                let description = user_preset.description.clone();
                let tags = user_preset.tags.clone();
                ui_state.restore_from(&state);
                ui_state.selected_preset = None;
                ui_state.current_preset_name = name;
                ui_state.current_preset_description = description;
                ui_state.current_preset_tags = tags;
                *awe_enabled = state.enabled;
                handle.send(EngineCommand::SetAweEnabled {
                    enabled: state.enabled,
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::RoomShape(state.room),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::Material(state.material),
                });
                handle.send(EngineCommand::SetAweState {
                    snapshot: state.snapshot,
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::SpatialEnabled(state.spatial_enabled),
                });
                handle.send(EngineCommand::SetAweParameter {
                    param: AweParam::NoteMapping(state.note_mapping),
                });
            }

            ui.separator();

            // Show current preset name with tooltip
            if !ui_state.current_preset_name.is_empty() {
                let name_label = ui.label(
                    egui::RichText::new(&ui_state.current_preset_name)
                        .color(theme().colors.accent_green),
                );
                let mut tooltip_parts = Vec::new();
                if !ui_state.current_preset_description.is_empty() {
                    tooltip_parts.push(ui_state.current_preset_description.clone());
                }
                if !ui_state.current_preset_tags.is_empty() {
                    tooltip_parts
                        .push(format!("Tags: {}", ui_state.current_preset_tags.join(", ")));
                }
                if !tooltip_parts.is_empty() {
                    name_label.on_hover_text(tooltip_parts.join("\n"));
                }
                ui.separator();
            }

            if ui.button("Save Preset...").clicked() {
                action = AweViewAction::SavePreset;
            }
            if ui.button("Load Preset...").clicked() {
                action = AweViewAction::LoadPreset;
            }
        });
    });

    // Right side panel with controls
    egui::Panel::right("awe_controls")
        .min_size(220.0)
        .default_size(260.0)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .content_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    draw_controls(ui, handle, ui_state);
                });
        });

    // Central panel — 2D floor plan
    egui::CentralPanel::default().show_inside(ui, |ui| {
        draw_floor_plan(ui, handle, ui_state);
    });

    action
}

/// Draw an arrowhead (filled triangle) at `tip` pointing in direction `dir`.
fn draw_arrowhead(
    painter: &egui::Painter,
    tip: egui::Pos2,
    dir: egui::Vec2,
    size: f32,
    color: egui::Color32,
) {
    let norm_dir = dir.normalized();
    let perp = egui::vec2(-norm_dir.y, norm_dir.x);
    let base = tip - norm_dir * size;
    let left = base + perp * size * 0.5;
    let right = base - perp * size * 0.5;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, left, right],
        color,
        egui::Stroke::NONE,
    ));
}

// --- Isometric projection constants and helpers ---

/// cos(30°) for isometric projection.
const ISO_COS30: f32 = 0.866_025_4;
/// sin(30°) for isometric projection.
const ISO_SIN30: f32 = 0.5;

/// Project a 3D room coordinate to 2D screen space (isometric).
///
/// - `rx`, `ry`: floor coordinates (room x = length, room y = width)
/// - `rz`: height coordinate
/// - `scale`: pixels per meter
/// - `offset`: screen pixel offset for centering
fn iso_to_screen(rx: f32, ry: f32, rz: f32, scale: f32, offset: egui::Pos2) -> egui::Pos2 {
    egui::pos2(
        (rx - ry) * ISO_COS30 * scale + offset.x,
        (rx + ry) * ISO_SIN30 * scale - rz * scale + offset.y,
    )
}

/// Inverse isometric: screen position → floor coordinates (z=0).
fn screen_to_floor(sx: f32, sy: f32, scale: f32, offset: egui::Pos2) -> (f32, f32) {
    let dx = (sx - offset.x) / scale;
    let dy = (sy - offset.y) / scale;
    // dx = (rx - ry) * cos30
    // dy = (rx + ry) * sin30
    let rx_plus_ry = dy / ISO_SIN30;
    let rx_minus_ry = dx / ISO_COS30;
    let rx = (rx_plus_ry + rx_minus_ry) * 0.5;
    let ry = (rx_plus_ry - rx_minus_ry) * 0.5;
    (rx, ry)
}

/// Draw an isometric ellipse (circle projected onto the floor plane).
///
/// A circle of radius `r` centered at `(cx, cy, cz)` in room coords,
/// approximated with `segments` line segments.
#[allow(clippy::too_many_arguments)]
fn draw_iso_ellipse(
    painter: &egui::Painter,
    cx: f32,
    cy: f32,
    cz: f32,
    r: f32,
    scale: f32,
    offset: egui::Pos2,
    stroke: egui::Stroke,
    segments: usize,
) {
    if segments < 3 {
        return;
    }
    let pts: Vec<egui::Pos2> = (0..segments)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32 / segments as f32);
            let px = cx + r * angle.cos();
            let py = cy + r * angle.sin();
            iso_to_screen(px, py, cz, scale, offset)
        })
        .collect();
    painter.add(egui::Shape::closed_line(pts, stroke));
}

/// Draw a dashed line with animated offset (marching ants).
fn draw_dashed_line_animated(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    stroke: egui::Stroke,
    dash_len: f32,
    gap_len: f32,
    dash_offset: f32,
) {
    let diff = to - from;
    let total_len = diff.length();
    if total_len < 0.1 {
        return;
    }
    let dir = diff / total_len;
    let cycle = dash_len + gap_len;
    // Start offset modulo cycle length
    let start = dash_offset.rem_euclid(cycle);
    let mut dist = -start;
    while dist < total_len {
        let seg_start_dist = dist.max(0.0);
        let seg_end_dist = (dist + dash_len).min(total_len);
        if seg_end_dist > seg_start_dist {
            let seg_start = from + dir * seg_start_dist;
            let seg_end = from + dir * seg_end_dist;
            painter.line_segment([seg_start, seg_end], stroke);
        }
        dist += cycle;
    }
}

/// Compute isometric scale and offset to center a room in the draw rect.
///
/// Returns `(scale, offset)` where scale is pixels/meter and offset is the
/// screen position of room origin (0,0,0).
fn iso_scale_and_offset(
    room_l: f32,
    room_w: f32,
    room_h: f32,
    draw_rect: egui::Rect,
) -> (f32, egui::Pos2) {
    // Isometric bounding box dimensions (in meters, before scaling)
    let iso_width = (room_l + room_w) * ISO_COS30;
    let iso_height = (room_l + room_w) * ISO_SIN30 + room_h;

    let scale = (draw_rect.width() / iso_width).min(draw_rect.height() / iso_height) * 0.9;

    // The origin (0,0,0) projects to a specific point. We want the bounding box
    // centered in draw_rect.
    // Bounding box corners:
    //   top-left-ish: iso_to_screen(0, room_w, room_h) — back-left-top
    //   bottom-right-ish: iso_to_screen(room_l, 0, 0) — front-right-bottom
    // With offset=0: find min/max screen coords, then shift.
    let zero = egui::pos2(0.0, 0.0);
    let corners = [
        iso_to_screen(0.0, 0.0, 0.0, scale, zero),
        iso_to_screen(room_l, 0.0, 0.0, scale, zero),
        iso_to_screen(0.0, room_w, 0.0, scale, zero),
        iso_to_screen(room_l, room_w, 0.0, scale, zero),
        iso_to_screen(0.0, 0.0, room_h, scale, zero),
        iso_to_screen(room_l, 0.0, room_h, scale, zero),
        iso_to_screen(0.0, room_w, room_h, scale, zero),
        iso_to_screen(room_l, room_w, room_h, scale, zero),
    ];
    let min_x = corners.iter().map(|p| p.x).fold(f32::MAX, f32::min);
    let max_x = corners.iter().map(|p| p.x).fold(f32::MIN, f32::max);
    let min_y = corners.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let max_y = corners.iter().map(|p| p.y).fold(f32::MIN, f32::max);

    let bb_center_x = (min_x + max_x) * 0.5;
    let bb_center_y = (min_y + max_y) * 0.5;

    let offset = egui::pos2(
        draw_rect.center().x - bb_center_x,
        draw_rect.center().y - bb_center_y,
    );

    (scale, offset)
}

/// Draw expanding sound rings from the source on the floor plane.
///
/// Rings are only spawned when `audio_level > 0` and their intensity
/// is proportional to the audio level at birth time.
#[allow(clippy::too_many_arguments)]
fn draw_sound_rings(
    painter: &egui::Painter,
    source_x: f32,
    source_y: f32,
    time: f64,
    room_l: f32,
    room_w: f32,
    scale: f32,
    offset: egui::Pos2,
    color: egui::Color32,
    shape_kind: RoomShapeKind,
    audio_level: f32,
) {
    // No rings when silent
    if audio_level < 0.005 {
        return;
    }

    let diag = (room_l * room_l + room_w * room_w).sqrt();
    let max_radius = diag * 0.8;
    // Ring interval inversely proportional to level: louder = more frequent rings
    let ring_interval = 0.3 + 0.4 * (1.0 - audio_level) as f64;
    let max_age: f32 = 2.5;
    let max_rings = 6;
    let speed = max_radius / max_age;

    // Base opacity scaled by audio level
    let level_opacity = audio_level.sqrt();

    for i in 0..max_rings {
        let ring_birth = (time / ring_interval).floor() as i64 - i as i64;
        if ring_birth < 0 {
            continue;
        }
        let birth_time = ring_birth as f64 * ring_interval;
        let age = (time - birth_time) as f32;
        if age < 0.0 || age > max_age {
            continue;
        }
        let radius = age * speed;
        let alpha = ((1.0 - age / max_age) * 100.0 * level_opacity) as u8;
        if alpha == 0 {
            continue;
        }
        let ring_color =
            egui::Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), alpha);
        let ring_stroke = egui::Stroke::new(1.0 + level_opacity, ring_color);
        draw_iso_ellipse(
            painter,
            source_x,
            source_y,
            0.0,
            radius,
            scale,
            offset,
            ring_stroke,
            48,
        );

        // Reflection rings for box/tube shapes
        if matches!(shape_kind, RoomShapeKind::Box | RoomShapeKind::Tube) {
            let mirror_sources: &[(f32, f32)] = match shape_kind {
                RoomShapeKind::Box => &[
                    (-source_x, source_y),
                    (2.0 * room_l - source_x, source_y),
                    (source_x, -source_y),
                    (source_x, 2.0 * room_w - source_y),
                ],
                _ => &[(source_x, -source_y), (source_x, 2.0 * room_w - source_y)],
            };
            for &(mx, my) in mirror_sources {
                let dist_to_wall = ((mx - source_x).powi(2) + (my - source_y).powi(2)).sqrt() * 0.5;
                if radius > dist_to_wall {
                    let refl_age = age - dist_to_wall / speed;
                    if refl_age > 0.0 && refl_age < max_age {
                        let refl_radius = refl_age * speed * 0.7;
                        let refl_alpha = ((1.0 - refl_age / max_age) * 50.0 * level_opacity) as u8;
                        let refl_color = egui::Color32::from_rgba_premultiplied(
                            color.r(),
                            color.g(),
                            color.b(),
                            refl_alpha,
                        );
                        let refl_stroke = egui::Stroke::new(1.0, refl_color);
                        let reflect_x = (mx + source_x) * 0.5;
                        let reflect_y = (my + source_y) * 0.5;
                        draw_iso_ellipse(
                            painter,
                            reflect_x,
                            reflect_y,
                            0.0,
                            refl_radius,
                            scale,
                            offset,
                            refl_stroke,
                            48,
                        );
                    }
                }
            }
        }
    }
}

/// Draw the isometric 3D cutaway view of the room.
#[allow(clippy::too_many_lines)]
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

    // Audio level from engine (drives animations)
    let (peak_l, peak_r) = handle.peak_meters();
    let audio_level = peak_l.as_f32().max(peak_r.as_f32()).clamp(0.0, 1.0);
    let has_audio = audio_level > 0.005;

    // Animated time for rings and marching ants
    let time = ui.input(|i| i.time);
    if has_audio {
        ui.request_repaint();
    }

    // Room dimensions
    let room_l = state.effective_length().max(0.5);
    let room_w = state.effective_width().max(0.5);
    let room_h = state.current_room_shape().height().as_f32().max(0.5);

    let margin = 30.0;
    let draw_rect = rect.shrink(margin);
    let (scale, offset) = iso_scale_and_offset(room_l, room_w, room_h, draw_rect);

    let dim_label_color = t.colors.text_dim;

    // --- Draw room shape (isometric 3D cutaway) ---
    draw_iso_room(
        &painter, state, room_l, room_w, room_h, scale, offset, wall_color, bg_color,
    );

    // --- Dimension labels along visible floor edges ---
    draw_iso_dimension_labels(
        &painter,
        state,
        room_l,
        room_w,
        scale,
        offset,
        dim_label_color,
    );

    // --- Reflection paths (first-order, Box and Tube only) with marching ants ---
    // Reflection alpha scales with audio level (dim when silent, bright when loud)
    let refl_base_alpha = if has_audio {
        (40.0 + 80.0 * audio_level.sqrt()) as u8
    } else {
        40 // dim static lines when silent
    };
    let reflection_color = egui::Color32::from_rgba_premultiplied(
        wall_color.r() / 2,
        wall_color.g() / 2,
        wall_color.b() / 2,
        refl_base_alpha,
    );
    let reflection_stroke = egui::Stroke::new(1.0, reflection_color);
    let (sx, sy) = (state.source_x, state.source_y);
    let (lx, ly) = (state.listener_x, state.listener_y);
    // Marching ants speed proportional to audio level (0 when silent)
    let dash_speed = if has_audio {
        30.0 + 60.0 * audio_level as f64
    } else {
        0.0
    };
    let dash_offset = (time * dash_speed) as f32;

    match state.shape_kind {
        RoomShapeKind::Box => {
            let mirrors: [(f32, f32, bool, f32); 4] = [
                (-sx, sy, true, 0.0),
                (2.0 * room_l - sx, sy, true, room_l),
                (sx, -sy, false, 0.0),
                (sx, 2.0 * room_w - sy, false, room_w),
            ];
            for (mx, my, is_vertical, wall_coord) in mirrors {
                let refl = if is_vertical {
                    let dx = lx - mx;
                    if dx.abs() < 1e-6 {
                        continue;
                    }
                    let t_param = (wall_coord - mx) / dx;
                    let ry = my + t_param * (ly - my);
                    (wall_coord, ry)
                } else {
                    let dy = ly - my;
                    if dy.abs() < 1e-6 {
                        continue;
                    }
                    let t_param = (wall_coord - my) / dy;
                    let rx = mx + t_param * (lx - mx);
                    (rx, wall_coord)
                };
                if refl.0 >= 0.0 && refl.0 <= room_l && refl.1 >= 0.0 && refl.1 <= room_w {
                    let refl_screen = iso_to_screen(refl.0, refl.1, 0.0, scale, offset);
                    let src_screen = iso_to_screen(sx, sy, 0.0, scale, offset);
                    let lis_screen = iso_to_screen(lx, ly, 0.0, scale, offset);
                    let seg1_len = (refl_screen - src_screen).length();
                    draw_dashed_line_animated(
                        &painter,
                        src_screen,
                        refl_screen,
                        reflection_stroke,
                        4.0,
                        3.0,
                        dash_offset,
                    );
                    draw_dashed_line_animated(
                        &painter,
                        refl_screen,
                        lis_screen,
                        reflection_stroke,
                        4.0,
                        3.0,
                        dash_offset + seg1_len,
                    );
                }
            }
        }
        RoomShapeKind::Tube => {
            let mirrors: [(f32, f32, f32); 2] = [(sx, -sy, 0.0), (sx, 2.0 * room_w - sy, room_w)];
            for (mx, my, wall_y) in mirrors {
                let dy = ly - my;
                if dy.abs() < 1e-6 {
                    continue;
                }
                let t_param = (wall_y - my) / dy;
                let rx = mx + t_param * (lx - mx);
                if rx >= 0.0 && rx <= room_l {
                    let refl_screen = iso_to_screen(rx, wall_y, 0.0, scale, offset);
                    let src_screen = iso_to_screen(sx, sy, 0.0, scale, offset);
                    let lis_screen = iso_to_screen(lx, ly, 0.0, scale, offset);
                    let seg1_len = (refl_screen - src_screen).length();
                    draw_dashed_line_animated(
                        &painter,
                        src_screen,
                        refl_screen,
                        reflection_stroke,
                        4.0,
                        3.0,
                        dash_offset,
                    );
                    draw_dashed_line_animated(
                        &painter,
                        refl_screen,
                        lis_screen,
                        reflection_stroke,
                        4.0,
                        3.0,
                        dash_offset + seg1_len,
                    );
                }
            }
        }
        _ => {}
    }

    // --- Sound rings (driven by audio level) ---
    draw_sound_rings(
        &painter,
        sx,
        sy,
        time,
        room_l,
        room_w,
        scale,
        offset,
        source_color,
        state.shape_kind,
        audio_level,
    );

    // --- Source marker ---
    let source_pos = iso_to_screen(state.source_x, state.source_y, 0.0, scale, offset);
    let marker_radius = 14.0;
    painter.circle_filled(source_pos, marker_radius, source_color);
    painter.circle_stroke(
        source_pos,
        marker_radius + 2.0,
        egui::Stroke::new(1.5, source_color),
    );
    painter.text(
        egui::pos2(source_pos.x, source_pos.y - marker_radius - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "S",
        egui::FontId::proportional(13.0),
        source_color,
    );
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    if let Some(pp) = pointer_pos
        && source_pos.distance(pp) < marker_radius + 4.0
    {
        painter.text(
            egui::pos2(source_pos.x, source_pos.y + marker_radius + 6.0),
            egui::Align2::CENTER_TOP,
            "Source",
            egui::FontId::proportional(11.0),
            source_color,
        );
    }

    // --- Listener marker ---
    let listener_pos = iso_to_screen(state.listener_x, state.listener_y, 0.0, scale, offset);
    painter.circle_filled(listener_pos, marker_radius, listener_color);
    painter.circle_stroke(
        listener_pos,
        marker_radius + 2.0,
        egui::Stroke::new(1.5, listener_color),
    );
    painter.text(
        egui::pos2(listener_pos.x, listener_pos.y - marker_radius - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "L",
        egui::FontId::proportional(13.0),
        listener_color,
    );
    if let Some(pp) = pointer_pos
        && listener_pos.distance(pp) < marker_radius + 4.0
    {
        painter.text(
            egui::pos2(listener_pos.x, listener_pos.y + marker_radius + 6.0),
            egui::Align2::CENTER_TOP,
            "Listener",
            egui::FontId::proportional(11.0),
            listener_color,
        );
    }

    // --- Arrow from source to listener ---
    let arrow_color = egui::Color32::from_rgba_premultiplied(140, 140, 140, 180);
    let dir = listener_pos - source_pos;
    let dir_len = dir.length();
    if dir_len > 1.0 {
        let norm_dir = dir / dir_len;
        let line_start = source_pos + norm_dir * marker_radius;
        let arrow_tip = listener_pos - norm_dir * (marker_radius + 2.0);
        let line_end = listener_pos - norm_dir * (marker_radius + 10.0);
        painter.line_segment([line_start, line_end], egui::Stroke::new(1.5, arrow_color));
        draw_arrowhead(&painter, arrow_tip, dir, 10.0, arrow_color);
    }

    // --- Info box ---
    let room_shape = state.current_room_shape();
    let material = state.current_material();
    let volume = room_shape.volume().as_f32();
    let surface_area = room_shape.surface_area().as_f32();
    let avg_absorption = material.average_absorption().as_f32();
    let distance = ((lx - sx).powi(2) + (ly - sy).powi(2)).sqrt();
    let rt60 = if avg_absorption * surface_area > 0.001 {
        0.161 * volume / (avg_absorption * surface_area) * state.tail_stretch
    } else {
        0.0
    };

    let info_text =
        format!("Distance: {distance:.1} m\nRT60: {rt60:.1} s\nVolume: {volume:.0} m\u{00B3}");
    let info_pos = egui::pos2(draw_rect.min.x + 6.0, draw_rect.min.y + 6.0);
    let info_font = egui::FontId::proportional(11.0);
    let info_color = t.colors.text_dim;
    let info_galley = painter.layout_no_wrap(info_text.clone(), info_font.clone(), info_color);
    let info_rect = egui::Rect::from_min_size(info_pos, info_galley.size() + egui::vec2(8.0, 4.0));
    painter.rect_filled(
        info_rect,
        3.0,
        egui::Color32::from_rgba_premultiplied(bg_color.r(), bg_color.g(), bg_color.b(), 200),
    );
    painter.galley(
        egui::pos2(info_pos.x + 4.0, info_pos.y + 2.0),
        info_galley,
        info_color,
    );

    // --- Per-voice spatial mapping dots ---
    let mapping = mapping_from_index(state.note_mapping_idx);
    if state.spatial_enabled && mapping != NotePositionMapping::Off {
        let dot_color = egui::Color32::from_rgba_premultiplied(180, 120, 255, 60);
        for octave in 1..=7u8 {
            let note = MidiNote::new(octave * 12);
            let eff_height = state.current_room_shape().height();
            let pos = mapping.position_for_note(
                note,
                Meters::new(room_l),
                Meters::new(room_w),
                eff_height,
            );
            let screen_pos = iso_to_screen(pos.x().as_f32(), pos.y().as_f32(), 0.0, scale, offset);
            painter.circle_filled(screen_pos, 4.0, dot_color);
        }
    }

    // --- Handle dragging ---
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if let Some(pos) = pointer
        && response.drag_started()
    {
        let dist_source = source_pos.distance(pos);
        let dist_listener = listener_pos.distance(pos);
        if dist_source < 24.0 && dist_source < dist_listener {
            state.dragging_source = true;
        } else if dist_listener < 24.0 {
            state.dragging_listener = true;
        }
    }

    if response.dragged()
        && let Some(pos) = pointer
    {
        let (rx, ry) = screen_to_floor(pos.x, pos.y, scale, offset);
        let rx = rx.clamp(0.1, room_l - 0.1);
        let ry = ry.clamp(0.1, room_w - 0.1);

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

/// Draw the isometric room geometry (3D cutaway).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn draw_iso_room(
    painter: &egui::Painter,
    state: &AweUiState,
    room_l: f32,
    room_w: f32,
    room_h: f32,
    scale: f32,
    offset: egui::Pos2,
    wall_color: egui::Color32,
    bg_color: egui::Color32,
) {
    let wall_stroke = egui::Stroke::new(2.0, wall_color);

    // Colors for solid shading (cutaway style)
    let floor_color = egui::Color32::from_rgba_premultiplied(
        bg_color.r().saturating_add(15),
        bg_color.g().saturating_add(15),
        bg_color.b().saturating_add(20),
        160,
    );
    let back_wall_color = egui::Color32::from_rgba_premultiplied(
        bg_color.r().saturating_add(25),
        bg_color.g().saturating_add(25),
        bg_color.b().saturating_add(35),
        140,
    );
    let right_wall_color = egui::Color32::from_rgba_premultiplied(
        bg_color.r().saturating_add(35),
        bg_color.g().saturating_add(35),
        bg_color.b().saturating_add(45),
        140,
    );

    match state.shape_kind {
        RoomShapeKind::Box => {
            draw_iso_box(
                painter,
                room_l,
                room_w,
                room_h,
                scale,
                offset,
                floor_color,
                back_wall_color,
                right_wall_color,
                wall_stroke,
            );
        }
        RoomShapeKind::Cylinder => {
            draw_iso_cylinder(
                painter,
                state.cyl_length,
                state.cyl_radius,
                scale,
                offset,
                floor_color,
                back_wall_color,
                wall_stroke,
                wall_color,
            );
        }
        RoomShapeKind::LShape => {
            draw_iso_lshape(
                painter,
                state,
                scale,
                offset,
                floor_color,
                back_wall_color,
                right_wall_color,
                wall_stroke,
            );
        }
        RoomShapeKind::Sphere => {
            draw_iso_sphere(
                painter,
                state.sphere_radius,
                scale,
                offset,
                back_wall_color,
                wall_stroke,
                wall_color,
            );
        }
        RoomShapeKind::Dome => {
            draw_iso_dome(
                painter,
                state.dome_radius,
                scale,
                offset,
                floor_color,
                back_wall_color,
                wall_stroke,
                wall_color,
            );
        }
        RoomShapeKind::Tube => {
            draw_iso_tube(
                painter,
                state.tube_length,
                state.tube_radius,
                scale,
                offset,
                floor_color,
                back_wall_color,
                wall_stroke,
                wall_color,
            );
        }
    }
}

/// Draw an isometric box room (cutaway: back wall, right wall, floor).
#[allow(clippy::too_many_arguments)]
fn draw_iso_box(
    painter: &egui::Painter,
    l: f32,
    w: f32,
    h: f32,
    scale: f32,
    offset: egui::Pos2,
    floor_color: egui::Color32,
    back_wall_color: egui::Color32,
    right_wall_color: egui::Color32,
    wall_stroke: egui::Stroke,
) {
    // 8 corners
    let p = |x: f32, y: f32, z: f32| iso_to_screen(x, y, z, scale, offset);

    // Back wall (y=W): (0,W,0), (L,W,0), (L,W,H), (0,W,H)
    painter.add(egui::Shape::convex_polygon(
        vec![p(0.0, w, 0.0), p(l, w, 0.0), p(l, w, h), p(0.0, w, h)],
        back_wall_color,
        egui::Stroke::NONE,
    ));

    // Right wall (x=L): (L,0,0), (L,W,0), (L,W,H), (L,0,H)
    painter.add(egui::Shape::convex_polygon(
        vec![p(l, 0.0, 0.0), p(l, w, 0.0), p(l, w, h), p(l, 0.0, h)],
        right_wall_color,
        egui::Stroke::NONE,
    ));

    // Floor (z=0): (0,0,0), (L,0,0), (L,W,0), (0,W,0)
    painter.add(egui::Shape::convex_polygon(
        vec![
            p(0.0, 0.0, 0.0),
            p(l, 0.0, 0.0),
            p(l, w, 0.0),
            p(0.0, w, 0.0),
        ],
        floor_color,
        egui::Stroke::NONE,
    ));

    // Visible edges
    // Floor front edges
    painter.line_segment([p(0.0, 0.0, 0.0), p(l, 0.0, 0.0)], wall_stroke);
    painter.line_segment([p(0.0, 0.0, 0.0), p(0.0, w, 0.0)], wall_stroke);
    // Floor back edges (along walls)
    painter.line_segment([p(l, 0.0, 0.0), p(l, w, 0.0)], wall_stroke);
    painter.line_segment([p(0.0, w, 0.0), p(l, w, 0.0)], wall_stroke);
    // Vertical edges
    painter.line_segment([p(l, 0.0, 0.0), p(l, 0.0, h)], wall_stroke);
    painter.line_segment([p(0.0, w, 0.0), p(0.0, w, h)], wall_stroke);
    painter.line_segment([p(l, w, 0.0), p(l, w, h)], wall_stroke);
    // Top edges (ceiling outline)
    painter.line_segment([p(l, 0.0, h), p(l, w, h)], wall_stroke);
    painter.line_segment([p(0.0, w, h), p(l, w, h)], wall_stroke);
}

/// Draw an isometric cylinder room.
#[allow(clippy::too_many_arguments)]
fn draw_iso_cylinder(
    painter: &egui::Painter,
    length: f32,
    radius: f32,
    scale: f32,
    offset: egui::Pos2,
    floor_color: egui::Color32,
    wall_color_fill: egui::Color32,
    wall_stroke: egui::Stroke,
    edge_color: egui::Color32,
) {
    let diameter = radius * 2.0;
    // Bounding box approach: length along x, diameter along y, diameter along z
    // Back half of cylinder (y = radius + r*sin(a) for a in PI..TAU)
    let n_segs = 32;

    // Draw back wall as polygon band (approximated)
    // Back cross-section at x=length: semi-ellipse top half
    let mut back_pts = Vec::with_capacity(n_segs + 2);
    for i in 0..=n_segs {
        let angle = std::f32::consts::PI * (i as f32 / n_segs as f32);
        let cy = radius + radius * angle.cos();
        let cz = radius * angle.sin();
        back_pts.push(iso_to_screen(length, cy, cz, scale, offset));
    }
    // Close with base
    back_pts.push(iso_to_screen(length, diameter, 0.0, scale, offset));
    back_pts.push(iso_to_screen(length, 0.0, 0.0, scale, offset));
    painter.add(egui::Shape::convex_polygon(
        back_pts,
        wall_color_fill,
        egui::Stroke::NONE,
    ));

    // Floor: rectangle (0,0,0) to (length,diameter,0)
    let p = |x: f32, y: f32, z: f32| iso_to_screen(x, y, z, scale, offset);
    painter.add(egui::Shape::convex_polygon(
        vec![
            p(0.0, 0.0, 0.0),
            p(length, 0.0, 0.0),
            p(length, diameter, 0.0),
            p(0.0, diameter, 0.0),
        ],
        floor_color,
        egui::Stroke::NONE,
    ));

    // Front cross-section at x=0: isometric ellipse
    let ellipse_stroke = egui::Stroke::new(1.5, edge_color);
    draw_iso_ellipse(
        painter,
        0.0,
        radius,
        radius,
        radius,
        scale,
        offset,
        ellipse_stroke,
        48,
    );

    // Back cross-section at x=length
    draw_iso_ellipse(
        painter,
        length,
        radius,
        radius,
        radius,
        scale,
        offset,
        ellipse_stroke,
        48,
    );

    // Top ridge lines (along length)
    painter.line_segment(
        [p(0.0, radius, diameter), p(length, radius, diameter)],
        wall_stroke,
    );

    // Bottom edge lines
    painter.line_segment([p(0.0, 0.0, 0.0), p(length, 0.0, 0.0)], wall_stroke);
    painter.line_segment(
        [p(0.0, diameter, 0.0), p(length, diameter, 0.0)],
        wall_stroke,
    );
}

/// Draw an isometric L-shaped room.
#[allow(clippy::too_many_arguments)]
fn draw_iso_lshape(
    painter: &egui::Painter,
    state: &AweUiState,
    scale: f32,
    offset: egui::Pos2,
    floor_color: egui::Color32,
    back_wall_color: egui::Color32,
    right_wall_color: egui::Color32,
    wall_stroke: egui::Stroke,
) {
    let la = state.l_length_a;
    let wa = state.l_width_a;
    let lb = state.l_length_b;
    let wb = state.l_width_b;
    let h = state.l_height;
    let total_w = wa.max(wb);
    let total_h = la + lb;
    let _ = total_w; // used implicitly by the L-shape geometry

    let p = |x: f32, y: f32, z: f32| iso_to_screen(x, y, z, scale, offset);

    // L-shape floor polygon: 6 points
    let floor_pts = [
        p(0.0, 0.0, 0.0),
        p(wa, 0.0, 0.0),
        p(wa, la, 0.0),
        p(wb, la, 0.0),
        p(wb, total_h, 0.0),
        p(0.0, total_h, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        vec![floor_pts[0], floor_pts[1], floor_pts[2], floor_pts[5]],
        floor_color,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![floor_pts[2], floor_pts[3], floor_pts[4], floor_pts[5]],
        floor_color,
        egui::Stroke::NONE,
    ));

    // Back wall sections (y=total_h): from (0,total_h,0) to (wb,total_h,0) to height
    painter.add(egui::Shape::convex_polygon(
        vec![
            p(0.0, total_h, 0.0),
            p(wb, total_h, 0.0),
            p(wb, total_h, h),
            p(0.0, total_h, h),
        ],
        back_wall_color,
        egui::Stroke::NONE,
    ));

    // Right wall of section A (x=wa, y=0 to la)
    painter.add(egui::Shape::convex_polygon(
        vec![p(wa, 0.0, 0.0), p(wa, la, 0.0), p(wa, la, h), p(wa, 0.0, h)],
        right_wall_color,
        egui::Stroke::NONE,
    ));

    // Right wall of section B (x=wb, y=la to total_h)
    if (wb - wa).abs() > 0.01 {
        painter.add(egui::Shape::convex_polygon(
            vec![
                p(wb, la, 0.0),
                p(wb, total_h, 0.0),
                p(wb, total_h, h),
                p(wb, la, h),
            ],
            right_wall_color,
            egui::Stroke::NONE,
        ));
    }

    // Step wall (horizontal at y=la from x=wb to x=wa, if different)
    if wa > wb {
        painter.add(egui::Shape::convex_polygon(
            vec![p(wb, la, 0.0), p(wa, la, 0.0), p(wa, la, h), p(wb, la, h)],
            back_wall_color,
            egui::Stroke::NONE,
        ));
    }

    // Floor outline
    let outline_pts = vec![
        p(0.0, 0.0, 0.0),
        p(wa, 0.0, 0.0),
        p(wa, la, 0.0),
        p(wb, la, 0.0),
        p(wb, total_h, 0.0),
        p(0.0, total_h, 0.0),
    ];
    painter.add(egui::Shape::closed_line(outline_pts, wall_stroke));

    // Vertical edges
    painter.line_segment([p(wa, 0.0, 0.0), p(wa, 0.0, h)], wall_stroke);
    painter.line_segment([p(0.0, total_h, 0.0), p(0.0, total_h, h)], wall_stroke);
    painter.line_segment([p(wb, total_h, 0.0), p(wb, total_h, h)], wall_stroke);
    painter.line_segment([p(wa, la, 0.0), p(wa, la, h)], wall_stroke);
    if (wb - wa).abs() > 0.01 {
        painter.line_segment([p(wb, la, 0.0), p(wb, la, h)], wall_stroke);
    }

    // Top edges
    painter.line_segment([p(wa, 0.0, h), p(wa, la, h)], wall_stroke);
    painter.line_segment([p(wa, la, h), p(wb, la, h)], wall_stroke);
    painter.line_segment([p(wb, la, h), p(wb, total_h, h)], wall_stroke);
    painter.line_segment([p(0.0, total_h, h), p(wb, total_h, h)], wall_stroke);
}

/// Draw an isometric sphere.
fn draw_iso_sphere(
    painter: &egui::Painter,
    radius: f32,
    scale: f32,
    offset: egui::Pos2,
    fill_color: egui::Color32,
    wall_stroke: egui::Stroke,
    highlight_color: egui::Color32,
) {
    let center = iso_to_screen(radius, radius, radius, scale, offset);
    let screen_radius = radius * scale * 0.7; // approximate screen radius

    // Filled circle
    painter.circle_filled(center, screen_radius, fill_color);

    // Highlight crescent (upper-left lighter area)
    let highlight = egui::Color32::from_rgba_premultiplied(
        highlight_color.r(),
        highlight_color.g(),
        highlight_color.b(),
        30,
    );
    let hl_center = egui::pos2(
        center.x - screen_radius * 0.2,
        center.y - screen_radius * 0.2,
    );
    painter.circle_filled(hl_center, screen_radius * 0.6, highlight);

    // Outline
    painter.circle_stroke(center, screen_radius, wall_stroke);
}

/// Draw an isometric dome (half-sphere on a floor).
#[allow(clippy::too_many_arguments)]
fn draw_iso_dome(
    painter: &egui::Painter,
    radius: f32,
    scale: f32,
    offset: egui::Pos2,
    floor_color: egui::Color32,
    fill_color: egui::Color32,
    wall_stroke: egui::Stroke,
    edge_color: egui::Color32,
) {
    let diameter = radius * 2.0;

    // Floor ellipse
    draw_iso_ellipse(
        painter,
        radius,
        radius,
        0.0,
        radius,
        scale,
        offset,
        egui::Stroke::new(1.5, edge_color),
        48,
    );

    // Floor fill (approximation: convex polygon from ellipse points)
    let floor_pts: Vec<egui::Pos2> = (0..48)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32 / 48.0);
            iso_to_screen(
                radius + radius * angle.cos(),
                radius + radius * angle.sin(),
                0.0,
                scale,
                offset,
            )
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        floor_pts,
        floor_color,
        egui::Stroke::NONE,
    ));

    // Dome surface: draw upper half arc
    let center = iso_to_screen(radius, radius, 0.0, scale, offset);
    let screen_radius = radius * scale * 0.7;

    // Upper dome fill
    let dome_center = iso_to_screen(radius, radius, radius * 0.5, scale, offset);
    painter.circle_filled(dome_center, screen_radius * 0.8, fill_color);

    // Upper arc (solid)
    let n_segs = 32;
    for i in 0..n_segs {
        let a0 = std::f32::consts::PI + std::f32::consts::PI * (i as f32 / n_segs as f32);
        let a1 = std::f32::consts::PI + std::f32::consts::PI * ((i + 1) as f32 / n_segs as f32);
        let p0 = egui::pos2(
            center.x + screen_radius * a0.cos(),
            center.y + screen_radius * a0.sin(),
        );
        let p1 = egui::pos2(
            center.x + screen_radius * a1.cos(),
            center.y + screen_radius * a1.sin(),
        );
        painter.line_segment([p0, p1], wall_stroke);
    }
    // Lower arc (dashed)
    let dash_stroke = egui::Stroke::new(1.5, edge_color);
    for i in 0..n_segs {
        let a0 = std::f32::consts::PI * (i as f32 / n_segs as f32);
        let a1 = std::f32::consts::PI * ((i + 1) as f32 / n_segs as f32);
        let p0 = egui::pos2(
            center.x + screen_radius * a0.cos(),
            center.y + screen_radius * a0.sin(),
        );
        let p1 = egui::pos2(
            center.x + screen_radius * a1.cos(),
            center.y + screen_radius * a1.sin(),
        );
        if i % 2 == 0 {
            painter.line_segment([p0, p1], dash_stroke);
        }
    }

    // Dimension label
    let label_pos = iso_to_screen(radius, 0.0, 0.0, scale, offset);
    painter.text(
        egui::pos2(label_pos.x, label_pos.y + 14.0),
        egui::Align2::CENTER_TOP,
        format!("r={:.1}m ({:.1}m dia)", radius, diameter),
        egui::FontId::proportional(11.0),
        edge_color,
    );
}

/// Draw an isometric tube (open ends, dashed).
#[allow(clippy::too_many_arguments)]
fn draw_iso_tube(
    painter: &egui::Painter,
    length: f32,
    radius: f32,
    scale: f32,
    offset: egui::Pos2,
    floor_color: egui::Color32,
    wall_color_fill: egui::Color32,
    wall_stroke: egui::Stroke,
    edge_color: egui::Color32,
) {
    let diameter = radius * 2.0;
    let p = |x: f32, y: f32, z: f32| iso_to_screen(x, y, z, scale, offset);

    // Back wall approximation
    let n_segs = 32;
    let mut back_pts = Vec::with_capacity(n_segs + 2);
    for i in 0..=n_segs {
        let angle = std::f32::consts::PI * (i as f32 / n_segs as f32);
        let cy = radius + radius * angle.cos();
        let cz = radius * angle.sin();
        back_pts.push(iso_to_screen(length, cy, cz, scale, offset));
    }
    back_pts.push(p(length, diameter, 0.0));
    back_pts.push(p(length, 0.0, 0.0));
    painter.add(egui::Shape::convex_polygon(
        back_pts,
        wall_color_fill,
        egui::Stroke::NONE,
    ));

    // Floor
    painter.add(egui::Shape::convex_polygon(
        vec![
            p(0.0, 0.0, 0.0),
            p(length, 0.0, 0.0),
            p(length, diameter, 0.0),
            p(0.0, diameter, 0.0),
        ],
        floor_color,
        egui::Stroke::NONE,
    ));

    // Long walls (solid)
    painter.line_segment([p(0.0, 0.0, 0.0), p(length, 0.0, 0.0)], wall_stroke);
    painter.line_segment(
        [p(0.0, diameter, 0.0), p(length, diameter, 0.0)],
        wall_stroke,
    );

    // Top ridge
    painter.line_segment(
        [p(0.0, radius, diameter), p(length, radius, diameter)],
        wall_stroke,
    );

    // End cross-sections (dashed ellipses)
    let dash_stroke = egui::Stroke::new(1.5, edge_color);
    // Front end (x=0) — dashed
    let front_pts: Vec<egui::Pos2> = (0..48)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32 / 48.0);
            iso_to_screen(
                0.0,
                radius + radius * angle.cos(),
                radius + radius * angle.sin(),
                scale,
                offset,
            )
        })
        .collect();
    for i in 0..48 {
        if i % 2 == 0 {
            painter.line_segment([front_pts[i], front_pts[(i + 1) % 48]], dash_stroke);
        }
    }
    // Back end (x=length) — dashed
    let back_end_pts: Vec<egui::Pos2> = (0..48)
        .map(|i| {
            let angle = std::f32::consts::TAU * (i as f32 / 48.0);
            iso_to_screen(
                length,
                radius + radius * angle.cos(),
                radius + radius * angle.sin(),
                scale,
                offset,
            )
        })
        .collect();
    for i in 0..48 {
        if i % 2 == 0 {
            painter.line_segment([back_end_pts[i], back_end_pts[(i + 1) % 48]], dash_stroke);
        }
    }
}

/// Draw dimension labels along the visible isometric floor edges.
fn draw_iso_dimension_labels(
    painter: &egui::Painter,
    state: &AweUiState,
    room_l: f32,
    room_w: f32,
    scale: f32,
    offset: egui::Pos2,
    color: egui::Color32,
) {
    let font = egui::FontId::proportional(12.0);

    match state.shape_kind {
        RoomShapeKind::Box => {
            // Length along front edge (y=0): midpoint of (0,0,0)-(L,0,0)
            let mid_front = iso_to_screen(room_l * 0.5, 0.0, 0.0, scale, offset);
            painter.text(
                egui::pos2(mid_front.x, mid_front.y + 14.0),
                egui::Align2::CENTER_TOP,
                format!("{room_l:.1}m"),
                font.clone(),
                color,
            );
            // Width along left edge (x=0): midpoint of (0,0,0)-(0,W,0)
            let mid_left = iso_to_screen(0.0, room_w * 0.5, 0.0, scale, offset);
            painter.text(
                egui::pos2(mid_left.x - 14.0, mid_left.y),
                egui::Align2::RIGHT_CENTER,
                format!("{room_w:.1}m"),
                font.clone(),
                color,
            );
            // Height along front-right vertical: midpoint of (L,0,0)-(L,0,H)
            let h = state.room_height;
            let mid_vert = iso_to_screen(room_l, 0.0, h * 0.5, scale, offset);
            painter.text(
                egui::pos2(mid_vert.x + 14.0, mid_vert.y),
                egui::Align2::LEFT_CENTER,
                format!("{h:.1}m"),
                font,
                color,
            );
        }
        RoomShapeKind::Cylinder => {
            let mid = iso_to_screen(state.cyl_length * 0.5, 0.0, 0.0, scale, offset);
            painter.text(
                egui::pos2(mid.x, mid.y + 14.0),
                egui::Align2::CENTER_TOP,
                format!("{:.1}m", state.cyl_length),
                font.clone(),
                color,
            );
            let side = iso_to_screen(0.0, state.cyl_radius, 0.0, scale, offset);
            painter.text(
                egui::pos2(side.x - 14.0, side.y),
                egui::Align2::RIGHT_CENTER,
                format!("r={:.1}m", state.cyl_radius),
                font,
                color,
            );
        }
        RoomShapeKind::LShape => {
            let mid = iso_to_screen(room_l * 0.25, 0.0, 0.0, scale, offset);
            painter.text(
                egui::pos2(mid.x, mid.y + 14.0),
                egui::Align2::CENTER_TOP,
                format!(
                    "A:{:.0}x{:.0} B:{:.0}x{:.0}",
                    state.l_length_a, state.l_width_a, state.l_length_b, state.l_width_b
                ),
                egui::FontId::proportional(11.0),
                color,
            );
        }
        RoomShapeKind::Sphere => {
            let bottom = iso_to_screen(state.sphere_radius, 0.0, 0.0, scale, offset);
            painter.text(
                egui::pos2(bottom.x, bottom.y + 14.0),
                egui::Align2::CENTER_TOP,
                format!("r={:.1}m", state.sphere_radius),
                font,
                color,
            );
        }
        RoomShapeKind::Dome => {
            // Handled inside draw_iso_dome
        }
        RoomShapeKind::Tube => {
            let mid = iso_to_screen(state.tube_length * 0.5, 0.0, 0.0, scale, offset);
            painter.text(
                egui::pos2(mid.x, mid.y + 14.0),
                egui::Align2::CENTER_TOP,
                format!("{:.1}m", state.tube_length),
                font.clone(),
                color,
            );
            let side = iso_to_screen(0.0, state.tube_radius, 0.0, scale, offset);
            painter.text(
                egui::pos2(side.x - 14.0, side.y),
                egui::Align2::RIGHT_CENTER,
                format!("r={:.1}m", state.tube_radius),
                font,
                color,
            );
        }
    }
}

/// Draw the right-side control panel.
#[allow(clippy::too_many_lines)]
fn draw_controls(ui: &mut egui::Ui, handle: &mut EngineHandle, state: &mut AweUiState) {
    let t = theme();

    ui.label(egui::RichText::new("Description").color(t.colors.text_dim))
        .on_hover_text(
            "Free-text description of the acoustic character. Saved with AWE presets \
            and read back via MCP `get_awe_state`.",
        );
    let desc_resp = ui.add(
        egui::TextEdit::multiline(&mut state.description)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    state.description_edit_in_progress = desc_resp.has_focus();

    ui.add_space(t.spacing.md);

    // --- Room Shape & Dimensions ---
    ui.heading(
        egui::RichText::new("Room")
            .color(t.colors.accent_cyan)
            .size(16.0),
    );
    ui.add_space(t.spacing.xs);

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
    ui.add_space(t.spacing.xs);

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
            .on_hover_text("How much the walls scatter sound (smooth vs rough surface)")
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
    ui.label(
        egui::RichText::new("Dry/wet signal balance")
            .size(10.0)
            .color(t.colors.text_dim),
    );
    ui.add_space(t.spacing.xs);

    ui.horizontal(|ui| {
        ui.label("Dry/Wet:");
        if ui
            .add(egui::Slider::new(&mut state.dry_wet, 0.0..=1.0))
            .on_hover_text("Balance between original signal (dry) and room effect (wet)")
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
            .on_hover_text("Balance between early reflections and reverb tail")
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
            .on_hover_text("Standing waves in the room \u{2014} room resonances")
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
            .on_hover_text("Extend or shorten the reverb tail")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::TailStretch(StretchFactor::new(state.tail_stretch)),
            });
        }
    });

    ui.separator();

    // --- Effects (formerly "Impossible") ---
    ui.heading(
        egui::RichText::new("Effects")
            .color(t.colors.accent_orange)
            .size(16.0),
    );
    ui.label(
        egui::RichText::new("Effects beyond physics")
            .size(10.0)
            .color(t.colors.text_dim),
    );
    ui.add_space(t.spacing.xs);

    ui.horizontal(|ui| {
        ui.label("Freq Warp:");
        if ui
            .add(egui::Slider::new(&mut state.freq_warp, -1.0..=1.0))
            .on_hover_text("Shift room resonance frequencies (not physically realistic)")
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
            .on_hover_text("Boost room resonance intensity")
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
            .on_hover_text("Simulates sound leaking in from adjacent rooms")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::PortalAmount(NormalizedValue::new(state.portal_amount)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Pre-delay:");
        if ui
            .add(egui::Slider::new(&mut state.pre_delay, 0.0..=200.0).suffix(" ms"))
            .on_hover_text("Delay before first reflection — separates dry signal from reverb")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::PreDelay(Milliseconds::new(state.pre_delay)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Mod Depth:");
        if ui
            .add(egui::Slider::new(&mut state.modulation_depth, 0.0..=1.0))
            .on_hover_text("FDN chorus depth \u{2014} breaks metallic reverb artifacts")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ModulationDepth(NormalizedValue::new(state.modulation_depth)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Mod Rate:");
        if ui
            .add(egui::Slider::new(&mut state.modulation_rate, 0.1..=10.0).suffix(" Hz"))
            .on_hover_text("FDN chorus rate")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::ModulationRate(Hertz::new(state.modulation_rate)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Air Absorb:");
        if ui
            .add(egui::Slider::new(&mut state.air_absorption, 0.0..=1.0))
            .on_hover_text("Distance-proportional high-frequency damping")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::AirAbsorption(NormalizedValue::new(state.air_absorption)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Width:");
        if ui
            .add(egui::Slider::new(&mut state.width, 0.0..=1.0))
            .on_hover_text("Stereo width (0 = mono, 1 = full stereo)")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::Width(NormalizedValue::new(state.width)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("High Cut:");
        if ui
            .add(
                egui::Slider::new(&mut state.high_cut, 200.0..=20000.0)
                    .logarithmic(true)
                    .suffix(" Hz"),
            )
            .on_hover_text("Low-pass filter on wet signal")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::HighCut(Hertz::new(state.high_cut)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Low Cut:");
        if ui
            .add(
                egui::Slider::new(&mut state.low_cut, 20.0..=2000.0)
                    .logarithmic(true)
                    .suffix(" Hz"),
            )
            .on_hover_text("High-pass filter on wet signal")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::LowCut(Hertz::new(state.low_cut)),
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Temperature:");
        if ui
            .add(egui::Slider::new(&mut state.temperature, -40.0..=60.0).suffix(" \u{00b0}C"))
            .on_hover_text("Room temperature \u{2014} affects speed of sound")
            .changed()
        {
            handle.send(EngineCommand::SetAweParameter {
                param: AweParam::Temperature(synth_awe::Celsius::new(state.temperature)),
            });
        }
    });

    ui.separator();

    // --- Spatial ---
    draw_spatial_section(ui, handle, state);

    ui.separator();

    // --- LFO sections (collapsible, default closed) ---
    draw_lfo_section(ui, handle, "LFO 1", &mut state.lfo1, 1);
    draw_lfo_section(ui, handle, "LFO 2", &mut state.lfo2, 2);
    draw_lfo_section(ui, handle, "LFO 3", &mut state.lfo3, 3);
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
    ui.add_space(t.spacing.xs);

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

/// Draw a single LFO section as a collapsible header.
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
    egui::CollapsingHeader::new(
        egui::RichText::new(label)
            .color(t.colors.accent_purple)
            .size(14.0),
    )
    .default_open(false)
    .show(ui, |ui| {
        ui.add_space(t.spacing.xxs);

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
    });
}
