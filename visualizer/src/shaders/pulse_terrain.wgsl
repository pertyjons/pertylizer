// Pulse Terrain — GPU vertex displacement shader.
//
// An undulating landscape driven by bass, RMS, and beat pulse.
// All wave math (ripple, noise) computed on GPU. CPU only passes
// smoothed scalar values as uniforms.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct PulseTerrainUniforms {
    time: f32,
    smoothed_bass: f32,
    smoothed_rms: f32,
    beat_pulse: f32,
    velocity_boost: f32,
    fade: f32,
    grid_half_extent: f32,
    max_dist: f32,
    hue_base: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    flux_boost: f32,
    _padding: f32,
}

@group(3) @binding(0)
var<uniform> uniforms: PulseTerrainUniforms;

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
    @location(2) dist_normalized: f32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // World-space XZ position from mesh vertex
    let px = in.position.x;
    let pz = in.position.z;
    let dist = sqrt(px * px + pz * pz);
    let dist_norm = clamp(dist / uniforms.max_dist, 0.0, 1.0);

    let t = uniforms.time;

    // Expanding ripple wave from center
    let ripple = sin(dist * 0.5 - t * 5.0);

    // Positional noise
    let noise = sin(px * 0.2 + t) * cos(pz * 0.2 + t) * 2.0;

    // Height displacement
    let ripple_contrib = ripple * (uniforms.smoothed_rms + uniforms.beat_pulse * 0.3) * 10.0;
    let bass_contrib = uniforms.smoothed_bass
        * (1.0 + uniforms.velocity_boost)
        / (1.0 + dist * 0.1);

    let target_y = (noise + ripple_contrib + bass_contrib) * uniforms.fade;

    var pos = in.position;
    pos.y += target_y;

    // Scale XZ by fade for fade-out shrink
    pos.x *= uniforms.fade;
    pos.z *= uniforms.fade;

    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(pos, 1.0));
    out.uv = in.uv;
    out.height = target_y;
    out.dist_normalized = dist_norm;

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
    let t = in.dist_normalized;

    // Distance-based gradient: center bright, edge dark
    let lit = (uniforms.lightness * (0.7 - t * 0.5)) * uniforms.fade;
    let hue = (uniforms.hue_base + t * 60.0) % 360.0;

    let base_color = hsl_to_rgb(hue, uniforms.saturation, lit);

    // Emissive: stronger at center, weaker at edges
    let emissive_factor = uniforms.emissive_strength * (1.0 - t * 0.8) * uniforms.flux_boost * uniforms.fade;
    let emissive = base_color * emissive_factor;

    let final_color = base_color + emissive;
    return vec4(final_color, 1.0);
}
