//! Wavetable data generation.
//!
//! Provides built-in wavetable banks for the wavetable oscillator.
//! Each bank contains multiple frames that can be scanned through.
//!
//! Frame layout: each frame is `FRAME_SIZE` samples of one waveform cycle.
//! Scanning between frames provides timbral morphing.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use synth_core::WavetableSelect;

/// Number of samples per wavetable frame.
pub const FRAME_SIZE: usize = 2048;

/// Number of frames per bank (except Basic which has 64).
pub const DEFAULT_NUM_FRAMES: usize = 32;

/// A wavetable bank: a collection of frames.
#[derive(Clone)]
pub struct WavetableBank {
    /// Frames of waveform data. Each frame is FRAME_SIZE samples.
    pub frames: Vec<Vec<f32>>,
}

impl WavetableBank {
    /// Get the number of frames in this bank.
    pub fn num_frames(&self) -> usize {
        self.frames.len()
    }

    /// Sample the wavetable at a given position (0.0-1.0) and phase (0.0-1.0).
    /// Uses linear interpolation between frames and within frames.
    #[inline]
    pub fn sample(&self, position: f32, phase: f32) -> f32 {
        let num_frames = self.frames.len();
        if num_frames == 0 {
            return 0.0;
        }

        // Position to frame index with interpolation. `position` may arrive NaN
        // from a modulated source; `clamp` passes NaN through, so coerce first.
        let position = crate::math::sanitize_cv(position);
        let pos = position.clamp(0.0, 1.0) * (num_frames - 1) as f32;
        let frame_idx = pos as usize;
        let frame_frac = pos - frame_idx as f32;

        // Phase to sample index with interpolation
        let p = phase.rem_euclid(1.0) * FRAME_SIZE as f32;
        let sample_idx = p as usize;
        let sample_frac = p - sample_idx as f32;

        let idx0 = sample_idx % FRAME_SIZE;
        let idx1 = (sample_idx + 1) % FRAME_SIZE;

        // Sample from current frame
        let s0 = self.frames[frame_idx][idx0] * (1.0 - sample_frac)
            + self.frames[frame_idx][idx1] * sample_frac;

        // If we need to interpolate between frames
        if frame_frac > 0.0001 && frame_idx + 1 < num_frames {
            let s1 = self.frames[frame_idx + 1][idx0] * (1.0 - sample_frac)
                + self.frames[frame_idx + 1][idx1] * sample_frac;
            s0 * (1.0 - frame_frac) + s1 * frame_frac
        } else {
            s0
        }
    }
}

// ============================================================================
// WAVETABLE GENERATION
// ============================================================================

/// Get a wavetable bank by selection. Uses lazy initialization.
pub fn get_wavetable(select: WavetableSelect) -> &'static WavetableBank {
    match select {
        WavetableSelect::Basic => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_basic)
        }
        WavetableSelect::Harmonics => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_harmonics)
        }
        WavetableSelect::Pwm => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_pwm)
        }
        WavetableSelect::Formant => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_formant)
        }
        WavetableSelect::Digital => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_digital)
        }
        WavetableSelect::Warm => {
            static BANK: OnceLock<WavetableBank> = OnceLock::new();
            BANK.get_or_init(generate_warm)
        }
    }
}

/// Generate a single frame from a function.
fn generate_frame(f: impl Fn(f32) -> f32) -> Vec<f32> {
    (0..FRAME_SIZE)
        .map(|i| {
            let phase = i as f32 / FRAME_SIZE as f32;
            f(phase)
        })
        .collect()
}

/// Basic: Sine -> Triangle -> Saw -> Square morph (64 frames)
fn generate_basic() -> WavetableBank {
    let num_frames = 64;
    let mut frames = Vec::with_capacity(num_frames);

    for frame in 0..num_frames {
        let t = frame as f32 / (num_frames - 1) as f32;

        frames.push(generate_frame(|phase| {
            let sine = (phase * TAU).sin();
            let triangle = if phase < 0.5 {
                4.0 * phase - 1.0
            } else {
                3.0 - 4.0 * phase
            };
            let sawtooth = 2.0 * phase - 1.0;
            let square = if phase < 0.5 { 1.0 } else { -1.0 };

            if t < 1.0 / 3.0 {
                // Sine -> Triangle
                let blend = t * 3.0;
                sine * (1.0 - blend) + triangle * blend
            } else if t < 2.0 / 3.0 {
                // Triangle -> Sawtooth
                let blend = (t - 1.0 / 3.0) * 3.0;
                triangle * (1.0 - blend) + sawtooth * blend
            } else {
                // Sawtooth -> Square
                let blend = (t - 2.0 / 3.0) * 3.0;
                sawtooth * (1.0 - blend) + square * blend
            }
        }));
    }

    WavetableBank { frames }
}

