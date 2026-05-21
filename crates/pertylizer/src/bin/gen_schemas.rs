//! Generator for JSON Schema documents describing Pertylizer's save/load
//! formats. Run with `cargo run -p pertylizer --bin gen_schemas`.
//!
//! Emits four files under `schemas/`:
//! - `project.schema.json` — full project (`file_type: "project"`)
//! - `patch.schema.json` — single instrument patch
//! - `awe-preset.schema.json` — AWE preset (`file_type: "awe_preset"`)
//! - `bundle-metadata.schema.json` — metadata.json inside `.zip` bundles
//!
//! Per-module parameter validation: after the base schema is emitted via
//! `schemars` derive, the `$defs.ModuleState` definition is replaced with a
//! `oneOf` over every entry in `ALL_MODULE_TYPES`. Each branch lists that
//! module's parameters with their range (or choice enum) sourced from the
//! `ModuleDescriptor`.
//!
//! Top-level `file_type` is tightened to a `const` discriminator so the
//! schema can be used to pick a format. Nested `Song` and `AweState` are
//! fully derived via `JsonSchema`.

use std::path::PathBuf;

use pertylizer::bundle::BundleMetadata;
use pertylizer::module_factory::{ALL_MODULE_TYPES, get_descriptor};
use pertylizer::patch::{AwePresetFile, Patch};
use pertylizer::project::ProjectFile;
use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};
use synth_core::params::{Param, SamplerParam};
use synth_core::{ModuleDescriptor, ModuleType, ParameterDescriptor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .ok_or("cannot locate workspace root")?
        .join("schemas");
    std::fs::create_dir_all(&out_dir)?;

    write_schema::<ProjectFile>(&out_dir, "project.schema.json", true, Some("project"))?;
    write_schema::<Patch>(&out_dir, "patch.schema.json", true, None)?;
    write_schema::<AwePresetFile>(
        &out_dir,
        "awe-preset.schema.json",
        false,
        Some("awe_preset"),
    )?;
    write_schema::<BundleMetadata>(&out_dir, "bundle-metadata.schema.json", false, None)?;

    println!("Wrote schemas to {}", out_dir.display());
    Ok(())
}

fn write_schema<T: schemars::JsonSchema>(
    out_dir: &std::path::Path,
    filename: &str,
    tighten_modules: bool,
    file_type: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut generator = SchemaGenerator::new(SchemaSettings::draft2020_12());
    let schema = generator.root_schema_for::<T>();
    let mut value: Value = serde_json::to_value(&schema)?;

    if tighten_modules {
        tighten_module_state(&mut value);
    }

    if let Some(ft) = file_type {
        tighten_file_type(&mut value, ft);
    }

    embed_examples(&mut value);

    let json = serde_json::to_string_pretty(&value)?;
    let path = out_dir.join(filename);
    std::fs::write(&path, json + "\n")?;
    println!("  {}", path.display());
    Ok(())
}

