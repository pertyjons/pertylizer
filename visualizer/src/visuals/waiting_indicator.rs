//! "Waiting for signal" overlay — shown when no OSC data is received.

use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Seconds of no data before showing the indicator.
const STALE_THRESHOLD_SECS: f32 = 2.0;

/// Marker for the waiting text entity.
#[derive(Component)]
pub struct WaitingIndicator;

/// Spawn the indicator text (initially hidden).
pub fn setup(mut commands: Commands) {
    commands.spawn((
        Text::new("Waiting for signal..."),
        TextFont {
            font_size: FontSize::Px(28.0),
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
    let visible = telemetry.stale_seconds > STALE_THRESHOLD_SECS;

    for mut color in &mut query {
        let target_alpha = if visible { 0.8 } else { 0.0 };
        let current = color.0.alpha();

        // Snap to target when close enough to avoid unnecessary writes
        let new_alpha = if (target_alpha - current).abs() < 0.005 {
            target_alpha
        } else {
            current + (target_alpha - current) * 0.1
        };

        if (new_alpha - current).abs() > f32::EPSILON {
            *color = TextColor(Color::srgba(0.7, 0.7, 0.8, new_alpha));
        }
    }
}
