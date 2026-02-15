//! Per-voice spatial processing for AWE Fas 3.
//!
//! Maps each active voice to a position in the room based on its MIDI note,
//! then processes individual early reflections and spatialisation per voice.

use serde::{Deserialize, Serialize};

use crate::early_reflections::EarlyReflections;
use crate::spatializer::Spatializer;

/// Maximum number of simultaneous spatial voice slots.
pub const MAX_SPATIAL_VOICES: usize = 16;

/// Pre-allocated mono buffer size per voice (samples).
const VOICE_BUFFER_SIZE: usize = 4096;

/// Delay line size for per-voice early reflections (16K samples, ~170ms at 96kHz).
const PER_VOICE_MAX_DELAY: usize = 16_384;

/// How MIDI notes map to positions in the room.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotePositionMapping {
    /// All voices use the global source position (spatial off per voice).
    #[default]
    Off,
    /// Note 0 at left wall, note 127 at right wall (X axis).
    LinearX,
    /// Note 0 at front wall, note 127 at back wall (Y axis).
    LinearY,
    /// Notes distributed in a circle around the room center.
    Circular,
}

impl NotePositionMapping {
    /// Compute a room position for a given MIDI note.
    #[must_use]
    pub fn position_for_note(
        self,
        note: u8,
        room_length: f32,
        room_width: f32,
        room_height: f32,
    ) -> [f32; 3] {
        let t = note as f32 / 127.0;
        let margin = 0.3;
        match self {
            Self::Off => [room_length * 0.5, room_width * 0.5, room_height * 0.5],
            Self::LinearX => {
                let x = margin + t * (room_length - 2.0 * margin);
                [x, room_width * 0.5, room_height * 0.5]
            }
            Self::LinearY => {
                let y = margin + t * (room_width - 2.0 * margin);
                [room_length * 0.5, y, room_height * 0.5]
            }
            Self::Circular => {
                let angle = t * std::f32::consts::TAU;
                let cx = room_length * 0.5;
                let cy = room_width * 0.5;
                let radius = (room_length.min(room_width) * 0.5 - margin).max(0.1);
                let x = cx + radius * angle.cos();
                let y = cy + radius * angle.sin();
                [x, y, room_height * 0.5]
            }
        }
    }

