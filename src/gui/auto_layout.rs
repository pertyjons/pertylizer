//! Auto-layout algorithm for organizing modules based on signal flow.
//!
//! This module provides automatic positioning of synth modules in a
//! left-to-right layout based on their connections. Modulation sources
//! (Envelopes, LFOs) are placed below the main signal path.
//!
//! The algorithm respects:
//! - Available canvas area (excluding side panels)
//! - Module dimensions to prevent overlap
//! - Signal flow direction (left to right)

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::{Pos2, Vec2};

use crate::engine::ModuleId;
use crate::modules::core::ModuleCategory;

/// Configuration for auto-layout algorithm.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Available area for layout (top-left corner).
    pub area_min: Pos2,
    /// Available area for layout (bottom-right corner).
    pub area_max: Pos2,
    /// Module dimensions (width, height).
    pub module_size: Vec2,
    /// Horizontal gap between modules.
    pub gap_x: f32,
    /// Vertical gap between modules.
    pub gap_y: f32,
    /// Extra vertical offset for modulation modules below main path.
    pub modulation_gap: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            // Default assumes no side panels - caller should override
            area_min: Pos2::new(0.0, 0.0),
            area_max: Pos2::new(1200.0, 800.0),
            module_size: Vec2::new(200.0, 160.0),
            gap_x: 20.0,
            gap_y: 20.0,
            modulation_gap: 40.0,
        }
    }
}

impl LayoutConfig {
    /// Calculate cell size (module + gap).
    fn cell_size(&self) -> Vec2 {
        Vec2::new(
            self.module_size.x + self.gap_x,
            self.module_size.y + self.gap_y,
        )
    }

    /// Available width for layout.
    fn available_width(&self) -> f32 {
        self.area_max.x - self.area_min.x
    }

    /// Available height for layout.
    fn available_height(&self) -> f32 {
        self.area_max.y - self.area_min.y
    }

    /// Maximum columns that fit in available width.
    fn max_columns(&self) -> usize {
        let cell = self.cell_size();
        ((self.available_width() - self.gap_x) / cell.x)
            .floor()
            .max(1.0) as usize
    }

    /// Maximum rows that fit in available height (for main signal path).
    fn max_main_rows(&self) -> usize {
        let cell = self.cell_size();
        // Reserve space for modulation row(s) below
        let main_height = self.available_height() * 0.6;
        ((main_height - self.gap_y) / cell.y).floor().max(1.0) as usize
    }

    /// Position for a module at given column and row.
    fn position_at(&self, col: usize, row: usize) -> Pos2 {
        let cell = self.cell_size();
        Pos2::new(
            self.area_min.x + self.gap_x + col as f32 * cell.x,
            self.area_min.y + self.gap_y + row as f32 * cell.y,
        )
    }

