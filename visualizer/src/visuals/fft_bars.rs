//! FFT bar visualization — 128 cubes driven by frequency band magnitudes.
//!
//! Uses shared materials per hue-group to allow Bevy batching.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

/// Marker component with the FFT band index.
#[derive(Component)]
pub struct FftBar(pub usize);

/// Total width of the bar array.
const TOTAL_WIDTH: f32 = 30.0;

/// Spacing between bars.
const BAR_WIDTH: f32 = TOTAL_WIDTH / NUM_FFT_BANDS as f32;

/// Maximum bar height.
const MAX_HEIGHT: f32 = 8.0;

/// Emissive intensity multiplier for bloom visibility.
const EMISSIVE_STRENGTH: f32 = 5.0;

/// Number of shared material buckets (bars grouped by hue).
const NUM_MATERIAL_BUCKETS: usize = 16;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 270.0,
    saturation: 0.8,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
};

/// Shared materials per hue bucket.
#[derive(Resource)]
pub struct FftBarMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Spawn 128 cubes spread along the X axis.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(BAR_WIDTH * 0.85, 1.0, BAR_WIDTH * 0.85));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG);
    commands.insert_resource(FftBarMaterials {
        materials: shared_mats.clone(),
    });

    for i in 0..NUM_FFT_BANDS {
        let x = (i as f32 - NUM_FFT_BANDS as f32 / 2.0) * BAR_WIDTH;
        let bucket = (i * NUM_MATERIAL_BUCKETS / NUM_FFT_BANDS).min(NUM_MATERIAL_BUCKETS - 1);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
            Transform::from_xyz(x, 0.0, 0.0).with_scale(Vec3::new(1.0, 0.01, 1.0)),
            FftBar(i),
            Visibility::Inherited,
            EffectLayer(EffectId::FftBars),
        ));
    }
}

/// Update bar heights from FFT telemetry with smooth lerp.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &FftBar)>,
    fft_materials: Res<FftBarMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_fade: Local<f32>,
) {
    if !effect_state.active.is_active(EffectId::FftBars) {
        return;
    }

    for (mut transform, bar) in &mut query {
        let target_height = telemetry.fft[bar.0] * MAX_HEIGHT;
        let target_height = target_height.max(0.01); // Minimum visible height

        // Smooth lerp — fast attack, slow decay
        let current = transform.scale.y;
        let speed = if target_height > current { 0.4 } else { 0.1 };
        let new_height = current + (target_height - current) * speed;

        transform.scale.y = new_height;
        // Move bar up so base stays on the ground
        transform.translation.y = new_height / 2.0;
    }

    effects::update_hue_materials_for_fade(
        &mut materials,
        &fft_materials.materials,
        &MAT_CONFIG,
        effect_state.fade,
        &mut last_fade,
    );
}
