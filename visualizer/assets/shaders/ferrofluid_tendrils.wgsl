// Ferrofluid Tendrils — Raymarched SDF with smooth blending.
//
// Renders vertical capsules arranged in a circle, driven by FFT magnitudes.
// Uses smooth minimum (smin) to blend capsules together like liquid mercury.
// A single cube mesh serves as the raymarching bounding volume.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}
#import bevy_pbr::mesh_view_bindings::view

struct FerrofluidUniforms {
    time: f32,
    fade: f32,
    num_tendrils: u32,
    hue_offset: f32,
    emissive_strength: f32,
    saturation: f32,
    lightness: f32,
    _padding: f32,
}

@group(3) @binding(0)
var<uniform> uniforms: FerrofluidUniforms;

// Per-tendril data: (x_pos, z_pos, height, hue)
@group(3) @binding(1)
var<storage, read> tendrils: array<vec4<f32>>;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = get_world_from_local(in.instance_index);
    let wp = world * vec4(in.position, 1.0);
    out.world_position = wp.xyz;
    out.clip_position = mesh_position_local_to_clip(world, vec4(in.position, 1.0));
    return out;
}

// Smooth minimum — blends two SDF distances like liquid.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Vertical capsule SDF: base at (px, 0, pz), extends to height h.
fn sd_capsule(p: vec3<f32>, px: f32, pz: f32, h: f32, r: f32) -> f32 {
    let rel = vec3(p.x - px, p.y, p.z - pz);
    let cy = clamp(rel.y, 0.0, max(h, 0.01));
    return length(vec3(rel.x, rel.y - cy, rel.z)) - r;
}

// Scene SDF: smooth union of all tendril capsules.
fn scene_sdf(p: vec3<f32>) -> f32 {
    var d = 999.0;
    let k = 1.5;  // Blending smoothness
    let r = 0.6;  // Capsule radius

    for (var i = 0u; i < uniforms.num_tendrils; i++) {
        let t = tendrils[i];
        let cd = sd_capsule(p, t.x, t.y, t.z, r);
        d = smin(d, cd, k);
    }

    // Add ground plane
    d = smin(d, p.y + 0.3, 0.8);

    return d;
}

// Find hue of the nearest tendril at a point.
fn nearest_hue(p: vec3<f32>) -> f32 {
    var min_d = 999.0;
    var hue = 0.0;
    let r = 0.6;

    for (var i = 0u; i < uniforms.num_tendrils; i++) {
        let t = tendrils[i];
        let d = sd_capsule(p, t.x, t.y, t.z, r);
        if d < min_d {
            min_d = d;
            hue = t.w;
        }
    }
    return hue;
}

// SDF gradient normal.
fn calc_normal(p: vec3<f32>) -> vec3<f32> {
    let e = 0.01;
    return normalize(vec3(
        scene_sdf(p + vec3(e, 0.0, 0.0)) - scene_sdf(p - vec3(e, 0.0, 0.0)),
        scene_sdf(p + vec3(0.0, e, 0.0)) - scene_sdf(p - vec3(0.0, e, 0.0)),
        scene_sdf(p + vec3(0.0, 0.0, e)) - scene_sdf(p - vec3(0.0, 0.0, e)),
    ));
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
    let camera_pos = view.world_position;
    let ray_dir = normalize(in.world_position - camera_pos);

    var pos = in.world_position;
    var total_dist = 0.0;
    let max_dist = 40.0;
    let max_steps = 80;
    let epsilon = 0.01;
    var hit = false;

    for (var i = 0; i < max_steps; i++) {
        let d = scene_sdf(pos);
        if d < epsilon {
            hit = true;
            break;
        }
        if total_dist > max_dist {
            break;
        }
        pos += ray_dir * d;
        total_dist += d;
    }

    if !hit {
        discard;
    }

    // Shade the hit point
    let normal = calc_normal(pos);
    let hue = (nearest_hue(pos) + uniforms.hue_offset) % 360.0;

    // Lambertian diffuse + specular highlights
    let light_dir = normalize(vec3(0.3, 1.0, 0.5));
    let diffuse = max(dot(normal, light_dir), 0.15);
    let reflect_dir = reflect(-light_dir, normal);
    let specular = pow(max(dot(reflect_dir, -ray_dir), 0.0), 48.0);

    // Fresnel rim effect (metallic liquid look)
    let fresnel = pow(1.0 - max(dot(normal, -ray_dir), 0.0), 3.0);

    let base_color = hsl_to_rgb(hue, uniforms.saturation, uniforms.lightness);
    let lit_color = base_color * diffuse + vec3(specular * 0.6) + base_color * fresnel * 0.3;
    let emissive = base_color * uniforms.emissive_strength * uniforms.fade;

    let final_color = (lit_color + emissive) * uniforms.fade;
    return vec4(final_color, 1.0);
}
