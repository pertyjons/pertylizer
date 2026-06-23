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
use std::collections::HashMap;
use std::fmt;

use crate::effect_chain::EffectChain;
use crate::graph::ModuleGraph;
use crate::voice::{VoiceId, VoiceState};
use crate::voice_allocator::{AllocatorConfig, VoiceAllocator};
use synth_awe::{SpatialContext, SpatialVoiceBank};
use synth_core::{
    AudioBuffer, BipolarValue, Gain, MidiNote, ModuleType, MuteState, NormalizedValue, Param,
    ProcessContext, SampleCount, SamplePosition, SampleRate, Seconds, Semitones, SoloState,
    Velocity,
};
use synth_dsp::oversampling::{Downsampler, OversamplingFactor};

// ============================================================================
// Key Range & Learn State Types
// ============================================================================

/// A range of MIDI notes that an instrument responds to.
///
/// This allows keyboard splitting (e.g., bass on lower keys, lead on upper keys)
/// and drum pad assignment (single note per instrument).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRange {
    /// Lowest note in the range (inclusive).
    pub low: MidiNote,
    /// Highest note in the range (inclusive).
    pub high: MidiNote,
}

impl Default for KeyRange {
    fn default() -> Self {
        Self {
            low: MidiNote::new(0),
            high: MidiNote::new(127),
        }
    }
}

impl KeyRange {
    /// Create a new key range.
    ///
    /// Automatically ensures low <= high by swapping if necessary.
    #[must_use]
    pub fn new(low: MidiNote, high: MidiNote) -> Self {
        let (l, h) = if low.as_u8() <= high.as_u8() {
            (low, high)
        } else {
            (high, low)
        };
        Self { low: l, high: h }
    }

    /// Create a single-note range (for drum pads).
    #[must_use]
    pub fn single(note: MidiNote) -> Self {
        Self {
            low: note,
            high: note,
        }
    }

    /// Check if a note is within this range.
    #[must_use]
    pub fn contains(&self, note: MidiNote) -> bool {
        note.as_u8() >= self.low.as_u8() && note.as_u8() <= self.high.as_u8()
    }

    /// Get the span of notes in this range.
    #[must_use]
    pub fn span(&self) -> u8 {
        self.high.as_u8() - self.low.as_u8() + 1
    }

    /// Check if this is a single-note range.
    #[must_use]
    pub fn is_single(&self) -> bool {
        self.low == self.high
    }

    /// Check if this covers the full MIDI range (0-127).
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.low.as_u8() == 0 && self.high.as_u8() == 127
    }

    /// Full range (all 128 MIDI notes).
    pub const FULL: Self = Self {
        low: MidiNote::new(0),
        high: MidiNote::new(127),
    };
}

/// State machine for MIDI learn functionality.
///
/// Uses an enum instead of a boolean to be explicit about what the instrument
/// is waiting for, and to allow future expansion (e.g., learning a range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LearnState {
    /// Not learning - normal operation.
    #[default]
    Idle,
    /// Waiting for a single note to set both low and high (drum pad mode).
    /// The next note received will set the entire range to that single note.
    WaitingForNote,
    /// Waiting for the low note of a range.
    WaitingForLowNote,
    /// Waiting for the high note of a range after low was set.
    WaitingForHighNote {
        /// The low note that was already captured.
        low: MidiNote,
    },
}

/// Unique identifier for an instrument.
///
/// Each instrument in the synth engine has a unique ID that persists for
/// its lifetime. IDs are never reused within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[repr(transparent)]
pub struct InstrumentId(pub u64);

impl InstrumentId {
    /// The default/first instrument ID.
    pub const FIRST: Self = Self(0);

    /// Sentinel ID for master bus effects (not a real instrument).
    pub const MASTER: Self = Self(u64::MAX);

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

/// Widen a sequencer-side `SeqInstrumentId` (`u16`) to an engine `InstrumentId`
/// (`u64`). Always lossless.
impl From<synth_sequencer::SeqInstrumentId> for InstrumentId {
    fn from(id: synth_sequencer::SeqInstrumentId) -> Self {
        Self(u64::from(id.0))
    }
}

/// Narrow an engine `InstrumentId` (`u64`) to a sequencer `SeqInstrumentId`
/// (`u16`). Fails when the id does not fit in `u16` instead of silently
/// truncating (the old `id.0 as u16` cast).
impl TryFrom<InstrumentId> for synth_sequencer::SeqInstrumentId {
    type Error = std::num::TryFromIntError;

