// FFT Terrain — GPU vertex displacement shader.
//
// A heightmap landscape driven by FFT magnitudes. CPU computes smoothed
// per-cell heights (instant attack, smooth decay) and uploads via storage buffer.
// Shader handles displacement and frequency-based coloring with Z-depth fade.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct FftTerrainUniforms {
    num_cols: u32,
    num_rows: u32,
    fade: f32,
    hue_offset: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    flux_boost: f32,
}

@group(3) @binding(0)
var<uniform> uniforms: FftTerrainUniforms;

@group(3) @binding(1)
var<storage, read> heights: array<f32>;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) height: f32,
    @location(2) z_fade: f32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // UV.x = frequency (col), UV.y = depth (row)
    let col = min(u32(in.uv.x * f32(uniforms.num_cols)), uniforms.num_cols - 1u);
    let row = min(u32(in.uv.y * f32(uniforms.num_rows)), uniforms.num_rows - 1u);

    // Look up pre-smoothed height from storage buffer
    let idx = col * uniforms.num_rows + row;
    let height = heights[idx];

    // Displace Y
    var pos = in.position;
    pos.y += height;

    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(pos, 1.0));
    out.uv = in.uv;
    out.height = height;
    out.z_fade = 1.0 - in.uv.y; // front=1.0, back=0.0

    return out;
}

// Nonlinear frequency-to-hue mapping (matches Rust band_frequency_hue).
fn band_frequency_hue(t: f32) -> f32 {
    return sqrt(clamp(t, 0.0, 1.0)) * 270.0;
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
    let hue = (band_frequency_hue(in.uv.x) + uniforms.hue_offset) % 360.0;

    // Z-depth faded lightness: front bright, back dim
    let color_lit = uniforms.lightness * (0.2 + in.z_fade * 0.8) * uniforms.fade;

    let base_color = hsl_to_rgb(hue, uniforms.saturation, color_lit);

    // Emissive with Z-fade and flux boost
    let emissive = base_color * uniforms.emissive_strength * uniforms.flux_boost * in.z_fade * uniforms.fade;

    let final_color = base_color + emissive;
    return vec4(final_color, 1.0);
}
