//! Semantic state enums to eliminate "Boolean Blindness".
//!
//! These enums replace primitive `bool` flags with self-documenting types
//! that make code intention clear at the call site.
//!
//! # Example
//!
//! ```ignore
//! // Before (Boolean Blindness):
//! module.set_muted(true);  // What does 'true' mean here?
//!
//! // After (Self-documenting):
//! module.set_mute_state(MuteState::Muted);  // Crystal clear!
//! ```

use serde::{Deserialize, Serialize};

// ============================================================================
// ENABLE STATE
// ============================================================================

/// Whether something is enabled or disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EnableState {
    /// Enabled and active.
    #[default]
    Enabled,
    /// Disabled and inactive.
    Disabled,
}

impl EnableState {
    /// Check if enabled.
    #[inline]
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Check if disabled.
    #[inline]
    #[must_use]
    pub const fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }

    /// Toggle between enabled and disabled.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Enabled => Self::Disabled,
            Self::Disabled => Self::Enabled,
        }
    }
}

impl From<bool> for EnableState {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

impl From<EnableState> for bool {
    fn from(state: EnableState) -> Self {
        state.is_enabled()
    }
}

// ============================================================================
// MUTE STATE
// ============================================================================

/// Whether audio output is muted or unmuted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MuteState {
    /// Audio is playing normally.
    #[default]
    Unmuted,
    /// Audio is silenced.
    Muted,
}

impl MuteState {
    /// Check if muted.
    #[inline]
    #[must_use]
    pub const fn is_muted(self) -> bool {
        matches!(self, Self::Muted)
    }

    /// Check if unmuted.
    #[inline]
    #[must_use]
    pub const fn is_unmuted(self) -> bool {
        matches!(self, Self::Unmuted)
    }

    /// Toggle between muted and unmuted.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Muted => Self::Unmuted,
            Self::Unmuted => Self::Muted,
        }
    }
}

impl From<bool> for MuteState {
    fn from(muted: bool) -> Self {
        if muted { Self::Muted } else { Self::Unmuted }
    }
}

impl From<MuteState> for bool {
    fn from(state: MuteState) -> Self {
        state.is_muted()
    }
}

// ============================================================================
// SOLO STATE
// ============================================================================

/// Whether a module is soloed (only this module plays).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SoloState {
    /// Normal playback (respects mute state).
    #[default]
    Normal,
    /// Soloed - this module plays even if others are muted.
    Solo,
}

impl SoloState {
    /// Check if soloed.
    #[inline]
    #[must_use]
    pub const fn is_solo(self) -> bool {
        matches!(self, Self::Solo)
    }

    /// Check if normal (not soloed).
    #[inline]
    #[must_use]
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Toggle between normal and solo.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Normal => Self::Solo,
            Self::Solo => Self::Normal,
        }
    }
}

impl From<bool> for SoloState {
    fn from(solo: bool) -> Self {
        if solo { Self::Solo } else { Self::Normal }
    }
}

impl From<SoloState> for bool {
    fn from(state: SoloState) -> Self {
        state.is_solo()
    }
}

// ============================================================================
// BYPASS STATE
// ============================================================================

/// Whether an effect or module is bypassed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BypassState {
    /// Processing is active.
    #[default]
    Active,
    /// Processing is bypassed (pass-through).
    Bypassed,
}

impl BypassState {
    /// Check if bypassed.
    #[inline]
    #[must_use]
    pub const fn is_bypassed(self) -> bool {
        matches!(self, Self::Bypassed)
    }

    /// Check if active (not bypassed).
    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Toggle between active and bypassed.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Active => Self::Bypassed,
            Self::Bypassed => Self::Active,
        }
    }
}

impl From<bool> for BypassState {
    fn from(bypassed: bool) -> Self {
        if bypassed {
            Self::Bypassed
        } else {
            Self::Active
        }
    }
}

impl From<BypassState> for bool {
    fn from(state: BypassState) -> Self {
        state.is_bypassed()
    }
}

// ============================================================================
// SYNC MODE
// ============================================================================

/// Whether timing is free-running or synced to tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SyncMode {
    /// Free-running (uses Hz rate).
    #[default]
    Free,
    /// Synced to host/project tempo.
    TempoSync,
}

impl SyncMode {
    /// Check if tempo synced.
    #[inline]
    #[must_use]
    pub const fn is_tempo_sync(self) -> bool {
        matches!(self, Self::TempoSync)
    }

    /// Check if free-running.
    #[inline]
    #[must_use]
    pub const fn is_free(self) -> bool {
        matches!(self, Self::Free)
    }

    /// Toggle between free and tempo sync.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Free => Self::TempoSync,
            Self::TempoSync => Self::Free,
        }
    }
}

impl From<bool> for SyncMode {
    fn from(tempo_sync: bool) -> Self {
        if tempo_sync {
            Self::TempoSync
        } else {
            Self::Free
        }
    }
}

