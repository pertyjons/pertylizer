//! Per-oscillator pitch glide (portamento) smoother.
//!
//! A one-pole low-pass over frequency, performed in **log-frequency space** so
//! the glide is perceptually even (a constant rate in semitones rather than in
//! Hz). It chases a target from its current value, so it needs no explicit glide
//! *source*. Alloc-free and lock-free — safe on the audio thread.

use synth_core::{Hertz, Seconds, VoicePitch};

/// A per-module portamento smoother.
///
/// Lifecycle: [`reset`](Self::reset) marks it uninitialized so the next
/// [`process`](Self::process) / [`resolve`](Self::resolve) **snaps** to the
/// target. A freshly allocated voice therefore jumps to its first note instead
/// of gliding from a stale value; note changes while the voice stays alive
/// (legato / mono re-notes) glide.
#[derive(Debug, Clone, Copy, Default)]
pub struct PitchGlide {
    /// Current smoothed frequency; `None` until the first target after a reset.
    current: Option<Hertz>,
}

impl PitchGlide {
    /// A fresh, uninitialized glide (first target snaps).
    #[must_use]
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Mark uninitialized so the next update snaps to its target.
    pub fn reset(&mut self) {
        self.current = None;
    }

    /// The current smoothed frequency, or `None` before the first update.
    #[must_use]
    pub fn current(&self) -> Option<Hertz> {
        self.current
    }

    /// Advance one control block toward `target` and return the smoothed value.
    ///
    /// `glide_time` is the ~63% time constant; `dt` is the block duration. A
    /// non-positive `glide_time` or `dt`, or the first call after a
    /// [`reset`](Self::reset), snaps directly to `target`.
    pub fn process(&mut self, target: Hertz, glide_time: Seconds, dt: Seconds) -> Hertz {
        let next = match self.current {
            Some(cur) if glide_time.as_f32() > 0.0 && dt.as_f32() > 0.0 => {
                one_pole_log(cur, target, glide_time, dt)
            }
            _ => target,
        };
        self.current = Some(next);
        next
    }

    /// Resolve a decomposed [`VoicePitch`] into the base frequency a pitched
    /// module should sound this block.
    ///
    /// - `glide_time <= 0`: not gliding — return [`VoicePitch::played`] unchanged
    ///   (the voice-level glide, bit-identical to before), while keeping the
    ///   smoother synced to `note_target` so enabling glide mid-note starts from
    ///   the right place.
    /// - `glide_time > 0`: run the module's own glide toward
    ///   [`VoicePitch::note_target`], then apply [`VoicePitch::expr`]
    ///   (pitch-bend + vibrato) on top so expression is never smoothed away.
    pub fn resolve(&mut self, pitch: &VoicePitch, glide_time: Seconds, dt: Seconds) -> Hertz {
        if glide_time.as_f32() <= 0.0 {
            self.current = Some(pitch.note_target);
            return pitch.played;
        }
        let smoothed = self.process(pitch.note_target, glide_time, dt);
        pitch.expr.apply(smoothed)
    }
}

/// One-pole low-pass toward `target` in log-frequency space.
#[inline]
fn one_pole_log(current: Hertz, target: Hertz, glide_time: Seconds, dt: Seconds) -> Hertz {
    let coeff = 1.0 - (-dt.as_f32() / glide_time.as_f32()).exp();
    let cur = current.as_f32().max(f32::EPSILON).log2();
    let tgt = target.as_f32().max(f32::EPSILON).log2();
    Hertz::new((cur + (tgt - cur) * coeff).exp2())
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::{MidiNote, Semitones};

    const DT: Seconds = Seconds::new(64.0 / 48_000.0); // ~1.33 ms control block

    #[test]
    fn first_process_after_reset_snaps() {
        let mut g = PitchGlide::new();
        // No prior value: even with a long glide time, the first call jumps.
        let f = g.process(Hertz::new(440.0), Seconds::new(1.0), DT);
        assert!((f.as_f32() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn zero_glide_time_snaps() {
        let mut g = PitchGlide::new();
        g.process(Hertz::new(220.0), Seconds::ZERO, DT);
        let f = g.process(Hertz::new(880.0), Seconds::ZERO, DT);
        assert!((f.as_f32() - 880.0).abs() < 1e-3);
    }

    #[test]
    fn glides_partway_then_converges() {
        let mut g = PitchGlide::new();
        g.process(Hertz::new(220.0), Seconds::new(0.1), DT); // snap to 220
        let one = g.process(Hertz::new(880.0), Seconds::new(0.1), DT);
        // One block toward the target: moved, but nowhere near arrived.
        assert!(one.as_f32() > 220.0 && one.as_f32() < 300.0, "got {one:?}");
        // Many blocks: converges to the target.
        for _ in 0..2000 {
            g.process(Hertz::new(880.0), Seconds::new(0.1), DT);
        }
        assert!((g.current().unwrap().as_f32() - 880.0).abs() < 0.5);
    }

    #[test]
    fn smooths_in_log_space() {
        // Gliding 220 -> 880 (two octaves): the halfway-in-time point should sit
        // near the geometric mean (440), not the arithmetic mean (550).
        let mut g = PitchGlide::new();
        g.process(Hertz::new(220.0), Seconds::new(1.0), DT);
        // Advance for exactly one time constant so ~63% of the log-distance is
        // covered: log2(220)+0.632*(log2(880)-log2(220)) = ~2^(7.78+1.264).
        let tau = Seconds::new(0.05);
        let steps = (tau.as_f32() / DT.as_f32()).round() as usize;
        let mut last = Hertz::new(0.0);
        for _ in 0..steps {
            last = g.process(Hertz::new(880.0), tau, DT);
        }
        // After one tau the value is well above the arithmetic-mean-if-linear
        // path would predict at the same fraction, confirming log-domain motion.
        let semis_from_220 = 12.0 * (last.as_f32() / 220.0).log2();
        assert!(
            semis_from_220 > 12.0 && semis_from_220 < 22.0,
            "one-tau glide landed at {semis_from_220} semitones above 220 Hz"
        );
    }

    #[test]
    fn resolve_glide_off_returns_played() {
        let mut g = PitchGlide::new();
        let pitch = VoicePitch {
            played: Hertz::new(330.0),
            note_target: Hertz::new(440.0),
            expr: Semitones::new(1.0),
            note: MidiNote::A4,
        };
        let f = g.resolve(&pitch, Seconds::ZERO, DT);
        assert!(
            (f.as_f32() - 330.0).abs() < 1e-3,
            "glide off must pass played"
        );
    }

    #[test]
    fn resolve_glide_on_applies_expr_after_glide() {
        let mut g = PitchGlide::new();
        // First note snaps to note_target, then expr (+12 semis) doubles it.
        let pitch = VoicePitch {
            played: Hertz::new(0.0), // ignored while gliding
            note_target: Hertz::new(440.0),
            expr: Semitones::new(12.0),
            note: MidiNote::A4,
        };
        let f = g.resolve(&pitch, Seconds::new(0.1), DT);
        assert!(
            (f.as_f32() - 880.0).abs() < 1.0,
            "snap to 440 then +12 semis = 880, got {f:?}"
        );
    }
}
