//! Auto-layout algorithm for organizing modules based on signal flow analysis.
//!
//! This module provides automatic positioning of synth modules using a 5-phase algorithm:
//! 1. **Classify** modules into SignalChain, Modulation, EffectChain, Global, or
//!    Disconnected groups
//! 2. **Topological depth** assignment via cycle-broken Kahn → columns (left-to-right)
//! 3. **Vertical ordering** within columns using multi-sweep median heuristic (reduces crossings)
//! 4. **Modulation placement** one column left of their primary targets (when possible),
//!    sorted deterministically by `(column, target_row, module_sort_key)`
//! 5. **Pixel positions** computed from each module's actual rendered size, snapped up
//!    to whole grid cells, anchored at logical `(GRID, GRID)`. Positions are in the
//!    canvas's logical coordinate system; the surrounding `ScrollArea` grows around
//!    them via `content_size()`, so no screen-space rect is needed here.
//!
//! Layout zones (left→right): Signal columns | Effect-chain column | Global column |
//! Mod Matrix column | Disconnected column. Within a signal column (top→bottom):
//! signal-chain rows, then modulation rows anchored to their target row. The Mod
//! Matrix column is separated from its neighbours by `ZONE_PADDING` so the Mod
//! Matrix and Effect zone background rectangles don't overlap.

use std::collections::{HashMap, HashSet};

use eframe::egui::{Pos2, Vec2};

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

/// A collapsed group treated as a single layout node.
///
/// Use together with [`prepare_layout_inputs`] to fold a collapsed
/// group into one rectangle for layout. The `representative` doubles
/// as the position key in the resulting [`LayoutResult`].
#[derive(Debug, Clone)]
pub(crate) struct CollapsedGroupNode {
    /// Representative `ModuleId` used as the layout node id. Must be
    /// one of `members` — typically the first existing one.
    pub representative: ModuleId,
    /// Category used to classify the group node (signal-chain vs.
    /// modulation vs. global).
    pub category: ModuleCategory,
    /// Rendered size of the collapsed group box.
    pub size: Vec2,
    /// All `ModuleId`s belonging to the group. Hidden members are
    /// removed from the visible set, and connection endpoints that
    /// touch them are rewired to the representative.
    pub members: Vec<ModuleId>,
}

/// Result of auto-layout calculation.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    /// New positions for each module.
    pub positions: HashMap<ModuleId, Pos2>,
}

/// Prepare layout inputs that fold collapsed groups into single nodes.
///
/// - Removes any module whose id is listed as a member of a collapsed
///   group from `modules`.
/// - Inserts one [`ModuleInfo`] per collapsed group, using the group's
///   representative id, collapsed size, and category.
/// - Rewrites connection endpoints that reference a hidden member to
///   reference its group representative.
/// - Drops connections whose endpoints both fall inside the same group.
#[must_use]
pub(crate) fn prepare_layout_inputs(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    collapsed_groups: &[CollapsedGroupNode],
) -> (Vec<ModuleInfo>, Vec<LayoutConnection>) {
    let mut member_to_repr: HashMap<ModuleId, ModuleId> = HashMap::new();
    for group in collapsed_groups {
        for &m in &group.members {
            member_to_repr.insert(m, group.representative);
        }
    }

    let mut out_modules: Vec<ModuleInfo> = modules
        .iter()
        .filter(|m| !member_to_repr.contains_key(&m.id))
        .cloned()
        .collect();
    for group in collapsed_groups {
        out_modules.push(ModuleInfo {
            id: group.representative,
            category: group.category,
            size: group.size,
        });
    }

    let out_connections: Vec<LayoutConnection> = connections
        .iter()
        .filter_map(|c| {
            let from = member_to_repr
                .get(&c.from_module)
                .copied()
                .unwrap_or(c.from_module);
            let to = member_to_repr
                .get(&c.to_module)
                .copied()
                .unwrap_or(c.to_module);
            if from == to {
                None
            } else {
                Some(LayoutConnection {
                    from_module: from,
                    to_module: to,
                })
            }
        })
        .collect();

    (out_modules, out_connections)
}

