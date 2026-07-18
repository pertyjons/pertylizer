//! Shared enumeration of an instrument's automatable module-param targets.
//!
//! The single source of truth for "which module params can be targeted" — used
//! by the Mod Grid Module-target picker (GUI) and the MCP
//! `get_instrument_automation_targets` tool, so the two never advertise a
//! different address space. Input is the instrument's live module descriptors
//! (`session.all_modules_for_instrument`); the positional [`ModuleId`] identity
//! matches the engine's `DestAddr` / `AutomationTarget::Module`.

use std::collections::HashMap;

use synth_core::{ModuleDescriptor, ModuleType};
use synth_engine::ModuleId;

/// One module's automatable params, grouped for a two-step target picker.
pub struct ModuleTargetGroup {
    /// Positional identity (`module_type` + 1-based `instance`).
    pub module_id: ModuleId,
    /// Display label, e.g. `"Filter 1"` (or `"Filter"` when it's the only one of
    /// its type).
    pub label: String,
    /// `(type_id, display_name)` for each automatable param, sorted by `type_id`.
    pub params: Vec<(String, String)>,
}

/// Enumerate an instrument's automatable module-param targets from its live
/// descriptors. A param counts when [`ParameterDescriptor::is_automatable`]
/// (continuous/integer and modulatable) — the same filter the MCP tool uses.
/// Modules with no automatable param are omitted; groups are sorted by
/// `(module_type, instance)` for stable display.
#[must_use]
pub fn module_target_groups(
    modules: &HashMap<ModuleId, ModuleDescriptor>,
) -> Vec<ModuleTargetGroup> {
    // Per-type counts drive the "Filter 1" vs "Filter" labelling.
    let mut type_counts: HashMap<ModuleType, usize> = HashMap::new();
    for id in modules.keys() {
        *type_counts.entry(id.module_type).or_default() += 1;
    }

    let mut groups: Vec<ModuleTargetGroup> = modules
        .iter()
        .filter_map(|(&module_id, desc)| {
            let mut params: Vec<(String, String)> = desc
                .parameters
                .iter()
                .filter(|p| p.is_automatable())
                .map(|p| (p.type_id.clone(), p.name.clone()))
                .collect();
            if params.is_empty() {
                return None;
            }
            params.sort_by(|a, b| a.0.cmp(&b.0));
            let multiple = type_counts
                .get(&module_id.module_type)
                .copied()
                .unwrap_or(0)
                > 1;
            let label = if multiple {
                format!("{} {}", module_id.module_type.name(), module_id.instance)
            } else {
                module_id.module_type.name().to_string()
            };
            Some(ModuleTargetGroup {
                module_id,
                label,
                params,
            })
        })
        .collect();
    groups.sort_by_key(|g| (g.module_id.module_type as u32, g.module_id.instance));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_modules::{Envelope, Filter, Oscillator};

    fn modules() -> HashMap<ModuleId, ModuleDescriptor> {
        use synth_core::module_traits::Describable;
        let mut m = HashMap::new();
        // Two filters (→ "Filter 1"/"Filter 2"), one envelope (→ "Envelope").
        m.insert(
            ModuleId::new(ModuleType::Filter, 1),
            Filter::new().descriptor(),
        );
        m.insert(
            ModuleId::new(ModuleType::Filter, 2),
            Filter::new().descriptor(),
        );
        m.insert(
            ModuleId::new(ModuleType::Envelope, 1),
            Envelope::new().descriptor(),
        );
        // An oscillator too, to prove modules with automatable params appear.
        m.insert(
            ModuleId::new(ModuleType::Oscillator, 1),
            Oscillator::new().descriptor(),
        );
        m
    }

    #[test]
    fn groups_are_labelled_sorted_and_only_automatable() {
        let groups = module_target_groups(&modules());
        // Every group has at least one automatable param.
        assert!(groups.iter().all(|g| !g.params.is_empty()));
        // Duplicated type is numbered; the singletons are not.
        let filters: Vec<&str> = groups
            .iter()
            .filter(|g| g.module_id.module_type == ModuleType::Filter)
            .map(|g| g.label.as_str())
            .collect();
        assert_eq!(filters, vec!["Filter 1", "Filter 2"]);
        let env = groups
            .iter()
            .find(|g| g.module_id.module_type == ModuleType::Envelope)
            .expect("envelope group");
        assert_eq!(env.label, "Envelope");
        // Filter cutoff is offered (continuous + modulatable).
        assert!(
            groups
                .iter()
                .find(|g| g.module_id == ModuleId::new(ModuleType::Filter, 1))
                .unwrap()
                .params
                .iter()
                .any(|(id, _)| id == "cutoff")
        );
        // Params are sorted by type_id within a group.
        for g in &groups {
            let mut sorted = g.params.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(g.params, sorted);
        }
    }
}
