//! Synth modules and audio effects for Pertylizer.
//!
//! This crate provides ready-to-use audio modules:
//! - Oscillators (sine, saw, square, etc.)
//! - Sub-oscillator (bass reinforcement)
//! - Noise generator (textures and percussion)
//! - Filters (lowpass, highpass, bandpass)
//! - Envelopes (ADSR)
//! - LFOs (modulation sources)
//! - Amplifiers/VCAs (level control)
//! - Output (final stereo output)
//! - Physical modeling (strings, resonators, body)
//! - Effects (delay, reverb, distortion, chorus)

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]

pub mod math;

pub mod additive_osc;
pub mod am_formant;
pub mod amplifier;
pub mod audio_input;
pub mod audio_script;
pub mod beat_detector;
pub mod chaotic_osc;
pub mod drift_generator;
pub mod envelope;
pub mod envelope_follower;
pub mod euclidean;
pub mod filter;
pub mod fof;
pub mod fooglers;
pub mod formant_filter;
pub mod formant_tables;
pub mod fractal_osc;
pub mod granular_osc;
pub mod kinetic_modulator;
pub mod la_synth;
pub mod lfo;
pub mod math_oscillator;
pub mod mod_matrix;
pub mod mseg;
pub mod noise;
pub mod oscillator;
pub mod output;
pub mod padsynth;
pub mod pitch_tracker;
pub mod random_gates;
pub mod ring_mod;
pub mod sampler;
pub mod script_module;
pub mod sid_oscillator;
pub mod signal_monitor;
pub mod sub_osc;
pub mod turing_machine;
pub mod vector_mixer;
pub mod vocal_tract;
pub mod voice_common;
pub mod voice_synth;
pub mod wavetable_data;
pub mod wavetable_osc;

#[cfg(test)]
mod voice_pitch_harness;

// Physical modeling modules
pub mod body_resonance;
pub mod keyboard_panner;
pub mod mechanical_noise;

// Effects
pub mod effects;

// Module exports
pub use additive_osc::AdditiveOsc;
pub use am_formant::AmFormant;
pub use amplifier::{Amplifier, Mixer};
pub use audio_input::AudioInput;
pub use audio_script::AudioScript;
pub use beat_detector::BeatDetector;
pub use chaotic_osc::ChaoticOsc;
pub use drift_generator::DriftGenerator;
pub use envelope::{Envelope, EnvelopePositionBuffer, EnvelopeStage};
pub use envelope_follower::EnvelopeFollower;
pub use euclidean::Euclidean;
pub use filter::{Filter, LadderFilter};
pub use fof::Fof;
pub use fooglers::Fooglers;
pub use formant_filter::FormantFilter;
pub use fractal_osc::FractalOscillator;
pub use granular_osc::GranularOsc;
pub use kinetic_modulator::KineticModulator;
pub use la_synth::LaSynth;
pub use lfo::Lfo;
pub use math_oscillator::MathOscillator;
pub use mod_matrix::ModMatrix;
pub use mseg::Mseg;
pub use noise::NoiseGenerator;
pub use oscillator::Oscillator;
pub use output::StereoOutput;
pub use padsynth::PadSynth;
pub use pitch_tracker::PitchTracker;
pub use random_gates::RandomGates;
pub use ring_mod::RingMod;
pub use sampler::Sampler;
pub use script_module::ScriptModule;
pub use sid_oscillator::SidOscillator;
pub use signal_monitor::SignalMonitor;
pub use sub_osc::SubOscillator;
pub use turing_machine::TuringMachine;
pub use vector_mixer::VectorMixer;
pub use vocal_tract::VocalTract;
pub use voice_synth::VoiceSynth;
pub use wavetable_osc::WavetableOsc;

// Physical modeling exports
pub use body_resonance::BodyResonance;
pub use keyboard_panner::KeyboardPanner;
pub use mechanical_noise::MechanicalNoise;

// Effects exports
pub use effects::{
    BbdDelay, Chorus, Compressor, Convolver, Delay, Distortion, DistortionType, Eq, Flanger,
    FrequencyShifter, Limiter, MidSide, PhaseVocoder, Phaser, Reverb, TiltEq, Univibe, Waveshaper,
};

// Re-export param types from synth_core for convenience
pub use synth_core::{NoiseType, SubOscOctave, SubOscWaveform};

#[cfg(test)]
mod width_bucket_guards {
    use crate::{Envelope, Mixer, ModMatrix, Mseg, Sampler};
    use synth_core::{Describable, ModuleWidth};

    /// Modules with a bespoke wide body (an interactive editor or a fixed-width
    /// grid) must sit at `Large` or wider, or their content overflows the bucket.
    /// This guards the Phase 1 classification against a regression to a narrow
    /// default.
    #[test]
    fn bespoke_wide_modules_are_at_least_large() {
        for width in [
            Envelope::new().descriptor().width,  // ADSR editor (min 150 px)
            ModMatrix::new().descriptor().width, // fixed 220 px slot grid
            Sampler::new().descriptor().width,
            Mixer::new().descriptor().width,
        ] {
            assert!(
                width >= ModuleWidth::Large,
                "expected >= Large, got {width:?}"
            );
        }
        // The MSEG editor is the widest body of all.
        assert_eq!(Mseg::new().descriptor().width, ModuleWidth::ExtraLarge);
    }
}
