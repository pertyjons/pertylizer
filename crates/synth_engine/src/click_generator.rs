//! Metronome click generator.
//!
//! Generates short sine wave clicks at beat boundaries.
//! Fully real-time safe — no heap allocations.

/// Frequency for accented clicks (beat 1).
const ACCENT_FREQ: f32 = 1200.0;
/// Frequency for normal clicks.
const NORMAL_FREQ: f32 = 800.0;
/// Duration of accented click in samples at 44100 Hz baseline.
const ACCENT_DURATION_MS: f32 = 40.0;
/// Duration of normal click in samples at 44100 Hz baseline.
const NORMAL_DURATION_MS: f32 = 30.0;

/// Generates metronome click sounds.
///
/// Produces short sine wave bursts with exponential decay.
/// Accented clicks (beat 1) are higher pitched and longer.
pub(crate) struct ClickGenerator {
    /// Whether the metronome is enabled.
    enabled: bool,
    /// Volume level (0.0–1.0).
    volume: f32,
    /// Current sample rate.
    sample_rate: f32,
    /// Samples remaining in the current click.
    click_remaining: u32,
    /// Total samples for the current click (for envelope calculation).
    click_total: u32,
    /// Whether the current click is accented (beat 1).
    accented: bool,
    /// Sine wave phase accumulator.
    phase: f32,
    /// Phase increment per sample.
    phase_inc: f32,
}

impl ClickGenerator {
    /// Create a new click generator.
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            enabled: false,
            volume: 0.6,
            sample_rate,
            click_remaining: 0,
            click_total: 0,
            accented: false,
            phase: 0.0,
            phase_inc: 0.0,
        }
    }

    /// Check if the metronome is enabled.
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the metronome.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set the volume (0.0–1.0).
    pub(crate) fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    /// Trigger a click sound.
    pub(crate) fn trigger_click(&mut self, accented: bool) {
        self.accented = accented;

        let freq = if accented { ACCENT_FREQ } else { NORMAL_FREQ };
        let duration_ms = if accented {
            ACCENT_DURATION_MS
        } else {
            NORMAL_DURATION_MS
        };

        let samples = (self.sample_rate * duration_ms / 1000.0) as u32;
        self.click_remaining = samples;
        self.click_total = samples;
        self.phase = 0.0;
        self.phase_inc = freq * std::f32::consts::TAU / self.sample_rate;
    }

    /// Mix click audio into an interleaved stereo buffer (additive).
    ///
    /// Real-time safe — no allocations.
    pub(crate) fn process(&mut self, buffer: &mut [f32], frames: usize) {
        if self.click_remaining == 0 {
            return;
        }

        let samples_to_process = frames.min(self.click_remaining as usize);

        for i in 0..samples_to_process {
            // Exponential decay envelope
            let progress = 1.0 - (self.click_remaining as f32 / self.click_total as f32);
            let envelope = (-progress * 5.0).exp();

            let sample = self.phase.sin() * envelope * self.volume;

            // Mix into stereo interleaved buffer
            let idx = i * 2;
            if idx + 1 < buffer.len() {
                buffer[idx] += sample;
                buffer[idx + 1] += sample;
            }

            self.phase += self.phase_inc;
            if self.phase >= std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }
            self.click_remaining -= 1;

            if self.click_remaining == 0 {
                break;
            }
        }
    }

    /// Update sample rate (e.g., on stream start).
    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_click_generator_creation() {
        let click = ClickGenerator::new(44100.0);
        assert!(!click.is_enabled());
        assert_eq!(click.click_remaining, 0);
    }

    #[test]
    fn test_trigger_click() {
        let mut click = ClickGenerator::new(44100.0);
        click.trigger_click(true);

        let expected_samples = (44100.0 * ACCENT_DURATION_MS / 1000.0) as u32;
        assert_eq!(click.click_remaining, expected_samples);
        assert!(click.accented);
    }

    #[test]
    fn test_process_generates_audio() {
        let mut click = ClickGenerator::new(44100.0);
        click.set_volume(1.0);
        click.trigger_click(false);

        let mut buffer = vec![0.0_f32; 256]; // 128 frames stereo
        click.process(&mut buffer, 128);

        // Should have generated non-zero audio
        let max = buffer.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(max > 0.01, "Click should produce audible output");
    }

    #[test]
    fn test_process_no_click() {
        let mut click = ClickGenerator::new(44100.0);
        let mut buffer = vec![0.0_f32; 256];
        click.process(&mut buffer, 128);

        // No click triggered, buffer should be unchanged
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_click_finishes() {
        let mut click = ClickGenerator::new(44100.0);
        click.trigger_click(false);

        // Process enough frames to exhaust the click
        let mut buffer = vec![0.0_f32; 44100 * 2];
        click.process(&mut buffer, 44100);

        assert_eq!(click.click_remaining, 0);
    }
}
