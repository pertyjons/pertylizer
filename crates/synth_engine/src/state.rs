//! Shared state between UI and audio threads.
//!
//! This module provides thread-safe primitives for sharing data
//! between the real-time audio thread and the UI thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32 as StdAtomicU32, AtomicU64, AtomicUsize, Ordering};

use parking_lot::RwLock;

use synth_core::{Amplitude, Bpm};
use synth_sequencer::Tick;

use crate::instrument::InstrumentId;
use crate::recording::RecordingState;
use crate::shared_state::{InstrumentSnapshot, ReturnBusSnapshot, SharedGraphState};
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
    pub fn update_peak(&self, left: Amplitude, right: Amplitude) {
        // Peak hold with decay would go here
        self.peak_left.store(left.as_f32());
        self.peak_right.store(right.as_f32());
    }

    /// Update RMS values (called from audio thread).
    pub fn update_rms(&self, left: Amplitude, right: Amplitude) {
        self.rms_left.store(left.as_f32());
        self.rms_right.store(right.as_f32());
    }

    /// Get peak values (called from UI thread).
    pub fn get_peak(&self) -> (Amplitude, Amplitude) {
        (
            Amplitude::new(self.peak_left.load()),
            Amplitude::new(self.peak_right.load()),
        )
    }

    /// Get RMS values (called from UI thread).
    pub fn get_rms(&self) -> (Amplitude, Amplitude) {
        (
            Amplitude::new(self.rms_left.load()),
            Amplitude::new(self.rms_right.load()),
        )
    }
}

/// Maximum number of per-channel / per-return meter slots published to the GUI.
/// Channels (instruments) and return busses beyond this cap simply aren't
/// metered — the audio path is unaffected.
pub const MAX_METER_SLOTS: usize = 128;

/// One published meter slot: a key (an `InstrumentId` or `ReturnBusId` as `u64`)
/// and that channel's post-fader peak level for the latest processed block.
#[derive(Debug)]
struct MeterSlot {
    key: AtomicU64,
    peak: AtomicF32,
}

impl MeterSlot {
    fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            peak: AtomicF32::new(0.0),
        }
    }
}

/// Lock-free bank of per-channel post-fader peak meters.
///
/// The audio thread publishes one slot per channel (or return bus) in processing
/// order each block and records the live count; the GUI reads slots by key each
/// frame. Fixed-size + atomic, so neither side allocates or takes a lock — the
/// same discipline as [`MeterState`] for the master meters, extended to a
/// dynamic, keyed set.
#[derive(Debug)]
pub struct ChannelMeterBank {
    slots: [MeterSlot; MAX_METER_SLOTS],
    count: AtomicUsize,
}

impl ChannelMeterBank {
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| MeterSlot::new()),
            count: AtomicUsize::new(0),
        }
    }

    /// Publish one channel's peak at `index` (audio thread). Indices beyond the
    /// cap are silently dropped.
    pub fn publish(&self, index: usize, key: u64, peak: f32) {
        if let Some(slot) = self.slots.get(index) {
            slot.key.store(key, Ordering::Relaxed);
            slot.peak.store(peak);
        }
    }

    /// Record how many slots are live this block (audio thread), clamped to the
    /// cap. Call after publishing all slots.
    pub fn set_count(&self, count: usize) {
        self.count
            .store(count.min(MAX_METER_SLOTS), Ordering::Relaxed);
    }

    /// Read the post-fader peak for `key`, or `0.0` if it isn't currently metered
    /// (GUI thread).
    #[must_use]
    pub fn peak_for(&self, key: u64) -> f32 {
        let n = self.count.load(Ordering::Relaxed).min(MAX_METER_SLOTS);
        for slot in self.slots.iter().take(n) {
            if slot.key.load(Ordering::Relaxed) == key {
                return slot.peak.load();
            }
        }
        0.0
    }
}

