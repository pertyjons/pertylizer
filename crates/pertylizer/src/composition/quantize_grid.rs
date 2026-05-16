//! Snap note start ticks to a grid with optional swing and humanization.
//!
//! The existing `Pattern::quantize_*` helpers do this in-place against the
//! pattern's hard-coded row resolution; the MCP tool needs a deterministic
//! single call that accepts an explicit grid in ticks and optional seeded
//! humanization (the AI agent reuses a seed when it wants to A/B compare).

/// Single-note view passed to [`quantize_grid`]. The caller is responsible
/// for assembling these from `Pattern::notes()` and writing the results back
/// through `Pattern::move_note`.
#[derive(Debug, Clone, Copy)]
pub struct NoteTiming {
    pub start_tick: u32,
}

/// Options for [`quantize_grid`].
pub struct GridQuantizeOptions {
    /// Grid resolution in ticks (e.g. 240 = sixteenth at 960 PPQN).
    pub grid_ticks: u32,
    /// Pattern length in ticks — used to clamp the final start so swing /
    /// humanize never push notes past the end of the pattern.
    pub pattern_length_ticks: u32,
    /// Quantize strength in 0..=1. `1.0` = full snap, `0.5` = halfway
    /// between original and grid, `0.0` = no movement.
    pub strength: f32,
    /// Swing amount in 0..=1. Even-indexed grid positions stay put, odd
    /// positions get pushed back by up to half the grid distance.
    pub swing: f32,
    /// Maximum random timing offset in ticks per note (deterministic with
    /// `seed`). `0` disables humanization.
    pub humanize_ticks: u32,
    /// Seed for humanization. Same seed + same notes → same output.
    pub seed: u64,
}

/// Per-pass diagnostics returned by [`quantize_grid`].
#[must_use]
#[derive(Debug, Clone, Copy, Default)]
pub struct GridQuantizeResult {
    pub notes_in: u32,
    pub notes_moved: u32,
    /// Sum of absolute tick deltas (input → output). Divide by `notes_moved`
    /// for the average correction; divide by `notes_in` for the average
    /// drift relative to the input.
    pub total_delta_ticks: u64,
    pub max_delta_ticks: u32,
    /// True when the requested `grid_ticks` was zero (no-op).
    pub disabled: bool,
}

