//! Ferrofluid Tendrils — Magnetic tendrils rising from bass frequencies.
//!
//! Visual idea: Vertical pillars that group together and stretch up smoothly
//! based on lower frequency bands, resembling ferrofluid on a magnet.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const NUM_TENDRILS: usize = 30;
const RADIUS: f32 = 8.0;
const MAX_HEIGHT: f32 = 12.0;
const EMISSIVE_STRENGTH: f32 = 6.0;

#[derive(Component)]
pub struct Tendril {
    pub angle: f32,
    pub band_index: usize,
    pub current_height: f32,
}

#[derive(Resource)]
pub struct FerrofluidMaterial(Handle<StandardMaterial>);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Tall cylinder
    let mesh = meshes.add(Cylinder::new(0.6, 1.0));

    // Very dark, glossy material
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.02, 0.02, 0.05),
        emissive: LinearRgba::from(Color::srgb(0.1, 0.2, 0.5)) * EMISSIVE_STRENGTH,
        perceptual_roughness: 0.1, // Shiny
        metallic: 0.9,             // Metallic
        ..default()
    });

    commands.insert_resource(FerrofluidMaterial(material.clone()));

    for i in 0..NUM_TENDRILS {
        let angle = (i as f32 / NUM_TENDRILS as f32) * std::f32::consts::TAU;

        // Map to lower bands, maybe repeat a bit so it wraps
        let band_index = (i * 2) % 32;

        let x = angle.cos() * RADIUS;
        let z = angle.sin() * RADIUS;

        // Add some noise to the radius so it's not a perfect circle
        let r_offset = (i as f32 * 2.4).sin() * 2.0;
        let pos = Vec3::new(x + r_offset, 0.0, z + r_offset);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(pos).with_scale(Vec3::new(1.0, 0.01, 1.0)),
            Visibility::Hidden,
            Tendril {
                angle,
                band_index,
                current_height: 0.01,
            },
            EffectLayer(EffectId::FerrofluidTendrils),
        ));
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Tendril, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let fade = effect_state.fade;

    for (mut tendril, mut transform) in &mut query {
        let mag = telemetry.fft[tendril.band_index];

        // Target height
        let target = (mag * MAX_HEIGHT).max(0.1);

        // Smooth lerp for liquid feel
        let speed = if target > tendril.current_height {
            5.0
        } else {
            2.0
        };
        tendril.current_height += (target - tendril.current_height) * speed * dt;

        // Scale Y and keep base on the ground
        let height = tendril.current_height * fade;
        transform.scale.y = height.max(0.01);
        transform.translation.y = transform.scale.y * 0.5;

        // Lean slightly inward towards center
        let lean = (tendril.current_height / MAX_HEIGHT) * 0.5;
        transform.rotation = Quat::from_axis_angle(
            Vec3::new(-tendril.angle.sin(), 0.0, tendril.angle.cos()),
            lean * fade,
        );
    }
}

pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    material_res: Res<FerrofluidMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_energy: Local<f32>,
    mut last_fade: Local<f32>,
    mut last_policy_version: Local<u64>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;

    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 3.0).clamp(0.1, 1.0);

    // Only update material when energy or fade changes meaningfully
    if (energy - *last_energy).abs() < 0.02
        && (fade - *last_fade).abs() < effects::FADE_EPSILON
        && *last_policy_version == policy.version
    {
        return;
    }

    if let Some(material) = materials.get_mut(&material_res.0) {
        let color = Color::srgb(0.1, 0.2 + energy * 0.5, 0.5 + energy * 0.5);
        material.emissive =
            LinearRgba::from(color) * EMISSIVE_STRENGTH * policy.emissive_multiplier * fade;
        material.metallic = policy.metallic;
        material.perceptual_roughness = policy.roughness;
    }

    *last_energy = energy;
    *last_fade = fade;
    *last_policy_version = policy.version;
}
