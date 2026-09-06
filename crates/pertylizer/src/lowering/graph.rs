//! Lowering one instrument's voice patch into a V2 graph.
//!
//! # What the mapping is, and where each half of it comes from
//!
//! Five saved module types have a counterpart in V2's node registry. Two of the mappings are
//! not the identity, and neither is guessed here:
//!
//! - **Resonance is a different quantity in each engine.** V1's filter takes a normalised
//!   resonance and forms `k = 2 - 2·res`; V2's takes a quality factor and forms
//!   `damping = 1/Q`. `EVD-0013` established that these are the same coefficient under two
//!   names, so the lowering law is `Q = 1 / (2 - 2·res)` — an equality between the two
//!   engines' own arithmetic, not a curve fitted to it.
//! - **The output nodes are not the same node.** V1's `StereoOutput` pans, limits, meters and
//!   writes two channels; V2's `Output` writes one source to however many channels the
//!   profile declares. `EVD-0013` records that too. The lowering keeps the routing and
//!   reports the rest as unrepresented rather than pretending the stages are absent.
//!
//! Three more asymmetries were found by an independent read of an earlier revision, and each
//! is verified against V1's own source rather than assumed:
//!
//! - **An unpatched amplifier control is unity in V1 and silence in V2.** V1 reads its `cv`
//!   input with a default of `1.0`, so an amplifier used as a plain gain stage sounds; V2's
//!   unpatched input reads defined silence, so the same graph is silent. The topology is
//!   refused rather than lowered into silence.
//! - **V1's amplifier pans, and at centre that is not unity.** Its equal-power law takes
//!   centre to `cos(π/4)` on each channel — about `0.707` — where V2's amplifier only
//!   multiplies. Every lowered amplifier reports the stage.
//! - **V1 clamps resonance into `[0, 0.99]` before using it.** A saved `1.0` is valid input
//!   that V1 renders at `0.99`; converting the raw value instead would refuse a filter V1
//!   plays. The clamp is applied first, so the conversion sees what V1 sees.
//!
//! # Why an unmapped parameter is a diagnostic rather than a default
//!
//! Every saved parameter is either consumed by the mapping or reported. A parameter that V2
//! has no home for is only harmless when the project leaves it at the value that makes it do
//! nothing, so the check is against that neutral value rather than against the key: a filter
//! whose `env_amt` is zero is fully represented, and one whose `env_amt` is `0.5` is not.
//! Silently dropping the second is the "accepting and silently ignoring" that `PROCESS.md`'s
//! phase-exit rule forbids.
//!
//! A key the mapping does not read at all is reported unconditionally. An earlier revision
//! read only the keys the corpus fixture happens to carry, so a filter `model`, a `drive` or
//! an envelope curve vanished while the lowering still reported success. Each kind now
//! declares the keys it consumes, and everything else in the saved map raises a diagnostic
//! naming that parameter.
//!
//! # Why the defaults and clamps are asked for rather than written down
//!
//! A saved parameter map is `serde`-defaulted, so a project may omit a key that V1 then
//! supplies from the module's own default; and V1 clamps most values into a declared range
//! before rendering them, so a saved `master` of `2.0` is unity there and a negative attack
//! is zero. Both facts are needed to lower faithfully, and both were originally transcribed
//! here by reading V1's modules one at a time.
//!
//! That was the wrong shape, and successive independent reads kept finding the transcription
//! incomplete — one more clamp, one more default, each a separate defect. So the lowerer asks
//! V1 instead: `module_factory::create_voice_module` yields the module's own
//! `ModuleDescriptor`, and each `ParameterDescriptor` carries the `ValueRange` that declares
//! its minimum, maximum and default. Every saved value is resolved against that declaration —
//! absent means the declared default, present means the value clamped into the declared range
//! — so a rule V1 declares cannot go missing from here.

use synth_core::{ModuleDescriptor, ModuleType};
use synth_engine::ModuleId;
use synth_engine::instrument::InstrumentId;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use synth_engine_v2::quantities::{
    Amplitude, CutoffFrequency, EventCount, Frequency, HeldNoteCount, NormalizedLevel, Resonance,
    Seconds,
};
use synth_engine_v2::tuning::PreparedTuning;

use super::diagnostics::{LoweringDiagnostic, LoweringReason, ProjectSubject};
use super::identity::ResolvedIdentities;
use crate::patch::{ConnectionState, ModuleState, ParamValue};

/// The frequency a lowered oscillator starts at, before any note has played.
///
/// A saved oscillator has no frequency of its own — in V1 the note supplies it, and in V2 the
/// note's key resolves through the plan's tuning and reaches this same control. So this is the
/// value the node is **prepared** with rather than the value it renders: a plan's first note
/// overwrites it, and nothing sounds before that note because the envelope's gate is low.
///
/// Concert A is chosen for being recognisable in a spectrum rather than for being right: an
/// arrangement whose notes all lie past the end of their patterns lowers to a plan with no
/// note at all, and this is the pitch that plan would sound if anything ever opened its gate.
fn placeholder_frequency() -> Frequency {
    // 440 Hz is finite, so this cannot fail; the fallback is silence rather than a panic
    // because a lowerer has no business unwrapping.
    Frequency::new(440.0).unwrap_or(Frequency::ZERO)
}

