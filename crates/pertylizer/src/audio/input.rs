//! Audio input manager — handles monitoring, metering, and recording.
//!
//! Uses dual SPSC ring buffers: one for the engine (low-latency passthrough)
//! and one for the GUI (metering + recording).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Split};

use crate::audio::traits::{AudioHostTrait, AudioStream};
use crate::audio::types::*;

/// Ring buffer size for GUI consumer (large, ~65536 samples for ~1.4s at 48 kHz).
const GUI_RING_SIZE: usize = 65536;

/// Ring buffer size for engine consumer (small, ~2048 samples for low-latency).
const ENGINE_RING_SIZE: usize = 2048;

/// Recording state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    /// No input stream active.
    Idle,
    /// Input stream active, audio is being monitored but not recorded.
    Monitoring,
    /// Actively capturing to the record buffer.
    Recording,
}

/// Manages audio input: monitoring, metering, and recording.
pub struct AudioInputManager {
    /// GUI-side ring buffer consumer (large buffer).
    gui_consumer: Option<ringbuf::HeapCons<f32>>,
    /// Engine-side consumer (sent to SynthEngine for live passthrough).
    engine_consumer: Option<ringbuf::HeapCons<f32>>,
    /// Current input stream.
    stream: Option<Box<dyn AudioStream>>,
    /// Current state.
    state: InputState,
    /// Accumulated recording buffer.
    record_buffer: Vec<f32>,
    /// Peak level for metering (f32 stored as u32 bits, atomic for GUI reads).
    peak_level: Arc<AtomicU32>,
    /// Number of channels in the input stream.
    channels: u16,
    /// Sample rate of the input stream.
    sample_rate: SampleRate,
    /// Temporary read buffer to avoid per-frame allocation.
    read_buf: Vec<f32>,
}

impl AudioInputManager {
    /// Create a new input manager (no stream active).
    pub fn new() -> Self {
        Self {
            gui_consumer: None,
            engine_consumer: None,
            stream: None,
            state: InputState::Idle,
            record_buffer: Vec::new(),
            peak_level: Arc::new(AtomicU32::new(0)),
            channels: 2,
            sample_rate: SampleRate::DVD_QUALITY,
            read_buf: Vec::with_capacity(GUI_RING_SIZE),
        }
    }

    /// Start monitoring from the given input device.
    ///
    /// Creates the input stream and ring buffers.
    pub fn start_monitoring(
        &mut self,
        host: &dyn AudioHostTrait,
        device_id: Option<&str>,
        config: &StreamConfig,
    ) -> AudioResult<()> {
        // Stop any existing stream
        self.stop_monitoring();

        // Create dual ring buffers
        let engine_rb = HeapRb::<f32>::new(ENGINE_RING_SIZE);
        let (engine_prod, engine_cons) = engine_rb.split();

        let gui_rb = HeapRb::<f32>::new(GUI_RING_SIZE);
        let (gui_prod, gui_cons) = gui_rb.split();

        // Create and start input stream
        let mut stream = host.create_input_stream(device_id, config, engine_prod, gui_prod)?;
        stream.start()?;

        self.gui_consumer = Some(gui_cons);
        self.engine_consumer = Some(engine_cons);
        self.stream = Some(stream);
        self.state = InputState::Monitoring;
        self.channels = config.channels.count();
        self.sample_rate = config.sample_rate;

        Ok(())
    }

    /// Stop monitoring and release the input stream.
    pub fn stop_monitoring(&mut self) {
        if let Some(ref mut stream) = self.stream {
            let _ = stream.stop();
        }
        self.stream = None;
        self.gui_consumer = None;
        self.engine_consumer = None;
        self.state = InputState::Idle;
    }

    /// Start recording (must be monitoring first).
    pub fn start_recording(&mut self) {
        if self.state == InputState::Monitoring {
            // Pre-allocate for ~30 seconds of stereo audio
            let capacity = self.sample_rate.0 as usize * self.channels as usize * 30;
            self.record_buffer = Vec::with_capacity(capacity);
            self.state = InputState::Recording;
        }
    }

    /// Stop recording and return the captured audio data.
    ///
    /// Returns `None` if not recording.
    pub fn stop_recording(&mut self) -> Option<Vec<f32>> {
        if self.state != InputState::Recording {
            return None;
        }
        self.state = InputState::Monitoring;
        Some(std::mem::take(&mut self.record_buffer))
    }

    /// Drain available samples from the GUI ring buffer.
    ///
    /// Call this at ~60fps from the GUI thread. Updates peak level
    /// and appends to record buffer if recording.
    pub fn drain_gui_buffer(&mut self) {
        let Some(consumer) = &mut self.gui_consumer else {
            return;
        };

        self.read_buf.clear();
        while let Some(sample) = consumer.try_pop() {
            self.read_buf.push(sample);
        }

        if self.read_buf.is_empty() {
            return;
        }

        // Compute peak level
        let peak = self
            .read_buf
            .iter()
            .fold(0.0_f32, |acc, &s| acc.max(s.abs()));
        self.peak_level.store(peak.to_bits(), Ordering::Relaxed);

        // Append to record buffer if recording
        if self.state == InputState::Recording {
            self.record_buffer.extend_from_slice(&self.read_buf);
        }
    }

    /// Get the current peak level (0.0 to 1.0+).
    pub fn peak_level(&self) -> f32 {
        f32::from_bits(self.peak_level.load(Ordering::Relaxed))
    }

    /// Get the current state.
    pub fn state(&self) -> InputState {
        self.state
    }

    /// Get the number of recorded frames so far.
    pub fn recorded_frames(&self) -> usize {
        if self.channels == 0 {
            return 0;
        }
        self.record_buffer.len() / self.channels as usize
    }

    /// Get the recording duration in seconds.
    pub fn recorded_seconds(&self) -> f64 {
        if self.sample_rate.0 == 0 {
            return 0.0;
        }
        self.recorded_frames() as f64 / f64::from(self.sample_rate.0)
    }

    /// Get the number of channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Get the sample rate.
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Check if the input stream is running.
    pub fn is_active(&self) -> bool {
        self.stream.as_ref().is_some_and(|s| s.is_running())
    }
}

impl Default for AudioInputManager {
    fn default() -> Self {
        Self::new()
    }
}
