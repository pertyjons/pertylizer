//! The application's owning edit lane, with an audible global-state control.
use super::super::live::*;
use crate::lowering::app::InstallSequence;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    quantities::{Amplitude, ChannelLayout, EventCount, Frequency, GainFactor, SampleRate},
    render::AudioBlockMut,
    time::FrameCount,
};

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(256),
        ChannelLayout::Stereo,
    )
    .unwrap()
}

fn lowered(extra: bool) -> LoweredProject {
    lowered_with_live_range(extra, false)
}

fn lowered_with_live_range(extra: bool, live: bool) -> LoweredProject {
    let source = NodeId::new(10);
    let gain = NodeId::new(15);
    let output = NodeId::new(20);
    let mut builder = GraphIr::builder()
        .node(
            source,
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(output, IrNodeKind::Output, ExecutionScope::Global);
    let from = if extra {
        builder = builder
            .node(
                gain,
                IrNodeKind::Gain {
                    factor: GainFactor::new(1.0).unwrap(),
                },
                ExecutionScope::Global,
            )
            .connect(
                (source, PortId::FIRST),
                (gain, PortId::FIRST),
                SignalDomain::Audio,
            );
        gain
    } else {
        source
    };
    if live {
        builder = builder.declaring(synth_engine_v2::ir::PlanDeclarations {
            note_producers: vec![synth_engine_v2::ir::NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(1),
                simultaneous_holds: EventCount::measured(1),
            }],
            ..synth_engine_v2::ir::PlanDeclarations::default()
        });
    }
    let graph = builder
        .connect(
            (from, PortId::FIRST),
            (output, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    LoweredProject {
        plan: Some(
            compile(&graph, &RenderConfig::new(profile()))
                .into_plan()
                .unwrap(),
        ),
        events: Vec::new(),
        diagnostics: Vec::new(),
        lowered_events: EventCount::NONE,
        lowered_frames: FrameCount::new(1_000_000),
        report: None,
    }
}

fn render(audio: &mut SongAudio, frames: usize) -> Vec<f32> {
    let mut output = vec![9.0; frames * 2];
    let mut success = false;
    let measured = allocation_counter::measure(|| {
        success =
            audio.render(AudioBlockMut::new(&mut output, frames, ChannelLayout::Stereo).unwrap());
    });
    assert!(success);
    assert_eq!(
        (measured.count_total, measured.count_current),
        (0, 0),
        "callback allocation or destruction"
    );
    output
}

fn paused() -> (SongPlayback, SongAudio) {
    paused_lowered(lowered(false))
}

fn paused_lowered(lowered: LoweredProject) -> (SongPlayback, SongAudio) {
    let (mut control, mut audio) = prepare_song(lowered, profile()).unwrap();
    let _at = control.play().unwrap();
    for _ in 0..12 {
        render(&mut audio, 256);
    }
    control.collect();
    let _at = control.pause().unwrap();
    for _ in 0..6 {
        render(&mut audio, 256);
    }
    control.collect();
    assert!(control.edit_ready());
    assert!(control.applied_position().unwrap().as_u64() > 0);
    (control, audio)
}

#[test]
fn compatible_edit_preserves_position_phase_and_callback_partitioning() {
    for prefix in [0, 1, 17, 63] {
        let (mut control, mut audio) = paused();
        let (mut reference, mut reference_audio) = paused();
        if prefix > 0 {
            render(&mut audio, prefix);
            render(&mut reference_audio, prefix);
        }
        let position = control.applied_position();
        let old = control.plan_id();
        let sequence = InstallSequence::NONE.next().unwrap();
        let candidate = control
            .replace_compatible(lowered(true), profile(), sequence)
            .unwrap();
        assert_ne!(candidate, old);
        assert_eq!(
            control.plan_id(),
            old,
            "control changes only upon collection"
        );
        assert!(matches!(control.play(), Err(SongPlaybackError::Command(_))));
        assert!(matches!(
            control.pause(),
            Err(SongPlaybackError::Command(_))
        ));
        assert_eq!(render(&mut audio, 256), render(&mut reference_audio, 256));
        assert_eq!(audio.install_sequence(), sequence);
        assert!(control.is_editing());
        control.collect();
        assert!(!control.is_editing());
        assert!(!control.edit_failed());
        assert_eq!(control.plan_id(), candidate);
        assert_eq!(control.applied_position(), position);
        assert_eq!(control.play().unwrap(), reference.play().unwrap());
        let mut audible = false;
        for _ in 0..10 {
            let got = render(&mut audio, 256);
            audible |= got.iter().any(|value| *value != 0.0);
            assert_eq!(
                got,
                render(&mut reference_audio, 256),
                "phase continuity, prefix {prefix}"
            );
        }
        assert!(audible, "silence cannot prove migration");
    }
}

#[test]
fn edit_waits_for_acknowledged_pause_and_blocks_a_second_candidate() {
    let (mut control, mut audio) = prepare_song(lowered(false), profile()).unwrap();
    let _at = control.play().unwrap();
    for _ in 0..10 {
        render(&mut audio, 256);
    }
    let _at = control.pause().unwrap();
    assert!(!control.edit_ready());
    let sequence = InstallSequence::NONE.next().unwrap();
    assert!(matches!(
        control.replace_compatible(lowered(true), profile(), sequence),
        Err(SongEditError::Busy)
    ));
    for _ in 0..6 {
        render(&mut audio, 256);
    }
    assert!(control.edit_ready());
    let _plan = control
        .replace_compatible(lowered(true), profile(), sequence)
        .unwrap();
    assert!(matches!(
        control.replace_compatible(lowered(false), profile(), sequence),
        Err(SongEditError::Busy)
    ));
    render(&mut audio, 256);
    assert!(control.edit_ready());
    assert!(control.take_refusals().is_empty());
}

#[test]
fn pending_edit_does_not_hide_an_oversized_callback() {
    let (mut control, mut audio) = paused();
    let _plan = control
        .replace_compatible(
            lowered(true),
            profile(),
            InstallSequence::NONE.next().unwrap(),
        )
        .unwrap();
    let mut output = vec![1.0; 257 * 2];
    assert!(!audio.render(AudioBlockMut::new(&mut output, 257, ChannelLayout::Stereo).unwrap()));
    assert!(output.iter().all(|value| *value == 0.0));
    assert!(control.is_faulted());
    assert_eq!(audio.install_sequence(), InstallSequence::NONE);
}

#[test]
fn installation_refusal_is_collected_and_requires_a_fresh_session() {
    let (mut control, mut audio) = paused();
    render(&mut audio, 1); // Leave carry to inject an impossible early-install attempt.
    let sequence = InstallSequence::NONE.next().unwrap();
    let _plan = control
        .replace_compatible(lowered(true), profile(), sequence)
        .unwrap();
    audio.pending_edit = audio.edits.try_pop();
    audio.install_edit();
    assert_eq!(audio.install_sequence(), InstallSequence::NONE);
    control.collect();
    assert!(control.edit_failed());
    assert!(!control.is_editing());
    assert!(
        !control.control.has_replacements(),
        "refused owner was collected off-thread"
    );
    assert!(matches!(control.play(), Err(SongPlaybackError::Faulted)));
    assert!(
        control
            .take_refusals()
            .iter()
            .any(|notice| notice.contains("Carry"))
    );
}

#[test]
fn an_uncancellable_command_is_retained_and_requires_repreparation() {
    let (mut control, mut audio) = paused();
    let (mut foreign, _foreign_audio) = prepare_song(lowered(false), profile()).unwrap();
    let packet = foreign.control.prepare_play(SampleTime::ZERO).unwrap();
    assert!(audio.returns.try_push(packet).is_ok());
    control.collect();
    assert_eq!(
        control.unresolved_commands.len(),
        1,
        "retain the foreign owning command"
    );
    assert!(
        control.edit_failed(),
        "a permanent credit error cannot become transient edit pressure"
    );
    assert!(matches!(control.play(), Err(SongPlaybackError::Faulted)));
    assert!(!control.edit_ready());
}

#[test]
fn publication_guard_cancels_the_complete_unpublished_edit_and_returns_its_credit() {
    let (mut control, _audio) = paused();
    let original = control.plan_id();
    let result = control.replace_compatible_guarded(
        lowered(true),
        profile(),
        InstallSequence::NONE.next().unwrap(),
        || false,
    );
    assert!(matches!(result, Err(SongEditError::Busy)));
    assert!(!control.control.has_replacements());
    assert!(!control.is_editing());
    assert_eq!(control.plan_id(), original);
    let _play = control.play().unwrap();
}

#[test]
fn a_declared_external_range_requests_restart_instead_of_retrying_forever() {
    let (mut control, _audio) = paused_lowered(lowered_with_live_range(false, true));
    let sequence = InstallSequence::NONE.next().unwrap();
    let outcome = control.replace_compatible(lowered(true), profile(), sequence);
    assert!(matches!(outcome, Err(SongEditError::Unsupported(_))));
    assert!(!control.is_editing());
    assert!(!control.control.has_replacements());
}
