//! Sequencer track definition.

use serde::{Deserialize, Serialize};

use super::ids::{ReturnBusId, SeqInstrumentId, TrackId};
use synth_core::{BipolarValue, NormalizedValue};

/// A send tap from a track's channel to a return bus.
///
/// The channel's signal is summed into the target return bus scaled by `level`.
/// `pre_fader` selects whether the tap is taken before (`true`) or after
/// (`false`, the default) the channel fader/pan — post-fader is the common case
/// (the send follows the fader), pre-fader keeps the send constant regardless of
/// the channel level (e.g. a fully-wet cue/monitor send).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrackSend {
    /// Destination return bus.
    pub target: ReturnBusId,
    /// Send level (0.0 = no send, 1.0 = unity).
    pub level: NormalizedValue,
    /// Tap point: `true` = pre-fader, `false` = post-fader.
    #[serde(default)]
    pub pre_fader: bool,
    /// Whether the send is active. A disabled send is kept (with its level and
    /// tap point) but contributes nothing to the return bus — a non-destructive
    /// bypass, unlike removing the send. Defaults to `true` so older projects and
    /// freshly-created sends are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Serde default for [`TrackSend::enabled`] — a send with no stored flag (older
/// project) is treated as enabled.
fn default_true() -> bool {
    true
}

impl TrackSend {
    /// Create an enabled post-fader send at the given level.
    #[must_use]
    pub fn new(target: ReturnBusId, level: NormalizedValue) -> Self {
        Self {
            target,
            level,
            pre_fader: false,
            enabled: true,
        }
    }
}

/// A send tap from one return bus into another (bus-to-bus aux routing).
///
/// The source return's post-fader output is summed into the `target` return,
/// scaled by `level`. Used for serial/parallel return chains (e.g. a delay
/// return feeding a reverb return). The routing must stay acyclic — see
/// [`super::Song::return_send_would_cycle`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReturnSend {
    /// Destination return bus.
    pub target: ReturnBusId,
    /// Send level (0.0 = no send, 1.0 = unity).
    pub level: NormalizedValue,
    /// Whether the send is active (non-destructive bypass when `false`).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ReturnSend {
    /// Create an enabled bus-to-bus send at the given level.
    #[must_use]
    pub fn new(target: ReturnBusId, level: NormalizedValue) -> Self {
        Self {
            target,
            level,
            enabled: true,
        }
    }
}

/// Definition of a return bus (effect-send destination) as stored in the song.
///
/// This is the persistent, source-of-truth description — id, name, and fader
/// (volume/pan/mute). The engine owns the runtime channel (the effect chain and
/// audio buffers) keyed by `id`, and reads this fader live each block, exactly
/// as it reads per-track controls (Model C: the song owns channel/routing
/// data). The effect-chain *contents* on a return are engine-side runtime state
/// and are not yet persisted here (a follow-up, matching master effects).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReturnBus {
    /// Unique identifier, referenced by [`TrackSend::target`].
    pub id: ReturnBusId,
    /// Display name (e.g. "Reverb", "Delay").
    pub name: String,
    /// Output fader level (0.0 = silent, 1.0 = unity).
    pub volume: NormalizedValue,
    /// Output pan (-1.0 = left, 0.0 = centre, 1.0 = right).
    pub pan: BipolarValue,
    /// Mute (excluded from the master mix).
    #[serde(default)]
    pub mute: bool,
    /// Solo: when any return is soloed, only soloed returns reach the master mix
    /// (bus-to-bus routing still flows). Independent of track solo.
    #[serde(default)]
    pub solo: bool,
    /// Display color for the mixer strip.
    #[serde(default)]
    pub color: TrackColor,
    /// Free-text description / intent (e.g. "shared plate reverb"). Empty by
    /// default; readable/writable via MCP and GUI. Never affects audio.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Bus-to-bus send taps from this return into other return busses. Empty by
    /// default; must stay acyclic (see [`super::Song::return_send_would_cycle`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<ReturnSend>,
}

impl ReturnBus {
    /// Create a return bus at unity gain / centre pan.
    #[must_use]
    pub fn new(id: ReturnBusId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            volume: NormalizedValue::MAX,
            pan: BipolarValue::CENTER,
            mute: false,
            solo: false,
            color: TrackColor::default(),
            description: String::new(),
            sends: Vec::new(),
        }
    }

    /// Set mute state. Muting clears solo (a channel can't be both at once),
    /// mirroring [`SequencerTrack::set_mute`].
    pub fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
        if mute {
            self.solo = false;
        }
    }

    /// Set solo state. Soloing clears mute (see [`Self::set_mute`]).
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
        if solo {
            self.mute = false;
        }
    }
}

