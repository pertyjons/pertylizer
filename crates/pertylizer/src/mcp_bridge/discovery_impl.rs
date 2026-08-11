use super::*;

pub fn search_module_types(
    category: Option<&str>,
    has_input_type: Option<&str>,
    has_output_type: Option<&str>,
    query: Option<&str>,
) -> ModuleSearchResult {
    use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};

    // Lowercased query tokens; empty/whitespace query behaves like "no query".
    let tokens: Vec<String> = query
        .map(|q| q.split_whitespace().map(str::to_lowercase).collect())
        .unwrap_or_default();
    let has_query = !tokens.is_empty();

    // (module_type, descriptor, score) for everything passing the hard filters.
    let mut scored: Vec<(synth_core::ModuleType, synth_core::ModuleDescriptor, u32)> = Vec::new();
    for &mt in ALL_MODULE_TYPES.iter() {
        let Some(desc) = get_descriptor(mt) else {
            continue;
        };
        if !passes_hard_filters(mt, &desc, category, has_input_type, has_output_type) {
            continue;
        }

        let score = if has_query {
            score_module(&tokens, mt, &desc)
        } else {
            // No query: filters alone decide membership; keep stable registry order.
            1
        };
        // Drop zero-relevance matches rather than padding the result.
        if score == 0 {
            continue;
        }
        scored.push((mt, desc, score));
    }

    // Best-first; ties keep the registry order (sort is stable).
    if has_query {
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.2));
    }

    let modules: Vec<ModuleTypeInfo> = scored
        .iter()
        .map(|(mt, desc, _)| build_module_type_info(*mt, desc))
        .collect();

    // Only offer a "did you mean" when a real query matched nothing — an empty
    // list with no hint reads as "feature absent", the exact trap to avoid.
    // Suggestions respect the same hard filters so we never propose a module the
    // caller's category/port filter would have excluded anyway.
    let did_you_mean = if has_query && modules.is_empty() {
        did_you_mean_modules(&tokens, category, has_input_type, has_output_type)
    } else {
        Vec::new()
    };

    ModuleSearchResult {
        // Counted here, over the whole match set. Capping is the MCP tool's job —
        // it owns the `limit` contract, and truncating here would leave
        // `total_matched` describing the truncation instead of the search.
        total_matched: modules.len(),
        modules,
        did_you_mean,
        // The wording of the "nothing matched, try this" advice belongs to the
        // MCP tool, which knows what its own filters were called; the bridge
        // supplies the facts it is built from.
        hint: None,
    }
}

/// Hard (non-scored) filters shared by the main search and the `did_you_mean`
/// fallback: category plus required input/output port signal types. A module
/// must pass all provided filters to be eligible.
fn passes_hard_filters(
    mt: synth_core::ModuleType,
    desc: &synth_core::ModuleDescriptor,
    category: Option<&str>,
    has_input_type: Option<&str>,
    has_output_type: Option<&str>,
) -> bool {
    use synth_core::PortDirection;

    if let Some(cat) = category {
        let mt_cat = if mt.is_voice_module() {
            "voice"
        } else if mt.is_effect() {
            "effect"
        } else {
            "visualizer"
        };
        if mt_cat != cat {
            return false;
        }
    }
    if let Some(input_type) = has_input_type
        && !desc.ports.iter().any(|p| {
            p.direction == PortDirection::Input && port_type_str(p.port_type) == input_type
        })
    {
        return false;
    }
    if let Some(output_type) = has_output_type
        && !desc.ports.iter().any(|p| {
            p.direction == PortDirection::Output && port_type_str(p.port_type) == output_type
        })
    {
        return false;
    }
    true
}

/// Weighted token score for one module: name 10, tags 5, description 2,
/// parameter name 2 — summed across query tokens. Matching is substring with a
/// cheap one-char-stem fallback so `multiply` hits `multiplies`.
fn score_module(
    tokens: &[String],
    mt: synth_core::ModuleType,
    desc: &synth_core::ModuleDescriptor,
) -> u32 {
    let name = mt.name().to_lowercase();
    let key = mt.prefix().to_lowercase();
    let description = desc.description.to_lowercase();
    let tags: Vec<String> = desc.tags.iter().map(|t| t.to_lowercase()).collect();
    let params: Vec<String> = desc
        .parameters
        .iter()
        .map(|p| p.name.to_lowercase())
        .collect();

    let mut score = 0u32;
    for tok in tokens {
        // Name and type key are both strong identity signals → name weight.
        if field_matches(&name, tok) || field_matches(&key, tok) {
            score += 10;
        }
        if tags.iter().any(|t| field_matches(t, tok)) {
            score += 5;
        }
        if field_matches(&description, tok) {
            score += 2;
        }
        if params.iter().any(|p| field_matches(p, tok)) {
            score += 2;
        }
    }
    score
}

