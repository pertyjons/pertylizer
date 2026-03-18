// Spectral waterfall — GPU vertex displacement shader.
//
// A single subdivided plane where each vertex is displaced upward (Y)
// based on FFT history data stored in a ring buffer. The fragment shader
// colors vertices by frequency band (hue) and magnitude (brightness).

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

const NUM_BANDS: u32 = 64u;
const NUM_ROWS: u32 = 32u;
const MAX_HEIGHT: f32 = 4.0;

struct WaterfallUniforms {
    front_row: u32,
    fade: f32,
    hue_offset: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    scroll_fraction: f32,
    row_depth: f32,
}

@group(3) @binding(0)
var<uniform> uniforms: WaterfallUniforms;

@group(3) @binding(1)
var<storage, read> fft_history: array<f32>;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) magnitude: f32,
    @location(2) age_normalized: f32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // UV.x = band position [0,1], UV.y = row position [0,1]
    let band = min(u32(in.uv.x * f32(NUM_BANDS)), NUM_BANDS - 1u);
    let visual_row = min(u32(in.uv.y * f32(NUM_ROWS)), NUM_ROWS - 1u);

    // Map visual row to actual ring buffer row
    let actual_row = (uniforms.front_row + NUM_ROWS - visual_row) % NUM_ROWS;

    // Look up magnitude from ring buffer
    let idx = actual_row * NUM_BANDS + band;
    let magnitude = fft_history[idx];

    // Displace Y based on magnitude, scaled by fade
    var pos = in.position;
    let height = magnitude * MAX_HEIGHT * uniforms.fade;
    pos.y += height;

    // Smooth scroll: shift Z slightly based on scroll fraction
    pos.z -= uniforms.scroll_fraction * uniforms.row_depth;

    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(pos, 1.0));
    out.uv = in.uv;
    out.magnitude = magnitude;
    out.age_normalized = in.uv.y; // 0 = newest, 1 = oldest

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

// Nonlinear frequency-to-hue mapping (matches Rust band_frequency_hue).
fn band_frequency_hue(t: f32) -> f32 {
    return sqrt(clamp(t, 0.0, 1.0)) * 270.0;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let hue = (band_frequency_hue(in.uv.x) + uniforms.hue_offset) % 360.0;

    // Modulate lightness by magnitude and fade; older rows fade toward dark
    let age_fade = 1.0 - in.age_normalized * 0.6; // back rows 40% darker
    let mag_factor = clamp(in.magnitude, 0.0, 1.0);
    let lightness = uniforms.lightness * uniforms.fade * (0.2 + 0.8 * mag_factor) * age_fade;

    let base_color = hsl_to_rgb(hue, uniforms.saturation, lightness);

    // Emissive glow: scales with magnitude, fade, and age
    let emissive = base_color * uniforms.emissive_strength * uniforms.fade * mag_factor * age_fade;

    // Output HDR color (base + emissive) for bloom
    let final_color = base_color + emissive;

    return vec4(final_color, 1.0);
}
