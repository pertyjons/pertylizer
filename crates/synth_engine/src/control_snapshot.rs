//! Control-thread publication of engine metadata snapshots.
//!
//! The command sender invokes this module before handing a command to the
//! real-time thread. Snapshot construction may allocate and take locks; DSP
//! command handling must not.

use std::sync::Arc;

use synth_core::{BypassState, ModuleParam, Param, PolyModule};

use crate::commands::{EngineCommand, InstrumentParam, ModuleId, ReorderDirection};
use crate::effect_chain::ChainSlot;
use crate::instrument::{Instrument, InstrumentId};
use crate::shared_state::{
    ConnectionSnapshot, InstrumentSnapshot, ModuleStateSnapshot, ReturnBusSnapshot,
    ReturnEffectSnapshot,
};
use crate::state::EngineState;

pub(crate) fn publish(command: &EngineCommand, state: &Arc<EngineState>) {
    match command {
        EngineCommand::AddInstrument { instrument } => {
            publish_instrument_graph(instrument, state);
            let mut instruments = state.instrument_snapshots.write();
            instruments.retain(|snapshot| snapshot.id != instrument.id());
            instruments.push(instrument_snapshot(instrument));
        }
        EngineCommand::RemoveInstrument { instrument_id } => {
            state
                .instrument_snapshots
                .write()
                .retain(|snapshot| snapshot.id != *instrument_id);
            state
                .shared_graph
                .remove_modules_for_instrument(*instrument_id);
            state
                .shared_graph
                .set_connections_for_instrument(*instrument_id, Vec::new());
            for snapshot in state.instrument_snapshots.write().iter_mut() {
                if snapshot.sidechain_source_id == Some(*instrument_id) {
                    snapshot.sidechain_source_id = None;
                }
            }
        }
        EngineCommand::RenameInstrument {
            instrument_id,
            name,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.name.clone_from(name);
        }),
        EngineCommand::SetInstrumentDescription {
            instrument_id,
            description,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.description.clone_from(description);
        }),
        EngineCommand::SetPatchDescription {
            instrument_id,
            description,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.patch_description.clone_from(description);
        }),
        EngineCommand::SetInstrumentColor {
            instrument_id,
            color,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.color.clone_from(color);
        }),
        EngineCommand::SetPatchColor {
            instrument_id,
            color,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.patch_color.clone_from(color);
        }),
        EngineCommand::SetModuleDescription {
            instrument_id,
            module_id,
            description,
        } => {
            state.shared_graph.set_module_description(
                *instrument_id,
                *module_id,
                description.clone().unwrap_or_default(),
            );
        }
        EngineCommand::SetSidechainSource {
            instrument_id,
            source,
        } => {
            if !sidechain_cycle(state, *instrument_id, *source) {
                patch_instrument(state, *instrument_id, |snapshot| {
                    snapshot.sidechain_source_id = *source;
                });
            }
        }
        EngineCommand::SetInstrumentParameter {
            instrument_id,
            param,
        } => publish_instrument_param(state, *instrument_id, *param),
        EngineCommand::SetInstrumentMidiChannel {
            instrument_id,
            channel,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.midi_channel = channel.as_channel();
        }),
        EngineCommand::SetInstrumentEnabled {
            instrument_id,
            enabled,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.enabled = *enabled;
            snapshot.muted = !enabled;
        }),
        EngineCommand::SetInstrumentCategory {
            instrument_id,
            category,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.category = *category;
        }),
        EngineCommand::SetInstrumentSolo {
            instrument_id,
            solo,
        } => patch_instrument(state, *instrument_id, |snapshot| {
            snapshot.solo = *solo;
        }),
        EngineCommand::CreateReturnBus { id } => {
            let mut busses = state.return_bus_effects.write();
            if !busses.iter().any(|bus| bus.id == *id) {
                busses.push(ReturnBusSnapshot {
                    id: *id,
                    effects: Vec::new(),
                });
            }
        }
        EngineCommand::RemoveReturnBus { id } => {
            state.return_bus_effects.write().retain(|bus| bus.id != *id);
        }
        EngineCommand::ClearReturnBusses => state.return_bus_effects.write().clear(),
        EngineCommand::AddReturnEffect {
            return_id,
            id,
            effect,
        } => {
            if let Some(bus) = state
                .return_bus_effects
                .write()
                .iter_mut()
                .find(|bus| bus.id == *return_id)
            {
                bus.effects.push(effect_snapshot(*id, effect.as_ref()));
            }
        }
        EngineCommand::RemoveReturnEffect { return_id, id } => {
            if let Some(bus) = state
                .return_bus_effects
                .write()
                .iter_mut()
                .find(|bus| bus.id == *return_id)
            {
                bus.effects.retain(|effect| effect.module_id != *id);
            }
        }
        EngineCommand::SetReturnEffectParameter {
            return_id,
            module_id,
            param,
        } => {
            if let Some(mut effect) = return_effect_mut(state, *return_id, *module_id) {
                replace_param(&mut effect.parameters, *param);
                effect.bypassed = false;
            }
        }
        EngineCommand::SetReturnEffectEnabled {
            return_id,
            module_id,
            enabled,
        } => {
            if let Some(mut effect) = return_effect_mut(state, *return_id, *module_id) {
                effect.bypassed = !enabled;
            }
        }
        EngineCommand::ReorderReturnEffect {
            return_id,
            module_id,
            direction,
        } => {
            if let Some(bus) = state
                .return_bus_effects
                .write()
                .iter_mut()
                .find(|bus| bus.id == *return_id)
            {
                move_item(&mut bus.effects, *module_id, *direction, |effect| {
                    effect.module_id
                });
            }
        }
        EngineCommand::ClearMasterEffects => state.master_effects.write().clear(),
        EngineCommand::SetVoiceParameter {
            instrument_id,
            target,
            param,
        } => state
            .shared_graph
            .set_module_parameter(*instrument_id, target.module_id(), *param),
        EngineCommand::SetModuleParameter {
            instrument_id: Some(instrument_id),
            module_id,
            param,
        } => state
            .shared_graph
            .set_module_parameter(*instrument_id, *module_id, *param),
        EngineCommand::SetModScript {
            instrument_id: Some(instrument_id),
            module_id,
            slot,
            script,
            ..
        } => state.shared_graph.set_module_script(
            *instrument_id,
            *module_id,
            usize::from(*slot) + 1,
            script.as_ref().map(|bound| bound.source.clone()),
            script.as_ref().map(|bound| bound.params.as_slice()),
        ),
        EngineCommand::ClearAllModules => {
            state.shared_graph.clear();
            state.master_effects.write().clear();
            for snapshot in state.instrument_snapshots.write().iter_mut() {
                snapshot.enabled = false;
                snapshot.muted = true;
                snapshot.module_count = 0;
                snapshot.effect_count = 0;
                snapshot.effect_chain_order.clear();
            }
        }
        EngineCommand::SetBypass {
            instrument_id,
            module,
            bypass,
        } => state
            .shared_graph
            .set_module_bypass(*instrument_id, *module, *bypass),
        EngineCommand::AddEffectInstance {
            instrument_id,
            id,
            effect,
        } => publish_added_effect(state, *instrument_id, *id, effect.as_ref()),
        EngineCommand::RemoveEffect { instrument_id, id } => {
            publish_removed_effect(state, *instrument_id, *id);
        }
        EngineCommand::ReorderEffect {
            instrument_id,
            module_id,
            direction,
        } => publish_reordered_effect(state, *instrument_id, *module_id, *direction),
        EngineCommand::SetEffectChainOrder {
            instrument_id,
            order,
        } => publish_effect_order(state, *instrument_id, order),
        EngineCommand::SetEffectParameter {
            instrument_id,
            module_id,
            param,
        } => publish_effect_param(state, *instrument_id, *module_id, *param),
        EngineCommand::SetEffectEnabled {
            instrument_id,
            module_id,
            enabled,
        } => publish_effect_enabled(state, *instrument_id, *module_id, *enabled),
        EngineCommand::AddModuleInstance {
            instrument_id: Some(instrument_id),
            id,
            module,
        } => {
            state
                .shared_graph
                .set_module(module_snapshot(*instrument_id, *id, module.as_ref()));
            patch_instrument(state, *instrument_id, |snapshot| {
                snapshot.module_count += 1;
            });
        }
        EngineCommand::RemoveModule {
            instrument_id: Some(instrument_id),
            id,
        } => {
            state.shared_graph.remove_module(*instrument_id, *id);
            state
                .shared_graph
                .disconnect_all_for_instrument(*instrument_id, *id);
            patch_instrument(state, *instrument_id, |snapshot| {
                snapshot.module_count = snapshot.module_count.saturating_sub(1);
            });
        }
        EngineCommand::Connect {
            instrument_id: Some(instrument_id),
            from,
            to,
        } => state.shared_graph.add_connection(ConnectionSnapshot::new(
            *instrument_id,
            from.module,
            from.port,
            to.module,
            to.port,
        )),
        EngineCommand::Disconnect {
            instrument_id: Some(instrument_id),
            from,
            to,
        } => state.shared_graph.remove_connection_for_instrument(
            *instrument_id,
            from.module,
            from.port,
            to.module,
            to.port,
        ),
        EngineCommand::DisconnectAll {
            instrument_id: Some(instrument_id),
            module,
        } => state
            .shared_graph
            .disconnect_all_for_instrument(*instrument_id, *module),
        _ => {}
    }
}

