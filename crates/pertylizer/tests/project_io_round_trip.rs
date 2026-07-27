//! Round-trip tests for project save/load (Phase 0.1 of the
//! `project-io-off-gui-thread` plan).
//!
//! For every bundled example project under `assets/examples/projects/`:
//!   1. Load the file (auto-detecting bundle vs plain JSON).
//!   2. Save it back to a temp path.
//!   3. Load the temp file.
//!   4. Assert the two `ProjectFile`s are equivalent at the JSON-value level
//!      (which captures every field serde sees, NaN-safely).
//!
//! For bundles, the sample library is also round-tripped; we compare the
//! sample IDs and total frame counts after reload to ensure the audio
//! payload survived the trip.
//!
//! These tests pin current behavior before the refactor moves I/O off the
//! GUI thread. After the refactor they must still pass — the file format
//! is unchanged; only the execution thread shifts.

use std::path::Path;

use pertylizer::bundle::{is_zip_file, load_bundle, save_bundle};
use pertylizer::project::{LoadedFile, ProjectFile, load_file};
use synth_sampler::SampleLibrary;

mod common;
use common::{examples_dir, list_example_projects};

/// Round-trip a plain-JSON project file: load → save → load → compare.
fn round_trip_project(path: &Path) {
    let original =
        ProjectFile::load(path).unwrap_or_else(|e| panic!("load {}: {e}", path.display()));

    let tmp = tempfile::NamedTempFile::new().expect("create temp file");
    original
        .save(tmp.path())
        .unwrap_or_else(|e| panic!("save {}: {e}", tmp.path().display()));

    let reloaded = ProjectFile::load(tmp.path())
        .unwrap_or_else(|e| panic!("reload {}: {e}", tmp.path().display()));

    let original_value = serde_json::to_value(&original).expect("serialize original");
    let reloaded_value = serde_json::to_value(&reloaded).expect("serialize reloaded");
    assert_eq!(
        original_value,
        reloaded_value,
        "project round-trip mismatch for {}",
        path.display()
    );
}

/// Round-trip a ZIP bundle: load → save → load → compare project + samples.
fn round_trip_bundle(path: &Path) {
    let mut original_lib = SampleLibrary::default();
    let original_project = load_bundle(path, &mut original_lib)
        .unwrap_or_else(|e| panic!("load bundle {}: {e}", path.display()));

    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let tmp_path = tmp_dir.path().join("round_trip.zip");
    save_bundle(&original_project, &original_lib, &tmp_path)
        .unwrap_or_else(|e| panic!("save bundle {}: {e}", tmp_path.display()));

    let mut reloaded_lib = SampleLibrary::default();
    let reloaded_project = load_bundle(&tmp_path, &mut reloaded_lib)
        .unwrap_or_else(|e| panic!("reload bundle {}: {e}", tmp_path.display()));

    let original_value = serde_json::to_value(&original_project).expect("serialize original");
    let reloaded_value = serde_json::to_value(&reloaded_project).expect("serialize reloaded");
    assert_eq!(
        original_value,
        reloaded_value,
        "bundle project-data round-trip mismatch for {}",
        path.display()
    );

    let mut original_ids: Vec<u64> = original_lib.list().iter().map(|m| m.id.as_u64()).collect();
    let mut reloaded_ids: Vec<u64> = reloaded_lib.list().iter().map(|m| m.id.as_u64()).collect();
    original_ids.sort_unstable();
    reloaded_ids.sort_unstable();
    assert_eq!(
        original_ids,
        reloaded_ids,
        "bundle sample IDs differ after round-trip for {}",
        path.display()
    );

    for meta in original_lib.list() {
        let id = meta.id;
        let original_sample = original_lib.get(id).expect("original sample present");
        let reloaded_sample = reloaded_lib.get(id).expect("reloaded sample present");
        assert_eq!(
            original_sample.meta.frame_count,
            reloaded_sample.meta.frame_count,
            "sample {} frame count changed in {}",
            id.as_u64(),
            path.display()
        );
        assert_eq!(
            original_sample.meta.channels,
            reloaded_sample.meta.channels,
            "sample {} channel count changed in {}",
            id.as_u64(),
            path.display()
        );
        assert_eq!(
            original_sample.data.len(),
            reloaded_sample.data.len(),
            "sample {} data length changed in {}",
            id.as_u64(),
            path.display()
        );
    }
}

#[test]
fn every_example_project_round_trips() {
    let files = list_example_projects();
    assert!(
        !files.is_empty(),
        "no example projects found under {}",
        examples_dir().display()
    );

    let mut failures: Vec<String> = Vec::new();

    for path in files {
        let result = std::panic::catch_unwind(|| {
            if is_zip_file(&path) {
                round_trip_bundle(&path);
            } else {
                let loaded = load_file(&path)
                    .unwrap_or_else(|e| panic!("auto-load {}: {e}", path.display()));
                match loaded {
                    LoadedFile::Project(_) => round_trip_project(&path),
                    LoadedFile::Bundle(_) => round_trip_bundle(&path),
                    LoadedFile::Patch(_) => {
                        // Example dir is for projects; warn but don't fail
                        eprintln!(
                            "note: {} is a patch file, skipping (not a project)",
                            path.display()
                        );
                    }
                }
            }
        });

        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    panic
                        .downcast_ref::<&'static str>()
                        .map(|s| (*s).to_string())
                })
                .unwrap_or_else(|| "<non-string panic>".to_string());
            failures.push(format!("{}: {msg}", path.display()));
        }
    }

    assert!(
        failures.is_empty(),
        "round-trip failed for {} example(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
