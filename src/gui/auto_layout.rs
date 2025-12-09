//! Auto-layout algorithm for organizing modules based on signal flow.
//!
//! This module provides automatic positioning of synth modules in a
//! left-to-right layout based on their connections. Modulation sources
//! (Envelopes, LFOs) are placed below the main signal path.

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::Pos2;

use crate::engine::ModuleId;
use crate::modules::core::ModuleCategory;

/// Configuration for auto-layout algorithm.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Horizontal spacing between columns.
    pub column_spacing: f32,
    /// Vertical spacing between rows within a column.
    pub row_spacing: f32,
    /// Starting X position.
    pub start_x: f32,
    /// Starting Y position for main signal path.
    pub start_y: f32,
    /// Y offset for modulation modules (below main path).
    pub modulation_y_offset: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            column_spacing: 220.0,
            row_spacing: 180.0,
            start_x: 50.0,
            start_y: 50.0,
            modulation_y_offset: 280.0,
        }
    }
}

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
/// 1. Build adjacency lists from connections
/// 2. Find source modules (no incoming connections)
/// 3. BFS to calculate depth (longest path from any source)
/// 4. Separate modulation modules from main signal path
/// 5. Place main path modules in columns by depth
/// 6. Place modulation modules below their targets
pub fn calculate_layout(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    config: &LayoutConfig,
) -> LayoutResult {
    let mut result = LayoutResult::default();

    if modules.is_empty() {
        return result;
    }

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

    // Separate modulation modules
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

    // Initialize with disconnected modules at depth 0
    for module in &main_modules {
        depth.insert(module.id, 0);
    }

    // BFS from each source
    for &source in &sources {
        let mut queue = VecDeque::new();
        queue.push_back((source, 0usize));

        while let Some((current, current_depth)) = queue.pop_front() {
            // Update depth to maximum seen
            let entry = depth.entry(current).or_insert(0);
            *entry = (*entry).max(current_depth);

            // Visit outgoing connections (skip modulation targets)
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

    // Place main path modules
    let max_depth = columns.keys().max().copied().unwrap_or(0);
    for col in 0..=max_depth {
        if let Some(module_ids) = columns.get(&col) {
            for (row, &module_id) in module_ids.iter().enumerate() {
                let x = config.start_x + col as f32 * config.column_spacing;
                let y = config.start_y + row as f32 * config.row_spacing;
                result.positions.insert(module_id, Pos2::new(x, y));
            }
        }
    }

    // Place modulation modules below their first target
    // Group by which column their target is in
    let mut mod_by_column: HashMap<usize, Vec<ModuleId>> = HashMap::new();

    for mod_module in &modulation_modules {
        // Find the target column (column of first connected main module)
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

    // Place modulation modules
    for (col, module_ids) in &mod_by_column {
        for (row, &module_id) in module_ids.iter().enumerate() {
            let x = config.start_x + *col as f32 * config.column_spacing;
            let y = config.start_y + config.modulation_y_offset + row as f32 * config.row_spacing;
            result.positions.insert(module_id, Pos2::new(x, y));
        }
    }

    // Handle any modules not yet placed (disconnected modulation)
    let mut unplaced_y = config.start_y + config.modulation_y_offset;
    for module in modules {
        if let std::collections::hash_map::Entry::Vacant(e) = result.positions.entry(module.id) {
            e.insert(Pos2::new(config.start_x, unplaced_y));
            unplaced_y += config.row_spacing;
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

    #[test]
    fn test_empty_layout() {
        let result = calculate_layout(&[], &[], &LayoutConfig::default());
        assert!(result.positions.is_empty());
    }

    #[test]
    fn test_single_module() {
        let modules = vec![ModuleInfo {
            id: make_id(1),
            category: ModuleCategory::Oscillator,
        }];
        let result = calculate_layout(&modules, &[], &LayoutConfig::default());
        assert_eq!(result.positions.len(), 1);
    }

    #[test]
    fn test_linear_chain() {
        let config = LayoutConfig::default();
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

        let result = calculate_layout(&modules, &connections, &config);

        // Oscillator should be leftmost
        let osc_pos = result.positions.get(&make_id(1)).unwrap();
        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let amp_pos = result.positions.get(&make_id(3)).unwrap();

        assert!(osc_pos.x < flt_pos.x);
        assert!(flt_pos.x < amp_pos.x);
    }

    #[test]
    fn test_modulation_below() {
        let config = LayoutConfig::default();
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

        let result = calculate_layout(&modules, &connections, &config);

        // Envelope should be below filter (its target)
        let flt_pos = result.positions.get(&make_id(2)).unwrap();
        let env_pos = result.positions.get(&make_id(3)).unwrap();

        assert!(env_pos.y > flt_pos.y);
    }
}
