//! Neon Calligraphy — Notes draw glyph strokes.
//!
//! A grid of 3D lines/strokes that light up based on pitch and velocity.
//! Uses shared materials to avoid per-entity material mutation overhead.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const NUM_STROKES: usize = 128; // One for each MIDI note
const STROKE_LENGTH: f32 = 4.0;
const EMISSIVE_STRENGTH: f32 = 8.0;
/// Number of shared material buckets (strokes are grouped by hue).
const NUM_MATERIAL_BUCKETS: usize = 16;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0,
    saturation: 0.9,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
};

#[derive(Component)]
pub struct GlyphStroke {
    pub note_index: usize,
    pub active_life: f32,
    pub max_life: f32,
}

/// Shared materials to avoid per-entity mutation.
#[derive(Resource)]
pub struct CalligraphyMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(0.2, STROKE_LENGTH, 0.2));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(CalligraphyMaterials {
        materials: shared_mats.clone(),
    });

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

        let mat_idx = (i * NUM_MATERIAL_BUCKETS / NUM_STROKES).min(NUM_MATERIAL_BUCKETS - 1);
        let material = shared_mats[mat_idx].clone();

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
            },
            EffectLayer(EffectId::NeonCalligraphy),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut GlyphStroke, &mut Transform)>,
    calligraphy_materials: Res<CalligraphyMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let dt = time.delta_secs();

    // Check for note on
    let mut triggered_note = None;
    if telemetry.note_age_frames < 2
        && let Some(note_event) = telemetry.last_note_on
    {
        triggered_note = Some((
            note_event.midi_note as usize,
            note_event.velocity as f32 / 127.0,
        ));
    }

    for (mut stroke, mut transform) in &mut query {
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

        // Control visibility entirely through scale — no material mutation per entity
        let life_pct = if stroke.max_life > 0.0 {
            stroke.active_life / stroke.max_life
        } else {
            0.0
        };

        let target_scale = life_pct;
        transform.scale.y += (target_scale - transform.scale.y) * 10.0 * dt;

        // Clamp to minimum
        if transform.scale.y < 0.01 {
            transform.scale.y = 0.01;
        }
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    effects::update_hue_materials_for_fade(
        &mut materials,
        &calligraphy_materials.materials,
        &MAT_CONFIG,
        &policy,
        effect_state.fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
