//! Harmonic Ribbons — Ribbons that track pitch and glide.
//!
//! Uses pre-allocated mesh and shared materials to avoid per-note allocations.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const MAX_RIBBONS: usize = 32;
const SEGMENTS_PER_RIBBON: usize = 40;
const RIBBON_LIFETIME: f32 = 3.0;
const SPEED: f32 = 10.0;
const EMISSIVE_STRENGTH: f32 = 4.0;
/// Number of shared material buckets.
const NUM_MATERIAL_BUCKETS: usize = 12;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0,
    saturation: 0.9,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: false,
};

#[derive(Component)]
pub struct Ribbon {
    pub life: f32,
    pub age: f32,
    pub base_y: f32,
    pub velocity: f32,
    pub segments: Vec<Entity>,
}

/// Pre-allocated mesh for ribbon segments.
#[derive(Resource)]
pub struct RibbonMesh(Handle<Mesh>);

/// Shared materials bucketed by hue.
#[derive(Resource)]
pub struct RibbonMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    commands.insert_resource(RibbonMesh(meshes.add(Cuboid::new(0.5, 0.2, 2.0))));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(RibbonMaterials {
        materials: shared_mats,
    });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_and_update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(Entity, &mut Ribbon)>,
    mut segment_query: Query<&mut Transform>,
    ribbon_mesh: Res<RibbonMesh>,
    ribbon_materials: Res<RibbonMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let is_active = effect_state.active.is_active(EffectId::HarmonicRibbons);
    let fade = effect_state.fade;

    let dt = time.delta_secs();

    // 1. Spawn new ribbons on note on
    if is_active
        && telemetry.note_age_frames < 2
        && let Some(note_event) = telemetry.last_note_on
    {
        // Count active ribbons to enforce cap
        let active_count = query.iter().count();

        if active_count < MAX_RIBBONS {
            let vel_norm = note_event.velocity as f32 / 127.0;

            // Map pitch to Y axis
            let y = ((note_event.midi_note as f32 / 127.0) * 20.0) - 5.0;

            // Pick shared material based on note + instrument category offset
            let cat_offset = telemetry_color::category_hue_offset(note_event.category);
            let shifted_note =
                ((note_event.midi_note as f32 + cat_offset * 128.0 / 360.0) as usize) % 128;
            let mat_idx = (shifted_note * NUM_MATERIAL_BUCKETS / 128).min(NUM_MATERIAL_BUCKETS - 1);
            let material = ribbon_materials.materials[mat_idx].clone();

            let mut segments = Vec::with_capacity(SEGMENTS_PER_RIBBON);
            for _ in 0..SEGMENTS_PER_RIBBON {
                let seg_ent = commands
                    .spawn((
                        Mesh3d(ribbon_mesh.0.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(0.0, -100.0, 0.0).with_scale(Vec3::ZERO),
                        Visibility::Inherited,
                        EffectLayer(EffectId::HarmonicRibbons),
                    ))
                    .id();
                segments.push(seg_ent);
            }

            commands.spawn((
                Ribbon {
                    life: RIBBON_LIFETIME * vel_norm, // Quieter notes die faster
                    age: 0.0,
                    base_y: y,
                    velocity: SPEED + vel_norm * 5.0,
                    segments,
                },
                EffectLayer(EffectId::HarmonicRibbons),
            ));
        }
    }

    // 2. Update existing ribbons — transform only, no material mutation per entity
    for (entity, mut ribbon) in &mut query {
        ribbon.age += dt;

        if ribbon.age >= ribbon.life {
            // Despawn ribbon and all segments
            for seg in &ribbon.segments {
                commands.entity(*seg).despawn();
            }
            commands.entity(entity).despawn();
            continue;
        }

        let life_pct = 1.0 - (ribbon.age / ribbon.life);
        let head_x = ribbon.age * ribbon.velocity - 15.0; // Start at left, move right

        for (i, seg_ent) in ribbon.segments.iter().enumerate() {
            let i_f = i as f32;
            let total_f = SEGMENTS_PER_RIBBON as f32;

            // Trail logic: segment 0 is the head, the rest trail behind in X
            let lag_x = i_f * 0.5;
            let seg_x = head_x - lag_x;

            // Wavy motion based on age and index, pitch bend widens the wave
            let bend_scale = 1.0 + telemetry.pitch_bend.abs() * 2.0;
            let wave = (seg_x * 0.5 + ribbon.age * 5.0).sin() * 2.0 * bend_scale;
            let z = (seg_x * 0.3).cos() * 5.0;

            if let Ok(mut transform) = segment_query.get_mut(*seg_ent) {
                transform.translation = Vec3::new(seg_x, ribbon.base_y + wave, z);

                // Tail tapers off in thickness
                let tail_scale = (1.0 - (i_f / total_f)).max(0.01);
                // Head scales down as it dies, encode fade into scale
                let scale_y = tail_scale * life_pct * fade;
                // Z-scale pulses drastically with pitch bend
                let z_pulse = 1.0 + telemetry.pitch_bend.abs() * 4.0;
                transform.scale = Vec3::new(1.0, scale_y, tail_scale * z_pulse);

                transform.rotation = Quat::from_rotation_x(wave * 0.2);
            }
        }
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    effects::update_hue_materials_for_fade(
        &mut materials,
        &ribbon_materials.materials,
        &MAT_CONFIG,
        &policy,
        fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
