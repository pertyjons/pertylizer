//! The committed V2 reference corpus has to agree with the code that describes
//! it.
//!
//! Three things can drift apart here and all three are silent: the manifest can
//! stop matching the model, the digests can stop matching the files, and the
//! files can stop matching the builders that are supposed to generate them. Each
//! failure turns the corpus from a baseline into four arbitrary projects, so
//! each gets a test.

use std::path::{Path, PathBuf};

use pertylizer::audio::arrangement_render::render_arrangement_to_buffer;
use pertylizer::corpus::{CORPUS_DIR, CorpusCategory, CorpusManifest, MANIFEST_FILE, fixtures};
use pertylizer::project::ProjectFile;
use pertylizer::render::headless::load_project_file;
use pertylizer::render::receipt::FileDigest;
/// The corpus directory in the checked-out workspace.
fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join(CORPUS_DIR)
}

fn manifest() -> CorpusManifest {
    let path = corpus_dir().join(MANIFEST_FILE);
    CorpusManifest::load(&path).unwrap_or_else(|e| panic!("loading {}: {e}", path.display()))
}

/// Loading applies every consistency rule, so this covers identifier shape and
/// uniqueness, claim classes, rationale presence, render bounds, and full
/// category coverage in one assertion.
#[test]
fn the_committed_manifest_loads_and_validates() {
    let manifest = manifest();
    assert!(
        !manifest.cases.is_empty(),
        "a corpus with no cases measures nothing"
    );
}

/// A digest mismatch means the input changed under the corpus, which silently
/// invalidates every baseline measured against it.
#[test]
fn every_case_input_exists_and_matches_its_digest() {
    let dir = corpus_dir();
    let problems = manifest().verify_inputs(&dir);
    assert!(
        problems.is_empty(),
        "corpus inputs are stale — re-run `cargo run -p pertylizer --bin gen_corpus`:\n{}",
        problems.join("\n")
    );
}

/// The committed projects must be exactly what the builders produce. Without
/// this the fixture code becomes documentation of a file it no longer describes,
/// and regenerating the corpus would change renders nobody asked to change.
#[test]
fn every_fixture_matches_its_committed_project() {
    let dir = corpus_dir();
    for fixture in fixtures::FIXTURES {
        let path = dir.join(fixtures::PROJECTS_SUBDIR).join(fixture.file_name);
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        // Serialized the same way `ProjectFile::save` does, so a difference is a
        // difference in content rather than in formatting.
        let rebuilt = serde_json::to_string_pretty(&(fixture.build)())
            .unwrap_or_else(|e| panic!("{}: {e}", fixture.case_id));
        assert_eq!(
            on_disk.trim_end(),
            rebuilt.trim_end(),
            "{} is out of date with its builder — re-run \
             `cargo run -p pertylizer --bin gen_corpus`",
            fixture.case_id
        );
    }
}

/// Every generated fixture belongs to a case and every case that names a
/// generated file has a builder. A fixture with no case is written and never
/// rendered; a case naming a missing builder silently keeps whatever bytes were
/// last committed.
#[test]
fn fixtures_and_cases_refer_to_each_other() {
    let manifest = manifest();
    for fixture in fixtures::FIXTURES {
        let case = manifest
            .case(&fixture.case_id())
            .unwrap_or_else(|| panic!("fixture {} has no case", fixture.case_id));
        assert_eq!(
            case.input,
            format!("{}/{}", fixtures::PROJECTS_SUBDIR, fixture.file_name),
            "{} names a different input than its fixture writes",
            fixture.case_id
        );
    }
    for case in &manifest.cases {
        let generated = case
            .input
            .starts_with(&format!("{}/", fixtures::PROJECTS_SUBDIR));
        assert_eq!(
            generated,
            fixtures::fixture(&case.id).is_some(),
            "{} is inconsistent: an input under {}/ must have a builder, and only those may live \
             there",
            case.id,
            fixtures::PROJECTS_SUBDIR
        );
    }
}

