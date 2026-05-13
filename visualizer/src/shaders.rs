//! Embeds WGSL shaders into the binary via Bevy's `embedded_asset!`.
//!
//! Each shader is exposed at `embedded://pertylizer_visualizer/shaders/<name>.wgsl`.

use bevy::asset::embedded_asset;
use bevy::prelude::*;

pub struct EmbeddedShadersPlugin;

impl Plugin for EmbeddedShadersPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/cyber_wireframe.wgsl");
        embedded_asset!(app, "shaders/ferrofluid_tendrils.wgsl");
        embedded_asset!(app, "shaders/fft_terrain.wgsl");
        embedded_asset!(app, "shaders/pulse_terrain.wgsl");
        embedded_asset!(app, "shaders/reaction_diffusion_compute.wgsl");
        embedded_asset!(app, "shaders/reaction_diffusion_display.wgsl");
        embedded_asset!(app, "shaders/spectral_aurora.wgsl");
        embedded_asset!(app, "shaders/spectral_waterfall.wgsl");
        embedded_asset!(app, "shaders/voronoi_shatter.wgsl");
    }
}
