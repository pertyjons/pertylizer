//! Auto-layout algorithm for organizing modules based on signal flow analysis.
//!
//! This module provides automatic positioning of synth modules using a 5-phase algorithm:
//! 1. **Classify** modules into SignalChain, Modulation, Global, or Disconnected groups
//! 2. **Topological depth** assignment via cycle-broken Kahn → columns (left-to-right)
//! 3. **Vertical ordering** within columns using multi-sweep median heuristic (reduces crossings)
//! 4. **Modulation placement** one column left of their primary targets (when possible)
//! 5. **Pixel positions** computed with fixed estimated sizes
//!
//! Layout zones (left→right): Signal columns | Global column | Disconnected column
//! Layout zones (top→bottom): Signal rows | Modulation gap | Modulation rows

use std::collections::{HashMap, HashSet};

use eframe::egui::{Pos2, Rect, Vec2};

use synth_core::{ModuleCategory, ModuleType};
use synth_engine::ModuleId;

// ── Public API (unchanged) ─────────────────────────────────────────────────

/// Information about a module for layout calculation.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub id: ModuleId,
    pub category: ModuleCategory,
    /// Rendered size of this module (used for overlap-free positioning).
    pub size: Vec2,
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

/// Extra gap between modules (1 grid cell).
const GAP: f32 = GRID;

/// Number of ordering sweeps for crossing reduction.
const MAX_SWEEPS: usize = 8;

// ── Internal types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleGroup {
    SignalChain,
    Modulation,
    Global,
    Disconnected,
}

fn module_sort_key(id: &ModuleId) -> (u32, u16) {
    (id.module_type as u32, id.instance)
}

fn index_of_min_by_key(items: &[ModuleId]) -> usize {
    let mut best_idx = 0;
    let mut best_key = module_sort_key(&items[0]);
    for (idx, id) in items.iter().enumerate().skip(1) {
        let key = module_sort_key(id);
        if key < best_key {
            best_key = key;
            best_idx = idx;
        }
    }
    best_idx
}

// ── Classification ─────────────────────────────────────────────────────────

fn classify_module(
    module_id: ModuleId,
    category: ModuleCategory,
    has_incoming: bool,
    has_outgoing: bool,
) -> ModuleGroup {
    if !(has_incoming || has_outgoing) {
        return ModuleGroup::Disconnected;
    }

    match category {
        // Modulation modules with connections go to modulation zone
        ModuleCategory::Envelope | ModuleCategory::LFO => ModuleGroup::Modulation,
        // Utility/Visualizer: explicit allow-list for global vs signal-chain
        ModuleCategory::Utility | ModuleCategory::Visualizer => {
            if is_global_aux_type(module_id.module_type) {
                ModuleGroup::Global
            } else {
                ModuleGroup::SignalChain
            }
        }
        // Everything else with connections participates in the signal chain
        _ => ModuleGroup::SignalChain,
    }
}

/// Utility/Visualizer modules that should live in the global column.
/// Everything else in Utility/Visualizer is treated as signal-chain.
fn is_global_aux_type(module_type: ModuleType) -> bool {
    matches!(
        module_type,
        ModuleType::Oscilloscope
            | ModuleType::LevelMeter
            | ModuleType::SpectrumAnalyzer
            | ModuleType::ModMatrix
    )
}

// ── Topological sort (Kahn's algorithm) ────────────────────────────────────

