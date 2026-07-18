//! Auto-layout algorithm for organizing modules based on signal flow analysis.
//!
//! This module positions synth modules with a layered (Sugiyama-style) algorithm:
//! 1. **Classify** modules into SignalChain, Modulation, EffectChain, Global, or
//!    Disconnected groups. The first two form the unified "voice graph".
//! 2. **Layer assignment** over the voice graph (signal + modulation as one DAG) by
//!    longest-path-to-sink: `column = max_dist − dist_to_sink`, so each module sits
//!    one column left of its nearest consumer and a modulation chain feeding the
//!    signal path gets its own successive columns. The column count is dynamic.
//! 3. **Vertical ordering** within columns using a multi-sweep median heuristic
//!    (reduces edge crossings); signal and modulation modules are ordered together.
//! 4. **Pixel positions** computed from each module's actual rendered size, snapped up
//!    to whole grid cells, anchored at logical `(GRID, GRID)`. Each column's x-advance
//!    is its widest module + GAP. Positions are in the canvas's logical coordinate
//!    system; the surrounding `ScrollArea` grows around them via `content_size()`.
//!
//! Layout zones (left→right): Voice columns | Effect-chain column | Global column |
//! Mod Matrix column | Disconnected column. The Mod Matrix column is separated from
//! its neighbours by `ZONE_PADDING` so the Mod Matrix and Effect zone background
//! rectangles don't overlap.

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

