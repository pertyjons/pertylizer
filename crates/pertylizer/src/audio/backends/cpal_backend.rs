//! CPAL audio backend implementation.
//!
//! This module provides the default audio backend using the `cpal` crate,
//! which supports Windows (WASAPI), macOS (CoreAudio), Linux (ALSA/JACK),
//! and WebAssembly.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Stream, StreamConfig as CpalStreamConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use synth_core::DenormalGuard;

use ringbuf::HeapProd;
use ringbuf::traits::Producer;

use crate::audio::traits::{AudioBackend, AudioProcessor, AudioStream};
use crate::audio::types::*;

/// Map a cpal 0.18 unified [`cpal::Error`] onto our [`AudioError`].
///
/// cpal 0.18 collapsed the per-operation error enums into a single `Error`
/// carrying an [`cpal::ErrorKind`]. Route the kinds we model explicitly to
/// dedicated variants (so callers can react — reconnect on a lost device,
/// surface an unsupported config) and fall back to `generic`, the
/// operation-specific wrapper, for everything else.
fn map_cpal_error(err: &cpal::Error, generic: impl FnOnce(String) -> AudioError) -> AudioError {
    use cpal::ErrorKind as K;
    let msg = err.to_string();
    match err.kind() {
        K::DeviceNotAvailable | K::StreamInvalidated => AudioError::DeviceDisconnected,
        K::UnsupportedConfig => AudioError::UnsupportedConfig(msg),
        K::Xrun => AudioError::BufferUnderrun,
        _ => generic(msg),
    }
}

/// CPAL audio backend.
pub struct CpalBackend {
    host: Host,
}

impl CpalBackend {
    /// Create a new CPAL backend with the default host.
    pub fn new() -> AudioResult<Self> {
        Ok(Self {
            host: cpal::default_host(),
        })
    }

    /// Create a CPAL backend with a specific host.
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux",))]
    pub fn with_host_id(host_id: cpal::HostId) -> AudioResult<Self> {
        let host = cpal::host_from_id(host_id)
            .map_err(|e| AudioError::BackendError(format!("Failed to get host: {e}")))?;
        Ok(Self { host })
    }

    fn device_to_info(&self, device: &Device, device_type: DeviceType) -> AudioResult<DeviceInfo> {
        let description = device.description().map_err(|e| {
            AudioError::BackendError(format!("Failed to get device description: {e}"))
        })?;
        let name = description.name().to_string();

        // Get supported output configs
        let (output_channels, supported_sample_rates, min_buffer, max_buffer) =
            if let Ok(configs) = device.supported_output_configs() {
                let configs: Vec<_> = configs.collect();
                if configs.is_empty() {
                    (
                        ChannelCount::Stereo,
                        vec![DeviceSampleRate::default()],
                        BufferSize::SMALL,
                        BufferSize::VERY_LARGE,
                    )
                } else {
                    let channels = ChannelCount::from(configs[0].channels());
                    let rates: Vec<DeviceSampleRate> = configs
                        .iter()
                        .flat_map(|c| {
                            let min = c.min_sample_rate();
                            let max = c.max_sample_rate();
                            [44100, 48000, 96000]
                                .into_iter()
                                .filter(move |&r| r >= min && r <= max)
                                .map(DeviceSampleRate::new)
                        })
                        .collect();
                    (
                        channels,
                        if rates.is_empty() {
                            vec![DeviceSampleRate::default()]
                        } else {
                            rates
                        },
                        BufferSize::SMALL,
                        BufferSize::VERY_LARGE,
                    )
                }
            } else {
                (
                    ChannelCount::Stereo,
                    vec![DeviceSampleRate::default()],
                    BufferSize::SMALL,
                    BufferSize::VERY_LARGE,
                )
            };

        // Get supported input configs
        let input_channels = if let Ok(configs) = device.supported_input_configs() {
            configs
                .into_iter()
                .next()
                .map(|c| ChannelCount::from(c.channels()))
                .unwrap_or(ChannelCount::Mono)
        } else {
            ChannelCount::Mono
        };

        Ok(DeviceInfo {
            id: name.clone(),
            name,
            device_type,
            supported_sample_rates,
            min_buffer_size: min_buffer,
            max_buffer_size: max_buffer,
            input_channels,
            output_channels,
        })
    }

