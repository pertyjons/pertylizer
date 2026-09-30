//! ADR-0077's experimental V2 song playback: a lowered project played through the ordered
//! session transport sounds exactly as the same lowering renders offline.
use super::*;
use crate::lowering::live::{
    SongAudio, SongPlaybackError, partition_is_qualified, prepare_song, prepare_unqualified,
};
use crate::lowering::render::{LoweredProject, OutputPolicy, lower_project};
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES};

const BLOCK: usize = 256;

/// Shared with the live module's own tests.
pub(crate) fn live_fixture_profile() -> HostProfile {
    profile()
}

/// Shared with the live module's own tests.
pub(crate) fn live_fixture_project(name: &str) -> crate::project::ProjectFile {
    corpus_project(name)
}

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Stereo,
    )
    .expect("profile")
}

fn lowered(name: &str) -> LoweredProject {
    let project = corpus_project(name);
    lower_project(
        &project.instruments,
        &project.song,
        &project.global,
        profile(),
        OutputPolicy::Parity,
    )
}

/// Render `count` whole callbacks, each measured for allocation and deallocation.
fn callbacks(audio: &mut SongAudio, count: usize) -> Vec<f32> {
    let mut output = vec![9.0; count * BLOCK * 2];
    for chunk in output.chunks_mut(BLOCK * 2) {
        let mut rendered = false;
        let measured = allocation_counter::measure(|| {
            rendered = audio.render(
                synth_engine_v2::render::AudioBlockMut::new(chunk, BLOCK, ChannelLayout::Stereo)
                    .expect("block"),
            );
        });
        assert!(rendered, "the callback rendered");
        assert_eq!(
            (measured.count_total, measured.count_current),
            (0, 0),
            "a song callback allocated or freed"
        );
    }
    output
}

fn offline(name: &str, frames: u64) -> Vec<f32> {
    let lowering = lowered(name);
    let plan = lowering.plan.expect("the project lowers");
    render_offline(
        plan,
        FrameCount::new(frames),
        PlanPosition::ZERO,
        &lowering.events,
    )
    .expect("offline render")
}

#[test]
fn a_lowered_project_plays_live_exactly_as_it_renders_offline() {
    for name in ["subtractive-voice", "tempo-map-arrangement"] {
        let (mut control, mut audio) =
            prepare_unqualified(lowered(name), profile()).expect("the project prepares");
        let idle = callbacks(&mut audio, 2);
        assert!(
            idle.iter().all(|sample| *sample == 0.0),
            "stopped is silent"
        );
        let at = control.play().expect("play is scheduled");
        let total = 400;
        let mut live = idle;
        live.extend(callbacks(&mut audio, total - 2));
        // Output follows its boundary by one quantum (ADR-0001's constant latency).
        let start = (usize::try_from(at.as_u64()).expect("frames") + QUANTUM_FRAMES as usize) * 2;
        let reference = offline(name, (live.len() / 2 - start / 2) as u64);
        assert!(
            live[..start].iter().all(|sample| *sample == 0.0),
            "{name}: nothing sounds before the play boundary plus the latency"
        );
        assert!(
            live[start..] == reference[..],
            "{name}: live playback equals the offline render from the play boundary"
        );
        assert!(
            reference.iter().any(|sample| *sample != 0.0),
            "{name} sounds"
        );
        assert_eq!(control.collect(), 1, "{name}: play applied");
    }
}

