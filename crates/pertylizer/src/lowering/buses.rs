//! The song's return buses and the sends into them, lowered onto V2's bus graph
//! (`P08-S004`, `SOUND-INV-034`).
//!
//! V1's return stage, read at its own boundary: every return the song declares is created in
//! the engine, fed or not; each block, every track's sends are resolved onto the track's
//! **instrument** — the map is keyed by instrument, and a later assigned track's list replaces
//! an earlier's — with the sixteenth resolved send the last one kept (`LIMIT-0024`); the
//! channel stage taps each resolved send pre-fader (`src × level`) or post-fader
//! (`src × (gain × level)`) and taps nothing from a channel that is not audible; each return
//! then runs its chain on the summed taps, applies its fader and pan under V1's constant-power
//! law, soft-clips, taps its bus-to-bus sends from that clipped output, and reaches the master
//! unless another return is soloed — all in a Kahn order recomputed per block.
//!
//! Here the same topology is compiled once: a bus is an entry sum, its chain, its strip and,
//! under the parity policy, its clipper, every node in the bus's own `Bus(tag)` scope; a
//! send is a `Send` or `PostFaderSend` node in the scope of the channel or bus it taps, cabled
//! into its target's entry. What V1 drops silently — a send past the cap, into a return the
//! song lacks, or from a return into itself — is refused by name. What V1 does per block by
//! list order — the last assigned track's list winning — is refused where two assigned tracks
//! differ, until ADR-0034 decides what a track and a channel are to each other.

use std::collections::HashMap;

use synth_engine::ModuleId;
use synth_engine_v2::ir::{ExecutionScope, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::quantities::{Amplitude, SendCount};
use synth_sequencer::{ReturnBusId, Song, TrackSend};

use super::diagnostics::{LoweringDiagnostic, LoweringReason, ProjectSubject};
use super::graph::{ChannelSend, GraphAccumulator, lower_insert};
use super::identity::{BusSlot, InstrumentSlot};
use super::render::OutputPolicy;
use crate::patch::InstrumentState;
use crate::project::GlobalProjectState;

/// Every return the song declares, with its place in the address space, in the song's list
/// order — which is V1's index order, the order its Kahn walk is seeded in.
#[derive(Debug, Clone, Default)]
#[must_use]
pub(super) struct BusSlots {
    by_id: HashMap<ReturnBusId, BusSlot>,
    order: Vec<ReturnBusId>,
}

impl BusSlots {
    /// The slot of one declared return.
    pub fn get(&self, bus: ReturnBusId) -> Option<BusSlot> {
        self.by_id.get(&bus).copied()
    }

    /// The declared returns, in the song's list order.
    pub fn order(&self) -> &[ReturnBusId] {
        &self.order
    }
}

/// Resolve every return the song declares to its slot, or `None` with the refusal recorded.
pub(super) fn bus_slots(
    song: &Song,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<BusSlots> {
    let mut slots = BusSlots::default();
    for bus in song.return_busses() {
        let slot = match BusSlot::of(bus.id) {
            Ok(slot) => slot,
            Err(error) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    ProjectSubject::ReturnBus { bus: bus.id },
                    LoweringReason::UnresolvedEndpoint {
                        spelling: error.to_string(),
                    },
                ));
                return None;
            }
        };
        if slots.by_id.insert(bus.id, slot).is_some() {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::ReturnBus { bus: bus.id },
                LoweringReason::UnresolvedEndpoint {
                    spelling: format!("return {} is declared twice", bus.id),
                },
            ));
            return None;
        }
        slots.order.push(bus.id);
    }
    Some(slots)
}