    fn find_device(&self, device_id: &str) -> AudioResult<Device> {
        let devices = self
            .host
            .output_devices()
            .map_err(|e| AudioError::BackendError(format!("Failed to enumerate devices: {e}")))?;

        for device in devices {
            if let Ok(desc) = device.description()
                && desc.name() == device_id
            {
                return Ok(device);
            }
        }

        Err(AudioError::DeviceNotFound(device_id.to_string()))
    }

    fn find_input_device(&self, device_id: &str) -> AudioResult<Device> {
        let devices = self.host.input_devices().map_err(|e| {
            AudioError::BackendError(format!("Failed to enumerate input devices: {e}"))
        })?;

        for device in devices {
            if let Ok(desc) = device.description()
                && desc.name() == device_id
            {
                return Ok(device);
            }
        }

        Err(AudioError::DeviceNotFound(device_id.to_string()))
    }
}

impl AudioBackend for CpalBackend {
    fn name(&self) -> &str {
        "cpal"
    }

    fn is_available(&self) -> bool {
        self.host.default_output_device().is_some()
    }

    fn devices(&self) -> AudioResult<Vec<DeviceInfo>> {
        let mut devices = Vec::new();

        if let Ok(output_devices) = self.host.output_devices() {
            for device in output_devices {
                if let Ok(info) = self.device_to_info(&device, DeviceType::Output) {
                    devices.push(info);
                }
            }
        }

        if let Ok(input_devices) = self.host.input_devices() {
            for device in input_devices {
                if let Ok(info) = self.device_to_info(&device, DeviceType::Input) {
                    // Avoid duplicates (some backends list duplex devices in both)
                    if !devices.iter().any(|d| d.id == info.id) {
                        devices.push(info);
                    }
                }
            }
        }

        Ok(devices)
    }

    fn default_output_device(&self) -> AudioResult<DeviceInfo> {
        let device = self
            .host
            .default_output_device()
            .ok_or(AudioError::NoDefaultDevice)?;
        self.device_to_info(&device, DeviceType::Output)
    }

    fn default_input_device(&self) -> AudioResult<DeviceInfo> {
        let device = self
            .host
            .default_input_device()
            .ok_or(AudioError::NoDefaultDevice)?;
        self.device_to_info(&device, DeviceType::Input)
    }

    fn create_output_stream(
        &self,
        device_id: Option<&str>,
        config: &StreamConfig,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<Box<dyn AudioStream>> {
        let device = if let Some(id) = device_id {
            self.find_device(id)?
        } else {
            self.host
                .default_output_device()
                .ok_or(AudioError::NoDefaultDevice)?
        };

        let stream = CpalStream::new(device, config, processor)?;
        Ok(Box::new(stream))
    }

    fn create_input_stream(
        &self,
        device_id: Option<&str>,
        config: &StreamConfig,
        engine_producer: HeapProd<synth_core::StereoSample>,
        gui_producer: HeapProd<synth_core::StereoSample>,
    ) -> AudioResult<Box<dyn AudioStream>> {
        let device = if let Some(id) = device_id {
            self.find_input_device(id)?
        } else {
            self.host
                .default_input_device()
                .ok_or(AudioError::NoDefaultDevice)?
        };

        let stream = CpalInputStream::new(device, config, engine_producer, gui_producer)?;
        Ok(Box::new(stream))
    }
}

/// Active CPAL audio stream.
struct CpalStream {
    stream: Stream,
    info: StreamInfo,
    running: Arc<AtomicBool>,
    position: Arc<AtomicU64>,
    async_errors: Arc<CpalAsyncErrorState>,
    diagnostic_source: CpalDiagnosticSource,
}

impl CpalStream {
    fn new(
        device: Device,
        config: &StreamConfig,
        mut processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<Self> {
        let channels = config.channels.count();
        let diagnostic_source =
            CpalDiagnosticSource::from_device(&device, CpalStreamDirection::Output);

        // Build cpal config
        let cpal_config = CpalStreamConfig {
            channels,
            sample_rate: config.sample_rate.as_u32(),
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size.as_u32()),
        };

        // Get actual latency
        let output_latency = device
            .default_output_config()
            .ok()
            .map(|c| {
                let buffer_frames = config.buffer_size.as_u32() as f64;
                let sample_rate = c.sample_rate() as f64;
                Duration::from_secs_f64(buffer_frames / sample_rate)
            })
            .unwrap_or(Duration::from_millis(10));

        let mut info = StreamInfo {
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
            channels: config.channels,
            output_latency,
            input_latency: None,
        };

        let running = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicU64::new(0));
        let async_errors = Arc::new(CpalAsyncErrorState::default());
        // Clone for the callback
        let running_clone = Arc::clone(&running);
        let position_clone = Arc::clone(&position);
        let async_errors_rt = Arc::clone(&async_errors);
        let async_errors_callback = Arc::clone(&async_errors);
        let sample_rate = config.sample_rate;
        let start_time = Instant::now();

