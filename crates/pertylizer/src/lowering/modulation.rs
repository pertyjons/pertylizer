//! Lowering V1's modulation routings into V2 modulation edges (`P07-S003`).
//!
//! V1 modulates a parameter from two places. A voice patch's **Mod Matrix** holds up to
//! sixteen slots, each a source address, a destination address, a bipolar amount and an
//! enabled flag; every block, the voice resolves each enabled slot's source, multiplies it by
//! the amount and hands `source × amount` to the destination module's `set_mod_offset`, where
//! the module scales it into its own unit — the filter's cutoff by ±48 semitones, an
//! oscillator's `frequency` by an octave. The song's **Mod Grid** hosts control-rate modules
//! outside any voice and routes their outputs into automation targets, through the same
//! `set_mod_offset` for a module-backed target.
//!
//! V2 has one mechanism for both: a modulation edge from a control-domain output into a
//! declared parameter, at a depth stated in the target law's units (`SOUND-INV-027`). So a
//! slot or a grid target lowers to one edge, and ADR-0007 clause 3's accepted cost is paid
//! here: **V1's per-target scale becomes the edge's amount** — a slot at `0.7` into a cutoff is
//! an edge of `0.7 × 48` semitones. The scales are read from V1's own constants rather than
//! transcribed, so a change to V1's scale reaches this lowering.
//!
//! # What lowers and what is refused, and why the line is where it is
//!
//! Sources and target laws arrive as the corpus demands them, one at a time. The corpus's
//! Mod Matrix case (`CORPUS-0003`) routes an LFO into the filter's cutoff, and the example
//! projects route LFOs and envelopes into cutoffs and pitches. So the source lowered here is
//! the **LFO** — the one native modulator V2 has — and the targets are those whose V1 law is
//! one V2 declares for the same parameter: the filter's cutoff and the oscillator's three
//! pitch keys, all semitone-additive in both engines, and an LFO's depth, normalized-additive
//! in both. Everything else is refused **by name**, and each refusal says which law V1 applies
//! that V2 does not: the filter's resonance is offset in its normalized unit and then mapped
//! into a quality factor, which no additive law on the quality expresses; the oscillator's
//! level is a normalized amplitude that V2 declares in decibels; the envelope's times are
//! offset through a normalized curve over their descriptor range. An envelope as a *source*
//! is refused because `SOUND-INV-027`'s source rule excludes a kind with an input port, and
//! the slice that lifts that rule owns it. Controller and note macros lower since `P07-S004`; a scripted
//! slot is `P07-S005`'s.
//!
//! # What is inert in V1 and lowers to nothing
//!
//! V1 skips a disabled slot and a slot with no destination before reading anything, pushes
//! no offset for a slot with no source, reads zero from a source address that names no
//! module, and applies nothing to a destination address that names none. Each of those is
//! neutral in V1 and lowers to no edge and no diagnostic, which is `LOWER-INV-004`'s rule:
//! what V1 short-circuits before running is neutral. A slot with a zero amount is *not* in that
//! list — V1 evaluates it and adds zero — so it lowers to an edge of zero depth, which holds
//! a Mod Matrix slot in the profile as V1's slot does.

use synth_core::{
    DestAddr, MAX_MOD_MATRIX_SLOTS, ModMatrixParam, ModuleParam, ModuleType, Param, SrcAddr,
};
use synth_engine::ModuleId;
use synth_engine::instrument::InstrumentId;
use synth_engine_v2::ir::{
    ExecutionScope, IrNodeKind, LfoPolarity, LfoWaveform, ModulationDepth, ModulationUnit, NodeId,
    ParameterId, parameters,
};
use synth_engine_v2::quantities::{Frequency, NormalizedLevel, PhaseOffset};
use synth_sequencer::{AutoInstrumentParam, AutomationTarget, CombineMode, ModGraphId, ModNodeId};

use super::diagnostics::{LoweringDiagnostic, LoweringReason, ProjectSubject};
use super::graph::{audit_parameters, choice, choice_or_declared_default, quantity, v1_value};
use crate::patch::ModuleState;

/// Where a modulation edge's source lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModulationSource {
    /// A module in the voice patch, by identity: an LFO whose `out` the edge reads.
    Module(ModuleId),
    /// A node the Mod Grid lowering adds to the plan, at its address.
    Grid(NodeId),
    /// One of V1's named note/controller macros.
    Macro(synth_core::MacroSource),
}