/// A corpus input the loader cannot read is not an input. Parsed rather than
/// merely hashed, because a digest matches a corrupt file just as happily.
#[test]
fn every_case_input_parses_as_a_project() {
    let dir = corpus_dir();
    for case in &manifest().cases {
        let path = case.input_path(&dir);
        let project = ProjectFile::load(&path)
            .unwrap_or_else(|e| panic!("{}: loading {}: {e}", case.id, path.display()));
        assert_eq!(project.file_type, "project", "{}", case.id);
        assert!(
            !project.instruments.is_empty(),
            "{} has no instrument to render",
            case.id
        );
    }
}

/// The manifest is the coverage matrix for the master plan's bounded corpus, so
/// a reader must be able to answer "is this category covered" from it alone.
/// `validate` already enforces the rule; this states the current split so a
/// change to it is a deliberate edit rather than a side effect.
#[test]
fn every_category_is_either_covered_or_recorded_as_a_gap() {
    let manifest = manifest();
    for category in CorpusCategory::all() {
        let covered = manifest.cases.iter().any(|c| c.category == category);
        let planned = manifest.planned.iter().any(|p| p.category == category);
        assert!(
            covered ^ planned,
            "{category:?} is {}",
            if covered {
                "both covered and listed as planned"
            } else {
                "neither covered nor listed as planned"
            }
        );
    }
    // Counted over *distinct* covered categories, not over cases: a category may
    // grow a second case (the README's "Adding a case" says so), and comparing
    // `cases.len()` against the enum would turn that addition into a failure of
    // a test that is about coverage rather than about case count.
    let covered: std::collections::BTreeSet<CorpusCategory> =
        manifest.cases.iter().map(|case| case.category).collect();
    assert_eq!(
        covered.len() + manifest.planned.len(),
        CorpusCategory::all().count(),
        "every category is covered by at least one case or recorded as one gap"
    );
}

/// The generator is idempotent: running it on an unchanged corpus must produce
/// the same digests it already records. A generator that churned would make
/// every regeneration look like a corpus change.
#[test]
fn regenerating_reproduces_the_recorded_digests() {
    let temp = tempfile::tempdir().expect("temp dir");
    let written = fixtures::write_all(temp.path()).expect("write fixtures");
    let manifest = manifest();
    for (fixture, path) in fixtures::FIXTURES.iter().zip(&written) {
        let case = manifest
            .case(&fixture.case_id())
            .unwrap_or_else(|| panic!("no case for {}", fixture.case_id));
        let digest = FileDigest::of(path).expect("digest");
        assert_eq!(
            digest.sha256, case.sha256,
            "{} regenerates to different bytes than the manifest records",
            fixture.case_id
        );
    }
}

/// RMS for one channel over an exact stereo-frame range.
fn channel_rms(samples: &[f32], frames: std::ops::Range<usize>, channel: usize) -> f64 {
    let sample_count = u32::try_from(frames.len()).expect("test window fits in u32");
    let sum = frames
        .map(|frame| f64::from(samples[frame * 2 + channel]))
        .map(|sample| sample * sample)
        .sum::<f64>();
    (sum / f64::from(sample_count)).sqrt()
}

/// Locate audible starts after a sustained quiet interval in stereo audio.
fn audible_onsets(samples: &[f32], threshold: f32, quiet_frames: usize) -> Vec<usize> {
    let mut quiet = quiet_frames;
    let mut onsets = Vec::new();
    for (frame, stereo) in samples.as_chunks::<2>().0.iter().enumerate() {
        if stereo[0].abs().max(stereo[1].abs()) <= threshold {
            quiet = quiet.saturating_add(1);
        } else {
            if quiet >= quiet_frames {
                onsets.push(frame);
            }
            quiet = 0;
        }
    }
    onsets
}

