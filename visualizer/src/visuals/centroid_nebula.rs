//! Centroid Nebula — Particle cloud shifting color and shape based on spectral brightness.

use super::rand_f32;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Number of particles in the nebula (reduced from 2000 for performance).
const NEBULA_PARTICLES: usize = 500;

/// Base bounds of the particle cloud.
const BOUNDS: f32 = 15.0;

/// Emissive multiplier.
const EMISSIVE_STRENGTH: f32 = 1.5;

/// Per-second centroid smoothing rate (symmetric, time-based).
const CENTROID_SMOOTH_RATE: f32 = 4.0;

/// Number of shared material buckets.
const NUM_MATERIAL_BUCKETS: usize = 16;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 60.0, // Spread hue slightly for a colorful nebula
    saturation: 0.8,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: false,
};

/// Shared materials bucketed by hue.
#[derive(Resource)]
pub struct NebulaMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// A single particle in the nebula.
#[derive(Component)]
pub struct NebulaParticle {
    /// Base (original) position.
    base_pos: Vec3,
    /// Random offset phase for noise/movement.
    phase: f32,
    /// Base size.
    base_size: f32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Sphere::new(0.15));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(NebulaMaterials {
        materials: shared_mats.clone(),
    });

    let mut rng = fastrand::Rng::new();

    for _ in 0..NEBULA_PARTICLES {
        let x = rand_f32(&mut rng, -BOUNDS..BOUNDS);
        let y = rand_f32(&mut rng, -BOUNDS..BOUNDS) + 5.0;
        let z = rand_f32(&mut rng, -BOUNDS..BOUNDS) - 5.0;
        let pos = Vec3::new(x, y, z);

        let phase = rand_f32(&mut rng, 0.0..std::f32::consts::TAU);
        let base_size = rand_f32(&mut rng, 0.5..1.5);

        // Assign material bucket based on spatial position to create color clusters
        let bucket = ((x + y + z).abs() as usize) % NUM_MATERIAL_BUCKETS;

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
            Transform::from_translation(pos).with_scale(Vec3::splat(base_size)),
            Visibility::Hidden,
            NebulaParticle {
                base_pos: pos,
                phase,
                base_size,
            },
            EffectLayer(EffectId::CentroidNebula),
        ));
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&NebulaParticle, &mut Transform)>,
    mut last_centroid: Local<f32>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Smooth the centroid value so it doesn't jitter violently (time-based)
    // Typical centroid is 500Hz - 8000Hz
    let raw_centroid = telemetry.centroid_hz.clamp(200.0, 10000.0);
    let alpha = 1.0 - (-CENTROID_SMOOTH_RATE * dt).exp();
    let centroid = if *last_centroid == 0.0 {
        raw_centroid
    } else {
        *last_centroid + (raw_centroid - *last_centroid) * alpha
    };
    *last_centroid = centroid;

    // Normalize centroid to a 0.0 - 1.0 range (logarithmic is better for frequency)
    let centroid_norm = ((centroid.log2() - 200.0_f32.log2())
        / (10000.0_f32.log2() - 200.0_f32.log2()))
    .clamp(0.0, 1.0);

    // RMS controls overall energy/size, voice count adds density
    let voice_factor = 1.0 + (telemetry.voice_count as f32 / 32.0).clamp(0.0, 1.0) * 0.5;
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 5.0 * voice_factor).clamp(0.1, 2.5);

    for (particle, mut transform) in &mut query {
        // High centroid = fast turbulent movement. Low centroid = slow rolling
        let speed = 0.5 + centroid_norm * 3.0;
        let p = particle.phase + t * speed;

        // Expansion based on energy
        let spread = 1.0 + energy * 0.5;

        // Simple pseudo-noise displacement using trig functions
        let offset = Vec3::new(
            (p * 1.3).sin() * 2.0 * centroid_norm,
            (p * 0.8).cos() * 2.0 * centroid_norm,
            (p * 1.1).sin() * 2.0 * centroid_norm,
        );

        // Scale transforms by fade so particles shrink/grow during crossfade
        transform.translation = (particle.base_pos * spread + offset) * fade;
        transform.scale = Vec3::splat(particle.base_size * energy * fade);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    time: Res<Time>,
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    nebula_materials: Res<NebulaMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_centroid: Local<f32>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let fade = effect_state.fade;
    let dt = time.delta_secs();

    let raw_centroid = telemetry.centroid_hz.clamp(200.0, 10000.0);
    let alpha = 1.0 - (-CENTROID_SMOOTH_RATE * dt).exp();
    let centroid = if *last_centroid == 0.0 {
        raw_centroid
    } else {
        *last_centroid + (raw_centroid - *last_centroid) * alpha
    };
    *last_centroid = centroid;

    let hue_offset = telemetry_color::centroid_to_hue(centroid, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);

    effects::update_hue_materials_for_fade(
        &mut materials,
        &nebula_materials.materials,
        &MAT_CONFIG,
        &policy,
        fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