/// One modulation edge before its endpoints are addressed.
///
/// The target is a saved module identity rather than a node, because the graph lowering is
/// what maps identities to addresses; a target the patch does not hold is V1's no-op and
/// lowers to nothing there. The depth is already in the law's units.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ModulationRoute {
    /// What is read.
    pub source: ModulationSource,
    /// The module whose parameter is modulated.
    pub target: ModuleId,
    /// Which of that node's parameters, in V2's declaration.
    pub parameter: ParameterId,
    /// `amount × V1's scale`, in the target law's units.
    pub depth: ModulationDepth,
    /// The project object a diagnostic about this edge names.
    pub subject: ProjectSubject,
}

/// What the song adds to one instrument's graph: the Mod Grid's hosted modulators as nodes,
/// and their routes into the instrument's modules.
///
/// Built by [`lower_mod_grid`] and handed to the graph lowering, which stays free of the song
/// as it always was. The empty value is what a caller without a song passes.
#[derive(Debug, Default)]
#[must_use]
pub struct SongModulators {
    /// Nodes to add to the plan, in the global scope: one per hosted LFO some route reads.
    pub(super) nodes: Vec<(NodeId, IrNodeKind)>,
    /// The edges, one per Mod Grid target that acts on this instrument.
    pub(super) routes: Vec<ModulationRoute>,
    /// What the grid asked for that V2 cannot do.
    pub diagnostics: Vec<LoweringDiagnostic>,
    /// Whether any of those stops the lowering.
    pub refused: bool,
}

impl SongModulators {
    fn refuse(&mut self, subject: ProjectSubject, capability: &'static str, owner: &'static str) {
        self.diagnostics.push(LoweringDiagnostic::refused(
            subject,
            LoweringReason::OwnedByLaterPhase { capability, owner },
        ));
        self.refused = true;
    }
}

/// A V2 parameter a V1 modulation destination lowers onto, with V1's scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ModulationTarget {
    /// The parameter in V2's declaration of the lowered kind.
    pub parameter: ParameterId,
    /// The unit the edge's depth is stated in, which is the target law's.
    pub unit: ModulationUnit,
    /// What V1's `set_mod_offset` multiplies the normalized contribution by.
    pub scale: f32,
}

/// The oscillator's frequency control, which both lowered oscillator kinds declare first.
const OSCILLATOR_FREQUENCY: ParameterId = parameters::SAW_FREQUENCY;
const _: () = assert!(OSCILLATOR_FREQUENCY.as_raw() == parameters::SINE_FREQUENCY.as_raw());

/// Which V2 parameter a V1 destination `(module type, parameter key)` modulates, or why it
/// cannot.
///
/// A row exists only where V1's `set_mod_offset` arithmetic for the key **is** the law V2
/// declares for the parameter, so that `amount × scale` composes in V2 exactly as V1's
/// `offset × scale` does. The scales are V1's own constants. Every other key names, in its
/// refusal, the law V1 applies that V2 does not.
pub(super) fn modulation_target(
    module_type: ModuleType,
    key: &str,
) -> Result<ModulationTarget, &'static str> {
    use synth_modules::{filter, oscillator};
    let target = |parameter, unit, scale| ModulationTarget {
        parameter,
        unit,
        scale,
    };
    match (module_type, key) {
        // `Filter::set_mod_offset("cutoff")`: `Semitones += value × 48`, applied as
        // `base × 2^(st/12)` — V2's semitone law on the corner.
        (ModuleType::Filter, "cutoff") => Ok(target(
            parameters::FILTER_CUTOFF,
            ModulationUnit::Semitones,
            filter::CUTOFF_MOD_SEMITONES,
        )),
        // `Oscillator::set_mod_offset`: all three pitch keys accumulate into one semitone
        // offset applied in `actual_frequency` as `2^(st/12)`, each with its own scale.
        (ModuleType::Oscillator, "pitch") => Ok(target(
            OSCILLATOR_FREQUENCY,
            ModulationUnit::Semitones,
            oscillator::PITCH_MOD_SEMITONES,
        )),
        (ModuleType::Oscillator, "detune") => Ok(target(
            OSCILLATOR_FREQUENCY,
            ModulationUnit::Semitones,
            oscillator::DETUNE_MOD_SEMITONES,
        )),
        (ModuleType::Oscillator, "frequency") => Ok(target(
            OSCILLATOR_FREQUENCY,
            ModulationUnit::Semitones,
            oscillator::FREQUENCY_MOD_SEMITONES,
        )),
        // `Lfo::set_mod_offset("depth")` accumulates through `BipolarValue::new`, which
        // clamps the running offset into `[-1, 1]` after **each** contribution; V2 clamps once
        // after the sum. One slot composes alike; two whose running sum leaves the interval do
        // not, and an independent read found the row claiming the law for both.
        (ModuleType::Lfo, "depth") => Err(
            "a modulation into an LFO's depth, whose offset V1 clamps after each contribution \
             and V2 after the sum",
        ),
        (ModuleType::Filter, "resonance") => Err(
            "a modulation into the filter's resonance, which V1 offsets in its normalized unit \
             before mapping it into the quality factor V2 declares, a curve no additive law \
             on the quality expresses",
        ),
        (ModuleType::Oscillator, "level") => Err(
            "a modulation into the oscillator's level, a normalized amplitude V1 offsets \
             linearly and V2 declares under the decibel law",
        ),
        (ModuleType::Amplifier, "level" | "pan") => Err(
            "a modulation into the amplifier's level or pan, which V2's amplifier declares no \
             control for",
        ),
        (ModuleType::Lfo, "rate") => Err(
            "a modulation into an LFO's rate, which V1 offsets in hertz and V2 declares under \
             the semitone law",
        ),
        (ModuleType::Envelope, "attack" | "decay" | "sustain" | "release") => Err(
            "a modulation into an envelope time or level, which V1 offsets through its \
             descriptor's normalized curve over the parameter's range",
        ),
        _ => Err("a modulation into a parameter V2 declares no control for"),
    }
}

