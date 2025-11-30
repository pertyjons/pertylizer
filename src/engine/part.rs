//! Synth part management for multitimbral playback.
//!
//! A `SynthPart` represents a single instrument/timbre that can respond
//! to MIDI events on a specific channel. Each part has its own voice
//! allocator for independent polyphony control.
//!
//! ## Type Safety
//!
//! This module uses domain-specific types throughout:
//! - [`PartId`] instead of `u64` for part identifiers
//! - [`MidiChannel`] instead of `u8` for MIDI channel numbers
//!
//! This prevents common errors like mixing up part IDs with other identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::engine::voice::VoiceState;
use crate::engine::voice_allocator::{AllocatorConfig, VoiceAllocator};
use crate::modules::{AudioBuffer, ProcessContext};
use crate::types::{BipolarValue, Gain, NormalizedValue};

/// Unique identifier for a synth part.
///
/// Each part in the synth engine has a unique ID that persists for
/// its lifetime. IDs are never reused within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct PartId(pub u64);

impl PartId {
    /// The default/first part ID.
    pub const FIRST: Self = Self(0);

    /// Create a new part ID.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PartId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Part({})", self.0)
    }
}

impl From<u64> for PartId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

/// MIDI channel number (1-16).
///
/// MIDI channels are one-indexed for human display (1-16),
/// but stored as zero-indexed internally (0-15) per MIDI spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct MidiChannel(u8);

impl MidiChannel {
    /// Channel 1 (omni/default).
    pub const CH1: Self = Self(0);
    /// Channel 10 (drums in General MIDI).
    pub const DRUMS: Self = Self(9);
    /// All channels (omni mode).
    pub const OMNI: Self = Self(255);

    /// Create a MIDI channel from a 1-indexed number (1-16).
    ///
    /// Returns `None` if the channel is out of range.
    #[inline]
    pub const fn from_one_indexed(channel: u8) -> Option<Self> {
        if channel >= 1 && channel <= 16 {
            Some(Self(channel - 1))
        } else {
            None
        }
    }

    /// Create a MIDI channel from a 0-indexed number (0-15).
    ///
    /// Returns `None` if the channel is out of range.
    #[inline]
    pub const fn from_zero_indexed(channel: u8) -> Option<Self> {
        if channel < 16 {
            Some(Self(channel))
        } else {
            None
        }
    }

    /// Get the zero-indexed channel number (0-15).
    #[inline]
    pub const fn as_zero_indexed(self) -> u8 {
        self.0
    }

    /// Get the one-indexed channel number (1-16).
    ///
    /// Returns 0 for OMNI mode.
    #[inline]
    pub const fn as_one_indexed(self) -> u8 {
        if self.0 == 255 {
            0 // OMNI
        } else {
            self.0 + 1
        }
    }

    /// Check if this channel matches a given zero-indexed channel.
    ///
    /// OMNI mode matches all channels.
    #[inline]
    pub const fn matches(self, channel: u8) -> bool {
        self.0 == 255 || self.0 == channel
    }

    /// Check if this is OMNI mode (responds to all channels).
    #[inline]
    pub const fn is_omni(self) -> bool {
        self.0 == 255
    }
}

impl fmt::Display for MidiChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_omni() {
            write!(f, "OMNI")
        } else {
            write!(f, "Ch{}", self.as_one_indexed())
        }
    }
}

impl Default for MidiChannel {
    fn default() -> Self {
        Self::CH1
    }
}

/// Maximum buffer size for part audio buffers.
const MAX_BUFFER_SIZE: usize = 4096;

