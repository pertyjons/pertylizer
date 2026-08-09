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
        ModuleType::SpatialPanner => {
            let m = synth_modules::SpatialPanner::new();
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
        ModuleType::FractalOsc => {
            let m = synth_modules::FractalOscillator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Sampler => {
            let m = synth_modules::Sampler::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::AudioInput => {
            let m = synth_modules::AudioInput::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::LadderFilter => {
            let m = synth_modules::LadderFilter::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::DriftGenerator => {
            let m = synth_modules::DriftGenerator::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::ChaoticOsc => {
            let m = synth_modules::ChaoticOsc::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::FormantFilter => {
            let m = synth_modules::FormantFilter::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Fooglers => {
            let m = synth_modules::Fooglers::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::BeatDetector => {
            let m = synth_modules::BeatDetector::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::PadSynth => {
            let m = synth_modules::PadSynth::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::AmFormant => {
            let m = synth_modules::AmFormant::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::VoiceSynth => {
            let m = synth_modules::VoiceSynth::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::VocalTract => {
            let m = synth_modules::VocalTract::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Fof => {
            let m = synth_modules::Fof::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::Script => {
            let m = synth_modules::ScriptModule::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::AudioScript => {
            let m = synth_modules::AudioScript::new();
            let d = m.descriptor();
            Some((Box::new(m), d))
        }
        ModuleType::SidOscillator => {
            let m = synth_modules::SidOscillator::new();
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
        ModuleType::TransientShaper => {
            let e = synth_modules::effects::TransientShaper::new();
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
        ModuleType::EnsembleChorus => {
            let e = synth_modules::effects::EnsembleChorus::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::ShimmerReverb => {
            let e = synth_modules::effects::ShimmerReverb::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::GranularFx => {
            let e = synth_modules::effects::GranularFx::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::SpectralBlur => {
            let e = synth_modules::effects::SpectralBlur::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::ModalResonator => {
            let e = synth_modules::effects::ModalResonator::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::ReverseGateReverb => {
            let e = synth_modules::effects::ReverseGateReverb::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::TiltEq => {
            let e = synth_modules::effects::TiltEq::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Univibe => {
            let e = synth_modules::effects::Univibe::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::CrossoverSplitter => {
            let e = synth_modules::effects::CrossoverSplitter::new();
            let d = e.descriptor();
            Some((Box::new(e), d))
        }
        ModuleType::Vocoder => {
            let e = synth_modules::effects::Vocoder::new();
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

/// All supported module types, for listing / discovery.
///
/// Derived directly from the [`ModuleType`] enum via `EnumIter` (in declaration
/// order), so a newly-added variant is **automatically** discoverable through
/// MCP (`list_module_types`) and the GUI "Add module" menu — there is no
/// hand-maintained list that could silently fall out of sync with the enum.
pub static ALL_MODULE_TYPES: std::sync::LazyLock<Vec<ModuleType>> =
    std::sync::LazyLock::new(|| ModuleType::all().collect());

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::{ModuleParam, PortDirection, PortType};

    /// Every `ModuleType` must resolve a descriptor. Since `ALL_MODULE_TYPES`
    /// is now derived from `ModuleType::iter()` (every enum variant), this also
    /// guards that a newly-added variant is wired into the factory — otherwise
    /// it would be discoverable via MCP / the "Add module" menu yet fail to
    /// instantiate.
    #[test]
    fn every_module_type_has_a_descriptor() {
        let missing: Vec<ModuleType> = ALL_MODULE_TYPES
            .iter()
            .copied()
            .filter(|&mt| get_descriptor(mt).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "module types in ALL_MODULE_TYPES with no descriptor: {missing:?}"
        );
    }

    /// A module's display name is spelled out twice — once in
    /// [`ModuleType::name`] and once in the module's own `ModuleDescriptor` — and
    /// the two live in crates that can't see each other (`synth_core` knows the
    /// enum, `synth_modules` builds the descriptors). This crate sees both, so it
    /// is where the copies are held to being one name: MCP reads `name()` while
    /// the GUI reads the descriptor, and a divergence means the same module is
    /// called two different things depending on where you look. If this fails,
    /// change one of the two strings — don't relax the assert.
    #[test]
    fn module_type_names_match_their_descriptors() {
        let mismatched: Vec<String> = ALL_MODULE_TYPES
            .iter()
            .copied()
            .filter_map(|mt| {
                let descriptor = get_descriptor(mt)?;
                (descriptor.name != mt.name()).then(|| {
                    format!(
                        "{mt:?}: ModuleType::name() = {:?}, descriptor.name = {:?}",
                        mt.name(),
                        descriptor.name
                    )
                })
            })
            .collect();
        assert!(
            mismatched.is_empty(),
            "module display names disagree between the enum and the descriptor: {mismatched:#?}"
        );
    }

    #[test]
    fn descriptor_port_names_are_unique_and_labels_are_present() {
        let mut violations = Vec::new();
        for mt in ALL_MODULE_TYPES.iter().copied() {
            let Some(desc) = get_descriptor(mt) else {
                continue;
            };
            let mut names = std::collections::HashSet::new();
            for port in &desc.ports {
                if !names.insert(port.name) {
                    violations.push(format!("{mt:?}: duplicate port name '{}'", port.name));
                }
                if port.label.trim().is_empty() {
                    violations.push(format!("{mt:?}/{}: empty port label", port.name));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "port descriptor invariant violations:\n{}",
            violations.join("\n")
        );
    }

    const SEMANTIC_PORT_TYPES: &[(ModuleType, &str, PortDirection, PortType)] = &[
        (
            ModuleType::DriftGenerator,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Envelope,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::ChaoticOsc,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::ChaoticOsc,
            "out_y",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::BeatDetector,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::BeatDetector,
            "gate",
            PortDirection::Output,
            PortType::Gate,
        ),
        (
            ModuleType::TuringMachine,
            "clock",
            PortDirection::Input,
            PortType::Gate,
        ),
        (
            ModuleType::TuringMachine,
            "pitch",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::TuringMachine,
            "gate",
            PortDirection::Output,
            PortType::Gate,
        ),
        (
            ModuleType::Euclidean,
            "clock",
            PortDirection::Input,
            PortType::Gate,
        ),
        (
            ModuleType::Euclidean,
            "gate",
            PortDirection::Output,
            PortType::Gate,
        ),
        (
            ModuleType::Euclidean,
            "accent",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::RandomGates,
            "clock",
            PortDirection::Input,
            PortType::Gate,
        ),
        (
            ModuleType::RandomGates,
            "gate",
            PortDirection::Output,
            PortType::Gate,
        ),
        (
            ModuleType::RandomGates,
            "cv",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::EnvelopeFollower,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::PitchTracker,
            "pitch_cv",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::PitchTracker,
            "gate",
            PortDirection::Output,
            PortType::Gate,
        ),
        (
            ModuleType::Mseg,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::KineticModulator,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Lfo,
            "out",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Script,
            "out1",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Script,
            "out2",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Script,
            "out3",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::Script,
            "out4",
            PortDirection::Output,
            PortType::Control,
        ),
        (
            ModuleType::SidOscillator,
            "test",
            PortDirection::Input,
            PortType::Gate,
        ),
        // These are logic-shaped but intentionally audio-rate sync signals.
        (
            ModuleType::Oscillator,
            "phase",
            PortDirection::Output,
            PortType::Audio,
        ),
        (
            ModuleType::SidOscillator,
            "msb",
            PortDirection::Output,
            PortType::Audio,
        ),
    ];

    #[test]
    fn modulation_and_gate_ports_use_semantic_signal_types() {
        let mut violations = Vec::new();
        for &(module_type, name, direction, port_type) in SEMANTIC_PORT_TYPES {
            let Some(desc) = get_descriptor(module_type) else {
                violations.push(format!("{module_type:?}: missing descriptor"));
                continue;
            };
            let actual = desc.ports.iter().find(|port| port.name == name);
            match actual {
                Some(port) if port.direction == direction && port.port_type == port_type => {}
                Some(port) => violations.push(format!(
                    "{module_type:?}/{name}: expected {direction:?} {port_type:?}, got {:?} {:?}",
                    port.direction, port.port_type
                )),
                None => violations.push(format!("{module_type:?}/{name}: missing port")),
            }
        }
        assert!(
            violations.is_empty(),
            "semantic port type violations:\n{}",
            violations.join("\n")
        );
    }

    /// Phase 1 acceptance: every parameter of every module descriptor must satisfy
    /// the `ParamKind` invariants — the classifier matches `id.kind()`, and the
    /// `kind`/`choices` relationship holds:
    ///   - `Enum`                       ⇒ `choices.is_some()`
    ///   - `{Continuous, Integer, Bool}` ⇒ `choices.is_none()`
    ///   - `choices.is_some()`          ⇒ `kind ∈ {Enum, Reference}` (mod-matrix
    ///     pickers are `Reference`; the `SampleId` reference carries no choices)
    #[test]
    fn param_kind_invariants_hold_for_every_descriptor() {
        use synth_core::ParamKind;
        // Every `Enum`-kind param now carries a choice list (`SpectralBlur/fft_size`
        // and `KeyboardPanner/invert` were converted from `float()` to `choice()`),
        // so there are no enum-as-float exceptions left — the invariant holds for all.
        let mut violations: Vec<String> = Vec::new();
        for mt in ALL_MODULE_TYPES.iter().copied() {
            let Some(desc) = get_descriptor(mt) else {
                continue;
            };
            for p in &desc.parameters {
                let kind = p.kind;
                // Construction wiring must hold for EVERY parameter, no exceptions.
                if kind != p.id.kind() {
                    violations.push(format!(
                        "{mt:?}/{}: descriptor.kind={kind:?} != id.kind()={:?}",
                        p.type_id,
                        p.id.kind()
                    ));
                }
                let has_choices = p.choices.is_some();
                let ok = match kind {
                    ParamKind::Enum => has_choices,
                    ParamKind::Continuous | ParamKind::Integer | ParamKind::Bool => !has_choices,
                    ParamKind::Reference => true, // choices optional (picker vs id)
                };
                if !ok {
                    violations.push(format!(
                        "{mt:?}/{}: kind={kind:?} but choices.is_some()={has_choices}",
                        p.type_id
                    ));
                }
                if has_choices && !matches!(kind, ParamKind::Enum | ParamKind::Reference) {
                    violations.push(format!(
                        "{mt:?}/{}: has choices but kind={kind:?} (expected Enum or Reference)",
                        p.type_id
                    ));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "ParamKind invariant violations:\n{}",
            violations.join("\n")
        );
    }

    /// Phase 2a: a descriptor's display `unit` must equal its value type's natural
    /// unit (`id.unit()`), unless the param is a deliberate override on the
    /// allow-list below. `float()` now seeds `unit` from the type, so any remaining
    /// mismatch is an *explicit* `.unit(X)` that disagrees with the type — either a
    /// legitimate override (kept on the allow-list) or a real drift bug (to fix).
    #[test]
    fn descriptor_unit_matches_type_unless_allow_listed() {
        use synth_core::{ParamKind, ParameterUnit};
        // Genuine type-vs-type overrides (a real unit deliberately replacing the
        // type's unit): (module, type_id, override unit). None remain — the two
        // historical mismatches were fixed at the source (BeatDivision → Beats at
        // the type level; WavetableOsc/octave's stale `.unit(Semitones)` removed so
        // it derives `Octaves`).
        let allow: &[(ModuleType, &str, ParameterUnit)] = &[];
        let mut violations: Vec<String> = Vec::new();
        for mt in ALL_MODULE_TYPES.iter().copied() {
            let Some(desc) = get_descriptor(mt) else {
                continue;
            };
            for p in &desc.parameters {
                let want = p.id.unit();
                if p.unit == want {
                    continue;
                }
                // A unitless Continuous value (NormalizedValue, PulseWidth, …)
                // presented as a percentage is a legitimate display choice, never
                // drift — allow `Percent` over a type-unit of `None`.
                if want == ParameterUnit::None
                    && p.unit == ParameterUnit::Percent
                    && p.kind == ParamKind::Continuous
                {
                    continue;
                }
                if allow
                    .iter()
                    .any(|&(m, id, u)| m == mt && id == p.type_id.as_str() && u == p.unit)
                {
                    continue;
                }
                violations.push(format!(
                    "{mt:?}/{}: descriptor.unit={:?} != id.unit()={want:?}",
                    p.type_id, p.unit
                ));
            }
        }
        assert!(
            violations.is_empty(),
            "unit-drift violations ({} — triage each as override→allow-list or bug→fix):\n{}",
            violations.len(),
            violations.join("\n")
        );
    }

    /// `is_automatable` = `modulatable && (Continuous | Integer)`: relative to
    /// the legacy `modulatable && choices.is_none()` rule, exactly the `Bool`
    /// toggles below lose sequencer-automation eligibility — two-state params
    /// that a normalized lane cannot meaningfully ramp. `Integer` params
    /// (counts/notes/chip registers) are automatable as stepped/sample-hold
    /// lanes since the SID register work (per-frame PWM lanes are the core SID
    /// idiom). The bools keep their **engine-side mod-matrix** eligibility
    /// (the `ParamModOffsets` store populates from `modulatable` directly).
    /// This guard pins the exact set so any change — especially the real
    /// regression of dropping a *Continuous* or *Integer* param — is surfaced
    /// for review.
    #[test]
    fn is_automatable_excludes_only_bool_toggles() {
        let mut dropped: Vec<String> = Vec::new();
        for mt in ALL_MODULE_TYPES.iter().copied() {
            let Some(desc) = get_descriptor(mt) else {
                continue;
            };
            for p in &desc.parameters {
                let old = p.modulatable && p.choices.is_none();
                if old && !p.is_automatable() {
                    dropped.push(format!("{mt:?}/{}", p.type_id));
                }
            }
        }
        dropped.sort();
        let mut expected = [
            "Compressor/sidechain",
            "GranularFx/freeze",
            "PhaseVocoder/freeze",
            "SpectralBlur/freeze",
        ];
        expected.sort_unstable();
        assert_eq!(
            dropped, expected,
            "set of params losing automation eligibility changed — review (a \
             Continuous or Integer param here would be a real regression)"
        );
    }

    /// Phase 4: the `ParamValue` that `from_param` produces must have the variant
    /// dictated by the descriptor's `kind` — the `ParamValue` ↔ `ParamKind` bridge
    /// (plan §1.5). Swept across every real descriptor's default value.
    #[test]
    fn param_value_variant_matches_kind() {
        use crate::patch::ParamValue;
        use synth_core::ParamKind;
        let mut bad: Vec<String> = Vec::new();
        for mt in ALL_MODULE_TYPES.iter().copied() {
            let Some(desc) = get_descriptor(mt) else {
                continue;
            };
            for p in &desc.parameters {
                let pv = ParamValue::from_param(&p.id, p);
                let ok = match p.kind {
                    ParamKind::Continuous => matches!(pv, ParamValue::Float(_)),
                    ParamKind::Integer => matches!(pv, ParamValue::Int(_)),
                    ParamKind::Bool => matches!(pv, ParamValue::Bool(_)),
                    // Enums map their index to a choice id. (No enum-as-float params
                    // remain; the bare-`Enum` arm is a defensive fallback.)
                    ParamKind::Enum if p.choices.is_some() => matches!(pv, ParamValue::Choice(_)),
                    ParamKind::Enum => matches!(pv, ParamValue::Float(_)),
                    // Sample id → struct SampleId; mod-matrix address → Choice string.
                    ParamKind::Reference => {
                        matches!(pv, ParamValue::Choice(_) | ParamValue::SampleId { .. })
                    }
                };
                if !ok {
                    bad.push(format!("{mt:?}/{}: kind={:?} -> {pv:?}", p.type_id, p.kind));
                }
            }
        }
        assert!(
            bad.is_empty(),
            "ParamValue variant disagrees with kind:\n{}",
            bad.join("\n")
        );
    }

    /// Resolve a module's descriptor *and* its `get_params()` snapshot, covering
    /// voice modules, effects, and visualizers (mirrors [`get_descriptor`]).
    fn descriptor_and_params(mt: ModuleType) -> Option<(ModuleDescriptor, Vec<synth_core::Param>)> {
        if let Some((m, d)) = create_voice_module(mt) {
            return Some((d, m.get_params()));
        }
        if let Some((e, d)) = create_effect(mt) {
            return Some((d, e.get_params()));
        }
        match mt {
            ModuleType::Oscilloscope => {
                let m = synth_engine::visualizers::Oscilloscope::new();
                Some((m.descriptor(), m.get_params()))
            }
            ModuleType::LevelMeter => {
                let m = synth_engine::visualizers::LevelMeter::new();
                Some((m.descriptor(), m.get_params()))
            }
            ModuleType::SpectrumAnalyzer => {
                let m = synth_engine::visualizers::SpectrumAnalyzer::new();
                Some((m.descriptor(), m.get_params()))
            }
            _ => None,
        }
    }

    /// Every parameter a module emits from `get_params()` must have a matching
    /// descriptor entry. Project save iterates `descriptor.parameters` and matches
    /// each against `get_params()` (`create_patch_from_editor`), so a param in
    /// `get_params()` with no descriptor entry is **silently dropped on save** —
    /// lost on reload, no GUI knob, not settable via MCP. This guards the whole
    /// module set against that class of state-sync mismatch.
    #[test]
    fn get_params_have_descriptor_entries() {
        let mut offenders = Vec::new();
        for &mt in ALL_MODULE_TYPES.iter() {
            let Some((desc, params)) = descriptor_and_params(mt) else {
                continue;
            };
            for p in &params {
                if !desc.parameters.iter().any(|d| p.same_kind(&d.id)) {
                    offenders.push(format!("{mt:?}: {p:?}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "get_params() entries with no descriptor (silently dropped on save): {offenders:#?}"
        );
    }

    /// `SynthSession::set_parameter` picks `SetEffectParameter` over
    /// `SetModuleParameter` from the descriptor's category, because the engine
    /// keeps effects and voice modules in separate maps and the wrong command
    /// finds nothing — the send succeeds and the value is dropped in silence.
    /// That makes "a factory-built effect is categorized `Effect`, and nothing
    /// else is" a routing invariant rather than a labelling detail, so it is
    /// asserted over every module type instead of trusted per descriptor.
    #[test]
    fn category_and_kind_agree_for_every_module_type() {
        use synth_core::ModuleCategory;

        let mut offenders = Vec::new();
        for module_type in ModuleType::all() {
            // A missing factory entry is an offence in its own right, not a
            // reason to skip the check: letting `None` pass silently would let a
            // type opt out of the very invariant this test exists to hold.
            if module_type.is_effect() {
                match create_effect(module_type) {
                    Some((_, descriptor)) if descriptor.category == ModuleCategory::Effect => {}
                    Some((_, descriptor)) => offenders.push(format!(
                        "{module_type:?} is an effect but is categorized {:?}",
                        descriptor.category
                    )),
                    None => offenders.push(format!(
                        "{module_type:?} is an effect but `create_effect` has no entry for it"
                    )),
                }
            }
            if module_type.is_voice_module() {
                match create_voice_module(module_type) {
                    Some((_, descriptor)) if descriptor.category == ModuleCategory::Effect => {
                        offenders.push(format!(
                            "{module_type:?} is a voice module but is categorized Effect"
                        ));
                    }
                    Some(_) => {}
                    None => offenders.push(format!(
                        "{module_type:?} is a voice module but `create_voice_module` has no \
                         entry for it"
                    )),
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "parameters would be routed to the wrong engine map: {offenders:#?}"
        );
    }
}