fn patch_instrument(
    state: &EngineState,
    instrument_id: InstrumentId,
    patch: impl FnOnce(&mut InstrumentSnapshot),
) {
    if let Some(snapshot) = state
        .instrument_snapshots
        .write()
        .iter_mut()
        .find(|snapshot| snapshot.id == instrument_id)
    {
        patch(snapshot);
    }
}

fn sidechain_cycle(
    state: &EngineState,
    instrument_id: InstrumentId,
    source: Option<InstrumentId>,
) -> bool {
    let instruments = state.instrument_snapshots.read();
    let mut current = source;
    for _ in 0..=instruments.len() {
        let Some(id) = current else {
            return false;
        };
        if id == instrument_id {
            return true;
        }
        current = instruments
            .iter()
            .find(|snapshot| snapshot.id == id)
            .and_then(|snapshot| snapshot.sidechain_source_id);
    }
    false
}

fn publish_instrument_param(
    state: &EngineState,
    instrument_id: InstrumentId,
    param: InstrumentParam,
) {
    patch_instrument(state, instrument_id, |snapshot| match param {
        InstrumentParam::Volume(value) => snapshot.volume = value,
        InstrumentParam::Pan(value) => snapshot.pan = value,
        InstrumentParam::Solo(value) => snapshot.solo = value,
        InstrumentParam::AllocationMode(value) => snapshot.allocation_mode = value,
        InstrumentParam::StealingStrategy(value) => snapshot.stealing_strategy = value,
        InstrumentParam::UnisonDetune(value) => snapshot.unison_detune = value,
        InstrumentParam::UnisonSpread(value) => snapshot.unison_spread = value,
        InstrumentParam::MaxVoices(value) => snapshot.max_voices = value,
        InstrumentParam::VelocityAmpSensitivity(value) => {
            snapshot.velocity_amp_sensitivity = value;
        }
        InstrumentParam::VelocityFilterSensitivity(value) => {
            snapshot.velocity_filter_sensitivity = value;
        }
        InstrumentParam::KeyRange(value) => snapshot.key_range = value,
        InstrumentParam::Transpose(value) => snapshot.transpose = value,
        InstrumentParam::OversamplingFactor(value) => snapshot.oversampling = value,
        InstrumentParam::GlideTime(_) | InstrumentParam::LearnState(_) => {}
    });
}