/// The plan address a Mod Grid node computes to.
///
/// Bit 31 set, the graph's identity in the next fifteen bits and the node's in the low
/// sixteen, so it cannot meet a saved module's address nor an inserted node's — every
/// instrument slot keeps bit 31 clear (`identity`). Both halves are persisted identities,
/// never positions, so the address survives the pool being reordered. `None` when an
/// identity does not fit, which the caller refuses by name rather than truncating into a
/// collision.
pub(super) fn grid_node_address(graph: ModGraphId, node: ModNodeId) -> Option<NodeId> {
    const GRID: u32 = 0x8000_0000;
    if graph.0 >= 0x7FFF || node.0 > 0xFFFF {
        return None;
    }
    Some(NodeId::new(GRID | (graph.0 << 16) | node.0))
}

/// The V2 node an LFO's V1-typed settings lower to, or the refusal that stops it.
///
/// One function for both paths — a saved module resolved through its descriptor, and a Mod
/// Grid's hosted module read back from the module V1 built — so the two cannot come to
/// disagree about what an LFO is. The values arrive already clamped and wrapped by V1's own
/// constructors: `Hertz` into `LFO_RANGE`, `NormalizedValue` into `[0, 1]`, `Phase` modulo
/// one, so a saved phase of `1.0` is the cycle's start here as it is in V1 rather than a
/// value `PhaseOffset` refuses.
///
/// V1's LFO has no polarity control: its mode is bipolar and nothing sets it otherwise.
fn lfo_kind(
    waveform: synth_core::LfoWaveform,
    rate: synth_core::Hertz,
    depth: synth_core::NormalizedValue,
    phase: synth_core::Phase,
    tempo_synced: bool,
    subject: &impl Fn() -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<IrNodeKind> {
    // Matched exhaustively on V1's enum, so a shape V1 grows is a compile error here rather
    // than a silent fall-through to some other shape.
    let waveform = match waveform {
        synth_core::LfoWaveform::Sine => LfoWaveform::Sine,
        synth_core::LfoWaveform::Triangle => LfoWaveform::Triangle,
        synth_core::LfoWaveform::Sawtooth => LfoWaveform::Sawtooth,
        synth_core::LfoWaveform::Square => LfoWaveform::Square,
        // V2 declares both and refuses them at compilation (`SeedlessRandomWaveform`); the
        // refusal is raised here so it names the module rather than a plan.
        synth_core::LfoWaveform::SampleAndHold | synth_core::LfoWaveform::SmoothRandom => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::OwnedByLaterPhase {
                    capability: "an LFO shape that draws on a random stream, which no node may \
                                 do before a seed exists (P06-R001)",
                    owner: "Phase 7, once ADR-0008 defines a seed",
                },
            ));
            return None;
        }
    };
    // A synced LFO takes its phase from the transport's beat position and ignores its rate;
    // V2's LFO has a rate and nothing else to follow.
    if tempo_synced {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a tempo-synced LFO, whose phase follows the transport's beat \
                             position instead of a rate",
                owner: "Phase 7",
            },
        ));
        return None;
    }
    let rate = quantity(Frequency::new(rate.as_f32()), subject(), diagnostics)?;
    let depth = quantity(NormalizedLevel::new(depth.as_f32()), subject(), diagnostics)?;
    let phase_offset = quantity(PhaseOffset::new(phase.as_f32()), subject(), diagnostics)?;
    Some(IrNodeKind::Lfo {
        waveform,
        rate,
        depth,
        phase_offset,
        polarity: LfoPolarity::Bipolar,
    })
}

