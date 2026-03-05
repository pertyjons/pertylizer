//! Spectral Origami — Folded planes open with harmonics.
//!
//! A grid of triangles/planes that rotate (fold/unfold) based on Spectral Centroid and FFT.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 12;
const SPACING: f32 = 2.5;
const EMISSIVE_STRENGTH: f32 = 6.0;

#[derive(Component)]
pub struct OrigamiFold {
    pub base_pos: Vec3,
    pub index: usize,
}

#[derive(Resource)]
pub struct OrigamiMaterial(Handle<StandardMaterial>);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A simple plane that we can rotate to look like it's folding
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(SPACING * 0.45)));

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.2, 0.5),
        emissive: LinearRgba::from(Color::srgb(0.8, 0.2, 0.5)) * EMISSIVE_STRENGTH,
        // Double sided so we see it when it folds over
        double_sided: true,
        ..default()
    });

    commands.insert_resource(OrigamiMaterial(material.clone()));

    let offset = (GRID_SIZE as f32 * SPACING) / 2.0;

    let mut i = 0;
    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * SPACING - offset;
            let pz = z as f32 * SPACING - offset;
            let pos = Vec3::new(px, 0.0, pz);

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(pos),
                Visibility::Hidden,
                OrigamiFold {
                    base_pos: pos,
                    index: i,
                },
                EffectLayer(EffectId::SpectralOrigami),
            ));
            i += 1;
        }
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&OrigamiFold, &mut Transform)>,
) {
    if !effect_state.active.is_active(EffectId::SpectralOrigami) && effect_state.fade == 0.0 {
        return;
    }

    let t = time.elapsed_secs();
    let fade = effect_state.fade;

    // Use centroid to drive how "open" the folds are globally
    let centroid_norm = (telemetry.centroid_hz / 5000.0).clamp(0.0, 1.0);

    // Overall energy to drive chaotic fluttering
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 5.0).clamp(0.0, 2.0);

    for (fold, mut transform) in &mut query {
        // Tie to a specific FFT band (looping through available bands)
        let band_idx = fold.index % 64;
        let mag = telemetry.fft[band_idx];

        // Base fold angle driven by centroid + individual band magnitude
        let target_angle_x = (mag * 3.0 + centroid_norm * 2.0) * fade;
        let target_angle_z = (mag * 2.0 + (fold.index as f32 * 0.1 + t).sin() * energy) * fade;

        // Apply rotation
        transform.rotation = Quat::from_euler(EulerRot::XYZ, target_angle_x, 0.0, target_angle_z);

        // Add a slight vertical hop when fluttering
        transform.translation.y = fold.base_pos.y + (mag * 2.0 * fade);
    }
}

pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    origami_material: Res<OrigamiMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_hue: Local<f32>,
    mut frame_counter: Local<u32>,
) {
    let fade = effect_state.fade;

    if !effect_state.active.is_active(EffectId::SpectralOrigami) && fade == 0.0 {
        return;
    }

    // Shift hue slowly over time, bumped by centroid
    let centroid_norm = (telemetry.centroid_hz / 5000.0).clamp(0.0, 1.0);
    *last_hue = (*last_hue + 0.5 + centroid_norm * 2.0) % 360.0;

    // Only update material every 3rd frame to reduce GPU re-uploads
    *frame_counter = frame_counter.wrapping_add(1);
    if !(*frame_counter).is_multiple_of(3) {
        return;
    }

    if let Some(material) = materials.get_mut(&origami_material.0) {
        let color = Color::hsl(*last_hue, 0.8, 0.5 * fade);
        material.base_color = color;
        material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
    }
}
