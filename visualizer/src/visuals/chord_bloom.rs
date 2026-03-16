//! Chord Bloom — Radial bursts triggered by note clusters (chords).
//!
//! Uses pre-allocated mesh and shared materials to avoid per-bloom allocations.

use bevy::prelude::*;
use std::f32::consts::PI;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::{NoteOnEvent, SynthTelemetry};

const MAX_BLOOMS: usize = 10;
const BURST_LIFETIME: f32 = 1.5;
const MAX_RADIUS: f32 = 40.0;
const EMISSIVE_STRENGTH: f32 = 3.0;
const SEGMENTS_PER_BLOOM: usize = 12;
/// Number of shared material buckets.
const NUM_MATERIAL_BUCKETS: usize = 8;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0,
    saturation: 0.9,
    lightness: 0.6,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: false,
};

#[derive(Component)]
pub struct BloomRing {
    pub life: f32,
    pub max_life: f32,
    pub segments: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct ChordState {
    last_note_frame: u32,
    notes_this_frame: usize,
}

/// Pre-allocated mesh for bloom segments.
#[derive(Resource)]
pub struct BloomMesh(Handle<Mesh>);

/// Shared materials bucketed by hue.
#[derive(Resource)]
pub struct BloomMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    commands.insert_resource(BloomMesh(meshes.add(Cuboid::new(1.0, 0.2, 4.0))));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(BloomMaterials {
        materials: shared_mats,
    });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_and_update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<ChordState>,
    mut query: Query<(Entity, &mut BloomRing)>,
    mut segment_query: Query<&mut Transform>,
    bloom_mesh: Res<BloomMesh>,
    bloom_materials: Res<BloomMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let is_active = effect_state.active.is_active(EffectId::ChordBloom);
    let fade = effect_state.fade;

    let dt = time.delta_secs();

    // 1. Detect chords (multiple notes in the same telemetry frame)
    if telemetry.note_age_frames == 0 {
        if state.last_note_frame != telemetry.seq as u32 {
            state.notes_this_frame = 1;
            state.last_note_frame = telemetry.seq as u32;
        } else {
            state.notes_this_frame += 1;
        }
    } else {
        state.notes_this_frame = 0;
    }

    // Trigger a bloom if 3 or more notes hit at once
    if is_active && state.notes_this_frame >= 3 && query.iter().count() < MAX_BLOOMS {
        // Reset so we only spawn one bloom per chord frame
        state.notes_this_frame = 0;

        let note = telemetry
            .last_note_on
            .unwrap_or(NoteOnEvent {
                midi_note: 60,
                velocity: 0,
                instrument_id: 0,
                category: synth_osc_protocol::InstrumentCategory::default(),
            })
            .midi_note;
        let mat_idx = (note as usize * NUM_MATERIAL_BUCKETS / 128).min(NUM_MATERIAL_BUCKETS - 1);
        let material = bloom_materials.materials[mat_idx].clone();

        let mut segments = Vec::with_capacity(SEGMENTS_PER_BLOOM);
        for i in 0..SEGMENTS_PER_BLOOM {
            let angle = (i as f32 / SEGMENTS_PER_BLOOM as f32) * PI * 2.0;

            let seg_ent = commands
                .spawn((
                    Mesh3d(bloom_mesh.0.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(-angle)),
                    Visibility::Inherited,
                    EffectLayer(EffectId::ChordBloom),
                ))
                .id();
            segments.push(seg_ent);
        }

        commands.spawn((
            BloomRing {
                life: BURST_LIFETIME,
                max_life: BURST_LIFETIME,
                segments,
            },
            EffectLayer(EffectId::ChordBloom),
        ));
    }

    // 2. Update expanding blooms — transform only, no per-entity material mutation
    for (entity, mut ring) in &mut query {
        ring.life -= dt;

        if ring.life <= 0.0 {
            for seg in &ring.segments {
                commands.entity(*seg).despawn();
            }
            commands.entity(entity).despawn();
            continue;
        }

        let life_pct = ring.life / ring.max_life;
        let radius = (1.0 - life_pct) * MAX_RADIUS;

        for (i, seg_ent) in ring.segments.iter().enumerate() {
            if let Ok(mut transform) = segment_query.get_mut(*seg_ent) {
                let angle = (i as f32 / ring.segments.len() as f32) * PI * 2.0;
                let x = angle.cos() * radius;
                let z = angle.sin() * radius;

                transform.translation = Vec3::new(x, 0.5, z);
                // Encode both life decay and fade into scale
                transform.scale = Vec3::splat(life_pct * fade);
            }
        }
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    effects::update_hue_materials_for_fade(
        &mut materials,
        &bloom_materials.materials,
        &MAT_CONFIG,
        &policy,
        fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
