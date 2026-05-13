//! Pertylizer Visualizer — Bevy 3D visualizer driven by OSC telemetry.
//!
//! Receives OSC data from Pertylizer on UDP port 9000 and renders
//! FFT bars, RMS-driven lighting, note particles, and beat-synced effects.
//!
//! # Usage
//!
//! 1. Start Pertylizer (OSC telemetry enabled by default)
//! 2. Run this visualizer: `cargo run --release`

mod osc_receiver;
mod shaders;
mod telemetry;
mod visuals;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Pertylizer Visualizer".to_string(),
                resolution: bevy::window::WindowResolution::new(1280, 720),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(shaders::EmbeddedShadersPlugin)
        .add_plugins(osc_receiver::OscReceiverPlugin)
        .add_plugins(visuals::VisualsPlugin)
        .init_resource::<telemetry::SynthTelemetry>()
        .add_message::<telemetry::NoteOnEvent>()
        .add_message::<visuals::camera::CameraModeEvent>()
        .run();
}
