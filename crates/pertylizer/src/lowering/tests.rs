//! Tests for the lowerer's typed boundary.
//!
//! These live inside the module tree rather than in `tests/` for a boundary reason:
//! `synth_engine_v2`'s `crate_boundary` permits exactly the measurement harnesses and this
//! module tree to name the experimental crate, and a file under `tests/` naming it would be
//! an offender. Keeping them here means the permitted set stays one prefix.

use std::collections::BTreeMap;

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine_v2::ir::NodeId;

use super::diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};
use super::identity::{IdentityError, ResolvedIdentities};
use crate::patch::{ModuleState, Position};

/// A module with only the fields identity resolution reads.
fn module(id: &str, module_type: ModuleType) -> ModuleState {
    ModuleState {
        id: id.to_owned(),
        module_type,
        position: Position::new(0.0, 0.0),
        description: String::new(),
        parameters: BTreeMap::new(),
        scripts: BTreeMap::new(),
    }
}

/// The corpus fixture's five modules, in the order the project stores them.
fn corpus_modules() -> Vec<ModuleState> {
    vec![
        module("env-1", ModuleType::Envelope),
        module("amp-1", ModuleType::Amplifier),
        module("out-1", ModuleType::StereoOutput),
        module("osc-1", ModuleType::Oscillator),
        module("flt-1", ModuleType::Filter),
    ]
}

#[test]
fn every_module_resolves_in_both_directions() {
    let resolved = ResolvedIdentities::resolve(&corpus_modules()).expect("the fixture resolves");
    assert_eq!(resolved.len(), 5);
    assert!(!resolved.is_empty());

    for (id, node) in resolved.pairs() {
        assert_eq!(
            resolved.node_for(id),
            Some(node),
            "{id} must resolve to the node it was assigned"
        );
        assert_eq!(
            resolved.module_for(node),
            Some(id),
            "{node} must name the module it came from, which is what a diagnostic needs"
        );
    }
}

/// The property the header claims: assignment is by identity, not by position.
///
/// Reversing the array is the cheapest mutation of authoring order that changes every index.
/// If assignment read the position, every node would move; because it reads the sorted
/// `ModuleId`, none does.
#[test]
fn reordering_the_modules_array_changes_no_assignment() {
    let forward = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");

    let mut reversed = corpus_modules();
    reversed.reverse();
    let backward = ResolvedIdentities::resolve(&reversed).expect("resolves");

    let forward_pairs: Vec<_> = forward.pairs().collect();
    let backward_pairs: Vec<_> = backward.pairs().collect();
    assert_eq!(
        forward_pairs, backward_pairs,
        "the same patch in a different array order must lower to the same addresses"
    );
}

/// Two patches whose modules differ only in identity must not share an address by accident.
#[test]
fn distinct_modules_receive_distinct_addresses() {
    let resolved = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");
    let mut seen = Vec::new();
    for (_, node) in resolved.pairs() {
        assert!(!seen.contains(&node), "{node} was assigned twice");
        seen.push(node);
    }
    assert_eq!(seen.len(), 5);
}

/// The property an independent review found the rank assignment did not have.
///
/// Adding a module whose identity sorts **before** every existing one is the mutation that
/// shifts every rank. Because the address is computed from the identity alone, none moves.
#[test]
fn adding_a_module_that_sorts_first_moves_no_other_address() {
    let before = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");

    let mut grown = corpus_modules();
    // `Oscillator` is the first `ModuleType` variant, so `osc-0` sorts before every module
    // the fixture declares.
    grown.push(module("osc-0", ModuleType::Oscillator));
    let after = ResolvedIdentities::resolve(&grown).expect("resolves");

    for (id, node) in before.pairs() {
        assert_eq!(
            after.node_for(id),
            Some(node),
            "{id} moved when an unrelated module was added"
        );
    }
    assert_eq!(after.len(), before.len() + 1);
}

/// The same property under removal.
#[test]
fn removing_a_module_moves_no_other_address() {
    let full = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");

    let mut fewer = corpus_modules();
    fewer.retain(|m| m.id != "amp-1");
    let reduced = ResolvedIdentities::resolve(&fewer).expect("resolves");

    for (id, node) in reduced.pairs() {
        assert_eq!(
            full.node_for(id),
            Some(node),
            "{id} moved when an unrelated module was removed"
        );
    }
}

#[test]
fn a_module_whose_id_and_type_disagree_is_refused() {
    let modules = vec![module("osc-1", ModuleType::Filter)];
    let error = ResolvedIdentities::resolve(&modules).expect_err("must refuse");
    assert_eq!(
        error,
        IdentityError::TypeMismatch {
            spelling: "osc-1".to_owned(),
            declared: ModuleType::Filter,
            named: ModuleType::Oscillator,
        },
        "a module stating its type twice and disagreeing must not reach lowering"
    );
}

#[test]
fn an_unparsable_module_id_is_refused_and_names_its_spelling() {
    let modules = vec![module("not a module id", ModuleType::Oscillator)];
    let error = ResolvedIdentities::resolve(&modules).expect_err("must refuse");
    match error {
        IdentityError::UnparsableModule { spelling, .. } => {
            assert_eq!(
                spelling, "not a module id",
                "the diagnostic has to name the project object as the project spells it"
            );
        }
        other => panic!("expected an unparsable-module error, got {other:?}"),
    }
}

#[test]
fn a_duplicate_module_id_is_refused_rather_than_resolved_to_the_last_one() {
    let modules = vec![
        module("osc-1", ModuleType::Oscillator),
        module("osc-1", ModuleType::Oscillator),
    ];
    let error = ResolvedIdentities::resolve(&modules).expect_err("must refuse");
    assert_eq!(
        error,
        IdentityError::DuplicateModule {
            id: ModuleId::new(ModuleType::Oscillator, 1)
        }
    );
}

#[test]
fn a_connection_naming_an_absent_module_resolves_to_nothing() {
    let resolved = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");
    assert_eq!(
        resolved.node_for(ModuleId::new(ModuleType::Lfo, 9)),
        None,
        "an endpoint the patch does not declare must not resolve to some other node"
    );
}

#[test]
fn an_empty_patch_resolves_to_nothing_without_failing() {
    let resolved = ResolvedIdentities::resolve(&[]).expect("an empty patch is not an error");
    assert!(resolved.is_empty());
    assert_eq!(resolved.module_for(NodeId::FIRST), None);
}

/// The fails-closed mechanism `P04-R001` requires.
#[test]
fn any_diagnostic_denies_a_parity_comparison() {
    assert_eq!(Fidelity::of(&[]), Fidelity::Faithful);
    assert!(Fidelity::Faithful.admits_parity_comparison());

    let unrepresented = LoweringDiagnostic::unrepresented(
        ProjectSubject::Note {
            pattern: synth_sequencer::PatternId::new(0),
            note: synth_sequencer::NoteId::new(0),
        },
        LoweringReason::OwnedByLaterPhase {
            capability: "anything at all",
            owner: "a later phase",
        },
    );
    assert_eq!(unrepresented.severity(), Severity::Unrepresented);
    assert_eq!(
        Fidelity::of(std::slice::from_ref(&unrepresented)),
        Fidelity::UnsupportedScope
    );
    assert!(
        !Fidelity::UnsupportedScope.admits_parity_comparison(),
        "a render that cannot represent a note's pitch must not be comparable for parity"
    );
}

#[test]
fn a_diagnostic_keeps_the_subject_and_reason_it_was_built_with() {
    let module_id = ModuleId::new(ModuleType::Lfo, 1);
    let diagnostic = LoweringDiagnostic::refused(
        ProjectSubject::Module {
            instrument: synth_engine::instrument::InstrumentId::new(0),
            module: module_id,
        },
        LoweringReason::UnsupportedModuleType {
            module_type: ModuleType::Lfo,
        },
    );
    assert_eq!(diagnostic.severity(), Severity::Refused);
    assert_eq!(
        diagnostic.subject(),
        &ProjectSubject::Module {
            instrument: synth_engine::instrument::InstrumentId::new(0),
            module: module_id,
        }
    );
    assert_eq!(
        diagnostic.reason(),
        &LoweringReason::UnsupportedModuleType {
            module_type: ModuleType::Lfo
        }
    );
}

// ---------------------------------------------------------------------------
// Voice-patch lowering
// ---------------------------------------------------------------------------

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{IrNodeKind, SignalDomain};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{ChannelLayout, Resonance, SampleRate};
use synth_engine_v2::time::FrameCount;

use super::graph::lower_voice_patch;
use crate::patch::{ConnectionState, ParamValue};

fn instrument() -> synth_engine::instrument::InstrumentId {
    synth_engine::instrument::InstrumentId::new(0)
}

fn floats(module: &mut ModuleState, pairs: &[(&str, f32)]) {
    for (key, value) in pairs {
        module
            .parameters
            .insert((*key).to_owned(), ParamValue::Float(*value));
    }
}

fn choice(module: &mut ModuleState, key: &str, value: &str) {
    module
        .parameters
        .insert(key.to_owned(), ParamValue::Choice(value.to_owned()));
}

/// `CORPUS-0001`'s patch exactly as the pinned project stores it, waveform included.
fn corpus_patch(waveform: &str) -> (Vec<ModuleState>, Vec<ConnectionState>) {
    let mut env = module("env-1", ModuleType::Envelope);
    floats(
        &mut env,
        &[
            ("attack", 0.01),
            ("decay", 0.2),
            ("release", 0.25),
            ("sustain", 0.6),
        ],
    );
    let mut amp = module("amp-1", ModuleType::Amplifier);
    floats(&mut amp, &[("level", 1.0)]);
    let mut out = module("out-1", ModuleType::StereoOutput);
    floats(&mut out, &[("master", 1.0)]);
    let mut osc = module("osc-1", ModuleType::Oscillator);
    floats(&mut osc, &[("level", 1.0), ("uni_phase", 0.0)]);
    choice(&mut osc, "waveform", waveform);
    let mut flt = module("flt-1", ModuleType::Filter);
    floats(
        &mut flt,
        &[("cutoff", 1200.0), ("env_amt", 0.0), ("resonance", 0.3)],
    );
    choice(&mut flt, "type", "lowpass");

    let connection = |from: (&str, &str), to: (&str, &str)| ConnectionState {
        from: (from.0.to_owned(), from.1.to_owned()),
        to: (to.0.to_owned(), to.1.to_owned()),
    };
    (
        vec![env, amp, out, osc, flt],
        vec![
            connection(("env-1", "out"), ("amp-1", "cv")),
            connection(("amp-1", "out"), ("out-1", "in")),
            connection(("osc-1", "out"), ("flt-1", "in")),
            connection(("flt-1", "out"), ("amp-1", "in")),
        ],
    )
}

/// `P04-R003` is discharged: the waveform the pinned corpus authors now lowers.
#[test]
fn the_corpus_fixtures_sawtooth_lowers_to_the_sawtooth_node() {
    let (modules, connections) = corpus_patch("sawtooth");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    let ir = lowered.ir.expect("V2 has a sawtooth now");
    assert!(
        ir.nodes()
            .iter()
            .any(|n| matches!(n.kind(), IrNodeKind::Saw { .. })),
        "the authored waveform must reach the node that renders it"
    );
    assert!(
        !ir.nodes()
            .iter()
            .any(|n| matches!(n.kind(), IrNodeKind::Sine { .. })),
        "a sawtooth must not quietly become a sine"
    );
}

/// A waveform V2 still has no node for is refused by name.
#[test]
fn a_waveform_with_no_v2_node_is_refused_and_names_the_parameter() {
    let (modules, connections) = corpus_patch("square");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(lowered.ir.is_none(), "V2 has no square wave");
    let named = lowered
        .diagnostics
        .iter()
        .find(|d| {
            matches!(
                d.reason(),
                LoweringReason::UnsupportedParameterValue { value } if value == "square"
            )
        })
        .expect("the square must be reported");
    assert_eq!(named.severity(), Severity::Refused);
    assert_eq!(
        named.subject(),
        &ProjectSubject::Parameter {
            instrument: instrument(),
            module: ModuleId::new(ModuleType::Oscillator, 1),
            parameter: "waveform".to_owned(),
        },
        "the exit gate requires the diagnostic to name the project object"
    );
}

/// The same patch with the one waveform V2 has lowers whole, and the result compiles.
///
/// Compiling is the check that matters: a graph that builds but that V2's admission refuses
/// would mean the port, domain and kind mapping agreed with nothing but itself.
#[test]
fn the_corpus_patch_with_a_sine_lowers_and_compiles() {
    let (modules, connections) = corpus_patch("sine");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    let ir = lowered.ir.expect("the supported subset must lower");
    assert_eq!(ir.nodes().len(), 5);
    assert_eq!(ir.edges().len(), 4);

    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("a harness profile");
    let outcome = compile(&ir, &RenderConfig::new(profile));
    assert!(
        outcome.plan().is_ok(),
        "the lowered graph must be admissible: {:?}",
        outcome.plan().err()
    );
}

/// The envelope's gate is a control edge and the audio path is not.
#[test]
fn the_envelope_reaches_the_amplifier_as_a_control_edge() {
    let (modules, connections) = corpus_patch("sine");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered.ir.expect("lowers");

    let control: Vec<_> = ir
        .edges()
        .iter()
        .filter(|e| e.domain() == SignalDomain::Control)
        .collect();
    assert_eq!(
        control.len(),
        1,
        "exactly one edge — the envelope's — carries control"
    );
}

/// The resonance law, checked against the correspondence `EVD-0013` established.
///
/// V1 forms `k = 2 - 2·res` and V2 forms `damping = 1/Q`. `EVD-0013` records that
/// `res = 0.2928932309150696` reproduces `Resonance::BUTTERWORTH` exactly in `f32`, so
/// lowering that `res` must produce that `Q` and nothing near it.
#[test]
fn the_resonance_law_reproduces_the_value_evd_0013_pinned() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("resonance", 0.292_893_23)]);
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered.ir.expect("lowers");

    let filter = ir
        .nodes()
        .iter()
        .find_map(|n| match n.kind() {
            IrNodeKind::Filter { resonance, .. } => Some(resonance),
            _ => None,
        })
        .expect("the patch has a filter");
    assert!(
        (filter.as_f32() - Resonance::BUTTERWORTH.as_f32()).abs() < 1e-6,
        "lowered Q was {}, and EVD-0013's correspondence requires {}",
        filter.as_f32(),
        Resonance::BUTTERWORTH.as_f32()
    );
}

/// A filter envelope amount is dormant without a cutoff cable, and is not reported.
///
/// V1 multiplies `env_amt` by the `cutoff_cv` input, which reads zero when nothing is cabled
/// there. Reporting the parameter on an unpatched filter would be a diagnostic about
/// behaviour neither engine has.
#[test]
fn a_dormant_filter_envelope_amount_is_not_reported() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("env_amt", 0.5)]);
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_some(),
        "an unpatched filter's env_amt does not stop lowering"
    );
    assert_eq!(
        lowered
            .diagnostics
            .iter()
            .filter(|d| matches!(
                d.subject(),
                ProjectSubject::Parameter { parameter, .. } if parameter == "env_amt"
            ))
            .count(),
        0,
        "env_amt does nothing without a cutoff_cv cable, so there is nothing to report"
    );
}

/// The cutoff-modulation cable itself is what V2 cannot represent, and it is refused.
#[test]
fn a_cable_into_the_filters_cutoff_modulation_is_refused() {
    let (modules, mut connections) = corpus_patch("sine");
    connections.push(ConnectionState {
        from: ("env-1".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "cutoff_cv".to_owned()),
    });
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(
        lowered.ir.is_none(),
        "V2's filter declares no cutoff modulation input"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::UnknownPort { port } if port == "cutoff_cv"
        )),
        "the refusal must name the port the user cabled, got {:?}",
        lowered.diagnostics
    );
}

/// A lowered oscillator no longer reports its pitch as unrepresented.
///
/// It was, while the payload could not carry a key: the node ran at a documented placeholder
/// frequency and said so. Now the note supplies the pitch, so the only thing left to report
/// about this patch is what the pan stage and the master volume own — and **not** the pitch.
#[test]
fn a_lowered_oscillator_no_longer_reports_its_pitch_as_unrepresented() {
    let (modules, connections) = corpus_patch("sine");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_some());
    assert!(
        !lowered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. }
                if capability.contains("pitch") || capability.contains("velocity")
        )),
        "the note carries the pitch now, so nothing here may report it as unrepresented: \
         {:?}",
        lowered.diagnostics
    );
}

#[test]
fn a_module_type_with_no_v2_counterpart_is_refused_and_named() {
    let (mut modules, connections) = corpus_patch("sine");
    modules.push(module("nse-1", ModuleType::Noise));
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(lowered.ir.is_none());
    assert!(
        lowered.diagnostics.iter().any(|d| {
            d.subject()
                == &ProjectSubject::Module {
                    instrument: instrument(),
                    module: ModuleId::new(ModuleType::Noise, 1),
                }
                && *d.reason()
                    == LoweringReason::UnsupportedModuleType {
                        module_type: ModuleType::Noise,
                    }
        }),
        "an unsupported module must be named as a project object with its reason"
    );
}

#[test]
fn a_connection_to_a_port_the_kind_does_not_declare_is_refused() {
    let (modules, mut connections) = corpus_patch("sine");
    connections.push(ConnectionState {
        from: ("osc-1".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "cv".to_owned()),
    });
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(lowered.ir.is_none());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::UnknownPort { port } if port == "cv"
        )),
        "a filter declares no control input, and routing into one must not be invented"
    );
}

/// The pinned corpus project itself, loaded from disk rather than rebuilt here.
///
/// The exit gate asks for saved projects to lower "without hand-rebuilding their patches in
/// tests", and every fixture above is a hand-rebuild. This one is not: it reads the bytes
/// `corpus/v2-reference/manifest.json` pins by digest, so a change to the fixture builders
/// reaches this test instead of passing it by.
#[test]
fn the_pinned_corpus_project_lowers_and_compiles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/v2-reference/projects/subtractive-voice.ptz");
    let project = crate::project::ProjectFile::load(&path)
        .unwrap_or_else(|e| panic!("CORPUS-0001 must load from {}: {e}", path.display()));

    let saved = project
        .instruments
        .first()
        .expect("CORPUS-0001 declares one instrument");
    let lowered = lower_voice_patch(
        saved.id,
        &saved.patch.modules,
        &saved.patch.connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    let ir = lowered
        .ir
        .unwrap_or_else(|| panic!("the pinned project must lower: {:?}", lowered.diagnostics));
    assert!(
        ir.nodes()
            .iter()
            .any(|n| matches!(n.kind(), IrNodeKind::Saw { .. })),
        "CORPUS-0001 authors a sawtooth, and it must reach the sawtooth node"
    );

    let outcome = compile(&ir, &RenderConfig::new(harness_profile()));
    assert!(
        outcome.plan().is_ok(),
        "the pinned project's graph must be admissible: {:?}",
        outcome.plan().err()
    );
}

/// Every module the pinned project declares resolves, even though the patch does not lower.
///
/// Separates two failures that would otherwise look alike: an identity the lowerer cannot
/// read, and a node kind V2 does not have. Only the second is true of `CORPUS-0001`.
#[test]
fn every_module_in_the_pinned_corpus_project_resolves() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/v2-reference/projects/subtractive-voice.ptz");
    let project = crate::project::ProjectFile::load(&path).expect("CORPUS-0001 loads");
    let saved = project.instruments.first().expect("one instrument");

    let resolved =
        ResolvedIdentities::resolve(&saved.patch.modules).expect("every saved identity resolves");
    assert_eq!(
        resolved.len(),
        saved.patch.modules.len(),
        "resolution must cover the patch rather than a subset of it"
    );
}

// ---------------------------------------------------------------------------
// The asymmetries an independent read of the first revision found
// ---------------------------------------------------------------------------

/// A saved parameter no V2 node kind reads is reported rather than dropped.
#[test]
fn a_parameter_the_mapping_does_not_read_is_reported() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("drive", 2.0)]);
            choice(m, "model", "acid");
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    for key in ["drive", "model"] {
        assert!(
            lowered.diagnostics.iter().any(|d| {
                matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == key)
            }),
            "{key} is authored state V2 does not read, and must not vanish"
        );
    }
    assert_eq!(
        Fidelity::of(&lowered.diagnostics),
        Fidelity::UnsupportedScope
    );
}

/// A saved YAMS script is authored state too.
#[test]
fn a_saved_script_is_reported() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            m.scripts.insert("1".to_owned(), "out = 1".to_owned());
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. } if capability.contains("YAMS")
        )),
        "a control script must not be silently discarded"
    );
}

/// A choice stored as its numeric index is refused, not treated as absent.
///
/// V1's own descriptor path decodes a numeric waveform, so defaulting here would turn a
/// saved sawtooth into a sine and render it — the reinterpretation `AGENTS.md` forbids.
#[test]
fn a_choice_stored_as_a_number_is_refused_rather_than_defaulted() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Oscillator {
            m.parameters
                .insert("waveform".to_owned(), ParamValue::Float(2.0));
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "a numeric waveform must not silently become a sine"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == "waveform")
                && d.severity() == Severity::Refused
        }),
        "the refusal must name the waveform parameter"
    );
}

/// An amplifier with no control cable is refused, because the two engines disagree about it.
///
/// V1 reads an unpatched `cv` at unity and sounds; V2 reads it as defined silence. Lowering
/// it would produce a graph that compiles, renders, and is silent with nothing saying so.
#[test]
fn an_amplifier_with_no_control_cable_is_refused() {
    let (modules, mut connections) = corpus_patch("sine");
    connections.retain(|c| !(c.to.0 == "amp-1" && c.to.1 == "cv"));
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(
        lowered.ir.is_none(),
        "the topology must not lower to silence"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("no control cable")
                )
        }),
        "the refusal must say why, got {:?}",
        lowered.diagnostics
    );
}

/// V1's resonance clamp is applied before the conversion, so a saved `1.0` still lowers.
///
/// V1 renders `1.0` at `0.99`, which is `k = 0.02` and therefore `Q = 50`. Converting the raw
/// value would divide by zero's neighbourhood and refuse a filter V1 plays.
#[test]
fn a_saved_resonance_of_one_lowers_at_the_value_v1_renders() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("resonance", 1.0)]);
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered
        .ir
        .expect("V1 plays this filter, so V2 must lower it");

    let q = ir
        .nodes()
        .iter()
        .find_map(|n| match n.kind() {
            IrNodeKind::Filter { resonance, .. } => Some(resonance.as_f32()),
            _ => None,
        })
        .expect("the patch has a filter");
    assert!(
        (q - 50.0).abs() < 0.01,
        "V1's 0.99 clamp gives k = 0.02 and therefore Q = 50, got {q}"
    );
}

/// The DSP stages of a voice patch are per-voice; only the terminating output is not.
#[test]
fn the_voice_patch_nodes_carry_voice_scope_and_the_output_does_not() {
    use synth_engine_v2::ir::ExecutionScope;

    let (modules, connections) = corpus_patch("sine");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered.ir.expect("lowers");

    for node in ir.nodes() {
        let expected = if matches!(node.kind(), IrNodeKind::Output) {
            ExecutionScope::Global
        } else {
            ExecutionScope::Voice
        };
        assert_eq!(
            node.scope(),
            expected,
            "{:?} carries the wrong execution scope",
            node.kind()
        );
    }
}

/// An omitted envelope key means what V1 means by omitting it.
#[test]
fn absent_envelope_parameters_use_v1s_own_defaults() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Envelope {
            m.parameters.clear();
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered.ir.expect("lowers");

    let envelope = ir
        .nodes()
        .iter()
        .find_map(|n| match n.kind() {
            IrNodeKind::Envelope {
                attack,
                decay,
                sustain,
                release,
                ..
            } => Some((
                attack.as_f32(),
                decay.as_f32(),
                sustain.as_f32(),
                release.as_f32(),
            )),
            _ => None,
        })
        .expect("the patch has an envelope");
    assert!(
        (envelope.0 - 0.01).abs() < 1e-6
            && (envelope.1 - 0.1).abs() < 1e-6
            && (envelope.2 - 0.7).abs() < 1e-6
            && (envelope.3 - 0.3).abs() < 1e-6,
        "V1's envelope defaults are 0.01/0.1/0.7/0.3, got {envelope:?}"
    );
}

