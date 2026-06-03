//! Example patches for Pertylizer.
//!
//! Each patch is defined in its own file for better organization.

mod acid_bass;
mod aggressive_bass;
mod ambient_keys;
mod analog_dream_machine;
mod auto_wah_bass;
mod brown_drone;
mod bytebeat_glitch;
mod chaos_drone;
mod choir;
mod deep_space_pad;
mod digital_chime;
mod drum_hihat;
mod drum_kick;
mod drum_snare;
mod ethereal_shimmer_pad;
mod euclidean_texture;
mod expressive_lead;
mod fluid_keys;
mod fluid_pad;
mod fm_bell;
mod formant_voice;
mod fractal_cosmos;
mod glitch_pad;
mod grand_piano;
mod granular_cathedral;
mod granular_storm;
mod harmonic_lead;
mod hybrid_resonator;
mod karplus_guitar;
mod kinetic_pad;
mod kinetic_pluck;
mod la_synth_pluck;
mod metallic_bell;
mod moog_resonant_sweep;
mod mseg_crystal_lead;
mod noise_sweep;
mod pitch_following_drone;
mod pluck_synth;
mod punchy_stab;
mod pwm_epiano;
mod resonant_percussion;
mod ring_mod_drone;
mod satb_alto;
mod satb_bass;
mod satb_soprano;
mod satb_tenor;
mod screamer_lead;
mod shepard_riser;
mod solo_voice;
mod spacey_bass;
mod spectral_drone;
mod spectral_freeze_pad;
mod stereo_unison_pad;
mod string_ensemble;
mod sub_bass;
mod unison_pwm_strings;
mod unison_supersaw;
mod unison_sync_lead;
mod vector_pad;
mod velocity_pad;
mod vintage_electric_piano;
mod vintage_lead;
mod vocal_pad;
mod vocal_tract;
mod warm_evolving;
mod wave_folder_bass;
mod waveshaper_lead;

pub use acid_bass::patch_acid_bass;
pub use aggressive_bass::patch_aggressive_bass;
pub use ambient_keys::patch_ambient_keys;
pub use analog_dream_machine::patch_analog_dream_machine;
pub use auto_wah_bass::patch_auto_wah_bass;
pub use brown_drone::patch_brown_drone;
pub use bytebeat_glitch::patch_bytebeat_glitch;
pub use chaos_drone::patch_chaos_drone;
pub use choir::patch_choir;
pub use deep_space_pad::patch_deep_space_pad;
pub use digital_chime::patch_digital_chime;
pub use drum_hihat::patch_drum_hihat;
pub use drum_kick::patch_drum_kick;
pub use drum_snare::patch_drum_snare;
pub use ethereal_shimmer_pad::patch_ethereal_shimmer_pad;
pub use euclidean_texture::patch_euclidean_texture;
pub use expressive_lead::patch_expressive_lead;
pub use fluid_keys::patch_fluid_keys;
pub use fluid_pad::patch_fluid_pad;
pub use fm_bell::patch_fm_bell;
pub use formant_voice::patch_formant_voice;
pub use fractal_cosmos::patch_fractal_cosmos;
pub use glitch_pad::patch_glitch_pad;
pub use grand_piano::patch_grand_piano;
pub use granular_cathedral::patch_granular_cathedral;
pub use granular_storm::patch_granular_storm;
pub use harmonic_lead::patch_harmonic_lead;
pub use hybrid_resonator::patch_hybrid_resonator;
pub use karplus_guitar::patch_karplus_guitar;
pub use kinetic_pad::patch_kinetic_pad;
pub use kinetic_pluck::patch_kinetic_pluck;
pub use la_synth_pluck::patch_la_synth_pluck;
pub use metallic_bell::patch_metallic_bell;
pub use moog_resonant_sweep::patch_moog_resonant_sweep;
pub use mseg_crystal_lead::patch_mseg_crystal_lead;
pub use noise_sweep::patch_noise_sweep;
pub use pitch_following_drone::patch_pitch_following_drone;
pub use pluck_synth::patch_pluck_synth;
pub use punchy_stab::patch_punchy_stab;
pub use pwm_epiano::patch_pwm_epiano;
pub use resonant_percussion::patch_resonant_percussion;
pub use ring_mod_drone::patch_ring_mod_drone;
pub use satb_alto::patch_satb_alto;
pub use satb_bass::patch_satb_bass;
pub use satb_soprano::patch_satb_soprano;
pub use satb_tenor::patch_satb_tenor;
pub use screamer_lead::patch_screamer_lead;
pub use shepard_riser::patch_shepard_riser;
pub use solo_voice::patch_solo_voice;
pub use spacey_bass::patch_spacey_bass;
pub use spectral_drone::patch_spectral_drone;
pub use spectral_freeze_pad::patch_spectral_freeze_pad;
pub use stereo_unison_pad::patch_stereo_unison_pad;
pub use string_ensemble::patch_string_ensemble;
pub use sub_bass::patch_sub_bass;
pub use unison_pwm_strings::patch_unison_pwm_strings;
pub use unison_supersaw::patch_unison_supersaw;
pub use unison_sync_lead::patch_unison_sync_lead;
pub use vector_pad::patch_vector_pad;
pub use velocity_pad::patch_velocity_pad;
pub use vintage_electric_piano::patch_vintage_electric_piano;
pub use vintage_lead::patch_vintage_lead;
pub use vocal_pad::patch_vocal_pad;
pub use vocal_tract::patch_vocal_tract;
pub use warm_evolving::patch_warm_evolving;
pub use wave_folder_bass::patch_wave_folder_bass;
pub use waveshaper_lead::patch_waveshaper_lead;