/// One instrument's sends, from the tracks assigned to it, or `None` with the refusal
/// recorded.
///
/// V1 keys its send lists by instrument and rebuilds them from **every** assigned track in
/// list order, the last one's list replacing the rest — an empty, muted or note-free track
/// included. Two assigned tracks with differing lists are therefore V1's list-order choice,
/// refused here by name until ADR-0034; equal lists are one list. From it, V1 resolves the
/// enabled sends whose target exists and keeps the first sixteen resolved; a resolved send at
/// zero level occupies a slot and contributes nothing. So the cap is checked over the resolved
/// list **before** the zero-level sends are dropped, and a send into a return the song does
/// not declare — which V1 skips silently — is refused by name.
pub(super) fn instrument_sends(
    saved: &InstrumentState,
    song: &Song,
    slots: &BusSlots,
    cap: SendCount,
    muted: bool,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Vec<ChannelSend>> {
    let assigned: Vec<&synth_sequencer::SequencerTrack> = song
        .tracks()
        .filter(|track| track.instrument == saved.id)
        .collect();
    let Some(first) = assigned.first() else {
        return Some(Vec::new());
    };
    for other in assigned.iter().skip(1) {
        if other.sends != first.sends {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Track {
                    track: other.id,
                    name: other.name.clone(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "an instrument assigned to two tracks with differing sends, \
                                 which V1 resolves by taking the last track's list each block",
                    owner: "ADR-0034, with the track, source and channel ownership model",
                },
            ));
            return None;
        }
    }
    let subject = || ProjectSubject::Track {
        track: first.id,
        name: first.name.clone(),
    };
    let mut resolved: Vec<&TrackSend> = Vec::with_capacity(first.sends.len());
    for send in &first.sends {
        if !send.enabled {
            continue;
        }
        if slots.get(send.target).is_none() {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnresolvedEndpoint {
                    spelling: format!(
                        "a send into return {}, which the song does not declare and V1 drops \
                         silently",
                        send.target
                    ),
                },
            ));
            return None;
        }
        resolved.push(send);
    }
    if resolved.len() > usize::try_from(cap.get()).unwrap_or(usize::MAX) {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnsupportedParameterValue {
                value: format!(
                    "{} resolved sends where the profile admits {} per channel; V1 keeps the \
                     first {} and drops the rest (LIMIT-0024)",
                    resolved.len(),
                    cap.get(),
                    cap.get()
                ),
            },
        ));
        return None;
    }
    let mut sends = Vec::with_capacity(resolved.len());
    for send in resolved {
        // A resolved send at zero level is multiplied by that zero in V1: it occupied a slot
        // above and contributes nothing here.
        if send.level == synth_core::NormalizedValue::MIN {
            continue;
        }
        let level = match Amplitude::new(send.level.as_f32()) {
            Ok(level) => level,
            Err(error) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: error.to_string(),
                    },
                ));
                return None;
            }
        };
        let target = slots.get(send.target)?;
        sends.push(ChannelSend {
            entry: target.entry(),
            level,
            pre_fader: send.pre_fader,
            muted,
        });
    }
    Some(sends)
}

/// The node an instrument's `k`th lowered send compiles to, for the lanes that fan out to
/// its post-fader sends.
pub(super) fn send_nodes(slot: InstrumentSlot, sends: &[ChannelSend]) -> Vec<(NodeId, bool)> {
    sends
        .iter()
        .enumerate()
        .filter_map(|(k, send)| {
            u16::try_from(k)
                .ok()
                .map(|k| (slot.send(k), send.pre_fader))
        })
        .collect()
}

