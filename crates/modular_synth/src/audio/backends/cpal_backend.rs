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

use crate::audio::traits::{AudioBackend, AudioProcessor, AudioStream};
use crate::audio::types::*;

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
                        vec![SampleRate::default()],
                        BufferSize::SMALL,
                        BufferSize::VERY_LARGE,
                    )
                } else {
                    let channels = ChannelCount::from(configs[0].channels());
                    let rates: Vec<SampleRate> = configs
                        .iter()
                        .flat_map(|c| {
                            let min = c.min_sample_rate();
                            let max = c.max_sample_rate();
                            [44100, 48000, 96000]
                                .into_iter()
                                .filter(move |&r| r >= min && r <= max)
                                .map(SampleRate)
                        })
                        .collect();
                    (
                        channels,
                        if rates.is_empty() {
                            vec![SampleRate::default()]
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
                    vec![SampleRate::default()],
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
}

impl Default for CpalBackend {
    /// # Panics
    /// Panics if CPAL backend cannot be initialized.
    #[allow(clippy::expect_used)]
    fn default() -> Self {
        Self::new().expect("Failed to create CPAL backend")
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
        let output_devices = self
            .host
            .output_devices()
            .map_err(|e| AudioError::BackendError(format!("Failed to enumerate devices: {e}")))?;

        let mut devices = Vec::new();
        for device in output_devices {
            if let Ok(info) = self.device_to_info(&device, DeviceType::Output) {
                devices.push(info);
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
}

/// Active CPAL audio stream.
struct CpalStream {
    stream: Stream,
    info: StreamInfo,
    running: Arc<AtomicBool>,
    position: Arc<AtomicU64>,
}

impl CpalStream {
    fn new(
        device: Device,
        config: &StreamConfig,
        mut processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<Self> {
        let channels = config.channels.count();

        // Build cpal config
        let cpal_config = CpalStreamConfig {
            channels,
            sample_rate: config.sample_rate.0,
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size.0),
        };

        // Get actual latency
        let output_latency = device
            .default_output_config()
            .ok()
            .map(|c| {
                let buffer_frames = config.buffer_size.0 as f64;
                let sample_rate = c.sample_rate() as f64;
                Duration::from_secs_f64(buffer_frames / sample_rate)
            })
            .unwrap_or(Duration::from_millis(10));

        let info = StreamInfo {
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
            channels: config.channels,
            output_latency,
            input_latency: None,
        };

        let running = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicU64::new(0));

        // Clone for the callback
        let running_clone = Arc::clone(&running);
        let position_clone = Arc::clone(&position);
        let sample_rate = config.sample_rate;
        let start_time = Instant::now();

        // Notify processor of stream parameters
        processor.on_stream_start(&info);

        // Build the stream
        let stream = device
            .build_output_stream(
                &cpal_config,
                move |data: &mut [f32], _output_info: &cpal::OutputCallbackInfo| {
                    if !running_clone.load(Ordering::Relaxed) {
                        // Fill with silence when not running
                        data.fill(0.0);
                        return;
                    }

                    let frames = data.len() / channels as usize;
                    let current_position = position_clone.load(Ordering::Relaxed);

                    let context = AudioCallbackContext {
                        sample_rate,
                        frames,
                        channels,
                        stream_time: start_time.elapsed().as_secs_f64(),
                        sample_position: current_position,
                        output_latency: output_latency.as_secs_f32(),
                    };

                    processor.process(data, &context);

                    position_clone.fetch_add(frames as u64, Ordering::Relaxed);
                },
                move |err| {
                    eprintln!("Audio stream error: {err}");
                },
                None, // No timeout, blocking mode
            )
            .map_err(|e| AudioError::StreamCreationFailed(format!("{e}")))?;

        Ok(Self {
            stream,
            info,
            running,
            position,
        })
    }
}

impl AudioStream for CpalStream {
    fn start(&mut self) -> AudioResult<()> {
        if self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamAlreadyRunning);
        }

        self.stream
            .play()
            .map_err(|e| AudioError::BackendError(format!("Failed to start stream: {e}")))?;

        self.running.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamNotRunning);
        }

        self.running.store(false, Ordering::Relaxed);

        self.stream
            .pause()
            .map_err(|e| AudioError::BackendError(format!("Failed to stop stream: {e}")))?;

        Ok(())
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

// SAFETY: CpalStream is safe to send between threads because:
// 1. The `stream` field (cpal::Stream) is managed by cpal and its audio callback
//    runs on a separate audio thread - we only control start/pause from here.
// 2. The `info` field (StreamInfo) contains only Copy/Send types.
// 3. The `running` and `position` fields are Arc<Atomic*> which are Send+Sync.
// 4. All mutable operations on `stream` (play/pause) are &mut self, ensuring
//    exclusive access. The audio callback closure captures its own Arc references.
// 5. This is required because AudioStream trait requires Send for use in
//    multi-threaded GUI applications.
unsafe impl Send for CpalStream {}
