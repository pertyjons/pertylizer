//! Per-voice spatial processing for AWE Fas 3.
//!
//! Maps each active voice to a position in the room based on its MIDI note,
//! then processes individual early reflections and spatialisation per voice.

use serde::{Deserialize, Serialize};

use synth_core::{BipolarValue, MidiNote, NormalizedValue, SampleCount, SampleRate};

use crate::early_reflections::EarlyReflections;
use crate::spatializer::Spatializer;
use crate::types::{Meters, Position3};

/// Maximum number of simultaneous spatial voice slots.
pub const MAX_SPATIAL_VOICES: usize = 16;

/// Pre-allocated mono buffer size per voice (samples).
const VOICE_BUFFER_SIZE: SampleCount = SampleCount::new(4096);

/// Delay line size for per-voice early reflections (16K samples, ~170ms at 96kHz).
const PER_VOICE_MAX_DELAY: SampleCount = SampleCount::new(16_384);

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
    pub fn position_for_note(
        self,
        note: MidiNote,
        room_length: Meters,
        room_width: Meters,
        room_height: Meters,
    ) -> Position3 {
        let t = note.as_u8() as f32 / 127.0;
        let margin = 0.3;
        let room_length_f = room_length.as_f32();
        let room_width_f = room_width.as_f32();
        let room_height_f = room_height.as_f32();
        match self {
            Self::Off => Position3::new(
                Meters::new(room_length_f * 0.5),
                Meters::new(room_width_f * 0.5),
                Meters::new(room_height_f * 0.5),
            ),
            Self::LinearX => {
                let x = margin + t * (room_length_f - 2.0 * margin);
                Position3::new(
                    Meters::new(x),
                    Meters::new(room_width_f * 0.5),
                    Meters::new(room_height_f * 0.5),
                )
            }
            Self::LinearY => {
                let y = margin + t * (room_width_f - 2.0 * margin);
                Position3::new(
                    Meters::new(room_length_f * 0.5),
                    Meters::new(y),
                    Meters::new(room_height_f * 0.5),
                )
            }
            Self::Circular => {
                let angle = t * std::f32::consts::TAU;
                let cx = room_length_f * 0.5;
                let cy = room_width_f * 0.5;
                let radius = (room_length_f.min(room_width_f) * 0.5 - margin).max(0.1);
                let x = cx + radius * angle.cos();
                let y = cy + radius * angle.sin();
                Position3::new(
                    Meters::new(x),
                    Meters::new(y),
                    Meters::new(room_height_f * 0.5),
                )
            }
        }
    }

    /// Compute a stereo pan value (-1..1) for a note's dry signal.
    ///
    /// Based on the note's X position relative to the listener.
    #[must_use]
    pub fn pan_for_note(
        self,
        note: MidiNote,
        room_length: Meters,
        room_width: Meters,
        room_height: Meters,
        listener_x: Meters,
    ) -> BipolarValue {
        if self == Self::Off {
            return BipolarValue::CENTER;
        }
        let pos = self.position_for_note(note, room_length, room_width, room_height);
        let dx = pos.x().as_f32() - listener_x.as_f32();
        if room_length.as_f32() > 0.0 {
            BipolarValue::new(dx / (room_length.as_f32() * 0.5))
        } else {
            BipolarValue::CENTER
        }
    }
}

/// Context passed from `SynthEngine` to `Instrument` for per-voice spatial capture.
#[derive(Debug, Clone, Copy)]
pub struct SpatialContext {
    /// How notes map to positions.
    pub mapping: NotePositionMapping,
    /// Room length (X axis) in meters.
    pub room_length: Meters,
    /// Room width (Y axis) in meters.
    pub room_width: Meters,
    /// Room height (Z axis) in meters.
    pub room_height: Meters,
    /// Listener X position for dry panning.
    pub listener_x: Meters,
}

/// Info about a single active spatial voice slot.
#[derive(Debug, Clone, Copy)]
pub struct SpatialVoiceInfo {
    /// Whether this slot is active.
    pub active: bool,
    /// MIDI note number.
    pub note: MidiNote,
    /// Number of valid samples in the buffer.
    pub sample_count: SampleCount,
}

