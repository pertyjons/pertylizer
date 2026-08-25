//! EVD-0016's release-platform CPAL timestamp probe.
//!
//! The audio callbacks copy CPAL's supplied timestamp into a fixed-size record
//! and attempt one push to a preallocated SPSC ring. CPAL 0.18.2 guarantees that
//! output arrives pre-filled with silence, so the output probe need not touch
//! the sample payload at all. In particular the callbacks do not read `Instant`,
//! allocate, lock, log, or perform file I/O. The main thread brackets
//! `Stream::now()` against a process-monotonic observer and writes the CSV after
//! both streams have stopped.

use std::{
    env,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    Device, DeviceType as CpalDeviceType, Stream, StreamConfig, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use ringbuf::{
    HeapCons, HeapRb,
    traits::{Consumer, Producer, Split},
};
use synth_core::{
    FrameCount, SampleCount, SamplePosition,
    audio::{BufferSize, ChannelCount, DeviceSampleRate},
};
use synth_engine_v2::time::QUANTUM_FRAMES;
use thiserror::Error;

const DEFAULT_CALLBACK_TARGET: u64 = 10_000;
const RING_HEADROOM: usize = 2_048;
const BRIDGE_INTERVAL: Duration = Duration::from_millis(20);
const BRIDGE_SAMPLE_BURST: usize = 1;
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const CALLBACK_STALL_TIMEOUT: Duration = Duration::from_secs(60);
const CPAL_VERSION: &str = "0.18.2";
#[cfg(test)]
const ENDPOINT_POLICY_CASES: &str = include_str!("evd_0016_endpoint_policy.tsv");

const fn callback_priority_path() -> &'static str {
    if cfg!(target_os = "linux") {
        "cpal-alsa-helper-noop-without-realtime-dbus"
    } else if cfg!(target_os = "windows") {
        "cpal-wasapi-promotion-attempt"
    } else if cfg!(target_os = "macos") {
        "coreaudio-backend-managed"
    } else {
        "outside-release-platforms"
    }
}

const fn callback_priority_observation() -> &'static str {
    if cfg!(target_os = "linux") {
        "no-cpal-promotion-by-build-contract"
    } else {
        "unobserved"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Input,
    Output,
}

impl Direction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeMode {
    Input,
    Output,
    Duplex,
}

impl ProbeMode {
    fn parse(value: &str) -> Result<Self, ProbeError> {
        match value {
            "input" => Ok(Self::Input),
            "output" => Ok(Self::Output),
            "duplex" => Ok(Self::Duplex),
            _ => Err(ProbeError::InvalidMode(value.to_owned())),
        }
    }

