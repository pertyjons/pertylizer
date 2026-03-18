//! Reaction Diffusion — GPU compute shader Gray-Scott simulation.
//!
//! Runs a 256x256 Gray-Scott reaction-diffusion simulation entirely on the GPU
//! via a compute shader with ping-pong double-buffered textures. The simulation
//! output is displayed on a floating quad with theme-aware coloring.
//!
//! Replaces the previous 144-entity CPU approach (12x12 spheres at 15 Hz) with
//! a single quad entity and GPU compute at 60 Hz on a 256x256 grid.

use std::borrow::Cow;

use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{self, RenderGraph, RenderLabel};
use bevy::render::render_resource::binding_types::{texture_storage_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Simulation grid size (width and height in pixels).
const SIM_SIZE: u32 = 256;
/// Compute shader workgroup size (must match @workgroup_size in WGSL).
const WORKGROUP_SIZE: u32 = 8;
/// Display quad world size.
const QUAD_SIZE: f32 = 22.0;
/// Height of the display quad in world space.
const QUAD_Y: f32 = 6.0;
/// Base emissive strength.
const EMISSIVE_STRENGTH: f32 = 2.0;

// ============================================================================
// Main world: display material
// ============================================================================

/// Uniform data for the display material fragment shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct DisplayUniforms {
    pub hue_base: f32,
    pub saturation: f32,
    pub emissive_strength: f32,
    pub fade: f32,
}

/// Custom material that samples the simulation texture and renders it with
/// theme-aware HSL coloring and emissive glow.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct ReactionDiffusionDisplayMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub display: DisplayUniforms,
}

impl Material for ReactionDiffusionDisplayMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/reaction_diffusion_display.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/reaction_diffusion_display.wgsl".into()
    }
}

// ============================================================================
// Main world: resources
// ============================================================================

/// Holds handles to the ping-pong simulation textures. Extracted to render world.
#[derive(Resource, Clone, ExtractResource)]
pub struct ReactionDiffusionImages {
    texture_a: Handle<Image>,
    texture_b: Handle<Image>,
}

/// Simulation parameters driven by telemetry. Extracted to render world.
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct ReactionDiffusionParams {
    feed: f32,
    kill: f32,
    d_a: f32,
    d_b: f32,
}

impl Default for ReactionDiffusionParams {
    fn default() -> Self {
        Self {
            feed: 0.04,
            kill: 0.06,
            d_a: 0.8,
            d_b: 0.4,
        }
    }
}

/// Tracks the display material handle and frame parity for texture swapping.
#[derive(Resource, Default)]
pub struct ReactionDiffusionState {
    material_handle: Handle<ReactionDiffusionDisplayMaterial>,
    /// Which texture was last displayed (toggles each frame).
    display_parity: bool,
}


// ============================================================================
// Main world: setup and update systems
// ============================================================================

/// Create the initial simulation textures with chemical A=1.0 everywhere and
/// B=0.8 seeded in the center region with some random spots.
fn create_sim_texture() -> Image {
    let size = SIM_SIZE as usize;
    // rgba32float = 16 bytes per pixel
    let mut data = vec![0u8; size * size * 16];

    for y in 0..size {
        for x in 0..size {
            let offset = (y * size + x) * 16;
            let a: f32 = 1.0;

            // Seed chemical B in center circle and a few random spots
            let dx = x as f32 - size as f32 / 2.0;
            let dy = y as f32 - size as f32 / 2.0;
            let dist = (dx * dx + dy * dy).sqrt();
            let b: f32 = if dist < 30.0 { 0.8 } else { 0.0 };

            data[offset..offset + 4].copy_from_slice(&a.to_le_bytes());
            data[offset + 4..offset + 8].copy_from_slice(&b.to_le_bytes());
            // G, B alpha channels stay 0.0
        }
    }

    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: SIM_SIZE,
            height: SIM_SIZE,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        TextureFormat::Rgba32Float,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );

    image.texture_descriptor.usage = TextureUsages::COPY_DST
        | TextureUsages::STORAGE_BINDING
        | TextureUsages::TEXTURE_BINDING;

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());

    image
}

