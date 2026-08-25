//! Traits defining the audio backend abstraction.
//!
//! This module provides the core abstractions that allow the synthesizer
//! to work with different audio backends (cpal, JACK, PortAudio, etc.)
//! without being tied to any specific implementation.

use std::collections::VecDeque;

use ringbuf::HeapProd;

use super::types::*;

/// Trait for processing audio.
///
/// Implement this trait in your synth engine to receive audio callbacks.
/// The callback runs in a real-time audio thread, so implementations must:
/// - Never allocate memory
/// - Never block (no mutexes, no I/O)
/// - Be as fast as possible
pub trait AudioProcessor: Send + 'static {
    /// Process audio output.
    ///
    /// Fill the `output` buffer with audio samples. The buffer is interleaved
    /// for multi-channel audio (L, R, L, R, ...).
    ///
    /// # Arguments
    /// * `output` - Buffer to fill with audio samples
    /// * `context` - Information about the current callback
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext);

    /// Called when a stream error occurs.
    ///
    /// A backend may call this from its real-time data callback. Implementations
    /// must obey the same no-allocation, no-locking, and no-I/O constraints as
    /// [`Self::process`]. A real-time caller must pass an allocation-free
    /// `AudioError` variant; passing an owned-string variant could deallocate on
    /// the audio thread when the value is dropped.
    ///
    /// The default implementation does nothing.
    fn on_error(&mut self, _error: AudioError) {}

    /// Called when the stream is about to start.
    ///
    /// Use this to initialize any state that depends on the actual
    /// stream configuration.
    fn on_stream_start(&mut self, _info: &StreamInfo) {}

    /// Called when the stream has stopped.
    fn on_stream_stop(&mut self) {}
}

/// Trait for audio backends.
///
/// Each audio backend (cpal, JACK, etc.) implements this trait to provide
/// a uniform interface for device enumeration and stream management.
pub trait AudioBackend: Send + Sync {
    /// Get the name of this backend.
    fn name(&self) -> &str;

    /// Check if this backend is available on the current system.
    fn is_available(&self) -> bool;

    /// Get all available audio devices.
    fn devices(&self) -> AudioResult<Vec<DeviceInfo>>;

    /// Get the default output device.
    fn default_output_device(&self) -> AudioResult<DeviceInfo>;

    /// Get the default input device.
    fn default_input_device(&self) -> AudioResult<DeviceInfo>;

    /// Create an output stream with the given configuration.
    ///
    /// # Arguments
    /// * `device_id` - ID of the device to use, or None for default
    /// * `config` - Desired stream configuration
    /// * `processor` - Audio processor to receive callbacks
    fn create_output_stream(
        &self,
        device_id: Option<&str>,
        config: &StreamConfig,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<Box<dyn AudioStream>>;

    /// Start an input stream that writes captured audio into ring buffer producers.
    ///
    /// Two producers are needed because ringbuf is SPSC: one for the engine
    /// (low-latency passthrough) and one for the GUI (metering + recording).
    /// The backend callback pushes each complete stereo frame to both.
    ///
    /// Default implementation returns an error (not all backends support input).
    fn create_input_stream(
        &self,
        _device_id: Option<&str>,
        _config: &StreamConfig,
        _engine_producer: HeapProd<crate::StereoSample>,
        _gui_producer: HeapProd<crate::StereoSample>,
    ) -> AudioResult<Box<dyn AudioStream>> {
        Err(AudioError::BackendError(
            "Input streams not supported by this backend".to_string(),
        ))
    }
}

/// Trait for an active audio stream.
pub trait AudioStream: Send {
    /// Start the stream.
    fn start(&mut self) -> AudioResult<()>;

    /// Stop the stream.
    fn stop(&mut self) -> AudioResult<()>;

    /// Take one coalesced asynchronous backend diagnostic, if present.
    ///
    /// This is separate from [`Self::stop`]: the return value of a lifecycle
    /// operation describes that operation, while errors reported independently
    /// by an active stream are diagnostics for a non-real-time consumer.
    fn take_async_error(&mut self) -> Option<AudioError> {
        None
    }

