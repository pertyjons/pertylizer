//! Example patches for the modular synthesizer.
//!
//! Each patch is defined in its own file for better organization.

mod deep_space_pad;
mod spacey_bass;
mod aggressive_bass;
mod vintage_lead;
mod ambient_keys;
mod drum_kick;
mod drum_snare;
mod drum_hihat;
mod pluck_synth;
mod fm_bell;
mod noise_sweep;
mod chaos_drone;
mod karplus_guitar;
mod shepard_riser;
mod bytebeat_glitch;
mod wave_folder_bass;
mod formant_voice;
mod sub_bass;
mod brown_drone;
mod punchy_stab;
mod string_ensemble;

pub use deep_space_pad::patch_deep_space_pad;
pub use spacey_bass::patch_spacey_bass;
pub use aggressive_bass::patch_aggressive_bass;
pub use vintage_lead::patch_vintage_lead;
pub use ambient_keys::patch_ambient_keys;
pub use drum_kick::patch_drum_kick;
pub use drum_snare::patch_drum_snare;
pub use drum_hihat::patch_drum_hihat;
pub use pluck_synth::patch_pluck_synth;
pub use fm_bell::patch_fm_bell;
pub use noise_sweep::patch_noise_sweep;
pub use chaos_drone::patch_chaos_drone;
pub use karplus_guitar::patch_karplus_guitar;
pub use shepard_riser::patch_shepard_riser;
pub use bytebeat_glitch::patch_bytebeat_glitch;
pub use wave_folder_bass::patch_wave_folder_bass;
pub use formant_voice::patch_formant_voice;
pub use sub_bass::patch_sub_bass;
pub use brown_drone::patch_brown_drone;
pub use string_ensemble::patch_string_ensemble;
pub use punchy_stab::patch_punchy_stab;

use crate::patch::Patch;

/// Get all example patches.
pub fn example_patches() -> Vec<Patch> {
    vec![
        patch_string_ensemble(), // Default startup patch
        patch_spacey_bass(),
        patch_deep_space_pad(),
        patch_aggressive_bass(),
        patch_vintage_lead(),
        patch_ambient_keys(),
        patch_drum_kick(),
        patch_drum_snare(),
        patch_drum_hihat(),
        patch_pluck_synth(),
        patch_fm_bell(),
        patch_noise_sweep(),
        // Math oscillator patches
        patch_chaos_drone(),
        patch_karplus_guitar(),
        patch_shepard_riser(),
        patch_bytebeat_glitch(),
        patch_wave_folder_bass(),
        patch_formant_voice(),
        // New DSP feature patches
        patch_sub_bass(),
        patch_brown_drone(),
        patch_punchy_stab(),
    ]
}
