//! Instrument management for multitimbral playback.
//!
//! An `Instrument` represents a single instrument/timbre that can respond
//! to MIDI events on a specific channel. Each instrument has its own voice
//! allocator for independent polyphony control.
//!
//! ## Type Safety
//!
//! This module uses domain-specific types throughout:
//! - [`InstrumentId`] instead of `u64` for instrument identifiers
//! - [`MidiChannel`] instead of `u8` for MIDI channel numbers
//!
//! This prevents common errors like mixing up instrument IDs with other identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::engine::effect_chain::EffectChain;
use crate::engine::graph::ModuleGraph;
use crate::engine::voice::VoiceState;
use crate::engine::voice_allocator::{AllocatorConfig, VoiceAllocator};
use crate::modules::{AudioBuffer, ProcessContext};
use crate::types::{BipolarValue, Gain, MidiNote, NormalizedValue, SampleCount};

/// Unique identifier for an instrument.
///
/// Each instrument in the synth engine has a unique ID that persists for
/// its lifetime. IDs are never reused within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct InstrumentId(pub u64);

impl InstrumentId {
    /// The default/first instrument ID.
    pub const FIRST: Self = Self(0);

    /// Create a new instrument ID.
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

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instrument({})", self.0)
    }
}

impl From<u64> for InstrumentId {
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

