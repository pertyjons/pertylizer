//! discovery MCP tool handlers.

use super::super::*;

/// How many distinct type keys an unknown-type-key error offers.
const MAX_TYPE_KEY_HINTS: usize = synth_core::suggest::DEFAULT_MAX_HINTS;

/// How many `search_modules` matches a caller gets without asking for a number.
///
/// Twenty full `ModuleTypeInfo` records is a few tens of KB; all 75 is 520 KB
/// over the wire (text plus the identical structured half). See the handler.
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// The `\nHint:` line for an unrecognized `type_key`, naming up to
/// [`MAX_TYPE_KEY_HINTS`] near misses from the catalog.
///
/// Ranks against the display names as well as the keys, then answers with the
/// key. Spelling the module out ("filter") is the likeliest way to get a key
/// wrong, and it is nowhere near its key ("flt") by any string measure — only
/// the name it abbreviates connects the two.
fn type_key_hint(type_key: &str, types: &[crate::ModuleTypeBrief]) -> String {
    // Ranked per *type* rather than per spelling: a type that matches by key and
    // by name is one answer, and ranking the loose strings would spend two of
    // the three slots on it ("reverb" answering 'rev' twice).
    let hits = synth_core::suggest::similar_by(
        type_key,
        types.iter(),
        |t| [t.type_key.as_str(), t.name.as_str()],
        MAX_TYPE_KEY_HINTS,
    );
    if hits.is_empty() {
        return "\nHint: use list_module_types to see all available type keys.".to_string();
    }
    let list: Vec<String> = hits
        .iter()
        .map(|t| format!("'{}'", t.type_key.as_str()))
        .collect();
    format!(
        "\nHint: did you mean {}? Use list_module_types to see all.",
        list.join(", ")
    )
}

