//! AWE parameters, snapshots, and serializable state.

use serde::{Deserialize, Serialize};

use crate::room::{Material, RoomShape};

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
    pub rate: f32,
    /// Modulation amount (0.0 – 1.0).
    pub amount: f32,
    /// Modulation target.
    pub target: AweLfoTarget,
}

impl Default for AweLfoState {
    fn default() -> Self {
        Self {
            rate: 0.5,
            amount: 0.0,
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
    SourcePos([f32; 3]),
    /// Listener position (x, y, z) in meters.
    ListenerPos([f32; 3]),
    /// Dry/wet mix (0.0 = fully dry, 1.0 = fully wet).
    DryWet(f32),
    /// Early/late reflection balance (0.0 = early only, 1.0 = late only).
    EarlyLateBalance(f32),
    /// Room mode resonance amount (0.0 = off, 1.0 = full).
    ModesAmount(f32),
    /// Frequency warping for non-uniform mode spacing.
    FreqWarp(f32),
    /// Resonance boost for room modes.
    ResonanceBoost(f32),
    /// Tail stretch factor (1.0 = natural, >1.0 = longer).
    TailStretch(f32),
    /// Portal amount (0.0 = off, 1.0 = full portal effect).
    PortalAmount(f32),
    /// Enable/disable the AWE engine.
    Enabled(bool),
    /// Set LFO 1 rate in Hz.
    Lfo1Rate(f32),
    /// Set LFO 1 amount (0.0–1.0).
    Lfo1Amount(f32),
    /// Set LFO 1 target.
    Lfo1Target(AweLfoTarget),
    /// Set LFO 2 rate in Hz.
    Lfo2Rate(f32),
    /// Set LFO 2 amount (0.0–1.0).
    Lfo2Amount(f32),
    /// Set LFO 2 target.
    Lfo2Target(AweLfoTarget),
    /// Set LFO 3 rate in Hz.
    Lfo3Rate(f32),
    /// Set LFO 3 amount (0.0–1.0).
    Lfo3Amount(f32),
    /// Set LFO 3 target.
    Lfo3Target(AweLfoTarget),
    /// Set LFO 4 rate in Hz.
    Lfo4Rate(f32),
    /// Set LFO 4 amount (0.0–1.0).
    Lfo4Amount(f32),
    /// Set LFO 4 target.
    Lfo4Target(AweLfoTarget),
}

/// Snapshot of all numeric AWE parameters for batch-updating.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AweSnapshot {
    /// Dry/wet mix.
    pub dry_wet: f32,
    /// Early/late reflection balance.
    pub early_late_balance: f32,
    /// Room mode amount.
    pub modes_amount: f32,
    /// Frequency warping.
    pub freq_warp: f32,
    /// Resonance boost.
    pub resonance_boost: f32,
    /// Tail stretch.
    pub tail_stretch: f32,
    /// Portal amount (0.0 = off, 1.0 = full).
    #[serde(default)]
    pub portal_amount: f32,
    /// Source position.
    pub source_pos: [f32; 3],
    /// Listener position.
    pub listener_pos: [f32; 3],
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
            dry_wet: 0.3,
            early_late_balance: 0.5,
            modes_amount: 0.5,
            freq_warp: 0.0,
            resonance_boost: 0.0,
            tail_stretch: 1.0,
            portal_amount: 0.0,
            source_pos: [2.0, 2.5, 1.5],
            listener_pos: [6.0, 2.5, 1.5],
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
        assert!((snap.dry_wet - 0.3).abs() < 0.001);
        assert!((snap.tail_stretch - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_state_round_trip() {
        let state = AweState::default();
        let json = serde_json::to_string(&state).unwrap();
        let parsed: AweState = serde_json::from_str(&json).unwrap();
        assert_eq!(state.enabled, parsed.enabled);
        assert!((state.snapshot.dry_wet - parsed.snapshot.dry_wet).abs() < 0.001);
    }
}
