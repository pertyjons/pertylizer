//! Integration tests for `pertylizer compare`
//! (`pertylizer::compare::run_compare_command`).
//!
//! The metrics themselves are unit-tested against synthetic signals with known
//! deviations; what is left to check at this level is everything around them —
//! that the command reads real WAV files off disk, refuses the argument
//! combinations that would produce a meaningless or destructive result, writes a
//! report that parses, and does not lose a measurement between the metric and
//! the JSON.
//!
//! The renders here go through the same `run_render_command` a corpus run uses,
//! so these also demonstrate the two commands composing: render, render, compare.

use std::path::{Path, PathBuf};

use pertylizer::compare::{
    COMPARE_PROTOCOL_VERSION, CompareCommand, CompareError, run_compare_command,
};
use pertylizer::corpus::{CorpusCaseId, fixtures};
use pertylizer::render::{MixSelection, RenderCommand, WavFormat, run_render_command};
use synth_core::Seconds;

/// Render the named corpus fixture into `dir`, returning the WAV's path.
///
/// A real corpus input rather than a synthetic buffer: the point of these tests
/// is the whole path, and a fixture is what the path will actually carry.
fn render_fixture(case_id: &str, dir: &Path, name: &str) -> PathBuf {
    let case_id: CorpusCaseId = case_id.parse().expect("a well-formed case id");
    let fixture = fixtures::fixture(&case_id).unwrap_or_else(|| panic!("no fixture {case_id}"));
    let project = dir.join(format!("{name}.ptz"));
    (fixture.build)().save(&project).expect("save project");

    let output = dir.join(format!("{name}.wav"));
    run_render_command(&RenderCommand {
        input: project,
        output: output.clone(),
        sample_rate: 44_100,
        format: WavFormat::Float32,
        seconds: Seconds::new(1.0),
        tail: Seconds::new(0.5),
        result_json: None,
        mix: MixSelection::default(),
        argv: Vec::new(),
    })
    .expect("render");
    output
}

fn compare(reference: &Path, candidate: &Path) -> pertylizer::compare::ComparisonReport {
    run_compare_command(&CompareCommand {
        reference: reference.to_path_buf(),
        candidate: candidate.to_path_buf(),
        result_json: None,
        argv: Vec::new(),
    })
    .expect("compare")
}

/// Two renders of one project must come back as no difference anywhere. This is
/// the check a corpus case's `bit-exact` determinism claim rests on, run through
/// the real render and compare paths rather than on synthetic buffers.
#[test]
fn two_renders_of_one_project_report_no_difference() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = render_fixture("CORPUS-0001", dir.path(), "first");
    let second = render_fixture("CORPUS-0001", dir.path(), "second");

    let report = compare(&first, &second);
    assert!(report.files_identical, "{:?}", report.warnings);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let samples = report.samples.expect("same time base");
    assert!(samples.identical);
    assert_eq!(samples.max_abs_error, 0.0);
    assert_eq!(report.level.peak_delta_db, 0.0);
    assert_eq!(report.level.rms_delta_db, 0.0);
    assert_eq!(report.loudness.delta_lu, Some(0.0));
    assert_eq!(
        report.spectrum.expect("comparable").max_abs_delta_db,
        0.0,
        "identical audio must have no spectral difference"
    );
}

/// Two different projects must come back as a difference. Without this the test
/// above would pass just as happily on a comparison that measured nothing.
#[test]
fn two_different_projects_report_a_difference() {
    let dir = tempfile::tempdir().expect("temp dir");
    let subtractive = render_fixture("CORPUS-0001", dir.path(), "subtractive");
    let sweep = render_fixture("CORPUS-0003", dir.path(), "sweep");

    let report = compare(&subtractive, &sweep);
    assert!(!report.files_identical);
    let samples = report.samples.expect("same time base");
    assert!(!samples.identical);
    assert!(samples.max_abs_error > 0.0);
    assert!(
        report.spectrum.expect("comparable").max_abs_delta_db > 1.0,
        "two different patches should differ spectrally"
    );
}