    fn try_from(id: InstrumentId) -> Result<Self, Self::Error> {
        Ok(Self(u16::try_from(id.0)?))
    }
}

/// Re-exported from `synth_osc_protocol` — shared across workspace and visualizer.
pub use synth_osc_protocol::InstrumentCategory;

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

// Maximum buffer size for instrument audio buffers — the engine-wide block ceiling.
use synth_core::MAX_BLOCK_SIZE as MAX_BUFFER_SIZE;

/// Soft clipper threshold - signals above this level start to be compressed.
const SOFT_CLIP_THRESHOLD: f32 = 0.8;

/// Apply soft clipping to prevent harsh digital clipping.
///
/// Uses a smooth tanh-like curve that preserves dynamics below the threshold
/// while gently compressing signals above it.
///
/// # Soft Clip Algorithm
/// - Below threshold: signal passes through unchanged
/// - Above threshold: smoothly compressed towards ±1.0
#[inline]
pub(crate) fn soft_clip(sample: f32) -> f32 {
    if sample.abs() <= SOFT_CLIP_THRESHOLD {
        sample
    } else {
        // Smooth soft clipping using tanh-like curve
        let sign = sample.signum();
        let abs_sample = sample.abs();
        // Map (threshold, inf) -> (threshold, 1.0) smoothly
        let excess = abs_sample - SOFT_CLIP_THRESHOLD;
        let headroom = 1.0 - SOFT_CLIP_THRESHOLD;
        // Asymptotic approach to 1.0: threshold + headroom * (1 - e^(-excess/headroom))
        let compressed = SOFT_CLIP_THRESHOLD + headroom * (1.0 - (-excess / headroom).exp());
        sign * compressed
    }
}

/// Sum an interleaved-stereo source into `dst`, applying per-channel gain and
/// per-sample [`soft_clip`]. Shared by the per-instrument channel-bus stage and
/// the return-bus output so both clip/mix identically.
#[inline]
pub(crate) fn mix_stereo_faded(src: &[f32], left_gain: f32, right_gain: f32, dst: &mut [f32]) {
    let frames = src.len().min(dst.len());
    let mut i = 0;
    while i + 1 < frames {
        dst[i] += soft_clip(src[i] * left_gain);
        dst[i + 1] += soft_clip(src[i + 1] * right_gain);
        i += 2;
    }
}

/// Post-fader peak amplitude of an interleaved stereo buffer with per-channel
/// gains — the metering counterpart to [`mix_stereo_faded`] (measured pre
/// soft-clip, matching the gain application).
#[inline]
pub(crate) fn stereo_peak(src: &[f32], left_gain: f32, right_gain: f32) -> f32 {
    let mut peak = 0.0_f32;
    for frame in src.chunks_exact(2) {
        peak = peak.max((frame[0] * left_gain).abs());
        peak = peak.max((frame[1] * right_gain).abs());
    }
    peak
}

/// A synthesizer instrument - an independent sound source with its own voice allocation.
///
/// Instruments enable multitimbral operation where different MIDI channels can
/// play different sounds simultaneously. Each instrument has:
/// - Its own voice graph (module structure for this instrument's sound)
/// - Its own voice allocator (polyphony, mono, legato modes)
/// - Its own effect chain (insert effects processed before mixing to output)
/// - Volume and pan controls
/// - MIDI channel assignment
/// - Key range for keyboard splitting
/// - Internal audio buffers for voice processing
pub struct Instrument {
    /// Unique identifier for this instrument.
    id: InstrumentId,
    /// Human-readable name.
    name: String,
    /// Free-text description / intent. Never affects audio; mirrored in
    /// `InstrumentSnapshot` so MCP can read+write per-instrument intent.
    description: String,
    /// Patch-level description (separate from instrument's per-instance
    /// description). Captures "what is this patch for" — author intent
    /// that travels with the patch when saved. `None` when no
    /// description was set or loaded.
    patch_description: Option<String>,
    /// Patch-level accent color (separate from the per-instance `color` below).
    /// Travels with the patch when saved. `None` when unset. Never affects
    /// audio; mirrored in `InstrumentSnapshot.patch_color`.
    patch_color: Option<String>,
    /// Optional accent color as a hex string (e.g. "#FF8800FF"). Never affects
    /// audio; mirrored in `InstrumentSnapshot` so MCP can read+write the color
    /// and the save path can persist it into `InstrumentState.color`. `None`
    /// when no color was set or loaded ("auto" / default tint).
    color: Option<String>,
    /// Free-text per-module-instance descriptions, keyed by `ModuleId`. Never
    /// affects audio; published into each `ModuleStateSnapshot.description` so
    /// MCP can read+write per-module intent and the save path can persist it.
    /// Distinct from the module *type* doc on `ModuleDescriptor`. Entries are
    /// pruned when the owning module is removed.
    module_descriptions: HashMap<crate::ModuleId, String>,
    /// Instrument that feeds this instrument's sidechain inputs (e.g.
    /// compressors with `sidechain_enabled` set). `None` = no sidechain.
    /// Audio routing is the engine's responsibility — see
    /// `SynthEngine::process_instruments`. Cycles are not currently
    /// detected; users / MCP should avoid them.
    sidechain_source_id: Option<InstrumentId>,
    /// Instrument category (drums, bass, pad, lead, etc.).
    category: InstrumentCategory,
    /// MIDI channel this instrument responds to.
    midi_channel: MidiChannel,
    /// Key range this instrument responds to.
    /// Notes outside this range are ignored.
    key_range: KeyRange,
    /// Transpose offset in semitones (-24 to +24).
    transpose: Semitones,
    /// MIDI learn state machine.
    learn_state: LearnState,
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
    /// Mute state (Unmuted = enabled, Muted = disabled).
    mute_state: MuteState,
    /// Solo state.
    /// When any instrument is soloed, only soloed instruments produce sound.
    solo_state: SoloState,
    /// Left channel buffer for voice summing.
    voice_left: AudioBuffer,
    /// Right channel buffer for voice summing.
    voice_right: AudioBuffer,
    /// Interleaved stereo buffer for effect processing.
    effect_buffer: AudioBuffer,
    /// Pre-allocated temporary left buffer for individual voice processing.
    temp_voice_left: AudioBuffer,
    /// Pre-allocated temporary right buffer for individual voice processing.
    temp_voice_right: AudioBuffer,
    /// Default velocity-to-amplitude sensitivity for new voices.
    velocity_amp_sensitivity: NormalizedValue,
    /// Default velocity-to-filter sensitivity for new voices.
    velocity_filter_sensitivity: NormalizedValue,
    /// Oversampling factor (Off/2x/4x) for anti-aliased voice processing.
    oversampling: OversamplingFactor,
    /// Left channel downsampler for oversampled voice output.
    downsampler_l: Downsampler,
    /// Right channel downsampler for oversampled voice output.
    downsampler_r: Downsampler,
    /// Oversampled left channel buffer (pre-allocated for up to 4x).
    os_voice_left: AudioBuffer,
    /// Oversampled right channel buffer (pre-allocated for up to 4x).
    os_voice_right: AudioBuffer,
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
            description: String::new(),
            patch_description: None,
            patch_color: None,
            color: None,
            module_descriptions: HashMap::new(),
            sidechain_source_id: None,
            category: InstrumentCategory::default(),
            midi_channel: MidiChannel::default(),
            key_range: KeyRange::default(),
            transpose: Semitones::ZERO,
            learn_state: LearnState::default(),
            voice_graph: ModuleGraph::new(),
            allocator: VoiceAllocator::new(AllocatorConfig::default()),
            effect_chain: EffectChain::new(),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            mute_state: MuteState::Unmuted,
            solo_state: SoloState::Normal,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            effect_buffer: AudioBuffer::new(MAX_BUFFER_SIZE * 2), // Interleaved stereo
            temp_voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            temp_voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            velocity_amp_sensitivity: NormalizedValue::MAX, // Full dynamic range
            velocity_filter_sensitivity: NormalizedValue::CENTER, // 50% filter sensitivity
            oversampling: OversamplingFactor::default(),
            downsampler_l: Downsampler::new(),
            downsampler_r: Downsampler::new(),
            os_voice_left: AudioBuffer::new(MAX_BUFFER_SIZE * 4),
            os_voice_right: AudioBuffer::new(MAX_BUFFER_SIZE * 4),
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
            description: String::new(),
            patch_description: None,
            patch_color: None,
            color: None,
            module_descriptions: HashMap::new(),
            sidechain_source_id: None,
            category: InstrumentCategory::default(),
            midi_channel: MidiChannel::default(),
            key_range: KeyRange::default(),
            transpose: Semitones::ZERO,
            learn_state: LearnState::default(),
            voice_graph: ModuleGraph::new(),
            allocator: VoiceAllocator::new(config),
            effect_chain: EffectChain::new(),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            mute_state: MuteState::Unmuted,
            solo_state: SoloState::Normal,
            voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            effect_buffer: AudioBuffer::new(MAX_BUFFER_SIZE * 2), // Interleaved stereo
            temp_voice_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            temp_voice_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            velocity_amp_sensitivity: NormalizedValue::MAX,
            velocity_filter_sensitivity: NormalizedValue::CENTER,
            oversampling: OversamplingFactor::default(),
            downsampler_l: Downsampler::new(),
            downsampler_r: Downsampler::new(),
            os_voice_left: AudioBuffer::new(MAX_BUFFER_SIZE * 4),
            os_voice_right: AudioBuffer::new(MAX_BUFFER_SIZE * 4),
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

