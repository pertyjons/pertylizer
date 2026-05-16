//! Shared bar-level repetition scoring used by `pattern_analysis` and
//! `drum_groove`. The algorithm is "quantize each note's in-bar tick onto a
//! 32nd-note grid, hash each bar's sorted signature, count distinct bars,
//! score = `1 - (distinct - 1) / (total - 1)`".

use std::collections::HashSet;
use std::hash::Hash;

use synth_sequencer::TICKS_PER_QUARTER;

/// 32nd-note quantization step for the bar-signature canonical hash.
pub(crate) const REPETITION_GRID_TICKS: u32 = TICKS_PER_QUARTER / 8;

/// One note projected into the bar-signature space.
///
/// `bar` is the 0-based bar index; `tick_in_bar` is the note's in-bar start
/// (already quantized to [`REPETITION_GRID_TICKS`]); `discriminant` is
/// whatever per-note tuple makes two bars "the same" — for melodic patterns
/// it's `midi_pitch`, for drums it's `(midi_pitch, component)`.
#[derive(Debug, Clone)]
pub(crate) struct BarSignatureNote<T> {
    pub bar: usize,
    pub tick_in_bar: u32,
    pub discriminant: T,
}

/// Compute the bar-level repetition score.
///
/// Returns `(distinct_bars, score)` where `score = 1.0 - (distinct - 1) /
/// (total_bars - 1)` clamped to `[0, 1]`. For `total_bars < 2` the score is
/// `0.0` (no repetition to measure) and `distinct_bars` is `1`; callers
/// typically push a "shorter than 2 bars" warning in that case.
pub(crate) fn bar_repetition_score<T>(notes: &[BarSignatureNote<T>], total_bars: u32) -> (u32, f32)
where
    T: Eq + Hash + Ord + Clone,
{
    if total_bars < 2 {
        return (1, 0.0);
    }
    let mut sigs: Vec<Vec<(u32, T)>> = vec![Vec::new(); total_bars as usize];
    for n in notes {
        if let Some(slot) = sigs.get_mut(n.bar) {
            slot.push((n.tick_in_bar, n.discriminant.clone()));
        }
    }
    for sig in &mut sigs {
        sig.sort_unstable();
    }
    let distinct = sigs.iter().collect::<HashSet<_>>().len() as u32;
    let denom = (total_bars - 1) as f32;
    let score = (1.0 - (distinct.saturating_sub(1)) as f32 / denom).clamp(0.0, 1.0);
    (distinct, score)
}