    const fn includes(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (Self::Input, Direction::Input) | (Self::Output, Direction::Output) | (Self::Duplex, _)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct CallbackTarget(u64);

impl CallbackTarget {
    fn parse(value: Option<&String>) -> Result<Self, ProbeError> {
        let callbacks = match value {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| ProbeError::InvalidCallbackTarget(raw.clone()))?,
            None => DEFAULT_CALLBACK_TARGET,
        };
        if callbacks == 0 {
            return Err(ProbeError::InvalidCallbackTarget(callbacks.to_string()));
        }
        Ok(Self(callbacks))
    }

    const fn as_u64(self) -> u64 {
        self.0
    }

    fn ring_capacity(self) -> Result<usize, ProbeError> {
        usize::try_from(self.0)
            .ok()
            .and_then(|value| value.checked_add(RING_HEADROOM))
            .ok_or(ProbeError::RingCapacityOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct PhysicalEndpointAttestation;

impl PhysicalEndpointAttestation {
    const fn as_str(self) -> &'static str {
        "true"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
struct SelectedDeviceId(String);

impl SelectedDeviceId {
    fn new(value: &str) -> Result<Self, ProbeError> {
        if value.is_empty() || value.starts_with("--") {
            return Err(ProbeError::InvalidDeviceId(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct ArtifactSequence(u64);

impl ArtifactSequence {
    const fn new(sequence: u64) -> Self {
        Self(sequence)
    }

    const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
struct StreamNanoseconds(u128);

impl StreamNanoseconds {
    const fn new(nanoseconds: u128) -> Self {
        Self(nanoseconds)
    }

    const fn as_u128(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
struct ObserverNanoseconds(u128);

impl ObserverNanoseconds {
    const fn new(nanoseconds: u128) -> Self {
        Self(nanoseconds)
    }

    const fn as_u128(self) -> u128 {
        self.0
    }
}

#[derive(Debug)]
struct RecordArguments {
    mode: ProbeMode,
    target: CallbackTarget,
    physical_attestation: PhysicalEndpointAttestation,
    input_device_id: Option<SelectedDeviceId>,
    output_device_id: Option<SelectedDeviceId>,
}

impl RecordArguments {
    fn parse(arguments: &[String]) -> Result<Self, ProbeError> {
        let mode = arguments
            .get(1)
            .ok_or(ProbeError::Usage)
            .and_then(|value| ProbeMode::parse(value))?;
        let mut callback_argument = None;
        let mut physical_attestation = None;
        let mut input_device_id = None;
        let mut output_device_id = None;
        let mut index = 2_usize;
        while let Some(argument) = arguments.get(index) {
            match argument.as_str() {
                "--physical" if physical_attestation.is_none() => {
                    physical_attestation = Some(PhysicalEndpointAttestation);
                    index += 1;
                }
                "--input-device" if input_device_id.is_none() => {
                    let value = arguments.get(index + 1).ok_or(ProbeError::Usage)?;
                    input_device_id = Some(SelectedDeviceId::new(value)?);
                    index += 2;
                }
                "--output-device" if output_device_id.is_none() => {
                    let value = arguments.get(index + 1).ok_or(ProbeError::Usage)?;
                    output_device_id = Some(SelectedDeviceId::new(value)?);
                    index += 2;
                }
                value if !value.starts_with("--") && callback_argument.is_none() => {
                    callback_argument = Some(argument);
                    index += 1;
                }
                _ => return Err(ProbeError::Usage),
            }
        }
        let physical_attestation =
            physical_attestation.ok_or(ProbeError::PhysicalAttestationRequired)?;
        if mode.includes(Direction::Input) && input_device_id.is_none() {
            return Err(ProbeError::ExplicitDeviceRequired(
                Direction::Input.as_str(),
            ));
        }
        if mode.includes(Direction::Output) && output_device_id.is_none() {
            return Err(ProbeError::ExplicitDeviceRequired(
                Direction::Output.as_str(),
            ));
        }
        if !mode.includes(Direction::Input) && input_device_id.is_some() {
            return Err(ProbeError::Usage);
        }
        if !mode.includes(Direction::Output) && output_device_id.is_some() {
            return Err(ProbeError::Usage);
        }
        Ok(Self {
            mode,
            target: CallbackTarget::parse(callback_argument)?,
            physical_attestation,
            input_device_id,
            output_device_id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct CallbackRecord {
    sequence: ArtifactSequence,
    sample_count: SampleCount,
    channels: ChannelCount,
    sample_rate: DeviceSampleRate,
    callback_ns: StreamNanoseconds,
    endpoint_ns: StreamNanoseconds,
    start_frame: SamplePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct BridgeRecord {
    sequence: ArtifactSequence,
    observer_before_ns: ObserverNanoseconds,
    stream_now_ns: StreamNanoseconds,
    observer_after_ns: ObserverNanoseconds,
}

struct MeasurementStream {
    direction: Direction,
    device_name: String,
    device_id: SelectedDeviceId,
    device_type: CpalDeviceType,
    config: SupportedStreamConfig,
    negotiated_buffer_frames: Result<BufferSize, String>,
    stream: Option<Stream>,
    consumer: HeapCons<CallbackRecord>,
    callback_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
    loss_count: Arc<AtomicU64>,
    xrun_count: Arc<AtomicU64>,
    device_unavailable_count: Arc<AtomicU64>,
    stream_invalidated_count: Arc<AtomicU64>,
    route_changed_count: Arc<AtomicU64>,
    realtime_denied_count: Arc<AtomicU64>,
    bridges: Vec<BridgeRecord>,
}

impl MeasurementStream {
    fn reached(&self, target: CallbackTarget) -> bool {
        self.callback_count.load(Ordering::Acquire) >= target.as_u64()
    }

    fn sample_bridge(&mut self, observer_start: Instant) -> Result<(), ProbeError> {
        let stream = self
            .stream
            .as_ref()
            .ok_or(ProbeError::StreamAlreadyStopped)?;
        for _ in 0..BRIDGE_SAMPLE_BURST {
            let before = ObserverNanoseconds::new(observer_start.elapsed().as_nanos());
            let stream_now = StreamNanoseconds::new(stream.now().as_nanos());
            let after = ObserverNanoseconds::new(observer_start.elapsed().as_nanos());
            let sequence = u64::try_from(self.bridges.len())
                .map(ArtifactSequence::new)
                .map_err(|_| ProbeError::BridgeSequenceOverflow)?;
            self.bridges.push(BridgeRecord {
                sequence,
                observer_before_ns: before,
                stream_now_ns: stream_now,
                observer_after_ns: after,
            });
        }
        Ok(())
    }

    fn print_metadata(&self, physical_attestation: PhysicalEndpointAttestation) {
        let direction = self.direction.as_str();
        println!("# {direction}_device_name={}", one_line(&self.device_name));
        println!(
            "# {direction}_device_id={}",
            one_line(self.device_id.as_str())
        );
        println!("# {direction}_device_type={:?}", self.device_type);
        println!(
            "# {direction}_physical_device_attested={}",
            physical_attestation.as_str()
        );
        println!("# {direction}_endpoint_policy=explicit-physical-endpoint");
        println!("# {direction}_stream_now_source=cpal-stream-instant-backend-private");
        println!("# {direction}_stream_now_backend_mode=unobservable-through-cpal-{CPAL_VERSION}");
        println!(
            "# {direction}_callback_priority_path={}",
            callback_priority_path()
        );
        println!(
            "# {direction}_callback_priority_observation={}",
            callback_priority_observation()
        );
        println!("# {direction}_bridge_sample_burst={BRIDGE_SAMPLE_BURST}");
        println!(
            "# {direction}_realtime_denied_count={}",
            self.realtime_denied_count.load(Ordering::Acquire)
        );
        println!(
            "# {direction}_sample_format={:?}",
            self.config.sample_format()
        );
        println!("# {direction}_sample_rate_hz={}", self.config.sample_rate());
        println!("# {direction}_channels={}", self.config.channels());
        println!(
            "# {direction}_supported_buffer_size={:?}",
            self.config.buffer_size()
        );
        match &self.negotiated_buffer_frames {
            Ok(frames) => println!("# {direction}_negotiated_buffer_frames={}", frames.as_u32()),
            Err(error) => {
                println!("# {direction}_negotiated_buffer_frames_error={error}");
            }
        }
        if cfg!(target_os = "linux") {
            println!(
                "# {direction}_stream_now_freshness_bound_source=cpal-{CPAL_VERSION}-alsa-one-negotiated-period-source-audit"
            );
            match &self.negotiated_buffer_frames {
                Ok(frames) => println!(
                    "# {direction}_stream_now_freshness_bound_frames={}",
                    frames.as_u32()
                ),
                Err(error) => println!("# {direction}_stream_now_freshness_bound_error={error}"),
            }
        } else {
            println!(
                "# {direction}_stream_now_freshness_bound_source=no-reviewed-release-platform-bound"
            );
            println!(
                "# {direction}_stream_now_freshness_bound_error=no pinned backend source audit for this release platform"
            );
        }
    }

    fn print_records(&mut self) {
        while let Some(record) = self.consumer.try_pop() {
            println!(
                "callback,{},{},{},{},{},{},{},{},,,,,,,,,",
                self.direction.as_str(),
                record.sequence.as_u64(),
                record.sample_count.as_usize(),
                record.channels.count(),
                record.sample_rate.as_u32(),
                record.callback_ns.as_u128(),
                record.endpoint_ns.as_u128(),
                record.start_frame.as_u64(),
            );
        }
        for bridge in &self.bridges {
            println!(
                "bridge,{},{},,,,,,,{},{},{},,,,,,",
                self.direction.as_str(),
                bridge.sequence.as_u64(),
                bridge.observer_before_ns.as_u128(),
                bridge.stream_now_ns.as_u128(),
                bridge.observer_after_ns.as_u128(),
            );
        }
        println!(
            "summary,{},0,,,,,,,,,,{},{},{},{},{},{}",
            self.direction.as_str(),
            self.error_count.load(Ordering::Acquire),
            self.loss_count.load(Ordering::Acquire),
            self.xrun_count.load(Ordering::Acquire),
            self.device_unavailable_count.load(Ordering::Acquire),
            self.stream_invalidated_count.load(Ordering::Acquire),
            self.route_changed_count.load(Ordering::Acquire),
        );
    }
}

#[derive(Debug, Error)]
enum ProbeError {
    #[error(
        "usage: evd_0016_cpal_timestamps list | record <input|output|duplex> [callbacks] --physical [--input-device <id>] [--output-device <id>]"
    )]
    Usage,
    #[error("invalid probe mode '{0}'; expected input, output, or duplex")]
    InvalidMode(String),
    #[error("invalid callback target '{0}'; expected a positive integer")]
    InvalidCallbackTarget(String),
    #[error("the requested callback count is too large for the probe ring")]
    RingCapacityOverflow,
    #[error("the observer bridge sequence does not fit the artifact schema")]
    BridgeSequenceOverflow,
    #[error("recording a release-platform artifact requires the explicit --physical attestation")]
    PhysicalAttestationRequired,
    #[error("recording {0} requires an explicit device ID from the list command")]
    ExplicitDeviceRequired(&'static str),
    #[error("invalid device ID '{0}'")]
    InvalidDeviceId(String),
    #[error("the selected {direction} device ID '{device_id}' was not found")]
    DeviceNotFound {
        direction: &'static str,
        device_id: String,
    },
    #[error("the selected {direction} endpoint is virtual or server-backed: {name} ({device_id})")]
    VirtualEndpointRejected {
        direction: &'static str,
        name: String,
        device_id: String,
    },
    #[error("the {0} devices could not be enumerated: {1}")]
    DeviceEnumeration(&'static str, String),
    #[error("device metadata is unavailable: {0}")]
    DeviceMetadata(String),
    #[error("the selected {0} device reported zero channels")]
    ZeroChannels(&'static str),
    #[error("the {direction} stream could not be built: {message}")]
    BuildStream {
        direction: &'static str,
        message: String,
    },
    #[error("the {direction} stream could not start: {message}")]
    StartStream {
        direction: &'static str,
        message: String,
    },
    #[error("the probe attempted to use a stream after it stopped")]
    StreamAlreadyStopped,
    #[error("the {0} callback direction made no progress for 60 seconds")]
    CallbackStalled(&'static str),
    #[error("the probe lost {count} {direction} callback records")]
    CallbackRecordsLost { direction: &'static str, count: u64 },
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn list_devices() -> Result<(), Box<dyn Error>> {
    let host = cpal::default_host();
    println!("host={}", host.id());
    println!("os={}", env::consts::OS);
    println!("arch={}", env::consts::ARCH);

    println!("output_devices:");
    for device in host.output_devices()? {
        print_device(Direction::Output, &device);
    }
    println!("input_devices:");
    for device in host.input_devices()? {
        print_device(Direction::Input, &device);
    }
    Ok(())
}

fn print_device(direction: Direction, device: &Device) {
    let (name, device_type) = device.description().map_or_else(
        |error| {
            (
                format!("<description error: {error}>"),
                "<unknown>".to_owned(),
            )
        },
        |description| {
            (
                description.name().to_owned(),
                format!("{:?}", description.device_type()),
            )
        },
    );
    let id = device
        .id()
        .map_or_else(|error| format!("<id error: {error}>"), |id| id.to_string());
    let config = match direction {
        Direction::Input => device.default_input_config(),
        Direction::Output => device.default_output_config(),
    };
    match config {
        Ok(config) => println!(
            "  name={} id={} type={} format={:?} rate={} channels={} buffers={:?}",
            one_line(&name),
            one_line(&id),
            device_type,
            config.sample_format(),
            config.sample_rate(),
            config.channels(),
            config.buffer_size(),
        ),
        Err(error) => println!(
            "  name={} id={} type={} config_error={}",
            one_line(&name),
            one_line(&id),
            device_type,
            one_line(&error.to_string()),
        ),
    }
}

fn stream_identity(
    device: &Device,
) -> Result<(String, SelectedDeviceId, CpalDeviceType), ProbeError> {
    let description = device
        .description()
        .map_err(|error| ProbeError::DeviceMetadata(error.to_string()))?;
    let id = device
        .id()
        .map_err(|error| ProbeError::DeviceMetadata(error.to_string()))?
        .to_string();
    Ok((
        description.name().to_owned(),
        SelectedDeviceId::new(&id)?,
        description.device_type(),
    ))
}

fn select_physical_device(
    host: &cpal::Host,
    direction: Direction,
    selected_id: &SelectedDeviceId,
) -> Result<Device, ProbeError> {
    let devices = match direction {
        Direction::Input => host.input_devices(),
        Direction::Output => host.output_devices(),
    }
    .map_err(|error| ProbeError::DeviceEnumeration(direction.as_str(), error.to_string()))?;
    for device in devices {
        let Ok(id) = device.id() else {
            continue;
        };
        if id.to_string() != selected_id.as_str() {
            continue;
        }
        let (name, device_id, device_type) = stream_identity(&device)?;
        if endpoint_is_disallowed(env::consts::OS, &name, device_id.as_str(), device_type) {
            return Err(ProbeError::VirtualEndpointRejected {
                direction: direction.as_str(),
                name,
                device_id: device_id.into_string(),
            });
        }
        return Ok(device);
    }
    Err(ProbeError::DeviceNotFound {
        direction: direction.as_str(),
        device_id: selected_id.as_str().to_owned(),
    })
}

fn endpoint_is_disallowed(
    platform: &str,
    name: &str,
    device_id: &str,
    device_type: CpalDeviceType,
) -> bool {
    let name = name.to_ascii_lowercase();
    let device_id = device_id.to_ascii_lowercase();
    let identity = format!("{name} {device_id}");
    const VIRTUAL_MARKERS: [&str; 12] = [
        "null",
        "pipewire",
        "pulseaudio",
        "virtual",
        "dummy",
        "blackhole",
        "soundflower",
        "loopback",
        "aggregate device",
        "multi-output device",
        "vb-cable",
        "cable input",
    ];
    if device_type == CpalDeviceType::Virtual
        || VIRTUAL_MARKERS
            .iter()
            .any(|marker| identity.contains(marker))
        || device_id == "alsa:default"
        || device_id == "alsa:pulse"
    {
        return true;
    }
    // CPAL's ALSA IDs expose the PCM kind. Only `hw:` is a direct hardware
    // endpoint; plughw, dmix, dsnoop, sysdefault, and other plugins may add a
    // converter, mixer, or server-backed clock.
    platform == "linux" && !device_id.starts_with("alsa:hw:")
}

fn count_stream_error(
    error: cpal::Error,
    total: &AtomicU64,
    xruns: &AtomicU64,
    device_unavailable: &AtomicU64,
    stream_invalidated: &AtomicU64,
    route_changed: &AtomicU64,
    realtime_denied: &AtomicU64,
) {
    total.fetch_add(1, Ordering::Relaxed);
    match error.kind() {
        cpal::ErrorKind::Xrun => {
            xruns.fetch_add(1, Ordering::Relaxed);
        }
        cpal::ErrorKind::DeviceNotAvailable => {
            device_unavailable.fetch_add(1, Ordering::Relaxed);
        }
        cpal::ErrorKind::StreamInvalidated => {
            stream_invalidated.fetch_add(1, Ordering::Relaxed);
        }
        cpal::ErrorKind::DeviceChanged => {
            route_changed.fetch_add(1, Ordering::Relaxed);
        }
        cpal::ErrorKind::RealtimeDenied => {
            realtime_denied.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

fn build_output(device: Device, target: CallbackTarget) -> Result<MeasurementStream, ProbeError> {
    let config = device
        .default_output_config()
        .map_err(|error| ProbeError::BuildStream {
            direction: Direction::Output.as_str(),
            message: error.to_string(),
        })?;
    let (device_name, device_id, device_type) = stream_identity(&device)?;
    if config.channels() == 0 {
        return Err(ProbeError::ZeroChannels(Direction::Output.as_str()));
    }
    let channels = ChannelCount::from(config.channels());
    let sample_rate = DeviceSampleRate::new(config.sample_rate());
    let sample_format = config.sample_format();
    let ring = HeapRb::<CallbackRecord>::new(target.ring_capacity()?);
    let (mut producer, consumer) = ring.split();
    let callback_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));
    let loss_count = Arc::new(AtomicU64::new(0));
    let xrun_count = Arc::new(AtomicU64::new(0));
    let device_unavailable_count = Arc::new(AtomicU64::new(0));
    let stream_invalidated_count = Arc::new(AtomicU64::new(0));
    let route_changed_count = Arc::new(AtomicU64::new(0));
    let realtime_denied_count = Arc::new(AtomicU64::new(0));
    let cumulative_frames = Arc::new(AtomicU64::new(0));

    let callback_count_rt = Arc::clone(&callback_count);
    let loss_count_rt = Arc::clone(&loss_count);
    let cumulative_frames_rt = Arc::clone(&cumulative_frames);
    let error_count_callback = Arc::clone(&error_count);
    let xrun_count_callback = Arc::clone(&xrun_count);
    let device_unavailable_count_callback = Arc::clone(&device_unavailable_count);
    let stream_invalidated_count_callback = Arc::clone(&stream_invalidated_count);
    let route_changed_count_callback = Arc::clone(&route_changed_count);
    let realtime_denied_count_callback = Arc::clone(&realtime_denied_count);
    let stream_config: StreamConfig = config.into();
    let stream = device
        .build_output_stream_raw(
            stream_config,
            sample_format,
            move |data, info| {
                let sequence =
                    ArtifactSequence::new(callback_count_rt.fetch_add(1, Ordering::Relaxed));
                let sample_count = SampleCount::new(data.len());
                let frames = FrameCount::new(data.len() / usize::from(channels.count()));
                let Ok(frames_u64) = u64::try_from(frames.as_usize()) else {
                    loss_count_rt.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let start_frame = SamplePosition::new(
                    cumulative_frames_rt.fetch_add(frames_u64, Ordering::Relaxed),
                );
                let timestamp = info.timestamp();
                let record = CallbackRecord {
                    sequence,
                    sample_count,
                    channels,
                    sample_rate,
                    callback_ns: StreamNanoseconds::new(timestamp.callback.as_nanos()),
                    endpoint_ns: StreamNanoseconds::new(timestamp.playback.as_nanos()),
                    start_frame,
                };
                if producer.try_push(record).is_err() {
                    loss_count_rt.fetch_add(1, Ordering::Relaxed);
                }
            },
            move |error| {
                count_stream_error(
                    error,
                    &error_count_callback,
                    &xrun_count_callback,
                    &device_unavailable_count_callback,
                    &stream_invalidated_count_callback,
                    &route_changed_count_callback,
                    &realtime_denied_count_callback,
                );
            },
            None,
        )
        .map_err(|error| ProbeError::BuildStream {
            direction: Direction::Output.as_str(),
            message: error.to_string(),
        })?;
    let negotiated_buffer_frames = stream
        .buffer_size()
        .map(BufferSize::new)
        .map_err(|error| one_line(&error.to_string()));

    Ok(MeasurementStream {
        direction: Direction::Output,
        device_name,
        device_id,
        device_type,
        config,
        negotiated_buffer_frames,
        stream: Some(stream),
        consumer,
        callback_count,
        error_count,
        loss_count,
        xrun_count,
        device_unavailable_count,
        stream_invalidated_count,
        route_changed_count,
        realtime_denied_count,
        bridges: Vec::new(),
    })
}

fn build_input(device: Device, target: CallbackTarget) -> Result<MeasurementStream, ProbeError> {
    let config = device
        .default_input_config()
        .map_err(|error| ProbeError::BuildStream {
            direction: Direction::Input.as_str(),
            message: error.to_string(),
        })?;
    let (device_name, device_id, device_type) = stream_identity(&device)?;
    if config.channels() == 0 {
        return Err(ProbeError::ZeroChannels(Direction::Input.as_str()));
    }
    let channels = ChannelCount::from(config.channels());
    let sample_rate = DeviceSampleRate::new(config.sample_rate());
    let sample_format = config.sample_format();
    let ring = HeapRb::<CallbackRecord>::new(target.ring_capacity()?);
    let (mut producer, consumer) = ring.split();
    let callback_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));
    let loss_count = Arc::new(AtomicU64::new(0));
    let xrun_count = Arc::new(AtomicU64::new(0));
    let device_unavailable_count = Arc::new(AtomicU64::new(0));
    let stream_invalidated_count = Arc::new(AtomicU64::new(0));
    let route_changed_count = Arc::new(AtomicU64::new(0));
    let realtime_denied_count = Arc::new(AtomicU64::new(0));
    let cumulative_frames = Arc::new(AtomicU64::new(0));

    let callback_count_rt = Arc::clone(&callback_count);
    let loss_count_rt = Arc::clone(&loss_count);
    let cumulative_frames_rt = Arc::clone(&cumulative_frames);
    let error_count_callback = Arc::clone(&error_count);
    let xrun_count_callback = Arc::clone(&xrun_count);
    let device_unavailable_count_callback = Arc::clone(&device_unavailable_count);
    let stream_invalidated_count_callback = Arc::clone(&stream_invalidated_count);
    let route_changed_count_callback = Arc::clone(&route_changed_count);
    let realtime_denied_count_callback = Arc::clone(&realtime_denied_count);
    let stream_config: StreamConfig = config.into();
    let stream = device
        .build_input_stream_raw(
            stream_config,
            sample_format,
            move |data, info| {
                let sequence =
                    ArtifactSequence::new(callback_count_rt.fetch_add(1, Ordering::Relaxed));
                let sample_count = SampleCount::new(data.len());
                let frames = FrameCount::new(data.len() / usize::from(channels.count()));
                let Ok(frames_u64) = u64::try_from(frames.as_usize()) else {
                    loss_count_rt.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let start_frame = SamplePosition::new(
                    cumulative_frames_rt.fetch_add(frames_u64, Ordering::Relaxed),
                );
                let timestamp = info.timestamp();
                let record = CallbackRecord {
                    sequence,
                    sample_count,
                    channels,
                    sample_rate,
                    callback_ns: StreamNanoseconds::new(timestamp.callback.as_nanos()),
                    endpoint_ns: StreamNanoseconds::new(timestamp.capture.as_nanos()),
                    start_frame,
                };
                if producer.try_push(record).is_err() {
                    loss_count_rt.fetch_add(1, Ordering::Relaxed);
                }
            },
            move |error| {
                count_stream_error(
                    error,
                    &error_count_callback,
                    &xrun_count_callback,
                    &device_unavailable_count_callback,
                    &stream_invalidated_count_callback,
                    &route_changed_count_callback,
                    &realtime_denied_count_callback,
                );
            },
            None,
        )
        .map_err(|error| ProbeError::BuildStream {
            direction: Direction::Input.as_str(),
            message: error.to_string(),
        })?;
    let negotiated_buffer_frames = stream
        .buffer_size()
        .map(BufferSize::new)
        .map_err(|error| one_line(&error.to_string()));

    Ok(MeasurementStream {
        direction: Direction::Input,
        device_name,
        device_id,
        device_type,
        config,
        negotiated_buffer_frames,
        stream: Some(stream),
        consumer,
        callback_count,
        error_count,
        loss_count,
        xrun_count,
        device_unavailable_count,
        stream_invalidated_count,
        route_changed_count,
        realtime_denied_count,
        bridges: Vec::new(),
    })
}

fn collect(streams: &mut [MeasurementStream], target: CallbackTarget) -> Result<(), ProbeError> {
    for measured in &*streams {
        measured
            .stream
            .as_ref()
            .ok_or(ProbeError::StreamAlreadyStopped)?
            .play()
            .map_err(|error| ProbeError::StartStream {
                direction: measured.direction.as_str(),
                message: error.to_string(),
            })?;
    }

    let observer_start = Instant::now();
    let mut next_bridge = Instant::now();
    let mut progress = vec![(0_u64, Instant::now()); streams.len()];
    loop {
        let now = Instant::now();
        if now >= next_bridge {
            for measured in &mut *streams {
                measured.sample_bridge(observer_start)?;
            }
            next_bridge = now + BRIDGE_INTERVAL;
        }
        if streams.iter().all(|measured| measured.reached(target)) {
            break;
        }
        for (measured, (previous_count, last_progress)) in streams.iter().zip(&mut progress) {
            if measured.reached(target) {
                continue;
            }
            let count = measured.callback_count.load(Ordering::Acquire);
            if count != *previous_count {
                *previous_count = count;
                *last_progress = now;
            } else if now.duration_since(*last_progress) >= CALLBACK_STALL_TIMEOUT {
                return Err(ProbeError::CallbackStalled(measured.direction.as_str()));
            }
        }
        thread::sleep(POLL_INTERVAL);
    }

    for measured in &mut *streams {
        // Some hosts cannot pause. Dropping the streams immediately after this
        // loop still stops callbacks, so a pause failure is harmless here.
        if let Some(stream) = measured.stream.take() {
            let _pause_result = stream.pause();
            drop(stream);
        }
    }
    Ok(())
}

fn record(arguments: RecordArguments) -> Result<(), Box<dyn Error>> {
    let host = cpal::default_host();
    let mut streams = Vec::with_capacity(2);
    if arguments.mode.includes(Direction::Output) {
        let selected_id = arguments
            .output_device_id
            .as_ref()
            .ok_or(ProbeError::ExplicitDeviceRequired("output"))?;
        let device = select_physical_device(&host, Direction::Output, selected_id)?;
        streams.push(build_output(device, arguments.target)?);
    }
    if arguments.mode.includes(Direction::Input) {
        let selected_id = arguments
            .input_device_id
            .as_ref()
            .ok_or(ProbeError::ExplicitDeviceRequired("input"))?;
        let device = select_physical_device(&host, Direction::Input, selected_id)?;
        streams.push(build_input(device, arguments.target)?);
    }

    collect(&mut streams, arguments.target)?;

    println!("# evd=EVD-0016");
    println!("# cpal_version={CPAL_VERSION}");
    println!(
        "# build_profile={}",
        if cfg!(debug_assertions) {
            "development"
        } else {
            "release"
        }
    );
    println!("# host={}", host.id());
    println!("# os={}", env::consts::OS);
    println!("# arch={}", env::consts::ARCH);
    println!("# control_fixture=none");
    println!("# observer_clock=std-instant-process-monotonic");
    println!("# observer_thread_priority=normal-not-promoted");
    println!("# quantum_frames={QUANTUM_FRAMES}");
    println!("# callback_target={}", arguments.target.as_u64());
    for measured in &streams {
        measured.print_metadata(arguments.physical_attestation);
    }
    println!(
        "record_type,direction,sequence,sample_count,channels,sample_rate_hz,callback_ns,endpoint_ns,start_frame,observer_before_ns,stream_now_ns,observer_after_ns,error_count,loss_count,xrun_count,device_unavailable_count,stream_invalidated_count,route_changed_count"
    );
    let mut first_loss = None;
    for measured in &mut streams {
        measured.print_records();
        let losses = measured.loss_count.load(Ordering::Acquire);
        if losses != 0 && first_loss.is_none() {
            first_loss = Some(ProbeError::CallbackRecordsLost {
                direction: measured.direction.as_str(),
                count: losses,
            });
        }
    }
    if let Some(error) = first_loss {
        return Err(Box::new(error));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("list") if arguments.len() == 1 => list_devices(),
        Some("record") => record(RecordArguments::parse(&arguments)?),
        _ => Err(Box::new(ProbeError::Usage)),
    }
}

#[cfg(test)]
mod tests {
    use cpal::DeviceType as CpalDeviceType;

    use super::{ENDPOINT_POLICY_CASES, endpoint_is_disallowed};

    #[test]
    fn shared_endpoint_policy_matrix_matches_probe() {
        let rows: Vec<_> = ENDPOINT_POLICY_CASES.lines().skip(1).collect();
        assert_eq!(rows.len(), 15, "endpoint policy matrix size changed");
        for line in rows {
            let mut fields = line.split('\t');
            let Some(platform) = fields.next() else {
                panic!("endpoint policy row has no platform");
            };
            let Some(name) = fields.next() else {
                panic!("endpoint policy row has no name");
            };
            let Some(device_id) = fields.next() else {
                panic!("endpoint policy row has no device ID");
            };
            let device_type = match fields.next() {
                Some("Speaker") => CpalDeviceType::Speaker,
                Some("Microphone") => CpalDeviceType::Microphone,
                Some("Virtual") => CpalDeviceType::Virtual,
                Some("Unknown") => CpalDeviceType::Unknown,
                _ => panic!("endpoint policy row has an unknown device type"),
            };
            let expected = match fields.next() {
                Some("true") => true,
                Some("false") => false,
                _ => panic!("endpoint policy row has no boolean outcome"),
            };
            assert!(
                fields.next().is_none(),
                "endpoint policy row has extra fields"
            );
            assert_eq!(
                endpoint_is_disallowed(platform, name, device_id, device_type),
                expected,
                "wrong endpoint policy for {platform} {name} ({device_id})"
            );
        }
    }
}