        // Notify processor of stream parameters
        processor.on_stream_start(&info);

        // Build the stream
        let stream = device
            .build_output_stream(
                cpal_config,
                move |data: &mut [f32], output_info: &cpal::OutputCallbackInfo| {
                    let _denormal_guard = DenormalGuard::new();

                    if !running_clone.load(Ordering::Relaxed) {
                        // Keep stopped-stream silence engine-owned even though
                        // CPAL 0.18.2 also pre-fills every output buffer.
                        data.fill(0.0);
                        return;
                    }
                    async_errors_rt.notify_processor(processor.as_mut());

                    let frames = data.len() / channels as usize;
                    let current_position = position_clone.load(Ordering::Relaxed);

                    // cpal 0.18 exposes the host's per-callback timestamps. The gap
                    // between when this callback runs and when the samples reach the
                    // DAC is the true output latency for this buffer; prefer it over
                    // the static estimate and fall back before the clock has warmed
                    // up. Integer-only, no alloc/lock — RT-safe.
                    let ts = output_info.timestamp();
                    let live_latency = ts.playback.duration_since(ts.callback);
                    let latency = if live_latency.is_zero() {
                        output_latency
                    } else {
                        live_latency
                    };

                    let context = AudioCallbackContext {
                        sample_rate,
                        frames,
                        channels,
                        stream_time: start_time.elapsed().as_secs_f64(),
                        sample_position: current_position,
                        output_latency: synth_core::Seconds::new(latency.as_secs_f32()),
                    };

                    processor.process(data, &context);

                    position_clone.fetch_add(frames as u64, Ordering::Relaxed);
                },
                move |err| {
                    async_errors_callback.record_output(err.kind());
                },
                None, // No timeout, blocking mode
            )
            .map_err(|e| map_cpal_error(&e, AudioError::StreamCreationFailed))?;

        // Refine the reported latency with the buffer size cpal actually
        // negotiated (0.18's `buffer_size()` accessor) — it can differ from
        // what we requested. Keep the estimate if the host can't report it.
        if let Ok(frames) = stream.buffer_size() {
            let sample_rate_hz = f64::from(config.sample_rate.as_u32());
            if frames > 0 && sample_rate_hz > 0.0 {
                info.output_latency = Duration::from_secs_f64(f64::from(frames) / sample_rate_hz);
            }
        }

        Ok(Self {
            stream,
            info,
            running,
            position,
            async_errors,
            diagnostic_source,
        })
    }
}

impl AudioStream for CpalStream {
    fn start(&mut self) -> AudioResult<()> {
        if self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamAlreadyRunning);
        }

        self.stream.play().map_err(|e| {
            map_cpal_error(&e, |m| {
                AudioError::BackendError(format!("start stream: {m}"))
            })
        })?;

        self.running.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamNotRunning);
        }

        self.running.store(false, Ordering::Relaxed);

        self.stream.pause().map_err(|e| {
            map_cpal_error(&e, |m| {
                AudioError::BackendError(format!("stop stream: {m}"))
            })
        })?;

        Ok(())
    }

    fn take_async_error(&mut self) -> Option<AudioError> {
        self.async_errors
            .take_error(CpalStreamDirection::Output, &self.diagnostic_source)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }
}

/// Active CPAL audio input stream.
///
/// Captures audio from an input device and writes to two ring buffer producers
/// (one for the engine, one for the GUI).
struct CpalInputStream {
    stream: Stream,
    info: StreamInfo,
    running: Arc<AtomicBool>,
    position: Arc<AtomicU64>,
    async_errors: Arc<CpalAsyncErrorState>,
    diagnostic_source: CpalDiagnosticSource,
}

impl CpalInputStream {
    fn new(
        device: Device,
        config: &StreamConfig,
        mut engine_producer: HeapProd<synth_core::StereoSample>,
        mut gui_producer: HeapProd<synth_core::StereoSample>,
    ) -> AudioResult<Self> {
        let channels = config.channels.count();
        let diagnostic_source =
            CpalDiagnosticSource::from_device(&device, CpalStreamDirection::Input);

        let cpal_config = CpalStreamConfig {
            channels,
            sample_rate: config.sample_rate.as_u32(),
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size.as_u32()),
        };

