use super::*;
use crate::compile::{RenderConfig, compile};
use crate::host::session::{PlaybackState, SessionError, SessionOutcome};
use crate::quantities::ChannelLayout;
use crate::render::AudioBlockMut;
use crate::time::SampleTime;

use super::super::tests::{graph, limits, notes_plan, profile, setup, setup_notes};

fn candidate(control: &mut SessionControl) -> Box<PreparedPlanReplacement> {
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    control
        .prepare_replacement(plan, stream, profile(), limits())
        .unwrap()
}
fn render(audio: &mut SessionAudio, frames: usize) -> Vec<f32> {
    let mut output = vec![9.0; frames];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            audio
                .render(AudioBlockMut::new(&mut output, frames, ChannelLayout::Mono).unwrap())
                .unwrap();
        }),
        0
    );
    output
}
fn install(
    audio: &mut SessionAudio,
    packet: Box<PreparedPlanReplacement>,
) -> Box<PreparedPlanReplacement> {
    let mut returned = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            returned = Some(audio.install_replacement(packet).unwrap());
        }),
        0
    );
    returned.unwrap()
}
fn collect_commands(control: &mut SessionControl, audio: &mut SessionAudio) {
    while let Some(packet) = audio.take_completed() {
        assert!(matches!(
            control.collect(packet).unwrap().outcome,
            SessionOutcome::Applied { .. }
        ));
    }
}

#[test]
fn installation_preserves_actual_clock_epoch_position_and_has_no_second_priming_quantum() {
    let (mut control, mut audio) = setup();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    audio
        .enqueue(control.prepare_stop(SampleTime::new(128)).unwrap())
        .unwrap();
    render(&mut audio, 256);
    collect_commands(&mut control, &mut audio);
    assert_eq!(
        audio.state(),
        PlaybackState::Stopped(PlanPosition::new(128))
    );
    let packet = candidate(&mut control);
    let old_epoch = audio.epoch();
    let old_generation = control.generation();
    let new_plan = packet.plan_id();
    render(&mut audio, 256); // Preparation's snapshot is deliberately stale.
    let clock = audio.clock();
    let returned = install(&mut audio, packet);
    assert_eq!(audio.clock(), clock);
    assert_eq!(audio.epoch(), old_epoch);
    assert_eq!(audio.plan_id(), new_plan);
    assert_eq!(audio.frames_until_plan_boundary().as_u64(), 0);
    collect_replacement(&mut control, returned);
    assert_eq!(control.snapshot().clock, clock);
    assert_eq!(
        control.control.anchor(),
        StreamAnchor::new(clock, PlanPosition::new(128))
    );
    assert_eq!(control.generation(), old_generation);
    assert!(!control.has_replacements());
    let play = control.prepare_play(clock).unwrap();
    assert_eq!(play.boundary().id.serial(), 3);
    audio.enqueue(play).unwrap();
    assert_eq!(render(&mut audio, 64), vec![0.25; 64]);
    collect_commands(&mut control, &mut audio);
}

#[test]
fn credits_cover_unpublished_candidates_and_survive_multiple_uncollected_installations() {
    let (mut control, mut audio) = setup();
    render(&mut audio, 64);
    let mut packets = Vec::new();
    for _ in 0..REPLACEMENT_CREDITS {
        packets.push(candidate(&mut control));
    }
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        control.prepare_replacement(plan, stream, profile(), limits()),
        Err(PlanReplacementError::Full)
    ));
    assert!(matches!(
        control.prepare_play(SampleTime::ZERO),
        Err(SessionError::ReplacementOutstanding)
    ));
    assert!(matches!(
        control.prepare_stop(SampleTime::ZERO),
        Err(SessionError::ReplacementOutstanding)
    ));
    let first = install(&mut audio, packets.remove(0));
    let second = install(&mut audio, packets.remove(0));
    let (second, error) = control.collect_replacement(second).unwrap_err();
    assert!(matches!(error, PlanReplacementError::OutgoingTable));
    collect_replacement(&mut control, first);
    collect_replacement(&mut control, second);
    for packet in packets {
        assert_eq!(
            control.cancel_replacement(packet).unwrap().outcome,
            PlanReplacementOutcome::Cancelled
        );
    }
    assert!(!control.has_replacements());
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    assert_eq!(render(&mut audio, 64), vec![0.25; 64]);
}