// ── Constants ──────────────────────────────────────────────────────────────

/// Grid cell size — re-exported from `patch_editor::GRID_SIZE`.
const GRID: f32 = super::patch_editor::GRID_SIZE;

/// Extra gap between modules (1 grid cell).
const GAP: f32 = GRID;

/// Extra horizontal space between zone-bearing columns. Each zone rectangle
/// adds `GRID` of padding around its content (see `draw_module_zone` in
/// `patch_editor`); using `2 * GRID` here leaves one clear grid cell between
/// neighbouring zone rectangles instead of letting them touch on the same x.
const ZONE_PADDING: f32 = 2.0 * GRID;

/// Number of ordering sweeps for crossing reduction.
const MAX_SWEEPS: usize = 8;

// ── Internal types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleGroup {
    SignalChain,
    Modulation,
    /// Engine effect-chain modules (e.g. bus effects without voice-graph cables).
    /// Placed in a dedicated column ordered by the engine's effect chain order.
    EffectChain,
    Global,
    /// Mod Matrix modules. Pinned to their own column with `ZONE_PADDING` so
    /// the Mod Matrix zone rectangle doesn't overlap the Effect zone.
    ModMatrix,
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
    // Mod Matrix routes via parameter slots rather than cables, so it never
    // has voice-graph connections of its own. Pin it to its own padded column
    // regardless of connectivity so the Mod Matrix zone rectangle stays clear
    // of the Effect zone.
    if module_id.module_type == ModuleType::ModMatrix {
        return ModuleGroup::ModMatrix;
    }

    if !(has_incoming || has_outgoing) {
        // Effects without voice-graph cables belong to the engine effect chain,
        // not the truly disconnected zone.
        if category == ModuleCategory::Effect {
            return ModuleGroup::EffectChain;
        }
        // Aux visualizers (Oscilloscope, LevelMeter, SpectrumAnalyzer) sit
        // next to the effect chain even without cables — the Effect zone
        // wraps them by category, so placing them far right in Disconnected
        // would stretch that zone across the whole canvas.
        if matches!(
            category,
            ModuleCategory::Visualizer | ModuleCategory::Utility
        ) && is_global_aux_type(module_id.module_type)
        {
            return ModuleGroup::Global;
        }
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
        ModuleType::Oscilloscope | ModuleType::LevelMeter | ModuleType::SpectrumAnalyzer
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

/// Final placement decision for a single modulation module.
///
/// Iteration order of a `Vec<ModPlacement>` is the deterministic ordering of
/// modulators within the final layout. Sort key is `(column, target_row.unwrap_or(MAX),
/// module_sort_key)` so that modulators with a real target row stack
/// top-to-bottom in target-row order, with `module_sort_key` breaking ties.
#[derive(Debug, Clone, Copy)]
struct ModPlacement {
    module_id: ModuleId,
    column: usize,
    target_row: Option<usize>,
}

/// Place modulation modules one column left of their primary target when possible.
///
/// Returns a vector sorted by `(column, target_row, module_sort_key)`. Iterating
/// in order yields a deterministic top-to-bottom stack within each column.
fn place_modulation(
    mod_modules: &[ModuleId],
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    signal_depth: &HashMap<ModuleId, usize>,
    signal_columns: &HashMap<usize, Vec<ModuleId>>,
) -> Vec<ModPlacement> {
    let mut placements: Vec<ModPlacement> = Vec::with_capacity(mod_modules.len());

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

        // Find the target's row position for sorting. `None` if the module has
        // no signal-chain target — those land after all row-anchored modulators.
        let target_row = outgoing.get(&mod_id).and_then(|targets| {
            targets.iter().find_map(|t| {
                let col = signal_depth.get(t).copied()?;
                if col == target_col {
                    let modules_in_col = signal_columns.get(&col)?;
                    modules_in_col.iter().position(|&id| id == *t)
                } else {
                    None
                }
            })
        });

        placements.push(ModPlacement {
            module_id: mod_id,
            column: mod_col,
            target_row,
        });
    }

    placements.sort_by(|a, b| {
        a.column.cmp(&b.column).then_with(|| {
            // Anchored modulators (Some) sort above unanchored (None).
            match (a.target_row, b.target_row) {
                (Some(ar), Some(br)) => ar.cmp(&br),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| module_sort_key(&a.module_id).cmp(&module_sort_key(&b.module_id)))
        })
    });

    placements
}