impl Default for SpatialVoiceInfo {
    fn default() -> Self {
        Self {
            active: false,
            note: MidiNote::MIN,
            sample_count: SampleCount::ZERO,
        }
    }
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
            buffers.push(vec![0.0; VOICE_BUFFER_SIZE.as_usize()]);
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
            info.sample_count = SampleCount::ZERO;
        }
    }

    /// Write a voice's mono audio into the next available slot.
    ///
    /// Returns the slot index, or `None` if the bank is full.
    pub fn write_voice(
        &mut self,
        note: MidiNote,
        left: &[f32],
        right: &[f32],
        count: SampleCount,
    ) -> Option<usize> {
        if self.active_count >= MAX_SPATIAL_VOICES {
            return None;
        }
        let idx = self.active_count;
        self.active_count += 1;

        let buf = &mut self.buffers[idx];
        let n = count
            .as_usize()
            .min(buf.len())
            .min(left.len())
            .min(right.len());
        for i in 0..n {
            buf[i] = (left[i] + right[i]) * 0.5;
        }

        self.infos[idx] = SpatialVoiceInfo {
            active: true,
            note,
            sample_count: SampleCount::new(n),
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
    pub(crate) position: Position3,
    pub(crate) note: MidiNote,
    pub(crate) active: bool,
    pub(crate) geometry_dirty: bool,
}

impl SpatialVoiceSlot {
    fn new() -> Self {
        Self {
            early_reflections: EarlyReflections::with_max_delay(PER_VOICE_MAX_DELAY),
            spatializer: Spatializer::new(),
            position: Position3::default(),
            note: MidiNote::MIN,
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
        note: MidiNote,
        mapping: NotePositionMapping,
        room_length: Meters,
        room_width: Meters,
        room_height: Meters,
        listener_pos: Position3,
        absorption: NormalizedValue,
        diffusion: NormalizedValue,
        sample_rate: SampleRate,
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
            diffusion,
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
        let pos = NotePositionMapping::Off.position_for_note(
            MidiNote::new(60),
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
        );
        assert!((pos.x().as_f32() - 4.0).abs() < 0.01);
        assert!((pos.y().as_f32() - 2.5).abs() < 0.01);
    }

    #[test]
    fn test_note_position_linear_x() {
        let pos_low = NotePositionMapping::LinearX.position_for_note(
            MidiNote::new(0),
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
        );
        let pos_high = NotePositionMapping::LinearX.position_for_note(
            MidiNote::new(127),
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
        );
        assert!(pos_low.x().as_f32() < pos_high.x().as_f32());
        assert!(pos_low.x().as_f32() > 0.0);
        assert!(pos_high.x().as_f32() < 8.0);
    }

    #[test]
    fn test_note_position_circular() {
        let pos = NotePositionMapping::Circular.position_for_note(
            MidiNote::new(0),
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
        );
        // Note 0 should be to the right of center (angle=0 → cos=1)
        assert!(pos.x().as_f32() > 4.0);
    }

    #[test]
    fn test_pan_for_note_off() {
        let pan = NotePositionMapping::Off.pan_for_note(
            MidiNote::new(60),
            Meters::new(8.0),
            Meters::new(5.0),
            Meters::new(3.0),
            Meters::new(6.0),
        );
        assert!(pan.as_f32().abs() < 0.01);
    }

    #[test]
    fn test_voice_bank_write_and_read() {
        let mut bank = SpatialVoiceBank::new();
        bank.clear();

        let left = [0.5_f32; 64];
        let right = [0.3_f32; 64];
        let idx = bank.write_voice(MidiNote::new(60), &left, &right, SampleCount::new(64));
        assert_eq!(idx, Some(0));
        assert_eq!(bank.active_count(), 1);
        assert!(bank.info(0).active);
        assert_eq!(bank.info(0).note, MidiNote::new(60));
        assert_eq!(bank.info(0).sample_count, SampleCount::new(64));
        // Mono should be average of left and right
        assert!((bank.buffer(0)[0] - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_voice_bank_full() {
        let mut bank = SpatialVoiceBank::new();
        bank.clear();

        let buf = [0.0_f32; 64];
        for i in 0..MAX_SPATIAL_VOICES {
            assert!(
                bank.write_voice(MidiNote::new(i as u8), &buf, &buf, SampleCount::new(64))
                    .is_some()
            );
        }
        // 17th should fail
        assert!(
            bank.write_voice(MidiNote::new(0), &buf, &buf, SampleCount::new(64))
                .is_none()
        );
    }

    #[test]
    fn test_voice_pool_create_and_clear() {
        let mut pool = SpatialVoicePool::new();
        assert_eq!(pool.slots.len(), MAX_SPATIAL_VOICES);
        pool.clear();
        assert!(!pool.slots[0].active);
    }
}
