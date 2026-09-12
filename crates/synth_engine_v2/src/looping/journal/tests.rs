//! Whole-callback audio and retained-observation oracles, including partial failure.

use super::{impulse, notes};
use crate::{
    diagnostics::RenderError,
    looping::{
        LoopBoundary, LoopFault,
        journal::{JournaledLoopStream, LoopJournalEndReason, LoopJournalPrepareError},
    },
    quantities::{ChannelLayout, LoopPassCount, PreparedBytes},
    render::AudioBlockMut,
    time::SampleTime,
};

fn journal(length: u64, entry: u64, passes: u32) -> JournaledLoopStream {
    JournaledLoopStream::prepare(
        impulse(length, entry, 0, 512),
        LoopPassCount::limit(passes).unwrap(),
        PreparedBytes::measured(1_000_000),
    )
    .unwrap()
}

#[test]
fn journal_and_audio_match_finite_pass_oracles_under_every_partition() {
    const TOTAL: usize = 2112;
    for length in [1_u64, 2, 63, 64, 65, 127, 513] {
        for entry in [0, length - 1, length] {
            let normalized = if entry == length { 0 } else { entry };
            for passes in [1_u32, 2, 4, 1000] {
                for partition in [1_usize, 37, 64, 256, 512] {
                    let mut owner = journal(length, entry, passes);
                    let initial = owner.initial();
                    let mut output = [9.0; TOTAL];
                    for chunk in output.chunks_mut(partition) {
                        let frames = chunk.len();
                        let mut result = Ok(());
                        assert_eq!(
                            crate::render_allocation::count_allocs(|| {
                                result = owner.render(
                                    AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
                                );
                            }),
                            0
                        );
                        result.unwrap();
                    }
                    // This oracle uses delivered samples, including Q priming;
                    // the observation oracle below uses rendered engine time.
                    for (frame, sample) in output.iter().enumerate() {
                        let expected = if frame < 64 {
                            0.0
                        } else {
                            f32::from((normalized + (frame - 64) as u64).is_multiple_of(length))
                        };
                        assert_eq!(*sample, expected);
                    }
                    let expected: Vec<_> = (length - normalized..2048)
                        .step_by(length as usize)
                        .collect();
                    let retained: Vec<_> = owner.boundaries().copied().collect();
                    assert_eq!(retained.len(), expected.len().min(passes as usize - 1));
                    for (index, boundary) in retained.iter().enumerate() {
                        assert_eq!(boundary.epoch, initial.epoch);
                        assert_eq!(boundary.at.as_u64(), expected[index]);
                        assert_eq!(boundary.previous.as_u64(), index as u64 + 1);
                        assert_eq!(boundary.next.as_u64(), index as u64 + 2);
                        assert_eq!(boundary.interval, owner.interval());
                    }
                    if let Some(at) = expected.get(passes as usize - 1) {
                        let end = owner.end().unwrap();
                        assert_eq!(end.reason, LoopJournalEndReason::PassLimit);
                        assert_eq!(end.at.as_u64(), *at);
                        assert_eq!(end.pass.as_u64(), u64::from(passes));
                        assert_eq!(end.epoch, initial.epoch);
                        assert_eq!(owner.finish(), end);
                    } else {
                        assert_eq!(owner.end(), None);
                        let end = owner.finish();
                        assert_eq!(end.reason, LoopJournalEndReason::Finished);
                        assert_eq!(end.at, SampleTime::new(2048));
                    }
                    // Reading, finishing and continued rendering do not replenish P.
                    assert_eq!(owner.boundaries().copied().collect::<Vec<_>>(), retained);
                    assert_eq!(owner.acknowledged().clock, SampleTime::new(2048));
                }
            }
        }
    }
}

