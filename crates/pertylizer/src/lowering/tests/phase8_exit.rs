//! Phase 8's bounded corpus qualification: pinned inputs, named refusals and
//! deterministic V2 renders. Whole-project V1/V2 orchestration remains ADR-0028's.
use super::*;
use crate::corpus::{CORPUS_DIR, CorpusManifest};
use crate::lowering::render::{OutputPolicy, smoke_render_project};
use sha2::{Digest, Sha256};

fn sample_digest(samples: &[f32]) -> String {
    let mut hash = Sha256::new();
    for sample in samples {
        hash.update(sample.to_bits().to_le_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn phase8_corpus_inputs_render_deterministically_or_refuse_with_named_diagnostics() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CORPUS_DIR);
    let manifest = CorpusManifest::load(&root.join("manifest.json")).expect("manifest");
    assert!(
        manifest.verify_inputs(&root).is_empty(),
        "corpus input bytes must stay pinned"
    );
    assert_eq!(manifest.cases.len(), 10, "qualify every pinned case");
    let mut rendered_cases = Vec::new();
    for case in &manifest.cases {
        let project =
            crate::project::ProjectFile::load(case.input_path(&root)).expect("pinned project");
        let rate = case.render.sample_rate.as_u32();
        let render = |block| {
            let profile = HostProfile::harness(
                SampleRate::new(case.render.sample_rate.as_f32()).expect("rate"),
                FrameCount::new(block),
                ChannelLayout::Stereo,
            )
            .expect("profile");
            // This is the bounded smoke API's complete arrangement plus one-second tail,
            // not the manifest's independently specified V1 window or a shared request.
            smoke_render_project(
                &project.instruments,
                &project.song,
                &project.global,
                profile,
                FrameCount::new(u64::from(rate)),
                OutputPolicy::Parity,
            )
        };
        let baseline = render(256);
        let refused = baseline
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused);
        assert_eq!(
            baseline.samples.is_empty(),
            refused,
            "{}: partial or silent refusal",
            case.id
        );
        assert_eq!(
            baseline.fidelity().admits_parity_comparison(),
            baseline.diagnostics.is_empty()
        );
        if !refused {
            assert!(
                baseline.is_audible(),
                "{} must render finite nonzero audio",
                case.id
            );
            rendered_cases.push(case.id.to_string());
        }
        let digest = sample_digest(&baseline.samples);
        for block in [37, 64, 512] {
            let other = render(block);
            assert_eq!(
                other.diagnostics, baseline.diagnostics,
                "{} block {block}",
                case.id
            );
            assert_eq!(
                sample_digest(&other.samples),
                digest,
                "{} block {block}",
                case.id
            );
            assert_eq!(other.lowered_events, baseline.lowered_events);
        }
        let row = serde_json::json!({
            "case": case.id.to_string(), "input_sha256": case.sha256,
            "sample_rate": rate, "smoke_tail_frames": rate,
            "outcome": if refused { "refused" } else if baseline.diagnostics.is_empty() { "rendered-faithful" } else { "rendered-with-markers" },
            "frames": baseline.samples.len() / 2, "lowered_events": baseline.lowered_events.get(),
            "samples_sha256": if refused { None } else { Some(digest) },
            "parity_eligible": baseline.fidelity().admits_parity_comparison(),
            "diagnostics": baseline.diagnostics.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>()
        });
        println!("P08_CORPUS {row}");
    }
    assert_eq!(
        rendered_cases,
        ["CORPUS-0001", "CORPUS-0003", "CORPUS-0005", "CORPUS-0009"]
    );
}

/// Two copies of one patch have separate nonlinear inserts and delay histories.
/// The sum of isolated routes is the control for cross-channel state contamination.
#[test]
fn two_channels_on_one_patch_keep_independent_insert_state_and_tails() {
    use super::phase8::{
        corpus_with_inserts, instrument_with, project_profile, two_instrument_song, unity_master,
    };
    let template = corpus_with_inserts(&["dst-1", "dly-1"]);
    let patch = template.instruments[0].patch.clone();
    let mut first = instrument_with(0, 0.8);
    first.patch = patch.clone();
    let mut second = instrument_with(1, 0.4);
    second.patch = patch;
    let song = two_instrument_song(1);
    let render = |instruments: &[crate::patch::InstrumentState]| {
        let result = smoke_render_project(
            instruments,
            &song,
            &unity_master(),
            project_profile(),
            FrameCount::new(96_000),
            OutputPolicy::Headroom,
        );
        assert!(result.is_audible(), "{:?}", result.diagnostics);
        result.samples
    };
    let assert_sum = |both: &[f32], a: &[f32], b: &[f32]| {
        assert_eq!(both.len(), a.len());
        assert_eq!(both.len(), b.len());
        for ((actual, first), second) in both.iter().zip(a).zip(b) {
            assert_eq!(actual.to_bits(), (first + second).to_bits());
        }
    };
    let a = render(std::slice::from_ref(&first));
    let b = render(std::slice::from_ref(&second));
    let both = render(&[first.clone(), second.clone()]);
    assert_sum(&both, &a, &b);
    // Remove both inserts only from A. A changes, B's original voice and complete
    // ring-out remain the exact other summand even while A's notes retrigger.
    first.patch = corpus_with_inserts(&[]).instruments[0].patch.clone();
    let dry_a = render(std::slice::from_ref(&first));
    let changed = render(&[first, second.clone()]);
    assert_ne!(a, dry_a);
    assert_sum(&changed, &dry_a, &b);
    // B's single note has finished well before 2.6 seconds; only its delay rings.
    second.patch = corpus_with_inserts(&[]).instruments[0].patch.clone();
    let dry_b = render(&[second]);
    let from = 2 * 124_800; // 2.6 seconds at the fixture's 48 kHz.
    let to = 2 * 168_000; // 3.5 seconds.
    assert!(dry_b[from..to].iter().all(|sample| *sample == 0.0));
    assert!(b[from..to].iter().any(|sample| sample.abs() > 1e-4));
}