impl From<SyncMode> for bool {
    fn from(mode: SyncMode) -> Self {
        mode.is_tempo_sync()
    }
}

// ============================================================================
// RETRIGGER MODE
// ============================================================================

/// Whether an envelope/LFO retriggers on note-on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RetriggerMode {
    /// Continue from current position.
    #[default]
    Continue,
    /// Reset to start on each note-on.
    Retrigger,
}

impl RetriggerMode {
    /// Check if retrigger is enabled.
    #[inline]
    #[must_use]
    pub const fn should_retrigger(self) -> bool {
        matches!(self, Self::Retrigger)
    }

    /// Toggle between continue and retrigger.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Continue => Self::Retrigger,
            Self::Retrigger => Self::Continue,
        }
    }
}

impl From<bool> for RetriggerMode {
    fn from(retrigger: bool) -> Self {
        if retrigger {
            Self::Retrigger
        } else {
            Self::Continue
        }
    }
}

impl From<RetriggerMode> for bool {
    fn from(mode: RetriggerMode) -> Self {
        mode.should_retrigger()
    }
}

// ============================================================================
// CLIP MODE
// ============================================================================

/// Clipping/saturation mode for amplifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ClipMode {
    /// No clipping (linear).
    #[default]
    Off,
    /// Soft clipping (tanh saturation).
    Soft,
    /// Hard clipping (at ±1.0).
    Hard,
}

impl ClipMode {
    /// All available clip modes.
    pub const ALL: [Self; 3] = [Self::Off, Self::Soft, Self::Hard];

    /// Get display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Soft => "Soft",
            Self::Hard => "Hard",
        }
    }

    /// Get the index of this mode.
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    /// Create from index.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Check if any clipping is active.
    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Check if soft clipping.
    #[inline]
    #[must_use]
    pub const fn is_soft(self) -> bool {
        matches!(self, Self::Soft)
    }

    /// Generate choice options for GUI.
    #[must_use]
    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::modules::core::ChoiceOption::new(m.name().to_lowercase(), m.name()))
            .collect()
    }
}

// For backwards compatibility with existing bool-based code
impl From<bool> for ClipMode {
    fn from(soft_clip: bool) -> Self {
        if soft_clip { Self::Soft } else { Self::Off }
    }
}

// ============================================================================
// LIMIT MODE
// ============================================================================

/// Limiter state for output modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LimitMode {
    /// Limiter is active.
    #[default]
    Enabled,
    /// Limiter is disabled (hard clip at ±1.0).
    Disabled,
}

impl LimitMode {
    /// Check if limiter is enabled.
    #[inline]
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Toggle limiter state.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Enabled => Self::Disabled,
            Self::Disabled => Self::Enabled,
        }
    }
}

impl From<bool> for LimitMode {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

impl From<LimitMode> for bool {
    fn from(mode: LimitMode) -> Self {
        mode.is_enabled()
    }
}

// ============================================================================
// FREEZE STATE
// ============================================================================

/// Whether something is frozen (paused) or running.
///
/// Used for reverb freeze, oscilloscope freeze, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FreezeState {
    /// Running normally.
    #[default]
    Unfrozen,
    /// Frozen/paused state.
    Frozen,
}

impl FreezeState {
    /// Check if frozen.
    #[inline]
    #[must_use]
    pub const fn is_frozen(self) -> bool {
        matches!(self, Self::Frozen)
    }

    /// Check if unfrozen (running).
    #[inline]
    #[must_use]
    pub const fn is_unfrozen(self) -> bool {
        matches!(self, Self::Unfrozen)
    }

    /// Toggle between frozen and unfrozen.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Frozen => Self::Unfrozen,
            Self::Unfrozen => Self::Frozen,
        }
    }
}

impl From<bool> for FreezeState {
    fn from(frozen: bool) -> Self {
        if frozen { Self::Frozen } else { Self::Unfrozen }
    }
}

impl From<FreezeState> for bool {
    fn from(state: FreezeState) -> Self {
        state.is_frozen()
    }
}

// ============================================================================
// POLARITY
// ============================================================================

/// Signal polarity (normal or inverted).
///
/// Used for inverting modulation, panning direction, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Polarity {
    /// Normal polarity.
    #[default]
    Normal,
    /// Inverted polarity (signal * -1).
    Inverted,
}

impl Polarity {
    /// Check if inverted.
    #[inline]
    #[must_use]
    pub const fn is_inverted(self) -> bool {
        matches!(self, Self::Inverted)
    }

    /// Check if normal (not inverted).
    #[inline]
    #[must_use]
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Toggle between normal and inverted.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Normal => Self::Inverted,
            Self::Inverted => Self::Normal,
        }
    }

    /// Get the multiplier for this polarity.
    #[inline]
    #[must_use]
    pub const fn multiplier(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Inverted => -1.0,
        }
    }
}