        let input_latency = device
            .default_input_config()
            .ok()
            .map(|c| {
                let buffer_frames = config.buffer_size.as_u32() as f64;
                let sample_rate = c.sample_rate() as f64;
                Duration::from_secs_f64(buffer_frames / sample_rate)
            })
            .unwrap_or(Duration::from_millis(10));

        let info = StreamInfo {
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
            channels: config.channels,
            output_latency: Duration::ZERO,
            input_latency: Some(input_latency),
        };

        let running = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicU64::new(0));
        let async_errors = Arc::new(CpalAsyncErrorState::default());
        let running_clone = Arc::clone(&running);
        let position_clone = Arc::clone(&position);
        let async_errors_callback = Arc::clone(&async_errors);

        let stream = device
            .build_input_stream(
                cpal_config,
                move |data: &[f32], _input_info: &cpal::InputCallbackInfo| {
                    if !running_clone.load(Ordering::Relaxed) {
                        return;
                    }

                    // Push complete stereo frames atomically. A full ring drops
                    // whole frames, so left/right can never become skewed.
                    let channel_count = usize::from(channels.max(1));
                    let mut frames = 0_u64;
                    for input_frame in data.chunks_exact(channel_count) {
                        let frame = stereo_sample_from_input(input_frame);
                        let _ = engine_producer.try_push(frame);
                        let _ = gui_producer.try_push(frame);
                        frames += 1;
                    }
                    position_clone.fetch_add(frames, Ordering::Relaxed);
                },
                move |err| {
                    async_errors_callback.record_input(err.kind());
                },
                None,
            )
            .map_err(|e| map_cpal_error(&e, AudioError::StreamCreationFailed))?;

        Ok(Self {
            stream,
            info,
            running,
            position,
            async_errors,
            diagnostic_source,
        })
    }
}

impl AudioStream for CpalInputStream {
    fn start(&mut self) -> AudioResult<()> {
        if self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamAlreadyRunning);
        }

        self.stream.play().map_err(|e| {
            map_cpal_error(&e, |m| {
                AudioError::BackendError(format!("start input stream: {m}"))
            })
        })?;

        self.running.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamNotRunning);
        }

        self.running.store(false, Ordering::Relaxed);

        self.stream.pause().map_err(|e| {
            map_cpal_error(&e, |m| {
                AudioError::BackendError(format!("stop input stream: {m}"))
            })
        })?;

        Ok(())
    }

    fn take_async_error(&mut self) -> Option<AudioError> {
        self.async_errors
            .take_error(CpalStreamDirection::Input, &self.diagnostic_source)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }
}

fn stereo_sample_from_input(input_frame: &[f32]) -> synth_core::StereoSample {
    let left = input_frame.first().copied().unwrap_or(0.0);
    let right = input_frame.get(1).copied().unwrap_or(left);
    synth_core::StereoSample::new(left, right)
}

const UNKNOWN_CPAL_ERROR_BIT: u64 = 1 << 63;
const KNOWN_CPAL_ERROR_KINDS: [cpal::ErrorKind; 14] = [
    cpal::ErrorKind::DeviceBusy,
    cpal::ErrorKind::DeviceChanged,
    cpal::ErrorKind::DeviceNotAvailable,
    cpal::ErrorKind::HostUnavailable,
    cpal::ErrorKind::InvalidInput,
    cpal::ErrorKind::PermissionDenied,
    cpal::ErrorKind::RealtimeDenied,
    cpal::ErrorKind::ResourceExhausted,
    cpal::ErrorKind::StreamInvalidated,
    cpal::ErrorKind::UnsupportedConfig,
    cpal::ErrorKind::UnsupportedOperation,
    cpal::ErrorKind::Xrun,
    cpal::ErrorKind::BackendError,
    cpal::ErrorKind::Other,
];

const fn cpal_error_bit(kind: cpal::ErrorKind) -> u64 {
    use cpal::ErrorKind as K;
    match kind {
        K::DeviceBusy => 1 << 0,
        K::DeviceChanged => 1 << 1,
        K::DeviceNotAvailable => 1 << 2,
        K::HostUnavailable => 1 << 3,
        K::InvalidInput => 1 << 4,
        K::PermissionDenied => 1 << 5,
        K::RealtimeDenied => 1 << 6,
        K::ResourceExhausted => 1 << 7,
        K::StreamInvalidated => 1 << 8,
        K::UnsupportedConfig => 1 << 9,
        K::UnsupportedOperation => 1 << 10,
        K::Xrun => 1 << 11,
        K::BackendError => 1 << 12,
        K::Other => 1 << 13,
        _ => UNKNOWN_CPAL_ERROR_BIT,
    }
}

