//! AWE engine - real-time room simulation processor.
//!
//! Fas 0: pass-through only. No DSP logic.

use crate::params::{AweParam, AweSnapshot};
use crate::room::{Material, RoomShape};

/// The Acoustic World Engine processor.
///
/// In Fas 0 this is a pass-through — the engine stores parameters
/// but does not modify audio. Future phases will add ISM, room modes,
/// and FDN-based late reverb.
pub struct AweEngine {
    enabled: bool,
    room: RoomShape,
    material: Material,
    snapshot: AweSnapshot,
    cached_sample_rate: f32,
}

impl AweEngine {
    /// Create a new AWE engine (disabled by default).
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            room: RoomShape::default(),
            material: Material::default(),
            snapshot: AweSnapshot::default(),
            cached_sample_rate: 48000.0,
        }
    }

    /// Process audio buffer (interleaved stereo).
    ///
    /// In Fas 0 this is a no-op (pass-through).
    pub fn process(&mut self, _buffer: &mut [f32], sample_rate: f32) {
        self.cached_sample_rate = sample_rate;
        // Fas 0: pass-through — no audio modification
    }

    /// Set a single parameter.
    pub fn set_param(&mut self, param: AweParam) {
        match param {
            AweParam::RoomShape(shape) => self.room = shape,
            AweParam::Material(mat) => self.material = mat,
            AweParam::SourcePos(pos) => self.snapshot.source_pos = pos,
            AweParam::ListenerPos(pos) => self.snapshot.listener_pos = pos,
            AweParam::DryWet(v) => self.snapshot.dry_wet = v,
            AweParam::EarlyLateBalance(v) => self.snapshot.early_late_balance = v,
            AweParam::ModesAmount(v) => self.snapshot.modes_amount = v,
            AweParam::FreqWarp(v) => self.snapshot.freq_warp = v,
            AweParam::ResonanceBoost(v) => self.snapshot.resonance_boost = v,
            AweParam::TailStretch(v) => self.snapshot.tail_stretch = v,
            AweParam::Enabled(v) => self.enabled = v,
        }
    }

    /// Apply a batch snapshot of numeric parameters.
    pub fn apply_snapshot(&mut self, snapshot: AweSnapshot) {
        self.snapshot = snapshot;
    }

    /// Check if the engine is enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the engine.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Get the current parameter snapshot.
    #[must_use]
    pub fn snapshot(&self) -> AweSnapshot {
        self.snapshot
    }

    /// Get the current room shape.
    #[must_use]
    pub fn room(&self) -> RoomShape {
        self.room
    }

    /// Get the current material.
    #[must_use]
    pub fn material(&self) -> Material {
        self.material
    }
}

impl Default for AweEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_new() {
        let engine = AweEngine::new();
        assert!(!engine.enabled());
    }

    #[test]
    fn test_engine_set_enabled() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        assert!(engine.enabled());
        engine.set_enabled(false);
        assert!(!engine.enabled());
    }

    #[test]
    fn test_engine_pass_through() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        let mut buffer = vec![0.5, -0.3, 0.1, 0.8];
        let original = buffer.clone();
        engine.process(&mut buffer, 48000.0);
        // Fas 0: buffer should be unmodified
        assert_eq!(buffer, original);
    }

    #[test]
    fn test_engine_set_param() {
        let mut engine = AweEngine::new();
        engine.set_param(AweParam::DryWet(0.7));
        assert!((engine.snapshot().dry_wet - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_engine_apply_snapshot() {
        let mut engine = AweEngine::new();
        let mut snap = AweSnapshot::default();
        snap.dry_wet = 0.8;
        snap.tail_stretch = 2.0;
        engine.apply_snapshot(snap);
        assert!((engine.snapshot().dry_wet - 0.8).abs() < 0.001);
        assert!((engine.snapshot().tail_stretch - 2.0).abs() < 0.001);
    }
}