fn instrument_snapshot(instrument: &Instrument) -> InstrumentSnapshot {
    let allocator = instrument.allocator().config();
    InstrumentSnapshot {
        id: instrument.id(),
        name: instrument.name().to_owned(),
        description: instrument.description().to_owned(),
        patch_description: instrument.patch_description().map(str::to_owned),
        color: instrument.color().map(str::to_owned),
        patch_color: instrument.patch_color().map(str::to_owned),
        sidechain_source_id: instrument.sidechain_source_id(),
        category: instrument.category(),
        midi_channel: instrument.midi_channel().as_channel(),
        volume: instrument.volume(),
        pan: instrument.pan(),
        enabled: instrument.is_enabled(),
        muted: !instrument.is_enabled(),
        solo: instrument.is_solo(),
        module_count: instrument.voice_graph().len(),
        effect_count: instrument.effect_chain().slot_order().len(),
        effect_chain_order: instrument.effect_chain().slot_order(),
        key_range: instrument.key_range(),
        transpose: instrument.transpose(),
        oversampling: instrument.oversampling(),
        allocation_mode: allocator.mode,
        stealing_strategy: allocator.stealing,
        unison_detune: allocator.unison_detune,
        unison_spread: allocator.unison_spread,
        max_voices: allocator.max_voices,
        velocity_amp_sensitivity: instrument.velocity_amp_sensitivity(),
        velocity_filter_sensitivity: instrument.velocity_filter_sensitivity(),
    }
}

