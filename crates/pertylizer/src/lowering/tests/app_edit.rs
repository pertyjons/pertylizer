//! Compatible app publication and teardown, using an eligible empty saved project.
use super::*;
use synth_core::audio::{BufferSize, ChannelCount, DeviceSampleRate};
use synth_engine_v2::time::PlanPosition;

struct SilentV1;
impl AudioProcessor for SilentV1 {
    fn process(&mut self, output: &mut [f32], _: &AudioCallbackContext) {
        output.fill(0.0);
    }
}
fn project() -> crate::project::ProjectFile {
    crate::project::ProjectFile::new(
        Vec::new(),
        0,
        None,
        synth_sequencer::Song::new("edit lane"),
        crate::project::GlobalProjectState::default(),
    )
}
fn callback(audio: &mut impl AudioProcessor, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    let context = AudioCallbackContext {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        frames,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    let measured = allocation_counter::measure(|| audio.process(&mut output, &context));
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    output
}
fn paused() -> (SongSwitch, EngineSwitch<SilentV1>) {
    paused_with(&project())
}
fn paused_with(project: &crate::project::ProjectFile) -> (SongSwitch, EngineSwitch<SilentV1>) {
    paused_with_source(project, None)
}
fn paused_with_source(
    project: &crate::project::ProjectFile,
    source: Option<Arc<synth_engine::EngineState>>,
) -> (SongSwitch, EngineSwitch<SilentV1>) {
    let (mut control, mut audio) = wrap(SilentV1);
    control.set_stream(&StreamInfo {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        buffer_size: BufferSize::MEDIUM,
        channels: ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    });
    if let Some(source) = source {
        control.track_identity_source(source);
    }
    assert!(control.enable_from(|| project.clone()));
    callback(&mut audio, 256);
    control.play(|| project.clone());
    for _ in 0..40 {
        callback(&mut audio, 256);
    }
    control.pause();
    // Pause is submitted, but has not reached the callback yet.
    assert!(!control.playback.as_mut().unwrap().edit_ready());
    (control, audio)
}
fn finish_pause(control: &mut SongSwitch, audio: &mut EngineSwitch<SilentV1>) -> PlanPosition {
    for _ in 0..40 {
        callback(audio, 256);
    }
    assert!(control.playback.as_mut().unwrap().edit_ready());
    control
        .playback
        .as_ref()
        .unwrap()
        .applied_position()
        .unwrap()
}

#[test]
fn paused_edit_waits_then_labels_only_the_installed_project() {
    let (mut control, mut audio) = paused();
    let mut edited = project();
    edited.global.master_volume = synth_core::Gain::new(0.37);
    let digest = LoweredProjectDigest::of(&edited).unwrap();
    let old_sequence = control.sent;
    let mut captured = false;
    control.project_changed(|| {
        captured = true;
        edited.clone()
    });
    assert!(!captured, "pause submission is insufficient");
    assert!(control.rebuild_pending);
    let position = finish_pause(&mut control, &mut audio);
    assert!(position.as_u64() > 0);
    control.poll(|| edited.clone());
    assert_eq!(control.sent, old_sequence.next().unwrap());
    assert!(control.playback.as_ref().unwrap().is_editing());
    assert_eq!(control.rendered_plan(), None);
    control.play(|| edited.clone());
    assert!(
        !control.is_playing(),
        "resume waits for installation and collection"
    );
    callback(&mut audio, 17);
    callback(&mut audio, 256);
    let rendered = control.rendered_plan().expect("new publication rendered");
    assert_eq!(rendered.project_digest(), digest);
    control.poll(|| edited.clone());
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        Some(position)
    );
    assert!(!control.needs_disable());
    assert!(
        control
            .take_notices()
            .iter()
            .any(|notice| notice.contains("compatible edit queued"))
    );
    control.play(|| edited.clone());
    assert!(control.is_playing());
    control.stop(|| edited.clone());
    callback(&mut audio, 256);
    assert!(!control.is_playing());
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        None,
        "explicit Stop returns to start"
    );
}