#[test]
fn exact_heap_budget_and_zero_heap_single_pass_are_admitted() {
    let required = (3 * size_of::<Option<LoopBoundary>>()) as u64;
    for budget in [required - 1, required] {
        let result = JournaledLoopStream::prepare(
            impulse(65, 0, 0, 512),
            LoopPassCount::limit(4).unwrap(),
            PreparedBytes::measured(budget),
        );
        if budget == required {
            assert_eq!(result.unwrap().storage_bytes().get(), required);
        } else {
            assert!(matches!(
                result,
                Err(LoopJournalPrepareError::Budget { .. })
            ));
        }
    }
    assert!(matches!(
        JournaledLoopStream::prepare(
            impulse(65, 0, 0, 512),
            LoopPassCount::NONE,
            PreparedBytes::NONE,
        ),
        Err(LoopJournalPrepareError::Empty)
    ));
    let mut single = JournaledLoopStream::prepare(
        impulse(1, 0, 0, 512),
        LoopPassCount::limit(1).unwrap(),
        PreparedBytes::NONE,
    )
    .unwrap();
    assert_eq!(single.storage_bytes(), PreparedBytes::NONE);
    single
        .render(AudioBlockMut::new(&mut [0.0; 128], 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(single.boundaries().count(), 0);
    assert_eq!(single.end().unwrap().at, SampleTime::new(1));
}

#[test]
fn partially_advanced_failed_callback_contributes_no_boundaries_or_frontier() {
    let mut source = notes(65, 0, &[(3, 60, true)]);
    source.source.control.minter_mut().generation_ceiling = 0;
    let mut owner = JournaledLoopStream::prepare(
        source,
        LoopPassCount::limit(100).unwrap(),
        PreparedBytes::measured(100_000),
    )
    .unwrap();
    let mut output = [9.0; 512];
    owner
        .render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    let before = owner.acknowledged();
    let retained: Vec<_> = owner.boundaries().copied().collect();
    assert_eq!(retained.len(), 6);
    let mut result = Ok(());
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            result =
                owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap());
        }),
        0
    );
    let error = result.unwrap_err();
    assert!(matches!(error, LoopFault::Identity(_)));
    assert_eq!(output, [0.0; 512]);
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(owner.boundaries().copied().collect::<Vec<_>>(), retained);
    let end = owner.end().unwrap();
    assert_eq!(end.at, before.clock);
    assert_eq!(end.pass, before.pass);
    assert_eq!(end.reason, LoopJournalEndReason::RenderFault(error));
    assert_eq!(owner.finish(), end);
    assert_eq!(
        owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap()),
        Err(error)
    );
    assert_eq!(owner.end(), Some(end));
}

#[test]
fn shape_refusal_does_not_duplicate_old_borrowed_boundaries() {
    let mut owner = journal(3, 0, 100);
    owner
        .render(AudioBlockMut::new(&mut [0.0; 128], 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let before = owner.acknowledged();
    let retained: Vec<_> = owner.boundaries().copied().collect();
    assert!(matches!(
        owner.render(AudioBlockMut::new(&mut [9.0; 128], 64, ChannelLayout::Stereo).unwrap()),
        Err(LoopFault::Render(RenderError::OutputBufferShape { .. }))
    ));
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(owner.end(), None);
    assert_eq!(owner.boundaries().copied().collect::<Vec<_>>(), retained);
    owner
        .render(AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(owner.acknowledged().clock, SampleTime::new(128));
    let times: Vec<_> = owner.boundaries().map(|b| b.at.as_u64()).collect();
    assert_eq!(times, (3..128).step_by(3).collect::<Vec<_>>());
}

#[test]
fn pass_limit_survives_later_playback_fault_and_explicit_finish() {
    let mut owner = journal(65, 0, 1);
    owner
        .render(AudioBlockMut::new(&mut [0.0; 256], 256, ChannelLayout::Mono).unwrap())
        .unwrap();
    let end = owner.end().unwrap();
    assert_eq!(end.reason, LoopJournalEndReason::PassLimit);
    assert!(
        owner
            .render(AudioBlockMut::new(&mut [0.0; 513], 513, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    assert_eq!(owner.end(), Some(end));
    assert_eq!(owner.finish(), end);
}

#[test]
fn attachment_at_pending_wrap_and_finish_without_another_callback() {
    let mut source = impulse(64, 0, 0, 512);
    source
        .render(AudioBlockMut::new(&mut [0.0; 128], 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let before = source.snapshot();
    assert_eq!(before.position.as_u64(), 64);
    let mut owner = JournaledLoopStream::prepare(
        source,
        LoopPassCount::limit(1).unwrap(),
        PreparedBytes::NONE,
    )
    .unwrap();
    assert_eq!(owner.initial(), before);
    assert_eq!(owner.boundaries().count(), 0);
    owner
        .render(AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(owner.end().unwrap().at, before.clock);
    assert_eq!(owner.end().unwrap().pass, before.pass);

    let mut untouched = journal(65, 0, 2);
    let mut end = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| end = Some(untouched.finish())),
        0
    );
    assert_eq!(end.unwrap().at, SampleTime::ZERO);
    untouched
        .render(AudioBlockMut::new(&mut [0.0; 512], 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(untouched.end(), end);
    assert_eq!(untouched.boundaries().count(), 0);
    assert!(untouched.acknowledged().clock > SampleTime::ZERO);
}

#[test]
fn an_already_faulted_source_cannot_open_a_fresh_journal() {
    let mut source = impulse(65, 0, 0, 64);
    let fault = source
        .render(AudioBlockMut::new(&mut [0.0; 65], 65, ChannelLayout::Mono).unwrap())
        .unwrap_err();
    assert!(matches!(
        JournaledLoopStream::prepare(source, LoopPassCount::limit(1).unwrap(), PreparedBytes::NONE),
        Err(LoopJournalPrepareError::Faulted(found)) if found == fault
    ));
}