fn module_snapshot(
    instrument_id: InstrumentId,
    id: ModuleId,
    module: &dyn PolyModule,
) -> ModuleStateSnapshot {
    let descriptor = module.descriptor();
    let mut snapshot =
        ModuleStateSnapshot::new(id, instrument_id, module.module_type(), descriptor.name);
    snapshot.parameters = module.get_params();
    if let Some(scripts) = module.scripts() {
        for (slot, script) in scripts.iter().enumerate() {
            if let Some(bound) = script {
                snapshot
                    .scripts
                    .insert((slot + 1).to_string(), bound.source.clone());
            }
        }
    }
    snapshot
}

fn effect_module_snapshot(
    instrument_id: InstrumentId,
    id: ModuleId,
    effect: &dyn synth_core::AudioEffect,
) -> ModuleStateSnapshot {
    let descriptor = effect.descriptor();
    let mut snapshot =
        ModuleStateSnapshot::new(id, instrument_id, effect.module_type(), descriptor.name);
    snapshot.parameters = effect.get_params();
    snapshot
}

fn effect_snapshot(id: ModuleId, effect: &dyn synth_core::AudioEffect) -> ReturnEffectSnapshot {
    ReturnEffectSnapshot {
        module_id: id,
        module_type: effect.module_type(),
        parameters: effect.get_params(),
        bypassed: false,
    }
}

fn publish_instrument_graph(instrument: &Instrument, state: &EngineState) {
    let instrument_id = instrument.id();
    state
        .shared_graph
        .remove_modules_for_instrument(instrument_id);
    for id in instrument.voice_graph().module_ids() {
        if let Some(module) = instrument.voice_graph().get_module(id) {
            let mut snapshot = module_snapshot(instrument_id, id, module);
            snapshot.description = instrument
                .module_description(id)
                .unwrap_or_default()
                .to_owned();
            snapshot.bypass_state = if instrument.voice_graph().is_bypassed(id) {
                BypassState::Bypassed
            } else {
                BypassState::Active
            };
            state.shared_graph.set_module(snapshot);
        }
    }
    for slot in instrument.effect_chain().slots() {
        if let ChainSlot::Effect(slot) = slot {
            let mut snapshot =
                effect_module_snapshot(instrument_id, slot.module_id, slot.effect.as_ref());
            snapshot.description = instrument
                .module_description(slot.module_id)
                .unwrap_or_default()
                .to_owned();
            snapshot.bypass_state = if slot.state.is_bypassed() {
                BypassState::Bypassed
            } else {
                BypassState::Active
            };
            state.shared_graph.set_module(snapshot);
        }
    }
    let connections = instrument
        .voice_graph()
        .connections()
        .map(|connection| {
            ConnectionSnapshot::new(
                instrument_id,
                connection.from_module,
                connection.from_port,
                connection.to_module,
                connection.to_port,
            )
        })
        .collect();
    state
        .shared_graph
        .set_connections_for_instrument(instrument_id, connections);
}

fn replace_param(parameters: &mut [Param], param: Param) {
    if let Some(current) = parameters
        .iter_mut()
        .find(|current| current.same_kind(&param))
    {
        *current = param;
    }
}

fn return_effect_mut(
    state: &EngineState,
    return_id: synth_sequencer::ReturnBusId,
    module_id: ModuleId,
) -> Option<parking_lot::MappedRwLockWriteGuard<'_, ReturnEffectSnapshot>> {
    parking_lot::RwLockWriteGuard::try_map(state.return_bus_effects.write(), |busses| {
        busses
            .iter_mut()
            .find(|bus| bus.id == return_id)
            .and_then(|bus| {
                bus.effects
                    .iter_mut()
                    .find(|effect| effect.module_id == module_id)
            })
    })
    .ok()
}