/// An audio cable into a control input is refused where the user authored it.
///
/// `GraphIr::build` does not compare domains, so without this the lowering would report
/// success and `compile` would refuse the plan much later with nothing naming the cable.
#[test]
fn an_audio_cable_into_a_control_input_is_refused_by_the_lowerer() {
    // The envelope's own cable is replaced rather than joined, so the fan-in check does not
    // fire first and the domain check is the one that answers.
    let (modules, mut connections) = corpus_patch("sine");
    connections.retain(|c| !(c.to.0 == "amp-1" && c.to.1 == "cv"));
    connections.push(ConnectionState {
        from: ("osc-1".to_owned(), "out".to_owned()),
        to: ("amp-1".to_owned(), "cv".to_owned()),
    });
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(
        lowered.ir.is_none(),
        "the domain mismatch must stop lowering"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Connection { .. })
                && matches!(d.reason(), LoweringReason::DomainMismatch { .. })
                && d.severity() == Severity::Refused
        }),
        "the refusal must be a domain mismatch rather than fan-in, got {:?}",
        lowered.diagnostics
    );
}

/// V1's amplifier pans and V2's does not, so every lowered amplifier reports the stage.
#[test]
fn the_amplifiers_pan_stage_is_reported_on_the_amplifier() {
    let (modules, connections) = corpus_patch("sine");
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(
        lowered.diagnostics.iter().any(|d| {
            d.subject()
                == &ProjectSubject::Module {
                    instrument: instrument(),
                    module: ModuleId::new(ModuleType::Amplifier, 1),
                }
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("pan stage")
                )
        }),
        "the pan stage must be reported against the amplifier, not another module"
    );
}

// ---------------------------------------------------------------------------
// Topologies and encodings V1 accepts and V2 does not
// ---------------------------------------------------------------------------

/// A numeric value this cannot read is refused, not quietly replaced by a default.
#[test]
fn a_parameter_value_of_an_unreadable_kind_is_refused() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            m.parameters
                .insert("cutoff".to_owned(), ParamValue::Bool(true));
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "a cutoff this cannot read must not become the 1000 Hz default"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == "cutoff")
                && d.severity() == Severity::Refused
        }),
        "the refusal must name the parameter, got {:?}",
        lowered.diagnostics
    );
}

/// Two cables into one input are refused where the user drew the second.
///
/// V1 sums them; V2 refuses fan-in. Lowering both would build cleanly and fail at `compile`,
/// with nothing naming either connection.
#[test]
fn two_cables_into_one_input_are_refused_at_the_connection() {
    let (mut modules, mut connections) = corpus_patch("sine");
    modules.push({
        let mut second = module("osc-2", ModuleType::Oscillator);
        floats(&mut second, &[("level", 1.0), ("uni_phase", 0.0)]);
        choice(&mut second, "waveform", "sine");
        second
    });
    connections.push(ConnectionState {
        from: ("osc-2".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "in".to_owned()),
    });

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "V2 refuses fan-in, so this must not lower"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Connection { to, .. } if to.0 == "flt-1")
                && d.severity() == Severity::Refused
        }),
        "the refusal must name the connection, got {:?}",
        lowered.diagnostics
    );
}

/// A patch V1 terminates at its amplifier is refused rather than lowered without an output.
#[test]
fn a_patch_with_no_output_module_is_refused() {
    let (mut modules, mut connections) = corpus_patch("sine");
    modules.retain(|m| m.module_type != ModuleType::StereoOutput);
    connections.retain(|c| c.to.0 != "out-1");

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "V1 terminates this at the amplifier; V2 has no output, so it must not lower"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("no explicit output module")
                )
        }),
        "the refusal must say the patch has no output, got {:?}",
        lowered.diagnostics
    );
}

/// A cable leaving the terminating node is refused at the connection, not at the instrument.
#[test]
fn a_cable_out_of_the_output_module_is_refused_at_the_connection() {
    let (modules, mut connections) = corpus_patch("sine");
    connections.push(ConnectionState {
        from: ("out-1".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "in".to_owned()),
    });

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_none());
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Connection { from, .. } if from.0 == "out-1")
                && d.severity() == Severity::Refused
        }),
        "V2's output node declares no output port, and the diagnostic must name the cable \
         rather than the instrument: {:?}",
        lowered.diagnostics
    );
}

/// An omitted parameter is judged against V1's default, not against zero.
#[test]
fn omitted_parameters_are_judged_against_v1s_defaults() {
    // `uni_phase` absent means 1.0 in V1 — full phase randomisation — which V2 does not do.
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Oscillator {
            m.parameters.remove("uni_phase");
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == "uni_phase")
        }),
        "an omitted uni_phase means 1.0 in V1 and must be reported"
    );

    // `master` absent means 0.8 in V1, which is not the unity V2's output applies.
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::StereoOutput {
            m.parameters.remove("master");
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == "master")
        }),
        "an omitted master means 0.8 in V1 and must be reported"
    );
}

/// A refused parameter value stops the lowering rather than sitting beside an IR.
///
/// `Severity::Refused` means lowering stopped; an outcome carrying one and an `ir: Some`
/// would make the severity mean nothing.
#[test]
fn a_refused_neutral_parameter_stops_the_lowering() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::StereoOutput {
            m.parameters
                .insert("master".to_owned(), ParamValue::Bool(true));
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "a Refused diagnostic and an IR cannot both be true"
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused)
    );
}

/// Two output modules are refused, and the extra one is named.
#[test]
fn a_second_output_module_is_refused_and_named() {
    let (mut modules, connections) = corpus_patch("sine");
    let mut second = module("out-2", ModuleType::StereoOutput);
    floats(&mut second, &[("master", 1.0)]);
    modules.push(second);

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_none(), "V2 admits exactly one output");
    assert!(
        lowered.diagnostics.iter().any(|d| {
            d.subject()
                == &ProjectSubject::Module {
                    instrument: instrument(),
                    module: ModuleId::new(ModuleType::StereoOutput, 2),
                }
        }),
        "the extra output must be named, got {:?}",
        lowered.diagnostics
    );
}

/// V1's own clamps are applied before conversion, so a value V1 renders is a value V2 renders.
#[test]
fn v1s_own_clamps_are_applied_before_conversion() {
    // Oscillator level: V1 clamps to [0, 2], so a saved -1.0 is silence there.
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Oscillator {
            floats(m, &[("level", -1.0)]);
        }
    }
    let ir = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    )
    .ir
    .expect("lowers");
    let amplitude = ir
        .nodes()
        .iter()
        .find_map(|n| match n.kind() {
            IrNodeKind::Sine { amplitude, .. } => Some(amplitude.as_f32()),
            _ => None,
        })
        .expect("the patch has an oscillator");
    assert!(
        amplitude.abs() < f32::EPSILON,
        "V1 clamps a negative level to silence; got {amplitude}"
    );

    // Filter cutoff: V1's FILTER_RANGE tops out at 20 kHz.
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("cutoff", 30_000.0)]);
        }
    }
    let ir = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    )
    .ir
    .expect("lowers");
    let cutoff = ir
        .nodes()
        .iter()
        .find_map(|n| match n.kind() {
            IrNodeKind::Filter { cutoff, .. } => Some(cutoff.as_f32()),
            _ => None,
        })
        .expect("the patch has a filter");
    assert!(
        (cutoff - 20_000.0).abs() < 1e-3,
        "V1 clamps a 30 kHz cutoff to 20 kHz; got {cutoff}"
    );
}

/// The most negative integer does not overflow the numeric boundary.
#[test]
fn the_most_negative_integer_parameter_is_refused_without_overflowing() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            m.parameters
                .insert("cutoff".to_owned(), ParamValue::Int(i32::MIN));
        }
    }
    // `i32::MIN.abs()` overflows; `unsigned_abs` does not. The assertion is that this
    // returns at all.
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_none());
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused)
    );
}

/// A cable repeated verbatim is not fan-in.
///
/// V1 keeps connections in a set, so the repeat is a no-op there and the input still has one
/// cable. Refusing it would refuse a patch V1 plays.
#[test]
fn a_cable_repeated_verbatim_is_not_treated_as_fan_in() {
    let (modules, mut connections) = corpus_patch("sine");
    let repeat = connections[0].clone();
    connections.push(repeat);

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_some(),
        "an exact duplicate cable is one cable, not fan-in: {:?}",
        lowered.diagnostics
    );
}

/// A non-finite saved value is refused rather than slipping through the neutrality test.
///
/// `NaN` is unequal to every neutral value and equal to none, so an epsilon or equality test
/// alone would let it reach a quantity.
#[test]
fn a_non_finite_saved_value_is_refused() {
    for bad in [f32::NAN, f32::INFINITY] {
        let (mut modules, connections) = corpus_patch("sine");
        for m in &mut modules {
            if m.module_type == ModuleType::StereoOutput {
                floats(m, &[("master", bad)]);
            }
        }
        let lowered = lower_voice_patch(
            instrument(),
            &modules,
            &connections,
            synth_engine_v2::quantities::EventCount::NONE,
        );
        assert!(lowered.ir.is_none(), "{bad} must not reach a quantity");
        assert!(
            lowered
                .diagnostics
                .iter()
                .any(|d| d.severity() == Severity::Refused)
        );
    }
}

/// A value a hair away from neutral is still not neutral.
#[test]
fn a_value_near_neutral_is_still_reported() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::StereoOutput {
            floats(m, &[("master", 0.999_999_94)]);
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Parameter { parameter, .. } if parameter == "master")
        }),
        "a master V1 applies and V2 does not must be reported however close to unity it is"
    );
}

/// A domain mismatch keeps the authored port name in the structured field.
#[test]
fn a_domain_mismatch_reports_the_port_the_user_authored() {
    // The envelope's own cable is replaced rather than joined, so the fan-in check does not
    // fire first and the domain check is the one that answers.
    let (modules, mut connections) = corpus_patch("sine");
    connections.retain(|c| !(c.to.0 == "amp-1" && c.to.1 == "cv"));
    connections.push(ConnectionState {
        from: ("osc-1".to_owned(), "out".to_owned()),
        to: ("amp-1".to_owned(), "cv".to_owned()),
    });
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    assert!(lowered.ir.is_none());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::DomainMismatch { port, expected, found }
                if port == "cv" && *expected == "control" && *found == "audio"
        )),
        "the port field must still be the name the project spells, got {:?}",
        lowered.diagnostics
    );
}

/// A feedback path is refused at the cable that closes it.
///
/// V2 refuses a cyclic graph at compilation and `GraphIr::build` does not look, so without a
/// check here the lowering would report success for a patch that can never render.
#[test]
fn a_feedback_path_is_refused_at_the_cable_that_closes_it() {
    let (modules, mut connections) = corpus_patch("sine");
    // amp-1.out already reaches out-1; sending it back into the filter closes a loop
    // osc -> flt -> amp -> flt.
    connections.retain(|c| !(c.to.0 == "flt-1" && c.to.1 == "in"));
    connections.push(ConnectionState {
        from: ("amp-1".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "in".to_owned()),
    });

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_none(), "a cycle must not lower");
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Connection { .. })
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("feedback")
                )
        }),
        "the refusal must name a cable, got {:?}",
        lowered.diagnostics
    );
}

/// A self-loop is a cycle too.
#[test]
fn a_self_loop_is_refused() {
    let (modules, mut connections) = corpus_patch("sine");
    connections.retain(|c| !(c.to.0 == "flt-1" && c.to.1 == "in"));
    connections.push(ConnectionState {
        from: ("flt-1".to_owned(), "out".to_owned()),
        to: ("flt-1".to_owned(), "in".to_owned()),
    });
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(lowered.ir.is_none(), "a self-loop must not lower");
}

/// A non-finite value on a parameter that is read but never judged is still refused.
///
/// `env_amt` is dormant without a cutoff cable, so nothing else looks at it — but a `NaN`
/// there poisons V1's cutoff through a multiplication by zero.
#[test]
fn a_non_finite_value_on_a_dormant_parameter_is_refused() {
    let (mut modules, connections) = corpus_patch("sine");
    for m in &mut modules {
        if m.module_type == ModuleType::Filter {
            floats(m, &[("env_amt", f32::NAN)]);
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_none(),
        "a NaN must not survive because the parameter happens to be dormant"
    );
}

/// Which output is named as the extra one does not depend on the array's order.
#[test]
fn extra_outputs_are_named_by_identity_not_array_order() {
    let (mut modules, connections) = corpus_patch("sine");
    let mut second = module("out-2", ModuleType::StereoOutput);
    floats(&mut second, &[("master", 1.0)]);
    modules.push(second);

    let forward = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    modules.reverse();
    let reversed = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );

    let named = |l: &super::graph::LoweredGraph| -> Vec<ProjectSubject> {
        l.diagnostics
            .iter()
            .filter(|d| d.severity() == Severity::Refused)
            .map(|d| d.subject().clone())
            .collect()
    };
    assert_eq!(
        named(&forward),
        named(&reversed),
        "the extra output must be chosen by identity, not by where it sits in the array"
    );
}

// ---------------------------------------------------------------------------
// The bounded in-process smoke render
// ---------------------------------------------------------------------------

/// The two engines count ticks the same way, and nothing but this says so.
///
/// `lower_performance` maps a saved tick to a V2 musical tick with no conversion. That is
/// only correct because two independently declared constants happen to agree; if either
/// moves, every position the lowerer computes moves with it and no other test would notice.
#[test]
fn both_engines_count_the_same_ticks_to_a_quarter_note() {
    assert_eq!(
        synth_sequencer::TICKS_PER_QUARTER,
        synth_engine_v2::tempo::TICKS_PER_QUARTER,
        "the lowerer maps a saved tick to a V2 tick unchanged, which requires these to agree"
    );
}

/// A saved project renders through V2, end to end, and makes sound.
///
/// The weakest useful claim, and deliberately so: this says the lowering, admission,
/// scheduling and rendering path connects — not that it matches V1, which V2's single
/// velocity scale still makes impossible to claim.
#[test]
fn a_saved_instrument_and_song_render_through_v2_and_are_audible() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let song = four_note_song();

    let profile = harness_profile();

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        profile,
        FrameCount::new(48_000),
    );
    assert!(
        rendered.is_audible(),
        "a saved note renders now: `P04-R001`'s precondition is discharged, got {:?}",
        rendered.diagnostics
    );
    assert!(
        !rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "and nothing refuses it, got {:?}",
        rendered.diagnostics
    );
    // And since ADR-0059 the velocity half of the reporting closes too: the composition is
    // V1's, so nothing names it as unrepresented. What still keeps this outcome from
    // `Faithful` is Phase 8's — the master volume and the pan stages — and is named as such.
    assert!(
        !names_the_composition(&rendered),
        "{:?}",
        rendered.diagnostics
    );
}

/// Whether any diagnostic names the velocity composition ADR-0059 built as unrepresented.
fn names_the_composition(rendered: &super::render::SmokeRender) -> bool {
    rendered.diagnostics.iter().any(|d| {
        matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, owner }
                if capability.contains("velocity") || owner.contains("composition law")
        )
    })
}

/// Two notes overlapping on one gate are refused rather than rendered wrongly.
#[test]
fn overlapping_notes_on_one_gate_are_refused() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let song = overlapping_song();

    let profile = harness_profile();

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        profile,
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(0),
        "one gate sounds one note; the second would end early and silently"
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("two notes sounding at once")
                )
        }),
        "the refusal must name the overlap, got {:?}",
        rendered.diagnostics
    );
}

/// A song whose tempo changes places its notes at unequal sample intervals.
///
/// `CORPUS-0009`'s property, reduced to the one thing this slice can check: the renderer
/// consumed the authored tempo map rather than one constant tempo.
#[test]
fn a_tempo_change_moves_the_notes_it_should() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let profile = harness_profile();

    let steady = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &crate::project::GlobalProjectState::default(),
        profile,
        FrameCount::new(4_800),
    );
    let doubled = super::render::smoke_render(
        &saved,
        &double_tempo_song(),
        &crate::project::GlobalProjectState::default(),
        profile,
        FrameCount::new(4_800),
    );

    assert!(
        steady.lowered_events != synth_engine_v2::quantities::EventCount::NONE
            && doubled.lowered_events != synth_engine_v2::quantities::EventCount::NONE
    );
    assert!(
        doubled.lowered_frames.as_u64() < steady.lowered_frames.as_u64(),
        "at twice the tempo the same notes occupy fewer frames: {} vs {}",
        doubled.lowered_frames.as_u64(),
        steady.lowered_frames.as_u64()
    );
}

// --- fixtures for the smoke render -----------------------------------------

/// The corpus instrument, with only the fields lowering reads.
fn saved_instrument(
    modules: Vec<ModuleState>,
    connections: Vec<ConnectionState>,
) -> crate::patch::InstrumentState {
    let mut patch = crate::patch::Patch::new("Subtractive Voice");
    patch.modules = modules;
    patch.connections = connections;
    crate::patch::InstrumentState {
        id: instrument(),
        name: "Subtractive Voice".to_owned(),
        channel: 1,
        volume: synth_core::Gain::UNITY,
        pan: synth_core::BipolarValue::CENTER,
        muted: false,
        solo: false,
        key_range: (0, 127),
        transpose: synth_core::Semitones::ZERO,
        oversampling: 1,
        category: 0,
        description: String::new(),
        color: None,
        allocation_mode: synth_engine::voice_allocator::AllocationMode::default(),
        stealing_strategy: synth_engine::voice_allocator::StealingStrategy::default(),
        unison_detune: synth_core::Cents::ZERO,
        unison_spread: synth_core::NormalizedValue::MIN,
        max_voices: synth_core::VoiceCount::new(8),
        velocity_amp_sensitivity: synth_core::NormalizedValue::MIN,
        velocity_filter_sensitivity: synth_core::NormalizedValue::MIN,
        sidechain_source_id: None,
        patch,
    }
}

/// A song with one pattern of separated notes on one track, at one tempo.
fn four_note_song() -> synth_sequencer::Song {
    song_with(
        120.0,
        &[(0, 720), (960, 720), (1920, 720), (2880, 720)],
        None,
    )
}

/// The same notes at twice the tempo.
///
/// Deliberately the *same* note list as [`four_note_song`]. An earlier version used a shorter
/// one, so the doubled-tempo render was shorter even if the tempo map were ignored entirely —
/// the test would have passed while the thing it names regressed. An independent review found
/// that; the fixtures now differ in exactly one thing.
fn double_tempo_song() -> synth_sequencer::Song {
    song_with(
        120.0,
        &[(0, 720), (960, 720), (1920, 720), (2880, 720)],
        Some((0, 240.0)),
    )
}

/// Two notes that sound at once through one gate.
fn overlapping_song() -> synth_sequencer::Song {
    song_with(120.0, &[(0, 960), (480, 960)], None)
}

/// Build a one-track song from `(start, duration)` tick pairs.
fn song_with(
    bpm: f32,
    notes: &[(u32, u32)],
    tempo_change: Option<(u64, f32)>,
) -> synth_sequencer::Song {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(bpm);
    let pattern_id = song.create_pattern(Duration(3840));
    let track_id = song.create_track("track");

    if let Some(pattern) = song.pattern_mut(pattern_id) {
        for (start, duration) in notes {
            let id = pattern.add_note(
                PatternTick(*start),
                Pitch::new(60).expect("middle C is a valid pitch"),
                Velocity::new(0.755_905_5),
            );
            if let Some(note) = pattern.note_mut(id) {
                note.duration = Some(Duration(*duration));
            }
        }
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick::ZERO));

    if let Some((tick, bpm)) = tempo_change {
        song.set_tempo_at(Tick(tick), synth_core::Bpm::new(bpm));
    }
    song
}

/// Notes on another instrument's track are not this plan's to render.
#[test]
fn a_placement_on_another_instruments_track_is_not_lowered() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let mut song = four_note_song();
    for track in song.tracks_mut() {
        track.instrument = synth_engine::instrument::InstrumentId::new(7);
    }

    let profile = harness_profile();
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        profile,
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(0),
        "no note reaches this instrument, so nothing is lowered for it"
    );
    assert!(
        !rendered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. }
                if capability.contains("two notes sounding at once")
        )),
        "another instrument's notes must not raise this instrument's overlap refusal"
    );
}

/// A muted track contributes nothing, and a soloed one silences the rest.
#[test]
fn track_mute_and_solo_decide_what_is_lowered() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let mut muted = four_note_song();
    for track in muted.tracks_mut() {
        track.mute = true;
    }
    let rendered = super::render::smoke_render(
        &saved,
        &muted,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(0),
        "a muted track contributes nothing"
    );

    // With the only track soloed, the same notes still play.
    let mut soloed = four_note_song();
    for track in soloed.tracks_mut() {
        track.solo = true;
    }
    let rendered = super::render::smoke_render(
        &saved,
        &soloed,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8),
        "the soloed track is this one, so its four notes still lower to eight edges"
    );
}

/// A note starting at or past its pattern's end is hidden, and is not lowered.
#[test]
fn a_note_past_its_patterns_end_is_not_lowered() {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    // A short pattern holding a note that begins after it ends.
    let pattern_id = song.create_pattern(Duration(960));
    let track_id = song.create_track("track");
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        for start in [0_u32, 1920] {
            let id = pattern.add_note(
                PatternTick(start),
                Pitch::new(60).expect("middle C"),
                Velocity::new(0.5),
            );
            if let Some(note) = pattern.note_mut(id) {
                note.duration = Some(Duration(480));
            }
        }
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick::ZERO));

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(2),
        "the visible note lowers to two edges and the hidden one to none"
    );

    // And no note was refused: the hidden one is skipped rather than reported, because a
    // pattern ending before it is not a project defect.
    assert!(
        !rendered
            .diagnostics
            .iter()
            .any(|d| matches!(d.subject(), ProjectSubject::Note { .. })),
        "a hidden note is skipped silently, got {:?}",
        rendered.diagnostics
    );
}

/// A placement whose absolute position cannot be formed is refused rather than wrapping.
#[test]
fn a_note_position_that_does_not_fit_is_refused() {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern_id = song.create_pattern(Duration(3840));
    let track_id = song.create_track("track");
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        let id = pattern.add_note(
            PatternTick(960),
            Pitch::new(60).expect("middle C"),
            Velocity::new(0.5),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(480));
        }
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick(u64::MAX - 10)));

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::NONE
    );
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "a wrapped position would put a release before its own onset"
    );
}

/// The bounded smoke render has a bound, and it is enforced before the allocation.
#[test]
fn a_render_longer_than_the_bounded_scope_is_refused() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    // Eleven minutes of tail at 48 kHz, past the ten-minute ceiling.
    let rendered = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(11 * 60 * 48_000),
    );
    assert!(
        rendered.samples.is_empty(),
        "the bound must stop this before `render_offline` allocates"
    );
    assert!(
        rendered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. }
                if capability.contains("bounded smoke scope")
        )),
        "the refusal must say the bound is what stopped it, got {:?}",
        rendered.diagnostics
    );
}

/// The declared event peak counts the notes the renderer is actually given.
#[test]
fn the_declared_event_peak_counts_the_lowered_timeline() {
    let song = four_note_song();
    let (modules, _) = corpus_patch("sawtooth");
    let peak = super::performance::peak_events_per_quantum(
        instrument(),
        &modules,
        &song,
        SampleRate::new(48_000.0).expect("a real rate"),
    )
    .expect("the arrangement reads");
    assert_eq!(
        peak,
        synth_engine_v2::quantities::EventCount::measured(1),
        "four separated notes never put two edges in one quantum"
    );
}

/// A harness profile at the rate every smoke-render test uses.
/// The smoke render's profile. **Stereo** since `P08-S001`: the lowered instrument renders
/// through its mix channel, whose output is stereo as V1's is, and a mono stream would
/// refuse the channel's edge into the output rather than down-mix it (`SOUND-INV-014`).
/// The samples are interleaved, so a frame count is half the sample count.
fn harness_profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Stereo,
    )
    .expect("a harness profile")
}

/// How many frames a smoke render holds: its interleaved samples over the stream's channels.
fn frames_of(rendered: &super::render::SmokeRender) -> u64 {
    (rendered.samples.len() / harness_profile().capabilities().channel_layout().channels()) as u64
}

