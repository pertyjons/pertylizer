//! Auto-layout algorithm for organizing modules based on signal flow analysis.
//!
//! This module provides automatic positioning of synth modules using a 5-phase algorithm:
//! 1. **Classify** modules into SignalChain, Modulation, Global, or Disconnected groups
//! 2. **Topological depth** assignment via Kahn's algorithm → columns (left-to-right)
//! 3. **Vertical ordering** within columns using median heuristic (minimizes cable crossings)
//! 4. **Modulation placement** below their primary signal-chain targets
//! 5. **Pixel positions** computed with fixed estimated sizes
//!
//! Layout zones (left→right): Signal columns | Global column | Disconnected column
//! Layout zones (top→bottom): Signal rows | Modulation gap | Modulation rows

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::{Pos2, Rect};

use synth_core::ModuleCategory;
use synth_engine::ModuleId;

// ── Public API (unchanged) ─────────────────────────────────────────────────

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

// ── Constants ──────────────────────────────────────────────────────────────

/// Grid cell size — must match `patch_editor::GRID_SIZE`.
const GRID: f32 = 50.0;

/// Module cell width in grid units (module ~200px + gap = 5 grid cells = 250px).
const CELL_W_GRIDS: f32 = 5.0;
/// Module cell height in grid units (module ~180px + gap = 4 grid cells = 200px).
const CELL_H_GRIDS: f32 = 4.0;
/// Extra vertical gap between signal and modulation zones (1 grid cell).
const MOD_GAP_GRIDS: f32 = 1.0;

// ── Internal types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleGroup {
    SignalChain,
    Modulation,
    Global,
    Disconnected,
}

// ── Classification ─────────────────────────────────────────────────────────

fn classify_module(category: ModuleCategory, has_signal_connections: bool) -> ModuleGroup {
    match category {
        // Global modules are always placed in the global zone
        ModuleCategory::Effect | ModuleCategory::Visualizer | ModuleCategory::Utility => {
            ModuleGroup::Global
        }
        // Modulation modules with connections go to modulation zone
        ModuleCategory::Envelope | ModuleCategory::LFO if has_signal_connections => {
            ModuleGroup::Modulation
        }
        // Signal chain categories with connections
        ModuleCategory::Oscillator
        | ModuleCategory::Filter
        | ModuleCategory::Mixer
        | ModuleCategory::Amplifier
        | ModuleCategory::Output
        | ModuleCategory::PhysicalModeling
        | ModuleCategory::Sampler
        | ModuleCategory::Sequencer
            if has_signal_connections =>
        {
            ModuleGroup::SignalChain
        }
        // Everything else without connections
        _ => ModuleGroup::Disconnected,
    }
}

fn has_connections(module_id: ModuleId, connections: &[LayoutConnection]) -> bool {
    connections
        .iter()
        .any(|c| c.from_module == module_id || c.to_module == module_id)
}

// ── Topological sort (Kahn's algorithm) ────────────────────────────────────