#[test]
fn same_plan_clones_get_distinct_tables_and_publications_and_older_values_refuse() {
    let (mut control, mut audio) = setup();
    render(&mut audio, 64);
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let a = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let b = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let older = control
        .prepare_replacement(plan.clone(), a, profile(), limits())
        .unwrap();
    let newer = control
        .prepare_replacement(plan, b, profile(), limits())
        .unwrap();
    assert_eq!(older.plan_id(), newer.plan_id());
    assert_ne!(older.id(), newer.id());
    assert_ne!(older.audio.origin.table, newer.audio.origin.table);
    let returned = install(&mut audio, newer);
    collect_replacement(&mut control, returned);
    let returned = install(&mut audio, older);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Superseded
        ))
    );
    collect_replacement(&mut control, returned);
    assert!(!control.has_replacements());
}

#[test]
fn failed_preparation_preserves_the_last_valid_candidate_and_available_credits() {
    let (mut control, mut audio) = setup();
    render(&mut audio, 64);
    let valid = candidate(&mut control);
    let old_plan = audio.plan_id();
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let unrelated = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let wrong_stream = AdmittedCompiledStream::admit(&unrelated, &[]).unwrap();
    assert!(
        control
            .prepare_replacement(plan, wrong_stream, profile(), limits())
            .is_err()
    );
    assert_eq!(audio.plan_id(), old_plan);
    assert_eq!(control.replacement_credits.iter().flatten().count(), 1);
    let returned = install(&mut audio, valid);
    collect_replacement(&mut control, returned);
    assert!(!control.has_replacements());
}

#[test]
fn mid_note_stop_refuses_without_stranding_credit_then_release_allows_replacement() {
    let (mut control, mut audio) = setup_notes();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    audio
        .enqueue(control.prepare_stop(SampleTime::new(64)).unwrap())
        .unwrap();
    render(&mut audio, 192);
    collect_commands(&mut control, &mut audio);
    assert!(!control.control.has_replacement_obligations()); // Closed compiled list.
    assert!(audio.renderer.has_replacement_obligations()); // Runtime stopped mid-note.
    let old = audio.plan_id();
    let packet = candidate(&mut control);
    let returned = install(&mut audio, packet);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::NoteObligations
        ))
    );
    assert_eq!(audio.plan_id(), old);
    collect_replacement(&mut control, returned);
    let at = audio.clock();
    audio.enqueue(control.prepare_play(at).unwrap()).unwrap();
    audio
        .enqueue(
            control
                .prepare_stop(SampleTime::new(at.as_u64() + 256))
                .unwrap(),
        )
        .unwrap();
    render(&mut audio, 320);
    collect_commands(&mut control, &mut audio);
    assert!(!audio.renderer.has_replacement_obligations());
    let packet = candidate(&mut control);
    let returned = install(&mut audio, packet);
    assert!(matches!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Installed(_))
    ));
    collect_replacement(&mut control, returned);
}