/// Lower a saved LFO module, resolving every value through V1's descriptor.
///
/// The keys read are the seven V1 declares. `sync_division` matters only when the LFO is
/// synced, which is refused; `retrigger` acts only through the `retrigger` gate port, and a
/// cable into a port V2's LFO does not declare is refused where the cable is. Both are
/// consumed so that a non-finite value cannot hide behind being dormant, and neither is
/// judged.
pub(super) fn lower_lfo_module(
    module: &ModuleState,
    declarations: &synth_core::ModuleDescriptor,
    subject: &impl Fn() -> ProjectSubject,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<IrNodeKind> {
    if !audit_parameters(
        module,
        declarations,
        &[
            "waveform",
            "rate",
            "depth",
            "phase",
            "tempo_sync",
            "sync_division",
            "retrigger",
        ],
        parameter,
        diagnostics,
    ) {
        return None;
    }
    let waveform =
        choice_or_declared_default(module, declarations, "waveform", parameter, diagnostics)?;
    // V1's own parser, aliases included, rather than a second table of spellings.
    let Some(waveform) = synth_core::LfoWaveform::from_id(&waveform) else {
        diagnostics.push(LoweringDiagnostic::refused(
            parameter("waveform"),
            LoweringReason::UnsupportedParameterValue { value: waveform },
        ));
        return None;
    };
    let rate = v1_value(module, declarations, "rate", parameter, diagnostics)?;
    let depth = v1_value(module, declarations, "depth", parameter, diagnostics)?;
    // The phase is **wrapped**, not clamped: V1's loader hands the saved number to
    // `LfoParam::with_f32`, whose `Phase::new` takes it modulo one, so a saved `1.25` starts
    // the cycle a quarter in. `v1_value`'s clamp into the descriptor's `[0, 1]` would make it
    // the cycle's start instead; an independent read found that. The finite number, or the
    // declared default, goes through the same conversion V1's loader uses.
    let phase = match raw_or_default(module, declarations, "phase", parameter, diagnostics)?
        .map(|declared, raw| declared.id.with_f32(raw))
    {
        Param::Lfo(synth_core::LfoParam::Phase(phase)) => phase,
        _ => {
            diagnostics.push(LoweringDiagnostic::refused(
                parameter("phase"),
                LoweringReason::UnsupportedParameterValue {
                    value: "V1's declaration does not convert it to a phase".to_owned(),
                },
            ));
            return None;
        }
    };
    // The saved toggle becomes a boolean by V1's own conversion, through the descriptor's
    // `Param`, rather than by a threshold written here.
    let tempo_sync = v1_value(module, declarations, "tempo_sync", parameter, diagnostics)?;
    let tempo_synced = match declarations
        .find_parameter("tempo_sync")
        .map(|declared| declared.id.with_f32(tempo_sync))
    {
        Some(Param::Lfo(synth_core::LfoParam::TempoSync(synced))) => synced,
        _ => {
            diagnostics.push(LoweringDiagnostic::refused(
                parameter("tempo_sync"),
                LoweringReason::UnsupportedParameterValue {
                    value: "V1's declaration does not convert it to a sync toggle".to_owned(),
                },
            ));
            return None;
        }
    };
    lfo_kind(
        waveform,
        synth_core::Hertz::new(rate),
        synth_core::NormalizedValue::new(depth),
        phase,
        tempo_synced,
        subject,
        diagnostics,
    )
}

/// A saved number and the declaration it is read against, for a value V1 converts itself.
struct RawValue<'a> {
    declared: &'a synth_core::ParameterDescriptor,
    raw: f32,
}

impl RawValue<'_> {
    fn map<T>(self, convert: impl FnOnce(&synth_core::ParameterDescriptor, f32) -> T) -> T {
        convert(self.declared, self.raw)
    }
}

/// The saved finite number for `key`, or the declared default when the project omits it —
/// **unclamped**, for a value whose V1 conversion is not a clamp.
///
/// `v1_value` clamps into the declared range because that is what V1's setter does for most
/// parameters; a `Phase` wraps instead, and clamping first would change what V1 renders.
fn raw_or_default<'a>(
    module: &ModuleState,
    declarations: &'a synth_core::ModuleDescriptor,
    key: &str,
    parameter: &impl Fn(&str) -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<RawValue<'a>> {
    let Some(declared) = declarations.find_parameter(key) else {
        diagnostics.push(LoweringDiagnostic::refused(
            parameter(key),
            LoweringReason::UnsupportedParameterValue {
                value: format!("{key} is not a parameter this module type declares"),
            },
        ));
        return None;
    };
    // `v1_value` establishes that the saved value is a finite number the reader accepts —
    // absent, a float or an exact integer — and refuses anything else by name; its clamped
    // result is then discarded for the unclamped number.
    v1_value(module, declarations, key, parameter, diagnostics)?;
    let raw = match module.parameters.get(key) {
        None => declared.range.default,
        Some(crate::patch::ParamValue::Float(value)) => *value,
        #[allow(clippy::cast_precision_loss)]
        Some(crate::patch::ParamValue::Int(value)) => *value as f32,
        Some(_) => return None,
    };
    Some(RawValue { declared, raw })
}

