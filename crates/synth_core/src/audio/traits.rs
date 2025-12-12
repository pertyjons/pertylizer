//! Traits defining the audio backend abstraction.
//!
//! This module provides the core abstractions that allow the synthesizer
//! to work with different audio backends (cpal, JACK, PortAudio, etc.)
//! without being tied to any specific implementation.

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
    /// Default implementation does nothing.
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
}

/// Trait for an active audio stream.
pub trait AudioStream: Send {
    /// Start the stream.
    fn start(&mut self) -> AudioResult<()>;

    /// Stop the stream.
    fn stop(&mut self) -> AudioResult<()>;

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

    /// Check if a stream is currently running.
    fn is_running(&self) -> bool;

    /// Get the current latency, if a stream is active.
    fn latency(&self) -> Option<std::time::Duration>;
}

impl AudioHost {
    /// Create a new audio host with the given backend.
    pub fn new(backend: Box<dyn AudioBackend>) -> Self {
        Self {
            backend,
            active_stream: None,
        }
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
        // Stop existing stream if any
        if let Some(ref mut stream) = self.active_stream {
            let _ = stream.stop();
        }

        // Create and start new stream
        let mut stream =
            self.backend
                .create_output_stream(device_id, config, Box::new(processor))?;
        stream.start()?;

        self.active_stream = Some(stream);
        // Safe: we just set active_stream to Some above
        #[allow(clippy::unwrap_used)]
        Ok(self.active_stream.as_ref().unwrap().info())
    }

    /// Stop the current stream.
    pub fn stop(&mut self) -> AudioResult<()> {
        if let Some(ref mut stream) = self.active_stream {
            stream.stop()?;
        }
        Ok(())
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
        // Stop existing stream if any
        if let Some(ref mut stream) = self.active_stream {
            let _ = stream.stop();
        }

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
        if let Some(ref mut stream) = self.active_stream {
            stream.stop()?;
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.active_stream.as_ref().is_some_and(|s| s.is_running())
    }

    fn latency(&self) -> Option<std::time::Duration> {
        self.active_stream.as_ref().map(|s| s.latency())
    }
}
