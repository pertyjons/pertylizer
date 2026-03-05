//! Pertylizer Visualizer — Bevy 3D visualizer driven by OSC telemetry.
//!
//! Receives OSC data from Pertylizer on UDP port 9000 and renders
//! FFT bars, RMS-driven lighting, and note-flash effects.
//!
//! # Usage
//!
//! 1. Start Pertylizer with `--osc` flag
//! 2. Run this visualizer: `cargo run --release`

mod osc_receiver;
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
        .add_plugins(osc_receiver::OscReceiverPlugin)
        .add_plugins(visuals::VisualsPlugin)
        .init_resource::<telemetry::SynthTelemetry>()
        .run();
}