/// Lower the Mod Matrix V1 applies to this voice patch into routes.
///
/// `None` when something in it stops the lowering. The routes name the matrix's slot as
/// their subject, so a later refusal at the edge can point at the slot the user filled.
///
/// The walk is V1's `resolve_routings_into_cache`, in its order: a disabled slot, then one
/// with no destination, is skipped before its source is read; a slot with a script is the
/// script's; a slot with no source pushes nothing. The saved strings become addresses through
/// the same two parsers V1's loader calls, legacy spellings included, and the enabled flag
/// becomes a boolean through the descriptor's own conversion.
pub(super) fn lower_mod_matrix(
    instrument: InstrumentId,
    id: ModuleId,
    module: &ModuleState,
    declarations: &synth_core::ModuleDescriptor,
    identities: &super::identity::ResolvedIdentities,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Vec<ModulationRoute>> {
    let subject = || ProjectSubject::Module {
        instrument,
        module: id,
    };
    let parameter = |key: &str| ProjectSubject::Parameter {
        instrument,
        module: id,
        parameter: key.to_owned(),
    };

    // A scripted slot's output *replaces* `source × amount`, so the scalar route below would
    // be a different modulator from the one V1 runs. Refused before anything is read.
    if !module.scripts.is_empty() {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a Mod Matrix slot driven by a YAMS control script, whose output \
                             replaces the slot's source and amount",
                owner: "P07-S005, with YAMS Control as a node kind",
            },
        ));
        return None;
    }

    // Every key the matrix declares: the grid size and the four per slot. The grid size
    // decides how many slots the GUI draws; V1's voice walks every slot regardless of it, so
    // it is consumed and not judged.
    let mut consumed: Vec<String> = vec!["grid_size".to_owned()];
    for slot in 1..=MAX_MOD_MATRIX_SLOTS {
        for field in ["source", "dest", "amount", "enabled"] {
            consumed.push(format!("slot_{slot}_{field}"));
        }
    }
    let consumed: Vec<&str> = consumed.iter().map(String::as_str).collect();
    if !audit_parameters(module, declarations, &consumed, &parameter, diagnostics) {
        return None;
    }

    let mut routes = Vec::new();
    for slot in 0..MAX_MOD_MATRIX_SLOTS {
        let number = slot + 1;
        let enabled_key = format!("slot_{number}_enabled");
        let enabled = v1_value(module, declarations, &enabled_key, &parameter, diagnostics)?;
        let enabled = match declarations
            .find_parameter(&enabled_key)
            .map(|declared| declared.id.with_f32(enabled))
        {
            Some(Param::ModMatrix(ModMatrixParam::SlotEnabled(_, enabled))) => enabled,
            _ => {
                diagnostics.push(LoweringDiagnostic::refused(
                    parameter(&enabled_key),
                    LoweringReason::UnsupportedParameterValue {
                        value: "V1's declaration does not convert it to an enabled flag".to_owned(),
                    },
                ));
                return None;
            }
        };
        if !enabled {
            continue;
        }

        // Absent means the module's own initial routing, which has no destination and no
        // source; a spelling neither parser accepts is likewise `None` to V1's loader.
        let dest_key = format!("slot_{number}_dest");
        let Some(destination) = choice(module, &dest_key, &parameter, diagnostics)?
            .as_deref()
            .and_then(DestAddr::parse)
        else {
            continue;
        };
        let source_key = format!("slot_{number}_source");
        let Some(source) = choice(module, &source_key, &parameter, diagnostics)?
            .as_deref()
            .and_then(SrcAddr::parse)
        else {
            continue;
        };
        let amount_key = format!("slot_{number}_amount");
        let amount = v1_value(module, declarations, &amount_key, &parameter, diagnostics)?;

        // Whether either end names a module the patch lacks is asked **before** either end
        // is classified: V1 reads zero from a dangling source and applies nothing to a
        // dangling destination, whatever kind or law the address would otherwise name, so a
        // stale route into a removed module is its no-op and not a refusal. An independent
        // read found the law refused first.
        let target = ModuleId::new(destination.module_type, destination.instance);
        if identities.node_for(target).is_none() {
            continue;
        }
        if let SrcAddr::Module {
            module_type,
            instance,
            ..
        } = source
            && identities
                .node_for(ModuleId::new(module_type, instance))
                .is_none()
        {
            continue;
        }

        let slot_subject = parameter(&source_key);
        let source = match source {
            SrcAddr::Macro(source) => ModulationSource::Macro(source),
            SrcAddr::Module {
                module_type,
                instance,
                name,
            } => {
                let source = ModuleId::new(module_type, instance);
                match (module_type, name.as_str()) {
                    (ModuleType::Lfo, "out") => ModulationSource::Module(source),
                    (ModuleType::Envelope, "out") => {
                        diagnostics.push(LoweringDiagnostic::refused(
                            slot_subject,
                            LoweringReason::OwnedByLaterPhase {
                                capability: "a Mod Matrix slot sourced from an envelope, whose \
                                             kind declares an input port and a \
                                             sample-positioned control and so cannot run ahead \
                                             of the quantum's positioned writes",
                                owner: "Phase 7, with the collection split that lifts \
                                        SOUND-INV-027's source rule",
                            },
                        ));
                        return None;
                    }
                    (_, "out") => {
                        diagnostics.push(LoweringDiagnostic::refused(
                            slot_subject,
                            LoweringReason::OwnedByLaterPhase {
                                capability: "a Mod Matrix slot sourced from a module output \
                                             that is not an LFO's",
                                owner: "Phase 7",
                            },
                        ));
                        return None;
                    }
                    // V1 reads a member that is not an output port as the module's live
                    // parameter, normalized through its descriptor.
                    _ => {
                        diagnostics.push(LoweringDiagnostic::refused(
                            slot_subject,
                            LoweringReason::OwnedByLaterPhase {
                                capability: "a Mod Matrix slot reading a module parameter as \
                                             its source",
                                owner: "Phase 7",
                            },
                        ));
                        return None;
                    }
                }
            }
        };

        let lowered = match modulation_target(destination.module_type, destination.param.as_str()) {
            Ok(lowered) => lowered,
            Err(capability) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    parameter(&dest_key),
                    LoweringReason::OwnedByLaterPhase {
                        capability,
                        owner: "Phase 7",
                    },
                ));
                return None;
            }
        };
        let depth = quantity(
            ModulationDepth::new(lowered.unit, amount * lowered.scale),
            parameter(&amount_key),
            diagnostics,
        )?;
        routes.push(ModulationRoute {
            source,
            target,
            parameter: lowered.parameter,
            depth,
            subject: parameter(&dest_key),
        });
    }
    Some(routes)
}

