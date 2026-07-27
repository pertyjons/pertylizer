//! discovery MCP tool handlers.

use super::super::*;

#[tool_router(router = discovery_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "List all instruments with ID, name, category, volume, pan, mute/solo state, and module/effect counts."
    )]
    pub(crate) async fn list_instruments(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_instruments() {
            Ok(instruments) => to_json(&instruments),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Auto-infer per-instrument profiles (role, envelope shape, pitch role, \
                       register, texture) for every instrument that at least one track routes to. \
                       Role values: drums, bass, lead, pad, pluck, keys, fx, unknown — each with a \
                       confidence in [0.0, 1.0] and a signal trail that explains the classification. \
                       Same inference path that `analyze_harmony`'s `exclude_drums = true` default \
                       uses; expose it directly to debug or override the classification. Manual \
                       `set_instrument_category` always wins (reports as `manual-override`)."
    )]
    pub(crate) async fn get_instrument_profiles(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_instrument_profiles() {
            Ok(profiles) => to_json(&profiles),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed information about a specific instrument including module count and effects"
    )]
    pub(crate) async fn get_instrument_info(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> String {
        match self.bridge.get_instrument_info(params.0.instrument_id) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all modules in an instrument's voice graph with their types and names"
    )]
    pub(crate) async fn list_modules(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.list_modules(params.0.instrument_id) {
            Ok(modules) => to_json(&modules),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed info for a specific module including all parameters and port connections"
    )]
    pub(crate) async fn get_module_info(&self, params: Parameters<ModuleParam>) -> String {
        match self
            .bridge
            .get_module_info(params.0.instrument_id, &params.0.module_id)
        {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get all connections (cables) between modules in the voice graph. Returns from_module:from_port → to_module:to_port pairs."
    )]
    pub(crate) async fn get_connections(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_connections(params.0.instrument_id) {
            Ok(conns) => to_json(&conns),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get all active Mod Matrix routings across every Mod Matrix module in the instrument. Slot rows include semantic source IDs (e.g. 'lfo-1', 'env-2', or 'velocity'/'mod_wheel' for non-module sources) and dotted destination IDs (e.g. 'flt-1.cutoff'), plus amount in -1..1 and enabled flag. A slot with a YAMS control script (Step 2) also reports its `script` source text — then the offset is the script's output, not amount × source. Inactive slots (None → None, no script) are filtered out."
    )]
    pub(crate) async fn get_mod_matrix_routings(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> String {
        match self.bridge.get_mod_matrix_routings(params.0.instrument_id) {
            Ok(routings) => to_json(&routings),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the current value of a specific module parameter. Returns name, raw value, formatted display string (e.g. '440 Hz'), and min/max/default range."
    )]
    pub(crate) async fn get_parameter(&self, params: Parameters<GetParameterParam>) -> String {
        match self.bridge.get_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
        ) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get engine status: CPU usage (0.0-1.0), active voice count, peak/RMS meters (dB), sample rate, tempo, and whether sequencer is playing."
    )]
    pub(crate) async fn get_engine_status(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_engine_status() {
            Ok(status) => to_json(&status),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get build/version info for the running application: version, build \
                       timestamp (ISO 8601 / RFC 3339 UTC, e.g. 2026-07-03T14:30:00Z), git \
                       commit hash, branch, and whether the working tree had uncommitted \
                       changes at build time. Git fields are null when the binary was built \
                       outside a git checkout."
    )]
    pub(crate) async fn get_version(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_version() {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Run diagnostics on the module graph to find issues like disconnected modules or missing connections"
    )]
    pub(crate) async fn get_graph_diagnostics(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> String {
        match self.bridge.get_graph_diagnostics(params.0.instrument_id) {
            Ok(diags) => to_json(&diags),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Return the authoritative on-disk JSON Schema for `.ptz` project files \
                       plus the build version that generated it. Use this to validate or diff project \
                       files against the exact committed schema — it avoids the introspection-vs-disk \
                       encoding drift you'd get from reading parameter values live (e.g. an enum reported \
                       numerically here but stored as a string on disk)."
    )]
    pub(crate) async fn get_project_schema(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_project_schema() {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Lint the whole project: run graph diagnostics over every instrument and \
                       aggregate them into one report. Surfaces behavioural issues schema validation \
                       can't — unconnected ports, silent voices, feedback loops, missing audio paths, \
                       and tracks that reference missing instruments — with total error/warning/info counts. A healthy project reports \
                       error_count = 0 and warning_count = 0. Use after loading a project or before export."
    )]
    pub(crate) async fn lint_project(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.lint_project() {
            Ok(report) => to_json(&report),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all available module types. By default returns the full catalog \
        (every port + parameter per type) — this is hundreds of KB and can exceed the tool-result \
        token cap, so pass brief:true for a compact {type_key, name, category} list, then call \
        get_module_type_info for the one type you want. Use the type_key to add modules with \
        add_module."
    )]
    pub(crate) async fn list_module_types(
        &self,
        params: Parameters<ListModuleTypesParam>,
    ) -> String {
        if params.0.brief.unwrap_or(false) {
            return match self.bridge.list_module_types_brief() {
                Ok(types) => to_json(&types),
                Err(e) => format!("Error: {e}"),
            };
        }
        match self.bridge.list_module_types() {
            Ok(types) => to_json(&types),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed info for a single module type by its type key (e.g. 'osc', 'flt', 'env'). \
                       Returns ports, parameters with ranges/units/choices, and signal flow hints. \
                       Lighter than list_module_types when you already know which module you need."
    )]
    pub(crate) async fn get_module_type_info(
        &self,
        params: Parameters<GetModuleTypeInfoParam>,
    ) -> String {
        let type_key = params.0.type_key.trim();
        if type_key.is_empty() {
            return validation_err(McpBridgeError::EmptyName { kind: "type_key" });
        }
        match self.bridge.get_module_type_info(type_key) {
            Ok(info) => to_json(&info),
            Err(e) => {
                let hint = if matches!(e, McpBridgeError::InvalidModuleType(_)) {
                    match self.bridge.list_module_types() {
                        Ok(types) => {
                            let keys: Vec<&str> =
                                types.iter().map(|t| t.type_key.as_str()).collect();
                            let similar = find_similar(type_key, &keys, 3);
                            if similar.is_empty() {
                                "\nHint: use list_module_types to see all available type keys."
                                    .to_string()
                            } else {
                                format!(
                                    "\nHint: did you mean {}? Use list_module_types to see all.",
                                    similar.join(", ")
                                )
                            }
                        }
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                format!("Error: {e}{hint}")
            }
        }
    }

    #[tool(
        description = "Search available module types by category, port signal type, or text query. \
                       All filters are optional and combined with AND logic. Returns matching modules \
                       with full port/parameter details."
    )]
    pub(crate) async fn search_modules(&self, params: Parameters<SearchModulesParam>) -> String {
        let p = params.0;
        // Validate category if provided
        if let Some(ref cat) = p.category
            && !["voice", "effect", "visualizer"].contains(&cat.as_str())
        {
            return format!(
                "Error: invalid category '{}'. Valid categories: voice, effect, visualizer",
                cat
            );
        }
        // Validate signal types if provided
        for (name, val) in [
            ("has_input_type", &p.has_input_type),
            ("has_output_type", &p.has_output_type),
        ] {
            if let Some(st) = val
                && !VALID_SIGNAL_TYPES.contains(&st.as_str())
            {
                return format!(
                    "Error: invalid {name} '{}'. Valid signal types: audio, control, gate, midi",
                    st
                );
            }
        }
        match self.bridge.search_modules(
            p.category.as_deref(),
            p.has_input_type.as_deref(),
            p.has_output_type.as_deref(),
            p.query.as_deref(),
        ) {
            Ok(result) => {
                if result.modules.is_empty() {
                    let mut hint = "No modules matched your filters.".to_string();
                    if !result.did_you_mean.is_empty() {
                        hint.push_str(&format!(
                            " Did you mean: {}?",
                            result.did_you_mean.join(", ")
                        ));
                    } else if p.query.is_some() {
                        hint.push_str(" Try a broader text query or remove filters.");
                    }
                    hint
                } else {
                    let count = result.modules.len();
                    let json = to_json(&result.modules);
                    format!("{json}\n\n({count} module(s) matched)")
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all port signal types with descriptions, value ranges, and compatibility. \
                       Use this to understand which port types can connect to each other."
    )]
    pub(crate) async fn list_port_types(&self, _params: Parameters<NoParams>) -> String {
        use crate::types::PortSignalTypeInfo;
        use synth_core::PortType;

        let compatible_with = |source: PortType| {
            PortType::ALL
                .into_iter()
                .filter(|destination| source.can_drive(*destination))
                .map(|destination| destination.id().to_owned())
                .collect()
        };
        let types = vec![
            PortSignalTypeInfo {
                signal_type: PortType::Audio.id().to_owned(),
                description: "Audio-rate signal, processed sample-by-sample at the engine sample rate.".to_string(),
                value_range: "Typically -1.0 to +1.0 (can exceed for hot signals)".to_string(),
                compatible_with: compatible_with(PortType::Audio),
            },
            PortSignalTypeInfo {
                signal_type: PortType::Control.id().to_owned(),
                description: "Control signal for parameter modulation (pitch CV, filter cutoff CV, etc.); carried sample-by-sample like audio.".to_string(),
                value_range: "0.0 to 1.0 (unipolar) or -1.0 to +1.0 (bipolar), depends on source".to_string(),
                compatible_with: compatible_with(PortType::Control),
            },
            PortSignalTypeInfo {
                signal_type: PortType::Gate.id().to_owned(),
                description: "Gate or trigger signal for note state, clocks, resets, and retriggering; inputs treat values above 0.5 as high.".to_string(),
                value_range: "Typically 0.0 or 1.0".to_string(),
                compatible_with: compatible_with(PortType::Gate),
            },
            PortSignalTypeInfo {
                signal_type: PortType::Midi.id().to_owned(),
                description: "MIDI event data (note on/off, CC, pitch bend). Only connects to MIDI ports.".to_string(),
                value_range: "Structured MIDI events".to_string(),
                compatible_with: compatible_with(PortType::Midi),
            },
        ];
        to_json(&types)
    }

    #[tool(
        description = "Get the full YAMS (Yet Another Modulation Script) Markdown reference: shared grammar, functions, arrays, state and `param` knobs, plus the Mod Matrix, Script (4 CV inputs/outputs), AudioScript, and Note Grid note-event dialects. Read this before using set_mod_matrix_script or set_note_graph_script."
    )]
    pub(crate) async fn get_yams_reference(&self, _params: Parameters<NoParams>) -> String {
        synth_script::REFERENCE.to_string()
    }

    #[tool(
        description = "Check whether a connection between two module ports would be valid. \
                       Returns compatibility info and hints. Use this before connect to avoid errors."
    )]
    pub(crate) async fn check_connection(
        &self,
        params: Parameters<CheckConnectionParam>,
    ) -> String {
        let p = params.0;
        match self.bridge.check_connection(
            p.instrument_id,
            &p.from_module,
            &p.from_port,
            &p.to_module,
            &p.to_port,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ========================================================================
    // BATCH EXECUTE
    // ========================================================================
}