#[tool_router(router = discovery_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "List all instruments with ID, name, category, volume, pan, mute/solo state, and module/effect counts.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_instruments(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<InstrumentInfo>>, String> {
        match self.bridge.list_instruments() {
            Ok(instruments) => Ok(Json(instruments.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Auto-infer per-instrument profiles (role, envelope shape, pitch role, \
                       register, texture) for every instrument that at least one track routes to. \
                       Role values: drums, bass, lead, pad, pluck, keys, fx, unknown — each with a \
                       confidence in [0.0, 1.0] and a signal trail that explains the classification. \
                       Same inference path that `analyze_harmony`'s `exclude_drums = true` default \
                       uses; expose it directly to debug or override the classification. Manual \
                       `set_instrument_category` always wins (reports as `manual-override`).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_instrument_profiles(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<InstrumentProfileResult>>, String> {
        match self.bridge.get_instrument_profiles() {
            Ok(profiles) => Ok(Json(profiles.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get detailed information about a specific instrument including module count and effects",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_instrument_info(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<InstrumentInfo>, String> {
        match self.bridge.get_instrument_info(params.0.instrument_id) {
            Ok(info) => Ok(Json(info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List all modules in an instrument's voice graph with their types and names",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_modules(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<Listing<ModuleInfo>>, String> {
        match self.bridge.list_modules(params.0.instrument_id) {
            Ok(modules) => Ok(Json(modules.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get detailed info for a specific module including all parameters and port connections",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_module_info(
        &self,
        params: Parameters<ModuleParam>,
    ) -> Result<Json<ModuleInfo>, String> {
        match self
            .bridge
            .get_module_info(params.0.instrument_id, &params.0.module_id)
        {
            Ok(info) => Ok(Json(info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get all connections (cables) between modules in the voice graph. Returns from_module:from_port → to_module:to_port pairs.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_connections(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<Listing<ConnectionInfo>>, String> {
        match self.bridge.get_connections(params.0.instrument_id) {
            Ok(conns) => Ok(Json(conns.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get all active Mod Matrix routings across every Mod Matrix module in the instrument. Slot rows include semantic source IDs (e.g. 'lfo-1', 'env-2', or 'velocity'/'mod_wheel' for non-module sources) and dotted destination IDs (e.g. 'flt-1.cutoff'), plus amount in -1..1 and enabled flag. A slot with a YAMS control script (Step 2) also reports its `script` source text — then the offset is the script's output, not amount × source. Inactive slots (None → None, no script) are filtered out.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_mod_matrix_routings(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<Listing<MatrixRoutingInfo>>, String> {
        match self.bridge.get_mod_matrix_routings(params.0.instrument_id) {
            Ok(routings) => Ok(Json(routings.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get the current value of a specific module parameter. Returns name, raw value, formatted display string (e.g. '440 Hz'), and min/max/default range.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_parameter(
        &self,
        params: Parameters<GetParameterParam>,
    ) -> Result<Json<ParameterInfo>, String> {
        match self.bridge.get_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
        ) {
            Ok(info) => Ok(Json(info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get engine status: CPU usage (0.0-1.0), active voice count, peak/RMS meters (dB), sample rate, tempo, and whether sequencer is playing.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_engine_status(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<EngineStatus>, String> {
        match self.bridge.get_engine_status() {
            Ok(status) => Ok(Json(status)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get build/version info for the running application: version, build \
                       timestamp (ISO 8601 / RFC 3339 UTC, e.g. 2026-07-03T14:30:00Z), git \
                       commit hash, branch, and whether the working tree had uncommitted \
                       changes at build time. Git fields are null when the binary was built \
                       outside a git checkout.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_version(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<VersionInfo>, String> {
        match self.bridge.get_version() {
            Ok(info) => Ok(Json(info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Run diagnostics on the module graph to find issues like disconnected modules or missing connections",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_graph_diagnostics(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<Listing<GraphDiagnostic>>, String> {
        match self.bridge.get_graph_diagnostics(params.0.instrument_id) {
            Ok(diags) => Ok(Json(diags.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // Prose, for the same reason as `get_yams_reference` — and measured, because
    // the number is what settles it.
    //
    // `Json<T>` fills `content` *and* `structuredContent` from one value
    // (`CallToolResult::structured`, rmcp `model.rs:3964`), which the MCP spec
    // asks for: a tool answering structured content SHOULD also serialize it into
    // a text block for clients that read only `content`. Doubling a 6 KB listing
    // to buy addressable fields is a fair trade. This tool's payload is one
    // committed JSON-Schema *document*, and typing it billed **258 KB of text
    // plus 276 KB of structured content — 534 KB for a single call** (probed over
    // the real stdio handshake, 2026-08-11), for zero addressability a caller did
    // not already have: `ProjectSchemaInfo`'s schema field is an opaque JSON blob
    // either way.
    //
    // So it keeps the shape it had: the document in `content`, once, and no
    // `output_schema`. `PROSE_TOOLS` records the exception.
    #[tool(
        description = "Return the authoritative on-disk JSON Schema for `.ptz` project files \
                       plus the build version that generated it. Use this to validate or diff project \
                       files against the exact committed schema — it avoids the introspection-vs-disk \
                       encoding drift you'd get from reading parameter values live (e.g. an enum reported \
                       numerically here but stored as a string on disk).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_project_schema(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<String, String> {
        match self.bridge.get_project_schema() {
            Ok(info) => Ok(to_json(&info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Lint the whole project: run graph diagnostics over every instrument and \
                       aggregate them into one report. Surfaces behavioural issues schema validation \
                       can't — unconnected ports, silent voices, feedback loops, missing audio paths, \
                       and tracks that reference missing instruments — with total error/warning/info counts. A healthy project reports \
                       error_count = 0 and warning_count = 0. Use after loading a project or before export.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn lint_project(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<ProjectLintReport>, String> {
        match self.bridge.lint_project() {
            Ok(report) => Ok(Json(report)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // One shape, always: `{type_key, name, category, gui_only}` per type.
    //
    // The full catalog used to be the *default* here — every port and parameter
    // of all 75 types, hundreds of KB, which the description itself warned could
    // exceed a tool-result token cap, and which `get_module_type_info` already
    // answers one type at a time. A tool that returns a different type depending
    // on a bool cannot publish one `outputSchema` either, so the flag went with
    // the branch rather than earning a union schema. The full listing survives
    // where it has a real caller: `list_resources` builds a `synth://module-types/…`
    // resource per type from it.
    #[tool(
        description = "List every available module type: type_key, display name, category, and \
        whether it is GUI-only. Use the type_key to add modules with add_module, and \
        get_module_type_info for one type's ports and parameters.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_module_types(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<ModuleTypeBrief>>, String> {
        match self.bridge.list_module_types_brief() {
            Ok(types) => Ok(Json(types.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get detailed info for a single module type by its type key (e.g. 'osc', 'flt', 'env'). \
                       Returns ports, parameters with ranges/units/choices, and signal flow hints. \
                       Use list_module_types first to pick the type_key; this is the per-type \
                       detail that listing deliberately leaves out.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_module_type_info(
        &self,
        params: Parameters<GetModuleTypeInfoParam>,
    ) -> Result<Json<ModuleTypeInfo>, String> {
        let type_key = params.0.type_key.trim();
        if type_key.is_empty() {
            return Err(validation_err(McpBridgeError::EmptyName {
                kind: "type_key",
            }));
        }
        match self.bridge.get_module_type_info(type_key) {
            Ok(info) => Ok(Json(info)),
            Err(e) => {
                let hint = if matches!(e, McpBridgeError::InvalidModuleType(_)) {
                    // The brief listing, not the full one: the hint needs only
                    // each type's key and name, and the full listing builds
                    // ports, parameters and algorithm JSON for all 75 types.
                    match self.bridge.list_module_types_brief() {
                        Ok(types) => type_key_hint(type_key, &types),
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                Err(format!("Error: {e}{hint}"))
            }
        }
    }

    // Zero hits is an empty `modules` array, not a sentence.
    //
    // The no-match branch used to answer prose — "No modules matched your
    // filters. Did you mean: …?" — so a caller could not tell an empty result
    // from a failure without reading English, and the two branches had no single
    // type to publish. The guidance was worth keeping, so it moved into the
    // payload as `hint`; `did_you_mean` was already a field.
    //
    // The `limit` exists because *every* filter is optional, so `{}` matches all
    // 75 types with full port/parameter detail — measured at 250 KB of text plus
    // 270 KB of structured content, 520 KB for one call, and newly the obvious
    // way to ask for the whole catalog now that `list_module_types` answers the
    // compact listing. A search returns candidates to choose between; twenty of
    // them is a search, seventy-five is a catalog dump with a different tool's
    // name on it. `total_matched` keeps the truncation visible rather than
    // implied.
    #[tool(
        description = "Search available module types by category, port signal type, or text query. \
                       All filters are optional and combined with AND logic. Returns up to `limit` \
                       matching modules (default 20) with full port/parameter details, best match \
                       first, plus `total_matched`. No match is an empty `modules` array plus \
                       `did_you_mean` near-misses and a `hint`. For the whole catalog in compact \
                       form use list_module_types.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn search_modules(
        &self,
        params: Parameters<SearchModulesParam>,
    ) -> Result<Json<ModuleSearchResult>, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        if limit == 0 {
            return Err(validation_err(McpBridgeError::Other(
                "limit must be at least 1".to_string(),
            )));
        }
        // The category and signal-type filters are enums, so an invalid value
        // is refused by deserialization against the schema's own `enum` list —
        // there is no hand-written check here to fall behind that list.
        match self.bridge.search_modules(
            p.category.map(ModuleCategoryFilter::as_str),
            p.has_input_type.map(SignalTypeFilter::as_str),
            p.has_output_type.map(SignalTypeFilter::as_str),
            p.query.as_deref(),
        ) {
            Ok(mut result) => {
                // `total_matched` arrives already counted over the full match set;
                // this only ever truncates `modules` *after* reading it, so the
                // count keeps describing the search rather than the truncation.
                if result.modules.is_empty() {
                    result.hint = Some(if result.did_you_mean.is_empty() && p.query.is_some() {
                        "No modules matched. Try a broader text query or remove filters."
                            .to_string()
                    } else {
                        "No modules matched your filters.".to_string()
                    });
                } else if result.total_matched > limit {
                    // Truncation is stated, not silent: the matches are ranked, so
                    // the ones dropped are the weakest, but a caller still has to
                    // know they existed.
                    result.modules.truncate(limit);
                    result.hint = Some(format!(
                        "Showing the {limit} best of {} matches; raise `limit` or narrow the \
                         filters to see the rest.",
                        result.total_matched
                    ));
                }
                Ok(Json(result))
            }
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List all port signal types with descriptions, value ranges, and compatibility. \
                       Use this to understand which port types can connect to each other.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_port_types(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<PortSignalTypeInfo>>, String> {
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
        // Infallible — nothing above can fail, so there is no error branch. The
        // `Result` is the shape every typed tool answers, and what the batch
        // dispatch arm expects.
        //
        // Note the entries are hand-written, one per `PortType` variant; only
        // `compatible_with` iterates `PortType::ALL`. A new variant therefore
        // shows up in every compatibility list while having no entry of its own —
        // add it here too.
        debug_assert_eq!(
            types.len(),
            PortType::ALL.len(),
            "every PortType needs an entry in list_port_types"
        );
        Ok(Json(types.into()))
    }

    // The one tool §6.7 deliberately leaves as prose, and `ActionResult` is only
    // half the reason: this is a read, not an action, and that envelope
    // duplicates its payload into `message` — here the whole 40 KB YAMS
    // reference, sent twice for no added information (its other fields would
    // read `ok_count: 1` on a document fetch).
    //
    // `Json<T>` does not fix that, which is the part worth recording: rmcp fills
    // `content` *and* `structuredContent` from one value, so a
    // `{ "reference": "…" }` payload also ships the document twice — 80 KB for a
    // string with no structure to expose. A tool whose entire result *is* one
    // document has nothing to gain from a structured half, so it keeps returning
    // the document and publishes no `output_schema`.
    // `every_tool_either_publishes_a_schema_or_is_listed_as_prose` holds the
    // exception so it stays deliberate.
    #[tool(
        description = "Get the full YAMS (Yet Another Modulation Script) Markdown reference: shared grammar, functions, arrays, state and `param` knobs, plus the Mod Matrix, Script (4 CV inputs/outputs), AudioScript, and Note Grid note-event dialects. Read this before using set_mod_matrix_script or set_note_graph_script.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_yams_reference(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<String, String> {
        // Infallible — a compiled-in document. `Result` because that is the shape
        // every dispatchable tool answers, so no arm has to guess a verdict from
        // the text.
        Ok(synth_script::REFERENCE.to_string())
    }

    #[tool(
        description = "Check whether a connection between two module ports would be valid. \
                       Returns compatibility info and hints. Use this before connect to avoid errors.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn check_connection(
        &self,
        params: Parameters<CheckConnectionParam>,
    ) -> Result<Json<ConnectionCheckResult>, String> {
        let p = params.0;
        match self.bridge.check_connection(
            p.instrument_id,
            &p.from_module,
            &p.from_port,
            &p.to_module,
            &p.to_port,
        ) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // ========================================================================
    // BATCH EXECUTE
    // ========================================================================
}
