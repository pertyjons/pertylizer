//! Null audio backend for testing.
//!
//! This backend doesn't produce any audio but can be used for:
//! - Unit testing without audio hardware
//! - Benchmarking audio processing code
//! - Offline rendering

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::traits::{AudioBackend, AudioProcessor, AudioStream};
use crate::audio::types::*;

/// Null audio backend for testing.
pub struct NullBackend;

impl NullBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for NullBackend {
    fn name(&self) -> &str {
        "null"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn devices(&self) -> AudioResult<Vec<DeviceInfo>> {
        Ok(vec![self.default_output_device()?])
    }

    fn default_output_device(&self) -> AudioResult<DeviceInfo> {
        Ok(DeviceInfo {
            id: "null".to_string(),
            name: "Null Device".to_string(),
            device_type: DeviceType::Output,
            supported_sample_rates: vec![
                DeviceSampleRate::CD_QUALITY,
                DeviceSampleRate::DVD_QUALITY,
                DeviceSampleRate::STUDIO_QUALITY,
            ],
            min_buffer_size: BufferSize::TINY,
            max_buffer_size: BufferSize::VERY_LARGE,
            input_channels: ChannelCount::Stereo,
            output_channels: ChannelCount::Stereo,
        })
    }

    fn default_input_device(&self) -> AudioResult<DeviceInfo> {
        self.default_output_device()
    }

    fn create_output_stream(
        &self,
        _device_id: Option<&str>,
        config: &StreamConfig,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<Box<dyn AudioStream>> {
        Ok(Box::new(NullStream::new(config.clone(), processor)))
    }
}

/// Null audio stream that simulates real-time audio processing.
struct NullStream {
    config: StreamConfig,
    processor: Option<Box<dyn AudioProcessor>>,
    info: StreamInfo,
    running: Arc<AtomicBool>,
    position: Arc<AtomicU64>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl NullStream {
    fn new(config: StreamConfig, mut processor: Box<dyn AudioProcessor>) -> Self {
        let info = StreamInfo {
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
            channels: config.channels,
            output_latency: config.buffer_size.latency_at(config.sample_rate),
            input_latency: None,
        };

        processor.on_stream_start(&info);

        Self {
            config,
            processor: Some(processor),
            info,
            running: Arc::new(AtomicBool::new(false)),
            position: Arc::new(AtomicU64::new(0)),
            thread_handle: None,
        }
    }
}

impl AudioStream for NullStream {
    /// Start the null audio processing thread.
    ///
    /// **Limitation:** The processor is moved into the spawned thread and not
    /// returned on [`stop`](Self::stop). Calling `start` a second time after
    /// `stop` will fail with [`AudioError::StreamNotRunning`] because the
    /// processor has been consumed. This is acceptable for a test-only backend;
    /// create a new `NullStream` if you need to restart.
    fn start(&mut self) -> AudioResult<()> {
        if self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamAlreadyRunning);
        }

        self.running.store(true, Ordering::Relaxed);

        let running = Arc::clone(&self.running);
        let position = Arc::clone(&self.position);
        let sample_rate = self.config.sample_rate;
        let buffer_size = self.config.buffer_size.as_u32() as usize;
        let channels = self.config.channels.count() as usize;
        let output_latency = self.info.output_latency;

        // Take ownership of processor for the thread
        let Some(mut processor) = self.processor.take() else {
            return Err(AudioError::StreamNotRunning);
        };

        let handle = thread::Builder::new()
            .name("null-audio".to_string())
            .spawn(move || {
                let mut buffer = vec![0.0f32; buffer_size * channels];
                let frame_duration =
                    Duration::from_secs_f64(buffer_size as f64 / sample_rate.as_f64());
                let start_time = Instant::now();

                while running.load(Ordering::Relaxed) {
                    let current_position = position.load(Ordering::Relaxed);

                    let context = AudioCallbackContext {
                        sample_rate,
                        frames: buffer_size,
                        channels: channels as u16,
                        stream_time: start_time.elapsed().as_secs_f64(),
                        sample_position: current_position,
                        output_latency: synth_core::Seconds::new(output_latency.as_secs_f32()),
                    };

                    // Clear buffer and process
                    buffer.fill(0.0);
                    processor.process(&mut buffer, &context);

                    position.fetch_add(buffer_size as u64, Ordering::Relaxed);

                    // Sleep to simulate real-time
                    thread::sleep(frame_duration);
                }

                processor.on_stream_stop();
            })
            .map_err(|e| AudioError::StreamCreationFailed(e.to_string()))?;

        self.thread_handle = Some(handle);
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        if !self.running.load(Ordering::Relaxed) {
            return Err(AudioError::StreamNotRunning);
        }

        self.running.store(false, Ordering::Relaxed);

        if let Some(handle) = self.thread_handle.take() {
            handle.join().ok();
        }

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

impl Drop for NullStream {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            handle.join().ok();
        }
    }
}