/// Lower every declared return: its entry, its chain, its strip, its clipper under the
/// parity policy, its cable into the master unless another return is soloed, and its
/// bus-to-bus sends. Returns whether anything refused the plan.
pub(super) fn lower_buses(
    graph: &mut GraphAccumulator,
    song: &Song,
    global: &GlobalProjectState,
    policy: OutputPolicy,
    slots: &BusSlots,
    master: NodeId,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> bool {
    let any_return_solo = song.return_busses().iter().any(|bus| bus.solo);
    let mut refused = false;
    for bus in song.return_busses() {
        let Some(slot) = slots.get(bus.id) else {
            continue;
        };
        let scope = ExecutionScope::Bus(slot.tag());
        let subject = || ProjectSubject::ReturnBus { bus: bus.id };
        // V1's own reads: the volume is a `NormalizedValue` and the pan a `BipolarValue`, so
        // only a value that is not a number can fail either, refused by name.
        let fader = match Amplitude::new(bus.volume.as_f32()) {
            Ok(level) => level,
            Err(error) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: error.to_string(),
                    },
                ));
                refused = true;
                continue;
            }
        };
        let pan = match synth_engine_v2::controller::BipolarLevel::new(bus.pan.as_f32()) {
            Ok(pan) => pan,
            Err(error) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: error.to_string(),
                    },
                ));
                refused = true;
                continue;
            }
        };

        // The chain, in V1's order: the entry sum, the return's effects in the order the
        // project stores them, the strip, and the clipper V1 applies to the return's output.
        let mut chain: Vec<NodeId> = vec![slot.entry()];
        if !place_in(
            graph,
            scope,
            &subject,
            slot.entry(),
            IrNodeKind::Mix,
            diagnostics,
        ) {
            refused = true;
        }
        let effects = global
            .return_bus_effects
            .iter()
            .find(|state| state.id == bus.id.0)
            .map_or(&[][..], |state| state.effects.as_slice());
        let mut placed: Vec<ModuleId> = Vec::with_capacity(effects.len());
        for module in effects {
            let Ok(id) = module.id.parse::<ModuleId>() else {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnresolvedEndpoint {
                        spelling: format!(
                            "effect id {:?} does not parse as a module identity",
                            module.id
                        ),
                    },
                ));
                refused = true;
                continue;
            };
            let module_subject = || ProjectSubject::ReturnBusModule {
                bus: bus.id,
                module: id,
            };
            if id.module_type != module.module_type {
                diagnostics.push(LoweringDiagnostic::refused(
                    module_subject(),
                    LoweringReason::UnresolvedEndpoint {
                        spelling: format!(
                            "effect {:?} is declared as {:?} but its id names {:?}",
                            module.id, module.module_type, id.module_type
                        ),
                    },
                ));
                refused = true;
                continue;
            }
            if placed.contains(&id) {
                diagnostics.push(LoweringDiagnostic::refused(
                    module_subject(),
                    LoweringReason::UnresolvedEndpoint {
                        spelling: format!("effect {id} is declared twice in one return"),
                    },
                ));
                refused = true;
                continue;
            }
            placed.push(id);
            let node = match slot.module(id) {
                Ok(node) => node,
                Err(error) => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        module_subject(),
                        LoweringReason::UnresolvedEndpoint {
                            spelling: error.to_string(),
                        },
                    ));
                    refused = true;
                    continue;
                }
            };
            let parameter = |key: &str| ProjectSubject::ReturnBusParameter {
                bus: bus.id,
                module: id,
                parameter: key.to_owned(),
            };
            match lower_insert(&module_subject, &parameter, module, diagnostics) {
                Some(kind) => {
                    if !place_in(graph, scope, &subject, node, kind, diagnostics) {
                        refused = true;
                    }
                    chain.push(node);
                }
                None => refused = true,
            }
        }
        if !place_in(
            graph,
            scope,
            &subject,
            slot.strip(),
            IrNodeKind::Channel {
                fader,
                pan,
                muted: bus.mute,
            },
            diagnostics,
        ) {
            refused = true;
        }
        chain.push(slot.strip());
        if policy == OutputPolicy::Parity {
            if !place_in(
                graph,
                scope,
                &subject,
                slot.soft_clip(),
                IrNodeKind::SoftClip,
                diagnostics,
            ) {
                refused = true;
            }
            chain.push(slot.soft_clip());
        }
        for pair in chain.windows(2) {
            graph.connect(
                (pair[0], PortId::FIRST),
                (pair[1], PortId::FIRST),
                SignalDomain::Audio,
            );
        }
        let Some(output) = chain.last().copied() else {
            continue;
        };
        // V1's return solo gates the master sum alone; the bus-to-bus taps still flow.
        if !any_return_solo || bus.solo {
            graph.connect(
                (output, PortId::FIRST),
                (master, PortId::FIRST),
                SignalDomain::Audio,
            );
        }

        // Bus-to-bus sends, always post-fader and post-clip in V1, each from this return's
        // output into the target's entry. A disabled send is V1's bypass and a send at zero
        // level is multiplied by that zero; a send into a return the song does not declare,
        // or into itself, is what V1 skips silently, refused here by name.
        let mut lowered = 0_u8;
        for send in &bus.sends {
            if !send.enabled {
                continue;
            }
            let Some(target) = slots.get(send.target) else {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnresolvedEndpoint {
                        spelling: format!(
                            "a send into return {}, which the song does not declare and V1 \
                             drops silently",
                            send.target
                        ),
                    },
                ));
                refused = true;
                continue;
            };
            if send.target == bus.id {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: "a send from a return into itself, which V1 drops silently and \
                                V2 refuses as a cycle"
                            .to_owned(),
                    },
                ));
                refused = true;
                continue;
            }
            if send.level == synth_core::NormalizedValue::MIN {
                continue;
            }
            let level = match Amplitude::new(send.level.as_f32()) {
                Ok(level) => level,
                Err(error) => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        subject(),
                        LoweringReason::UnsupportedParameterValue {
                            value: error.to_string(),
                        },
                    ));
                    refused = true;
                    continue;
                }
            };
            let Some(node) = slot.send(lowered) else {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: "more bus-to-bus sends than the address space holds".to_owned(),
                    },
                ));
                refused = true;
                break;
            };
            lowered = lowered.saturating_add(1);
            if !place_in(
                graph,
                scope,
                &subject,
                node,
                IrNodeKind::Send {
                    level,
                    muted: false,
                },
                diagnostics,
            ) {
                refused = true;
            }
            graph.connect(
                (output, PortId::FIRST),
                (node, PortId::FIRST),
                SignalDomain::Audio,
            );
            graph.connect(
                (node, PortId::FIRST),
                (target.entry(), PortId::FIRST),
                SignalDomain::Audio,
            );
        }
    }
    refused
}

