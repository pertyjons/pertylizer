//! Tracker-style effect commands.
//!
//! These effects are inspired by ProTracker/FastTracker and can be attached to notes.

use serde::{Deserialize, Serialize};

use super::pitch::Pitch;

/// Effect command that can be applied to a note.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EffectCommand {
    // === Pitch effects ===
    /// Cycle between base note, +x semitones, +y semitones per tick.
    Arpeggio { x: u8, y: u8 },
    /// Slide pitch up (units per tick).
    PortamentoUp(u8),
    /// Slide pitch down (units per tick).
    PortamentoDown(u8),
    /// Slide toward target note.
    TonePortamento { speed: u8, target: Option<Pitch> },
    /// Periodic pitch variation.
    Vibrato { speed: u8, depth: u8 },
    /// Waveform for vibrato.
    VibratoWaveform(EffectWaveform),
    /// Quantize portamento to semitones.
    Glissando(bool),
    /// Fine-tune pitch (-128 to +127 cents).
    FineTune(i8),

    // === Volume effects ===
    /// Set volume (0-64 tracker style).
    SetVolume(u8),
    /// Gradual volume change per tick.
    VolumeSlide { up: u8, down: u8 },
    /// Fine volume change (per row instead of tick).
    FineVolumeSlide { up: u8, down: u8 },
    /// Periodic volume variation.
    Tremolo { speed: u8, depth: u8 },
    /// Waveform for tremolo.
    TremoloWaveform(EffectWaveform),

    // === Panning effects ===
    /// Set panning (0=left, 128=center, 255=right).
    SetPanning(u8),
    /// Gradual panning change.
    PanningSlide { left: u8, right: u8 },

    // === Sample/playback effects ===
    /// Start sample from position (in 256ths).
    SampleOffset(u16),
    /// Retrigger note with interval and volume change.
    Retrigger { interval: u8, volume_change: i8 },
    /// Cut note after n ticks.
    NoteCut(u8),
    /// Delay note n ticks.
    NoteDelay(u8),
    /// Gradual fade out.
    NoteFadeOut(u8),
    /// Play sample in reverse.
    Reverse,

    // === Timing effects (global) ===
    /// Change tempo (BPM).
    SetTempo(u16),
    /// Change speed (ticks per row, tracker compatibility).
    SetSpeed(u8),
    /// Delay pattern n rows.
    PatternDelay(u8),

    // === Navigation (global) ===
    /// Loop section, count=0 sets loop start.
    PatternLoop { count: u8 },
    /// Jump to pattern in order list.
    PatternJump(u8),
    /// Break pattern, jump to row in next.
    PatternBreak(u8),
}

impl EffectCommand {
    /// Returns true if the effect is global (affects entire pattern/song).
    pub fn is_global(&self) -> bool {
        matches!(
            self,
            Self::SetTempo(_)
                | Self::SetSpeed(_)
                | Self::PatternDelay(_)
                | Self::PatternLoop { .. }
                | Self::PatternJump(_)
                | Self::PatternBreak(_)
        )
    }

    /// Returns true if the effect is continuous (runs every tick).
    pub fn is_continuous(&self) -> bool {
        matches!(
            self,
            Self::Arpeggio { .. }
                | Self::PortamentoUp(_)
                | Self::PortamentoDown(_)
                | Self::TonePortamento { .. }
                | Self::Vibrato { .. }
                | Self::VolumeSlide { .. }
                | Self::Tremolo { .. }
                | Self::PanningSlide { .. }
        )
    }

    /// Returns true if the effect affects pitch.
    pub fn is_pitch_effect(&self) -> bool {
        matches!(
            self,
            Self::Arpeggio { .. }
                | Self::PortamentoUp(_)
                | Self::PortamentoDown(_)
                | Self::TonePortamento { .. }
                | Self::Vibrato { .. }
                | Self::VibratoWaveform(_)
                | Self::Glissando(_)
                | Self::FineTune(_)
        )
    }