/// Which V2 oscillator a saved waveform selects.
///
/// A local enum rather than a `bool`, so adding the third waveform V2 grows is an arm here
/// rather than an inversion somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waveform {
    Sine,
    Saw,
}

/// A lowered voice patch, and everything the lowerer could not represent about it.
#[derive(Debug)]
#[must_use]
pub struct LoweredGraph {
    /// The graph, when one could be built at all.
    pub ir: Option<GraphIr>,
    /// What the lowering has to say. Empty means fully represented.
    pub diagnostics: Vec<LoweringDiagnostic>,
    /// The identity mapping the graph was built through, so a caller can name a node.
    pub identities: ResolvedIdentities,
}

/// Lower one instrument's modules and connections.
///
/// Never returns `Err`: a failure that stops the graph is a `Refused` diagnostic with a
/// subject, which is what the exit gate asks for, and a `Result` would lose the subject.
pub fn lower_voice_patch(
    instrument: InstrumentId,
    modules: &[ModuleState],
    connections: &[ConnectionState],
    events_per_quantum: EventCount,
) -> LoweredGraph {
    lower_voice_patch_with(instrument, modules, connections, events_per_quantum, None)
}

/// [`lower_voice_patch`], with V1's voice-output velocity stage (ADR-0059).
///
/// `velocity_amp_sensitivity` is the instrument's saved sensitivity; `Some` places a
/// [`IrNodeKind::VelocityScaler`] between the voice's terminating node and the output, so a
/// note is scaled by V1's `(1 − s) + s × v` there as V1 scales it at the voice's output.
/// `None` lowers the patch as it is, for a caller that lowers no instrument.
pub fn lower_voice_patch_with(
    instrument: InstrumentId,
    modules: &[ModuleState],
    connections: &[ConnectionState],
    events_per_quantum: EventCount,
    velocity_amp_sensitivity: Option<NormalizedLevel>,
) -> LoweredGraph {
    let mut diagnostics = Vec::new();

    let identities = match ResolvedIdentities::resolve(modules) {
        Ok(identities) => identities,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument,
                    name: String::new(),
                },
                LoweringReason::UnresolvedEndpoint {
                    spelling: error.to_string(),
                },
            ));
            return LoweredGraph {
                ir: None,
                diagnostics,
                identities: ResolvedIdentities::default(),
            };
        }
    };

    // Which `(module, port)` destinations the patch actually cables. An amplifier's control
    // input is the one place where V1 and V2 disagree about what an *absent* cable means, so
    // the answer has to be known before a module is lowered rather than after.
    // Keyed by the **parsed** identity, not the spelling: V1 resolves `amp-01` and `amp-1` to
    // the same `ModuleId`, so a cable spelled one way into a module declared the other is
    // patched there. A raw-string table called it unpatched; an independent review found it.
    // A spelling that does not parse is left out here and refused by name in
    // `lower_connection`, where the diagnostic can carry it.
    let patched: Vec<(ModuleId, String)> = connections
        .iter()
        .filter_map(|c| cable_end(&c.to))
        .collect();

    // V1 picks a voice's terminal from `StereoOutput`, then `Amplifier`, then `Mixer`, so a
    // patch without an explicit output still sounds there. V2 requires an `Output` node, and
    // synthesising one would be inventing structure the project does not have — so the
    // topology is refused with a reason rather than lowered into a graph `compile` rejects
    // as `MissingOutput`, which names nothing the user authored.
    // V2 admits exactly one output. V1 resolves a winner among several deterministically, so
    // a two-output patch is valid input there and a plan no profile can compile here.
    // Sorted by identity, so which output is treated as the patch's and which are named as
    // extras does not depend on the array's order. V1 resolves its own winner from a map
    // keyed the same way.
    let mut outputs: Vec<&ModuleState> = modules
        .iter()
        .filter(|m| m.module_type == ModuleType::StereoOutput)
        .collect();
    outputs.sort_by_key(|m| m.id.parse::<ModuleId>().ok());
    if outputs.len() > 1 {
        for extra in outputs.iter().skip(1) {
            let subject = match extra.id.parse::<ModuleId>() {
                Ok(id) => ProjectSubject::Module {
                    instrument,
                    module: id,
                },
                Err(_) => ProjectSubject::Instrument {
                    instrument,
                    name: String::new(),
                },
            };
            diagnostics.push(LoweringDiagnostic::refused(
                subject,
                LoweringReason::OwnedByLaterPhase {
                    capability: "a second output module, which V1 resolves between and V2 \
                                 refuses outright",
                    owner: "Phase 8, with the mixer and bus model",
                },
            ));
        }
        return LoweredGraph {
            ir: None,
            diagnostics,
            identities,
        };
    }
    if outputs.is_empty() {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Instrument {
                instrument,
                name: String::new(),
            },
            LoweringReason::OwnedByLaterPhase {
                capability: "a voice patch with no explicit output module, which V1 terminates \
                             at its amplifier or mixer instead",
                owner: "Phase 6, with the voice-instantiation model",
            },
        ));
        return LoweredGraph {
            ir: None,
            diagnostics,
            identities,
        };
    }

    let mut builder = GraphIr::builder();
    let mut refused = false;
    let mut edges: Vec<(NodeId, NodeId, ConnectionState)> = Vec::new();
    // The output node's address, so the cable into it can be routed through the scaler.
    let output_node = outputs
        .first()
        .and_then(|module| module.id.parse::<ModuleId>().ok())
        .and_then(|id| identities.node_for(id));
    let scaler = velocity_amp_sensitivity.map(|_| super::identity::VOICE_OUTPUT_SCALER);
    if let Some(sensitivity) = velocity_amp_sensitivity {
        builder = builder.node(
            super::identity::VOICE_OUTPUT_SCALER,
            IrNodeKind::VelocityScaler { sensitivity },
            ExecutionScope::Voice,
        );
    }

    for module in modules {
        let Ok(id) = module.id.parse::<ModuleId>() else {
            continue;
        };
        let Some(node) = identities.node_for(id) else {
            continue;
        };

        match lower_module(instrument, id, module, &patched, &mut diagnostics) {
            Some((kind, scope)) => builder = builder.node(node, kind, scope),
            None => refused = true,
        }
    }

    // V1 sums every cable arriving at one input; V2 refuses fan-in outright. Two cables into
    // one port therefore build cleanly here and fail at `compile`, with no diagnostic naming
    // either connection — so the second one is refused where the user authored it.
    // A patch may repeat a cable verbatim. V1 keeps its connections in a set, so the repeat
    // is a no-op there and the input still has one cable — treating it as fan-in would refuse
    // a patch V1 plays. Exact duplicates are dropped first; only genuinely different sources
    // arriving at one port are fan-in.
    // Both tables below compare **parsed** endpoints, for the reason `patched` does: a repeat
    // spelled `osc-01` is the cable V1 already has, and a second cable spelled that way into a
    // port is fan-in there. A spelling that does not parse compares by its text, so two such
    // cables are neither merged nor called fan-in on the strength of a name that resolves to
    // nothing; `lower_connection` refuses each by name.
    let same_cable = |a: &ConnectionState, b: &ConnectionState| {
        let key = |c: &ConnectionState| (cable_end(&c.from), cable_end(&c.to));
        match (key(a), key(b)) {
            ((Some(af), Some(at)), (Some(bf), Some(bt))) => af == bf && at == bt,
            _ => a.from == b.from && a.to == b.to,
        }
    };
    let mut seen: Vec<&ConnectionState> = Vec::new();
    let unique: Vec<&ConnectionState> = connections
        .iter()
        .filter(|c| {
            let repeated = seen.iter().any(|prior| same_cable(prior, c));
            if !repeated {
                seen.push(c);
            }
            !repeated
        })
        .collect();

    let mut occupied: Vec<(ModuleId, String)> = Vec::new();
    for connection in unique {
        let destination = cable_end(&connection.to);
        if destination
            .as_ref()
            .is_some_and(|destination| occupied.contains(destination))
        {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Connection {
                    instrument,
                    from: connection.from.clone(),
                    to: connection.to.clone(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "two cables into one input, which V1 sums and V2 refuses",
                    owner: "Phase 8, with the mixer and summing model",
                },
            ));
            refused = true;
            continue;
        }
        if let Some(destination) = destination {
            occupied.push(destination);
        }

        match lower_connection(
            instrument,
            connection,
            &identities,
            modules,
            &mut diagnostics,
        ) {
            Some((from, to, domain)) => {
                // ADR-0059: the cable into the output enters the velocity stage instead, and
                // the stage feeds the output below.
                let to = match scaler {
                    Some(scaler) if Some(to.0) == output_node => (scaler, PortId::FIRST),
                    _ => to,
                };
                edges.push((from.0, to.0, connection.clone()));
                builder = builder.connect(from, to, domain);
            }
            None => refused = true,
        }
    }
    if let (Some(scaler), Some(output)) = (scaler, output_node) {
        builder = builder.connect(
            (scaler, PortId::FIRST),
            (output, PortId::FIRST),
            SignalDomain::Audio,
        );
    }

    // V2 refuses a cyclic graph at compilation, and `GraphIr::build` does not look. Without
    // this the lowering would report success for a patch that can never render, and the
    // refusal would arrive later naming a plan rather than the cable the user drew.
    if let Some(closing) = cycle_closing_connection(&edges) {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Connection {
                instrument,
                from: closing.from.clone(),
                to: closing.to.clone(),
            },
            LoweringReason::OwnedByLaterPhase {
                capability: "a feedback path, which V2 refuses as a cycle",
                owner: "Phase 8, with the latency and feedback model",
            },
        ));
        refused = true;
    }

    if refused {
        return LoweredGraph {
            ir: None,
            diagnostics,
            identities,
        };
    }

    // ADR-0047 clause 3 partitions identity ranges across the producers a plan declares, so a
    // plan that says nothing cannot stamp a note at all. This lowering has exactly one
    // producer, and it is compiled: every note the arrangement places is in the plan, and a
    // compiled producer's releases are in the plan with them, so it owes no release holds.
    // One simultaneous note, because one scalar gate sounds one note — the same fact that
    // makes `lower_performance` refuse an overlap.
    // `SOUND-INV-021`: a scope holding a pitch destination states the tuning its keys
    // resolve through, and admission refuses a plan that does not. Every lowered oscillator
    // is in the voice scope, so that is the scope that states one.
    //
    // Twelve-tone equal temperament, because that is what a V1 project plays: V1 resolves a
    // note through `MidiNote::to_frequency`, which is 12-TET about A440 and is not selectable
    // per project. A saved project therefore *has* no other tuning to carry, and choosing
    // this one reproduces it rather than substituting for it. Per-project tuning selection is
    // Phase 10A's authored model.
    let builder = match PreparedTuning::equal_temperament() {
        Ok(tuning) => builder.tuning(ExecutionScope::Voice, tuning),
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument,
                    name: String::new(),
                },
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return LoweredGraph {
                ir: None,
                diagnostics,
                identities,
            };
        }
    };

    let builder = builder.declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::NONE,
        }],
        held_notes: HeldNoteCount::measured(1),
        // The most note edges the arrangement puts in one quantum, counted from the same
        // timeline the renderer is later given. Admission partitions its event capacity
        // across declared producers, so a plan declaring zero is admitted for a load it does
        // not carry: the edges then arrive under the profile's global cap and never meet the
        // compiled producer's own share. An independent review found exactly that.
        events_per_quantum,
        ..PlanDeclarations::default()
    });

    match builder.build() {
        Ok(ir) => LoweredGraph {
            ir: Some(ir),
            diagnostics,
            identities,
        },
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument,
                    name: String::new(),
                },
                LoweringReason::UnresolvedEndpoint {
                    spelling: error.to_string(),
                },
            ));
            LoweredGraph {
                ir: None,
                diagnostics,
                identities,
            }
        }
    }
}

