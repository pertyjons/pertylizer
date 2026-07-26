//! Shared snapshot-to-engine hydration for preview and arrangement rendering.

use std::collections::{HashMap, HashSet};

use synth_core::ModuleParam;
use synth_engine::commands::PortId;
use synth_engine::shared_state::ConnectionSnapshot;
use synth_engine::{EngineCommand, EngineHandle, InstrumentId, ModuleId, ModuleStateSnapshot};

use crate::audio::preview::SharedSampleLibrary;
use crate::session::SynthSession;

pub(crate) struct SnapshotHydration<'a> {
    pub(crate) instrument_id: InstrumentId,
    pub(crate) modules: &'a [ModuleStateSnapshot],
    pub(crate) connections: &'a [ConnectionSnapshot],
    pub(crate) effect_chain_order: &'a [ModuleId],
    pub(crate) sample_library: &'a SharedSampleLibrary,
    pub(crate) context: &'static str,
}

/// Rebuild one instrument graph from immutable engine snapshots.
///
/// Instrument enable/mix state stays with the caller because preview deliberately
/// ignores live mute while arrangement rendering must preserve it.
pub(crate) fn hydrate_snapshot_instrument(
    session: &SynthSession,
    handle: &mut EngineHandle,
    request: SnapshotHydration<'_>,
    warnings: &mut Vec<String>,
    mut after_batch: impl FnMut(&mut EngineHandle),
) {
    let mut voice_modules: Vec<&ModuleStateSnapshot> = Vec::new();
    let mut effect_modules: HashMap<ModuleId, &ModuleStateSnapshot> = HashMap::new();
    for module in request.modules {
        if module.module_type.is_visualizer() {
            continue;
        }
        if module.module_type.is_effect() {
            effect_modules.insert(module.id, module);
        } else {
            voice_modules.push(module);
        }
    }

    let appended_effects = {
        let mut install = |module: &ModuleStateSnapshot, kind: &str| {
            let module_id = module.id;
            let descriptor = match session.add_module_with_id(
                request.instrument_id,
                module_id,
                module_id.module_type,
            ) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    warnings.push(format!(
                        "{}: failed to add {kind} {module_id}: {error}",
                        request.context
                    ));
                    return;
                }
            };

            let is_effect = module_id.module_type.is_effect();
            for descriptor_param in &descriptor.parameters {
                let Some(param) = module
                    .parameters
                    .iter()
                    .find(|param| param.same_kind(&descriptor_param.id))
                    .copied()
                else {
                    continue;
                };
                let sent = if is_effect {
                    handle.send_blocking(EngineCommand::SetEffectParameter {
                        instrument_id: Some(request.instrument_id),
                        module_id,
                        param,
                    })
                } else {
                    handle.send_blocking(EngineCommand::SetModuleParameter {
                        instrument_id: Some(request.instrument_id),
                        module_id,
                        param,
                    })
                };
                if !sent {
                    warnings.push(format!(
                        "{}: failed to enqueue parameter for module {module_id}",
                        request.context
                    ));
                }
            }

            if matches!(module.bypass_state, synth_core::BypassState::Bypassed) {
                if is_effect {
                    handle.send_blocking(EngineCommand::SetEffectEnabled {
                        instrument_id: Some(request.instrument_id),
                        module_id,
                        enabled: false,
                    });
                } else {
                    handle.send_blocking(EngineCommand::SetBypass {
                        instrument_id: Some(request.instrument_id),
                        module: module_id,
                        bypass: true,
                    });
                }
            }

            crate::audio::replay_module_scripts(
                handle,
                request.instrument_id,
                module,
                warnings,
                request.context,
            );
            after_batch(handle);
        };

        for module in &voice_modules {
            install(module, "module");
        }

        let mut installed_effects = HashSet::new();
        for module_id in request.effect_chain_order {
            if let Some(module) = effect_modules.get(module_id) {
                installed_effects.insert(*module_id);
                install(module, "effect");
            }
        }
        let mut appended_effects = Vec::new();
        for (module_id, module) in effect_modules {
            if installed_effects.contains(&module_id) {
                continue;
            }
            appended_effects.push(module_id);
            install(module, "effect");
        }
        appended_effects
    };

    warnings.extend(appended_effects.into_iter().map(|module_id| {
        format!(
            "{}: effect {module_id} present but missing from chain order — appending",
            request.context
        )
    }));

    crate::audio::preview::load_sample_data_for_samplers(
        handle,
        request.instrument_id,
        &voice_modules,
        request.sample_library,
        warnings,
    );

    for connection in request.connections {
        if connection.from_module.module_type.is_visualizer()
            || connection.to_module.module_type.is_visualizer()
        {
            continue;
        }
        let sent = handle.send_blocking(EngineCommand::Connect {
            instrument_id: Some(request.instrument_id),
            from: PortId::new(connection.from_module, connection.from_port),
            to: PortId::new(connection.to_module, connection.to_port),
        });
        if !sent {
            warnings.push(format!(
                "{}: failed to enqueue connection {} → {}",
                request.context, connection.from_module, connection.to_module
            ));
        }
        after_batch(handle);
    }
}
