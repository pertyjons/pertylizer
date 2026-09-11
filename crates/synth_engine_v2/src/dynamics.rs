//! Validated authored dynamics settings. Coefficients are prepared off the audio thread.

use crate::quantities::{CutoffFrequency, NormalizedLevel, QuantityError};
use synth_core::{Decibels, Milliseconds, Ratio};

/// The detector's input. An external detector with no cable falls back to the main input,
/// matching V1; a patched silent cable remains silence.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub enum CompressorDetector {
    /// Detect the peak of the main stereo input.
    Internal,
    /// Detect the declared sidechain input with V1's optional rectified-signal high pass.
    External { cutoff: CutoffFrequency },
}

/// V1's compressor parameters, validated together before entering domain logic.
/// These are authored constants; changing them requires recompilation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct CompressorSettings {
    pub(crate) threshold: Decibels,
    pub(crate) ratio: Ratio,
    pub(crate) attack: Milliseconds,
    pub(crate) release: Milliseconds,
    pub(crate) makeup: Decibels,
    pub(crate) mix: NormalizedLevel,
    pub(crate) detector: CompressorDetector,
}

impl CompressorSettings {
    /// Validate the same domains V1 declares, without silently clamping an authored value.
    pub fn new(
        threshold: Decibels,
        ratio: Ratio,
        attack: Milliseconds,
        release: Milliseconds,
        makeup: Decibels,
        mix: NormalizedLevel,
        detector: CompressorDetector,
    ) -> Result<Self, QuantityError> {
        for (quantity, value, minimum, maximum) in [
            ("compressor threshold (dB)", threshold.as_f32(), -60.0, 0.0),
            ("compressor ratio", ratio.as_f32(), 1.0, 20.0),
            ("compressor attack (ms)", attack.as_f32(), 0.1, 100.0),
            ("compressor release (ms)", release.as_f32(), 10.0, 1000.0),
            ("compressor makeup (dB)", makeup.as_f32(), 0.0, 24.0),
        ] {
            validate_interval(quantity, value, minimum, maximum)?;
        }
        if let CompressorDetector::External { cutoff } = detector {
            validate_interval(
                "compressor sidechain cutoff (Hz)",
                cutoff.as_f32(),
                20.0,
                500.0,
            )?;
        }
        Ok(Self {
            threshold,
            ratio,
            attack,
            release,
            makeup,
            mix,
            detector,
        })
    }

    pub const fn detector(self) -> CompressorDetector {
        self.detector
    }
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            threshold: Decibels::new(-20.0),
            ratio: Ratio::MEDIUM,
            attack: Milliseconds::new(10.0),
            release: Milliseconds::new(100.0),
            makeup: Decibels::new(0.0),
            mix: NormalizedLevel::FULL,
            detector: CompressorDetector::Internal,
        }
    }
}

fn validate_interval(
    quantity: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), QuantityError> {
    if !value.is_finite() {
        return Err(QuantityError::NotFinite { quantity, value });
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(QuantityError::OutsideInterval {
            quantity,
            value,
            minimum,
            maximum,
        });
    }
    Ok(())
}
