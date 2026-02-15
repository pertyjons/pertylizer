//! AWE parameters, snapshots, and serializable state.

use serde::{Deserialize, Serialize};
use synth_core::{BipolarValue, Hertz, NormalizedValue};

use crate::room::{Material, RoomShape};
use crate::spatial_voice::NotePositionMapping;
use crate::types::{Meters, Position3, StretchFactor};

/// Target for an AWE-internal LFO modulation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AweLfoTarget {
    /// Modulate room length.
    RoomLength,
    /// Modulate room width.
    RoomWidth,
    /// Modulate source X position.
    #[default]
    SourceX,
    /// Modulate source Y position.
    SourceY,
    /// Modulate listener X position.
    ListenerX,
    /// Modulate listener Y position.
    ListenerY,
    /// Modulate dry/wet mix.
    DryWet,
    /// Modulate frequency warp.
    FreqWarp,
    /// Modulate early/late balance.
    EarlyLate,
    /// Modulate modes amount.
    ModesAmount,
    /// Modulate resonance boost.
    ResonanceBoost,
    /// Modulate tail stretch.
    TailStretch,
    /// Modulate portal amount.
    PortalAmount,
}

/// State of one AWE-internal LFO for persistence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AweLfoState {
    /// LFO rate in Hz (0.01 – 20.0).
    pub rate: Hertz,
    /// Modulation amount (0.0 – 1.0).
    pub amount: NormalizedValue,
    /// Modulation target.
    pub target: AweLfoTarget,
}

impl Default for AweLfoState {
    fn default() -> Self {
        Self {
            rate: Hertz::new(0.5),
            amount: NormalizedValue::MIN,
            target: AweLfoTarget::default(),
        }
    }
}

/// Parameters that can be set on the AWE engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AweParam {
    /// Set the room shape / dimensions.
    RoomShape(RoomShape),
    /// Set the wall material.
    Material(Material),
    /// Source position (x, y, z) in meters.
    SourcePos(Position3),
    /// Listener position (x, y, z) in meters.
    ListenerPos(Position3),
    /// Dry/wet mix (0.0 = fully dry, 1.0 = fully wet).
    DryWet(NormalizedValue),
    /// Early/late reflection balance (0.0 = early only, 1.0 = late only).
    EarlyLateBalance(NormalizedValue),
    /// Room mode resonance amount (0.0 = off, 1.0 = full).
    ModesAmount(NormalizedValue),
    /// Frequency warping for non-uniform mode spacing.
    FreqWarp(BipolarValue),
    /// Resonance boost for room modes.
    ResonanceBoost(NormalizedValue),
    /// Tail stretch factor (1.0 = natural, >1.0 = longer).
    TailStretch(StretchFactor),
    /// Portal amount (0.0 = off, 1.0 = full portal effect).
    PortalAmount(NormalizedValue),
    /// Enable/disable the AWE engine.
    Enabled(bool),
    /// Enable/disable per-voice spatial processing.
    SpatialEnabled(bool),
    /// Set the note-to-position mapping for per-voice spatial.
    NoteMapping(NotePositionMapping),
    /// Set LFO 1 rate in Hz.
    Lfo1Rate(Hertz),
    /// Set LFO 1 amount (0.0–1.0).
    Lfo1Amount(NormalizedValue),
    /// Set LFO 1 target.
    Lfo1Target(AweLfoTarget),
    /// Set LFO 2 rate in Hz.
    Lfo2Rate(Hertz),
    /// Set LFO 2 amount (0.0–1.0).
    Lfo2Amount(NormalizedValue),
    /// Set LFO 2 target.
    Lfo2Target(AweLfoTarget),
    /// Set LFO 3 rate in Hz.
    Lfo3Rate(Hertz),
    /// Set LFO 3 amount (0.0–1.0).
    Lfo3Amount(NormalizedValue),
    /// Set LFO 3 target.
    Lfo3Target(AweLfoTarget),
    /// Set LFO 4 rate in Hz.
    Lfo4Rate(Hertz),
    /// Set LFO 4 amount (0.0–1.0).
    Lfo4Amount(NormalizedValue),
    /// Set LFO 4 target.
    Lfo4Target(AweLfoTarget),
}

