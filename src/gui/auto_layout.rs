//! Auto-layout algorithm for organizing modules based on signal flow.
//!
//! This module provides automatic positioning of synth modules that:
//! - Fills the available canvas area
//! - Places modules left-to-right based on signal flow depth
//! - Stacks modules vertically within each depth level
//! - Places modulation sources (Envelope, LFO) below main signal path
//! - Ensures no modules extend outside the visible area

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::{Pos2, Rect};

use crate::engine::ModuleId;
use crate::modules::core::ModuleCategory;

/// Information about a module for layout calculation.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub id: ModuleId,
    pub category: ModuleCategory,
}

/// A connection between two modules for layout calculation.
#[derive(Debug, Clone)]
pub struct LayoutConnection {
    pub from_module: ModuleId,
    pub to_module: ModuleId,
}

/// Result of auto-layout calculation.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    /// New positions for each module.
    pub positions: HashMap<ModuleId, Pos2>,
}

/// Check if a module category is a modulation source.
fn is_modulation_module(category: ModuleCategory) -> bool {
    matches!(category, ModuleCategory::Envelope | ModuleCategory::LFO)
}

/// Calculate automatic layout for modules based on signal flow.
///
/// The algorithm:
/// 1. Build adjacency lists and calculate signal flow depth
/// 2. Separate modulation modules from main signal path
/// 3. Calculate grid dimensions needed
/// 4. Compute module size to fill available space
/// 5. Place modules in their calculated positions
pub fn calculate_layout(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    available_rect: Rect,
) -> LayoutResult {
    let mut result = LayoutResult::default();

    if modules.is_empty() {
        return result;
    }

    // Minimum module size constraints
    const MIN_MODULE_WIDTH: f32 = 150.0;
    const MIN_MODULE_HEIGHT: f32 = 120.0;
    const GAP: f32 = 15.0;
    const MODULATION_GAP: f32 = 25.0;

    // Build adjacency lists
    let mut outgoing: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    let mut incoming: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

    for module in modules {
        outgoing.entry(module.id).or_default();
        incoming.entry(module.id).or_default();
    }

    for conn in connections {
        outgoing
            .entry(conn.from_module)
            .or_default()
            .push(conn.to_module);
        incoming
            .entry(conn.to_module)
            .or_default()
            .push(conn.from_module);
    }

    // Separate modulation modules from main signal path
    let (modulation_modules, main_modules): (Vec<_>, Vec<_>) = modules
        .iter()
        .partition(|m| is_modulation_module(m.category));

    let modulation_ids: HashSet<ModuleId> = modulation_modules.iter().map(|m| m.id).collect();

    // Find sources in main path (modules with no incoming from main path)
    let sources: Vec<ModuleId> = main_modules
        .iter()
        .filter(|m| {
            incoming
                .get(&m.id)
                .map(|inc| inc.iter().all(|id| modulation_ids.contains(id)))
                .unwrap_or(true)
        })
        .map(|m| m.id)
        .collect();

    // BFS to calculate depth (longest path from any source)
    let mut depth: HashMap<ModuleId, usize> = HashMap::new();
    for module in &main_modules {
        depth.insert(module.id, 0);
    }

    for &source in &sources {
        let mut queue = VecDeque::new();
        queue.push_back((source, 0usize));

        while let Some((current, current_depth)) = queue.pop_front() {
            let entry = depth.entry(current).or_insert(0);
            *entry = (*entry).max(current_depth);

            if let Some(neighbors) = outgoing.get(&current) {
                for &neighbor in neighbors {
                    if !modulation_ids.contains(&neighbor) {
                        queue.push_back((neighbor, current_depth + 1));
                    }
                }
            }
        }
    }

    // Group main modules by depth
    let mut columns: HashMap<usize, Vec<ModuleId>> = HashMap::new();
    for module in &main_modules {
        let d = depth.get(&module.id).copied().unwrap_or(0);
        columns.entry(d).or_default().push(module.id);
    }

    // Calculate grid dimensions
    let num_columns = columns.keys().max().map(|m| m + 1).unwrap_or(1);
    let max_modules_per_column = columns.values().map(|v| v.len()).max().unwrap_or(1);

    // Determine if we need a modulation row
    let has_modulation = !modulation_modules.is_empty();
    let _modulation_columns = if has_modulation {
        // Group modulation by target column
        let mut mod_by_col: HashMap<usize, usize> = HashMap::new();
        for mod_module in &modulation_modules {
            let target_col = outgoing
                .get(&mod_module.id)
                .and_then(|targets| {
                    targets
                        .iter()
                        .filter(|t| !modulation_ids.contains(t))
                        .filter_map(|t| depth.get(t))
                        .min()
                        .copied()
                })
                .unwrap_or(0);
            *mod_by_col.entry(target_col).or_insert(0) += 1;
        }
        mod_by_col.values().max().copied().unwrap_or(1)
    } else {
        0
    };

    // Calculate available space
    let available_width = available_rect.width() - GAP;
    let available_height = available_rect.height() - GAP;

    // Calculate module size to fill space
    // Width: divide available width by number of columns
    let module_width =
        ((available_width - GAP * num_columns as f32) / num_columns as f32).max(MIN_MODULE_WIDTH);

    // Height: divide available height by total rows needed
    let main_rows = max_modules_per_column;
    let mod_rows = if has_modulation { 1 } else { 0 };
    let total_rows = main_rows + mod_rows;

    let height_for_modules = if has_modulation {
        available_height - MODULATION_GAP
    } else {
        available_height
    };

    let module_height =
        ((height_for_modules - GAP * total_rows as f32) / total_rows as f32).max(MIN_MODULE_HEIGHT);

    // Cell size including gap
    let cell_width = module_width + GAP;
    let cell_height = module_height + GAP;

    // Calculate starting position to center the grid if modules are at minimum size
    let total_width = num_columns as f32 * cell_width;
    let start_x = available_rect.min.x + GAP + (available_width - total_width).max(0.0) / 2.0;
    let start_y = available_rect.min.y + GAP;

    // Place main path modules
    for (&depth_level, module_ids) in &columns {
        let col = depth_level;
        for (row, &module_id) in module_ids.iter().enumerate() {
            let x = start_x + col as f32 * cell_width;
            let y = start_y + row as f32 * cell_height;
            result.positions.insert(module_id, Pos2::new(x, y));
        }
    }

    // Place modulation modules below main path
    if has_modulation {
        let mod_base_y = start_y + main_rows as f32 * cell_height + MODULATION_GAP;

        // Group modulation by target column
        let mut mod_by_column: HashMap<usize, Vec<ModuleId>> = HashMap::new();
        for mod_module in &modulation_modules {
            let target_col = outgoing
                .get(&mod_module.id)
                .and_then(|targets| {
                    targets
                        .iter()
                        .filter(|t| !modulation_ids.contains(t))
                        .filter_map(|t| depth.get(t))
                        .min()
                        .copied()
                })
                .unwrap_or(0);
            mod_by_column
                .entry(target_col)
                .or_default()
                .push(mod_module.id);
        }

        for (col, module_ids) in &mod_by_column {
            for (idx, &module_id) in module_ids.iter().enumerate() {
                // Stack horizontally within the column if multiple
                let x = start_x + *col as f32 * cell_width + idx as f32 * cell_width / 2.0;
                let y = mod_base_y;
                result.positions.insert(module_id, Pos2::new(x, y));
            }
        }

        // Handle disconnected modulation modules
        let mut unplaced_idx = 0;
        for module in &modulation_modules {
            if let std::collections::hash_map::Entry::Vacant(e) = result.positions.entry(module.id)
            {
                let x = start_x + unplaced_idx as f32 * cell_width;
                let y = mod_base_y;
                e.insert(Pos2::new(x, y));
                unplaced_idx += 1;
            }
        }
    }

    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_id(n: u16) -> ModuleId {
        ModuleId::new(crate::engine::typed_params::ModuleType::Oscillator, n)
    }

    fn test_rect() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 500.0))
    }

    #[test]
    fn test_empty_layout() {
        let result = calculate_layout(&[], &[], test_rect());
        assert!(result.positions.is_empty());
    }

    #[test]
    fn test_single_module() {
        let modules = vec![ModuleInfo {
            id: make_id(1),
            category: ModuleCategory::Oscillator,
        }];
        let rect = test_rect();
        let result = calculate_layout(&modules, &[], rect);
        assert_eq!(result.positions.len(), 1);

        let pos = result.positions.get(&make_id(1)).unwrap();
        assert!(pos.x >= rect.min.x);
        assert!(pos.y >= rect.min.y);
    }

    #[test]
    fn test_linear_chain() {
        let modules = vec![
            ModuleInfo {
                id: make_id(1),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(2),
                category: ModuleCategory::Filter,
            },
            ModuleInfo {
                id: make_id(3),
                category: ModuleCategory::Amplifier,
            },
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(1),
                to_module: make_id(2),
            },
            LayoutConnection {
                from_module: make_id(2),
                to_module: make_id(3),
            },
        ];

        let result = calculate_layout(&modules, &connections, test_rect());

        let osc_pos = result.positions.get(&make_id(1)).unwrap();
        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let amp_pos = result.positions.get(&make_id(3)).unwrap();

        // Should be left to right
        assert!(osc_pos.x < flt_pos.x);
        assert!(flt_pos.x < amp_pos.x);
    }

    #[test]
    fn test_modulation_below() {
        let modules = vec![
            ModuleInfo {
                id: make_id(1),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(2),
                category: ModuleCategory::Filter,
            },
            ModuleInfo {
                id: make_id(3),
                category: ModuleCategory::Envelope,
            },
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(1),
                to_module: make_id(2),
            },
            LayoutConnection {
                from_module: make_id(3),
                to_module: make_id(2),
            },
        ];

        let result = calculate_layout(&modules, &connections, test_rect());

        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let env_pos = result.positions.get(&make_id(3)).unwrap();

        assert!(env_pos.y > flt_pos.y);
    }

    #[test]
    fn test_within_bounds() {
        let rect = test_rect();
        let modules: Vec<ModuleInfo> = (1..=6)
            .map(|i| ModuleInfo {
                id: make_id(i),
                category: ModuleCategory::Oscillator,
            })
            .collect();

        let result = calculate_layout(&modules, &[], rect);

        for pos in result.positions.values() {
            assert!(
                pos.x >= rect.min.x,
                "Module x {} < min {}",
                pos.x,
                rect.min.x
            );
            assert!(
                pos.y >= rect.min.y,
                "Module y {} < min {}",
                pos.y,
                rect.min.y
            );
        }
    }
}