/// Returns modules in topological order, breaking cycles deterministically.
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

    let mut available: Vec<ModuleId> = in_degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut remaining: HashSet<ModuleId> = id_set.clone();
    let mut sorted = Vec::with_capacity(module_ids.len());

    while sorted.len() < module_ids.len() {
        if available.is_empty() {
            // Break a cycle: pick a deterministic remaining node and cut its incoming edges.
            let mut candidates: Vec<ModuleId> = remaining.iter().copied().collect();
            candidates.sort_by_key(module_sort_key);
            if let Some(&id) = candidates.first() {
                if let Some(deg) = in_degree.get_mut(&id) {
                    *deg = 0;
                }
                available.push(id);
            }
        }

        let next_idx = index_of_min_by_key(&available);
        let node = available.swap_remove(next_idx);
        if !remaining.remove(&node) {
            continue;
        }
        sorted.push(node);

        if let Some(neighbors) = outgoing.get(&node) {
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(&neighbor)
                    && *deg > 0
                {
                    *deg -= 1;
                    if *deg == 0 {
                        available.push(neighbor);
                    }
                }
            }
        }
    }

    sorted
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
    let order_index: HashMap<ModuleId, usize> = topo_order
        .iter()
        .enumerate()
        .map(|(idx, &id)| (id, idx))
        .collect();

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
                    let order_ok = order_index
                        .get(&neighbor)
                        .and_then(|&n_idx| order_index.get(&node).map(|&c_idx| n_idx > c_idx))
                        .unwrap_or(false);
                    if !order_ok {
                        continue;
                    }
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
fn build_columns(
    depth: &HashMap<ModuleId, usize>,
    signal_ids: &[ModuleId],
) -> HashMap<usize, Vec<ModuleId>> {
    let mut columns: HashMap<usize, Vec<ModuleId>> = HashMap::new();
    let mut ordered_ids: Vec<ModuleId> = signal_ids.to_vec();
    ordered_ids.sort_by_key(module_sort_key);
    for id in ordered_ids {
        if let Some(&col) = depth.get(&id) {
            columns.entry(col).or_default().push(id);
        }
    }
    columns
}

/// Assign vertical positions within columns using multi-sweep median heuristic to reduce crossings.
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

    let mut best_columns = columns.clone();
    let mut best_crossings = total_crossings(columns, outgoing, num_columns);

    for _ in 0..MAX_SWEEPS {
        let mut changed = false;

        // Left→right pass
        for col in 1..num_columns {
            changed |= sort_column_by_median(columns, &mut row_of, col, depth, incoming, true);
        }
        // Right→left pass
        for col in (0..num_columns.saturating_sub(1)).rev() {
            changed |= sort_column_by_median(columns, &mut row_of, col, depth, outgoing, false);
        }

        let crossings = total_crossings(columns, outgoing, num_columns);
        if crossings < best_crossings {
            best_crossings = crossings;
            best_columns = columns.clone();
        }

        if !changed {
            break;
        }
    }

    *columns = best_columns;
}