fn publish_added_effect(
    state: &EngineState,
    instrument_id: Option<InstrumentId>,
    id: ModuleId,
    effect: &dyn synth_core::AudioEffect,
) {
    if let Some(instrument_id) = instrument_id {
        state
            .shared_graph
            .set_module(effect_module_snapshot(instrument_id, id, effect));
        patch_instrument(state, instrument_id, |snapshot| {
            snapshot.effect_count += 1;
            snapshot.effect_chain_order.push(id);
        });
    } else {
        state
            .master_effects
            .write()
            .push(effect_snapshot(id, effect));
    }
}

fn publish_removed_effect(state: &EngineState, instrument_id: Option<InstrumentId>, id: ModuleId) {
    if let Some(instrument_id) = instrument_id {
        state.shared_graph.remove_module(instrument_id, id);
        patch_instrument(state, instrument_id, |snapshot| {
            snapshot.effect_count = snapshot.effect_count.saturating_sub(1);
            snapshot
                .effect_chain_order
                .retain(|module_id| *module_id != id);
        });
    } else {
        state
            .master_effects
            .write()
            .retain(|effect| effect.module_id != id);
    }
}

fn publish_reordered_effect(
    state: &EngineState,
    instrument_id: Option<InstrumentId>,
    module_id: ModuleId,
    direction: ReorderDirection,
) {
    if let Some(instrument_id) = instrument_id {
        patch_instrument(state, instrument_id, |snapshot| {
            move_item(
                &mut snapshot.effect_chain_order,
                module_id,
                direction,
                |id| *id,
            );
        });
    } else {
        move_item(
            &mut state.master_effects.write(),
            module_id,
            direction,
            |effect| effect.module_id,
        );
    }
}

fn publish_effect_order(
    state: &EngineState,
    instrument_id: Option<InstrumentId>,
    order: &[ModuleId],
) {
    if let Some(instrument_id) = instrument_id {
        patch_instrument(state, instrument_id, |snapshot| {
            apply_order(&mut snapshot.effect_chain_order, order, |id| *id);
        });
    } else {
        apply_order(&mut state.master_effects.write(), order, |effect| {
            effect.module_id
        });
    }
}

fn publish_effect_param(
    state: &EngineState,
    instrument_id: Option<InstrumentId>,
    module_id: ModuleId,
    param: Param,
) {
    if let Some(instrument_id) = instrument_id {
        state
            .shared_graph
            .set_module_parameter(instrument_id, module_id, param);
        state
            .shared_graph
            .set_module_bypass(Some(instrument_id), module_id, false);
    } else if let Some(effect) = state
        .master_effects
        .write()
        .iter_mut()
        .find(|effect| effect.module_id == module_id)
    {
        replace_param(&mut effect.parameters, param);
        effect.bypassed = false;
    }
}

fn publish_effect_enabled(
    state: &EngineState,
    instrument_id: Option<InstrumentId>,
    module_id: ModuleId,
    enabled: bool,
) {
    if let Some(instrument_id) = instrument_id {
        state
            .shared_graph
            .set_module_bypass(Some(instrument_id), module_id, !enabled);
    } else if let Some(effect) = state
        .master_effects
        .write()
        .iter_mut()
        .find(|effect| effect.module_id == module_id)
    {
        effect.bypassed = !enabled;
    }
}

fn move_item<T>(
    items: &mut [T],
    id: ModuleId,
    direction: ReorderDirection,
    get_id: impl Fn(&T) -> ModuleId,
) {
    let Some(index) = items.iter().position(|item| get_id(item) == id) else {
        return;
    };
    let target = match direction {
        ReorderDirection::Up => index.saturating_sub(1),
        ReorderDirection::Down => (index + 1).min(items.len().saturating_sub(1)),
    };
    items.swap(index, target);
}

fn apply_order<T>(items: &mut Vec<T>, order: &[ModuleId], get_id: impl Fn(&T) -> ModuleId) {
    let mut reordered = Vec::with_capacity(items.len());
    for id in order {
        if let Some(index) = items.iter().position(|item| get_id(item) == *id) {
            reordered.push(items.remove(index));
        }
    }
    reordered.append(items);
    *items = reordered;
}