impl Default for ChannelMeterBank {
    fn default() -> Self {
        Self::new()
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
    /// Recording state: 0=off, 1=armed, 2=count_in, 3=capturing.
    pub recording: StdAtomicU32,
    /// Metronome state: 0=off, 1=on.
    pub metronome: StdAtomicU32,
    /// Transport loop region: 1 = enabled, 0 = disabled.
    /// Mirrors `SequencerEngine.{looping, loop_start, loop_end}` so the GUI
    /// and MCP can observe the current loop region without touching the
    /// audio thread.
    pub loop_enabled: std::sync::atomic::AtomicBool,
    /// Transport loop start in sequencer ticks.
    pub loop_start_ticks: AtomicU64,
    /// Transport loop end in sequencer ticks (exclusive).
    pub loop_end_ticks: AtomicU64,
    /// Orphan-preview pattern id + 1, or 0 = none. Mirrors
    /// `SequencerEngine.preview_pattern` so the GUI can render a playhead
    /// in the piano roll even when the pattern has no arrangement placement.
    pub preview_pattern: AtomicU64,
}

impl TransportState {
    pub fn new() -> Self {
        Self {
            tempo: AtomicF32::new(120.0),
            position_beats: AtomicF64::new(0.0),
            position_samples: AtomicU64::new(0),
            position_ticks: AtomicU64::new(0),
            is_playing: std::sync::atomic::AtomicBool::new(false),
            recording: StdAtomicU32::new(0),
            metronome: StdAtomicU32::new(0),
            loop_enabled: std::sync::atomic::AtomicBool::new(false),
            loop_start_ticks: AtomicU64::new(0),
            loop_end_ticks: AtomicU64::new(0),
            preview_pattern: AtomicU64::new(0),
        }
    }

    /// Set the orphan-preview pattern id (or clear with `None`).
    pub fn set_preview_pattern(&self, pattern: Option<synth_sequencer::PatternId>) {
        let raw = pattern.map_or(0, |p| u64::from(p.0).saturating_add(1));
        self.preview_pattern.store(raw, Ordering::Relaxed);
    }

    /// Get the orphan-preview pattern id, if any.
    pub fn preview_pattern(&self) -> Option<synth_sequencer::PatternId> {
        let raw = self.preview_pattern.load(Ordering::Relaxed);
        if raw == 0 {
            None
        } else {
            #[allow(clippy::cast_possible_truncation)]
            Some(synth_sequencer::PatternId((raw - 1) as u32))
        }
    }

    pub fn set_tempo(&self, bpm: Bpm) {
        self.tempo.store(bpm.as_f32());
    }

    pub fn get_tempo(&self) -> Bpm {
        Bpm::new(self.tempo.load())
    }

