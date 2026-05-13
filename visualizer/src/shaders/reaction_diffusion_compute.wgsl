// Gray-Scott reaction-diffusion compute shader.
//
// Reads chemical concentrations (A in R channel, B in G channel) from input
// texture, computes one simulation step, writes to output texture.
// Uses 5-point Laplacian stencil with wrapping boundary conditions.

struct SimParams {
    feed: f32,
    kill: f32,
    d_a: f32,
    d_b: f32,
}

@group(0) @binding(0)
var tex_in: texture_storage_2d<rgba32float, read>;

@group(0) @binding(1)
var tex_out: texture_storage_2d<rgba32float, write>;

@group(0) @binding(2)
var<uniform> params: SimParams;

@compute @workgroup_size(8, 8, 1)
fn update(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(tex_in));
    let pos = vec2<i32>(id.xy);

    if pos.x >= size.x || pos.y >= size.y {
        return;
    }

    let current = textureLoad(tex_in, pos);
    let a = current.r;
    let b = current.g;

    // 5-point Laplacian stencil with wrapping
    let left  = textureLoad(tex_in, (pos + vec2(-1, 0) + size) % size);
    let right = textureLoad(tex_in, (pos + vec2( 1, 0)) % size);
    let up    = textureLoad(tex_in, (pos + vec2( 0,-1) + size) % size);
    let down  = textureLoad(tex_in, (pos + vec2( 0, 1)) % size);

    let lap_a = left.r + right.r + up.r + down.r - 4.0 * a;
    let lap_b = left.g + right.g + up.g + down.g - 4.0 * b;

    // Gray-Scott reaction-diffusion
    let reaction = a * b * b;
    let new_a = clamp(a + params.d_a * lap_a * 0.2 - reaction + params.feed * (1.0 - a), 0.0, 1.0);
    let new_b = clamp(b + params.d_b * lap_b * 0.2 + reaction - (params.kill + params.feed) * b, 0.0, 1.0);

    textureStore(tex_out, pos, vec4(new_a, new_b, 0.0, 1.0));
}