    /// Check if the stream is currently running.
    fn is_running(&self) -> bool;

    /// Get information about the stream.
    fn info(&self) -> &StreamInfo;

    /// Get the current stream position in samples.
    fn position(&self) -> u64;

    /// Get the estimated output latency.
    fn latency(&self) -> std::time::Duration {
        self.info().output_latency
    }
}

/// A handle to manage the audio host and active streams.
pub struct AudioHost {
    backend: Box<dyn AudioBackend>,
    active_stream: Option<Box<dyn AudioStream>>,
    pending_async_errors: VecDeque<AudioError>,
}

/// Trait for audio host operations.
///
/// This allows the GUI backends to work with any audio host implementation.
pub trait AudioHostTrait: Send {
    /// Get the backend name.
    fn backend_name(&self) -> &str;

    /// Get all available devices.
    fn devices(&self) -> AudioResult<Vec<DeviceInfo>>;

    /// Start an output stream.
    fn start_output(
        &mut self,
        device_id: Option<&str>,
        config: &StreamConfig,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<StreamInfo>;

    /// Stop the current stream.
    fn stop(&mut self) -> AudioResult<()>;

    /// Take one coalesced asynchronous stream diagnostic, if present.
    fn take_async_error(&mut self) -> Option<AudioError> {
        None
    }

    /// Take a finite snapshot of all currently retained stream diagnostics.
    ///
    /// The default supports hosts that retain at most one diagnostic. Hosts
    /// with a queue override this so callers never need an open-ended drain
    /// loop while a backend may still be producing errors.
    fn take_async_errors(&mut self) -> Vec<AudioError> {
        self.take_async_error().into_iter().collect()
    }

    /// Check if a stream is currently running.
    fn is_running(&self) -> bool;

    /// Get the current latency, if a stream is active.
    fn latency(&self) -> Option<std::time::Duration>;

    /// Create an input stream with dual ring buffer producers.
    fn create_input_stream(
        &self,
        device_id: Option<&str>,
        config: &StreamConfig,
        engine_producer: HeapProd<crate::StereoSample>,
        gui_producer: HeapProd<crate::StereoSample>,
    ) -> AudioResult<Box<dyn AudioStream>>;

    /// Get the default input device.
    fn default_input_device(&self) -> AudioResult<DeviceInfo>;
}

impl AudioHost {
    /// Create a new audio host with the given backend.
    pub fn new(backend: Box<dyn AudioBackend>) -> Self {
        Self {
            backend,
            active_stream: None,
            pending_async_errors: VecDeque::new(),
        }
    }

    fn prepare_active_stream_replacement(&mut self) {
        let Some(stream) = self.active_stream.as_mut() else {
            return;
        };
        let stop_result = if stream.is_running() {
            stream.stop()
        } else {
            Ok(())
        };
        let async_error = stream.take_async_error();
        if let Err(error) = stop_result {
            self.pending_async_errors.push_back(retired_stream_error(
                RetiredStreamDiagnostic::StopFailed,
                error,
            ));
        }
        if let Some(error) = async_error {
            self.pending_async_errors.push_back(retired_stream_error(
                RetiredStreamDiagnostic::Asynchronous,
                error,
            ));
        }
    }

    fn stop_active_stream(&mut self) -> AudioResult<()> {
        let Some(stream) = self.active_stream.as_mut() else {
            return Ok(());
        };
        let stop_result = stream.stop();
        let async_error = stream.take_async_error();
        if let Some(error) = async_error {
            self.pending_async_errors.push_back(error);
        }
        stop_result
    }

    /// Get the backend name.
    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    /// Check if the backend is available.
    pub fn is_available(&self) -> bool {
        self.backend.is_available()
    }

    /// Get all available devices.
    pub fn devices(&self) -> AudioResult<Vec<DeviceInfo>> {
        self.backend.devices()
    }

    /// Get the default output device.
    pub fn default_output_device(&self) -> AudioResult<DeviceInfo> {
        self.backend.default_output_device()
    }

