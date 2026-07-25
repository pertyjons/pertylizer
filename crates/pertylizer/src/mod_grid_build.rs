//! Build the engine-side [`ModGridRuntime`] from the serializable `Song`
//! mod-graph pool.
//!
//! The running instances own real `Box<dyn PolyModule>` DSP, which allocates to
//! build — so this runs off the audio thread (on the GUI thread's per-frame
//! sync, or once at offline-render setup) and the result is shipped to the
//! engine as a pre-built [`synth_engine::EngineCommand::SetModGrid`].

use std::collections::HashMap;

use synth_core::ModuleParam;
use synth_core::{DestAddr, ModuleType, PortName};
use synth_engine::ModuleId;
use synth_engine::graph::ModuleGraph;
use synth_engine::mod_grid::{
    InputInjection, ModGridInstance, ModGridRuntime, ModSource, ResolvedTarget,
};
use synth_sequencer::{
    AutoInstrumentParam, AutomationTarget, ModGraph, ModGraphScope, ModNodeConfig, ModNodeId, Song,
    TARGET_INPUT_PORT, TrackId,
};

use crate::module_factory::create_voice_module;

/// Map a module-backed [`AutoInstrumentParam`] to the *first* module of the
/// relevant type + its descriptor `type_id` (the A1 "instrument param" convention
/// the automation-lane path uses). Volume/Pan are channel-level, not a module
/// param, so they return `None`. Single source of truth for both the build-time
/// [`DestAddr`] and the GUI grid-dest marker.
pub(crate) fn instrument_param_module(
    param: AutoInstrumentParam,
) -> Option<(ModuleType, u16, &'static str)> {
    let (module_type, type_id) = match param {
        AutoInstrumentParam::FilterCutoff => (ModuleType::Filter, "cutoff"),
        AutoInstrumentParam::FilterResonance => (ModuleType::Filter, "resonance"),
        AutoInstrumentParam::Attack => (ModuleType::Envelope, "attack"),
        AutoInstrumentParam::Decay => (ModuleType::Envelope, "decay"),
        AutoInstrumentParam::Sustain => (ModuleType::Envelope, "sustain"),
        AutoInstrumentParam::Release => (ModuleType::Envelope, "release"),
        AutoInstrumentParam::Volume | AutoInstrumentParam::Pan => return None,
    };
    Some((module_type, 1, type_id))
}

/// Build the DSP address of a module-backed [`AutoInstrumentParam`] (`None` for
/// the channel-level Volume/Pan).
fn instrument_param_dest(param: AutoInstrumentParam) -> Option<DestAddr> {
    instrument_param_module(param)
        .map(|(module_type, instance, type_id)| DestAddr::new(module_type, instance, type_id))
}

/// Build every running mod-grid instance from the current song pool. A Global
/// graph yields one instance; a Track graph yields one per assigned track.
/// Instances with no resolvable routing are skipped (they'd process DSP for
/// nothing).
#[must_use]
pub fn build_mod_grid_runtime(song: &Song) -> ModGridRuntime {
    let mut instances = Vec::new();
    for graph in song.mod_graphs() {
        match graph.scope {
            ModGraphScope::Global => {
                if let Some(inst) = build_instance(graph, None, song) {
                    instances.push(inst);
                }
            }
            ModGraphScope::Track => {
                for &track in &graph.assigned_tracks {
                    if let Some(inst) = build_instance(graph, Some(track), song) {
                        instances.push(inst);
                    }
                }
            }
        }
    }

    // Pre-key the per-block offset accumulators here, off the audio thread, so the
    // engine's SetModGrid handler never inserts (allocates) on the audio thread.
    let mut runtime = ModGridRuntime {
        instances,
        ..Default::default()
    };
    runtime.prekey_offsets();
    runtime
}

