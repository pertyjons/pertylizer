//! Orbital Satellites — A ring of metallic spheres orbiting the center.
//!
//! On note-on events, satellites briefly shoot thin laser beams toward the center
//! and recoil outward.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Number of satellites in the orbit ring.
const NUM_SATELLITES: usize = 12;

/// Base orbit radius from center.
const ORBIT_RADIUS: f32 = 10.0;

/// Y position of the orbit ring.
const ORBIT_Y: f32 = 5.0;

/// Radius of each satellite sphere.
const SATELLITE_RADIUS: f32 = 0.4;

/// Emissive intensity multiplier.
const EMISSIVE_STRENGTH: f32 = 3.0;

/// Length of laser beams (slightly less than orbit radius so they reach toward center).
const LASER_LENGTH: f32 = 9.0;

/// How quickly recoil decays (exponential decay rate).
const RECOIL_DECAY: f32 = 5.0;

/// Recoil distance added on note trigger.
const RECOIL_IMPULSE: f32 = 3.0;

/// Laser beam lifetime in seconds.
const LASER_LIFETIME: f32 = 0.3;

/// An orbiting satellite sphere.
#[derive(Component)]
pub struct Satellite {
    /// Current angular position in radians.
    pub angle: f32,
    /// Angular speed in radians per second.
    pub orbit_speed: f32,
    /// Current recoil offset (decays exponentially).
    pub recoil: f32,
}

/// A laser beam paired to a satellite.
#[derive(Component)]
pub struct LaserBeam {
    /// Remaining life in seconds (fades and hides when <= 0).
    pub life: f32,
}

/// Shared materials for satellites and lasers.
#[derive(Resource)]
pub struct SatelliteMaterials {
    /// One material per satellite (unique hue each).
    pub satellite_materials: Vec<Handle<StandardMaterial>>,
    /// Shared material for all laser beams.
    pub laser_material: Handle<StandardMaterial>,
}


/// Spawn satellites and their paired laser beams.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let sphere_mesh = meshes.add(Sphere::new(SATELLITE_RADIUS));
    // Thin cylinder for laser beams (radius 0.03, half-height = LASER_LENGTH / 2)
    let cylinder_mesh = meshes.add(Cylinder::new(0.03, LASER_LENGTH));


    let sat_value = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let mut satellite_mats = Vec::with_capacity(NUM_SATELLITES);

    for i in 0..NUM_SATELLITES {
        #[allow(clippy::cast_precision_loss)]
        let hue = (i as f32 / NUM_SATELLITES as f32) * 360.0;
        let color = Color::hsl(hue, sat_value, lit);
        satellite_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            metallic: policy.metallic.max(0.7), // Always fairly metallic
            perceptual_roughness: policy.roughness.min(0.3),
            ..default()
        }));
    }

    // Bright white-ish laser material
    let laser_color = Color::hsl(200.0, 0.9, 0.8);
    let laser_material = materials.add(StandardMaterial {
        base_color: laser_color,
        emissive: LinearRgba::from(laser_color) * emissive * 2.0,
        ..default()
    });

    commands.insert_resource(SatelliteMaterials {
        satellite_materials: satellite_mats.clone(),
        laser_material: laser_material.clone(),
    });

    let mut rng = fastrand::Rng::new();

    for (i, sat_mat) in satellite_mats.iter().enumerate().take(NUM_SATELLITES) {
        #[allow(clippy::cast_precision_loss)]
        let angle = (i as f32 / NUM_SATELLITES as f32) * std::f32::consts::TAU;
        let orbit_speed = 0.3 + rng.f32() * 0.4; // 0.3 to 0.7 rad/s

        let x = angle.cos() * ORBIT_RADIUS;
        let z = angle.sin() * ORBIT_RADIUS;

        // Spawn satellite sphere
        commands.spawn((
            Mesh3d(sphere_mesh.clone()),
            MeshMaterial3d(sat_mat.clone()),
            Transform::from_xyz(x, ORBIT_Y, z),
            Visibility::Hidden,
            Satellite {
                angle,
                orbit_speed,
                recoil: 0.0,
            },
            EffectLayer(EffectId::OrbitalSatellites),
        ));

        // Spawn paired laser beam (hidden, will be shown on note-on)
        commands.spawn((
            Mesh3d(cylinder_mesh.clone()),
            MeshMaterial3d(laser_material.clone()),
            Transform::from_xyz(x, ORBIT_Y, z).with_scale(Vec3::new(1.0, 0.0, 1.0)),
            Visibility::Hidden,
            LaserBeam { life: 0.0 },
            EffectLayer(EffectId::OrbitalSatellites),
        ));
    }
}

