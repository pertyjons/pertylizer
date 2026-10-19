//! ADR-0077's application switch: V2 song playback around the V1 processor.
use super::*;
use crate::lowering::app::{
    LoweredProjectDigest, MAX_CALLBACK_FRAMES, SongSwitch, SwitchScope, wrap,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use synth_core::audio::{
    AudioCallbackContext, AudioProcessor, BufferSize, ChannelCount, DeviceSampleRate, StreamInfo,
};

const FRAMES: usize = 256;
const V1_LEVEL: f32 = 0.5;

/// Stands in for the V1 engine: fills its output with a constant and counts callbacks.
struct FakeV1 {
    calls: Arc<AtomicUsize>,
}

impl AudioProcessor for FakeV1 {
    fn process(&mut self, output: &mut [f32], _context: &AudioCallbackContext) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        output.fill(V1_LEVEL);
    }
}

fn stream(channels: ChannelCount) -> StreamInfo {
    StreamInfo {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        buffer_size: BufferSize::MEDIUM,
        channels,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    }
}

fn context() -> AudioCallbackContext {
    AudioCallbackContext {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        frames: FRAMES,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    }
}

/// One callback, measured: the switch neither allocates nor frees on the audio thread.
fn callback(processor: &mut impl AudioProcessor) -> Vec<f32> {
    callback_of(processor, FRAMES)
}

/// One measured callback of `frames` frames.
fn callback_of(processor: &mut impl AudioProcessor, frames: usize) -> Vec<f32> {
    callback_of_with_rate(processor, frames, DeviceSampleRate::DVD_QUALITY)
}

fn callback_of_with_rate(
    processor: &mut impl AudioProcessor,
    frames: usize,
    rate: DeviceSampleRate,
) -> Vec<f32> {
    let mut output = vec![9.0; frames * 2];
    let mut context = context();
    context.frames = frames;
    context.sample_rate = rate;
    let measured = allocation_counter::measure(|| processor.process(&mut output, &context));
    assert_eq!(
        (measured.count_total, measured.count_current),
        (0, 0),
        "the switch allocated or freed on the callback"
    );
    output
}

fn switch() -> (
    SongSwitch,
    crate::lowering::app::EngineSwitch<FakeV1>,
    Arc<AtomicUsize>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut control, processor) = wrap(FakeV1 {
        calls: Arc::clone(&calls),
    });
    control.set_stream(&stream(ChannelCount::Stereo));
    (control, processor, calls)
}

#[test]
fn rendered_install_acknowledges_only_a_completed_unmuted_callback() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert_eq!(control.rendered_install(), None);
    assert!(control.enable(&project));
    assert_eq!(
        control.rendered_install(),
        None,
        "the install is only queued"
    );
    assert_eq!(control.rendered_plan_id(), None);
    callback(&mut processor);
    let first = control.rendered_install().expect("first install rendered");
    let first_plan = control.rendered_plan_id().expect("first plan rendered");

    control.project_changed(|| project.clone());
    control.project_changed(|| project.clone());
    assert_eq!(
        control.rendered_install(),
        Some(first),
        "queued edits are not yet audible"
    );
    assert_eq!(
        control.rendered_plan_id(),
        None,
        "a queued plan cannot borrow the old acknowledgement"
    );
    callback(&mut processor);
    let latest = control.rendered_install().expect("latest install rendered");
    let latest_plan = control.rendered_plan_id().expect("latest plan rendered");
    assert_ne!(latest_plan, first_plan);
    assert_eq!(
        latest.as_u64(),
        first.as_u64() + 2,
        "intermediate installs can be skipped"
    );

    control.stop(|| project.clone());
    assert_eq!(
        control.rendered_install(),
        None,
        "stop mutes the prior install at once"
    );
    assert_eq!(control.rendered_plan_id(), None);
    callback(&mut processor);
    let restarted = control
        .rendered_install()
        .expect("fresh stopped session rendered");
    assert_eq!(restarted.as_u64(), latest.as_u64() + 1);
    let restarted_plan = control
        .rendered_plan_id()
        .expect("fresh stopped plan rendered");
    assert_ne!(restarted_plan, latest_plan);

    control.disable();
    assert_eq!(
        control.rendered_install(),
        None,
        "removal masks the historical ack"
    );
    assert_eq!(control.rendered_plan_id(), None);
    callback(&mut processor);
    assert_eq!(
        control.rendered_install(),
        None,
        "the V1 callback clears it"
    );
}