use crate::patch::Patch;
use egui_remixicon::icons as ri;

/// Get the default patch (loaded on startup).
pub fn default_patch() -> Patch {
    patch_moog_resonant_sweep()
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
pub fn categorized_patches() -> Vec<(String, Vec<Patch>)> {
    vec![
        (
            format!("{} Keys & Piano", ri::PIANO_FILL),
            vec![
                patch_grand_piano(),
                patch_ambient_keys(),
                patch_fluid_keys(),
                patch_pwm_epiano(),
                patch_vintage_electric_piano(),
            ],
        ),
        (
            format!("{} Bass", ri::VOICEPRINT_FILL),
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
            format!("{} Lead", ri::MUSIC_FILL),
            vec![
                patch_vintage_lead(),
                patch_moog_resonant_sweep(),
                patch_expressive_lead(),
                patch_waveshaper_lead(),
                patch_screamer_lead(),
                patch_unison_supersaw(),
                patch_unison_sync_lead(),
                patch_harmonic_lead(),
                patch_mseg_crystal_lead(),
            ],
        ),
        (
            format!("{} Pad", ri::HAZE_FILL),
            vec![
                patch_deep_space_pad(),
                patch_velocity_pad(),
                patch_glitch_pad(),
                patch_fluid_pad(),
                patch_stereo_unison_pad(),
                patch_vocal_pad(),
                patch_vector_pad(),
                patch_ethereal_shimmer_pad(),
                patch_spectral_freeze_pad(),
                patch_fractal_cosmos(),
            ],
        ),
        (
            format!("{} Drums", ri::RHYTHM_FILL),
            vec![
                patch_drum_kick(),
                patch_drum_snare(),
                patch_drum_hihat(),
                patch_resonant_percussion(),
            ],
        ),
        (
            format!("{} Strings & Bell", ri::BELL_FILL),
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
            format!("{} Experimental", ri::FLASK_FILL),
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
                patch_euclidean_texture(),
                patch_pitch_following_drone(),
            ],
        ),
        (
            format!("{} Vocal & Choir", ri::USER_VOICE_FILL),
            vec![
                patch_solo_voice(),
                patch_choir(),
                patch_vocal_tract(),
                patch_satb_soprano(),
                patch_satb_alto(),
                patch_satb_tenor(),
                patch_satb_bass(),
            ],
        ),
        (
            format!("{} Ambient & Texture", ri::MIST_FILL),
            vec![
                patch_brown_drone(),
                patch_noise_sweep(),
                patch_ring_mod_drone(),
                patch_warm_evolving(),
                patch_spectral_drone(),
                patch_granular_cathedral(),
                patch_granular_storm(),
                patch_analog_dream_machine(),
            ],
        ),
    ]
}