/// Update satellite orbits, recoil, and laser beam lifetimes.
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    sat_materials: Res<SatelliteMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_fade: Local<f32>,
    mut last_hue_offset: Local<f32>,
    mut last_policy_version: Local<u64>,
    mut satellites: Query<(&mut Satellite, &mut Transform, &mut Visibility), Without<LaserBeam>>,
    mut lasers: Query<(&mut LaserBeam, &mut Transform, &mut Visibility), Without<Satellite>>,
) {
    let dt = time.delta_secs();
    let fade = effect_state.fade;
    let flux_mod = 1.0 + telemetry.flux * 0.5;

    // Beat pulse modulates Y slightly
    let beat_y_offset = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy) * 0.5;

    // Determine if a note-on just happened
    let note_triggered = telemetry.note_age_frames < 2;

    // Pick a random satellite index to trigger on note-on
    let mut rng = fastrand::Rng::new();
    let trigger_index = if note_triggered {
        Some(rng.usize(..NUM_SATELLITES))
    } else {
        None
    };

    // Update satellites
    for (idx, (mut sat, mut transform, _vis)) in satellites.iter_mut().enumerate() {
        // Rotate based on orbit speed, modulated by flux
        sat.angle += sat.orbit_speed * dt * flux_mod;
        if sat.angle > std::f32::consts::TAU {
            sat.angle -= std::f32::consts::TAU;
        }

        // Trigger recoil on the chosen satellite
        if trigger_index == Some(idx) {
            sat.recoil = RECOIL_IMPULSE;
        }

        // Decay recoil exponentially
        sat.recoil *= (-RECOIL_DECAY * dt).exp();
        if sat.recoil < 0.001 {
            sat.recoil = 0.0;
        }

        let radius = ORBIT_RADIUS + sat.recoil;
        let x = sat.angle.cos() * radius;
        let z = sat.angle.sin() * radius;
        let y = ORBIT_Y + beat_y_offset;

        transform.translation = Vec3::new(x, y, z);
        // Apply fade as scale
        transform.scale = Vec3::splat(fade);
    }

    // Update lasers (paired by index with satellites)
    let satellite_data: Vec<(f32, f32)> = satellites
        .iter()
        .map(|(sat, _, _)| (sat.angle, sat.recoil))
        .collect();

    for (idx, (mut laser, mut transform, mut vis)) in lasers.iter_mut().enumerate() {
        // Trigger laser on the same satellite that got recoil
        if trigger_index == Some(idx) {
            laser.life = LASER_LIFETIME;
            *vis = Visibility::Inherited;
        }

        if laser.life > 0.0 {
            laser.life -= dt;

            let life_frac = (laser.life / LASER_LIFETIME).max(0.0);

            if let Some(&(angle, recoil)) = satellite_data.get(idx) {
                let radius = ORBIT_RADIUS + recoil;
                let x = angle.cos() * radius;
                let z = angle.sin() * radius;
                let y = ORBIT_Y + beat_y_offset;

                // Position the laser halfway between satellite and center
                let half_x = x * 0.5;
                let half_z = z * 0.5;
                transform.translation = Vec3::new(half_x, y, half_z);

                // Orient toward center: the cylinder points along Y by default,
                // so we rotate it to lie along the satellite-to-center direction
                let dir = Vec3::new(-x, 0.0, -z).normalize_or_zero();
                if dir.length_squared() > 0.01 {
                    let rotation = Quat::from_rotation_arc(Vec3::Y, dir);
                    transform.rotation = rotation;
                }

                // Scale: Y is the length, X/Z are thickness, fade out with life
                transform.scale = Vec3::new(life_frac * fade, life_frac * fade, life_frac * fade);
            }

            if laser.life <= 0.0 {
                laser.life = 0.0;
                *vis = Visibility::Hidden;
            }
        }
    }

    // Update shared materials for crossfade, theme changes, and hue shifts
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    if (fade - *last_fade).abs() > effects::FADE_EPSILON
        || *last_policy_version != policy.version
        || (hue_offset - *last_hue_offset).abs() > 1.0
    {
        let sat_value = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
        let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier * flux_boost;

        for (i, handle) in sat_materials.satellite_materials.iter().enumerate() {
            if let Some(material) = materials.get_mut(handle) {
                #[allow(clippy::cast_precision_loss)]
                let hue = ((i as f32 / NUM_SATELLITES as f32) * 360.0 + hue_offset) % 360.0;
                let color = Color::hsl(hue, sat_value, lit * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive * fade;
                material.metallic = policy.metallic.max(0.7);
                material.perceptual_roughness = policy.roughness.min(0.3);
            }
        }

        // Update laser material
        if let Some(laser_mat) = materials.get_mut(&sat_materials.laser_material) {
            let laser_color = Color::hsl((200.0 + hue_offset) % 360.0, 0.9, 0.8 * fade);
            laser_mat.base_color = laser_color;
            laser_mat.emissive = LinearRgba::from(laser_color) * emissive * 2.0 * fade;
        }

        *last_fade = fade;
        *last_hue_offset = hue_offset;
        *last_policy_version = policy.version;
    }
}