    /// Get the instrument description (free-text intent).
    #[inline]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Set the instrument description.
    #[inline]
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    /// Get the patch description, if any.
    #[inline]
    pub fn patch_description(&self) -> Option<&str> {
        self.patch_description.as_deref()
    }

    /// Set or clear the patch description. `None` clears.
    #[inline]
    pub fn set_patch_description(&mut self, description: Option<String>) {
        self.patch_description = description;
    }

    /// Get the accent color (hex string), if any.
    #[inline]
    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    /// Set or clear the accent color. `None` clears back to "auto" / default.
    #[inline]
    pub fn set_color(&mut self, color: Option<String>) {
        self.color = color;
    }

    /// Get the patch-level accent color (hex string), if any.
    #[inline]
    pub fn patch_color(&self) -> Option<&str> {
        self.patch_color.as_deref()
    }

    /// Set or clear the patch-level accent color. `None` clears.
    #[inline]
    pub fn set_patch_color(&mut self, color: Option<String>) {
        self.patch_color = color;
    }

    /// Get the free-text description for a specific module instance, if set.
    #[inline]
    pub fn module_description(&self, module_id: crate::ModuleId) -> Option<&str> {
        self.module_descriptions.get(&module_id).map(String::as_str)
    }

    /// Set or clear a module instance's description. `None` (or an empty
    /// string) removes the entry so empty descriptions never persist.
    #[inline]
    pub fn set_module_description(
        &mut self,
        module_id: crate::ModuleId,
        description: Option<String>,
    ) {
        match description {
            Some(desc) if !desc.is_empty() => {
                self.module_descriptions.insert(module_id, desc);
            }
            _ => {
                self.module_descriptions.remove(&module_id);
            }
        }
    }

    /// Drop a module instance's stored description (called when the module is
    /// removed from the graph so the map doesn't accumulate stale entries).
    #[inline]
    pub fn remove_module_description(&mut self, module_id: crate::ModuleId) {
        self.module_descriptions.remove(&module_id);
    }

    /// Drop all stored module descriptions (called when the whole graph is
    /// cleared so descriptions don't resurface on modules later given the same
    /// `ModuleId`).
    #[inline]
    pub fn clear_module_descriptions(&mut self) {
        self.module_descriptions.clear();
    }