/// Add a node to the bus's scope, or refuse the plan naming the address another holds.
fn place_in(
    graph: &mut GraphAccumulator,
    scope: ExecutionScope,
    subject: &dyn Fn() -> ProjectSubject,
    id: NodeId,
    kind: IrNodeKind,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> bool {
    match graph.node(id, kind, scope) {
        Ok(()) => true,
        Err(taken) => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnresolvedEndpoint {
                    spelling: format!("{taken} is claimed by two different nodes"),
                },
            ));
            false
        }
    }
}

/// V1's return processing order: Kahn's algorithm over the enabled, resolved bus-to-bus
/// sends, seeded in index order, which is `resolve_return_routing` term for term. A return
/// left out by a cycle is appended in index order, as V1 appends it.
fn v1_return_order(song: &Song, slots: &BusSlots) -> Vec<ReturnBusId> {
    let order = slots.order();
    let position = |id: ReturnBusId| order.iter().position(|bus| *bus == id);
    let n = order.len();
    let mut sends: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0_usize; n];
    for bus in song.return_busses() {
        let Some(from) = position(bus.id) else {
            continue;
        };
        for send in &bus.sends {
            if !send.enabled {
                continue;
            }
            let Some(to) = position(send.target) else {
                continue;
            };
            if to == from {
                continue;
            }
            sends[from].push(to);
            indegree[to] += 1;
        }
    }
    let mut walked: Vec<usize> = (0..n).filter(|index| indegree[*index] == 0).collect();
    let mut read = 0;
    while read < walked.len() {
        let node = walked[read];
        read += 1;
        for target in sends[node].clone() {
            indegree[target] -= 1;
            if indegree[target] == 0 {
                walked.push(target);
            }
        }
    }
    if walked.len() < n {
        for index in 0..n {
            if !walked.contains(&index) {
                walked.push(index);
            }
        }
    }
    walked.into_iter().map(|index| order[index]).collect()
}