    pub fn set_playing(&self, playing: bool) {
        self.is_playing.store(playing, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn advance(&self, samples: u64, sample_rate: synth_core::SampleRate) {
        let old_samples = self.position_samples.fetch_add(samples, Ordering::Relaxed);
        let tempo = self.tempo.load();
        let beats_per_sample = tempo / 60.0 / sample_rate.as_f32();
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

    /// Get the recording state.
    pub fn recording_state(&self) -> RecordingState {
        RecordingState::from_u32(self.recording.load(Ordering::Relaxed))
    }

    /// Set the recording state.
    pub fn set_recording_state(&self, state: RecordingState) {
        self.recording.store(state.as_u32(), Ordering::Relaxed);
    }

    /// Check if recording is armed.
    pub fn is_armed(&self) -> bool {
        self.recording_state() == RecordingState::Armed
    }

    /// Check if actively capturing.
    pub fn is_recording(&self) -> bool {
        self.recording_state() == RecordingState::Capturing
    }

    /// Check if in count-in phase.
    pub fn is_count_in(&self) -> bool {
        self.recording_state() == RecordingState::CountIn
    }

    /// Check if metronome is on.
    pub fn is_metronome_on(&self) -> bool {
        self.metronome.load(Ordering::Relaxed) == 1
    }

    /// Set metronome state.
    pub fn set_metronome(&self, enabled: bool) {
        self.metronome.store(u32::from(enabled), Ordering::Relaxed);
    }

    /// Update the transport loop region. Read by GUI ruler markers and MCP
    /// `get_song_info` via [`loop_state`](Self::loop_state).
    pub fn set_loop_state(&self, start: Tick, end: Tick, enabled: bool) {
        self.loop_start_ticks.store(start.0, Ordering::Relaxed);
        self.loop_end_ticks.store(end.0, Ordering::Relaxed);
        self.loop_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Read the current transport loop region as `(enabled, start, end)`.
    pub fn loop_state(&self) -> (bool, Tick, Tick) {
        (
            self.loop_enabled.load(Ordering::Relaxed),
            Tick(self.loop_start_ticks.load(Ordering::Relaxed)),
            Tick(self.loop_end_ticks.load(Ordering::Relaxed)),
        )
    }
}

impl Default for TransportState {
    fn default() -> Self {
        Self::new()
    }
}

/// Sentinel value indicating no focused instrument (use MIDI channel routing).
/// Uses `u64::MAX` which matches `InstrumentId::MASTER` — a non-real instrument.
pub const NO_FOCUSED_INSTRUMENT: u64 = u64::MAX;

/// Complete engine state shared between threads.
#[derive(Debug)]
pub struct EngineState {
    /// Metering.
    pub meters: MeterState,
    /// Per-channel (per-instrument) post-fader peak meters, keyed by
    /// `InstrumentId`. Published by the channel-bus stage each block.
    pub channel_meters: ChannelMeterBank,
    /// Per-return-bus post-fader peak meters, keyed by `ReturnBusId`.
    pub return_meters: ChannelMeterBank,
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
    /// Focused instrument for keyboard input (stores `InstrumentId` as u64).
    /// When set (not NO_FOCUSED_INSTRUMENT), keyboard input goes only to this instrument.
    /// When NO_FOCUSED_INSTRUMENT, traditional MIDI channel routing is used.
    pub focused_instrument: AtomicU64,
    /// Master output waveform buffer for oscilloscope display.
    pub master_scope: VisualizationBuffer,
    /// Shared graph state for MCP and multi-GUI access.
    pub shared_graph: SharedGraphState,
    /// Number of effects in the focused instrument's effect chain.
    pub effect_count: AtomicU32,
    /// Instrument metadata snapshots for MCP and multi-GUI access.
    pub instrument_snapshots: RwLock<Vec<InstrumentSnapshot>>,
    /// Return-bus effect chains, published for the save path (the fader/name
    /// live in the `Song`; the effect chain is engine-side runtime state).
    pub return_bus_effects: RwLock<Vec<ReturnBusSnapshot>>,
    /// Per-instrument keyboard octave offset, applied by note-rendering paths
    /// that don't go through the GUI keyboard (e.g. MCP `preview_note`).
    /// Set by `apply_patch` from `Patch::settings.octave_offset`. Default 0.
    pub octave_offsets: RwLock<HashMap<InstrumentId, i32>>,
    /// Count of dropped note events (ring buffer overflow).
    pub event_drops: StdAtomicU32,
}

impl EngineState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            meters: MeterState::new(),
            channel_meters: ChannelMeterBank::new(),
            return_meters: ChannelMeterBank::new(),
            transport: TransportState::new(),
            master_volume: AtomicF32::new(1.0),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48000),
            focused_instrument: AtomicU64::new(NO_FOCUSED_INSTRUMENT),
            master_scope: VisualizationBuffer::new(4096),
            shared_graph: SharedGraphState::new(),
            effect_count: AtomicU32::new(0),
            instrument_snapshots: RwLock::new(Vec::new()),
            return_bus_effects: RwLock::new(Vec::new()),
            octave_offsets: RwLock::new(HashMap::new()),
            event_drops: StdAtomicU32::new(0),
        })
    }

