//! Chord Bloom — Radial bursts triggered by note clusters (chords).
//!
//! When multiple notes are struck simultaneously, an expanding, glowing geometric pattern
//! (bloom) expands outward.

use bevy::color::LinearRgba;
use bevy::prelude::*;
use std::f32::consts::PI;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

const MAX_BLOOMS: usize = 10;
const BURST_LIFETIME: f32 = 1.5;
const MAX_RADIUS: f32 = 40.0;
const EMISSIVE_STRENGTH: f32 = 8.0;

#[derive(Component)]
pub struct BloomRing {
    pub life: f32,
    pub max_life: f32,
    #[allow(dead_code)]
    pub velocity: f32,
    pub segments: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct ChordState {
    last_note_frame: u32,
    notes_this_frame: usize,
}

pub fn setup() {
    // Blooms spawned dynamically
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_and_update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<ChordState>,
    mut query: Query<(Entity, &mut BloomRing)>,
    mut segment_query: Query<(&mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let is_active = effect_state.active.is_active(EffectId::ChordBloom);
    let fade = effect_state.fade;

    if !is_active && fade == 0.0 {
        return;
    }

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

        let hue = ((telemetry.last_note_on.unwrap_or((60, 0, 0, 0)).0 as f32) / 127.0) * 360.0;

        // Spawn a ring of cubes
        let segment_count = 12;
        let mesh = meshes.add(Cuboid::new(1.0, 0.2, 4.0));
        let color = Color::hsl(hue, 0.9, 0.6);
        let material = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
            ..default()
        });

        let mut segments = Vec::new();
        for i in 0..segment_count {
            let angle = (i as f32 / segment_count as f32) * PI * 2.0;

            let seg_ent = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(-angle)),
                    Visibility::Hidden,
                    EffectLayer(EffectId::ChordBloom),
                ))
                .id();
            segments.push(seg_ent);
        }

        commands.spawn(BloomRing {
            life: BURST_LIFETIME,
            max_life: BURST_LIFETIME,
            velocity: MAX_RADIUS / BURST_LIFETIME,
            segments,
        });
    }

    let fade = effect_state.fade;

    // 2. Update expanding blooms
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
        let segment_count = ring.segments.len();

        for (i, seg_ent) in ring.segments.iter().enumerate() {
            if let Ok((mut transform, mat_handle)) = segment_query.get_mut(*seg_ent) {
                let angle = (i as f32 / segment_count as f32) * PI * 2.0;
                let x = angle.cos() * radius;
                let z = angle.sin() * radius;

                transform.translation = Vec3::new(x, 0.5, z);
                transform.scale = Vec3::splat(life_pct);

                if fade < 1.0 {
                    if let Some(material) = materials.get_mut(&mat_handle.0) {
                        let mut base: Hsla = material.base_color.into();
                        base.lightness = 0.6 * life_pct * fade;
                        material.base_color = base.into();
                        material.emissive =
                            LinearRgba::from(base) * EMISSIVE_STRENGTH * life_pct * fade;
                    }
                } else if let Some(material) = materials.get_mut(&mat_handle.0) {
                    let mut base: Hsla = material.base_color.into();
                    base.lightness = 0.6 * life_pct;
                    material.base_color = base.into();
                    material.emissive = LinearRgba::from(base) * EMISSIVE_STRENGTH * life_pct;
                }
            }
        }
    }
}