    /// Position for a modulation module at given column and row.
    fn modulation_position_at(&self, col: usize, row: usize, main_rows: usize) -> Pos2 {
        let cell = self.cell_size();
        let base_y = self.area_min.y + self.gap_y + main_rows as f32 * cell.y + self.modulation_gap;
        Pos2::new(
            self.area_min.x + self.gap_x + col as f32 * cell.x,
            base_y + row as f32 * cell.y,
        )
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
/// 2. Find source modules (no incoming connections from main path)
/// 3. BFS to calculate depth (longest path from any source)
/// 4. Separate modulation modules from main signal path
/// 5. Place main path modules in columns by depth, wrapping to next row if needed
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

    // Initialize all main modules at depth 0
    for module in &main_modules {
        depth.insert(module.id, 0);
    }

    // BFS from each source to find longest paths
    for &source in &sources {
        let mut queue = VecDeque::new();
        queue.push_back((source, 0usize));

        while let Some((current, current_depth)) = queue.pop_front() {
            // Update depth to maximum seen
            let entry = depth.entry(current).or_insert(0);
            *entry = (*entry).max(current_depth);

            // Visit outgoing connections (skip modulation modules)
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

    // Calculate layout constraints
    let max_cols = config.max_columns();
    let max_main_rows = config.max_main_rows();
    let max_depth = columns.keys().max().copied().unwrap_or(0);

    // Place main path modules
    // If we have more depth levels than columns, we need to wrap
    let mut current_col = 0;
    let mut current_row = 0;
    let mut actual_main_rows = 1usize;

    for depth_level in 0..=max_depth {
        if let Some(module_ids) = columns.get(&depth_level) {
            for &module_id in module_ids {
                // Place module
                let pos = config.position_at(current_col, current_row);
                result.positions.insert(module_id, pos);

                // Move to next column
                current_col += 1;

                // Wrap to next row if we exceed available columns
                if current_col >= max_cols {
                    current_col = 0;
                    current_row += 1;
                    actual_main_rows = actual_main_rows.max(current_row + 1);

                    // If we're out of vertical space, just keep going (overflow)
                    if current_row >= max_main_rows {
                        // Could add scrolling support here in future
                    }
                }
            }
        }
    }

    // Track how many main rows we actually used
    if current_col > 0 {
        actual_main_rows = actual_main_rows.max(current_row + 1);
    }

    // Place modulation modules below their first target
    // Group by which column their target is in
    let mut mod_by_column: HashMap<usize, Vec<ModuleId>> = HashMap::new();

    for mod_module in &modulation_modules {
        // Find target column by looking at where connected main modules ended up
        let target_col = outgoing
            .get(&mod_module.id)
            .and_then(|targets| {
                targets
                    .iter()
                    .filter(|t| !modulation_ids.contains(t))
                    .filter_map(|t| result.positions.get(t))
                    .map(|pos| {
                        // Calculate which column this position is in
                        ((pos.x - config.area_min.x - config.gap_x) / config.cell_size().x) as usize
                    })
                    .min()
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
            let pos = config.modulation_position_at(*col, row, actual_main_rows);
            result.positions.insert(module_id, pos);
        }
    }

    // Handle any modules not yet placed (disconnected modulation)
    let mut unplaced_col = 0;
    let mut unplaced_row = 0;
    for module in modules {
        if let std::collections::hash_map::Entry::Vacant(e) = result.positions.entry(module.id) {
            let pos = config.modulation_position_at(unplaced_col, unplaced_row, actual_main_rows);
            e.insert(pos);
            unplaced_col += 1;
            if unplaced_col >= max_cols {
                unplaced_col = 0;
                unplaced_row += 1;
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

    fn test_config() -> LayoutConfig {
        LayoutConfig {
            area_min: Pos2::new(0.0, 0.0),
            area_max: Pos2::new(1000.0, 600.0),
            module_size: Vec2::new(180.0, 150.0),
            gap_x: 20.0,
            gap_y: 20.0,
            modulation_gap: 30.0,
        }
    }

    #[test]
    fn test_empty_layout() {
        let result = calculate_layout(&[], &[], &test_config());
        assert!(result.positions.is_empty());
    }

    #[test]
    fn test_single_module() {
        let modules = vec![ModuleInfo {
            id: make_id(1),
            category: ModuleCategory::Oscillator,
        }];
        let result = calculate_layout(&modules, &[], &test_config());
        assert_eq!(result.positions.len(), 1);

        // Should be at first position
        let pos = result.positions.get(&make_id(1)).unwrap();
        assert!(pos.x >= 0.0);
        assert!(pos.y >= 0.0);
    }

    #[test]
    fn test_linear_chain() {
        let config = test_config();
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
        let config = test_config();
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

    #[test]
    fn test_no_overlap() {
        let config = test_config();
        let modules: Vec<ModuleInfo> = (1..=6)
            .map(|i| ModuleInfo {
                id: make_id(i),
                category: ModuleCategory::Oscillator,
            })
            .collect();

        let result = calculate_layout(&modules, &[], &config);

        // Check that no modules overlap
        let positions: Vec<_> = result.positions.values().collect();
        for (i, pos1) in positions.iter().enumerate() {
            for pos2 in positions.iter().skip(i + 1) {
                let dx = (pos1.x - pos2.x).abs();
                let dy = (pos1.y - pos2.y).abs();
                // Either horizontally or vertically separated by at least module size
                assert!(
                    dx >= config.module_size.x || dy >= config.module_size.y,
                    "Modules overlap at {:?} and {:?}",
                    pos1,
                    pos2
                );
            }
        }
    }

    #[test]
    fn test_within_bounds() {
        let config = test_config();
        let modules: Vec<ModuleInfo> = (1..=10)
            .map(|i| ModuleInfo {
                id: make_id(i),
                category: ModuleCategory::Oscillator,
            })
            .collect();

        let result = calculate_layout(&modules, &[], &config);

        // All modules should start within bounds
        for pos in result.positions.values() {
            assert!(
                pos.x >= config.area_min.x,
                "Module x {} < min {}",
                pos.x,
                config.area_min.x
            );
            assert!(
                pos.y >= config.area_min.y,
                "Module y {} < min {}",
                pos.y,
                config.area_min.y
            );
            // Note: we allow overflow if there are many modules
        }
    }
}
