//! Note Tree — L-system inspired branching structure that grows with notes.
//!
//! A tree of cylinders: trunk + branches that grow on note events and
//! wither during silence. Voice count affects branch density,
//! RMS drives sway and glow.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const EMISSIVE_STRENGTH: f32 = 6.0;

/// Growth rate when notes arrive (per second).
const GROWTH_RATE: f32 = 3.0;
/// Wither rate during silence (per second).
const WITHER_RATE: f32 = 0.5;

/// Branch definition for setup.
struct BranchDef {
    /// Local position relative to parent attachment point.
    pos: Vec3,
    /// Euler rotation (radians).
    rotation: Vec3,
    /// Base scale.
    scale: Vec3,
    /// Depth level (0 = trunk).
    level: u32,
}

#[derive(Component)]
pub struct TreeBranch {
    pub level: u32,
    pub index: usize,
    /// Current growth (0.0 = withered, 1.0 = fully grown).
    pub growth: f32,
    pub base_transform: Transform,
}

#[derive(Resource)]
pub struct TreeMaterial(Handle<StandardMaterial>);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cylinder = meshes.add(Cylinder::new(0.15, 1.0));
    let thin_cylinder = meshes.add(Cylinder::new(0.08, 1.0));

    let color = Color::hsl(120.0, 0.7, 0.4);
    let material = materials.add(StandardMaterial {
        base_color: color,
        emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
        ..default()
    });
    commands.insert_resource(TreeMaterial(material.clone()));

    // Define the tree structure
    let branches: Vec<BranchDef> = vec![
        // Trunk
        BranchDef {
            pos: Vec3::new(0.0, 2.0, 0.0),
            rotation: Vec3::ZERO,
            scale: Vec3::new(1.0, 4.0, 1.0),
            level: 0,
        },
        // Level 1: 4 main branches from top of trunk
        BranchDef {
            pos: Vec3::new(0.8, 5.5, 0.0),
            rotation: Vec3::new(0.0, 0.0, -0.6),
            scale: Vec3::new(0.8, 2.5, 0.8),
            level: 1,
        },
        BranchDef {
            pos: Vec3::new(-0.8, 5.5, 0.0),
            rotation: Vec3::new(0.0, 0.0, 0.6),
            scale: Vec3::new(0.8, 2.5, 0.8),
            level: 1,
        },
        BranchDef {
            pos: Vec3::new(0.0, 5.5, 0.8),
            rotation: Vec3::new(0.6, 0.0, 0.0),
            scale: Vec3::new(0.8, 2.5, 0.8),
            level: 1,
        },
        BranchDef {
            pos: Vec3::new(0.0, 5.5, -0.8),
            rotation: Vec3::new(-0.6, 0.0, 0.0),
            scale: Vec3::new(0.8, 2.5, 0.8),
            level: 1,
        },
        // Level 2: 8 sub-branches (2 per level-1 branch)
        BranchDef {
            pos: Vec3::new(2.5, 7.5, 0.5),
            rotation: Vec3::new(0.0, 0.0, -0.9),
            scale: Vec3::new(0.6, 1.8, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(1.8, 7.8, -0.5),
            rotation: Vec3::new(0.3, 0.0, -0.7),
            scale: Vec3::new(0.6, 1.5, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(-2.5, 7.5, 0.5),
            rotation: Vec3::new(0.0, 0.0, 0.9),
            scale: Vec3::new(0.6, 1.8, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(-1.8, 7.8, -0.5),
            rotation: Vec3::new(0.3, 0.0, 0.7),
            scale: Vec3::new(0.6, 1.5, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(0.5, 7.5, 2.5),
            rotation: Vec3::new(0.9, 0.0, 0.0),
            scale: Vec3::new(0.6, 1.8, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(-0.5, 7.8, 1.8),
            rotation: Vec3::new(0.7, 0.0, 0.3),
            scale: Vec3::new(0.6, 1.5, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(0.5, 7.5, -2.5),
            rotation: Vec3::new(-0.9, 0.0, 0.0),
            scale: Vec3::new(0.6, 1.8, 0.6),
            level: 2,
        },
        BranchDef {
            pos: Vec3::new(-0.5, 7.8, -1.8),
            rotation: Vec3::new(-0.7, 0.0, 0.3),
            scale: Vec3::new(0.6, 1.5, 0.6),
            level: 2,
        },
    ];

    for (i, def) in branches.iter().enumerate() {
        let mesh_handle = if def.level == 0 {
            cylinder.clone()
        } else {
            thin_cylinder.clone()
        };

        let base_transform = Transform {
            translation: def.pos,
            rotation: Quat::from_euler(
                EulerRot::XYZ,
                def.rotation.x,
                def.rotation.y,
                def.rotation.z,
            ),
            scale: def.scale,
        };

        commands.spawn((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material.clone()),
            base_transform,
            Visibility::Hidden,
            TreeBranch {
                level: def.level,
                index: i,
                growth: if def.level == 0 { 0.5 } else { 0.0 },
                base_transform,
            },
            EffectLayer(EffectId::NoteTree),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut TreeBranch, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tree_material: Res<TreeMaterial>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
    mut last_rms: Local<f32>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let fade = effect_state.fade;

    // Voice count determines how many branch levels are active
    let voice_density = (telemetry.voice_count as f32 / 16.0).clamp(0.0, 1.0);

    // Notes trigger growth
    let has_recent_note = telemetry.note_age_frames < 8;

    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    let policy_changed = *last_policy_version != policy.version;
    if policy_changed {
        *last_policy_version = policy.version;
    }

    // Update material color (RMS drives lightness, so also update on RMS change)
    let rms_changed = (rms_mono - *last_rms).abs() > 0.02;
    if rms_changed {
        *last_rms = rms_mono;
    }
    if policy_changed || fade < 1.0 || rms_changed {
        let hue = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
        let sat = (0.6 + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (0.4 + rms_mono * 0.3 + policy.lightness_offset).clamp(0.0, 1.0);
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

        if let Some(material) = materials.get_mut(&tree_material.0) {
            let color = Color::hsl(hue, sat, lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = policy.metallic;
            material.perceptual_roughness = policy.roughness;
        }
    }

    for (mut branch, mut transform) in &mut query {
        // Growth/wither logic
        let target_growth = match branch.level {
            0 => 0.3 + voice_density * 0.7, // Trunk always partially visible
            1 => {
                if has_recent_note {
                    1.0
                } else {
                    voice_density * 0.5
                }
            }
            _ => {
                if has_recent_note && voice_density > 0.3 {
                    voice_density
                } else {
                    0.0
                }
            }
        };

        if branch.growth < target_growth {
            branch.growth = (branch.growth + GROWTH_RATE * dt).min(target_growth);
        } else {
            branch.growth = (branch.growth - WITHER_RATE * dt).max(target_growth);
        }

        let growth = branch.growth * fade;

        // Apply growth as scale
        let base = &branch.base_transform;
        transform.scale = base.scale * growth;

        // Sway in the wind (RMS-driven)
        let sway_x = (t * 0.7 + branch.index as f32 * 0.5).sin() * rms_mono * 0.15;
        let sway_z = (t * 0.9 + branch.index as f32 * 0.3).cos() * rms_mono * 0.15;
        transform.rotation = base.rotation * Quat::from_euler(EulerRot::XYZ, sway_x, 0.0, sway_z);

        // Position stays at base
        transform.translation = base.translation;
    }
}
