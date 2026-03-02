//! Built-in group templates for common synthesizer building blocks.
//!
//! Each template is defined in its own file, mirroring the `patches/` pattern.

mod am_synthesis;
mod basic_synth_voice;
mod chorus_reverb;
mod delay_reverb;
mod distortion_eq;
mod dual_envelope;
mod dual_oscillator_voice;
mod filter_sweep;
mod fm_pair;
mod slow_modulation;
mod subtractive_101;
mod vibrato_lfo;

use crate::patch::{GroupCategory, GroupTemplate};

/// Get all built-in group templates grouped by category.
#[must_use]
pub fn categorized_group_templates() -> Vec<(GroupCategory, Vec<GroupTemplate>)> {
    vec![
        (
            GroupCategory::Voice,
            vec![
                basic_synth_voice::template(),
                dual_oscillator_voice::template(),
                fm_pair::template(),
            ],
        ),
        (
            GroupCategory::Effect,
            vec![
                chorus_reverb::template(),
                delay_reverb::template(),
                distortion_eq::template(),
            ],
        ),
        (
            GroupCategory::Utility,
            vec![
                filter_sweep::template(),
                vibrato_lfo::template(),
                slow_modulation::template(),
                dual_envelope::template(),
            ],
        ),
        (
            GroupCategory::Tutorial,
            vec![subtractive_101::template(), am_synthesis::template()],
        ),
    ]
}

/// Get all built-in group templates as a flat list.
#[must_use]
pub fn builtin_group_templates() -> Vec<GroupTemplate> {
    categorized_group_templates()
        .into_iter()
        .flat_map(|(_, templates)| templates)
        .collect()
}
