//! The committed V2 reference corpus has to agree with the code that describes
//! it.
//!
//! Three things can drift apart here and all three are silent: the manifest can
//! stop matching the model, the digests can stop matching the files, and the
//! files can stop matching the builders that are supposed to generate them. Each
//! failure turns the corpus from a baseline into four arbitrary projects, so
//! each gets a test.

use std::path::{Path, PathBuf};

use pertylizer::corpus::{CORPUS_DIR, CorpusCategory, CorpusManifest, MANIFEST_FILE, fixtures};
use pertylizer::project::ProjectFile;
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