    /// Get the sidechain source instrument id, if any.
    #[inline]
    pub fn sidechain_source_id(&self) -> Option<InstrumentId> {
        self.sidechain_source_id
    }

    /// Set or clear the sidechain source instrument. `None` disables
    /// sidechain routing into this instrument's effect chain.
    #[inline]
    pub fn set_sidechain_source_id(&mut self, source: Option<InstrumentId>) {
        self.sidechain_source_id = source;
    }

    /// Get the most recently rendered post-effect-chain output as
    /// interleaved-stereo samples. Valid after `process` has run; before
    /// the first `process` it returns an empty slice. Used by the engine
    /// to feed cross-instrument sidechain routing.
    #[inline]
    pub fn last_output_interleaved(&self) -> &[f32] {
        self.effect_buffer.as_slice()
    }

    /// Push every sidechain-capable effect in this instrument's chain
    /// a fresh sidechain input buffer. Called by the engine before
    /// `process` when this instrument has a sidechain source configured.
    pub fn feed_sidechain_inputs(&mut self, buffer: &[f32]) {
        for slot in self.effect_chain.slots_mut() {
            if let crate::effect_chain::ChainSlot::Effect(effect_slot) = slot {
                effect_slot.effect.set_sidechain_input(buffer);
            }
        }
    }

    /// Get the instrument category.
    #[inline]
    pub fn category(&self) -> InstrumentCategory {
        self.category
    }

    /// Set the instrument category.
    #[inline]
    pub fn set_category(&mut self, category: InstrumentCategory) {
        self.category = category;
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

    /// Get the key range.
    #[inline]
    pub fn key_range(&self) -> KeyRange {
        self.key_range
    }

    /// Set the key range.
    #[inline]
    pub fn set_key_range(&mut self, range: KeyRange) {
        self.key_range = range;
    }

    /// Get the transpose offset in semitones.
    #[inline]
    pub fn transpose(&self) -> Semitones {
        self.transpose
    }

    /// Set the transpose offset in semitones.
    ///
    /// Clamped to -24..=24 (two octaves up or down).
    #[inline]
    pub fn set_transpose(&mut self, semitones: Semitones) {
        self.transpose = Semitones::new(semitones.as_f32().clamp(-24.0, 24.0));
    }

    /// Get the current learn state.
    #[inline]
    pub fn learn_state(&self) -> LearnState {
        self.learn_state
    }

    /// Set the learn state.
    #[inline]
    pub fn set_learn_state(&mut self, state: LearnState) {
        self.learn_state = state;
    }

    /// Check if this instrument should play a specific note.
    ///
    /// Returns true if the note is within the key range and the instrument is enabled.
    #[inline]
    pub fn should_play_note(&self, note: MidiNote) -> bool {
        self.mute_state.is_unmuted() && self.key_range.contains(note)
    }

    /// Handle a note for MIDI learn functionality.
    ///
    /// If the instrument is in a learn state, this will capture the note
    /// and potentially update the key range. Returns true if the note was
    /// captured for learning (and should NOT be played).
    pub fn handle_note_learn(&mut self, note: MidiNote) -> bool {
        match self.learn_state {
            LearnState::Idle => false,
            LearnState::WaitingForNote => {
                // Single note mode: set range to exactly this note
                self.key_range = KeyRange::single(note);
                self.learn_state = LearnState::Idle;
                true
            }
            LearnState::WaitingForLowNote => {
                // Captured low note, now wait for high note
                self.learn_state = LearnState::WaitingForHighNote { low: note };
                true
            }
            LearnState::WaitingForHighNote { low } => {
                // Captured high note, set the full range
                self.key_range = KeyRange::new(low, note);
                self.learn_state = LearnState::Idle;
                true
            }
        }
    }

    /// Apply transpose to a note.
    ///
    /// Returns None if the transposed note would be outside valid MIDI range.
    #[inline]
    pub fn transpose_note(&self, note: MidiNote) -> Option<MidiNote> {
        note.transpose(self.transpose)
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

    /// Get the mute state.
    #[inline]
    pub fn mute_state(&self) -> MuteState {
        self.mute_state
    }

    /// Check if this instrument is enabled (not muted).
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.mute_state.is_unmuted()
    }

    /// Enable or disable this instrument.
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.mute_state = MuteState::from(!enabled);
    }

    /// Set the mute state directly.
    #[inline]
    pub fn set_mute_state(&mut self, state: MuteState) {
        self.mute_state = state;
    }

    /// Get the solo state.
    #[inline]
    pub fn solo_state(&self) -> SoloState {
        self.solo_state
    }

    /// Check if this instrument is soloed.
    #[inline]
    pub fn is_solo(&self) -> bool {
        self.solo_state.is_solo()
    }

    /// Set the solo state for this instrument.
    #[inline]
    pub fn set_solo(&mut self, solo: bool) {
        self.solo_state = SoloState::from(solo);
    }

    /// Set the solo state directly.
    #[inline]
    pub fn set_solo_state(&mut self, state: SoloState) {
        self.solo_state = state;
    }