/// Lower the Mod Grid instances V1's builder returns, as they act on one instrument.
///
/// Asked of V1's own builder rather than of the pool, as the refusal before this slice was:
/// an instance is what `build_mod_grid_runtime` returns, its targets are resolved as V1
/// resolves them — a module-backed instrument parameter already carries the address V1
/// writes — and its hosted modules are the modules V1 built, read back through their own
/// parameters rather than through the pool's saved map.
///
/// A global instance whose hosted LFOs feed module-backed targets on this instrument lowers
/// to global-scope LFO nodes and edges; a target on another instrument is that instrument's
/// and lowers to nothing here, as its notes do. A track-scoped instance, a track, master or
/// channel-level target, a macro, transport, MIDI CC or audio-tap source, an injection into a
/// hosted module, a cable into one, and a hosted module other than an LFO are each refused by
/// name. A target with no source is the `continue` V1 takes for it.
pub fn lower_mod_grid(
    song: &synth_sequencer::Song,
    instrument: InstrumentId,
    modules: &[ModuleState],
) -> SongModulators {
    use synth_engine::mod_grid::ModSource;

    let mut lowered = SongModulators::default();
    // The patch's modules by identity, for the dangling-target check. A spelling that does
    // not parse is refused by the graph lowering; here it simply holds nothing.
    let present: Vec<ModuleId> = modules
        .iter()
        .filter_map(|module| module.id.parse::<ModuleId>().ok())
        .collect();
    let runtime = crate::mod_grid_build::build_mod_grid_runtime(song);
    for instance in &runtime.instances {
        let graph = instance.graph_id;
        let subject = || ProjectSubject::ModGraph {
            graph,
            host_track: instance.host_track,
        };
        let node_subject = |node: ModNodeId| ProjectSubject::ModGraphNode { graph, node };

        if instance.host_track.is_some() {
            lowered.refuse(
                subject(),
                "a track-scoped Mod Grid graph, which runs once per assigned track against \
                 that track's controls",
                "Phase 8, with the mixer and bus model",
            );
            continue;
        }
        if !instance.injections.is_empty() {
            lowered.refuse(
                subject(),
                "a Mod Grid macro, transport, MIDI CC or audio-tap source driving a hosted \
                 module's input",
                "Phase 7",
            );
            continue;
        }
        // A cable between two hosted modules is a DSP connection V1 wires; V2's LFO declares
        // no input port to carry it.
        if let Some((node, _)) = instance
            .node_modules
            .iter()
            .find(|(_, module)| !instance.dsp.incoming_connections(*module).is_empty())
        {
            lowered.refuse(
                node_subject(*node),
                "a cable into a Mod Grid module's input port",
                "Phase 7",
            );
            continue;
        }

        for target in &instance.targets {
            // V1's pre-pass `continue`s past a sink with no cable.
            let Some(source) = &target.source else {
                continue;
            };

            // What the sink writes, as V1 resolved it — classified **before** the source, so
            // that a target this render does not carry is settled on its own terms: another
            // instrument's lowers to nothing whatever feeds it, and a module the patch lacks
            // is V1's no-op. An independent read found a macro into another instrument's
            // module refusing this one's render.
            let address = match &target.target {
                AutomationTarget::Track { .. } => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid target on a track's volume, pan or pitch",
                        "Phase 8, with the mixer and bus model",
                    );
                    continue;
                }
                AutomationTarget::Global(_) => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid target on the project's master volume",
                        "Phase 8, with the mixer and bus model",
                    );
                    continue;
                }
                AutomationTarget::Module {
                    instrument: owner, ..
                }
                | AutomationTarget::Instrument {
                    instrument: owner, ..
                } if *owner != instrument => {
                    // Another instrument's, as a placement on its track is.
                    continue;
                }
                AutomationTarget::Instrument {
                    param: AutoInstrumentParam::Volume | AutoInstrumentParam::Pan,
                    ..
                } => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid target on an instrument's channel volume or pan",
                        "Phase 8, with the mixer and bus model",
                    );
                    continue;
                }
                AutomationTarget::Module { .. } | AutomationTarget::Instrument { .. } => {
                    // The builder interned the address for every module-backed target; a
                    // module-backed one without it would be a builder defect, refused rather
                    // than guessed.
                    let Some(address) = target.dest_addr else {
                        lowered.refuse(
                            subject(),
                            "a Mod Grid module target V1's builder resolved to no address",
                            "Phase 7",
                        );
                        continue;
                    };
                    address
                }
            };
            // A module the patch lacks: `apply_mod_offset_addr` finds nothing and does nothing.
            if !present.contains(&ModuleId::new(address.module_type, address.instance)) {
                continue;
            }
            // V1 has one combine mode. Matched so that a second one is a compile error here.
            match target.combine {
                CombineMode::Add => {}
            }
            let (source_node, source_module, port) = match source {
                ModSource::Dsp(module, port) => {
                    let Some((node, _)) = instance.node_modules.iter().find(|(_, m)| m == module)
                    else {
                        // The builder names only modules it hosted, so this names one.
                        continue;
                    };
                    (*node, *module, *port)
                }
                ModSource::Constant(_) => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid macro knob as a modulation source",
                        "Phase 7",
                    );
                    continue;
                }
                ModSource::Transport(_) => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid transport source — beat phase, bar phase, tempo or \
                         position",
                        "Phase 7",
                    );
                    continue;
                }
                ModSource::InstrumentLevel(_) | ModSource::MasterLevel => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid audio tap, which follows a track's or the master's level",
                        "Phase 8, with the mixer and bus model",
                    );
                    continue;
                }
                ModSource::MidiCc { .. } => {
                    lowered.refuse(
                        subject(),
                        "a Mod Grid MIDI CC source, which reads the live controller state",
                        "Phase 9, with live ingress",
                    );
                    continue;
                }
            };

            // The source's port is not validated by the builder: a name that is not an
            // output reads zero in V1's pre-pass, so the sink lowers to nothing whatever its
            // target's law — settled before the law is asked, as the focused reread required.
            if port != synth_core::PortName::OUT {
                continue;
            }
            let lowered_target =
                match modulation_target(address.module_type, address.param.as_str()) {
                    Ok(lowered_target) => lowered_target,
                    Err(capability) => {
                        lowered.refuse(subject(), capability, "Phase 7");
                        continue;
                    }
                };
            // The hosted module, as V1 built it.
            let Some(module) = instance.dsp.get_module(source_module) else {
                continue;
            };
            if module.module_type() != ModuleType::Lfo {
                lowered.refuse(
                    node_subject(source_node),
                    "a Mod Grid node hosting a module other than an LFO",
                    "Phase 7",
                );
                continue;
            }
            let Some(node) = grid_node_address(graph, source_node) else {
                lowered.diagnostics.push(LoweringDiagnostic::refused(
                    node_subject(source_node),
                    LoweringReason::UnsupportedParameterValue {
                        value: format!(
                            "graph {} node {} lies outside the addressable range",
                            graph.0, source_node.0
                        ),
                    },
                ));
                lowered.refused = true;
                continue;
            };
            let Some(kind) = hosted_lfo(
                module,
                &|| node_subject(source_node),
                &mut lowered.diagnostics,
            ) else {
                lowered.refused = true;
                continue;
            };
            let Some(depth) = quantity(
                ModulationDepth::new(lowered_target.unit, target.amount * lowered_target.scale),
                subject(),
                &mut lowered.diagnostics,
            ) else {
                lowered.refused = true;
                continue;
            };
            if !lowered.nodes.iter().any(|(id, _)| *id == node) {
                lowered.nodes.push((node, kind));
            }
            lowered.routes.push(ModulationRoute {
                source: ModulationSource::Grid(node),
                target: ModuleId::new(address.module_type, address.instance),
                parameter: lowered_target.parameter,
                depth,
                subject: node_subject(source_node),
            });
        }
    }
    lowered
}