    /// Start an output stream.
    ///
    /// Stops any currently running stream first.
    pub fn start_output<P: AudioProcessor>(
        &mut self,
        device_id: Option<&str>,
        config: &StreamConfig,
        processor: P,
    ) -> AudioResult<&StreamInfo> {
        self.prepare_active_stream_replacement();

        // Create and start new stream
        let mut stream =
            self.backend
                .create_output_stream(device_id, config, Box::new(processor))?;
        stream.start()?;

        let stream = self.active_stream.insert(stream);
        Ok(stream.info())
    }

    /// Stop the current stream.
    pub fn stop(&mut self) -> AudioResult<()> {
        self.stop_active_stream()
    }

    /// Take one coalesced asynchronous stream diagnostic, if present.
    pub fn take_async_error(&mut self) -> Option<AudioError> {
        self.pending_async_errors.pop_front().or_else(|| {
            self.active_stream
                .as_mut()
                .and_then(|stream| stream.take_async_error())
        })
    }

    /// Take a finite snapshot of all currently retained stream diagnostics.
    pub fn take_async_errors(&mut self) -> Vec<AudioError> {
        let mut errors: Vec<_> = self.pending_async_errors.drain(..).collect();
        if let Some(error) = self
            .active_stream
            .as_mut()
            .and_then(|stream| stream.take_async_error())
        {
            errors.push(error);
        }
        errors
    }

    /// Check if a stream is currently running.
    pub fn is_running(&self) -> bool {
        self.active_stream.as_ref().is_some_and(|s| s.is_running())
    }

    /// Get the current stream info, if a stream is active.
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.active_stream.as_ref().map(|s| s.info())
    }

    /// Get the current latency, if a stream is active.
    pub fn latency(&self) -> Option<std::time::Duration> {
        self.active_stream.as_ref().map(|s| s.latency())
    }

    /// Get the current stream position in samples.
    pub fn position(&self) -> Option<u64> {
        self.active_stream.as_ref().map(|s| s.position())
    }
}

impl AudioHostTrait for AudioHost {
    fn backend_name(&self) -> &str {
        self.backend.name()
    }

    fn devices(&self) -> AudioResult<Vec<DeviceInfo>> {
        self.backend.devices()
    }