/// Harmonics: additive synthesis 1 to 32 harmonics
fn generate_harmonics() -> WavetableBank {
    let mut frames = Vec::with_capacity(DEFAULT_NUM_FRAMES);

    for frame in 0..DEFAULT_NUM_FRAMES {
        let num_harmonics = frame + 1; // 1 to 32

        frames.push(generate_frame(|phase| {
            let mut sum = 0.0;
            for h in 1..=num_harmonics {
                sum += (phase * TAU * h as f32).sin() / h as f32;
            }
            // Normalize
            sum * 0.6
        }));
    }

    WavetableBank { frames }
}

/// PWM: pulse width from 50% down to ~5%
fn generate_pwm() -> WavetableBank {
    let mut frames = Vec::with_capacity(DEFAULT_NUM_FRAMES);

    for frame in 0..DEFAULT_NUM_FRAMES {
        let pw = 0.5 - (frame as f32 / (DEFAULT_NUM_FRAMES - 1) as f32) * 0.45;

        frames.push(generate_frame(|phase| if phase < pw { 1.0 } else { -1.0 }));
    }

    WavetableBank { frames }
}

/// Formant: vowel-like formant sweeps (a/e/i/o/u and blends)
fn generate_formant() -> WavetableBank {
    // Formant frequencies for vowels (simplified 2-formant model)
    let vowels: [(f32, f32); 5] = [
        (800.0, 1200.0), // a
        (400.0, 2200.0), // e
        (300.0, 2800.0), // i
        (500.0, 800.0),  // o
        (350.0, 700.0),  // u
    ];

    let mut frames = Vec::with_capacity(DEFAULT_NUM_FRAMES);
    let base_freq = 130.0; // Reference frequency for formant generation

    for frame in 0..DEFAULT_NUM_FRAMES {
        let t = frame as f32 / (DEFAULT_NUM_FRAMES - 1) as f32;
        let vowel_pos = t * 4.0; // 0-4 across 5 vowels
        let vowel_idx = (vowel_pos as usize).min(3);
        let vowel_frac = vowel_pos - vowel_idx as f32;

        let (f1a, f2a) = vowels[vowel_idx];
        let (f1b, f2b) = vowels[vowel_idx + 1];
        let f1 = f1a * (1.0 - vowel_frac) + f1b * vowel_frac;
        let f2 = f2a * (1.0 - vowel_frac) + f2b * vowel_frac;

        frames.push(generate_frame(|phase| {
            // Additive formant synthesis
            let mut sum = 0.0;
            for h in 1..=32 {
                let freq = base_freq * h as f32;
                let amp_f1 = (-((freq - f1) / 80.0).powi(2)).exp();
                let amp_f2 = (-((freq - f2) / 100.0).powi(2)).exp();
                let amp = (amp_f1 + amp_f2 * 0.7) / h as f32;
                sum += (phase * TAU * h as f32).sin() * amp;
            }
            (sum * 1.5).clamp(-1.0, 1.0)
        }));
    }

    WavetableBank { frames }
}

/// Digital: mathematical waveforms (bit-crushed, FM, sync-like, etc.)
fn generate_digital() -> WavetableBank {
    let mut frames = Vec::with_capacity(DEFAULT_NUM_FRAMES);

    for frame in 0..DEFAULT_NUM_FRAMES {
        let t = frame as f32 / (DEFAULT_NUM_FRAMES - 1) as f32;

        frames.push(generate_frame(|phase| {
            if t < 0.25 {
                // FM-like: sine with increasing modulation
                let mod_depth = t * 4.0 * 8.0;
                (phase * TAU + mod_depth * (phase * TAU * 2.0).sin()).sin()
            } else if t < 0.5 {
                // Hard sync-like: increasing frequency ratio
                let ratio = 1.0 + (t - 0.25) * 4.0 * 7.0;
                (phase * ratio * TAU).sin()
            } else if t < 0.75 {
                // Bit-crush effect: reducing resolution
                let bits = 16.0 - (t - 0.5) * 4.0 * 14.0;
                let levels = (2.0_f32).powf(bits.max(2.0));
                let sine = (phase * TAU).sin();
                (sine * levels).round() / levels
            } else {
                // Ring mod-like: two oscillator multiply
                let ratio = 1.0 + (t - 0.75) * 4.0 * 5.0;
                (phase * TAU).sin() * (phase * ratio * TAU).sin()
            }
        }));
    }

    WavetableBank { frames }
}

