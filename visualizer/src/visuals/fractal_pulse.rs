//! Fractal Pulse — Recursive rings synced to the beat.
//!
//! A set of concentric tori that scale and rotate. Based on Tempo and RMS.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Number of concentric rings.
const NUM_RINGS: usize = 6;

/// Base scale difference between rings.
const SCALE_STEP: f32 = 1.5;

/// Base emissive strength.
const EMISSIVE_STRENGTH: f32 = 3.0;

#[derive(Component)]
pub struct FractalRing {
    /// Ring index (0 is innermost).
    pub index: usize,
    /// Base rotation speed.
    pub base_speed: f32,
    /// Rotation axis.
    pub axis: Dir3,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Torus::new(1.0, 0.05));

    // Create materials for each ring
    let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let mut ring_materials = Vec::new();
    for i in 0..NUM_RINGS {
        let hue = (i as f32 / NUM_RINGS as f32) * 360.0;
        let color = Color::hsl(hue, sat, lit);
        ring_materials.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            ..default()
        }));
    }

    for (i, ring_mat) in ring_materials.iter().enumerate().take(NUM_RINGS) {
        let axis = if i % 2 == 0 { Dir3::X } else { Dir3::Z };
        let speed = if i % 2 == 0 { 0.5 } else { -0.5 } * (1.0 - (i as f32 * 0.1));

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(ring_mat.clone()),
            Transform::from_scale(Vec3::splat(1.0 + i as f32 * SCALE_STEP)),
            Visibility::Hidden,
            FractalRing {
                index: i,
                base_speed: speed,
                axis,
            },
            EffectLayer(EffectId::FractalPulse),
        ));
    }
}

#[derive(Resource, Default)]
pub struct FractalState {
    full_update_needed: bool,
    last_policy_version: u64,
    last_hue_offset: f32,
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<FractalState>,
    mut query: Query<(
        &FractalRing,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let dt = time.delta_secs();

    // Energy drives scale pulsing, voice count adds complexity
    let voice_boost = 1.0 + (telemetry.voice_count as f32 / 32.0).clamp(0.0, 1.0) * 0.5;
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 3.0 * voice_boost).clamp(0.1, 2.0);

    // Tempo drives rotation speed multiplier, transport stopped slows down
    let transport_scale = if telemetry.playing { 1.0 } else { 0.15 };
    let tempo_mult = (telemetry.tempo / 120.0).max(0.5) * transport_scale;

    let fade = effect_state.fade;

    if fade < 1.0 {
        state.full_update_needed = true;
    }
    if state.last_policy_version != policy.version {
        state.full_update_needed = true;
        state.last_policy_version = policy.version;
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let hue_changed = (hue_offset - state.last_hue_offset).abs() > 1.0;
    if hue_changed {
        state.last_hue_offset = hue_offset;
    }

    for (ring, mut transform, material_handle) in &mut query {
        // Rotate
        let speed = ring.base_speed * tempo_mult;
        transform.rotate_axis(ring.axis, speed * dt);

        // Always rotate slowly on Y as well for global movement
        transform.rotate_local_y(0.2 * tempo_mult * dt);

        // Pulse scale based on energy + beat phase, more pronounced on inner rings
        let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);
        let pulse_amount =
            (energy + beat_pulse * 0.5) * (1.0 - (ring.index as f32 / NUM_RINGS as f32) * 0.5);
        let base_scale = 1.0 + ring.index as f32 * SCALE_STEP;
        transform.scale = Vec3::splat(base_scale + pulse_amount * 2.0);

        if (fade < 1.0 || state.full_update_needed || hue_changed)
            && let Some(material) = materials.get_mut(&material_handle.0)
        {
            let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
            let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
            let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
            let metallic = policy.metallic;
            let roughness = policy.roughness;
            let hue = ((ring.index as f32 / NUM_RINGS as f32) * 360.0 + hue_offset) % 360.0;
            let color = Color::hsl(hue, sat, lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = metallic;
            material.perceptual_roughness = roughness;
        }
    }

    if fade > 0.999 {
        state.full_update_needed = false;
    }
}
