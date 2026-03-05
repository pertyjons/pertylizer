//! "Waiting for signal" overlay — shown when no OSC data is received.

use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Frames of no data before showing the indicator (~2 seconds at 60 fps).
const STALE_THRESHOLD: u32 = 120;

/// Marker for the waiting text entity.
#[derive(Component)]
pub struct WaitingIndicator;

/// Spawn the indicator text (initially hidden).
pub fn setup(mut commands: Commands) {
    commands.spawn((
        Text::new("Waiting for signal..."),
        TextFont {
            font_size: 28.0,
            ..default()
        },
        TextColor(Color::srgba(0.7, 0.7, 0.8, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(30.0),
            left: Val::Px(30.0),
            ..default()
        },
        WaitingIndicator,
    ));
}

/// Show or hide the indicator based on stale frames.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    mut query: Query<&mut TextColor, With<WaitingIndicator>>,
) {
    let visible = telemetry.stale_frames > STALE_THRESHOLD;

    for mut color in &mut query {
        // Fade in/out smoothly
        let target_alpha = if visible { 0.8 } else { 0.0 };
        let current = color.0.alpha();
        let new_alpha = current + (target_alpha - current) * 0.1;
        *color = TextColor(Color::srgba(0.7, 0.7, 0.8, new_alpha));
    }
}
