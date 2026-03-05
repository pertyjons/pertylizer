//! Harmonic Ribbons — Ribbons that track pitch and glide.
//!
//! Visual idea: Sine wave-like trails that spawn based on note events and their lengths are
//! driven by note hold (although we don't have note-off yet, we can simulate a decaying trail).

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

const MAX_RIBBONS: usize = 32;
const SEGMENTS_PER_RIBBON: usize = 40;
const RIBBON_LIFETIME: f32 = 3.0;
const SPEED: f32 = 10.0;
const EMISSIVE_STRENGTH: f32 = 10.0;

#[derive(Component)]
pub struct Ribbon {
    pub hue: f32,
    pub life: f32,
    pub age: f32,
    pub base_y: f32,
    pub velocity: f32,
    pub segments: Vec<Entity>,
}

pub fn setup() {
    // Ribbons will be dynamically spawned on note-on
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_and_update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(Entity, &mut Ribbon)>,
    mut segment_query: Query<(&mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let is_active = effect_state.active.is_active(EffectId::HarmonicRibbons);
    let fade = effect_state.fade;

    if !is_active && fade == 0.0 {
        return;
    }

    let dt = time.delta_secs();

    // 1. Spawn new ribbons on note on
    if is_active
        && telemetry.note_age_frames < 2
        && let Some((note, velocity, _instrument_id, _category)) = telemetry.last_note_on
    {
        // Count active ribbons to enforce cap
        let active_count = query.iter().count();

        if active_count < MAX_RIBBONS {
            let hue = (note as f32 / 127.0) * 360.0;
            let vel_norm = velocity as f32 / 127.0;

            // Map pitch to Y axis
            let y = ((note as f32 / 127.0) * 20.0) - 5.0;

            let mesh = meshes.add(Cuboid::new(0.5, 0.2, 2.0));
            let color = Color::hsl(hue, 0.9, 0.5);
            let material = materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
                ..default()
            });

            let mut segments = Vec::with_capacity(SEGMENTS_PER_RIBBON);
            for _ in 0..SEGMENTS_PER_RIBBON {
                let seg_ent = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(0.0, -100.0, 0.0).with_scale(Vec3::ZERO),
                        Visibility::Hidden,
                        EffectLayer(EffectId::HarmonicRibbons),
                    ))
                    .id();
                segments.push(seg_ent);
            }

            commands.spawn(Ribbon {
                hue,
                life: RIBBON_LIFETIME * vel_norm, // Quieter notes die faster
                age: 0.0,
                base_y: y,
                velocity: SPEED + vel_norm * 5.0,
                segments,
            });
        }
    }

    let fade = effect_state.fade;

    // 2. Update existing ribbons
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

            // Wavy motion based on age and index
            let wave = (seg_x * 0.5 + ribbon.age * 5.0).sin() * 2.0;
            let z = (seg_x * 0.3).cos() * 5.0;

            if let Ok((mut transform, mat_handle)) = segment_query.get_mut(*seg_ent) {
                transform.translation = Vec3::new(seg_x, ribbon.base_y + wave, z);

                // Tail tapers off in thickness
                let tail_scale = (1.0 - (i_f / total_f)).max(0.01);
                // Head scales down as it dies
                let scale_y = tail_scale * life_pct;
                transform.scale = Vec3::new(1.0, scale_y, tail_scale);

                // Look at previous segment for smooth curve
                if seg_x > -20.0 && scale_y > 0.01 {
                    // Since we do procedural translation, rotation can be kept simple or derived
                    // We just rotate along X axis slightly to give a ribbon feel
                    transform.rotation = Quat::from_rotation_x(wave * 0.2);
                }

                // Update material for crossfade
                if fade < 1.0
                    && let Some(material) = materials.get_mut(&mat_handle.0)
                {
                    let color = Color::hsl(ribbon.hue, 0.9, 0.5 * fade * life_pct);
                    material.base_color = color;
                    material.emissive =
                        LinearRgba::from(color) * EMISSIVE_STRENGTH * fade * life_pct;
                }
            }
        }
    }
}
