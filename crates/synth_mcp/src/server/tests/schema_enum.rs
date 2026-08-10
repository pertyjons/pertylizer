//! Tests that closed-set tool arguments carry their values in the schema.
//!
//! MCP cannot complete a *tool* argument — `completion/complete` reaches only
//! prompts and resource templates — so a tool's `inputSchema` is the one place
//! a client can learn an argument's valid values without a discovery round
//! trip. Where the set really is closed, it belongs there as a JSON Schema
//! `enum` rather than as prose in the description.

use super::*;

/// Collect a property's `enum` values, following a `$ref`/`anyOf` if schemars
/// wrapped the type (an `Option<T>` of a named enum becomes a reference).
fn enum_values(schema: &serde_json::Value, property: &str) -> Vec<String> {
    let prop = &schema["properties"][property];
    let resolve = |node: &serde_json::Value| -> Option<Vec<String>> {
        if let Some(values) = node["enum"].as_array() {
            return Some(
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
            );
        }
        let name = node["$ref"].as_str()?.rsplit('/').next()?.to_string();
        let values = schema["$defs"][&name]["enum"].as_array()?;
        Some(
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        )
    };

    if let Some(values) = resolve(prop) {
        return values;
    }
    // `Option<T>` may render as anyOf[T, null].
    prop["anyOf"]
        .as_array()
        .and_then(|branches| branches.iter().find_map(resolve))
        .unwrap_or_default()
}

#[test]
fn search_filters_expose_their_closed_sets_in_the_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(SearchModulesParam))
        .expect("SearchModulesParam schema serializes");

    assert_eq!(
        enum_values(&schema, "category"),
        vec!["voice", "effect", "visualizer"],
        "category must advertise its three values"
    );
    for port_filter in ["has_input_type", "has_output_type"] {
        assert_eq!(
            enum_values(&schema, port_filter),
            vec!["audio", "control", "gate", "midi"],
            "{port_filter} must advertise the signal types"
        );
    }
}

/// The wire spellings the bridge matches on must be exactly what the schema
/// advertises, or a value a client picked straight out of the schema would be
/// rejected downstream.
#[test]
fn the_filter_wire_spellings_match_the_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(SearchModulesParam))
        .expect("SearchModulesParam schema serializes");

    let categories = enum_values(&schema, "category");
    for filter in [
        ModuleCategoryFilter::Voice,
        ModuleCategoryFilter::Effect,
        ModuleCategoryFilter::Visualizer,
    ] {
        assert!(
            categories.contains(&filter.as_str().to_string()),
            "{:?} serializes as {:?}, which the schema does not list",
            filter,
            filter.as_str()
        );
    }

    let signals = enum_values(&schema, "has_input_type");
    for filter in [
        SignalTypeFilter::Audio,
        SignalTypeFilter::Control,
        SignalTypeFilter::Gate,
        SignalTypeFilter::Midi,
    ] {
        assert!(
            signals.contains(&filter.as_str().to_string()),
            "{:?} serializes as {:?}, which the schema does not list",
            filter,
            filter.as_str()
        );
    }
}

/// Deserialization is now the only gate, so it must actually refuse a bad
/// value — there is no hand-written check behind it any more.
#[test]
fn an_invalid_filter_value_is_refused_by_deserialization() {
    assert!(serde_json::from_str::<SearchModulesParam>(r#"{"category":"voice"}"#).is_ok());
    assert!(serde_json::from_str::<SearchModulesParam>(r#"{"category":"Voice"}"#).is_err());
    assert!(serde_json::from_str::<SearchModulesParam>(r#"{"category":"instrument"}"#).is_err());
    assert!(serde_json::from_str::<SearchModulesParam>(r#"{"has_input_type":"cv"}"#).is_err());
}

/// Module-type arguments are deliberately **not** enumerated, and this pins the
/// reason so nobody "completes the job" later: every one of them is parsed
/// leniently (`parse_module_type` accepts the short key, the snake_case name
/// and the display name), and three of the four field descriptions promise
/// exactly that. A 75-key `enum` would advertise a constraint stricter than the
/// server enforces, so a strict client would refuse calls that work.
#[test]
fn module_type_arguments_stay_open_because_the_parser_is_lenient() {
    let schema = serde_json::to_value(schemars::schema_for!(GetModuleTypeInfoParam))
        .expect("GetModuleTypeInfoParam schema serializes");
    assert!(
        enum_values(&schema, "type_key").is_empty(),
        "type_key must not claim a closed set while the parser accepts names too"
    );
}