/// A note expression or ornament is refused, because V1 expands it before playing.
#[test]
fn a_note_expression_is_refused_rather_than_played_as_authored() {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern_id = song.create_pattern(Duration(3840));
    let track_id = song.create_track("track");
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(60).expect("middle C"),
            Velocity::new(0.5),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(480));
            note.expression = Some(synth_sequencer::NoteExpression::default());
        }
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick::ZERO));

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(0),
        "an expression can suppress the note, so the authored span must not be lowered"
    );

    // An ornament, on a note the pattern **hides**. V1 evaluates an ornament on every active
    // tick regardless of the note's own start, so a lead-in figure on a note at the pattern's
    // end lands its grace hits inside the pattern; the note's own onset never plays. An
    // independent review found the ornament check after the hidden-note skip, where this note
    // never reached it — and found this test exercising only an expression.
    let mut song = four_note_song();
    let pattern_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id;
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        let id = pattern.add_note(
            PatternTick(3840),
            Pitch::new(60).expect("middle C"),
            Velocity::new(0.5),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(480));
            note.ornament = Some(synth_sequencer::Ornament::default());
        }
    }
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("ornament")
                )
        }) && rendered.samples.is_empty(),
        "a hidden note's lead-in ornament sounds in V1, so it must be refused by name: {:?}",
        rendered.diagnostics
    );
}

/// A muted instrument lowers to a muted channel and renders the silence V1 renders.
///
/// Until `P08-S001` a mute was refused by name; now it is the channel's mute, held from the
/// first sample, so the notes are lowered — V1 plays them into a silenced channel — and the
/// render is zeros with no mark naming the mute.
#[test]
fn a_muted_instrument_lowers_to_a_muted_channel_and_renders_silence() {
    let (modules, connections) = corpus_patch("sine");
    let mut saved = saved_instrument(modules, connections);
    saved.muted = true;

    let rendered = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8),
        "the notes are lowered into the silenced channel: {:?}",
        rendered.diagnostics
    );
    assert!(
        !rendered.samples.is_empty() && rendered.samples.iter().all(|s| *s == 0.0),
        "and the render is the silence V1 renders"
    );
    assert!(
        !rendered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. }
                if capability.contains("muted")
        )),
        "nothing names the mute any more, got {:?}",
        rendered.diagnostics
    );
}

/// The instrument's fader, pan and mute are the channel's authored bases (`P08-S001`).
#[test]
fn the_instruments_strip_lowers_onto_its_mix_channel() {
    use synth_engine_v2::ir::IrNodeKind;
    let (modules, connections) = corpus_patch("sine");
    let mut saved = saved_instrument(modules, connections);
    saved.volume = synth_core::Gain::new(0.5);
    saved.pan = synth_core::BipolarValue::new(-0.25);
    saved.muted = true;

    let lowered = super::graph::lower_voice_patch_with(
        instrument(),
        &saved.patch.modules,
        &saved.patch.connections,
        synth_engine_v2::quantities::EventCount::NONE,
        Some(synth_engine_v2::quantities::NormalizedLevel::FULL),
        Some(super::graph::ChannelStrip {
            fader: synth_engine_v2::quantities::Amplitude::new(0.5).expect("finite"),
            pan: synth_engine_v2::controller::BipolarLevel::new(-0.25).expect("in range"),
            muted: true,
        }),
        &super::modulation::SongModulators::default(),
    );
    let ir = lowered.ir.expect("the strip lowers");
    let channel = ir
        .node(super::identity::CHANNEL)
        .expect("one channel at the reserved address");
    assert_eq!(
        channel.scope(),
        synth_engine_v2::ir::ExecutionScope::Channel
    );
    match channel.kind() {
        IrNodeKind::Channel { fader, pan, muted } => {
            assert_eq!(fader.as_f32(), 0.5);
            assert_eq!(pan.as_f32(), -0.25);
            assert!(muted);
        }
        other => panic!("{other:?}"),
    }
    // The chain: the scaler feeds the channel and the channel feeds the output, and nothing
    // else reaches the output.
    let output = ir
        .nodes()
        .iter()
        .find(|node| matches!(node.kind(), IrNodeKind::Output))
        .expect("an output")
        .id();
    let into_output: Vec<_> = ir
        .edges()
        .iter()
        .filter(|edge| edge.to().0 == output)
        .map(|edge| edge.from().0)
        .collect();
    assert_eq!(into_output, vec![super::identity::CHANNEL]);
    assert!(
        ir.edges()
            .iter()
            .any(|edge| edge.from().0 == super::identity::VOICE_OUTPUT_SCALER
                && edge.to().0 == super::identity::CHANNEL)
    );
}

/// The fader scales the render, the pan places it and neither is a mark any more.
#[test]
fn the_instruments_fader_and_pan_reach_the_render_under_v1s_laws() {
    let rendered_at = |volume: f32, pan: f32| {
        let (modules, connections) = corpus_patch("sine");
        let mut saved = saved_instrument(modules, connections);
        saved.volume = synth_core::Gain::new(volume);
        saved.pan = synth_core::BipolarValue::new(pan);
        super::render::smoke_render(
            &saved,
            &four_note_song(),
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let unity = rendered_at(1.0, 0.0);
    let half = rendered_at(0.5, 0.0);
    assert!(unity.is_audible() && half.is_audible());
    let ratio = peak(&half) / peak(&unity);
    assert!(
        (ratio - 0.5).abs() < 1e-6,
        "half the fader is half the peak, got {ratio}"
    );
    // Hard left: the right side is exactly silent — `sin(0)` is zero, not nearly zero —
    // and the left side is the centre render scaled by V1's own coefficient ratio.
    let left = rendered_at(1.0, -1.0);
    let sides = |rendered: &super::render::SmokeRender| {
        let l: Vec<f32> = rendered.samples.iter().copied().step_by(2).collect();
        let r: Vec<f32> = rendered
            .samples
            .iter()
            .copied()
            .skip(1)
            .step_by(2)
            .collect();
        (l, r)
    };
    let (left_l, left_r) = sides(&left);
    assert!(
        left_r.iter().all(|s| *s == 0.0),
        "hard left renders nothing on the right"
    );
    assert!(
        left_l.iter().any(|s| s.abs() > 0.01),
        "and something on the left"
    );
    let (centre_l, centre_r) = sides(&unity);
    assert_eq!(centre_l, centre_r, "centre is symmetric");
    let (hard, _) = synth_core::Gain::from_pan(synth_core::BipolarValue::new(-1.0));
    let (centre, _) = synth_core::Gain::from_pan(synth_core::BipolarValue::CENTER);
    let expected = hard.as_f32() / centre.as_f32();
    let measured = peak(&left) / peak(&unity);
    assert!(
        (measured - expected).abs() < 1e-5,
        "the left side carries V1's coefficient ratio {expected}, got {measured}"
    );
    // And no diagnostic names the fader or the pan: they are lowered, not reported.
    for rendered in [&unity, &half, &left] {
        assert!(
            !rendered.diagnostics.iter().any(|d| matches!(
                d.reason(),
                LoweringReason::OwnedByLaterPhase { capability, .. }
                    if capability.contains("instrument volume")
                        || capability.contains("instrument pan")
            )),
            "{:?}",
            rendered.diagnostics
        );
    }
}

/// A saved volume outside V1's own mixer range is refused by name and by value, and one
/// inside it — above unity, where V1's range reaches — lowers.
#[test]
fn a_saved_volume_outside_v1s_mixer_range_is_refused_and_one_inside_it_lowers() {
    let rendered_at = |volume: f32| {
        let (modules, connections) = corpus_patch("sine");
        let mut saved = saved_instrument(modules, connections);
        saved.volume = synth_core::Gain::new(volume);
        super::render::smoke_render(
            &saved,
            &four_note_song(),
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let over = rendered_at(3.0);
    assert!(over.samples.is_empty(), "refused, so nothing renders");
    assert!(
        over.diagnostics.iter().any(|d| matches!(
            (d.severity(), d.reason()),
            (Severity::Refused, LoweringReason::UnsupportedParameterValue { value })
                if value.contains("3") && value.contains("mixer range")
        )),
        "refused by name and by value, got {:?}",
        over.diagnostics
    );
    let hot = rendered_at(1.5);
    assert!(hot.is_audible(), "{:?}", hot.diagnostics);
    let unity = rendered_at(1.0);
    let ratio = peak(&hot) / peak(&unity);
    assert!(
        (ratio - 1.5).abs() < 1e-5,
        "V1's range reaches above unity, got {ratio}"
    );
    let nan = rendered_at(f32::NAN);
    assert!(nan.samples.is_empty());
    assert!(
        nan.diagnostics
            .iter()
            .any(|d| matches!(d.reason(), LoweringReason::UnsupportedParameterValue { .. })),
        "{:?}",
        nan.diagnostics
    );
}

/// A non-finite render is not "audible".
#[test]
fn a_non_finite_render_is_not_audible() {
    let poisoned = super::render::SmokeRender {
        samples: vec![0.0, f32::NAN, 0.0],
        diagnostics: Vec::new(),
        lowered_events: synth_engine_v2::quantities::EventCount::NONE,
        lowered_frames: FrameCount::new(0),
    };
    assert!(
        !poisoned.is_audible(),
        "NaN != 0.0, so a bare non-zero test would call this audible"
    );

    let real = super::render::SmokeRender {
        samples: vec![0.0, 0.5, 0.0],
        diagnostics: Vec::new(),
        lowered_events: synth_engine_v2::quantities::EventCount::NONE,
        lowered_frames: FrameCount::new(0),
    };
    assert!(real.is_audible());
}

/// The pinned corpus project lowers, schedules and **renders** from its own bytes.
///
/// The first gate bullet, in full: a project nothing here rebuilt lowers, compiles, schedules
/// its own notes and makes sound. The work list's precondition — "before rendering the first
/// saved pitched note, close P03-R003 with minimum typed pitch and velocity payload
/// semantics" — is met, so the refusal that used to stand here is gone.
#[test]
fn the_pinned_corpus_project_lowers_to_events_and_renders() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/v2-reference/projects/subtractive-voice.ptz");
    let project = crate::project::ProjectFile::load(&path).expect("CORPUS-0001 loads");
    let saved = project.instruments.first().expect("one instrument");

    let rendered = super::render::smoke_render(
        saved,
        &project.song,
        &project.global,
        harness_profile(),
        FrameCount::new(48_000),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8),
        "CORPUS-0001's four notes lower to eight edges: {:?}",
        rendered.diagnostics
    );
    assert!(
        rendered.lowered_frames.as_u64() > 0,
        "and the tempo map places them"
    );
    assert!(
        rendered.is_audible(),
        "CORPUS-0001 renders its own notes, got {:?}",
        rendered.diagnostics
    );
    assert!(
        !rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "and nothing refuses it, got {:?}",
        rendered.diagnostics
    );
}

/// The second eligible pinned case, from its own bytes.
///
/// `CORPUS-0009` is the other project `P04-R002` leaves eligible, and an independent review
/// pointed out that nothing had actually loaded it — the tempo test above builds its material
/// by hand. This closes that: the pinned project lowers, compiles, and renders, and its
/// authored tempo map is what places its notes.
#[test]
fn the_second_eligible_pinned_project_lowers_to_events() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/v2-reference/projects/tempo-map-arrangement.ptz");
    let project = crate::project::ProjectFile::load(&path)
        .unwrap_or_else(|e| panic!("CORPUS-0009 must load from {}: {e}", path.display()));
    let saved = project
        .instruments
        .first()
        .expect("CORPUS-0009 declares one instrument");

    assert!(
        !project.song.tempo_changes().is_empty(),
        "CORPUS-0009 is the tempo-map case, so it must author tempo changes"
    );

    let rendered = super::render::smoke_render(
        saved,
        &project.song,
        &project.global,
        harness_profile(),
        FrameCount::new(48_000),
    );
    assert!(
        rendered.lowered_events == synth_engine_v2::quantities::EventCount::measured(12),
        "CORPUS-0009's six notes lower to exactly twelve edges: {:?}",
        rendered.diagnostics
    );
    assert!(
        rendered.is_audible(),
        "CORPUS-0009 renders its own notes through its own tempo map, got {:?}",
        rendered.diagnostics
    );
}

/// An arrangement with no notes still renders, which is what keeps the render path checked.
///
/// `P04-R001`'s precondition is on rendering a saved **note**, so a project with none is the
/// one case it does not reach. Rendering it drives lowering, admission, preparation and the
/// render loop end to end, so a regression anywhere in that chain fails here rather than
/// waiting for ADR-0025.
#[test]
fn a_note_free_arrangement_still_renders_through_the_whole_path() {
    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern_id = song.create_pattern(synth_sequencer::Duration(3840));
    let track_id = song.create_track("track");
    assert!(song.place_pattern(pattern_id, track_id, synth_sequencer::Tick::ZERO));

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(0),
        "the arrangement places no note"
    );
    // The arrangement is silent but not empty: V1 renders the placed pattern's two seconds of
    // rest and auto-stops at its end, so the render covers the song as V1 bounds it, plus the
    // tail it was asked for. An earlier revision rendered the tail alone.
    assert!(
        rendered.lowered_frames.as_u64() > 0,
        "the placed pattern gives the song an end: {:?}",
        rendered.diagnostics
    );
    assert_eq!(
        frames_of(&rendered),
        rendered.lowered_frames.as_u64() + 4_800,
        "so the render proceeds over the song's extent, for the tail it was asked for: {:?}",
        rendered.diagnostics
    );
    assert!(
        rendered.samples.iter().all(|s| s.is_finite()),
        "and every sample is finite"
    );
}

/// A refused lowering does not render, even though its event list is empty like a
/// note-free arrangement's.
///
/// The two are indistinguishable by the event list alone, and an earlier revision read only
/// that — so an overlap, an expression or an unrepresentable position produced a `Refused`
/// diagnostic beside a tail-sized buffer of audio.
#[test]
fn a_refused_lowering_does_not_fall_through_to_the_render() {
    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);

    let rendered = super::render::smoke_render(
        &saved,
        &overlapping_song(),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "the overlap must be refused"
    );
    assert!(
        rendered.samples.is_empty(),
        "and a refusal must not produce audio: {} samples came back",
        rendered.samples.len()
    );
}

// ---------------------------------------------------------------------------
// P04-R002: how many saved projects the Phase 4 subset can actually take
// ---------------------------------------------------------------------------

/// Every saved project in the repository, lowered, so the eligible count is measured.
///
/// `P04-R002` says the pinned corpus supplies two eligible cases where the first gate bullet
/// asks for three, and its recorded resolution is to add a third or amend the count. This is
/// the measurement that decides between them: it lowers **every** `.ptz` the repository
/// contains — the ten pinned corpus cases and the seventeen shipped examples — and reports
/// which reach a plan.
///
/// The assertion is on the exact set rather than on a count, so it fails in both directions:
/// a project that becomes eligible is as much a change to `P04-R002` as one that stops being.
#[test]
fn exactly_three_saved_projects_in_the_repository_lower_to_a_plan() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    let mut eligible: Vec<String> = Vec::new();
    let mut examined = 0_usize;
    for directory in [
        root.join("corpus/v2-reference/projects"),
        root.join("assets/examples/projects"),
    ] {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", directory.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            // Both saved-project forms: the plain `.ptz` and the sample-embedding
            // `.ptz.zip` bundle. An extension test alone misses the bundle, and an earlier
            // revision did — it claimed to have classified every saved project while never
            // opening the one the repository ships with its samples inside.
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let Some(file_name) = file_name else { continue };
            let is_bundle = file_name.ends_with(".ptz.zip");
            if !is_bundle && !file_name.ends_with(".ptz") {
                continue;
            }
            examined += 1;
            let name = file_name
                .trim_end_matches(".zip")
                .trim_end_matches(".ptz")
                .to_owned();

            // Every saved project the repository ships must load. Treating a load failure as
            // mere ineligibility would let a persistence regression pass this test silently,
            // since the file is already outside the eligible set.
            let project = if is_bundle {
                let mut library = synth_sampler::SampleLibrary::default();
                crate::bundle::load_bundle(&path, &mut library)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            } else {
                crate::project::ProjectFile::load(&path)
                    .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
            };

            // Eligible means **every** instrument in the project lowers and at least one
            // schedules notes. Looking at the first instrument alone would let a supported
            // first patch hide an unsupported later one, and a note-free first patch hide the
            // notes that follow it — neither of which is "the saved project lowers".
            let mut all_lower = !project.instruments.is_empty();
            let mut any_notes = false;
            for saved in &project.instruments {
                let outcome = super::render::smoke_render(
                    saved,
                    &project.song,
                    &project.global,
                    harness_profile(),
                    FrameCount::new(4_800),
                );
                // Every refusal counts now. `P04-R001`'s render refusal used to be the one
                // this survey looked past — the contract declining to render what the
                // lowering successfully produced — and it is gone: a saved note renders.
                let lowering_failed = outcome
                    .diagnostics
                    .iter()
                    .any(|d| d.severity() == Severity::Refused);
                if lowering_failed {
                    all_lower = false;
                    break;
                }
                if outcome.lowered_events != synth_engine_v2::quantities::EventCount::NONE {
                    any_notes = true;
                }
            }
            if all_lower && any_notes {
                eligible.push(name);
            }
        }
    }

    assert!(
        examined >= 28,
        "the survey must cover both directories and both saved-project forms; it examined \
         only {examined} projects"
    );
    eligible.sort();
    assert_eq!(
        eligible,
        vec![
            "mod-matrix".to_owned(),
            "subtractive-voice".to_owned(),
            "tempo-map-arrangement".to_owned()
        ],
        "P04-R002 recorded two eligible saved projects where the gate asked three, and \
         P07-S003 made the corpus's Mod Matrix case the third; this is the measurement behind \
         that number. A change here is a change to that record."
    );
}

/// A disabled send is a bypass, not a routing V2 must refuse.
#[test]
fn a_disabled_send_does_not_refuse_the_project() {
    use synth_sequencer::{ReturnBusId, TrackSend};

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);

    let mut song = four_note_song();
    for track in song.tracks_mut() {
        track.sends.push(TrackSend {
            target: ReturnBusId::new(0),
            level: synth_core::NormalizedValue::MAX,
            pre_fader: false,
            enabled: false,
        });
    }
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8),
        "a bypassed send contributes nothing, so the project still lowers: {:?}",
        rendered.diagnostics
    );

    // Enabling it is what V2 cannot represent.
    for track in song.tracks_mut() {
        for send in &mut track.sends {
            send.enabled = true;
        }
    }
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("send into a return bus")
                )
        }),
        "an active send routes audio V2 has nowhere to put"
    );
}

/// A saved key range and a saved transpose are refused, because V1 plays other notes than the
/// authored ones when either is set.
///
/// Found by an independent review of the Phase 4 exit, in the same class as the project-global
/// hole `P04-R002`'s measurement found: the lowerer read the instrument's mixer state and its
/// patch, and never the note input `Instrument::note_on_expr` applies before a voice exists.
#[test]
fn instrument_note_input_is_refused_rather_than_ignored() {
    let (modules, connections) = corpus_patch("sawtooth");
    let song = four_note_song();
    let global = crate::project::GlobalProjectState::default();

    // V1's `note_on_expr` returns `None` for a note outside the range, so those notes never
    // sound. Lowering the authored notes would sound a stream V1 never plays.
    let mut narrowed = saved_instrument(modules.clone(), connections.clone());
    narrowed.key_range = (48, 72);
    let rendered = super::render::smoke_render(
        &narrowed,
        &song,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("key range")
                )
                && matches!(d.subject(), ProjectSubject::Instrument { .. })
        }),
        "a key range narrower than the keyboard must be refused by name: {:?}",
        rendered.diagnostics
    );
    assert!(
        rendered.samples.is_empty(),
        "a refusal produces no plan and therefore no audio"
    );

    // V1 reads the range through `KeyRange::new(MidiNote::new(..), ..)`, which swaps reversed
    // endpoints and clamps above 127, so both of these are the **full** keyboard to V1 and
    // neither may be refused. Comparing the serialized tuple would refuse both.
    for neutral in [(127_u8, 0_u8), (0, 255)] {
        let mut odd = saved_instrument(modules.clone(), connections.clone());
        odd.key_range = neutral;
        let rendered = super::render::smoke_render(
            &odd,
            &song,
            &global,
            harness_profile(),
            FrameCount::new(4_800),
        );
        assert!(
            rendered.is_audible(),
            "V1 normalizes {neutral:?} to the full keyboard, so it must render: {:?}",
            rendered.diagnostics
        );
    }

    // V1 transposes every note and **drops** one the transpose moves off the keyboard, which
    // is not what the placement transpose does — that one falls back to the authored pitch.
    let mut transposed = saved_instrument(modules, connections);
    transposed.transpose = synth_core::Semitones::new(5.0);
    let rendered = super::render::smoke_render(
        &transposed,
        &song,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("instrument transpose")
                )
        }),
        "an instrument transpose must be refused by name: {:?}",
        rendered.diagnostics
    );
    assert!(rendered.samples.is_empty());
}

/// A track's own fader and pan are reported, and once per track rather than once per placement.
///
/// V1 mixes each track through `auto.volume.unwrap_or(track.volume)` and the same for pan, so a
/// non-neutral value changes what the render means. V2 has no mixer stage to carry it.
#[test]
fn track_mixer_state_is_reported_rather_than_ignored() {
    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let global = crate::project::GlobalProjectState::default();

    let count = |rendered: &super::render::SmokeRender, needle: &str| {
        rendered
            .diagnostics
            .iter()
            .filter(|d| {
                matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains(needle)
                )
            })
            .count()
    };

    // The control: a neutral track says nothing, so the assertions below cannot pass by the
    // diagnostic being unconditional.
    let neutral = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert_eq!(count(&neutral, "track volume"), 0);
    assert_eq!(count(&neutral, "track pan"), 0);

    // Two placements of the same pattern on the **one** track, so a per-placement diagnostic
    // reports twice and a per-track one reports once. `four_note_song` alone cannot tell the
    // two apart: it holds a single placement, and an earlier version of this test claimed
    // otherwise — the mutation that reports per placement passed against it.
    let mut song = four_note_song();
    let track_id = song.tracks().next().expect("the fixture has one track").id;
    let pattern_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id;
    assert!(
        song.place_pattern(pattern_id, track_id, synth_sequencer::Tick(7_680)),
        "the second placement must be accepted for this test to mean anything"
    );
    assert_eq!(song.arrangement().len(), 2);
    {
        let track = song.track_mut(track_id).expect("the track resolves");
        track.volume = synth_core::NormalizedValue::new(0.5);
        track.pan = synth_core::BipolarValue::new(-0.5);
    }
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );

    // Two placements sit on that one track, so a per-placement diagnostic reports it twice.
    // It is a property of the track.
    assert_eq!(
        count(&rendered, "track volume"),
        1,
        "a track fader is reported once for the track: {:?}",
        rendered.diagnostics
    );
    assert_eq!(count(&rendered, "track pan"), 1);
    assert!(
        rendered.diagnostics.iter().any(|d| {
            matches!(d.subject(), ProjectSubject::Track { .. })
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("track volume")
                )
        }),
        "and it names the track rather than the song"
    );
    assert_eq!(
        rendered.fidelity(),
        Fidelity::UnsupportedScope,
        "a render missing V1's fader is not a faithful one"
    );
}