    /// Compute a stereo pan value (-1..1) for a note's dry signal.
    ///
    /// Based on the note's X position relative to the listener.
    #[must_use]
    pub fn pan_for_note(
        self,
        note: u8,
        room_length: f32,
        room_width: f32,
        room_height: f32,
        listener_x: f32,
    ) -> f32 {
        if self == Self::Off {
            return 0.0;
        }
        let pos = self.position_for_note(note, room_length, room_width, room_height);
        let dx = pos[0] - listener_x;
        if room_length > 0.0 {
            (dx / (room_length * 0.5)).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Context passed from `SynthEngine` to `Instrument` for per-voice spatial capture.
#[derive(Debug, Clone, Copy)]
pub struct SpatialContext {
    /// How notes map to positions.
    pub mapping: NotePositionMapping,
    /// Room length (X axis) in meters.
    pub room_length: f32,
    /// Room width (Y axis) in meters.
    pub room_width: f32,
    /// Room height (Z axis) in meters.
    pub room_height: f32,
    /// Listener X position for dry panning.
    pub listener_x: f32,
}

/// Info about a single active spatial voice slot.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpatialVoiceInfo {
    /// Whether this slot is active.
    pub active: bool,
    /// MIDI note number.
    pub note: u8,
    /// Number of valid samples in the buffer.
    pub sample_count: usize,
}

/// Pre-allocated bank of mono buffers for per-voice audio capture.
///
/// Written by `Instrument::process()`, read by `AweEngine::process_spatial()`.
pub struct SpatialVoiceBank {
    infos: [SpatialVoiceInfo; MAX_SPATIAL_VOICES],
    buffers: Vec<Vec<f32>>,
    active_count: usize,
}

impl SpatialVoiceBank {
    /// Create a new bank with pre-allocated buffers.
    #[must_use]
    pub fn new() -> Self {
        let mut buffers = Vec::with_capacity(MAX_SPATIAL_VOICES);
        for _ in 0..MAX_SPATIAL_VOICES {
            buffers.push(vec![0.0; VOICE_BUFFER_SIZE]);
        }
        Self {
            infos: [SpatialVoiceInfo::default(); MAX_SPATIAL_VOICES],
            buffers,
            active_count: 0,
        }
    }

    /// Clear all slots for a new processing block.
    pub fn clear(&mut self) {
        self.active_count = 0;
        for info in &mut self.infos {
            info.active = false;
            info.sample_count = 0;
        }
    }

    /// Write a voice's mono audio into the next available slot.
    ///
    /// Returns the slot index, or `None` if the bank is full.
    pub fn write_voice(
        &mut self,
        note: u8,
        left: &[f32],
        right: &[f32],
        count: usize,
    ) -> Option<usize> {
        if self.active_count >= MAX_SPATIAL_VOICES {
            return None;
        }
        let idx = self.active_count;
        self.active_count += 1;

        let buf = &mut self.buffers[idx];
        let n = count.min(buf.len()).min(left.len()).min(right.len());
        for i in 0..n {
            buf[i] = (left[i] + right[i]) * 0.5;
        }

        self.infos[idx] = SpatialVoiceInfo {
            active: true,
            note,
            sample_count: n,
        };

        Some(idx)
    }

    /// Number of active voice slots.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    /// Get info for a slot.
    #[must_use]
    pub fn info(&self, idx: usize) -> &SpatialVoiceInfo {
        &self.infos[idx]
    }

    /// Get the mono buffer for a slot.
    #[must_use]
    pub fn buffer(&self, idx: usize) -> &[f32] {
        &self.buffers[idx]
    }
}

impl Default for SpatialVoiceBank {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-voice DSP slot: individual early reflections and spatializer.
pub(crate) struct SpatialVoiceSlot {
    pub(crate) early_reflections: EarlyReflections,
    pub(crate) spatializer: Spatializer,
    pub(crate) position: [f32; 3],
    pub(crate) note: u8,
    pub(crate) active: bool,
    pub(crate) geometry_dirty: bool,
}

impl SpatialVoiceSlot {
    fn new() -> Self {
        Self {
            early_reflections: EarlyReflections::with_max_delay(PER_VOICE_MAX_DELAY),
            spatializer: Spatializer::new(),
            position: [0.0; 3],
            note: 0,
            active: false,
            geometry_dirty: false,
        }
    }

    fn clear(&mut self) {
        self.early_reflections.clear();
        self.spatializer.clear();
        self.active = false;
        self.geometry_dirty = false;
    }
}

/// Pool of per-voice DSP processors.
pub(crate) struct SpatialVoicePool {
    pub(crate) slots: Vec<SpatialVoiceSlot>,
}

impl SpatialVoicePool {
    /// Create a new pool with pre-allocated DSP processors.
    pub(crate) fn new() -> Self {
        let mut slots = Vec::with_capacity(MAX_SPATIAL_VOICES);
        for _ in 0..MAX_SPATIAL_VOICES {
            slots.push(SpatialVoiceSlot::new());
        }
        Self { slots }
    }

    /// Update a slot's geometry for the given note and room parameters.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_slot(
        &mut self,
        idx: usize,
        note: u8,
        mapping: NotePositionMapping,
        room_length: f32,
        room_width: f32,
        room_height: f32,
        listener_pos: [f32; 3],
        absorption: f32,
        sample_rate: f32,
    ) {
        let slot = &mut self.slots[idx];
        let pos = mapping.position_for_note(note, room_length, room_width, room_height);
        slot.position = pos;
        slot.note = note;
        slot.active = true;

        slot.early_reflections.update_geometry(
            room_length,
            room_width,
            room_height,
            pos,
            listener_pos,
            absorption,
            sample_rate,
        );

        slot.spatializer.update(pos, listener_pos, sample_rate);
        slot.geometry_dirty = false;
    }

    /// Process a single mono sample through a slot's early reflections and spatializer.
    ///
    /// Returns `(early_l, early_r, spat_l, spat_r)`.
    #[inline]
    pub(crate) fn process_slot(&mut self, idx: usize, mono_sample: f32) -> (f32, f32, f32, f32) {
        let slot = &mut self.slots[idx];
        let (el, er) = slot.early_reflections.process(mono_sample);
        let (sl, sr) = slot.spatializer.process(mono_sample);
        (el, er, sl, sr)
    }

    /// Deactivate all slots and clear DSP state.
    pub(crate) fn clear(&mut self) {
        for slot in &mut self.slots {
            slot.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_position_off() {
        let pos = NotePositionMapping::Off.position_for_note(60, 8.0, 5.0, 3.0);
        assert!((pos[0] - 4.0).abs() < 0.01);
        assert!((pos[1] - 2.5).abs() < 0.01);
    }

    #[test]
    fn test_note_position_linear_x() {
        let pos_low = NotePositionMapping::LinearX.position_for_note(0, 8.0, 5.0, 3.0);
        let pos_high = NotePositionMapping::LinearX.position_for_note(127, 8.0, 5.0, 3.0);
        assert!(pos_low[0] < pos_high[0]);
        assert!(pos_low[0] > 0.0);
        assert!(pos_high[0] < 8.0);
    }

    #[test]
    fn test_note_position_circular() {
        let pos = NotePositionMapping::Circular.position_for_note(0, 8.0, 5.0, 3.0);
        // Note 0 should be to the right of center (angle=0 → cos=1)
        assert!(pos[0] > 4.0);
    }

    #[test]
    fn test_pan_for_note_off() {
        let pan = NotePositionMapping::Off.pan_for_note(60, 8.0, 5.0, 3.0, 6.0);
        assert!((pan).abs() < 0.01);
    }

    #[test]
    fn test_voice_bank_write_and_read() {
        let mut bank = SpatialVoiceBank::new();
        bank.clear();

        let left = [0.5_f32; 64];
        let right = [0.3_f32; 64];
        let idx = bank.write_voice(60, &left, &right, 64);
        assert_eq!(idx, Some(0));
        assert_eq!(bank.active_count(), 1);
        assert!(bank.info(0).active);
        assert_eq!(bank.info(0).note, 60);
        assert_eq!(bank.info(0).sample_count, 64);
        // Mono should be average of left and right
        assert!((bank.buffer(0)[0] - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_voice_bank_full() {
        let mut bank = SpatialVoiceBank::new();
        bank.clear();

        let buf = [0.0_f32; 64];
        for i in 0..MAX_SPATIAL_VOICES {
            assert!(bank.write_voice(i as u8, &buf, &buf, 64).is_some());
        }
        // 17th should fail
        assert!(bank.write_voice(0, &buf, &buf, 64).is_none());
    }

    #[test]
    fn test_voice_pool_create_and_clear() {
        let mut pool = SpatialVoicePool::new();
        assert_eq!(pool.slots.len(), MAX_SPATIAL_VOICES);
        pool.clear();
        assert!(!pool.slots[0].active);
    }
}
