//! Application state types for the synthesizer GUI.
//!
//! Contains state structures for master effects, navigation, and other UI elements.

use crate::engine::ModuleId;
use crate::gui::patch_editor::EffectType;
use serde::{Deserialize, Serialize};

// Re-export TrackerViewState for convenience
pub use crate::sequencer::view::TrackerViewState;

// ============================================================================
// NAVIGATION STATE
// ============================================================================

/// Main application view (tab-based navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppView {
    /// Instrument rack and patch editor view (default).
    #[default]
    Rack,
    /// Pattern/song sequencer view.
    Sequencer,
    /// Mixer view with channel strips.
    Mixer,
}

impl AppView {
    /// Get the display icon for this view.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Rack => "🔌",
            Self::Sequencer => "🎹",
            Self::Mixer => "🎚",
        }
    }

    /// Get the display label for this view.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rack => "Rack",
            Self::Sequencer => "Sequencer",
            Self::Mixer => "Mixer",
        }
    }
}

/// Top panel drawer state (expandable panels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TopPanel {
    /// No panel open.
    #[default]
    None,
    /// MIDI settings panel.
    Midi,
    /// Engine/audio settings panel.
    Engine,
}

/// Stored parameter values for each effect type.
#[derive(Clone)]
pub enum MasterEffectParams {
    Compressor {
        threshold: f32, // dB: -60 to 0
        ratio: f32,     // 1:1 to 20:1
        attack: f32,    // ms: 0.1 to 100
        release: f32,   // ms: 10 to 1000
        makeup: f32,    // dB: 0 to 24
        mix: f32,       // 0-1
    },
    Eq {
        low_gain: f32,  // dB: -12 to 12
        mid_gain: f32,  // dB: -12 to 12
        high_gain: f32, // dB: -12 to 12
        mix: f32,       // 0-1
    },
    Reverb {
        room_size: f32, // 0-1
        damping: f32,   // 0-1
        width: f32,     // 0-1
        mix: f32,       // 0-1
    },
    Delay {
        time: f32,     // seconds: 0.01 to 2.0
        feedback: f32, // 0-1
        mix: f32,      // 0-1
    },
    Chorus {
        rate: f32,  // Hz: 0.1 to 5.0
        depth: f32, // 0-1
        mix: f32,   // 0-1
    },
    Phaser {
        rate: f32,     // Hz: 0.1 to 5.0
        depth: f32,    // 0-1
        feedback: f32, // -1 to 1
        mix: f32,      // 0-1
    },
    Flanger {
        rate: f32,     // Hz: 0.1 to 5.0
        depth: f32,    // 0-1
        feedback: f32, // -1 to 1
        mix: f32,      // 0-1
    },
    Distortion {
        drive: f32, // 0-1
        tone: f32,  // 0-1
        mix: f32,   // 0-1
    },
}

impl MasterEffectParams {
    /// Create default parameters for an effect type.
    pub fn new(effect_type: EffectType) -> Self {
        match effect_type {
            EffectType::Compressor => Self::Compressor {
                threshold: -20.0,
                ratio: 4.0,
                attack: 10.0,
                release: 100.0,
                makeup: 0.0,
                mix: 1.0,
            },
            EffectType::Eq => Self::Eq {
                low_gain: 0.0,
                mid_gain: 0.0,
                high_gain: 0.0,
                mix: 1.0,
            },
            EffectType::Reverb => Self::Reverb {
                room_size: 0.5,
                damping: 0.5,
                width: 1.0,
                mix: 0.3,
            },
            EffectType::Delay => Self::Delay {
                time: 0.25,
                feedback: 0.4,
                mix: 0.3,
            },
            EffectType::Chorus => Self::Chorus {
                rate: 1.0,
                depth: 0.5,
                mix: 0.5,
            },
            EffectType::Phaser => Self::Phaser {
                rate: 0.5,
                depth: 0.5,
                feedback: 0.3,
                mix: 0.5,
            },
            EffectType::Flanger => Self::Flanger {
                rate: 0.3,
                depth: 0.5,
                feedback: 0.3,
                mix: 0.5,
            },
            EffectType::Distortion => Self::Distortion {
                drive: 0.5,
                tone: 0.5,
                mix: 0.5,
            },
        }
    }
}

/// UI state for a master effect in the effects chain.
#[derive(Clone)]
pub struct MasterEffectUiState {
    /// Module ID for this effect
    pub id: ModuleId,
    /// Effect type
    pub effect_type: EffectType,
    /// Whether the panel is expanded (showing parameters)
    pub expanded: bool,
    /// Whether the effect is bypassed
    pub bypassed: bool,
    /// Current parameter values
    pub params: MasterEffectParams,
}

impl MasterEffectUiState {
    /// Create a new effect UI state with default parameters.
    pub fn new(id: ModuleId, effect_type: EffectType) -> Self {
        Self {
            id,
            effect_type,
            expanded: true, // Start expanded so user can see parameters
            bypassed: false,
            params: MasterEffectParams::new(effect_type),
        }
    }

    /// Get a display name for this effect
    pub fn display_name(&self) -> &'static str {
        match self.effect_type {
            EffectType::Compressor => "Compressor",
            EffectType::Eq => "EQ",
            EffectType::Reverb => "Reverb",
            EffectType::Delay => "Delay",
            EffectType::Chorus => "Chorus",
            EffectType::Phaser => "Phaser",
            EffectType::Flanger => "Flanger",
            EffectType::Distortion => "Distortion",
        }
    }
}