/// Song-level state that changes what V1 plays is refused, and a value V1 rounds away is not.
///
/// Each of these was found by an independent review or by the persisted-field pin above, and
/// each is the same shape: something outside the voice patch that V1 acts on and V2 has no
/// place for.
#[test]
fn song_level_state_is_refused_rather_than_ignored() {
    use synth_sequencer::{
        AutomationTarget, MacroNode, ModConnection, ModGraphScope, ModNodeConfig, ModNodeId,
        ModTarget, ModulationAmount, TrackParam,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules.clone(), connections.clone());
    let global = crate::project::GlobalProjectState::default();
    let refused_for = |song: &synth_sequencer::Song, needle: &str| {
        let rendered = super::render::smoke_render(
            &saved,
            song,
            &global,
            harness_profile(),
            FrameCount::new(4_800),
        );
        let named = rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains(needle)
                )
        });
        assert!(
            named && rendered.samples.is_empty(),
            "{needle} must be refused by name and produce no audio: {:?}",
            rendered.diagnostics
        );
    };

    let renders = |song: &synth_sequencer::Song, why: &str| {
        let rendered = super::render::smoke_render(
            &saved,
            song,
            &global,
            harness_profile(),
            FrameCount::new(4_800),
        );
        assert!(
            rendered.is_audible(),
            "{why}, so it must render: {:?}",
            rendered.diagnostics
        );
    };

    // A Mod Grid graph modulates track and instrument controls while the song plays. V1's
    // offline renderer installs its runtime; since `P07-S003` the shapes V2 carries lower to
    // edges and the rest are refused by name (`the_mod_grid_shapes_v2_does_not_carry_are_refused_by_name`).
    // Whether V1 *runs* a graph is decided by its own builder: a graph with no routing sink
    // builds no instance, and neither does a track-scoped graph assigned to no track. An
    // independent review found the check refusing on the pool being non-empty, which blocked
    // a project holding a freshly created, still-empty graph — one V1 plays unchanged.
    let mut song = four_note_song();
    let track_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .track_id;
    let graph_id = song.create_mod_graph("wobble");
    renders(&song, "an empty Mod Grid graph builds no instance in V1");

    let route = |graph: &mut synth_sequencer::ModGraph| {
        graph
            .try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Macro(MacroNode {
                    name: "depth".into(),
                    value: 1.0.into(),
                }),
            )
            .expect("a macro node inserts");
        graph
            .try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Track {
                        track: Some(track_id),
                        param: TrackParam::Volume,
                    },
                    amount: ModulationAmount::new(1.0),
                    combine: Default::default(),
                }),
            )
            .expect("a target node inserts");
        graph
            .try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(1),
                "in",
            ))
            .expect("the cable connects");
    };
    // Routed but track-scoped and assigned to no track: V1 builds no instance for it either.
    {
        let graph = song.mod_graph_mut(graph_id).expect("the graph resolves");
        route(graph);
        graph.scope = ModGraphScope::Track;
    }
    renders(
        &song,
        "a track-scoped Mod Grid graph assigned to no track runs nowhere in V1",
    );
    // Assigned, it runs, and a track-scoped instance is refused as such. Global scope runs
    // unconditionally, and its track target — settled before the macro that feeds it is
    // read — is what names the refusal there.
    song.mod_graph_mut(graph_id)
        .expect("the graph resolves")
        .assigned_tracks
        .push(track_id);
    refused_for(&song, "track-scoped Mod Grid graph");
    {
        let graph = song.mod_graph_mut(graph_id).expect("the graph resolves");
        graph.assigned_tracks.clear();
        graph.scope = ModGraphScope::Global;
    }
    refused_for(&song, "track's volume, pan or pitch");

    // A note-processor rack expands the notes a pattern plays, exactly as an ornament does.
    // The per-note refusal cannot see it, because the rack lives on the pattern.
    let mut song = four_note_song();
    let pattern_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id;
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    refused_for(&song, "note-processor rack");

    // But only where V1 runs it. V1 expands a pattern under `if audible`, for a placement it
    // is walking: a rack on a pattern the arrangement never places, or placed only on a muted
    // track, expands nothing V1 plays. An independent review found the check refusing every
    // pattern in the song.
    let mut song = four_note_song();
    let unplaced = song.create_pattern(synth_sequencer::Duration(960));
    song.pattern_mut(unplaced)
        .expect("the pattern resolves")
        .add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    renders(
        &song,
        "a rack on a pattern the arrangement never places expands nothing",
    );
    let muted = song.create_track("muted");
    assert!(song.place_pattern(unplaced, muted, synth_sequencer::Tick(3_840)));
    for track in song.tracks_mut() {
        if track.id == muted {
            track.mute = true;
        }
    }
    renders(&song, "a rack placed only on a muted track expands nothing");
    // And on a zero-length pattern, which `pattern_tick_at` never resolves a tick inside, so V1
    // never expands it.
    let zero = song.create_pattern(synth_sequencer::Duration(0));
    song.pattern_mut(zero)
        .expect("the pattern resolves")
        .add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    assert!(song.place_pattern(zero, track_id, synth_sequencer::Tick(7_680)));
    renders(&song, "a rack on a zero-length pattern is never expanded");
    // Even under a length override, which is refused for an active pattern: `pattern_tick_at`
    // resolves no tick inside a zero-length pattern however long its placement is, so the
    // override changes nothing V1 plays. The zero-length skip therefore precedes the override
    // refusal; an independent review found them the other way round.
    assert!(song.set_placement_length(
        zero,
        track_id,
        synth_sequencer::Tick(7_680),
        Some(synth_sequencer::Duration(1_920)),
    ));
    renders(
        &song,
        "a zero-length pattern under a length override is still never expanded",
    );

    // **Where the rule stops**, pinned so it is a decision rather than an oversight. A rack on
    // an audible placement whose pattern holds no note computes nothing in V1, but V1 installs
    // and runs it; the contract refuses a stage V1 installs with something to act with, and
    // does not evaluate what it computes — as a master effect at neutral settings is refused
    // rather than measured. Refined further, this class has no floor.
    let mut song = four_note_song();
    let empty = song.create_pattern(synth_sequencer::Duration(960));
    song.pattern_mut(empty)
        .expect("the pattern resolves")
        .add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    assert!(song.place_pattern(empty, track_id, synth_sequencer::Tick(3_840)));
    refused_for(&song, "note-processor rack");

    // A Note Grid graph is the rack's successor and is resolved the way V1 resolves it: a
    // pooled graph nothing binds is inert; a pattern bound to one with **no nodes** is the
    // pass-through V1 makes of it, and bound to one with a node is refused; a **dangling**
    // binding is pass-through in V1's expansion and so is neutral here; and a note-scope
    // binding on one note is refused through that note — including a note past the pattern's
    // end, which V1 never plays but whose graph it still seeds on every active tick, so a
    // source-independent generator there emits. An independent review found the note-scope
    // check sitting after the hidden-note skip, where that note never reached it.
    let mut song = four_note_song();
    let graph_id = song.create_note_graph("triad");
    renders(
        &song,
        "a pooled Note Grid graph nothing binds is never expanded",
    );
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .set_note_graph(Some(graph_id));
    renders(
        &song,
        "a bound Note Grid graph with no nodes expands to its seeded source",
    );
    // And it shadows the rack: a resolved graph is the arm V1 takes, and the rack is the other
    // arm, so a rack under a node-less graph never runs. An independent review found the rack
    // refused underneath it.
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    renders(
        &song,
        "a rack under a bound node-less graph is the arm V1 does not take",
    );
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .clear_processors();
    let chord = || {
        synth_sequencer::NoteModuleConfig::Processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ))
    };
    song.note_graph_mut(graph_id)
        .expect("the graph resolves")
        .try_insert_node(synth_sequencer::NoteModuleId::new(0), chord())
        .expect("a node inserts");
    refused_for(&song, "bound to a Note Grid graph");
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .set_note_graph(Some(synth_sequencer::NoteGraphId::new(99)));
    renders(&song, "a dangling pattern binding is pass-through in V1");
    let mut song = four_note_song();
    let graph_id = song.create_note_graph("triad");
    song.note_graph_mut(graph_id)
        .expect("the graph resolves")
        .try_insert_node(synth_sequencer::NoteModuleId::new(0), chord())
        .expect("a node inserts");
    let hidden = song
        .pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .add_note(
            synth_sequencer::PatternTick(3_840),
            synth_sequencer::Pitch::new(60).expect("middle C is a valid pitch"),
            synth_sequencer::Velocity::new(0.5),
        );
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .note_mut(hidden)
        .expect("the note resolves")
        .note_graph = Some(graph_id);
    refused_for(&song, "note-scope Note Grid graph");

    // A length override clips its pattern's later onsets or repeats it for further passes, so
    // lowering the source notes once would sound a stream V1 never plays. It was *reported*
    // until an independent review pointed out that it changes the note set.
    let mut song = four_note_song();
    let placement = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .clone();
    assert!(song.set_placement_length(
        placement.pattern_id,
        placement.track_id,
        placement.start,
        Some(synth_sequencer::Duration(1_920)),
    ));
    refused_for(&song, "length override");

    // And the other direction: V1's `MidiNote::transpose` rounds, so a saved instrument
    // transpose of 0.4 moves no note. Refusing it would reject a project V1 plays exactly as an
    // untransposed one.
    let mut rounded = saved_instrument(modules, connections);
    rounded.transpose = synth_core::Semitones::new(0.4);
    let rendered = super::render::smoke_render(
        &rounded,
        &four_note_song(),
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.is_audible(),
        "V1 rounds a 0.4-semitone transpose to nothing, so it must render: {:?}",
        rendered.diagnostics
    );
    // Half a semitone does move a note, and must still be refused.
    let mut moved = rounded;
    moved.transpose = synth_core::Semitones::new(0.6);
    let rendered = super::render::smoke_render(
        &moved,
        &four_note_song(),
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(rendered.samples.is_empty(), "0.6 rounds to 1 and moves it");
}

/// Every persisted field of the song types this lowerer reads has a disposition, and a new one
/// fails here.
///
/// # Why this exists beside the destructurings
///
/// `InstrumentState`, `SequencerTrack` and `GlobalProjectState` are dispositioned by exhaustive
/// destructuring, so a new field on any of them is a compile error. `Song`, `Pattern`, `Note`
/// and `PatternPlacement` live in `synth_sequencer` and expose their contents through
/// accessors, so they cannot be destructured from here at all. This is the same guarantee by another route: the field list
/// each type *persists* is pinned, and a change to it fails this test with the disposition
/// question attached rather than becoming a silent difference in a render.
///
/// Taken from the JSON schema rather than from a serialized default, because
/// `skip_serializing_if` hides an empty collection — and an empty collection is exactly the
/// shape a new field arrives in.
#[test]
fn every_persisted_song_field_has_a_disposition() {
    fn fields<T: schemars::JsonSchema>() -> Vec<String> {
        let schema = serde_json::to_value(schemars::schema_for!(T)).expect("the schema is JSON");
        let mut names: Vec<String> = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("a struct schema has properties")
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    // Each list is the persisted surface this lowerer has dispositioned. Adding a name here is
    // the *second* half of the work: the first is deciding, at the site named beside it, whether
    // the field is represented, refused, reported, or never audible — and saying why.
    assert_eq!(
        fields::<synth_sequencer::PatternPlacement>(),
        [
            // `gain` reported and `transpose` applied in `performance::note_spans`;
            // `length_override` and `loop_mode` refused there; the rest are addressing.
            "gain",
            "length_override",
            "loop_mode",
            "pattern_id",
            "start",
            "track_id",
            "transpose",
        ],
        "a placement field changed: disposition it in `performance::note_spans`"
    );
    assert_eq!(
        fields::<synth_sequencer::SequencerTrack>(),
        [
            // Dispositioned by exhaustive destructuring in `performance::track_dispositions`;
            // pinned here too so a *persisted* rename is caught beside the compile error.
            "color",
            "description",
            "id",
            "instrument",
            "mode",
            "mute",
            "name",
            "pan",
            "sends",
            "solo",
            "volume",
        ],
        "a track field changed: disposition it in `performance::track_dispositions`"
    );
    assert_eq!(
        fields::<synth_sequencer::Pattern>(),
        [
            // `automation` lowered in `performance::active_lanes` over every placement, when a
            // lane holds a point: an instrument lane on a module parameter becomes override
            // writes (`P07-S002b`), every other class is refused there by name; `processors`
            // and `note_graph` refused in
            // `performance::note_spans` on a placement V1 plays — the first expands notes
            // exactly as an ornament does, the second is its successor and is resolved through
            // the pool as V1 resolves it; `length` bounds which notes are hidden; `notes` are
            // lowered; `next_note_id` is an id allocator and `description`, `color` are metadata.
            "automation",
            "description",
            "id",
            "length",
            "name",
            "next_note_id",
            "note_graph",
            "notes",
            "processors",
        ],
        "a pattern field changed: disposition it in `performance::note_spans`"
    );
    assert_eq!(
        fields::<synth_sequencer::Note>(),
        [
            // `start`, `duration`, `pitch` and `velocity` are lowered in `performance::note_spans`,
            // the last two revalidated as typed magnitudes, and `duration: None` is refused
            // there; `expression`, `ornament` and a `note_graph` that resolves in the pool are
            // refused there because V1 expands them before playing; `legato` and `glide` are
            // reported; `id` is the subject of every note diagnostic; `lane` is a tracker
            // column, which the sequencer never reads; `track` is vestigial — the placement's
            // track is the sole source of the instrument, and `make_pending_note` never reads
            // the note's own.
            "duration",
            "expression",
            "glide",
            "id",
            "lane",
            "legato",
            "note_graph",
            "ornament",
            "pitch",
            "start",
            "track",
            "velocity",
        ],
        "a note field changed: disposition it in `performance::note_spans`"
    );
    assert_eq!(
        fields::<synth_sequencer::Song>(),
        [
            // `arrangement`, `tracks` and `patterns` are the pinned types above, walked in
            // `performance::note_spans`; `tempo_changes` and `default_tempo` are lowered by
            // `performance::lower_tempo`; `mod_graphs` are refused in
            // `render::project_diagnostics` when V1's own builder makes an instance of one;
            // `note_graphs` are the pool a pattern or note binding resolves through, refused at
            // the binding; `return_busses` carry audio only through a send, and an enabled send
            // at a non-zero level is refused with the bus's effects; `transport_loop` is the
            // saved loop region, which neither `audio::export` nor `audio::arrangement_render`
            // reads; `sections` extend the song's end through `Song::calculate_length`, which
            // `performance` reads to clip a release and bound the render exactly where V1's
            // auto-stop does; `time_signature_changes`, `default_time_signature` and
            // `row_resolution` are grid metadata no playback path reads; every `next_*_id` is
            // an id allocator; `name`, `author` and `description` are metadata.
            "arrangement",
            "author",
            "default_tempo",
            "default_time_signature",
            "description",
            "mod_graphs",
            "name",
            "next_mod_graph_id",
            "next_note_graph_id",
            "next_pattern_id",
            "next_return_bus_id",
            "next_section_id",
            "next_track_id",
            "note_graphs",
            "patterns",
            "return_busses",
            "row_resolution",
            "sections",
            "tempo_changes",
            "time_signature_changes",
            "tracks",
            "transport_loop",
        ],
        "a song field changed: disposition it where it acts, and record it here"
    );
}

/// Every persisted name under `ProjectFile` — every type the project format reaches, nested
/// or not — is pinned, so a change anywhere in the format fails here and asks for a
/// disposition.
///
/// # Why a third pin beside the typed ones
///
/// The typed pins above and the exhaustive destructures cover the types the lowerer reads
/// **directly**, with a disposition per field. They do not reach the types those fields hold:
/// a field added to `TempoChange`, `AutomationLane`, `TrackSend`, `ModGraph` or `NoteGraph`
/// changes none of their lists, so it would arrive silently. An independent review found that
/// the specification claimed the set was closed while it was not. This pin closes it the only
/// way a claim like that can be closed — by asking about every name the format persists — and
/// it carries no disposition of its own: the register names the type and field, and the
/// disposition lives at the site that reads the owner, which the register's comments point to.
///
/// The names come from the live project schema, generated with the `SchemaSettings`
/// `gen_schemas` uses, and walked for every `properties` object under each definition, which
/// is what reaches a struct variant's fields inside an enum. The generator rather than the
/// committed `schemas/project.schema.json`, because that file is post-processed —
/// `tighten_module_state` rewrites a module parameter's schema — and the register pins the
/// types as the lowerer sees them. A failure prints the added and removed names rather than
/// both whole lists.
#[test]
fn every_persisted_project_name_is_registered() {
    use std::collections::BTreeSet;

    fn collect(node: &serde_json::Value, prefix: &str, into: &mut BTreeSet<String>) {
        match node {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::Object(properties)) = map.get("properties") {
                    for field in properties.keys() {
                        into.insert(format!("{prefix}.{field}"));
                    }
                }
                // Every value, the property values included: an externally tagged enum
                // variant with named fields is a property whose value carries its own
                // `properties`, and a walk that stopped at the keys never reached them. An
                // independent review found `AutomationTarget::Instrument`'s fields missing.
                for value in map.values() {
                    collect(value, prefix, into);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, prefix, into);
                }
            }
            _ => {}
        }
    }

    let mut generator =
        schemars::SchemaGenerator::new(schemars::generate::SchemaSettings::draft2020_12());
    let schema = serde_json::to_value(generator.root_schema_for::<crate::project::ProjectFile>())
        .expect("the schema is JSON");
    let mut actual = BTreeSet::new();
    if let Some(serde_json::Value::Object(properties)) = schema.get("properties") {
        for field in properties.keys() {
            actual.insert(format!("ProjectFile.{field}"));
        }
    }
    let definitions = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .expect("the schema has definitions");
    for (name, definition) in definitions {
        collect(definition, name, &mut actual);
    }

    let registered: BTreeSet<String> = include_str!("persisted_fields.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let added: Vec<&String> = actual.difference(&registered).collect();
    let removed: Vec<&String> = registered.difference(&actual).collect();
    assert!(
        added.is_empty() && removed.is_empty(),
        "the persisted project format changed. Added: {added:?}. Removed: {removed:?}. \
         Decide where the lowerer reads each owner and disposition it there, then update \
         `lowering/persisted_fields.txt`"
    );
}

/// Every saved instrument setting `offline_instrument_settings` measures as reaching V1's
/// renderer is represented, refused or reported here.
///
/// That test file is the evidence: it changes one field at a time and asserts the rendered
/// bytes change. A field it measures as audible and this lowerer says nothing about is a silent
/// difference, which is the hole class five reviews of this phase kept finding.
#[test]
fn every_audible_instrument_setting_is_dispositioned() {
    let (modules, connections) = corpus_patch("sawtooth");
    let song = four_note_song();
    let global = crate::project::GlobalProjectState::default();

    let render = |adjust: &dyn Fn(&mut crate::patch::InstrumentState)| {
        let mut saved = saved_instrument(modules.clone(), connections.clone());
        adjust(&mut saved);
        super::render::smoke_render(
            &saved,
            &song,
            &global,
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let says = |rendered: &super::render::SmokeRender, needle: &str| {
        rendered.diagnostics.iter().any(|d| {
            matches!(
                d.reason(),
                LoweringReason::OwnedByLaterPhase { capability, .. }
                    if capability.contains(needle)
            )
        })
    };

    // Oversampling changes the anti-aliasing of everything the voice does. Reported, because
    // the notes are unchanged.
    let over = render(&|i| i.oversampling = 4);
    assert!(
        says(&over, "oversampling") && over.is_audible(),
        "oversampling must be reported without stopping the render: {:?}",
        over.diagnostics
    );
    // V1 reads `1 | 2 | 4` and sends everything else to `X1`, so a saved `3` is neutral to V1.
    let odd = render(&|i| i.oversampling = 3);
    assert!(
        !says(&odd, "oversampling"),
        "a saved 3 is X1 to V1 and must be neutral here: {:?}",
        odd.diagnostics
    );

    // Voice allocation decides what V1 does when a release rings under the next note, and
    // unison lives under the mode rather than beside it.
    let allocator_changes: [&dyn Fn(&mut crate::patch::InstrumentState); 3] = [
        &|i| {
            i.allocation_mode = synth_engine::voice_allocator::AllocationMode::Unison;
        },
        &|i| {
            i.max_voices = synth_core::VoiceCount::new(1);
        },
        &|i| {
            i.stealing_strategy = synth_engine::voice_allocator::StealingStrategy::Quietest;
        },
    ];
    for adjust in allocator_changes {
        let rendered = render(adjust);
        assert!(
            says(&rendered, "voice-allocation setting") && rendered.samples.is_empty(),
            "an allocator setting must be refused: {:?}",
            rendered.diagnostics
        );
    }

    // A sidechain source ducks this instrument on what another one plays.
    let ducked = render(&|i| i.sidechain_source_id = Some(1));
    assert!(
        says(&ducked, "sidechain source") && ducked.samples.is_empty(),
        "a sidechain source must be refused: {:?}",
        ducked.diagnostics
    );

    // The control: the fixture's own settings say none of the above.
    let neutral = render(&|_| {});
    for needle in [
        "oversampling",
        "voice-allocation setting",
        "sidechain source",
    ] {
        assert!(
            !says(&neutral, needle),
            "a default instrument must not raise {needle}: {:?}",
            neutral.diagnostics
        );
    }
}

/// A placed pattern carrying automation V2 does not lower — a track lane here — is refused,
/// because V1 applies it. The instrument lanes `P07-S002b` lowers are covered below.
#[test]
fn pattern_automation_is_refused_rather_than_flattened() {
    use synth_sequencer::{
        AutomationLane, AutomationPoint, AutomationTarget, PatternTick, TrackParam,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let mut song = four_note_song();
    let pattern_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id;

    // A lane with no points first. `AutomationLane::value_at` returns `None` for it, so V1's
    // sequencer emits nothing: it is a lane the user opened and never drew in. An independent
    // review found the check reading the lane list's length, which refused this project.
    let mut lane = AutomationLane::new(AutomationTarget::Track {
        track: None,
        param: TrackParam::Volume,
    });
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .automation
        .push(lane.clone());
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.is_audible(),
        "a lane with no points emits nothing in V1, so it must render: {:?}",
        rendered.diagnostics
    );

    // One point makes it automation.
    lane.add_point(AutomationPoint::new(
        PatternTick(0),
        synth_core::NormalizedValue::new(0.5),
    ));
    song.pattern_mut(pattern_id)
        .expect("the pattern resolves")
        .add_automation_lane(lane);

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("automation")
                )
        }),
        "a pattern carrying automation must be refused by name: {:?}",
        rendered.diagnostics
    );
    assert!(rendered.samples.is_empty());

    // And on a placement this lowering's note walk **skips**. V1 executes a pattern's
    // automation whether or not that track's notes are audible, and a lane can target another
    // track, an instrument, a module parameter or a global control. An independent review found
    // this check sitting after the track filter, where a muted track's automation never reached
    // it; the check is now a song-level pass, and this is what falsifies moving it back.
    for track in song.tracks_mut() {
        track.mute = true;
    }
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("automation")
                )
        }),
        "a muted track's automation still runs in V1, so it must still be refused: {:?}",
        rendered.diagnostics
    );

    // A zero-length pattern is never active — `pattern_tick_at` resolves no tick inside it —
    // so V1 never reads its lanes. Placed beside the audible pattern, it must not refuse the
    // lowering; an independent review found that it did.
    let mut song = four_note_song();
    let track_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .track_id;
    let zero = song.create_pattern(synth_sequencer::Duration(0));
    let mut lane = AutomationLane::new(AutomationTarget::Track {
        track: None,
        param: TrackParam::Volume,
    });
    lane.add_point(AutomationPoint::new(
        PatternTick(0),
        synth_core::NormalizedValue::new(0.5),
    ));
    song.pattern_mut(zero)
        .expect("the pattern resolves")
        .add_automation_lane(lane);
    assert!(song.place_pattern(zero, track_id, synth_sequencer::Tick(3_840)));
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.is_audible(),
        "a zero-length pattern's lanes are never read in V1, so it must render: {:?}",
        rendered.diagnostics
    );
}

/// A master chain is refused, and the master volume is reported.
#[test]
fn project_global_state_is_read_rather_than_ignored() {
    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let song = four_note_song();

    // The default master volume is 0.8, which V2's output does not apply.
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.subject() == &ProjectSubject::Project
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("master volume")
                )
        }),
        "a master volume other than unity changes what V1 renders"
    );

    // A master effect is audible processing on everything, and V2 has no master bus.
    let mut global = crate::project::GlobalProjectState::default();
    global
        .master_effects
        .push(module("cmp-1", ModuleType::Compressor));
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.subject() == &ProjectSubject::MasterChain && d.severity() == Severity::Refused
        }),
        "a master chain must be refused, not silently absent: {:?}",
        rendered.diagnostics
    );
}

