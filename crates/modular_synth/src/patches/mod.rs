//! Example patches for the modular synthesizer.
//!
//! Each patch is defined in its own file for better organization.

mod acid_bass;
mod aggressive_bass;
mod ambient_keys;
mod auto_wah_bass;
mod brown_drone;
mod bytebeat_glitch;
mod chaos_drone;
mod deep_space_pad;
mod digital_chime;
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
mod harmonic_lead;
mod hybrid_resonator;
mod karplus_guitar;
mod kinetic_pad;
mod kinetic_pluck;
mod la_synth_pluck;
mod metallic_bell;
mod moog_resonant_sweep;
mod noise_sweep;
mod pluck_synth;
mod punchy_stab;
mod pwm_epiano;
mod ring_mod_drone;
mod screamer_lead;
mod shepard_riser;
mod spacey_bass;
mod spectral_drone;
mod stereo_unison_pad;
mod string_ensemble;
mod sub_bass;
mod unison_pwm_strings;
mod unison_supersaw;
mod unison_sync_lead;
mod vector_pad;
mod velocity_pad;
mod vintage_lead;
mod vocal_pad;
mod warm_evolving;
mod wave_folder_bass;
mod waveshaper_lead;

pub use acid_bass::patch_acid_bass;
pub use aggressive_bass::patch_aggressive_bass;
pub use ambient_keys::patch_ambient_keys;
pub use auto_wah_bass::patch_auto_wah_bass;
pub use brown_drone::patch_brown_drone;
pub use bytebeat_glitch::patch_bytebeat_glitch;
pub use chaos_drone::patch_chaos_drone;
pub use deep_space_pad::patch_deep_space_pad;
pub use digital_chime::patch_digital_chime;
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
pub use harmonic_lead::patch_harmonic_lead;
pub use hybrid_resonator::patch_hybrid_resonator;
pub use karplus_guitar::patch_karplus_guitar;
pub use kinetic_pad::patch_kinetic_pad;
pub use kinetic_pluck::patch_kinetic_pluck;
pub use la_synth_pluck::patch_la_synth_pluck;
pub use metallic_bell::patch_metallic_bell;
pub use moog_resonant_sweep::patch_moog_resonant_sweep;
pub use noise_sweep::patch_noise_sweep;
pub use pluck_synth::patch_pluck_synth;
pub use punchy_stab::patch_punchy_stab;
pub use pwm_epiano::patch_pwm_epiano;
pub use ring_mod_drone::patch_ring_mod_drone;
pub use screamer_lead::patch_screamer_lead;
pub use shepard_riser::patch_shepard_riser;
pub use spacey_bass::patch_spacey_bass;
pub use spectral_drone::patch_spectral_drone;
pub use stereo_unison_pad::patch_stereo_unison_pad;
pub use string_ensemble::patch_string_ensemble;
pub use sub_bass::patch_sub_bass;
pub use unison_pwm_strings::patch_unison_pwm_strings;
pub use unison_supersaw::patch_unison_supersaw;
pub use unison_sync_lead::patch_unison_sync_lead;
pub use vector_pad::patch_vector_pad;
pub use velocity_pad::patch_velocity_pad;
pub use vintage_lead::patch_vintage_lead;
pub use vocal_pad::patch_vocal_pad;
pub use warm_evolving::patch_warm_evolving;
pub use wave_folder_bass::patch_wave_folder_bass;
pub use waveshaper_lead::patch_waveshaper_lead;

use crate::patch::Patch;

/// Get the default patch (loaded on startup).
pub fn default_patch() -> Patch {
    patch_grand_piano()
}

/// Get all example patches.
pub fn example_patches() -> Vec<Patch> {
    categorized_patches()
        .into_iter()
        .flat_map(|(_, patches)| patches)
        .collect()
}

/// Get example patches grouped by category.
#[must_use]
pub fn categorized_patches() -> Vec<(&'static str, Vec<Patch>)> {
    vec![
        (
            "\u{1f3b9} Keys & Piano",
            vec![
                patch_grand_piano(),
                patch_ambient_keys(),
                patch_fluid_keys(),
                patch_pwm_epiano(),
            ],
        ),
        (
            "\u{1f3b8} Bass",
            vec![
                patch_spacey_bass(),
                patch_aggressive_bass(),
                patch_sub_bass(),
                patch_acid_bass(),
                patch_wave_folder_bass(),
                patch_auto_wah_bass(),
            ],
        ),
        (
            "\u{1f3b5} Lead",
            vec![
                patch_vintage_lead(),
                patch_moog_resonant_sweep(),
                patch_expressive_lead(),
                patch_waveshaper_lead(),
                patch_screamer_lead(),
                patch_unison_supersaw(),
                patch_unison_sync_lead(),
                patch_harmonic_lead(),
            ],
        ),
        (
            "\u{1f30a} Pad",
            vec![
                patch_deep_space_pad(),
                patch_velocity_pad(),
                patch_glitch_pad(),
                patch_fluid_pad(),
                patch_stereo_unison_pad(),
                patch_vocal_pad(),
                patch_vector_pad(),
            ],
        ),
        (
            "\u{1f941} Drums",
            vec![patch_drum_kick(), patch_drum_snare(), patch_drum_hihat()],
        ),
        (
            "\u{1f3bb} Strings & Bell",
            vec![
                patch_string_ensemble(),
                patch_unison_pwm_strings(),
                patch_fm_bell(),
                patch_metallic_bell(),
                patch_pluck_synth(),
                patch_punchy_stab(),
            ],
        ),
        (
            "\u{1f52c} Experimental",
            vec![
                patch_chaos_drone(),
                patch_karplus_guitar(),
                patch_shepard_riser(),
                patch_bytebeat_glitch(),
                patch_formant_voice(),
                patch_hybrid_resonator(),
                patch_digital_chime(),
                patch_kinetic_pluck(),
                patch_kinetic_pad(),
                patch_la_synth_pluck(),
            ],
        ),
        (
            "\u{1f32b}\u{fe0f} Ambient & Texture",
            vec![
                patch_brown_drone(),
                patch_noise_sweep(),
                patch_ring_mod_drone(),
                patch_warm_evolving(),
                patch_spectral_drone(),
            ],
        ),
    ]
}
