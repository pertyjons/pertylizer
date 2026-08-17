//! The crate can be deleted, and it reaches nothing it may not.
//!
//! The Phase 1 exit gate's last bullet is "the crate can be deleted without affecting V1
//! behavior or public APIs". That is a claim about coupling, and coupling is what rots
//! first, so it is checked from the manifests rather than asserted in prose. Two halves:
//! nothing depends on this crate, and this crate depends on nothing the work list
//! forbids. An earlier form of this check tested only the first, which would have passed
//! while the crate imported the GUI.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `crates/*/Cargo.toml` in the workspace.
fn crate_manifests() -> Vec<(String, String)> {
    let crates = repo_root().join("crates");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&crates).expect("the crates directory exists");
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if manifest.is_file() {
            let name = entry
                .file_name()
                .to_str()
                .expect("crate directory names are UTF-8")
                .to_owned();
            out.push((name, read(&manifest)));
        }
    }
    assert!(
        out.len() > 5,
        "expected to find the workspace's crates, found {}",
        out.len()
    );
    out
}

#[test]
fn no_workspace_crate_depends_on_the_experimental_crate() {
    let mut dependents = Vec::new();
    for (name, manifest) in crate_manifests() {
        if name == "synth_engine_v2" {
            continue;
        }
        if manifest.contains("synth_engine_v2") {
            dependents.push(name);
        }
    }
    assert!(
        dependents.is_empty(),
        "these crates depend on the experimental crate, so it can no longer be deleted: {dependents:?}"
    );
}

/// The dependency names in a manifest's `[dependencies]` table.
///
/// Only that table: `edition.workspace = true` is a package key, not a dependency, and a
/// scan over the whole file cannot tell them apart.
fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[dependencies]";
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            names.push(key.trim().to_owned());
        }
    }
    names
}

#[test]
fn the_experimental_crate_reaches_nothing_the_work_list_forbids() {
    let manifest = read(&repo_root().join("crates/synth_engine_v2/Cargo.toml"));
    let dependencies = dependency_names(&manifest);
    assert!(
        !dependencies.is_empty(),
        "no `[dependencies]` table was found, so this check would pass vacuously"
    );

    // The Phase 1 work list fixes the surface: `synth_core` for existing domain
    // newtypes, selected DSP kernels or module implementations, and no GUI, MCP, OSC,
    // CPAL, filesystem, or project-loading dependency. `thiserror` is the repository's
    // mandated error-derive crate.
    let allowed = ["synth_core", "synth_dsp", "synth_modules", "thiserror"];
    for name in &dependencies {
        assert!(
            allowed.contains(&name.as_str()),
            "`{name}` is not on this crate's dependency allowlist; widening the surface is a \
             deliberate change to the Phase 1 work list, not a manifest edit"
        );
    }

    // Named explicitly as well as excluded by the allowlist, so the failure message says
    // *why* rather than only that the name is unfamiliar.
    for forbidden in [
        "egui",
        "eframe",
        "cpal",
        "midir",
        "rmcp",
        "rosc",
        "tokio",
        "axum",
        "hound",
        "zip",
        "dirs",
        "synth_engine",
        "synth_mcp",
        "synth_osc",
        "synth_sequencer",
        "pertylizer",
    ] {
        assert!(
            !dependencies.iter().any(|name| name == forbidden),
            "the experimental crate must not depend on {forbidden}: the renderer cannot query a \
             device, a protocol, a project, or a file"
        );
    }
}

#[test]
fn the_workspace_lists_the_crate_as_a_member() {
    // Otherwise none of these tests run, and a green build would mean nothing.
    let workspace = read(&repo_root().join("Cargo.toml"));
    assert!(
        workspace.contains("\"crates/synth_engine_v2\""),
        "the workspace must list the crate, or `cargo test --workspace` skips it entirely"
    );
}
