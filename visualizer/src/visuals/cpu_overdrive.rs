//! CPU Overdrive Core — Glowing core that spins and fractures under load.
//!
//! Visualizes system strain via CPU Usage and Voice Count.
//! High CPU causes it to spin erratically, scale up, and shift from cool blue to angry red.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

/// The base rotation speed when CPU is 0%.
const BASE_SPEED: f32 = 0.5;

/// Multiplier for how much CPU adds to rotation speed.
const CPU_SPEED_MULT: f32 = 10.0;

/// Base scale of the core.
const BASE_SCALE: f32 = 4.0;

/// Base emissive strength.
const BASE_EMISSIVE: f32 = 4.0;

#[derive(Component)]
pub struct CpuCore;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A Torus Knot gives a complex "core" look
    let mesh = meshes.add(Torus::new(1.5, 0.6));

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.5, 1.0),
        emissive: LinearRgba::from(Color::srgb(0.1, 0.5, 1.0)) * BASE_EMISSIVE,
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 8.0, 0.0).with_scale(Vec3::splat(BASE_SCALE)),
        Visibility::Hidden,
        CpuCore,
        EffectLayer(EffectId::CpuOverdriveCore),
    ));
}

#[derive(Resource, Default)]
pub struct CpuState {
    last_strain: f32,
    last_fade: f32,
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<CpuState>,
    mut query: Query<(&mut Transform, &MeshMaterial3d<StandardMaterial>), With<CpuCore>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !effect_state.active.is_active(EffectId::CpuOverdriveCore) && effect_state.fade == 0.0 {
        return;
    }

    let dt = time.delta_secs();

    // CPU is 0.0 - 100.0, normalize to 0.0 - 1.0
    let cpu_norm = (telemetry.cpu / 100.0).clamp(0.0, 1.0);

    // Voice count adds an extra layer of "strain"
    let voice_strain = (telemetry.voice_count as f32 / 32.0).clamp(0.0, 1.0);

    // Combined strain factor
    let strain = (cpu_norm * 0.7 + voice_strain * 0.3).clamp(0.0, 1.0);

    let fade = effect_state.fade;
    let needs_material_update = (strain - state.last_strain).abs() > 0.005
        || (fade - state.last_fade).abs() > 0.005
        || fade < 1.0;

    for (mut transform, material_handle) in &mut query {
        // Spin speed scales up dramatically under load
        let speed = BASE_SPEED + (strain * CPU_SPEED_MULT);

        // Add erratic wobbling when strain is high
        let wobble = (time.elapsed_secs() * 20.0).sin() * strain * 0.2;

        transform.rotate_local_y(speed * dt);
        transform.rotate_local_x((speed * 0.5 + wobble) * dt);
        transform.rotate_local_z((speed * 0.3 + wobble) * dt);

        // Bulge and pulse
        let pulse = (time.elapsed_secs() * (2.0 + strain * 10.0)).sin() * 0.1 * strain;
        transform.scale = Vec3::splat(BASE_SCALE + strain * 2.0 + pulse);

        if needs_material_update && let Some(material) = materials.get_mut(&material_handle.0) {
            // Map 0.0->1.0 to 220.0->0.0
            let hue = 220.0 * (1.0 - strain);
            let lightness = 0.5 + (strain * 0.3); // Gets brighter when hot

            let color = Color::hsl(hue, 0.9, lightness * fade);
            let emissive_strength = BASE_EMISSIVE + (strain * 15.0);

            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive_strength * fade;
        }
    }

    state.last_strain = strain;
    state.last_fade = fade;
}
