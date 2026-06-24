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
        // Known pre-existing exceptions: enum-typed params whose descriptors were
        // hand-built with `float()` instead of `choice()`, so `kind == Enum` but
        // they carry no choice list. They are genuine descriptor-modeling gaps
        // (notably `SpectralBlur/fft_size`, which should mirror PhaseVocoder's
        // `choice()` over `FftSizeOption::ALL`). Converting them to `choice()`
        // changes widget + serialization behavior, so it is deferred out of the
        // Phase 1 (engine-only, no-behavior-change) foundation — see
        // `plans/param-type-system-plan.md` Phase 1 follow-up. The strict
        // `kind == id.kind()` invariant below still holds for them.
        let enum_as_float_todo: &[(ModuleType, &str)] = &[
            (ModuleType::SpectralBlur, "fft_size"),
            (ModuleType::KeyboardPanner, "invert"),
        ];
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
                if enum_as_float_todo.contains(&(mt, p.type_id.as_str())) {
                    continue; // documented choices-relationship exception
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

    /// Phase 2b: tightening `is_automatable` from `modulatable && choices.is_none()`
    /// to `modulatable && kind == Continuous` removes sequencer-automation
    /// eligibility from exactly the params below — non-continuous targets
    /// (`Integer` counts/notes, `Bool` toggles, the enum-as-float `fft_size`) that
    /// the old `choices.is_none()` rule let through but that cannot be meaningfully
    /// ramped by a normalized lane. They keep their **mod-matrix** eligibility
    /// (which uses `modulatable` directly, unchanged). This guard pins the exact
    /// set so any change — especially the real regression of dropping a *Continuous*
    /// param — is surfaced for review.
    #[test]
    fn is_automatable_tightening_drops_only_non_continuous() {
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
            "Chorus/voices",
            "Compressor/sidechain",
            "EnsembleChorus/voices",
            "GranularFx/freeze",
            "KeyboardPanner/center",
            "ModalResonator/base_note",
            "ModalResonator/modes",
            "PhaseVocoder/freeze",
            "SpectralBlur/fft_size",
            "SpectralBlur/freeze",
        ];
        expected.sort_unstable();
        assert_eq!(
            dropped, expected,
            "set of params losing automation eligibility changed — review (a \
             Continuous param here would be a real regression)"
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
}