/// Warm: soft analog-style waveforms with gentle harmonics
fn generate_warm() -> WavetableBank {
    let mut frames = Vec::with_capacity(DEFAULT_NUM_FRAMES);

    for frame in 0..DEFAULT_NUM_FRAMES {
        let t = frame as f32 / (DEFAULT_NUM_FRAMES - 1) as f32;

        frames.push(generate_frame(|phase| {
            if t < 0.25 {
                // Sine with gentle even harmonics (tube-like warmth)
                let blend = t * 4.0;
                let sine = (phase * TAU).sin();
                let warm = sine
                    + blend * 0.3 * (phase * TAU * 2.0).sin()
                    + blend * 0.15 * (phase * TAU * 3.0).sin();
                warm / (1.0 + blend * 0.45)
            } else if t < 0.5 {
                // Soft triangle with rounded corners
                let sharpness = 1.0 + (t - 0.25) * 4.0 * 4.0;
                let tri = if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                };
                // Soft clip the triangle
                (tri * sharpness).tanh() / sharpness.tanh()
            } else if t < 0.75 {
                // Soft saw (band-limited feel)
                let harmonics = 4 + ((t - 0.5) * 4.0 * 12.0) as u32;
                let mut sum = 0.0;
                for h in 1..=harmonics {
                    // Roll off higher harmonics for warmth
                    let rolloff = 1.0 / (1.0 + (h as f32 / 6.0).powi(2));
                    sum += (phase * TAU * h as f32).sin() / h as f32 * rolloff;
                }
                sum * 1.2
            } else {
                // Saturated mix of low harmonics
                let drive = 1.0 + (t - 0.75) * 4.0 * 4.0;
                let base = (phase * TAU).sin()
                    + 0.5 * (phase * TAU * 2.0).sin()
                    + 0.3 * (phase * TAU * 3.0).sin()
                    + 0.15 * (phase * TAU * 4.0).sin();
                (base * drive * 0.5).tanh()
            }
        }));
    }

    WavetableBank { frames }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_wavetables_generate() {
        for select in WavetableSelect::ALL {
            let bank = get_wavetable(select);
            assert!(bank.num_frames() > 0, "{:?} has no frames", select);
            // Verify all frames have correct size
            for (i, frame) in bank.frames.iter().enumerate() {
                assert_eq!(
                    frame.len(),
                    FRAME_SIZE,
                    "{:?} frame {} has wrong size: {}",
                    select,
                    i,
                    frame.len()
                );
            }
        }
    }

    #[test]
    fn test_wavetable_sampling() {
        let bank = get_wavetable(WavetableSelect::Basic);

        // Sample at various positions
        for pos in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for phase in [0.0, 0.25, 0.5, 0.75] {
                let sample = bank.sample(pos, phase);
                assert!(
                    sample.abs() <= 1.1, // Allow slight overshoot from interpolation
                    "Sample at pos={}, phase={} out of range: {}",
                    pos,
                    phase,
                    sample
                );
            }
        }
    }

    #[test]
    fn test_basic_wavetable_sine_at_start() {
        let bank = get_wavetable(WavetableSelect::Basic);

        // First frame should be pure sine
        let sample_0 = bank.sample(0.0, 0.0); // sin(0) = 0
        let sample_25 = bank.sample(0.0, 0.25); // sin(pi/2) = 1

        assert!(
            sample_0.abs() < 0.01,
            "Expected ~0 at phase 0, got {}",
            sample_0
        );
        assert!(
            (sample_25 - 1.0).abs() < 0.05,
            "Expected ~1 at phase 0.25, got {}",
            sample_25
        );
    }
}
