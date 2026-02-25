//! Centralized module creation by `ModuleType`.
//!
//! Maps `ModuleType` → `Box<dyn PolyModule>` + `ModuleDescriptor`.
//! Used by MCP bridge (add_module) and could replace duplicated creation
//! logic in `patch_bridge.rs` and `egui_backend.rs`.

use synth_core::{AudioEffect, Describable, ModuleDescriptor, ModuleType, PolyModule};

/// Create a voice module instance from its type.
///
/// Returns `None` for effects, visualizers, and other non-voice module types.
#[must_use]
pub fn create_voice_module(
    module_type: ModuleType,
) -> Option<(Box<dyn PolyModule>, ModuleDescriptor)> {
    match module_type {
        ModuleType::Oscillator => {
            let m = synth_modules::Oscillator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::MathOscillator => {
            let m = synth_modules::MathOscillator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::SubOscillator => {
            let m = synth_modules::SubOscillator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Noise => {
            let m = synth_modules::NoiseGenerator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Filter => {
            let m = synth_modules::Filter::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Envelope => {
            let m = synth_modules::Envelope::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Lfo => {
            let m = synth_modules::Lfo::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Amplifier => {
            let m = synth_modules::Amplifier::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Mixer => {
            let m = synth_modules::Mixer::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::StereoOutput => {
            let m = synth_modules::StereoOutput::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::ModMatrix => {
            let m = synth_modules::ModMatrix::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::RingMod => {
            let m = synth_modules::RingMod::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::EnvelopeFollower => {
            let m = synth_modules::EnvelopeFollower::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::WavetableOsc => {
            let m = synth_modules::WavetableOsc::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Mseg => {
            let m = synth_modules::Mseg::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::AdditiveOsc => {
            let m = synth_modules::AdditiveOsc::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Euclidean => {
            let m = synth_modules::Euclidean::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::TuringMachine => {
            let m = synth_modules::TuringMachine::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::RandomGates => {
            let m = synth_modules::RandomGates::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::KeyboardPanner => {
            let m = synth_modules::KeyboardPanner::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::BodyResonance => {
            let m = synth_modules::BodyResonance::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::MechanicalNoise => {
            let m = synth_modules::MechanicalNoise::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::GranularOsc => {
            let m = synth_modules::GranularOsc::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::KineticModulator => {
            let m = synth_modules::KineticModulator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::SignalMonitor => {
            let m = synth_modules::SignalMonitor::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::VectorMixer => {
            let m = synth_modules::VectorMixer::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::LaSynth => {
            let m = synth_modules::LaSynth::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::PitchTracker => {
            let m = synth_modules::PitchTracker::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        // Effects and visualizers are not voice modules
        _ => None,
    }
}

/// Create an effect instance from its type.
///
/// Returns `None` for voice modules, visualizers, and other non-effect types.
#[must_use]
pub fn create_effect(module_type: ModuleType) -> Option<(Box<dyn AudioEffect>, ModuleDescriptor)> {
    match module_type {
        ModuleType::Delay => {
            let e = synth_modules::effects::Delay::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Reverb => {
            let e = synth_modules::effects::Reverb::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Distortion => {
            let e = synth_modules::effects::Distortion::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Chorus => {
            let e = synth_modules::effects::Chorus::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Phaser => {
            let e = synth_modules::effects::Phaser::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Flanger => {
            let e = synth_modules::effects::Flanger::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Compressor => {
            let e = synth_modules::effects::Compressor::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Eq => {
            let e = synth_modules::effects::Eq::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Waveshaper => {
            let e = synth_modules::effects::Waveshaper::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::BbdDelay => {
            let e = synth_modules::effects::BbdDelay::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::MidSide => {
            let e = synth_modules::effects::MidSide::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Limiter => {
            let e = synth_modules::effects::Limiter::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Convolver => {
            let e = synth_modules::effects::Convolver::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::PhaseVocoder => {
            let e = synth_modules::effects::PhaseVocoder::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::FrequencyShifter => {
            let e = synth_modules::effects::FrequencyShifter::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        _ => None,
    }
}

/// Get the descriptor for any supported module type (voice, effect, or visualizer).
///
/// Creates a temporary instance just to call `.descriptor()`.
#[must_use]
pub fn get_descriptor(module_type: ModuleType) -> Option<ModuleDescriptor> {
    // Try voice modules first
    if let Some((_, desc)) = create_voice_module(module_type) {
        return Some(desc);
    }

    // Try effects
    if let Some((_, desc)) = create_effect(module_type) {
        return Some(desc);
    }

    // Visualizers
    match module_type {
        ModuleType::Oscilloscope => {
            Some(synth_engine::visualizers::Oscilloscope::new().descriptor())
        }
        ModuleType::LevelMeter => Some(synth_engine::visualizers::LevelMeter::new().descriptor()),
        ModuleType::SpectrumAnalyzer => {
            Some(synth_engine::visualizers::SpectrumAnalyzer::new().descriptor())
        }
        _ => None,
    }
}

/// All supported module types for listing purposes.
pub const ALL_MODULE_TYPES: &[ModuleType] = &[
    // Voice modules
    ModuleType::Oscillator,
    ModuleType::MathOscillator,
    ModuleType::SubOscillator,
    ModuleType::Noise,
    ModuleType::Filter,
    ModuleType::Envelope,
    ModuleType::Lfo,
    ModuleType::Amplifier,
    ModuleType::Mixer,
    ModuleType::StereoOutput,
    ModuleType::ModMatrix,
    ModuleType::RingMod,
    ModuleType::EnvelopeFollower,
    ModuleType::WavetableOsc,
    ModuleType::Mseg,
    ModuleType::AdditiveOsc,
    ModuleType::Euclidean,
    ModuleType::TuringMachine,
    ModuleType::RandomGates,
    ModuleType::KeyboardPanner,
    ModuleType::BodyResonance,
    ModuleType::MechanicalNoise,
    ModuleType::GranularOsc,
    ModuleType::KineticModulator,
    ModuleType::SignalMonitor,
    ModuleType::VectorMixer,
    ModuleType::LaSynth,
    ModuleType::PitchTracker,
    // Effects
    ModuleType::Delay,
    ModuleType::Reverb,
    ModuleType::Distortion,
    ModuleType::Chorus,
    ModuleType::Phaser,
    ModuleType::Flanger,
    ModuleType::Compressor,
    ModuleType::Eq,
    ModuleType::Waveshaper,
    ModuleType::BbdDelay,
    ModuleType::MidSide,
    ModuleType::Limiter,
    ModuleType::Convolver,
    ModuleType::PhaseVocoder,
    ModuleType::FrequencyShifter,
    // Visualizers
    ModuleType::Oscilloscope,
    ModuleType::LevelMeter,
    ModuleType::SpectrumAnalyzer,
];
