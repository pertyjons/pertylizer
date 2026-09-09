//! EVD-0021: the session share's high-water over every saved project (`P08-S004`).
//!
//! ADR-0054 clause 2 stages the reselection of a producer share at the first real consumer
//! that measures its occupancy. The session share's one plan-dependent contributor is
//! ADR-0051's locate catch-up — one event per writable control per node, plus the boundary
//! release — which admission charges exactly (`HOST-INV-022`), so its occupancy is not a
//! runtime observation but the session row of every real project's admission report. The
//! survey below reads that row for every saved project the repository holds and reports the
//! largest request among the projects that reach a plan.
//!
//! The control, run first: a plan with no writable control requests exactly one, the
//! boundary release alone. A method that reported zero for it, or one that counted controls
//! a write cannot reach, would be counting something other than the batch.
use super::*;
use synth_engine_v2::quantities::EventCount;
use synth_engine_v2::report::{ResourceAmount, ResourceField};

use crate::lowering::render::{OutputPolicy, smoke_render_project};

/// The session row's requested amount, or `None` where the report has no such row.
fn session_request(render: &super::super::render::SmokeRender) -> Option<u64> {
    let row = render
        .report
        .as_ref()?
        .row(ResourceField::SessionEventShare)?;
    match row.requested() {
        ResourceAmount::Events(count) => Some(u64::from(count.get())),
        ResourceAmount::EventsBeyondCount(beyond) => Some(beyond.get()),
        _ => None,
    }
}

/// The control: a plan holding no writable control requests one event, the boundary
/// release, whatever the profile's share.
#[test]
fn a_plan_with_no_writable_control_requests_one_session_event() {
    use synth_engine_v2::compile::{RenderConfig, compile};
    use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, PortId, SignalDomain};
    let source = NodeId::new(1);
    let output = NodeId::new(2);
    let ir = GraphIr::builder()
        .node(source, IrNodeKind::Silence, ExecutionScope::Global)
        .node(output, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (source, PortId::FIRST),
            (output, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a silent plan");
    let outcome = compile(&ir, &RenderConfig::new(harness_profile()));
    let row = outcome
        .report()
        .row(ResourceField::SessionEventShare)
        .expect("the session row");
    assert_eq!(
        row.requested(),
        ResourceAmount::Events(EventCount::measured(1)),
        "the batch of a plan with nothing to restore is the boundary release alone"
    );
}

/// EVD-0021's measurement: every saved project under the roomier partition, the session
/// row read from each admission report, and the high-water printed for the record.
///
/// Ignored in the ordinary run because it lowers and renders twenty-eight projects; run it
/// with `--ignored --nocapture` to reproduce the record's table.
#[test]
#[ignore = "EVD-0021's survey; run with --ignored --nocapture to reproduce its table"]
fn evd_0021_session_share_high_water_over_every_saved_project() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut rows: Vec<(String, bool, Option<u64>, String)> = Vec::new();
    for directory in [
        root.join("corpus/v2-reference/projects"),
        root.join("assets/examples/projects"),
    ] {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", directory.display()));
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let Some(file_name) = file_name else { continue };
            let is_bundle = file_name.ends_with(".ptz.zip");
            if !is_bundle && !file_name.ends_with(".ptz") {
                continue;
            }
            let name = file_name
                .trim_end_matches(".zip")
                .trim_end_matches(".ptz")
                .to_owned();
            let project = if is_bundle {
                let mut library = synth_sampler::SampleLibrary::default();
                crate::bundle::load_bundle(&path, &mut library)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            } else {
                crate::project::ProjectFile::load(&path)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            };
            let rendered = smoke_render_project(
                &project.instruments,
                &project.song,
                &project.global,
                super::phase8::project_profile(),
                FrameCount::new(4_800),
                OutputPolicy::Parity,
            );
            let refused = rendered
                .diagnostics
                .iter()
                .any(|d| d.severity() == Severity::Refused);
            let first_refusal = rendered
                .diagnostics
                .iter()
                .find(|d| d.severity() == Severity::Refused)
                .map(|d| format!("{:?}", d.reason()))
                .unwrap_or_default();
            rows.push((name, !refused, session_request(&rendered), first_refusal));
        }
    }
    // The supported bus and send shapes this slice builds its tests on, which no saved
    // project exercises end to end: one instrument with one send into one delayed return,
    // and two instruments each sending into their own.
    for (name, (instruments, song, global)) in [
        (
            "synthetic: one instrument, one post-fader send, one delayed return",
            super::buses::returned_project(0.6, false),
        ),
        (
            "synthetic: two instruments, two sends, two delayed returns",
            super::buses::two_returned_project(0.6),
        ),
    ] {
        let rendered = smoke_render_project(
            &instruments,
            &song,
            &global,
            super::phase8::project_profile(),
            FrameCount::new(4_800),
            OutputPolicy::Parity,
        );
        let refused = rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused);
        rows.push((
            name.to_owned(),
            !refused,
            session_request(&rendered),
            String::new(),
        ));
    }
    println!("| Project | Reaches a plan | Session request | First refusal |");
    println!("|---|---|---|---|");
    let mut high_water: Option<(u64, String)> = None;
    for (name, eligible, request, refusal) in &rows {
        let shown = request.map_or("—".to_owned(), |r| r.to_string());
        let refusal: String = refusal.chars().take(110).collect();
        println!("| {name} | {eligible} | {shown} | {refusal} |");
        if *eligible
            && let Some(request) = request
            && high_water.as_ref().is_none_or(|(best, _)| request > best)
        {
            high_water = Some((*request, name.clone()));
        }
    }
    let (high, at) = high_water.expect("at least one project reaches a plan");
    println!("high-water: {high} ({at}) over {} projects", rows.len());
    assert!(
        rows.len() >= 30,
        "the survey must cover both directories and the fixtures"
    );
}

/// EVD-0021's consequence, held as an ordinary test: every project and fixture the survey
/// measured below the selected share is admitted under the engine's **default** profile —
/// the corpus insert case, and the two synthetic bus fixtures — and the largest request
/// the survey saw is what the selection rule was applied to.
#[test]
fn the_selected_session_share_admits_every_project_the_survey_measured_below_it() {
    let default = harness_profile();
    let share = default
        .limits()
        .events()
        .shares()
        .session_event_share()
        .get();
    assert_eq!(share, 128, "EVD-0021's selection: 32 × ⌈2 × 64 / 32⌉");
    assert_eq!(
        default.limits().events().max_events_per_quantum().get(),
        360,
        "the cap rose by the share's increase, so the six shares still sum to it"
    );
    let corpus = corpus_project("instrument-inserts");
    let (one, one_song, one_global) = super::buses::returned_project(0.6, false);
    let (two, two_song, two_global) = super::buses::two_returned_project(0.6);
    for (name, instruments, song, global, expected) in [
        (
            "instrument-inserts",
            corpus.instruments.clone(),
            corpus.song.clone(),
            corpus.global.clone(),
            29,
        ),
        ("one send, one return", one, one_song, one_global, 33),
        ("two sends, two returns", two, two_song, two_global, 64),
    ] {
        let rendered = smoke_render_project(
            &instruments,
            &song,
            &global,
            default,
            FrameCount::new(4_800),
            OutputPolicy::Parity,
        );
        assert!(rendered.is_audible(), "{name}: {:?}", rendered.diagnostics);
        assert_eq!(session_request(&rendered), Some(expected), "{name}");
    }
}
