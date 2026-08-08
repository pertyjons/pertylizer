use super::*;
use synth_mcp::bridge::{AutomationBridge, SequencerBridge};

impl synth_mcp::bridge::InstrumentBuildBridge for AppSynthBridge {
    fn build_instrument(
        &self,
        spec: &BridgeInstrumentDef,
    ) -> Result<BuildInstrumentResult, McpBridgeError> {
        use crate::patch::ParamValue;

        // 1. Create or reuse instrument
        let mut errors = Vec::new();
        let inst_id = if let Some(id) = spec.instrument_id {
            let iid = id;
            if !self.session.instrument_exists(iid) {
                return Err(McpBridgeError::InstrumentNotFound(id.as_u64()));
            }
            // Clear existing graph before rebuilding
            self.session
                .clear_graph(iid)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?;
            // Rename
            if let Err(e) = self.session.rename_instrument(iid, &spec.name) {
                errors.push(format!("rename: {e}"));
            }
            iid
        } else {
            self.session
                .add_instrument(&spec.name)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?
        };

        // 2. Set optional instrument params
        if let Some(ch) = spec.midi_channel
            && let Err(e) = self.session.set_instrument_midi_channel(
                inst_id,
                MidiChannelSelection::from_one_indexed(ch.as_u8())
                    .unwrap_or(MidiChannelSelection::CH1),
            )
        {
            errors.push(format!("midi_channel: {e}"));
        }
        if let Some(vol) = spec.volume
            && let Err(e) = self.session.set_instrument_volume(inst_id, vol)
        {
            errors.push(format!("volume: {e}"));
        }
        if let Some(pan) = spec.pan
            && let Err(e) = self.session.set_instrument_pan(inst_id, pan)
        {
            errors.push(format!("pan: {e}"));
        }

        // 3. Add modules (keep descriptors for port validation)
        let mut module_ids: Vec<Option<ModuleId>> = Vec::with_capacity(spec.modules.len());
        let mut module_id_strings: Vec<String> = Vec::with_capacity(spec.modules.len());
        let mut module_descriptors: Vec<Option<synth_core::ModuleDescriptor>> =
            Vec::with_capacity(spec.modules.len());

        for module_def in &spec.modules {
            let mt = match parse_module_type(&module_def.module_type) {
                Some(mt) => mt,
                None => {
                    errors.push(format!("invalid module type: {}", module_def.module_type));
                    module_ids.push(None);
                    module_id_strings.push(String::new());
                    module_descriptors.push(None);
                    continue;
                }
            };

            match self.session.add_module(inst_id, mt) {
                Ok((mid, descriptor)) => {
                    let mid_str = mid.to_string();

                    // Set parameters
                    for (param_name, value) in &module_def.params {
                        let pv = match value {
                            BridgeParamValue::Number(n) => ParamValue::Float(*n as f32),
                            BridgeParamValue::Choice(s) => ParamValue::Choice(s.clone()),
                            BridgeParamValue::Bool(b) => ParamValue::Bool(*b),
                        };
                        if let Err(e) = self.session.set_parameter(inst_id, mid, param_name, &pv) {
                            errors.push(format!("{mid} param '{param_name}': {e}"));
                        }
                    }

                    module_ids.push(Some(mid));
                    module_id_strings.push(mid_str);
                    module_descriptors.push(Some(descriptor));
                }
                Err(e) => {
                    errors.push(format!("add module '{}': {e}", module_def.module_type));
                    module_ids.push(None);
                    module_id_strings.push(String::new());
                    module_descriptors.push(None);
                }
            }
        }

        // 4. Wire connections (with port validation)
        let mut connection_count = 0;
        for conn in &spec.connections {
            let from_mid = match module_ids.get(conn.from_index).and_then(|m| *m) {
                Some(id) => id,
                None => {
                    errors.push(format!(
                        "connection from_index {} has no module",
                        conn.from_index
                    ));
                    continue;
                }
            };
            let to_mid = match module_ids.get(conn.to_index).and_then(|m| *m) {
                Some(id) => id,
                None => {
                    errors.push(format!(
                        "connection to_index {} has no module",
                        conn.to_index
                    ));
                    continue;
                }
            };

            // Resolve + validate the source (output) port, honoring output/input aliases.
            let from_port = match module_descriptors
                .get(conn.from_index)
                .and_then(|d| d.as_ref())
            {
                Some(desc) => {
                    match resolve_port_name(&desc.ports, &conn.from_port, PortDirection::Output) {
                        Ok(name) => name,
                        Err(available) => {
                            errors.push(format!(
                                "{from_mid} has no output port '{}', available: {available}",
                                conn.from_port
                            ));
                            continue;
                        }
                    }
                }
                None => PortName::from(conn.from_port.as_str()),
            };

            // Resolve + validate the destination (input) port.
            let to_port = match module_descriptors
                .get(conn.to_index)
                .and_then(|d| d.as_ref())
            {
                Some(desc) => {
                    match resolve_port_name(&desc.ports, &conn.to_port, PortDirection::Input) {
                        Ok(name) => name,
                        Err(available) => {
                            errors.push(format!(
                                "{to_mid} has no input port '{}', available: {available}",
                                conn.to_port
                            ));
                            continue;
                        }
                    }
                }
                None => PortName::from(conn.to_port.as_str()),
            };

            match self
                .session
                .connect(inst_id, from_mid, from_port, to_mid, to_port)
            {
                Ok(()) => connection_count += 1,
                Err(e) => errors.push(format!(
                    "connect {}:{} → {}:{}: {e}",
                    from_mid, conn.from_port, to_mid, conn.to_port
                )),
            }
        }

        // Reject a "successful" NEW instrument that got none of its requested
        // wiring: every connection failing almost always means wrong port names,
        // and a zero-connection instrument is silently broken. Roll it back so a
        // fully-failed build leaves no orphan (and a retry can't accumulate
        // duplicates). This applies only to freshly-created instruments — an
        // existing one targeted by `instrument_id` (e.g. a rebuild) is left to
        // return its normal result + errors so callers like
        // `rebuild_instrument_preserve_automation` still get their lane report.
        if spec.instrument_id.is_none() && connection_count == 0 && !spec.connections.is_empty() {
            let _ = self.session.remove_instrument(inst_id);
            return Err(McpBridgeError::Other(format!(
                "all {} requested connection(s) failed — instrument rolled back, none created: {}",
                spec.connections.len(),
                errors.join("; ")
            )));
        }

        Ok(BuildInstrumentResult {
            instrument_id: inst_id,
            partial_success: !errors.is_empty(),
            module_ids: module_id_strings,
            connection_count,
            errors,
            hint: Some(format!(
                "Run get_graph_diagnostics(instrument_id: {}) to validate the instrument",
                inst_id.as_u64()
            )),
        })
    }