#[test]
fn rendered_plan_digest_follows_the_project_that_was_installed() {
    let first_project = corpus_project("subtractive-voice");
    let mut edited_project = first_project.clone();
    edited_project.global.master_volume = synth_core::Gain::new(0.37);
    let first_digest = LoweredProjectDigest::of(&first_project).unwrap();
    let edited_digest = LoweredProjectDigest::of(&edited_project).unwrap();
    assert_ne!(first_digest, edited_digest);

    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&first_project));
    assert_eq!(control.rendered_plan(), None);
    callback(&mut processor);
    let first = control.rendered_plan().expect("the first plan rendered");
    assert_eq!(first.project_digest(), first_digest);
    assert_eq!(Some(first.plan_id()), control.rendered_plan_id());

    control.project_changed(|| edited_project.clone());
    assert_eq!(control.rendered_plan(), None, "the edit is only queued");
    callback(&mut processor);
    let edited = control.rendered_plan().expect("the edited plan rendered");
    assert_ne!(edited.plan_id(), first.plan_id());
    assert_eq!(edited.project_digest(), edited_digest);

    control.play(|| first_project.clone());
    for _ in 0..8 {
        callback(&mut processor);
    }
    control.project_changed(|| first_project.clone());
    assert_eq!(
        control.rendered_plan().map(|plan| plan.project_digest()),
        Some(edited_digest),
        "a deferred edit must not relabel the audible plan"
    );
    control.pause();
    let mut rebuilt = false;
    for _ in 0..64 {
        callback(&mut processor);
        control.poll(|| {
            rebuilt = true;
            first_project.clone()
        });
        if rebuilt {
            break;
        }
    }
    assert!(rebuilt, "the deferred edit must eventually be installed");
    assert_eq!(
        control.rendered_plan(),
        None,
        "the new plan awaits a callback"
    );
    callback(&mut processor);
    assert_eq!(
        control
            .rendered_plan()
            .expect("the deferred plan rendered")
            .project_digest(),
        first_digest
    );
}

#[test]
fn wrong_output_shape_clears_the_rendered_install_without_faulting_it() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    callback(&mut processor);
    let installed = control.rendered_install().expect("V2 callback rendered");

    let mut output = vec![9.0; FRAMES * 2];
    let mut wrong = context();
    wrong.channels = 1;
    let measured = allocation_counter::measure(|| processor.process(&mut output, &wrong));
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    assert!(output.iter().all(|sample| *sample == 0.0));
    assert_eq!(control.rendered_install(), None);

    callback(&mut processor);
    assert_eq!(control.rendered_install(), Some(installed));
}

#[test]
fn retirement_backpressure_keeps_ack_on_the_last_rendered_install() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    callback(&mut processor);
    for _ in 0..4 {
        control.project_changed(|| project.clone());
    }
    callback(&mut processor);
    let filled = control
        .rendered_install()
        .expect("last queued session rendered");
    for _ in 0..4 {
        control.project_changed(|| project.clone());
    }
    callback(&mut processor);
    let blocked = control
        .rendered_install()
        .expect("one further session rendered");
    assert_eq!(blocked.as_u64(), filled.as_u64() + 1);
    for _ in 0..3 {
        callback(&mut processor);
        assert_eq!(control.rendered_install(), Some(blocked));
    }
    control.poll(|| project.clone());
    callback(&mut processor);
    assert!(
        control.rendered_install().expect("queued sessions resumed") > blocked,
        "retirement collection permits later installations"
    );
}

#[test]
fn v2_mode_takes_over_the_audio_with_its_own_transport_and_hands_back_to_v1() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, calls) = switch();
    assert!(
        callback(&mut processor).iter().all(|s| *s == V1_LEVEL),
        "V1 mode"
    );
    assert!(control.is_released());

    assert!(control.enable(&project), "{:?}", control.take_notices());
    assert!(control.is_active() && !control.is_released());
    assert!(!control.take_notices().is_empty(), "V2 mode is announced");
    let before = calls.load(Ordering::Relaxed);
    assert!(
        callback(&mut processor).iter().all(|s| *s == 0.0),
        "stopped V2 is silent, and V1 is not heard"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        before + 1,
        "V1 still processes"
    );

    control.play(|| project.clone());
    assert!(control.is_playing());
    let mut heard = false;
    for _ in 0..60 {
        let output = callback(&mut processor);
        control.poll(|| project.clone());
        assert!(output.iter().all(|s| *s != V1_LEVEL), "only V2 is heard");
        heard |= output.iter().any(|s| *s != 0.0);
    }
    assert!(heard, "V2 plays");

    // Commands are scheduled two maximum callbacks plus a quantum ahead (8256 frames).
    control.pause();
    let mut silent = false;
    for _ in 0..64 {
        silent = callback(&mut processor).iter().all(|s| *s == 0.0);
        control.poll(|| project.clone());
        if silent {
            break;
        }
    }
    assert!(silent, "paused");
    assert!(!control.is_playing());

    // Stop prepares a fresh session at the song start; the old one retires off the callback.
    let mut rebuilt = false;
    control.stop(|| {
        rebuilt = true;
        project.clone()
    });
    assert!(rebuilt);
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(
        control.take_notices().is_empty(),
        "{:?}",
        control.take_notices()
    );

    control.disable();
    assert!(!control.is_active());
    assert!(
        !control.is_released(),
        "released only after the callback retires V2"
    );
    assert!(
        callback(&mut processor).iter().all(|s| *s == V1_LEVEL),
        "back to V1"
    );
    control.poll(|| project.clone());
    assert!(control.is_released());
}

#[test]
fn an_edit_while_playing_applies_once_paused() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    callback(&mut processor);
    control.play(|| project.clone());
    for _ in 0..8 {
        callback(&mut processor);
    }
    let _announced = control.take_notices();
    let mut built = false;
    control.project_changed(|| {
        built = true;
        project.clone()
    });
    assert!(!built, "unacknowledged Play cannot prepare a replacement");
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("edit remains pending"))
    );
    control.pause();
    let mut rebuilt = false;
    for _ in 0..64 {
        callback(&mut processor);
        control.poll(|| {
            rebuilt = true;
            project.clone()
        });
        if rebuilt {
            break;
        }
    }
    assert!(
        rebuilt,
        "the deferred edit is lowered once the pause applies"
    );
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("compatible edit queued"))
    );
}

