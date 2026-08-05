//! Instrument UI state for managing multiple instruments.
//!
//! Contains the `InstrumentUiState` type that mirrors engine instrument state
//! for display purposes in the GUI.

use super::patch_editor::PatchEditor;
use crate::patch::HexColor;
use synth_core::{BipolarValue, Gain, Semitones};
use synth_engine::InstrumentCategory;
use synth_engine::instrument::{InstrumentId, KeyRange, LearnState, MidiChannelSelection};

/// GUI state for a single instrument.
///
/// This mirrors the engine's Instrument state for display purposes.
/// Updates are sent to the engine via EngineCommands when values change.
/// Each instrument owns its own PatchEditor for independent visual graphs.
#[derive(Clone)]
pub struct InstrumentUiState {
    /// Unique identifier matching the engine's InstrumentId.
    pub id: InstrumentId,
    /// Display name for this instrument.
    pub name: String,
    /// MIDI channel this instrument responds to.
    pub channel: MidiChannelSelection,
    /// Output volume (0.0 = mute, 1.0 = unity).
    pub volume: Gain,
    /// Stereo pan position (-1.0 = left, 0.0 = center, +1.0 = right).
    pub pan: BipolarValue,
    /// Whether this instrument is muted (uses volume = 0 for soft mute).
    pub muted: bool,
    /// Whether this instrument is soloed.
    /// When any instrument is soloed, only soloed instruments produce sound.
    pub solo: bool,
    /// Stored volume when muted (to restore on unmute).
    stored_volume: Gain,
    /// The patch editor for this instrument's visual module graph.
    pub patch_editor: PatchEditor,
    /// Key range for keyboard splitting (which notes this instrument responds to).
    pub key_range: KeyRange,
    /// Transpose offset in semitones (-24 to +24).
    pub transpose: Semitones,
    /// MIDI learn state for key range assignment.
    pub learn_state: LearnState,
    /// Oversampling factor (Off/2x/4x).
    pub oversampling: synth_dsp::OversamplingFactor,
    /// Category for visualization/routing (Drums, Bass, Lead, ...).
    pub category: InstrumentCategory,
    /// Free-text description shown in the instrument edit window.
    pub description: String,
    /// Optional accent color as hex string (e.g. "#FF8800FF").
    pub color: Option<HexColor>,
    /// Voice allocation mode (Poly / Mono / Legato / Unison).
    pub allocation_mode: synth_engine::voice_allocator::AllocationMode,
    /// Strategy for stealing voices when all are busy.
    pub stealing_strategy: synth_engine::voice_allocator::StealingStrategy,
    /// Total unison detune spread (cents), used in `Unison` allocation mode.
    pub unison_detune: synth_core::Cents,
    /// Unison stereo spread (0..1), used in `Unison` allocation mode.
    pub unison_spread: synth_core::NormalizedValue,
    /// Maximum polyphony for this instrument.
    pub max_voices: synth_core::VoiceCount,
    /// Velocity → amplitude sensitivity (0 = constant, 1 = full dynamic).
    pub velocity_amp_sensitivity: synth_core::NormalizedValue,
    /// Velocity → filter cutoff sensitivity (0 = none, 1 = full).
    pub velocity_filter_sensitivity: synth_core::NormalizedValue,
    /// Sidechain source instrument id, or `None` when no sidechain is
    /// configured.
    pub sidechain_source_id: Option<InstrumentId>,
    /// Patch-level description. Empty string means no description set.
    pub patch_description: String,
}

impl Default for InstrumentUiState {
    fn default() -> Self {
        Self {
            id: InstrumentId::FIRST,
            name: "Instrument 1".to_string(),
            channel: MidiChannelSelection::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            solo: false,
            stored_volume: Gain::UNITY,
            patch_editor: PatchEditor::new(),
            key_range: KeyRange::FULL,
            transpose: Semitones::ZERO,
            learn_state: LearnState::Idle,
            oversampling: synth_dsp::OversamplingFactor::default(),
            category: InstrumentCategory::default(),
            description: String::new(),
            color: None,
            allocation_mode: synth_engine::voice_allocator::AllocationMode::default(),
            stealing_strategy: synth_engine::voice_allocator::StealingStrategy::default(),
            unison_detune: synth_core::Cents::new(10.0),
            unison_spread: synth_core::NormalizedValue::MIN,
            max_voices: synth_core::VoiceCount::OCTO,
            velocity_amp_sensitivity: synth_core::NormalizedValue::MAX,
            velocity_filter_sensitivity: synth_core::NormalizedValue::MIN,
            sidechain_source_id: None,
            patch_description: String::new(),
        }
    }
}

