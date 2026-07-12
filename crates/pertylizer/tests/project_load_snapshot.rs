//! Snapshot tests for project loading (Phase 0.2 of the
//! `project-io-off-gui-thread` plan).
//!
//! For each bundled example project, build a stable summary of the data
//! that crosses the file → shared-state boundary (instrument shape, song
//! structure, global state, sample inventory) and compare it against a
//! committed fixture. The fixture is the *expected shape* of what
//! `project_apply::apply_project` (Phase 1.1) will eventually write into
//! shared state.
//!
//! Today the fixture is generated from `ProjectFile::load`. After the
//! refactor the same summary will be computed by reading shared state back
//! after `apply_project` runs — and must produce the same JSON, proving
//! the new path preserves behavior.
//!
//! Workflow:
//!   - Run `cargo test --test project_load_snapshot` to verify.
//!   - Set `UPDATE_SNAPSHOTS=1` to (re)generate fixtures from current
//!     behavior. Review the diff before committing.

use std::path::{Path, PathBuf};

use pertylizer::bundle::{is_zip_file, load_bundle};
use pertylizer::patch::InstrumentState;
use pertylizer::project::{LoadedFile, ProjectFile, load_file};
use serde::Serialize;
use synth_sampler::SampleLibrary;

mod common;
use common::{examples_dir, list_example_projects};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_snapshots")
}

/// Sanitize an example filename for use as a fixture filename.
fn fixture_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let mut sanitized = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    sanitized
}

#[derive(Serialize)]
struct ProjectSnapshot {
    file_type: String,
    version: String,
    instrument_count: usize,
    active_instrument_id: u64,
    author_present: bool,
    instruments: Vec<InstrumentSummary>,
    song: SongSummary,
    global: GlobalSummary,
    samples: SampleSummary,
}

#[derive(Serialize)]
struct InstrumentSummary {
    id: u64,
    name: String,
    channel: u8,
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
    key_range: (u8, u8),
    transpose_semitones: f32,
    oversampling: u8,
    category: u8,
    description_len: usize,
    color_present: bool,
    allocation_mode: String,
    unison_detune_cents: f32,
    unison_spread: f32,
    max_voices: usize,
    velocity_amp_sensitivity: f32,
    velocity_filter_sensitivity: f32,
    sidechain_source_id: Option<u64>,
    patch_name: String,
    patch_description_present: bool,
    module_count: usize,
    connection_count: usize,
    effect_chain_order_len: usize,
    group_count: usize,
}

#[derive(Serialize)]
struct SongSummary {
    name: String,
    author: String,
    default_tempo_bpm: f32,
    time_signature_numerator: u8,
    time_signature_denominator: u8,
    pattern_count: usize,
    track_count: usize,
    placement_count: usize,
    total_notes: usize,
    tempo_change_count: usize,
}

#[derive(Serialize)]
struct GlobalSummary {
    master_volume: f32,
    octave_offset: i32,
    glide_time_seconds: f32,
}

#[derive(Serialize)]
struct SampleSummary {
    count: usize,
    total_frames: usize,
}

fn build_instrument_summary(inst: &InstrumentState) -> InstrumentSummary {
    InstrumentSummary {
        id: inst.id.0,
        name: inst.name.clone(),
        channel: inst.channel,
        volume: inst.volume.as_f32(),
        pan: inst.pan.as_f32(),
        muted: inst.muted,
        solo: inst.solo,
        key_range: inst.key_range,
        transpose_semitones: inst.transpose.as_f32(),
        oversampling: inst.oversampling,
        category: inst.category,
        description_len: inst.description.len(),
        color_present: inst.color.is_some(),
        allocation_mode: format!("{:?}", inst.allocation_mode),
        unison_detune_cents: inst.unison_detune.0,
        unison_spread: inst.unison_spread.as_f32(),
        max_voices: inst.max_voices.as_usize(),
        velocity_amp_sensitivity: inst.velocity_amp_sensitivity.as_f32(),
        velocity_filter_sensitivity: inst.velocity_filter_sensitivity.as_f32(),
        sidechain_source_id: inst.sidechain_source_id,
        patch_name: inst.patch.name.clone(),
        patch_description_present: inst.patch.description.is_some(),
        module_count: inst.patch.modules.len(),
        connection_count: inst.patch.connections.len(),
        effect_chain_order_len: inst.patch.settings.effect_chain_order.len(),
        group_count: inst.patch.groups.len(),
    }
}

fn build_song_summary(song: &synth_sequencer::Song) -> SongSummary {
    let total_notes: usize = song.patterns().map(|p| p.note_count()).sum();
    SongSummary {
        name: song.name.clone(),
        author: song.author.clone(),
        default_tempo_bpm: song.default_tempo.as_f32(),
        time_signature_numerator: song.default_time_signature.numerator,
        time_signature_denominator: song.default_time_signature.denominator,
        pattern_count: song.pattern_count(),
        track_count: song.track_count(),
        placement_count: song.arrangement().len(),
        total_notes,
        tempo_change_count: song.tempo_changes().len(),
    }
}