/// No file loading reaches V2, through the lowerer or inside the crate itself.
///
/// The work list requires that "samples and other assets" arrive as already-prepared immutable
/// data and that "no file loading reaches V2". The second half is the checkable one, and this
/// is the check: no production source in **either** tree that can reach the renderer — the
/// lowering module that consumes V2, and `synth_engine_v2` itself — opens, reads, or names a
/// loader.
///
/// The first half is vacuous today and says so rather than claiming a guarantee: V2's node
/// registry has no sampler, so there is no asset for the lowerer to prepare. It becomes real
/// with ADR-0026's zone model, and this test will not notice that on its own.
///
/// # What this establishes, and what it does not
///
/// It is a scan for spellings, and it claims no more than `crate_boundary`'s equivalent does: a
/// scan for a grammar fails open, one spelling at a time. Two earlier revisions failed open in
/// ways an independent review found rather than a determined author would have had to
/// engineer — it read only the immediate directory, so a nested module was invisible, and it
/// matched `::load(` while the repository's own project loader is `::load_file(`. Both are
/// closed below; the class is not.
///
/// What is stronger and lives elsewhere: `synth_engine_v2`'s manifest allows `synth_core` and
/// `thiserror` and nothing else, which `crate_boundary` checks by asking Cargo. That bounds
/// which *crates* it can reach; this bounds what its own source does with the standard library.
#[test]
fn no_file_loading_reaches_v2() {
    const FORBIDDEN: [&str; 11] = [
        "std::fs",
        "fs::read",
        "fs::write",
        "File::open",
        "File::create",
        "read_to_string",
        "BufReader",
        "OpenOptions",
        "::load",
        "load_file",
        "PathBuf",
    ];

    /// Every `.rs` under `root`, recursively. An earlier revision read one directory and
    /// therefore could not see a nested module.
    fn sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        out
    }

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut scanned = 0_usize;
    let mut offenders: Vec<String> = Vec::new();
    for root in [
        manifest.join("src/lowering"),
        manifest.join("../synth_engine_v2/src"),
    ] {
        for path in sources(&root) {
            let relative = path
                .strip_prefix(manifest)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // Test sources are the exception, and they have to be: the survey that establishes
            // `P04-R002` loads every pinned corpus project from disk.
            if relative.contains("/tests/") || relative.ends_with("tests.rs") {
                continue;
            }
            scanned += 1;

            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
            for spelling in FORBIDDEN {
                if source.contains(spelling) {
                    offenders.push(format!("{relative} names {spelling}"));
                }
            }
        }
    }

    assert!(
        scanned >= 25,
        "the scan must cover both production trees; it read only {scanned} files"
    );
    assert!(
        offenders.is_empty(),
        "the lowerer takes already-deserialized values and prepared assets, and V2 reads no \
         file of its own, so no file loading reaches V2: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// The note's own magnitudes reach the render
// ---------------------------------------------------------------------------

/// A one-note song at a chosen pitch and velocity, with an optional placement transpose.
fn one_note_song(midi: u8, velocity: f32, transpose: f32) -> synth_sequencer::Song {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern_id = song.create_pattern(Duration(3840));
    let track_id = song.create_track("track");
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(midi).expect("the fixture's pitch is a keyboard position"),
            Velocity::new(velocity),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(1_920));
        }
    }
    if transpose == 0.0 {
        assert!(song.place_pattern(pattern_id, track_id, Tick::ZERO));
    } else {
        assert!(
            song.insert_placement(
                synth_sequencer::PatternPlacement::new(pattern_id, track_id, Tick::ZERO)
                    .with_transpose(synth_core::Semitones::new(transpose))
            )
        );
    }
    song
}

/// Render one such song through the lowerer.
///
/// The velocity goes through `Velocity::new`, which clamps — and `NaN.clamp(0, 1)` is `NaN`, so
/// a non-finite value survives it while an out-of-range one does not. That asymmetry is what
/// `a_saved_velocity_that_is_not_a_number_is_refused` turns on.
fn render_one_note(midi: u8, velocity: f32, transpose: f32) -> super::render::SmokeRender {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    super::render::smoke_render(
        &saved,
        &one_note_song(midi, velocity, transpose),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    )
}

/// How many times the render crosses zero, as a stand-in for its pitch.
fn crossings(rendered: &super::render::SmokeRender) -> usize {
    rendered
        .samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}

/// The loudest sample in the render.
fn peak(rendered: &super::render::SmokeRender) -> f32 {
    rendered
        .samples
        .iter()
        .fold(0.0_f32, |held, sample| held.max(sample.abs()))
}

#[test]
fn a_saved_notes_own_pitch_reaches_the_render() {
    // The half of `P04-R001` that is about pitch. Two songs differing in **one** field, so a
    // lowerer that sent a constant key — which is exactly what it did until this slice — would
    // render them identically.
    let low = render_one_note(48, 0.8, 0.0);
    let high = render_one_note(60, 0.8, 0.0);
    assert!(low.is_audible() && high.is_audible(), "both have to sound");

    // An octave, so twice the crossings. The ratio rather than mere inequality, because two
    // notes differing in *any* way would satisfy an inequality.
    let ratio = crossings(&high) as f32 / crossings(&low) as f32;
    assert!(
        (ratio - 2.0).abs() < 0.05,
        "MIDI 48 rendered {} crossings and MIDI 60 rendered {}, a ratio of {ratio}",
        crossings(&low),
        crossings(&high)
    );
}

#[test]
fn a_saved_notes_own_velocity_reaches_the_render() {
    // The other half. The fixture's instrument has an amp sensitivity of zero, so only the
    // envelope's sensitivity — V1's default, one — scales the note: half the velocity is half
    // the peak, a stronger claim than "they differ" and what distinguishes velocity reaching
    // the amplitude from velocity reaching anything at all.
    let loud = render_one_note(60, 1.0, 0.0);
    let soft = render_one_note(60, 0.5, 0.0);
    assert!(loud.is_audible() && soft.is_audible(), "both have to sound");

    let ratio = peak(&soft) / peak(&loud);
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "half the velocity rendered a peak ratio of {ratio}, from {} against {}",
        peak(&soft),
        peak(&loud)
    );
}

#[test]
fn the_envelopes_own_sensitivity_lowers_from_vel_sens() {
    // ADR-0059 clause 5, the other saved sensitivity: the envelope module's `vel_sens` reaches
    // the envelope's own control. At zero the envelope ignores the velocity, and with the
    // fixture's amp sensitivity also zero, half the velocity is the **same** peak — a lowerer
    // sending V1's default of one instead would render half.
    let (mut modules, connections) = corpus_patch("sine");
    let env = modules
        .iter_mut()
        .find(|m| m.id == "env-1")
        .expect("the corpus patch has its envelope");
    floats(env, &[("vel_sens", 0.0)]);
    let saved = saved_instrument(modules, connections);
    let render = |velocity: f32| {
        super::render::smoke_render(
            &saved,
            &one_note_song(60, velocity, 0.0),
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let loud = render(1.0);
    let soft = render(0.5);
    assert!(loud.is_audible() && soft.is_audible());
    let ratio = peak(&soft) / peak(&loud);
    assert!(
        (ratio - 1.0).abs() < 0.02,
        "an envelope ignoring the velocity rendered a peak ratio of {ratio}"
    );
}

#[test]
fn v1s_two_sensitivities_compose_to_the_velocity_squared() {
    // ADR-0059 end to end through the lowerer: with the instrument's amp sensitivity at V1's
    // default of one beside the envelope's, half the velocity is a **quarter** of the peak —
    // V1's product — where a single scale would give half.
    let (modules, connections) = corpus_patch("sine");
    let mut saved = saved_instrument(modules, connections);
    saved.velocity_amp_sensitivity = synth_core::NormalizedValue::MAX;
    let render = |velocity: f32| {
        super::render::smoke_render(
            &saved,
            &one_note_song(60, velocity, 0.0),
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let loud = render(1.0);
    let soft = render(0.5);
    assert!(loud.is_audible() && soft.is_audible());
    assert!(!names_the_composition(&loud), "{:?}", loud.diagnostics);
    let ratio = peak(&soft) / peak(&loud);
    assert!(
        (ratio - 0.25).abs() < 0.02,
        "half the velocity under two sensitivities rendered a peak ratio of {ratio}"
    );
}

#[test]
fn a_placement_transpose_moves_the_notes_it_places() {
    // A placement transpose used to be reported as unrepresented, because the payload could
    // not carry a key at all. Now it is **applied**: a lowerer that ignored it would render
    // the authored pitch and sound the wrong music with nothing saying so.
    let plain = render_one_note(48, 0.8, 0.0);
    let up_an_octave = render_one_note(48, 0.8, 12.0);
    assert!(plain.is_audible() && up_an_octave.is_audible());

    let ratio = crossings(&up_an_octave) as f32 / crossings(&plain) as f32;
    assert!(
        (ratio - 2.0).abs() < 0.05,
        "a placement transposed by an octave rendered a crossing ratio of {ratio}"
    );

    // And it is the same render as authoring the transposed pitch directly, which is what
    // makes it a transpose rather than merely a change.
    assert_eq!(
        crossings(&up_an_octave),
        crossings(&render_one_note(60, 0.8, 0.0)),
        "transposing MIDI 48 by an octave must render what MIDI 60 renders"
    );
}

#[test]
fn a_note_transposed_off_the_keyboard_keeps_its_authored_pitch_as_v1_does() {
    // V1 does **not** drop such a note: `sequencer_engine::make_pending_note` writes
    // `.transpose(transpose).unwrap_or(expanded.pitch)` and plays the authored pitch. An
    // earlier revision of this lowerer refused the whole performance on the belief that V1
    // dropped it, which an independent review read the V1 site and refuted — and which would
    // also have suppressed every unrelated note in the arrangement.
    let off_the_end = render_one_note(120, 0.8, 12.0);
    assert!(
        off_the_end.is_audible(),
        "the note still sounds, at its authored pitch, got {:?}",
        off_the_end.diagnostics
    );
    assert_eq!(
        crossings(&off_the_end),
        crossings(&render_one_note(120, 0.8, 0.0)),
        "a transpose that leaves the keyboard falls back to the authored pitch, so the two \
         renders are the same note"
    );
}

#[test]
fn a_placement_transpose_that_is_not_a_keyboard_offset_is_refused() {
    // `Semitones` is a transparent `f32` with a derived `Deserialize`, so a persisted `1e40`
    // arrives as an infinity. `Pitch::transpose` rounds it, saturates the cast to `i16::MAX`
    // and adds it to a pitch, which **overflows and panics** in a checked build — so it is
    // refused before the arithmetic rather than after. An independent review found it.
    for offset in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1e30, -1e30] {
        let rendered = render_one_note(60, 0.8, offset);
        assert!(
            rendered.diagnostics.iter().any(|d| {
                d.severity() == Severity::Refused
                    && matches!(d.subject(), ProjectSubject::Track { .. })
                    && matches!(
                        d.reason(),
                        LoweringReason::UnsupportedParameterValue { value }
                            // The **offending value**, not merely its class: a diagnostic
                            // that named the fault without the number would leave a reader
                            // to find which placement carried it.
                            if value.contains("not a keyboard offset")
                                && value.contains(&format!("{offset}"))
                    )
            }),
            "a transpose of {offset} must be refused by name and by value, got {:?}",
            rendered.diagnostics
        );
        assert!(
            rendered.samples.is_empty(),
            "and nothing renders from a placement that cannot be read"
        );
    }
}

#[test]
fn a_saved_velocity_that_is_not_a_number_is_refused() {
    // The one out-of-domain persisted velocity this boundary can actually catch, and the
    // reason it can is worth stating: `synth_core::Velocity` clamps at deserialization, so a
    // saved `2.0` arrives here as `1.0` and is already a different magnitude before this
    // module sees it. `f32::clamp` returns `NaN` unchanged, so that one survives — and it
    // would otherwise multiply every sample an envelope emits for the rest of the render.
    let rendered = render_one_note(60, f32::NAN, 0.0);
    assert!(
        rendered.samples.is_empty(),
        "a note whose velocity is not a number must not render"
    );
    assert!(
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Refused
                && matches!(d.subject(), ProjectSubject::Note { .. })
                && matches!(
                    d.reason(),
                    LoweringReason::UnsupportedParameterValue { value }
                        if value.contains("finite")
                )
        }),
        "the refusal must name the note and why, got {:?}",
        rendered.diagnostics
    );

    // And the clamped case is **not** caught here, which is the half that is easy to assume
    // and wrong: an out-of-range saved velocity has already become `1.0`, so it renders.
    let clamped = render_one_note(60, 2.0, 0.0);
    assert!(
        clamped.is_audible(),
        "an out-of-range saved velocity is clamped by the project format's own type, not by \
         this boundary, so it still renders: {:?}",
        clamped.diagnostics
    );
}

#[test]
fn an_instrument_diagnostic_names_the_instrument_rather_than_the_song() {
    // `ProjectSubject::Instrument::name` documents itself as the instrument's. An earlier
    // revision passed the **song's** name, so a diagnostic about an instrument named the
    // project; an independent review found it. The two names differ here on purpose.
    // Two notes through the one gate raise an instrument-level diagnostic until Phase 6's
    // allocator lowers them, which is what this test now reads it from.
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let mut song = four_note_song();
    song.name = "A Song By Another Name".to_owned();

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    let named: Vec<&str> = rendered
        .diagnostics
        .iter()
        .filter_map(|d| match d.subject() {
            ProjectSubject::Instrument { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !named.is_empty(),
        "the lowering has to raise an instrument diagnostic for this to check anything"
    );
    assert!(
        named.iter().all(|name| *name == saved.name),
        "an instrument diagnostic must carry the instrument's name, got {named:?} against \
         instrument {:?} and song {:?}",
        saved.name,
        song.name
    );
}

#[test]
fn a_placed_note_names_no_velocity_gap_and_still_refuses_a_parity_verdict() {
    // The reporting half's velocity clause, closed by `P06-S004` under ADR-0059: V1 applies
    // one saved velocity twice — at the envelope and again at the voice output — and V2 now
    // applies both, each with its saved sensitivity, so a placed note no longer names the
    // composition as unrepresented. The outcome is **still** `UnsupportedScope` and still
    // refuses a parity verdict, and rightly: Phase 8's stages — the master volume and the
    // pans — remain named, and `LOWER-INV-003` waits for the *last* unrepresented
    // capability, not for this one. An independent read caught an earlier name for this test
    // that claimed the verdict was admitted.
    let rendered = render_one_note(60, 0.8, 0.0);
    assert!(rendered.is_audible());
    assert_eq!(rendered.fidelity(), Fidelity::UnsupportedScope);
    assert!(
        !rendered.fidelity().admits_parity_comparison(),
        "Phase 8's marks still refuse the verdict"
    );
    assert!(
        !names_the_composition(&rendered),
        "nothing names the velocity composition as unrepresented, got {:?}",
        rendered.diagnostics
    );
    assert!(
        rendered.diagnostics.iter().all(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { owner, .. } if owner == &"Phase 8"
        )),
        "and what remains is Phase 8's alone, got {:?}",
        rendered.diagnostics
    );

    // And with four notes through the one gate, the same holds: nothing names the velocity
    // composition, however many notes the project places.
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let four = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(48_000),
    );
    assert!(!names_the_composition(&four), "{:?}", four.diagnostics);
}

/// A cable spelled `amp-01` reaches the module declared `amp-1`, because that is how V1
/// resolves it: `ModuleId` parses the instance as a number, so the two spellings are one
/// identity. Keying the topology tables by spelling called the amplifier unpatched and a
/// respelled repeat a second cable; a squash review found both.
#[test]
fn a_cable_spelled_with_a_leading_zero_resolves_as_v1_does() {
    let (modules, mut connections) = corpus_patch("sine");
    for c in &mut connections {
        if c.to.0 == "amp-1" {
            c.to.0 = "amp-01".to_owned();
        }
    }
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_some(),
        "V1 resolves `amp-01` to `amp-1`, so its control is patched: {:?}",
        lowered.diagnostics
    );

    // A repeat of the control cable spelled the other way is the cable V1 already has.
    let (modules, mut connections) = corpus_patch("sine");
    let mut repeat = connections
        .iter()
        .find(|c| c.to.0 == "amp-1" && c.to.1 == "cv")
        .expect("the corpus patch drives the amplifier")
        .clone();
    repeat.to.0 = "amp-01".to_owned();
    connections.push(repeat);
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    assert!(
        lowered.ir.is_some(),
        "a respelled repeat is one cable in V1, not fan-in: {:?}",
        lowered.diagnostics
    );
}

/// An absent choice lowers as the choice V1's own descriptor declares, not as a literal.
///
/// The declared default is read from the descriptor here too, so this test does not know
/// which waveform it is: it asserts that omitting the key renders **exactly** what naming the
/// declared default renders, and not what naming the other supported waveform renders.
#[test]
fn an_absent_choice_lowers_as_the_descriptor_declares() {
    let (_, descriptor) =
        crate::module_factory::create_voice_module(ModuleType::Oscillator).expect("V1 has one");
    let waveform = descriptor
        .parameters
        .iter()
        .find(|p| p.type_id == "waveform")
        .expect("the oscillator declares a waveform");
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let declared = waveform
        .choices
        .as_ref()
        .and_then(|choices| choices.get(waveform.range.default.round().max(0.0) as usize))
        .map(|c| c.id.clone())
        .expect("the waveform declares a default choice");
    let other = if declared == "sine" {
        "sawtooth"
    } else {
        "sine"
    };

    let render = |waveform: Option<&str>| {
        let (mut modules, connections) = corpus_patch("sine");
        for m in &mut modules {
            if m.module_type == ModuleType::Oscillator {
                match waveform {
                    Some(value) => choice(m, "waveform", value),
                    None => {
                        m.parameters.remove("waveform");
                    }
                }
            }
        }
        super::render::smoke_render(
            &saved_instrument(modules, connections),
            &four_note_song(),
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let absent = render(None);
    assert!(absent.is_audible(), "{:?}", absent.diagnostics);
    assert_eq!(
        absent.samples,
        render(Some(&declared)).samples,
        "an absent waveform must render as the descriptor's `{declared}`"
    );
    assert_ne!(
        absent.samples,
        render(Some(other)).samples,
        "and not as `{other}`"
    );
}

/// A note held past the song's end is released where V1's auto-stop releases it, and a
/// section drawn past the last placement extends the render as it extends V1's.
///
/// Both bounds are `Song::calculate_length`, read rather than recomputed. A squash review found
/// the authored release used and the frame count stopping at the last release.
#[test]
fn the_render_is_bounded_by_the_song_end_as_v1_bounds_it() {
    use synth_sequencer::{Duration, SectionKind, Tick};

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let render = |song: &synth_sequencer::Song| {
        super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let with_last_duration = |duration: u32| {
        let mut song = four_note_song();
        let pattern_id = song
            .arrangement()
            .first()
            .expect("the fixture places one pattern")
            .pattern_id;
        let pattern = song.pattern_mut(pattern_id).expect("the pattern resolves");
        let last = pattern.notes().last().expect("the fixture has notes").id;
        pattern.note_mut(last).expect("the note resolves").duration = Some(Duration(duration));
        song
    };

    // The last note starts at 2880 in a 3840-tick pattern. Held for 960 it ends at the song's
    // end; held for 3000 it would end at 5880, past where V1 has already stopped.
    let exact = render(&with_last_duration(960));
    let held = render(&with_last_duration(3000));
    assert!(exact.is_audible() && held.is_audible());
    assert_eq!(
        held.lowered_frames, exact.lowered_frames,
        "a release past the song's end lands where V1 auto-stops"
    );
    assert_eq!(held.lowered_events, exact.lowered_events);

    // A section reaching to twice the arrangement's end is silence V1 renders.
    let mut song = with_last_duration(960);
    let _outro = song.create_section("outro", SectionKind::default(), Tick(3840), Duration(3840));
    let extended = render(&song);
    assert_eq!(
        extended.lowered_frames.as_u64(),
        exact.lowered_frames.as_u64() * 2,
        "a section past the last placement extends the render to its end"
    );
}

/// The declared event peak is the worst case over every anchor phase, as admission counts it.
///
/// Admission slides a `Q`-frame window rather than bucketing by absolute quantum, because which
/// quantum a frame belongs to depends on where the stream is anchored. Two edges 25 frames
/// apart that straddle an absolute 64-frame boundary are one quantum's load after an ordinary
/// seek, so the declaration must say two; a bucketed count said one, and admission would have
/// accepted the plan and refused its stream. The squash review found the bucketing.
#[test]
fn the_declared_event_peak_slides_a_window_as_admission_does() {
    use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

    // At 120 BPM and 48 kHz one tick is 25 frames, so a one-tick note at tick 2 puts its two
    // edges at frames 50 and 75: different absolute 64-frame quanta, one sliding window.
    let mut song = synth_sequencer::Song::default();
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern_id = song.create_pattern(Duration(3840));
    let track_id = song.create_track("track");
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        let id = pattern.add_note(
            PatternTick(2),
            Pitch::new(60).expect("middle C"),
            Velocity::new(0.5),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(1));
        }
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick::ZERO));
    assert_eq!(
        synth_engine_v2::time::QUANTUM_FRAMES,
        64,
        "the fixture assumes Q = 64"
    );

    let (modules, _) = corpus_patch("sawtooth");
    let peak = super::performance::peak_events_per_quantum(
        instrument(),
        &modules,
        &song,
        synth_engine_v2::quantities::SampleRate::new(48_000.0).expect("a real rate"),
    );
    assert_eq!(
        peak,
        Some(synth_engine_v2::quantities::EventCount::measured(2)),
        "both edges fall in one 64-frame window once the anchor shifts"
    );
}

/// A tempo ramp toward a later change is marked unrepresented, because the two engines ramp
/// different quantities: V1 the tempo number, V2 the beat's period. Every event after such a
/// ramp lands elsewhere, which ADR-0049 accepts as a semantic change that must map to a
/// comparison category. A ramp with nothing after it, and a step, are not marked.
#[test]
fn a_tempo_ramp_toward_a_later_change_is_marked_unrepresented() {
    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let says_ramp = |song: &synth_sequencer::Song| {
        let rendered = super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        );
        assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
        rendered.diagnostics.iter().any(|d| {
            d.severity() == Severity::Unrepresented
                && matches!(
                    d.reason(),
                    LoweringReason::OwnedByLaterPhase { capability, .. }
                        if capability.contains("tempo ramp")
                )
        })
    };

    assert!(
        !says_ramp(&double_tempo_song()),
        "a step lands every event where V1 lands it"
    );

    let mut song = four_note_song();
    song.set_tempo_ramp_at(synth_sequencer::Tick(0), synth_core::Bpm::new(120.0), true);
    assert!(
        !says_ramp(&song),
        "a ramp with no later change ramps toward nothing in both engines"
    );

    song.set_tempo_at(synth_sequencer::Tick(1920), synth_core::Bpm::new(180.0));
    assert!(
        says_ramp(&song),
        "a ramp toward a later change moves every event after it in V2"
    );
}

