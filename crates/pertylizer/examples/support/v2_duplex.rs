//! Experimental ALSA duplex acquisition. Backend timestamp estimates are not calibration.
use crate::pcm::{self, PcmInput, PcmOutput, PcmTake};
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use synth_engine_v2::{
    host::{
        OutputFormat,
        audio_input::{AudioInputConfig, InputFrame},
        input::{InputRate, InputTick, InputTickSpan, SimulatedInputClock},
    },
    quantities::{ChannelLayout, EventCount, PreparedBytes, SampleRate},
    render::AudioBlockMut,
    time::{FrameCount, SampleTime, issue_epoch},
};
use thiserror::Error;

type Error = Box<dyn std::error::Error>;
const MONITOR_GAIN: synth_core::Gain = synth_core::Gain::new(0.1);
#[derive(Error)]
#[error("duplex stopped: {message}; {retained} completed takes remain in error custody", retained = .takes.len())]
struct RunFailure {
    message: String,
    takes: Vec<PcmTake>,
}
impl std::fmt::Debug for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunFailure")
            .field("message", &self.message)
            .field("retained", &self.takes.len())
            .finish()
    }
}
struct InputState {
    owner: PcmInput,
    scratch: Box<[f32]>,
}
impl InputState {
    fn process<T: cpal::SizedSample>(
        &mut self,
        samples: &[T],
        timing: cpal::InputStreamTimestamp,
    ) -> bool
    where
        f32: cpal::FromSample<T>,
    {
        let Some(scratch) = self.scratch.get_mut(..samples.len()) else {
            return false;
        };
        for (to, from) in scratch.iter_mut().zip(samples) {
            *to = cpal::Sample::from_sample(*from);
        }
        self.owner.process(scratch, Some(timing)).is_ok()
    }
}
struct OutputState {
    owner: PcmOutput,
    scratch: Box<[f32]>,
    layout: ChannelLayout,
    latency: Option<(Duration, Duration)>,
    invalid_timestamps: u64,
}
impl OutputState {
    fn process<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        samples: &mut [T],
        timing: cpal::OutputStreamTimestamp,
    ) -> bool {
        let Some(scratch) = self.scratch.get_mut(..samples.len()) else {
            return false;
        };
        let Ok(block) =
            AudioBlockMut::new(scratch, samples.len() / self.layout.channels(), self.layout)
        else {
            return false;
        };
        if self.owner.render(block).is_err() {
            return false;
        }
        for (to, from) in samples.iter_mut().zip(scratch) {
            *to = T::from_sample(*from * MONITOR_GAIN.as_f32());
        }
        if let Some(age) = timing.playback.checked_duration_since(timing.callback) {
            self.latency = Some(
                self.latency
                    .map_or((age, age), |(low, high)| (low.min(age), high.max(age))),
            );
        } else {
            self.invalid_timestamps = self.invalid_timestamps.saturating_add(1);
        }
        true
    }
}
struct Streams {
    input: Option<cpal::Stream>,
    output: Option<cpal::Stream>,
    input_pool: Arc<HeapRb<InputState>>,
    output_pool: Arc<HeapRb<OutputState>>,
    failed: Arc<AtomicBool>,
    backend_kinds: Arc<AtomicU64>,
}
impl Streams {
    fn join(&mut self) {
        drop(self.input.take());
        drop(self.output.take());
    }
}
impl Drop for Streams {
    fn drop(&mut self) {
        self.join();
    }
}
fn format(config: &cpal::SupportedStreamConfig) -> Result<OutputFormat, Error> {
    let layout = match config.channels() {
        1 => ChannelLayout::Mono,
        2 => ChannelLayout::Stereo,
        _ => return Err("duplex supports one or two channels".into()),
    };
    let rate = config.sample_rate();
    if !(8000..=synth_core::audio::DeviceSampleRate::MAX_SUPPORTED.as_u32()).contains(&rate) {
        return Err("duplex sample rate outside engine range".into());
    }
    Ok(OutputFormat {
        rate: SampleRate::new(synth_core::audio::DeviceSampleRate::new(rate).as_f32())?,
        layout,
    })
}
pub(super) fn select(host: &cpal::Host, id: &str, input: bool) -> Result<cpal::Device, Error> {
    let devices: Vec<_> = if input {
        host.input_devices()?.collect()
    } else {
        host.output_devices()?.collect()
    };
    for device in devices {
        if device.id()?.to_string() == id {
            return Ok(device);
        }
    }
    Err(format!("ALSA endpoint unavailable: {id}").into())
}
/// An explicit supported PCM representation, selected before streams are built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
struct DuplexFormat(SampleFormat);
impl DuplexFormat {
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "f32" => Ok(Self(SampleFormat::F32)),
            "i32" => Ok(Self(SampleFormat::I32)),
            "i16" => Ok(Self(SampleFormat::I16)),
            _ => Err("duplex --format must be f32, i32 or i16".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ReopenRate {
    #[default]
    Alternate,
    RepeatDefault,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[must_use]
pub(super) struct DuplexOptions {
    format: Option<DuplexFormat>,
    reopen: ReopenRate,
}
impl DuplexOptions {
    pub(super) fn parse(arguments: &[String]) -> Result<Self, Error> {
        let mut options = Self::default();
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--format" if options.format.is_none() => {
                    let value = arguments
                        .next()
                        .ok_or("--format requires f32, i32 or i16")?;
                    options.format = Some(DuplexFormat::parse(value)?);
                }
                "--repeat-default" if options.reopen == ReopenRate::Alternate => {
                    options.reopen = ReopenRate::RepeatDefault;
                }
                _ => return Err(format!("unknown or repeated duplex option: {argument}").into()),
            }
        }
        Ok(options)
    }
    fn attempts(self) -> [bool; 2] {
        match self.reopen {
            ReopenRate::Alternate => [false, true],
            ReopenRate::RepeatDefault => [false, false],
        }
    }
}
fn choose_config(
    default: &cpal::SupportedStreamConfig,
    ranges: &[cpal::SupportedStreamConfigRange],
    alternate: bool,
    requested: Option<DuplexFormat>,
) -> Result<cpal::SupportedStreamConfig, Error> {
    if !alternate && requested.is_none() {
        return Ok(*default);
    }
    let rate = if alternate {
        if default.sample_rate() == 44_100 {
            48_000
        } else {
            44_100
        }
    } else {
        default.sample_rate()
    };
    let format = requested.map_or(default.sample_format(), |format| format.0);
    for range in ranges {
        if range.channels() == default.channels()
            && range.sample_format() == format
            && range.min_sample_rate() <= rate
            && range.max_sample_rate() >= rate
        {
            return Ok(range.with_sample_rate(rate));
        }
    }
    Err(format!(
        "endpoint does not advertise {:?} at {rate} Hz with {} channels",
        format,
        default.channels()
    )
    .into())
}
fn supported(
    device: &cpal::Device,
    input: bool,
    alternate: bool,
    requested: Option<DuplexFormat>,
) -> Result<cpal::SupportedStreamConfig, Error> {
    let default = if input {
        device.default_input_config()?
    } else {
        device.default_output_config()?
    };
    if !alternate && requested.is_none() {
        return Ok(default);
    }
    let ranges: Vec<_> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };
    choose_config(&default, &ranges, alternate, requested)
}
/// Shared format checks only: opening both streams, actual buffer negotiation,
/// allocation admission, clock quality and physical routing remain untested here.
pub(super) fn validate_pair(
    input: &cpal::SupportedStreamConfig,
    output: &cpal::SupportedStreamConfig,
) -> Result<(OutputFormat, OutputFormat), Error> {
    let input_format = format(input)?;
    let output_format = format(output)?;
    if input_format.layout != output_format.layout {
        return Err("duplex requires equal input/output channel layouts".into());
    }
    for config in [input, output] {
        if !matches!(
            config.sample_format(),
            SampleFormat::F32 | SampleFormat::I32 | SampleFormat::I16
        ) {
            return Err("duplex requires F32, I32 or I16 samples".into());
        }
    }
    Ok((input_format, output_format))
}
fn attempt(
    input: &cpal::Device,
    output: &cpal::Device,
    duration: Duration,
    alternate: bool,
    requested: Option<DuplexFormat>,
) -> Result<(PcmTake, bool), Error> {
    let input_config = supported(input, true, alternate, requested)?;
    let output_config = supported(output, false, alternate, requested)?;
    let (input_format, output_format) = validate_pair(&input_config, &output_config)?;
    let maximum = 8192;
    let channels = input_format.layout.channels();
    let (input_owner, output_owner, mut worker, bytes) = pcm::prepare(
        AudioInputConfig {
            input: input_format,
            output: output_format,
            clock: SimulatedInputClock::new(
                issue_epoch()?,
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(
                    FrameCount::new(u64::from(output_config.sample_rate())),
                    InputTickSpan::new(u64::from(input_config.sample_rate())),
                )?,
                InputTickSpan::new(0),
            ),
            first: InputFrame::new(0),
            chunk_frames: FrameCount::new(maximum as u64),
            chunk_cells: EventCount::measured(1),
            take_frames: FrameCount::new(
                u64::from(input_config.sample_rate()) * (duration.as_secs() + 1),
            ),
            monitor_frames: FrameCount::new(32_768),
            target_frames: FrameCount::new(4096),
            bytes: PreparedBytes::measured(256_000_000),
        },
        FrameCount::new(32_768),
    )?;
    let input_pool = Arc::new(HeapRb::new(1));
    let output_pool = Arc::new(HeapRb::new(1));
    let (mut input_writer, mut input_reader) = Arc::clone(&input_pool).split();
    let (mut output_writer, mut output_reader) = Arc::clone(&output_pool).split();
    if input_writer
        .try_push(InputState {
            owner: input_owner,
            scratch: vec![0.0; maximum * channels].into_boxed_slice(),
        })
        .is_err()
        || output_writer
            .try_push(OutputState {
                owner: output_owner,
                scratch: vec![0.0; maximum * channels].into_boxed_slice(),
                layout: output_format.layout,
                latency: None,
                invalid_timestamps: 0,
            })
            .is_err()
    {
        return Err("empty callback custody pool refused preparation".into());
    }
    drop(input_writer);
    drop(output_writer);
    let mut streams = Streams {
        input: None,
        output: None,
        input_pool,
        output_pool,
        failed: Arc::new(AtomicBool::new(false)),
        backend_kinds: Arc::new(AtomicU64::new(0)),
    };
    let failed = Arc::clone(&streams.failed);
    let backend = Arc::clone(&streams.failed);
    let kinds = Arc::clone(&streams.backend_kinds);
    let mut config = input_config.config();
    config.buffer_size = cpal::BufferSize::Fixed(256);
    streams.input = Some(input.build_input_stream_raw(
        config,
        input_config.sample_format(),
        move |data, info| {
            let Some(state) = input_reader.first_mut() else {
                failed.store(true, Ordering::Release);
                return;
            };
            let success = match data.sample_format() {
                SampleFormat::F32 => data
                    .as_slice::<f32>()
                    .is_some_and(|v| state.process(v, info.timestamp())),
                SampleFormat::I32 => data
                    .as_slice::<i32>()
                    .is_some_and(|v| state.process(v, info.timestamp())),
                SampleFormat::I16 => data
                    .as_slice::<i16>()
                    .is_some_and(|v| state.process(v, info.timestamp())),
                _ => false,
            };
            if !success {
                failed.store(true, Ordering::Release);
            }
        },
        move |error| {
            let bit = crate::linux::BACKEND_KINDS
                .iter()
                .position(|kind| *kind == error.kind())
                .unwrap_or(63);
            kinds.fetch_or(1_u64 << bit, Ordering::Relaxed);
            backend.store(true, Ordering::Release);
        },
        None,
    )?);
    let failed = Arc::clone(&streams.failed);
    let backend = Arc::clone(&streams.failed);
    let kinds = Arc::clone(&streams.backend_kinds);
    let mut config = output_config.config();
    config.buffer_size = cpal::BufferSize::Fixed(256);
    streams.output = Some(output.build_output_stream_raw(
        config,
        output_config.sample_format(),
        move |data, info| {
            let Some(state) = output_reader.first_mut() else {
                failed.store(true, Ordering::Release);
                return;
            };
            let success = match data.sample_format() {
                SampleFormat::F32 => data.as_slice_mut::<f32>().is_some_and(|v| {
                    v.fill(0.0);
                    state.process(v, info.timestamp())
                }),
                SampleFormat::I32 => data.as_slice_mut::<i32>().is_some_and(|v| {
                    v.fill(0);
                    state.process(v, info.timestamp())
                }),
                SampleFormat::I16 => data.as_slice_mut::<i16>().is_some_and(|v| {
                    v.fill(0);
                    state.process(v, info.timestamp())
                }),
                _ => false,
            };
            if !success {
                failed.store(true, Ordering::Release);
            }
        },
        move |error| {
            let bit = crate::linux::BACKEND_KINDS
                .iter()
                .position(|kind| *kind == error.kind())
                .unwrap_or(63);
            kinds.fetch_or(1_u64 << bit, Ordering::Relaxed);
            backend.store(true, Ordering::Release);
        },
        None,
    )?);
    for stream in [&streams.input, &streams.output].into_iter().flatten() {
        let negotiated = stream.buffer_size()?;
        if negotiated == 0 || u64::from(negotiated) > maximum as u64 {
            return Err("negotiated callback exceeds prepared duplex storage".into());
        }
        println!("duplex requested_buffer_frames=256 negotiated_buffer_frames={negotiated}");
    }
    println!(
        "duplex input={input_config:?} output={output_config:?} prepared_bytes={} monitor_target_input_frames=4096 monitor_gain=0.1 recording_compensation=none round_trip=unmeasured",
        bytes.get()
    );
    let mut run_error = None;
    for stream in [&streams.input, &streams.output] {
        if let Some(stream) = stream
            && let Err(error) = stream.play()
        {
            run_error = Some(error.to_string());
            break;
        }
    }
    let started = Instant::now();
    while run_error.is_none() && started.elapsed() < duration {
        worker.drain();
        if streams.failed.load(Ordering::Acquire) {
            run_error = Some("backend or callback fault".into());
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    streams.join();
    let Some(mut input_state) = Arc::get_mut(&mut streams.input_pool).and_then(Consumer::try_pop)
    else {
        return Err("input callback did not release custody after ALSA join".into());
    };
    let Some(output_state) = Arc::get_mut(&mut streams.output_pool).and_then(Consumer::try_pop)
    else {
        return Err("output callback did not release custody after ALSA join".into());
    };
    let lost = run_error.is_some();
    println!(
        "duplex_backend_error_mask={}",
        streams.backend_kinds.load(Ordering::Acquire)
    );
    let take = worker.finish(input_state.owner.stop(lost));
    println!(
        "duplex frames={} peak={} first_gap={:?} input_backend_latency={:?} output_backend_latency={:?} invalid_output_timestamps={} monitor={:?} input_monitor_dropped={} buffered={} error={run_error:?}",
        take.frames().as_u64(),
        take.peak(),
        take.report.gap,
        take.input_latency(),
        output_state.latency,
        output_state.invalid_timestamps,
        output_state.owner.counters(),
        take.report.monitor_dropped.as_u64(),
        output_state.owner.buffered().as_u64()
    );
    Ok((take, lost))
}
pub(super) fn run(
    host: &cpal::Host,
    input: &str,
    output: &str,
    seconds: &str,
    options: DuplexOptions,
) -> Result<(), Error> {
    let seconds: u32 = seconds.parse()?;
    if !(1..=30).contains(&seconds) {
        return Err("duplex duration must be 1 to 30 seconds per attempt".into());
    }
    println!(
        "duplex_format_selection={:?} reopen_rate={:?}",
        options.format, options.reopen
    );
    let mut takes = Vec::with_capacity(2);
    for alternate in options.attempts() {
        let result = (|| {
            attempt(
                &select(host, input, true)?,
                &select(host, output, false)?,
                Duration::from_secs(u64::from(seconds)),
                alternate,
                options.format,
            )
        })();
        match result {
            Ok((take, lost)) => {
                takes.push(take);
                if lost {
                    return Err(Box::new(RunFailure {
                        message: "callback/device failure".into(),
                        takes,
                    }));
                }
            }
            Err(error) => {
                return Err(Box::new(RunFailure {
                    message: error.to_string(),
                    takes,
                }));
            }
        }
    }
    println!(
        "retained_takes={} lifecycle=stop-join-finalize-prepare-reopen",
        takes.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::{SupportedBufferSize, SupportedStreamConfigRange};
    fn range(format: SampleFormat, low: u32, high: u32) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            2,
            low,
            high,
            SupportedBufferSize::Range { min: 64, max: 512 },
            format,
        )
    }
    #[test]
    fn explicit_i16_admits_an_i24_default_without_silently_falling_back() {
        let default = range(SampleFormat::I24, 48_000, 48_000).with_sample_rate(48_000);
        let available = [range(SampleFormat::I16, 44_100, 48_000)];
        assert_eq!(
            choose_config(&default, &available, false, None).expect("unchanged default"),
            default
        );
        let explicit = Some(DuplexFormat::parse("i16").expect("format"));
        for alternate in [false, true] {
            let selected = choose_config(&default, &available, alternate, explicit)
                .expect("explicit supported format");
            assert_eq!(selected.sample_format(), SampleFormat::I16);
            assert_eq!(
                selected.sample_rate(),
                if alternate { 44_100 } else { 48_000 }
            );
            assert!(validate_pair(&selected, &selected).is_ok());
        }
        assert!(
            choose_config(
                &default,
                &available,
                false,
                Some(DuplexFormat::parse("f32").expect("format"))
            )
            .is_err()
        );
        assert!(DuplexFormat::parse("i24").is_err());
        assert!(DuplexFormat::parse("").is_err());
    }
    #[test]
    fn explicit_format_does_not_invent_a_rate_or_change_the_channel_layout() {
        let default = range(SampleFormat::I24, 48_000, 48_000).with_sample_rate(48_000);
        let explicit = Some(DuplexFormat::parse("i16").expect("format"));
        assert!(
            choose_config(
                &default,
                &[range(SampleFormat::I16, 44_100, 44_100)],
                false,
                explicit
            )
            .is_err()
        );
        assert!(
            choose_config(
                &default,
                &[SupportedStreamConfigRange::new(
                    1,
                    48_000,
                    48_000,
                    SupportedBufferSize::Unknown,
                    SampleFormat::I16
                )],
                false,
                explicit
            )
            .is_err()
        );
        assert!(
            choose_config(
                &default,
                &[range(SampleFormat::I16, 48_000, 48_000)],
                true,
                explicit
            )
            .is_err()
        );
    }
    #[test]
    fn duplex_options_keep_legacy_reconfiguration_and_require_explicit_overrides() {
        let arguments = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        let legacy = DuplexOptions::parse(&[]).expect("legacy invocation");
        assert_eq!(legacy.format, None);
        assert_eq!(legacy.attempts(), [false, true]);
        let explicit = DuplexOptions::parse(&arguments(&["--format", "i16", "--repeat-default"]))
            .expect("explicit options");
        assert_eq!(
            explicit.format,
            Some(DuplexFormat::parse("i16").expect("format"))
        );
        assert_eq!(explicit.attempts(), [false, false]);
        for values in [
            &["--format"][..],
            &["--format", "i24"],
            &["--format", "i16", "--format", "f32"],
            &["--repeat-default", "--repeat-default"],
            &["--unknown"],
        ] {
            assert!(
                DuplexOptions::parse(&arguments(values)).is_err(),
                "{values:?}"
            );
        }
    }
}
