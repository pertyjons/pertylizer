//! Centralized module creation by `ModuleType`.
//!
//! Maps `ModuleType` → `Box<dyn PolyModule>` + `ModuleDescriptor`.
//! Used by MCP bridge (add_module) and could replace duplicated creation
//! logic in `patch_bridge.rs` and `egui_backend.rs`.

use synth_core::{Describable, ModuleDescriptor, ModuleType, PolyModule};

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
        // Effects and visualizers are not voice modules
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

    // Effects
    match module_type {
        ModuleType::Delay => Some(synth_modules::effects::Delay::new().descriptor()),
        ModuleType::Reverb => Some(synth_modules::effects::Reverb::new().descriptor()),
        ModuleType::Distortion => Some(synth_modules::effects::Distortion::new().descriptor()),
        ModuleType::Chorus => Some(synth_modules::effects::Chorus::new().descriptor()),
        ModuleType::Phaser => Some(synth_modules::effects::Phaser::new().descriptor()),
        ModuleType::Flanger => Some(synth_modules::effects::Flanger::new().descriptor()),
        ModuleType::Compressor => Some(synth_modules::effects::Compressor::new().descriptor()),
        ModuleType::Eq => Some(synth_modules::effects::Eq::new().descriptor()),
        ModuleType::Waveshaper => Some(synth_modules::effects::Waveshaper::new().descriptor()),
        ModuleType::BbdDelay => Some(synth_modules::effects::BbdDelay::new().descriptor()),
        ModuleType::MidSide => Some(synth_modules::effects::MidSide::new().descriptor()),
        ModuleType::Limiter => Some(synth_modules::effects::Limiter::new().descriptor()),
        ModuleType::Convolver => Some(synth_modules::effects::Convolver::new().descriptor()),
        ModuleType::PhaseVocoder => Some(synth_modules::effects::PhaseVocoder::new().descriptor()),
        // Visualizers
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
    // Visualizers
    ModuleType::Oscilloscope,
    ModuleType::LevelMeter,
    ModuleType::SpectrumAnalyzer,
];
