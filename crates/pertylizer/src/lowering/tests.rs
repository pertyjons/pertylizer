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
    modules.push(module("lfo-1", ModuleType::Lfo));
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
                    module: ModuleId::new(ModuleType::Lfo, 1),
                }
                && *d.reason()
                    == LoweringReason::UnsupportedModuleType {
                        module_type: ModuleType::Lfo,
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

    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("a harness profile");

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
    // The **reporting** half is unchanged, and it is a different question: V2 applies velocity
    // as one scale where V1 composes two sensitivities, so the render is admissible and a
    // parity claim over it is not.
    assert_eq!(rendered.fidelity(), Fidelity::UnsupportedScope);
}

/// Two notes overlapping on one gate are refused rather than rendered wrongly.
#[test]
fn overlapping_notes_on_one_gate_are_refused() {
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let song = overlapping_song();

    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("a harness profile");

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

    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("a harness profile");

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
    let peak = super::performance::peak_events_per_quantum(
        instrument(),
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
fn harness_profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("a real rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("a harness profile")
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

/// A muted instrument renders nothing, and says so.
#[test]
fn a_muted_instrument_is_refused_rather_than_rendered_audible() {
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
        synth_engine_v2::quantities::EventCount::measured(0),
        "a project the user silenced must not be lowered as sounding"
    );
    assert!(
        rendered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { capability, .. }
                if capability.contains("muted instrument")
        )),
        "the refusal must name the mute, got {:?}",
        rendered.diagnostics
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
        rendered.samples.len() as u64,
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
fn exactly_two_saved_projects_in_the_repository_lower_to_a_plan() {
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
            "subtractive-voice".to_owned(),
            "tempo-map-arrangement".to_owned()
        ],
        "P04-R002 records two eligible saved projects where the gate asks three, and this is \
         the measurement behind that number. A change here is a change to that obligation."
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
    // offline renderer installs its runtime; V2 has no modulation at all. Whether V1 *runs* a
    // graph is decided by its own builder: a graph with no routing sink builds no instance, and
    // neither does a track-scoped graph assigned to no track. An independent review found the
    // check refusing on the pool being non-empty, which blocked a project holding a freshly
    // created, still-empty graph — one V1 plays unchanged.
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
    // Assigned, it runs, and is refused. Global scope runs unconditionally, and is refused.
    song.mod_graph_mut(graph_id)
        .expect("the graph resolves")
        .assigned_tracks
        .push(track_id);
    refused_for(&song, "Mod Grid graph");
    {
        let graph = song.mod_graph_mut(graph_id).expect("the graph resolves");
        graph.assigned_tracks.clear();
        graph.scope = ModGraphScope::Global;
    }
    refused_for(&song, "Mod Grid graph");

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
            // `automation` refused in `render::project_diagnostics` over every placement, when a
            // lane holds a point; `processors` and `note_graph` refused in
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

/// A placed pattern carrying automation is refused, because V1 applies its lanes.
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
    // The other half. V2 applies velocity as one scale on the envelope, so half the velocity
    // is half the peak — which is a stronger claim than "they differ" and is what
    // distinguishes velocity reaching the amplitude from velocity reaching anything at all.
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
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let mut song = one_note_song(60, 0.8, 0.0);
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
fn a_render_that_places_a_note_still_refuses_a_parity_comparison() {
    // The reporting half, and it is deliberately *not* closed by this slice. V1 applies one
    // saved velocity twice — at the envelope and again at the voice output — and V2 applies it
    // once, so the render is admissible and a parity claim over it is not. The work list says
    // closing `P03-R003` "does not decide Phase 6's tuning or expression-composition model",
    // which is what this asserts rather than assumes.
    let rendered = render_one_note(60, 0.8, 0.0);
    assert!(rendered.is_audible());
    assert_eq!(rendered.fidelity(), Fidelity::UnsupportedScope);
    assert!(
        rendered.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { owner, .. } if owner.contains("composition law")
        )),
        "and it names what is unrepresented, got {:?}",
        rendered.diagnostics
    );

    // Raised **once**, not once per note: it is a property of the lowering rather than of any
    // note the project holds.
    let (modules, connections) = corpus_patch("sine");
    let saved = saved_instrument(modules, connections);
    let four = super::render::smoke_render(
        &saved,
        &four_note_song(),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(48_000),
    );
    assert_eq!(
        four.diagnostics
            .iter()
            .filter(|d| matches!(
                d.reason(),
                LoweringReason::OwnedByLaterPhase { owner, .. } if owner.contains("composition law")
            ))
            .count(),
        1,
        "four notes must not raise it four times, got {:?}",
        four.diagnostics
    );
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

    let peak = super::performance::peak_events_per_quantum(
        instrument(),
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