    /// Set the focused instrument for keyboard input.
    /// Pass None to use traditional MIDI channel routing.
    pub fn set_focused_instrument(&self, instrument_id: Option<crate::instrument::InstrumentId>) {
        let value = instrument_id.map_or(NO_FOCUSED_INSTRUMENT, |id| id.as_u64());
        self.focused_instrument.store(value, Ordering::Relaxed);
    }

    /// Get the focused instrument ID, or None if using MIDI channel routing.
    pub fn get_focused_instrument(&self) -> Option<crate::instrument::InstrumentId> {
        let value = self.focused_instrument.load(Ordering::Relaxed);
        if value == NO_FOCUSED_INSTRUMENT {
            None
        } else {
            Some(crate::instrument::InstrumentId::new(value))
        }
    }

    /// Set the keyboard octave offset for an instrument.
    ///
    /// Used by non-GUI note-rendering paths (e.g. MCP `preview_note`) so the
    /// rendered audio matches what the user would hear when pressing the
    /// corresponding key on the on-screen keyboard. Updated by `apply_patch`.
    pub fn set_octave_offset(&self, instrument_id: InstrumentId, offset: i32) {
        self.octave_offsets.write().insert(instrument_id, offset);
    }

    /// Get the keyboard octave offset for an instrument (0 if unset).
    pub fn get_octave_offset(&self, instrument_id: InstrumentId) -> i32 {
        self.octave_offsets
            .read()
            .get(&instrument_id)
            .copied()
            .unwrap_or(0)
    }

    /// Clear the stored octave offset for an instrument (use when the
    /// instrument is removed).
    pub fn clear_octave_offset(&self, instrument_id: InstrumentId) {
        self.octave_offsets.write().remove(&instrument_id);
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            meters: MeterState::new(),
            channel_meters: ChannelMeterBank::new(),
            return_meters: ChannelMeterBank::new(),
            transport: TransportState::new(),
            master_volume: AtomicF32::new(1.0),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48000),
            focused_instrument: AtomicU64::new(NO_FOCUSED_INSTRUMENT),
            master_scope: VisualizationBuffer::new(4096),
            shared_graph: SharedGraphState::new(),
            effect_count: AtomicU32::new(0),
            instrument_snapshots: RwLock::new(Vec::new()),
            return_bus_effects: RwLock::new(Vec::new()),
            octave_offsets: RwLock::new(HashMap::new()),
            event_drops: StdAtomicU32::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_meter_bank_publishes_and_reads_by_key() {
        let bank = ChannelMeterBank::new();
        // Nothing published yet → every key reads 0.
        assert_eq!(bank.peak_for(7), 0.0);

        bank.publish(0, 7, 0.5);
        bank.publish(1, 42, 0.25);
        bank.set_count(2);

        assert_eq!(bank.peak_for(7), 0.5);
        assert_eq!(bank.peak_for(42), 0.25);
        // A key that isn't in the live range reads 0.
        assert_eq!(bank.peak_for(99), 0.0);
    }

    #[test]
    fn channel_meter_bank_count_bounds_visible_slots() {
        let bank = ChannelMeterBank::new();
        bank.publish(0, 1, 0.9);
        bank.publish(1, 2, 0.8);
        // Only one slot is declared live, so slot 1's key is invisible.
        bank.set_count(1);
        assert_eq!(bank.peak_for(1), 0.9);
        assert_eq!(bank.peak_for(2), 0.0);
    }

    #[test]
    fn channel_meter_bank_ignores_out_of_range_index() {
        let bank = ChannelMeterBank::new();
        // Out-of-range publish must not panic and must not become visible.
        bank.publish(MAX_METER_SLOTS, 5, 1.0);
        bank.set_count(MAX_METER_SLOTS);
        assert_eq!(bank.peak_for(5), 0.0);
    }
}
