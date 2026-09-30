//! ADR-0077's application switch: V2 song playback around the V1 processor.
use super::*;
use crate::lowering::app::{MAX_CALLBACK_FRAMES, SongSwitch, wrap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    let mut output = vec![9.0; frames * 2];
    let mut context = context();
    context.frames = frames;
    let measured = allocation_counter::measure(|| processor.process(&mut output, &context));
    assert_eq!(
        (measured.count_total, measured.count_current),
        (0, 0),
        "the switch allocated or freed on the callback"
    );
    output
}

fn switch() -> (SongSwitch, impl AudioProcessor, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut control, processor) = wrap(FakeV1 {
        calls: Arc::clone(&calls),
    });
    control.set_stream(&stream(ChannelCount::Stereo));
    (control, processor, calls)
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
    assert!(!built, "no project is built while playing");
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("applies when playback stops or pauses"))
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
            .any(|n| n.contains("restarts from the song start"))
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
    control.play(|| project.clone());
    let before = calls.load(Ordering::Relaxed);
    assert!(
        callback_of(&mut processor, MAX_CALLBACK_FRAMES + 1)
            .iter()
            .all(|s| *s == 0.0),
        "the oversized callback is silent"
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
    let (mut control, mut processor, _) = switch();
    assert!(control.enable(&project));
    // No callback drains the ring; stopped edits each install a session until it is full.
    while control.can_switch() {
        control.project_changed(|| project.clone());
    }
    control.project_changed(|| project.clone());
    let _notices = control.take_notices();
    control.play(|| project.clone());
    assert!(!control.is_playing(), "the old session is not started");
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("busy applying the edit"))
    );
    // Once the callback drains the ring, the edit applies and play starts the new session.
    callback(&mut processor);
    control.poll(|| project.clone());
    assert!(
        control
            .take_notices()
            .iter()
            .any(|n| n.contains("the edit is applied"))
    );
    control.play(|| project.clone());
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