/// A placement whose `length_override` is zero is as inactive as a zero-length pattern, so
/// its automation is never read and its rack never expanded; V1's `pattern_tick_at` resolves
/// no tick in either. The squash review found the source pattern's length read instead.
#[test]
fn a_zero_length_override_is_as_inactive_as_a_zero_length_pattern() {
    use synth_sequencer::{
        AutomationLane, AutomationPoint, AutomationTarget, Duration, PatternTick, Tick, TrackParam,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let mut song = four_note_song();
    let track_id = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .track_id;
    let overridden = song.create_pattern(Duration(960));
    let mut lane = AutomationLane::new(AutomationTarget::Track {
        track: None,
        param: TrackParam::Volume,
    });
    lane.add_point(AutomationPoint::new(
        PatternTick(0),
        synth_core::NormalizedValue::new(0.5),
    ));
    {
        let pattern = song.pattern_mut(overridden).expect("the pattern resolves");
        pattern.add_automation_lane(lane);
        pattern.add_processor(synth_sequencer::NoteProcessor::Chord(
            synth_sequencer::Chord::default(),
        ));
    }
    assert!(song.place_pattern(overridden, track_id, Tick(3_840)));
    assert!(song.set_placement_length(overridden, track_id, Tick(3_840), Some(Duration(0))));

    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(
        rendered.is_audible(),
        "a zero-length override is never active in V1, so it must render: {:?}",
        rendered.diagnostics
    );
}

// ---------------------------------------------------------------------------------------------
// `P07-S002b`: a placed pattern's automation lanes as override writes.
// ---------------------------------------------------------------------------------------------

/// An instrument lane for the fixture instrument, from `(tick, value, curve)` points.
fn instrument_lane(
    param: synth_sequencer::AutoInstrumentParam,
    points: &[(u32, f32, synth_sequencer::CurveType)],
) -> synth_sequencer::AutomationLane {
    use synth_sequencer::{AutomationLane, AutomationPoint, AutomationTarget, PatternTick};
    let mut lane = AutomationLane::new(AutomationTarget::Instrument {
        instrument: instrument(),
        param,
    });
    for (tick, value, curve) in points {
        lane.add_point(
            AutomationPoint::new(PatternTick(*tick), synth_core::NormalizedValue::new(*value))
                .with_curve(*curve),
        );
    }
    lane
}

/// The fixture song's one placed pattern.
fn placed_pattern(song: &synth_sequencer::Song) -> synth_sequencer::PatternId {
    song.arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id
}

/// V1's own denormalization of a lane value for one instrument parameter, through the
/// descriptor of the module type V1 resolves it on.
fn v1_denormalized(param: synth_sequencer::AutoInstrumentParam, normalized: f32) -> f32 {
    let (kind, _, key) =
        crate::mod_grid_build::instrument_param_module(param).expect("a module parameter");
    let (_, declarations) =
        crate::module_factory::create_voice_module(kind).expect("V1 builds the module");
    declarations
        .find_parameter(key)
        .expect("V1 declares the parameter")
        .denormalize(normalized)
}

/// The `SetParameter` events among a lowering's, as `(frame, slot, value)`.
fn parameter_writes(
    events: &[synth_engine_v2::offline::OfflineEvent],
) -> Vec<(u64, synth_engine_v2::plan::ParameterSlot, f32)> {
    use synth_engine_v2::schedule::CompiledPayload;
    events
        .iter()
        .filter_map(|event| match event.payload() {
            CompiledPayload::SetParameter { slot, value } => {
                Some((event.time().as_u64(), slot, value.as_f32()))
            }
            _ => None,
        })
        .collect()
}

/// Lower the fixture patch against `song` directly, returning the plan and the performance.
fn lowered_performance(
    modules: &[ModuleState],
    connections: &[ConnectionState],
    song: &synth_sequencer::Song,
) -> (
    synth_engine_v2::plan::CompiledPlan,
    super::performance::LoweredPerformance,
) {
    lowered_performance_at(
        modules,
        connections,
        song,
        SampleRate::new(48_000.0).expect("a real rate"),
    )
}

/// [`lowered_performance`] with the arrangement mapped at `rate`.
fn lowered_performance_at(
    modules: &[ModuleState],
    connections: &[ConnectionState],
    song: &synth_sequencer::Song,
    rate: SampleRate,
) -> (
    synth_engine_v2::plan::CompiledPlan,
    super::performance::LoweredPerformance,
) {
    use synth_engine_v2::quantities::NormalizedLevel;
    // `None` is a refusal the performance lowering will name, and the render path declares
    // no events for it exactly as `smoke_render` does.
    let peak = super::performance::peak_events_per_quantum(instrument(), modules, song, rate)
        .unwrap_or(synth_engine_v2::quantities::EventCount::NONE);
    let lowered = super::graph::lower_voice_patch_with(
        instrument(),
        modules,
        connections,
        peak,
        Some(NormalizedLevel::new(0.0).expect("a level")),
        // The channel `smoke_render` inserts for `saved_instrument`'s strip (`P08-S001`):
        // unity, centre, unmuted. Without it the oracle would be the smoke render's samples
        // less V1's centre coefficient.
        Some(super::graph::ChannelStrip {
            fader: synth_engine_v2::quantities::Amplitude::UNITY,
            pan: synth_engine_v2::controller::BipolarLevel::ZERO,
            muted: false,
        }),
        &super::modulation::SongModulators::default(),
    );
    let ir = lowered.ir.expect("the fixture lowers");
    let plan = compile(&ir, &RenderConfig::new(harness_profile()))
        .into_plan()
        .expect("the fixture compiles");
    let gate = lowered
        .identities
        .pairs()
        .find(|(id, _)| id.module_type == ModuleType::Envelope)
        .map(|(_, node)| node)
        .expect("the fixture has an envelope");
    let targets = super::performance::AutomationTargets::resolve(&lowered.identities);
    let performance = super::performance::lower_performance(
        instrument(),
        "Subtractive Voice",
        song,
        &plan,
        gate,
        &targets,
        rate,
    );
    (plan, performance)
}

/// A step lane lowers to exactly the writes V1 emits — one per point, in the control's unit
/// through V1's own descriptor — plus one restoring write of the prepared base where V1's
/// transport stops; and the render is bit-identical to the same plan given those writes by
/// hand.
#[test]
fn a_step_lane_lowers_to_v1s_emissions_and_a_restore_at_the_songs_end() {
    use synth_engine_v2::offline::{OfflineEvent, render_offline};
    use synth_engine_v2::schedule::CompiledPayload;
    use synth_engine_v2::time::{PlanPosition, SampleTime};
    use synth_sequencer::{AutoInstrumentParam, CurveType};

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules.clone(), connections.clone());
    let plain = four_note_song();
    let mut song = plain.clone();
    let pattern = placed_pattern(&song);
    song.pattern_mut(pattern)
        .expect("the pattern resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.2, CurveType::Step), (1920, 0.8, CurveType::Step)],
        ));

    // The lowering's own events: eight note edges, two lane writes, one restore.
    let (plan, performance) = lowered_performance(&modules, &connections, &song);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    let writes = parameter_writes(&performance.events);
    assert_eq!(writes.len(), 3, "two emissions and a restore: {writes:?}");
    assert_eq!(performance.events.len(), 11);
    let slot = writes[0].1;
    assert!(
        writes.iter().all(|(_, s, _)| *s == slot),
        "one target, one slot: {writes:?}"
    );
    // At 120 BPM and 48 kHz one tick is 25 frames: tick 1920 is frame 48 000, the song's end
    // at tick 3840 is frame 96 000.
    let expected = |normalized: f32| v1_denormalized(AutoInstrumentParam::FilterCutoff, normalized);
    assert_eq!(writes[0], (0, slot, expected(0.2)));
    assert_eq!(writes[1], (48_000, slot, expected(0.8)));
    let base = plan.parameter_targets()[slot.index()].base.as_f32();
    assert_eq!(base, 1200.0, "the prepared base is the authored cutoff");
    assert_eq!(
        writes[2],
        (96_000, slot, base),
        "the restore writes the base"
    );
    assert!(
        expected(0.2) != base && expected(0.8) != base,
        "the fixture's lane must move the corner away from its authored value"
    );

    // The render is exactly the plan rendered with those writes placed by hand beside the
    // lane-less song's note edges.
    let rendered = super::render::smoke_render(
        &saved,
        &song,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(11)
    );
    // The note edges from the same lowering — a slot names its plan, so the writes have to be
    // placed against this plan rather than against a second compile's.
    let notes: Vec<OfflineEvent> = performance
        .events
        .iter()
        .copied()
        .filter(|event| !matches!(event.payload(), CompiledPayload::SetParameter { .. }))
        .collect();
    assert_eq!(notes.len(), 8);
    let value = |hz: f32| {
        synth_engine_v2::quantities::ParameterValue::from_cutoff(
            synth_engine_v2::quantities::CutoffFrequency::new(hz).expect("a corner"),
        )
    };
    let mut by_hand = notes;
    by_hand.push(OfflineEvent::new(
        SampleTime::new(0),
        CompiledPayload::SetParameter {
            slot,
            value: value(expected(0.2)),
        },
    ));
    by_hand.push(OfflineEvent::new(
        SampleTime::new(48_000),
        CompiledPayload::SetParameter {
            slot,
            value: value(expected(0.8)),
        },
    ));
    by_hand.push(OfflineEvent::new(
        SampleTime::new(96_000),
        CompiledPayload::SetParameter {
            slot,
            value: value(base),
        },
    ));
    by_hand.sort_by_key(OfflineEvent::time);
    let frames = FrameCount::new(performance.frames.as_u64() + 4_800);
    let oracle =
        render_offline(plan, frames, PlanPosition::ZERO, &by_hand).expect("the oracle renders");
    assert_eq!(rendered.samples.len(), oracle.len());
    assert!(
        rendered.samples == oracle,
        "the lowered lane must render exactly as the hand-placed writes"
    );

    // And the lane changes the sound: the control that the writes reach the filter.
    let unautomated = super::render::smoke_render(
        &saved,
        &plain,
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(unautomated.samples != rendered.samples);
}

/// A linear lane emits V1's staircase: one write each time the value has moved by more than
/// the sequencer's threshold, each the descriptor's denormalization of `value_at` at its tick.
#[test]
fn a_linear_lane_emits_v1s_staircase_through_v1s_curve() {
    use synth_sequencer::{AutoInstrumentParam, CurveType, PatternTick};

    let (modules, connections) = corpus_patch("sawtooth");
    let mut song = four_note_song();
    let pattern = placed_pattern(&song);
    let lane = instrument_lane(
        AutoInstrumentParam::FilterCutoff,
        &[(0, 0.0, CurveType::Linear), (3840, 1.0, CurveType::Linear)],
    );
    song.pattern_mut(pattern)
        .expect("the pattern resolves")
        .add_automation_lane(lane.clone());

    let (_, performance) = lowered_performance(&modules, &connections, &song);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    let writes = parameter_writes(&performance.events);
    // One in 3840 per tick moves past 0.001 every fourth tick: ticks 0, 4, ..., 3836.
    assert_eq!(
        writes.len(),
        960 + 1,
        "960 emissions and the restore: {}",
        writes.len()
    );
    let (kind, _, key) =
        crate::mod_grid_build::instrument_param_module(AutoInstrumentParam::FilterCutoff)
            .expect("a module parameter");
    let (_, declarations) = crate::module_factory::create_voice_module(kind).expect("builds");
    let descriptor = declarations.find_parameter(key).expect("declared");
    let mut last: Option<f32> = None;
    for (frame, _, hz) in &writes[..960] {
        assert_eq!(frame % 25, 0, "a write lands on a tick");
        let tick = u32::try_from(frame / 25).expect("fits");
        assert_eq!(
            tick % 4,
            0,
            "V1 emits every fourth tick of this ramp, not at {tick}"
        );
        let at_tick = lane.value_at(PatternTick(tick)).expect("a pointed lane");
        assert_eq!(*hz, descriptor.denormalize(at_tick.as_f32()));
        let normalized = descriptor.normalize(*hz);
        if let Some(previous) = last {
            assert!(
                (normalized - previous).abs()
                    > synth_engine::sequencer_engine::AUTOMATION_DEDUP_THRESHOLD,
                "an emission moves past the threshold"
            );
        }
        last = Some(normalized);
    }
    assert_eq!(writes[960].0, 96_000, "the restore is at the song's end");

    // The threshold's own boundary: a move of exactly the threshold is **not** a change in V1
    // (`> threshold`, not `>=`), and `0.001 - 0.0` is exactly the constant in `f32`.
    let mut song = four_note_song();
    let pattern = placed_pattern(&song);
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.0, CurveType::Step), (1920, 0.001, CurveType::Step)],
        ));
    let (_, performance) = lowered_performance(&modules, &connections, &song);
    let writes = parameter_writes(&performance.events);
    assert_eq!(
        writes.len(),
        1 + 1,
        "a move of exactly the threshold emits nothing in V1: {writes:?}"
    );
}

/// Two lanes writing one target at one sample are refused, naming both patterns and the
/// target; the same two lanes placed end to end lower, as do two lanes on two targets.
#[test]
fn two_writers_on_one_target_at_one_sample_are_refused_by_name() {
    use synth_sequencer::{AutoInstrumentParam, CurveType, Duration, Tick};

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let cutoff = |value: f32| {
        instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, value, CurveType::Step)],
        )
    };
    let render = |song: &synth_sequencer::Song| {
        super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let conflict = |rendered: &super::render::SmokeRender| {
        rendered.diagnostics.iter().find_map(|d| match d.reason() {
            LoweringReason::ConflictingWriters {
                target,
                first,
                second,
            } if d.severity() == Severity::Refused => {
                Some((target.clone(), *first, *second, d.subject().clone()))
            }
            _ => None,
        })
    };

    // A second, note-free pattern with its own cutoff lane, placed on another track so the
    // note walk has nothing to refuse first. Overlapping the first from tick 1920.
    let mut song = four_note_song();
    let first = placed_pattern(&song);
    song.pattern_mut(first)
        .expect("resolves")
        .add_automation_lane(cutoff(0.3));
    let second = song.create_pattern(Duration(3840));
    song.pattern_mut(second)
        .expect("resolves")
        .add_automation_lane(cutoff(0.9));
    let other_track = song.create_track("automation");
    assert!(song.place_pattern(second, other_track, Tick(1920)));
    let rendered = render(&song);
    assert_eq!(
        conflict(&rendered),
        Some((
            "Filter Cutoff".to_owned(),
            first,
            second,
            ProjectSubject::Pattern {
                pattern: second,
                name: String::new(),
            }
        )),
        "{:?}",
        rendered.diagnostics
    );
    assert!(rendered.samples.is_empty());

    // End to end — the second starts where the first ends — both lower, and the second's
    // first tick emits because its value differs from where the first left off.
    let mut adjacent = four_note_song();
    adjacent
        .pattern_mut(first)
        .expect("resolves")
        .add_automation_lane(cutoff(0.3));
    let second = adjacent.create_pattern(Duration(3840));
    adjacent
        .pattern_mut(second)
        .expect("resolves")
        .add_automation_lane(cutoff(0.9));
    let other_track = adjacent.create_track("automation");
    assert!(adjacent.place_pattern(second, other_track, Tick(3840)));
    let rendered = render(&adjacent);
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8 + 2 + 1),
        "eight edges, one write per lane, one restore"
    );

    // A `Clip` placement drawn longer than its pattern is silent past the pattern, and its
    // lane with it: a lane starting where the pattern ends shares no tick with it. The
    // carrier sits on another track, where a length override is not the note walk's to
    // refuse.
    let mut clipped = four_note_song();
    let carrier = clipped.create_pattern(Duration(1920));
    clipped
        .pattern_mut(carrier)
        .expect("resolves")
        .add_automation_lane(cutoff(0.3));
    let follower = clipped.create_pattern(Duration(1920));
    clipped
        .pattern_mut(follower)
        .expect("resolves")
        .add_automation_lane(cutoff(0.9));
    let carrier_track = clipped.create_track("clip");
    clipped
        .track_mut(carrier_track)
        .expect("resolves")
        .instrument = synth_engine::instrument::InstrumentId::new(7);
    assert!(clipped.place_pattern(carrier, carrier_track, Tick::ZERO));
    assert!(clipped.set_placement_length(carrier, carrier_track, Tick::ZERO, Some(Duration(3840))));
    assert!(clipped.set_placement_loop_mode(
        carrier,
        carrier_track,
        Tick::ZERO,
        synth_sequencer::PlacementLoopMode::Clip
    ));
    let follower_track = clipped.create_track("follower");
    assert!(clipped.place_pattern(follower, follower_track, Tick(1920)));
    let rendered = render(&clipped);
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8 + 2 + 1),
        "the clip's lane ends with its pattern, so the follower is its successor"
    );

    // Two targets at once are two slots, not two writers.
    let mut two_targets = four_note_song();
    two_targets
        .pattern_mut(first)
        .expect("resolves")
        .add_automation_lane(cutoff(0.3));
    two_targets
        .pattern_mut(first)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterResonance,
            &[(0, 0.9, CurveType::Step)],
        ));
    let rendered = render(&two_targets);
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8 + 2 + 2)
    );

    // Two lanes on one target inside one pattern — the field is a list, so a file can carry
    // them even though the editor replaces by target — share every tick.
    let mut doubled = four_note_song();
    doubled
        .pattern_mut(first)
        .expect("resolves")
        .automation
        .push(cutoff(0.3));
    doubled
        .pattern_mut(first)
        .expect("resolves")
        .automation
        .push(cutoff(0.9));
    let rendered = render(&doubled);
    assert_eq!(
        conflict(&rendered).map(|(target, a, b, _)| (target, a, b)),
        Some(("Filter Cutoff".to_owned(), first, first)),
        "{:?}",
        rendered.diagnostics
    );
}

/// V1 runs a lane wherever its placement is: on a muted track, and on a track routed to
/// another instrument. A lane naming another instrument is that instrument's, and is skipped
/// exactly as its notes are.
#[test]
fn a_lane_runs_where_v1_runs_it_and_another_instruments_lane_is_skipped() {
    use synth_sequencer::{
        AutoInstrumentParam, AutomationLane, AutomationPoint, AutomationTarget, CurveType,
        Duration, PatternTick, Tick,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let render = |song: &synth_sequencer::Song| {
        super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let plain = render(&four_note_song());
    assert!(plain.is_audible(), "{:?}", plain.diagnostics);

    // A note-free pattern carrying this instrument's cutoff lane, on a muted track routed to
    // another instrument: V1 still runs it, so the render changes and the writes are counted.
    let mut song = four_note_song();
    let carrier = song.create_pattern(Duration(3840));
    song.pattern_mut(carrier)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 1.0, CurveType::Step)],
        ));
    let track = song.create_track("muted, elsewhere");
    {
        let track = song.track_mut(track).expect("resolves");
        track.mute = true;
        track.instrument = synth_engine::instrument::InstrumentId::new(7);
    }
    assert!(song.place_pattern(carrier, track, Tick::ZERO));
    let rendered = render(&song);
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8 + 1 + 1)
    );
    assert!(
        rendered.samples != plain.samples,
        "a muted track's lane still writes"
    );

    // The same lane naming another instrument is not this render's.
    let mut song = four_note_song();
    let pattern = placed_pattern(&song);
    let mut lane = AutomationLane::new(AutomationTarget::Instrument {
        instrument: synth_engine::instrument::InstrumentId::new(7),
        param: AutoInstrumentParam::FilterCutoff,
    });
    lane.add_point(AutomationPoint::new(
        PatternTick(0),
        synth_core::NormalizedValue::new(1.0),
    ));
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(lane);
    let rendered = render(&song);
    assert!(
        rendered.diagnostics == plain.diagnostics,
        "{:?}",
        rendered.diagnostics
    );
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8)
    );
    assert!(rendered.samples == plain.samples);
}

/// A lane on a module the patch does not hold is V1's no-op — `apply_normalized_override`
/// returns when it finds no module of the type — so it lowers to nothing and changes nothing.
#[test]
fn a_lane_on_a_module_the_patch_lacks_is_v1s_no_op() {
    use synth_sequencer::{AutoInstrumentParam, CurveType};

    // The corpus patch with its filter removed and the oscillator cabled straight in.
    let (modules, _) = corpus_patch("sawtooth");
    let modules: Vec<ModuleState> = modules
        .into_iter()
        .filter(|module| module.module_type != ModuleType::Filter)
        .collect();
    let connection = |from: (&str, &str), to: (&str, &str)| ConnectionState {
        from: (from.0.to_owned(), from.1.to_owned()),
        to: (to.0.to_owned(), to.1.to_owned()),
    };
    let connections = vec![
        connection(("env-1", "out"), ("amp-1", "cv")),
        connection(("amp-1", "out"), ("out-1", "in")),
        connection(("osc-1", "out"), ("amp-1", "in")),
    ];
    let saved = saved_instrument(modules.clone(), connections.clone());
    let render = |song: &synth_sequencer::Song| {
        super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let plain = four_note_song();
    let unautomated = render(&plain);
    assert!(unautomated.is_audible(), "{:?}", unautomated.diagnostics);

    let mut song = plain.clone();
    let pattern = placed_pattern(&song);
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 1.0, CurveType::Step)],
        ));
    let rendered = render(&song);
    assert_eq!(rendered.diagnostics, unautomated.diagnostics);
    assert_eq!(
        rendered.lowered_events,
        synth_engine_v2::quantities::EventCount::measured(8),
        "no module, no write, no restore"
    );
    assert!(rendered.samples == unautomated.samples);

    // Two inert lanes overlapping conflict over nothing: `apply_normalized_override` returns
    // before either writes, so neither is a writer. An independent read found them refused.
    let mut two_inert = plain.clone();
    // Shorter than the host and placed at its start, so the song's end does not move.
    let carrier = two_inert.create_pattern(synth_sequencer::Duration(960));
    two_inert
        .pattern_mut(carrier)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.3, CurveType::Step)],
        ));
    two_inert
        .pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.9, CurveType::Step)],
        ));
    let track = two_inert.create_track("carrier");
    assert!(two_inert.place_pattern(carrier, track, synth_sequencer::Tick::ZERO));
    let rendered = render(&two_inert);
    assert_eq!(rendered.diagnostics, unautomated.diagnostics);
    assert!(rendered.samples == unautomated.samples);

    // And the peak agrees, from the saved modules alone.
    let (_, performance) = lowered_performance(&modules, &connections, &song);
    assert_eq!(performance.events.len(), 8);
    let peak = super::performance::peak_events_per_quantum(
        instrument(),
        &modules,
        &song,
        SampleRate::new(48_000.0).expect("a real rate"),
    );
    assert_eq!(
        peak,
        Some(synth_engine_v2::quantities::EventCount::measured(1)),
        "the lane counts nothing where it lands nowhere"
    );
}

/// Every one of the six parameters reaches its control: a lane at one extreme renders
/// differently from the unautomated fixture, with exactly one write and one restore.
#[test]
fn each_instrument_parameter_lane_reaches_its_control() {
    use synth_sequencer::{AutoInstrumentParam, CurveType};

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let render = |song: &synth_sequencer::Song| {
        super::render::smoke_render(
            &saved,
            song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(48_000),
        )
    };
    let plain = render(&four_note_song());
    assert!(plain.is_audible(), "{:?}", plain.diagnostics);

    for (param, value) in [
        (AutoInstrumentParam::FilterCutoff, 1.0),
        (AutoInstrumentParam::FilterResonance, 1.0),
        (AutoInstrumentParam::Attack, 1.0),
        (AutoInstrumentParam::Decay, 1.0),
        (AutoInstrumentParam::Sustain, 0.0),
        (AutoInstrumentParam::Release, 1.0),
    ] {
        let mut song = four_note_song();
        let pattern = placed_pattern(&song);
        song.pattern_mut(pattern)
            .expect("resolves")
            .add_automation_lane(instrument_lane(param, &[(0, value, CurveType::Step)]));
        let rendered = render(&song);
        assert!(
            rendered.is_audible(),
            "{param:?}: {:?}",
            rendered.diagnostics
        );
        assert_eq!(
            rendered.diagnostics, plain.diagnostics,
            "{param:?}: a lowered lane is represented, not marked"
        );
        assert_eq!(
            rendered.lowered_events,
            synth_engine_v2::quantities::EventCount::measured(8 + 1 + 1),
            "{param:?}"
        );
        assert!(
            rendered.samples != plain.samples,
            "{param:?} at {value} must change the render"
        );
    }
}

/// The lane classes V2 does not carry are refused by name, each with its owner.
#[test]
fn the_lane_classes_v2_does_not_carry_are_refused_by_name() {
    use synth_sequencer::{
        AutoInstrumentParam, AutomationLane, AutomationPoint, AutomationTarget, GlobalParam,
        PatternTick, TrackParam,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules, connections);
    let refusal = |target: AutomationTarget| {
        let mut song = four_note_song();
        let pattern = placed_pattern(&song);
        let mut lane = AutomationLane::new(target);
        lane.add_point(AutomationPoint::new(
            PatternTick(0),
            synth_core::NormalizedValue::new(0.5),
        ));
        song.pattern_mut(pattern)
            .expect("resolves")
            .add_automation_lane(lane);
        let rendered = super::render::smoke_render(
            &saved,
            &song,
            &crate::project::GlobalProjectState::default(),
            harness_profile(),
            FrameCount::new(4_800),
        );
        assert!(rendered.samples.is_empty());
        rendered
            .diagnostics
            .into_iter()
            .find_map(|d| match (d.severity(), d.reason()) {
                (Severity::Refused, LoweringReason::OwnedByLaterPhase { capability, owner })
                    if *d.subject()
                        == (ProjectSubject::Pattern {
                            pattern,
                            name: String::new(),
                        }) =>
                {
                    Some((*capability, *owner))
                }
                _ => None,
            })
            .expect("refused on the pattern by name")
    };
    let this = instrument();
    let (capability, owner) = refusal(AutomationTarget::Instrument {
        instrument: this,
        param: AutoInstrumentParam::Volume,
    });
    assert!(capability.contains("volume or pan") && owner.contains("Phase 8"));
    let (capability, owner) = refusal(AutomationTarget::Instrument {
        instrument: this,
        param: AutoInstrumentParam::Pan,
    });
    assert!(capability.contains("volume or pan") && owner.contains("Phase 8"));
    let (capability, owner) = refusal(AutomationTarget::Module {
        instrument: this,
        module_type: ModuleType::Filter,
        instance: 1,
        param_id: "cutoff".into(),
    });
    assert!(capability.contains("module-addressed") && owner.contains("Phase 7"));
    let (capability, owner) = refusal(AutomationTarget::Track {
        track: None,
        param: TrackParam::Pitch,
    });
    assert!(capability.contains("track pitch") && owner.contains("Phase 7"));
    let (capability, owner) = refusal(AutomationTarget::Track {
        track: None,
        param: TrackParam::Mute,
    });
    assert!(capability.contains("fader, pan or mute") && owner.contains("Phase 8"));
    let (capability, owner) = refusal(AutomationTarget::Global(GlobalParam::MasterVolume));
    assert!(capability.contains("master volume") && owner.contains("Phase 8"));
}

