// Spectral Aurora — Vertex displacement shader for aurora borealis curtains.
//
// A tall vertical plane (XY) with vertices displaced horizontally and vertically
// based on time, position, RMS amplitude, and spectral centroid. Colors shift
// through HSL space driven by centroid frequency. Alpha < 1.0 for ethereal look.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct SpectralAuroraUniforms {
    time: f32,
    rms: f32,
    fade: f32,
    hue_base: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    wave_amplitude: f32,
    num_cols: u32,
    num_rows: u32,
    _padding0: u32,
    _padding1: u32,
}

@group(3) @binding(0)
var<uniform> uniforms: SpectralAuroraUniforms;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) wave_intensity: f32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let t = uniforms.time;
    let amp = uniforms.wave_amplitude;

    // UV coordinates drive the wave shape
    let u = in.uv.x;
    let v = in.uv.y;

    var pos = in.position;

    // Horizontal wave: multiple sine layers for organic curtain movement
    let wave1 = sin(v * 6.2832 * 2.0 + t * 1.2 + u * 3.0) * 0.6;
    let wave2 = sin(v * 6.2832 * 3.5 + t * 0.7 - u * 2.0) * 0.3;
    let wave3 = sin(u * 6.2832 * 1.5 + t * 0.5) * 0.4;
    let horizontal = (wave1 + wave2 + wave3) * amp;
    pos.x += horizontal;

    // Vertical wave: gentle breathing motion, stronger at top
    let vert_wave = sin(u * 6.2832 * 2.0 + t * 0.8) * 0.15 * amp * v;
    pos.y += vert_wave;

    // Compute wave intensity for fragment coloring (0..1)
    let intensity = clamp(abs(wave1 + wave2) * 0.8 + 0.2, 0.0, 1.0);
    out.wave_intensity = intensity;

    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(pos, 1.0));
    out.uv = in.uv;

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
    let u = in.uv.x;
    let v = in.uv.y;

    // Hue shifts across the curtain width and with centroid-driven base
    let hue = (uniforms.hue_base + u * 40.0 + v * 20.0) % 360.0;

    // Lightness: brighter in middle vertical band, dimmer at edges
    let edge_falloff = 1.0 - abs(u - 0.5) * 1.6;
    let lit = uniforms.lightness * edge_falloff * in.wave_intensity * uniforms.fade;

    let base_color = hsl_to_rgb(hue, uniforms.saturation, clamp(lit, 0.0, 1.0));

    // Emissive glow scaled by wave intensity and fade
    let emissive = base_color * uniforms.emissive_strength * in.wave_intensity * uniforms.fade;

    let final_color = base_color + emissive;

    // Alpha: ethereal transparency, stronger at vertical edges, softer at top
    let alpha_base = 0.45 + in.wave_intensity * 0.3;
    let alpha_edge = clamp(edge_falloff, 0.0, 1.0);
    let alpha_top = 1.0 - v * 0.4; // fade out slightly toward the top
    let alpha = alpha_base * alpha_edge * alpha_top * uniforms.fade;

    return vec4(final_color, clamp(alpha, 0.0, 0.85));
}