#[test]
fn keyboard_panner_fixture_is_gated_and_moves_across_channels() {
    let path = corpus_dir().join("projects/keyboard-panner-stereo.ptz");
    let project = load_project_file(&path).expect("load keyboard-panner fixture");
    assert!(
        project.report.is_clean(),
        "{:?}",
        project.report.diagnostics
    );
    let rendered = render_arrangement_to_buffer(
        &project.session,
        &project.sample_library,
        &project.song,
        0,
        2_880,
    )
    .expect("render keyboard-panner fixture");
    assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);

    // 120 BPM maps the three note starts to 0.0, 0.5, and 1.0 seconds.
    // These windows avoid attack, note-off, and the short release tail.
    let rms_lr = |window: std::ops::Range<usize>| {
        (
            channel_rms(&rendered.samples, window.clone(), 0),
            channel_rms(&rendered.samples, window, 1),
        )
    };
    let (low_l, low_r) = rms_lr(4_410..13_230);
    let (center_l, center_r) = rms_lr(26_460..35_280);
    let (high_l, high_r) = rms_lr(48_510..57_330);

    assert!(low_l > low_r * 1.5, "low note: left={low_l}, right={low_r}");
    assert!(
        (center_l - center_r).abs() < center_l * 0.01,
        "center note: left={center_l}, right={center_r}"
    );
    assert!(
        high_r > high_l * 1.5,
        "high note: left={high_l}, right={high_r}"
    );
}

/// CORPUS-0007's Script program is the amplifier's only CV source, so a
/// build that fails to install or evaluate it renders silence — and silence
/// is bit-exact deterministic, so every digest test would stay green. This
/// guards the manifest's audibility claim (CORPUS-0007-P2) with a render.
#[test]
fn yams_control_fixture_is_audible_only_through_its_script() {
    let path = corpus_dir().join("projects/yams-control.ptz");
    let project = load_project_file(&path).expect("load yams-control fixture");
    assert!(
        project.report.is_clean(),
        "{:?}",
        project.report.diagnostics
    );
    let rendered = render_arrangement_to_buffer(
        &project.session,
        &project.sample_library,
        &project.song,
        0,
        2_880,
    )
    .expect("render yams-control fixture");
    assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);

    // Steady-state window inside the held note: 0.2..0.9 s at 44.1 kHz.
    let rms = channel_rms(&rendered.samples, 8_820..39_690, 0);
    assert!(
        rms > 1.0e-3,
        "CORPUS-0007 must be audible through out1 = 0.65; got RMS {rms}"
    );
}

/// CORPUS-0008's AudioScript applies +0.75 to the left copy and -0.5 to the
/// right copy of one sine, so the channel RMS ratio pins audio-rate program
/// evaluation and both stereo cables at once (CORPUS-0008-P1/P2). Neither
/// digest tests nor determinism runs can see a silent or mono fallback.
#[test]
fn yams_audio_script_fixture_applies_the_authored_signed_gains() {
    let path = corpus_dir().join("projects/yams-audio-script.ptz");
    let project = load_project_file(&path).expect("load yams-audio-script fixture");
    assert!(
        project.report.is_clean(),
        "{:?}",
        project.report.diagnostics
    );
    let rendered = render_arrangement_to_buffer(
        &project.session,
        &project.sample_library,
        &project.song,
        0,
        2_880,
    )
    .expect("render yams-audio-script fixture");
    assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);

    let left = channel_rms(&rendered.samples, 8_820..39_690, 0);
    let right = channel_rms(&rendered.samples, 8_820..39_690, 1);
    assert!(
        right > 1.0e-3,
        "CORPUS-0008 right channel must be audible; got RMS {right}"
    );
    let ratio = left / right;
    assert!(
        (1.45..=1.55).contains(&ratio),
        "authored gains 0.75/-0.5 give |L|/|R| = 1.5; got left={left}, right={right}, ratio={ratio}"
    );
}