#[test]
fn v2_mode_refuses_an_unlowerable_project_a_mono_device_and_an_unknown_stream() {
    let (mut control, mut processor, _) = switch();
    assert!(!control.enable(&corpus_project("keyboard-panner-stereo")));
    assert!(!control.is_active());
    assert!(control.take_notices().iter().any(|n| n.contains("refused")));
    assert!(callback(&mut processor).iter().all(|s| *s == V1_LEVEL));

    let (mut mono, _) = wrap(FakeV1 {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    mono.set_stream(&stream(ChannelCount::Mono));
    assert!(!mono.enable(&corpus_project("subtractive-voice")));
    assert!(
        mono.take_notices()
            .iter()
            .any(|n| n.contains("stereo only"))
    );

    let (mut unknown, _) = wrap(FakeV1 {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    assert!(!unknown.enable(&corpus_project("subtractive-voice")));
    assert!(unknown.take_notices().iter().any(|n| n.contains("unknown")));
}

#[test]
fn entering_v2_keeps_its_omissions_visible_until_it_ends() {
    let project = corpus_project("subtractive-voice");
    let (mut control, _processor, _) = switch();
    assert!(control.enable(&project));
    assert!(
        control.omissions().iter().any(|n| n.contains("omits")),
        "unrepresented behaviour stays visible while V2 mode is active"
    );
    control.disable();
    assert!(control.omissions().is_empty());
}

/// Leaving V2 mode cannot fail: even with the command ring full of queued installs, V2 is
/// silent from the next callback and V1 returns once every queued install is admitted, so
/// no session queued before the removal comes back after it.
#[test]
fn a_disable_with_a_full_command_ring_still_takes_effect() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    control.disable();
    assert!(!control.is_active());
    let mut v1 = false;
    for _ in 0..8 {
        let output = callback(&mut processor);
        assert!(
            output.iter().all(|s| *s == 0.0 || *s == V1_LEVEL),
            "no V2 audio after the removal"
        );
        v1 = output.iter().all(|s| *s == V1_LEVEL);
        control.poll(|| project.clone());
        if v1 {
            break;
        }
    }
    assert!(v1, "V1 is heard once the queued installs are admitted");
    for _ in 0..4 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert!(
        control.is_released(),
        "every session is retired and collected"
    );
    assert!(callback(&mut processor).iter().all(|s| *s == V1_LEVEL));
}

/// An oversized callback takes V2's terminal fault (IO-INV-002): it is silent, allocates
/// nothing, is never skipped or split, and the session is visibly prepared again at the song
/// start.
#[test]
fn an_oversized_callback_faults_v2_and_prepares_it_again() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, calls) = switch();
    assert!(control.enable(&project));
    let _announced = control.take_notices();
    callback(&mut processor);
    let before_fault = control.rendered_install().expect("first session rendered");
    control.play(|| project.clone());
    let before = calls.load(Ordering::Relaxed);
    assert!(
        callback_of(&mut processor, MAX_CALLBACK_FRAMES + 1)
            .iter()
            .all(|s| *s == 0.0),
        "the oversized callback is silent"
    );
    assert_eq!(
        control.rendered_install(),
        None,
        "faulted callback clears the ack"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        before,
        "V1 is never handed a callback above the ceiling in V2 mode"
    );
    control.poll(|| project.clone());
    assert!(
        control.take_notices().iter().any(|n| n.contains("faulted")),
        "the fault is visible"
    );
    assert!(
        !control.is_playing(),
        "the fresh session is stopped at the start"
    );
    callback(&mut processor);
    assert!(
        control
            .rendered_install()
            .is_some_and(|id| id > before_fault)
    );
    control.play(|| project.clone());
    let mut heard = false;
    for _ in 0..60 {
        heard |= callback(&mut processor).iter().any(|s| *s != 0.0);
        control.poll(|| project.clone());
    }
    assert!(heard, "the prepared session plays");
}

/// A deferred edit that cannot be installed yet is never replaced by the old session.
#[test]
fn play_waits_for_an_edit_it_cannot_install_yet() {
    let project = corpus_project("subtractive-voice");
    let mut edited = project.clone();
    edited.global.master_volume = synth_core::Gain::new(0.37);
    let original_digest = LoweredProjectDigest::of(&project).unwrap();
    let edited_digest = LoweredProjectDigest::of(&edited).unwrap();
    assert_ne!(original_digest, edited_digest);
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    // No callback drains the ring; stopped edits each install a session until it is full.
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    control.project_changed(|| edited.clone());
    let _notices = control.take_notices();
    control.play(|| edited.clone());
    assert!(!control.is_playing(), "the old session is not started");
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("busy applying the edit"))
    );
    // Once the callback drains the ring, the edit applies and play starts the new session.
    callback(&mut processor);
    assert_eq!(
        control.rendered_plan().map(|plan| plan.project_digest()),
        Some(original_digest),
        "a refused install cannot relabel the last rendered plan"
    );
    control.poll(|| edited.clone());
    assert_eq!(control.rendered_plan(), None, "the retry is only queued");
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("the edit is applied"))
    );
    callback(&mut processor);
    assert_eq!(
        control
            .rendered_plan()
            .expect("the retried plan rendered")
            .project_digest(),
        edited_digest
    );
    control.play(|| edited.clone());
    assert!(control.is_playing());
}