/// The node kind and execution scope one saved module lowers to.
///
/// The scope is the module's, not the graph's. Every DSP stage of a voice patch is per-voice
/// — an oscillator's phase, a filter's state and an envelope's position all belong to one
/// note — and only the terminating output is global. Declaring them all `Global`, as an
/// earlier revision did, would make one set of state serve every note the moment scope is
/// used for instantiation.
fn lower_module(
    instrument: InstrumentId,
    id: ModuleId,
    module: &ModuleState,
    patched: &[(ModuleId, String)],
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<(IrNodeKind, ExecutionScope)> {
    let subject = || ProjectSubject::Module {
        instrument,
        module: id,
    };
    let parameter = |key: &str| ProjectSubject::Parameter {
        instrument,
        module: id,
        parameter: key.to_owned(),
    };

    // V1's own declaration of this module type: the source of every default and every clamp
    // below. A type with no voice module has no declarations and no V2 counterpart either.
    let Some((_, declarations)) = crate::module_factory::create_voice_module(module.module_type)
    else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnsupportedModuleType {
                module_type: module.module_type,
            },
        ));
        return None;
    };

    let lowered = match module.module_type {
        ModuleType::Oscillator => {
            if !audit_parameters(
                module,
                &declarations,
                &["waveform", "level", "uni_phase"],
                &parameter,
                diagnostics,
            ) {
                return None;
            }

            // An absent `waveform` is whatever V1's own descriptor declares as the default —
            // read from it rather than transcribed, so that if V1's default moves, this moves
            // with it instead of lowering the old waveform silently. A squash review found a
            // literal here.
            let waveform = choice_or_declared_default(
                module,
                &declarations,
                "waveform",
                &parameter,
                diagnostics,
            )?;
            // `P04-R003` is discharged for the sawtooth: V2 now has one. Every *other*
            // waveform is still a node kind that does not exist, rather than a parameter V2
            // chose not to read, so it is refused by name.
            let oscillator = match waveform.as_str() {
                "sine" => Waveform::Sine,
                "sawtooth" => Waveform::Saw,
                _ => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        parameter("waveform"),
                        LoweringReason::UnsupportedParameterValue { value: waveform },
                    ));
                    return None;
                }
            };
            // V1's descriptor default is 1.0 — full phase randomisation on note-on — so an
            // omitted key is the *most* audible setting rather than the neutral one.
            if !require_neutral(
                module,
                &declarations,
                "uni_phase",
                0.0,
                &parameter,
                diagnostics,
            ) {
                return None;
            }

            // V1 clamps the level it renders into `[0, 2]`, so a saved `-1` is silence there
            // and would be an inverted full-scale sine here. The clamp is applied first, for
            // the same reason the filter's resonance clamp is.
            let amplitude = v1_value(module, &declarations, "level", &parameter, diagnostics)?;
            let amplitude = quantity(Amplitude::new(amplitude), parameter("level"), diagnostics)?;
            (
                match oscillator {
                    Waveform::Sine => IrNodeKind::Sine {
                        frequency: placeholder_frequency(),
                        amplitude,
                    },
                    Waveform::Saw => IrNodeKind::Saw {
                        frequency: placeholder_frequency(),
                        amplitude,
                    },
                },
                ExecutionScope::Voice,
            )
        }

        ModuleType::Filter => {
            if !audit_parameters(
                module,
                &declarations,
                &["type", "cutoff", "resonance", "env_amt"],
                &parameter,
                diagnostics,
            ) {
                return None;
            }

            let kind =
                choice_or_declared_default(module, &declarations, "type", &parameter, diagnostics)?;
            if kind != "lowpass" {
                diagnostics.push(LoweringDiagnostic::refused(
                    parameter("type"),
                    LoweringReason::UnsupportedParameterValue { value: kind },
                ));
                return None;
            }
            // `env_amt` is read and deliberately not judged. V1 only ever multiplies it by
            // the `cutoff_cv` input, which reads zero when nothing is cabled there, so the
            // parameter is dormant in an unpatched filter whatever its value — and when a
            // project *does* cable `cutoff_cv`, V2's filter declares no such port, so the
            // cable is refused by the connection mapping. The difference lives at the cable,
            // which is what the user drew, rather than at a parameter that does nothing
            // without one.
            let _ = &patched;

            // V1 puts every cutoff it is given through `Hertz::clamp_filter`, whose range is
            // 20 Hz to 20 kHz. Passing the raw value would render a saved 30 kHz cutoff at
            // 30 kHz here and at 20 kHz there — or fail admission below Nyquist.
            let cutoff = v1_value(module, &declarations, "cutoff", &parameter, diagnostics)?;
            // V1 clamps the resonance it renders into `[0, 0.99]`, so the conversion is fed
            // the value V1 uses rather than the value the file stores. A saved `1.0` is valid
            // input that V1 plays at `0.99`; refusing it would refuse a filter V1 sounds.
            let normalised =
                v1_value(module, &declarations, "resonance", &parameter, diagnostics)?.min(0.99);
            // `k = 2 - 2·res` is V1's damping; V2 spells the same coefficient `1/Q`. The
            // clamp above keeps this strictly positive, so the division is safe by
            // construction rather than by check.
            let damping = 2.0 - 2.0 * normalised;
            (
                IrNodeKind::Filter {
                    cutoff: quantity(
                        CutoffFrequency::new(cutoff),
                        parameter("cutoff"),
                        diagnostics,
                    )?,
                    resonance: quantity(
                        Resonance::new(1.0 / damping),
                        parameter("resonance"),
                        diagnostics,
                    )?,
                },
                ExecutionScope::Voice,
            )
        }

        ModuleType::Envelope => {
            if !audit_parameters(
                module,
                &declarations,
                &["attack", "decay", "sustain", "release", "vel_sens"],
                &parameter,
                diagnostics,
            ) {
                return None;
            }
            // V1's own defaults, so an omitted key means what V1 means by omitting it.
            (
                IrNodeKind::Envelope {
                    attack: quantity(
                        Seconds::new(v1_value(
                            module,
                            &declarations,
                            "attack",
                            &parameter,
                            diagnostics,
                        )?),
                        parameter("attack"),
                        diagnostics,
                    )?,
                    decay: quantity(
                        Seconds::new(v1_value(
                            module,
                            &declarations,
                            "decay",
                            &parameter,
                            diagnostics,
                        )?),
                        parameter("decay"),
                        diagnostics,
                    )?,
                    sustain: quantity(
                        NormalizedLevel::new(v1_value(
                            module,
                            &declarations,
                            "sustain",
                            &parameter,
                            diagnostics,
                        )?),
                        parameter("sustain"),
                        diagnostics,
                    )?,
                    release: quantity(
                        Seconds::new(v1_value(
                            module,
                            &declarations,
                            "release",
                            &parameter,
                            diagnostics,
                        )?),
                        parameter("release"),
                        diagnostics,
                    )?,
                    // ADR-0059: V1's `vel_sens`, the envelope's own velocity sensitivity, read
                    // with V1's default where the key is omitted.
                    velocity_sensitivity: quantity(
                        NormalizedLevel::new(v1_value(
                            module,
                            &declarations,
                            "vel_sens",
                            &parameter,
                            diagnostics,
                        )?),
                        parameter("vel_sens"),
                        diagnostics,
                    )?,
                },
                ExecutionScope::Voice,
            )
        }

        ModuleType::Amplifier => {
            if !audit_parameters(
                module,
                &declarations,
                &["level", "pan"],
                &parameter,
                diagnostics,
            ) {
                return None;
            }
            // V2's amplifier multiplies its input by its control and has no level of its
            // own, so a level other than unity is a stage V2 does not have.
            if !require_neutral(module, &declarations, "level", 1.0, &parameter, diagnostics) {
                return None;
            }

            // V1 reads an unpatched `cv` as `1.0`; V2 reads an unpatched control input as
            // defined silence. So the same graph that V1 sounds, V2 renders silent — and a
            // silent render with no diagnostic is exactly what fail-closed forbids.
            let has_control = patched
                .iter()
                .any(|(to_module, to_port)| *to_module == id && to_port == "cv");
            if !has_control {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::OwnedByLaterPhase {
                        capability: "an amplifier with no control cable, which V1 drives at \
                                     unity and V2 would render silent",
                        owner: "Phase 5, with the declarative node API's default-value law",
                    },
                ));
                return None;
            }

            // V1's amplifier pans, and its equal-power law puts centre at `cos(π/4)` on each
            // channel rather than at unity. V2's does not pan at all, so the stage is absent
            // whatever the saved value is — including when there is none.
            diagnostics.push(LoweringDiagnostic::unrepresented(
                subject(),
                LoweringReason::OwnedByLaterPhase {
                    capability: "the amplifier's pan stage, whose equal-power centre is not \
                                 unity gain",
                    owner: "Phase 8",
                },
            ));
            (IrNodeKind::Amplifier, ExecutionScope::Voice)
        }

        ModuleType::StereoOutput => {
            if !audit_parameters(module, &declarations, &["master"], &parameter, diagnostics) {
                return None;
            }
            // V1's terminating node defaults its master level to 0.8, not unity, so an
            // omitted key is already an amplitude V2 does not apply.
            if !require_neutral(
                module,
                &declarations,
                "master",
                1.0,
                &parameter,
                diagnostics,
            ) {
                return None;
            }
            // Recorded by `EVD-0013`: V1's terminating node pans, limits and meters, and
            // V2's writes one source to the profile's channels. The routing survives; those
            // three stages do not.
            diagnostics.push(LoweringDiagnostic::unrepresented(
                subject(),
                LoweringReason::OwnedByLaterPhase {
                    capability: "the terminating node's pan, limiter and metering stages",
                    owner: "Phase 8",
                },
            ));
            // The one node of a voice patch that is not per-voice: every voice mixes into it.
            (IrNodeKind::Output, ExecutionScope::Global)
        }

        other => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnsupportedModuleType { module_type: other },
            ));
            return None;
        }
    };
    Some(lowered)
}

