//! EVD-0025: the producer partition compiled song playback admits.
//!
//! The method, falsifiers and selection rule are fixed in
//! `plans/v2/evidence/phase-09/EVD-0025-song-playback-partition.md`. The fast controls run
//! in every test pass; the full matrix is `evd_0025_song_partition_matrix`, run with
//! `--ignored --nocapture` in release mode, which prints one CSV row per run.
use super::*;
use crate::lowering::render::{OutputPolicy, lower_project};
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::profile::{EventLimits, ProducerShares, RenderLimits};
use synth_engine_v2::publish::ProducerClass;
use synth_engine_v2::quantities::{ChannelLayout, EventCount, SampleRate};
use synth_engine_v2::report::{ResourceAmount, ResourceField};
use synth_engine_v2::time::FrameCount;

const Q: u64 = QUANTUM_FRAMES as u64;
const SELECTED_COMPILED: u32 = 96;
const SELECTED_SESSION: u32 = 128;
const SELECTED_TOTAL: u32 = 360;
const IRREGULAR: [usize; 6] = [37, 64, 1, 250, 63, 129];
const PROFILES: [(f32, u64); 9] = [
    (44_100.0, 64),
    (44_100.0, 256),
    (44_100.0, 8192),
    (48_000.0, 64),
    (48_000.0, 256),
    (48_000.0, 8192),
    (96_000.0, 64),
    (96_000.0, 256),
    (96_000.0, 8192),
];

fn profile(rate: f32, block: u64) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(rate).expect("rate"),
        FrameCount::new(block),
        ChannelLayout::Stereo,
    )
    .expect("profile")
}

/// Every saved project anywhere in the repository, loaded, with its path for the report.
fn saved_projects() -> Vec<(String, crate::project::ProjectFile)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        // Loud rather than skipped: a directory the scan cannot read could hide a project.
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", directory.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|e| panic!("an entry of {} failed: {e}", directory.display()));
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            // The entry's own type, loud on failure: `Path::is_dir` answers false when the
            // metadata cannot be read, which would hide an unreadable directory.
            let kind = entry
                .file_type()
                .unwrap_or_else(|e| panic!("the type of {} failed: {e}", path.display()));
            assert!(
                !(kind.is_symlink() && path.is_dir()),
                "{} is a symlinked directory the scan would not enter",
                path.display()
            );
            if kind.is_dir() {
                if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                    pending.push(path);
                }
                continue;
            }
            let project = if name.ends_with(".ptz.zip") {
                let mut library = synth_sampler::SampleLibrary::default();
                crate::bundle::load_bundle(&path, &mut library)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            } else if name.ends_with(".ptz") {
                crate::project::ProjectFile::load(&path)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            } else {
                continue;
            };
            let relative = path
                .strip_prefix(&root)
                .map_or_else(|_| path.display().to_string(), |p| p.display().to_string());
            found.push((relative, project));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn lower(project: &crate::project::ProjectFile, profile: HostProfile) -> LoweredProject {
    lower_project(
        &project.instruments,
        &project.song,
        &project.global,
        profile,
        OutputPolicy::Parity,
    )
}

fn requested(lowered: &LoweredProject, field: ResourceField) -> Option<u32> {
    match lowered.report.as_ref()?.row(field)?.requested() {
        ResourceAmount::Events(count) => Some(count.get()),
        _ => None,
    }
}

/// The largest number of lowered events in one quantum-aligned window before `end`: what
/// the arbiter charges per rendered quantum when activation is on a quantum boundary.
fn aligned_compiled_peak(lowered: &LoweredProject, end: u64) -> u32 {
    let mut counts = std::collections::BTreeMap::<u64, u32>::new();
    for event in lowered.events.iter().filter(|e| e.time().as_u64() < end) {
        *counts.entry(event.time().as_u64() / Q).or_default() += 1;
    }
    counts.values().copied().max().unwrap_or(0)
}

/// The largest number of lowered events in any `Q`-frame window at any phase: the bound
/// admission checks against the compiled share.
fn sliding_compiled_peak(lowered: &LoweredProject) -> u32 {
    let times: Vec<u64> = lowered.events.iter().map(|e| e.time().as_u64()).collect();
    let mut peak = 0;
    let mut first = 0;
    for (last, time) in times.iter().enumerate() {
        while times[first] + Q <= *time {
            first += 1;
        }
        peak = peak.max(u32::try_from(last - first + 1).unwrap_or(u32::MAX));
    }
    peak
}

