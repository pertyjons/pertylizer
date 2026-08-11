use super::*;

impl synth_mcp::bridge::InstrumentBridge for AppSynthBridge {
    fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, McpBridgeError> {
        let snapshots = self.session.list_instruments();
        Ok(snapshots.iter().map(Self::snapshot_to_info).collect())
    }

    fn list_orphaned_tracks(
        &self,
    ) -> Result<Vec<synth_mcp::types::OrphanedTrackLint>, McpBridgeError> {
        let live: std::collections::HashSet<_> = self
            .session
            .list_instruments()
            .iter()
            .map(|instrument| instrument.id)
            .collect();
        let song = self.shared.song.read();
        Ok(song
            .tracks()
            .filter(|track| !live.contains(&track.instrument))
            .map(|track| synth_mcp::types::OrphanedTrackLint {
                track_id: track.id,
                track_name: track.name.clone(),
                missing_instrument_id: track.instrument,
                message: format!(
                    "track references missing instrument {}; track mixer, automation, and Mod Grid control will not apply",
                    track.instrument
                ),
            })
            .collect())
    }

    fn list_hidden_events(
        &self,
    ) -> Result<Vec<synth_mcp::types::HiddenEventsLint>, McpBridgeError> {
        let song = self.shared.song.read();
        let mut lints = Vec::new();
        for pattern in song.patterns() {
            if pattern.length.0 == 0 {
                continue;
            }
            let s = pattern.hidden_event_summary();
            if !s.has_hidden() {
                continue;
            }
            // `ticks_to_beats` (quarter-note beats, 960 PPQN) — same conversion
            // every other MCP beat field uses.
            let length_beats = ticks_to_beats(pattern.length.0);
            lints.push(synth_mcp::types::HiddenEventsLint {
                pattern_id: pattern.id,
                pattern_name: pattern.name.clone(),
                pattern_length_beats: length_beats,
                last_hidden_note_start_beats: ticks_to_beats(s.last_hidden_note_start.0),
                last_note_end_beats: ticks_to_beats(s.last_note_end.0),
                last_hidden_automation_beats: ticks_to_beats(s.last_hidden_automation.0),
                hidden_note_count: s.hidden_note_count,
                hidden_automation_count: s.hidden_automation_count,
                message: format!(
                    "pattern {} is {length_beats:.3} beats long but has {} note onset(s) and {} \
                     automation point(s) at/after that boundary; they remain in the file but are \
                     never played — raise the pattern length to reveal them",
                    pattern.id, s.hidden_note_count, s.hidden_automation_count
                ),
            });
        }
        Ok(lints)
    }

    fn get_instrument_profiles(
        &self,
    ) -> Result<Vec<synth_mcp::types::InstrumentProfileResult>, McpBridgeError> {
        let song = self.shared.song.read();
        let profiles = crate::analysis::infer_all_profiles(&song, self.session.state());
        Ok(profiles.into_iter().map(profile_to_result).collect())
    }

