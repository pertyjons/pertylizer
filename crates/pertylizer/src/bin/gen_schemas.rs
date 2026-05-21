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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

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
    let out_dir = match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .ok_or("cannot locate workspace root")?
            .join("schemas"),
    };
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

/// Shared definitions injected into any schema that references `ModuleState`.
struct ModuleStateDefs {
    module_state: Value,
    shared_enums: BTreeMap<String, Value>,
}

/// Cache the (ModuleState, shared-enum) defs since they're identical across
/// project and patch schemas and building them instantiates every module.
fn module_state_defs() -> &'static ModuleStateDefs {
    static CELL: OnceLock<ModuleStateDefs> = OnceLock::new();
    CELL.get_or_init(build_module_state_defs)
}

fn build_module_state_defs() -> ModuleStateDefs {
    let mut branches: Vec<Value> = ALL_MODULE_TYPES
        .iter()
        .filter_map(|&mt| get_descriptor(mt).map(|desc| module_branch(mt, &desc)))
        .collect();

    let mut counts: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    for branch in &branches {
        walk_enums(branch, &mut |arr| {
            *counts.entry(arr.to_vec()).or_insert(0) += 1;
        });
    }

    let promoted: BTreeMap<Vec<String>, String> = counts
        .iter()
        .filter(|(_, c)| **c >= 3)
        .enumerate()
        .map(|(idx, (key, _))| (key.clone(), format!("SharedEnum{}", idx + 1)))
        .collect();

    for branch in &mut branches {
        rewrite_enums(branch, &promoted);
    }

    let shared_enums: BTreeMap<String, Value> = promoted
        .into_iter()
        .map(|(enum_arr, name)| {
            let hint = enum_arr
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if enum_arr.len() > 3 { ", …" } else { "" };
            let def = json!({
                "description": format!("Shared enum ({} entries): {hint}{suffix}", enum_arr.len()),
                "type": "string",
                "enum": enum_arr,
            });
            (name, def)
        })
        .collect();

    let module_state = json!({
        "description": "State of a single module. The `type` discriminator selects which parameter set is valid.",
        "oneOf": branches
    });

    ModuleStateDefs {
        module_state,
        shared_enums,
    }
}

/// Replace the generic `$defs.ModuleState` with a `oneOf` over every entry in
/// `ALL_MODULE_TYPES`, and inject the shared choice-enum defs that branches
/// reference via `$ref`. Also strips the auto-derived `ParamValue` def: it
/// becomes orphan once the strict `additionalProperties: false` is in place
/// on module `parameters`.
fn tighten_module_state(root: &mut Value) {
    let Some(defs) = root.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };

    let cached = module_state_defs();
    defs.insert("ModuleState".to_string(), cached.module_state.clone());
    for (name, def) in &cached.shared_enums {
        defs.insert(name.clone(), def.clone());
    }
    defs.remove("ParamValue");
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
    let id_pattern = format!("^{}-\\d+$", mt.prefix());

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
                "additionalProperties": false,
                "default": {}
            }
        },
        "required": ["id", "type", "position"]
    })
}

/// Build a JSON Schema fragment for a single parameter.
///
/// - Choice parameters accept the choice ID string (canonical).
/// - The sample-id object form (`{"sample_id": N}`) is only valid on
///   `Sampler::SampleSelect`. Other numeric params get a plain number with
///   the descriptor's `min`/`max`/`default` applied (the four formerly
///   normalized-then-remapped params now wrap dedicated semantic newtypes,
///   so on-disk values match the descriptor range directly).
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
            "type": "string",
            "enum": ids,
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
        "minimum": param.range.min,
        "maximum": param.range.max,
        "default": param.range.default,
    });
    if !description.is_empty() {
        schema["description"] = json!(description);
    }
    schema
}

/// Match the serde `rename_all = "snake_case"` output for `ModuleType`. This
/// is the string saved files use in their `type` discriminator; the
/// descriptor's `type_id` is a separate UI-facing label and does not always
/// match (e.g. `SubOscillator` → `sub_oscillator` here vs `sub_osc` in the
/// descriptor).
fn serde_plain_snake_case(mt: ModuleType) -> String {
    serde_json::to_value(mt)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{:?}", mt))
}
