//! Per-oscillator glide (portamento) state + policy, shared by the pitched
//! oscillator modules.
//!
//! Wraps a [`PitchGlide`] smoother together with the `glide_time` parameter and
//! the last [`VoicePitch`], so a module only has to: store the incoming pitch in
//! `set_voice_pitch`, ask [`resolve`](OscGlide::resolve) once per block for the
//! base frequency to sound (or `None` when this oscillator's own glide is off),
//! and [`reset`](OscGlide::reset) on note/voice reset. The per-module field type
//! and clamp stay in the module (an oscillator clamps to `OSC_RANGE`, the SID
//! oscillator converts to a register, …), so this only owns the *timing* policy.

use synth_core::{Hertz, SampleCount, SampleRate, Seconds, VoicePitch};
use synth_dsp::PitchGlide;

/// Shared per-oscillator glide state.
#[derive(Debug, Clone, Copy)]
pub struct OscGlide {
    /// `0.0` = follow the voice-level glide (module keeps `pitch.played`);
    /// `> 0` = run our own glide toward the note target.
    glide_time: Seconds,
    /// Portamento smoother (log-frequency one-pole).
    smoother: PitchGlide,
    /// Last decomposed voice pitch delivered by the voice.
    voice_pitch: VoicePitch,
}

impl Default for OscGlide {
    fn default() -> Self {
        Self::new()
    }
}

impl OscGlide {
    /// A fresh glide: off, and seeded with a neutral voice pitch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            glide_time: Seconds::ZERO,
            smoother: PitchGlide::new(),
            voice_pitch: VoicePitch::tracking(Hertz::A4),
        }
    }

    /// Store the block's decomposed voice pitch (call from `set_voice_pitch`).
    pub fn store(&mut self, pitch: VoicePitch) {
        self.voice_pitch = pitch;
    }

    /// Set the glide time (clamped to `>= 0`).
    pub fn set_time(&mut self, time: Seconds) {
        self.glide_time = Seconds::new(time.as_f32().max(0.0));
    }

    /// The configured glide time.
    #[must_use]
    pub fn time(&self) -> Seconds {
        self.glide_time
    }

    /// Uninitialize the smoother so the first note after a reset snaps (a freshly
    /// allocated voice jumps to its first note instead of gliding from stale
    /// state); note changes while the voice stays alive glide.
    pub fn reset(&mut self) {
        self.smoother.reset();
    }

    /// The base frequency this oscillator should sound this block when its own
    /// glide is active, or `None` when `glide_time == 0` — in which case the
    /// module keeps the voice's `played` pitch it stored in `set_voice_pitch`.
    ///
    /// The smoother is advanced every block regardless (it tracks `note_target`
    /// while the glide is off) so enabling glide mid-note starts cleanly.
    pub fn resolve(&mut self, sample_rate: SampleRate, samples: SampleCount) -> Option<Hertz> {
        let block_dt = Seconds::new(samples.as_usize() as f32 / sample_rate.as_f32());
        let resolved = self
            .smoother
            .resolve(&self.voice_pitch, self.glide_time, block_dt);
        (self.glide_time.as_f32() > 0.0).then_some(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::MidiNote;

    const SR: SampleRate = SampleRate::DVD_QUALITY;
    const BLOCK: SampleCount = SampleCount::new(256);

    #[test]
    fn glide_off_returns_none() {
        let mut g = OscGlide::new();
        g.store(VoicePitch::tracking(Hertz::new(440.0)));
        assert!(g.resolve(SR, BLOCK).is_none());
    }

    #[test]
    fn glide_on_snaps_then_glides() {
        let mut g = OscGlide::new();
        g.set_time(Seconds::new(0.2));
        g.reset();
        g.store(VoicePitch::tracking(Hertz::new(220.0)));
        let first = g.resolve(SR, BLOCK).expect("glide on");
        assert!((first.as_f32() - 220.0).abs() < 0.5, "first note snaps");
        g.store(VoicePitch::tracking(Hertz::new(880.0)));
        let one = g.resolve(SR, BLOCK).expect("glide on").as_f32();
        assert!(one > 221.0 && one < 800.0, "one block partway, got {one}");
    }

    #[test]
    fn glide_on_applies_expr_after_glide() {
        let mut g = OscGlide::new();
        g.set_time(Seconds::new(0.2));
        g.reset();
        g.store(VoicePitch {
            played: Hertz::new(440.0),
            note_target: Hertz::new(440.0),
            expr: synth_core::Semitones::new(12.0),
            note: MidiNote::A4,
        });
        let f = g.resolve(SR, BLOCK).expect("glide on").as_f32();
        assert!(
            (f - 880.0).abs() < 2.0,
            "snap 440 + 12 semis = 880, got {f}"
        );
    }
}