/// A synthesizer part - an independent instrument with its own voice allocation.
///
/// Parts enable multitimbral operation where different MIDI channels can
/// play different sounds simultaneously. Each part has:
/// - Its own voice allocator (polyphony, mono, legato modes)
/// - Volume and pan controls
/// - MIDI channel assignment
/// - Internal audio buffers for voice processing
pub struct SynthPart {
    /// Unique identifier for this part.
    id: PartId,
    /// Human-readable name.
    name: String,
    /// MIDI channel this part responds to.
    midi_channel: MidiChannel,
    /// Voice allocator for this part.
    allocator: VoiceAllocator,
    /// Output volume.
    volume: Gain,
    /// Stereo pan position.
    pan: BipolarValue,
    /// Whether this part is enabled.
    enabled: bool,
    /// Left channel buffer for voice summing.
    voice_left: AudioBuffer,
    /// Right channel buffer for voice summing.
    voice_right: AudioBuffer,
    /// Default velocity-to-amplitude sensitivity for new voices.
    velocity_amp_sensitivity: NormalizedValue,
    /// Default velocity-to-filter sensitivity for new voices.
    velocity_filter_sensitivity: NormalizedValue,
}

impl SynthPart {
    /// Create a new synth part with the given ID and name.
    pub fn new(id: PartId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            midi_channel: MidiChannel::default(),
            allocator: VoiceAllocator::new(AllocatorConfig::default()),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            enabled: true,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            velocity_amp_sensitivity: NormalizedValue::MAX,       // Full dynamic range
            velocity_filter_sensitivity: NormalizedValue::CENTER, // 50% filter sensitivity
        }
    }

    /// Create a new synth part with a custom allocator configuration.
    pub fn with_config(id: PartId, name: impl Into<String>, config: AllocatorConfig) -> Self {
        Self {
            id,
            name: name.into(),
            midi_channel: MidiChannel::default(),
            allocator: VoiceAllocator::new(config),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            enabled: true,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            velocity_amp_sensitivity: NormalizedValue::MAX,
            velocity_filter_sensitivity: NormalizedValue::CENTER,
        }
    }

    /// Get the part ID.
    #[inline]
    pub fn id(&self) -> PartId {
        self.id
    }

    /// Get the part name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the part name.
    #[inline]
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Get the MIDI channel.
    #[inline]
    pub fn midi_channel(&self) -> MidiChannel {
        self.midi_channel
    }

    /// Set the MIDI channel.
    #[inline]
    pub fn set_midi_channel(&mut self, channel: MidiChannel) {
        self.midi_channel = channel;
    }

    /// Get the voice allocator.
    #[inline]
    pub fn allocator(&self) -> &VoiceAllocator {
        &self.allocator
    }

    /// Get mutable access to the voice allocator.
    #[inline]
    pub fn allocator_mut(&mut self) -> &mut VoiceAllocator {
        &mut self.allocator
    }

    /// Get the volume.
    #[inline]
    pub fn volume(&self) -> Gain {
        self.volume
    }

    /// Set the volume.
    #[inline]
    pub fn set_volume(&mut self, volume: Gain) {
        self.volume = volume;
    }

    /// Get the pan position.
    #[inline]
    pub fn pan(&self) -> BipolarValue {
        self.pan
    }

    /// Set the pan position.
    #[inline]
    pub fn set_pan(&mut self, pan: BipolarValue) {
        self.pan = pan;
    }

    /// Check if this part is enabled.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable this part.
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if this part should respond to a MIDI event on the given channel.
    #[inline]
    pub fn responds_to_channel(&self, channel: u8) -> bool {
        self.enabled && self.midi_channel.matches(channel)
    }

    /// Get velocity-to-amplitude sensitivity.
    #[inline]
    pub fn velocity_amp_sensitivity(&self) -> NormalizedValue {
        self.velocity_amp_sensitivity
    }

    /// Set velocity-to-amplitude sensitivity.
    ///
    /// This updates all existing voices and will be applied to new voices.
    pub fn set_velocity_amp_sensitivity(&mut self, sensitivity: NormalizedValue) {
        self.velocity_amp_sensitivity = sensitivity;
        // Update all existing voices
        for voice in self.allocator.voices_mut() {
            voice.expression.velocity_to_amp = sensitivity;
        }
    }

    /// Get velocity-to-filter sensitivity.
    #[inline]
    pub fn velocity_filter_sensitivity(&self) -> NormalizedValue {
        self.velocity_filter_sensitivity
    }

    /// Set velocity-to-filter sensitivity.
    ///
    /// This updates all existing voices and will be applied to new voices.
    pub fn set_velocity_filter_sensitivity(&mut self, sensitivity: NormalizedValue) {
        self.velocity_filter_sensitivity = sensitivity;
        // Update all existing voices
        for voice in self.allocator.voices_mut() {
            voice.expression.velocity_to_filter = sensitivity;
        }
    }

    /// Get the number of active voices in this part.
    #[inline]
    pub fn active_voice_count(&self) -> usize {
        self.allocator.active_voice_count()
    }

    /// Handle a note on event.
    ///
    /// Returns the voice ID if a voice was allocated.
    pub fn note_on(&mut self, note: u8, velocity: f32) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        self.allocator.note_on(note, velocity)
    }

    /// Handle a note off event.
    pub fn note_off(&mut self, note: u8) {
        self.allocator.note_off(note);
    }

    /// Release all notes.
    pub fn all_notes_off(&mut self) {
        self.allocator.all_notes_off();
    }

    /// Kill all voices immediately.
    pub fn panic(&mut self) {
        self.allocator.panic();
    }

    /// Process all active voices in this part and mix into the output buffer.
    ///
    /// This method:
    /// 1. Processes each active voice through its signal chain
    /// 2. Applies part volume and pan
    /// 3. Mixes the result into the stereo output buffer
    ///
    /// # Arguments
    /// * `output` - Stereo interleaved output buffer to mix into (samples * 2)
    /// * `context` - Processing context with sample rate, buffer size, etc.
    ///
    /// # Returns
    /// The number of active voices processed.
    pub fn process(&mut self, output: &mut AudioBuffer, context: &ProcessContext) -> u32 {
        if !self.enabled {
            return 0;
        }

        let samples = context.samples;
        let mut active_count = 0u32;

        // Ensure internal buffers are sized correctly
        self.voice_left.resize(samples);
        self.voice_right.resize(samples);

        // Get part's stereo gain (includes volume and pan)
        let (left_gain, right_gain) = self.stereo_gain();
        let left_gain = left_gain.as_f32();
        let right_gain = right_gain.as_f32();

        // Process each voice in this part
        for voice in self.allocator.voices_mut() {
            if !voice.is_active() {
                continue;
            }

            active_count += 1;

            // Update glide and increment age
            let delta_time = samples as f32 / context.sample_rate;
            voice.glide.update(delta_time);
            voice.age += samples as u64;

            // Handle stealing fade-out
            if voice.state == VoiceState::Stealing {
                if voice.steal_fade_counter == 0 {
                    voice.reset();
                    continue;
                }
            }

            // Clear voice output buffers
            self.voice_left.clear();
            self.voice_right.clear();

            // Process the voice signal chain
            voice.process_audio(&mut self.voice_left, &mut self.voice_right, context);

            // Apply stealing fade-out if needed
            if voice.state == VoiceState::Stealing {
                let fade_samples = voice.steal_fade_counter.min(samples);
                for i in 0..samples {
                    let fade = if i < fade_samples {
                        (voice.steal_fade_counter - i) as f32 / voice.steal_fade_samples as f32
                    } else {
                        0.0
                    };
                    self.voice_left[i] *= fade;
                    self.voice_right[i] *= fade;
                }
                voice.steal_fade_counter = voice.steal_fade_counter.saturating_sub(samples);
            }

            // Mix stereo output into main buffer with part volume/pan
            for i in 0..samples {
                output[i * 2] += self.voice_left[i] * left_gain;
                output[i * 2 + 1] += self.voice_right[i] * right_gain;
            }
        }

        // Advance allocator time
        self.allocator.advance_time(samples as u64);

        active_count
    }

    /// Get the stereo gain based on volume and pan.
    ///
    /// Returns `(left_gain, right_gain)` using constant-power panning.
    #[inline]
    pub fn stereo_gain(&self) -> (Gain, Gain) {
        let (left, right) = Gain::from_pan(self.pan);
        (
            Gain::new(left.as_f32() * self.volume.as_f32()),
            Gain::new(right.as_f32() * self.volume.as_f32()),
        )
    }
}