/// The edge one saved connection becomes, reporting either endpoint that does not resolve.
///
/// Returns the edge rather than adding it, because the builder is consuming: handing it out
/// and taking it back would need a `Default` it deliberately does not have.
type LoweredEdge = (
    (synth_engine_v2::ir::NodeId, PortId),
    (synth_engine_v2::ir::NodeId, PortId),
    SignalDomain,
);

fn lower_connection(
    instrument: InstrumentId,
    connection: &ConnectionState,
    identities: &ResolvedIdentities,
    modules: &[ModuleState],
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<LoweredEdge> {
    let subject = || ProjectSubject::Connection {
        instrument,
        from: connection.from.clone(),
        to: connection.to.clone(),
    };

    let Some((from_id, from_node)) = endpoint(&connection.from.0, identities) else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnresolvedEndpoint {
                spelling: connection.from.0.clone(),
            },
        ));
        return None;
    };
    let Some((to_id, to_node)) = endpoint(&connection.to.0, identities) else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnresolvedEndpoint {
                spelling: connection.to.0.clone(),
            },
        ));
        return None;
    };
    let _ = modules;

    let Some(from_port) = source_port(from_id.module_type, &connection.from.1) else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnknownPort {
                port: connection.from.1.clone(),
            },
        ));
        return None;
    };
    let Some((to_port, to_domain)) = destination_port(to_id.module_type, &connection.to.1) else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnknownPort {
                port: connection.to.1.clone(),
            },
        ));
        return None;
    };

    // The source's domain is its kind's: an envelope's only output is a control stream and
    // every other supported source's is audio.
    let from_domain = if from_id.module_type == ModuleType::Envelope {
        SignalDomain::Control
    } else {
        SignalDomain::Audio
    };

    // `GraphIr::build` does not compare domains, so a cable from an oscillator into a control
    // input would build cleanly here and be refused much later by `compile`, with the
    // lowering already reporting success. Comparing them here is what keeps the refusal
    // attached to the connection the user authored.
    if from_domain != to_domain {
        let name = |domain: SignalDomain| match domain {
            SignalDomain::Audio => "audio",
            SignalDomain::Control => "control",
            SignalDomain::Gate => "gate",
            _ => "another domain",
        };
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::DomainMismatch {
                port: connection.to.1.clone(),
                expected: name(to_domain),
                found: name(from_domain),
            },
        ));
        return None;
    }

    Some(((from_node, from_port), (to_node, to_port), from_domain))
}

