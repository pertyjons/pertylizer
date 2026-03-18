// Reaction diffusion display material — renders the simulation texture as a
// glowing organic surface with theme-aware coloring.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct DisplayUniforms {
    hue_base: f32,
    saturation: f32,
    emissive_strength: f32,
    fade: f32,
}

@group(2) @binding(0)
var sim_texture: texture_2d<f32>;

@group(2) @binding(1)
var sim_sampler: sampler;

@group(2) @binding(2)
var<uniform> display: DisplayUniforms;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = get_world_from_local(in.instance_index);
    out.clip_position = mesh_position_local_to_clip(world, vec4(in.position, 1.0));
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
    let sample = textureSample(sim_texture, sim_sampler, in.uv);
    let b = sample.g; // Chemical B concentration

    // Map chemical B to color: low B = dark, high B = bright
    let hue = (display.hue_base + b * 120.0) % 360.0;
    let lit = (0.15 + b * 0.55) * display.fade;

    let base_color = hsl_to_rgb(hue, display.saturation, lit);

    // Emissive glow based on concentration
    let emissive = base_color * display.emissive_strength * b * display.fade;

    let final_color = base_color + emissive;
    return vec4(final_color, 1.0);
}
