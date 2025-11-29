//! Track definition.

use serde::{Deserialize, Serialize};

use super::ids::{InstrumentId, TrackId};

/// A track in the song.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// Unique identifier.
    pub id: TrackId,
    /// Track name.
    pub name: String,
    /// Instrument this track controls (None = MIDI out or none).
    pub instrument: Option<InstrumentId>,
    /// Volume (0.0 - 1.0).
    pub volume: f32,
    /// Panning (0.0 = left, 0.5 = center, 1.0 = right).
    pub pan: f32,
    /// Muted state.
    pub mute: bool,
    /// Solo state.
    pub solo: bool,
    /// Track color for UI.
    pub color: TrackColor,
}

impl Track {
    /// Create a new track.
    pub fn new(id: TrackId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            instrument: None,
            volume: 1.0,
            pan: 0.5,
            mute: false,
            solo: false,
            color: TrackColor::default(),
        }
    }

    /// Set the instrument (builder pattern).
    pub fn with_instrument(mut self, instrument: InstrumentId) -> Self {
        self.instrument = Some(instrument);
        self
    }

    /// Set the volume (builder pattern).
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }

    /// Set the panning (builder pattern).
    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan.clamp(0.0, 1.0);
        self
    }

    /// Set the color (builder pattern).
    pub fn with_color(mut self, color: TrackColor) -> Self {
        self.color = color;
        self
    }

    /// Toggle mute state.
    pub fn toggle_mute(&mut self) {
        self.mute = !self.mute;
    }

    /// Toggle solo state.
    pub fn toggle_solo(&mut self) {
        self.solo = !self.solo;
    }

    /// Check if this track should be audible given solo states.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl TrackColor {
    pub const RED: TrackColor = TrackColor { r: 255, g: 100, b: 100 };
    pub const GREEN: TrackColor = TrackColor { r: 100, g: 255, b: 100 };
    pub const BLUE: TrackColor = TrackColor { r: 100, g: 100, b: 255 };
    pub const YELLOW: TrackColor = TrackColor { r: 255, g: 255, b: 100 };
    pub const CYAN: TrackColor = TrackColor { r: 100, g: 255, b: 255 };
    pub const MAGENTA: TrackColor = TrackColor { r: 255, g: 100, b: 255 };
    pub const ORANGE: TrackColor = TrackColor { r: 255, g: 180, b: 100 };
    pub const PURPLE: TrackColor = TrackColor { r: 180, g: 100, b: 255 };

    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Get as RGB tuple.
    pub fn as_rgb(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }

    /// Get as normalized RGB.
    pub fn as_rgb_f32(&self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }

    /// Preset colors for cycling.
    pub fn presets() -> &'static [TrackColor] {
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
        let track = Track::new(TrackId(0), "Lead");
        assert_eq!(track.name, "Lead");
        assert_eq!(track.volume, 1.0);
        assert_eq!(track.pan, 0.5);
        assert!(!track.mute);
        assert!(!track.solo);
    }

    #[test]
    fn test_track_builder() {
        let track = Track::new(TrackId(0), "Bass")
            .with_instrument(InstrumentId(1))
            .with_volume(0.8)
            .with_pan(0.3);

        assert_eq!(track.instrument, Some(InstrumentId(1)));
        assert_eq!(track.volume, 0.8);
        assert_eq!(track.pan, 0.3);
    }

    #[test]
    fn test_track_audibility() {
        let mut track = Track::new(TrackId(0), "Test");

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
    fn test_track_color() {
        let color = TrackColor::new(128, 64, 32);
        let (r, g, b) = color.as_rgb_f32();
        assert!((r - 0.502).abs() < 0.01);
        assert!((g - 0.251).abs() < 0.01);
        assert!((b - 0.125).abs() < 0.01);
    }
}
