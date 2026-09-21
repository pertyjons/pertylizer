use super::*;
use crate::recording::notes::projection::ProjectionError;

fn take(mode: CaptureMode, quantization: CaptureQuantization) -> LoopCaptureSession {
    take_times(mode, quantization, 25, 75)
}

fn take_times(
    mode: CaptureMode,
    quantization: CaptureQuantization,
    on: u64,
    off: u64,
) -> LoopCaptureSession {
    let mut owner = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(2, 4, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let mut arm = input();
    arm.mode = mode;
    arm.quantization = quantization;
    let _ticket = owner.arm(arm, &[source]).unwrap();
    fence(&mut owner, source, 0);
    owner.start().unwrap();
    let _on = publish(&mut owner, source, on, on, [0x90, 60, 100]);
    let _off = publish(&mut owner, source, off, off, [0x80, 60, 0]);
    fence(&mut owner, source, 200);
    let _pcm = render(&mut owner, 264, 37);
    owner.finalize().unwrap();
    owner
}

#[test]
fn pass_projection_preserves_occurrence_carry_and_replace_overdub_intent() {
    for mode in [CaptureMode::Replace, CaptureMode::Overdub] {
        let mut owner = take(mode, CaptureQuantization::Off);
        let raw_count = owner.result().unwrap().records().count();
        {
            let projection = owner.project_notes().unwrap();
            assert_eq!(projection.raw().context().mode(), mode);
            assert_eq!(projection.certified_ticks().get(), 3);
            assert!(projection.bytes_reserved().get() <= 1_048_576);
            let [first, next] = projection.notes() else {
                panic!("two pass segments");
            };
            assert_eq!(first.note().occurrence(), next.note().occurrence());
            assert_ne!(first.pass(), next.pass());
            assert!(first.attack() && first.continues());
            assert_eq!(
                first.note().ending(),
                crate::recording::notes::projection::ProjectedEnding::LoopContinuation
            );
            assert_eq!(
                next.note().ending(),
                crate::recording::notes::projection::ProjectedEnding::KeyRelease
            );
            assert!(!next.attack() && !next.continues());
            assert_eq!(
                (first.note().start(), first.note().end()),
                (MusicalTick::new(1), MusicalTick::new(2))
            );
            assert_eq!(
                (next.note().start(), next.note().end()),
                (MusicalTick::new(0), MusicalTick::new(1))
            );
            assert_eq!(projection.raw().loop_carry().count(), 2);
        }
        assert_eq!(owner.result().unwrap().records().count(), raw_count);
    }
}

#[test]
fn quantization_that_breaks_loop_continuation_refuses_without_changing_raw_take() {
    let mut owner = take(
        CaptureMode::Overdub,
        CaptureQuantization::Grid(QuantizationGrid::new(MusicalTick::new(2)).unwrap()),
    );
    assert!(matches!(
        owner.project_notes(),
        Err(ProjectionError::OutsideTarget { .. })
    ));
    assert_eq!(owner.result().unwrap().records().count(), 2);
    assert_eq!(owner.result().unwrap().loop_carry().count(), 2);
}

#[test]
fn loop_projection_refuses_unsealed_take_and_unsupported_pedals() {
    let (mut owner, source) = prepared(2, 4);
    assert!(owner.project_notes().is_err());
    let _pedal = publish(&mut owner, source, 1, 1, [0xb0, 64, 127]);
    let _on = publish(&mut owner, source, 25, 25, [0x90, 60, 100]);
    let _off = publish(&mut owner, source, 75, 75, [0x80, 60, 0]);
    fence(&mut owner, source, 200);
    let _pcm = render(&mut owner, 264, 64);
    owner.finalize().unwrap();
    assert!(matches!(
        owner.project_notes(),
        Err(ProjectionError::UnsupportedControllers { .. })
    ));
    assert_eq!(owner.result().unwrap().records().count(), 3);
}

#[test]
fn boundary_release_has_no_positive_next_pass_segment_and_sub_tick_fragments_refuse() {
    let mut owner = take_times(CaptureMode::Overdub, CaptureQuantization::Off, 25, 50);
    let projection = owner.project_notes().unwrap();
    let [note] = projection.notes() else {
        panic!("one positive segment");
    };
    assert!(note.attack() && !note.continues());
    assert_eq!(
        note.note().ending(),
        crate::recording::notes::projection::ProjectedEnding::KeyRelease
    );
    // Raw carry describes the frontier before its equal-time release, while this
    // projection represents positive musical intervals only.
    assert_eq!(projection.raw().loop_carry().count(), 2);
    drop(projection);
    let mut owner = take_times(CaptureMode::Overdub, CaptureQuantization::Off, 49, 75);
    assert!(matches!(
        owner.project_notes(),
        Err(ProjectionError::InvalidLifetime { .. })
    ));
    assert_eq!(owner.result().unwrap().records().count(), 2);
}