/// Does `field` contain `tok`, allowing a one-trailing-char stem so plural/verb
/// endings bridge (`multiply` → `multipl` ⊂ `multiplies`)? Char-safe for UTF-8.
fn field_matches(field: &str, tok: &str) -> bool {
    if field.contains(tok) {
        return true;
    }
    // Drop the last char as a poor-man's stemmer; only worth it for longer tokens.
    if tok.chars().count() >= 4 {
        let stem: String = {
            let n = tok.chars().count() - 1;
            tok.chars().take(n).collect()
        };
        if field.contains(&stem) {
            return true;
        }
    }
    false
}

/// How many near-miss modules a suggestion names.
const MAX_MODULE_SUGGESTIONS: usize = 5;

/// Near-miss modules when a query matched nothing, so an empty result reads as
/// "mis-spelled" rather than "feature absent" — `ringmd` surfaces
/// `Ring Mod (rng)` while a random `xyz` yields nothing.
///
/// Each type is ranked by the best its key or its display name can do against
/// any one query token, on the shared ladder in [`synth_core::suggest`]. Ranking
/// both spellings and answering with the type is what lets a caller who wrote
/// the name get the key back, which is the same shape `ModuleType::suggest` and
/// the `get_module_type_info` hint use.
///
/// Honours the caller's hard filters, so a suggestion is never something the
/// category/port filter would have excluded anyway.
fn did_you_mean_modules(
    tokens: &[String],
    category: Option<&str>,
    has_input_type: Option<&str>,
    has_output_type: Option<&str>,
) -> Vec<String> {
    use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};

    let mut scored: Vec<(usize, String)> = Vec::new();
    for &mt in ALL_MODULE_TYPES.iter() {
        let Some(desc) = get_descriptor(mt) else {
            continue;
        };
        if !passes_hard_filters(mt, &desc, category, has_input_type, has_output_type) {
            continue;
        }
        let best = tokens
            .iter()
            .flat_map(|token| {
                [mt.prefix(), mt.name()]
                    .map(|spelling| synth_core::suggest::match_rank(token, spelling))
            })
            .flatten()
            .min();
        if let Some(rank) = best {
            scored.push((rank, format!("{} ({})", mt.name(), mt.prefix())));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(MAX_MODULE_SUGGESTIONS)
        .map(|(_, s)| s)
        .collect()
}

/// Coarse catalog category for a module type: "voice", "effect", or "visualizer".
pub(super) fn module_category(mt: synth_core::ModuleType) -> &'static str {
    if mt.is_voice_module() {
        "voice"
    } else if mt.is_effect() {
        "effect"
    } else {
        "visualizer"
    }
}