/// Quantize `notes` in place. Returns aggregate stats. Pure function — given
/// the same `notes`, `options`, and seed it produces identical output.
pub fn quantize_grid(
    notes: &mut [NoteTiming],
    options: &GridQuantizeOptions,
) -> GridQuantizeResult {
    let mut out = GridQuantizeResult {
        notes_in: notes.len() as u32,
        ..GridQuantizeResult::default()
    };
    if options.grid_ticks == 0 {
        out.disabled = true;
        return out;
    }
    let grid = options.grid_ticks;
    let strength = options.strength.clamp(0.0, 1.0);
    let swing = options.swing.clamp(0.0, 1.0);
    let humanize = options.humanize_ticks;
    let max_tick = options.pattern_length_ticks.saturating_sub(1);

    let mut rng = fastrand::Rng::with_seed(options.seed);

    for note in notes.iter_mut() {
        let original = note.start_tick;
        let nearest = ((original + grid / 2) / grid) * grid;
        let blended: i64 = {
            let diff = nearest as f32 - original as f32;
            (original as f32 + diff * strength).round() as i64
        };
        #[allow(clippy::cast_sign_loss)]
        let blended_u = blended.max(0) as u64;
        let grid_pos = blended_u / u64::from(grid);
        let swung = if grid_pos % 2 == 1 {
            let shift = (i64::from(grid) as f32 * 0.5 * swing) as i64;
            blended + shift
        } else {
            blended
        };
        let humanized = if humanize > 0 {
            let range = i64::from(humanize);
            swung + rng.i64(-range..=range)
        } else {
            swung
        };
        let clamped = humanized.clamp(0, i64::from(max_tick));
        #[allow(clippy::cast_sign_loss)]
        let final_tick = clamped as u32;
        let delta = u32::try_from((i64::from(final_tick) - i64::from(original)).unsigned_abs())
            .unwrap_or(u32::MAX);
        if delta > 0 {
            out.notes_moved += 1;
            out.total_delta_ticks += u64::from(delta);
            if delta > out.max_delta_ticks {
                out.max_delta_ticks = delta;
            }
        }
        note.start_tick = final_tick;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(grid: u32, length: u32) -> GridQuantizeOptions {
        GridQuantizeOptions {
            grid_ticks: grid,
            pattern_length_ticks: length,
            strength: 1.0,
            swing: 0.0,
            humanize_ticks: 0,
            seed: 0,
        }
    }

    #[test]
    fn snaps_to_nearest_grid_position() {
        // 240-tick grid (16ths at 960 PPQN). 200 rounds up to 240; 470 rounds
        // up to 480; 481 is closer to 480 than 720 so stays at 480.
        let mut notes = [
            NoteTiming { start_tick: 200 },
            NoteTiming { start_tick: 470 },
            NoteTiming { start_tick: 481 },
        ];
        let r = quantize_grid(&mut notes, &opts(240, 3840));
        assert_eq!(notes[0].start_tick, 240);
        assert_eq!(notes[1].start_tick, 480);
        assert_eq!(notes[2].start_tick, 480);
        assert_eq!(r.notes_moved, 3);
    }

    #[test]
    fn strength_halfway_blends() {
        // 200 → nearest 240 at distance 40, strength 0.5 → new_tick = 220.
        let mut notes = [NoteTiming { start_tick: 200 }];
        let mut options = opts(240, 3840);
        options.strength = 0.5;
        let _ = quantize_grid(&mut notes, &options);
        assert_eq!(notes[0].start_tick, 220);
    }

    #[test]
    fn zero_grid_returns_disabled_and_does_not_move_notes() {
        let mut notes = [NoteTiming { start_tick: 100 }];
        let r = quantize_grid(&mut notes, &opts(0, 3840));
        assert!(r.disabled);
        assert_eq!(notes[0].start_tick, 100);
    }

    #[test]
    fn swing_pushes_odd_grid_positions_back() {
        // Snap to 480 grid first, then swing 100% pushes odd positions by
        // grid/2 = 240. Position 0 (tick 0) doesn't move; position 1 (tick
        // 480) moves to 720.
        let mut notes = [
            NoteTiming { start_tick: 0 },
            NoteTiming { start_tick: 480 },
            NoteTiming { start_tick: 960 },
            NoteTiming { start_tick: 1440 },
        ];
        let mut options = opts(480, 3840);
        options.swing = 1.0;
        let _ = quantize_grid(&mut notes, &options);
        assert_eq!(notes[0].start_tick, 0);
        assert_eq!(notes[1].start_tick, 720);
        assert_eq!(notes[2].start_tick, 960);
        assert_eq!(notes[3].start_tick, 1680);
    }

    #[test]
    fn humanize_is_deterministic_for_same_seed() {
        let mut a = [
            NoteTiming { start_tick: 480 },
            NoteTiming { start_tick: 960 },
        ];
        let mut b = a;
        let mut options = opts(480, 3840);
        options.humanize_ticks = 20;
        options.seed = 42;
        let _ = quantize_grid(&mut a, &options);
        let _ = quantize_grid(&mut b, &options);
        assert_eq!(a[0].start_tick, b[0].start_tick);
        assert_eq!(a[1].start_tick, b[1].start_tick);
    }

    #[test]
    fn humanize_changes_with_different_seed() {
        let mut a = [
            NoteTiming { start_tick: 480 },
            NoteTiming { start_tick: 960 },
        ];
        let mut b = a;
        let mut options = opts(480, 3840);
        options.humanize_ticks = 20;
        options.seed = 1;
        let _ = quantize_grid(&mut a, &options);
        options.seed = 2;
        let _ = quantize_grid(&mut b, &options);
        let any_different =
            a[0].start_tick != b[0].start_tick || a[1].start_tick != b[1].start_tick;
        assert!(any_different);
    }

    #[test]
    fn output_never_exceeds_pattern_length() {
        let mut notes = [NoteTiming { start_tick: 3800 }];
        let mut options = opts(480, 3840);
        options.swing = 1.0;
        options.humanize_ticks = 200;
        options.seed = 7;
        let _ = quantize_grid(&mut notes, &options);
        assert!(notes[0].start_tick < 3840);
    }
}