/// A project refused after an edit never plays: V2 is silent from the next callback and
/// the application leaves V2 mode.
#[test]
fn a_refused_edit_never_plays_the_old_session() {
    let project = corpus_project("subtractive-voice");
    let refused = corpus_project("keyboard-panner-stereo");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    let _notices = control.take_notices();
    control.project_changed(|| refused.clone());
    assert!(
        control.needs_disable(),
        "the application must leave V2 mode"
    );
    control.play(|| project.clone());
    assert!(!control.is_playing(), "the refused project does not play");
    assert!(
        callback(&mut processor).iter().all(|s| *s == 0.0),
        "silent at once"
    );
    control.disable();
    assert!(!control.is_active() && !control.needs_disable());
    let mut v1 = false;
    for _ in 0..8 {
        v1 = callback(&mut processor).iter().all(|s| *s == V1_LEVEL);
        control.poll(|| project.clone());
        if v1 {
            break;
        }
    }
    assert!(v1, "V1 returns");
}

/// A stop that finds the edited project refused silences a playing session from the next
/// callback, even with the switch ring full.
#[test]
fn a_refused_edit_at_stop_silences_a_playing_session() {
    let project = corpus_project("subtractive-voice");
    let refused = corpus_project("keyboard-panner-stereo");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    control.play(|| project.clone());
    let mut heard = false;
    for _ in 0..64 {
        heard = callback(&mut processor).iter().any(|s| *s != 0.0);
        if heard {
            break;
        }
    }
    assert!(heard, "V2 is audibly playing before the edit");
    control.project_changed(|| refused.clone());
    control.stop(|| refused.clone());
    assert!(control.needs_disable());
    assert!(
        callback(&mut processor).iter().all(|s| *s == 0.0),
        "the playing session is silent from the next callback"
    );
}

/// A stop that cannot queue its fresh session still silences V2: installs queued before the
/// stop are admitted afterwards, and none of them ends the silence it asked for.
#[test]
fn installs_queued_before_a_stop_stay_silent() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    control.play(|| project.clone());
    control.stop(|| project.clone());
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("busy switching")),
        "the full ring refused the stop's fresh session"
    );
    for _ in 0..64 {
        assert!(
            callback(&mut processor).iter().all(|s| *s == 0.0),
            "no queued session is heard after the stop"
        );
    }
}

/// After a refused edit, neither a stop nor a later valid edit lifts the silence; only
/// leaving V2 mode ends it.
#[test]
fn nothing_but_leaving_lifts_a_refusal() {
    let project = corpus_project("subtractive-voice");
    let refused = corpus_project("keyboard-panner-stereo");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    control.play(|| project.clone());
    let mut heard = false;
    for _ in 0..64 {
        heard = callback(&mut processor).iter().any(|s| *s != 0.0);
        if heard {
            break;
        }
    }
    assert!(heard, "V2 is audibly playing before the refusal");
    control.stop(|| refused.clone());
    assert!(control.needs_disable());
    control.stop(|| project.clone());
    control.project_changed(|| project.clone());
    control.play(|| project.clone());
    for _ in 0..64 {
        assert!(callback(&mut processor).iter().all(|s| *s == 0.0));
        control.poll(|| project.clone());
    }
    assert!(control.needs_disable(), "V2 mode must still be left");
}

#[test]
fn observational_switch_scopes_refuse_permanently_at_the_counter_ceiling() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(
        SwitchScope::issue(&counter).unwrap().value().get(),
        u64::MAX
    );
    for _ in 0..3 {
        assert!(SwitchScope::issue(&counter).is_none());
    }
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn a_playback_view_reports_actual_control_intent_and_confirmation() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    let inactive = control.playback_view();
    assert!(inactive.scope.is_some());
    assert!(!inactive.active && !inactive.releasing && !inactive.requested_playing);
    assert!(!inactive.edit_pending && !inactive.refused && inactive.rendered.is_none());
    assert!(control.enable(&project));
    let queued = control.playback_view();
    assert_eq!(queued.scope, inactive.scope);
    assert!(queued.active && queued.releasing);
    assert!(!queued.requested_playing && !queued.refused && queued.rendered.is_none());
    callback(&mut processor);
    assert!(control.playback_view().rendered.is_some());
    control.play(|| project.clone());
    assert!(control.playback_view().requested_playing);
    control.stop(|| project.clone());
    let stopped = control.playback_view();
    assert!(stopped.active);
    assert!(!stopped.requested_playing && stopped.rendered.is_none());
}