/// The connection that closes a cycle, if the lowered edges contain one.
///
/// A depth-first walk marking nodes grey while they are on the stack: an edge into a grey
/// node is a back edge, and that edge is the one the user can act on. Iterative rather than
/// recursive, because a patch's depth is the user's to choose and a stack overflow is not a
/// diagnostic.
fn cycle_closing_connection(
    edges: &[(NodeId, NodeId, ConnectionState)],
) -> Option<&ConnectionState> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unseen,
        OnStack,
        Done,
    }

    let mut nodes: Vec<NodeId> = Vec::new();
    for (from, to, _) in edges {
        for node in [from, to] {
            if !nodes.contains(node) {
                nodes.push(*node);
            }
        }
    }
    let mut marks = vec![Mark::Unseen; nodes.len()];
    let index_of = |node: NodeId, nodes: &[NodeId]| nodes.iter().position(|n| *n == node);

    for start in 0..nodes.len() {
        if marks[start] != Mark::Unseen {
            continue;
        }
        // Each stack entry is a node plus how many of its outgoing edges have been taken.
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        marks[start] = Mark::OnStack;
        while let Some((node, taken)) = stack.pop() {
            let outgoing: Vec<&(NodeId, NodeId, ConnectionState)> = edges
                .iter()
                .filter(|(from, _, _)| *from == nodes[node])
                .collect();
            if taken >= outgoing.len() {
                marks[node] = Mark::Done;
                continue;
            }
            stack.push((node, taken + 1));
            let (_, to, connection) = outgoing[taken];
            let Some(next) = index_of(*to, &nodes) else {
                continue;
            };
            match marks[next] {
                Mark::OnStack => return Some(connection),
                Mark::Done => {}
                Mark::Unseen => {
                    marks[next] = Mark::OnStack;
                    stack.push((next, 0));
                }
            }
        }
    }
    None
}