#[test]
fn repeated_paused_edits_wait_for_collection_and_keep_the_position() {
    let (mut control, mut audio) = paused();
    let position = finish_pause(&mut control, &mut audio);
    let mut first = project();
    first.global.master_volume = synth_core::Gain::new(0.37);
    control.project_changed(|| first.clone());
    let first_sequence = control.sent;
    let mut second = first.clone();
    second.global.master_volume = synth_core::Gain::new(0.61);
    control.project_changed(|| panic!("must wait for the first candidate"));
    assert!(control.rebuild_pending);
    assert_eq!(control.sent, first_sequence);
    callback(&mut audio, 256);
    control.poll(|| second.clone());
    assert_eq!(control.sent, first_sequence.next().unwrap());
    callback(&mut audio, 256);
    control.poll(|| second.clone());
    assert!(!control.rebuild_pending);
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        Some(position)
    );
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&second).unwrap()
    );
}

#[test]
fn disable_and_stop_retire_pending_edits_without_another_old_callback() {
    for disable in [false, true] {
        let (mut control, mut audio) = paused();
        let _position = finish_pause(&mut control, &mut audio);
        control.project_changed(project);
        assert!(control.playback.as_ref().unwrap().is_editing());
        if disable {
            control.disable();
        } else {
            control.stop(project);
        }
        callback(&mut audio, 256);
        control.poll(project);
        if disable {
            assert!(control.is_released());
        } else {
            assert!(!control.is_playing());
        }
        // The old lane (including its unattempted candidate) retires off-thread.
        assert_eq!(control.retiring, 0);
    }
}

#[test]
fn a_refused_positive_position_edit_mutes_instead_of_falling_back() {
    let (mut control, mut audio) = paused();
    let _position = finish_pause(&mut control, &mut audio);
    let sequence = control.sent;
    let mut refused = project();
    refused.global.master_volume = synth_core::Gain::new(99.0);
    control.project_changed(|| refused.clone());
    assert!(control.needs_disable());
    assert_eq!(control.sent, sequence, "no refused candidate was queued");
    assert_eq!(control.rendered_plan(), None);
    callback(&mut audio, 256);
    assert_eq!(control.rendered_install(), None);
    assert!(
        control
            .take_notices()
            .iter()
            .any(|notice| notice.contains("paused edit preparation failed")
                && notice.contains("UnsupportedParameterValue"))
    );
}

#[test]
fn a_refused_installation_restarts_with_the_latest_edited_project() {
    let (mut control, mut audio) = paused();
    let _position = finish_pause(&mut control, &mut audio);
    callback(&mut audio, 1); // Keep carry so an injected early attempt must refuse.
    let mut edited = project();
    edited.global.master_volume = synth_core::Gain::new(0.37);
    control.project_changed(|| edited.clone());
    let refused_sequence = control.sent;
    audio
        .active
        .as_mut()
        .unwrap()
        .attempt_edit_before_boundary();
    control.poll(|| edited.clone());
    assert_eq!(
        control.sent,
        refused_sequence.next().unwrap(),
        "refusal queues a fresh session"
    );
    assert_eq!(control.playback.as_ref().unwrap().applied_position(), None);
    assert!(!control.playback.as_ref().unwrap().edit_failed());
    callback(&mut audio, 256);
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&edited).unwrap()
    );
    assert!(
        control
            .take_notices()
            .iter()
            .any(|notice| notice.contains("transfer failed"))
    );
}

#[test]
fn stop_keeps_fresh_start_intent_when_the_switch_ring_was_full() {
    let (mut control, mut audio) = paused();
    let position = finish_pause(&mut control, &mut audio);
    assert!(position.as_u64() > 0);
    // Saturate the concrete install lane while retaining an acknowledged paused
    // controller, so a retry cannot infer Stop's intent merely from its position.
    let paused_control = control.playback.take().unwrap();
    for _ in 0..SLOTS {
        let (unused, song_audio, digest, _) = control.prepare(&project()).unwrap();
        control
            .install(song_audio, unused.plan_id(), digest)
            .unwrap();
    }
    control.playback = Some(paused_control);
    assert!(!control.can_switch());
    control.stop(project);
    assert!(control.stop_pending);
    assert!(control.rebuild_pending);
    callback(&mut audio, 256);
    control.poll(project);
    assert!(!control.stop_pending);
    assert!(!control.rebuild_pending);
    assert!(!control.playback.as_ref().unwrap().is_editing());
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        None,
        "Stop retries a fresh pair, never a positive-position replacement"
    );
    callback(&mut audio, 256);
    control.play(project);
    for _ in 0..40 {
        callback(&mut audio, 256);
    }
    control.playback.as_mut().unwrap().collect();
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        Some(PlanPosition::ZERO)
    );
}

