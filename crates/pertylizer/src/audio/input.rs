//! Audio input manager — handles monitoring, metering, and recording.
//!
//! Uses dual SPSC ring buffers: one for the engine (low-latency passthrough)
//! and one for the GUI (metering + recording).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Split};

use crate::audio::traits::{AudioHostTrait, AudioStream};
use crate::audio::types::*;

/// Ring buffer size for GUI consumer (~131072 stereo frames, 2.7 s at 48 kHz).
/// Extra large to tolerate GUI lag spikes without dropping recorded audio.
const GUI_RING_FRAMES: usize = 131_072;

/// Ring buffer size for engine consumer (8192 stereo frames).
/// Must be large enough that even at buffer sizes of 2048+ frames the cpal callback
/// never overflows before the engine drains.
const ENGINE_RING_FRAMES: usize = 8_192;

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
    gui_consumer: Option<ringbuf::HeapCons<synth_core::StereoSample>>,
    /// Engine-side consumer (sent to SynthEngine for live passthrough).
    engine_consumer: Option<ringbuf::HeapCons<synth_core::StereoSample>>,
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
    sample_rate: DeviceSampleRate,
    /// Temporary read buffer to avoid per-frame allocation.
    read_buf: Vec<synth_core::StereoSample>,
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
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            read_buf: Vec::with_capacity(GUI_RING_FRAMES),
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
        let engine_rb = HeapRb::<synth_core::StereoSample>::new(ENGINE_RING_FRAMES);
        let (engine_prod, engine_cons) = engine_rb.split();

        let gui_rb = HeapRb::<synth_core::StereoSample>::new(GUI_RING_FRAMES);
        let (gui_prod, gui_cons) = gui_rb.split();

        // Create and start input stream
        let mut stream = host.create_input_stream(device_id, config, engine_prod, gui_prod)?;
        stream.start()?;

        self.gui_consumer = Some(gui_cons);
        self.engine_consumer = Some(engine_cons);
        self.stream = Some(stream);
        self.state = InputState::Monitoring;
        self.channels = config.channels.count().clamp(1, 2);
        self.sample_rate = config.sample_rate;

        Ok(())
    }

    /// Take the engine-side ring buffer consumer (to send to SynthEngine).
    /// Returns `None` if already taken or not monitoring.
    pub fn take_engine_consumer(
        &mut self,
    ) -> Option<(
        ringbuf::HeapCons<synth_core::StereoSample>,
        DeviceSampleRate,
    )> {
        self.engine_consumer
            .take()
            .map(|consumer| (consumer, self.sample_rate))
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
            let capacity = self.sample_rate.as_u32() as usize * self.channels as usize * 30;
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

        let avail = consumer.occupied_len();
        self.read_buf.resize(avail, synth_core::StereoSample::ZERO);
        let popped = consumer.pop_slice(&mut self.read_buf);
        self.read_buf.truncate(popped);

        if self.read_buf.is_empty() {
            return;
        }

        // Compute peak level
        let peak = self.read_buf.iter().fold(0.0_f32, |acc, frame| {
            acc.max(frame.left.abs()).max(frame.right.abs())
        });
        self.peak_level.store(peak.to_bits(), Ordering::Relaxed);

        // Append to record buffer if recording
        if self.state == InputState::Recording {
            for frame in &self.read_buf {
                self.record_buffer.push(frame.left);
                if self.channels == 2 {
                    self.record_buffer.push(frame.right);
                }
            }
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
        if self.sample_rate.as_u32() == 0 {
            return 0.0;
        }
        self.recorded_frames() as f64 / f64::from(self.sample_rate.as_u32())
    }

    /// Get the number of channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Get the sample rate.
    pub fn sample_rate(&self) -> DeviceSampleRate {
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