/// Resolve one endpoint's module spelling.
fn endpoint(
    spelling: &str,
    identities: &ResolvedIdentities,
) -> Option<(ModuleId, synth_engine_v2::ir::NodeId)> {
    let id: ModuleId = spelling.parse().ok()?;
    identities.node_for(id).map(|node| (id, node))
}

/// Which port a saved source-port name is, on the kind that carries it.
///
/// The kind matters: V2's `Output` node declares no output at all, so a cable leaving a
/// `StereoOutput` names a port that does not exist. Accepting it here let `GraphIr::build`
/// raise `NotASource` instead, which this module could only report against the *instrument* —
/// naming neither the cable the user drew nor the reason.
fn source_port(module_type: ModuleType, name: &str) -> Option<PortId> {
    match (module_type, name) {
        (ModuleType::StereoOutput, _) => None,
        (_, "out") => Some(PortId::FIRST),
        _ => None,
    }
}

/// Which port a saved destination-port name is, and what domain that port carries.
fn destination_port(module_type: ModuleType, name: &str) -> Option<(PortId, SignalDomain)> {
    match (module_type, name) {
        (ModuleType::Amplifier, "cv") => Some((
            synth_engine_v2::node::AMPLIFIER_CONTROL,
            SignalDomain::Control,
        )),
        (ModuleType::Filter | ModuleType::Amplifier | ModuleType::StereoOutput, "in") => {
            Some((PortId::FIRST, SignalDomain::Audio))
        }
        _ => None,
    }
}