/// The node a hosted LFO lowers to, read from the module V1 built.
///
/// `get_params` hands back every setting typed and already passed through V1's own
/// `set_param` — the rate clamped into `LFO_RANGE`, the phase wrapped — so nothing is
/// re-derived from the pool's saved map here.
fn hosted_lfo(
    module: &dyn synth_core::PolyModule,
    subject: &impl Fn() -> ProjectSubject,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<IrNodeKind> {
    use synth_core::LfoParam;
    let mut waveform = None;
    let mut rate = None;
    let mut depth = None;
    let mut phase = None;
    let mut tempo_synced = None;
    for param in module.get_params() {
        match param {
            Param::Lfo(LfoParam::Waveform(value)) => waveform = Some(value),
            Param::Lfo(LfoParam::Rate(value)) => rate = Some(value),
            Param::Lfo(LfoParam::Depth(value)) => depth = Some(value),
            Param::Lfo(LfoParam::Phase(value)) => phase = Some(value),
            Param::Lfo(LfoParam::TempoSync(value)) => tempo_synced = Some(value),
            // Consumed as the saved module's are: the division matters only when synced,
            // and the retrigger only through a port no grid cable reaches.
            Param::Lfo(LfoParam::SyncDivision(_) | LfoParam::Retrigger(_)) => {}
            _ => {}
        }
    }
    let (Some(waveform), Some(rate), Some(depth), Some(phase), Some(tempo_synced)) =
        (waveform, rate, depth, phase, tempo_synced)
    else {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnsupportedParameterValue {
                value: "V1's LFO did not report every setting this lowering reads".to_owned(),
            },
        ));
        return None;
    };
    lfo_kind(
        waveform,
        rate,
        depth,
        phase,
        tempo_synced,
        subject,
        diagnostics,
    )
}