/// Snapshot of all numeric AWE parameters for batch-updating.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AweSnapshot {
    /// Dry/wet mix.
    pub dry_wet: NormalizedValue,
    /// Early/late reflection balance.
    pub early_late_balance: NormalizedValue,
    /// Room mode amount.
    pub modes_amount: NormalizedValue,
    /// Frequency warping.
    pub freq_warp: BipolarValue,
    /// Resonance boost.
    pub resonance_boost: NormalizedValue,
    /// Tail stretch.
    pub tail_stretch: StretchFactor,
    /// Portal amount (0.0 = off, 1.0 = full).
    #[serde(default)]
    pub portal_amount: NormalizedValue,
    /// Source position.
    pub source_pos: Position3,
    /// Listener position.
    pub listener_pos: Position3,
    /// Whether per-voice spatial is enabled.
    #[serde(default)]
    pub spatial_enabled: bool,
    /// Note-to-position mapping for per-voice spatial.
    #[serde(default)]
    pub note_mapping: NotePositionMapping,
    /// LFO 1 state.
    #[serde(default)]
    pub lfo1: AweLfoState,
    /// LFO 2 state.
    #[serde(default)]
    pub lfo2: AweLfoState,
    /// LFO 3 state.
    #[serde(default)]
    pub lfo3: AweLfoState,
    /// LFO 4 state.
    #[serde(default)]
    pub lfo4: AweLfoState,
}

impl Default for AweSnapshot {
    fn default() -> Self {
        Self {
            dry_wet: NormalizedValue::new(0.3),
            early_late_balance: NormalizedValue::new(0.5),
            modes_amount: NormalizedValue::new(0.5),
            freq_warp: BipolarValue::new(0.0),
            resonance_boost: NormalizedValue::new(0.0),
            tail_stretch: StretchFactor::new(1.0),
            portal_amount: NormalizedValue::new(0.0),
            source_pos: Position3::new(Meters::new(2.0), Meters::new(2.5), Meters::new(1.5)),
            listener_pos: Position3::new(Meters::new(6.0), Meters::new(2.5), Meters::new(1.5)),
            spatial_enabled: false,
            note_mapping: NotePositionMapping::Off,
            lfo1: AweLfoState::default(),
            lfo2: AweLfoState {
                target: AweLfoTarget::SourceY,
                ..AweLfoState::default()
            },
            lfo3: AweLfoState {
                target: AweLfoTarget::EarlyLate,
                ..AweLfoState::default()
            },
            lfo4: AweLfoState {
                target: AweLfoTarget::ModesAmount,
                ..AweLfoState::default()
            },
        }
    }
}

/// Serializable AWE state for persistence in patches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AweState {
    /// Whether AWE is enabled.
    pub enabled: bool,
    /// Room geometry.
    #[serde(default)]
    pub room: RoomShape,
    /// Wall material.
    #[serde(default)]
    pub material: Material,
    /// Per-voice spatial enabled.
    #[serde(default)]
    pub spatial_enabled: bool,
    /// Note-to-position mapping.
    #[serde(default)]
    pub note_mapping: NotePositionMapping,
    /// Numeric parameter snapshot.
    #[serde(default)]
    pub snapshot: AweSnapshot,
}

impl AweState {
    /// Convert to a snapshot for the engine.
    #[must_use]
    pub fn to_snapshot(&self) -> AweSnapshot {
        self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_default() {
        let snap = AweSnapshot::default();
        assert!((snap.dry_wet.as_f32() - 0.3).abs() < 0.001);
        assert!((snap.tail_stretch.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_state_round_trip() {
        let state = AweState::default();
        let json = serde_json::to_string(&state).unwrap();
        let parsed: AweState = serde_json::from_str(&json).unwrap();
        assert_eq!(state.enabled, parsed.enabled);
        assert!((state.snapshot.dry_wet.as_f32() - parsed.snapshot.dry_wet.as_f32()).abs() < 0.001);
    }
}