impl From<bool> for Polarity {
    fn from(inverted: bool) -> Self {
        if inverted {
            Self::Inverted
        } else {
            Self::Normal
        }
    }
}

impl From<Polarity> for bool {
    fn from(polarity: Polarity) -> Self {
        polarity.is_inverted()
    }
}

// ============================================================================
// TEMPO SYNC STATE
// ============================================================================

/// Whether tempo sync is enabled for delay/LFO/etc.
///
/// Alias for `SyncMode` for backwards compatibility and clearer naming.
pub type TempoSyncState = SyncMode;

// ============================================================================
// NOTE RELEASE STATE
// ============================================================================

/// Runtime state for note release in sample players and envelopes.
///
/// This is different from `ReleaseMode` which configures *how* to release.
/// `NoteReleaseState` tracks *whether* we're currently in the release phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NoteReleaseState {
    /// Note is held - normal playback with looping.
    #[default]
    Held,
    /// Note was released - playing to end, no looping.
    Released,
}

impl NoteReleaseState {
    /// Check if in release phase.
    #[inline]
    #[must_use]
    pub const fn is_released(self) -> bool {
        matches!(self, Self::Released)
    }

    /// Check if note is held.
    #[inline]
    #[must_use]
    pub const fn is_held(self) -> bool {
        matches!(self, Self::Held)
    }

    /// Trigger release.
    #[inline]
    #[must_use]
    pub const fn release(self) -> Self {
        Self::Released
    }

    /// Reset to held state.
    #[inline]
    #[must_use]
    pub const fn hold(self) -> Self {
        Self::Held
    }
}

impl From<bool> for NoteReleaseState {
    fn from(releasing: bool) -> Self {
        if releasing {
            Self::Released
        } else {
            Self::Held
        }
    }
}

impl From<NoteReleaseState> for bool {
    fn from(state: NoteReleaseState) -> Self {
        state.is_released()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enable_state() {
        let enabled = EnableState::Enabled;
        assert!(enabled.is_enabled());
        assert!(!enabled.is_disabled());
        assert_eq!(enabled.toggle(), EnableState::Disabled);

        // From/Into bool
        assert_eq!(EnableState::from(true), EnableState::Enabled);
        assert!(bool::from(EnableState::Enabled));
    }

    #[test]
    fn test_mute_state() {
        let muted = MuteState::Muted;
        assert!(muted.is_muted());
        assert!(!muted.is_unmuted());
        assert_eq!(muted.toggle(), MuteState::Unmuted);

        // Default is unmuted
        assert_eq!(MuteState::default(), MuteState::Unmuted);
    }

    #[test]
    fn test_solo_state() {
        let solo = SoloState::Solo;
        assert!(solo.is_solo());
        assert!(!solo.is_normal());
        assert_eq!(solo.toggle(), SoloState::Normal);
    }

    #[test]
    fn test_bypass_state() {
        let bypassed = BypassState::Bypassed;
        assert!(bypassed.is_bypassed());
        assert!(!bypassed.is_active());
        assert_eq!(bypassed.toggle(), BypassState::Active);
    }

    #[test]
    fn test_sync_mode() {
        let sync = SyncMode::TempoSync;
        assert!(sync.is_tempo_sync());
        assert!(!sync.is_free());
        assert_eq!(sync.toggle(), SyncMode::Free);
    }

    #[test]
    fn test_retrigger_mode() {
        let retrig = RetriggerMode::Retrigger;
        assert!(retrig.should_retrigger());
        assert_eq!(retrig.toggle(), RetriggerMode::Continue);
    }

    #[test]
    fn test_clip_mode() {
        assert_eq!(ClipMode::Off.name(), "Off");
        assert_eq!(ClipMode::Soft.name(), "Soft");
        assert_eq!(ClipMode::Hard.name(), "Hard");

        assert!(!ClipMode::Off.is_active());
        assert!(ClipMode::Soft.is_active());
        assert!(ClipMode::Hard.is_active());

        // From bool (backwards compat)
        assert_eq!(ClipMode::from(true), ClipMode::Soft);
        assert_eq!(ClipMode::from(false), ClipMode::Off);
    }

    #[test]
    fn test_limit_mode() {
        let limit = LimitMode::Enabled;
        assert!(limit.is_enabled());
        assert_eq!(limit.toggle(), LimitMode::Disabled);
    }

    #[test]
    fn test_note_release_state() {
        let held = NoteReleaseState::Held;
        assert!(held.is_held());
        assert!(!held.is_released());
        assert_eq!(held.release(), NoteReleaseState::Released);

        let released = NoteReleaseState::Released;
        assert!(released.is_released());
        assert!(!released.is_held());
        assert_eq!(released.hold(), NoteReleaseState::Held);

        // From bool
        assert_eq!(NoteReleaseState::from(true), NoteReleaseState::Released);
        assert_eq!(NoteReleaseState::from(false), NoteReleaseState::Held);
    }
}
