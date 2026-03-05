//! Neon Calligraphy — Notes draw glyph strokes.
//!
//! A grid of 3D lines/strokes that light up based on pitch and velocity.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

const NUM_STROKES: usize = 128; // One for each MIDI note
const STROKE_LENGTH: f32 = 4.0;
const EMISSIVE_STRENGTH: f32 = 8.0;

#[derive(Component)]
pub struct GlyphStroke {
    pub note_index: usize,
    pub active_life: f32,
    pub max_life: f32,
    pub hue: f32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(0.2, STROKE_LENGTH, 0.2));

    let mut rng = rand::thread_rng();
    use rand::Rng;

    for i in 0..NUM_STROKES {
        // Distribute randomly in a sphere/cloud
        let r = rng.gen_range(5.0..20.0);
        let theta = rng.gen_range(0.0..std::f32::consts::TAU);
        let phi = rng.gen_range(0.0..std::f32::consts::PI);

        let x = r * phi.sin() * theta.cos();
        let y = r * phi.sin() * theta.sin() + 10.0;
        let z = r * phi.cos();

        let pos = Vec3::new(x, y, z);

        // Random rotation
        let rot_x = rng.gen_range(0.0..std::f32::consts::TAU);
        let rot_y = rng.gen_range(0.0..std::f32::consts::TAU);
        let rot_z = rng.gen_range(0.0..std::f32::consts::TAU);
        let rot = Quat::from_euler(EulerRot::XYZ, rot_x, rot_y, rot_z);

        let hue = (i as f32 / NUM_STROKES as f32) * 360.0;
        let color = Color::hsl(hue, 0.9, 0.5);

        let material = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::BLACK,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(pos)
                .with_rotation(rot)
                .with_scale(Vec3::new(1.0, 0.01, 1.0)),
            Visibility::Hidden,
            GlyphStroke {
                note_index: i,
                active_life: 0.0,
                max_life: 1.0,
                hue,
            },
            EffectLayer(EffectId::NeonCalligraphy),
        ));
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(
        &mut GlyphStroke,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !effect_state.active.is_active(EffectId::NeonCalligraphy) && effect_state.fade == 0.0 {
        return;
    }

    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Check for note on
    let mut triggered_note = None;
    if telemetry.note_age_frames < 2
        && let Some((note, velocity, _instrument_id, _category)) = telemetry.last_note_on
    {
        triggered_note = Some((note as usize, velocity as f32 / 127.0));
    }

    for (mut stroke, mut transform, mat_handle) in &mut query {
        if let Some((note, vel)) = triggered_note
            && stroke.note_index == note
        {
            stroke.active_life = 2.0 * vel;
            stroke.max_life = stroke.active_life;
        }

        if stroke.active_life > 0.0 {
            stroke.active_life -= dt;
            if stroke.active_life < 0.0 {
                stroke.active_life = 0.0;
            }
        }

        let life_pct = if stroke.max_life > 0.0 {
            stroke.active_life / stroke.max_life
        } else {
            0.0
        };

        // Scale up like drawing a stroke
        let target_scale = if stroke.active_life > 0.0 { 1.0 } else { 0.01 };
        transform.scale.y += (target_scale - transform.scale.y) * 10.0 * dt;

        // Update material emissive
        if let Some(material) = materials.get_mut(&mat_handle.0) {
            if stroke.active_life > 0.0 || fade < 1.0 {
                let color = Color::hsl(stroke.hue, 0.9, 0.5 * life_pct * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * life_pct * fade;
            } else {
                material.emissive = LinearRgba::BLACK;
            }
        }
    }
}