    fn get_instrument_info(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<InstrumentInfo, McpBridgeError> {
        let snapshots = self.session.list_instruments();
        snapshots
            .iter()
            .find(|s| s.id == instrument_id)
            .map(Self::snapshot_to_info)
            .ok_or(McpBridgeError::InstrumentNotFound(instrument_id.as_u64()))
    }

    fn list_modules(&self, instrument_id: InstrumentId) -> Result<Vec<ModuleInfo>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let state = self.session.state();
        let inst_id = instrument_id;
        let modules = state.shared_graph.get_modules_for_instrument(inst_id);
        let connections = state.shared_graph.get_connections_for_instrument(inst_id);

        let module_index = InstrumentModuleIndex::from_snapshots(&modules);
        let matrix_sources = collect_mod_matrix_sources(&modules, &module_index);
        let matrix_destinations = collect_mod_matrix_destinations(&modules, &module_index);

        Ok(modules
            .into_iter()
            .map(|m| {
                let id_str = m.id.to_string();
                let input_ports: Vec<String> = m
                    .input_connection_counts
                    .keys()
                    .map(|k| k.to_string())
                    .collect();
                let output_ports: Vec<String> = m
                    .output_connection_counts
                    .keys()
                    .map(|k| k.to_string())
                    .collect();

                // Build input/output ports from connections if snapshot doesn't have them
                let mut inputs = input_ports;
                let mut outputs = output_ports;

                for conn in &connections {
                    let to_port_str = conn.to_port.to_string();
                    if conn.to_module.to_string() == id_str && !inputs.contains(&to_port_str) {
                        inputs.push(to_port_str);
                    }
                    let from_port_str = conn.from_port.to_string();
                    if conn.from_module.to_string() == id_str && !outputs.contains(&from_port_str) {
                        outputs.push(from_port_str);
                    }
                }

                // Get descriptor for parameter ranges and units
                let descriptor = self.session.module_descriptor(inst_id, m.id);

                // Matrix routings aren't cables — surface a virtual port so
                // matrix-only modules don't read as dead, and attach the
                // live slot table for matrix modules themselves.
                let is_mod_matrix = m.module_type == ModuleType::ModMatrix;
                let mod_matrix_routings = if is_mod_matrix {
                    Some(collect_mod_matrix_routings(&m.parameters, &m.scripts))
                } else {
                    None
                };
                if is_mod_matrix && outputs.is_empty() {
                    outputs.push(MATRIX_VIRTUAL_PORT.to_string());
                }

                // Script (`scr`) modules: surface every declared CV port — 4 inputs
                // (`in1`..`in4`) and 4 outputs (`out1`..`out4`) — even when
                // unconnected, and read back the installed program — symmetric with
                // `set_mod_matrix_script`, so a client can inspect/diff a Script
                // module it just configured.
                let is_script = m.module_type == ModuleType::Script;
                let scripts = if is_script {
                    if let Some(desc) = descriptor.as_ref() {
                        for p in &desc.ports {
                            let name = p.name.as_str().to_string();
                            match p.direction {
                                PortDirection::Output if !outputs.contains(&name) => {
                                    outputs.push(name);
                                }
                                PortDirection::Input if !inputs.contains(&name) => {
                                    inputs.push(name);
                                }
                                _ => {}
                            }
                        }
                    }
                    let mut slots: Vec<synth_mcp::types::ScriptSlotInfo> = m
                        .scripts
                        .iter()
                        .filter_map(|(slot_key, source)| {
                            // Keys are 1-based; the canonical generator is 0-based
                            // and returns None past the module's real port count,
                            // so a stray out-of-range slot is skipped rather than
                            // reported as a phantom port.
                            let slot = slot_key.parse::<u8>().ok()?;
                            let output_port = synth_modules::script_module::output_port_name(
                                usize::from(slot.checked_sub(1)?),
                            )?;
                            Some(synth_mcp::types::ScriptSlotInfo {
                                slot,
                                output_port,
                                source: source.clone(),
                            })
                        })
                        .collect();
                    slots.sort_by_key(|s| s.slot);
                    Some(slots)
                } else {
                    None
                };
                if matrix_sources.contains(&m.id)
                    && !outputs.iter().any(|p| p == MATRIX_VIRTUAL_PORT)
                {
                    outputs.push(MATRIX_VIRTUAL_PORT.to_string());
                }
                if matrix_destinations.contains(&m.id)
                    && !inputs.iter().any(|p| p == MATRIX_VIRTUAL_PORT)
                {
                    inputs.push(MATRIX_VIRTUAL_PORT.to_string());
                }

                ModuleInfo {
                    id: id_str,
                    module_type: m.module_type.name().to_string(),
                    name: m.name.clone(),
                    bypassed: m.bypass_state == synth_core::BypassState::Bypassed,
                    description: m.description.clone(),
                    parameters: m
                        .parameters
                        .iter()
                        .map(|p| {
                            // Match by typed parameter identity first. Indexed parameters such
                            // as MSEG segments deliberately share a display name, so a
                            // name-only lookup loses their index and cannot attach the stable
                            // `segN_*` type_id. Retain the normalized-name fallback for legacy
                            // modules whose runtime and descriptor parameter variants differ.
                            let name = p.name().to_string();
                            let pd = descriptor.as_ref().and_then(|desc| {
                                let needle = normalize_param_name(&name);
                                desc.parameters
                                    .iter()
                                    .find(|pd| pd.id.same_kind(p))
                                    .or_else(|| {
                                        desc.parameters
                                            .iter()
                                            .find(|pd| normalize_param_name(&pd.name) == needle)
                                    })
                            });
                            ParameterInfo {
                                name: name.clone(),
                                value: p.as_f32(),
                                display: format_param_display(p, pd.map(|pd| pd.unit)),
                                min: pd.map(|pd| pd.range.min),
                                max: pd.map(|pd| pd.range.max),
                                default: pd.map(|pd| pd.range.default),
                                choices: pd.and_then(|pd| {
                                    pd.choices
                                        .as_ref()
                                        .map(|c| c.iter().map(|ch| ch.name.clone()).collect())
                                }),
                                type_id: pd.map(|pd| pd.type_id.clone()),
                                is_automatable: pd.map(|pd| pd.is_automatable()),
                                modulatable: pd.map(|pd| pd.modulatable),
                                response_curve: pd.map(|pd| format!("{:?}", pd.response_curve)),
                                value_kind: pd.map(|pd| pd.kind),
                            }
                        })
                        .collect(),
                    input_ports: inputs,
                    output_ports: outputs,
                    mod_matrix_routings,
                    scripts,
                }
            })
            .collect())
    }