#[test]
fn saved_voice_insert_and_bus_projects_keep_position_and_resume_audibly() {
    for original in crate::lowering::tests::state_migration_projects() {
        let (mut control, mut audio) = paused_with(&original);
        let position = finish_pause(&mut control, &mut audio);
        let mut edited = original.clone();
        edited.instruments[0].patch.modules.reverse();
        let digest = LoweredProjectDigest::of(&edited).unwrap();
        control.project_changed(|| edited.clone());
        assert!(
            control.playback.as_ref().unwrap().is_editing(),
            "{:?}",
            control.take_notices()
        );
        assert!(
            callback(&mut audio, 256)
                .iter()
                .all(|sample| *sample == 0.0)
        );
        control.poll(|| edited.clone());
        assert_eq!(
            control.playback.as_ref().unwrap().applied_position(),
            Some(position),
            "{:?}",
            control.take_notices()
        );
        assert_eq!(control.rendered_plan().unwrap().project_digest(), digest);
        let notices = control.take_notices();
        assert!(
            !notices.iter().any(|notice| notice.contains("restart")),
            "{notices:?}"
        );
        control.play(|| edited.clone());
        let mut audible = false;
        // Resume cuts crossing notes under ADR-0050; render through later onsets.
        for _ in 0..240 {
            let output = callback(&mut audio, 256);
            audible |= output.iter().any(|sample| *sample != 0.0);
        }
        assert!(audible, "each saved project must sound after later onsets");
    }
}

#[test]
fn coalesced_clear_and_readd_under_identical_ids_forces_a_fresh_pair() {
    let original = crate::lowering::tests::state_migration_projects().remove(0);
    let source = synth_engine::EngineState::new();
    let (mut control, mut audio) = paused_with_source(&original, Some(Arc::clone(&source)));
    let _position = finish_pause(&mut control, &mut audio);
    let before = source.shared_graph.identity_lineage();
    source.shared_graph.clear();
    source
        .shared_graph
        .set_module(synth_engine::ModuleStateSnapshot::new(
            synth_engine::ModuleId::new(synth_core::ModuleType::Oscillator, 1),
            synth_engine::instrument::InstrumentId::FIRST,
            synth_core::ModuleType::Oscillator,
            "osc".to_owned(),
        ));
    assert_ne!(source.shared_graph.identity_lineage(), before);
    control.project_changed(|| original.clone());
    assert!(!control.playback.as_ref().unwrap().is_editing());
    assert_eq!(control.playback.as_ref().unwrap().applied_position(), None);
    callback(&mut audio, 256);
    assert!(!control.needs_disable());
}

#[test]
fn lineage_change_inside_project_snapshot_cannot_authorize_migration() {
    let original = crate::lowering::tests::state_migration_projects().remove(0);
    let source = synth_engine::EngineState::new();
    let (mut control, mut audio) = paused_with_source(&original, Some(Arc::clone(&source)));
    let _position = finish_pause(&mut control, &mut audio);
    control.project_changed(|| {
        source.shared_graph.clear();
        original.clone()
    });
    assert!(!control.playback.as_ref().unwrap().is_editing());
    assert_eq!(control.playback.as_ref().unwrap().applied_position(), None);
}

#[test]
fn unchanged_authoritative_lineage_allows_compatible_saved_project_edits() {
    let original = crate::lowering::tests::state_migration_projects().remove(0);
    let source = synth_engine::EngineState::new();
    let (mut control, mut audio) = paused_with_source(&original, Some(Arc::clone(&source)));
    let position = finish_pause(&mut control, &mut audio);
    let lineage = source.shared_graph.identity_lineage();
    let mut edited = original.clone();
    edited.instruments[0].patch.modules.reverse();
    control.project_changed(|| edited.clone());
    assert!(control.playback.as_ref().unwrap().is_editing());
    callback(&mut audio, 256);
    control.poll(|| edited.clone());
    assert_eq!(source.shared_graph.identity_lineage(), lineage);
    assert_eq!(
        control.playback.as_ref().unwrap().applied_position(),
        Some(position)
    );
    assert!(!control.needs_disable());
}