#[test]
fn a_new_notes_plan_can_resume_after_a_long_stopped_clock() {
    let (mut control, mut audio) = setup();
    render(&mut audio, 512);
    let (plan, stream) = notes_plan();
    let packet = control
        .prepare_replacement(plan, stream, profile(), limits())
        .unwrap();
    let returned = install(&mut audio, packet);
    collect_replacement(&mut control, returned);
    audio
        .enqueue(control.prepare_play(audio.clock()).unwrap())
        .unwrap();
    let output = render(&mut audio, 320);
    assert!(output[..128].iter().all(|sample| *sample == 1.0));
    assert!(output[128..192].iter().all(|sample| *sample == 0.0));
    assert!(output[192..256].iter().all(|sample| *sample == 1.0));
    assert!(output[256..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn carry_foreign_origin_and_stale_watermark_refuse_without_mutating_the_old_pair() {
    let (mut control, mut audio) = setup();
    let initial = candidate(&mut control);
    let returned = install(&mut audio, initial);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Carry
        ))
    );
    collect_replacement(&mut control, returned);
    render(&mut audio, 64);
    let (mut foreign, _) = setup();
    let packet = candidate(&mut foreign);
    let returned = install(&mut audio, packet);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Origin
        ))
    );
    let (returned, error) = control.collect_replacement(returned).unwrap_err();
    assert!(matches!(error, PlanReplacementError::Credit));
    collect_replacement(&mut foreign, returned);
    let mut packet = candidate(&mut control);
    packet.command_watermark = 1; // Private seam models a stale command snapshot.
    let returned = install(&mut audio, packet);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::CommandsChanged
        ))
    );
    collect_replacement(&mut control, returned);
    assert_eq!(audio.clock(), SampleTime::ZERO);
}

#[test]
fn a_cancelled_unpublished_command_does_not_make_the_audio_watermark_unreachable() {
    let (mut control, mut audio) = setup();
    let command = control.prepare_stop(SampleTime::ZERO).unwrap();
    assert_eq!(
        control.cancel(command).unwrap().outcome,
        SessionOutcome::Cancelled
    );
    render(&mut audio, 64);
    let packet = candidate(&mut control);
    let returned = install(&mut audio, packet);
    collect_replacement(&mut control, returned);
    let command = control.prepare_play(SampleTime::ZERO).unwrap();
    assert_eq!(command.boundary().id.serial(), 2);
    audio.enqueue(command).unwrap();
    assert_eq!(render(&mut audio, 64), vec![0.25; 64]);
}

fn collect_replacement(control: &mut SessionControl, packet: Box<PreparedPlanReplacement>) {
    let expected = packet.outcome().unwrap();
    let id = packet.id();
    let receipt = control.collect_replacement(packet).unwrap();
    assert_eq!(receipt.outcome, expected);
    assert_eq!(receipt.id, id);
}

#[test]
fn a_newer_refused_publication_still_fences_older_candidates() {
    let (mut control, mut audio) = setup();
    let older = candidate(&mut control);
    let newer = candidate(&mut control);
    let returned = install(&mut audio, newer);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Carry
        ))
    );
    collect_replacement(&mut control, returned);
    render(&mut audio, 64);
    let returned = install(&mut audio, older);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Superseded
        ))
    );
    collect_replacement(&mut control, returned);
}

#[test]
fn cancellation_after_quiescence_needs_no_callback_and_never_reopens_closed_admission() {
    let (mut control, mut audio) = setup();
    let packet = candidate(&mut control);
    control.close_admission();
    audio.close_after_quiescence().unwrap();
    assert_eq!(
        control.cancel_replacement(packet).unwrap().outcome,
        PlanReplacementOutcome::Cancelled
    );
    assert!(!control.has_replacements());
    assert!(matches!(
        control.prepare_play(SampleTime::ZERO),
        Err(SessionError::Closed)
    ));
    let (mut control, mut audio) = setup();
    render(&mut audio, 64);
    let packet = candidate(&mut control);
    let returned = install(&mut audio, packet);
    control.close_admission();
    audio.close_after_quiescence().unwrap();
    collect_replacement(&mut control, returned);
    assert!(matches!(
        control.prepare_stop(audio.clock()),
        Err(SessionError::Closed)
    ));
}

#[test]
fn actual_installation_crosses_threads_and_retirement_waits_for_the_reader() {
    let (mut control, mut audio) = setup();
    let packet = candidate(&mut control);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        render(&mut audio, 64);
        let returned = install(&mut audio, packet);
        send.send(returned).unwrap();
        // Controller may reclaim outgoing state while the new owner renders.
        for _ in 0..100 {
            assert_eq!(render(&mut audio, 37), vec![0.0; 37]);
        }
        audio
    });
    let returned = receive.recv().unwrap();
    assert!(control.has_replacements());
    collect_replacement(&mut control, returned);
    let mut audio = worker.join().unwrap();
    audio
        .enqueue(control.prepare_play(audio.clock()).unwrap())
        .unwrap();
    let carry = audio.frames_until_plan_boundary().as_u64() as usize;
    if carry != 0 {
        render(&mut audio, carry);
    }
    assert_eq!(render(&mut audio, 64), vec![0.25; 64]);
}