/// F3: only a compiled note producer, no authored source, no internal request.
fn assert_compiled_only(name: &str, lowered: &LoweredProject) {
    let plan = lowered.plan.as_ref().expect("plan");
    let ranges = plan.note_producer_ranges();
    assert!(
        ranges.is_empty() || (ranges.len() == 1 && plan.compiled_note_producer().is_some()),
        "F3 {name}: a non-compiled note producer is declared"
    );
    assert!(
        plan.note_producer_holds()
            .iter()
            .all(|h| *h == EventCount::NONE),
        "F3 {name}: a release hold is declared"
    );
    assert!(
        plan.authored_sources().is_empty(),
        "F3 {name}: authored source"
    );
    assert_eq!(
        requested(lowered, ResourceField::InternalEventShare),
        Some(0),
        "F3 {name}: an internal producer is requested"
    );
}

struct Run {
    audio: Vec<f32>,
    compiled: u32,
    session: u32,
    total: u32,
    others: u32,
    stop_position: PlanPosition,
    replay_position: PlanPosition,
    replay_from: usize,
}

/// One run: play at `start`, stop at `stop`, replay at `replay`, then the end stop.
fn run(
    name: &str,
    project: &crate::project::ProjectFile,
    profile: HostProfile,
    partition: &[usize],
    times: (u64, u64, u64),
) -> Run {
    let (start, stop, replay) = times;
    let lowered = lower(project, profile);
    let (mut control, mut audio) = prepare_unqualified(lowered, profile).expect("prepares");
    let channels = 2;
    let mut output = Vec::new();
    let mut part = 0;
    let mut callback = |audio: &mut SongAudio, output: &mut Vec<f32>| {
        let frames = partition[part % partition.len()];
        part += 1;
        let mut block = vec![0.0_f32; frames * channels];
        let mut rendered = false;
        let measured = allocation_counter::measure(|| {
            rendered = audio.render(
                AudioBlockMut::new(&mut block, frames, ChannelLayout::Stereo).expect("block"),
            );
        });
        assert!(rendered, "F6 {name}: a callback failed");
        assert_eq!(
            (measured.count_total, measured.count_current),
            (0, 0),
            "F6 {name}: a callback allocated or freed"
        );
        output.extend(block);
    };
    let submit = |control: &mut SongPlayback, play: bool, at: u64| {
        let packet = if play {
            control.control.prepare_play(SampleTime::new(at))
        } else {
            control.control.prepare_stop(SampleTime::new(at))
        }
        .expect("the fixed boundary is admissible");
        control.submit(packet).expect("queued");
        control.playing = play;
    };
    submit(&mut control, true, start);
    submit(&mut control, false, stop);
    while (output.len() / channels) as u64 <= stop + Q || control.outstanding != 0 {
        callback(&mut audio, &mut output);
        let _applied = control.collect();
    }
    assert!(
        control.take_refusals().is_empty(),
        "F6 {name}: a command was not applied"
    );
    submit(&mut control, true, replay);
    let replay_from = usize::try_from(replay + Q).expect("frames") * channels;
    let mut guard = 0;
    while control.outstanding != 0 || control.is_playing() || control.ending.is_some() {
        callback(&mut audio, &mut output);
        control.poll().expect("poll");
        guard += 1;
        assert!(guard < 1_000_000, "F6 {name}: the end stop never applied");
    }
    for _ in 0..2 {
        callback(&mut audio, &mut output);
    }
    assert!(
        control.take_refusals().is_empty(),
        "F6 {name}: a command was not applied"
    );
    assert!(!control.is_faulted(), "F6 {name}: the session faulted");
    assert_eq!(
        audio.publication_faults(),
        0,
        "F6 {name}: publication fault"
    );
    let applied = control.applied_log.clone();
    assert_eq!(
        applied.len(),
        4,
        "F6 {name}: play, stop, replay and end stop apply"
    );
    let others = [
        ProducerClass::AuthoredRuntime,
        ProducerClass::Live,
        ProducerClass::Release,
        ProducerClass::Internal,
    ]
    .iter()
    .map(|class| audio.high_water(*class).get())
    .max()
    .unwrap_or(0);
    Run {
        audio: output,
        compiled: audio.high_water(ProducerClass::Compiled).get(),
        session: audio.high_water(ProducerClass::Session).get(),
        total: audio.high_water_external_total().get(),
        others,
        stop_position: applied[1],
        replay_position: applied[2],
        replay_from,
    }
}

