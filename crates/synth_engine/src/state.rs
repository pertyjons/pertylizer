//! Shared state between UI and audio threads.
//!
//! This module provides thread-safe primitives for sharing data
//! between the real-time audio thread and the UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32 as StdAtomicU32, AtomicU64, Ordering};

use crate::shared_state::SharedGraphState;
use crate::visualizers::VisualizationBuffer;

/// Atomic float for thread-safe parameter sharing.
#[derive(Debug)]
pub struct AtomicF32 {
    bits: StdAtomicU32,
}

impl AtomicF32 {
    pub const fn new(value: f32) -> Self {
        Self {
            bits: StdAtomicU32::new(value.to_bits()),
        }
    }

    #[inline]
    pub fn load(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn store(&self, value: f32) {
        self.bits.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Atomically swap and return old value.
    #[inline]
    pub fn swap(&self, value: f32) -> f32 {
        f32::from_bits(self.bits.swap(value.to_bits(), Ordering::Relaxed))
    }
}

impl Default for AtomicF32 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Clone for AtomicF32 {
    fn clone(&self) -> Self {
        Self::new(self.load())
    }
}

/// Atomic u32 wrapper for convenience.
#[derive(Debug)]
pub struct AtomicU32 {
    inner: StdAtomicU32,
}

impl AtomicU32 {
    pub const fn new(value: u32) -> Self {
        Self {
            inner: StdAtomicU32::new(value),
        }
    }

    #[inline]
    pub fn load(&self) -> u32 {
        self.inner.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn store(&self, value: u32) {
        self.inner.store(value, Ordering::Relaxed);
    }

    #[inline]
    pub fn fetch_add(&self, value: u32) -> u32 {
        self.inner.fetch_add(value, Ordering::Relaxed)
    }
}

impl Default for AtomicU32 {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clone for AtomicU32 {
    fn clone(&self) -> Self {
        Self::new(self.load())
    }
}

/// Atomic f64 for high-precision values like phase.
#[derive(Debug)]
pub struct AtomicF64 {
    bits: AtomicU64,
}

impl AtomicF64 {
    pub const fn new(value: f64) -> Self {
        Self {
            bits: AtomicU64::new(value.to_bits()),
        }
    }

    #[inline]
    pub fn load(&self) -> f64 {
        f64::from_bits(self.bits.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn store(&self, value: f64) {
        self.bits.store(value.to_bits(), Ordering::Relaxed);
    }
}

impl Default for AtomicF64 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Clone for AtomicF64 {
    fn clone(&self) -> Self {
        Self::new(self.load())
    }
}

/// Metering data shared between audio and UI threads.
#[derive(Debug, Default)]
pub struct MeterState {
    /// Peak level, left channel.
    pub peak_left: AtomicF32,
    /// Peak level, right channel.
    pub peak_right: AtomicF32,
    /// RMS level, left channel.
    pub rms_left: AtomicF32,
    /// RMS level, right channel.
    pub rms_right: AtomicF32,
}

impl MeterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update peak values (called from audio thread).
    pub fn update_peak(&self, left: f32, right: f32) {
        // Peak hold with decay would go here
        self.peak_left.store(left);
        self.peak_right.store(right);
    }

    /// Update RMS values (called from audio thread).
    pub fn update_rms(&self, left: f32, right: f32) {
        self.rms_left.store(left);
        self.rms_right.store(right);
    }

    /// Get peak values (called from UI thread).
    pub fn get_peak(&self) -> (f32, f32) {
        (self.peak_left.load(), self.peak_right.load())
    }

    /// Get RMS values (called from UI thread).
    pub fn get_rms(&self) -> (f32, f32) {
        (self.rms_left.load(), self.rms_right.load())
    }
}

/// Transport state shared between threads.
#[derive(Debug)]
pub struct TransportState {
    /// Current tempo in BPM.
    pub tempo: AtomicF32,
    /// Current position in beats.
    pub position_beats: AtomicF64,
    /// Current position in samples.
    pub position_samples: AtomicU64,
    /// Current position in sequencer ticks.
    pub position_ticks: AtomicU64,
    /// Is playing.
    pub is_playing: std::sync::atomic::AtomicBool,
}

impl TransportState {
    pub fn new() -> Self {
        Self {
            tempo: AtomicF32::new(120.0),
            position_beats: AtomicF64::new(0.0),
            position_samples: AtomicU64::new(0),
            position_ticks: AtomicU64::new(0),
            is_playing: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn set_tempo(&self, bpm: f32) {
        self.tempo.store(bpm);
    }

    pub fn get_tempo(&self) -> f32 {
        self.tempo.load()
    }

    pub fn set_playing(&self, playing: bool) {
        self.is_playing.store(playing, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn advance(&self, samples: u64, sample_rate: f32) {
        let old_samples = self.position_samples.fetch_add(samples, Ordering::Relaxed);
        let tempo = self.tempo.load();
        let beats_per_sample = tempo / 60.0 / sample_rate;
        let beats = (old_samples + samples) as f64 * beats_per_sample as f64;
        self.position_beats.store(beats);
    }

    pub fn set_ticks(&self, ticks: u64) {
        self.position_ticks.store(ticks, Ordering::Relaxed);
    }

    pub fn get_ticks(&self) -> u64 {
        self.position_ticks.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.position_samples.store(0, Ordering::Relaxed);
        self.position_beats.store(0.0);
        self.position_ticks.store(0, Ordering::Relaxed);
    }
}

impl Default for TransportState {
    fn default() -> Self {
        Self::new()
    }
}

/// Constant indicating no focused instrument (use MIDI channel routing).
pub const NO_FOCUSED_INSTRUMENT: u32 = u32::MAX;

/// Complete engine state shared between threads.
#[derive(Debug)]
pub struct EngineState {
    /// Metering.
    pub meters: MeterState,
    /// Transport.
    pub transport: TransportState,
    /// Master volume.
    pub master_volume: AtomicF32,
    /// Active voice count.
    pub voice_count: AtomicU32,
    /// CPU usage (0.0 - 1.0).
    pub cpu_usage: AtomicF32,
    /// Sample rate.
    pub sample_rate: AtomicU32,
    /// Focused instrument for keyboard input.
    /// When set (not NO_FOCUSED_INSTRUMENT), keyboard input goes only to this instrument.
    /// When NO_FOCUSED_INSTRUMENT, traditional MIDI channel routing is used.
    pub focused_instrument: AtomicU32,
    /// Master output waveform buffer for oscilloscope display.
    pub master_scope: VisualizationBuffer,
    /// Shared graph state for MCP and multi-GUI access.
    pub shared_graph: SharedGraphState,
}

impl EngineState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            meters: MeterState::new(),
            transport: TransportState::new(),
            master_volume: AtomicF32::new(1.0),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48000),
            focused_instrument: AtomicU32::new(NO_FOCUSED_INSTRUMENT),
            master_scope: VisualizationBuffer::new(4096),
            shared_graph: SharedGraphState::new(),
        })
    }

    /// Set the focused instrument for keyboard input.
    /// Pass None to use traditional MIDI channel routing.
    pub fn set_focused_instrument(&self, instrument_index: Option<u32>) {
        let value = instrument_index.unwrap_or(NO_FOCUSED_INSTRUMENT);
        self.focused_instrument.store(value);
    }

    /// Get the focused instrument index, or None if using MIDI channel routing.
    pub fn get_focused_instrument(&self) -> Option<u32> {
        let value = self.focused_instrument.load();
        if value == NO_FOCUSED_INSTRUMENT {
            None
        } else {
            Some(value)
        }
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            meters: MeterState::new(),
            transport: TransportState::new(),
            master_volume: AtomicF32::new(1.0),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48000),
            focused_instrument: AtomicU32::new(NO_FOCUSED_INSTRUMENT),
            master_scope: VisualizationBuffer::new(4096),
            shared_graph: SharedGraphState::new(),
        }
    }
}