    fn start_output(
        &mut self,
        device_id: Option<&str>,
        config: &StreamConfig,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<StreamInfo> {
        self.prepare_active_stream_replacement();

        // Create and start new stream
        let mut stream = self
            .backend
            .create_output_stream(device_id, config, processor)?;
        stream.start()?;

        let info = stream.info().clone();
        self.active_stream = Some(stream);
        Ok(info)
    }

    fn stop(&mut self) -> AudioResult<()> {
        self.stop_active_stream()
    }

    fn take_async_error(&mut self) -> Option<AudioError> {
        Self::take_async_error(self)
    }

    fn take_async_errors(&mut self) -> Vec<AudioError> {
        Self::take_async_errors(self)
    }

    fn is_running(&self) -> bool {
        self.active_stream.as_ref().is_some_and(|s| s.is_running())
    }

    fn latency(&self) -> Option<std::time::Duration> {
        self.active_stream.as_ref().map(|s| s.latency())
    }

    fn create_input_stream(
        &self,
        device_id: Option<&str>,
        config: &StreamConfig,
        engine_producer: HeapProd<crate::StereoSample>,
        gui_producer: HeapProd<crate::StereoSample>,
    ) -> AudioResult<Box<dyn AudioStream>> {
        self.backend
            .create_input_stream(device_id, config, engine_producer, gui_producer)
    }

    fn default_input_device(&self) -> AudioResult<DeviceInfo> {
        self.backend.default_input_device()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetiredStreamDiagnostic {
    StopFailed,
    Asynchronous,
}

impl RetiredStreamDiagnostic {
    const fn as_str(self) -> &'static str {
        match self {
            Self::StopFailed => "stop failed",
            Self::Asynchronous => "asynchronous diagnostic",
        }
    }
}

fn retired_stream_error(context: RetiredStreamDiagnostic, error: AudioError) -> AudioError {
    AudioError::BackendError(format!(
        "retired audio stream {}: {error}",
        context.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedBackend;

    impl AudioBackend for UnusedBackend {
        fn name(&self) -> &str {
            "unused"
        }

        fn is_available(&self) -> bool {
            false
        }

        fn devices(&self) -> AudioResult<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        fn default_output_device(&self) -> AudioResult<DeviceInfo> {
            Err(AudioError::NoDefaultDevice)
        }

        fn default_input_device(&self) -> AudioResult<DeviceInfo> {
            Err(AudioError::NoDefaultDevice)
        }

        fn create_output_stream(
            &self,
            _device_id: Option<&str>,
            config: &StreamConfig,
            _processor: Box<dyn AudioProcessor>,
        ) -> AudioResult<Box<dyn AudioStream>> {
            Ok(Box::new(DiagnosticStream {
                running: false,
                stop_error: false,
                diagnostic: None,
                info: StreamInfo {
                    sample_rate: config.sample_rate,
                    buffer_size: config.buffer_size,
                    channels: config.channels,
                    output_latency: std::time::Duration::ZERO,
                    input_latency: None,
                },
            }))
        }
    }

    struct NoopProcessor;

    impl AudioProcessor for NoopProcessor {
        fn process(&mut self, _output: &mut [f32], _context: &AudioCallbackContext) {}
    }

    struct DiagnosticStream {
        running: bool,
        stop_error: bool,
        diagnostic: Option<AudioError>,
        info: StreamInfo,
    }

    impl AudioStream for DiagnosticStream {
        fn start(&mut self) -> AudioResult<()> {
            self.running = true;
            Ok(())
        }

        fn stop(&mut self) -> AudioResult<()> {
            if self.stop_error {
                return Err(AudioError::BackendError("pause failed".to_string()));
            }
            self.running = false;
            Ok(())
        }

        fn take_async_error(&mut self) -> Option<AudioError> {
            self.diagnostic.take()
        }

        fn is_running(&self) -> bool {
            self.running
        }

        fn info(&self) -> &StreamInfo {
            &self.info
        }

        fn position(&self) -> u64 {
            0
        }
    }

    #[test]
    fn asynchronous_diagnostic_does_not_change_successful_stop_result() {
        let stream = DiagnosticStream {
            running: true,
            stop_error: false,
            diagnostic: Some(AudioError::BackendError("xrun".to_string())),
            info: StreamInfo {
                sample_rate: DeviceSampleRate::DVD_QUALITY,
                buffer_size: BufferSize::MEDIUM,
                channels: ChannelCount::Stereo,
                output_latency: std::time::Duration::ZERO,
                input_latency: None,
            },
        };
        let mut host = AudioHost {
            backend: Box::new(UnusedBackend),
            active_stream: Some(Box::new(stream)),
            pending_async_errors: VecDeque::new(),
        };

        assert!(host.stop().is_ok());
        let Some(AudioError::BackendError(message)) = host.take_async_error() else {
            panic!("the asynchronous diagnostic was not retained");
        };
        assert_eq!(message, "xrun");
        assert!(host.take_async_error().is_none());
    }

    #[test]
    fn replacement_continues_after_retaining_the_retired_streams_stop_failure() {
        let stream = DiagnosticStream {
            running: true,
            stop_error: true,
            diagnostic: Some(AudioError::DeviceDisconnected),
            info: StreamInfo {
                sample_rate: DeviceSampleRate::DVD_QUALITY,
                buffer_size: BufferSize::MEDIUM,
                channels: ChannelCount::Stereo,
                output_latency: std::time::Duration::ZERO,
                input_latency: None,
            },
        };
        let mut host = AudioHost {
            backend: Box::new(UnusedBackend),
            active_stream: Some(Box::new(stream)),
            pending_async_errors: VecDeque::new(),
        };
        let config = StreamConfig {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            buffer_size: BufferSize::MEDIUM,
            channels: ChannelCount::Stereo,
        };

        assert!(host.start_output(None, &config, NoopProcessor).is_ok());
        let errors = host.take_async_errors();
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0]
                .to_string()
                .contains("retired audio stream stop failed")
        );
        assert!(
            errors[1]
                .to_string()
                .contains("retired audio stream asynchronous diagnostic")
        );
    }
}