/// Build a [`ModuleTypeInfo`] from a [`ModuleType`] and its descriptor.
pub(super) fn build_module_type_info(
    mt: synth_core::ModuleType,
    desc: &synth_core::ModuleDescriptor,
) -> ModuleTypeInfo {
    use synth_core::PortDirection;

    let category = module_category(mt);

    let port_to_info = |p: &synth_core::PortDescriptor| {
        let nominal_range = p.value_domain.nominal_range();
        synth_mcp::types::PortTypeInfo {
            name: p.name.to_string(),
            label: p.label.clone(),
            description: p.description.clone(),
            signal_type: port_type_str(p.port_type).to_owned(),
            value_domain: synth_mcp::types::PortValueDomainInfo {
                id: p.value_domain.id().to_owned(),
                accepted_values: p.value_domain.accepted_values().to_owned(),
                nominal_min: nominal_range.map(synth_core::PortValueRange::min),
                nominal_max: nominal_range.map(synth_core::PortValueRange::max),
                unit: p.value_domain.unit().map(str::to_owned),
            },
        }
    };

    let input_ports = desc
        .ports
        .iter()
        .filter(|p| p.direction == PortDirection::Input)
        .map(port_to_info)
        .collect();
    let output_ports = desc
        .ports
        .iter()
        .filter(|p| p.direction == PortDirection::Output)
        .map(port_to_info)
        .collect();
    let parameters = desc
        .parameters
        .iter()
        .map(|p| ParamTypeInfo {
            name: p.name.clone(),
            description: p.description.clone(),
            min: p.range.min,
            max: p.range.max,
            default: p.range.default,
            unit: p.unit.suffix().to_owned(),
            choices: p.choices.as_ref().map(|opts| {
                opts.iter()
                    .enumerate()
                    .map(|(i, c)| synth_mcp::types::ChoiceInfo {
                        value: i as f32,
                        id: c.id.clone(),
                        name: c.name.clone(),
                        description: c.description.clone().unwrap_or_default(),
                    })
                    .collect()
            }),
            value_kind: Some(p.kind),
        })
        .collect();

    ModuleTypeInfo {
        type_key: mt.prefix().to_string(),
        name: mt.name().to_string(),
        description: desc.description.clone(),
        category: category.to_string(),
        gui_only: mt.is_visualizer(),
        input_ports,
        output_ports,
        parameters,
        signal_flow_hint: signal_flow_hint(&desc.category),
        algorithm_parameters: algorithm_parameters_json(mt),
    }
}

/// Static per-algorithm documentation of the math oscillator's generic
/// `param_a`/`param_b`/`param_c` knobs, as JSON keyed by algorithm id. `None`
/// for every module type whose knobs don't change role with an algorithm.
fn algorithm_parameters_json(mt: synth_core::ModuleType) -> Option<serde_json::Value> {
    use serde_json::{Map, Value, json};
    use synth_core::MathAlgo;

    if mt != synth_core::ModuleType::MathOscillator {
        return None;
    }

    let mut table = Map::new();
    for algo in MathAlgo::ALL {
        let [a, b, c] = algo.param_info();
        let entry = json!({
            "param_a": { "name": a.name, "description": a.description },
            "param_b": { "name": b.name, "description": b.description },
            "param_c": { "name": c.name, "description": c.description },
        });
        table.insert(algo.id().to_string(), entry);
    }
    Some(Value::Object(table))
}

/// Convert a `PortType` to its string name.
pub(super) fn port_type_str(pt: synth_core::PortType) -> &'static str {
    pt.id()
}

/// Return a hint about which input types a given output type can drive.
pub(super) fn compatible_types_hint(pt: synth_core::PortType) -> String {
    synth_core::PortType::ALL
        .into_iter()
        .filter(|destination| pt.can_drive(*destination))
        .map(synth_core::PortType::id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Return a signal flow hint based on module category.
fn signal_flow_hint(category: &synth_core::ModuleCategory) -> Option<String> {
    use synth_core::ModuleCategory;
    match category {
        ModuleCategory::Oscillator => Some(
            "Connect 'out' → filter or mixer input. Use 'gate' and 'freq' CV inputs from note data."
                .to_owned(),
        ),
        ModuleCategory::Filter => Some(
            "Connect audio 'in' from oscillator/mixer, 'out' → amplifier. Use 'cutoff_cv' for envelope modulation."
                .to_owned(),
        ),
        ModuleCategory::Amplifier => Some(
            "Connect audio 'in' from filter, 'out' → output module. Use 'cv' from envelope for volume shaping."
                .to_owned(),
        ),
        ModuleCategory::Envelope => Some(
            "Connect 'out' → amplifier 'cv' or filter 'cutoff_cv'. Needs 'gate' input from note data."
                .to_owned(),
        ),
        ModuleCategory::LFO => Some(
            "Connect 'out' → any CV input for modulation (e.g. filter cutoff, oscillator frequency)."
                .to_owned(),
        ),
        ModuleCategory::Mixer => Some(
            "Connect multiple audio sources to 'in1'..'in8', output mixed signal from 'out'."
                .to_owned(),
        ),
        ModuleCategory::Output => Some(
            "Final module in voice chain. Connect audio to 'in_l'/'in_r'. Sends audio to instrument output."
                .to_owned(),
        ),
        ModuleCategory::Effect => Some(
            "Effect module in the instrument's effect chain. Audio passes through automatically."
                .to_owned(),
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Group B — symbolic composition helper bridge impls
// ---------------------------------------------------------------------------