/// Insert `examples` arrays on key `$defs` so schema readers can see real
/// snippets without opening `assets/examples/`. Examples are JSON-Schema
/// `examples` (Draft 2020-12), which tooling renders inline.
fn embed_examples(root: &mut Value) {
    let Some(defs) = root.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };

    let examples: &[(&str, Value)] = &[
        (
            "Note",
            json!([{
                "id": 0,
                "start": 0,
                "duration": 480,
                "pitch": 60,
                "velocity": 0.8,
                "instrument": 0
            }]),
        ),
        (
            "Pattern",
            json!([{
                "id": 0,
                "name": "Verse",
                "length": 3840,
                "notes": [
                    {"id": 0, "start": 0, "duration": 960, "pitch": 60, "velocity": 0.8, "instrument": 0},
                    {"id": 1, "start": 960, "duration": 960, "pitch": 64, "velocity": 0.8, "instrument": 0}
                ],
                "automation": [],
                "row_resolution": {"rows": 64, "ticks_per_row": 60},
                "next_note_id": 2
            }]),
        ),
        (
            "RoomShape",
            json!([
                {"Sphere": {"radius": 8.0}},
                {"Box": {"length": 10.0, "width": 8.0, "height": 3.5}}
            ]),
        ),
        (
            "Material",
            json!([{
                "absorption_low": 0.05,
                "absorption_mid": 0.08,
                "absorption_high": 0.15,
                "diffusion": 0.15
            }]),
        ),
        (
            "AweLfoState",
            json!([{"rate": 0.5, "amount": 0.0, "target": "SourceX"}]),
        ),
        (
            "Author",
            json!([{
                "name": "Per Jonsson",
                "email": "",
                "website": "",
                "license": "CC BY 4.0"
            }]),
        ),
        (
            "ConnectionState",
            json!([{"from": ["osc-1", "out"], "to": ["flt-1", "in"]}]),
        ),
    ];

    for (name, ex) in examples {
        if let Some(def) = defs.get_mut(*name).and_then(Value::as_object_mut) {
            def.insert("examples".to_string(), ex.clone());
        }
    }

    // Examples on the per-module-type branches: pick a few representative
    // modules so readers see a real patch fragment.
    if let Some(branches) = defs
        .get_mut("ModuleState")
        .and_then(|ms| ms.get_mut("oneOf"))
        .and_then(Value::as_array_mut)
    {
        let module_examples: &[(&str, Value)] = &[
            (
                "oscillator",
                json!({
                    "id": "osc-1",
                    "type": "oscillator",
                    "position": {"x": 32.0, "y": 32.0},
                    "parameters": {
                        "waveform": "sawtooth",
                        "frequency": 440.0,
                        "detune": 0.0,
                        "level": 1.0
                    }
                }),
            ),
            (
                "filter",
                json!({
                    "id": "flt-1",
                    "type": "filter",
                    "position": {"x": 224.0, "y": 32.0},
                    "parameters": {
                        "type": "lowpass",
                        "cutoff": 0.5,
                        "resonance": 0.3
                    }
                }),
            ),
            (
                "reverb",
                json!({
                    "id": "rev-1",
                    "type": "reverb",
                    "position": {"x": 1760.0, "y": 32.0},
                    "parameters": {
                        "decay": 2.0,
                        "damping": 0.6,
                        "mix": 0.3
                    }
                }),
            ),
        ];
        for branch in branches.iter_mut() {
            let Some(branch_obj) = branch.as_object_mut() else {
                continue;
            };
            let Some(branch_type) = branch_obj
                .get("properties")
                .and_then(|p| p.get("type"))
                .and_then(|t| t.get("const"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Some((_, ex)) = module_examples.iter().find(|(t, _)| *t == branch_type) {
                branch_obj.insert("examples".to_string(), json!([ex]));
            }
        }
    }
}

/// Replace the generic `string` schema on `properties.file_type` with a
/// `const` discriminator. Lets the schema actually identify the file format.
fn tighten_file_type(root: &mut Value, value: &str) {
    let Some(props) = root.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(field) = props.get_mut("file_type").and_then(Value::as_object_mut) else {
        return;
    };
    field.insert("const".to_string(), json!(value));
    field.insert("type".to_string(), json!("string"));
}

/// Replace the generic `$defs.ModuleState` with a `oneOf` over every entry in
/// `ALL_MODULE_TYPES`. Each branch constrains `type` to one snake-case string
/// and lists the module's parameters with their range / choice enum.
fn tighten_module_state(root: &mut Value) {
    let defs = root
        .get_mut("$defs")
        .and_then(Value::as_object_mut)
        .expect("schema should have $defs");

    let mut branches = Vec::new();
    for &mt in ALL_MODULE_TYPES {
        let Some(desc) = get_descriptor(mt) else {
            continue;
        };
        branches.push(module_branch(mt, &desc));
    }

    let module_state = json!({
        "description": "State of a single module. The `type` discriminator selects which parameter set is valid.",
        "oneOf": branches
    });

    defs.insert("ModuleState".to_string(), module_state);

    // Hoist any choice-enum used 3+ times into `$defs` and replace inline
    // occurrences with `$ref`. The mod_matrix module alone uses each of its
    // 16-/19-entry source/destination enums sixteen times.
    hoist_repeated_enums(defs);
}

/// Walk every `{"type": "string", "enum": [...]}` fragment inside the schema,
/// count distinct enum tuples, and extract any that appear 3+ times into
/// shared `$defs.SharedEnum<N>` entries. Inline copies become `$ref`s.
fn hoist_repeated_enums(defs: &mut serde_json::Map<String, Value>) {
    use std::collections::BTreeMap;

    // Pass 1: collect every string-enum tuple and count occurrences.
    let mut counts: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    if let Some(ms) = defs.get("ModuleState") {
        walk_enums(ms, &mut |arr| {
            *counts.entry(arr.to_vec()).or_insert(0) += 1;
        });
    }

    // Promote those used 3+ times to stable names.
    let mut promoted: BTreeMap<Vec<String>, String> = BTreeMap::new();
    for (idx, (key, _)) in counts.iter().filter(|(_, c)| **c >= 3).enumerate() {
        let name = format!("SharedEnum{}", idx + 1);
        promoted.insert(key.clone(), name);
    }

    if promoted.is_empty() {
        return;
    }

    // Insert the promoted enums into $defs.
    for (enum_arr, name) in &promoted {
        let hint = enum_arr
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if enum_arr.len() > 3 { ", …" } else { "" };
        defs.insert(
            name.clone(),
            json!({
                "description": format!("Shared enum ({} entries): {hint}{suffix}", enum_arr.len()),
                "type": "string",
                "enum": enum_arr,
            }),
        );
    }

    // Pass 2: rewrite ModuleState in place, swapping inline matches for $refs.
    if let Some(ms) = defs.get_mut("ModuleState") {
        rewrite_enums(ms, &promoted);
    }
}

/// Extract the string-enum tuple from a schema fragment, if any.
fn read_string_enum(map: &serde_json::Map<String, Value>) -> Option<Vec<String>> {
    let t = map.get("type")?.as_str()?;
    if t != "string" {
        return None;
    }
    let arr = map.get("enum")?.as_array()?;
    arr.iter().map(|v| v.as_str().map(String::from)).collect()
}

/// Visit every `{"type": "string", "enum": [strings]}` fragment and call `f`
/// with the enum string list.
fn walk_enums<F: FnMut(&[String])>(value: &Value, f: &mut F) {
    match value {
        Value::Object(map) => {
            if let Some(strings) = read_string_enum(map) {
                f(&strings);
            }
            for v in map.values() {
                walk_enums(v, f);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                walk_enums(v, f);
            }
        }
        _ => {}
    }
}

/// Replace every inline `{"type": "string", "enum": [...]}` fragment whose
/// content matches a key in `promoted` with `{"$ref": "#/$defs/<name>"}`.
fn rewrite_enums(value: &mut Value, promoted: &std::collections::BTreeMap<Vec<String>, String>) {
    match value {
        Value::Object(map) => {
            let replace_with: Option<String> =
                read_string_enum(map).and_then(|s| promoted.get(&s).cloned());
            if let Some(name) = replace_with {
                map.clear();
                map.insert("$ref".to_string(), Value::String(format!("#/$defs/{name}")));
                return;
            }
            for v in map.values_mut() {
                rewrite_enums(v, promoted);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                rewrite_enums(v, promoted);
            }
        }
        _ => {}
    }
}

/// Build the `oneOf` branch schema for a single module type.
fn module_branch(mt: ModuleType, desc: &ModuleDescriptor) -> Value {
    let type_id = serde_plain_snake_case(mt);
    let id_pattern = format!("^{}-\\d+$", regex_escape(mt.prefix()));

    let mut param_props = serde_json::Map::new();
    for param in &desc.parameters {
        param_props.insert(param.type_id.clone(), parameter_schema(param));
    }

    let mut header = format!("{} ({})", desc.name, type_id);
    if !desc.description.is_empty() {
        header.push_str(" — ");
        header.push_str(&desc.description);
    }

    json!({
        "title": desc.name,
        "description": header,
        "type": "object",
        "properties": {
            "id": {
                "description": format!("Module instance ID, e.g. `{}-1`.", mt.prefix()),
                "type": "string",
                "pattern": id_pattern
            },
            "type": {
                "description": "Module type discriminator.",
                "const": type_id
            },
            "position": { "$ref": "#/$defs/Position" },
            "parameters": {
                "description": "Parameter values for this module.",
                "type": "object",
                "properties": param_props,
                "additionalProperties": { "$ref": "#/$defs/ParamValue" },
                "default": {}
            }
        },
        "required": ["id", "type", "position"]
    })
}

/// Build a JSON Schema fragment for a single parameter.
///
/// Numeric ranges are intentionally **not** taken from the descriptor: many
/// parameters (e.g. distortion `bit_depth`, reverb mix levels) are persisted
/// as engine-internal normalized 0..1 floats while the descriptor reports
/// the semantic display range (1..16 bits etc.). The two don't line up,
/// so the schema constrains only the JSON shape: number-vs-string-vs-object.
/// The descriptor's `default` is kept as informational metadata.
///
/// - Choice parameters accept either the choice ID string (canonical) or a
///   number — legacy patches sometimes store the index.
/// - The sample-id object form (`{"sample_id": N}`) is only valid on
///   `Sampler::SampleSelect`. Other numeric params get a plain number.
fn parameter_schema(param: &ParameterDescriptor) -> Value {
    let mut description = param.description.clone();
    let unit_suffix = param.unit.suffix();
    if !unit_suffix.trim().is_empty() {
        if !description.is_empty() {
            description.push_str(" — ");
        }
        description.push_str(&format!("unit: {}", unit_suffix.trim()));
    }

    if let Some(choices) = &param.choices {
        let ids: Vec<Value> = choices.iter().map(|c| json!(c.id)).collect();
        let mut schema = json!({
            "title": param.name,
            "anyOf": [
                { "type": "string", "enum": ids },
                { "type": "number" }
            ]
        });
        if !description.is_empty() {
            schema["description"] = json!(description);
        }
        return schema;
    }

    if matches!(param.id, Param::Sampler(SamplerParam::SampleSelect(_))) {
        let mut schema = json!({
            "title": param.name,
            "type": "object",
            "properties": {
                "sample_id": { "type": "integer", "minimum": 0 }
            },
            "required": ["sample_id"]
        });
        if !description.is_empty() {
            schema["description"] = json!(description);
        }
        return schema;
    }

    let mut schema = json!({
        "title": param.name,
        "type": "number",
        "default": param.range.default,
    });
    if !description.is_empty() {
        schema["description"] = json!(description);
    }
    schema
}

/// Match the serde `rename_all = "snake_case"` output for `ModuleType`.
fn serde_plain_snake_case(mt: ModuleType) -> String {
    // `mt` already serializes as a snake_case string under serde; round-trip
    // through serde_json to avoid duplicating the mapping.
    serde_json::to_value(mt)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{:?}", mt))
}

/// Escape regex special characters in a prefix. Prefixes are short ASCII
/// strings (e.g. `osc`, `flt`); the escape is defensive against future
/// prefixes that include special characters.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