impl InstrumentUiState {
    /// Create a new instrument with the given ID and name.
    pub fn new(id: InstrumentId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            channel: MidiChannelSelection::CH1,
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            muted: false,
            solo: false,
            stored_volume: Gain::UNITY,
            patch_editor: PatchEditor::new(),
            key_range: KeyRange::FULL,
            transpose: Semitones::ZERO,
            learn_state: LearnState::Idle,
            oversampling: synth_dsp::OversamplingFactor::default(),
            category: InstrumentCategory::default(),
            description: String::new(),
            color: None,
            allocation_mode: synth_engine::voice_allocator::AllocationMode::default(),
            stealing_strategy: synth_engine::voice_allocator::StealingStrategy::default(),
            unison_detune: synth_core::Cents::new(10.0),
            unison_spread: synth_core::NormalizedValue::MIN,
            max_voices: synth_core::VoiceCount::OCTO,
            velocity_amp_sensitivity: synth_core::NormalizedValue::MAX,
            velocity_filter_sensitivity: synth_core::NormalizedValue::MIN,
            sidechain_source_id: None,
            patch_description: String::new(),
        }
    }

    /// Create a new instrument with a specific MIDI channel.
    pub fn with_channel(mut self, channel: MidiChannelSelection) -> Self {
        self.channel = channel;
        self
    }

    /// Set volume and update the stored volume for mute/unmute.
    pub fn set_volume(&mut self, volume: Gain) {
        self.volume = volume;
        self.stored_volume = volume;
    }

    /// Toggle mute state (soft mute via volume).
    pub fn toggle_mute(&mut self) -> Gain {
        if self.muted {
            // Unmute: restore previous volume
            self.muted = false;
            self.volume = self.stored_volume;
        } else {
            // Mute: store current volume and set to 0
            self.muted = true;
            self.stored_volume = self.volume;
            self.volume = Gain::MUTE;
        }
        self.volume
    }
}

/// Every persisted property of an instrument, captured at one instant.
///
/// A snapshot rather than one undo variant per property: the patch bar edits
/// around fifteen of them, they share an inverse (swap the two sides) and an
/// apply path (write the fields back, then push them to the engine), and a
/// property added later is covered without anyone remembering to extend the
/// undo layer.
///
/// Excludes `patch_editor` (the module graph, which the engine owns and which
/// has its own undo entries) and `learn_state` (transient MIDI-learn UI).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstrumentSettings {
    pub name: String,
    pub channel: MidiChannelSelection,
    pub volume: Gain,
    pub pan: BipolarValue,
    pub muted: bool,
    pub solo: bool,
    pub key_range: KeyRange,
    pub transpose: Semitones,
    pub oversampling: synth_dsp::OversamplingFactor,
    pub category: InstrumentCategory,
    pub description: String,
    pub color: Option<HexColor>,
    pub allocation_mode: synth_engine::voice_allocator::AllocationMode,
    pub stealing_strategy: synth_engine::voice_allocator::StealingStrategy,
    pub unison_detune: synth_core::Cents,
    pub unison_spread: synth_core::NormalizedValue,
    pub max_voices: synth_core::VoiceCount,
    pub velocity_amp_sensitivity: synth_core::NormalizedValue,
    pub velocity_filter_sensitivity: synth_core::NormalizedValue,
    pub sidechain_source_id: Option<InstrumentId>,
    pub patch_description: String,
}

impl InstrumentUiState {
    /// Capture this instrument's persisted properties.
    pub(crate) fn settings(&self) -> InstrumentSettings {
        InstrumentSettings {
            name: self.name.clone(),
            channel: self.channel,
            volume: self.volume,
            pan: self.pan,
            muted: self.muted,
            solo: self.solo,
            key_range: self.key_range,
            transpose: self.transpose,
            oversampling: self.oversampling,
            category: self.category,
            description: self.description.clone(),
            color: self.color.clone(),
            allocation_mode: self.allocation_mode,
            stealing_strategy: self.stealing_strategy,
            unison_detune: self.unison_detune,
            unison_spread: self.unison_spread,
            max_voices: self.max_voices,
            velocity_amp_sensitivity: self.velocity_amp_sensitivity,
            velocity_filter_sensitivity: self.velocity_filter_sensitivity,
            sidechain_source_id: self.sidechain_source_id,
            patch_description: self.patch_description.clone(),
        }
    }

    /// Write captured properties back, leaving the module graph untouched.
    pub(crate) fn apply_settings(&mut self, settings: &InstrumentSettings) {
        self.name = settings.name.clone();
        self.channel = settings.channel;
        self.volume = settings.volume;
        self.pan = settings.pan;
        self.muted = settings.muted;
        self.solo = settings.solo;
        self.key_range = settings.key_range;
        self.transpose = settings.transpose;
        self.oversampling = settings.oversampling;
        self.category = settings.category;
        self.description = settings.description.clone();
        self.color = settings.color.clone();
        self.allocation_mode = settings.allocation_mode;
        self.stealing_strategy = settings.stealing_strategy;
        self.unison_detune = settings.unison_detune;
        self.unison_spread = settings.unison_spread;
        self.max_voices = settings.max_voices;
        self.velocity_amp_sensitivity = settings.velocity_amp_sensitivity;
        self.velocity_filter_sensitivity = settings.velocity_filter_sensitivity;
        self.sidechain_source_id = settings.sidechain_source_id;
        self.patch_description = settings.patch_description.clone();
    }
}
