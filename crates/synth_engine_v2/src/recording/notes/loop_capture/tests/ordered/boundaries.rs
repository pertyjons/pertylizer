//! Phase 9A item 8: panic and sustain at exact loop boundaries in the ordered session.
//! The fixture loop is 0..50, so every multiple of 50 is a pass boundary.
use super::*;
use crate::recording::notes::loop_capture::LoopCarryDirection;
use crate::recording::notes::{CaptureStopReason, Midi1Event};

const PARTITIONS: [&[usize]; 4] = [&[2048], &[64], &[256], &[17, 1, 99, 7]];

/// TAKE-INV-002: panic retains the take, and its synthetic closure preserves whether the
/// key and the pedal were still held. At the exact wrap the panic wins; no new pass opens.
#[test]
fn panic_at_exact_loop_wrap_retains_the_take_and_closes_held_key_and_pedal() {
    let mut reference = None;
    for partition in PARTITIONS {
        let (mut owner, sources) = setup();
        let _play = owner.offer(SampleTime::ZERO, SessionCommand::Play).unwrap();
        let _panic = owner
            .offer(SampleTime::new(1600), SessionCommand::Panic)
            .unwrap();
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        queue_input(&mut owner, sources[0], 1590, [0xb0, 64, 127]);
        queue_input(&mut owner, sources[0], 1599, [0x90, 60, 100]);
        for source in sources {
            queue_fence(&mut owner, source, 1600);
        }
        let output = audio(&mut owner, 2048, partition);
        assert!(output[1664..].iter().all(|sample| *sample == 0.0));
        owner.finalize().unwrap();
        let end = owner.observation_end().unwrap();
        assert_eq!(end.at, SampleTime::new(1600));
        assert_eq!(end.pass.as_u64(), 32);
        let _play = owner.collect().unwrap();
        let panic = owner.collect().unwrap();
        assert_eq!(
            panic.outcome,
            SessionOutcome::Applied {
                position: PlanPosition::new(50)
            }
        );
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(result.loop_passes().count(), 32);
        let closures: Vec<_> = result.closures().copied().collect();
        assert_eq!(closures.len(), 1);
        assert_eq!(closures[0].time, SampleTime::new(1600));
        assert!(closures[0].key_held);
        assert_eq!(closures[0].pedal_held, Some(true));
        assert_eq!(closures[0].reason, CaptureStopReason::Panic);
        // Connection generations are process-unique, so compare everything else.
        let trace = closures
            .iter()
            .map(|c| (c.time, c.key_held, c.pedal_held, c.reason))
            .collect::<Vec<_>>();
        match &reference {
            None => reference = Some((output, trace)),
            Some((audio, expected)) => {
                assert!(output == *audio, "audio depends on the callback partition");
                assert_eq!(&trace, expected);
            }
        }
    }
}