/// Spawn the display quad and initialize simulation resources.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ReactionDiffusionDisplayMaterial>>,
    mut state: ResMut<ReactionDiffusionState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    // Create ping-pong textures
    let texture_a = images.add(create_sim_texture());
    let texture_b = images.add(create_sim_texture());

    commands.insert_resource(ReactionDiffusionImages {
        texture_a: texture_a.clone(),
        texture_b: texture_b.clone(),
    });

    // Create display quad mesh (horizontal plane)
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(QUAD_SIZE / 2.0)));

    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    // Display initially shows texture_a
    let material = materials.add(ReactionDiffusionDisplayMaterial {
        texture: texture_a,
        display: DisplayUniforms {
            hue_base: 200.0,
            saturation: sat,
            emissive_strength: emissive,
            fade: 1.0,
        },
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, QUAD_Y, 0.0),
        Visibility::Hidden,
        EffectLayer(EffectId::ReactionDiffusion),
    ));
}

/// Update simulation parameters from telemetry and swap display texture.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<ReactionDiffusionState>,
    mut params: ResMut<ReactionDiffusionParams>,
    sim_images: Option<Res<ReactionDiffusionImages>>,
    mut materials: ResMut<Assets<ReactionDiffusionDisplayMaterial>>,
) {
    let Some(sim_images) = sim_images else {
        return;
    };

    let fade = effect_state.fade;

    // Centroid drives feed rate: low centroid (bass) = slow organic, high = fast chaotic
    let centroid_norm = (telemetry.centroid_hz / 5000.0).clamp(0.0, 1.0);
    params.feed = 0.03 + centroid_norm * 0.04;

    // RMS drives kill rate
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    params.kill = 0.06 + rms_mono * 0.02;

    // Swap which texture is displayed (ping-pong with render world)
    state.display_parity = !state.display_parity;
    let display_texture = if state.display_parity {
        sim_images.texture_b.clone()
    } else {
        sim_images.texture_a.clone()
    };

    let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.texture = display_texture;
        mat.display = DisplayUniforms {
            hue_base,
            saturation: sat,
            emissive_strength: emissive,
            fade,
        };
    }
}

// ============================================================================
// Render world: compute pipeline plugin
// ============================================================================

/// Label for the compute node in the render graph.
#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ReactionDiffusionLabel;

/// Plugin that sets up the compute shader pipeline in the render world.
pub struct ReactionDiffusionComputePlugin;

impl Plugin for ReactionDiffusionComputePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractResourcePlugin::<ReactionDiffusionImages>::default(),
            ExtractResourcePlugin::<ReactionDiffusionParams>::default(),
        ));

        let render_app = app.sub_app_mut(RenderApp);

        render_app.add_systems(RenderStartup, init_compute_pipeline);
        render_app.add_systems(
            Render,
            prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
        );

        let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
        render_graph.add_node(ReactionDiffusionLabel, ReactionDiffusionNode::default());
        render_graph.add_node_edge(
            ReactionDiffusionLabel,
            bevy::render::graph::CameraDriverLabel,
        );
    }
}

/// Cached compute pipeline and bind group layout descriptor.
#[derive(Resource)]
struct RdComputePipeline {
    layout_descriptor: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

/// Initialize the compute pipeline on render startup.
fn init_compute_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout_descriptor = BindGroupLayoutDescriptor::new(
        "ReactionDiffusionBindGroupLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_storage_2d(TextureFormat::Rgba32Float, StorageTextureAccess::ReadOnly),
                texture_storage_2d(TextureFormat::Rgba32Float, StorageTextureAccess::WriteOnly),
                uniform_buffer::<ReactionDiffusionParams>(false),
            ),
        ),
    );

    let shader = asset_server.load("shaders/reaction_diffusion_compute.wgsl");

    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::from("rd_compute_pipeline")),
        layout: vec![layout_descriptor.clone()],
        shader,
        shader_defs: vec![],
        entry_point: Some(Cow::from("update")),
        push_constant_ranges: vec![],
        zero_initialize_workgroup_memory: false,
    });

    commands.insert_resource(RdComputePipeline {
        layout_descriptor,
        pipeline,
    });
}