/// Comparing a file with itself would report zeroes that read exactly like "the
/// two renders agree", and one keystroke separates the two paths in a batch
/// script. It has to be refused rather than answered.
#[test]
fn comparing_a_file_with_itself_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let wav = render_fixture("CORPUS-0001", dir.path(), "only");

    let error = run_compare_command(&CompareCommand {
        reference: wav.clone(),
        candidate: wav.clone(),
        result_json: None,
        argv: Vec::new(),
    })
    .expect_err("should refuse");
    assert!(matches!(error, CompareError::SameFile { .. }), "{error}");

    // The same file reached by two spellings of one path is still one file.
    let dotted = dir.path().join(".").join(
        wav.file_name()
            .expect("the render has a file name")
            .to_string_lossy()
            .as_ref(),
    );
    let error = run_compare_command(&CompareCommand {
        reference: wav,
        candidate: dotted,
        result_json: None,
        argv: Vec::new(),
    })
    .expect_err("should refuse");
    assert!(matches!(error, CompareError::SameFile { .. }), "{error}");
}

/// Writing the report over an input would destroy the very file the report
/// claims to describe — and the digest in it was taken before the write, so the
/// result would look clean.
#[test]
fn writing_the_report_over_an_input_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = render_fixture("CORPUS-0001", dir.path(), "first");
    let second = render_fixture("CORPUS-0001", dir.path(), "second");

    for target in [&first, &second] {
        let error = run_compare_command(&CompareCommand {
            reference: first.clone(),
            candidate: second.clone(),
            result_json: Some(target.clone()),
            argv: Vec::new(),
        })
        .expect_err("should refuse");
        assert!(
            matches!(error, CompareError::ReportOverwritesInput),
            "{error}"
        );
    }
}

/// The report has to reach disk as parseable JSON carrying the version, or a
/// harness cannot consume it. Checked against the file rather than the returned
/// struct, because it is the file that anything downstream reads.
#[test]
fn the_report_is_written_as_parseable_json() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = render_fixture("CORPUS-0001", dir.path(), "first");
    let second = render_fixture("CORPUS-0003", dir.path(), "second");
    // Nested, so the run also has to create the directory.
    let report_path = dir.path().join("reports").join("comparison.json");

    let report = run_compare_command(&CompareCommand {
        reference: first,
        candidate: second,
        result_json: Some(report_path.clone()),
        argv: vec!["pertylizer".to_string(), "compare".to_string()],
    })
    .expect("compare");

    let text = std::fs::read_to_string(&report_path).expect("report written");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(
        parsed["protocol_version"].as_u64(),
        Some(u64::from(COMPARE_PROTOCOL_VERSION))
    );
    assert_eq!(
        parsed["reference"]["sha256"].as_str(),
        Some(report.reference.sha256.as_str()),
        "the written report must describe the same files the returned one does"
    );
    assert!(
        parsed["spectrum"]["bands"]
            .as_array()
            .is_some_and(|bands| !bands.is_empty()),
        "the spectrum section must survive serialization"
    );
    assert_eq!(parsed["command"].as_array().map(Vec::len), Some(2));
}

/// A missing input is an error naming the file, not a panic and not a report
/// full of zeroes.
#[test]
fn a_missing_input_is_reported() {
    let dir = tempfile::tempdir().expect("temp dir");
    let real = render_fixture("CORPUS-0001", dir.path(), "real");
    let error = run_compare_command(&CompareCommand {
        reference: real,
        candidate: dir.path().join("never-rendered.wav"),
        result_json: None,
        argv: Vec::new(),
    })
    .expect_err("should fail");
    assert!(
        error.to_string().contains("never-rendered.wav"),
        "the error must name the missing file: {error}"
    );
}