// ── Main layout function ───────────────────────────────────────────────────

/// Calculate automatic layout for modules based on signal flow analysis.
///
/// Layout rules, left → right:
/// 1. **Signal-chain** modules: one column per topological depth, in cumulative
///    x-order. Modulation modules attached to a column stack underneath that
///    column's signal modules (one column left of their primary target when
///    possible).
/// 2. **Effect-chain** modules: a single vertical column right of the signal
///    zone, ordered top→bottom by `effect_chain_order` (engine processing
///    order). Effects with no voice-graph cables live here, not in the
///    Disconnected zone.
/// 3. **Global** modules (aux sinks like Oscilloscope, LevelMeter,
///    SpectrumAnalyzer): one vertical column right of the effect-chain zone.
/// 4. **Mod Matrix** modules: one vertical column right of global, with
///    `ZONE_PADDING` extra space on each side so the Mod Matrix zone rectangle
///    doesn't overlap the Effect zone.
/// 5. **Disconnected** modules: the rightmost column.
pub fn calculate_layout(modules: &[ModuleInfo], connections: &[LayoutConnection]) -> LayoutResult {
    calculate_layout_with_chain_order(modules, connections, &[])
}

/// Calculate layout with an explicit effect-chain ordering.
///
/// `effect_chain_order` is the engine's effect processing order. When non-empty,
/// it controls top→bottom placement in **both** the effect-chain and global
/// zones; modules absent from the list are appended in their classification
/// order. Pass `&[]` to fall back to classification order alone, which is what
/// [`calculate_layout`] does.
pub fn calculate_layout_with_chain_order(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
    effect_chain_order: &[ModuleId],
) -> LayoutResult {
    let mut result = LayoutResult::default();

    if modules.is_empty() {
        return result;
    }

    // ── Phase 1: Classify modules ──────────────────────────────────────

    let mut signal_ids: Vec<ModuleId> = Vec::new();
    let mut mod_ids: Vec<ModuleId> = Vec::new();
    let mut effect_chain_ids: Vec<ModuleId> = Vec::new();
    let mut global_ids: Vec<ModuleId> = Vec::new();
    let mut mod_matrix_ids: Vec<ModuleId> = Vec::new();
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
            ModuleGroup::EffectChain => effect_chain_ids.push(module.id),
            ModuleGroup::Global => global_ids.push(module.id),
            ModuleGroup::ModMatrix => mod_matrix_ids.push(module.id),
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

    let mod_placements = place_modulation(&mod_ids, &outgoing_full, &signal_depth, &columns);

    // ── Phase 5: Size-aware pixel positions ────────────────────────────

    // Build size lookup from input modules
    let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();

    let num_signal_columns = columns.keys().copied().max().map_or(0, |m| m + 1);

    // Module positions are in logical canvas coordinates. Always start at
    // (GRID, GRID); the surrounding `ScrollArea` grows around the resulting
    // content via `content_size()`.
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
    for placement in &mod_placements {
        let x = col_x.get(placement.column).copied().unwrap_or(start_x);
        let y = col_bottom.get(placement.column).copied().unwrap_or(start_y);
        let snapped = snap_size_to_grid(
            sizes
                .get(&placement.module_id)
                .copied()
                .unwrap_or(DEFAULT_SIZE),
        );
        result
            .positions
            .insert(placement.module_id, Pos2::new(x, y));
        if let Some(bottom) = col_bottom.get_mut(placement.column) {
            *bottom = y + snapped.y + GAP;
        }
    }

    // Extra zones, left→right: effect-chain | global | disconnected.
    let signal_end_x = if num_signal_columns > 0 {
        col_x[num_signal_columns - 1] + col_widths[num_signal_columns - 1]
    } else {
        start_x
    };

    let mut current_x = signal_end_x;
    current_x += place_vertical_column(
        &mut effect_chain_ids,
        effect_chain_order,
        current_x,
        start_y,
        &sizes,
        &mut result.positions,
    );
    // EffectChain and Global panels share the Effect zone rectangle, so they
    // sit in adjacent columns without extra padding.
    current_x += place_vertical_column(
        &mut global_ids,
        effect_chain_order,
        current_x,
        start_y,
        &sizes,
        &mut result.positions,
    );
    // Mod Matrix lives in its own zone — pad both sides so the surrounding
    // zone rectangle has clear room next to the Effect zone (and next to
    // anything that may follow on the right).
    let has_mod_matrix = !mod_matrix_ids.is_empty();
    if has_mod_matrix {
        current_x += ZONE_PADDING;
    }
    current_x += place_vertical_column(
        &mut mod_matrix_ids,
        &[],
        current_x,
        start_y,
        &sizes,
        &mut result.positions,
    );
    if has_mod_matrix {
        current_x += ZONE_PADDING;
    }
    let _ = place_vertical_column(
        &mut disconnected_ids,
        &[],
        current_x,
        start_y,
        &sizes,
        &mut result.positions,
    );

    result
}