    /// Check if this instrument should respond to a MIDI event on the given channel.
    #[inline]
    pub fn responds_to_channel(&self, channel: u8) -> bool {
        self.mute_state.is_unmuted() && self.midi_channel.matches(channel)
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

    /// Get the oversampling factor.
    #[inline]
    pub fn oversampling(&self) -> OversamplingFactor {
        self.oversampling
    }

    /// Set the oversampling factor and reset downsampler state.
    pub fn set_oversampling(&mut self, factor: OversamplingFactor) {
        if self.oversampling != factor {
            self.oversampling = factor;
            self.downsampler_l.reset();
            self.downsampler_r.reset();
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
    /// The note is checked against the key range and transposed before playing.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity) -> Option<VoiceId> {
        self.note_on_expr(note, velocity, crate::voice::NoteTrigger::default())
    }

    /// Handle a note on event with per-note expression (legato/glide).
    ///
    /// Same key-range/transpose handling as [`note_on`](Self::note_on); the
    /// `trigger` carries the per-note legato/glide resolved from the sequencer
    /// event. Per-note glide offsets are note-relative, so they survive the
    /// instrument transpose unchanged.
    pub fn note_on_expr(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        trigger: crate::voice::NoteTrigger,
    ) -> Option<VoiceId> {
        if self.mute_state.is_muted() {
            return None;
        }

        // Check if note is within key range
        if !self.key_range.contains(note) {
            return None;
        }

        // Apply transpose
        let transposed_note = self.transpose_note(note)?;

        self.allocator
            .note_on_expr(transposed_note, velocity, trigger)
    }

    /// Handle a note off event.
    ///
    /// The note is checked against the key range and transposed to match note_on.
    pub fn note_off(&mut self, note: MidiNote) {
        // Check if note is within key range (same check as note_on)
        if !self.key_range.contains(note) {
            return;
        }

        // Apply same transpose as note_on
        if let Some(transposed_note) = self.transpose_note(note) {
            self.allocator.note_off(transposed_note);
        }
    }

    /// Release all notes.
    pub fn all_notes_off(&mut self) {
        self.allocator.all_notes_off();
    }

    /// Hard-reset all DSP state instantly: every voice (envelopes/filters/
    /// oscillator phase/per-voice delay lines), the instrument's effect chain
    /// (delay lines, reverb buffers), and the oversampling downsamplers'
    /// half-band FIR delay lines. Distinct from [`all_notes_off`](Self::all_notes_off),
    /// which only releases and lets tails ring — this returns the instrument to a
    /// clean slate for tail-proof isolation between offline renders.
    pub fn reset_dsp(&mut self) {
        self.allocator.reset_voices();
        self.effect_chain.reset();
        // Half-band FIR state carried by the 2x/4x oversampling downsamplers —
        // otherwise only reset on an oversampling-factor change, so it would
        // survive the reset and colour the next render's first samples.
        self.downsampler_l.reset();
        self.downsampler_r.reset();
    }

    /// Apply a transient automation override to a module, fanned out to every
    /// voice in the pool so all currently allocated voices reflect the
    /// automation value. The override is also written to the template
    /// `voice_graph`, which `clone_structure` copies — so voices later rebuilt
    /// from the template inherit it (the allocator reuses pooled voices for new
    /// notes, so this only matters on an explicit voice rebuild). The base
    /// parameter is never mutated. Real-time safe.
    pub fn apply_param_override(&mut self, module_id: crate::ModuleId, param: Param) {
        self.voice_graph.apply_param_override(module_id, param);
        for voice in self.allocator.voices_mut() {
            voice.apply_param_override(module_id, param);
        }
    }

    /// Clear all transient automation overrides on the template graph and every
    /// voice, reverting affected parameters to their base values. Called on
    /// transport stop. Real-time safe.
    pub fn clear_param_overrides(&mut self) {
        self.voice_graph.clear_param_overrides();
        for voice in self.allocator.voices_mut() {
            voice.clear_param_overrides();
        }
    }

    /// Apply a normalized (`0..1`) automation value to the **first** module of
    /// `module_type` in this instrument's graph, via the transient override
    /// path.
    ///
    /// The value is denormalized through the descriptor of the first parameter
    /// matching `is_target` (so the descriptor `range`/curve is the single
    /// source of truth for the mapping); `build` then constructs the concrete
    /// [`Param`] from the denormalized value. No-op if the module or a matching
    /// parameter is absent. The base parameter is never mutated.
    ///
    /// Real-time safe: reads the cached descriptor (no allocation) and applies
    /// through [`Self::apply_param_override`].
    pub fn apply_normalized_override(
        &mut self,
        module_type: ModuleType,
        is_target: impl Fn(&Param) -> bool,
        build: impl Fn(f32) -> Param,
        normalized: NormalizedValue,
    ) {
        let Some(module_id) = self.voice_graph.find_module_by_type(module_type) else {
            return;
        };
        let Some(value) = self.voice_graph.module_descriptor(module_id).and_then(|d| {
            d.parameters
                .iter()
                .find(|p| is_target(&p.id))
                .map(|p| p.denormalize(normalized.as_f32()))
        }) else {
            return;
        };
        self.apply_param_override(module_id, build(value));
    }

    /// Apply a normalized (`0..1`) automation value to a parameter on a
    /// specific module, identified positionally by `module_type` + `instance`
    /// (the [`AutomationTarget::Module`](synth_sequencer::AutomationTarget) form)
    /// and by the descriptor `type_id` string `param_id` (e.g. `"cutoff"`).
    ///
    /// Resolves the parameter's cached descriptor, denormalizes through its
    /// range/curve, and rebuilds the concrete [`Param`] with
    /// [`Param::with_f32`](synth_core::Param::with_f32) before applying it via
    /// the transient override path. No-op if the module or parameter is absent.
    /// Real-time safe (cached descriptor read, no allocation).
    pub fn apply_module_param_override(
        &mut self,
        module_type: ModuleType,
        instance: u16,
        param_id: &str,
        normalized: NormalizedValue,
    ) {
        let module_id = crate::ModuleId::new(module_type, instance);
        let Some(param) = self.voice_graph.module_descriptor(module_id).and_then(|d| {
            d.parameters
                .iter()
                // Only automatable (continuous, RT-safe, non-enum) params: guards
                // against malformed/legacy targets whose `type_id` names a choice
                // param, where `with_f32` would synthesize a garbage enum value.
                .find(|p| p.type_id == param_id && p.is_automatable())
                .map(|p| p.id.with_f32(p.denormalize(normalized.as_f32())))
        }) else {
            return;
        };
        self.apply_param_override(module_id, param);
    }

    /// Kill all voices immediately.
    pub fn panic(&mut self) {
        self.allocator.panic();
    }

    /// Process all active voices in this instrument into its channel bus.
    ///
    /// This method:
    /// 1. Processes each active voice through its signal chain
    /// 2. Sums voice output into stereo buffers
    /// 3. Interleaves into `effect_buffer` and processes through `effect_chain`
    ///
    /// The result is left in `effect_buffer` (the channel's post-effect,
    /// **pre-fader** signal, exposed via [`Self::last_output_interleaved`]).
    /// The channel fader/pan ([`Self::stereo_gain`]) and the mix into the
    /// master buffer are applied later by the engine's bus stage
    /// (`SynthEngine::mix_channel_busses`) — the instrument is a pure sound
    /// source and never touches the master mix directly.
    ///
    /// # Arguments
    /// * `context` - Processing context with sample rate, buffer size, etc.
    ///
    /// # Returns
    /// The number of active voices processed.
    #[allow(clippy::too_many_lines)]
    pub fn process(
        &mut self,
        context: &ProcessContext,
        spatial_ctx: Option<&SpatialContext>,
        spatial_bank: &mut SpatialVoiceBank,
    ) -> u32 {
        if self.mute_state.is_muted() {
            return 0;
        }

        let samples = context.samples;
        let sample_count = samples.as_usize();
        let mut active_count = 0u32;

        // Determine oversampled parameters
        let os_factor = self.oversampling.factor();
        let os_count = sample_count * os_factor;

        // Ensure internal buffers are sized correctly
        self.voice_left.resize(sample_count);
        self.voice_right.resize(sample_count);
        self.effect_buffer.resize(sample_count * 2); // Interleaved stereo

        // Clear instrument buffers for accumulation
        self.voice_left.clear();
        self.voice_right.clear();

        // Take pre-allocated temp buffers out temporarily to avoid borrow conflicts
        // (Default::default() creates empty Vec with no allocation)
        let mut temp_left = std::mem::take(&mut self.temp_voice_left);
        let mut temp_right = std::mem::take(&mut self.temp_voice_right);

        if os_factor > 1 {
            // === OVERSAMPLED PATH ===
            // Process voices at higher sample rate, then downsample

            let os_sample_rate = SampleRate::new(context.sample_rate.as_f32() * os_factor as f32);
            let os_samples = SampleCount::new(os_count);
            let os_context = ProcessContext {
                sample_rate: os_sample_rate,
                samples: os_samples,
                ..*context
            };

            // Resize oversampled buffers (no allocation if capacity sufficient)
            self.os_voice_left.resize(os_count);
            self.os_voice_right.resize(os_count);
            self.os_voice_left.clear();
            self.os_voice_right.clear();

            // Resize temp buffers for oversampled processing
            temp_left.resize(os_count);
            temp_right.resize(os_count);

            // Process each voice at the oversampled rate
            for voice in self.allocator.voices_mut() {
                if !voice.is_active() {
                    continue;
                }

                active_count += 1;

                // Update glide and increment age (at original rate)
                let delta_time = Seconds::new(sample_count as f32 / context.sample_rate.as_f32());
                voice.glide.update(delta_time);
                voice.advance_vibrato(delta_time);
                voice.age = voice.age + samples;

                // Handle stealing fade-out completion
                if let VoiceState::Stealing {
                    fade_counter,
                    pending_note,
                    ..
                } = voice.state
                    && fade_counter.as_usize() == 0
                {
                    if let Some((note, velocity, time)) = pending_note {
                        // Fade-out done: trigger the pending note on this voice
                        voice.reset();
                        voice.note_on(note, velocity, time);
                    } else {
                        voice.reset();
                        continue;
                    }
                }

                // Clear temp buffers for this voice
                temp_left.clear();
                temp_right.clear();

                // Create per-voice context with voice_start_time for sweep arbitration
                let voice_context = ProcessContext {
                    voice_start_time: voice.state.start_time().unwrap_or(SamplePosition::ZERO),
                    ..os_context
                };

                // Process the voice signal chain at oversampled rate
                voice.process_audio(&mut temp_left, &mut temp_right, &voice_context);

                // Release finished voices: if output is silent, reclaim the voice
                if matches!(voice.state, VoiceState::Releasing { .. }) {
                    let mut peak: f32 = 0.0;
                    for i in 0..os_count {
                        let l = temp_left[i].abs();
                        let r = temp_right[i].abs();
                        if l > peak {
                            peak = l;
                        }
                        if r > peak {
                            peak = r;
                        }
                    }
                    if peak < 1e-6 {
                        voice.reset();
                        continue;
                    }
                }

                // Apply stealing fade-out if needed (at oversampled rate)
                if let VoiceState::Stealing {
                    fade_counter,
                    fade_total,
                    pending_note,
                } = voice.state
                {
                    let fc = fade_counter.as_usize();
                    let ft = fade_total.as_usize();
                    // Scale fade counters to oversampled domain
                    let os_fade_counter = fc * os_factor;
                    let os_fade_total = ft * os_factor;
                    let fade_samples = os_fade_counter.min(os_count);
                    for i in 0..os_count {
                        let fade = if i < fade_samples {
                            (os_fade_counter - i) as f32 / os_fade_total as f32
                        } else {
                            0.0
                        };
                        temp_left[i] *= fade;
                        temp_right[i] *= fade;
                    }
                    // Update the fade counter (at original rate)
                    let new_counter = SampleCount::new(fc.saturating_sub(sample_count));
                    voice.state = VoiceState::Stealing {
                        fade_counter: new_counter,
                        fade_total,
                        pending_note,
                    };
                }

                // Per-voice spatial capture (naive decimation from oversampled)
                if spatial_ctx.is_some()
                    && let Some(note) = voice.note()
                {
                    // Decimate oversampled voice data to original rate for spatial bank
                    let mut dec_left = [0.0f32; 1024];
                    let mut dec_right = [0.0f32; 1024];
                    for i in 0..sample_count {
                        dec_left[i] = temp_left[i * os_factor];
                        dec_right[i] = temp_right[i * os_factor];
                    }
                    spatial_bank.write_voice(
                        note,
                        &dec_left[..sample_count],
                        &dec_right[..sample_count],
                        SampleCount::new(sample_count),
                    );
                }

                // Sum into oversampled instrument buffers
                for i in 0..os_count {
                    self.os_voice_left[i] += temp_left[i];
                    self.os_voice_right[i] += temp_right[i];
                }
            }

            // Downsample oversampled buffers into voice_left/voice_right
            self.downsampler_l.process(
                &self.os_voice_left.as_slice()[..os_count],
                &mut self.voice_left.as_mut_slice()[..sample_count],
                self.oversampling,
            );
            self.downsampler_r.process(
                &self.os_voice_right.as_slice()[..os_count],
                &mut self.voice_right.as_mut_slice()[..sample_count],
                self.oversampling,
            );
        } else {
            // === NORMAL PATH (1x) — zero overhead ===

            // Resize temp buffers if needed (no allocation if capacity is sufficient)
            temp_left.resize(sample_count);
            temp_right.resize(sample_count);

            // Process each voice and sum into voice_left/voice_right
            for voice in self.allocator.voices_mut() {
                if !voice.is_active() {
                    continue;
                }

                active_count += 1;

                // Update glide and increment age
                let delta_time = Seconds::new(sample_count as f32 / context.sample_rate.as_f32());
                voice.glide.update(delta_time);
                voice.advance_vibrato(delta_time);
                voice.age = voice.age + samples;

                // Handle stealing fade-out completion
                if let VoiceState::Stealing {
                    fade_counter,
                    pending_note,
                    ..
                } = voice.state
                    && fade_counter.as_usize() == 0
                {
                    if let Some((note, velocity, time)) = pending_note {
                        // Fade-out done: trigger the pending note on this voice
                        voice.reset();
                        voice.note_on(note, velocity, time);
                    } else {
                        voice.reset();
                        continue;
                    }
                }

                // Clear temp buffers for this voice
                temp_left.clear();
                temp_right.clear();

                // Create per-voice context with voice_start_time for sweep arbitration
                let voice_context = ProcessContext {
                    voice_start_time: voice.state.start_time().unwrap_or(SamplePosition::ZERO),
                    ..*context
                };

                // Process the voice signal chain
                voice.process_audio(&mut temp_left, &mut temp_right, &voice_context);

                // Reclaim releasing voices once output is silent
                if matches!(voice.state, VoiceState::Releasing { .. }) {
                    let mut peak: f32 = 0.0;
                    for i in 0..sample_count {
                        let l = temp_left[i].abs();
                        let r = temp_right[i].abs();
                        if l > peak {
                            peak = l;
                        }
                        if r > peak {
                            peak = r;
                        }
                    }
                    if peak < 1e-6 {
                        voice.reset();
                        continue;
                    }
                }

                // Apply stealing fade-out if needed
                if let VoiceState::Stealing {
                    fade_counter,
                    fade_total,
                    pending_note,
                } = voice.state
                {
                    let fc = fade_counter.as_usize();
                    let ft = fade_total.as_usize();
                    let fade_samples = fc.min(sample_count);
                    for i in 0..sample_count {
                        let fade = if i < fade_samples {
                            (fc - i) as f32 / ft as f32
                        } else {
                            0.0
                        };
                        temp_left[i] *= fade;
                        temp_right[i] *= fade;
                    }
                    // Update the fade counter in the state
                    let new_counter = SampleCount::new(fc.saturating_sub(sample_count));
                    voice.state = VoiceState::Stealing {
                        fade_counter: new_counter,
                        fade_total,
                        pending_note,
                    };
                }

                // Per-voice spatial capture + dry panning
                if let Some(ctx) = spatial_ctx
                    && let Some(note) = voice.note()
                {
                    spatial_bank.write_voice(
                        note,
                        temp_left.as_slice(),
                        temp_right.as_slice(),
                        SampleCount::new(sample_count),
                    );
                    let pan = ctx.mapping.pan_for_note(
                        note,
                        ctx.room_length,
                        ctx.room_width,
                        ctx.room_height,
                        ctx.listener_x,
                    );
                    let pan_f = pan.as_f32();
                    let gain_l = ((1.0 - pan_f) * 0.5).sqrt();
                    let gain_r = ((1.0 + pan_f) * 0.5).sqrt();
                    for i in 0..sample_count {
                        temp_left[i] *= gain_l;
                        temp_right[i] *= gain_r;
                    }
                }

                // Sum into instrument buffers
                for i in 0..sample_count {
                    self.voice_left[i] += temp_left[i];
                    self.voice_right[i] += temp_right[i];
                }
            }
        }

        // Put pre-allocated temp buffers back (preserves capacity for next frame)
        self.temp_voice_left = temp_left;
        self.temp_voice_right = temp_right;

        // Advance allocator time
        self.allocator.advance_time(samples);

        // Interleave voice_left/voice_right into effect_buffer (L, R, L, R, ...)
        for i in 0..sample_count {
            self.effect_buffer[i * 2] = self.voice_left[i];
            self.effect_buffer[i * 2 + 1] = self.voice_right[i];
        }

        // Process through effect chain (modifies effect_buffer in place)
        self.effect_chain.process(&mut self.effect_buffer, context);

        // Feed post-effect signal to per-instrument visualizers
        self.effect_chain.process_visualizers(&self.effect_buffer);

        // The channel fader/pan and the mix into the master buffer are applied
        // by the engine's bus stage; `effect_buffer` now holds the finished
        // post-effect, pre-fader channel signal.
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
            .field("mute_state", &self.mute_state)
            .field("solo_state", &self.solo_state)
            .field("active_voices", &self.active_voice_count())
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
        let voice_id = instrument.note_on(MidiNote::C4, Velocity::new(0.8));
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

    #[test]
    fn test_solo() {
        let mut instrument = Instrument::new(InstrumentId::new(1), "Test");

        // Default is not soloed
        assert!(!instrument.is_solo());

        // Set solo
        instrument.set_solo(true);
        assert!(instrument.is_solo());

        // Unset solo
        instrument.set_solo(false);
        assert!(!instrument.is_solo());
    }

    #[test]
    fn test_soft_clip() {
        // Below threshold - unchanged
        assert!((super::soft_clip(0.5) - 0.5).abs() < 0.001);
        assert!((super::soft_clip(-0.5) - (-0.5)).abs() < 0.001);
        assert!((super::soft_clip(0.8) - 0.8).abs() < 0.001);

        // At/above threshold - compressed
        let clipped = super::soft_clip(1.5);
        assert!(clipped > 0.8); // Above threshold
        assert!(clipped < 1.0); // Below hard clip
        assert!(clipped > super::soft_clip(1.2)); // Monotonic

        // Negative side mirrors positive
        let neg_clipped = super::soft_clip(-1.5);
        assert!(neg_clipped < -0.8);
        assert!(neg_clipped > -1.0);
        assert!((clipped + neg_clipped).abs() < 0.001); // Symmetric

        // Very high input asymptotically approaches 1.0
        let extreme = super::soft_clip(2.0);
        assert!(extreme < 1.0);
        assert!(extreme > 0.9);
    }

    #[test]
    fn test_filter_cutoff_automation_denormalizes_and_reverts() {
        use synth_core::{FilterParam, Hertz, SampleCount, SampleRate};
        use synth_modules::{Filter, Oscillator};

        // A real draw -> sound -> stop -> revert cycle: an Osc -> Filter graph
        // (filter is the sink) processed directly.
        let mut inst = Instrument::new(InstrumentId::new(1), "test");
        let g = inst.voice_graph_mut();
        let osc_id = g.add_module(Box::new(Oscillator::new()));
        let flt_id = g.add_module(Box::new(Filter::new()));
        g.connect(osc_id, "out", flt_id, "in").unwrap();

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        // Run several warm-up blocks so the filter reaches steady state, then
        // measure the energy of the next block (avoids start/retune transients).
        fn settled_energy(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
            let mut out = AudioBuffer::new(256);
            for _ in 0..16 {
                graph.process(&mut out, ctx);
            }
            graph.process(&mut out, ctx);
            (0..256).map(|i| out[i] * out[i]).sum()
        }

        // Base cutoff is 1000 Hz: the 440 Hz sawtooth passes.
        let base = settled_energy(inst.voice_graph_mut(), &ctx);
        assert!(base > 1e-3, "expected audible base output, got {base}");

        // Normalized 0.0 denormalizes to the descriptor minimum (20 Hz, via the
        // logarithmic cutoff range), heavily attenuating the fundamental.
        inst.apply_normalized_override(
            ModuleType::Filter,
            |p| matches!(p, Param::Filter(FilterParam::Cutoff(_))),
            |v| Param::Filter(FilterParam::Cutoff(Hertz::new(v))),
            NormalizedValue::MIN,
        );
        let low = settled_energy(inst.voice_graph_mut(), &ctx);
        assert!(low < base * 0.25, "low-cutoff energy {low} vs base {base}");

        // The transport-stop path reverts to the base cutoff.
        inst.clear_param_overrides();
        let reverted = settled_energy(inst.voice_graph_mut(), &ctx);
        assert!(reverted > base * 0.5, "reverted {reverted} vs base {base}");
    }
}
