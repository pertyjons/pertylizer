//! Pre-defined AWE presets demonstrating various room configurations.

use crate::params::{AweSnapshot, AweState};
use crate::room::{Material, RoomShape};
use crate::spatial_voice::NotePositionMapping;

/// A named AWE preset with description and full state.
pub struct AwePreset {
    /// Display name.
    pub name: &'static str,
    /// Short description of the acoustic character.
    pub description: &'static str,
    /// Full AWE state to apply.
    pub state: AweState,
}

/// Returns all built-in AWE presets.
#[must_use]
pub fn awe_presets() -> Vec<AwePreset> {
    vec![
        // 1. Katedral
        AwePreset {
            name: "Katedral",
            description: "Stor, majestätisk katedral med lång reverb",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 40.0,
                    width: 25.0,
                    height: 20.0,
                },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.7,
                    modes_amount: 0.4,
                    freq_warp: 0.0,
                    resonance_boost: 0.1,
                    tail_stretch: 2.5,
                    portal_amount: 0.0,
                    source_pos: [10.0, 12.5, 10.0],
                    listener_pos: [30.0, 12.5, 10.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 2. Badrum
        AwePreset {
            name: "Badrum",
            description: "Kort, ljust rum med tydliga rumsmoder",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 3.0,
                    width: 2.0,
                    height: 2.5,
                },
                material: Material::TILE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.4,
                    early_late_balance: 0.4,
                    modes_amount: 0.7,
                    freq_warp: 0.0,
                    resonance_boost: 0.2,
                    tail_stretch: 1.0,
                    portal_amount: 0.0,
                    source_pos: [1.0, 1.0, 1.2],
                    listener_pos: [2.0, 1.0, 1.2],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 3. Grotta
        AwePreset {
            name: "Grotta",
            description: "Mörk, omslutande sfärisk grotta",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 8.0 },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.6,
                    early_late_balance: 0.6,
                    modes_amount: 0.5,
                    freq_warp: -0.3,
                    resonance_boost: 0.3,
                    tail_stretch: 2.0,
                    portal_amount: 0.0,
                    source_pos: [6.0, 8.0, 8.0],
                    listener_pos: [10.0, 8.0, 8.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 4. Pipeline
        AwePreset {
            name: "Pipeline",
            description: "Metalliskt rör med flutterekos",
            state: AweState {
                enabled: true,
                room: RoomShape::Tube {
                    radius: 1.0,
                    length: 80.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.8,
                    freq_warp: 0.0,
                    resonance_boost: 0.4,
                    tail_stretch: 1.5,
                    portal_amount: 0.0,
                    source_pos: [10.0, 1.0, 1.0],
                    listener_pos: [70.0, 1.0, 1.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 5. Konserthall
        AwePreset {
            name: "Konserthall",
            description: "Varm, balanserad konserthall i trä",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 30.0,
                    width: 20.0,
                    height: 12.0,
                },
                material: Material::WOOD,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.4,
                    early_late_balance: 0.5,
                    modes_amount: 0.3,
                    freq_warp: 0.0,
                    resonance_boost: 0.0,
                    tail_stretch: 1.5,
                    portal_amount: 0.0,
                    source_pos: [5.0, 10.0, 6.0],
                    listener_pos: [20.0, 10.0, 6.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 6. Sci-Fi Korridor
        AwePreset {
            name: "Sci-Fi Korridor",
            description: "Kall metallisk L-formad korridor med portal",
            state: AweState {
                enabled: true,
                room: RoomShape::LShape {
                    length_a: 12.0,
                    width_a: 3.0,
                    length_b: 8.0,
                    width_b: 3.0,
                    height: 3.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.5,
                    freq_warp: -0.5,
                    resonance_boost: 0.3,
                    tail_stretch: 1.2,
                    portal_amount: 0.6,
                    source_pos: [3.0, 1.5, 1.5],
                    listener_pos: [15.0, 1.5, 1.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 7. Dröm
        AwePreset {
            name: "Dröm",
            description: "Shimmer-reverb med frekvensförskjutning",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 15.0,
                    width: 10.0,
                    height: 5.0,
                },
                material: Material::FABRIC,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.6,
                    early_late_balance: 0.7,
                    modes_amount: 0.3,
                    freq_warp: 0.7,
                    resonance_boost: 0.5,
                    tail_stretch: 3.0,
                    portal_amount: 0.0,
                    source_pos: [4.0, 5.0, 2.5],
                    listener_pos: [11.0, 5.0, 2.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 8. Underjorden
        AwePreset {
            name: "Underjorden",
            description: "Mörk tunnel med starka moder och portal",
            state: AweState {
                enabled: true,
                room: RoomShape::Cylinder {
                    radius: 3.0,
                    length: 40.0,
                },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.7,
                    freq_warp: -0.2,
                    resonance_boost: 0.3,
                    tail_stretch: 1.8,
                    portal_amount: 0.4,
                    source_pos: [5.0, 3.0, 3.0],
                    listener_pos: [30.0, 3.0, 3.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 9. Industrihall
        AwePreset {
            name: "Industrihall",
            description: "Stor metallisk hall med resonansboost",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 50.0,
                    width: 30.0,
                    height: 8.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.5,
                    freq_warp: 0.0,
                    resonance_boost: 0.5,
                    tail_stretch: 1.5,
                    portal_amount: 0.0,
                    source_pos: [10.0, 15.0, 4.0],
                    listener_pos: [35.0, 15.0, 4.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 10. Liten Studio
        AwePreset {
            name: "Liten Studio",
            description: "Kontrollerat, torrt rum i trä",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 5.0,
                    width: 4.0,
                    height: 3.0,
                },
                material: Material::WOOD,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.2,
                    early_late_balance: 0.3,
                    modes_amount: 0.2,
                    freq_warp: 0.0,
                    resonance_boost: 0.0,
                    tail_stretch: 0.8,
                    portal_amount: 0.0,
                    source_pos: [1.5, 2.0, 1.5],
                    listener_pos: [3.5, 2.0, 1.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 11. Rymdstation
        AwePreset {
            name: "Rymdstation",
            description: "Sci-fi sfär med per-röst spatialisering",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 4.0 },
                material: Material::METAL,
                spatial_enabled: true,
                note_mapping: NotePositionMapping::LinearX,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.6,
                    freq_warp: 0.4,
                    resonance_boost: 0.3,
                    tail_stretch: 1.5,
                    portal_amount: 0.2,
                    source_pos: [3.0, 4.0, 4.0],
                    listener_pos: [5.0, 4.0, 4.0],
                    spatial_enabled: true,
                    note_mapping: NotePositionMapping::LinearX,
                    ..AweSnapshot::default()
                },
            },
        },
        // 12. Bergseko
        AwePreset {
            name: "Bergseko",
            description: "Extremt långa ekon i bergspassage",
            state: AweState {
                enabled: true,
                room: RoomShape::Tube {
                    radius: 5.0,
                    length: 150.0,
                },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.7,
                    modes_amount: 0.4,
                    freq_warp: 0.0,
                    resonance_boost: 0.2,
                    tail_stretch: 3.5,
                    portal_amount: 0.3,
                    source_pos: [20.0, 5.0, 5.0],
                    listener_pos: [130.0, 5.0, 5.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 13. Kupol
        AwePreset {
            name: "Kupol",
            description: "Fokuserad overhead-reflektion i glaskupol",
            state: AweState {
                enabled: true,
                room: RoomShape::Dome { radius: 12.0 },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.5,
                    freq_warp: 0.2,
                    resonance_boost: 0.2,
                    tail_stretch: 1.2,
                    portal_amount: 0.0,
                    source_pos: [8.0, 12.0, 6.0],
                    listener_pos: [16.0, 12.0, 6.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 14. Portal
        AwePreset {
            name: "Portal",
            description: "Maxad portal-effekt i glasformat L-rum",
            state: AweState {
                enabled: true,
                room: RoomShape::LShape {
                    length_a: 10.0,
                    width_a: 6.0,
                    length_b: 8.0,
                    width_b: 5.0,
                    height: 4.0,
                },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.4,
                    freq_warp: 0.3,
                    resonance_boost: 0.2,
                    tail_stretch: 1.5,
                    portal_amount: 0.8,
                    source_pos: [3.0, 3.0, 2.0],
                    listener_pos: [14.0, 3.0, 2.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_count() {
        assert_eq!(awe_presets().len(), 14);
    }

    #[test]
    fn test_presets_have_names() {
        for preset in &awe_presets() {
            assert!(!preset.name.is_empty());
            assert!(!preset.description.is_empty());
        }
    }

    #[test]
    fn test_presets_enabled() {
        for preset in &awe_presets() {
            assert!(preset.state.enabled);
        }
    }

    #[test]
    fn test_presets_valid_dry_wet() {
        for preset in &awe_presets() {
            let dw = preset.state.snapshot.dry_wet;
            assert!(
                (0.0..=1.0).contains(&dw),
                "Preset '{}' dry_wet out of range: {dw}",
                preset.name
            );
        }
    }
}
