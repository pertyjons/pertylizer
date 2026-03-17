#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}
#import bevy_pbr::mesh_vertex_output::MeshVertexOutput

struct CustomMaterial {
    color: vec4<f32>,
}

@group(2) @binding(0)
var<uniform> material: CustomMaterial;

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> MeshVertexOutput {
    var out: MeshVertexOutput;
    
    // Default vertex pass
    var world_from_local = get_world_from_local(0u);
    out.world_position = world_from_local * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.uv = uv;
    out.clip_position = mesh_position_local_to_clip(world_from_local, vec4<f32>(position, 1.0));
    
    return out;
}

@fragment
fn fragment(
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> @location(0) vec4<f32> {
    return material.color;
}