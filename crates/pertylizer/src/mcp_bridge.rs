//! Bridge between the MCP server and the running synth engine.
//!
//! `AppSynthBridge` implements `SynthBridge` by reading from `EngineState`
//! (shared graph, meters, transport) and sending commands via `CommandSender`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use synth_core::ModuleParam;
use synth_core::{
    BipolarValue, Bpm, DestAddr, Gain, MidiChannel, MidiNote, ModMatrixParam, ModuleType,
    NormalizedValue, Param, ParameterUnit, PortDescriptor, PortDirection, PortName, PortType,
    SampleCount, Semitones, SrcAddr, Velocity,
};
use synth_engine::EngineCommand;
use synth_engine::commands::ModuleId;
use synth_engine::instrument::{InstrumentId, MidiChannelSelection};
use synth_mcp::bridge::{
    BridgeAutomationPointData, BridgeExpression, BridgeGlide, BridgeInstrumentDef, BridgeNoteData,
    BridgeNoteUpdate, BridgeParamSet, BridgeParamValue, BridgePatternData, BridgePlacementData,
    BridgePlacementUpdate, BridgeSongPlacement, BridgeTrackData,
};
use synth_mcp::error::McpBridgeError;
use synth_mcp::types::{
    AnalyzeHarmonyResult, AnalyzeMaskingMatrixResult, AnalyzeMasterChainResult,
    AnalyzeMixBusResult, AnalyzePatternResult, AnalyzeReturnBussesResult, AnalyzeSectionResult,
    ApplyExamplePatchResult, AutoGainStageResult, AutomationLaneInfo, AutomationPointInfo,
    AutomationTargetInfo, BandOverlap, BatchItemResult, BatchResult, BuildInstrumentResult,
    CompareMixResult, ConnectionCheckResult, ConnectionInfo, DiagnosticSeverity, EngineStatus,
    ExamplePatchInfo, GraphDiagnostic, HarmonyChordEvent, HarmonyKeyEstimate, HarmonyScope,
    HarmonyStats, InstrumentInfo, MaskingPair, MatrixRoutingInfo, MixBusMetrics, MixDelta,
    ModGraphConnectionInfo, ModGraphDetail, ModGraphInfo, ModGraphNodeInfo, ModTargetInfo,
    ModuleInfo, ModuleSearchResult, ModuleTypeBrief, ModuleTypeInfo, NoteGraphConnectionInfo,
    NoteGraphDetail, NoteGraphInfo, NoteGraphModuleInfo, NoteInfo, OptimizeResult, ParamTypeInfo,
    ParameterInfo, PatchModuleInfo, PatchParamInfo, PatchParamValue, PatchResourceData,
    PatternInfo, PlacementInfo, ProjectSchemaInfo, RebuildInstrumentResult, RenderToWavResult,
    SetSongResult, SongInfo, TempoPoint, TrackInfo, UiConnectionInfo, UiModuleInfo, UiOverlap,
    UiSnapshot, VersionInfo,
};
use synth_sequencer::{
    ModGraphId, ModNodeId, NoteGraphId, NoteId, NoteModuleId, PatternId, ReturnBusId, Tick, TrackId,
};

use crate::mcp_shared::McpSharedState;
use crate::session::SynthSession;

/// Virtual port name surfaced on `ModuleInfo.input_ports` / `output_ports`
/// to mark a module that is wired only through a Mod Matrix slot rather
/// than via real audio/CV cables.
const MATRIX_VIRTUAL_PORT: &str = "matrix";

/// Hard cap on a per-module-instance description (characters). Long enough for
/// a paragraph of intent, short enough to stay readable in tooltips. The TODO
/// suggests a 500-char soft / 2000-char hard split; only the hard limit is
/// enforced (the soft limit is advisory).
const MAX_MODULE_DESCRIPTION_LEN: usize = 2000;

/// The committed `.ptz` JSON Schema, embedded at build time. Surfaced by
/// `get_project_schema` so external tools validate or diff project files against
/// the exact on-disk artifact — returning this (rather than a live `schema_for!`
/// re-derivation) guarantees zero introspection-vs-disk drift. The `gen_schemas`
/// `checked_in_schemas_match_generated` test keeps it in sync with `ProjectFile`.
///
/// NOTE: the `../../../schemas/` workspace-root-relative path is assumed in three
/// places that must move together if the layout changes: this `include_str!`,
/// `crates/pertylizer/src/bin/gen_schemas.rs` (writes it), and the drift test in
/// `crates/pertylizer/tests/` (byte-compares it).
const PROJECT_SCHEMA_JSON: &str = include_str!("../../../schemas/project.schema.json");

/// Bridge implementation for the Pertylizer application.
pub struct AppSynthBridge {
    session: Arc<SynthSession>,
    shared: Arc<McpSharedState>,
    sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
    /// Per-(return bus, effect type) high-water mark for fresh effect instance
    /// numbers. The engine's `return_bus_effects` snapshot only catches up after
    /// the audio thread drains its command queue, so a `batch_execute` that adds
    /// two effects of the same type to one bus would otherwise read a stale
    /// snapshot and assign both the same `ModuleId`. Bumping a high-water mark on
    /// every add keeps instance numbers unique without waiting for the audio thread.
    return_effect_hw: parking_lot::Mutex<HashMap<(ReturnBusId, ModuleType), u16>>,
    /// Master-effect counterpart of [`Self::return_effect_hw`] (the master chain
    /// has no bus id, so it gets its own per-effect-type high-water map).
    master_effect_hw: parking_lot::Mutex<HashMap<ModuleType, u16>>,
    /// Pre-batch project snapshot held while a `batch_execute` with
    /// `rollback: true` runs. Captured before the first op, applied back if any
    /// op fails, then cleared. `None` outside a rollback batch.
    rollback_snapshot: parking_lot::Mutex<Option<Box<crate::project::ProjectFile>>>,
    /// MCP-owned input path, independent of GUI frame cadence so headless
    /// recording works as well as desktop recording.
    audio_input: Arc<parking_lot::Mutex<crate::audio::input::AudioInputManager>>,
    audio_input_host: parking_lot::Mutex<Option<Box<dyn crate::audio::AudioHostTrait>>>,
    selected_input_device: parking_lot::Mutex<Option<String>>,
}

/// Legacy port-name aliases so the historical example port names still connect
/// instead of silently producing a zero-connection instrument. Single source of
/// truth for the alias table shared by `resolve_port_name` and `check_connection`.
fn port_alias(requested: &str) -> Option<&'static str> {
    match requested {
        "output" => Some("out"),
        "input" => Some("in"),
        _ => None,
    }
}

/// Resolve a requested port name against a module's `ports` for `direction`.
///
/// Exact name matches win; otherwise the [`port_alias`] table is tried. Returns
/// the canonical [`PortName`] to hand to the engine, or a debug list of the
/// available ports in that direction on failure.
fn resolve_port_name(
    ports: &[PortDescriptor],
    requested: &str,
    direction: PortDirection,
) -> Result<PortName, String> {
    if let Some(p) = ports
        .iter()
        .find(|p| p.direction == direction && p.name.as_str() == requested)
    {
        return Ok(p.name);
    }
    if let Some(alias) = port_alias(requested)
        && let Some(p) = ports
            .iter()
            .find(|p| p.direction == direction && p.name.as_str() == alias)
    {
        return Ok(p.name);
    }
    let available: Vec<&str> = ports
        .iter()
        .filter(|p| p.direction == direction)
        .map(|p| p.name.as_str())
        .collect();
    Err(format!("{available:?}"))
}

