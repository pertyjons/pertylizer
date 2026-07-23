//! Beat Fracture — Neon-edged translucent shards that appear when the air "cracks like glass".
//!
//! On high flux+rms, semi-transparent shard particles burst into existence and tumble
//! downward briefly (0.3 s lifetime). CPU-based particle effect using `StandardMaterial`.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer};
use super::rand_f32;
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Maximum number of live shards at any time.
const MAX_SHARDS: u32 = 128;

/// Shard lifetime in seconds.
const LIFETIME: f32 = 0.3;

/// Number of shards spawned per burst.
const SHARDS_PER_BURST: u32 = 8;

/// Combined flux + rms_mono threshold to trigger a burst.
const FLUX_RMS_THRESHOLD: f32 = 0.5;

/// Emissive multiplier for the neon edge glow.
const EMISSIVE_STRENGTH: f32 = 8.0;

/// Gravity applied to shard velocity (units/s²).
const GRAVITY: f32 = 15.0;

/// A falling, tumbling fracture shard.
#[derive(Component)]
pub struct FractureShard {
    pub velocity: Vec3,
    pub life: f32,
    pub spin: Vec3,
}

/// Cached mesh and material for shard spawning.
#[derive(Resource)]
pub struct FractureResources {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// Tracks live shard count to enforce the cap.
#[derive(Resource, Default)]
pub struct ShardCount(pub u32);

/// Create the shared shard mesh and semi-transparent neon material.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(0.8, 0.02, 0.5));

    let hue = policy.flux_burst_hue;
    let base_color = Color::hsla(hue, 0.9, 0.7, 0.3);
    let emissive_color = Color::hsl(hue, 0.9, 0.7);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let material = materials.add(StandardMaterial {
        base_color,
        emissive: LinearRgba::from(emissive_color) * emissive,
        alpha_mode: AlphaMode::Blend,
        metallic: policy.metallic,
        perceptual_roughness: policy.roughness,
        ..default()
    });

    commands.insert_resource(FractureResources { mesh, material });
    commands.init_resource::<ShardCount>();
}

/// Spawn a burst of shards when flux + rms exceeds the threshold.
pub fn spawn(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    resources: Res<FractureResources>,
    mut shard_count: ResMut<ShardCount>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    if telemetry.flux + rms_mono <= FLUX_RMS_THRESHOLD {
        return;
    }
    if shard_count.0 >= MAX_SHARDS {
        return;
    }

    let mut rng = fastrand::Rng::new();

    let to_spawn = SHARDS_PER_BURST.min(MAX_SHARDS - shard_count.0);
    for _ in 0..to_spawn {
        let x = rand_f32(&mut rng, -15.0..15.0);
        let y = rand_f32(&mut rng, 5.0..15.0);
        let z = rand_f32(&mut rng, -10.0..10.0);

        let velocity = Vec3::new(
            rand_f32(&mut rng, -2.0..2.0),
            rand_f32(&mut rng, -8.0..-2.0),
            rand_f32(&mut rng, -2.0..2.0),
        );

        let spin = Vec3::new(
            rand_f32(&mut rng, -10.0..10.0),
            rand_f32(&mut rng, -10.0..10.0),
            rand_f32(&mut rng, -10.0..10.0),
        );

        commands.spawn((
            Mesh3d(resources.mesh.clone()),
            MeshMaterial3d(resources.material.clone()),
            Transform::from_xyz(x, y, z),
            FractureShard {
                velocity,
                life: LIFETIME,
                spin,
            },
            EffectLayer(EffectId::BeatFracture),
        ));
        shard_count.0 += 1;
    }
}

/// Update shards: apply gravity, move, spin, shrink, and despawn expired ones.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    resources: Res<FractureResources>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
    mut query: Query<(Entity, &mut FractureShard, &mut Transform)>,
    mut shard_count: ResMut<ShardCount>,
) {
    let dt = time.delta_secs();

    // Update shared material when theme changes
    if *last_policy_version != policy.version {
        if let Some(mut material) = materials.get_mut(&resources.material) {
            let hue = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
            let base_color = Color::hsla(hue, 0.9, 0.7, 0.3);
            let emissive_color = Color::hsl(hue, 0.9, 0.7);
            let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

            material.base_color = base_color;
            material.emissive = LinearRgba::from(emissive_color) * emissive;
            material.metallic = policy.metallic;
            material.perceptual_roughness = policy.roughness;
        }
        *last_policy_version = policy.version;
    }

    let mut alive = 0u32;
    for (entity, mut shard, mut transform) in &mut query {
        // Apply gravity
        shard.velocity.y -= GRAVITY * dt;

        // Update position
        transform.translation += shard.velocity * dt;

        // Apply spin rotation (tumbling)
        transform.rotate_local_x(shard.spin.x * dt);
        transform.rotate_local_y(shard.spin.y * dt);
        transform.rotate_local_z(shard.spin.z * dt);

        // Decrease life
        shard.life -= dt;

        if shard.life <= 0.0 {
            commands.entity(entity).despawn();
        } else {
            // Scale shrinks with remaining life
            let life_frac = (shard.life / LIFETIME).max(0.01);
            transform.scale = Vec3::splat(life_frac);
            alive += 1;
        }
    }

    // Resync shard count with actual alive count
    shard_count.0 = alive;
}