    fn get_module_info(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
    ) -> Result<ModuleInfo, McpBridgeError> {
        let modules = self.list_modules(instrument_id)?;
        modules
            .into_iter()
            .find(|m| m.id == module_id)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_id.to_string()))
    }

    fn get_connections(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<Vec<ConnectionInfo>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let connections = self
            .session
            .state()
            .shared_graph
            .get_connections_for_instrument(instrument_id);
        Ok(connections
            .into_iter()
            .map(|c| ConnectionInfo {
                from_module: c.from_module.to_string(),
                from_port: c.from_port.to_string(),
                to_module: c.to_module.to_string(),
                to_port: c.to_port.to_string(),
            })
            .collect())
    }

    fn get_mod_matrix_routings(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<Vec<MatrixRoutingInfo>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let modules = self
            .session
            .state()
            .shared_graph
            .get_modules_for_instrument(instrument_id);

        Ok(modules
            .iter()
            .filter(|m| m.module_type == ModuleType::ModMatrix)
            .flat_map(|m| collect_mod_matrix_routings(&m.parameters, &m.scripts))
            .collect())
    }

    fn set_mod_matrix_script(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        slot: u8,
        source: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // The tool's `slot` is 1-based (matching the routings report); the engine
        // is 0-based. Range-check against the real slot count for this module
        // type — a Script (`scr`) module is now a single program (slot 1 only),
        // so a too-large slot is rejected up front instead of being a silent no-op.
        let max = match mid.module_type {
            ModuleType::Script | ModuleType::AudioScript => 1,
            _ => synth_core::MAX_MOD_MATRIX_SLOTS as u8,
        };
        if !(1..=max).contains(&slot) {
            return Err(McpBridgeError::Other(format!(
                "slot {slot} out of range (expected 1..={max})"
            )));
        }
        let slot0 = slot - 1;

        let to_bridge_err = |e: crate::session::SessionError| match e {
            crate::session::SessionError::ScriptCompile(msg) => {
                McpBridgeError::Other(format!("script compile error: {msg}"))
            }
            _ => McpBridgeError::CommandSendFailed {
                command: "set_mod_matrix_script",
            },
        };

        // An empty (or whitespace-only) source clears the slot — YAMS can't
        // compile an empty program, so this is a distinct command, not a compile.
        if source.trim().is_empty() {
            self.session
                .clear_mod_script(inst_id, mid, slot0)
                .map_err(to_bridge_err)
        } else {
            self.session
                .set_mod_script(inst_id, mid, slot0, source)
                .map_err(to_bridge_err)
        }
    }

    fn get_parameter(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        param_name: &str,
    ) -> Result<ParameterInfo, McpBridgeError> {
        let module = self.get_module_info(instrument_id, module_id)?;
        let available: Vec<String> = module.parameters.iter().map(|p| p.name.clone()).collect();
        module
            .parameters
            .into_iter()
            .find(|p| p.name == param_name)
            .ok_or_else(|| {
                McpBridgeError::ParameterNotFound(format!(
                    "'{param_name}' not found, available: {available:?}"
                ))
            })
    }

    fn get_engine_status(&self) -> Result<EngineStatus, McpBridgeError> {
        let state = self.session.state();
        let (peak_left, peak_right) = state.meters.get_peak();
        let (rms_left, rms_right) = state.meters.get_rms();

        Ok(EngineStatus {
            cpu_usage: state.cpu_usage.load(),
            voice_count: state.voice_count.load(),
            sample_rate: state.sample_rate.load(),
            peak_left: peak_left.as_f32(),
            peak_right: peak_right.as_f32(),
            rms_left: rms_left.as_f32(),
            rms_right: rms_right.as_f32(),
            master_volume: state.master_volume.load(),
            tempo: state.transport.get_tempo().as_f32(),
            is_playing: state.transport.is_playing(),
            instrument_count: state.instrument_snapshots.read().len(),
        })
    }

    fn get_version(&self) -> Result<VersionInfo, McpBridgeError> {
        // Empty strings mean "built without git state" (see build.rs).
        let non_empty = |s: &'static str| (!s.is_empty()).then(|| s.to_string());
        Ok(VersionInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_timestamp: env!("BUILD_TIMESTAMP").to_string(),
            commit_hash: non_empty(env!("GIT_COMMIT_HASH")),
            branch: non_empty(env!("GIT_BRANCH")),
            dirty: match env!("GIT_DIRTY") {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            },
        })
    }

    fn get_graph_diagnostics(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<Vec<GraphDiagnostic>, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mut diagnostics = Vec::new();
        let state = self.session.state();
        let inst_id = instrument_id;
        let modules = state.shared_graph.get_modules_for_instrument(inst_id);
        let connections = state.shared_graph.get_connections_for_instrument(inst_id);

        if modules.is_empty() {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Info,
                module_id: None,
                message: "Voice graph is empty — no modules added yet".to_string(),
            });
            return Ok(diagnostics);
        }

        // Check for essential module types
        let has_sound_source = modules.iter().any(|m| is_sound_source(m.id.module_type));
        let has_output = modules
            .iter()
            .any(|m| m.id.module_type == synth_core::ModuleType::StereoOutput);
        let has_envelope = modules
            .iter()
            .any(|m| m.id.module_type == synth_core::ModuleType::Envelope);

        if !has_sound_source {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Error,
                module_id: None,
                message: "No sound source (oscillator / sampler / voice generator) — instrument will be silent"
                    .to_string(),
            });
        }
        if !has_output {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Error,
                module_id: None,
                message: "No stereo_output module — audio cannot reach the mixer".to_string(),
            });
        }
        if !has_envelope && has_sound_source {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Warning,
                module_id: None,
                message: "No envelope — notes will have no attack/release shaping".to_string(),
            });
        }

        // Collect module IDs referenced as sources by any Mod Matrix slot.
        // The Mod Matrix routes via parameter slots (not cables), so these
        // modules are "in use" even with no cable connections.
        let module_index = InstrumentModuleIndex::from_snapshots(&modules);
        let mod_matrix_sources = collect_mod_matrix_sources(&modules, &module_index);

        // ModuleStateSnapshot::has_inputs/has_outputs read from
        // input_connection_counts/output_connection_counts, which currently
        // have no writers (see shared_state.rs) — they always return false,
        // so every module would otherwise be flagged disconnected.
        let modules_with_input: HashSet<ModuleId> =
            connections.iter().map(|c| c.to_module).collect();
        let modules_with_output: HashSet<ModuleId> =
            connections.iter().map(|c| c.from_module).collect();

        // Check for disconnected modules
        for module in &modules {
            let id_str = module.id.to_string();

            // Effect-chain modules are auto-wired in series outside the voice
            // graph — they legitimately have no voice-graph cables.
            if module.id.module_type.is_effect() {
                continue;
            }

            // ModMatrix and KineticModulator route via parameters, not cables.
            if matches!(
                module.id.module_type,
                synth_core::ModuleType::ModMatrix | synth_core::ModuleType::KineticModulator
            ) {
                continue;
            }

            // Modulator referenced by a Mod Matrix slot is in use.
            if mod_matrix_sources.contains(&module.id) {
                continue;
            }

            let has_input = modules_with_input.contains(&module.id);
            let has_output_conn = modules_with_output.contains(&module.id);

            if !has_input && !has_output_conn {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    module_id: Some(id_str.clone()),
                    message: format!("Module {} ({}) has no connections", id_str, module.name),
                });
            } else if !has_output_conn
                && module.id.module_type != synth_core::ModuleType::StereoOutput
                && module.id.module_type != synth_core::ModuleType::Amplifier
            {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    module_id: Some(id_str.clone()),
                    message: format!(
                        "Module {} ({}) has inputs but no outputs — signal dead-end",
                        id_str, module.name
                    ),
                });
            }
        }

        if diagnostics.is_empty() {
            diagnostics.push(GraphDiagnostic {
                severity: DiagnosticSeverity::Info,
                module_id: None,
                message: format!(
                    "Graph looks healthy: {} modules, {} connections",
                    modules.len(),
                    connections.len()
                ),
            });
        }

        // Surface per-instance descriptions so an AI agent sees the patch's
        // intent alongside the structural diagnostics above. Appended after the
        // health check so the "looks healthy" summary still reflects problems
        // only, not the presence of annotations.
        for module in &modules {
            if !module.description.is_empty() {
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    module_id: Some(module.id.to_string()),
                    message: format!(
                        "Module {} ({}) intent: {}",
                        module.id, module.name, module.description
                    ),
                });
            }
        }

        Ok(diagnostics)
    }

    fn get_project_schema(&self) -> Result<ProjectSchemaInfo, McpBridgeError> {
        // Pass the embedded artifact through verbatim as a `RawValue` — this
        // validates it's well-formed JSON once but skips building (and the caller
        // skips re-serializing) a full 251 KB `Value` tree on every call.
        let schema = serde_json::value::RawValue::from_string(PROJECT_SCHEMA_JSON.to_string())
            .map_err(|e| {
                McpBridgeError::Other(format!("embedded project schema is invalid JSON: {e}"))
            })?;

        Ok(ProjectSchemaInfo {
            schema_file: "project.schema.json".to_string(),
            schema_format_version: crate::project::ProjectFile::FORMAT_VERSION.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            schema,
        })
    }

    // === Instrument lifecycle ===

    fn create_instrument(&self, name: &str) -> Result<InstrumentInfo, McpBridgeError> {
        let id =
            self.session
                .add_instrument(name)
                .map_err(|_| McpBridgeError::CommandSendFailed {
                    command: "create_instrument",
                })?;

        // Return basic info — the engine will update the snapshots asynchronously.
        // Allocator fields mirror `AllocatorConfig::default()` (derived, not
        // hardcoded) since the real values only arrive with the async snapshot.
        let alloc = synth_engine::voice_allocator::AllocatorConfig::default();
        Ok(InstrumentInfo {
            id,
            name: name.to_string(),
            description: String::new(),
            patch_description: None,
            color: String::new(),
            patch_color: String::new(),
            sidechain_source_id: None,
            midi_channel: Some(MidiChannel::CH1),
            volume: Gain::UNITY,
            pan: BipolarValue::CENTER,
            enabled: true,
            muted: false,
            solo: false,
            module_count: 0,
            effect_count: 0,
            category: "uncategorized".to_owned(),
            allocation_mode: alloc.mode.to_string(),
            stealing_strategy: alloc.stealing.to_string(),
            unison_detune: alloc.unison_detune.as_f32(),
            unison_spread: alloc.unison_spread.as_f32(),
            max_voices: u32::from(alloc.max_voices.as_u8()),
        })
    }

    fn delete_instrument(&self, instrument_id: InstrumentId) -> Result<(), McpBridgeError> {
        if instrument_id == InstrumentId::FIRST {
            return Err(McpBridgeError::Other(
                "cannot delete the default instrument".to_string(),
            ));
        }
        self.validate_instrument(instrument_id)?;
        self.session.remove_instrument(instrument_id).map_err(|_| {
            McpBridgeError::CommandSendFailed {
                command: "delete_instrument",
            }
        })
    }

    fn rename_instrument(
        &self,
        instrument_id: InstrumentId,
        name: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .rename_instrument(instrument_id, name)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_description(
        &self,
        instrument_id: InstrumentId,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_description(instrument_id, description)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_color(
        &self,
        instrument_id: InstrumentId,
        color: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let normalized = normalize_color_arg(color)?;
        self.session
            .set_instrument_color(instrument_id, normalized.as_deref())
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_patch_color(
        &self,
        instrument_id: InstrumentId,
        color: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let normalized = normalize_color_arg(color)?;
        self.session
            .set_patch_color(instrument_id, normalized.as_deref())
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_patch_description(
        &self,
        instrument_id: InstrumentId,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let value = if description.is_empty() {
            None
        } else {
            Some(description)
        };
        self.session
            .set_patch_description(instrument_id, value)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_module_description(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let len = description.chars().count();
        if len > MAX_MODULE_DESCRIPTION_LEN {
            return Err(McpBridgeError::DescriptionTooLong {
                len,
                max: MAX_MODULE_DESCRIPTION_LEN,
            });
        }
        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        // Reject unknown modules up-front so a typo can't seed a phantom entry
        // in the engine's description map (which would resurface if a module
        // were later created with that id).
        if self.session.module_descriptor(inst_id, mid).is_none() {
            return Err(McpBridgeError::ModuleNotFound(module_id.to_string()));
        }
        let value = if description.is_empty() {
            None
        } else {
            Some(description)
        };
        self.session
            .set_module_description(inst_id, mid, value)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_sidechain_source(
        &self,
        instrument_id: InstrumentId,
        source: Option<InstrumentId>,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        if let Some(src) = source {
            if src == instrument_id {
                return Err(McpBridgeError::Other(
                    "sidechain source must differ from the target instrument".into(),
                ));
            }
            self.validate_instrument(src)?;
            // Walk the current chain from `src` to detect cycles.
            // The engine also rejects cycles defensively, but pre-checking
            // here lets MCP report a clear error to the caller.
            let snapshots = self.session.list_instruments();
            let chain_len = snapshots.len();
            let mut current = Some(src);
            for _ in 0..=chain_len {
                let Some(id) = current else {
                    break;
                };
                if id == instrument_id {
                    return Err(McpBridgeError::Other(format!(
                        "sidechain would form a cycle through instrument {}",
                        id.as_u64()
                    )));
                }
                current = snapshots
                    .iter()
                    .find(|s| s.id == id)
                    .and_then(|s| s.sidechain_source_id);
            }
        }
        self.session
            .set_sidechain_source(instrument_id, source)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_instrument_volume(
        &self,
        instrument_id: InstrumentId,
        volume: Gain,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_volume(instrument_id, volume)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_volume",
            })
    }

    fn set_instrument_pan(
        &self,
        instrument_id: InstrumentId,
        pan: BipolarValue,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_pan(instrument_id, pan)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_pan",
            })
    }

    fn set_instrument_mute(
        &self,
        instrument_id: InstrumentId,
        muted: bool,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_mute(instrument_id, muted)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_mute",
            })
    }

    fn set_instrument_solo(
        &self,
        instrument_id: InstrumentId,
        solo: bool,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_solo(instrument_id, solo)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_solo",
            })
    }

    fn set_instrument_midi_channel(
        &self,
        instrument_id: InstrumentId,
        channel: MidiChannel,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let midi_channel = MidiChannelSelection::from_one_indexed(channel.as_u8())
            .unwrap_or(MidiChannelSelection::CH1);
        self.session
            .set_instrument_midi_channel(instrument_id, midi_channel)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_midi_channel",
            })
    }

    fn set_instrument_enabled(
        &self,
        instrument_id: InstrumentId,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_enabled(instrument_id, enabled)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_enabled",
            })
    }

    fn set_instrument_category(
        &self,
        instrument_id: InstrumentId,
        category: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let cat: synth_engine::InstrumentCategory =
            category.parse().map_err(McpBridgeError::Other)?;
        self.session
            .set_instrument_category(instrument_id, cat)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_category",
            })
    }

    fn set_instrument_allocation_mode(
        &self,
        instrument_id: InstrumentId,
        mode: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let parsed: synth_engine::voice_allocator::AllocationMode =
            mode.parse().map_err(McpBridgeError::Other)?;
        self.session
            .set_instrument_allocation_mode(instrument_id, parsed)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_allocation_mode",
            })
    }

    fn set_instrument_stealing_strategy(
        &self,
        instrument_id: InstrumentId,
        strategy: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let parsed: synth_engine::voice_allocator::StealingStrategy =
            strategy.parse().map_err(McpBridgeError::Other)?;
        self.session
            .set_instrument_stealing_strategy(instrument_id, parsed)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_stealing_strategy",
            })
    }

    fn set_instrument_unison_detune(
        &self,
        instrument_id: InstrumentId,
        cents: f32,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_unison_detune(instrument_id, synth_core::Cents::new(cents))
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_unison_detune",
            })
    }

    fn set_instrument_unison_spread(
        &self,
        instrument_id: InstrumentId,
        spread: f32,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.session
            .set_instrument_unison_spread(instrument_id, synth_core::NormalizedValue::new(spread))
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_unison_spread",
            })
    }

    fn set_instrument_max_voices(
        &self,
        instrument_id: InstrumentId,
        max_voices: u32,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        // `VoiceCount` is a u8 domain (1..=128); the server tool already rejects
        // out-of-range values, and `VoiceCount::new` clamps as a backstop.
        let count = synth_core::VoiceCount::new(max_voices.min(u32::from(u8::MAX)) as u8);
        self.session
            .set_instrument_max_voices(instrument_id, count)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "set_instrument_max_voices",
            })
    }

    fn set_parameter(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // Fetch the descriptor once and resolve the parameter entry through the
        // shared `find_parameter`, so this path accepts exactly what
        // session.set_parameter does — `type_id` first (the stable identifier
        // clients and project files usually pass), then display name. Matching
        // only on name would silently skip validation for type_id-addressed
        // params (e.g. "pre_delay" vs the "Pre-Delay" display name). The resolved
        // entry is reused for both validation and the response below; the
        // descriptor is the single source of truth shared with schema generation
        // and MCP discovery.
        let descriptor = self.session.module_descriptor(inst_id, mid);
        let param_desc = descriptor
            .as_ref()
            .and_then(|desc| desc.find_parameter(param_name));

        // The mod-matrix source and destination accept a free-form address string
        // ("lfo-1.out" / "flt-1.cutoff", a macro id, a legacy id, or "none") so
        // MCP can address any module — the numeric choice path only reaches the
        // legacy roles. Hand the string to the session, which parses it
        // (dual-format) in `ParamValue::to_param`.
        let addr_param = match param_desc.map(|pd| pd.id) {
            Some(Param::ModMatrix(ModMatrixParam::SlotSource(..))) => Some(true),
            Some(Param::ModMatrix(ModMatrixParam::SlotDestination(..))) => Some(false),
            _ => None,
        };
        let (value, pv) = if let (Some(is_source), BridgeParamValue::Choice(s)) =
            (addr_param, &value)
        {
            let (legacy_index, display) = if s.eq_ignore_ascii_case("none") {
                (0.0, "none".to_string())
            } else if is_source {
                let a =
                    synth_core::SrcAddr::parse(s).ok_or_else(|| McpBridgeError::InvalidChoice {
                        name: param_name.to_string(),
                        value: s.clone(),
                        detail: "expected a source address like \"lfo-1.out\" or a macro \
                                 id (or a legacy id, or \"none\")"
                            .to_string(),
                    })?;
                (a.legacy_index() as f32, a.to_address_string())
            } else {
                let a = synth_core::DestAddr::parse(s).ok_or_else(|| {
                    McpBridgeError::InvalidChoice {
                        name: param_name.to_string(),
                        value: s.clone(),
                        detail: "expected a destination address like \"flt-1.cutoff\" \
                                 (or a legacy id, or \"none\")"
                            .to_string(),
                    }
                })?;
                (a.legacy_index() as f32, a.to_address_string())
            };
            (legacy_index, crate::patch::ParamValue::Choice(display))
        } else {
            // Resolve the supplied value (number / bool / string choice) to the
            // parameter's native f32, rejecting unknown choices at the boundary
            // instead of silently mapping them to index 0.
            let value = resolve_param_value(&value, param_desc, param_name)?;
            // Kind-aware validation BEFORE applying — rounds integers, accepts
            // bools, rejects out-of-range. Use the returned value so a rounded
            // integer (e.g. `4.3` → `4`) is what actually gets applied.
            let value = if let Some(pd) = param_desc {
                pd.validate_f32(value)
                    .map_err(|source| McpBridgeError::InvalidParameterValue {
                        name: pd.name.clone(),
                        source,
                    })?
            } else {
                value
            };
            (value, crate::patch::ParamValue::Float(value))
        };

        // Use session.set_parameter for correct effect/module routing
        self.session
            .set_parameter(inst_id, mid, param_name, &pv)
            .map_err(|e| match e {
                crate::session::SessionError::ModuleNotFound(s) => {
                    McpBridgeError::ModuleNotFound(s)
                }
                crate::session::SessionError::ParameterNotFound(s) => {
                    McpBridgeError::ParameterNotFound(s)
                }
                _ => McpBridgeError::CommandSendFailed {
                    command: "set_parameter",
                },
            })?;

        // Read back the actual value directly from the descriptor (avoids listing all modules)
        if let Some(pd) = param_desc {
            return Ok(ParameterInfo {
                name: pd.name.clone(),
                value,
                display: pd.unit.format(value),
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
            });
        }
        Ok(ParameterInfo {
            name: param_name.to_string(),
            value,
            display: format!("{value}"),
            min: None,
            max: None,
            default: None,
            choices: None,
            type_id: None,
            is_automatable: None,
            modulatable: None,
            response_curve: None,
            value_kind: None,
        })
    }

    fn note_on(
        &self,
        note: MidiNote,
        velocity: u8,
        channel: MidiChannel,
        instrument_id: Option<InstrumentId>,
    ) -> Result<(), McpBridgeError> {
        if let Some(id) = instrument_id {
            self.validate_instrument(id)?;
        }
        let midi_channel = MidiChannelSelection::from_one_indexed(channel.as_u8())
            .unwrap_or(MidiChannelSelection::CH1);
        if self.session.command_sender().send(EngineCommand::NoteOn {
            note,
            velocity: Velocity::from_midi(velocity),
            channel: midi_channel,
            instrument_id,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "note_on" })
        }
    }

    fn note_off(
        &self,
        note: MidiNote,
        channel: MidiChannel,
        instrument_id: Option<InstrumentId>,
    ) -> Result<(), McpBridgeError> {
        if let Some(id) = instrument_id {
            self.validate_instrument(id)?;
        }
        let midi_channel = MidiChannelSelection::from_one_indexed(channel.as_u8())
            .unwrap_or(MidiChannelSelection::CH1);
        if self.session.command_sender().send(EngineCommand::NoteOff {
            note,
            channel: midi_channel,
            instrument_id,
        }) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed {
                command: "note_off",
            })
        }
    }

    fn list_example_patches(&self) -> Result<Vec<ExamplePatchInfo>, McpBridgeError> {
        let categories = crate::patches::categorized_patches();
        let mut result = Vec::new();
        for (category, patches) in categories {
            for patch in patches {
                result.push(ExamplePatchInfo {
                    name: patch.name.clone(),
                    category: category.to_string(),
                    description: patch.description.clone().unwrap_or_default(),
                    tags: patch.tags.clone(),
                    module_count: patch.modules.len(),
                    connection_count: patch.connections.len(),
                });
            }
        }
        Ok(result)
    }

    fn get_example_patch(&self, name: &str) -> Result<PatchResourceData, McpBridgeError> {
        use crate::patch::ParamValue;

        let categories = crate::patches::categorized_patches();
        let name_lower = name.to_ascii_lowercase();

        for (category, patches) in &categories {
            for patch in patches {
                if patch.name.to_ascii_lowercase() == name_lower {
                    let modules = patch
                        .modules
                        .iter()
                        .map(|m| PatchModuleInfo {
                            id: m.id.clone(),
                            module_type: m.module_type.name().to_string(),
                            parameters: m
                                .parameters
                                .iter()
                                .map(|(k, v)| PatchParamInfo {
                                    name: k.clone(),
                                    value: match v {
                                        ParamValue::Float(f) => PatchParamValue::Float(*f),
                                        ParamValue::Int(i) => PatchParamValue::Int(*i),
                                        ParamValue::SampleId { sample_id } => {
                                            PatchParamValue::SampleId {
                                                sample_id: *sample_id,
                                            }
                                        }
                                        ParamValue::Bool(b) => PatchParamValue::Bool(*b),
                                        ParamValue::Choice(s) => PatchParamValue::Choice(s.clone()),
                                    },
                                    // The `PatchParamValue` variant already conveys
                                    // the type here; a per-param descriptor lookup
                                    // would rebuild a module instance, so left None.
                                    value_kind: None,
                                })
                                .collect(),
                        })
                        .collect();

                    let connections = patch
                        .connections
                        .iter()
                        .map(|c| UiConnectionInfo {
                            from_module: c.from.0.clone(),
                            from_port: c.from.1.clone(),
                            to_module: c.to.0.clone(),
                            to_port: c.to.1.clone(),
                        })
                        .collect();

                    return Ok(PatchResourceData {
                        name: patch.name.clone(),
                        category: (*category).to_string(),
                        description: patch.description.clone().unwrap_or_default(),
                        tags: patch.tags.clone(),
                        modules,
                        connections,
                    });
                }
            }
        }

        Err(McpBridgeError::PatchNotFound(name.to_string()))
    }

    fn load_example_patch(&self, name: &str) -> Result<String, McpBridgeError> {
        let categories = crate::patches::categorized_patches();
        let name_lower = name.to_ascii_lowercase();

        for (_category, patches) in categories {
            for patch in patches {
                if patch.name.to_ascii_lowercase() == name_lower {
                    let patch_name = patch.name.clone();
                    // Apply to engine immediately so audio works regardless of
                    // whether the GUI tick happens to consume the queue.
                    let inst_id = self
                        .session
                        .add_instrument(&patch.name)
                        .map_err(|e| McpBridgeError::Other(e.to_string()))?;
                    let _ = self.session.apply_patch(inst_id, &patch);
                    // Queue for GUI so it can update its current-patch label and
                    // patch_editor cache on the next frame; reconcile_with_session
                    // covers the parameter-state sync.
                    if let Ok(mut pending) = self.shared.pending_patch.lock() {
                        *pending = Some((patch, patch_name.clone()));
                    }
                    self.bump_gui_revision();
                    return Ok(format!(
                        "OK: {patch_name} loaded as instrument {}",
                        inst_id.as_u64()
                    ));
                }
            }
        }

        Err(McpBridgeError::PatchNotFound(name.to_string()))
    }

    fn request_auto_layout(&self) -> Result<String, McpBridgeError> {
        self.shared
            .pending_auto_layout
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok("OK: auto-layout queued — applied on the next Rack-view frame".to_string())
    }

    fn get_ui_snapshot(&self, instrument_id: InstrumentId) -> Result<UiSnapshot, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let layout = self
            .shared
            .ui_layout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let modules: Vec<UiModuleInfo> = layout
            .modules
            .iter()
            .map(|m| UiModuleInfo {
                id: m.id.clone(),
                module_type: m.module_type.clone(),
                name: m.name.clone(),
                position: m.position,
                size: m.size,
                parameters: m.parameters.clone(),
            })
            .collect();

        let connections: Vec<UiConnectionInfo> = layout
            .connections
            .iter()
            .map(|c| UiConnectionInfo {
                from_module: c.from_module.clone(),
                from_port: c.from_port.clone(),
                to_module: c.to_module.clone(),
                to_port: c.to_port.clone(),
            })
            .collect();

        // Compute overlaps between module pairs
        let overlaps = compute_overlaps(&layout.modules);

        Ok(UiSnapshot {
            patch_name: layout.patch_name,
            modules,
            connections,
            window_size: layout.window_size,
            overlaps,
        })
    }

    fn list_module_types(&self) -> Result<Vec<ModuleTypeInfo>, McpBridgeError> {
        use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};

        let mut result = Vec::new();
        for &mt in ALL_MODULE_TYPES.iter() {
            if let Some(desc) = get_descriptor(mt) {
                result.push(build_module_type_info(mt, &desc));
            }
        }
        Ok(result)
    }

    fn list_module_types_brief(&self) -> Result<Vec<ModuleTypeBrief>, McpBridgeError> {
        use crate::module_factory::ALL_MODULE_TYPES;

        // No descriptor needed — type_key/name/category come straight off the
        // ModuleType, so this stays tiny even at ~70 module types.
        Ok(ALL_MODULE_TYPES
            .iter()
            .map(|&mt| ModuleTypeBrief {
                type_key: mt.prefix().to_string(),
                name: mt.name().to_string(),
                category: module_category(mt).to_string(),
                gui_only: mt.is_visualizer(),
            })
            .collect())
    }

    fn add_module(
        &self,
        instrument_id: InstrumentId,
        module_type: &str,
    ) -> Result<String, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mt = parse_module_type(module_type)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(module_type.to_string()))?;

        if mt.is_visualizer() {
            return Err(McpBridgeError::Other(format!(
                "{} is a GUI-only visualizer and cannot be added over MCP (it needs a \
                 VisualizationBuffer); such types are flagged gui_only:true by \
                 list_module_types so you can filter them out",
                mt.name()
            )));
        }

        let (module_id, _descriptor) = self
            .session
            .add_module(instrument_id, mt)
            .map_err(|e| McpBridgeError::InvalidModuleType(e.to_string()))?;

        Ok(module_id.to_string())
    }

    fn remove_module(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        self.session
            .remove_module(instrument_id, mid)
            .map_err(|e| McpBridgeError::ModuleNotFound(e.to_string()))
    }

    fn connect(
        &self,
        instrument_id: InstrumentId,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;

        let (from_pn, from_type) =
            self.validate_port(inst_id, from_module, from_port, PortDirection::Output)?;
        let (to_pn, to_type) =
            self.validate_port(inst_id, to_module, to_port, PortDirection::Input)?;
        if !from_type.can_drive(to_type) {
            return Err(McpBridgeError::InvalidConnection { from_type, to_type });
        }

        let from_id: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_id: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        self.session
            .connect(inst_id, from_id, from_pn, to_id, to_pn)
            .map_err(|_| McpBridgeError::CommandSendFailed { command: "connect" })
    }

    fn disconnect(
        &self,
        instrument_id: InstrumentId,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;

        let (from_pn, _) =
            self.validate_port(inst_id, from_module, from_port, PortDirection::Output)?;
        let (to_pn, _) = self.validate_port(inst_id, to_module, to_port, PortDirection::Input)?;

        let from_id: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_id: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        self.session
            .disconnect(instrument_id, from_id, from_pn, to_id, to_pn)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "disconnect",
            })
    }

    fn clear_graph(&self, instrument_id: InstrumentId) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        self.session
            .clear_graph(instrument_id)
            .map_err(|_| McpBridgeError::CommandSendFailed {
                command: "clear_graph",
            })
    }

    // === Sequencer: Song ===
}