/// The declared peak counts the lane writes admission must be told about: a ramp that emits
/// on every tick puts three writes in one 64-frame window at 25 frames a tick.
#[test]
fn the_declared_event_peak_counts_the_lane_writes() {
    use synth_sequencer::{AutoInstrumentParam, CurveType};

    let (modules, _) = corpus_patch("sawtooth");
    let mut song = song_with(120.0, &[], None);
    let pattern = placed_pattern(&song);
    // One in 96 per tick clears the threshold every tick: writes at frames 0, 25, 50, ...
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.0, CurveType::Linear), (96, 1.0, CurveType::Linear)],
        ));
    let rate = SampleRate::new(48_000.0).expect("a real rate");
    let peak = super::performance::peak_events_per_quantum(instrument(), &modules, &song, rate);
    assert_eq!(
        peak,
        Some(synth_engine_v2::quantities::EventCount::measured(3)),
        "three writes fall in one window"
    );

    // The restoring write is counted where it lands: beside a release at the song's end. One
    // note from tick 100 to the end and one step lane: the on and the lane's first write are
    // 2 500 frames apart, the off and the restore share frame 96 000.
    let mut ending = song_with(120.0, &[(100, 3740)], None);
    let pattern = placed_pattern(&ending);
    ending
        .pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.5, CurveType::Step)],
        ));
    let peak = super::performance::peak_events_per_quantum(instrument(), &modules, &ending, rate);
    assert_eq!(
        peak,
        Some(synth_engine_v2::quantities::EventCount::measured(2)),
        "the release and the restore share the song's last frame"
    );

    // The same lanes with no filter to land on declare nothing.
    let without: Vec<ModuleState> = modules
        .into_iter()
        .filter(|module| module.module_type != ModuleType::Filter)
        .collect();
    let peak = super::performance::peak_events_per_quantum(instrument(), &without, &song, rate);
    assert_eq!(
        peak,
        Some(synth_engine_v2::quantities::EventCount::measured(0))
    );
}

/// A lane lands on the first module of its type as V1 resolves it: the lowest identity, not
/// instance one and not the last declared. Two filters named `flt-3` and `flt-2`, declared in
/// that order, and the write reaches `flt-2`.
#[test]
fn a_lane_lands_on_the_lowest_module_of_its_type_as_v1_resolves_it() {
    use synth_engine_v2::ir::parameters;
    use synth_sequencer::{AutoInstrumentParam, CurveType};

    let (modules, _) = corpus_patch("sawtooth");
    let mut modules: Vec<ModuleState> = modules
        .into_iter()
        .filter(|module| module.module_type != ModuleType::Filter)
        .collect();
    for id in ["flt-3", "flt-2"] {
        let mut flt = module(id, ModuleType::Filter);
        floats(
            &mut flt,
            &[("cutoff", 1200.0), ("env_amt", 0.0), ("resonance", 0.3)],
        );
        choice(&mut flt, "type", "lowpass");
        modules.push(flt);
    }
    let connection = |from: (&str, &str), to: (&str, &str)| ConnectionState {
        from: (from.0.to_owned(), from.1.to_owned()),
        to: (to.0.to_owned(), to.1.to_owned()),
    };
    let connections = vec![
        connection(("env-1", "out"), ("amp-1", "cv")),
        connection(("amp-1", "out"), ("out-1", "in")),
        connection(("osc-1", "out"), ("flt-3", "in")),
        connection(("flt-3", "out"), ("flt-2", "in")),
        connection(("flt-2", "out"), ("amp-1", "in")),
    ];
    let mut song = four_note_song();
    let pattern = placed_pattern(&song);
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.9, CurveType::Step)],
        ));
    let (plan, performance) = lowered_performance(&modules, &connections, &song);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    let identities = ResolvedIdentities::resolve(&modules).expect("resolves");
    let node_of = |id: &str| {
        identities
            .node_for(id.parse().expect("a module id"))
            .expect("declared")
    };
    let lowest = plan
        .resolve_parameter(node_of("flt-2"), parameters::FILTER_CUTOFF)
        .expect("a control");
    let other = plan
        .resolve_parameter(node_of("flt-3"), parameters::FILTER_CUTOFF)
        .expect("a control");
    assert_ne!(lowest, other);
    let writes = parameter_writes(&performance.events);
    assert_eq!(writes.len(), 2);
    assert!(
        writes.iter().all(|(_, slot, _)| *slot == lowest),
        "the lane writes flt-2, V1's first filter by identity: {writes:?}"
    );
}

/// Disjoint ticks are not disjoint samples: at 8 kHz and 1000 BPM a tick is half a frame, so
/// a lane's last emission at tick 1 and its successor's first at tick 2 both round to frame
/// 1 — two absolute writers at one sample, refused naming both. Placed one tick later the
/// successor lands on frame 2 and both lower. An independent read found the collision.
#[test]
fn two_writers_landing_on_one_sample_from_disjoint_ticks_are_refused() {
    use synth_sequencer::{AutoInstrumentParam, CurveType, Duration, Tick};

    let (modules, connections) = corpus_patch("sawtooth");
    let rate = SampleRate::new(8_000.0).expect("a real rate");
    let lowered = |successor_at: u64| {
        let mut song = song_with(1000.0, &[], None);
        let host = placed_pattern(&song);
        // The fixture's own placement carries no lane; the two writers are short patterns
        // on their own tracks.
        let _ = host;
        let first = song.create_pattern(Duration(2));
        song.pattern_mut(first)
            .expect("resolves")
            .add_automation_lane(instrument_lane(
                AutoInstrumentParam::FilterCutoff,
                &[(0, 0.2, CurveType::Step), (1, 0.9, CurveType::Step)],
            ));
        let second = song.create_pattern(Duration(2));
        song.pattern_mut(second)
            .expect("resolves")
            .add_automation_lane(instrument_lane(
                AutoInstrumentParam::FilterCutoff,
                &[(0, 0.3, CurveType::Step)],
            ));
        let a = song.create_track("first");
        let b = song.create_track("second");
        assert!(song.place_pattern(first, a, Tick::ZERO));
        assert!(song.place_pattern(second, b, Tick(successor_at)));
        let (_, performance) = lowered_performance_at(&modules, &connections, &song, rate);
        (first, second, performance)
    };

    let (first, second, performance) = lowered(2);
    assert!(
        performance.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::ConflictingWriters { target, first: a, second: b }
                if target == "Filter Cutoff" && *a == first && *b == second
        )),
        "{:?}",
        performance.diagnostics
    );
    assert!(performance.refused());

    let (_, _, performance) = lowered(3);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    let writes = parameter_writes(&performance.events);
    let positions: Vec<u64> = writes.iter().map(|(frame, ..)| *frame).collect();
    // Frames 0 and 1 from the first lane, 2 from the second, and the restore at the song's
    // end — tick 3840 at half a frame a tick.
    assert_eq!(positions, vec![0, 1, 2, 1_920], "{writes:?}");

    // One lane emitting twice on one frame is one writer, and the later value is in force as
    // it is in V1's block: ticks 1 and 2 of one lane both round to frame 1 and both lower.
    let mut song = song_with(1000.0, &[], None);
    let one = song.create_pattern(Duration(3));
    song.pattern_mut(one)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[
                (0, 0.2, CurveType::Step),
                (1, 0.9, CurveType::Step),
                (2, 0.3, CurveType::Step),
            ],
        ));
    let track = song.create_track("one");
    assert!(song.place_pattern(one, track, Tick::ZERO));
    let (_, performance) = lowered_performance_at(&modules, &connections, &song, rate);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    let writes = parameter_writes(&performance.events);
    let positions: Vec<u64> = writes.iter().map(|(frame, ..)| *frame).collect();
    assert_eq!(positions, vec![0, 1, 1, 1_920], "{writes:?}");
    let expected = v1_denormalized(AutoInstrumentParam::FilterCutoff, 0.3);
    assert_eq!(
        writes[2].2, expected,
        "the later emission is the last on its frame"
    );
}

/// The lane walk is bounded in ticks and refuses past the bound by name, before a tick is
/// walked: a tempo is any finite positive number, so no frame bound bounds it.
#[test]
fn an_automation_walk_past_the_tick_bound_is_refused_by_name() {
    use synth_sequencer::{AutoInstrumentParam, CurveType, Duration, Tick};

    let (modules, connections) = corpus_patch("sawtooth");
    let mut song = song_with(120.0, &[], None);
    let long = song.create_pattern(Duration((1 << 25) + 1));
    song.pattern_mut(long)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.5, CurveType::Step)],
        ));
    let track = song.create_track("long");
    assert!(song.place_pattern(long, track, Tick::ZERO));
    let (_, performance) = lowered_performance(&modules, &connections, &song);
    assert!(
        performance.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, owner }
                if capability.contains("more ticks") && owner.contains("ADR-0028")
        )),
        "{:?}",
        performance.diagnostics
    );
    assert!(performance.refused());

    // One tick shorter is inside the bound and lowers: one write and the restore.
    let mut song = song_with(120.0, &[], None);
    let bounded = song.create_pattern(Duration(1 << 25));
    song.pattern_mut(bounded)
        .expect("resolves")
        .add_automation_lane(instrument_lane(
            AutoInstrumentParam::FilterCutoff,
            &[(0, 0.5, CurveType::Step)],
        ));
    let track = song.create_track("bounded");
    assert!(song.place_pattern(bounded, track, Tick::ZERO));
    let (_, performance) = lowered_performance(&modules, &connections, &song);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    assert_eq!(parameter_writes(&performance.events).len(), 2);
}

// ---------------------------------------------------------------------------
// `P07-S003`: the Mod Matrix and the Mod Grid as modulation edges
// ---------------------------------------------------------------------------

/// The corpus project by name, loaded from the bytes the manifest pins.
fn corpus_project(name: &str) -> crate::project::ProjectFile {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../corpus/v2-reference/projects/{name}.ptz"));
    crate::project::ProjectFile::load(&path)
        .unwrap_or_else(|e| panic!("{} must load: {e}", path.display()))
}

/// A saved LFO with the parameters given, the rest V1's defaults.
fn lfo(id: &str, params: &[(&str, f32)]) -> ModuleState {
    let mut lfo = module(id, ModuleType::Lfo);
    floats(&mut lfo, params);
    lfo
}

/// A saved Mod Matrix with the slots given as `(source, destination, amount)`, all enabled.
fn mod_matrix(id: &str, slots: &[(&str, &str, f32)]) -> ModuleState {
    let mut matrix = module(id, ModuleType::ModMatrix);
    for (index, (source, destination, amount)) in slots.iter().enumerate() {
        let number = index + 1;
        choice(&mut matrix, &format!("slot_{number}_source"), source);
        choice(&mut matrix, &format!("slot_{number}_dest"), destination);
        floats(
            &mut matrix,
            &[
                (&format!("slot_{number}_amount"), *amount),
                (&format!("slot_{number}_enabled"), 1.0),
            ],
        );
    }
    matrix
}

/// The one modulation edge of a lowered graph, as `(source node, target node, parameter,
/// unit, amount)`.
fn edges(
    ir: &synth_engine_v2::ir::GraphIr,
) -> Vec<(
    NodeId,
    NodeId,
    synth_engine_v2::ir::ParameterId,
    synth_engine_v2::ir::ModulationUnit,
    f32,
)> {
    ir.modulations()
        .iter()
        .map(|m| {
            assert_eq!(
                m.source().1,
                synth_engine_v2::ir::PortId::FIRST,
                "every lowered source is the LFO's one output"
            );
            (
                m.source().0,
                m.target().0,
                m.target().1,
                m.depth().unit(),
                m.depth().amount(),
            )
        })
        .collect()
}

/// `CORPUS-0003`: one slot carrying an LFO into the filter's cutoff lowers to one edge whose
/// depth is V1's amount times V1's own cutoff scale, from an LFO node carrying the saved
/// settings; the matrix itself is no node; and the edge reaches the filter in the render.
#[test]
fn the_corpus_mod_matrix_slot_lowers_to_one_edge_at_v1s_scale() {
    use synth_engine_v2::ir::{ExecutionScope, IrNodeKind, LfoPolarity, LfoWaveform, parameters};
    use synth_engine_v2::quantities::{Frequency, NormalizedLevel, PhaseOffset};

    let project = corpus_project("mod-matrix");
    let saved = project
        .instruments
        .first()
        .expect("CORPUS-0003 declares one instrument");
    let lowered = lower_voice_patch(
        saved.id,
        &saved.patch.modules,
        &saved.patch.connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("CORPUS-0003 must lower: {:?}", lowered.diagnostics));
    assert!(
        lowered
            .diagnostics
            .iter()
            .all(|d| d.severity() == Severity::Unrepresented),
        "{:?}",
        lowered.diagnostics
    );
    let matrix = ModuleId::new(ModuleType::ModMatrix, 1);
    // The one thing said about the matrix or the LFO is the edge's timing, marked as the
    // corpus's intentional correction; nothing else about either is unrepresented.
    let about_modulation: Vec<&LoweringDiagnostic> = lowered
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.subject(),
                ProjectSubject::Module { module, .. } | ProjectSubject::Parameter { module, .. }
                    if *module == matrix || module.module_type == ModuleType::Lfo
            )
        })
        .collect();
    assert_eq!(
        about_modulation,
        vec![&LoweringDiagnostic::unrepresented(
            ProjectSubject::Parameter {
                instrument: saved.id,
                module: matrix,
                parameter: "slot_1_dest".to_owned(),
            },
            LoweringReason::OwnedByLaterPhase {
                capability: "a modulation's timing, which V1 reads once per host block — the \
                             Mod Matrix from the source's previous block, the Mod Grid from \
                             the current block's last sample — and V2 composes from the \
                             current quantum's first frame (CORPUS-0003-C1)",
                owner: "the first A/B consumer, under the corpus's intentional-correction class",
            },
        )],
        "{:?}",
        lowered.diagnostics
    );

    // The LFO node, from the saved settings: triangle at 2 Hz, full depth, no phase offset.
    let lfo = lowered
        .identities
        .node_for(ModuleId::new(ModuleType::Lfo, 1))
        .expect("the LFO resolves");
    let node = ir
        .nodes()
        .iter()
        .find(|n| n.id() == lfo)
        .expect("the LFO is a node");
    assert_eq!(
        node.kind(),
        IrNodeKind::Lfo {
            waveform: LfoWaveform::Triangle,
            rate: Frequency::new(2.0).expect("finite"),
            depth: NormalizedLevel::FULL,
            phase_offset: PhaseOffset::ZERO,
            polarity: LfoPolarity::Bipolar,
        }
    );
    assert_eq!(node.scope(), ExecutionScope::Voice);
    // The matrix is no node: one fewer than the saved modules.
    let matrix_node = lowered
        .identities
        .node_for(matrix)
        .expect("the matrix resolves");
    assert!(ir.nodes().iter().all(|n| n.id() != matrix_node));
    assert_eq!(ir.nodes().len(), saved.patch.modules.len() - 1);

    // The edge: `0.7 × 48` semitones into the filter's cutoff, and nothing else.
    let filter = lowered
        .identities
        .node_for(ModuleId::new(ModuleType::Filter, 1))
        .expect("the filter resolves");
    assert_eq!(
        edges(ir),
        vec![(
            lfo,
            filter,
            parameters::FILTER_CUTOFF,
            synth_engine_v2::ir::ModulationUnit::Semitones,
            0.7_f32 * synth_modules::filter::CUTOFF_MOD_SEMITONES,
        )]
    );

    // It renders, and the edge is what changes the sound: the same project with the slot
    // disabled renders differently, and with a zero amount renders exactly as disabled — an
    // edge of zero depth composes the identity.
    let render = |saved: &crate::patch::InstrumentState| {
        super::render::smoke_render(
            saved,
            &project.song,
            &project.global,
            harness_profile(),
            FrameCount::new(4_800),
        )
    };
    let modulated = render(saved);
    assert!(modulated.is_audible(), "{:?}", modulated.diagnostics);
    let with = |key: &str, value: f32| {
        let mut saved = saved.clone();
        let matrix = saved
            .patch
            .modules
            .iter_mut()
            .find(|m| m.module_type == ModuleType::ModMatrix)
            .expect("the matrix is saved");
        floats(matrix, &[(key, value)]);
        saved
    };
    let disabled = render(&with("slot_1_enabled", 0.0));
    assert!(disabled.is_audible(), "{:?}", disabled.diagnostics);
    assert!(
        modulated.samples != disabled.samples,
        "the slot must reach the filter"
    );
    let zero = render(&with("slot_1_amount", 0.0));
    assert!(
        zero.samples == disabled.samples,
        "a zero amount is an edge of zero depth, which composes the identity"
    );
    let zero = with("slot_1_amount", 0.0);
    let lowered = lower_voice_patch(
        zero.id,
        &zero.patch.modules,
        &zero.patch.connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered.ir.as_ref().expect("a zero amount lowers");
    assert_eq!(
        edges(ir).iter().map(|e| e.4).collect::<Vec<f32>>(),
        vec![0.0],
        "V1 evaluates a zero-amount slot, so it is an edge, holding its slot in the profile"
    );
}

/// V1's legacy spellings resolve through V1's own parsers, and each of the oscillator's three
/// pitch keys lowers at its own scale onto the one frequency control.
#[test]
fn a_mod_matrix_slot_lowers_v1s_legacy_spellings_and_each_pitch_key_at_its_scale() {
    use synth_engine_v2::ir::{ModulationUnit, parameters};
    use synth_modules::oscillator;

    let (mut modules, connections) = corpus_patch("sawtooth");
    modules.push(lfo("lfo-1", &[]));
    modules.push(mod_matrix(
        "mmx-1",
        &[
            ("lfo1", "osc1_pitch", 0.5),
            ("lfo-1.out", "osc-1.detune", -1.0),
            ("lfo-01.out", "osc-01.frequency", 0.25),
            ("lfo1", "flt1_cutoff", 0.3),
        ],
    ));
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
    let node = |kind, instance| {
        lowered
            .identities
            .node_for(ModuleId::new(kind, instance))
            .expect("resolves")
    };
    let lfo1 = node(ModuleType::Lfo, 1);
    let (osc, flt) = (node(ModuleType::Oscillator, 1), node(ModuleType::Filter, 1));
    assert_eq!(
        edges(ir),
        vec![
            (
                lfo1,
                osc,
                parameters::SAW_FREQUENCY,
                ModulationUnit::Semitones,
                0.5 * oscillator::PITCH_MOD_SEMITONES
            ),
            (
                lfo1,
                osc,
                parameters::SAW_FREQUENCY,
                ModulationUnit::Semitones,
                -oscillator::DETUNE_MOD_SEMITONES
            ),
            (
                lfo1,
                osc,
                parameters::SAW_FREQUENCY,
                ModulationUnit::Semitones,
                0.25 * oscillator::FREQUENCY_MOD_SEMITONES
            ),
            (
                lfo1,
                flt,
                parameters::FILTER_CUTOFF,
                ModulationUnit::Semitones,
                0.3 * synth_modules::filter::CUTOFF_MOD_SEMITONES
            ),
        ]
    );
    // The scales are V1's, not a transcription: the pitch key is one semitone per unit and
    // the frequency key one octave, which is what makes the two rows above differ.
    assert_eq!(oscillator::PITCH_MOD_SEMITONES, 1.0);
    assert_eq!(oscillator::FREQUENCY_MOD_SEMITONES, 12.0);
    assert_eq!(synth_modules::filter::CUTOFF_MOD_SEMITONES, 48.0);
    // A sine oscillator declares the same frequency control, so the row does not depend on
    // which oscillator kind the patch lowered to.
    assert_eq!(parameters::SAW_FREQUENCY, parameters::SINE_FREQUENCY);

    // And the plan compiles: four edges into the voice scope are four slots per voice.
    let outcome = compile(ir, &RenderConfig::new(harness_profile()));
    assert!(outcome.plan().is_ok(), "{:?}", outcome.plan().err());
}

/// What V1 skips before reading lowers to no edge and no diagnostic: a disabled slot, a slot
/// with no destination or no source, a spelling neither parser accepts, and an address
/// naming a module the patch does not hold. A second Mod Matrix is stored and never walked,
/// as V1's voice never walks it.
#[test]
fn an_inert_mod_matrix_slot_lowers_to_no_edge_and_no_diagnostic() {
    let (mut modules, connections) = corpus_patch("sawtooth");
    modules.push(lfo("lfo-1", &[]));
    let mut matrix = mod_matrix(
        "mmx-1",
        &[
            ("lfo-1.out", "flt-1.cutoff", 0.7),
            ("none", "flt-1.cutoff", 0.7),
            ("lfo-1.out", "none", 0.7),
            ("lfo-3.out", "flt-1.cutoff", 0.7),
            ("lfo-1.out", "flt-2.cutoff", 0.7),
            ("lfo-1.out", "not an address", 0.7),
            ("also not one", "flt-1.cutoff", 0.7),
            // A missing endpoint whose kind or law would otherwise be refused: V1 reads zero
            // from the absent envelope and applies nothing to the absent filter.
            ("env-9.out", "flt-1.cutoff", 0.7),
            ("lfo-1.out", "flt-9.resonance", 0.7),
            ("velocity", "flt-9.cutoff", 0.7),
        ],
    );
    floats(&mut matrix, &[("slot_1_enabled", 0.0)]);
    // A slot the project never wrote at all is the module's own empty routing.
    matrix.parameters.remove("slot_11_source");
    modules.push(matrix);
    // The second matrix would route, and V1 never asks it.
    modules.push(mod_matrix("mmx-2", &[("lfo-1.out", "flt-1.cutoff", 1.0)]));

    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
    assert!(ir.modulations().is_empty(), "{:?}", edges(ir));
    assert!(
        !lowered.diagnostics.iter().any(|d| matches!(
            d.subject(),
            ProjectSubject::Module { module, .. } | ProjectSubject::Parameter { module, .. }
                if module.module_type == ModuleType::ModMatrix
        )),
        "{:?}",
        lowered.diagnostics
    );

    // The reverse: the lowest matrix routes, and a second one that would be refused is not
    // read either.
    let (mut modules, connections) = corpus_patch("sawtooth");
    modules.push(lfo("lfo-1", &[]));
    modules.push(mod_matrix("mmx-1", &[("lfo-1.out", "flt-1.cutoff", 0.7)]));
    modules.push(mod_matrix("mmx-2", &[("velocity", "flt-1.cutoff", 1.0)]));
    let lowered = lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
    );
    let ir = lowered
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
    assert_eq!(ir.modulations().len(), 1);
}

