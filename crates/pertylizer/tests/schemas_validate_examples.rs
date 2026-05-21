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
    "awe-preset.schema.json",
    "bundle-metadata.schema.json",
];

/// Regenerate schemas into a temp directory and byte-compare with the
/// checked-in files under `schemas/`. Fails if any file differs — the
/// developer needs to run `cargo run -p pertylizer --bin gen_schemas` and
/// commit the result.
#[test]
fn checked_in_schemas_match_generated() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = env!("CARGO_BIN_EXE_gen_schemas");

    // The binary writes into `<workspace>/schemas`. We point it at the temp
    // dir by setting CARGO_MANIFEST_DIR-style override via a wrapper. Since
    // the binary derives its output path from its own CARGO_MANIFEST_DIR
    // (resolved at compile time), we instead let it write to the real
    // location and copy from there — but that would clobber the working
    // tree. Easier: invoke it pointing the working directory inside the
    // tempdir, but it derives path at compile time so cwd doesn't matter.
    //
    // Solution: read the checked-in schemas and the freshly-generated ones
    // from the real `schemas/` dir, after backing the real ones up to
    // tempdir first.
    let schemas_dir = workspace_root().join("schemas");

    // Snapshot current contents.
    let mut snapshots = Vec::new();
    for name in SCHEMA_FILES {
        let path = schemas_dir.join(name);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        snapshots.push((name, body));
    }

    // Regenerate in-place.
    let output = Command::new(bin)
        .output()
        .expect("invoke gen_schemas binary");
    assert!(
        output.status.success(),
        "gen_schemas exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    // Compare and restore if drift detected.
    let mut drift: Vec<&str> = Vec::new();
    for (name, original) in &snapshots {
        let path = schemas_dir.join(name);
        let regenerated = std::fs::read(&path).unwrap_or_default();
        if regenerated != *original {
            drift.push(name);
        }
        // Always restore so the test is idempotent and doesn't leave the
        // working tree modified if a different schema drifts later.
        std::fs::write(&path, original).expect("restore original");
    }

    // Save tempdir until end so the dir guard outlives the test body.
    drop(tmp);

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
        ("awe-preset.schema.json", "awe", "awe preset"),
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

fn assert_example_validates(path: &Path, validator: &Validator, label: &str) {
    let body = std::fs::read(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let doc: Value =
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
    let mut errors: Vec<String> = validator
        .iter_errors(&doc)
        .map(|e| format!("at /{}: {}", e.instance_path, e))
        .collect();
    if !errors.is_empty() {
        errors.truncate(5);
        panic!(
            "{label} example {} fails schema validation:\n  {}",
            path.display(),
            errors.join("\n  ")
        );
    }
}