/// Reserved beside the instrument's voice scaler, outside saved-module and Mod Grid address
/// ranges, and per instrument: a controller macro is that instrument's MIDI channel state.
pub(super) const fn macro_node(
    slot: super::identity::InstrumentSlot,
    source: synth_core::MacroSource,
) -> (NodeId, IrNodeKind, ExecutionScope) {
    use synth_core::MacroSource;
    use synth_engine_v2::controller::{ControllerKind, NoteSource};
    let (tag, kind, scope) = match source {
        MacroSource::Velocity => (
            1,
            IrNodeKind::NoteSource {
                kind: NoteSource::Velocity,
            },
            ExecutionScope::Voice,
        ),
        MacroSource::NoteNumber => (
            2,
            IrNodeKind::NoteSource {
                kind: NoteSource::NoteNumber,
            },
            ExecutionScope::Voice,
        ),
        MacroSource::Aftertouch => (
            3,
            IrNodeKind::Controller {
                kind: ControllerKind::Aftertouch,
            },
            ExecutionScope::InstrumentInstance,
        ),
        MacroSource::ModWheel => (
            4,
            IrNodeKind::Controller {
                kind: ControllerKind::ModWheel,
            },
            ExecutionScope::InstrumentInstance,
        ),
        MacroSource::PitchBend => (
            5,
            IrNodeKind::Controller {
                kind: ControllerKind::PitchBend,
            },
            ExecutionScope::InstrumentInstance,
        ),
        MacroSource::PolyAftertouch => (
            6,
            IrNodeKind::NoteSource {
                kind: NoteSource::Pressure,
            },
            ExecutionScope::Voice,
        ),
    };
    (slot.macro_source(tag), kind, scope)
}