/// Track playback mode - determines how notes are allocated to voices.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum TrackMode {
    /// Polyphonic mode - notes are allocated from the voice pool dynamically.
    /// Standard keyboard/MIDI behavior.
    #[default]
    Polyphonic,
}

/// A sequencer track in the song.
/// Named SequencerTrack to distinguish from future AudioTrack.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SequencerTrack {
    /// Unique identifier.
    pub id: TrackId,
    /// Track name.
    pub name: String,
    /// Free-text description capturing intent (e.g. "kick layer",
    /// "sidechain source"). Empty by default; readable/writable via MCP and GUI.
    #[serde(default)]
    pub description: String,
    /// Instrument this track routes its notes to. Every track has one;
    /// instruments may be shared across tracks (intentional layering). Defaults
    /// to `SeqInstrumentId(0)` for a freshly created track until reassigned.
    #[serde(default)]
    pub instrument: SeqInstrumentId,
    /// Volume (type-safe normalized 0.0-1.0).
    pub volume: NormalizedValue,
    /// Panning (type-safe: -1.0 = left, 0.0 = center, 1.0 = right).
    pub pan: BipolarValue,
    /// Muted state.
    pub mute: bool,
    /// Solo state.
    pub solo: bool,
    /// Track color for UI.
    pub color: TrackColor,
    /// Playback mode (polyphonic or mono-voice).
    pub mode: TrackMode,
    /// Effect-send taps from this channel to return busses. Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<TrackSend>,
}

impl SequencerTrack {
    /// Create a new sequencer track.
    #[must_use]
    pub fn new(id: TrackId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            instrument: SeqInstrumentId::default(),
            volume: NormalizedValue::MAX,
            pan: BipolarValue::CENTER,
            mute: false,
            solo: false,
            color: TrackColor::default(),
            mode: TrackMode::default(),
            sends: Vec::new(),
        }
    }

    /// Set the instrument (builder pattern).
    #[must_use]
    pub fn with_instrument(mut self, instrument: SeqInstrumentId) -> Self {
        self.instrument = instrument;
        self
    }

    /// Set the volume (builder pattern).
    #[must_use]
    pub fn with_volume(mut self, volume: NormalizedValue) -> Self {
        self.volume = volume;
        self
    }

    /// Set the panning (builder pattern).
    #[must_use]
    pub fn with_pan(mut self, pan: BipolarValue) -> Self {
        self.pan = pan;
        self
    }

    /// Set the color (builder pattern).
    #[must_use]
    pub fn with_color(mut self, color: TrackColor) -> Self {
        self.color = color;
        self
    }

    /// Set mute state. Muting clears solo: a single channel can't be both
    /// muted and soloed at once (several channels may still be soloed together).
    pub fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
        if mute {
            self.solo = false;
        }
    }

    /// Set solo state. Soloing clears mute (see [`Self::set_mute`]).
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
        if solo {
            self.mute = false;
        }
    }

    /// Toggle mute state. Enabling mute clears solo (see [`Self::set_mute`]).
    pub fn toggle_mute(&mut self) {
        self.set_mute(!self.mute);
    }

    /// Toggle solo state. Enabling solo clears mute (see [`Self::set_solo`]).
    pub fn toggle_solo(&mut self) {
        self.set_solo(!self.solo);
    }

    /// Check if this track should be audible given solo states.
    #[must_use]
    pub fn is_audible(&self, any_solo: bool) -> bool {
        if self.mute {
            return false;
        }
        if any_solo && !self.solo {
            return false;
        }
        true
    }
}

/// Track color for UI display.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrackColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl TrackColor {
    pub const RED: Self = Self {
        r: 255,
        g: 100,
        b: 100,
    };
    pub const GREEN: Self = Self {
        r: 100,
        g: 255,
        b: 100,
    };
    pub const BLUE: Self = Self {
        r: 100,
        g: 100,
        b: 255,
    };
    pub const YELLOW: Self = Self {
        r: 255,
        g: 255,
        b: 100,
    };
    pub const CYAN: Self = Self {
        r: 100,
        g: 255,
        b: 255,
    };
    pub const MAGENTA: Self = Self {
        r: 255,
        g: 100,
        b: 255,
    };
    pub const ORANGE: Self = Self {
        r: 255,
        g: 180,
        b: 100,
    };
    pub const PURPLE: Self = Self {
        r: 180,
        g: 100,
        b: 255,
    };

    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Get as RGB tuple.
    #[must_use]
    pub fn as_rgb(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }

    /// Get as normalized RGB.
    #[must_use]
    pub fn as_rgb_f32(&self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }

    /// Format as a `"#RRGGBB"` hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Parse from `"#RRGGBB"` or `"#RRGGBBAA"` (alpha ignored), with or without
    /// the leading `#`. Returns `None` for malformed input.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let s = hex.strip_prefix('#').unwrap_or(hex);
        if (s.len() != 6 && s.len() != 8) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    /// Preset colors for cycling.
    pub fn presets() -> &'static [Self] {
        &[
            Self::RED,
            Self::GREEN,
            Self::BLUE,
            Self::YELLOW,
            Self::CYAN,
            Self::MAGENTA,
            Self::ORANGE,
            Self::PURPLE,
        ]
    }
}