#[test]
fn processor_field_retirement_waits_for_a_preceding_destructor_and_survives_owner_drop() {
    use crate::lowering::app::ProcessorRetirement;
    use std::sync::mpsc;
    struct Dropped {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl AudioProcessor for Dropped {
        fn process(&mut self, _: &mut [f32], _: &AudioCallbackContext) {}
    }
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
    }
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let (control, processor) = wrap(Dropped {
        entered,
        release: released,
    });
    assert_eq!(control.processor_retirement(), ProcessorRetirement::Pending);
    let worker = std::thread::spawn(move || drop(processor));
    entry
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        control.processor_retirement(),
        ProcessorRetirement::Pending,
        "completion must stay pending while an earlier field's destructor is blocked"
    );
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(
        control.processor_retirement(),
        ProcessorRetirement::FieldsDropped
    );
}

#[test]
fn processor_field_retirement_reports_unwinding_instead_of_success() {
    use crate::lowering::app::ProcessorRetirement;
    struct Panicking;
    impl AudioProcessor for Panicking {
        fn process(&mut self, _: &mut [f32], _: &AudioCallbackContext) {}
    }
    impl Drop for Panicking {
        fn drop(&mut self) {
            panic!("controlled field destructor panic");
        }
    }
    let (control, processor) = wrap(Panicking);
    assert!(std::thread::spawn(move || drop(processor)).join().is_err());
    assert_eq!(control.processor_retirement(), ProcessorRetirement::Failed);
}