/// Pause keeps the song position and resume starts there (ADR-0077 decision 2). A resume
/// is a new activation with ADR-0050's catch-up, not a continuation of the voices' DSP
/// state, so the audio after it is checked for sounding, not for bit identity.
#[test]
fn pause_and_resume_keep_the_song_position() {
    let name = "subtractive-voice";
    let (mut control, mut audio) =
        prepare_unqualified(lowered(name), profile()).expect("the project prepares");
    let mut live = callbacks(&mut audio, 1);
    let play = control.play().expect("play");
    live.extend(callbacks(&mut audio, 40));
    let pause = control.pause().expect("pause");
    live.extend(callbacks(&mut audio, 10));
    // `pause` already collected the play receipt; this collects the pause itself.
    assert_eq!(control.collect(), 1, "pause applied");
    let paused_at = PlanPosition::new(pause.as_u64() - play.as_u64());
    assert_eq!(control.applied_position(), Some(paused_at));
    let resume = control.play().expect("resume");
    live.extend(callbacks(&mut audio, 40));
    assert_eq!(control.collect(), 1, "resume applied");
    assert_eq!(
        control.applied_position(),
        Some(paused_at),
        "resume starts at the paused position"
    );
    // Output follows its boundary by one quantum (ADR-0001's constant latency).
    let frame = |time: synth_engine_v2::time::SampleTime| {
        (usize::try_from(time.as_u64()).expect("frames") + QUANTUM_FRAMES as usize) * 2
    };
    let first = &live[frame(play)..frame(pause)];
    let reference = offline(name, (first.len() / 2) as u64);
    assert!(
        first == &reference[..],
        "the song plays exactly until the pause"
    );
    assert!(
        live[frame(pause)..frame(resume)]
            .iter()
            .all(|sample| *sample == 0.0),
        "paused is silent"
    );
    assert!(
        live[frame(resume)..].iter().any(|sample| *sample != 0.0),
        "the resumed song sounds"
    );
}

/// EVD-0025 qualified the song-playback partition, so the gate opens; a refused project
/// still cannot play.
#[test]
fn the_qualified_gate_opens_and_a_refused_project_still_cannot_play() {
    assert!(partition_is_qualified(&profile()));
    assert!(prepare_song(lowered("subtractive-voice"), profile()).is_ok());
    // Any other session share is outside what EVD-0025 qualified.
    let base = profile();
    let limits = base.limits();
    let events = limits.events();
    let shares = events.shares();
    let shares = synth_engine_v2::profile::ProducerShares::new(
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        synth_engine_v2::quantities::EventCount::measured(127),
        shares.internal_event_share(),
        shares.release_event_share(),
        shares.release_hold_capacity(),
    )
    .expect("shares");
    let events = synth_engine_v2::profile::EventLimits::new(
        events.max_events_per_quantum(),
        events.max_note_expansion_per_tick(),
        events.max_scheduled_events_in_flight(),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("event limits");
    let limits = synth_engine_v2::profile::RenderLimits::new(
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
    let other = HostProfile::new(base.capabilities(), limits).expect("profile");
    assert!(!partition_is_qualified(&other));
    let project = corpus_project("subtractive-voice");
    let lowered_other = lower_project(
        &project.instruments,
        &project.song,
        &project.global,
        other,
        OutputPolicy::Parity,
    );
    assert!(matches!(
        prepare_song(lowered_other, other),
        Err(SongPlaybackError::CapacityUnqualified)
    ));
    assert!(matches!(
        prepare_song(lowered("keyboard-panner-stereo"), profile()),
        Err(SongPlaybackError::Refused(_))
    ));
    assert!(matches!(
        prepare_unqualified(lowered("keyboard-panner-stereo"), profile()),
        Err(SongPlaybackError::Refused(_))
    ));
}

/// The song stops by itself at the arrangement's end, and the state reports it.
#[test]
fn playback_stops_at_the_arrangement_end() {
    let name = "subtractive-voice";
    let lowering = lowered(name);
    let end = lowering.lowered_frames.as_u64();
    let (mut control, mut audio) =
        prepare_unqualified(lowering, profile()).expect("the project prepares");
    callbacks(&mut audio, 1);
    let _start = control.play().expect("play");
    let limit = usize::try_from(end).expect("frames") / BLOCK + 40;
    let mut rendered = 0;
    while rendered < limit && (control.is_playing() || rendered < 4) {
        callbacks(&mut audio, 1);
        control.poll().expect("poll");
        rendered += 1;
    }
    assert!(!control.is_playing(), "playback ended by itself");
    let stopped = control.applied_position().expect("the end stop applied");
    assert!(
        stopped.as_u64() >= end && stopped.as_u64() < end + u64::from(QUANTUM_FRAMES),
        "stopped at the arrangement end: {} vs {end}",
        stopped.as_u64()
    );
    let tail = callbacks(&mut audio, 4);
    assert!(
        tail.iter().all(|sample| *sample == 0.0),
        "silent after the end"
    );
}