#[test]
fn authoritative_lineage_change_after_preparation_cancels_before_publication() {
    let original = crate::lowering::tests::state_migration_projects().remove(0);
    let source = synth_engine::EngineState::new();
    let (mut control, mut audio) = paused_with_source(&original, Some(Arc::clone(&source)));
    let _position = finish_pause(&mut control, &mut audio);
    let sequence = control.sent;
    let before = source.shared_graph.identity_lineage();
    let changed = Arc::clone(&source);
    control.before_edit_publication = Some(Box::new(move || changed.shared_graph.clear()));
    control.project_changed(|| original.clone());
    assert_ne!(
        source.shared_graph.identity_lineage(),
        before,
        "the post-preparation hook must run"
    );
    assert_eq!(
        control.sent, sequence,
        "no stale candidate may be published"
    );
    assert!(!control.playback.as_ref().unwrap().is_editing());
    assert!(control.rebuild_pending);
    control.poll(|| original.clone());
    assert!(
        !control.rebuild_pending,
        "the cancelled edit falls back to a fresh restart"
    );
    assert_eq!(control.playback.as_ref().unwrap().applied_position(), None);
    callback(&mut audio, 256);
    assert!(!control.needs_disable());
}

fn playing() -> (
    SongSwitch,
    EngineSwitch<SilentV1>,
    crate::project::ProjectFile,
) {
    let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
    let (mut control, mut audio) = paused_with(&project);
    let _position = finish_pause(&mut control, &mut audio);
    control.play(|| project.clone());
    for _ in 0..40 {
        callback(&mut audio, 256);
    }
    control.playback.as_mut().unwrap().collect();
    assert!(control.is_playing());
    (control, audio, project)
}

#[test]
fn running_edit_crossfades_then_retains_a_pause_requested_during_the_swap() {
    let (mut control, mut audio, mut edited) = playing();
    let old = control.sent;
    edited.global.master_volume = synth_core::Gain::new(0.37);
    control.project_changed(|| edited.clone());
    assert_eq!(control.sent, old.next().unwrap());
    assert!(control.playback.as_ref().unwrap().is_editing());
    control.pause();
    assert!(control.pause_pending);
    assert!(control.is_playing());
    for frames in [1, 17, 64, 255, 256].into_iter().cycle().take(100) {
        callback(&mut audio, frames);
        control.poll(|| edited.clone());
    }
    assert!(!control.pause_pending);
    assert!(!control.is_playing());
    assert!(!control.needs_disable());
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&edited).unwrap()
    );
    assert!(
        control
            .playback
            .as_ref()
            .unwrap()
            .applied_position()
            .unwrap()
            .as_u64()
            > 0
    );
    assert!(
        control
            .take_notices()
            .iter()
            .all(|notice| !notice.contains("could not pause"))
    );
}

#[test]
fn late_running_edit_keeps_old_metadata_and_waits_without_an_automatic_retry() {
    let (mut control, mut audio, mut edited) = playing();
    let old = control.rendered_plan().unwrap();
    edited.global.master_volume = synth_core::Gain::new(0.37);
    control.project_changed(|| edited.clone());
    let sequence = control.sent;
    let active = audio.active.as_mut().unwrap();
    // Hold the owning cell on the callback side while its old renderer advances past B.
    active.defer_edit_past_boundary();
    callback(&mut audio, 256);
    control.poll(|| edited.clone());
    assert_eq!(control.rendered_plan(), Some(old));
    assert!(control.rebuild_pending);
    assert!(control.running_declined);
    assert!(!control.playback.as_ref().unwrap().edit_failed());
    for _ in 0..8 {
        callback(&mut audio, 256);
        control.poll(|| panic!("declined attempt must wait for a new edit or pause"));
    }
    assert_eq!(control.sent, sequence);
    assert!(control.is_playing());
    control.pause();
    for _ in 0..60 {
        callback(&mut audio, 256);
        control.poll(|| edited.clone());
    }
    assert!(!control.is_playing());
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&edited).unwrap()
    );
}

#[test]
fn stop_can_retire_an_unrendered_running_swap() {
    let (mut control, mut audio, mut edited) = playing();
    let old = control.rendered_plan().unwrap();
    edited.global.master_volume = synth_core::Gain::new(0.4);
    control.project_changed(|| edited.clone());
    assert!(control.playback.as_ref().unwrap().is_editing());
    control.stop(|| edited.clone());
    callback(&mut audio, 256);
    control.poll(|| edited.clone());
    assert!(!control.is_playing());
    assert!(!control.needs_disable());
    assert_ne!(control.rendered_plan().unwrap(), old);
}

