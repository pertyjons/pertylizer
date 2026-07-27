//! Integration test that guards `schemas/` against drift.
//!
//! Two checks run:
//!
//! 1. **Drift detection** — invokes the `gen_schemas` binary into a temp
//!    directory and byte-compares each generated file with the matching
//!    file in `schemas/`. If they differ, the checked-in schemas are stale
//!    relative to the current Rust types and the developer must
//!    regenerate.
//!
//! 2. **Correctness** — loads each schema with `jsonschema` and validates
//!    every fixture in `assets/examples/`. Catches the case where the
//!    schema is up to date but rejects real saved files (i.e. the schema
//!    is wrong about the format).
//!
//! Run via `cargo test -p pertylizer --test schemas_validate_examples`.

use std::path::{Path, PathBuf};
use std::process::Command;

use jsonschema::Validator;
use serde_json::Value;
use synth_core::PortType;

fn workspace_root() -> PathBuf {
    // tests/ live two levels under workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

const SCHEMA_FILES: &[&str] = &[
    "project.schema.json",
    "patch.schema.json",
    "bundle-metadata.schema.json",
    "descriptors.json",
];

/// Regenerate schemas into a temp directory and byte-compare with the
/// checked-in files under `schemas/`. Fails if any file differs — the
/// developer needs to run `cargo run -p pertylizer --bin gen_schemas` and
/// commit the result.
#[test]
fn checked_in_schemas_match_generated() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = env!("CARGO_BIN_EXE_gen_schemas");

    let output = Command::new(bin)
        .arg(tmp.path())
        .output()
        .expect("invoke gen_schemas binary");
    assert!(
        output.status.success(),
        "gen_schemas exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let schemas_dir = workspace_root().join("schemas");
    let mut drift: Vec<&str> = Vec::new();
    for name in SCHEMA_FILES {
        let checked_in = std::fs::read(schemas_dir.join(name))
            .unwrap_or_else(|e| panic!("read checked-in {name}: {e}"));
        let regenerated = std::fs::read(tmp.path().join(name))
            .unwrap_or_else(|e| panic!("read regenerated {name}: {e}"));
        if checked_in != regenerated {
            drift.push(name);
        }
    }

    assert!(
        drift.is_empty(),
        "checked-in schemas differ from `gen_schemas` output: {drift:?}. \
         Run `cargo run -p pertylizer --bin gen_schemas` and commit the result."
    );
}

/// Load each schema and validate every example file under
/// `assets/examples/<kind>/*.json`. Catches the case where a schema is up
/// to date with code but doesn't actually match real saved files (e.g. a
/// type change broke serialized form but the schema was regenerated to
/// match the new form, while existing fixtures still use the old form).
#[test]
fn example_files_validate_against_schemas() {
    let schemas_dir = workspace_root().join("schemas");
    let examples_dir = workspace_root().join("assets").join("examples");

    let groups: &[(&str, &str, &str)] = &[
        ("project.schema.json", "projects", "project"),
        ("patch.schema.json", "patches", "patch"),
    ];

    for (schema_file, subdir, label) in groups {
        let schema_path = schemas_dir.join(schema_file);
        let schema: Value = serde_json::from_slice(
            &std::fs::read(&schema_path).unwrap_or_else(|e| panic!("read {schema_path:?}: {e}")),
        )
        .unwrap_or_else(|e| panic!("parse {schema_path:?}: {e}"));

        let validator =
            Validator::new(&schema).unwrap_or_else(|e| panic!("compile {schema_file}: {e}"));

        let dir = examples_dir.join(subdir);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));

        let mut checked = 0;
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            checked += 1;
            assert_example_validates(&path, &validator, label);
        }
        assert!(checked > 0, "no example {label} files found in {dir:?}");
    }
}

#[test]
fn descriptor_catalog_exposes_ports_and_canonical_compatibility() {
    use pertylizer::module_factory::{ALL_MODULE_TYPES, get_descriptor};

    let path = workspace_root().join("schemas").join("descriptors.json");
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {path:?}: {e}"));

    for source in PortType::ALL {
        let actual = catalog["connection_compatibility"][source.id()]
            .as_array()
            .unwrap_or_else(|| panic!("missing compatibility list for {}", source.id()));
        let expected: Vec<Value> = PortType::ALL
            .into_iter()
            .filter(|destination| source.can_drive(*destination))
            .map(|destination| Value::String(destination.id().to_owned()))
            .collect();
        assert_eq!(
            actual,
            &expected,
            "catalog compatibility differs for {}",
            source.id()
        );
    }

    for &module_type in ALL_MODULE_TYPES.iter() {
        let descriptor = get_descriptor(module_type)
            .unwrap_or_else(|| panic!("missing descriptor for {module_type:?}"));
        let type_key = serde_json::to_value(module_type)
            .expect("serialize module type")
            .as_str()
            .expect("module type serializes as a string")
            .to_owned();
        let module = &catalog["modules"][&type_key];
        assert!(
            module.is_object(),
            "catalog is missing module type {type_key}"
        );
        assert_eq!(module["category"], descriptor.category.id());
        assert_eq!(module["tags"], serde_json::json!(descriptor.tags));
        assert_eq!(module["gui_only"], module_type.is_visualizer());

        let ports = module["ports"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_key} is missing its ports array"));
        assert_eq!(
            ports.len(),
            descriptor.ports.len(),
            "{type_key} port count differs"
        );
        for (actual, expected) in ports.iter().zip(&descriptor.ports) {
            assert_eq!(actual["name"], expected.name.as_str());
            assert_eq!(actual["label"], expected.label);
            assert_eq!(actual["description"], expected.description);
            assert_eq!(actual["direction"], expected.direction.id());
            assert_eq!(actual["signal_type"], expected.port_type.id());
            let value_domain = &actual["value_domain"];
            assert_eq!(value_domain["id"], expected.value_domain.id());
            assert_eq!(
                value_domain["accepted_values"],
                expected.value_domain.accepted_values()
            );
            if let Some(range) = expected.value_domain.nominal_range() {
                assert_eq!(value_domain["nominal_min"], range.min());
                assert_eq!(value_domain["nominal_max"], range.max());
            } else {
                assert!(value_domain.get("nominal_min").is_none());
                assert!(value_domain.get("nominal_max").is_none());
            }
            if let Some(unit) = expected.value_domain.unit() {
                assert_eq!(value_domain["unit"], unit);
            } else {
                assert!(value_domain.get("unit").is_none());
            }
        }
    }
}

fn assert_example_validates(path: &Path, validator: &Validator, label: &str) {
    let body = std::fs::read(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let doc: Value =
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
    let errors: Vec<String> = validator
        .iter_errors(&doc)
        .take(5)
        .map(|e| format!("at {}: {}", e.instance_path(), e))
        .collect();
    if !errors.is_empty() {
        panic!(
            "{label} example {} fails schema validation:\n  {}",
            path.display(),
            errors.join("\n  ")
        );
    }
}