fn build_project_snapshot(project: &ProjectFile, samples: &SampleSummary) -> ProjectSnapshot {
    let instruments: Vec<InstrumentSummary> = project
        .instruments
        .iter()
        .map(build_instrument_summary)
        .collect();

    let global = GlobalSummary {
        master_volume: project.global.master_volume.as_f32(),
        octave_offset: project.global.octave_offset,
        glide_time_seconds: project.global.glide_time.as_f32(),
    };

    ProjectSnapshot {
        file_type: project.file_type.clone(),
        version: project.version.clone(),
        instrument_count: project.instruments.len(),
        active_instrument_id: project.active_instrument_id,
        author_present: project.author.is_some(),
        instruments,
        song: build_song_summary(&project.song),
        global,
        samples: SampleSummary {
            count: samples.count,
            total_frames: samples.total_frames,
        },
    }
}

fn sample_summary_from_library(library: &SampleLibrary) -> SampleSummary {
    let metas = library.list();
    let total_frames = metas.iter().map(|m| m.frame_count.as_usize()).sum();
    SampleSummary {
        count: metas.len(),
        total_frames,
    }
}

fn snapshot_for_example(path: &Path) -> Option<ProjectSnapshot> {
    if is_zip_file(path) {
        let mut lib = SampleLibrary::default();
        let project = load_bundle(path, &mut lib)
            .unwrap_or_else(|e| panic!("load bundle {}: {e}", path.display()));
        let samples = sample_summary_from_library(&lib);
        Some(build_project_snapshot(&project, &samples))
    } else {
        match load_file(path).unwrap_or_else(|e| panic!("load {}: {e}", path.display())) {
            LoadedFile::Project(project) => {
                let samples = SampleSummary {
                    count: 0,
                    total_frames: 0,
                };
                Some(build_project_snapshot(&project, &samples))
            }
            LoadedFile::Bundle(p) => {
                // Auto-detection said bundle even though magic check said no — defer
                let mut lib = SampleLibrary::default();
                let project = load_bundle(&p, &mut lib)
                    .unwrap_or_else(|e| panic!("load bundle {}: {e}", p.display()));
                let samples = sample_summary_from_library(&lib);
                Some(build_project_snapshot(&project, &samples))
            }
            LoadedFile::Patch(_) => None,
        }
    }
}

fn write_fixture(path: &Path, json: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create fixture dir {}: {e}", parent.display()));
    }
    std::fs::write(path, json).unwrap_or_else(|e| panic!("write fixture {}: {e}", path.display()));
}

/// Strip `\r` and trim so snapshot comparison is insensitive to the line
/// endings a fixture happens to be checked out with (CRLF on Windows).
fn normalize_eol(s: &str) -> String {
    s.replace('\r', "").trim().to_string()
}

fn compare_snapshot(name: &str, snapshot: &ProjectSnapshot) {
    let actual = serde_json::to_string_pretty(snapshot).expect("serialize snapshot");
    let fixture_path = fixtures_dir().join(format!("{name}.json"));

    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        write_fixture(&fixture_path, &actual);
        return;
    }

    if !fixture_path.exists() {
        write_fixture(&fixture_path, &actual);
        eprintln!(
            "note: created missing fixture {} — review and commit",
            fixture_path.display()
        );
        return;
    }

    let expected = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", fixture_path.display()));
    // Compare line-ending-agnostically. `serde_json` always emits `\n`, but a
    // fixture checked out on Windows can carry `\r\n` regardless of
    // `.gitattributes`, depending on the runner's autocrlf config. Without this
    // every multi-line fixture would spuriously mismatch on the Windows CI.
    if normalize_eol(&expected) != normalize_eol(&actual) {
        let actual_path = fixture_path.with_extension("actual.json");
        std::fs::write(&actual_path, &actual).ok();
        panic!(
            "snapshot mismatch for {name}\n  expected: {}\n  actual:   {} (also written)\n  run with UPDATE_SNAPSHOTS=1 to refresh",
            fixture_path.display(),
            actual_path.display()
        );
    }
}

#[test]
fn every_example_project_matches_snapshot() {
    let entries = list_example_projects();
    assert!(
        !entries.is_empty(),
        "no example projects found under {}",
        examples_dir().display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in entries {
        let Some(snapshot) = snapshot_for_example(&path) else {
            continue;
        };
        let name = fixture_name(&path);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compare_snapshot(&name, &snapshot)
        }));
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
        "snapshot mismatch for {} example(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
