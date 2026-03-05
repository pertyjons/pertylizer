//! Note flash — emissive sphere that flashes on note-on events.

use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

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
    mut query: Query<
        (&mut FlashState, &MeshMaterial3d<StandardMaterial>),
        With<NoteFlashSphere>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (mut flash, material_handle) in &mut query {
        // Trigger on fresh note-on
        if telemetry.note_age_frames < 2
            && let Some((note, velocity, _channel)) = telemetry.last_note_on
        {
            flash.hue = (note as f32 / 127.0) * 360.0;
            flash.brightness = velocity as f32 / 127.0;
        }

        // Decay
        flash.brightness *= 0.92;

        // Update material
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let lightness = 0.5 * flash.brightness;
            let color = Color::hsl(flash.hue, 0.9, lightness);
            let emissive_lightness = lightness * 8.0;
            material.emissive = Color::hsl(flash.hue, 0.9, emissive_lightness).into();
            material.base_color = color;
        }
    }
}
