// Cyber Wireframe — GPU vertex displacement shader for a LineList grid.
//
// Nodes displace Y based on FFT heights from a storage buffer. Color sweeps
// radially like a radar based on time. Beat pulse modulates line brightness.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct CyberWireframeUniforms {
    time: f32,
    fade: f32,
    beat_pulse: f32,
    hue_offset: f32,
    saturation: f32,
    lightness: f32,
    emissive_strength: f32,
    num_cols: u32,
    num_rows: u32,
    flux_boost: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(3) @binding(0)
var<uniform> uniforms: CyberWireframeUniforms;

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
    @location(2) world_xz: vec2<f32>,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Map UV to grid col/row
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
    // Pass world XZ for radar sweep (use local pos since transform is identity)
    out.world_xz = vec2(pos.x, pos.z);

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
    // Radar sweep: compute angle from grid center, compare to rotating time angle
    let angle = atan2(in.world_xz.y, in.world_xz.x); // -PI..PI
    let sweep_angle = (uniforms.time * 1.5) % 6.283185; // rotating sweep 0..2PI
    // Signed angular difference, wrapped to -PI..PI
    var angle_diff = (angle + 3.141593) - (sweep_angle - 3.141593);
    angle_diff = (angle_diff % 6.283185 + 6.283185) % 6.283185;
    // Radar trail: bright at sweep front, fading behind (0..1 over ~120 degrees)
    let trail = exp(-angle_diff * 1.5);

    // Base hue sweeps with the radar angle, offset by centroid-driven hue_offset
    let hue = ((angle + 3.141593) / 6.283185 * 360.0 + uniforms.hue_offset) % 360.0;

    // Beat pulse modulates overall brightness
    let pulse_brightness = 1.0 + uniforms.beat_pulse * 0.6;

    // Height-based lightness boost
    let height_factor = clamp(in.height * 0.08, 0.0, 0.4);

    let lit = uniforms.lightness * (0.3 + trail * 0.7 + height_factor) * uniforms.fade;
    let base_color = hsl_to_rgb(hue, uniforms.saturation, clamp(lit, 0.0, 1.0));

    // Emissive with radar trail, beat pulse, and flux boost
    let emissive = base_color
        * uniforms.emissive_strength
        * uniforms.flux_boost
        * pulse_brightness
        * (0.2 + trail * 0.8)
        * uniforms.fade;

    let final_color = base_color + emissive;
    return vec4(final_color, 1.0);
}