    fn build_instruments(
        &self,
        specs: &[BridgeInstrumentDef],
    ) -> Result<Vec<BuildInstrumentResult>, McpBridgeError> {
        let mut results: Vec<BuildInstrumentResult> = Vec::with_capacity(specs.len());
        for spec in specs {
            match self.build_instrument(spec) {
                Ok(result) => results.push(result),
                Err(e) => {
                    // Keep the batch atomic: on any failure, roll back the
                    // instruments this call already created so no orphans are
                    // left and a retry can't accumulate duplicates. Only the
                    // freshly-created ones (id assigned by this call) are
                    // removed; existing instruments targeted by `instrument_id`
                    // were owned by the caller before the call. The failing
                    // spec already rolled itself back inside build_instrument.
                    for (built, built_spec) in results.iter().zip(specs) {
                        if built_spec.instrument_id.is_none() {
                            let _ = self.session.remove_instrument(built.instrument_id);
                        }
                    }
                    return Err(e);
                }
            }
        }
        Ok(results)
    }

    fn rebuild_instrument_preserve_automation(
        &self,
        spec: &BridgeInstrumentDef,
        drop_orphaned: bool,
    ) -> Result<RebuildInstrumentResult, McpBridgeError> {
        let id = spec.instrument_id.ok_or_else(|| {
            McpBridgeError::Other(
                "rebuild_instrument_preserve_automation requires instrument_id".to_string(),
            )
        })?;
        let iid = id;
        if !self.session.instrument_exists(iid) {
            return Err(McpBridgeError::InstrumentNotFound(id.as_u64()));
        }
        // Reset per-(type) instance counters so the rebuilt graph numbers
        // modules deterministically from 1 in add order. Wherever the new
        // module set matches the old, modules keep their ids and their
        // automation lanes stay valid; only genuinely-removed modules orphan.
        self.session.reset_counters_for_instrument(iid);

        let built = self.build_instrument(spec)?;

        // Classify the instrument's automation lanes against the rebuilt graph.
        let valid_modules = self.instrument_module_ids(id);
        let patterns = self.list_patterns()?;
        let mut preserved_lanes = 0u32;
        let mut orphaned_lanes = Vec::new();
        for pat in &patterns {
            for lane in self.list_automation_lanes(pat.id)? {
                // Only module-scoped lanes can be orphaned by a graph rebuild.
                // Gate on the unambiguous `module:` prefix: the bare
                // `instrument_id` can't distinguish a `Track` lane that happens
                // to share this instrument's numeric id, and instrument-/track-/
                // global-level automation is unaffected by the rebuild anyway.
                if lane.instrument_id != Some(id) || !lane.target.starts_with("module:") {
                    continue;
                }
                let resolves = self
                    .build_live_automation_target(&lane.target, id, &valid_modules)
                    .is_ok();
                if resolves {
                    preserved_lanes += 1;
                } else {
                    orphaned_lanes.push(synth_mcp::types::OrphanedAutomationLane {
                        pattern_id: pat.id,
                        target: lane.target,
                    });
                }
            }
        }

        // Don't delete lanes when the rebuild itself failed partway: a module
        // that failed to add (bad type, send failure) is absent from the graph
        // and so looks orphaned, but it's recoverable by fixing the spec and
        // rebuilding — dropping here would destroy still-wanted automation.
        let mut errors = built.errors;
        let dropped = drop_orphaned && errors.is_empty();
        if drop_orphaned && !errors.is_empty() {
            errors.push(
                "orphaned lanes were NOT dropped because the rebuild reported errors — fix the \
                 spec and rebuild so a transiently-missing module isn't mistaken for a removed one"
                    .to_string(),
            );
        }

        // Drop the orphaned lanes. Their targets no longer resolve, so
        // `clear_automation_lane` can't address them (it validates + would
        // recreate an empty lane) — retain them out by rendered target instead.
        if dropped && !orphaned_lanes.is_empty() {
            let mut song = self.shared.song.write();
            for orphan in &orphaned_lanes {
                if let Some(pattern) = song.pattern_mut(orphan.pattern_id) {
                    pattern.automation.retain(|lane| {
                        let (target, lane_inst, _scope) = automation_target_info(&lane.target);
                        !(lane_inst == Some(id) && target == orphan.target)
                    });
                }
            }
        }

        Ok(RebuildInstrumentResult {
            instrument_id: built.instrument_id,
            module_ids: built.module_ids,
            connection_count: built.connection_count,
            preserved_lanes,
            orphaned_lanes,
            dropped_orphaned: dropped,
            errors,
        })
    }

    fn apply_example_patch(
        &self,
        instrument_id: Option<InstrumentId>,
        patch_name: &str,
    ) -> Result<ApplyExamplePatchResult, McpBridgeError> {
        // Find the patch
        let categories = crate::patches::categorized_patches();
        let name_lower = patch_name.to_ascii_lowercase();

        let patch = categories
            .iter()
            .flat_map(|(_cat, patches)| patches)
            .find(|p| p.name.to_ascii_lowercase() == name_lower)
            .ok_or_else(|| McpBridgeError::PatchNotFound(patch_name.to_string()))?
            .clone();

        // Create or reuse instrument
        let inst_id = if let Some(id) = instrument_id {
            let iid = id;
            if !self.session.instrument_exists(iid) {
                return Err(McpBridgeError::InstrumentNotFound(id.as_u64()));
            }
            iid
        } else {
            self.session
                .add_instrument(&patch.name)
                .map_err(|e| McpBridgeError::Other(e.to_string()))?
        };

        let result = self.session.apply_patch(inst_id, &patch);

        Ok(ApplyExamplePatchResult {
            instrument_id: inst_id,
            patch_name: patch.name,
            module_count: result.module_count,
            connection_count: result.connection_count,
            errors: result.error_lines(),
        })
    }
}
