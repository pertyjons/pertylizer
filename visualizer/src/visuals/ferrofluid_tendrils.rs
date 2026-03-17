//! Ferrofluid Tendrils — Magnetic tendrils rising from bass frequencies.
//!
//! Visual idea: Vertical pillars that group together and stretch up smoothly
//! based on lower frequency bands, resembling ferrofluid on a magnet.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const NUM_TENDRILS: usize = 30;
const RADIUS: f32 = 8.0;
const MAX_HEIGHT: f32 = 12.0;
const EMISSIVE_STRENGTH: f32 = 2.5;

#[derive(Component)]
pub struct Tendril {
    pub angle: f32,
    pub band_index: usize,
    pub current_height: f32,
}

const NUM_MATERIAL_BUCKETS: usize = 32;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0, // Overridden by frequency_mapped: true
    saturation: 0.7,
    lightness: 0.3,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: true,
};

#[derive(Resource)]
pub struct FerrofluidMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cylinder::new(0.6, 1.0));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(FerrofluidMaterials {
        materials: shared_mats.clone(),
    });

    for i in 0..NUM_TENDRILS {
        let angle = (i as f32 / NUM_TENDRILS as f32) * std::f32::consts::TAU;

        // Map to lower bands, maybe repeat a bit so it wraps
        let band_index = (i * 2) % 32;

        let x = angle.cos() * RADIUS;
        let z = angle.sin() * RADIUS;

        // Add some noise to the radius so it's not a perfect circle
        let r_offset = (i as f32 * 2.4).sin() * 2.0;
        let pos = Vec3::new(x + r_offset, 0.0, z + r_offset);

        // Assign material bucket based on the frequency band
        let bucket = band_index % NUM_MATERIAL_BUCKETS;

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
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

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    material_res: Res<FerrofluidMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let fade = effect_state.fade;
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);

    effects::update_hue_materials_for_fade(
        &mut materials,
        &material_res.materials,
        &MAT_CONFIG,
        &policy,
        fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