#[test]
fn processor_field_retirement_does_not_attribute_an_unrelated_unwind_to_a_field() {
    use crate::lowering::app::ProcessorRetirement;
    struct Dropped(Arc<AtomicUsize>);
    impl AudioProcessor for Dropped {
        fn process(&mut self, _: &mut [f32], _: &AudioCallbackContext) {}
    }
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(1, Ordering::Release);
        }
    }
    let dropped = Arc::new(AtomicUsize::new(0));
    let (control, processor) = wrap(Dropped(dropped.clone()));
    assert!(
        std::thread::spawn(move || {
            let _processor = processor;
            panic!("controlled unrelated unwind");
        })
        .join()
        .is_err()
    );
    assert_eq!(dropped.load(Ordering::Acquire), 1);
    assert_eq!(control.processor_retirement(), ProcessorRetirement::Failed);
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_uses_real_saved_lowering_and_owns_software_keys_until_release() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let project = corpus_project("subtractive-voice");
    let instrument = project.instruments[0].id;
    let (mut control, mut processor, calls) = switch();
    assert!(
        control.enable_mixed(&project, instrument),
        "{:?}",
        control.take_notices()
    );
    assert!(control.mixed_active());
    assert_eq!(control.mixed_instrument(), Some(instrument));
    assert_eq!(control.rendered_plan(), None);
    callback(&mut processor);
    assert!(control.rendered_plan().is_some());
    for source in [
        SoftwareSource::ComputerKeyboard,
        SoftwareSource::OnScreenKeyboard,
    ] {
        control.mixed_input(source, Midi1Input::from_bytes([0x90, 60, 102]).unwrap());
    }
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.release_mixed_sources();
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    assert!(control.take_notices().is_empty());
    control.disable();
    assert!(control.rendered_plan().is_none());
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released(), "{:?}", control.take_notices());
    assert!(control.take_notices().is_empty());
    assert!(calls.load(Ordering::Relaxed) > 0);
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_pause_uses_actual_stop_cursor_and_preserves_live_input() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(
        control.enable_mixed(&project, project.instruments[0].id),
        "{:?}",
        control.take_notices()
    );
    callback(&mut processor);
    control.mixed_input(
        SoftwareSource::ComputerKeyboard,
        Midi1Input::from_bytes([0x90, 64, 102]).unwrap(),
    );
    control.pause();
    for _ in 0..40 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.poll(|| project.clone());
    assert!(!control.is_playing());
    assert!(control.take_notices().is_empty());
    control.play(|| project.clone());
    for _ in 0..40 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.poll(|| project.clone());
    assert!(control.is_playing());
    control.release_mixed_sources();
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    assert!(control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_confirms_compatible_edit_and_closes_the_same_source_owner() {
    let mut project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    for _ in 0..4 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    let old = control.rendered_plan().unwrap();
    project.global.master_volume = synth_core::Gain::new(0.37);
    control.project_changed(|| project.clone());
    assert!(
        control.playback_view().edit_pending,
        "{:?}",
        control.take_notices()
    );
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    let new = control.rendered_plan().unwrap();
    assert_ne!(new.plan_id(), old.plan_id());
    assert!(!control.playback_view().edit_pending);
    assert!(control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(
        control.take_notices().is_empty(),
        "retired owner must use its stable initial installation"
    );
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_resets_changed_voice_program_and_accepts_the_old_physical_key_up() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    for paused in [false, true] {
        let mut project = corpus_project("subtractive-voice");
        let (mut control, mut processor, _) = switch();
        assert!(control.enable_mixed(&project, project.instruments[0].id));
        callback(&mut processor);
        control.mixed_input(
            SoftwareSource::ComputerKeyboard,
            Midi1Input::from_bytes([0x90, 62, 100]).unwrap(),
        );
        for _ in 0..4 {
            control.poll(|| project.clone());
            callback(&mut processor);
        }
        if paused {
            control.pause();
            for _ in 0..40 {
                control.poll(|| project.clone());
                callback(&mut processor);
            }
            control.poll(|| project.clone());
        }
        let old = control.rendered_plan().unwrap();
        let envelope = project.instruments[0]
            .patch
            .modules
            .iter_mut()
            .find(|module| module.module_type == synth_core::ModuleType::Envelope)
            .unwrap();
        envelope
            .parameters
            .insert("attack".to_owned(), crate::patch::ParamValue::Float(0.025));
        control.project_changed(|| project.clone());
        for _ in 0..44 {
            control.poll(|| project.clone());
            callback(&mut processor);
        }
        control.poll(|| project.clone());
        let new = control.rendered_plan().unwrap();
        assert_ne!(old.plan_id(), new.plan_id(), "{:?}", control.take_notices());
        assert_eq!(control.is_playing(), !paused);
        assert!(
            !control.playback_view().edit_pending,
            "{:?}",
            control.take_notices()
        );
        control.mixed_input(
            SoftwareSource::ComputerKeyboard,
            Midi1Input::from_bytes([0x80, 62, 0]).unwrap(),
        );
        for _ in 0..4 {
            control.poll(|| project.clone());
            callback(&mut processor);
        }
        assert!(control.take_notices().is_empty());
        control.disable();
        callback(&mut processor);
        control.poll(|| project.clone());
        assert!(control.is_released());
        assert!(control.take_notices().is_empty());
    }
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_retains_pause_intent_while_an_edit_owns_the_operation_credit() {
    let mut project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    project.global.master_volume = synth_core::Gain::new(0.42);
    control.project_changed(|| project.clone());
    control.pause();
    assert!(!control.is_playing());
    for _ in 0..80 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert!(!control.is_playing());
    assert!(!control.playback_view().edit_pending);
    assert!(control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_closes_before_and_during_a_whole_owner_edit() {
    for callbacks_before_disable in [0, 1, 8, 32, 33, 34, 40] {
        let mut project = corpus_project("subtractive-voice");
        let (mut control, mut processor, _) = switch();
        assert!(control.enable_mixed(&project, project.instruments[0].id));
        callback(&mut processor);
        let envelope = project.instruments[0]
            .patch
            .modules
            .iter_mut()
            .find(|module| module.module_type == synth_core::ModuleType::Envelope)
            .unwrap();
        envelope
            .parameters
            .insert("attack".to_owned(), crate::patch::ParamValue::Float(0.025));
        control.project_changed(|| project.clone());
        for _ in 0..callbacks_before_disable {
            control.poll(|| project.clone());
            callback_of(&mut processor, 255);
        }
        control.disable();
        callback_of(&mut processor, 1);
        control.poll(|| project.clone());
        assert!(control.is_released());
        let notices = control.take_notices();
        assert!(
            notices.is_empty(),
            "close after {callbacks_before_disable}: {notices:?}"
        );
    }
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_without_software_notes_matches_saved_project_reference() {
    for (rate_hz, rate_float) in [
        (44_100, 44_100.0),
        (48_000, 48_000.0),
        (96_000, 96_000.0),
        (192_000, 192_000.0),
    ] {
        let project = corpus_project("subtractive-voice");
        let (mut control, mut processor, _) = switch();
        let rate = DeviceSampleRate::new(rate_hz);
        let mut info = stream(ChannelCount::Stereo);
        info.sample_rate = rate;
        control.set_stream(&info);
        let profile = HostProfile::harness(
            SampleRate::new(rate_float).unwrap(),
            FrameCount::new(4096),
            ChannelLayout::Stereo,
        )
        .unwrap();
        let lowered = crate::lowering::render::lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            crate::lowering::render::OutputPolicy::Parity,
        );
        let reference = synth_engine_v2::offline::render_offline(
            lowered.plan.unwrap(),
            FrameCount::new(8192),
            synth_engine_v2::time::PlanPosition::ZERO,
            &lowered.events,
        )
        .unwrap();
        assert!(control.enable_mixed(&project, project.instruments[0].id));
        let mut rendered = callback_of_with_rate(&mut processor, 4096, rate);
        rendered.extend(callback_of_with_rate(&mut processor, 4096, rate));
        rendered.extend(callback_of_with_rate(&mut processor, 64, rate));
        let priming_samples = 64 * 2;
        assert!(
            rendered[..priming_samples]
                .iter()
                .all(|sample| sample.to_bits() == 0)
        );
        assert!(reference.iter().any(|sample| sample.abs() > 0.001));
        assert_eq!(reference.len(), rendered.len() - priming_samples);
        assert!(
            reference
                .iter()
                .zip(&rendered[priming_samples..])
                .all(|(left, right)| left.to_bits() == right.to_bits()),
            "after ADR-0001 priming, the complete signal must be bit-identical"
        );
    }
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_crosses_actual_threads_with_an_edit_and_a_stalled_control_reader() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let mut project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    let (drive, commands) = std::sync::mpsc::sync_channel::<usize>(1);
    let (done, rendered) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        while let Ok(frames) = commands.recv() {
            let output = callback_of(&mut processor, frames);
            done.send(output).unwrap();
        }
    });
    for index in 0..100 {
        if index == 1 {
            control.mixed_input(
                SoftwareSource::ComputerKeyboard,
                Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            );
        }
        if index == 3 {
            project.global.master_volume = synth_core::Gain::new(0.42);
            control.project_changed(|| project.clone());
        }
        if !(5..50).contains(&index) {
            control.poll(|| project.clone());
        }
        if index == 75 {
            control.release_mixed_sources();
        }
        drive.send(256).unwrap();
        let output = rendered.recv().unwrap();
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
    control.poll(|| project.clone());
    assert!(!control.playback_view().edit_pending);
    assert!(control.take_notices().is_empty());
    control.disable();
    drive.send(256).unwrap();
    rendered.recv().unwrap();
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(control.take_notices().is_empty());
    drop(drive);
    worker.join().unwrap();
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_loops_and_seeks_while_the_software_key_remains_owned() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    control.mixed_input(
        SoftwareSource::OnScreenKeyboard,
        Midi1Input::from_bytes([0x90, 67, 100]).unwrap(),
    );
    control.mixed_input(
        SoftwareSource::OnScreenKeyboard,
        Midi1Input::from_bytes([0xb0, 64, 127]).unwrap(),
    );
    control.mixed_loop(true);
    for _ in 0..70 {
        control.poll(|| project.clone());
        callback_of(&mut processor, 4096);
    }
    control.poll(|| project.clone());
    assert!(control.mixed_looping());
    assert!(control.take_notices().is_empty());
    control.mixed_seek(synth_engine_v2::time::PlanPosition::new(48_000));
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback_of(&mut processor, 4096);
    }
    control.poll(|| project.clone());
    assert!(!control.mixed_looping());
    control.release_mixed_sources();
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback_of(&mut processor, 4096);
    }
    assert!(control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_refuses_software_overload_before_source_exhaustion_and_keeps_releases() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    for key in 60..80 {
        control.mixed_input(
            SoftwareSource::ComputerKeyboard,
            Midi1Input::from_bytes([0x90, key, 100]).unwrap(),
        );
    }
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    assert!(!control.take_notices().is_empty());
    assert!(!control.needs_disable());
    control.release_mixed_sources();
    for _ in 0..8 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    let _refusal_reports = control.take_notices();
    assert!(!control.needs_disable());
    control.mixed_input(
        SoftwareSource::ComputerKeyboard,
        Midi1Input::from_bytes([0x90, 84, 100]).unwrap(),
    );
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.release_mixed_sources();
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    assert!(control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_preserves_pause_seek_and_stop_play_dimensions_behind_an_edit() {
    use synth_engine_v2::time::PlanPosition;
    for pause_seek in [true, false] {
        let mut project = corpus_project("subtractive-voice");
        let (mut control, mut processor, _) = switch();
        assert!(control.enable_mixed(&project, project.instruments[0].id));
        callback(&mut processor);
        project.global.master_volume = synth_core::Gain::new(0.38);
        control.project_changed(|| project.clone());
        if pause_seek {
            control.pause();
            control.mixed_seek(PlanPosition::new(1234));
        } else {
            control.stop(|| project.clone());
            control.play(|| project.clone());
        }
        for _ in 0..125 {
            callback(&mut processor);
            control.poll(|| project.clone());
        }
        if pause_seek {
            assert!(!control.is_playing());
            assert_eq!(
                control.mixed_stopped_position_for_test(),
                Some(PlanPosition::new(1234))
            );
        } else {
            assert!(control.is_playing());
            assert_eq!(
                control.mixed_anchor_position_for_test(),
                Some(PlanPosition::ZERO)
            );
        }
        assert!(control.take_notices().is_empty());
        control.disable();
        callback(&mut processor);
        control.poll(|| project.clone());
        assert!(control.is_released());
        assert!(control.take_notices().is_empty());
    }
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_paused_seek_survives_changed_program_reset_and_later_play() {
    use synth_engine_v2::time::PlanPosition;
    let mut project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    control.pause();
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    control.mixed_seek(PlanPosition::new(1234));
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert_eq!(
        control.mixed_stopped_position_for_test(),
        Some(PlanPosition::new(1234))
    );
    let previous_plan = control.rendered_plan_id().unwrap();
    let envelope = project.instruments[0]
        .patch
        .modules
        .iter_mut()
        .find(|module| module.module_type == synth_core::ModuleType::Envelope)
        .unwrap();
    envelope
        .parameters
        .insert("attack".to_owned(), crate::patch::ParamValue::Float(0.025));
    control.project_changed(|| project.clone());
    assert!(
        control.mixed_reset_pending_for_test(),
        "changed program must prepare a whole-owner reset"
    );
    for _ in 0..45 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert_eq!(
        control.mixed_stopped_position_for_test(),
        Some(PlanPosition::new(1234))
    );
    assert_ne!(control.rendered_plan_id().unwrap(), previous_plan);
    assert_eq!(
        control.mixed_anchor_position_for_test(),
        Some(PlanPosition::new(1234))
    );
    assert_eq!(
        processor.actual_mixed_stopped_position_for_test(),
        Some(PlanPosition::new(1234)),
        "the actual replacement audio owner must hold the requested stopped cursor"
    );
    control.play(|| project.clone());
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert!(control.is_playing());
    assert_eq!(
        control.mixed_anchor_position_for_test(),
        Some(PlanPosition::new(1234))
    );
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_oversized_callback_latches_silence_and_accounts_the_held_key() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    control.mixed_input(
        SoftwareSource::ComputerKeyboard,
        Midi1Input::from_bytes([0x90, 64, 100]).unwrap(),
    );
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    assert!(
        callback_of(&mut processor, MAX_CALLBACK_FRAMES + 1)
            .iter()
            .all(|sample| *sample == 0.0)
    );
    assert!(control.rendered_plan().is_none());
    assert!(callback(&mut processor).iter().all(|sample| *sample == 0.0));
    assert!(control.rendered_plan().is_none());
    control.poll(|| project.clone());
    assert!(control.needs_disable());
    assert!(!control.take_notices().is_empty());
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert_eq!(
        control
            .last_mixed_retirement()
            .unwrap()
            .physical_keys()
            .get(),
        1
    );
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_retries_a_late_declined_reset_after_pause() {
    use crate::lowering::app::SoftwareSource;
    use synth_engine_v2::recording::notes::Midi1Input;
    let mut project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    let old = control.rendered_plan().unwrap();
    control.mixed_input(
        SoftwareSource::ComputerKeyboard,
        Midi1Input::from_bytes([0x90, 64, 100]).unwrap(),
    );
    control.poll(|| project.clone()); // Command unserved; reset readiness must wait.
    let envelope = project.instruments[0]
        .patch
        .modules
        .iter_mut()
        .find(|module| module.module_type == synth_core::ModuleType::Envelope)
        .unwrap();
    envelope
        .parameters
        .insert("attack".to_owned(), crate::patch::ParamValue::Float(0.025));
    control.project_changed(|| project.clone());
    for _ in 0..100 {
        callback(&mut processor);
    } // GUI stalls beyond the sealed boundary.
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.poll(|| project.clone());
    assert_eq!(control.rendered_plan().unwrap().plan_id(), old.plan_id());
    assert!(control.playback_view().edit_pending);
    assert!(!control.needs_disable());
    let notices = control.take_notices();
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("reset refused")),
        "{notices:?}"
    );
    control.pause();
    for _ in 0..100 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.poll(|| project.clone());
    assert_ne!(control.rendered_plan().unwrap().plan_id(), old.plan_id());
    assert!(!control.playback_view().edit_pending);
    assert!(!control.is_playing());
    assert!(control.take_notices().is_empty());
    control.release_mixed_sources();
    for _ in 0..4 {
        control.poll(|| project.clone());
        callback(&mut processor);
    }
    control.disable();
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(control.is_released());
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_loop_entry_starts_paused_score_and_loop_exit_preserves_pause() {
    use synth_engine_v2::time::PlanPosition;
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    control.pause();
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    control.mixed_loop(true);
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert!(control.is_playing() && control.mixed_looping());
    assert_eq!(
        control.mixed_anchor_position_for_test(),
        Some(PlanPosition::ZERO)
    );
    control.pause();
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    control.mixed_loop(false);
    for _ in 0..40 {
        callback(&mut processor);
        control.poll(|| project.clone());
    }
    assert!(!control.is_playing() && !control.mixed_looping());
    assert_eq!(
        control.mixed_stopped_position_for_test(),
        Some(PlanPosition::ZERO)
    );
    assert!(control.take_notices().is_empty());
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_old_receipt_cannot_erase_a_new_identical_seek_or_loop_exit() {
    use synth_engine_v2::time::PlanPosition;
    for loop_entry in [false, true] {
        let project = corpus_project("subtractive-voice");
        let (mut control, mut processor, _) = switch();
        assert!(control.enable_mixed(&project, project.instruments[0].id));
        callback(&mut processor);
        if loop_entry {
            control.pause();
            for _ in 0..40 {
                callback(&mut processor);
                control.poll(|| project.clone());
            }
            control.mixed_loop(true);
        } else {
            control.mixed_seek(PlanPosition::ZERO);
        }
        // The old operation adopts on audio, but its owning receipt stays uncollected.
        for _ in 0..40 {
            callback(&mut processor);
        }
        control.mixed_seek(PlanPosition::ZERO);
        control.poll(|| project.clone());
        let older_boundary = control.mixed_anchor_time_for_test().unwrap();
        for _ in 0..40 {
            callback(&mut processor);
            control.poll(|| project.clone());
        }
        assert!(
            control.mixed_anchor_time_for_test().unwrap() > older_boundary,
            "a new identical seek must receive its own actual adoption"
        );
        assert_eq!(
            control.mixed_anchor_position_for_test(),
            Some(PlanPosition::ZERO)
        );
        assert!(!control.mixed_looping());
        assert!(control.take_notices().is_empty());
    }
}

#[cfg(feature = "v2-mixed-app")]
#[test]
fn mixed_application_wrong_buffer_length_latches_silence_before_render_is_called() {
    let project = corpus_project("subtractive-voice");
    let (mut control, mut processor, _) = switch();
    assert!(control.enable_mixed(&project, project.instruments[0].id));
    callback(&mut processor);
    let mut malformed = [9.0; FRAMES * 2 - 1];
    let measured = allocation_counter::measure(|| processor.process(&mut malformed, &context()));
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    assert!(malformed.iter().all(|sample| *sample == 0.0));
    assert!(callback(&mut processor).iter().all(|sample| *sample == 0.0));
    assert!(control.rendered_plan().is_none());
    control.poll(|| project.clone());
    assert!(control.needs_disable());
}
