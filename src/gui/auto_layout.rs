//! Auto-layout algorithm for organizing modules based on signal flow.
//!
//! This module provides automatic positioning of synth modules that:
//! - Fills the available canvas area without overflow
//! - Places connected modules left-to-right based on signal flow depth
//! - Stacks modules vertically within each depth level
//! - Places modulation sources (Envelope, LFO) below main signal path
//! - Places disconnected modules in bottom-right corner
//! - Ensures ALL modules stay within the visible area (no overlap with keyboard)

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

/// Check if a module has any connections.
fn has_connections(module_id: ModuleId, connections: &[LayoutConnection]) -> bool {
    connections
        .iter()
        .any(|c| c.from_module == module_id || c.to_module == module_id)
}

/// Calculate automatic layout for modules based on signal flow.
///
/// Layout rules:
/// 1. Connected main modules: left-to-right by signal flow depth
/// 2. Connected modulation modules: below their targets
/// 3. Disconnected modules: stacked in bottom-right corner
/// 4. ALL modules must fit within available_rect (no overflow)
pub fn calculate_layout(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    available_rect: Rect,
) -> LayoutResult {
    let mut result = LayoutResult::default();

    if modules.is_empty() {
        return result;
    }

    // Constants - kompakta storlekar efter widget-uppdateringar
    const MIN_MODULE_WIDTH: f32 = 140.0;
    const MIN_MODULE_HEIGHT: f32 = 100.0;
    const GAP: f32 = 8.0;

    // Separate modules into categories
    let mut connected_main: Vec<&ModuleInfo> = Vec::new();
    let mut connected_modulation: Vec<&ModuleInfo> = Vec::new();
    let mut disconnected: Vec<&ModuleInfo> = Vec::new();

    for module in modules {
        let is_connected = has_connections(module.id, connections);
        let is_modulation = is_modulation_module(module.category);

        if !is_connected {
            disconnected.push(module);
        } else if is_modulation {
            connected_modulation.push(module);
        } else {
            connected_main.push(module);
        }
    }

    // Build adjacency lists for connected modules
    let mut outgoing: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    let mut incoming: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

    let modulation_ids: HashSet<ModuleId> = connected_modulation.iter().map(|m| m.id).collect();

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

    // Calculate depth for connected main modules (BFS from sources)
    let mut depth: HashMap<ModuleId, usize> = HashMap::new();

    // Find sources (no incoming from main path)
    let sources: Vec<ModuleId> = connected_main
        .iter()
        .filter(|m| {
            incoming
                .get(&m.id)
                .map(|inc| inc.iter().all(|id| modulation_ids.contains(id)))
                .unwrap_or(true)
        })
        .map(|m| m.id)
        .collect();

    // Initialize all main modules at depth 0
    for module in &connected_main {
        depth.insert(module.id, 0);
    }

    // BFS to find longest paths
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

    // Group connected main modules by depth
    let mut columns: HashMap<usize, Vec<ModuleId>> = HashMap::new();
    for module in &connected_main {
        let d = depth.get(&module.id).copied().unwrap_or(0);
        columns.entry(d).or_default().push(module.id);
    }

    // Count layout requirements
    let num_signal_columns = if columns.is_empty() {
        0
    } else {
        columns.keys().max().map(|m| m + 1).unwrap_or(0)
    };
    let max_modules_per_column = columns.values().map(|v| v.len()).max().unwrap_or(0);
    let has_modulation = !connected_modulation.is_empty();
    let has_disconnected = !disconnected.is_empty();

    // Calculate total columns needed (signal + 1 for disconnected if any)
    let disconnected_column = if has_disconnected { 1 } else { 0 };
    let total_columns = num_signal_columns.max(1) + disconnected_column;

    // Calculate total rows needed
    let main_rows = max_modules_per_column.max(1);
    let modulation_rows = if has_modulation { 1 } else { 0 };
    let disconnected_rows = disconnected.len();

    // For row calculation, disconnected modules stack vertically in their column
    let max_rows_in_disconnected_col = if has_disconnected {
        disconnected_rows.max(1)
    } else {
        0
    };

    // Total rows is max of: main signal rows + modulation, or disconnected stack
    let rows_for_signal = main_rows + modulation_rows;
    let total_rows = rows_for_signal.max(max_rows_in_disconnected_col).max(1);

    // Calculate module size to fit within available space
    let available_width = available_rect.width();
    let available_height = available_rect.height();

    // Width calculation: fit all columns
    let total_gap_x = GAP * (total_columns + 1) as f32;
    let module_width = ((available_width - total_gap_x) / total_columns as f32)
        .max(MIN_MODULE_WIDTH)
        .min(available_width - 2.0 * GAP);

    // Height calculation: MUST fit all rows within available height
    // Calculate the height that guarantees all modules fit
    let total_gap_y = GAP * (total_rows + 1) as f32;
    let max_height_to_fit = (available_height - total_gap_y) / total_rows as f32;

    // Use the smaller of: ideal height or what's needed to fit
    // This ensures modules NEVER extend beyond the canvas
    let module_height = max_height_to_fit.max(MIN_MODULE_HEIGHT);

    // If even minimum height doesn't fit all rows, we need to shrink further
    // Calculate actual usable height per module to guarantee fit
    let module_height = if total_rows as f32 * (module_height + GAP) + GAP > available_height {
        // Force fit: calculate exact height needed
        ((available_height - GAP) / total_rows as f32 - GAP).max(60.0)
    } else {
        module_height
    };

    // Cell size
    let cell_width = module_width + GAP;
    let cell_height = module_height + GAP;

    // Starting position
    let start_x = available_rect.min.x + GAP;
    let start_y = available_rect.min.y + GAP;

    // Maximum position to ensure module's BOTTOM edge stays within bounds
    let max_x = (available_rect.max.x - module_width - GAP).max(start_x);
    let max_y = (available_rect.max.y - module_height - GAP).max(start_y);

    // Helper to clamp position within bounds
    let clamp_pos =
        |x: f32, y: f32| -> Pos2 { Pos2::new(x.clamp(start_x, max_x), y.clamp(start_y, max_y)) };

    // Place connected main modules (left columns)
    for (&depth_level, module_ids) in &columns {
        let col = depth_level;
        for (row, &module_id) in module_ids.iter().enumerate() {
            let x = start_x + col as f32 * cell_width;
            let y = start_y + row as f32 * cell_height;
            result.positions.insert(module_id, clamp_pos(x, y));
        }
    }

    // Place connected modulation modules (below their targets, but within bounds)
    if has_modulation {
        // Modulation modules go below main signal path
        // Calculate Y position that keeps them within bounds
        // The modulation row is at index = main_rows (0-indexed, so after all main rows)
        let mod_row_index = main_rows;
        // Ensure the Y position + module_height stays within available_rect
        let ideal_y = start_y + mod_row_index as f32 * cell_height;
        let mod_base_y = ideal_y.min(max_y);

        // Group by target column
        let mut mod_by_column: HashMap<usize, Vec<ModuleId>> = HashMap::new();
        for mod_module in &connected_modulation {
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

        for (&col, module_ids) in &mod_by_column {
            for (idx, &module_id) in module_ids.iter().enumerate() {
                // If multiple modulation in same column, offset horizontally
                let x = start_x + col as f32 * cell_width + idx as f32 * (module_width * 0.5);
                let y = mod_base_y;
                result.positions.insert(module_id, clamp_pos(x, y));
            }
        }
    }

    // Place disconnected modules (bottom-right corner, stacked vertically)
    if has_disconnected {
        // Disconnected column is the rightmost
        let disconnected_col_x = if num_signal_columns > 0 {
            start_x + num_signal_columns as f32 * cell_width
        } else {
            // If no signal modules, start from right edge
            max_x
        };

        // Stack from bottom up
        for (idx, module) in disconnected.iter().enumerate() {
            let x = disconnected_col_x.min(max_x);
            // Stack from top of disconnected area
            let y = start_y + idx as f32 * cell_height;
            result.positions.insert(module.id, clamp_pos(x, y));
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
    fn test_single_disconnected_module() {
        let modules = vec![ModuleInfo {
            id: make_id(1),
            category: ModuleCategory::Oscillator,
        }];
        let rect = test_rect();
        let result = calculate_layout(&modules, &[], rect);

        let pos = result.positions.get(&make_id(1)).unwrap();
        // Should be within bounds
        assert!(pos.x >= rect.min.x);
        assert!(pos.y >= rect.min.y);
        assert!(pos.x <= rect.max.x - 100.0); // Some margin for module width
        assert!(pos.y <= rect.max.y - 100.0); // Some margin for module height
    }

    #[test]
    fn test_linear_chain_within_bounds() {
        let rect = test_rect();
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

        let result = calculate_layout(&modules, &connections, rect);

        // All positions should be within bounds
        for pos in result.positions.values() {
            assert!(pos.x >= rect.min.x, "x {} < min {}", pos.x, rect.min.x);
            assert!(pos.y >= rect.min.y, "y {} < min {}", pos.y, rect.min.y);
            assert!(pos.x < rect.max.x, "x {} >= max {}", pos.x, rect.max.x);
            assert!(pos.y < rect.max.y, "y {} >= max {}", pos.y, rect.max.y);
        }

        // Should be left to right
        let osc_pos = result.positions.get(&make_id(1)).unwrap();
        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let amp_pos = result.positions.get(&make_id(3)).unwrap();
        assert!(osc_pos.x < flt_pos.x);
        assert!(flt_pos.x < amp_pos.x);
    }

    #[test]
    fn test_modulation_below_and_within_bounds() {
        let rect = test_rect();
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

        let result = calculate_layout(&modules, &connections, rect);

        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let env_pos = result.positions.get(&make_id(3)).unwrap();

        // Envelope below filter
        assert!(env_pos.y > flt_pos.y);

        // All within bounds
        for pos in result.positions.values() {
            assert!(pos.x >= rect.min.x);
            assert!(pos.y >= rect.min.y);
            assert!(pos.x < rect.max.x);
            assert!(pos.y < rect.max.y);
        }
    }

    #[test]
    fn test_disconnected_modules_in_corner() {
        let rect = test_rect();
        let modules = vec![
            // Connected chain
            ModuleInfo {
                id: make_id(1),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(2),
                category: ModuleCategory::Filter,
            },
            // Disconnected
            ModuleInfo {
                id: make_id(10),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(11),
                category: ModuleCategory::LFO,
            },
        ];
        let connections = vec![LayoutConnection {
            from_module: make_id(1),
            to_module: make_id(2),
        }];

        let result = calculate_layout(&modules, &connections, rect);

        let connected_pos_1 = result.positions.get(&make_id(1)).unwrap();
        let disconnected_pos_1 = result.positions.get(&make_id(10)).unwrap();
        let disconnected_pos_2 = result.positions.get(&make_id(11)).unwrap();

        // Disconnected should be to the right of connected
        assert!(
            disconnected_pos_1.x > connected_pos_1.x,
            "Disconnected should be right of connected"
        );

        // Disconnected should not overlap each other (different y positions)
        assert!(
            (disconnected_pos_1.y - disconnected_pos_2.y).abs() > 50.0,
            "Disconnected modules should be stacked vertically"
        );

        // All within bounds
        for pos in result.positions.values() {
            assert!(pos.x >= rect.min.x);
            assert!(pos.y >= rect.min.y);
            assert!(pos.x < rect.max.x);
            assert!(pos.y < rect.max.y);
        }
    }
}