/// What a saved numeric parameter turned out to be.
///
/// Three states rather than two. An earlier revision mapped both "the project does not carry
/// this key" and "the project carries something this cannot read" to `None`, so a `cutoff`
/// stored as a boolean, a choice or an out-of-range integer silently became the default — the
/// reinterpretation of persisted input `AGENTS.md` forbids. Only [`SavedFloat::Absent`] may
/// take a default.
enum SavedFloat {
    /// The project does not carry the key. V1's own default applies.
    Absent,
    /// The project carries a number.
    Value(f32),
    /// The project carries something else, described for the diagnostic.
    Unsupported(String),
}

/// A saved numeric parameter.
fn float(module: &ModuleState, key: &str) -> SavedFloat {
    match module.parameters.get(key) {
        None => SavedFloat::Absent,
        // A non-finite parameter must not reach a quantity: `NaN` compares unequal to every
        // neutral value and equal to none, so it would slip through the neutrality test as
        // "not different", and an infinity would reach an amplitude or a cutoff.
        Some(ParamValue::Float(value)) if value.is_finite() => SavedFloat::Value(*value),
        Some(ParamValue::Float(value)) => {
            SavedFloat::Unsupported(format!("{value} is not a finite value"))
        }
        // Exact for every integer a `f32` represents without rounding. Beyond that the value
        // is refused rather than rounded, because a rounded cutoff is a different filter.
        // `unsigned_abs` rather than `abs`: `i32::MIN` has no positive counterpart, so
        // `abs` overflows on it — a panic with overflow checks on, and a wrapped negative
        // that passes the bound with them off.
        Some(ParamValue::Int(value)) => match value.unsigned_abs() <= (1 << 24) {
            true => SavedFloat::Value(*value as f32),
            false => SavedFloat::Unsupported(format!("integer {value} is not exact in f32")),
        },
        Some(other) => SavedFloat::Unsupported(format!("{other:?}")),
    }
}