/// Every route this slice does not carry is refused naming the slot and what V1 does with
/// it; an LFO shape or setting V2 cannot hold, and a cable out of an LFO, likewise.
#[test]
fn the_mod_matrix_routes_v2_does_not_carry_are_refused_by_name() {
    let refused = |modules: Vec<ModuleState>,
                   connections: Vec<crate::patch::ConnectionState>,
                   subject: ProjectSubject,
                   needle: &str| {
        let lowered = lower_voice_patch(
            instrument(),
            &modules,
            &connections,
            synth_engine_v2::quantities::EventCount::NONE,
        );
        assert!(lowered.ir.is_none(), "{needle} must refuse");
        assert!(
            lowered.diagnostics.iter().any(|d| {
                d.severity() == Severity::Refused
                    && *d.subject() == subject
                    && matches!(
                        d.reason(),
                        LoweringReason::OwnedByLaterPhase { capability, .. }
                            if capability.contains(needle)
                    )
            }),
            "{needle} must be refused by name on {subject:?}: {:?}",
            lowered.diagnostics
        );
    };
    let slot = |key: &str| ProjectSubject::Parameter {
        instrument: instrument(),
        module: ModuleId::new(ModuleType::ModMatrix, 1),
        parameter: key.to_owned(),
    };
    let lfo_module = ProjectSubject::Module {
        instrument: instrument(),
        module: ModuleId::new(ModuleType::Lfo, 1),
    };
    let routed = |source: &str, destination: &str| {
        let (mut modules, connections) = corpus_patch("sawtooth");
        modules.push(lfo("lfo-1", &[]));
        modules.push(mod_matrix("mmx-1", &[(source, destination, 0.5)]));
        (modules, connections)
    };

    // Sources.
    for source in ["env-1.out", "env1"] {
        let (modules, connections) = routed(source, "flt-1.cutoff");
        refused(
            modules,
            connections,
            slot("slot_1_source"),
            "sourced from an envelope",
        );
    }
    let (modules, connections) = routed("osc-1.out", "flt-1.cutoff");
    refused(modules, connections, slot("slot_1_source"), "not an LFO's");
    let (modules, connections) = routed("flt-1.cutoff", "osc-1.pitch");
    refused(
        modules,
        connections,
        slot("slot_1_source"),
        "module parameter as its source",
    );

    // Destinations, each naming the law V1 applies.
    for (destination, needle) in [
        ("flt-1.resonance", "filter's resonance"),
        ("flt1_reso", "filter's resonance"),
        ("osc-1.level", "oscillator's level"),
        ("amp-1.level", "amplifier's level or pan"),
        ("amp-1.pan", "amplifier's level or pan"),
        ("lfo-1.rate", "LFO's rate"),
        ("lfo-1.depth", "LFO's depth"),
        ("env-1.attack", "envelope time or level"),
        ("env-1.sustain", "envelope time or level"),
        ("flt-1.drive", "declares no control for"),
    ] {
        let (modules, connections) = routed("lfo-1.out", destination);
        refused(modules, connections, slot("slot_1_dest"), needle);
    }

    // A scripted slot.
    let (mut modules, connections) = routed("lfo-1.out", "flt-1.cutoff");
    modules
        .iter_mut()
        .find(|m| m.module_type == ModuleType::ModMatrix)
        .expect("the matrix")
        .scripts
        .insert("0".to_owned(), "out = 0.5".to_owned());
    refused(
        modules,
        connections,
        ProjectSubject::Module {
            instrument: instrument(),
            module: ModuleId::new(ModuleType::ModMatrix, 1),
        },
        "YAMS control script",
    );

    // The LFO itself.
    for waveform in ["sample_and_hold", "smooth_random", "s&h", "random"] {
        let (mut modules, connections) = routed("lfo-1.out", "flt-1.cutoff");
        choice(
            modules
                .iter_mut()
                .find(|m| m.module_type == ModuleType::Lfo)
                .expect("the LFO"),
            "waveform",
            waveform,
        );
        refused(modules, connections, lfo_module.clone(), "random stream");
    }
    let (mut modules, connections) = routed("lfo-1.out", "flt-1.cutoff");
    floats(
        modules
            .iter_mut()
            .find(|m| m.module_type == ModuleType::Lfo)
            .expect("the LFO"),
        &[("tempo_sync", 1.0)],
    );
    refused(modules, connections, lfo_module.clone(), "tempo-synced LFO");

    // A cable out of an LFO, into the one control input V2's table admits.
    let (mut modules, mut connections) = corpus_patch("sawtooth");
    modules.push(lfo("lfo-1", &[]));
    connections.retain(|c| c.to.1 != "cv");
    connections.push(crate::patch::ConnectionState {
        from: ("lfo-1".to_owned(), "out".to_owned()),
        to: ("amp-1".to_owned(), "cv".to_owned()),
    });
    refused(
        modules,
        connections,
        ProjectSubject::Connection {
            instrument: instrument(),
            from: ("lfo-1".to_owned(), "out".to_owned()),
            to: ("amp-1".to_owned(), "cv".to_owned()),
        },
        "cable out of an LFO",
    );
}

/// A saved LFO resolves through V1's descriptor: absent keys are V1's defaults, a rate past
/// the range is clamped as V1 clamps it, a depth past one likewise, and a phase is **wrapped**
/// as V1's `Phase::new` wraps it — one whole period is the cycle's start, and a quarter past
/// it is a quarter in, where a clamp would make both the start.
#[test]
fn an_lfo_lowers_with_v1s_defaults_clamps_and_wrap() {
    use synth_engine_v2::ir::{IrNodeKind, LfoPolarity, LfoWaveform};
    use synth_engine_v2::quantities::{Frequency, NormalizedLevel, PhaseOffset};

    let kind_of = |params: &[(&str, f32)], waveform: Option<&str>| {
        let (mut modules, connections) = corpus_patch("sawtooth");
        let mut saved = lfo("lfo-1", params);
        if let Some(waveform) = waveform {
            choice(&mut saved, "waveform", waveform);
        }
        modules.push(saved);
        let lowered = lower_voice_patch(
            instrument(),
            &modules,
            &connections,
            synth_engine_v2::quantities::EventCount::NONE,
        );
        let ir = lowered
            .ir
            .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
        let node = lowered
            .identities
            .node_for(ModuleId::new(ModuleType::Lfo, 1))
            .expect("the LFO resolves");
        ir.nodes()
            .iter()
            .find(|n| n.id() == node)
            .expect("the LFO is a node")
            .kind()
    };
    let expect = |waveform, rate: f32, depth: f32, phase: f32| IrNodeKind::Lfo {
        waveform,
        rate: Frequency::new(rate).expect("finite"),
        depth: NormalizedLevel::new(depth).expect("a level"),
        phase_offset: PhaseOffset::new(phase).expect("a phase"),
        polarity: LfoPolarity::Bipolar,
    };
    assert_eq!(kind_of(&[], None), expect(LfoWaveform::Sine, 1.0, 1.0, 0.0));
    assert_eq!(
        kind_of(
            &[("rate", 1000.0), ("depth", 2.0), ("phase", 0.25)],
            Some("square")
        ),
        expect(
            LfoWaveform::Square,
            synth_core::Hertz::LFO_RANGE.max,
            1.0,
            0.25
        )
    );
    assert_eq!(
        kind_of(&[("phase", 1.0)], Some("sawtooth")),
        expect(LfoWaveform::Sawtooth, 1.0, 1.0, 0.0)
    );
    assert_eq!(
        kind_of(&[("phase", 1.25)], Some("triangle")),
        expect(LfoWaveform::Triangle, 1.0, 1.0, 0.25)
    );
    assert_eq!(
        kind_of(&[("phase", -0.25)], None),
        expect(LfoWaveform::Sine, 1.0, 1.0, 0.75)
    );
}

/// A global Mod Grid graph hosting an LFO into a module-backed target on this instrument
/// lowers to one global-scope LFO node — read back from the module V1 built — and one edge
/// per target at V1's scale; the render carries it.
#[test]
fn a_global_mod_grid_lfo_into_a_module_target_lowers_to_a_global_node_and_edges() {
    use synth_engine_v2::ir::{
        ExecutionScope, IrNodeKind, LfoPolarity, LfoWaveform, ModulationUnit, parameters,
    };
    use synth_engine_v2::quantities::{Frequency, NormalizedLevel, PhaseOffset};
    use synth_sequencer::{
        AutoInstrumentParam, AutomationTarget, ModConnection, ModNodeConfig, ModNodeId, ModTarget,
        ModulationAmount, ModuleNode,
    };

    let (modules, connections) = corpus_patch("sawtooth");
    let saved = saved_instrument(modules.clone(), connections.clone());
    let plain = four_note_song();
    let mut song = plain.clone();
    let graph_id = song.create_mod_graph("wobble");
    {
        let graph = song.mod_graph_mut(graph_id).expect("the graph resolves");
        graph
            .try_insert_node(
                ModNodeId::new(3),
                ModNodeConfig::Module(ModuleNode {
                    module_type: ModuleType::Lfo,
                    params: BTreeMap::from([
                        ("rate".to_owned(), 3.0),
                        ("depth".to_owned(), 0.5),
                        ("waveform".to_owned(), 1.0),
                        ("phase".to_owned(), 1.25),
                    ]),
                    seed: Some(7),
                }),
            )
            .expect("the LFO inserts");
        graph
            .try_insert_node(
                ModNodeId::new(4),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Module {
                        instrument: instrument(),
                        module_type: ModuleType::Filter,
                        instance: 1,
                        param_id: "cutoff".into(),
                    },
                    amount: ModulationAmount::new(0.5),
                    combine: Default::default(),
                }),
            )
            .expect("the module target inserts");
        graph
            .try_insert_node(
                ModNodeId::new(5),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Instrument {
                        instrument: instrument(),
                        param: AutoInstrumentParam::FilterCutoff,
                    },
                    amount: ModulationAmount::new(-0.25),
                    combine: Default::default(),
                }),
            )
            .expect("the instrument target inserts");
        for target in [4, 5] {
            graph
                .try_connect(ModConnection::new(
                    ModNodeId::new(3),
                    "out",
                    ModNodeId::new(target),
                    "in",
                ))
                .expect("the cable connects");
        }
    }

    let modulators = super::modulation::lower_mod_grid(&song, instrument(), &modules);
    assert!(!modulators.refused, "{:?}", modulators.diagnostics);
    assert!(
        modulators.diagnostics.is_empty(),
        "{:?}",
        modulators.diagnostics
    );
    let lowered = super::graph::lower_voice_patch_with(
        instrument(),
        &modules,
        &connections,
        synth_engine_v2::quantities::EventCount::NONE,
        None,
        None,
        &modulators,
    );
    let ir = lowered
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
    let grid_node = super::modulation::grid_node_address(graph_id, ModNodeId::new(3))
        .expect("the address fits");
    let node = ir
        .nodes()
        .iter()
        .find(|n| n.id() == grid_node)
        .expect("the hosted LFO is a node");
    // As V1 built it: the waveform index through V1's own conversion, the phase wrapped.
    assert_eq!(
        node.kind(),
        IrNodeKind::Lfo {
            waveform: LfoWaveform::Triangle,
            rate: Frequency::new(3.0).expect("finite"),
            depth: NormalizedLevel::new(0.5).expect("a level"),
            phase_offset: PhaseOffset::new(0.25).expect("a phase"),
            polarity: LfoPolarity::Bipolar,
        }
    );
    assert_eq!(node.scope(), ExecutionScope::Global);
    let filter = lowered
        .identities
        .node_for(ModuleId::new(ModuleType::Filter, 1))
        .expect("the filter resolves");
    let scale = synth_modules::filter::CUTOFF_MOD_SEMITONES;
    assert_eq!(
        edges(ir),
        vec![
            (
                grid_node,
                filter,
                parameters::FILTER_CUTOFF,
                ModulationUnit::Semitones,
                0.5 * scale
            ),
            (
                grid_node,
                filter,
                parameters::FILTER_CUTOFF,
                ModulationUnit::Semitones,
                -0.25 * scale
            ),
        ]
    );

    // The render carries it.
    let global = crate::project::GlobalProjectState::default();
    let modulated = super::render::smoke_render(
        &saved,
        &song,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(modulated.is_audible(), "{:?}", modulated.diagnostics);
    let unmodulated = super::render::smoke_render(
        &saved,
        &plain,
        &global,
        harness_profile(),
        FrameCount::new(4_800),
    );
    assert!(modulated.samples != unmodulated.samples);
}

/// Every Mod Grid shape this slice does not carry is refused by name; what V1 does not act
/// on for this instrument lowers to nothing.
#[test]
fn the_mod_grid_shapes_v2_does_not_carry_are_refused_by_name() {
    use synth_sequencer::{
        AudioTapNode, AudioTapSource, AutoInstrumentParam, AutomationTarget, GlobalParam,
        MacroNode, MidiCcNode, ModConnection, ModGraph, ModGraphScope, ModNodeConfig, ModNodeId,
        ModTarget, ModulationAmount, ModuleNode, TrackParam, TransportNode,
    };

    let lfo_node = || {
        ModNodeConfig::Module(ModuleNode {
            module_type: ModuleType::Lfo,
            params: BTreeMap::new(),
            seed: None,
        })
    };
    let cutoff = |instrument: synth_core::InstrumentId| AutomationTarget::Module {
        instrument,
        module_type: ModuleType::Filter,
        instance: 1,
        param_id: "cutoff".into(),
    };
    let target = |target: AutomationTarget| {
        ModNodeConfig::Target(ModTarget {
            target,
            amount: ModulationAmount::new(0.5),
            combine: Default::default(),
        })
    };
    // A graph of `(node id, config)` with the cables given as `(from, port, to, port)`.
    let song_with = |nodes: Vec<(u32, ModNodeConfig)>,
                     cables: &[(u32, &str, u32, &str)],
                     shape: &dyn Fn(&mut ModGraph, synth_sequencer::TrackId)| {
        let mut song = four_note_song();
        let track_id = placed_track(&song);
        let graph_id = song.create_mod_graph("shape");
        let graph = song.mod_graph_mut(graph_id).expect("the graph resolves");
        for (id, config) in nodes {
            graph
                .try_insert_node(ModNodeId::new(id), config)
                .expect("the node inserts");
        }
        for (from, from_port, to, to_port) in cables {
            graph
                .try_connect(ModConnection::new(
                    ModNodeId::new(*from),
                    *from_port,
                    ModNodeId::new(*to),
                    *to_port,
                ))
                .expect("the cable connects");
        }
        shape(graph, track_id);
        song
    };
    let (patch, _) = corpus_patch("sawtooth");
    let refused = |song: &synth_sequencer::Song, needle: &str| {
        let modulators = super::modulation::lower_mod_grid(song, instrument(), &patch);
        assert!(modulators.refused, "{needle} must refuse");
        assert!(
            modulators.diagnostics.iter().any(|d| {
                d.severity() == Severity::Refused
                    && matches!(
                        d.reason(),
                        LoweringReason::OwnedByLaterPhase { capability, .. }
                            if capability.contains(needle)
                    )
            }),
            "{needle} must be refused by name: {:?}",
            modulators.diagnostics
        );
    };
    let inert = |song: &synth_sequencer::Song, why: &str| {
        let modulators = super::modulation::lower_mod_grid(song, instrument(), &patch);
        assert!(
            !modulators.refused
                && modulators.diagnostics.is_empty()
                && modulators.routes.is_empty(),
            "{why}: {:?}",
            modulators.diagnostics
        );
    };
    let global = |_: &mut ModGraph, _: synth_sequencer::TrackId| {};

    // Track scope, assigned: refused as such before anything in it is read.
    refused(
        &song_with(
            vec![(0, lfo_node()), (1, target(cutoff(instrument())))],
            &[(0, "out", 1, "in")],
            &|graph, track| {
                graph.scope = ModGraphScope::Track;
                graph.assigned_tracks.push(track);
            },
        ),
        "track-scoped Mod Grid graph",
    );
    // Sources V2 does not carry, each feeding a target it would otherwise lower.
    refused(
        &song_with(
            vec![
                (
                    0,
                    ModNodeConfig::Macro(MacroNode {
                        name: "depth".into(),
                        value: 0.5.into(),
                    }),
                ),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "macro knob",
    );
    refused(
        &song_with(
            vec![
                (0, ModNodeConfig::Transport(TransportNode::default())),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "transport source",
    );
    refused(
        &song_with(
            vec![
                (
                    0,
                    ModNodeConfig::AudioTap(AudioTapNode {
                        source: AudioTapSource::Master,
                    }),
                ),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "audio tap",
    );
    refused(
        &song_with(
            vec![
                (0, ModNodeConfig::MidiCc(MidiCcNode::default())),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "MIDI CC",
    );
    // Targets V2 does not carry, each fed by an LFO.
    for (automation, needle) in [
        (
            AutomationTarget::Track {
                track: None,
                param: TrackParam::Volume,
            },
            // Relative to no host: dropped by V1's builder, so a resolved one is used below.
            "",
        ),
        (
            AutomationTarget::Global(GlobalParam::MasterVolume),
            "master volume",
        ),
        (
            AutomationTarget::Instrument {
                instrument: instrument(),
                param: AutoInstrumentParam::Volume,
            },
            "channel volume or pan",
        ),
        (
            AutomationTarget::Module {
                instrument: instrument(),
                module_type: ModuleType::Filter,
                instance: 1,
                param_id: "resonance".into(),
            },
            "filter's resonance",
        ),
    ] {
        let song = song_with(
            vec![(0, lfo_node()), (1, target(automation))],
            &[(0, "out", 1, "in")],
            &global,
        );
        if needle.is_empty() {
            inert(
                &song,
                "a relative track target in a global graph is dropped by V1",
            );
        } else {
            refused(&song, needle);
        }
    }
    refused(
        &song_with(
            vec![(0, lfo_node()), (1, target(cutoff(instrument())))],
            &[(0, "out", 1, "in")],
            &|graph, track| {
                // An absolute track target, which a global graph does resolve.
                let node = graph.node(ModNodeId::new(1)).cloned();
                let _ = node;
                graph
                    .try_insert_node(
                        ModNodeId::new(2),
                        target(AutomationTarget::Track {
                            track: Some(track),
                            param: TrackParam::Pan,
                        }),
                    )
                    .expect("the track target inserts");
                graph
                    .try_connect(ModConnection::new(
                        ModNodeId::new(0),
                        "out",
                        ModNodeId::new(2),
                        "in",
                    ))
                    .expect("the cable connects");
            },
        ),
        "track's volume, pan or pitch",
    );
    // A hosted module other than an LFO, a cable into a hosted module, and an injection.
    refused(
        &song_with(
            vec![
                (
                    0,
                    ModNodeConfig::Module(ModuleNode {
                        module_type: ModuleType::Envelope,
                        params: BTreeMap::new(),
                        seed: None,
                    }),
                ),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "module other than an LFO",
    );
    refused(
        &song_with(
            vec![
                (0, lfo_node()),
                (2, lfo_node()),
                (1, target(cutoff(instrument()))),
            ],
            &[(2, "out", 0, "rate_cv"), (0, "out", 1, "in")],
            &global,
        ),
        "cable into a Mod Grid module's input",
    );
    refused(
        &song_with(
            vec![
                (
                    3,
                    ModNodeConfig::Macro(MacroNode {
                        name: "rate".into(),
                        value: 0.5.into(),
                    }),
                ),
                (0, lfo_node()),
                (1, target(cutoff(instrument()))),
            ],
            &[(3, "out", 0, "rate_cv"), (0, "out", 1, "in")],
            &global,
        ),
        "driving a hosted module's input",
    );
    // A random shape on a hosted LFO.
    refused(
        &song_with(
            vec![
                (
                    0,
                    ModNodeConfig::Module(ModuleNode {
                        module_type: ModuleType::Lfo,
                        params: BTreeMap::from([("waveform".to_owned(), 4.0)]),
                        seed: Some(1),
                    }),
                ),
                (1, target(cutoff(instrument()))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "random stream",
    );

    // What lowers to nothing: another instrument's target — whatever feeds it, since the
    // target is settled before the source is read — a target on a module the patch lacks,
    // whatever its law, and a target with no cable.
    inert(
        &song_with(
            vec![
                (0, lfo_node()),
                (1, target(cutoff(synth_core::InstrumentId::new(7)))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "another instrument's target is that instrument's",
    );
    inert(
        &song_with(
            vec![
                (
                    0,
                    ModNodeConfig::Macro(MacroNode {
                        name: "depth".into(),
                        value: 0.5.into(),
                    }),
                ),
                (1, target(cutoff(synth_core::InstrumentId::new(7)))),
            ],
            &[(0, "out", 1, "in")],
            &global,
        ),
        "another instrument's target fed by a macro is still that instrument's",
    );
    for param in ["cutoff", "resonance"] {
        inert(
            &song_with(
                vec![
                    (0, lfo_node()),
                    (
                        1,
                        target(AutomationTarget::Module {
                            instrument: instrument(),
                            module_type: ModuleType::Filter,
                            instance: 9,
                            param_id: param.into(),
                        }),
                    ),
                ],
                &[(0, "out", 1, "in")],
                &global,
            ),
            "a target on a module the patch lacks is V1's no-op",
        );
        // A cable from a port the LFO does not have reads zero in V1, whatever the law of
        // the target it feeds.
        inert(
            &song_with(
                vec![
                    (0, lfo_node()),
                    (
                        1,
                        target(AutomationTarget::Module {
                            instrument: instrument(),
                            module_type: ModuleType::Filter,
                            instance: 1,
                            param_id: param.into(),
                        }),
                    ),
                ],
                &[(0, "nope", 1, "in")],
                &global,
            ),
            "a cable from a port the hosted LFO lacks is V1's zero",
        );
    }
    inert(
        &song_with(
            vec![(0, lfo_node()), (1, target(cutoff(instrument())))],
            &[],
            &global,
        ),
        "a target with no cable is V1's continue",
    );
}

/// The track the fixture places its pattern on.
fn placed_track(song: &synth_sequencer::Song) -> synth_sequencer::TrackId {
    song.arrangement()
        .first()
        .expect("the fixture places one pattern")
        .track_id
}

/// The Mod Grid node address keeps clear of every saved module's and of the scaler's, and
/// an identity that does not fit is refused rather than truncated into a collision.
#[test]
fn a_mod_grid_node_address_cannot_meet_a_saved_modules_or_the_scalers() {
    use super::modulation::grid_node_address;
    use synth_sequencer::{ModGraphId, ModNodeId};

    let lowest = grid_node_address(ModGraphId::new(0), ModNodeId::new(0)).expect("fits");
    let highest = grid_node_address(ModGraphId::new(0x7FFE), ModNodeId::new(0xFFFF)).expect("fits");
    assert!(lowest < highest);
    assert!(highest < super::identity::VOICE_OUTPUT_SCALER);
    // Every saved module address keeps bit 31 clear.
    let resolved = ResolvedIdentities::resolve(&corpus_modules()).expect("resolves");
    for (_, node) in resolved.pairs() {
        assert!(node < lowest, "{node} must sort below every grid address");
    }
    assert!(grid_node_address(ModGraphId::new(0x7FFF), ModNodeId::new(0)).is_none());
    assert!(grid_node_address(ModGraphId::new(0), ModNodeId::new(0x1_0000)).is_none());
}

#[test]
fn each_mod_matrix_macro_lowers_once_at_v1s_target_scale_and_in_its_scope() {
    use synth_core::MacroSource;
    use synth_engine_v2::controller::{ControllerKind, NoteSource};
    use synth_engine_v2::ir::{ExecutionScope, IrNodeKind};
    for (source, kind, scope) in [
        (
            MacroSource::Velocity,
            IrNodeKind::NoteSource {
                kind: NoteSource::Velocity,
            },
            ExecutionScope::Voice,
        ),
        (
            MacroSource::NoteNumber,
            IrNodeKind::NoteSource {
                kind: NoteSource::NoteNumber,
            },
            ExecutionScope::Voice,
        ),
        (
            MacroSource::Aftertouch,
            IrNodeKind::Controller {
                kind: ControllerKind::Aftertouch,
            },
            ExecutionScope::InstrumentInstance,
        ),
        (
            MacroSource::ModWheel,
            IrNodeKind::Controller {
                kind: ControllerKind::ModWheel,
            },
            ExecutionScope::InstrumentInstance,
        ),
        (
            MacroSource::PitchBend,
            IrNodeKind::Controller {
                kind: ControllerKind::PitchBend,
            },
            ExecutionScope::InstrumentInstance,
        ),
        (
            MacroSource::PolyAftertouch,
            IrNodeKind::NoteSource {
                kind: NoteSource::Pressure,
            },
            ExecutionScope::Voice,
        ),
    ] {
        let (mut modules, connections) = corpus_patch("sawtooth");
        modules.push(mod_matrix(
            "mmx-1",
            &[
                (source.id(), "flt-1.cutoff", 0.5),
                (source.id(), "osc-1.pitch", 0.25),
            ],
        ));
        let lowered = lower_voice_patch(
            instrument(),
            &modules,
            &connections,
            synth_engine_v2::quantities::EventCount::NONE,
        );
        let ir = lowered
            .ir
            .as_ref()
            .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics));
        let sources: Vec<_> = ir
            .nodes()
            .iter()
            .filter(|node| node.kind() == kind)
            .collect();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].scope(), scope);
        assert_eq!(ir.modulations().len(), 2);
        assert!(
            ir.modulations()
                .iter()
                .all(|edge| edge.source().0 == sources[0].id())
        );
        assert_eq!(
            ir.modulations()[0].depth().amount(),
            0.5 * synth_modules::filter::CUTOFF_MOD_SEMITONES
        );
        assert_eq!(
            ir.modulations()[1].depth().amount(),
            0.25 * synth_modules::oscillator::PITCH_MOD_SEMITONES
        );
        assert!(
            lowered
                .diagnostics
                .iter()
                .any(|diagnostic| format!("{diagnostic:?}").contains("current voice macro state"))
        );
        let ids: std::collections::HashSet<_> = ir
            .nodes()
            .iter()
            .map(synth_engine_v2::ir::IrNode::id)
            .collect();
        assert_eq!(ids.len(), ir.nodes().len());
    }
}

mod phase7;