#[test]
fn running_compile_refusal_keeps_old_audio_and_metadata_without_recompiling() {
    let (mut control, mut audio, mut refused) = playing();
    let old = control.rendered_plan().unwrap();
    let sent = control.sent;
    refused.global.master_volume = synth_core::Gain::new(99.0);
    control.project_changed(|| refused.clone());
    assert!(control.running_declined);
    assert!(!control.needs_disable());
    assert_eq!(control.sent, sent);
    assert_eq!(control.rendered_plan(), Some(old));
    let mut heard = false;
    for _ in 0..12 {
        heard |= callback(&mut audio, 256).iter().any(|value| *value != 0.0);
        control.poll(|| panic!("refused edit must not be compiled again automatically"));
    }
    assert!(heard);
    assert!(control.is_playing());
    assert_eq!(control.rendered_plan(), Some(old));
}

#[test]
fn consecutive_running_edits_publish_the_latest_input_and_a_new_transport_still_works() {
    let (mut control, mut audio, mut edited) = playing();
    for level in [0.3, 0.4, 0.5] {
        edited.global.master_volume = synth_core::Gain::new(level);
        control.project_changed(|| edited.clone());
        for _ in 0..45 {
            callback(&mut audio, 256);
            control.poll(|| edited.clone());
        }
        assert!(control.is_playing());
        assert!(!control.playback.as_ref().unwrap().is_editing());
        assert_eq!(
            control.rendered_plan().unwrap().project_digest(),
            LoweredProjectDigest::of(&edited).unwrap()
        );
    }
    control.pause();
    for _ in 0..45 {
        callback(&mut audio, 256);
        control.poll(|| edited.clone());
    }
    assert!(!control.is_playing());
    assert!(!control.needs_disable());
}

#[test]
fn a_newer_edit_survives_an_older_pending_attempts_late_refusal() {
    let (mut control, mut audio, mut first) = playing();
    let old = control.rendered_plan().unwrap();
    let old_omissions = control.omissions().to_vec();
    first.global.master_volume = synth_core::Gain::new(0.3);
    control.project_changed(|| first.clone());
    let sequence = control.sent;
    control.pending_running.as_mut().unwrap().omissions = vec!["new plan omission".to_owned()];
    assert_eq!(control.rendered_plan(), Some(old));
    assert_eq!(control.omissions(), old_omissions);
    let mut second = first.clone();
    second.global.master_volume = synth_core::Gain::new(0.7);
    control.project_changed(|| panic!("one candidate already owns the lane"));
    audio.active.as_mut().unwrap().defer_edit_past_boundary();
    callback(&mut audio, 256);
    control.poll(|| second.clone());
    assert_eq!(control.sent, sequence.next().unwrap());
    assert!(!control.running_declined);
    for _ in 0..45 {
        callback(&mut audio, 256);
        control.poll(|| second.clone());
    }
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&second).unwrap()
    );
}

#[test]
fn play_cancels_a_pause_that_has_not_been_sent_during_a_running_swap() {
    let (mut control, mut audio, mut edited) = playing();
    edited.global.master_volume = synth_core::Gain::new(0.3);
    control.project_changed(|| edited.clone());
    control.pause();
    assert!(control.pause_pending);
    control.play(|| edited.clone());
    assert!(!control.pause_pending);
    for _ in 0..45 {
        callback(&mut audio, 256);
        control.poll(|| edited.clone());
    }
    assert!(control.is_playing());
}

#[test]
fn stop_and_disable_collect_both_running_owners_after_a_partial_fade() {
    for disable in [false, true] {
        let (mut control, mut audio, mut edited) = playing();
        edited.global.master_volume = synth_core::Gain::new(0.3);
        // Add a real insert so this control takes reset/fade, not compatible Trim continuation.
        let inserts = crate::lowering::tests::state_migration_projects().remove(1);
        edited.global.master_effects.push(
            inserts.instruments[0]
                .patch
                .modules
                .iter()
                .find(|module| module.module_type.is_effect())
                .unwrap()
                .clone(),
        );
        control.project_changed(|| edited.clone());
        let mut fading = false;
        for _ in 0..10_000 {
            callback(&mut audio, 1);
            if audio.active.as_ref().unwrap().edit_is_fading() {
                fading = true;
                break;
            }
        }
        assert!(fading);
        if disable {
            control.disable();
        } else {
            control.stop(|| edited.clone());
        }
        // No further callback reaches the fading owner, only its replacement/removal.
        callback(&mut audio, 256);
        control.poll(|| edited.clone());
        assert_eq!(control.retiring, 0);
        assert!(!control.is_playing());
        if disable {
            assert!(control.is_released());
        } else {
            assert!(!control.needs_disable());
        }
    }
}