#[test]
fn replacement_audio_and_clock_are_independent_of_callback_partitions() {
    fn trace(partition: usize) -> (Vec<f32>, SampleTime) {
        let (mut control, mut audio) = setup();
        audio
            .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
            .unwrap();
        audio
            .enqueue(control.prepare_stop(SampleTime::new(64)).unwrap())
            .unwrap();
        let mut output = Vec::new();
        let mut installed = false;
        while output.len() < 640 {
            let callback = partition.min(640 - output.len());
            let mut remaining = callback;
            while remaining > 0 {
                collect_commands(&mut control, &mut audio);
                let carry = audio.frames_until_plan_boundary().as_u64() as usize;
                if !installed && carry == 0 && audio.clock() == SampleTime::new(192) {
                    let packet = candidate(&mut control);
                    let returned = install(&mut audio, packet);
                    collect_replacement(&mut control, returned);
                    audio
                        .enqueue(control.prepare_play(audio.clock()).unwrap())
                        .unwrap();
                    installed = true;
                }
                let frames = remaining.min(if carry == 0 { 64 } else { carry });
                output.extend(render(&mut audio, frames));
                remaining -= frames;
            }
        }
        assert!(installed);
        assert_eq!(&output[..64], &[0.0; 64]);
        assert_eq!(&output[64..128], &[0.25; 64]);
        assert_eq!(&output[128..256], &[0.0; 128]);
        assert_eq!(&output[256..], &[0.25; 384]);
        (output, audio.clock())
    }
    let reference = trace(512);
    for partition in [1, 37, 64, 256] {
        assert_eq!(trace(partition), reference);
    }
}

#[test]
fn live_registry_query_tracks_last_slot_orphans_and_mass_release_without_allocation() {
    use crate::identity::{IdentityTable, LiveNotes, ProducerId, ReleaseScope};
    use crate::quantities::{HeldNoteCount, KeyIdentity};
    let (plan, _) = notes_plan();
    let slot = plan.resolve_note(crate::ir::NodeId::new(2)).unwrap();
    let mut table = IdentityTable::from_admitted_ranges(&[HeldNoteCount::measured(2)]).unwrap();
    let mut registry = LiveNotes::for_ranges(table.id(), &[HeldNoteCount::measured(2)]).unwrap();
    let first = table
        .mint_keyed(ProducerId::new(0), slot, KeyIdentity::new(60).unwrap())
        .unwrap();
    let last = table
        .mint_keyed(ProducerId::new(0), slot, KeyIdentity::new(61).unwrap())
        .unwrap();
    let mut ended = [None; 2];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(!registry.has_live());
            registry.admit(last, slot, KeyIdentity::new(61).unwrap());
            assert!(registry.has_live());
            assert!(registry.release(first).is_none());
            assert!(registry.has_live());
            assert_eq!(
                registry
                    .release_all(ReleaseScope::Everything, &mut [])
                    .get(),
                0
            );
            assert!(registry.has_live());
            assert_eq!(
                registry
                    .release_all(ReleaseScope::Everything, &mut ended)
                    .get(),
                1
            );
            assert!(!registry.has_live());
            registry.admit(first, slot, KeyIdentity::new(60).unwrap());
            assert!(registry.has_live());
            assert_eq!(registry.release(first), Some(slot));
            assert!(!registry.has_live());
        }),
        0
    );
}

