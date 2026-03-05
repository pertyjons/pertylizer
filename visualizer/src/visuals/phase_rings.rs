//! Phase Rings — Concentric rings expanding with the beat phase.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
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
    /// Which beat index this ring was spawned on.
    #[allow(dead_code)]
    pub beat_index: i32,
    /// The exact beat time it was spawned.
    pub spawned_at_beat: f32,
}

/// Cached mesh handle for the ring (torus).
#[derive(Resource)]
pub struct RingMesh(Handle<Mesh>);

/// Tracks when to spawn new rings.
#[derive(Resource, Default)]
pub struct RingState {
    last_beat_index: i32,
}

pub fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // A Torus looks like a neon ring
    commands.insert_resource(RingMesh(meshes.add(Torus::new(1.0, RING_THICKNESS))));
}

pub fn spawn_and_update(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    ring_mesh: Res<RingMesh>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<RingState>,
    mut query: Query<(
        Entity,
        &PhaseRing,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
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

            // Downbeat (beat 0 of a bar) gets a different color
            let is_downbeat = current_beat_index.rem_euclid(4) == 0;
            let hue = if is_downbeat { 200.0 } else { 320.0 }; // Blue vs Pink

            let color = Color::hsl(hue, 0.9, 0.5);
            let material = materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
                alpha_mode: AlphaMode::Blend,
                ..default()
            });

            commands.spawn((
                Mesh3d(ring_mesh.0.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 0.5, 0.0).with_scale(Vec3::ZERO), // Start invisible
                PhaseRing {
                    beat_index: current_beat_index,
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

    let fade = effect_state.fade;

    // Update existing rings based on how many beats have passed since they spawned
    for (entity, ring, mut transform, material_handle) in &mut query {
        let beats_passed = telemetry.beat_position - ring.spawned_at_beat;

        // Despawn if it has expanded past our tracked limit
        if beats_passed >= MAX_RINGS as f32 {
            commands.entity(entity).despawn();
            materials.remove(&material_handle.0);
            continue;
        }

        // Scale linearly with beats passed
        let radius = beats_passed * (MAX_RADIUS / MAX_RINGS as f32);
        transform.scale = Vec3::new(radius, 1.0, radius);

        // Fade out as it expands
        let life_pct = 1.0 - (beats_passed / MAX_RINGS as f32);

        if let Some(material) = materials.get_mut(&material_handle.0) {
            let mut base: Hsla = material.base_color.into();
            base.lightness = 0.5 * life_pct * fade;
            material.base_color = base.into();
            material.emissive = LinearRgba::from(base) * EMISSIVE_STRENGTH * life_pct * fade;
        }
    }
}