impl fmt::Debug for SynthPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SynthPart")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("midi_channel", &self.midi_channel)
            .field("volume", &self.volume)
            .field("pan", &self.pan)
            .field("enabled", &self.enabled)
            .field("active_voices", &self.active_voice_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_part_id() {
        let id = PartId::new(42);
        assert_eq!(id.as_u64(), 42);
        assert_eq!(format!("{}", id), "Part(42)");
    }

    #[test]
    fn test_midi_channel_creation() {
        // One-indexed
        let ch1 = MidiChannel::from_one_indexed(1).unwrap();
        assert_eq!(ch1.as_zero_indexed(), 0);
        assert_eq!(ch1.as_one_indexed(), 1);

        let ch16 = MidiChannel::from_one_indexed(16).unwrap();
        assert_eq!(ch16.as_zero_indexed(), 15);
        assert_eq!(ch16.as_one_indexed(), 16);

        // Invalid
        assert!(MidiChannel::from_one_indexed(0).is_none());
        assert!(MidiChannel::from_one_indexed(17).is_none());

        // Zero-indexed
        let ch0 = MidiChannel::from_zero_indexed(0).unwrap();
        assert_eq!(ch0.as_one_indexed(), 1);

        assert!(MidiChannel::from_zero_indexed(16).is_none());
    }

    #[test]
    fn test_midi_channel_matching() {
        let ch5 = MidiChannel::from_one_indexed(5).unwrap();
        assert!(ch5.matches(4)); // zero-indexed
        assert!(!ch5.matches(3));

        let omni = MidiChannel::OMNI;
        assert!(omni.matches(0));
        assert!(omni.matches(15));
        assert!(omni.is_omni());
    }

    #[test]
    fn test_synth_part_creation() {
        let part = SynthPart::new(PartId::FIRST, "Lead");
        assert_eq!(part.id(), PartId::FIRST);
        assert_eq!(part.name(), "Lead");
        assert_eq!(part.midi_channel(), MidiChannel::CH1);
        assert!(part.is_enabled());
        assert_eq!(part.volume(), Gain::UNITY);
        assert_eq!(part.pan(), BipolarValue::CENTER);
    }

    #[test]
    fn test_synth_part_responds_to_channel() {
        let mut part = SynthPart::new(PartId::new(1), "Bass");
        part.set_midi_channel(MidiChannel::from_one_indexed(2).unwrap());

        // Should respond to channel 2 (zero-indexed: 1)
        assert!(part.responds_to_channel(1));
        assert!(!part.responds_to_channel(0));

        // Disabled part should not respond
        part.set_enabled(false);
        assert!(!part.responds_to_channel(1));
    }

    #[test]
    fn test_synth_part_note_handling() {
        let mut part = SynthPart::new(PartId::new(1), "Synth");

        // Note on should allocate a voice
        let voice_id = part.note_on(60, 0.8);
        assert!(voice_id.is_some());
        assert_eq!(part.active_voice_count(), 1);

        // Note off should release
        part.note_off(60);
        // Voice should be releasing (still "active" in allocator terms until envelope finishes)
    }

    #[test]
    fn test_stereo_gain() {
        let mut part = SynthPart::new(PartId::new(1), "Test");

        // Center pan at unity volume
        let (left, right) = part.stereo_gain();
        let sqrt_half = (0.5_f32).sqrt();
        assert!((left.as_f32() - sqrt_half).abs() < 0.01);
        assert!((right.as_f32() - sqrt_half).abs() < 0.01);

        // Full left pan
        part.set_pan(BipolarValue::MIN);
        let (left, right) = part.stereo_gain();
        assert!((left.as_f32() - 1.0).abs() < 0.01);
        assert!(right.as_f32() < 0.01);

        // Half volume
        part.set_pan(BipolarValue::CENTER);
        part.set_volume(Gain::new(0.5));
        let (left, right) = part.stereo_gain();
        assert!((left.as_f32() - sqrt_half * 0.5).abs() < 0.01);
        assert!((right.as_f32() - sqrt_half * 0.5).abs() < 0.01);
    }
}