    /// All 16 standard MIDI channels.
    pub const ALL: [Self; 16] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
        Self(10),
        Self(11),
        Self(12),
        Self(13),
        Self(14),
        Self(15),
    ];

    /// Iterator over all 16 channels.
    pub fn iter() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter()
    }

    /// Get the next channel (wraps from 16 to 1).
    /// Returns self if OMNI.
    #[inline]
    pub fn next(self) -> Self {
        if self.is_omni() {
            self
        } else {
            Self((self.0 + 1) % 16)
        }
    }

    /// Get the previous channel (wraps from 1 to 16).
    /// Returns self if OMNI.
    #[inline]
    pub fn prev(self) -> Self {
        if self.is_omni() {
            self
        } else {
            Self((self.0 + 15) % 16)
        }
    }

    /// Check if this is the drums channel (channel 10 in GM).
    #[inline]
    pub const fn is_drums(self) -> bool {
        self.0 == 9
    }

    /// Get channel by number (1-16), returning None for invalid.
    /// Alias for from_one_indexed for clarity.
    #[inline]
    pub const fn channel(num: u8) -> Option<Self> {
        Self::from_one_indexed(num)
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

/// Maximum buffer size for instrument audio buffers.
const MAX_BUFFER_SIZE: usize = 4096;

/// A synthesizer instrument - an independent sound source with its own voice allocation.
///
/// Instruments enable multitimbral operation where different MIDI channels can
/// play different sounds simultaneously. Each instrument has:
/// - Its own voice graph (module structure for this instrument's sound)
/// - Its own voice allocator (polyphony, mono, legato modes)
/// - Its own effect chain (insert effects processed before mixing to output)
/// - Volume and pan controls
/// - MIDI channel assignment
/// - Internal audio buffers for voice processing
pub struct Instrument {
    /// Unique identifier for this instrument.
    id: InstrumentId,
    /// Human-readable name.
    name: String,
    /// MIDI channel this instrument responds to.
    midi_channel: MidiChannel,
    /// The module graph defining this instrument's voice architecture.
    /// Each instrument can have a completely different sound design.
    voice_graph: ModuleGraph,
    /// Voice allocator for this instrument.
    allocator: VoiceAllocator,
    /// Per-instrument effect chain (insert effects).
    /// Processes audio after voice summing, before mixing to main output.
    effect_chain: EffectChain,
    /// Output volume.
    volume: Gain,
    /// Stereo pan position.
    pan: BipolarValue,
    /// Whether this instrument is enabled.
    enabled: bool,
    /// Left channel buffer for voice summing.
    voice_left: AudioBuffer,
    /// Right channel buffer for voice summing.
    voice_right: AudioBuffer,
    /// Interleaved stereo buffer for effect processing.
    effect_buffer: AudioBuffer,
    /// Default velocity-to-amplitude sensitivity for new voices.
    velocity_amp_sensitivity: NormalizedValue,
    /// Default velocity-to-filter sensitivity for new voices.
    velocity_filter_sensitivity: NormalizedValue,
}

impl Instrument {
    /// Create a new instrument with the given ID and name.
    ///
    /// The voice_graph starts empty - populate it via engine commands or patch loading.
    /// The effect_chain starts empty - effects are added dynamically.
    pub fn new(id: InstrumentId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            midi_channel: MidiChannel::default(),
            voice_graph: ModuleGraph::new(),
            allocator: VoiceAllocator::new(AllocatorConfig::default()),
            effect_chain: EffectChain::new(),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            enabled: true,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            effect_buffer: AudioBuffer::new(MAX_BUFFER_SIZE * 2), // Interleaved stereo
            velocity_amp_sensitivity: NormalizedValue::MAX,       // Full dynamic range
            velocity_filter_sensitivity: NormalizedValue::CENTER, // 50% filter sensitivity
        }
    }

    /// Create a new instrument with a custom allocator configuration.
    ///
    /// The voice_graph starts empty - populate it via engine commands or patch loading.
    /// The effect_chain starts empty - effects are added dynamically.
    pub fn with_config(id: InstrumentId, name: impl Into<String>, config: AllocatorConfig) -> Self {
        Self {
            id,
            name: name.into(),
            midi_channel: MidiChannel::default(),
            voice_graph: ModuleGraph::new(),
            allocator: VoiceAllocator::new(config),
            effect_chain: EffectChain::new(),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            enabled: true,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            effect_buffer: AudioBuffer::new(MAX_BUFFER_SIZE * 2), // Interleaved stereo
            velocity_amp_sensitivity: NormalizedValue::MAX,
            velocity_filter_sensitivity: NormalizedValue::CENTER,
        }
    }

    /// Get the instrument ID.
    #[inline]
    pub fn id(&self) -> InstrumentId {
        self.id
    }

    /// Get the instrument name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the instrument name.
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

    /// Get the voice graph (module architecture for this instrument).
    #[inline]
    pub fn voice_graph(&self) -> &ModuleGraph {
        &self.voice_graph
    }

    /// Get mutable access to the voice graph.
    #[inline]
    pub fn voice_graph_mut(&mut self) -> &mut ModuleGraph {
        &mut self.voice_graph
    }

    /// Get the effect chain.
    #[inline]
    pub fn effect_chain(&self) -> &EffectChain {
        &self.effect_chain
    }

    /// Get mutable access to the effect chain.
    #[inline]
    pub fn effect_chain_mut(&mut self) -> &mut EffectChain {
        &mut self.effect_chain
    }

    /// Rebuild all voices from this instrument's voice graph.
    ///
    /// Uses the voice_graph as a template to rebuild all voice allocator
    /// voices. Call this after modifying the voice_graph.
    pub fn rebuild_voices(&mut self) {
        // Clone the graph structure to avoid borrow checker issues
        // (we need &voice_graph and &mut allocator simultaneously)
        let graph_clone = self.voice_graph.clone_structure();
        self.allocator.rebuild_from_graph(&graph_clone);
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

    /// Check if this instrument is enabled.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable this instrument.
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if this instrument should respond to a MIDI event on the given channel.
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

    /// Get the number of active voices in this instrument.
    #[inline]
    pub fn active_voice_count(&self) -> usize {
        self.allocator.active_voice_count()
    }

    /// Handle a note on event.
    ///
    /// Returns the voice ID if a voice was allocated.
    pub fn note_on(&mut self, note: MidiNote, velocity: f32) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        self.allocator.note_on(note, velocity)
    }

    /// Handle a note off event.
    pub fn note_off(&mut self, note: MidiNote) {
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

    /// Process all active voices in this instrument and mix into the output buffer.
    ///
    /// This method:
    /// 1. Processes each active voice through its signal chain
    /// 2. Sums voice output into stereo buffers
    /// 3. Interleaves into effect_buffer and processes through effect_chain
    /// 4. Applies instrument volume and pan
    /// 5. Mixes the result into the stereo output buffer
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
        self.effect_buffer.resize(samples * 2); // Interleaved stereo

        // Clear instrument buffers for accumulation
        self.voice_left.clear();
        self.voice_right.clear();

        // Temporary buffers for individual voice processing
        let mut temp_left = AudioBuffer::new(samples);
        let mut temp_right = AudioBuffer::new(samples);

        // Process each voice in this instrument and sum into voice_left/voice_right
        for voice in self.allocator.voices_mut() {
            if !voice.is_active() {
                continue;
            }

            active_count += 1;

            // Update glide and increment age
            let delta_time = samples as f32 / context.sample_rate.as_f32();
            voice.glide.update(delta_time);
            voice.age = voice.age + SampleCount::new(samples);

            // Handle stealing fade-out completion
            if let VoiceState::Stealing { fade_counter, .. } = voice.state
                && fade_counter == 0
            {
                voice.reset();
                continue;
            }

            // Clear temp buffers for this voice
            temp_left.clear();
            temp_right.clear();

            // Process the voice signal chain
            voice.process_audio(&mut temp_left, &mut temp_right, context);

            // Apply stealing fade-out if needed
            if let VoiceState::Stealing {
                fade_counter,
                fade_total,
            } = voice.state
            {
                let fade_samples = fade_counter.min(samples);
                for i in 0..samples {
                    let fade = if i < fade_samples {
                        (fade_counter - i) as f32 / fade_total as f32
                    } else {
                        0.0
                    };
                    temp_left[i] *= fade;
                    temp_right[i] *= fade;
                }
                // Update the fade counter in the state
                let new_counter = fade_counter.saturating_sub(samples);
                voice.state = VoiceState::Stealing {
                    fade_counter: new_counter,
                    fade_total,
                };
            }

            // Sum into instrument buffers
            for i in 0..samples {
                self.voice_left[i] += temp_left[i];
                self.voice_right[i] += temp_right[i];
            }
        }

        // Advance allocator time
        self.allocator.advance_time(SampleCount::new(samples));

        // Interleave voice_left/voice_right into effect_buffer (L, R, L, R, ...)
        for i in 0..samples {
            self.effect_buffer[i * 2] = self.voice_left[i];
            self.effect_buffer[i * 2 + 1] = self.voice_right[i];
        }

        // Process through effect chain (modifies effect_buffer in place)
        self.effect_chain.process(&mut self.effect_buffer, context);

        // Get instrument's stereo gain (includes volume and pan)
        let (left_gain, right_gain) = self.stereo_gain();
        let left_gain = left_gain.as_f32();
        let right_gain = right_gain.as_f32();

        // Mix processed effect_buffer into main output with volume/pan
        for i in 0..samples {
            output[i * 2] += self.effect_buffer[i * 2] * left_gain;
            output[i * 2 + 1] += self.effect_buffer[i * 2 + 1] * right_gain;
        }

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

impl fmt::Debug for Instrument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instrument")
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
    fn test_instrument_id() {
        let id = InstrumentId::new(42);
        assert_eq!(id.as_u64(), 42);
        assert_eq!(format!("{}", id), "Instrument(42)");
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
    fn test_instrument_creation() {
        let instrument = Instrument::new(InstrumentId::FIRST, "Lead");
        assert_eq!(instrument.id(), InstrumentId::FIRST);
        assert_eq!(instrument.name(), "Lead");
        assert_eq!(instrument.midi_channel(), MidiChannel::CH1);
        assert!(instrument.is_enabled());
        assert_eq!(instrument.volume(), Gain::UNITY);
        assert_eq!(instrument.pan(), BipolarValue::CENTER);
    }

    #[test]
    fn test_instrument_responds_to_channel() {
        let mut instrument = Instrument::new(InstrumentId::new(1), "Bass");
        instrument.set_midi_channel(MidiChannel::from_one_indexed(2).unwrap());

        // Should respond to channel 2 (zero-indexed: 1)
        assert!(instrument.responds_to_channel(1));
        assert!(!instrument.responds_to_channel(0));

        // Disabled instrument should not respond
        instrument.set_enabled(false);
        assert!(!instrument.responds_to_channel(1));
    }

    #[test]
    fn test_instrument_note_handling() {
        let mut instrument = Instrument::new(InstrumentId::new(1), "Synth");

        // Note on should allocate a voice
        let voice_id = instrument.note_on(MidiNote::C4, 0.8);
        assert!(voice_id.is_some());
        assert_eq!(instrument.active_voice_count(), 1);

        // Note off should release
        instrument.note_off(MidiNote::C4);
        // Voice should be releasing (still "active" in allocator terms until envelope finishes)
    }

    #[test]
    fn test_stereo_gain() {
        let mut instrument = Instrument::new(InstrumentId::new(1), "Test");

        // Center pan at unity volume
        let (left, right) = instrument.stereo_gain();
        let sqrt_half = (0.5_f32).sqrt();
        assert!((left.as_f32() - sqrt_half).abs() < 0.01);
        assert!((right.as_f32() - sqrt_half).abs() < 0.01);

        // Full left pan
        instrument.set_pan(BipolarValue::MIN);
        let (left, right) = instrument.stereo_gain();
        assert!((left.as_f32() - 1.0).abs() < 0.01);
        assert!(right.as_f32() < 0.01);

        // Half volume
        instrument.set_pan(BipolarValue::CENTER);
        instrument.set_volume(Gain::new(0.5));
        let (left, right) = instrument.stereo_gain();
        assert!((left.as_f32() - sqrt_half * 0.5).abs() < 0.01);
        assert!((right.as_f32() - sqrt_half * 0.5).abs() < 0.01);
    }
}