/// Lay out an arbitrary node graph with the Rack's Sugiyama flow algorithm,
/// without applying any Rack-specific category zones.
///
/// Every supplied node participates in longest-path layer assignment and the
/// median crossing-reduction sweeps. Pixel placement uses each node's measured
/// size, so callers such as Note Grid and Mod Grid get a free flow layout while
/// sharing Rack's graph analysis.
#[must_use]
pub(crate) fn calculate_free_flow_layout(
    modules: &[ModuleInfo],
    connections: &[LayoutConnection],
) -> LayoutResult {
    let ids: Vec<ModuleId> = modules.iter().map(|module| module.id).collect();
    let id_set: HashSet<ModuleId> = ids.iter().copied().collect();
    let mut outgoing: HashMap<ModuleId, Vec<ModuleId>> =
        ids.iter().map(|&id| (id, Vec::new())).collect();
    let mut incoming: HashMap<ModuleId, Vec<ModuleId>> =
        ids.iter().map(|&id| (id, Vec::new())).collect();
    for connection in connections {
        if id_set.contains(&connection.from_module) && id_set.contains(&connection.to_module) {
            outgoing
                .entry(connection.from_module)
                .or_default()
                .push(connection.to_module);
            incoming
                .entry(connection.to_module)
                .or_default()
                .push(connection.from_module);
        }
    }

    let depth = assign_layers(&ids, &outgoing, &incoming);
    let mut columns = build_columns(&depth, &ids);
    order_within_columns(&mut columns, &outgoing, &incoming, &depth);
    let sizes: HashMap<ModuleId, Vec2> = modules
        .iter()
        .map(|module| (module.id, module.size))
        .collect();
    let num_columns = columns.keys().copied().max().map_or(0, |max| max + 1);
    let mut col_widths = vec![0.0_f32; num_columns];
    for (&column, module_ids) in &columns {
        let width = module_ids
            .iter()
            .map(|id| snap_size_to_grid(sizes.get(id).copied().unwrap_or(DEFAULT_SIZE)).x)
            .fold(0.0_f32, f32::max);
        if let Some(slot) = col_widths.get_mut(column) {
            *slot = width + GAP;
        }
    }
    let mut col_x = vec![GRID; num_columns];
    for column in 1..num_columns {
        col_x[column] = col_x[column - 1] + col_widths[column - 1];
    }

    let mut result = LayoutResult::default();
    for column in 0..num_columns {
        let Some(module_ids) = columns.get(&column) else {
            continue;
        };
        let mut y = GRID;
        for &id in module_ids {
            let size = snap_size_to_grid(sizes.get(&id).copied().unwrap_or(DEFAULT_SIZE));
            result.positions.insert(id, Pos2::new(col_x[column], y));
            y += size.y + GAP;
        }
    }
    result
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

/// Grid cell size — the shared canvas layer's `GRID_SIZE`.
const GRID: f32 = super::scene_canvas::GRID_SIZE;

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

/// Assign each voice module to a column (layer) by **longest path to a sink**.
///
/// `dist_to_sink(v)` is the length of the longest directed path from `v` to any
/// sink (a node with no outgoing voice edge); the column is `max_dist −
/// dist_to_sink(v)`, so sinks (e.g. the Output) land in the rightmost column and
/// every node sits exactly one column left of its nearest consumer. This is the
/// standard layered-graph-drawing "longest-path" ranking (Sugiyama framework),
/// ranked toward the sink so sources hug what they feed.
///
/// Computed over the reverse topological order so every successor is resolved
/// before the node; nodes left out of the topo order by a cycle keep `dist = 0`.
fn assign_layers(
    voice_ids: &[ModuleId],
    outgoing: &HashMap<ModuleId, Vec<ModuleId>>,
    incoming: &HashMap<ModuleId, Vec<ModuleId>>,
) -> HashMap<ModuleId, usize> {
    let id_set: HashSet<ModuleId> = voice_ids.iter().copied().collect();
    let topo_order = topological_sort_kahn(voice_ids, outgoing, incoming);

    let mut dist: HashMap<ModuleId, usize> = voice_ids.iter().map(|&id| (id, 0)).collect();
    for &node in topo_order.iter().rev() {
        let mut d = 0;
        if let Some(neighbors) = outgoing.get(&node) {
            for &neighbor in neighbors {
                if id_set.contains(&neighbor) {
                    d = d.max(dist.get(&neighbor).copied().unwrap_or(0) + 1);
                }
            }
        }
        dist.insert(node, d);
    }

    let max_dist = dist.values().copied().max().unwrap_or(0);
    let mut depth: HashMap<ModuleId, usize> =
        dist.into_iter().map(|(id, d)| (id, max_dist - d)).collect();
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

// ── Main layout function ───────────────────────────────────────────────────

/// Calculate automatic layout for modules based on signal flow analysis.
///
/// Layout rules, left → right:
/// 1. **Voice-graph** modules (signal-chain + modulation): laid out together as
///    one DAG by longest-path-to-sink layering (Sugiyama), one column per layer
///    in cumulative x-order. Each module sits one column left of its nearest
///    consumer, so a modulation chain feeding the signal path gets its own
///    successive columns and the column count is dynamic.
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

    // ── Build the voice-graph adjacency ────────────────────────────────
    //
    // Signal-chain and modulation modules are laid out together as one DAG
    // (Phase 2), so a modulation chain feeding the signal path gets its own
    // successive columns instead of collapsing into one. Effect-chain, global,
    // mod-matrix and disconnected modules live in their own zones to the right
    // and are excluded from this graph.
    let voice_ids: Vec<ModuleId> = signal_ids.iter().chain(mod_ids.iter()).copied().collect();
    let voice_set: HashSet<ModuleId> = voice_ids.iter().copied().collect();

    let mut outgoing_voice: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    let mut incoming_voice: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
    for &id in &voice_ids {
        outgoing_voice.entry(id).or_default();
        incoming_voice.entry(id).or_default();
    }
    for conn in connections {
        if voice_set.contains(&conn.from_module) && voice_set.contains(&conn.to_module) {
            outgoing_voice
                .entry(conn.from_module)
                .or_default()
                .push(conn.to_module);
            incoming_voice
                .entry(conn.to_module)
                .or_default()
                .push(conn.from_module);
        }
    }

    // ── Phase 2: Layer assignment (longest path to sink) ───────────────
    //
    // Each node lands one column left of its nearest consumer, so audio sources
    // hug what they feed and modulation chains extend leftward column by column.
    // Output (a sink) ends up in the rightmost column. The number of columns is
    // therefore dynamic — as deep as the longest signal-or-modulation chain.
    let depth = assign_layers(&voice_ids, &outgoing_voice, &incoming_voice);

    let mut columns = build_columns(&depth, &voice_ids);

    // ── Phase 3: Vertical ordering (crossing minimization) ─────────────

    order_within_columns(&mut columns, &outgoing_voice, &incoming_voice, &depth);

    // ── Phase 4: Size-aware pixel positions ────────────────────────────

    // Build size lookup from input modules.
    let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();

    let num_columns = columns.keys().copied().max().map_or(0, |m| m + 1);

    // Module positions are in logical canvas coordinates. Always start at
    // (GRID, GRID); the surrounding `ScrollArea` grows around the resulting
    // content via `content_size()`.
    let start_x = GRID;
    let start_y = GRID;

    // Column width = widest snapped module in the column (signal OR modulation,
    // since both share these columns now) + GAP. Cumulative x per column.
    let mut col_widths: Vec<f32> = vec![0.0; num_columns];
    for (&col, module_ids) in &columns {
        let max_w = module_ids
            .iter()
            .map(|id| snap_size_to_grid(sizes.get(id).copied().unwrap_or(DEFAULT_SIZE)).x)
            .fold(0.0_f32, f32::max);
        if let Some(w) = col_widths.get_mut(col) {
            *w = max_w + GAP;
        }
    }
    let mut col_x: Vec<f32> = vec![start_x; num_columns];
    for c in 1..num_columns {
        col_x[c] = col_x[c - 1] + col_widths[c - 1];
    }

    // Place every module at its column's x, stacked vertically in row order.
    for col in 0..num_columns {
        if let Some(module_ids) = columns.get(&col) {
            let mut y = start_y;
            for &module_id in module_ids {
                let snapped =
                    snap_size_to_grid(sizes.get(&module_id).copied().unwrap_or(DEFAULT_SIZE));
                result.positions.insert(module_id, Pos2::new(col_x[col], y));
                y += snapped.y + GAP;
            }
        }
    }

    // Extra zones, left→right: effect-chain | global | disconnected.
    let voice_end_x = if num_columns > 0 {
        col_x[num_columns - 1] + col_widths[num_columns - 1]
    } else {
        start_x
    };

    let mut current_x = voice_end_x;
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
    fn free_flow_uses_connections_and_measured_widths() {
        let source = make_id(ModuleType::Oscillator, 1);
        let middle = make_id(ModuleType::Oscillator, 2);
        let sink = make_id(ModuleType::Oscillator, 3);
        let modules = vec![
            ModuleInfo {
                id: source,
                category: ModuleCategory::Oscillator,
                size: Vec2::new(337.0, 140.0),
            },
            ModuleInfo {
                id: middle,
                category: ModuleCategory::Oscillator,
                size: Vec2::new(173.0, 220.0),
            },
            make_module(sink, ModuleCategory::Oscillator),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: source,
                to_module: middle,
            },
            LayoutConnection {
                from_module: middle,
                to_module: sink,
            },
        ];

        let result = calculate_free_flow_layout(&modules, &connections);
        let source_pos = result.positions.get(&source).unwrap();
        let middle_pos = result.positions.get(&middle).unwrap();
        let sink_pos = result.positions.get(&sink).unwrap();

        assert!(source_pos.x < middle_pos.x);
        assert!(middle_pos.x < sink_pos.x);
        assert!(middle_pos.x - source_pos.x >= 337.0 + GAP);
        assert!(sink_pos.x - middle_pos.x >= 173.0 + GAP);
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

    /// A modulation CHAIN feeding the signal path gets its own successive
    /// columns (left → right), instead of collapsing into one column. Mirrors a
    /// real patch: `env_clock → tur → env_cut → flt`, with `nse → flt` as the
    /// audio source. Each modulator must sit strictly left of what it feeds.
    #[test]
    fn test_modulation_chain_gets_successive_columns() {
        let nse = make_id(ModuleType::Noise, 1);
        let flt = make_id(ModuleType::Filter, 1);
        let amp = make_id(ModuleType::Amplifier, 1);
        let out = make_id(ModuleType::StereoOutput, 1);
        let env_cut = make_id(ModuleType::Envelope, 1); // modulates the filter
        let tur = make_id(ModuleType::TuringMachine, 1); // clocks env_cut
        let env_clock = make_id(ModuleType::Envelope, 2); // clocks the turing machine

        let modules = vec![
            make_module(nse, ModuleCategory::Oscillator),
            make_module(flt, ModuleCategory::Filter),
            make_module(amp, ModuleCategory::Amplifier),
            make_module(out, ModuleCategory::Output),
            make_module(env_cut, ModuleCategory::Envelope),
            make_module(tur, ModuleCategory::Sequencer),
            make_module(env_clock, ModuleCategory::Envelope),
        ];
        let edge = |from, to| LayoutConnection {
            from_module: from,
            to_module: to,
        };
        let connections = vec![
            edge(nse, flt),
            edge(flt, amp),
            edge(amp, out),
            edge(env_cut, flt),
            edge(tur, env_cut),
            edge(env_clock, tur),
        ];

        let result = calculate_layout(&modules, &connections);
        let col = |id| result.positions.get(&id).map(|p| p.x).unwrap();

        // The modulation chain flows strictly left → right into the filter.
        assert!(
            col(env_clock) < col(tur),
            "env_clock (col {}) must be left of tur (col {})",
            col(env_clock),
            col(tur)
        );
        assert!(
            col(tur) < col(env_cut),
            "tur (col {}) must be left of env_cut (col {})",
            col(tur),
            col(env_cut)
        );
        assert!(
            col(env_cut) < col(flt),
            "env_cut (col {}) must be left of the filter it modulates (col {})",
            col(env_cut),
            col(flt)
        );
        // …and they occupy three DISTINCT columns (not all collapsed into one).
        assert!(
            col(env_clock) != col(tur) && col(tur) != col(env_cut),
            "modulation chain must span distinct columns: {} {} {}",
            col(env_clock),
            col(tur),
            col(env_cut)
        );
        // The audio source hugs the filter (one column left), not the far left.
        assert!(col(nse) < col(flt), "nse must be left of flt");
    }

    /// Regression: a modulation module (envelope/LFO) wider than the signal
    /// modules in its column must widen that column so it does not overflow into
    /// — and overlap — the next column. Previously `col_widths` was computed from
    /// the signal `columns` only, so a wide envelope stacked under a narrow signal
    /// column overlapped the column to its right.
    #[test]
    fn test_wide_modulation_module_widens_its_column() {
        // Signal chain spanning three columns: osc(0) → flt(1) → amp(2).
        let osc = make_id(ModuleType::Oscillator, 1);
        let flt = make_id(ModuleType::Filter, 1);
        let amp = make_id(ModuleType::Amplifier, 1);
        // One modulation module under each of the first two columns (each lands one
        // column left of its target → env_wide in col 0, env_next in col 1), so
        // they share the same vertical "modulation band". env_wide is far wider
        // than col 0's signal module, so before the fix it overlapped env_next.
        let env_wide = make_id(ModuleType::Envelope, 1);
        let env_next = make_id(ModuleType::Envelope, 2);

        let modules = vec![
            make_module(osc, ModuleCategory::Oscillator),
            make_module(flt, ModuleCategory::Filter),
            make_module(amp, ModuleCategory::Amplifier),
            ModuleInfo {
                id: env_wide,
                category: ModuleCategory::Envelope,
                size: Vec2::new(500.0, 200.0),
            },
            make_module(env_next, ModuleCategory::Envelope),
        ];
        let connections = vec![
            LayoutConnection {
                from_module: osc,
                to_module: flt,
            },
            LayoutConnection {
                from_module: flt,
                to_module: amp,
            },
            // env_wide modulates flt (col 1) → placed in col 0.
            LayoutConnection {
                from_module: env_wide,
                to_module: flt,
            },
            // env_next modulates amp (col 2) → placed in col 1.
            LayoutConnection {
                from_module: env_next,
                to_module: amp,
            },
        ];

        let result = calculate_layout(&modules, &connections);

        // No pair of modules overlaps (snapped rects) — including the wide env.
        let sizes: HashMap<ModuleId, Vec2> = modules.iter().map(|m| (m.id, m.size)).collect();
        let rects: Vec<(ModuleId, Rect)> = result
            .positions
            .iter()
            .map(|(&id, &pos)| (id, Rect::from_min_size(pos, snap_size_to_grid(sizes[&id]))))
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