fn sort_column_by_median(
    columns: &mut HashMap<usize, Vec<ModuleId>>,
    row_of: &mut HashMap<ModuleId, usize>,
    col: usize,
    depth: &HashMap<ModuleId, usize>,
    adj: &HashMap<ModuleId, Vec<ModuleId>>,
    use_incoming: bool,
) -> bool {
    let Some(col_modules) = columns.get(&col) else {
        return false;
    };
    let col_modules_clone: Vec<ModuleId> = col_modules.clone();

    // Compute median for each module
    let mut medians: Vec<(ModuleId, Option<f32>, usize)> = col_modules_clone
        .iter()
        .map(|&id| {
            let neighbors = adj.get(&id).map(|n| {
                let mut rows: Vec<usize> = n
                    .iter()
                    .filter(|&&neighbor| {
                        let neighbor_col = depth.get(&neighbor).copied().unwrap_or(usize::MAX);
                        if neighbor_col == usize::MAX {
                            false
                        } else if use_incoming {
                            neighbor_col < col
                        } else {
                            neighbor_col > col
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
                    if rows.len() % 2 == 1 {
                        Some(rows[mid] as f32)
                    } else {
                        let a = rows[mid - 1] as f32;
                        let b = rows[mid] as f32;
                        Some((a + b) / 2.0)
                    }
                }
            });
            let prev_row = row_of.get(&id).copied().unwrap_or(usize::MAX);
            (id, median, prev_row)
        })
        .collect();

    // Sort: modules with medians first (by median), then modules without.
    // Tie-break with previous row to keep ordering stable.
    medians.sort_by(|a, b| match (a.1, b.1) {
        (Some(ma), Some(mb)) => ma
            .partial_cmp(&mb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.2.cmp(&b.2),
    });

    let sorted_ids: Vec<ModuleId> = medians.iter().map(|(id, _, _)| *id).collect();
    let changed = sorted_ids != col_modules_clone;

    // Update row_of
    for (row, &id) in sorted_ids.iter().enumerate() {
        row_of.insert(id, row);
    }

    if let Some(col_vec) = columns.get_mut(&col) {
        *col_vec = sorted_ids;
    }

    changed
}

fn total_crossings(
    columns: &HashMap<usize, Vec<ModuleId>>,
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    num_columns: usize,
) -> usize {
    let mut row_of: HashMap<ModuleId, usize> = HashMap::new();
    for modules in columns.values() {
        for (row, &id) in modules.iter().enumerate() {
            row_of.insert(id, row);
        }
    }

    let mut total = 0;
    for col in 0..num_columns.saturating_sub(1) {
        total += crossings_between_columns(col, col + 1, columns, outgoing, &row_of);
    }
    total
}

fn crossings_between_columns(
    left_col: usize,
    right_col: usize,
    columns: &HashMap<usize, Vec<ModuleId>>,
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    row_of: &HashMap<ModuleId, usize>,
) -> usize {
    let Some(left_ids) = columns.get(&left_col) else {
        return 0;
    };
    let Some(right_ids) = columns.get(&right_col) else {
        return 0;
    };

    let left_set: HashSet<ModuleId> = left_ids.iter().copied().collect();
    let right_set: HashSet<ModuleId> = right_ids.iter().copied().collect();

    let mut seen: HashSet<(ModuleId, ModuleId)> = HashSet::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    for &u in left_ids {
        if let Some(neighbors) = outgoing.get(&u) {
            for &v in neighbors {
                if right_set.contains(&v)
                    && seen.insert((u, v))
                    && let (Some(&ru), Some(&rv)) = (row_of.get(&u), row_of.get(&v))
                {
                    pairs.push((ru, rv));
                }
            }
        }
    }

    for &u in right_ids {
        if let Some(neighbors) = outgoing.get(&u) {
            for &v in neighbors {
                if left_set.contains(&v)
                    && seen.insert((v, u))
                    && let (Some(&ru), Some(&rv)) = (row_of.get(&v), row_of.get(&u))
                {
                    pairs.push((ru, rv));
                }
            }
        }
    }

    let mut crossings = 0;
    for i in 0..pairs.len() {
        for j in (i + 1)..pairs.len() {
            let (a1, b1) = pairs[i];
            let (a2, b2) = pairs[j];
            if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                crossings += 1;
            }
        }
    }
    crossings
}

// ── Modulation placement ───────────────────────────────────────────────────

/// Returns (column, modulation_row) for each modulation module.
/// Modules are placed one column left of their primary target when possible.
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
        let mod_col = target_col.saturating_sub(1);

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
            .entry(mod_col)
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
/// 2. Modulation modules: below and one column left of their primary targets (when possible)
/// 3. Global modules (aux sinks like visualizers/utility): column after signal chain
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
    let mut has_incoming: HashMap<ModuleId, bool> = HashMap::new();
    let mut has_outgoing: HashMap<ModuleId, bool> = HashMap::new();

    for module in modules {
        has_incoming.insert(module.id, false);
        has_outgoing.insert(module.id, false);
    }
    for conn in connections {
        has_outgoing.insert(conn.from_module, true);
        has_incoming.insert(conn.to_module, true);
    }

    for module in modules {
        categories.insert(module.id, module.category);
        let incoming = *has_incoming.get(&module.id).unwrap_or(&false);
        let outgoing = *has_outgoing.get(&module.id).unwrap_or(&false);
        match classify_module(module.id, module.category, incoming, outgoing) {
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

    let mut columns = build_columns(&signal_depth, &signal_ids);

    // ── Phase 3: Vertical ordering ─────────────────────────────────────

    order_within_columns(
        &mut columns,
        &outgoing_signal,
        &incoming_signal,
        &signal_depth,
    );

    // ── Phase 4: Place modulation modules ──────────────────────────────

    let mod_positions = place_modulation(&mod_ids, &outgoing_full, &signal_depth, &columns);

    // ── Phase 5: Size-aware pixel positions ────────────────────────────

    // Build size lookup from input modules
    let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();

    let num_signal_columns = columns.keys().copied().max().map_or(0, |m| m + 1);

    let start_x = GRID;
    let start_y = GRID;

    // Compute column widths = max snapped width of modules in that column + GAP
    let mut col_widths: Vec<f32> = vec![0.0; num_signal_columns];
    for (&col, module_ids) in &columns {
        let max_w = module_ids
            .iter()
            .map(|id| snap_size_to_grid(sizes.get(id).copied().unwrap_or(DEFAULT_SIZE)).x)
            .fold(0.0_f32, f32::max);
        if let Some(w) = col_widths.get_mut(col) {
            *w = max_w + GAP;
        }
    }

    // Cumulative column x-positions
    let mut col_x: Vec<f32> = vec![start_x; num_signal_columns];
    for c in 1..num_signal_columns {
        col_x[c] = col_x[c - 1] + col_widths[c - 1];
    }

    // Place signal-chain modules with cumulative y per column
    let mut col_bottom: Vec<f32> = vec![start_y; num_signal_columns];
    for col in 0..num_signal_columns {
        if let Some(module_ids) = columns.get(&col) {
            let mut y = start_y;
            for &module_id in module_ids {
                let snapped =
                    snap_size_to_grid(sizes.get(&module_id).copied().unwrap_or(DEFAULT_SIZE));
                result.positions.insert(module_id, Pos2::new(col_x[col], y));
                y += snapped.y + GAP;
            }
            col_bottom[col] = y;
        }
    }

    // Place modulation modules directly below their assigned column's signal modules.
    // This avoids pushing modulators far down when one column is much taller than others.
    for (&mod_id, &(col, _mod_row)) in &mod_positions {
        let x = col_x.get(col).copied().unwrap_or(start_x);
        let y = col_bottom.get(col).copied().unwrap_or(start_y);
        let snapped = snap_size_to_grid(sizes.get(&mod_id).copied().unwrap_or(DEFAULT_SIZE));
        result.positions.insert(mod_id, Pos2::new(x, y));
        // Update col_bottom so subsequent modulators in the same column stack below
        if let Some(bottom) = col_bottom.get_mut(col) {
            *bottom = y + snapped.y + GAP;
        }
    }

    // Place unplaced modulation modules (those without signal targets)
    let max_bottom = col_bottom.iter().copied().fold(start_y, f32::max);
    let mut unplaced_y = max_bottom;
    for &mod_id in &mod_ids {
        if let std::collections::hash_map::Entry::Vacant(e) = result.positions.entry(mod_id) {
            let snapped = snap_size_to_grid(sizes.get(&mod_id).copied().unwrap_or(DEFAULT_SIZE));
            e.insert(Pos2::new(start_x, unplaced_y));
            unplaced_y += snapped.y + GAP;
        }
    }

    // x-offset for extra columns (global, disconnected)
    let signal_end_x = if num_signal_columns > 0 {
        col_x[num_signal_columns - 1] + col_widths[num_signal_columns - 1]
    } else {
        start_x
    };

    // Place global modules
    if !global_ids.is_empty() {
        let mut y = start_y;
        for &id in &global_ids {
            let snapped = snap_size_to_grid(sizes.get(&id).copied().unwrap_or(DEFAULT_SIZE));
            result.positions.insert(id, Pos2::new(signal_end_x, y));
            y += snapped.y + GAP;
        }
    }

    // Place disconnected modules
    if !disconnected_ids.is_empty() {
        // Compute global column width for offset
        let global_max_w = if global_ids.is_empty() {
            0.0
        } else {
            global_ids
                .iter()
                .map(|id| snap_size_to_grid(sizes.get(id).copied().unwrap_or(DEFAULT_SIZE)).x)
                .fold(0.0_f32, f32::max)
                + GAP
        };
        let disc_x = signal_end_x + global_max_w;
        let mut y = start_y;
        for &id in &disconnected_ids {
            let snapped = snap_size_to_grid(sizes.get(&id).copied().unwrap_or(DEFAULT_SIZE));
            result.positions.insert(id, Pos2::new(disc_x, y));
            y += snapped.y + GAP;
        }
    }

    result
}

/// Default module size (fallback before first render).
const DEFAULT_SIZE: Vec2 = Vec2::new(250.0, 200.0);

/// Snap a size up to whole grid cells.
#[must_use]
fn snap_size_to_grid(size: Vec2) -> Vec2 {
    Vec2::new((size.x / GRID).ceil() * GRID, (size.y / GRID).ceil() * GRID)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Default test size for modules.
    const TEST_SIZE: Vec2 = Vec2::new(200.0, 180.0);

    fn make_id(module_type: ModuleType, n: u16) -> ModuleId {
        ModuleId::new(module_type, n)
    }

    fn make_module(id: ModuleId, category: ModuleCategory) -> ModuleInfo {
        ModuleInfo {
            id,
            category,
            size: TEST_SIZE,
        }
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
        let modules = vec![make_module(
            make_id(ModuleType::Oscillator, 1),
            ModuleCategory::Oscillator,
        )];
        let rect = test_rect();
        let result = calculate_layout(&modules, &[], rect);

        let pos = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        assert!(pos.x >= rect.min.x);
        assert!(pos.y >= rect.min.y);
    }

    #[test]
    fn test_linear_chain_within_bounds() {
        let rect = test_rect();
        let modules = vec![
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 2), ModuleCategory::Filter),
            make_module(make_id(ModuleType::Amplifier, 3), ModuleCategory::Amplifier),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Filter, 2),
                to_module: make_id(ModuleType::Amplifier, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        // All positions should be within bounds
        for pos in result.positions.values() {
            assert!(pos.x >= rect.min.x, "x {} < min {}", pos.x, rect.min.x);
            assert!(pos.y >= rect.min.y, "y {} < min {}", pos.y, rect.min.y);
        }

        // Should be left to right
        let osc_pos = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let flt_pos = result
            .positions
            .get(&make_id(ModuleType::Filter, 2))
            .unwrap();
        let amp_pos = result
            .positions
            .get(&make_id(ModuleType::Amplifier, 3))
            .unwrap();
        assert!(osc_pos.x < flt_pos.x);
        assert!(flt_pos.x < amp_pos.x);
    }

    #[test]
    fn test_modulation_below_and_within_bounds() {
        let rect = test_rect();
        let modules = vec![
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 2), ModuleCategory::Filter),
            make_module(make_id(ModuleType::Envelope, 3), ModuleCategory::Envelope),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Envelope, 3),
                to_module: make_id(ModuleType::Filter, 2),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc_pos = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let flt_pos = result
            .positions
            .get(&make_id(ModuleType::Filter, 2))
            .unwrap();
        let env_pos = result
            .positions
            .get(&make_id(ModuleType::Envelope, 3))
            .unwrap();

        // Envelope below filter
        assert!(env_pos.y > flt_pos.y);
        // Envelope in column left of filter when possible
        assert!(env_pos.x <= flt_pos.x, "Envelope should be left of filter");
        assert!(
            (env_pos.x - osc_pos.x).abs() < 1.0,
            "Envelope should align with the previous column"
        );

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
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 2), ModuleCategory::Filter),
            // Disconnected
            make_module(
                make_id(ModuleType::Oscillator, 10),
                ModuleCategory::Oscillator,
            ),
            make_module(
                make_id(ModuleType::Euclidean, 11),
                ModuleCategory::Sequencer,
            ),
        ];
        let connections = vec![LayoutConnection {
            from_module: make_id(ModuleType::Oscillator, 1),
            to_module: make_id(ModuleType::Filter, 2),
        }];

        let result = calculate_layout(&modules, &connections, rect);

        let connected_pos_1 = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let disconnected_pos_1 = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 10))
            .unwrap();
        let disconnected_pos_2 = result
            .positions
            .get(&make_id(ModuleType::Euclidean, 11))
            .unwrap();

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
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(
                make_id(ModuleType::Oscillator, 2),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Mixer, 3), ModuleCategory::Mixer),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Mixer, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 2),
                to_module: make_id(ModuleType::Mixer, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc1 = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let osc2 = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 2))
            .unwrap();
        let mixer = result
            .positions
            .get(&make_id(ModuleType::Mixer, 3))
            .unwrap();

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
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 2), ModuleCategory::Filter),
            make_module(make_id(ModuleType::Amplifier, 3), ModuleCategory::Amplifier),
            make_module(make_id(ModuleType::StereoOutput, 4), ModuleCategory::Output),
            make_module(make_id(ModuleType::Envelope, 10), ModuleCategory::Envelope),
            make_module(make_id(ModuleType::Lfo, 11), ModuleCategory::LFO),
        ];
        let connections = vec![
            // Signal chain: OSC → Filter → Amp → Output
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Filter, 2),
                to_module: make_id(ModuleType::Amplifier, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Amplifier, 3),
                to_module: make_id(ModuleType::StereoOutput, 4),
            },
            // ENV → Filter, LFO → Amp
            LayoutConnection {
                from_module: make_id(ModuleType::Envelope, 10),
                to_module: make_id(ModuleType::Filter, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Lfo, 11),
                to_module: make_id(ModuleType::Amplifier, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let flt = result
            .positions
            .get(&make_id(ModuleType::Filter, 2))
            .unwrap();
        let amp = result
            .positions
            .get(&make_id(ModuleType::Amplifier, 3))
            .unwrap();
        let out = result
            .positions
            .get(&make_id(ModuleType::StereoOutput, 4))
            .unwrap();
        let env = result
            .positions
            .get(&make_id(ModuleType::Envelope, 10))
            .unwrap();
        let lfo = result.positions.get(&make_id(ModuleType::Lfo, 11)).unwrap();

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
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(
                make_id(ModuleType::Oscillator, 2),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 3), ModuleCategory::Filter),
            make_module(make_id(ModuleType::Amplifier, 4), ModuleCategory::Amplifier),
            make_module(make_id(ModuleType::StereoOutput, 5), ModuleCategory::Output),
            make_module(make_id(ModuleType::Envelope, 10), ModuleCategory::Envelope),
            make_module(
                make_id(ModuleType::Oscilloscope, 20),
                ModuleCategory::Visualizer,
            ),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 2),
                to_module: make_id(ModuleType::Filter, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Filter, 3),
                to_module: make_id(ModuleType::Amplifier, 4),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Amplifier, 4),
                to_module: make_id(ModuleType::StereoOutput, 5),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Envelope, 10),
                to_module: make_id(ModuleType::Filter, 3),
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
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Filter, 2), ModuleCategory::Filter),
            make_module(make_id(ModuleType::StereoOutput, 3), ModuleCategory::Output),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Filter, 2),
                to_module: make_id(ModuleType::StereoOutput, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let flt = result
            .positions
            .get(&make_id(ModuleType::Filter, 2))
            .unwrap();
        let out = result
            .positions
            .get(&make_id(ModuleType::StereoOutput, 3))
            .unwrap();

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
    fn test_effect_in_signal_chain() {
        let rect = test_rect();
        let modules = vec![
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::Delay, 2), ModuleCategory::Effect),
            make_module(make_id(ModuleType::StereoOutput, 3), ModuleCategory::Output),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Delay, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Delay, 2),
                to_module: make_id(ModuleType::StereoOutput, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let eff = result
            .positions
            .get(&make_id(ModuleType::Delay, 2))
            .unwrap();
        let out = result
            .positions
            .get(&make_id(ModuleType::StereoOutput, 3))
            .unwrap();

        assert!(osc.x < eff.x, "Effect should be right of oscillator");
        assert!(eff.x < out.x, "Output should be right of effect");
    }

    #[test]
    fn test_utility_in_signal_chain() {
        let rect = test_rect();
        let modules = vec![
            make_module(
                make_id(ModuleType::Oscillator, 1),
                ModuleCategory::Oscillator,
            ),
            make_module(make_id(ModuleType::VectorMixer, 2), ModuleCategory::Utility),
            make_module(make_id(ModuleType::StereoOutput, 3), ModuleCategory::Output),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::VectorMixer, 2),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::VectorMixer, 2),
                to_module: make_id(ModuleType::StereoOutput, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        let osc = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        let util = result
            .positions
            .get(&make_id(ModuleType::VectorMixer, 2))
            .unwrap();
        let out = result
            .positions
            .get(&make_id(ModuleType::StereoOutput, 3))
            .unwrap();

        assert!(osc.x < util.x, "Utility should be right of oscillator");
        assert!(util.x < out.x, "Output should be right of utility");
    }

    #[test]
    fn test_no_overlap_mixed_sizes() {
        let rect = test_rect();
        // Simulate realistic mixed sizes: small oscillator, tall envelope, wide mixer
        let modules = vec![
            ModuleInfo {
                id: make_id(ModuleType::Oscillator, 1),
                category: ModuleCategory::Oscillator,
                size: Vec2::new(200.0, 150.0),
            },
            ModuleInfo {
                id: make_id(ModuleType::Oscillator, 2),
                category: ModuleCategory::Oscillator,
                size: Vec2::new(200.0, 260.0),
            },
            ModuleInfo {
                id: make_id(ModuleType::Filter, 3),
                category: ModuleCategory::Filter,
                size: Vec2::new(220.0, 280.0),
            },
            ModuleInfo {
                id: make_id(ModuleType::Amplifier, 4),
                category: ModuleCategory::Amplifier,
                size: Vec2::new(180.0, 200.0),
            },
            ModuleInfo {
                id: make_id(ModuleType::StereoOutput, 5),
                category: ModuleCategory::Output,
                size: Vec2::new(160.0, 120.0),
            },
            ModuleInfo {
                id: make_id(ModuleType::Envelope, 10),
                category: ModuleCategory::Envelope,
                size: Vec2::new(250.0, 360.0),
            },
        ];
        let connections = vec![
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 1),
                to_module: make_id(ModuleType::Filter, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Oscillator, 2),
                to_module: make_id(ModuleType::Filter, 3),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Filter, 3),
                to_module: make_id(ModuleType::Amplifier, 4),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Amplifier, 4),
                to_module: make_id(ModuleType::StereoOutput, 5),
            },
            LayoutConnection {
                from_module: make_id(ModuleType::Envelope, 10),
                to_module: make_id(ModuleType::Filter, 3),
            },
        ];

        let result = calculate_layout(&modules, &connections, rect);

        // Build rects from positions + snapped sizes
        let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();
        let rects: Vec<(ModuleId, Rect)> = result
            .positions
            .iter()
            .map(|(&id, &pos)| {
                let snapped = snap_size_to_grid(sizes[&id]);
                (id, Rect::from_min_size(pos, snapped))
            })
            .collect();

        // Check no pair of modules overlaps
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let (id_a, rect_a) = rects[i];
                let (id_b, rect_b) = rects[j];
                assert!(
                    !rect_a.intersects(rect_b),
                    "Modules {:?} ({:?}) and {:?} ({:?}) overlap!",
                    id_a,
                    rect_a,
                    id_b,
                    rect_b
                );
            }
        }
    }
}