/// The marks for every sum whose V1 order is not V2's (`SOUND-INV-008`).
///
/// V1 sums the master in list order — the instruments as the project lists them, then the
/// returns in its Kahn order — and each return's input in instrument list order then Kahn
/// order; V2 sums every port's cables in ascending source identity. A float sum of three or
/// more terms depends on its order, so a sum of that many whose two orders differ is a
/// marked difference, not a translation; two terms sum the same either way. The terms are
/// named by the node whose cable enters the sum, which is exactly what V2 orders by.
pub(super) fn summation_order_marks(
    instruments: &[InstrumentState],
    song: &Song,
    slots: &BusSlots,
    policy: OutputPolicy,
) -> Vec<LoweringDiagnostic> {
    let mut marks = Vec::new();
    let any_return_solo = song.return_busses().iter().any(|bus| bus.solo);
    let returns = v1_return_order(song, slots);
    let instrument_term = |saved: &InstrumentState| {
        InstrumentSlot::of(saved.id).ok().map(|slot| match policy {
            OutputPolicy::Parity => slot.soft_clip(),
            OutputPolicy::Headroom => slot.channel(),
        })
    };
    let return_term = |id: ReturnBusId| {
        slots.get(id).map(|slot| match policy {
            OutputPolicy::Parity => slot.soft_clip(),
            OutputPolicy::Headroom => slot.strip(),
        })
    };
    let differs = |terms: &[NodeId]| {
        let mut sorted = terms.to_vec();
        sorted.sort_unstable();
        terms.len() >= 3 && sorted != terms
    };

    // The master: every instrument's channel in list order, then every return that reaches
    // it in V1's order.
    let mut master: Vec<NodeId> = instruments.iter().filter_map(instrument_term).collect();
    for id in &returns {
        let reaches = song
            .return_busses()
            .iter()
            .find(|bus| bus.id == *id)
            .is_some_and(|bus| !any_return_solo || bus.solo);
        if reaches && let Some(term) = return_term(*id) {
            master.push(term);
        }
    }
    if differs(&master) {
        marks.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "three or more terms into the master whose V1 order — instruments \
                             in list order, then returns in V1's Kahn order — is not identity \
                             order, which V1 sums in the first and V2 in the second",
                owner: "the first A/B consumer, under the corpus's intentional-correction class",
            },
        ));
    }

    // Each return's input: every instrument's taps in instrument list order, then the
    // bus-to-bus taps in V1's return order.
    for bus in song.return_busses() {
        let mut terms: Vec<NodeId> = Vec::new();
        for saved in instruments {
            let Ok(slot) = InstrumentSlot::of(saved.id) else {
                continue;
            };
            // The one list V1 keys by instrument, from its assigned tracks; a differing pair
            // is refused before this runs, so the last assigned track's list is every one's.
            let Some(track) = song.tracks().filter(|t| t.instrument == saved.id).last() else {
                continue;
            };
            let mut k = 0_u16;
            for send in &track.sends {
                if !send.enabled || slots.get(send.target).is_none() {
                    continue;
                }
                if send.level != synth_core::NormalizedValue::MIN {
                    if send.target == bus.id {
                        terms.push(slot.send(k));
                    }
                    k = k.saturating_add(1);
                }
            }
        }
        for id in &returns {
            let Some(from) = song.return_busses().iter().find(|b| b.id == *id) else {
                continue;
            };
            let Some(from_slot) = slots.get(*id) else {
                continue;
            };
            let mut k = 0_u8;
            for send in &from.sends {
                if !send.enabled || slots.get(send.target).is_none() || send.target == *id {
                    continue;
                }
                if send.level != synth_core::NormalizedValue::MIN {
                    if send.target == bus.id
                        && let Some(node) = from_slot.send(k)
                    {
                        terms.push(node);
                    }
                    k = k.saturating_add(1);
                }
            }
        }
        if differs(&terms) {
            marks.push(LoweringDiagnostic::unrepresented(
                ProjectSubject::ReturnBus { bus: bus.id },
                LoweringReason::OwnedByLaterPhase {
                    capability: "three or more sends into one return whose V1 order — \
                                 instruments in list order, then returns in V1's Kahn order — \
                                 is not identity order, which V1 sums in the first and V2 in \
                                 the second",
                    owner: "the first A/B consumer, under the corpus's intentional-correction \
                            class",
                },
            ));
        }
    }
    marks
}
