//! Note flash — emissive sphere that flashes on note-on events.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Decay rate (higher = faster fade).
const DECAY_RATE: f32 = 6.0;

/// Emissive intensity multiplier for bloom visibility.
const EMISSIVE_STRENGTH: f32 = 20.0;

/// Marker for the note-flash sphere.
#[derive(Component)]
pub struct NoteFlashSphere;

/// Current flash state.
#[derive(Component)]
pub struct FlashState {
    /// Current brightness (decays over time).
    pub brightness: f32,
    /// Current hue (set on note-on).
    pub hue: f32,
}

/// Spawn the flash sphere.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.5))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::BLACK,
            emissive: Color::BLACK.into(),
            ..default()
        })),
        Transform::from_xyz(0.0, 5.0, -5.0),
        NoteFlashSphere,
        FlashState {
            brightness: 0.0,
            hue: 0.0,
        },
    ));
}

/// Update flash sphere: trigger on note-on, decay over time.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    time: Res<Time>,
    mut query: Query<(&mut FlashState, &MeshMaterial3d<StandardMaterial>), With<NoteFlashSphere>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();

    for (mut flash, material_handle) in &mut query {
        // Trigger on fresh note-on
        if telemetry.note_age_frames < 2
            && let Some((note, velocity, _channel)) = telemetry.last_note_on
        {
            flash.hue = (note as f32 / 127.0) * 360.0;
            flash.brightness = velocity as f32 / 127.0;
        }

        // Frame-rate-independent exponential decay
        flash.brightness *= (-DECAY_RATE * dt).exp();

        // Skip material update when fully faded
        if flash.brightness < 0.001 {
            continue;
        }

        // Update material
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let lightness = 0.5 * flash.brightness;
            let color = Color::hsl(flash.hue, 0.9, lightness);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * flash.brightness;
        }
    }
}