/// The fixed command times for one profile and song length.
fn times(profile: HostProfile, song: u64) -> (u64, u64, u64) {
    let block = profile.capabilities().maximum_block_size().as_u64();
    let start = (2 * block + Q).div_ceil(Q) * Q;
    let stop = start + (song / 2) / Q * Q;
    let replay = stop + (4 * block + Q).div_ceil(Q) * Q;
    (start, stop, replay)
}

fn measure(name: &str, project: &crate::project::ProjectFile, rate: f32, block: u64) -> String {
    let profile = profile(rate, block);
    let lowered = lower(project, profile);
    assert_compiled_only(name, &lowered);
    let song = lowered.lowered_frames.as_u64();
    let session_request = requested(&lowered, ResourceField::SessionEventShare).unwrap_or(0);
    let end = song.div_ceil(Q) * Q;
    let times = times(profile, song);
    let maximum = [usize::try_from(block).expect("block")];
    let irregular: Vec<usize> = IRREGULAR
        .iter()
        .map(|f| (*f).min(usize::try_from(block).expect("block")))
        .collect();
    let whole = run(name, project, profile, &maximum, times);
    let split = run(name, project, profile, &irregular, times);

    // F6: faithful audio before the stop, partition-invariant replay, resumed position.
    let (start, stop, _) = times;
    let from = usize::try_from(start + Q).expect("frames") * 2;
    let to = usize::try_from(stop + Q).expect("frames") * 2;
    let lowering = lower(project, profile);
    let reference = render_offline(
        lowering.plan.expect("plan"),
        FrameCount::new(((to - from) / 2) as u64),
        PlanPosition::ZERO,
        &lowering.events,
    )
    .expect("offline");
    assert!(
        whole.audio[from..to] == reference[..],
        "F6 {name}: pre-stop audio differs"
    );
    assert!(
        split.audio[from..to] == reference[..],
        "F6 {name}: pre-stop audio differs under the irregular partition"
    );
    let tail = whole.replay_from.min(split.replay_from);
    let length = whole.audio.len().min(split.audio.len());
    assert!(
        whole.audio[tail..length] == split.audio[tail..length],
        "F6 {name}: replay depends on the callback partition"
    );
    assert!(whole.audio[tail..length].iter().all(|s| s.is_finite()));
    assert!(
        whole.audio[tail..length].iter().any(|s| *s != 0.0),
        "F6 {name}: the replay is silent"
    );
    assert_eq!(
        whole.stop_position, whole.replay_position,
        "F6 {name}: replay position"
    );

    // F3, F4, F5.
    for run in [&whole, &split] {
        assert_eq!(run.others, 0, "F3 {name}: another class was charged");
        let peaks = lowering_for_peak(project, profile);
        assert_eq!(
            run.compiled,
            aligned_compiled_peak(&peaks, end),
            "F4 {name}: compiled high water differs from the aligned static peak"
        );
        assert!(
            sliding_compiled_peak(&peaks) <= SELECTED_COMPILED,
            "F5 {name}: the sliding compiled peak exceeds the share"
        );
        assert!(run.compiled <= SELECTED_COMPILED, "F5 {name}: compiled");
        assert!(run.session <= SELECTED_SESSION, "F5 {name}: session");
        assert!(run.total <= SELECTED_TOTAL, "F5 {name}: total");
    }
    format!(
        "evd_0025,{name},{rate},{block},{song},{},{},{},{},{},{session_request}",
        whole.compiled, whole.session, whole.total, split.compiled, split.session
    )
}

fn lowering_for_peak(
    project: &crate::project::ProjectFile,
    profile: HostProfile,
) -> LoweredProject {
    lower(project, profile)
}

/// The measured set: every saved project that lowers as a whole, and why the rest refuse.
fn measured_set() -> (Vec<(String, crate::project::ProjectFile)>, Vec<String>) {
    let mut measured = Vec::new();
    let mut refused = Vec::new();
    for (path, project) in saved_projects() {
        let lowers_somewhere = PROFILES
            .iter()
            .any(|(rate, block)| lower(&project, profile(*rate, *block)).plan.is_some());
        if lowers_somewhere {
            measured.push((path, project));
        } else {
            let lowered = lower(&project, profile(48_000.0, 256));
            let first = lowered
                .diagnostics
                .iter()
                .find(|d| d.severity() == super::super::Severity::Refused)
                .map_or_else(|| "no plan".to_owned(), |d| format!("{:?}", d.reason()));
            refused.push(format!("evd_0025_refused,{path},{first}"));
        }
    }
    (measured, refused)
}

