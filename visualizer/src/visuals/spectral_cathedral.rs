//! Spectral Cathedral — FFT bands form arches that breathe.
//!
//! Visual idea: Large arches forming a tunnel/cathedral structure.
//! The height/thickness of each arch segment is driven by FFT, and the whole
//! structure pulses with RMS.

use bevy::color::LinearRgba;
use bevy::prelude::*;
use std::f32::consts::PI;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

/// We use a subset of bands for the arches so it's not too cluttered.
const ARCH_BANDS: usize = 32;

/// Number of points making up each arch.
const ARCH_SEGMENTS: usize = 20;

/// Radius of the cathedral arches.
const ARCH_RADIUS: f32 = 15.0;

/// Spacing between arches along the Z axis.
const ARCH_SPACING: f32 = 2.0;

/// Emissive multiplier.
const EMISSIVE_STRENGTH: f32 = 6.0;

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
) {
    let mesh = meshes.add(Cuboid::new(0.5, 0.5, 0.5));

    // Shared materials, one per band
    let mut shared_mats = Vec::with_capacity(ARCH_BANDS);
    for band in 0..ARCH_BANDS {
        // Hue goes from 0 (red/bass) at the front to 300 (purple/treble) at the back
        let hue = (band as f32 / ARCH_BANDS as f32) * 300.0;
        let color = Color::hsl(hue, 0.8, 0.5);
        shared_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
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

/// Downsample 128-band FFT to the number of arch bands.
fn downsample_fft(fft: &[f32; NUM_FFT_BANDS]) -> [f32; ARCH_BANDS] {
    let mut out = [0.0; ARCH_BANDS];
    let ratio = NUM_FFT_BANDS / ARCH_BANDS;
    for (i, val) in out.iter_mut().enumerate() {
        let start = i * ratio;
        let mut sum = 0.0;
        for j in 0..ratio {
            sum += fft[start + j];
        }
        *val = sum / ratio as f32;
    }
    out
}

#[derive(Resource, Default)]
pub struct CathedralState {
    last_fade: f32,
}

pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: Local<CathedralState>,
    mut query: Query<(&ArchSegment, &mut Transform)>,
    cathedral_materials: Res<CathedralMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let fade = effect_state.fade;

    if !effect_state.active.is_active(EffectId::SpectralCathedral) && fade == 0.0 {
        return;
    }

    let fft_downsampled = downsample_fft(&telemetry.fft);
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    // RMS creates a global "breathing" expansion
    let breathe = rms_mono * 2.0;

    for (seg, mut transform) in &mut query {
        let mag = fft_downsampled[seg.band_index];

        // Displacement combines FFT magnitude for the specific band and global RMS breathe
        let displacement = seg.normal * (mag * 8.0 + breathe);
        transform.translation = seg.base_pos + displacement;

        // Scale thickness based on magnitude
        let thickness = (0.5 + mag * 3.0) * fade;
        transform.scale = Vec3::new(thickness, thickness, thickness * 2.0);
    }

    // Handle crossfade material updating
    if (fade - state.last_fade).abs() > 0.001 {
        for (band, handle) in cathedral_materials.materials.iter().enumerate() {
            if let Some(material) = materials.get_mut(handle) {
                let hue = (band as f32 / ARCH_BANDS as f32) * 300.0;
                let color = Color::hsl(hue, 0.8, 0.5 * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
            }
        }
        state.last_fade = fade;
    }
}