/// The value V1 would render for one saved parameter.
///
/// Resolved against V1's own `ParameterDescriptor`: an absent key becomes the declared
/// default, and a present one is clamped into the declared range exactly as V1 clamps it
/// before use. `None` means the value was refused and a diagnostic was recorded; it never
/// means "absent".
fn v1_value(
    module: &ModuleState,
    declarations: &ModuleDescriptor,
    key: &str,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<f32> {
    let Some(declared) = declarations.find_parameter(key).map(|p| p.range) else {
        // A key V1 does not declare for this kind is not something V1 renders either, so
        // there is no value to resolve and no default to fall back to.
        diagnostics.push(LoweringDiagnostic::refused(
            parameter(key),
            LoweringReason::UnsupportedParameterValue {
                value: format!("{key} is not a parameter this module type declares"),
            },
        ));
        return None;
    };

    match float(module, key) {
        SavedFloat::Absent => Some(declared.default),
        SavedFloat::Value(value) => Some(value.clamp(declared.min, declared.max)),
        SavedFloat::Unsupported(description) => {
            diagnostics.push(LoweringDiagnostic::refused(
                parameter(key),
                LoweringReason::UnsupportedParameterValue { value: description },
            ));
            None
        }
    }
}

/// A saved choice parameter, or `None` when the project does not carry one.
///
/// Returns the outer `None` when the key is present but is **not** a choice. A legacy project
/// may store a choice as its numeric index, and V1's own descriptor path decodes that — so
/// treating it as absent and applying a default would silently reinterpret it as a different
/// waveform or filter type. Refusing keeps the reinterpretation from happening at all.
fn choice(
    module: &ModuleState,
    key: &str,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Option<String>> {
    match module.parameters.get(key) {
        None => Some(None),
        Some(ParamValue::Choice(value)) => Some(Some(value.clone())),
        Some(other) => {
            diagnostics.push(LoweringDiagnostic::refused(
                parameter(key),
                LoweringReason::UnsupportedParameterValue {
                    value: format!("{other:?}"),
                },
            ));
            None
        }
    }
}

/// A saved choice, or the one V1's descriptor declares when the project saves none.
///
/// V1 creates a module from its descriptor and applies the saved parameters over it, so an
/// absent choice *is* the descriptor's default there. The default is read the way
/// `gen_schemas` reads it — `range.default` is the index into `choices` — rather than
/// transcribed as a literal that would go on lowering the old default after V1's moved. A
/// descriptor that declares no such choice is refused by name: nothing says what V1 would do.
fn choice_or_declared_default(
    module: &ModuleState,
    declarations: &ModuleDescriptor,
    key: &str,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<String> {
    if let Some(saved) = choice(module, key, parameter, diagnostics)? {
        return Some(saved);
    }
    let declared = declarations
        .parameters
        .iter()
        .find(|p| p.type_id == key)
        .and_then(|p| {
            let index = usize::try_from(p.range.default.round().max(0.0) as i64).ok()?;
            p.choices.as_ref()?.get(index).map(|c| c.id.clone())
        });
    match declared {
        Some(id) => Some(id),
        None => {
            diagnostics.push(LoweringDiagnostic::refused(
                parameter(key),
                LoweringReason::UnsupportedParameterValue {
                    value: format!(
                        "absent, and V1's descriptor declares no default choice for `{key}`"
                    ),
                },
            ));
            None
        }
    }
}

/// A cable endpoint as V1 resolves it: the parsed module identity and the port's spelling.
///
/// `None` when the module spelling does not parse; the caller decides whether that is refused
/// here or left to `lower_connection`, which names it.
fn cable_end(end: &(String, String)) -> Option<(ModuleId, String)> {
    end.0.parse::<ModuleId>().ok().map(|id| (id, end.1.clone()))
}

/// Report every saved parameter the mapping does not read.
///
/// Unconditional, because a key the mapping never looks at is a stage V2 does not have
/// whatever its value is, and there is no neutral value to compare it against without
/// knowing what it means.
fn audit_parameters(
    module: &ModuleState,
    declarations: &ModuleDescriptor,
    consumed: &[&str],
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> bool {
    // Every consumed key is resolved even when its value is not otherwise used, so a
    // non-finite one cannot hide behind being dormant. `env_amt` is the case that motivated
    // this: it is read and deliberately not judged, and a `NaN` there poisons V1's cutoff
    // through a multiplication this lowerer would otherwise never look at.
    let mut ok = true;
    for key in consumed {
        if !module.parameters.contains_key(*key) {
            continue;
        }
        // A key V1 declares with choices is a choice, and `choice` is what validates it.
        // Resolving it as a number here would refuse every waveform and filter type.
        let is_choice = declarations
            .find_parameter(key)
            .is_some_and(|p| p.choices.is_some());
        if is_choice {
            continue;
        }
        if v1_value(module, declarations, key, parameter, diagnostics).is_none() {
            ok = false;
        }
    }

    for key in module.parameters.keys() {
        if !consumed.contains(&key.as_str()) {
            diagnostics.push(LoweringDiagnostic::unrepresented(
                parameter(key),
                LoweringReason::OwnedByLaterPhase {
                    capability: "a saved parameter no V2 node kind reads",
                    owner: "Phase 5, with the declarative node and parameter API",
                },
            ));
        }
    }
    // A saved script is authored state as much as a parameter is, and V2 compiles none.
    if !module.scripts.is_empty() {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            parameter("scripts"),
            LoweringReason::OwnedByLaterPhase {
                capability: "a YAMS control script",
                owner: "Phase 7",
            },
        ));
    }
    ok
}

/// Report a parameter V2 has no home for, unless the project leaves it doing nothing.
///
/// `v1_default` is what V1 uses when the key is absent, and it is resolved **before** the
/// comparison. That matters more than it looks: an omitted `uni_phase` means `1.0` in V1 —
/// full phase randomisation on every note-on — and an omitted `master` means `0.8`. Treating
/// absence as neutral, as an earlier revision did, reported neither.
/// Returns `false` when the value itself was refused, so the caller can stop. Reporting a
/// `Refused` diagnostic and then returning an IR would contradict what that severity means.
fn require_neutral(
    module: &ModuleState,
    declarations: &ModuleDescriptor,
    key: &str,
    neutral: f32,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> bool {
    let Some(value) = v1_value(module, declarations, key, parameter, diagnostics) else {
        return false;
    };
    // Exact, not within an epsilon. A `master` of 0.99999994 or a `uni_phase` of 1e-8 is a
    // value V1 renders and V2 does not, and an epsilon window would report neither. Finiteness
    // is already established by `v1_value`, so this comparison is total.
    if value != neutral {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            parameter(key),
            LoweringReason::UnsupportedParameterValue {
                value: value.to_string(),
            },
        ));
    }
    true
}

/// Turn a refused quantity into a diagnostic that names the parameter it came from.
fn quantity<T, E: std::fmt::Display>(
    built: Result<T, E>,
    subject: ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<T> {
    match built {
        Ok(value) => Some(value),
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            None
        }
    }
}
