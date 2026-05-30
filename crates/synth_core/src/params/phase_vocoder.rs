//! Phase vocoder parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{NormalizedValue, Semitones};

/// FFT size option for phase vocoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FftSizeOption {
    /// 512 samples (low latency, lower quality).
    Fft512,
    /// 1024 samples (balanced).
    #[default]
    Fft1024,
    /// 2048 samples (high quality, higher latency).
    Fft2048,
    /// 4096 samples (highest quality).
    Fft4096,
}

impl FftSizeOption {
    pub const ALL: [Self; 4] = [Self::Fft512, Self::Fft1024, Self::Fft2048, Self::Fft4096];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fft512 => "512",
            Self::Fft1024 => "1024",
            Self::Fft2048 => "2048",
            Self::Fft4096 => "4096",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Fft512 => "512",
            Self::Fft1024 => "1024",
            Self::Fft2048 => "2048",
            Self::Fft4096 => "4096",
        }
    }

    #[must_use]
    pub fn size(self) -> usize {
        match self {
            Self::Fft512 => 512,
            Self::Fft1024 => 1024,
            Self::Fft2048 => 2048,
            Self::Fft4096 => 4096,
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Fft512 => "512-sample FFT — low latency and CPU, coarser frequency resolution.",
            Self::Fft1024 => "1024-sample FFT — a balance of latency and quality.",
            Self::Fft2048 => "2048-sample FFT — finer frequency resolution, higher latency.",
            Self::Fft4096 => "4096-sample FFT — the finest resolution, highest latency and CPU.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(1)
    }
}

/// Phase vocoder parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PhaseVocoderParam {
    /// Pitch shift amount (-24 to +24 semitones).
    PitchShift(Semitones),
    /// Spectral freeze mode.
    Freeze(bool),
    /// FFT size selection.
    FftSize(FftSizeOption),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl PhaseVocoderParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::PitchShift(_) => "Pitch Shift",
            Self::Freeze(_) => "Freeze",
            Self::FftSize(_) => "FFT Size",
            Self::Mix(_) => "Mix",
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::PitchShift(st) => st.0,
            Self::Freeze(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::FftSize(f) => f.index() as f32,
            Self::Mix(v) => v.as_f32(),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::PitchShift(_) => Self::PitchShift(Semitones(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::FftSize(_) => Self::FftSize(FftSizeOption::from_index(value as usize)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for PhaseVocoderParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}
