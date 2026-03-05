//! Phase Rings — Concentric rings expanding with the beat phase.
//!
//! Uses shared materials instead of creating one per ring to avoid draw call bloat.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

/// Maximum radius a ring expands to before disappearing.
const MAX_RADIUS: f32 = 40.0;

/// Base thickness of the rings.
const RING_THICKNESS: f32 = 0.5;

/// Number of rings to keep alive (one per beat).
const MAX_RINGS: usize = 8;

/// Emissive intensity multiplier.
const EMISSIVE_STRENGTH: f32 = 12.0;

/// A single expanding ring.
#[derive(Component)]
pub struct PhaseRing {
    /// The exact beat time it was spawned.
    pub spawned_at_beat: f32,
}

/// Cached mesh handle for the ring (torus).
#[derive(Resource)]
pub struct RingMesh(Handle<Mesh>);

/// Shared materials — downbeat and offbeat, to allow batching.
#[derive(Resource)]
pub struct RingMaterials {
    downbeat: Handle<StandardMaterial>,
    offbeat: Handle<StandardMaterial>,
}

/// Tracks when to spawn new rings.
#[derive(Resource, Default)]
pub struct RingState {
    last_beat_index: i32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A Torus looks like a neon ring
    commands.insert_resource(RingMesh(meshes.add(Torus::new(1.0, RING_THICKNESS))));

    let downbeat_color = Color::hsl(200.0, 0.9, 0.5);
    let offbeat_color = Color::hsl(320.0, 0.9, 0.5);

    let downbeat = materials.add(StandardMaterial {
        base_color: downbeat_color,
        emissive: LinearRgba::from(downbeat_color) * EMISSIVE_STRENGTH,
        ..default()
    });
    let offbeat = materials.add(StandardMaterial {
        base_color: offbeat_color,
        emissive: LinearRgba::from(offbeat_color) * EMISSIVE_STRENGTH,
        ..default()
    });

    commands.insert_resource(RingMaterials { downbeat, offbeat });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_and_update(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    ring_mesh: Res<RingMesh>,
    ring_materials: Res<RingMaterials>,
    mut state: ResMut<RingState>,
    mut query: Query<(Entity, &PhaseRing, &mut Transform)>,
    mut last_fade: Local<f32>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let is_active = effect_state.active.is_active(EffectId::PhaseRings);
    let fade = effect_state.fade;

    if !is_active && fade == 0.0 {
        return;
    }

    if telemetry.playing {
        let current_beat_index = telemetry.beat_position.floor() as i32;

        // ONLY spawn new rings if this effect is actually the active terrain!
        if is_active && current_beat_index > state.last_beat_index {
            state.last_beat_index = current_beat_index;

            let material = if current_beat_index.rem_euclid(4) == 0 {
                ring_materials.downbeat.clone()
            } else {
                ring_materials.offbeat.clone()
            };

            commands.spawn((
                Mesh3d(ring_mesh.0.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 0.5, 0.0).with_scale(Vec3::ZERO), // Start invisible
                PhaseRing {
                    spawned_at_beat: telemetry.beat_position,
                },
                EffectLayer(EffectId::PhaseRings),
            ));
        } else if current_beat_index > state.last_beat_index {
            // Keep state updated even when inactive so it doesn't spawn old beats when activated
            state.last_beat_index = current_beat_index;
        }
    } else {
        state.last_beat_index = telemetry.beat_position.floor() as i32;
    }

    // Update existing rings — fade via scale, no per-entity material mutation
    for (entity, ring, mut transform) in &mut query {
        let beats_passed = telemetry.beat_position - ring.spawned_at_beat;

        // Despawn if it has expanded past our tracked limit
        if beats_passed >= MAX_RINGS as f32 {
            commands.entity(entity).despawn();
            continue;
        }

        // Scale linearly with beats passed
        let radius = beats_passed * (MAX_RADIUS / MAX_RINGS as f32);
        let life_pct = 1.0 - (beats_passed / MAX_RINGS as f32);

        // Use scale to encode both radius and fade-out (Y scale shrinks as it fades)
        transform.scale = Vec3::new(radius, life_pct * fade, radius);
    }

    // Only update the 2 shared materials during crossfade
    if (fade - *last_fade).abs() > effects::FADE_EPSILON {
        for (handle, hue) in [
            (&ring_materials.downbeat, 200.0),
            (&ring_materials.offbeat, 320.0),
        ] {
            if let Some(material) = materials.get_mut(handle) {
                let color = Color::hsl(hue, 0.9, 0.5 * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
            }
        }
        *last_fade = fade;
    }
}