/// Place a vertical stack of modules at `x`, optionally sorted by `chain_order`.
///
/// Returns the x-advance for the next column (max snapped width + GAP), or 0 if
/// the input is empty so callers can chain `current_x += …` unconditionally.
fn place_vertical_column(
    ids: &mut [ModuleId],
    chain_order: &[ModuleId],
    x: f32,
    start_y: f32,
    sizes: &HashMap<ModuleId, Vec2>,
    positions: &mut HashMap<ModuleId, Pos2>,
) -> f32 {
    if ids.is_empty() {
        return 0.0;
    }
    if !chain_order.is_empty() {
        ids.sort_by_key(|id| {
            chain_order
                .iter()
                .position(|chain_id| chain_id == id)
                .unwrap_or(usize::MAX)
        });
    }
    let mut y = start_y;
    let mut max_w = 0.0_f32;
    for &id in &*ids {
        let snapped = snap_size_to_grid(sizes.get(&id).copied().unwrap_or(DEFAULT_SIZE));
        positions.insert(id, Pos2::new(x, y));
        y += snapped.y + GAP;
        max_w = max_w.max(snapped.x);
    }
    max_w + GAP
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
    use eframe::egui::Rect;

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

    #[test]
    fn test_empty_layout() {
        let result = calculate_layout(&[], &[]);
        assert!(result.positions.is_empty());
    }

    #[test]
    fn test_single_disconnected_module() {
        let modules = vec![make_module(
            make_id(ModuleType::Oscillator, 1),
            ModuleCategory::Oscillator,
        )];
        let result = calculate_layout(&modules, &[]);

        let pos = result
            .positions
            .get(&make_id(ModuleType::Oscillator, 1))
            .unwrap();
        assert_eq!(pos.x, GRID);
        assert_eq!(pos.y, GRID);
    }

    #[test]
    fn test_linear_chain_within_bounds() {
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

        let result = calculate_layout(&modules, &connections);

        for pos in result.positions.values() {
            assert!(pos.x >= GRID);
            assert!(pos.y >= GRID);
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

        let result = calculate_layout(&modules, &connections);

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

        for pos in result.positions.values() {
            assert!(pos.x >= GRID);
            assert!(pos.y >= GRID);
        }
    }

    #[test]
    fn test_disconnected_modules_in_corner() {
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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);
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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);

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

        let result = calculate_layout(&modules, &connections);

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

    // ── Phase 1 regression tests (added for auto-layout improvement plan) ──

    /// Phase 2: modulation modules targeting different rows in the same
    /// column should stack top-to-bottom in target-row order.
    #[test]
    fn test_modulators_follow_target_row_order() {
        let osc1 = make_id(ModuleType::Oscillator, 1);
        let osc2 = make_id(ModuleType::Oscillator, 2);
        let osc3 = make_id(ModuleType::Oscillator, 3);
        let f1 = make_id(ModuleType::Filter, 1);
        let f2 = make_id(ModuleType::Filter, 2);
        let f3 = make_id(ModuleType::Filter, 3);
        let mix = make_id(ModuleType::Mixer, 1);
        // Instance numbers chosen so module_sort_key is the OPPOSITE of target
        // row order — ensures the test fails if placement falls back to
        // module_sort_key instead of honoring target row.
        let env_top = make_id(ModuleType::Envelope, 9); // targets f1 (row 0)
        let env_mid = make_id(ModuleType::Envelope, 5); // targets f2 (row 1)
        let env_bot = make_id(ModuleType::Envelope, 1); // targets f3 (row 2)

        let modules = vec![
            make_module(osc1, ModuleCategory::Oscillator),
            make_module(osc2, ModuleCategory::Oscillator),
            make_module(osc3, ModuleCategory::Oscillator),
            make_module(f1, ModuleCategory::Filter),
            make_module(f2, ModuleCategory::Filter),
            make_module(f3, ModuleCategory::Filter),
            make_module(mix, ModuleCategory::Mixer),
            make_module(env_top, ModuleCategory::Envelope),
            make_module(env_mid, ModuleCategory::Envelope),
            make_module(env_bot, ModuleCategory::Envelope),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: osc1,
                to_module: f1,
            },
            LayoutConnection {
                from_module: osc2,
                to_module: f2,
            },
            LayoutConnection {
                from_module: osc3,
                to_module: f3,
            },
            LayoutConnection {
                from_module: f1,
                to_module: mix,
            },
            LayoutConnection {
                from_module: f2,
                to_module: mix,
            },
            LayoutConnection {
                from_module: f3,
                to_module: mix,
            },
            LayoutConnection {
                from_module: env_top,
                to_module: f1,
            },
            LayoutConnection {
                from_module: env_mid,
                to_module: f2,
            },
            LayoutConnection {
                from_module: env_bot,
                to_module: f3,
            },
        ];

        let result = calculate_layout(&modules, &connections);

        let top = *result.positions.get(&env_top).unwrap();
        let mid = *result.positions.get(&env_mid).unwrap();
        let bot = *result.positions.get(&env_bot).unwrap();

        // All three modulators are one column left of the filters → share x.
        assert!(
            (top.x - mid.x).abs() < 1.0 && (mid.x - bot.x).abs() < 1.0,
            "modulators should share a column: top.x={}, mid.x={}, bot.x={}",
            top.x,
            mid.x,
            bot.x
        );

        // Y-order matches target row order: env_top (row 0) above env_mid (row 1) above env_bot (row 2).
        assert!(
            top.y < mid.y,
            "env_top (target row 0) should sit above env_mid (target row 1): top.y={}, mid.y={}",
            top.y,
            mid.y
        );
        assert!(
            mid.y < bot.y,
            "env_mid (target row 1) should sit above env_bot (target row 2): mid.y={}, bot.y={}",
            mid.y,
            bot.y
        );
    }

    /// Phase 2: when multiple modulators target the same row, the tie-break
    /// must be `module_sort_key` (deterministic), not `HashMap` iteration.
    #[test]
    fn test_modulators_same_target_row_tie_break() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let flt = make_id(ModuleType::Filter, 1);
        // Same module_type so the sort key differs only on `instance`.
        let env_a = make_id(ModuleType::Envelope, 1); // lower sort key
        let env_b = make_id(ModuleType::Envelope, 9); // higher sort key

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(flt, ModuleCategory::Filter),
            make_module(env_a, ModuleCategory::Envelope),
            make_module(env_b, ModuleCategory::Envelope),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: osc,
                to_module: flt,
            },
            LayoutConnection {
                from_module: env_a,
                to_module: flt,
            },
            LayoutConnection {
                from_module: env_b,
                to_module: flt,
            },
        ];

        let result = calculate_layout(&modules, &connections);
        let a = *result.positions.get(&env_a).unwrap();
        let b = *result.positions.get(&env_b).unwrap();

        // Same column.
        assert!(
            (a.x - b.x).abs() < 1.0,
            "envs should share a column: a.x={}, b.x={}",
            a.x,
            b.x
        );
        // env_a (lower module_sort_key) sits above env_b — deterministic tie-break.
        assert!(
            a.y < b.y,
            "env_a (sort key {:?}) should sit above env_b (sort key {:?}): a.y={}, b.y={}",
            module_sort_key(&env_a),
            module_sort_key(&env_b),
            a.y,
            b.y
        );
    }

    /// Effect-chain modules with cables plus disconnected modules: no overlap,
    /// effects retain signal-chain order. This passes on current code and
    /// guards against regressions in later phases.
    #[test]
    fn test_effects_in_chain_plus_disconnected_no_overlap() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let delay = make_id(ModuleType::Delay, 1);
        let reverb = make_id(ModuleType::Reverb, 1);
        let out = make_id(ModuleType::StereoOutput, 1);
        let disc_osc = make_id(ModuleType::Oscillator, 10);
        let disc_flt = make_id(ModuleType::Filter, 10);

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(delay, ModuleCategory::Effect),
            make_module(reverb, ModuleCategory::Effect),
            make_module(out, ModuleCategory::Output),
            make_module(disc_osc, ModuleCategory::Oscillator),
            make_module(disc_flt, ModuleCategory::Filter),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: osc,
                to_module: delay,
            },
            LayoutConnection {
                from_module: delay,
                to_module: reverb,
            },
            LayoutConnection {
                from_module: reverb,
                to_module: out,
            },
        ];

        let result = calculate_layout(&modules, &connections);

        // No pair of modules overlaps (snapped rects).
        let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();
        let rects: Vec<(ModuleId, Rect)> = result
            .positions
            .iter()
            .map(|(&id, &pos)| {
                let snapped = snap_size_to_grid(sizes[&id]);
                (id, Rect::from_min_size(pos, snapped))
            })
            .collect();
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

        // Effects keep signal-chain order: delay left of reverb.
        let delay_pos = *result.positions.get(&delay).unwrap();
        let reverb_pos = *result.positions.get(&reverb).unwrap();
        assert!(
            delay_pos.x < reverb_pos.x,
            "Delay should sit left of Reverb in the signal chain"
        );
    }

    /// Phase 3: Effect-chain modules with no graph cables should land in the
    /// effect-chain zone in chain order — not in the Disconnected column.
    #[test]
    fn test_effects_without_cables_classified_as_effect_chain() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let out = make_id(ModuleType::StereoOutput, 1);
        let delay = make_id(ModuleType::Delay, 1);
        let reverb = make_id(ModuleType::Reverb, 1);
        // Truly disconnected non-effect module — should land in the
        // rightmost (Disconnected) column.
        let disc = make_id(ModuleType::Oscillator, 99);

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(out, ModuleCategory::Output),
            make_module(delay, ModuleCategory::Effect),
            make_module(reverb, ModuleCategory::Effect),
            make_module(disc, ModuleCategory::Oscillator),
        ];
        let connections = vec![LayoutConnection {
            from_module: osc,
            to_module: out,
        }];
        let chain_order = vec![reverb, delay]; // chain order is reverb-first

        let result = calculate_layout_with_chain_order(&modules, &connections, &chain_order);

        let delay_pos = *result.positions.get(&delay).unwrap();
        let reverb_pos = *result.positions.get(&reverb).unwrap();
        let disc_pos = *result.positions.get(&disc).unwrap();
        let out_pos = *result.positions.get(&out).unwrap();

        // Effects share a single x column.
        assert!(
            (delay_pos.x - reverb_pos.x).abs() < 1.0,
            "Effects should share the effect-chain column: delay.x={}, reverb.x={}",
            delay_pos.x,
            reverb_pos.x
        );

        // Effects appear in chain order top-to-bottom: reverb above delay.
        assert!(
            reverb_pos.y < delay_pos.y,
            "Effects should be in chain order: reverb above delay"
        );

        // Effects sit to the right of the signal chain, but left of truly disconnected modules.
        assert!(
            delay_pos.x > out_pos.x,
            "Effects should be right of the signal chain"
        );
        assert!(
            disc_pos.x > delay_pos.x,
            "Disconnected modules should be right of the effect-chain column"
        );
    }

    /// Phase 5: `prepare_layout_inputs` folds collapsed groups into a
    /// single layout node — hidden members vanish, internal connections
    /// drop, external connections retarget the representative.
    #[test]
    fn test_prepare_layout_inputs_folds_collapsed_groups() {
        let osc1 = make_id(ModuleType::Oscillator, 1);
        let osc2 = make_id(ModuleType::Oscillator, 2);
        let flt = make_id(ModuleType::Filter, 1);
        let amp = make_id(ModuleType::Amplifier, 1);
        let out = make_id(ModuleType::StereoOutput, 1);

        let modules = vec![
            make_module(osc1, ModuleCategory::Oscillator),
            make_module(osc2, ModuleCategory::Oscillator),
            make_module(flt, ModuleCategory::Filter),
            make_module(amp, ModuleCategory::Amplifier),
            make_module(out, ModuleCategory::Output),
        ];
        let connections = vec![
            // External: osc1 -> flt
            LayoutConnection {
                from_module: osc1,
                to_module: flt,
            },
            // Internal to the collapsed group (flt -> amp): dropped
            LayoutConnection {
                from_module: flt,
                to_module: amp,
            },
            // External: amp -> out (rewritten to flt -> out)
            LayoutConnection {
                from_module: amp,
                to_module: out,
            },
            // External: osc2 -> amp (rewritten to osc2 -> flt)
            LayoutConnection {
                from_module: osc2,
                to_module: amp,
            },
        ];

        let group_node = CollapsedGroupNode {
            representative: flt,
            category: ModuleCategory::Mixer,
            size: Vec2::new(300.0, 200.0),
            members: vec![flt, amp],
        };

        let (out_modules, out_connections) =
            prepare_layout_inputs(&modules, &connections, &[group_node]);

        // Group members (flt, amp) replaced by a single representative (flt).
        let ids: HashSet<ModuleId> = out_modules.iter().map(|m| m.id).collect();
        assert!(ids.contains(&osc1));
        assert!(ids.contains(&osc2));
        assert!(ids.contains(&out));
        assert!(ids.contains(&flt));
        assert!(!ids.contains(&amp), "hidden member amp must be removed");
        assert_eq!(out_modules.len(), 4);

        // Representative entry uses the collapsed size + category from the node.
        let repr = out_modules.iter().find(|m| m.id == flt).unwrap();
        assert!((repr.size.x - 300.0).abs() < f32::EPSILON);
        assert!((repr.size.y - 200.0).abs() < f32::EPSILON);
        assert_eq!(repr.category, ModuleCategory::Mixer);

        // Connections involving hidden members get rewired to the representative;
        // the internal flt->amp self-loop is dropped.
        let pairs: HashSet<(ModuleId, ModuleId)> = out_connections
            .iter()
            .map(|c| (c.from_module, c.to_module))
            .collect();
        assert!(pairs.contains(&(osc1, flt)));
        assert!(
            pairs.contains(&(flt, out)),
            "amp->out should rewrite to flt->out"
        );
        assert!(
            pairs.contains(&(osc2, flt)),
            "osc2->amp should rewrite to osc2->flt"
        );
        assert_eq!(out_connections.len(), 3);
    }

    /// Phase 5: a collapsed group composed of effect modules is folded
    /// into a single node and `calculate_layout_with_chain_order` places
    /// it as part of the signal chain — hidden members are absent from
    /// the result, and the group node takes one rectangle.
    #[test]
    fn test_calculate_layout_with_collapsed_group_node() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let delay = make_id(ModuleType::Delay, 1);
        let reverb = make_id(ModuleType::Reverb, 1);
        let out = make_id(ModuleType::StereoOutput, 1);

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(delay, ModuleCategory::Effect),
            make_module(reverb, ModuleCategory::Effect),
            make_module(out, ModuleCategory::Output),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: osc,
                to_module: delay,
            },
            LayoutConnection {
                from_module: delay,
                to_module: reverb,
            },
            LayoutConnection {
                from_module: reverb,
                to_module: out,
            },
        ];

        // Delay + Reverb are inside a collapsed group; delay is the representative.
        let collapsed = vec![CollapsedGroupNode {
            representative: delay,
            category: ModuleCategory::Effect,
            size: Vec2::new(320.0, 240.0),
            members: vec![delay, reverb],
        }];

        let (folded_modules, folded_conns) =
            prepare_layout_inputs(&modules, &connections, &collapsed);

        let result = calculate_layout_with_chain_order(&folded_modules, &folded_conns, &[]);

        // Reverb (hidden member) must not appear; delay (representative) must.
        assert!(
            !result.positions.contains_key(&reverb),
            "hidden member reverb should not get a position"
        );
        assert!(
            result.positions.contains_key(&delay),
            "representative delay should get a position"
        );

        let osc_pos = *result.positions.get(&osc).unwrap();
        let group_pos = *result.positions.get(&delay).unwrap();
        let out_pos = *result.positions.get(&out).unwrap();
        // Group sits between osc and out in the signal chain.
        assert!(
            osc_pos.x < group_pos.x,
            "collapsed group must sit right of upstream osc"
        );
        assert!(
            group_pos.x < out_pos.x,
            "collapsed group must sit left of downstream out"
        );
    }

    /// Module positions are in the canvas's logical coordinate system and
    /// always anchored at `(GRID, GRID)`. The surrounding `ScrollArea` grows
    /// around the result via `content_size()`, so auto-layout takes no rect.
    #[test]
    fn test_layout_origin_is_grid_grid() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let out = make_id(ModuleType::StereoOutput, 1);
        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(out, ModuleCategory::Output),
        ];
        let connections = vec![LayoutConnection {
            from_module: osc,
            to_module: out,
        }];

        let result = calculate_layout(&modules, &connections);

        let leftmost_x = result
            .positions
            .values()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min);
        let topmost_y = result
            .positions
            .values()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min);
        assert_eq!(leftmost_x, GRID, "leftmost x must equal GRID");
        assert_eq!(topmost_y, GRID, "topmost y must equal GRID");
    }

    /// Mod Matrix sits in its own padded column so the Mod Matrix and Effect
    /// zone background rectangles can each have their `GRID`-wide padding
    /// without overlapping. Verifies the horizontal distance between the
    /// Effect/Global column's right edge and the Mod Matrix column's left
    /// edge is at least `GAP + ZONE_PADDING` (= 2*GRID).
    #[test]
    fn test_mod_matrix_column_padded_from_effect_zone() {
        let osc = make_id(ModuleType::Oscillator, 1);
        let out = make_id(ModuleType::StereoOutput, 1);
        let delay = make_id(ModuleType::Delay, 1);
        let scope = make_id(ModuleType::Oscilloscope, 1);
        let matrix = make_id(ModuleType::ModMatrix, 1);

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(out, ModuleCategory::Output),
            make_module(delay, ModuleCategory::Effect),
            make_module(scope, ModuleCategory::Visualizer),
            make_module(matrix, ModuleCategory::Utility),
        ];
        let connections = vec![LayoutConnection {
            from_module: osc,
            to_module: out,
        }];

        let result = calculate_layout(&modules, &connections);

        let delay_pos = *result.positions.get(&delay).unwrap();
        let scope_pos = *result.positions.get(&scope).unwrap();
        let matrix_pos = *result.positions.get(&matrix).unwrap();

        let effect_zone_right = (delay_pos.x + TEST_SIZE.x).max(scope_pos.x + TEST_SIZE.x);
        assert!(
            matrix_pos.x - effect_zone_right >= GAP + ZONE_PADDING - 0.5,
            "Mod Matrix column must clear the Effect zone by at least GAP+ZONE_PADDING (got {})",
            matrix_pos.x - effect_zone_right
        );
    }
}
