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
                    length: 32.0,
                    width: 20.0,
                    height: 18.0,
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
                    tail_stretch: 2.8,
                    portal_amount: 0.0,
                    source_pos: [6.0, 10.0, 9.0],
                    listener_pos: [26.0, 10.0, 9.0],
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
                    radius: 1.2,
                    length: 45.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.85,
                    freq_warp: 0.0,
                    resonance_boost: 0.45,
                    tail_stretch: 1.8,
                    portal_amount: 0.0,
                    source_pos: [6.0, 1.2, 1.2],
                    listener_pos: [39.0, 1.2, 1.2],
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
                    length: 26.0,
                    width: 18.0,
                    height: 10.0,
                },
                material: Material::WOOD,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.55,
                    modes_amount: 0.25,
                    freq_warp: 0.0,
                    resonance_boost: 0.0,
                    tail_stretch: 1.6,
                    portal_amount: 0.0,
                    source_pos: [4.0, 9.0, 5.0],
                    listener_pos: [22.0, 9.0, 5.0],
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
                    radius: 2.8,
                    length: 32.0,
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
                    tail_stretch: 2.0,
                    portal_amount: 0.5,
                    source_pos: [4.0, 2.8, 2.8],
                    listener_pos: [26.0, 2.8, 2.8],
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
                    length: 36.0,
                    width: 24.0,
                    height: 8.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.55,
                    freq_warp: 0.0,
                    resonance_boost: 0.55,
                    tail_stretch: 1.7,
                    portal_amount: 0.0,
                    source_pos: [7.0, 12.0, 4.0],
                    listener_pos: [29.0, 12.0, 4.0],
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
                    radius: 4.0,
                    length: 50.0,
                },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.7,
                    modes_amount: 0.45,
                    freq_warp: 0.0,
                    resonance_boost: 0.2,
                    tail_stretch: 3.4,
                    portal_amount: 0.4,
                    source_pos: [8.0, 4.0, 4.0],
                    listener_pos: [42.0, 4.0, 4.0],
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
                room: RoomShape::Dome { radius: 10.0 },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.5,
                    modes_amount: 0.5,
                    freq_warp: 0.2,
                    resonance_boost: 0.2,
                    tail_stretch: 1.3,
                    portal_amount: 0.0,
                    source_pos: [6.0, 10.0, 5.0],
                    listener_pos: [14.0, 10.0, 5.0],
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
        // 15. Iskatedral
        AwePreset {
            name: "Iskatedral",
            description: "Glittrande isvalv med kristallklar svans",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 28.0,
                    width: 18.0,
                    height: 16.0,
                },
                material: Material::ICE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.55,
                    early_late_balance: 0.65,
                    modes_amount: 0.35,
                    freq_warp: -0.15,
                    resonance_boost: 0.2,
                    tail_stretch: 2.8,
                    portal_amount: 0.15,
                    source_pos: [6.0, 9.0, 8.0],
                    listener_pos: [22.0, 9.0, 8.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 16. Marmorgalleria
        AwePreset {
            name: "Marmorgalleria",
            description: "Lång marmorkorridor med glansiga reflexer",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 22.0,
                    width: 6.0,
                    height: 6.0,
                },
                material: Material::MARBLE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.45,
                    modes_amount: 0.25,
                    freq_warp: 0.1,
                    resonance_boost: 0.15,
                    tail_stretch: 1.6,
                    portal_amount: 0.0,
                    source_pos: [3.0, 3.0, 3.0],
                    listener_pos: [19.0, 3.0, 3.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 17. Neonatrium
        AwePreset {
            name: "Neonatrium",
            description: "Glaskupol med neonreflexer och portal-svep",
            state: AweState {
                enabled: true,
                room: RoomShape::Dome { radius: 9.0 },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.4,
                    freq_warp: 0.45,
                    resonance_boost: 0.25,
                    tail_stretch: 2.0,
                    portal_amount: 0.45,
                    source_pos: [5.0, 9.0, 4.5],
                    listener_pos: [13.0, 9.0, 4.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 18. Vattenreservoar
        AwePreset {
            name: "Vattenreservoar",
            description: "Fuktig cistern med mörk botten och lång svans",
            state: AweState {
                enabled: true,
                room: RoomShape::Cylinder {
                    radius: 6.0,
                    length: 35.0,
                },
                material: Material::WATER,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.55,
                    modes_amount: 0.7,
                    freq_warp: -0.1,
                    resonance_boost: 0.2,
                    tail_stretch: 2.2,
                    portal_amount: 0.3,
                    source_pos: [5.0, 6.0, 6.0],
                    listener_pos: [30.0, 6.0, 6.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 19. Dunrum
        AwePreset {
            name: "Dunrum",
            description: "Supertorrt rum med matta väggar och nära reflexer",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 7.0,
                    width: 5.0,
                    height: 3.0,
                },
                material: Material::CARPET,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.2,
                    early_late_balance: 0.2,
                    modes_amount: 0.1,
                    freq_warp: 0.0,
                    resonance_boost: 0.0,
                    tail_stretch: 0.7,
                    portal_amount: 0.0,
                    source_pos: [1.5, 2.5, 1.5],
                    listener_pos: [5.5, 2.5, 1.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 20. Glaslabyrint
        AwePreset {
            name: "Glaslabyrint",
            description: "Fladdrande reflexer i en glaskorridor-labyrint",
            state: AweState {
                enabled: true,
                room: RoomShape::LShape {
                    length_a: 14.0,
                    width_a: 4.0,
                    length_b: 10.0,
                    width_b: 4.0,
                    height: 3.5,
                },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.6,
                    freq_warp: 0.5,
                    resonance_boost: 0.25,
                    tail_stretch: 1.4,
                    portal_amount: 0.7,
                    source_pos: [2.0, 2.0, 1.7],
                    listener_pos: [20.0, 2.0, 1.7],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 21. Maskinrum
        AwePreset {
            name: "Maskinrum",
            description: "Metallisk resonans med starka rumsmoder",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 16.0,
                    width: 12.0,
                    height: 5.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.4,
                    modes_amount: 0.8,
                    freq_warp: -0.3,
                    resonance_boost: 0.5,
                    tail_stretch: 1.3,
                    portal_amount: 0.2,
                    source_pos: [3.0, 6.0, 2.5],
                    listener_pos: [13.0, 6.0, 2.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 22. Stjärnport
        AwePreset {
            name: "Stjärnport",
            description: "Rymdklot med per-röstspatialitet och portal",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 6.0 },
                material: Material::ICE,
                spatial_enabled: true,
                note_mapping: NotePositionMapping::Circular,
                snapshot: AweSnapshot {
                    dry_wet: 0.55,
                    early_late_balance: 0.65,
                    modes_amount: 0.5,
                    freq_warp: 0.2,
                    resonance_boost: 0.2,
                    tail_stretch: 2.2,
                    portal_amount: 0.8,
                    source_pos: [3.0, 6.0, 6.0],
                    listener_pos: [9.0, 6.0, 6.0],
                    spatial_enabled: true,
                    note_mapping: NotePositionMapping::Circular,
                    ..AweSnapshot::default()
                },
            },
        },
        // 23. Kvantkammare
        AwePreset {
            name: "Kvantkammare",
            description: "Glasig sfär med skiftande rymd och portalglimt",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 5.5 },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.6,
                    modes_amount: 0.35,
                    freq_warp: 0.7,
                    resonance_boost: 0.25,
                    tail_stretch: 1.9,
                    portal_amount: 0.7,
                    source_pos: [3.0, 5.5, 5.5],
                    listener_pos: [8.0, 5.5, 5.5],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 24. Basaltklyfta
        AwePreset {
            name: "Basaltklyfta",
            description: "Hög, smal klyfta med mörka rumsmode-svärmar",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 18.0,
                    width: 8.0,
                    height: 14.0,
                },
                material: Material::CONCRETE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.4,
                    early_late_balance: 0.45,
                    modes_amount: 0.85,
                    freq_warp: -0.5,
                    resonance_boost: 0.35,
                    tail_stretch: 1.8,
                    portal_amount: 0.1,
                    source_pos: [3.0, 4.0, 7.0],
                    listener_pos: [15.0, 4.0, 7.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 25. Aurorahall
        AwePreset {
            name: "Aurorahall",
            description: "Iskall sal med skimmrande svans och mjuka reflexer",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 24.0,
                    width: 16.0,
                    height: 12.0,
                },
                material: Material::ICE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.55,
                    early_late_balance: 0.7,
                    modes_amount: 0.4,
                    freq_warp: 0.2,
                    resonance_boost: 0.2,
                    tail_stretch: 2.6,
                    portal_amount: 0.2,
                    source_pos: [5.0, 8.0, 6.0],
                    listener_pos: [19.0, 8.0, 6.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 26. Gravitationstunnel
        AwePreset {
            name: "Gravitationstunnel",
            description: "Metalltunnel med tung basresonans och lång svans",
            state: AweState {
                enabled: true,
                room: RoomShape::Tube {
                    radius: 1.8,
                    length: 38.0,
                },
                material: Material::METAL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.65,
                    modes_amount: 0.9,
                    freq_warp: -0.4,
                    resonance_boost: 0.5,
                    tail_stretch: 2.2,
                    portal_amount: 0.3,
                    source_pos: [4.0, 1.8, 1.8],
                    listener_pos: [34.0, 1.8, 1.8],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 27. Regnrum
        AwePreset {
            name: "Regnrum",
            description: "Fuktigt, dämpat rum med korta, mjuka reflexer",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 12.0,
                    width: 9.0,
                    height: 4.0,
                },
                material: Material::WATER,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.35,
                    early_late_balance: 0.35,
                    modes_amount: 0.45,
                    freq_warp: 0.1,
                    resonance_boost: 0.15,
                    tail_stretch: 1.4,
                    portal_amount: 0.0,
                    source_pos: [2.5, 4.5, 2.0],
                    listener_pos: [9.5, 4.5, 2.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 28. Svävande Kör
        AwePreset {
            name: "Svävande Kör",
            description: "Eterisk sfär med per-röstspatialitet och mjuk svans",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 6.5 },
                material: Material::FABRIC,
                spatial_enabled: true,
                note_mapping: NotePositionMapping::Circular,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.25,
                    freq_warp: 0.3,
                    resonance_boost: 0.1,
                    tail_stretch: 2.1,
                    portal_amount: 0.25,
                    source_pos: [3.0, 6.5, 6.5],
                    listener_pos: [10.0, 6.5, 6.5],
                    spatial_enabled: true,
                    note_mapping: NotePositionMapping::Circular,
                    ..AweSnapshot::default()
                },
            },
        },
        // 29. Spegelplan
        AwePreset {
            name: "Spegelplan",
            description: "Glaslabyrint med fladdrande spegelportar",
            state: AweState {
                enabled: true,
                room: RoomShape::LShape {
                    length_a: 12.0,
                    width_a: 5.0,
                    length_b: 9.0,
                    width_b: 4.0,
                    height: 3.5,
                },
                material: Material::GLASS,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.55,
                    modes_amount: 0.55,
                    freq_warp: 0.6,
                    resonance_boost: 0.25,
                    tail_stretch: 1.6,
                    portal_amount: 0.9,
                    source_pos: [2.0, 2.5, 1.75],
                    listener_pos: [18.0, 2.5, 1.75],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 30. Kristallvalv
        AwePreset {
            name: "Kristallvalv",
            description: "Marmorkupol med skimrande tidiga reflexer",
            state: AweState {
                enabled: true,
                room: RoomShape::Dome { radius: 7.5 },
                material: Material::MARBLE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.45,
                    early_late_balance: 0.5,
                    modes_amount: 0.35,
                    freq_warp: 0.15,
                    resonance_boost: 0.2,
                    tail_stretch: 1.7,
                    portal_amount: 0.1,
                    source_pos: [4.0, 7.5, 3.75],
                    listener_pos: [11.0, 7.5, 3.75],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 31. Singularitet
        AwePreset {
            name: "EXT: Singularitet",
            description: "Fysiklös vakuumkammare med oändlig svans",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 7.0 },
                material: Material::VOID,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.7,
                    early_late_balance: 0.85,
                    modes_amount: 0.2,
                    freq_warp: -0.8,
                    resonance_boost: 0.6,
                    tail_stretch: 3.6,
                    portal_amount: 0.9,
                    source_pos: [3.0, 7.0, 7.0],
                    listener_pos: [11.0, 7.0, 7.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 32. Plasmastorm
        AwePreset {
            name: "EXT: Plasmastorm",
            description: "Het plasmahall med aggressiva färgskiftningar",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 18.0,
                    width: 12.0,
                    height: 8.0,
                },
                material: Material::PLASMA,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.5,
                    early_late_balance: 0.6,
                    modes_amount: 0.7,
                    freq_warp: 0.9,
                    resonance_boost: 0.45,
                    tail_stretch: 2.4,
                    portal_amount: 0.6,
                    source_pos: [4.0, 6.0, 4.0],
                    listener_pos: [14.0, 6.0, 4.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 33. Prismaspiral
        AwePreset {
            name: "EXT: Prismaspiral",
            description: "Prismatiskt valv med spektral rotation",
            state: AweState {
                enabled: true,
                room: RoomShape::Dome { radius: 8.5 },
                material: Material::PRISM,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.55,
                    early_late_balance: 0.55,
                    modes_amount: 0.35,
                    freq_warp: 0.8,
                    resonance_boost: 0.25,
                    tail_stretch: 2.0,
                    portal_amount: 0.4,
                    source_pos: [4.0, 8.5, 4.2],
                    listener_pos: [13.0, 8.5, 4.2],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 34. Membranhåla
        AwePreset {
            name: "EXT: Membranhåla",
            description: "Omvänd dämpning med pulserande basgrotta",
            state: AweState {
                enabled: true,
                room: RoomShape::Box {
                    length: 12.0,
                    width: 7.0,
                    height: 4.0,
                },
                material: Material::MEMBRANE,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.4,
                    early_late_balance: 0.5,
                    modes_amount: 0.9,
                    freq_warp: -0.7,
                    resonance_boost: 0.5,
                    tail_stretch: 1.6,
                    portal_amount: 0.2,
                    source_pos: [2.0, 3.5, 2.0],
                    listener_pos: [10.0, 3.5, 2.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 35. Nanodimma
        AwePreset {
            name: "EXT: Nanodimma",
            description: "Ultradämpad dimkammare med mjuka kanter",
            state: AweState {
                enabled: true,
                room: RoomShape::Cylinder {
                    radius: 5.0,
                    length: 20.0,
                },
                material: Material::NANOGEL,
                spatial_enabled: false,
                note_mapping: NotePositionMapping::Off,
                snapshot: AweSnapshot {
                    dry_wet: 0.3,
                    early_late_balance: 0.35,
                    modes_amount: 0.25,
                    freq_warp: 0.2,
                    resonance_boost: 0.1,
                    tail_stretch: 1.4,
                    portal_amount: 0.1,
                    source_pos: [3.0, 5.0, 5.0],
                    listener_pos: [17.0, 5.0, 5.0],
                    spatial_enabled: false,
                    note_mapping: NotePositionMapping::Off,
                    ..AweSnapshot::default()
                },
            },
        },
        // 36. Antigrav
        AwePreset {
            name: "EXT: Antigrav",
            description: "Fysiklös rymd med per-röstsväv och portal",
            state: AweState {
                enabled: true,
                room: RoomShape::Sphere { radius: 6.0 },
                material: Material::VOID,
                spatial_enabled: true,
                note_mapping: NotePositionMapping::Circular,
                snapshot: AweSnapshot {
                    dry_wet: 0.6,
                    early_late_balance: 0.7,
                    modes_amount: 0.3,
                    freq_warp: 0.6,
                    resonance_boost: 0.2,
                    tail_stretch: 2.6,
                    portal_amount: 0.8,
                    source_pos: [3.0, 6.0, 6.0],
                    listener_pos: [9.0, 6.0, 6.0],
                    spatial_enabled: true,
                    note_mapping: NotePositionMapping::Circular,
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
        assert_eq!(awe_presets().len(), 36);
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
