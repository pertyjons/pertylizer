#![cfg(feature = "simulated-ingress")]
mod common;
use synth_engine_v2::{
    host::{
        OutputFormat,
        audio_input::{AudioGapReason, AudioInputConfig, InputFrame, SimulatedAudioInput},
        input::{InputRate, InputTick, InputTickSpan, SimulatedInputClock},
    },
    ir::IrNodeKind,
    quantities::{ChannelLayout, EventCount, PreparedBytes},
    render::AudioBlockMut,
    stream::StreamControl,
    time::{FrameCount, PlanPosition, SampleTime, StreamAnchor},
};
fn config() -> AudioInputConfig {
    let plan = common::admit(
        &common::source_plan(IrNodeKind::Silence),
        common::profile(256, ChannelLayout::Mono),
    );
    let (_, renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap();
    AudioInputConfig {
        input: OutputFormat {
            rate: common::rate(48000.0),
            layout: ChannelLayout::Mono,
        },
        output: OutputFormat {
            rate: common::rate(48000.0),
            layout: ChannelLayout::Mono,
        },
        clock: SimulatedInputClock::new(
            renderer.epoch(),
            SampleTime::new(100),
            InputTick::new(0),
            InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
            InputTickSpan::new(0),
        ),
        first: InputFrame::new(0),
        chunk_frames: FrameCount::new(256),
        chunk_cells: EventCount::measured(4),
        take_frames: FrameCount::new(65536),
        monitor_frames: FrameCount::new(1024),
        target_frames: FrameCount::new(512),
        bytes: PreparedBytes::measured(8_000_000),
    }
}
#[test]
fn monitoring_overflow_never_changes_original_pcm_or_timing() {
    let mut recorder = SimulatedAudioInput::prepare(config()).unwrap();
    let mut reference = Vec::new();
    for block in 0..100_u16 {
        let samples = vec![f32::from(block) / 100.0; 256];
        assert!(
            recorder
                .input(InputFrame::new(u64::from(block) * 256), &samples)
                .unwrap()
                .is_none()
        );
        recorder.drain_worker();
        reference.extend(samples);
    }
    assert!(recorder.monitor_counters().dropped.as_u64() > 0);
    assert!(recorder.buffered_frames().as_u64() <= 1024);
    let take = recorder.finish();
    assert_eq!(take.samples(), reference);
    assert_eq!(take.stamps().len(), 100);
    assert!(take.first_gap().is_none());
    for (index, stamp) in take.stamps().iter().enumerate() {
        assert_eq!(stamp.first.as_u64(), index as u64 * 256);
        assert_eq!(stamp.nominal.as_u64(), 100 + index as u64 * 256);
    }
}
#[test]
fn stalled_worker_pool_exhaustion_and_storage_exhaustion_retain_exact_prefixes() {
    let mut recorder = SimulatedAudioInput::prepare(config()).unwrap();
    for block in 0..4 {
        assert!(
            recorder
                .input(InputFrame::new(block * 256), &[0.25; 256])
                .unwrap()
                .is_none()
        );
    }
    let gap = recorder
        .input(InputFrame::new(1024), &[0.5; 256])
        .unwrap()
        .unwrap();
    assert_eq!(gap.reason, AudioGapReason::PoolFull);
    assert_eq!(gap.first, InputFrame::new(1024));
    recorder.drain_worker();
    recorder.input(InputFrame::new(1280), &[0.75; 256]).unwrap();
    let take = recorder.finish();
    assert_eq!(take.samples(), &[0.25; 1024]);
    assert_eq!(take.first_gap(), Some(gap));
    let mut options = config();
    options.take_frames = FrameCount::new(300);
    let mut recorder = SimulatedAudioInput::prepare(options).unwrap();
    recorder.input(InputFrame::new(0), &[0.1; 256]).unwrap();
    recorder.drain_worker();
    recorder.input(InputFrame::new(256), &[0.2; 128]).unwrap();
    recorder.drain_worker();
    let take = recorder.finish();
    assert_eq!(take.samples(), &[0.1; 256]);
    assert_eq!(take.first_gap().unwrap().first, InputFrame::new(256));
    assert_eq!(take.first_gap().unwrap().reason, AudioGapReason::TakeFull);
}
#[test]
fn source_gap_nonfinite_input_and_loss_need_no_final_callback() {
    for reason in [
        AudioGapReason::SourceGap,
        AudioGapReason::InvalidInput,
        AudioGapReason::DeviceLost,
    ] {
        let mut recorder = SimulatedAudioInput::prepare(config()).unwrap();
        recorder.input(InputFrame::new(0), &[0.5; 128]).unwrap();
        match reason {
            AudioGapReason::SourceGap => {
                recorder.input(InputFrame::new(256), &[1.0; 128]).unwrap();
            }
            AudioGapReason::InvalidInput => {
                assert!(
                    recorder
                        .input(InputFrame::new(128), &[f32::NAN; 128])
                        .is_err()
                );
            }
            _ => recorder.stop(true),
        }
        let take = recorder.finish();
        assert_eq!(take.samples(), &[0.5; 128]);
        let gap = take.first_gap().unwrap();
        assert_eq!(gap.reason, reason);
        assert_eq!(gap.first, InputFrame::new(128));
    }
}
#[test]
fn conversion_uses_source_rate_and_occupancy_feedback_bounds_synthetic_drift() {
    let mut options = config();
    options.input.rate = common::rate(24000.0);
    options.clock = SimulatedInputClock::new(
        options.clock.epoch(),
        SampleTime::ZERO,
        InputTick::new(0),
        InputRate::new(FrameCount::new(2), InputTickSpan::new(1)).unwrap(),
        InputTickSpan::new(0),
    );
    let mut recorder = SimulatedAudioInput::prepare(options).unwrap();
    let mut ramp = Vec::new();
    for i in 0..512_u16 {
        ramp.push(f32::from(i));
    }
    recorder.input(InputFrame::new(0), &ramp[..256]).unwrap();
    recorder.input(InputFrame::new(256), &ramp[256..]).unwrap();
    let mut pcm = [0.0; 128];
    recorder
        .monitor(AudioBlockMut::new(&mut pcm, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(pcm[0], 0.0);
    assert_eq!(pcm[1], 0.5);
    assert_eq!(pcm[2], 1.0);
    let take = recorder.finish();
    assert_eq!(take.samples(), ramp);
    assert_eq!(take.stamps()[1].nominal.as_u64(), 512);

    let mut recorder = SimulatedAudioInput::prepare(config()).unwrap();
    let mut input_frame = 0;
    for _ in 0..2 {
        recorder
            .input(InputFrame::new(input_frame), &[0.2; 256])
            .unwrap();
        recorder.drain_worker();
        input_frame += 256;
    }
    // 1 additional input frame per 10,000 output frames = +100 ppm. The finite
    // recording eventually fills; monitoring continues under its independent budget.
    let mut output = [0.0; 64];
    let mut extra = 0;
    for _ in 0..40_000 {
        extra += 64;
        let frames = if extra >= 10_000 {
            extra -= 10_000;
            65
        } else {
            64
        };
        recorder
            .input(InputFrame::new(input_frame), &vec![0.2; frames])
            .unwrap();
        input_frame += frames as u64;
        recorder.drain_worker();
        recorder
            .monitor(AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap())
            .unwrap();
        assert!(output.iter().all(|sample| *sample == 0.2));
    }
    assert_eq!(recorder.monitor_counters().dropped, FrameCount::ZERO);
    assert_eq!(recorder.monitor_counters().silent, FrameCount::ZERO);
    assert!((450..=650).contains(&recorder.buffered_frames().as_u64()));
}