    /// Returns true if the effect affects volume.
    pub fn is_volume_effect(&self) -> bool {
        matches!(
            self,
            Self::SetVolume(_)
                | Self::VolumeSlide { .. }
                | Self::FineVolumeSlide { .. }
                | Self::Tremolo { .. }
                | Self::TremoloWaveform(_)
                | Self::NoteFadeOut(_)
        )
    }

    /// Get a tracker-style display code (e.g., "0xy" for arpeggio).
    pub fn display_code(&self) -> String {
        match self {
            Self::Arpeggio { x, y } => format!("0{:X}{:X}", x & 0xF, y & 0xF),
            Self::PortamentoUp(n) => format!("1{:02X}", n),
            Self::PortamentoDown(n) => format!("2{:02X}", n),
            Self::TonePortamento { speed, .. } => format!("3{:02X}", speed),
            Self::Vibrato { speed, depth } => format!("4{:X}{:X}", speed & 0xF, depth & 0xF),
            Self::VolumeSlide { up, down } => format!("A{:X}{:X}", up & 0xF, down & 0xF),
            Self::SetVolume(v) => format!("C{:02X}", v),
            Self::SetTempo(bpm) => format!("F{:02X}", (*bpm).min(255) as u8),
            Self::SetSpeed(s) => format!("F{:02X}", s),
            Self::PatternBreak(row) => format!("D{:02X}", row),
            Self::PatternJump(pos) => format!("B{:02X}", pos),
            Self::NoteCut(n) => format!("EC{:X}", n & 0xF),
            Self::NoteDelay(n) => format!("ED{:X}", n & 0xF),
            Self::Retrigger { interval, .. } => format!("E9{:X}", interval & 0xF),
            _ => "...".to_string(),
        }
    }
}

/// Waveform types for vibrato/tremolo effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EffectWaveform {
    #[default]
    Sine,
    Ramp,
    Square,
    Random,
}

impl EffectWaveform {
    /// Sample the waveform at phase (0.0-1.0).
    pub fn sample(&self, phase: f32) -> f32 {
        match self {
            Self::Sine => (phase * std::f32::consts::TAU).sin(),
            Self::Ramp => 2.0 * phase - 1.0,
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Random => {
                // Simple hash-based random
                let seed = (phase * 1000.0) as u32;
                let hash = seed.wrapping_mul(2_654_435_761);
                (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_is_global() {
        assert!(EffectCommand::SetTempo(120).is_global());
        assert!(EffectCommand::PatternBreak(0).is_global());
        assert!(!EffectCommand::Arpeggio { x: 3, y: 7 }.is_global());
        assert!(!EffectCommand::SetVolume(64).is_global());
    }

    #[test]
    fn test_effect_is_continuous() {
        assert!(EffectCommand::Arpeggio { x: 3, y: 7 }.is_continuous());
        assert!(EffectCommand::Vibrato { speed: 4, depth: 8 }.is_continuous());
        assert!(!EffectCommand::SetVolume(64).is_continuous());
        assert!(!EffectCommand::NoteCut(4).is_continuous());
    }

    #[test]
    fn test_effect_display_code() {
        assert_eq!(EffectCommand::Arpeggio { x: 3, y: 7 }.display_code(), "037");
        assert_eq!(EffectCommand::SetVolume(64).display_code(), "C40");
        assert_eq!(EffectCommand::PortamentoUp(16).display_code(), "110");
    }

    #[test]
    fn test_waveform_sample() {
        let sine = EffectWaveform::Sine;
        assert!((sine.sample(0.0) - 0.0).abs() < 0.001);
        assert!((sine.sample(0.25) - 1.0).abs() < 0.001);

        let square = EffectWaveform::Square;
        assert_eq!(square.sample(0.25), 1.0);
        assert_eq!(square.sample(0.75), -1.0);
    }
}