impl AppSynthBridge {
    /// Create a new bridge with access to the session, shared MCP state, and sample library.
    pub fn new(
        session: Arc<SynthSession>,
        shared: Arc<McpSharedState>,
        sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
    ) -> Self {
        let audio_input = Arc::new(parking_lot::Mutex::new(
            crate::audio::input::AudioInputManager::new(),
        ));
        let drain_input = Arc::downgrade(&audio_input);
        let _ = std::thread::Builder::new()
            .name("mcp-recording-drain".to_string())
            .spawn(move || {
                while let Some(input) = drain_input.upgrade() {
                    input.lock().drain_gui_buffer();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
        Self {
            session,
            shared,
            sample_library,
            return_effect_hw: parking_lot::Mutex::new(HashMap::new()),
            master_effect_hw: parking_lot::Mutex::new(HashMap::new()),
            rollback_snapshot: parking_lot::Mutex::new(None),
            audio_input,
            audio_input_host: parking_lot::Mutex::new(
                crate::audio::default_host()
                    .ok()
                    .map(|host| Box::new(host) as Box<dyn crate::audio::AudioHostTrait>),
            ),
            selected_input_device: parking_lot::Mutex::new(None),
        }
    }

    /// Allocate a fresh master-effect instance number for `module_type`, unique
    /// even across same-type adds the audio thread hasn't published yet (the
    /// master analogue of [`Self::next_return_effect_instance`]).
    fn next_master_effect_instance(&self, module_type: ModuleType) -> u16 {
        let snapshot_max = self
            .session
            .state()
            .master_effects
            .read()
            .iter()
            .filter(|e| e.module_type == module_type)
            .map(|e| e.module_id.instance)
            .max()
            .unwrap_or(0);
        let mut hw = self.master_effect_hw.lock();
        let next = snapshot_max
            .max(hw.get(&module_type).copied().unwrap_or(0))
            .saturating_add(1);
        hw.insert(module_type, next);
        next
    }

    /// Error out unless the song currently owns a return bus with this id.
    fn require_return_bus(
        &self,
        return_id: synth_sequencer::ReturnBusId,
    ) -> Result<(), McpBridgeError> {
        if self
            .shared
            .song
            .read()
            .return_busses()
            .iter()
            .any(|b| b.id == return_id)
        {
            Ok(())
        } else {
            Err(McpBridgeError::ReturnBusNotFound(return_id))
        }
    }

    /// Allocate a fresh effect instance number for `(return bus, effect type)`,
    /// unique even across same-type adds the audio thread hasn't published yet.
    /// Takes the max of the live snapshot's highest instance and a per-key
    /// high-water mark, then bumps and records the result.
    fn next_return_effect_instance(
        &self,
        return_id: synth_sequencer::ReturnBusId,
        module_type: ModuleType,
    ) -> u16 {
        let snapshot_max = self
            .session
            .state()
            .return_bus_effects
            .read()
            .iter()
            .find(|b| b.id == return_id)
            .map(|b| {
                b.effects
                    .iter()
                    .filter(|e| e.module_type == module_type)
                    .map(|e| e.module_id.instance)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let mut hw = self.return_effect_hw.lock();
        let key = (return_id, module_type);
        let next = snapshot_max
            .max(hw.get(&key).copied().unwrap_or(0))
            .saturating_add(1);
        hw.insert(key, next);
        next
    }

    /// Resolve a return bus's live effect chain into MCP-facing info, mapping
    /// each snapshot effect's params through its descriptor for names/units.
    fn return_bus_effect_infos(
        &self,
        return_id: synth_sequencer::ReturnBusId,
    ) -> Vec<synth_mcp::ReturnEffectInfo> {
        self.session
            .state()
            .return_bus_effects
            .read()
            .iter()
            .find(|b| b.id == return_id)
            .map(|b| b.effects.iter().map(return_effect_info).collect())
            .unwrap_or_default()
    }

    /// Resolve an insert-effect parameter write shared by the return-bus and
    /// master-bus paths: parse the module id, look the parameter up in the effect
    /// type's descriptor (by `type_id` then display name), resolve + range-check
    /// the value, and build both the typed `Param` to send and the `ParameterInfo`
    /// to return. The caller wraps the `Param` in the appropriate engine command
    /// (return vs master), which is the only difference between the two paths.
    fn resolve_effect_param(
        &self,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<(ModuleId, Param, ParameterInfo), McpBridgeError> {
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        let descriptor = crate::module_factory::get_descriptor(mid.module_type);
        let param_desc = descriptor
            .as_ref()
            .and_then(|desc| desc.find_parameter(param_name));
        let Some(pd) = param_desc else {
            // Name the near miss: the descriptor in hand lists every spelling
            // this lookup would have accepted, so a typo need not cost a
            // `get_module_type_info` round trip.
            let hint = descriptor
                .as_ref()
                .map(|desc| desc.param_lookup_hint(param_name))
                .unwrap_or_default();
            return Err(McpBridgeError::ParameterNotFound(format!(
                "{param_name}{hint}"
            )));
        };

        let value = resolve_param_value(&value, Some(pd), param_name)?;
        // Kind-aware validation: rounds integers, accepts bools, rejects
        // out-of-range. Use the returned value so the applied + echoed value is
        // exactly what took effect (e.g. a `4.3` integer becomes `4`).
        let value =
            pd.validate_f32(value)
                .map_err(|source| McpBridgeError::InvalidParameterValue {
                    name: pd.name.clone(),
                    source,
                })?;

        let info = ParameterInfo {
            name: pd.name.clone(),
            value,
            display: pd.format(value),
            min: Some(pd.range.min),
            max: Some(pd.range.max),
            default: Some(pd.range.default),
            choices: pd
                .choices
                .as_ref()
                .map(|c| c.iter().map(|ch| ch.name.clone()).collect()),
            type_id: Some(pd.type_id.clone()),
            is_automatable: Some(pd.is_automatable()),
            modulatable: Some(pd.modulatable),
            response_curve: Some(format!("{:?}", pd.response_curve)),
            value_kind: Some(pd.kind),
        };
        Ok((mid, pd.id.with_f32(value), info))
    }

    /// Validate that a module exists and has the given port in the expected
    /// direction, returning the canonical port name (resolving `output`/`input`
    /// aliases) and its signal type.
    fn validate_port(
        &self,
        instrument_id: InstrumentId,
        module_str: &str,
        port: &str,
        direction: PortDirection,
    ) -> Result<(PortName, PortType), McpBridgeError> {
        let mid: ModuleId = module_str
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_str.to_string()))?;

        let descriptor = self
            .session
            .module_descriptor(instrument_id, mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_str.to_string()))?;

        let canonical =
            resolve_port_name(&descriptor.ports, port, direction).map_err(|available| {
                McpBridgeError::PortNotFound {
                    module: module_str.to_string(),
                    port: port.to_string(),
                    available,
                }
            })?;
        descriptor
            .ports
            .iter()
            .find(|candidate| candidate.name == canonical)
            .map(|candidate| (canonical, candidate.port_type))
            .ok_or_else(|| McpBridgeError::PortNotFound {
                module: module_str.to_string(),
                port: port.to_string(),
                available: descriptor
                    .ports
                    .iter()
                    .filter(|candidate| candidate.direction == direction)
                    .map(|candidate| candidate.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }

    /// Validate that an instrument exists in the shared snapshots.
    fn validate_instrument(&self, instrument_id: InstrumentId) -> Result<(), McpBridgeError> {
        if !self.session.instrument_exists(instrument_id) {
            return Err(McpBridgeError::InstrumentNotFound(instrument_id.as_u64()));
        }
        Ok(())
    }

    /// Validate the entity addressed by an automation target before a lane is
    /// read or created. Host-track and global targets need no concrete owner.
    fn validate_automation_target_owner(
        &self,
        target: &synth_sequencer::AutomationTarget,
    ) -> Result<(), McpBridgeError> {
        match target {
            synth_sequencer::AutomationTarget::Instrument { instrument, .. }
            | synth_sequencer::AutomationTarget::Module { instrument, .. } => {
                self.validate_instrument(*instrument)
            }
            synth_sequencer::AutomationTarget::Track {
                track: Some(track), ..
            } => {
                if self.shared.song.read().track(*track).is_none() {
                    return Err(McpBridgeError::TrackNotFound(*track));
                }
                Ok(())
            }
            synth_sequencer::AutomationTarget::Track { track: None, .. }
            | synth_sequencer::AutomationTarget::Global(_) => Ok(()),
        }
    }

    /// The `ModuleId`s in an instrument's graph (empty if the instrument is
    /// absent). Used to validate that a `Module` automation target names a module
    /// that actually exists.
    ///
    /// Reads the session's **synchronous** registry, not the engine's
    /// `shared_graph` (which the audio thread only rebuilds after draining its
    /// command queue). This matters because a `batch_execute` that adds a module
    /// and then automates it in the same request must see the just-added module —
    /// the same reason the session keeps `alive_instruments` for `instrument_exists`.
    fn instrument_module_ids(&self, instrument_id: InstrumentId) -> Vec<synth_engine::ModuleId> {
        let inst_id = instrument_id;
        self.session
            .all_modules_for_instrument(inst_id)
            .into_keys()
            .collect()
    }

    /// Resolve automation against the instrument's live descriptors. Script
    /// modules extend their descriptor at runtime with user-declared knobs, so
    /// the factory descriptor is insufficient for both authoring and rebuild
    /// orphan detection.
    fn build_live_automation_target(
        &self,
        target: &str,
        instrument_id: InstrumentId,
        valid_modules: &[synth_engine::ModuleId],
    ) -> Result<synth_sequencer::AutomationTarget, McpBridgeError> {
        let Some(body) = target.strip_prefix("module:") else {
            return build_automation_target(target, instrument_id, valid_modules);
        };
        let (module_type, instance, param_id) = parse_module_automation_target(body)?;
        let module_id = synth_engine::ModuleId::new(module_type, instance);
        if !valid_modules.contains(&module_id) {
            // instrument_id defaults to 0, so a caller who omitted it (or passed
            // the wrong one) otherwise gets a misleading "no such module". Point
            // them at the instrument(s) that actually own this module instead.
            let elsewhere: Vec<u64> = self
                .session
                .list_instruments()
                .iter()
                .map(|snap| snap.id)
                .filter(|&iid| {
                    iid != instrument_id && self.instrument_module_ids(iid).contains(&module_id)
                })
                .map(|iid| iid.as_u64())
                .collect();
            let hint = if elsewhere.is_empty() {
                String::new()
            } else {
                let ids = elsewhere
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    " — it exists on instrument(s) {ids}; pass instrument_id to target it \
                     (instrument_id defaults to 0)"
                )
            };
            return Err(McpBridgeError::Other(format!(
                "instrument {} has no '{}-{instance}' module to automate{hint}",
                instrument_id.as_u64(),
                module_type.prefix()
            )));
        }
        let descriptor = self
            .session
            .module_descriptor(instrument_id, module_id)
            .ok_or_else(|| {
                McpBridgeError::Other(format!("no live descriptor for module '{module_id}'"))
            })?;
        let parameter = descriptor
            .parameters
            .iter()
            .find(|parameter| parameter.type_id == param_id)
            .ok_or_else(|| {
                McpBridgeError::Other(format!(
                    "module '{}' has no parameter '{param_id}'{}",
                    module_type.prefix(),
                    descriptor.param_id_hint(param_id)
                ))
            })?;
        if !parameter.is_automatable() {
            return Err(McpBridgeError::Other(format!(
                "parameter '{param_id}' on module '{}' is not automatable",
                module_type.prefix()
            )));
        }
        Ok(synth_sequencer::AutomationTarget::Module {
            instrument: instrument_id,
            module_type,
            instance,
            param_id: synth_sequencer::ParamId::from(param_id),
        })
    }

    /// Build a deduped `instrument_id → live module ids` cache for a set of
    /// instruments, so bulk automation operations validate Module targets with
    /// one graph query per instrument rather than one per point.
    fn module_id_cache(
        &self,
        instrument_ids: impl IntoIterator<Item = InstrumentId>,
    ) -> std::collections::HashMap<InstrumentId, Vec<synth_engine::ModuleId>> {
        let mut cache = std::collections::HashMap::new();
        for iid in instrument_ids {
            cache
                .entry(iid)
                .or_insert_with(|| self.instrument_module_ids(iid));
        }
        cache
    }

    /// Convert an `InstrumentSnapshot` to an `InstrumentInfo`.
    fn snapshot_to_info(snap: &synth_engine::shared_state::InstrumentSnapshot) -> InstrumentInfo {
        InstrumentInfo {
            id: snap.id,
            name: snap.name.clone(),
            description: snap.description.clone(),
            patch_description: snap.patch_description.clone(),
            color: snap.color.clone().unwrap_or_default(),
            patch_color: snap.patch_color.clone().unwrap_or_default(),
            sidechain_source_id: snap.sidechain_source_id.map(|id| id.as_u64()),
            category: snap.category.name().to_owned(),
            midi_channel: snap.midi_channel,
            volume: snap.volume,
            pan: snap.pan,
            enabled: snap.enabled,
            muted: snap.muted,
            solo: snap.solo,
            module_count: snap.module_count,
            effect_count: snap.effect_count,
            allocation_mode: snap.allocation_mode.to_string(),
            stealing_strategy: snap.stealing_strategy.to_string(),
            unison_detune: snap.unison_detune.as_f32(),
            unison_spread: snap.unison_spread.as_f32(),
            max_voices: u32::from(snap.max_voices.as_u8()),
        }
    }
}

/// Translate an MCP color argument into the engine's `Option<String>` form.
///
/// An empty string clears the color (`None`); otherwise the input is validated
/// and normalized to canonical `"#RRGGBBAA"` so MCP-set and GUI-set colors
/// round-trip identically. Shared by `set_instrument_color` and
/// `set_patch_color`.
fn normalize_color_arg(color: &str) -> Result<Option<String>, McpBridgeError> {
    if color.is_empty() {
        return Ok(None);
    }
    crate::patch::normalize_hex_color(color)
        .map(Some)
        .ok_or_else(|| {
            McpBridgeError::Other(format!(
                "invalid color {color:?}; expected \"#RRGGBB\" or \"#RRGGBBAA\""
            ))
        })
}

/// Whether a module type can originate audio on its own — the test behind the
/// "instrument will be silent" graph diagnostic.
///
/// Driven by `ModuleCategory` rather than a hand-maintained list, so every
/// `Oscillator`-category generator (including the `VoiceSynth` and `VocalTract`
/// voices, `AmFormant`, `PadSynth`, …) and every `Sampler`-category source
/// (`Sampler`, `AudioInput`) counts automatically. `MechanicalNoise` is added
/// explicitly: it is a real generator but sits in the `PhysicalModeling`
/// category next to input-fed resonators (`BodyResonance`), so the category
/// alone wouldn't capture it.
fn is_sound_source(module_type: synth_core::ModuleType) -> bool {
    if module_type == synth_core::ModuleType::MechanicalNoise {
        return true;
    }
    crate::module_factory::get_descriptor(module_type).is_some_and(|desc| {
        matches!(
            desc.category,
            synth_core::ModuleCategory::Oscillator | synth_core::ModuleCategory::Sampler
        )
    })
}

mod analysis;
mod analysis_impl;
mod audio_input;
mod automation;
mod composition_impl;
mod discovery;
mod discovery_impl;
mod instrument_build;
mod instruments;
mod mixing;
mod project;
mod samples;
mod sequencer;

pub use analysis_impl::*;
pub use composition_impl::*;
pub use discovery_impl::*;

/// Decoded routing from a Mod Matrix module's parameter snapshot. Address-based
/// — no legacy enum, so arbitrary sources/destinations survive (a third LFO,
/// `osc-1.detune`).
#[derive(Debug, Clone, Copy)]
struct ModMatrixSlot {
    source: Option<SrcAddr>,
    destination: Option<DestAddr>,
    amount: f32,
    enabled: bool,
}

impl Default for ModMatrixSlot {
    fn default() -> Self {
        Self {
            source: None,
            destination: None,
            amount: 0.0,
            enabled: true,
        }
    }
}

/// Decode a Mod Matrix module's param snapshot into its dynamic routing list.
/// The list length follows the `SlotN` params present; the vestigial `GridSize`
/// is ignored (the engine no longer gates by it).
fn decode_mod_matrix_slots(params: &[Param]) -> Vec<ModMatrixSlot> {
    let mut slots: Vec<ModMatrixSlot> = Vec::new();
    for p in params {
        let Param::ModMatrix(mp) = p else { continue };
        let idx = match mp {
            ModMatrixParam::SlotSource(i, _)
            | ModMatrixParam::SlotDestination(i, _)
            | ModMatrixParam::SlotAmount(i, _)
            | ModMatrixParam::SlotEnabled(i, _) => *i as usize,
            _ => continue,
        };
        // Ignore out-of-range slots, matching the module's own `set_param`.
        // Keeps the dense Vec ≤ MAX so `slot: i as u8 + 1` can't overflow.
        if idx >= synth_core::MAX_MOD_MATRIX_SLOTS {
            continue;
        }
        if slots.len() <= idx {
            slots.resize(idx + 1, ModMatrixSlot::default());
        }
        match mp {
            ModMatrixParam::SlotSource(_, src) => slots[idx].source = *src,
            ModMatrixParam::SlotDestination(_, dst) => slots[idx].destination = *dst,
            ModMatrixParam::SlotAmount(_, amt) => slots[idx].amount = amt.as_f32(),
            ModMatrixParam::SlotEnabled(_, en) => slots[idx].enabled = *en,
            _ => {}
        }
    }
    slots
}

/// Iterate enabled routings across every Mod Matrix module in the snapshot.
fn active_mod_matrix_slots(
    modules: &[synth_engine::ModuleStateSnapshot],
) -> impl Iterator<Item = ModMatrixSlot> + '_ {
    modules
        .iter()
        .filter(|m| m.id.module_type == synth_core::ModuleType::ModMatrix)
        .flat_map(|m| decode_mod_matrix_slots(&m.parameters))
        .filter(|s| s.enabled)
}

/// Per-instrument positional lookup: for each `ModuleType`, the in-instrument
/// `ModuleId`s sorted ascending by `instance`. The audio engine resolves
/// `ModSource::Envelope(i)` and `ModDestination::OscPitch(i)` positionally —
/// i.e. as the i-th envelope / oscillator in the instrument's voice graph —
/// so MCP and GUI surfaces must do the same lookup to report the actual
/// module instance rather than a global-instance guess.
struct InstrumentModuleIndex {
    by_type: HashMap<synth_core::ModuleType, Vec<ModuleId>>,
}

impl InstrumentModuleIndex {
    fn from_snapshots(modules: &[synth_engine::ModuleStateSnapshot]) -> Self {
        let mut by_type: HashMap<synth_core::ModuleType, Vec<ModuleId>> = HashMap::new();
        for m in modules {
            by_type.entry(m.id.module_type).or_default().push(m.id);
        }
        for v in by_type.values_mut() {
            v.sort_by_key(|id| id.instance);
        }
        Self { by_type }
    }
}

/// Resolve a source address to the actual `ModuleId` in this instrument, or
/// `None` for macros (no `ModuleId`) and addresses that name an absent module.
fn resolve_source(source: SrcAddr, idx: &InstrumentModuleIndex) -> Option<ModuleId> {
    match source {
        SrcAddr::Macro(_) => None,
        SrcAddr::Module {
            module_type,
            instance,
            ..
        } => {
            let id = ModuleId::new(module_type, instance);
            idx.by_type.get(&module_type)?.contains(&id).then_some(id)
        }
    }
}

/// Resolve a destination address to `(ModuleId, param)` in this instrument, or
/// `None` when it names an absent module (a dangling routing).
fn resolve_destination(dest: DestAddr, idx: &InstrumentModuleIndex) -> Option<(ModuleId, String)> {
    let id = ModuleId::new(dest.module_type, dest.instance);
    idx.by_type
        .get(&dest.module_type)?
        .contains(&id)
        .then(|| (id, dest.param.as_str().to_string()))
}

/// Collect module IDs referenced as sources by any active Mod Matrix slot.
///
/// The Mod Matrix routes via parameter slots rather than cables, so an LFO,
/// Envelope, or Envelope Follower selected in `Slot N Source` is considered
/// "in use" by the diagnostic even if it has no cable connections.
fn collect_mod_matrix_sources(
    modules: &[synth_engine::ModuleStateSnapshot],
    idx: &InstrumentModuleIndex,
) -> HashSet<ModuleId> {
    active_mod_matrix_slots(modules)
        .filter_map(|s| s.source.and_then(|src| resolve_source(src, idx)))
        .collect()
}

/// Collect module IDs referenced as destinations by any active Mod Matrix
/// slot. Used to surface `"matrix"` as a virtual input port so destination
/// modules don't look unused in MCP responses.
fn collect_mod_matrix_destinations(
    modules: &[synth_engine::ModuleStateSnapshot],
    idx: &InstrumentModuleIndex,
) -> HashSet<ModuleId> {
    active_mod_matrix_slots(modules)
        .filter_map(|s| s.destination.and_then(|dst| resolve_destination(dst, idx)))
        .map(|(id, _)| id)
        .collect()
}

/// Collect the per-instance descriptions of an instrument's modules. Only
/// modules carrying a non-empty description are returned, in snapshot order.
/// Shared by `get_graph_diagnostics` and `analyze_note` so an AI agent sees the
/// patch's *intent* alongside its structural / signal analysis.
fn collect_module_descriptions(
    modules: &[synth_engine::ModuleStateSnapshot],
) -> Vec<synth_mcp::types::ModuleDescriptionEntry> {
    modules
        .iter()
        .filter(|m| !m.description.is_empty())
        .map(|m| synth_mcp::types::ModuleDescriptionEntry {
            module_id: m.id.to_string(),
            module_type: m.module_type.name().to_string(),
            name: m.name.clone(),
            description: m.description.clone(),
        })
        .collect()
}

fn meta_to_sample_info(meta: &synth_sampler::SampleMeta) -> synth_mcp::types::SampleInfo {
    synth_mcp::types::SampleInfo {
        id: meta.id.as_u64(),
        name: meta.name.clone(),
        description: meta.description.clone(),
        duration_seconds: meta.duration_seconds(),
        sample_rate: meta.sample_rate.as_u32(),
        channels: meta.channels.count(),
        frame_count: meta.frame_count.as_usize(),
        root_note: meta.root_note.map(synth_core::MidiNote::as_u8),
        loop_enabled: meta.loop_region.is_some(),
        has_crop: meta.crop.is_some(),
        source: match &meta.source {
            synth_sampler::SampleSource::Recorded => "recorded".to_string(),
            synth_sampler::SampleSource::Imported { .. } => "imported".to_string(),
            synth_sampler::SampleSource::Generated => "generated".to_string(),
        },
    }
}

impl AppSynthBridge {
    /// Extend an active transport loop's end so a newly-added placement
    /// isn't silently clipped. No-op if the loop is disabled or if the
    /// placement already fits.
    fn auto_extend_transport_loop(&self, new_end: synth_sequencer::Tick) {
        let (enabled, start, end) = self.session.transport_loop_state();
        if !enabled || new_end.0 <= end.0 {
            return;
        }
        let _ = self.session.set_transport_loop(start, new_end, true);
    }

    /// Apply a `ProjectFile` to the engine, stash it for the GUI to
    /// refresh on next frame, and update the path/status/revision
    /// metadata. The runtime guards against concurrent loads with the
    /// shared `project_io_lock`.
    fn do_load_project(&self, path: std::path::PathBuf) -> Result<String, McpBridgeError> {
        use crate::mcp_shared::ProjectRefresh;

        let _guard = self.shared.project_io_lock.lock();

        match self.load_project_inner(&path) {
            Ok((msg, project)) => {
                // Sync project-level metadata so an immediate follow-up
                // save (before the GUI's revision-gated refresh runs)
                // sees the loaded values rather than stale ones.
                self.set_shared_author(project.author.clone());
                self.stash_refresh(ProjectRefresh::Loaded(project));
                self.set_last_loaded_path(Some(path));
                self.clear_mix_baseline();
                self.record_io_result(Ok(msg))
            }
            Err(e) => self.record_io_result(Err(e)),
        }
    }

    fn load_project_inner(
        &self,
        path: &std::path::Path,
    ) -> Result<(String, Box<crate::project::ProjectFile>), McpBridgeError> {
        use crate::project::{LoadedFile, load_file};

        let project: Box<crate::project::ProjectFile> =
            match load_file(path).map_err(|e| McpBridgeError::Other(e.to_string()))? {
                LoadedFile::Project(p) => {
                    crate::project_apply::clear_sample_library(&self.sample_library);
                    p
                }
                LoadedFile::Bundle(bundle_path) => {
                    let mut lib = self
                        .sample_library
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    let p = crate::bundle::load_bundle(&bundle_path, &mut lib)
                        .map_err(|e| McpBridgeError::Other(e.to_string()))?;
                    Box::new(p)
                }
                LoadedFile::Patch(_) => {
                    return Err(McpBridgeError::Other(
                    "File is a single-instrument patch — use load_patch instead of load_project"
                        .to_string(),
                ));
                }
            };

        let mut report = crate::project_apply::apply_project(
            &project,
            &self.session,
            &self.shared.song,
            &self.sample_library,
        )
        .map_err(McpBridgeError::Other)?;

        // A caller that gets "Loaded <path>" back and nothing else has no way
        // to learn the file held an effect this build skipped. This path names
        // the file rather than the loader's instrument counts, so swap the
        // summary and let the report append what it lost — one formatting rule
        // for every caller. Reads exactly as before when the load was faithful.
        report.summary = format!("Loaded {}", path.display());
        Ok((report.summary_with_diagnostics(), project))
    }

    /// Save the current shared state as a project file (bundle if the
    /// sample library is non-empty, plain JSON otherwise).
    ///
    /// Module positions, group metadata, canvas size, instrument
    /// colour, and visualiser modules default to engine-only values
    /// because this path doesn't have access to a `PatchEditor` —
    /// re-loading an MCP-saved project into the GUI will collapse
    /// those fields. Use the GUI File-menu save when canvas fidelity
    /// matters.
    fn do_save_project(&self, path: std::path::PathBuf) -> Result<String, McpBridgeError> {
        let _guard = self.shared.project_io_lock.lock();

        // The command-drain barrier that keeps a save from reading a stale/truncated
        // graph (queued add_module/connect not yet mirrored) now lives inside
        // `build_project_from_engine`, the shared builder both branches below reach
        // (`save_project_to` and `save_project_as_bundle`), so every save path — GUI
        // and MCP — is covered by one mechanism.
        let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
        let path = crate::project::normalize_project_path(&path, has_samples);
        let opts = self.build_save_options();

        let result = if has_samples {
            self.save_project_as_bundle(&path, opts)
        } else {
            crate::project_apply::save_project_to(
                &path,
                &self.session,
                &self.shared.song,
                &self.sample_library,
                opts,
            )
            .map_err(McpBridgeError::Other)
        };

        self.record_io_result(result)
    }

    fn do_save_patch(
        &self,
        instrument_id: InstrumentId,
        path: std::path::PathBuf,
    ) -> Result<String, McpBridgeError> {
        let _guard = self.shared.project_io_lock.lock();

        let result = crate::project_apply::save_patch_to(&path, &self.session, instrument_id)
            .map_err(McpBridgeError::Other);

        self.record_io_result(result)
    }

    fn do_new_project(&self) -> Result<String, McpBridgeError> {
        use crate::mcp_shared::ProjectRefresh;

        let _guard = self.shared.project_io_lock.lock();

        let result = crate::project_apply::reset_to_new_project(
            &self.session,
            &self.shared.song,
            &self.sample_library,
        )
        .map(|report| report.summary_with_diagnostics())
        .map_err(McpBridgeError::Other);

        if result.is_ok() {
            self.set_shared_author(None);
            self.stash_refresh(ProjectRefresh::Reset);
            self.set_last_loaded_path(None);
            self.clear_mix_baseline();
        }
        self.record_io_result(result)
    }

    /// Construct a `ProjectFile` from engine state, bundle it with the
    /// current sample library, and write to disk.
    fn save_project_as_bundle(
        &self,
        path: &std::path::Path,
        opts: crate::project_apply::ProjectBuildOptions,
    ) -> Result<String, McpBridgeError> {
        let project = crate::project_apply::build_project_from_engine(
            &self.session,
            &self.shared.song,
            &self.sample_library,
            opts,
        );

        let lib = self
            .sample_library
            .read()
            .unwrap_or_else(|e| e.into_inner());

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| McpBridgeError::Other(format!("create parent dir: {e}")))?;
        }

        crate::bundle::save_bundle(&project, &lib, path)
            .map_err(|e| McpBridgeError::Other(e.to_string()))?;

        Ok(format!(
            "Saved bundle: {} ({} sample(s))",
            path.display(),
            lib.list().len()
        ))
    }

    fn stash_refresh(&self, refresh: crate::mcp_shared::ProjectRefresh) {
        *self
            .shared
            .pending_project_refresh
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(refresh);
    }

    /// Bump `gui_revision` so the GUI's revision-gated poll picks up a
    /// one-shot mirror payload (`pending_patch`)
    /// on its next frame. Same role as `record_io_status` plays for
    /// project I/O, but kept distinct because the GUI consumes them via
    /// separate paths.
    fn bump_gui_revision(&self) {
        self.shared
            .gui_revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// Record the outcome of a project I/O operation and bump
    /// `project_revision` so the GUI's revision-gated poll picks it
    /// up. Save events ride the same signal as load/new — there's no
    /// `pending_project_refresh` in the save case, but the status
    /// message still needs to surface in the GUI status line.
    fn record_io_status(&self, status: Result<String, String>) {
        *self
            .shared
            .last_project_io_status
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(status);
        self.shared
            .project_revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// Record a project-I/O `Result` (mapping the error to its string form) and
    /// pass it through unchanged, so callers can `self.record_io_result(result)`
    /// instead of repeating the `match { Ok => record; Err => record }` arms.
    fn record_io_result(
        &self,
        result: Result<String, McpBridgeError>,
    ) -> Result<String, McpBridgeError> {
        match &result {
            Ok(msg) => self.record_io_status(Ok(msg.clone())),
            Err(e) => self.record_io_status(Err(e.to_string())),
        }
        result
    }

    fn set_last_loaded_path(&self, path: Option<std::path::PathBuf>) {
        *self
            .shared
            .last_loaded_project_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = path;
    }

    /// Drop any captured `compare_mix_before_after` baseline. Called when the
    /// project changes (load / new) so a later `compare` can't silently A/B
    /// against an unrelated song.
    fn clear_mix_baseline(&self) {
        *self
            .shared
            .mix_baseline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn set_shared_author(&self, author: Option<crate::patch::Author>) {
        *self.shared.author.lock().unwrap_or_else(|e| e.into_inner()) = author;
    }

    /// Snapshot shared-state metadata (author) into save options for
    /// `project_apply::build_project_from_engine`.
    ///
    /// `glide_time` and `octave_offset` live only in the GUI, which publishes them
    /// into [`McpSharedState::gui_globals`]; they were unreachable from here and so
    /// were written as zeros — over whatever the user had set, on every MCP save
    /// and every rollback restore. They stay `None` when nothing has published
    /// them, which is the headless case, where zero *is* the value.
    fn build_save_options(&self) -> crate::project_apply::ProjectBuildOptions {
        let author = self
            .shared
            .author
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let globals = self
            .shared
            .gui_globals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .copied();
        crate::project_apply::ProjectBuildOptions {
            author,
            glide_time: globals.map(|g| g.glide_time),
            octave_offset: globals.map(|g| g.octave_offset),
            active_instrument_id: globals.and_then(|g| g.active_instrument_id),
        }
    }
}

/// Resolve a bridge-level glide spec into the sequencer's `Glide` type.
/// `from_pitch` (absolute) takes precedence; otherwise a relative semitone
/// offset is used (defaulting to -2 semitones).
fn glide_from_bridge(g: &BridgeGlide, target: synth_sequencer::Pitch) -> synth_sequencer::Glide {
    let from = match g.from_pitch {
        Some(p) => {
            synth_sequencer::GlideFrom::Pitch(synth_sequencer::Pitch::new(p).unwrap_or(target))
        }
        None => synth_sequencer::GlideFrom::Semitones(synth_core::Semitones::new(
            g.from_semitones.unwrap_or(-2.0),
        )),
    };
    synth_sequencer::Glide {
        from,
        time: synth_core::Milliseconds::new(g.time_ms),
        interp: if g.stepped {
            synth_sequencer::GlideInterp::Stepped
        } else {
            synth_sequencer::GlideInterp::Continuous
        },
    }
}

/// Resolve a vibrato-shape token to a `VibratoShape`, defaulting to sine when
/// absent. An unrecognized token is *rejected at validation* (see
/// `validate_note_input`), so reaching here with garbage falls back to sine.
fn vibrato_shape_from_token(token: Option<&str>) -> synth_sequencer::VibratoShape {
    token
        .and_then(synth_sequencer::VibratoShape::from_token)
        .unwrap_or(synth_sequencer::VibratoShape::Sine)
}

/// Resolve a bridge-level expression block into the sequencer's `NoteExpression`.
fn expression_from_bridge(e: &BridgeExpression) -> synth_sequencer::NoteExpression {
    synth_sequencer::NoteExpression {
        vibrato: e.vibrato.as_ref().map(|v| synth_sequencer::Vibrato {
            depth: synth_core::Semitones::new(v.depth),
            rate: synth_core::Hertz::new(v.rate),
            delay: synth_core::Milliseconds::new(v.delay_ms),
            shape: vibrato_shape_from_token(v.shape.as_deref()),
        }),
        accent: e.accent,
        gate: e.gate.map(synth_core::NormalizedValue::new),
        ghost: e.ghost,
        probability: e.probability.map(synth_core::NormalizedValue::new),
    }
}

/// Apply the per-note expression (legato/glide/expression block) from
/// `BridgeNoteData` to a note. An all-default expression block collapses to
/// `None` (matches the GUI editor's normalization).
fn apply_bridge_expression(note: &mut synth_sequencer::Note, n: &BridgeNoteData) {
    note.legato = n.legato;
    note.glide = n.glide.as_ref().map(|g| glide_from_bridge(g, note.pitch));
    note.expression = n
        .expression
        .as_ref()
        .map(expression_from_bridge)
        .and_then(synth_sequencer::NoteExpression::normalized);
}

/// Try to insert a note from `BridgeNoteData` into a pattern.
/// Returns the assigned note ID as u64, or an error string if the pitch is invalid.
fn try_insert_note_into_pattern(
    pattern: &mut synth_sequencer::Pattern,
    n: &BridgeNoteData,
) -> Result<u64, String> {
    let pitch = synth_sequencer::Pitch::new(n.pitch.as_u8())
        .ok_or_else(|| format!("invalid pitch: {} (must be 0..=127)", n.pitch))?;
    let start = synth_sequencer::PatternTick(beats_to_ticks(n.start_beat));
    let vel = synth_core::Velocity::from_midi(n.velocity);

    let mut note = synth_sequencer::Note::new(
        synth_sequencer::NoteId(0), // reassigned by insert_note
        start,
        pitch,
        vel,
    )
    .with_duration(synth_sequencer::Duration(beats_to_ticks(n.duration_beats)));
    apply_bridge_expression(&mut note, n);

    Ok(pattern.insert_note(note).0)
}

/// Insert a note from `BridgeNoteData` into a pattern. Returns the assigned note ID as u64.
/// Falls back to middle C for invalid pitches (used in bulk import paths where per-note
/// errors are not reported).
fn insert_note_into_pattern(pattern: &mut synth_sequencer::Pattern, n: &BridgeNoteData) -> u64 {
    let pitch =
        synth_sequencer::Pitch::new(n.pitch.as_u8()).unwrap_or(synth_sequencer::Pitch::MIDDLE_C);
    let start = synth_sequencer::PatternTick(beats_to_ticks(n.start_beat));
    let vel = synth_core::Velocity::from_midi(n.velocity);

    let mut note = synth_sequencer::Note::new(
        synth_sequencer::NoteId(0), // reassigned by insert_note
        start,
        pitch,
        vel,
    )
    .with_duration(synth_sequencer::Duration(beats_to_ticks(n.duration_beats)));
    apply_bridge_expression(&mut note, n);

    pattern.insert_note(note).0
}

/// Convert beats (float) to ticks (u32). 1 beat = 960 ticks.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn beats_to_ticks(beats: f32) -> u32 {
    (beats * synth_sequencer::TICKS_PER_QUARTER as f32).round() as u32
}

fn placement_from_bridge(data: &BridgePlacementData) -> synth_sequencer::PatternPlacement {
    let mut placement =
        synth_sequencer::PatternPlacement::new(data.pattern_id, data.track_id, data.start.tick())
            .with_transpose(synth_core::Semitones::new(data.transpose_semitones))
            .with_gain(synth_core::Gain::new(data.gain));
    placement.length_override = data.length_ticks.map(synth_sequencer::Duration);
    placement.loop_mode = data.loop_mode;
    placement
}

/// Convert ticks (u32) to beats (float).
#[allow(clippy::cast_precision_loss)]
fn ticks_to_beats(ticks: u32) -> f32 {
    ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
}

/// Normalize a parameter name for fuzzy matching (lowercase, underscores →
/// spaces). Re-exported from `synth_core` rather than re-spelled here, so this
/// crate's name-matching folds exactly the way
/// [`ModuleDescriptor::find_parameter`](synth_core::ModuleDescriptor::find_parameter)
/// does.
use synth_core::normalize_param_name;

/// Strip every non-alphanumeric character and lowercase the rest, so that the
/// prefix, `snake_case`, `CamelCase`, and spaced display forms of a name all
/// collapse to the same comparable token (e.g. `"Math Oscillator"`,
/// `"math_oscillator"`, `"MathOscillator"` → `"mathoscillator"`).
fn squash_token(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Resolve a bridge parameter value (number / bool / string choice) to the
/// parameter's native `f32`.
///
/// Numbers and booleans map directly (`true`/`false` → `1.0`/`0.0`). A string
/// is treated as a choice and matched case-insensitively against the
/// descriptor's choice ids **and** display names, returning the choice index.
/// Unknown choices — or a string aimed at a non-choice parameter — are errors,
/// so bad input is rejected at the boundary rather than silently becoming the
/// first option.
fn resolve_param_value(
    value: &BridgeParamValue,
    pd: Option<&synth_core::ParameterDescriptor>,
    param_name: &str,
) -> Result<f32, McpBridgeError> {
    match value {
        BridgeParamValue::Number(n) => Ok(*n as f32),
        BridgeParamValue::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        BridgeParamValue::Choice(s) => {
            let Some(pd) = pd else {
                return Err(McpBridgeError::InvalidChoice {
                    name: param_name.to_string(),
                    value: s.clone(),
                    detail: "parameter descriptor unavailable; pass a numeric value".to_string(),
                });
            };
            let Some(choices) = pd.choices.as_ref() else {
                return Err(McpBridgeError::InvalidChoice {
                    name: pd.name.clone(),
                    value: s.clone(),
                    detail: "this parameter takes a number, not a string".to_string(),
                });
            };
            choices
                .iter()
                .position(|c| c.id.eq_ignore_ascii_case(s) || c.name.eq_ignore_ascii_case(s))
                .map(|i| i as f32)
                .ok_or_else(|| McpBridgeError::InvalidChoice {
                    name: pd.name.clone(),
                    value: s.clone(),
                    detail: format!(
                        "valid choices are {}",
                        choices
                            .iter()
                            .map(|c| c.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })
        }
    }
}

/// Build MCP `ReturnEffectInfo` from an engine return-effect snapshot, resolving
/// each parameter's name / type-id / unit via the effect type's descriptor.
fn return_effect_info(effect: &synth_engine::ReturnEffectSnapshot) -> synth_mcp::ReturnEffectInfo {
    let descriptor = crate::module_factory::get_descriptor(effect.module_type);
    let parameters = effect
        .parameters
        .iter()
        .map(|p| {
            let pd = descriptor
                .as_ref()
                .and_then(|d| d.parameters.iter().find(|pd| pd.id.same_kind(p)));
            let value = p.as_f32();
            synth_mcp::ReturnEffectParamInfo {
                name: pd.map(|pd| pd.name.clone()).unwrap_or_default(),
                type_id: pd.map(|pd| pd.type_id.clone()).unwrap_or_default(),
                value,
                display: pd.map_or_else(|| format!("{value}"), |pd| pd.format(value)),
                value_kind: pd.map(|pd| pd.kind),
            }
        })
        .collect();
    synth_mcp::ReturnEffectInfo {
        module_id: effect.module_id.to_string(),
        effect_type: effect.module_type.prefix().to_string(),
        bypassed: effect.bypassed,
        parameters,
    }
}

/// Resolve a client-supplied module-type token to a [`ModuleType`].
///
/// Tries the canonical forms first via [`ModuleType::from_token`] (prefix +
/// `snake_case`/spaced name), then falls back to a separator-insensitive match
/// against each known type's display name so that anything `list_module_types`
/// advertises — its `type_key` *or* its `name` — round-trips, along with
/// `CamelCase` variants.
fn parse_module_type(token: &str) -> Option<synth_core::ModuleType> {
    if let Some(mt) = synth_core::ModuleType::from_token(token) {
        return Some(mt);
    }
    let squashed = squash_token(token);
    crate::module_factory::ALL_MODULE_TYPES
        .iter()
        .copied()
        .find(|mt| squash_token(mt.name()) == squashed)
}

/// Extract Mod Matrix routings from a `ModMatrix` module's parameter snapshot.
/// Each routing reports its source and destination as **address strings**
/// (`"lfo-3.out"`, `"velocity"`, `"flt-1.cutoff"`) — so arbitrary addresses are
/// named directly rather than lost to a legacy-enum round-trip. Fully-empty
/// routings (no source and no destination) are skipped.
fn collect_mod_matrix_routings(
    params: &[Param],
    scripts: &std::collections::BTreeMap<String, String>,
) -> Vec<MatrixRoutingInfo> {
    let decoded = decode_mod_matrix_slots(params);
    // `decode_mod_matrix_slots` only densifies up to the highest *param* slot, so
    // a script-only slot (no source/dest params) can sit beyond it — extend the
    // iteration to cover the highest 1-based scripted slot too.
    let max_scripted = scripts
        .keys()
        .filter_map(|k| k.parse::<usize>().ok())
        .filter(|n| (1..=synth_core::MAX_MOD_MATRIX_SLOTS).contains(n))
        .max()
        .unwrap_or(0);
    let count = decoded.len().max(max_scripted);

    (0..count)
        .filter_map(|i| {
            let slot = decoded.get(i).cloned().unwrap_or_default();
            // Scripts are keyed by 1-based slot number (matching the report's
            // `slot` field), mirroring the persisted `ModuleState.scripts`.
            let script = scripts.get(&(i + 1).to_string()).cloned();
            // Report a slot with a source/dest OR a script (a scripted slot may
            // have no scalar source — the script supplies the value).
            if slot.source.is_none() && slot.destination.is_none() && script.is_none() {
                return None;
            }
            let source = slot
                .source
                .map_or_else(|| "none".to_string(), |a| a.to_address_string());
            let destination = slot
                .destination
                .map_or_else(|| "none".to_string(), |a| a.to_address_string());
            Some(MatrixRoutingInfo {
                slot: i as u8 + 1,
                source,
                destination,
                amount: slot.amount,
                enabled: slot.enabled,
                script,
            })
        })
        .collect()
}

/// Parse an externally-tagged `NoteModuleConfig` (accepting a stringified-JSON
/// payload for LLM clients that send the config as a string).
fn parse_note_module(
    value: serde_json::Value,
) -> Result<synth_sequencer::NoteModuleConfig, McpBridgeError> {
    let value = match value {
        serde_json::Value::String(s) => {
            serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
        }
        other => other,
    };
    serde_json::from_value(value)
        .map_err(|e| McpBridgeError::Other(format!("invalid note graph module: {e}")))
}

/// Parse a connection port name (`note_stream` / `value` / `gate`).
fn parse_note_port(port: &str) -> Result<synth_sequencer::NotePortType, McpBridgeError> {
    match port {
        "note_stream" => Ok(synth_sequencer::NotePortType::NoteStream),
        "value" => Ok(synth_sequencer::NotePortType::Value),
        "gate" => Ok(synth_sequencer::NotePortType::Gate),
        other => Err(McpBridgeError::Other(format!(
            "invalid port '{other}' (expected one of: note_stream, value, gate)"
        ))),
    }
}

/// Snake_case name of a connection port, for MCP readers.
fn note_port_to_str(port: synth_sequencer::NotePortType) -> &'static str {
    match port {
        synth_sequencer::NotePortType::NoteStream => "note_stream",
        synth_sequencer::NotePortType::Value => "value",
        synth_sequencer::NotePortType::Gate => "gate",
    }
}

fn validated_description(description: Option<String>) -> Result<String, McpBridgeError> {
    let description = description.unwrap_or_default();
    let len = description.chars().count();
    if len > MAX_MODULE_DESCRIPTION_LEN {
        return Err(McpBridgeError::DescriptionTooLong {
            len,
            max: MAX_MODULE_DESCRIPTION_LEN,
        });
    }
    Ok(description)
}

/// Build the summary `NoteGraphInfo` for a pooled graph.
fn note_graph_info(
    song: &synth_sequencer::Song,
    graph: &synth_sequencer::NoteGraph,
) -> NoteGraphInfo {
    NoteGraphInfo {
        id: graph.id,
        name: graph.name.clone(),
        description: graph.description.clone(),
        color: graph.color.map(|c| c.to_hex()),
        node_count: graph.node_count(),
        connection_count: graph.connections().len(),
        used_by_patterns: song.note_graph_usage(graph.id),
    }
}

/// Build the summary `ModGraphInfo` for a pooled mod graph.
fn mod_graph_info(graph: &synth_sequencer::ModGraph) -> ModGraphInfo {
    ModGraphInfo {
        id: graph.id,
        name: graph.name.clone(),
        description: graph.description.clone(),
        scope: match graph.scope {
            synth_sequencer::ModGraphScope::Global => "global".to_string(),
            synth_sequencer::ModGraphScope::Track => "track".to_string(),
        },
        assigned_tracks: graph.assigned_tracks.clone(),
        color: graph.color.map(|c| c.to_hex()),
        node_count: graph.node_count(),
        connection_count: graph.connections().len(),
    }
}

/// Parse a mod-graph scope name (`global` / `track`), defaulting to `global`.
fn parse_mod_graph_scope(
    scope: Option<&str>,
) -> Result<synth_sequencer::ModGraphScope, McpBridgeError> {
    match scope.map(str::trim) {
        None | Some("") | Some("global") => Ok(synth_sequencer::ModGraphScope::Global),
        Some("track") => Ok(synth_sequencer::ModGraphScope::Track),
        Some(other) => Err(McpBridgeError::Other(format!(
            "invalid scope '{other}' (expected one of: global, track)"
        ))),
    }
}

/// Parse an externally-tagged `ModNodeConfig` (accepting a stringified-JSON
/// payload for LLM clients that send the config as a string).
fn parse_mod_node(
    value: serde_json::Value,
) -> Result<synth_sequencer::ModNodeConfig, McpBridgeError> {
    let value = match value {
        serde_json::Value::String(s) => {
            serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
        }
        other => other,
    };
    let mut config: synth_sequencer::ModNodeConfig = serde_json::from_value(value)
        .map_err(|e| McpBridgeError::Other(format!("invalid mod graph node: {e}")))?;
    if let synth_sequencer::ModNodeConfig::Module(module) = &mut config {
        let Some((_, descriptor)) = crate::module_factory::create_voice_module(module.module_type)
        else {
            return Err(McpBridgeError::Other(format!(
                "module type '{:?}' cannot be hosted in a Mod Grid",
                module.module_type
            )));
        };
        for (type_id, value) in &mut module.params {
            let param = descriptor
                .parameters
                .iter()
                .find(|param| param.type_id == *type_id)
                .ok_or_else(|| {
                    McpBridgeError::Other(format!(
                        "module '{:?}' has no parameter '{type_id}'{}",
                        module.module_type,
                        descriptor.param_id_hint(type_id)
                    ))
                })?;
            *value = param.validate_f32(*value).map_err(|error| {
                McpBridgeError::Other(format!(
                    "invalid value for '{:?}.{}': {error}",
                    module.module_type, type_id
                ))
            })?;
        }
    }
    Ok(config)
}

/// Validate Mod Grid cable endpoints against hosted module descriptors and the
/// fixed ports of grid-native source/target nodes.
fn validate_mod_connection_ports(
    graph: &synth_sequencer::ModGraph,
    from: ModNodeId,
    from_port: &str,
    to: ModNodeId,
    to_port: &str,
) -> Result<(), McpBridgeError> {
    let from_config = graph
        .node(from)
        .ok_or(McpBridgeError::ModGraphNodeNotFound {
            graph_id: graph.id,
            node_id: from,
        })?;
    let to_config = graph.node(to).ok_or(McpBridgeError::ModGraphNodeNotFound {
        graph_id: graph.id,
        node_id: to,
    })?;

    let valid_module_port =
        |module: &synth_sequencer::ModuleNode, port: &str, direction: PortDirection| {
            crate::module_factory::create_voice_module(module.module_type).is_some_and(
                |(_, descriptor)| {
                    descriptor
                        .ports
                        .iter()
                        .any(|candidate| candidate.direction == direction && candidate.name == port)
                },
            )
        };

    let source_valid = match from_config {
        synth_sequencer::ModNodeConfig::Module(module) => {
            valid_module_port(module, from_port, PortDirection::Output)
        }
        synth_sequencer::ModNodeConfig::Macro(_)
        | synth_sequencer::ModNodeConfig::Transport(_)
        | synth_sequencer::ModNodeConfig::MidiCc(_)
        | synth_sequencer::ModNodeConfig::AudioTap(_) => from_port == "out",
        synth_sequencer::ModNodeConfig::Target(_) => false,
    };
    if !source_valid {
        return Err(McpBridgeError::Other(format!(
            "node {from} has no output port '{from_port}'"
        )));
    }

    let target_valid = match to_config {
        synth_sequencer::ModNodeConfig::Module(module) => {
            valid_module_port(module, to_port, PortDirection::Input)
        }
        synth_sequencer::ModNodeConfig::Target(_) => to_port == synth_sequencer::TARGET_INPUT_PORT,
        synth_sequencer::ModNodeConfig::Macro(_)
        | synth_sequencer::ModNodeConfig::Transport(_)
        | synth_sequencer::ModNodeConfig::MidiCc(_)
        | synth_sequencer::ModNodeConfig::AudioTap(_) => false,
    };
    if !target_valid {
        return Err(McpBridgeError::Other(format!(
            "node {to} has no input port '{to_port}'"
        )));
    }
    Ok(())
}

fn note_to_info(n: &synth_sequencer::Note) -> NoteInfo {
    NoteInfo {
        id: n.id,
        pitch: MidiNote::new(n.pitch.as_midi()),
        pitch_name: n.pitch.to_string(),
        start_beat: ticks_to_beats(n.start.0),
        duration_beats: n.duration.map_or(1.0, |d| ticks_to_beats(d.0)),
        velocity: synth_core::Velocity::to_midi(n.velocity),
        ornament: n
            .ornament
            .as_ref()
            .and_then(|o| serde_json::to_value(o).ok()),
        note_graph: n.note_graph.map(|g| g.0),
    }
}

/// Convert ticks (u64) to beats (float).
#[allow(clippy::cast_precision_loss)]
fn ticks_to_beats_u64(ticks: u64) -> f32 {
    ticks as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
}

/// Snapshot a song's tempo map as MCP `TempoPoint`s (sorted by tick).
fn tempo_points(song: &synth_sequencer::Song) -> Vec<TempoPoint> {
    song.tempo_changes()
        .iter()
        .map(|c| TempoPoint {
            tick: c.tick,
            bpm: c.bpm,
            ramp: c.ramp,
        })
        .collect()
}

/// The automation DSL's instrument-level parameter names.
///
/// A table rather than a `match` arm so the parser and the near-miss hint read
/// the same list: a name reachable by one is offered by the other, and neither
/// can quietly fall behind.
const AUTO_INSTRUMENT_PARAMS: &[(&str, synth_sequencer::AutoInstrumentParam)] = {
    use synth_sequencer::AutoInstrumentParam as P;
    &[
        ("Volume", P::Volume),
        ("Pan", P::Pan),
        ("FilterCutoff", P::FilterCutoff),
        ("FilterResonance", P::FilterResonance),
        ("Attack", P::Attack),
        ("Decay", P::Decay),
        ("Sustain", P::Sustain),
        ("Release", P::Release),
    ]
};

/// The automation DSL's track-parameter names, read straight off the enum
/// through `display_name` — which is also what [`automation_target_info`]
/// renders, so parser, hint and renderer cannot disagree.
///
/// A hand-written table here would be a *fourth* copy of a list the enum already
/// owns (`TrackParam::ALL`), and a new variant would render to a `track:<name>`
/// string the parser then rejected — a lane `list_automation_lanes` reports that
/// no other automation tool can address.
fn track_param_names() -> impl Iterator<Item = &'static str> {
    synth_sequencer::TrackParam::ALL
        .iter()
        .map(synth_sequencer::TrackParam::display_name)
}

/// The automation DSL's global-parameter names.
const GLOBAL_PARAMS: &[(&str, synth_sequencer::GlobalParam)] =
    &[("MasterVolume", synth_sequencer::GlobalParam::MasterVolume)];

/// Look a name up in one of the DSL tables above. Case-sensitive, as the DSL
/// has always been — a case slip is recovered by the hint instead, which
/// matches case-insensitively.
fn lookup<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    table
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, value)| *value)
}

/// A hint naming the closest entries in one of the DSL tables.
fn table_hint<T>(table: &[(&str, T)], name: &str) -> String {
    synth_core::suggest::did_you_mean(
        name,
        table.iter().map(|(candidate, _)| *candidate),
        synth_core::suggest::DEFAULT_MAX_HINTS,
    )
}

/// Parse a parameter name string to `AutoInstrumentParam`.
fn parse_auto_instrument_param(name: &str) -> Option<synth_sequencer::AutoInstrumentParam> {
    lookup(AUTO_INSTRUMENT_PARAMS, name)
}

/// Parse a track-parameter name (case-sensitive, matching `TrackParam`'s
/// `display_name`): `Volume`, `Pan`, `Mute`, `Pitch`.
fn parse_track_param(name: &str) -> Option<synth_sequencer::TrackParam> {
    synth_sequencer::TrackParam::ALL
        .iter()
        .copied()
        .find(|param| param.display_name() == name)
}

/// Parse a global-parameter name: `MasterVolume`.
fn parse_global_param(name: &str) -> Option<synth_sequencer::GlobalParam> {
    lookup(GLOBAL_PARAMS, name)
}

/// Map a bridge [`CurveKind`](synth_mcp::bridge::CurveKind) plus optional
/// strength to the sequencer's `CurveType`. The strength is only meaningful for
/// `Exponential` (validated as `-127..=127` by the type); it defaults to 0.
fn curve_from_kind(
    kind: synth_mcp::bridge::CurveKind,
    strength: Option<synth_sequencer::CurveStrength>,
) -> synth_sequencer::CurveType {
    use synth_mcp::bridge::CurveKind;
    use synth_sequencer::CurveType;
    match kind {
        CurveKind::Linear => CurveType::Linear,
        CurveKind::Step => CurveType::Step,
        CurveKind::Exponential => {
            CurveType::Exponential(strength.unwrap_or(synth_sequencer::CurveStrength::ZERO))
        }
        CurveKind::SCurve => CurveType::SCurve,
    }
}

/// Format a `CurveType` to a string.
fn format_curve_type(curve: synth_sequencer::CurveType) -> String {
    use synth_sequencer::CurveType;
    match curve {
        CurveType::Linear => "Linear".to_string(),
        CurveType::Step => "Step".to_string(),
        CurveType::Exponential(strength) => format!("Exponential({strength})"),
        CurveType::SCurve => "SCurve".to_string(),
    }
}

/// Build an `AutomationTarget` from a target string and instrument ID.
///
/// Two forms are accepted:
/// - An instrument-level param name (e.g. `"FilterCutoff"`) →
///   [`AutomationTarget::Instrument`](synth_sequencer::AutomationTarget::Instrument).
/// - A generic module-parameter target `"module:<type>:<instance>:<param_id>"`
///   (e.g. `"module:flt:1:cutoff"`, or the `ModuleId`-style dash form
///   `"module:flt-1:cutoff"` that every other tool uses) →
///   [`AutomationTarget::Module`](synth_sequencer::AutomationTarget::Module).
///   This is the inverse of [`automation_target_info`]'s Module rendering.
///
/// Module targets are validated against the automation allowlist (the parameter
/// must exist on that module type's descriptor and be
/// [`ParameterDescriptor::is_automatable`](synth_core::ParameterDescriptor::is_automatable))
/// AND against `valid_modules` — the `ModuleId`s that actually exist in the
/// target instrument's graph — so a target can't bind to a non-existent module
/// instance (which would be silently dead automation). Pass the instrument's
/// live module ids (see `instrument_module_ids`); instrument-level params ignore
/// it.
fn build_automation_target(
    target: &str,
    instrument_id: InstrumentId,
    valid_modules: &[synth_engine::ModuleId],
) -> Result<synth_sequencer::AutomationTarget, McpBridgeError> {
    let instrument = instrument_id;

    if let Some(rest) = target.strip_prefix("module:") {
        return build_module_automation_target(rest, instrument, valid_modules);
    }

    // Track lane: `track:<param>` (host track — follows the placement) or
    // `track:<param>:<track_id>` (a specific track, cross-track automation).
    if let Some(rest) = target.strip_prefix("track:") {
        let (param_str, track) = match rest.split_once(':') {
            Some((p, id_str)) => {
                let id: u16 = id_str.parse().map_err(|_| {
                    McpBridgeError::Other(format!("invalid track id in target: '{id_str}'"))
                })?;
                (p, Some(TrackId(id)))
            }
            None => (rest, None),
        };
        let param = parse_track_param(param_str).ok_or_else(|| {
            McpBridgeError::Other(format!(
                "unknown track param: '{param_str}'{}",
                synth_core::suggest::did_you_mean(
                    param_str,
                    track_param_names(),
                    synth_core::suggest::DEFAULT_MAX_HINTS,
                )
            ))
        })?;
        return Ok(synth_sequencer::AutomationTarget::Track { track, param });
    }

    // Global lane: `global:<param>` (e.g. `global:MasterVolume`).
    if let Some(rest) = target.strip_prefix("global:") {
        let param = parse_global_param(rest).ok_or_else(|| {
            McpBridgeError::Other(format!(
                "unknown global param: '{rest}'{}",
                table_hint(GLOBAL_PARAMS, rest)
            ))
        })?;
        return Ok(synth_sequencer::AutomationTarget::Global(param));
    }

    let param = parse_auto_instrument_param(target).ok_or_else(|| {
        McpBridgeError::Other(format!(
            "unknown automation param: '{target}'{}",
            table_hint(AUTO_INSTRUMENT_PARAMS, target)
        ))
    })?;
    Ok(synth_sequencer::AutomationTarget::Instrument { instrument, param })
}

/// Parse and validate a module-parameter automation target from the body after
/// `"module:"`. Accepts both `"<type>:<instance>:<param_id>"` (colon form) and
/// `"<type>-<instance>:<param_id>"` (the dash form that mirrors the `ModuleId`
/// rendering — e.g. `"flt-1"` — used by every other tool). The `param_id` is
/// the final `':'`-separated segment; the module identity precedes it and may
/// separate its type token from the instance index with `':'` or `'-'`. The
/// type token itself is resolved leniently (prefix or full name) via
/// [`parse_module_type`].
fn build_module_automation_target(
    body: &str,
    instrument: synth_sequencer::InstrumentId,
    valid_modules: &[synth_engine::ModuleId],
) -> Result<synth_sequencer::AutomationTarget, McpBridgeError> {
    let (module_type, instance, param_id) = parse_module_automation_target(body)?;
    let prefix = module_type.prefix();

    // Validate the instance actually exists in the instrument's graph, so a
    // target can't silently bind to a missing module. Name the instrument and
    // point at instrument_id: the module often *does* exist, just on another
    // instrument, and instrument_id defaults to 0 when omitted — so the bare
    // "no such module" wording otherwise sends callers hunting a phantom bug.
    // (The live-authoring path additionally reports which instrument owns it.)
    if !valid_modules.contains(&synth_engine::ModuleId::new(module_type, instance)) {
        return Err(McpBridgeError::Other(format!(
            "instrument {} has no '{prefix}-{instance}' module to automate — check \
             instrument_id (it defaults to 0)",
            instrument.as_u64()
        )));
    }

    // Validate against the allowlist: the param must exist on this module type
    // and be automatable (numeric scalar — continuous or stepped integer —
    // and RT-safe; non-choice).
    let descriptor = crate::module_factory::get_descriptor(module_type).ok_or_else(|| {
        McpBridgeError::Other(format!("no descriptor for module type '{prefix}'"))
    })?;
    let param = descriptor
        .parameters
        .iter()
        .find(|p| p.type_id == param_id)
        .ok_or_else(|| {
            McpBridgeError::Other(format!(
                "module '{prefix}' has no parameter '{param_id}'{}",
                descriptor.param_id_hint(param_id)
            ))
        })?;
    if !param.is_automatable() {
        return Err(McpBridgeError::Other(format!(
            "parameter '{param_id}' on module '{prefix}' is not automatable"
        )));
    }

    Ok(synth_sequencer::AutomationTarget::Module {
        instrument,
        module_type,
        instance,
        param_id: synth_sequencer::ParamId::from(param_id),
    })
}

fn parse_module_automation_target(body: &str) -> Result<(ModuleType, u16, &str), McpBridgeError> {
    let malformed = || {
        McpBridgeError::Other(format!(
            "module target must be 'module:<type>:<instance>:<param>' \
             (or 'module:<type>-<instance>:<param>'), got 'module:{body}'"
        ))
    };
    let (module_ref, param_id) = body.rsplit_once(':').ok_or_else(malformed)?;
    // Split off the instance from the END: the instance is always the trailing
    // token, while the type token itself may contain a '-' (a hyphenated
    // multi-word name like "ladder-filter" that `parse_module_type` accepts).
    // Splitting on the first separator would mis-slice such names.
    let (type_token, instance_str) = module_ref.rsplit_once([':', '-']).ok_or_else(malformed)?;

    let module_type = parse_module_type(type_token).ok_or_else(|| {
        McpBridgeError::Other(format!(
            "unknown module type: '{type_token}'{}",
            synth_core::ModuleType::suggestion_hint(type_token)
        ))
    })?;
    let instance: u16 = instance_str
        .parse()
        .map_err(|_| McpBridgeError::Other(format!("invalid module instance: '{instance_str}'")))?;
    Ok((module_type, instance, param_id))
}

/// Extract a round-trippable target string, optional instrument ID, and scope
/// tag (`instrument`/`module`/`track`/`global`) from an `AutomationTarget`.
/// The string is parseable back by [`build_automation_target`], so a lane read
/// from `list_automation_lanes` can be addressed by the other automation tools.
fn automation_target_info(
    target: &synth_sequencer::AutomationTarget,
) -> (String, Option<InstrumentId>, &'static str) {
    use synth_sequencer::AutoInstrumentParam;
    match target {
        synth_sequencer::AutomationTarget::Instrument { instrument, param } => {
            let name = match param {
                AutoInstrumentParam::Volume => "Volume",
                AutoInstrumentParam::Pan => "Pan",
                AutoInstrumentParam::FilterCutoff => "FilterCutoff",
                AutoInstrumentParam::FilterResonance => "FilterResonance",
                AutoInstrumentParam::Attack => "Attack",
                AutoInstrumentParam::Decay => "Decay",
                AutoInstrumentParam::Sustain => "Sustain",
                AutoInstrumentParam::Release => "Release",
            };
            (name.to_string(), Some(*instrument), "instrument")
        }
        synth_sequencer::AutomationTarget::Track { track, param } => {
            // `track:<param>` (host) or `track:<param>:<id>` (specific track).
            // Track lanes are not instrument-scoped, so instrument_id is None.
            let name = param.display_name();
            let s = match track {
                Some(t) => format!("track:{name}:{}", t.0),
                None => format!("track:{name}"),
            };
            (s, None, "track")
        }
        synth_sequencer::AutomationTarget::Global(param) => {
            // Explicit token (not Debug) so the wire form stays decoupled from
            // the enum's Debug output — mirrors the Instrument arm and pairs
            // with `parse_global_param`.
            let name = match param {
                synth_sequencer::GlobalParam::MasterVolume => "MasterVolume",
            };
            (format!("global:{name}"), None, "global")
        }
        synth_sequencer::AutomationTarget::Module {
            instrument,
            module_type,
            instance,
            param_id,
        } => (
            // Canonical, round-trippable form parsed by `build_automation_target`.
            format!("module:{}:{instance}:{param_id}", module_type.prefix()),
            Some(*instrument),
            "module",
        ),
    }
}

/// Reconstruct the value the engine would produce at `tick` on the segment
/// `a → b`, using `a`'s own interpolation curve (the curve stored on a point
/// governs the segment leaving it). This is exactly what playback does once the
/// interior points between `a` and `b` are removed, so it is the right yardstick
/// for measuring simplification error.
fn reconstruct_on_segment(
    a: &synth_sequencer::AutomationPoint,
    b: &synth_sequencer::AutomationPoint,
    tick: synth_sequencer::PatternTick,
) -> f32 {
    let span = b.tick.0.saturating_sub(a.tick.0);
    if span == 0 {
        return a.value.as_f32();
    }
    let t = synth_core::NormalizedValue::new((tick.0 - a.tick.0) as f32 / span as f32);
    a.curve.interpolate(a.value, b.value, t).as_f32()
}

/// Curve-aware Douglas–Peucker over `points[lo..=hi]` (both anchors already
/// kept). Marks the interior points that must survive so every dropped point
/// stays within `tolerance` of the value the surrounding segment reproduces.
/// Iterative (explicit stack) so a 25k-point lane can't blow the call stack.
/// Updates `max_err` with the largest error actually accepted.
fn rdp_simplify(
    points: &[synth_sequencer::AutomationPoint],
    lo: usize,
    hi: usize,
    tolerance: f32,
    keep: &mut [bool],
    max_err: &mut f32,
) {
    let mut stack = vec![(lo, hi)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let a = &points[lo];
        let b = &points[hi];
        let mut worst = 0.0f32;
        let mut worst_idx = 0usize;
        for k in (lo + 1)..hi {
            let err =
                (points[k].value.as_f32() - reconstruct_on_segment(a, b, points[k].tick)).abs();
            if err > worst {
                worst = err;
                worst_idx = k;
            }
        }
        if worst > tolerance {
            keep[worst_idx] = true;
            stack.push((lo, worst_idx));
            stack.push((worst_idx, hi));
        } else if worst > *max_err {
            // Whole interior dropped within tolerance — record its true error.
            *max_err = worst;
        }
    }
}

/// Simplify a sorted automation-point list: drop points whose value the
/// surrounding segment reproduces within `tolerance` (normalized 0..1 units).
///
/// Step points, and the point that ends each Step hold, are always kept, and no
/// segment is simplified across a Step boundary — so hold timing is preserved
/// exactly. Linear / Exponential / S-Curve segments are measured with their own
/// interpolation. Returns the kept points (sorted) and the maximum
/// reconstruction error incurred.
fn simplify_automation_points(
    points: &[synth_sequencer::AutomationPoint],
    tolerance: f32,
) -> (Vec<synth_sequencer::AutomationPoint>, f32) {
    let n = points.len();
    if n <= 2 {
        return (points.to_vec(), 0.0);
    }
    let tolerance = tolerance.max(0.0);

    // Force-keep the endpoints, every Step point (it starts a hold), and the
    // point right after a Step (it lands the jump / ends the hold).
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    for i in 0..n {
        if matches!(points[i].curve, synth_sequencer::CurveType::Step) {
            keep[i] = true;
            if i + 1 < n {
                keep[i + 1] = true;
            }
        }
    }

    // Run curve-aware RDP within each run between consecutive force-kept anchors
    // (no run spans a Step boundary, so hold segments are never flattened).
    let anchors: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
    let mut max_err = 0.0f32;
    for w in anchors.windows(2) {
        if w[1] > w[0] + 1 {
            rdp_simplify(points, w[0], w[1], tolerance, &mut keep, &mut max_err);
        }
    }

    let kept = (0..n).filter(|&i| keep[i]).map(|i| points[i]).collect();
    (kept, max_err)
}

/// Insert automation points from `BridgeAutomationPointData` into a pattern.
///
/// `module_cache` maps each point's `instrument_id` to that instrument's live
/// module ids, used to validate `module:` targets (built by the caller before
/// the song lock, see `module_id_cache`).
///
/// Returns the number of points skipped because their target was unknown,
/// non-automatable, or named a module that doesn't exist — so the caller can
/// surface them instead of dropping them silently.
fn insert_automation_into_pattern(
    pattern: &mut synth_sequencer::Pattern,
    points: &[BridgeAutomationPointData],
    module_cache: &std::collections::HashMap<InstrumentId, Vec<synth_engine::ModuleId>>,
) -> usize {
    use synth_sequencer::{AutomationPoint, PatternTick};

    let mut skipped = 0usize;
    for pt in points {
        // Use the shared builder so bulk pattern creation accepts
        // `module:<type>:<instance>:<param>` targets (validated against the
        // automatable allowlist + instrument graph), not just plain instrument
        // params.
        let valid = module_cache
            .get(&pt.instrument_id)
            .map_or(&[][..], Vec::as_slice);
        let Ok(target) = build_automation_target(&pt.param, pt.instrument_id, valid) else {
            skipped += 1;
            continue;
        };
        let tick = PatternTick(beats_to_ticks(pt.beat));
        let curve = curve_from_kind(pt.curve, pt.curve_strength);
        let lane = pattern.get_or_create_automation(target);
        lane.add_point(
            AutomationPoint::new(tick, NormalizedValue::new(pt.value)).with_curve(curve),
        );
    }
    skipped
}

/// Compute overlapping module pairs from their positions and sizes.
fn compute_overlaps(modules: &[crate::mcp_shared::ModuleLayout]) -> Vec<UiOverlap> {
    let mut overlaps = Vec::new();
    for i in 0..modules.len() {
        for j in (i + 1)..modules.len() {
            let a = &modules[i];
            let b = &modules[j];
            // Rectangle intersection
            let ax1 = a.position.0;
            let ay1 = a.position.1;
            let ax2 = ax1 + a.size.0;
            let ay2 = ay1 + a.size.1;
            let bx1 = b.position.0;
            let by1 = b.position.1;
            let bx2 = bx1 + b.size.0;
            let by2 = by1 + b.size.1;

            let overlap_x = (ax2.min(bx2) - ax1.max(bx1)).max(0.0);
            let overlap_y = (ay2.min(by2) - ay1.max(by1)).max(0.0);
            let area = overlap_x * overlap_y;

            if area > 0.0 {
                overlaps.push(UiOverlap {
                    module_a: a.id.clone(),
                    module_b: b.id.clone(),
                    overlap_area: area,
                });
            }
        }
    }
    overlaps
}

/// Render a note offline and compute analysis metrics from the f32 buffer.
///
/// Shared between the MCP bridge `analyze_note` method and any other caller
/// that wants quantitative metrics rather than an opaque WAV blob.
///
/// `expected_note` (when `Some`) anchors `expected_fundamental_hz` to that
/// MIDI note and narrows the fundamental search to ±tritone around it. This
/// keeps the pitch metric meaningful for patches where the loudest spectral
/// peak is not the fundamental (sub-bass with a dominant sub-osc, wave-folded
/// patches with redistributed harmonics, etc.).
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
#[path = "mcp_bridge/tests/mod_matrix_routing.rs"]
mod mod_matrix_routing_tests;

#[cfg(test)]
#[allow(clippy::float_cmp)]
#[path = "mcp_bridge/tests/pre_master_peak.rs"]
mod pre_master_peak_tests;

#[cfg(test)]
#[path = "mcp_bridge/tests/automation_target.rs"]
mod automation_target_tests;

#[cfg(test)]
#[path = "mcp_bridge/tests/helpers.rs"]
mod mcp_helper_tests;