#[test]
fn tempo_fixture_has_an_observable_interval_after_the_step() {
    let path = corpus_dir().join("projects/tempo-map-arrangement.ptz");
    let project = load_project_file(&path).expect("load tempo-map fixture");
    assert!(
        project.report.is_clean(),
        "{:?}",
        project.report.diagnostics
    );
    let rendered = render_arrangement_to_buffer(
        &project.session,
        &project.sample_library,
        &project.song,
        0,
        5_760,
    )
    .expect("render tempo-map fixture");
    assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);

    // Every note is short enough to leave a long silent gap. Detect the six
    // actual attacks rather than trusting the same tick conversion that the
    // renderer could fail to consume.
    let onsets = audible_onsets(&rendered.samples, 1.0e-4, 2_000);
    assert_eq!(onsets.len(), 6, "rendered attacks: {onsets:?}");
    let intervals: Vec<_> = onsets.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let first_ramp_interval = intervals[0];
    let second_ramp_interval = intervals[1];
    let before_step = intervals[3];
    let after_step = intervals[4];

    assert!(
        first_ramp_interval > second_ramp_interval,
        "the rendered ramp must change equal-tick spacing: {intervals:?}"
    );
    assert!(
        after_step * 10 > before_step * 14,
        "the rendered 120 BPM segment needs a longer post-step interval: {intervals:?}"
    );
}

#[test]
fn shared_instrument_fixture_preserves_both_track_faders() {
    let path = corpus_dir().join("projects/shared-instrument-tracks.ptz");
    let project = load_project_file(&path).expect("load shared-instrument fixture");
    assert!(
        project.report.is_clean(),
        "{:?}",
        project.report.diagnostics
    );
    let track_ids: Vec<_> = project.song.read().tracks().map(|track| track.id).collect();
    assert_eq!(track_ids.len(), 2);

    let render = || {
        render_arrangement_to_buffer(
            &project.session,
            &project.sample_library,
            &project.song,
            0,
            3_840,
        )
        .expect("render shared-instrument fixture")
    };
    let render_solo = |track_id| {
        project.song.write().set_solo_only(track_id);
        let rendered = render();
        assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
        rendered.samples
    };
    let unity = render_solo(track_ids[0]);
    let half = render_solo(track_ids[1]);
    let unity_rms = channel_rms(&unity, 4_410..35_280, 0);
    let half_rms = channel_rms(&half, 4_410..35_280, 0);
    let ratio = half_rms / unity_rms;

    assert!(
        (0.45..=0.55).contains(&ratio),
        "shared instrument lost track-local gain: unity={unity_rms}, half={half_rms}, ratio={ratio}"
    );

    // Re-enable both simultaneous streams. Their full mix must equal the sum
    // of the independently rendered track contributions; otherwise one track
    // replaced the other or one fader was applied to both sets of voices.
    {
        let mut song = project.song.write();
        for track_id in &track_ids {
            if let Some(track) = song.track_mut(*track_id) {
                track.set_solo(false);
            }
        }
    }
    let full = render();
    assert!(full.warnings.is_empty(), "{:?}", full.warnings);
    assert_eq!(full.samples.len(), unity.len());
    assert_eq!(full.samples.len(), half.len());

    let (error_energy, expected_energy) = full.samples.iter().zip(&unity).zip(&half).fold(
        (0.0_f64, 0.0_f64),
        |(error, expected), ((full, unity), half)| {
            let expected_sample = f64::from(*unity) + f64::from(*half);
            let delta = f64::from(*full) - expected_sample;
            (
                error + delta * delta,
                expected + expected_sample * expected_sample,
            )
        },
    );
    let relative_error = (error_energy / expected_energy).sqrt();
    assert!(
        relative_error < 1.0e-5,
        "simultaneous shared-track mix differs from independent contributions: {relative_error}"
    );
}