#[test]
fn f1_admission_refuses_one_compiled_event_over_the_share() {
    let (_, project) = saved_projects()
        .into_iter()
        .find(|(path, _)| path.ends_with("subtractive-voice.ptz"))
        .expect("the corpus project");
    let lowered = lower(&project, profile(48_000.0, 256));
    let plan = lowered.plan.as_ref().expect("plan");
    let payload = lowered.events.first().expect("an event").payload();
    let share = plan.compiled_event_share().get();
    let at = |count: u32| -> Vec<PlanEvent> {
        (0..count)
            .map(|_| PlanEvent::new(PlanPosition::ZERO, payload))
            .collect()
    };
    assert!(AdmittedCompiledStream::admit(plan, &at(share)).is_ok());
    assert!(matches!(
        AdmittedCompiledStream::admit(plan, &at(share + 1)),
        Err(synth_engine_v2::schedule::CompiledStreamError::Window(
            synth_engine_v2::admit::AdmissionError::WindowOverShare { .. }
        ))
    ));
}

#[test]
fn f2_compilation_refuses_a_session_share_below_the_activation_cost() {
    let (_, project) = saved_projects()
        .into_iter()
        .find(|(path, _)| path.ends_with("subtractive-voice.ptz"))
        .expect("the corpus project");
    let base = profile(48_000.0, 256);
    let cost = requested(&lower(&project, base), ResourceField::SessionEventShare)
        .expect("session request");
    let limits = base.limits();
    let events = limits.events();
    let shares = events.shares();
    let shares = ProducerShares::new(
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        EventCount::measured(cost - 1),
        shares.internal_event_share(),
        shares.release_event_share(),
        shares.release_hold_capacity(),
    )
    .expect("shares");
    let events = EventLimits::new(
        events.max_events_per_quantum(),
        events.max_note_expansion_per_tick(),
        events.max_scheduled_events_in_flight(),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("event limits");
    let limits = RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        events,
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("limits");
    let reduced = HostProfile::new(base.capabilities(), limits).expect("profile");
    let refused = lower(&project, reduced);
    assert!(refused.plan.is_none(), "F2: the plan was admitted");
    let row = refused
        .report
        .as_ref()
        .and_then(|r| r.row(ResourceField::SessionEventShare))
        .expect("the session row");
    assert!(
        row.fit() == synth_engine_v2::report::Fit::Exceeds,
        "F2: compilation did not refuse on SessionEventShare"
    );
}

#[test]
fn stopped_control_charges_nothing() {
    let (_, project) = saved_projects()
        .into_iter()
        .find(|(path, _)| path.ends_with("subtractive-voice.ptz"))
        .expect("the corpus project");
    let profile = profile(48_000.0, 256);
    let (_control, mut audio) =
        prepare_unqualified(lower(&project, profile), profile).expect("prepares");
    let mut block = vec![0.0; 512];
    for _ in 0..64 {
        assert!(
            audio.render(AudioBlockMut::new(&mut block, 256, ChannelLayout::Stereo).expect("b"))
        );
    }
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Session,
        ProducerClass::AuthoredRuntime,
        ProducerClass::Live,
        ProducerClass::Release,
        ProducerClass::Internal,
    ] {
        assert_eq!(audio.high_water(class), EventCount::NONE, "{class:?}");
    }
    assert_eq!(audio.high_water_external_total(), EventCount::NONE);
}

#[test]
fn one_measured_run_satisfies_the_falsifiers() {
    let (measured, _) = measured_set();
    let (name, project) = measured.first().expect("a project lowers");
    let _row = measure(name, project, 48_000.0, 256);
}

#[test]
#[ignore = "EVD-0025 matrix: run in release with --ignored --nocapture"]
fn evd_0025_song_partition_matrix() {
    let (measured, refused) = measured_set();
    assert!(
        !measured.is_empty(),
        "Inconclusive: no saved project lowers"
    );
    println!(
        "scope=evd_0025 measured={} refused={}",
        measured.len(),
        refused.len()
    );
    for line in &refused {
        println!("{line}");
    }
    println!(
        "evd_0025,project,rate,block,song_frames,compiled,session,total,split_compiled,split_session,session_request"
    );
    for (name, project) in &measured {
        for (rate, block) in PROFILES {
            if lower(project, profile(rate, block)).plan.is_some() {
                println!("{}", measure(name, project, rate, block));
            } else {
                println!("evd_0025_profile_refused,{name},{rate},{block}");
            }
        }
    }
}
