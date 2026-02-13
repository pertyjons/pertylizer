//! Example patches for the modular synthesizer.
//!
//! Each patch is defined in its own file for better organization.

mod acid_bass;
mod aggressive_bass;
mod ambient_keys;
mod brown_drone;
mod bytebeat_glitch;
mod chaos_drone;
mod deep_space_pad;
mod drum_hihat;
mod drum_kick;
mod drum_snare;
mod expressive_lead;
mod fluid_keys;
mod fluid_pad;
mod fm_bell;
mod formant_voice;
mod glitch_pad;
mod grand_piano;
mod karplus_guitar;
mod noise_sweep;
mod pluck_synth;
mod punchy_stab;
mod screamer_lead;
mod shepard_riser;
mod spacey_bass;
mod stereo_unison_pad;
mod string_ensemble;
mod sub_bass;
mod unison_pwm_strings;
mod unison_supersaw;
mod unison_sync_lead;
mod velocity_pad;
mod vintage_lead;
mod wave_folder_bass;
mod waveshaper_lead;

pub use acid_bass::patch_acid_bass;
pub use aggressive_bass::patch_aggressive_bass;
pub use ambient_keys::patch_ambient_keys;
pub use brown_drone::patch_brown_drone;
pub use bytebeat_glitch::patch_bytebeat_glitch;
pub use chaos_drone::patch_chaos_drone;
pub use deep_space_pad::patch_deep_space_pad;
pub use drum_hihat::patch_drum_hihat;
pub use drum_kick::patch_drum_kick;
pub use drum_snare::patch_drum_snare;
pub use expressive_lead::patch_expressive_lead;
pub use fluid_keys::patch_fluid_keys;
pub use fluid_pad::patch_fluid_pad;
pub use fm_bell::patch_fm_bell;
pub use formant_voice::patch_formant_voice;
pub use glitch_pad::patch_glitch_pad;
pub use grand_piano::patch_grand_piano;
pub use karplus_guitar::patch_karplus_guitar;
pub use noise_sweep::patch_noise_sweep;
pub use pluck_synth::patch_pluck_synth;
pub use punchy_stab::patch_punchy_stab;
pub use screamer_lead::patch_screamer_lead;
pub use shepard_riser::patch_shepard_riser;
pub use spacey_bass::patch_spacey_bass;
pub use stereo_unison_pad::patch_stereo_unison_pad;
pub use string_ensemble::patch_string_ensemble;
pub use sub_bass::patch_sub_bass;
pub use unison_pwm_strings::patch_unison_pwm_strings;
pub use unison_supersaw::patch_unison_supersaw;
pub use unison_sync_lead::patch_unison_sync_lead;
pub use velocity_pad::patch_velocity_pad;
pub use vintage_lead::patch_vintage_lead;
pub use wave_folder_bass::patch_wave_folder_bass;
pub use waveshaper_lead::patch_waveshaper_lead;

use crate::patch::Patch;

/// Get the default patch (loaded on startup).
pub fn default_patch() -> Patch {
    patch_grand_piano()
}

/// Get all example patches.
pub fn example_patches() -> Vec<Patch> {
    vec![
        patch_grand_piano(), // Default startup patch
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
        patch_string_ensemble(),
        // Mod Matrix patches
        patch_velocity_pad(),
        patch_expressive_lead(),
        // Waveshaper patches
        patch_waveshaper_lead(),
        patch_glitch_pad(),
        // Unison patches
        patch_unison_supersaw(),
        patch_stereo_unison_pad(),
        patch_unison_sync_lead(),
        patch_unison_pwm_strings(),
        // Character filter patches
        patch_fluid_pad(),
        patch_fluid_keys(),
        patch_screamer_lead(),
        patch_acid_bass(),
    ]
}