/// Returns modules in topological order. Modules involved in cycles are placed at the front.
fn topological_sort_kahn(
    module_ids: &[ModuleId],
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    incoming: &HashMap<ModuleId, Vec<ModuleId>>,
) -> Vec<ModuleId> {
    let id_set: HashSet<ModuleId> = module_ids.iter().copied().collect();

    // Compute in-degree within the subset
    let mut in_degree: HashMap<ModuleId, usize> = HashMap::new();
    for &id in module_ids {
        let deg = incoming
            .get(&id)
            .map(|inc| inc.iter().filter(|n| id_set.contains(n)).count())
            .unwrap_or(0);
        in_degree.insert(id, deg);
    }

    let mut queue: VecDeque<ModuleId> = in_degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut sorted = Vec::with_capacity(module_ids.len());

    while let Some(node) = queue.pop_front() {
        sorted.push(node);
        if let Some(neighbors) = outgoing.get(&node) {
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(&neighbor) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    // Cycle handling: any remaining nodes go at the front (depth 0)
    let sorted_set: HashSet<ModuleId> = sorted.iter().copied().collect();
    let mut cycle_nodes: Vec<ModuleId> = module_ids
        .iter()
        .filter(|id| !sorted_set.contains(id))
        .copied()
        .collect();
    cycle_nodes.append(&mut sorted);
    cycle_nodes
}

// ── Depth assignment (longest path) ────────────────────────────────────────

fn assign_signal_depths(
    signal_ids: &[ModuleId],
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    incoming: &HashMap<ModuleId, Vec<ModuleId>>,
    categories: &HashMap<ModuleId, ModuleCategory>,
) -> HashMap<ModuleId, usize> {
    let id_set: HashSet<ModuleId> = signal_ids.iter().copied().collect();
    let topo_order = topological_sort_kahn(signal_ids, outgoing, incoming);

    // Longest-path forward pass
    let mut depth: HashMap<ModuleId, usize> = HashMap::new();
    for &id in signal_ids {
        depth.insert(id, 0);
    }

    for &node in &topo_order {
        let current_depth = depth.get(&node).copied().unwrap_or(0);
        if let Some(neighbors) = outgoing.get(&node) {
            for &neighbor in neighbors {
                if id_set.contains(&neighbor) {
                    let entry = depth.entry(neighbor).or_insert(0);
                    *entry = (*entry).max(current_depth + 1);
                }
            }
        }
    }

    // Force Output modules to the maximum depth
    let max_depth = depth.values().copied().max().unwrap_or(0);
    for (&id, d) in &mut depth {
        if categories.get(&id) == Some(&ModuleCategory::Output) {
            *d = max_depth;
        }
    }

    compact_depths(&mut depth);
    depth
}

/// Remove empty depth levels and renumber 0, 1, 2…
fn compact_depths(depth: &mut HashMap<ModuleId, usize>) {
    let mut used_levels: Vec<usize> = depth
        .values()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    used_levels.sort_unstable();

    let remap: HashMap<usize, usize> = used_levels
        .into_iter()
        .enumerate()
        .map(|(new, old)| (old, new))
        .collect();

    for d in depth.values_mut() {
        if let Some(&new_d) = remap.get(d) {
            *d = new_d;
        }
    }
}

// ── Vertical ordering (median heuristic) ───────────────────────────────────

/// Build column→Vec<ModuleId> map from depth assignments.
fn build_columns(depth: &HashMap<ModuleId, usize>) -> HashMap<usize, Vec<ModuleId>> {
    let mut columns: HashMap<usize, Vec<ModuleId>> = HashMap::new();
    for (&id, &col) in depth {
        columns.entry(col).or_default().push(id);
    }
    columns
}

/// Assign vertical positions within columns using median heuristic to minimize crossings.
/// Returns a map from ModuleId → row index within its column.
fn order_within_columns(
    columns: &mut HashMap<usize, Vec<ModuleId>>,
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    incoming: &HashMap<ModuleId, Vec<ModuleId>>,
    depth: &HashMap<ModuleId, usize>,
) {
    let num_columns = columns.keys().copied().max().map_or(0, |m| m + 1);
    if num_columns == 0 {
        return;
    }

    // Build a reverse lookup: module → row position
    let mut row_of: HashMap<ModuleId, usize> = HashMap::new();
    for modules in columns.values() {
        for (row, &id) in modules.iter().enumerate() {
            row_of.insert(id, row);
        }
    }

    // Left→right pass
    for col in 1..num_columns {
        sort_column_by_median(columns, &mut row_of, col, depth, incoming, true);
    }
    // Right→left pass
    for col in (0..num_columns.saturating_sub(1)).rev() {
        sort_column_by_median(columns, &mut row_of, col, depth, outgoing, false);
    }
}

fn sort_column_by_median(
    columns: &mut HashMap<usize, Vec<ModuleId>>,
    row_of: &mut HashMap<ModuleId, usize>,
    col: usize,
    depth: &HashMap<ModuleId, usize>,
    adj: &HashMap<ModuleId, Vec<ModuleId>>,
    use_incoming: bool,
) {
    let Some(col_modules) = columns.get(&col) else {
        return;
    };
    let col_modules_clone: Vec<ModuleId> = col_modules.clone();

    // Compute median for each module
    let mut medians: Vec<(ModuleId, Option<f32>)> = col_modules_clone
        .iter()
        .map(|&id| {
            let neighbors = adj.get(&id).map(|n| {
                let mut rows: Vec<usize> = n
                    .iter()
                    .filter(|&&neighbor| {
                        let neighbor_col = depth.get(&neighbor).copied().unwrap_or(usize::MAX);
                        if use_incoming {
                            neighbor_col + 1 == col
                        } else {
                            col + 1 == neighbor_col
                        }
                    })
                    .filter_map(|&neighbor| row_of.get(&neighbor).copied())
                    .collect();
                rows.sort_unstable();
                rows
            });

            let median = neighbors.and_then(|rows| {
                if rows.is_empty() {
                    None
                } else {
                    let mid = rows.len() / 2;
                    Some(rows[mid] as f32)
                }
            });
            (id, median)
        })
        .collect();

    // Sort: modules with medians first (by median), then modules without
    medians.sort_by(|a, b| match (a.1, b.1) {
        (Some(ma), Some(mb)) => ma.partial_cmp(&mb).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    let sorted_ids: Vec<ModuleId> = medians.iter().map(|(id, _)| *id).collect();

    // Update row_of
    for (row, &id) in sorted_ids.iter().enumerate() {
        row_of.insert(id, row);
    }

    if let Some(col_vec) = columns.get_mut(&col) {
        *col_vec = sorted_ids;
    }
}

// ── Modulation placement ───────────────────────────────────────────────────

/// Returns (column, modulation_row) for each modulation module.
fn place_modulation(
    mod_modules: &[ModuleId],
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    signal_depth: &HashMap<ModuleId, usize>,
    signal_columns: &HashMap<usize, Vec<ModuleId>>,
) -> HashMap<ModuleId, (usize, usize)> {
    let mut result: HashMap<ModuleId, (usize, usize)> = HashMap::new();

    // For each mod module, find its primary target (signal-chain module with lowest column)
    let mut mod_by_column: HashMap<usize, Vec<(ModuleId, usize)>> = HashMap::new();

    for &mod_id in mod_modules {
        let target_col = outgoing
            .get(&mod_id)
            .and_then(|targets| {
                targets
                    .iter()
                    .filter_map(|t| signal_depth.get(t).copied())
                    .min()
            })
            .unwrap_or(0);

        // Find the target's row position for sorting
        let target_row = outgoing
            .get(&mod_id)
            .and_then(|targets| {
                targets.iter().find_map(|t| {
                    let col = signal_depth.get(t).copied()?;
                    if col == target_col {
                        let modules_in_col = signal_columns.get(&col)?;
                        modules_in_col.iter().position(|&id| id == *t)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0);

        mod_by_column
            .entry(target_col)
            .or_default()
            .push((mod_id, target_row));
    }

    // Sort modulation modules within each column by their target's row position
    for (col, modules) in &mut mod_by_column {
        modules.sort_by_key(|&(_, target_row)| target_row);
        for (mod_row, &(mod_id, _)) in modules.iter().enumerate() {
            result.insert(mod_id, (*col, mod_row));
        }
    }

    result
}

// ── Main layout function ───────────────────────────────────────────────────

/// Calculate automatic layout for modules based on signal flow analysis.
///
/// Layout rules:
/// 1. Signal-chain modules: left-to-right by topological depth
/// 2. Modulation modules: below their primary targets
/// 3. Global modules (effects, visualizers, utility): column after signal chain
/// 4. Disconnected modules: rightmost column
pub fn calculate_layout(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    _available_rect: Rect,
) -> LayoutResult {
    let mut result = LayoutResult::default();

    if modules.is_empty() {
        return result;
    }

    // ── Phase 1: Classify modules ──────────────────────────────────────

    let mut signal_ids: Vec<ModuleId> = Vec::new();
    let mut mod_ids: Vec<ModuleId> = Vec::new();
    let mut global_ids: Vec<ModuleId> = Vec::new();
    let mut disconnected_ids: Vec<ModuleId> = Vec::new();

    let mut categories: HashMap<ModuleId, ModuleCategory> = HashMap::new();

    for module in modules {
        categories.insert(module.id, module.category);
        let connected = has_connections(module.id, connections);
        match classify_module(module.category, connected) {
            ModuleGroup::SignalChain => signal_ids.push(module.id),
            ModuleGroup::Modulation => mod_ids.push(module.id),
            ModuleGroup::Global => global_ids.push(module.id),
            ModuleGroup::Disconnected => disconnected_ids.push(module.id),
        }
    }

    // ── Build adjacency lists (excluding modulation edges for depth) ───

    let mod_id_set: HashSet<ModuleId> = mod_ids.iter().copied().collect();

    // Full adjacency (all connections)
    let mut outgoing_full: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    let mut incoming_full: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

    // Signal-only adjacency (excluding modulation sources)
    let mut outgoing_signal: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    let mut incoming_signal: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

    for module in modules {
        outgoing_full.entry(module.id).or_default();
        incoming_full.entry(module.id).or_default();
        outgoing_signal.entry(module.id).or_default();
        incoming_signal.entry(module.id).or_default();
    }

    for conn in connections {
        outgoing_full
            .entry(conn.from_module)
            .or_default()
            .push(conn.to_module);
        incoming_full
            .entry(conn.to_module)
            .or_default()
            .push(conn.from_module);

        // Signal-only: skip edges where source is modulation
        if !mod_id_set.contains(&conn.from_module) {
            outgoing_signal
                .entry(conn.from_module)
                .or_default()
                .push(conn.to_module);
            incoming_signal
                .entry(conn.to_module)
                .or_default()
                .push(conn.from_module);
        }
    }

    // ── Phase 2: Topological depth assignment ──────────────────────────

    let signal_depth =
        assign_signal_depths(&signal_ids, &outgoing_signal, &incoming_signal, &categories);

    let mut columns = build_columns(&signal_depth);

    // ── Phase 3: Vertical ordering ─────────────────────────────────────

    order_within_columns(
        &mut columns,
        &outgoing_signal,
        &incoming_signal,
        &signal_depth,
    );

    // ── Phase 4: Place modulation modules ──────────────────────────────

    let mod_positions = place_modulation(&mod_ids, &outgoing_full, &signal_depth, &columns);

    // ── Phase 5: Compute grid-snapped pixel positions ────────────────

    let num_signal_columns = columns.keys().copied().max().map_or(0, |m| m + 1);
    let max_signal_rows = columns.values().map(Vec::len).max().unwrap_or(0);
    let has_global = !global_ids.is_empty();
    let has_disconnected = !disconnected_ids.is_empty();

    let global_col_offset = num_signal_columns;
    let disconnected_col_offset = global_col_offset + if has_global { 1 } else { 0 };

    // Grid-aligned cell sizes and origin at grid position (1,1)
    let cell_w = CELL_W_GRIDS * GRID;
    let cell_h = CELL_H_GRIDS * GRID;
    let start_x = GRID; // grid column 1
    let start_y = GRID; // grid row 1

    // Place signal-chain modules
    for (&col, module_ids) in &columns {
        for (row, &module_id) in module_ids.iter().enumerate() {
            let x = start_x + col as f32 * cell_w;
            let y = start_y + row as f32 * cell_h;
            result.positions.insert(module_id, Pos2::new(x, y));
        }
    }

    // Place modulation modules below signal rows
    let mod_base_y = start_y + max_signal_rows.max(1) as f32 * cell_h + MOD_GAP_GRIDS * GRID;

    for (&mod_id, &(col, mod_row)) in &mod_positions {
        let x = start_x + col as f32 * cell_w;
        let y = mod_base_y + mod_row as f32 * cell_h;
        result.positions.insert(mod_id, Pos2::new(x, y));
    }

    // Place unplaced modulation modules (those without signal targets)
    for &mod_id in &mod_ids {
        result.positions.entry(mod_id).or_insert_with(|| {
            let existing_count = mod_positions.len();
            let x = start_x;
            let y = mod_base_y + existing_count as f32 * cell_h;
            Pos2::new(x, y)
        });
    }

    // Place global modules
    if has_global {
        let global_x = start_x + global_col_offset as f32 * cell_w;
        for (row, &id) in global_ids.iter().enumerate() {
            let y = start_y + row as f32 * cell_h;
            result.positions.insert(id, Pos2::new(global_x, y));
        }
    }

    // Place disconnected modules
    if has_disconnected {
        let disc_x = start_x + disconnected_col_offset as f32 * cell_w;
        for (row, &id) in disconnected_ids.iter().enumerate() {
            let y = start_y + row as f32 * cell_h;
            result.positions.insert(id, Pos2::new(disc_x, y));
        }
    }

    result
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_id(n: u16) -> ModuleId {
        ModuleId::new(synth_core::ModuleType::Oscillator, n)
    }

    fn test_rect() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1600.0, 1000.0))
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
        assert!(pos.x >= rect.min.x);
        assert!(pos.y >= rect.min.y);
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
                category: ModuleCategory::Sequencer,
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
    }

    // ── New tests ──────────────────────────────────────────────────────

    #[test]
    fn test_multi_source_to_mixer() {
        let rect = test_rect();
        let modules = vec![
            ModuleInfo {
                id: make_id(1),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(2),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(3),
                category: ModuleCategory::Mixer,
            },
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(1),
                to_module: make_id(3),
            },
            LayoutConnection {
                from_module: make_id(2),
                to_module: make_id(3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc1 = result.positions.get(&make_id(1)).unwrap();
        let osc2 = result.positions.get(&make_id(2)).unwrap();
        let mixer = result.positions.get(&make_id(3)).unwrap();

        // Both oscillators in column 0
        assert!(
            (osc1.x - osc2.x).abs() < 1.0,
            "OSC1 and OSC2 should be in same column, got x={} and x={}",
            osc1.x,
            osc2.x
        );
        // Mixer in column 1 (to the right)
        assert!(
            mixer.x > osc1.x,
            "Mixer should be to the right of oscillators"
        );
        // Oscillators should be stacked vertically
        assert!(
            (osc1.y - osc2.y).abs() > 50.0,
            "Oscillators should be stacked vertically"
        );
    }

    #[test]
    fn test_complex_patch() {
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
            ModuleInfo {
                id: make_id(4),
                category: ModuleCategory::Output,
            },
            ModuleInfo {
                id: make_id(10),
                category: ModuleCategory::Envelope,
            },
            ModuleInfo {
                id: make_id(11),
                category: ModuleCategory::LFO,
            },
        ];
        let connections = vec![
            // Signal chain: OSC → Filter → Amp → Output
            LayoutConnection {
                from_module: make_id(1),
                to_module: make_id(2),
            },
            LayoutConnection {
                from_module: make_id(2),
                to_module: make_id(3),
            },
            LayoutConnection {
                from_module: make_id(3),
                to_module: make_id(4),
            },
            // ENV → Filter, LFO → Amp
            LayoutConnection {
                from_module: make_id(10),
                to_module: make_id(2),
            },
            LayoutConnection {
                from_module: make_id(11),
                to_module: make_id(3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result.positions.get(&make_id(1)).unwrap();
        let flt = result.positions.get(&make_id(2)).unwrap();
        let amp = result.positions.get(&make_id(3)).unwrap();
        let out = result.positions.get(&make_id(4)).unwrap();
        let env = result.positions.get(&make_id(10)).unwrap();
        let lfo = result.positions.get(&make_id(11)).unwrap();

        // Signal chain left-to-right
        assert!(osc.x < flt.x, "OSC should be left of Filter");
        assert!(flt.x < amp.x, "Filter should be left of Amp");
        assert!(
            amp.x < out.x || (amp.x - out.x).abs() < 1.0,
            "Amp should be left of or same column as Output"
        );

        // ENV should be below the signal chain (y > signal rows)
        assert!(env.y > osc.y, "ENV should be below signal chain");
        assert!(lfo.y > osc.y, "LFO should be below signal chain");

        // ENV targets Filter (col 1), LFO targets Amp (col 2)
        assert!(
            lfo.x > env.x || (lfo.x - env.x).abs() < 1.0,
            "LFO (targeting Amp) should be at same or higher column than ENV (targeting Filter)"
        );
    }

    #[test]
    fn test_no_overlap() {
        let rect = test_rect();
        let modules = vec![
            ModuleInfo {
                id: make_id(1),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(2),
                category: ModuleCategory::Oscillator,
            },
            ModuleInfo {
                id: make_id(3),
                category: ModuleCategory::Filter,
            },
            ModuleInfo {
                id: make_id(4),
                category: ModuleCategory::Amplifier,
            },
            ModuleInfo {
                id: make_id(5),
                category: ModuleCategory::Output,
            },
            ModuleInfo {
                id: make_id(10),
                category: ModuleCategory::Envelope,
            },
            ModuleInfo {
                id: make_id(20),
                category: ModuleCategory::Visualizer,
            },
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(1),
                to_module: make_id(3),
            },
            LayoutConnection {
                from_module: make_id(2),
                to_module: make_id(3),
            },
            LayoutConnection {
                from_module: make_id(3),
                to_module: make_id(4),
            },
            LayoutConnection {
                from_module: make_id(4),
                to_module: make_id(5),
            },
            LayoutConnection {
                from_module: make_id(10),
                to_module: make_id(3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);
        let positions: Vec<(ModuleId, Pos2)> = result
            .positions
            .iter()
            .map(|(&id, &pos)| (id, pos))
            .collect();

        // Check all pairs have sufficient separation
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                let (id_a, pos_a) = positions[i];
                let (id_b, pos_b) = positions[j];
                let dx = (pos_a.x - pos_b.x).abs();
                let dy = (pos_a.y - pos_b.y).abs();

                // Modules must differ in at least one axis by at least one grid cell
                let separated = dx >= GRID || dy >= GRID;
                assert!(
                    separated,
                    "Modules {:?} at ({:.0},{:.0}) and {:?} at ({:.0},{:.0}) overlap! dx={:.0} dy={:.0}",
                    id_a, pos_a.x, pos_a.y, id_b, pos_b.x, pos_b.y, dx, dy
                );
            }
        }
    }

    #[test]
    fn test_output_rightmost() {
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
                category: ModuleCategory::Output,
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

        let osc = result.positions.get(&make_id(1)).unwrap();
        let flt = result.positions.get(&make_id(2)).unwrap();
        let out = result.positions.get(&make_id(3)).unwrap();

        // Output must be at the rightmost signal column
        assert!(
            out.x >= osc.x && out.x >= flt.x,
            "Output should be in rightmost column. osc.x={}, flt.x={}, out.x={}",
            osc.x,
            flt.x,
            out.x
        );
    }

    #[test]
    fn test_utility_is_global() {
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
                id: make_id(10),
                category: ModuleCategory::Utility,
            },
        ];
        let connections = vec![LayoutConnection {
            from_module: make_id(1),
            to_module: make_id(2),
        }];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result.positions.get(&make_id(1)).unwrap();
        let flt = result.positions.get(&make_id(2)).unwrap();
        let util = result.positions.get(&make_id(10)).unwrap();

        // Utility should be to the right of the signal chain (global zone)
        let max_signal_x = osc.x.max(flt.x);
        assert!(
            util.x > max_signal_x,
            "Utility should be in global zone (right of signal chain). util.x={}, max_signal_x={}",
            util.x,
            max_signal_x
        );
    }
}
