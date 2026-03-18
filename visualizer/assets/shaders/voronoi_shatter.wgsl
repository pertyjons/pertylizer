// Voronoi Shatter — GPU vertex displacement with per-cell rotation.
//
// A single mesh of separate (unconnected) quads. The vertex shader computes
// per-cell shatter displacement, rotation, and hop from uniforms.
// Cell identity derived from vertex base position in the grid.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

const GRID_SIZE: f32 = 10.0;
const SPACING: f32 = 2.8;

struct VoronoiUniforms {
    time: f32,
    shatter: f32,
    fade: f32,
    beat_pulse: f32,
    hue_base: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    flux_boost: f32,
    shatter_boost: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(2) @binding(0)
var<uniform> uniforms: VoronoiUniforms;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) cell_index: f32,
    @location(1) @interpolate(flat) cell_id: u32,
}

// Rotate a point around X axis.
fn rot_x(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(p.x, p.y * c - p.z * s, p.y * s + p.z * c);
}

// Rotate a point around Z axis.
fn rot_z(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(p.x * c - p.y * s, p.x * s + p.y * c, p.z);
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let offset = (GRID_SIZE * SPACING) / 2.0;

    // Find which cell this vertex belongs to by snapping to grid
    let cell_x = round((in.position.x + offset - SPACING * 0.5) / SPACING);
    let cell_z = round((in.position.z + offset - SPACING * 0.5) / SPACING);
    let center_x = cell_x * SPACING - offset + SPACING * 0.5;
    let center_z = cell_z * SPACING - offset + SPACING * 0.5;
    let cell_center = vec3(center_x, 0.05, center_z);

    let idx = cell_x * GRID_SIZE + cell_z;
    let idx_f = idx;

    // Local vertex position relative to cell center
    let local = in.position - cell_center;

    // Shatter direction: away from grid center
    let dir_len = max(length(vec2(center_x, center_z)), 0.01);
    let shatter_dir = vec2(center_x, center_z) / dir_len;
    let displacement = vec3(shatter_dir.x, 0.0, shatter_dir.y) * uniforms.shatter * 4.0;

    // Vertical hop during shatter
    let hop = uniforms.shatter * 2.0 * (1.0 + sin(idx_f * 0.7 + uniforms.time * 3.0) * 0.5);

    // Beat pulse when assembled
    let pulse_y = uniforms.beat_pulse * 0.3 * (1.0 - uniforms.shatter);

    // Rotation: tumble during shatter
    let angle_x = uniforms.shatter * sin(idx_f * 0.3 + uniforms.time * 2.0) * 1.5;
    let angle_z = uniforms.shatter * cos(idx_f * 0.5 + uniforms.time * 1.7) * 1.5;
    var rotated = rot_x(local, angle_x * uniforms.fade);
    rotated = rot_z(rotated, angle_z * uniforms.fade);

    let final_pos = cell_center + rotated + displacement * uniforms.fade
        + vec3(0.0, (hop + pulse_y) * uniforms.fade, 0.0);

    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(final_pos, 1.0));
    out.cell_index = idx_f / (GRID_SIZE * GRID_SIZE);
    out.cell_id = u32(idx);

    return out;
}

// HSL to RGB conversion.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> vec3<f32> {
    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - abs(hp % 2.0 - 1.0));
    let m = l - c / 2.0;

    var rgb: vec3<f32>;
    if hp < 1.0 {
        rgb = vec3(c, x, 0.0);
    } else if hp < 2.0 {
        rgb = vec3(x, c, 0.0);
    } else if hp < 3.0 {
        rgb = vec3(0.0, c, x);
    } else if hp < 4.0 {
        rgb = vec3(0.0, x, c);
    } else if hp < 5.0 {
        rgb = vec3(x, 0.0, c);
    } else {
        rgb = vec3(c, 0.0, x);
    }

    return rgb + vec3(m);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Per-cell hue variation
    let cell_hue_offset = f32(in.cell_id % 32u) * (120.0 / 32.0);
    let hue = (uniforms.hue_base + cell_hue_offset) % 360.0;

    // Lightness varies per cell and boosts during shatter
    let lit_variation = (f32(in.cell_id % 32u) / 32.0) * 0.3 - 0.15;
    let lit = (uniforms.lightness * uniforms.shatter_boost + lit_variation) * uniforms.fade;

    let base_color = hsl_to_rgb(hue, uniforms.saturation, clamp(lit, 0.0, 1.0));
    let emissive = base_color * uniforms.emissive_strength * uniforms.flux_boost * uniforms.fade;

    let final_color = base_color + emissive;
    return vec4(final_color, 1.0);
}