impl Default for TrackColor {
    fn default() -> Self {
        Self::BLUE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_creation() {
        let track = SequencerTrack::new(TrackId(0), "Lead");
        assert_eq!(track.name, "Lead");
        assert_eq!(track.volume, NormalizedValue::MAX);
        assert_eq!(track.pan, BipolarValue::CENTER);
        assert!(!track.mute);
        assert!(!track.solo);
    }

    #[test]
    fn test_track_builder() {
        let track = SequencerTrack::new(TrackId(0), "Bass")
            .with_instrument(SeqInstrumentId(1))
            .with_volume(NormalizedValue::new(0.8))
            .with_pan(BipolarValue::new(0.3));

        assert_eq!(track.instrument, SeqInstrumentId(1));
        assert_eq!(track.volume, NormalizedValue::new(0.8));
        assert_eq!(track.pan, BipolarValue::new(0.3));
    }

    #[test]
    fn new_track_has_a_default_instrument() {
        // Phase 3: every track has an instrument (no `Option`); a freshly
        // created track defaults to `SeqInstrumentId(0)` until reassigned.
        let track = SequencerTrack::new(TrackId(3), "Fresh");
        assert_eq!(track.instrument, SeqInstrumentId(0));
    }

    #[test]
    fn instruments_may_be_shared_across_tracks() {
        // Phase 3: sharing is allowed (intentional layering, e.g. Kick +
        // Syncopated Kick). Two tracks can route to the same instrument.
        let a = SequencerTrack::new(TrackId(0), "Kick").with_instrument(SeqInstrumentId(7));
        let b =
            SequencerTrack::new(TrackId(1), "Syncopated Kick").with_instrument(SeqInstrumentId(7));
        assert_eq!(a.instrument, b.instrument);
    }

    #[test]
    fn track_color_hex_round_trips() {
        let c = TrackColor::new(0x12, 0x34, 0x56);
        assert_eq!(c.to_hex(), "#123456");
        assert_eq!(TrackColor::from_hex("#123456"), Some(c));
        // Without leading '#', lowercase, and 8-digit (alpha ignored) all parse.
        assert_eq!(TrackColor::from_hex("123456"), Some(c));
        assert_eq!(TrackColor::from_hex("#123456ff"), Some(c));
        // Malformed input is rejected, not panicked on.
        assert_eq!(TrackColor::from_hex(""), None);
        assert_eq!(TrackColor::from_hex("#12345"), None);
        assert_eq!(TrackColor::from_hex("#nothex"), None);
        assert_eq!(TrackColor::from_hex("#1234😀"), None);
    }

    #[test]
    fn test_track_audibility() {
        let mut track = SequencerTrack::new(TrackId(0), "Test");

        // Normal state
        assert!(track.is_audible(false));

        // Muted
        track.mute = true;
        assert!(!track.is_audible(false));

        // Unmuted but another track is soloed
        track.mute = false;
        assert!(!track.is_audible(true));

        // This track is soloed
        track.solo = true;
        assert!(track.is_audible(true));
    }

    #[test]
    fn test_mute_solo_mutually_exclusive() {
        let mut track = SequencerTrack::new(TrackId(0), "Test");

        // Soloing a muted track clears the mute.
        track.set_mute(true);
        track.toggle_solo();
        assert!(track.solo);
        assert!(!track.mute);

        // Muting a soloed track clears the solo.
        track.toggle_mute();
        assert!(track.mute);
        assert!(!track.solo);

        // Turning a flag off never touches the other.
        track.set_solo(true);
        track.set_solo(false);
        assert!(!track.solo);
        assert!(!track.mute);
    }

    #[test]
    fn test_track_color() {
        let color = TrackColor::new(128, 64, 32);
        let (r, g, b) = color.as_rgb_f32();
        assert!((r - 0.502).abs() < 0.01);
        assert!((g - 0.251).abs() < 0.01);
        assert!((b - 0.125).abs() < 0.01);
    }
}