/// Bind groups for ping-pong: [0] reads A→writes B, [1] reads B→writes A.
#[derive(Resource)]
struct RdBindGroups([BindGroup; 2]);

/// Prepare bind groups from extracted resources each frame.
#[allow(clippy::too_many_arguments)]
fn prepare_bind_group(
    mut commands: Commands,
    pipeline: Res<RdComputePipeline>,
    pipeline_cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    sim_images: Res<ReactionDiffusionImages>,
    params: Res<ReactionDiffusionParams>,
    render_device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let (Some(gpu_a), Some(gpu_b)) = (
        gpu_images.get(&sim_images.texture_a),
        gpu_images.get(&sim_images.texture_b),
    ) else {
        return; // Images not yet uploaded to GPU
    };

    let mut uniform_buf = UniformBuffer::from(ReactionDiffusionParams {
        feed: params.feed,
        kill: params.kill,
        d_a: params.d_a,
        d_b: params.d_b,
    });
    uniform_buf.write_buffer(&render_device, &queue);

    let Some(uniform_binding) = uniform_buf.binding() else {
        return;
    };

    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout_descriptor);

    // Bind group 0: Read A → Write B
    let bg_a_to_b = render_device.create_bind_group(
        Some("rd_bind_group_a_to_b"),
        &layout,
        &BindGroupEntries::sequential((
            &gpu_a.texture_view,
            &gpu_b.texture_view,
            uniform_binding.clone(),
        )),
    );

    // Bind group 1: Read B → Write A
    let bg_b_to_a = render_device.create_bind_group(
        Some("rd_bind_group_b_to_a"),
        &layout,
        &BindGroupEntries::sequential((
            &gpu_b.texture_view,
            &gpu_a.texture_view,
            uniform_binding,
        )),
    );

    commands.insert_resource(RdBindGroups([bg_a_to_b, bg_b_to_a]));
}

// ============================================================================
// Render graph node
// ============================================================================

enum RdState {
    Loading,
    Update(usize), // 0 or 1 — ping-pong parity
}

struct ReactionDiffusionNode {
    state: RdState,
}

impl Default for ReactionDiffusionNode {
    fn default() -> Self {
        Self {
            state: RdState::Loading,
        }
    }
}

impl render_graph::Node for ReactionDiffusionNode {
    fn update(&mut self, world: &mut World) {
        let pipeline = world.resource::<RdComputePipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            RdState::Loading => {
                match pipeline_cache.get_compute_pipeline_state(pipeline.pipeline) {
                    CachedPipelineState::Ok(_) => {
                        self.state = RdState::Update(0);
                    }
                    CachedPipelineState::Err(err) => {
                        panic!("Reaction diffusion pipeline error: {err:?}");
                    }
                    _ => {} // Still loading
                }
            }
            RdState::Update(index) => {
                self.state = RdState::Update(1 - index);
            }
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        let RdState::Update(index) = self.state else {
            return Ok(());
        };

        let Some(bind_groups) = world.get_resource::<RdBindGroups>() else {
            return Ok(());
        };

        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<RdComputePipeline>();

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
            return Ok(());
        };

        let mut pass =
            render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("reaction_diffusion_compute"),
                    ..default()
                });

        pass.set_bind_group(0, &bind_groups.0[index], &[]);
        pass.set_pipeline(compute_pipeline);
        pass.dispatch_workgroups(SIM_SIZE / WORKGROUP_SIZE, SIM_SIZE / WORKGROUP_SIZE, 1);

        Ok(())
    }
}