#[test]
fn geometry_exhaustion_closed_audio_and_changed_position_keep_identified_ownership() {
    use crate::quantities::SampleRate;
    use crate::time::FrameCount;
    let (mut control, mut audio) = setup();
    let other_profile = HostProfile::harness(
        SampleRate::new(44_100.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let plan = compile(&graph(), &RenderConfig::new(other_profile))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        control.prepare_replacement(plan, stream, other_profile, limits()),
        Err(PlanReplacementError::Geometry)
    ));
    assert!(!control.has_replacements());
    control.publication_serial = u64::MAX;
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        control.prepare_replacement(plan, stream, profile(), limits()),
        Err(PlanReplacementError::IdentityExhausted)
    ));
    control.publication_serial = 0;
    render(&mut audio, 64);
    let mut packet = candidate(&mut control);
    packet.expected_position = PlanPosition::new(1);
    let returned = install(&mut audio, packet);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::PositionChanged
        ))
    );
    collect_replacement(&mut control, returned);
    let packet = candidate(&mut control);
    audio.close_after_quiescence().unwrap();
    let returned = install(&mut audio, packet);
    assert_eq!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Unavailable
        ))
    );
    let (returned, error) = control.cancel_replacement(returned).unwrap_err();
    assert!(matches!(error, PlanReplacementError::State));
    let (returned, error) = audio.install_replacement(returned).unwrap_err();
    assert!(matches!(error, PlanReplacementError::State));
    collect_replacement(&mut control, returned);
}

#[test]
fn unpublished_packet_loss_never_releases_its_credit_and_limits_charge_packet_storage() {
    let (mut control, _) = setup();
    let minimum = size_of::<SessionControl>()
        + control.credits.len() * size_of::<Option<crate::host::session::SessionBoundary>>()
        + REPLACEMENT_CREDITS * size_of::<PreparedPlanReplacement>();
    assert_eq!(
        control.control_bytes(),
        crate::quantities::PreparedBytes::measured(minimum as u64)
    );
    let packet = candidate(&mut control);
    drop(packet); // Model a host custody bug; admission must remain frozen.
    assert!(control.has_replacements());
    assert!(matches!(
        control.prepare_stop(SampleTime::ZERO),
        Err(SessionError::ReplacementOutstanding)
    ));
}

#[test]
fn an_uncollected_incoming_minter_cannot_be_skipped_by_a_later_candidate() {
    use crate::quantities::{KeyIdentity, NoteVelocity};
    use crate::schedule::{CompiledPayload, PlanEvent};
    // Up to three harmless installs precede the obligated pair, filling all five credits.
    for harmless_prefix in 0..=3 {
        let (mut control, mut audio) = setup();
        render(&mut audio, 64);
        let mut preceding = Vec::new();
        for _ in 0..harmless_prefix {
            preceding.push(candidate(&mut control));
        }
        let (plan, _) = notes_plan();
        let slot = plan.resolve_note(crate::ir::NodeId::new(2)).unwrap();
        let stream = AdmittedCompiledStream::admit(
            &plan,
            &[PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot,
                    key: KeyIdentity::new(60).unwrap(),
                    velocity: NoteVelocity::FULL,
                },
            )],
        )
        .unwrap();
        let first = control
            .prepare_replacement(plan, stream, profile(), limits())
            .unwrap();
        assert!(first.control.control.has_replacement_obligations());
        let second = candidate(&mut control); // Original minter is still authoritative here.
        let mut retired_prefix = Vec::new();
        for packet in preceding {
            let returned = install(&mut audio, packet);
            assert!(matches!(
                returned.outcome(),
                Some(PlanReplacementOutcome::Installed(_))
            ));
            retired_prefix.push(returned);
        }
        let first = install(&mut audio, first);
        assert!(matches!(
            first.outcome(),
            Some(PlanReplacementOutcome::Installed(_))
        ));
        let second = install(&mut audio, second);
        assert_eq!(
            second.outcome(),
            Some(PlanReplacementOutcome::Refused(
                PlanReplacementRefusal::NoteObligations
            ))
        );
        for returned in retired_prefix {
            collect_replacement(&mut control, returned);
        }
        collect_replacement(&mut control, first);
        collect_replacement(&mut control, second);
    }
}
