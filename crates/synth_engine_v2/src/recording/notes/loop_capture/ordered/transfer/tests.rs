use super::*;
use crate::{quantities::ChannelLayout, render::AudioBlockMut};

#[test]
fn missing_private_mapping_retains_the_serial_receipt_until_mapping_is_restored() {
    let (session, _) = crate::recording::notes::loop_capture::tests::ordered::setup();
    let (mut control, mut audio) = session.split(PreparedBytes::measured(65_536)).unwrap();
    let packet = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Stop)
        .unwrap();
    let id = packet.id();
    audio.enqueue(packet).unwrap();
    let mut samples = [0.0; 128];
    audio
        .render(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    // Only a private invariant violation can remove this map while its lane receipt exists.
    let index = audio.pending.iter().position(Option::is_some).unwrap();
    let retained = audio.pending[index].take();
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(audio.take_completed().is_none());
        }),
        0
    );
    assert_eq!(audio.session.commands.held, 1);
    assert_eq!(audio.session.commands.completed, 1);
    assert!(control.has_outstanding());
    audio.pending[index] = retained;
    let completion = audio.take_completed().unwrap();
    assert_eq!(completion.id(), id);
    let (collected, _) = control.collect(completion).unwrap();
    assert_eq!(collected, id);
    assert!(!control.has_outstanding());
    assert!(audio.take_completed().is_none());
}