#[test]
fn a_saved_project_master_level_edit_uses_voice_continuation() {
    let (mut control, mut audio, mut edited) = playing();
    let _old_notices = control.take_notices();
    for level in [0.3, 0.6] {
        edited.global.master_volume = synth_core::Gain::new(level);
        control.project_changed(|| edited.clone());
        assert!(
            control
                .take_notices()
                .iter()
                .any(|notice| notice.contains("sounding voices")),
            "must choose the actual continuation path"
        );
        let mut heard = false;
        for _ in 0..45 {
            heard |= callback(&mut audio, 256)
                .iter()
                .any(|sample| *sample != 0.0);
            control.poll(|| edited.clone());
        }
        assert!(heard);
        assert!(control.is_playing());
        assert_eq!(
            control.rendered_plan().unwrap().project_digest(),
            LoweredProjectDigest::of(&edited).unwrap()
        );
    }
}

#[test]
fn a_running_voice_continuation_rechecks_authoritative_lineage_before_publication() {
    let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
    let source = synth_engine::EngineState::new();
    let (mut control, mut audio) = paused_with_source(&project, Some(Arc::clone(&source)));
    let _position = finish_pause(&mut control, &mut audio);
    control.play(|| project.clone());
    for _ in 0..40 {
        callback(&mut audio, 256);
    }
    control.playback.as_mut().unwrap().collect();
    let old = control.sent;
    let old_rendered = control.rendered_plan();
    let changed = Arc::clone(&source);
    let lineage = source.shared_graph.identity_lineage();
    control.before_edit_publication = Some(Box::new(move || changed.shared_graph.clear()));
    let mut edited = project.clone();
    edited.global.master_volume = synth_core::Gain::new(0.3);
    control.project_changed(|| edited.clone());
    assert_ne!(
        source.shared_graph.identity_lineage(),
        lineage,
        "post-preparation guard ran"
    );
    assert_eq!(control.sent, old);
    assert!(!control.playback.as_ref().unwrap().is_editing());
    assert_eq!(control.rendered_plan(), old_rendered);
    assert!(control.rebuild_pending);
    control.poll(|| edited.clone());
    assert_eq!(control.sent, old.next().unwrap());
    let notices = control.take_notices();
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("graph identity changed"))
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("edit remains pending"))
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("reset crossfade"))
    );
    for _ in 0..45 {
        callback(&mut audio, 256);
        control.poll(|| edited.clone());
    }
    assert_eq!(
        control.rendered_plan().unwrap().project_digest(),
        LoweredProjectDigest::of(&edited).unwrap()
    );
    // The reset's private Play also retained a real adopted witness, so a later
    // unchanged-lineage level edit can continue immediately without another Play.
    edited.global.master_volume = synth_core::Gain::new(0.6);
    control.project_changed(|| edited.clone());
    assert!(
        control
            .take_notices()
            .iter()
            .any(|notice| notice.contains("sounding voices"))
    );
}

#[test]
fn native_voice_insert_and_return_projects_continue_on_a_master_trim_edit() {
    for original in crate::lowering::tests::state_migration_projects() {
        let (mut control, mut audio) = wrap(SilentV1);
        control.set_stream(&StreamInfo {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            buffer_size: BufferSize::MEDIUM,
            channels: ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        });
        assert!(control.enable_from(|| original.clone()));
        callback(&mut audio, 256);
        control.play(|| original.clone());
        for _ in 0..40 {
            callback(&mut audio, 256);
        }
        control.playback.as_mut().unwrap().collect();
        let _old_notices = control.take_notices();
        let mut edited = original.clone();
        edited.global.master_volume = synth_core::Gain::new(0.37);
        control.project_changed(|| edited.clone());
        assert!(
            control
                .take_notices()
                .iter()
                .any(|notice| notice.contains("sounding voices")),
            "native voice/insert/return project must choose continuation"
        );
        for _ in 0..45 {
            callback(&mut audio, 256);
            control.poll(|| edited.clone());
        }
        assert!(!control.needs_disable());
        assert!(control.is_playing());
        assert_eq!(
            control.rendered_plan().unwrap().project_digest(),
            LoweredProjectDigest::of(&edited).unwrap()
        );
    }
}