#[derive(Default)]
struct CpalAsyncErrorState {
    diagnostic_bits: AtomicU64,
    output_xrun_pending: AtomicBool,
}

impl CpalAsyncErrorState {
    fn record_output(&self, kind: cpal::ErrorKind) {
        self.record_diagnostic(kind);
        if kind == cpal::ErrorKind::Xrun {
            // Multiple xruns before the next output callback deliberately
            // coalesce into one processor notification. EVD-0016 owns exact
            // occurrence counters; this path owns bounded fault signaling.
            self.output_xrun_pending.store(true, Ordering::Release);
        }
    }

    fn record_input(&self, kind: cpal::ErrorKind) {
        self.record_diagnostic(kind);
    }

    fn record_diagnostic(&self, kind: cpal::ErrorKind) {
        let bit = cpal_error_bit(kind);
        // Repeated occurrences of the same category coalesce until the
        // non-real-time consumer drains this bitset.
        self.diagnostic_bits.fetch_or(bit, Ordering::Release);
    }

    fn notify_processor(&self, processor: &mut dyn AudioProcessor) {
        if self.output_xrun_pending.swap(false, Ordering::AcqRel) {
            processor.on_error(AudioError::BufferUnderrun);
        }
    }

    fn take_error(
        &self,
        direction: CpalStreamDirection,
        source: &CpalDiagnosticSource,
    ) -> Option<AudioError> {
        let bits = self.diagnostic_bits.swap(0, Ordering::AcqRel);
        if bits == 0 {
            return None;
        }
        let mut labels = Vec::new();
        for kind in KNOWN_CPAL_ERROR_KINDS {
            if bits & cpal_error_bit(kind) != 0 {
                labels.push(cpal_error_category(kind).as_str());
            }
        }
        if bits & UNKNOWN_CPAL_ERROR_BIT != 0 {
            labels.push("unknown");
        }
        Some(AudioError::BackendError(format!(
            "one or more asynchronous CPAL {} stream errors for {}: {}",
            direction.as_str(),
            source.as_str(),
            labels.join(", ")
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpalStreamDirection {
    Input,
    Output,
}

impl CpalStreamDirection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
struct CpalDiagnosticSource(String);

impl CpalDiagnosticSource {
    fn from_device(device: &Device, direction: CpalStreamDirection) -> Self {
        let name = device.description().map_or_else(
            |error| format!("<name unavailable: {error}>"),
            |description| description.name().replace(['\r', '\n'], " "),
        );
        let id = device.id().map_or_else(
            |error| format!("<id unavailable: {error}>"),
            |id| id.to_string().replace(['\r', '\n'], " "),
        );
        Self(format!("{} device {name:?} ({id})", direction.as_str()))
    }

    #[cfg(test)]
    fn for_test(value: &str) -> Self {
        Self(value.to_string())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct CpalErrorCategory(&'static str);

impl CpalErrorCategory {
    const fn as_str(self) -> &'static str {
        self.0
    }
}

const fn cpal_error_category(kind: cpal::ErrorKind) -> CpalErrorCategory {
    use cpal::ErrorKind as K;
    CpalErrorCategory(match kind {
        K::DeviceBusy => "device-busy",
        K::DeviceChanged => "device-changed",
        K::DeviceNotAvailable => "device-not-available",
        K::HostUnavailable => "host-unavailable",
        K::InvalidInput => "invalid-input",
        K::PermissionDenied => "permission-denied",
        K::RealtimeDenied => "realtime-denied",
        K::ResourceExhausted => "resource-exhausted",
        K::StreamInvalidated => "stream-invalidated",
        K::UnsupportedConfig => "unsupported-config",
        K::UnsupportedOperation => "unsupported-operation",
        K::Xrun => "xrun",
        K::BackendError => "backend-error",
        K::Other => "other",
        _ => "unknown",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ErrorRecordingProcessor {
        underruns: usize,
    }

    impl AudioProcessor for ErrorRecordingProcessor {
        fn process(&mut self, _output: &mut [f32], _context: &AudioCallbackContext) {}

        fn on_error(&mut self, error: AudioError) {
            if let AudioError::BufferUnderrun = error {
                self.underruns += 1;
            }
        }
    }

    #[test]
    fn input_frame_conversion_duplicates_mono_and_selects_first_stereo_pair() {
        assert_eq!(
            stereo_sample_from_input(&[0.25]),
            synth_core::StereoSample::new(0.25, 0.25)
        );
        assert_eq!(
            stereo_sample_from_input(&[0.25, -0.5, 0.75]),
            synth_core::StereoSample::new(0.25, -0.5)
        );
    }

    #[test]
    fn known_cpal_018_error_kinds_have_stable_diagnostic_categories() {
        use cpal::ErrorKind as K;

        // ErrorKind is non-exhaustive, so the wildcard is required. The
        // evidence dependency gate makes a resolved CPAL version change fail
        // until the probe/analyzer are updated; that review must also extend
        // this known-variant table. A newly introduced kind remains visibly
        // "unknown" rather than being mislabeled as an existing category.
        let cases = [
            (K::DeviceBusy, "device-busy"),
            (K::DeviceChanged, "device-changed"),
            (K::DeviceNotAvailable, "device-not-available"),
            (K::HostUnavailable, "host-unavailable"),
            (K::InvalidInput, "invalid-input"),
            (K::PermissionDenied, "permission-denied"),
            (K::RealtimeDenied, "realtime-denied"),
            (K::ResourceExhausted, "resource-exhausted"),
            (K::StreamInvalidated, "stream-invalidated"),
            (K::UnsupportedConfig, "unsupported-config"),
            (K::UnsupportedOperation, "unsupported-operation"),
            (K::Xrun, "xrun"),
            (K::BackendError, "backend-error"),
            (K::Other, "other"),
        ];
        let case_count = cases.len();
        assert_eq!(case_count, KNOWN_CPAL_ERROR_KINDS.len());
        let mut observed_bits = 0_u64;
        for ((kind, expected), listed_kind) in cases.into_iter().zip(KNOWN_CPAL_ERROR_KINDS) {
            assert_eq!(kind, listed_kind);
            assert_eq!(cpal_error_category(kind).as_str(), expected);
            let bit = cpal_error_bit(kind);
            assert_eq!(bit.count_ones(), 1);
            assert_eq!(bit & UNKNOWN_CPAL_ERROR_BIT, 0);
            assert_eq!(observed_bits & bit, 0, "duplicate bit for {kind:?}");
            observed_bits |= bit;
        }
        let Ok(case_count_u32) = u32::try_from(case_count) else {
            panic!("CPAL error-kind table does not fit a u32");
        };
        assert_eq!(observed_bits.count_ones(), case_count_u32);
    }

    #[test]
    fn asynchronous_errors_cross_the_callback_without_logging_or_allocation() {
        let state = CpalAsyncErrorState::default();
        state.record_output(cpal::ErrorKind::Xrun);
        state.record_output(cpal::ErrorKind::Xrun);
        state.record_output(cpal::ErrorKind::StreamInvalidated);
        state.record_output(cpal::ErrorKind::RealtimeDenied);

        let mut processor = ErrorRecordingProcessor::default();
        state.notify_processor(&mut processor);
        assert_eq!(processor.underruns, 1);
        state.notify_processor(&mut processor);
        assert_eq!(processor.underruns, 1);

        let source = CpalDiagnosticSource::for_test("test output");
        let Some(AudioError::BackendError(summary)) =
            state.take_error(CpalStreamDirection::Output, &source)
        else {
            panic!("asynchronous error summary was not retained off-thread");
        };
        assert!(summary.contains("test output"));
        assert!(summary.contains("xrun"));
        assert!(summary.contains("stream-invalidated"));
        assert!(summary.contains("realtime-denied"));
        assert!(
            state
                .take_error(CpalStreamDirection::Output, &source)
                .is_none()
        );
    }

    #[test]
    fn input_errors_never_schedule_output_processor_notifications() {
        let state = CpalAsyncErrorState::default();
        state.record_input(cpal::ErrorKind::Xrun);
        state.record_input(cpal::ErrorKind::DeviceNotAvailable);

        let mut processor = ErrorRecordingProcessor::default();
        state.notify_processor(&mut processor);
        assert_eq!(processor.underruns, 0);

        let source = CpalDiagnosticSource::for_test("test input");
        let Some(AudioError::BackendError(summary)) =
            state.take_error(CpalStreamDirection::Input, &source)
        else {
            panic!("input diagnostic was not retained off-thread");
        };
        assert!(summary.contains("test input"));
        assert!(summary.contains("xrun"));
        assert!(summary.contains("device-not-available"));
    }
}
