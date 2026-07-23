//! Spectral Cathedral — FFT bands form arches that breathe.
//!
//! Visual idea: Large arches forming a tunnel/cathedral structure.
//! The height/thickness of each arch segment is driven by FFT, and the whole
//! structure pulses with RMS.

use bevy::color::LinearRgba;
use bevy::prelude::*;
use std::f32::consts::PI;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// We use a subset of bands for the arches so it's not too cluttered.
const ARCH_BANDS: usize = 32;

/// Number of points making up each arch.
const ARCH_SEGMENTS: usize = 20;

/// Radius of the cathedral arches.
const ARCH_RADIUS: f32 = 15.0;

/// Spacing between arches along the Z axis.
const ARCH_SPACING: f32 = 2.0;

/// Emissive multiplier.
const EMISSIVE_STRENGTH: f32 = 2.5;

#[derive(Component)]
pub struct ArchSegment {
    /// Which frequency band this arch represents (0..ARCH_BANDS).
    pub band_index: usize,
    /// Base position without FFT displacement.
    pub base_pos: Vec3,
    /// Outward normal for displacement.
    pub normal: Vec3,
}

#[derive(Resource)]
pub struct CathedralMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(0.5, 0.5, 0.5));

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    // Shared materials, one per band
    let mut shared_mats = Vec::with_capacity(ARCH_BANDS);
    for band in 0..ARCH_BANDS {
        // Nonlinear frequency→hue: bass→red, mids→green, highs→blue
        let hue = telemetry_color::band_frequency_hue(band as f32 / ARCH_BANDS as f32);
        let color = Color::hsl(hue, sat, lit);
        shared_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            ..default()
        }));
    }
    commands.insert_resource(CathedralMaterials {
        materials: shared_mats.clone(),
    });

    // Build the arches
    for (band, shared_mat) in shared_mats.iter().enumerate().take(ARCH_BANDS) {
        // Lower frequencies closer to camera, high frequencies further back
        let z = -(band as f32) * ARCH_SPACING + (ARCH_BANDS as f32 * ARCH_SPACING * 0.3);

        for seg in 0..=ARCH_SEGMENTS {
            // Angle from 0 to PI (half circle)
            let angle = (seg as f32 / ARCH_SEGMENTS as f32) * PI;

            let x = angle.cos() * ARCH_RADIUS;
            let y = angle.sin() * ARCH_RADIUS;

            let pos = Vec3::new(x, y, z);
            // Normal points outward from center (0,0,z)
            let normal = Vec3::new(x, y, 0.0).normalize_or_zero();

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(shared_mat.clone()),
                Transform::from_translation(pos).looking_at(pos + normal, Vec3::Z),
                Visibility::Hidden,
                ArchSegment {
                    band_index: band,
                    base_pos: pos,
                    normal,
                },
                EffectLayer(EffectId::SpectralCathedral),
            ));
        }
    }
}

/// Downsample FFT bands to the number of arch bands.
fn downsample_fft(fft: &[f32], bin_count: usize) -> [f32; ARCH_BANDS] {
    let mut out = [0.0; ARCH_BANDS];
    if bin_count == 0 {
        return out;
    }
    let ratio = bin_count / ARCH_BANDS;
    if ratio == 0 {
        // Fewer bins than arch bands — map directly
        for (i, val) in out.iter_mut().enumerate() {
            let src = i * bin_count / ARCH_BANDS;
            *val = fft.get(src).copied().unwrap_or(0.0);
        }
    } else {
        for (i, val) in out.iter_mut().enumerate() {
            let start = i * ratio;
            let mut sum = 0.0;
            for j in 0..ratio {
                sum += fft.get(start + j).copied().unwrap_or(0.0);
            }
            *val = sum / ratio as f32;
        }
    }
    out
}

#[derive(Resource, Default)]
pub struct CathedralState {
    last_fade: f32,
    last_hue_offset: f32,
    last_policy_version: u64,
}

pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<CathedralState>,
    mut query: Query<(&ArchSegment, &mut Transform)>,
    cathedral_materials: Res<CathedralMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;

    let bin_count = telemetry.fft_bin_count;
    let fft_downsampled = downsample_fft(&telemetry.fft[..bin_count], bin_count);
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    // RMS + beat phase create a global "breathing" expansion
    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    // Recent high-velocity notes add extra breathing
    let velocity_boost = if telemetry.note_age_frames < 8 {
        if let Some(note) = telemetry.last_note_on {
            let vel_norm = note.velocity as f32 / 127.0;
            let age_decay = 1.0 - (telemetry.note_age_frames as f32 / 8.0);
            vel_norm * age_decay * 2.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let breathe = rms_mono * 2.0 + beat_pulse * 1.5 + velocity_boost;

    for (seg, mut transform) in &mut query {
        let mag = fft_downsampled[seg.band_index];

        // Displacement combines FFT magnitude for the specific band and global RMS breathe
        let displacement = seg.normal * (mag * 8.0 + breathe);
        transform.translation = seg.base_pos + displacement;

        // Scale thickness based on magnitude
        let thickness = (0.5 + mag * 3.0) * fade;
        transform.scale = Vec3::new(thickness, thickness, thickness * 2.0);
    }

    // Handle crossfade and centroid-driven material updating
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    if (fade - state.last_fade).abs() > 0.001
        || state.last_policy_version != policy.version
        || (hue_offset - state.last_hue_offset).abs() > 1.0
    {
        let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
        let metallic = policy.metallic;
        let roughness = policy.roughness;

        for (band, handle) in cathedral_materials.materials.iter().enumerate() {
            if let Some(mut material) = materials.get_mut(handle) {
                let hue = (telemetry_color::band_frequency_hue(band as f32 / ARCH_BANDS as f32)
                    + hue_offset)
                    % 360.0;
                let color = Color::hsl(hue, sat, lit * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive * fade;
                material.metallic = metallic;
                material.perceptual_roughness = roughness;
            }
        }
        state.last_fade = fade;
        state.last_hue_offset = hue_offset;
        state.last_policy_version = policy.version;
    }
}