/// TAKE-INV-002/003: sustain never postpones key pairing, a release at a wrap belongs to
/// the new pass while closing the occurrence begun in the old one, and a pedal edge after
/// the wrap is retained as a raw observation owned by the new pass.
#[test]
fn sustain_across_a_wrap_keeps_key_pairing_and_pass_ownership() {
    let mut reference = None;
    for partition in PARTITIONS {
        let (mut owner, sources) = setup();
        // Stop must be quantum aligned; 320 is not a pass boundary of the 0..50 loop.
        queue_transport(&mut owner, 320);
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        for (at, bytes) in [
            (10, [0xb0, 64, 127]),
            (20, [0x90, 60, 100]),
            (30, [0x80, 60, 0]),
            (45, [0x90, 62, 100]),
            (50, [0x80, 62, 0]),
            (60, [0xb0, 64, 0]),
        ] {
            queue_input(&mut owner, sources[0], at, bytes);
        }
        for source in sources {
            queue_fence(&mut owner, source, 320);
        }
        let output = audio(&mut owner, 512, partition);
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(result.closures().count(), 0, "nothing is left open");
        let records: Vec<_> = result.records().copied().collect();
        let at = |time: u64| {
            records
                .iter()
                .find(|record| record.stamp().nominal() == SampleTime::new(time))
                .copied()
                .unwrap()
        };
        // Key-up at 30 closes the onset at 20 although the pedal is still down.
        assert!(at(20).occurrence().is_some());
        assert_eq!(at(30).occurrence(), at(20).occurrence());
        assert!(
            at(60).occurrence().is_none(),
            "a pedal edge is no occurrence"
        );
        assert!(matches!(
            at(60).input().event(),
            Midi1Event::Sustain { down: false }
        ));
        // The release exactly at the wrap closes the occurrence begun before it.
        let crossing = at(45).occurrence().unwrap();
        assert_eq!(at(50).occurrence(), Some(crossing));
        let carry: Vec<_> = result
            .loop_carry()
            .filter(|carry| carry.occurrence() == crossing)
            .map(|carry| (carry.at(), carry.direction()))
            .collect();
        assert_eq!(
            carry,
            [
                (SampleTime::new(50), LoopCarryDirection::Out),
                (SampleTime::new(50), LoopCarryDirection::In),
            ]
        );
        let passes: Vec<_> = result.loop_passes().copied().collect();
        let owning = |time: u64| {
            passes
                .iter()
                .position(|pass| {
                    pass.window().start() <= SampleTime::new(time)
                        && SampleTime::new(time) < pass.window().end()
                })
                .unwrap()
        };
        assert_eq!(owning(45), 0);
        assert_eq!(
            owning(50),
            1,
            "a release at the wrap belongs to the new pass"
        );
        assert_eq!(owning(60), 1);
        let initial = result.initial_sources().next().unwrap();
        let terminal = result.terminal_sources().next().unwrap();
        assert!(!initial.controls.pedals()[0]);
        assert!(!terminal.controls.pedals()[0]);
        let trace = records
            .iter()
            .map(|r| {
                (
                    r.stamp().nominal(),
                    r.input().event(),
                    r.occurrence().map(|o| o.serial()),
                )
            })
            .collect::<Vec<_>>();
        match &reference {
            None => reference = Some((output, trace)),
            Some((audio, expected)) => {
                assert!(output == *audio, "audio depends on the callback partition");
                assert_eq!(&trace, expected);
            }
        }
    }
}

/// TAKE-INV-003: loop segmentation keeps no pedal state of its own (`LoopCarry` covers key
/// continuation only). Sealing derives pedal state from the initial controller snapshot,
/// every admitted raw event across all passes and any refused pedal observations. With a
/// neutral start and none refused, a pedal edge before several wraps decides the closure.
#[test]
fn a_pedal_pressed_before_several_wraps_decides_the_stop_closure() {
    let mut reference = None;
    for partition in PARTITIONS {
        let (mut owner, sources) = setup();
        queue_transport(&mut owner, 320);
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        queue_input(&mut owner, sources[0], 10, [0xb0, 64, 127]);
        queue_input(&mut owner, sources[0], 45, [0x90, 62, 100]);
        for source in sources {
            queue_fence(&mut owner, source, 320);
        }
        let output = audio(&mut owner, 512, partition);
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert!(
            result.loop_passes().count() > 2,
            "the stop is several wraps later"
        );
        assert!(!result.initial_sources().next().unwrap().controls.pedals()[0]);
        assert!(result.terminal_sources().next().unwrap().controls.pedals()[0]);
        let closures: Vec<_> = result
            .closures()
            .map(|c| (c.time, c.key_held, c.pedal_held, c.reason))
            .collect();
        assert_eq!(
            closures,
            [(
                SampleTime::new(320),
                true,
                Some(true),
                CaptureStopReason::Stop
            )]
        );
        match &reference {
            None => reference = Some(output),
            Some(audio) => assert!(output == *audio, "audio depends on the callback partition"),
        }
    }
}