/// Build one running instance of `graph` for the given host track (`None` =
/// Global). Returns `None` if the instance drives no routing.
fn build_instance(
    graph: &ModGraph,
    host_track: Option<TrackId>,
    song: &Song,
) -> Option<ModGridInstance> {
    let mut dsp = ModuleGraph::new();
    let mut node_module_ids: HashMap<ModNodeId, ModuleId> = HashMap::new();

    // 1. Build the hosted control-rate modules and apply their persisted params.
    for (&node_id, config) in &graph.nodes {
        if let ModNodeConfig::Module(m) = config {
            let Some((mut module, descriptor)) = create_voice_module(m.module_type) else {
                tracing::warn!(
                    graph_id = graph.id.0,
                    node_id = node_id.0,
                    module_type = ?m.module_type,
                    "dropping unhostable Mod Grid module node"
                );
                continue;
            };
            // Apply the persisted per-node seed to random-family modules, folding
            // in the host track so the same graph on several tracks decorrelates
            // (a Global instance has no host, so it uses the seed verbatim).
            // Deterministic → an offline render reproduces the live sequence. A
            // module with a `seed` *param* set below still overrides this.
            if let Some(seed) = m.seed {
                let host = host_track.map_or(0u64, |t| u64::from(t.0));
                module.set_seed(seed ^ host.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            }
            let module_id = dsp.add_module(module);
            for (type_id, &value) in &m.params {
                if let Some(p) = descriptor.parameters.iter().find(|p| p.type_id == *type_id) {
                    dsp.set_param(module_id, p.id.with_f32(value));
                } else {
                    tracing::warn!(
                        graph_id = graph.id.0,
                        node_id = node_id.0,
                        parameter = type_id,
                        "dropping unknown Mod Grid module parameter"
                    );
                }
            }
            node_module_ids.insert(node_id, module_id);
        }
        // Cheap grid-native nodes (Macro/Transport/MidiCc/AudioTap) are hosted
        // in a later phase; Target nodes are routing sinks, not DSP modules.
    }

    // 2. Wire cables between two hosted module nodes (cables into a Target node
    //    are routing edges, resolved in step 3, not DSP connections).
    for cable in &graph.connections {
        let (Some(&from_id), Some(&to_id)) = (
            node_module_ids.get(&cable.from),
            node_module_ids.get(&cable.to),
        ) else {
            continue;
        };
        if let Err(error) = dsp.connect(
            from_id,
            cable.from_port.clone(),
            to_id,
            cable.to_port.clone(),
        ) {
            tracing::warn!(
                graph_id = graph.id.0,
                from_node = cable.from.0,
                to_node = cable.to.0,
                %error,
                "dropping invalid Mod Grid runtime connection"
            );
        }
    }

    // 2b. Cheap→module cables: a cheap source (Macro/Transport/AudioTap) feeding a
    //     hosted module's control input is injected as a block-constant value —
    //     the source is not a DSP module, so it can't be a real graph connection.
    let mut injections = Vec::new();
    for cable in &graph.connections {
        // `to` must be a hosted module; `from` must NOT be (else step 2 wired it).
        let Some(&to_id) = node_module_ids.get(&cable.to) else {
            continue;
        };
        if node_module_ids.contains_key(&cable.from) {
            continue;
        }
        if let Some(source) = resolve_source(graph, cable, &node_module_ids, song) {
            let port = PortName::from(cable.to_port.as_str());
            dsp.push_input_injection(to_id, port);
            injections.push(InputInjection {
                module: to_id,
                port,
                source,
                smooth: 0.0,
            });
        }
    }

    // 3. Resolve the routing sinks (Target nodes).
    let mut targets = Vec::new();
    for (&node_id, config) in &graph.nodes {
        let ModNodeConfig::Target(t) = config else {
            continue;
        };
        // Resolve the address against the host track: relative `Track { None }`
        // becomes the host (Track scope) or is dropped (Global scope, no host).
        let Some(resolved) = t.target.resolved(host_track) else {
            continue;
        };
        // Find the single incoming cable feeding this target's input port and
        // resolve its source node (hosted module output or a cheap source).
        let source = graph
            .connections
            .iter()
            .find(|c| c.to == node_id && c.to_port == TARGET_INPUT_PORT)
            .and_then(|c| resolve_source(graph, c, &node_module_ids, song));
        // A Module (or module-backed Instrument) target resolves to a DSP address
        // once, here (off the audio thread) — `DestAddr::new` interns the param id
        // under a global lock, so it must never run in the RT pre-pass.
        let dest_addr = match &resolved {
            AutomationTarget::Module {
                module_type,
                instance,
                param_id,
                ..
            } => Some(DestAddr::new(*module_type, *instance, param_id.as_str())),
            AutomationTarget::Instrument { param, .. } => instrument_param_dest(*param),
            _ => None,
        };
        targets.push(ResolvedTarget {
            source,
            target: resolved,
            amount: t.amount,
            combine: t.combine,
            smooth: 0.0,
            dest_addr,
        });
    }

    // An instance with no routing sink writes nothing to the automation space,
    // even if it has injections feeding modules — skip it (no wasted DSP).
    if targets.is_empty() {
        return None;
    }
    Some(ModGridInstance {
        graph_id: graph.id,
        host_track,
        dsp,
        injections,
        targets,
    })
}

/// Resolve the cable's source node to a [`ModSource`]: a hosted module's output
/// port, or one of the cheap grid-native sources. Cheap sources feed Target
/// sinks directly; they are not injected into the DSP graph (so a cheap→module
/// cable resolves to no source here).
fn resolve_source(
    graph: &ModGraph,
    cable: &synth_sequencer::ModConnection,
    node_module_ids: &HashMap<ModNodeId, ModuleId>,
    song: &Song,
) -> Option<ModSource> {
    match graph.nodes.get(&cable.from)? {
        ModNodeConfig::Module(_) => node_module_ids
            .get(&cable.from)
            .map(|&mid| ModSource::Dsp(mid, PortName::from(cable.from_port.as_str()))),
        ModNodeConfig::Macro(m) => Some(ModSource::Constant(m.value.as_f32())),
        ModNodeConfig::Transport(tn) => Some(ModSource::Transport(tn.source)),
        ModNodeConfig::AudioTap(tap) => Some(match tap.source {
            synth_sequencer::AudioTapSource::Master => ModSource::MasterLevel,
            synth_sequencer::AudioTapSource::Track(track) => {
                // The Track tap follows that track's instrument output.
                let inst = song.tracks().find(|t| t.id == track)?.instrument;
                ModSource::InstrumentLevel(inst)
            }
        }),
        ModNodeConfig::MidiCc(m) => Some(ModSource::MidiCc {
            cc: m.cc.as_u8(),
            channel: m.channel,
        }),
        ModNodeConfig::Target(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::ModuleType;
    use synth_sequencer::{AutomationTarget, ModConnection, ModTarget, ModuleNode, TrackParam};

    /// A module-backed `Instrument` target (FilterCutoff) resolves to a per-voice
    /// `dest_addr` (first Filter's cutoff), while Volume takes the channel-level
    /// path (no `dest_addr`).
    #[test]
    fn instrument_target_dest_addr_resolution() {
        let mut song = Song::new("t");
        let gid = song.create_mod_graph("g");
        {
            let g = song.mod_graph_mut(gid).unwrap();
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Macro(synth_sequencer::MacroNode {
                    name: "M".into(),
                    value: 1.0.into(),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Instrument {
                        instrument: synth_sequencer::InstrumentId::new(0),
                        param: AutoInstrumentParam::FilterCutoff,
                    },
                    amount: 1.0,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(2),
                ModNodeConfig::Macro(synth_sequencer::MacroNode {
                    name: "V".into(),
                    value: 1.0.into(),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(3),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Instrument {
                        instrument: synth_sequencer::InstrumentId::new(0),
                        param: AutoInstrumentParam::Volume,
                    },
                    amount: 0.5,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(1),
                "in",
            ))
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(2),
                "out",
                ModNodeId::new(3),
                "in",
            ))
            .unwrap();
        }
        let runtime = build_mod_grid_runtime(&song);
        let targets = &runtime.instances[0].targets;
        let cutoff = targets
            .iter()
            .find(|t| {
                matches!(
                    t.target,
                    AutomationTarget::Instrument {
                        param: AutoInstrumentParam::FilterCutoff,
                        ..
                    }
                )
            })
            .expect("cutoff target");
        let volume = targets
            .iter()
            .find(|t| {
                matches!(
                    t.target,
                    AutomationTarget::Instrument {
                        param: AutoInstrumentParam::Volume,
                        ..
                    }
                )
            })
            .expect("volume target");
        assert!(
            cutoff.dest_addr.is_some(),
            "FilterCutoff must resolve a DestAddr"
        );
        assert!(
            volume.dest_addr.is_none(),
            "Volume is channel-level, no DestAddr"
        );
    }

    /// The persisted per-node `seed` reaches the built random-family module: two
    /// seeds decorrelate its output, and the same seed reproduces it (offline ==
    /// live). Processes the grid `dsp` directly — no oscillator (whose unison
    /// phase uses per-module deterministic RNG state) to confound determinism.
    #[test]
    fn per_node_seed_reaches_the_built_module() {
        use synth_core::{AudioBuffer, ProcessContext, SampleCount};
        let build_and_sum = |seed: u64| -> f32 {
            let mut song = Song::new("t");
            let gid = song.create_mod_graph("g");
            {
                let g = song.mod_graph_mut(gid).unwrap();
                g.try_insert_node(
                    ModNodeId::new(0),
                    ModNodeConfig::Module(ModuleNode {
                        module_type: ModuleType::RandomGates,
                        params: Default::default(),
                        seed: Some(seed),
                    }),
                )
                .unwrap();
                g.try_insert_node(
                    ModNodeId::new(1),
                    ModNodeConfig::Target(ModTarget {
                        target: AutomationTarget::Global(
                            synth_sequencer::GlobalParam::MasterVolume,
                        ),
                        amount: 1.0,
                        combine: Default::default(),
                    }),
                )
                .unwrap();
                g.try_connect(ModConnection::new(
                    ModNodeId::new(0),
                    "cv",
                    ModNodeId::new(1),
                    "in",
                ))
                .unwrap();
            }
            let mut runtime = build_mod_grid_runtime(&song);
            let inst = &mut runtime.instances[0];
            let Some(ModSource::Dsp(mid, port)) = inst.targets[0].source.as_ref() else {
                panic!("expected a Dsp routing source");
            };
            let (mid, port) = (*mid, *port);
            let ctx = ProcessContext {
                samples: SampleCount::new(64000),
                ..ProcessContext::default()
            };
            let mut scratch = AudioBuffer::new(64000);
            inst.dsp.process(&mut scratch, &ctx);
            let out = inst.dsp.get_module_output(mid, port).expect("cv output");
            (0..out.len()).map(|i| out.as_slice()[i]).sum::<f32>()
        };
        assert!(
            (build_and_sum(1) - build_and_sum(424_242)).abs() > 1e-6,
            "different per-node seeds must decorrelate the built module"
        );
        assert_eq!(
            build_and_sum(7),
            build_and_sum(7),
            "the same seed must reproduce the module output"
        );
    }

    /// A Global graph LFO → "this track" volume drops (no host) but a Track
    /// graph assigned to two tracks yields two instances, each resolved to its
    /// host, with the LFO wired as the routing source.
    #[test]
    fn track_scope_expands_per_assignment_and_resolves_host() {
        let mut song = Song::new("t");
        let t0 = song.create_track("a");
        let t1 = song.create_track("b");
        let gid = song.create_mod_graph("wobble");
        song.set_mod_graph_scope(gid, ModGraphScope::Track);
        song.assign_mod_graph(gid, &[t0, t1]);
        {
            let g = song.mod_graph_mut(gid).unwrap();
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Module(ModuleNode {
                    module_type: ModuleType::Lfo,
                    params: Default::default(),
                    seed: None,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Track {
                        track: None,
                        param: TrackParam::Volume,
                    },
                    amount: 0.25,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(1),
                TARGET_INPUT_PORT,
            ))
            .unwrap();
        }

        let runtime = build_mod_grid_runtime(&song);
        assert_eq!(runtime.instances.len(), 2);
        let hosts: Vec<_> = runtime.instances.iter().map(|i| i.host_track).collect();
        assert!(hosts.contains(&Some(t0)) && hosts.contains(&Some(t1)));
        for inst in &runtime.instances {
            assert_eq!(inst.targets.len(), 1);
            let rt = &inst.targets[0];
            // The relative target resolved to this instance's host track.
            assert_eq!(
                rt.target,
                AutomationTarget::Track {
                    track: inst.host_track,
                    param: TrackParam::Volume,
                }
            );
            // The LFO output is wired as the source.
            assert!(rt.source.is_some());
            assert_eq!(rt.amount, 0.25);
        }
    }

    #[test]
    fn global_scope_drops_relative_track_targets() {
        let mut song = Song::new("t");
        let gid = song.create_mod_graph("g");
        // Global scope (default). A relative "this track" target has no host.
        {
            let g = song.mod_graph_mut(gid).unwrap();
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Module(ModuleNode {
                    module_type: ModuleType::Lfo,
                    params: Default::default(),
                    seed: None,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Track {
                        track: None,
                        param: TrackParam::Volume,
                    },
                    amount: 1.0,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(1),
                TARGET_INPUT_PORT,
            ))
            .unwrap();
        }
        // The only target is unresolvable → the instance has no routing → skipped.
        let runtime = build_mod_grid_runtime(&song);
        assert!(runtime.is_empty());
    }

    #[test]
    fn cheap_sources_resolve_to_mod_source() {
        use synth_sequencer::{MacroNode, TransportNode, TransportSource};
        let mut song = Song::new("t");
        let gid = song.create_mod_graph("g");
        {
            let g = song.mod_graph_mut(gid).unwrap();
            // A Macro source → master volume.
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Macro(MacroNode {
                    name: "M".into(),
                    value: 0.6.into(),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Global(synth_sequencer::GlobalParam::MasterVolume),
                    amount: 0.5,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(1),
                "in",
            ))
            .unwrap();
            // A Transport source → master volume (a second target).
            g.try_insert_node(
                ModNodeId::new(2),
                ModNodeConfig::Transport(TransportNode {
                    source: TransportSource::BeatPhase,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(3),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Global(synth_sequencer::GlobalParam::MasterVolume),
                    amount: 1.0,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(2),
                "out",
                ModNodeId::new(3),
                "in",
            ))
            .unwrap();
        }
        let runtime = build_mod_grid_runtime(&song);
        assert_eq!(runtime.instances.len(), 1);
        let targets = &runtime.instances[0].targets;
        assert_eq!(targets.len(), 2);
        let mut saw_constant = false;
        let mut saw_transport = false;
        for rt in targets {
            match &rt.source {
                Some(ModSource::Constant(v)) => {
                    assert!((*v - 0.6).abs() < 1e-6);
                    saw_constant = true;
                }
                Some(ModSource::Transport(TransportSource::BeatPhase)) => saw_transport = true,
                other => panic!("unexpected source: {}", other.is_some()),
            }
        }
        assert!(saw_constant && saw_transport);
    }

    /// Replicate the exact running-app scenario that failed live: a track-scoped
    /// graph assigned to track 0 with a Macro → relative "this track" Volume
    /// target. The built runtime's instance must resolve the target to
    /// `Track { Some(track0) }` (not leave it relative) or the engine drops it.
    #[test]
    fn track_scope_macro_to_relative_volume_resolves() {
        use synth_sequencer::{MacroNode, TrackParam};
        let mut song = Song::new("t");
        let t0 = song.create_track("lead");
        let gid = song.create_mod_graph("Mod 1");
        song.set_mod_graph_scope(gid, ModGraphScope::Track);
        song.assign_mod_graph(gid, &[t0]);
        {
            let g = song.mod_graph_mut(gid).unwrap();
            g.try_insert_node(
                ModNodeId::new(6),
                ModNodeConfig::Macro(MacroNode {
                    name: "M".into(),
                    value: 0.43.into(),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(8),
                ModNodeConfig::Target(ModTarget {
                    // Relative "this track" — track: None.
                    target: AutomationTarget::Track {
                        track: None,
                        param: TrackParam::Volume,
                    },
                    amount: 0.8,
                    combine: Default::default(),
                }),
            )
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(6),
                "out",
                ModNodeId::new(8),
                "in",
            ))
            .unwrap();
        }
        let runtime = build_mod_grid_runtime(&song);
        assert_eq!(
            runtime.instances.len(),
            1,
            "one instance for the one assignment"
        );
        let inst = &runtime.instances[0];
        assert_eq!(inst.host_track, Some(t0));
        assert_eq!(inst.targets.len(), 1, "the Target node yields one routing");
        let rt = &inst.targets[0];
        assert!(
            matches!(rt.source, Some(ModSource::Constant(_))),
            "Macro source"
        );
        assert_eq!(
            rt.target,
            AutomationTarget::Track {
                track: Some(t0),
                param: TrackParam::Volume,
            },
            "relative Track{{None}} must resolve to the host track, got {:?}",
            rt.target
        );
    }
}
