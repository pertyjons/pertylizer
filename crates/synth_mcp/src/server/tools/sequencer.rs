//! sequencer MCP tool handlers.

use super::super::*;

#[tool_router(router = sequencer_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "Set or clear the song's free-text description (intent / mood / production \
        notes). Pass \"\" to clear. Surfaces in get_song_info.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_song_description(
        &self,
        params: Parameters<SetSongDescriptionParam>,
    ) -> String {
        match self.bridge.set_song_description(&params.0.description) {
            Ok(()) => format!(
                "OK: set song description ({} chars)",
                params.0.description.chars().count()
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set or clear a pattern's free-text description (its musical intent, e.g. \
        \"chorus drop, half-time feel\"). Pass \"\" to clear. Surfaces in list_patterns.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_pattern_description(
        &self,
        params: Parameters<SetPatternDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_pattern_description(it.pattern_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "pattern descriptions set", &[], &errors)
    }

    #[tool(
        description = "Get song info: name, author, tempo, time signature, length, pattern/track counts",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_song_info(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_song_info() {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the song tempo in BPM (typically 60-200, e.g. 120.0 for standard pop tempo).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_song_tempo(&self, params: Parameters<SetSongTempoParam>) -> String {
        if let Err(e) = validate_range("tempo", params.0.bpm.as_f32(), 20.0, 999.0) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_tempo(params.0.bpm) {
            Ok(()) => format!("OK: tempo set to {} BPM", params.0.bpm),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add or replace tempo-map points at absolute ticks (960 ticks per quarter note). Each point is a step by default, or set ramp=true for a linear accelerando/ritardando toward the next point. This edits the tempo MAP (position-specific tempo), NOT the global default tempo — use set_song_tempo for that. A point replaces any existing change at the same tick. Array-first: pass multiple points in one call. Inspect the map via get_tempo_map or get_song_info.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_tempo_at(&self, params: Parameters<SetTempoAtParam>) -> String {
        for point in &params.0.points {
            if let Err(e) = validate_range("tempo", point.bpm.as_f32(), 20.0, 999.0) {
                return format!("Error: {e}");
            }
        }
        let points: Vec<(Tick, f32, bool)> = params
            .0
            .points
            .iter()
            .map(|p| (p.tick, p.bpm.as_f32(), p.ramp))
            .collect();
        match self.bridge.set_tempo_at(&points) {
            Ok(()) => format!("OK: set {} tempo-map point(s)", points.len()),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove tempo-map points at the given absolute ticks. Returns how many were removed. Does not affect the global default tempo (set_song_tempo).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_tempo_at(&self, params: Parameters<RemoveTempoAtParam>) -> String {
        match self.bridge.remove_tempo_at(&params.0.ticks) {
            Ok(n) => format!("OK: removed {n} tempo-map point(s)"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the tempo map: position-specific tempo changes, sorted by tick. Does not include the global default tempo — see get_song_info for that.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_tempo_map(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_tempo_map() {
            Ok(map) => to_json(&map),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the transport loop region in beats. When enabled, playback wraps from end_beats back to start_beats. Visible on the arrangement ruler. Use clear_transport_loop to disable, or get_song_info to inspect the current state.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_transport_loop(
        &self,
        params: Parameters<SetTransportLoopParam>,
    ) -> String {
        if let Err(e) = validate_range("start_beats", params.0.start_beats, 0.0, 9999.0) {
            return validation_err(e);
        }
        if let Err(e) = validate_range("end_beats", params.0.end_beats, 0.0, 9999.0) {
            return validation_err(e);
        }
        match self.bridge.set_transport_loop(
            params.0.start_beats,
            params.0.end_beats,
            params.0.enabled,
        ) {
            Ok(()) => {
                if params.0.enabled {
                    format!(
                        "OK: transport loop {} -> {} beats (enabled)",
                        params.0.start_beats, params.0.end_beats
                    )
                } else {
                    "OK: transport loop stored (disabled)".to_string()
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Clear the transport loop region. Equivalent to set_transport_loop with enabled=false; playback stops wrapping.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn clear_transport_loop(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.clear_transport_loop() {
            Ok(()) => "OK: transport loop cleared".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the song name. Shown in the transport bar and saved with the project.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_song_name(&self, params: Parameters<SetSongNameParam>) -> String {
        if let Err(e) = validate_name("song", &params.0.name) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_name(&params.0.name) {
            Ok(()) => format!("OK: song name set to '{}'", params.0.name),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Patterns ===

    #[tool(
        description = "List all patterns in the song with their names, lengths, and note counts",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_patterns(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_patterns() {
            Ok(patterns) => to_json(&patterns),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Delete one or more patterns by ID. Also removes all placements of each pattern.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_pattern(&self, params: Parameters<DeletePatternsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.delete_pattern(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "patterns deleted", &[], &errors)
    }

    // === Sequencer: Notes ===

    #[tool(
        description = "List all notes in a pattern. Returns note ID, MIDI pitch (0-127), pitch name (e.g. 'C4'), start/duration in beats, and velocity (0-127).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_notes(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.list_notes(params.0.pattern_id) {
            Ok(notes) => to_json(&notes),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove one or more notes from a pattern by note ID.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_note(&self, params: Parameters<RemoveNotesParam>) -> String {
        let p = params.0;
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for note_id in &p.note_ids {
            match self.bridge.remove_note(p.pattern_id, *note_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("note {note_id}: {e}")),
            }
        }
        batch_msg(ok_count, "notes removed", &[], &errors)
    }

    // === Sequencer: pattern freeze ===

    #[tool(
        description = "Bake a pattern's note processing into concrete notes (Model-A freeze), for hand-editing. A bound note graph bakes (the binding is cleared; the pooled graph survives); otherwise per-note ornaments and note-scope articulation bake. DESTRUCTIVE: the generative setup cannot be un-baked — re-bind the graph to restore it. Returns the resulting note count.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn freeze_pattern(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.freeze_pattern(params.0.pattern_id) {
            Ok((note_count, dropped)) => {
                // `dropped > 0` = a graph node hit the 128-event cap during the
                // bake; surface it so the overflow isn't silently swallowed.
                let warning = (dropped > 0).then(|| {
                    format!(
                        "{dropped} events dropped during freeze (a graph node hit the \
                         128-event cap)"
                    )
                });
                to_json(&serde_json::json!({
                    "pattern_id": params.0.pattern_id,
                    "note_count": note_count,
                    "dropped_events": dropped,
                    "warning": warning,
                }))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set or clear per-note timed-repeat ornaments (flam/drag/ruff/roll/grace note) on one or more notes. Each item gives the Ornament JSON to set, or null to clear it. Ornaments expand each note into its figure at playback time.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_note_ornament(
        &self,
        params: Parameters<SetNoteOrnamentParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let (pattern_id, note_id) = (it.pattern_id, it.note_id);
            match self
                .bridge
                .set_note_ornament(pattern_id, note_id, it.ornament)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("pattern {pattern_id} note {note_id}: {e}")),
            }
        }
        batch_msg(ok_count, "note ornaments updated", &[], &errors)
    }

    // === Sequencer: Note Grid (pooled note-processing graphs) ===

    #[tool(
        description = "List every pooled Note Grid graph in summary form: id, name, description, color, module/connection counts, and how many patterns bind it.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_note_graphs(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_note_graphs() {
            Ok(graphs) => to_json(&graphs),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get full detail for selected Note Grid graphs, or every graph when graph_ids is omitted. Returns stable graph-id order and a per-graph detail/error result.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_note_graph(&self, params: Parameters<GetNoteGraphParam>) -> String {
        if params.0.graph_id.is_some() && params.0.graph_ids.is_some() {
            return "Error: provide graph_id or graph_ids, not both".to_string();
        }
        if let Some(graph_id) = params.0.graph_id {
            return match self.bridge.get_note_graph(graph_id) {
                Ok(detail) => to_json(&detail),
                Err(e) => format!("Error: {e}"),
            };
        }
        let mut ids = match params.0.graph_ids {
            Some(ids) => ids,
            None => match self.bridge.list_note_graphs() {
                Ok(graphs) => graphs.into_iter().map(|graph| graph.id).collect(),
                Err(e) => return format!("Error: {e}"),
            },
        };
        ids.sort_unstable();
        ids.dedup();
        let results: Vec<_> = ids
            .into_iter()
            .map(|graph_id| match self.bridge.get_note_graph(graph_id) {
                Ok(detail) => serde_json::json!({"graph_id": graph_id, "detail": detail}),
                Err(e) => serde_json::json!({"graph_id": graph_id, "error": e.to_string()}),
            })
            .collect();
        to_json(&results)
    }

    #[tool(
        description = "Create an empty pooled Note Grid graph. Returns the new graph id. Add modules with add_note_graph_module, wire them with connect_note_graph, then bind it to a pattern with set_pattern_note_graph.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_note_graph(
        &self,
        params: Parameters<CreateNoteGraphParam>,
    ) -> String {
        let p = params.0;
        match self
            .bridge
            .create_note_graph(p.name, p.description, p.color)
        {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Partially update name, description, and color for one or more Note Grid graphs.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_note_graph_metadata(
        &self,
        params: Parameters<SetNoteGraphMetadataParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            match self.bridge.set_note_graph_metadata(
                item.graph_id,
                item.name,
                item.description,
                item.color,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {}: {e}", item.graph_id)),
            }
        }
        batch_msg(ok_count, "note graph metadata updated", &[], &errors)
    }

    #[tool(
        description = "Duplicate a pooled Note Grid graph — nodes, connections, metadata, and editor layout — as '<name> copy'. Use before diverging a shared graph for one pattern (pair with set_pattern_note_graph to repoint). Returns the new graph's id.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn duplicate_note_graph(
        &self,
        params: Parameters<NoteGraphIdParam>,
    ) -> String {
        match self.bridge.duplicate_note_graph(params.0.graph_id) {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Delete one or more pooled Note Grid graphs. DESTRUCTIVE: every pattern that binds a deleted graph is unbound (falls back to dry playback). Returns the per-graph count of patterns that were unbound.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_note_graph(
        &self,
        params: Parameters<DeleteNoteGraphParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for graph_id in params.0.graph_ids {
            match self.bridge.delete_note_graph(graph_id) {
                Ok(unbound) => oks.push(format!("graph {graph_id} (unbound {unbound} patterns)")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "note graphs deleted", &oks, &errors)
    }

    #[tool(
        description = "Add one or more modules to Note Grid graphs. Each module is externally-tagged NoteModuleConfig JSON (Processor/Euclidean/ProbabilityGate/NoteLfo/StepLfo/NoteEnvelope/NoteScriptTransform/NoteDelay/Ratchet). For a NoteScriptTransform, add it with a source then compile it with set_note_graph_script. Returns each new module id.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_note_graph_module(
        &self,
        params: Parameters<AddNoteGraphModuleParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            let module = match serde_json::to_value(it.module) {
                Ok(module) => module,
                Err(e) => {
                    errors.push(format!("{graph_id}: {e}"));
                    continue;
                }
            };
            match self
                .bridge
                .add_note_graph_module(graph_id, module, it.description)
            {
                Ok(module_id) => oks.push(format!("graph {graph_id} @ module {module_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "note graph modules added", &oks, &errors)
    }

    #[tool(
        description = "Replace a Note Grid module's config in place (config edit), keeping its id and connections. A change that would orphan an existing connection is rejected.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_note_graph_module(
        &self,
        params: Parameters<SetNoteGraphModuleParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            let module = match serde_json::to_value(item.module) {
                Ok(module) => module,
                Err(e) => {
                    errors.push(format!(
                        "graph {} module {}: {e}",
                        item.graph_id, item.module_id
                    ));
                    continue;
                }
            };
            match self.bridge.set_note_graph_module(
                item.graph_id,
                item.module_id,
                module,
                item.description,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "graph {} module {}: {e}",
                    item.graph_id, item.module_id
                )),
            }
        }
        batch_msg(ok_count, "note graph modules updated", &[], &errors)
    }

    #[tool(
        description = "Set a Note Grid NoteScriptTransform node's YAMS note_event source, compile it, and install the program. The script runs per note (1:1): read note_pitch/note_vel/note_dur/tick and value inputs in1..in4, assign out.pitch/out.vel/out.dur/out.gate. Returns the compile status; the source is always saved, and an empty source or a compile error leaves the node pass-through (the diagnostic is in the returned status). Add the node first with add_note_graph_module ({\"NoteScriptTransform\":{\"source\":\"\"}}).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_note_graph_script(
        &self,
        params: Parameters<SetNoteGraphScriptParam>,
    ) -> String {
        let p = params.0;
        match self
            .bridge
            .set_note_graph_script(p.graph_id, p.module_id, p.source)
        {
            Ok(status) => status,
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove a module (and every connection touching it) from a Note Grid graph.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_note_graph_module(
        &self,
        params: Parameters<RemoveNoteGraphModuleParam>,
    ) -> String {
        let p = params.0;
        match self
            .bridge
            .remove_note_graph_module(p.graph_id, p.module_id)
        {
            Ok(()) => format!("Module {} removed from graph {}", p.module_id, p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect two Note Grid modules (one or many). port is 'note_stream' (the linear spine), 'value', or 'gate'; to_input selects the target's value-input port for modulation edges. Each connection is validated for linearity (one stream in/out per node), acyclicity, and endpoint types.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn connect_note_graph(
        &self,
        params: Parameters<ConnectNoteGraphParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let port = it.port.unwrap_or_else(|| "note_stream".to_string());
            let to_input = it.to_input.unwrap_or(0);
            match self
                .bridge
                .connect_note_graph(it.graph_id, it.from, it.to, port, to_input)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {} {}→{}: {e}", it.graph_id, it.from, it.to)),
            }
        }
        batch_msg(ok_count, "note graph connections added", &[], &errors)
    }

    #[tool(
        description = "Bind patterns to Note Grid graphs (one or many). Set graph_id to bind, or null/omitted to clear the binding (the pattern's raw notes + per-note ornaments then play). A bound graph processes the pattern's notes at playback.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_pattern_note_graph(
        &self,
        params: Parameters<SetPatternNoteGraphParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let pattern_id = it.pattern_id;
            match self.bridge.set_pattern_note_graph(pattern_id, it.graph_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("pattern {pattern_id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            "pattern note-graph bindings updated",
            &[],
            &errors,
        )
    }

    #[tool(
        description = "Bind individual notes to Note Grid graphs for per-note articulation (flam / strum / arp / echo of one note), one or many. Set graph_id to bind, or null/omitted to clear. The note-scope graph runs on that note's material during source collection — before, and feeding, the pattern-scope graph / rack — and is decorrelated per note. Dangling graph ids are rejected.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_note_note_graph(
        &self,
        params: Parameters<SetNoteNoteGraphParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            match self
                .bridge
                .set_note_note_graph(it.pattern_id, it.note_id, it.graph_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "pattern {} note {}: {e}",
                    it.pattern_id, it.note_id
                )),
            }
        }
        batch_msg(ok_count, "note note-graph bindings updated", &[], &errors)
    }

    // === Sequencer: Mod Grid (pooled control-rate modulator graphs) ===

    #[tool(
        description = "List every pooled Mod Grid graph in summary form (id, name, scope, assigned tracks, node/cable counts).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_mod_graphs(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_mod_graphs() {
            Ok(graphs) => to_json(&graphs),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get full detail for selected Mod Grid graphs, or every graph when graph_ids is omitted. Returns stable graph-id order and a per-graph detail/error result.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_mod_graph(&self, params: Parameters<GetModGraphParam>) -> String {
        if params.0.graph_id.is_some() && params.0.graph_ids.is_some() {
            return "Error: provide graph_id or graph_ids, not both".to_string();
        }
        if let Some(graph_id) = params.0.graph_id {
            return match self.bridge.get_mod_graph(graph_id) {
                Ok(detail) => to_json(&detail),
                Err(e) => format!("Error: {e}"),
            };
        }
        let mut ids = match params.0.graph_ids {
            Some(ids) => ids,
            None => match self.bridge.list_mod_graphs() {
                Ok(graphs) => graphs.into_iter().map(|graph| graph.id).collect(),
                Err(e) => return format!("Error: {e}"),
            },
        };
        ids.sort_unstable();
        ids.dedup();
        let results: Vec<_> = ids
            .into_iter()
            .map(|graph_id| match self.bridge.get_mod_graph(graph_id) {
                Ok(detail) => serde_json::json!({"graph_id": graph_id, "detail": detail}),
                Err(e) => serde_json::json!({"graph_id": graph_id, "error": e.to_string()}),
            })
            .collect();
        to_json(&results)
    }

    #[tool(
        description = "Create an empty pooled Mod Grid graph (a control-rate modulator graph whose outputs write into the automation target space). scope is 'global' (one always-on instance, default) or 'track'. Returns the new graph id.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_mod_graph(&self, params: Parameters<CreateModGraphParam>) -> String {
        let p = params.0;
        match self
            .bridge
            .create_mod_graph(p.name, p.description, p.color, p.scope)
        {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Duplicate one Mod Grid graph, preserving nodes, layout, metadata, scope, and assignments.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn duplicate_mod_graph(
        &self,
        params: Parameters<DuplicateModGraphParam>,
    ) -> String {
        let graph_id = params.0.graph_id;
        match self.bridge.duplicate_mod_graph(graph_id) {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Partially update name, description, and color for one or more Mod Grid graphs.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_mod_graph_metadata(
        &self,
        params: Parameters<SetModGraphMetadataParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            match self.bridge.set_mod_graph_metadata(
                item.graph_id,
                item.name,
                item.description,
                item.color,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {}: {e}", item.graph_id)),
            }
        }
        batch_msg(ok_count, "mod graph metadata updated", &[], &errors)
    }

    #[tool(
        description = "Delete one or more pooled Mod Grid graphs (removing their running instances).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_mod_graph(&self, params: Parameters<DeleteModGraphParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for graph_id in params.0.graph_ids {
            match self.bridge.delete_mod_graph(graph_id) {
                Ok(()) => oks.push(format!("graph {graph_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graphs deleted", &oks, &errors)
    }

    #[tool(
        description = "Set a Mod Grid graph's scope: 'global' (one always-on instance) or 'track' (one instance per assigned track). Switching to 'global' clears any track assignments.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_mod_graph_scope(
        &self,
        params: Parameters<SetModGraphScopeParam>,
    ) -> String {
        let p = params.0;
        match self.bridge.set_mod_graph_scope(p.graph_id, p.scope) {
            Ok(()) => format!("Graph {} scope updated", p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Assign a track-scope Mod Grid graph to a set of tracks (one running instance per track; relative 'this track' targets resolve to each host). Replaces the current assignment; unknown track ids are rejected.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn assign_mod_graph(&self, params: Parameters<AssignModGraphParam>) -> String {
        let p = params.0;
        match self.bridge.assign_mod_graph(p.graph_id, p.tracks) {
            Ok(()) => format!("Graph {} assignments updated", p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add one or more nodes to Mod Grid graphs. Each node is externally-tagged ModNodeConfig JSON: a hosted Module (lfo/mseg/envelope_follower/etc.), a Macro/Transport/MidiCc/AudioTap source, or a Target routing sink. Connect a source's output to a Target's 'in' port with connect_mod_graph. Returns each new node id.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_mod_graph_node(
        &self,
        params: Parameters<AddModGraphNodeParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            let node = match serde_json::to_value(it.node) {
                Ok(node) => node,
                Err(e) => {
                    errors.push(format!("{graph_id}: {e}"));
                    continue;
                }
            };
            match self
                .bridge
                .add_mod_graph_node(graph_id, node, it.description)
            {
                Ok(node_id) => oks.push(format!("graph {graph_id} @ node {node_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph nodes added", &oks, &errors)
    }

    #[tool(
        description = "Remove a Mod Grid node and every cable touching it.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_mod_graph_node(
        &self,
        params: Parameters<RemoveModGraphNodeParam>,
    ) -> String {
        let p = params.0;
        match self.bridge.remove_mod_graph_node(p.graph_id, p.node_id) {
            Ok(()) => format!("Node {} removed from graph {}", p.node_id, p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect one or more Mod Grid cables between named ports (e.g. a source module's 'out' to a Target node's 'in'). Validated: endpoints exist, a target isn't used as a source, single source per input port, no cycle.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn connect_mod_graph(
        &self,
        params: Parameters<ConnectModGraphParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            match self
                .bridge
                .connect_mod_graph(graph_id, it.from, it.from_port, it.to, it.to_port)
            {
                Ok(()) => oks.push(format!("graph {graph_id}: {} → {}", it.from, it.to)),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph cables added", &oks, &errors)
    }

    #[tool(
        description = "Remove one or more Mod Grid cables by their exact endpoints (the inverse of connect_mod_graph), leaving both nodes and every other cable intact. Use this to rewire a source without remove/re-add (which would drop the node's other cables and change its id).",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn disconnect_mod_graph(
        &self,
        params: Parameters<DisconnectModGraphParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            match self.bridge.disconnect_mod_graph(
                graph_id,
                it.from,
                it.from_port,
                it.to,
                it.to_port,
            ) {
                Ok(()) => oks.push(format!("graph {graph_id}: {} ↛ {}", it.from, it.to)),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph cables removed", &oks, &errors)
    }

    #[tool(
        description = "Edit a Mod Grid node's config in place, keeping its id and every cable touching it (unlike remove + re-add, which changes the id and drops its cables). Use to change a Target's address/amount, a Macro's value, or a hosted module's params. The graph is re-validated. `node` is externally-tagged ModNodeConfig JSON.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_mod_graph_node(
        &self,
        params: Parameters<SetModGraphNodeParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            let node = match serde_json::to_value(item.node) {
                Ok(node) => node,
                Err(e) => {
                    errors.push(format!(
                        "graph {} node {}: {e}",
                        item.graph_id, item.node_id
                    ));
                    continue;
                }
            };
            match self.bridge.set_mod_graph_node(
                item.graph_id,
                item.node_id,
                node,
                item.description,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "graph {} node {}: {e}",
                    item.graph_id, item.node_id
                )),
            }
        }
        batch_msg(ok_count, "mod graph nodes updated", &[], &errors)
    }

    #[tool(
        description = "List Mod Grid routing sinks — 'what writes to a target' — across all graphs, or just one when graph_id is set. The provenance answer to 'why is this parameter moving?'.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_mod_targets(&self, params: Parameters<ListModTargetsParam>) -> String {
        match self.bridge.list_mod_targets(params.0.graph_id) {
            Ok(targets) => to_json(&targets),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Tracks ===

    #[tool(
        description = "List all sequencer tracks with their names, instruments, and mute/solo state",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_tracks(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_tracks() {
            Ok(tracks) => to_json(&tracks),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Arrangement ===

    #[tool(
        description = "Remove one or more pattern placements from the arrangement. Each placement is identified exactly by pattern_id, track_id, and either start_beat or start_tick.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_placement(
        &self,
        params: Parameters<RemovePlacementsParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for pl in &params.0.placements {
            let start_tick = match arrangement_tick(pl.start_beat, pl.start_tick, "start") {
                Ok(tick) => tick,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            match self
                .bridge
                .remove_placement(pl.pattern_id, pl.track_id, start_tick.tick())
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "pattern {} on track {} at {}: {e}",
                    pl.pattern_id,
                    pl.track_id,
                    start_tick.tick()
                )),
            }
        }
        batch_msg(ok_count, "placements removed", &[], &errors)
    }

    #[tool(
        description = "List complete pattern-placement state, including exact ticks, transpose, gain, optional length override, effective length, and end tick.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_arrangement(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_arrangement() {
            Ok(placements) => to_json(&placements),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Batch operations ===

    #[tool(
        description = "Add one or more notes to a pattern in one call. Each note: pitch (MIDI 0-127, 60=C4), start_beat/duration_beats in beats, velocity (0-127).",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_note(&self, params: Parameters<AddNotesParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_note_input(n) {
                return validation_err(e);
            }
        }
        let notes: Vec<_> = params.0.notes.iter().map(note_input_to_bridge).collect();
        match self.bridge.add_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Update one or more notes in a pattern in one call. Only provided fields are changed per note; null fields keep their current value.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn update_note(&self, params: Parameters<UpdateNotesParam>) -> String {
        for u in &params.0.updates {
            if let Err(e) =
                validate_note_update_fields(u.pitch, u.velocity, u.start_beat, u.duration_beats)
            {
                return validation_err(e);
            }
        }
        let updates: Vec<_> = params
            .0
            .updates
            .iter()
            .map(|u| crate::bridge::BridgeNoteUpdate {
                note_id: u.note_id,
                pitch: u.pitch,
                start_beat: u.start_beat,
                duration_beats: u.duration_beats,
                velocity: u.velocity,
            })
            .collect();
        match self.bridge.update_notes(params.0.pattern_id, &updates) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Replace all notes in a pattern: clears existing notes, then adds the new ones. Use for full pattern rewrites.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn replace_notes(&self, params: Parameters<ReplaceNotesParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_note_input(n) {
                return validation_err(e);
            }
        }
        let notes: Vec<_> = params.0.notes.iter().map(note_input_to_bridge).collect();
        match self.bridge.replace_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Clear all notes from one or more patterns. Returns the total number of notes removed.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn clear_pattern(&self, params: Parameters<ClearPatternParam>) -> String {
        let mut ok_count = 0usize;
        let mut total = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.clear_pattern(*id) {
                Ok(count) => {
                    ok_count += 1;
                    total += count;
                }
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            &format!("patterns cleared ({total} notes removed)"),
            &[],
            &errors,
        )
    }

    #[tool(
        description = "Rename one or more patterns. The name is shown in the arrangement timeline and piano roll.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_pattern(&self, params: Parameters<RenamePatternParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("pattern", &it.name) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_pattern(it.pattern_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "patterns renamed", &[], &errors)
    }

    #[tool(
        description = "Set the length in beats of one or more patterns (e.g. 4.0 = one bar in 4/4, 8.0 = two bars).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_pattern_length(
        &self,
        params: Parameters<SetPatternLengthParam>,
    ) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_range("length_beats", it.length_beats, 0.001, 1024.0) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_pattern_length(it.pattern_id, it.length_beats)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "pattern lengths set", &[], &errors)
    }

    #[tool(
        description = "Duplicate one or more patterns including all notes and automation. Returns the new pattern IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn duplicate_pattern(
        &self,
        params: Parameters<DuplicatePatternParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.duplicate_pattern(*id) {
                Ok(new_id) => oks.push(format!("{id} → {new_id}")),
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(oks.len(), "patterns duplicated", &oks, &errors)
    }

    // === Song metadata ===

    #[tool(
        description = "Set the song author name.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_song_author(&self, params: Parameters<SetSongAuthorParam>) -> String {
        if let Err(e) = validate_name("author", &params.0.author) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_author(&params.0.author) {
            Ok(()) => format!("OK: song author set to '{}'", params.0.author),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the song time signature (e.g. 4/4, 3/4, 6/8).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_song_time_signature(
        &self,
        params: Parameters<SetSongTimeSignatureParam>,
    ) -> String {
        if let Err(e) = validate_time_signature(params.0.numerator, params.0.denominator) {
            return validation_err(e);
        }
        match self
            .bridge
            .set_song_time_signature(params.0.numerator, params.0.denominator)
        {
            Ok(()) => format!(
                "OK: time signature set to {}/{}",
                params.0.numerator, params.0.denominator
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Batch parameter set ===

    #[tool(
        description = "Create one or more patterns in one call, optionally with inline notes and automation. Returns per-pattern results with assigned IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_pattern(&self, params: Parameters<CreatePatternsParam>) -> String {
        for (i, pat) in params.0.patterns.iter().enumerate() {
            if let Err(e) = validate_name("pattern", &pat.name) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            if let Err(e) = validate_range("length_beats", pat.length_beats, 0.001, 1024.0) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            if let Some(ref notes) = pat.notes {
                for n in notes {
                    if let Err(e) = validate_note_input(n) {
                        return validation_err(McpBridgeError::Other(format!(
                            "pattern[{i}] note: {e}"
                        )));
                    }
                }
            }
            if let Some(ref auto) = pat.automation
                && let Err(e) = validate_automation_points_input(auto)
            {
                return validation_err(McpBridgeError::Other(format!(
                    "pattern[{i}] automation: {e}"
                )));
            }
        }
        let patterns: Vec<_> = params
            .0
            .patterns
            .into_iter()
            .map(|p| crate::bridge::BridgePatternData {
                name: p.name,
                length_beats: p.length_beats,
                notes: p
                    .notes
                    .unwrap_or_default()
                    .iter()
                    .map(note_input_to_bridge)
                    .collect(),
                automation: convert_automation_points(p.automation),
            })
            .collect();
        match self.bridge.create_patterns(&patterns) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create one or more tracks in one call. Optionally assign an instrument per track. Returns per-track results with assigned IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_track(&self, params: Parameters<CreateTracksParam>) -> String {
        for t in &params.0.tracks {
            if let Err(e) = validate_name("track", &t.name) {
                return validation_err(e);
            }
        }
        let tracks: Vec<_> = params
            .0
            .tracks
            .into_iter()
            .map(|t| crate::bridge::BridgeTrackData {
                name: t.name,
                instrument_id: t.instrument_id,
            })
            .collect();
        match self.bridge.create_tracks(&tracks) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Place one or more patterns with complete PatternPlacement properties. Address positions and optional lengths in beats or exact ticks; transpose_semitones defaults to 0, gain to 1, and loop_mode to repeat.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn place_pattern(&self, params: Parameters<PlacePatternsParam>) -> String {
        let placements = match params
            .0
            .placements
            .into_iter()
            .map(placement_to_bridge)
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(placements) => placements,
            Err(error) => return validation_err(error),
        };
        match self.bridge.place_patterns(&placements) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Update or move one or more existing pattern placements. Identify each by pattern_id, track_id and start_beat/start_tick; optionally set a new track/start, transpose, gain, length, or loop_mode (repeat/clip). Set clear_length_override to restore the pattern length.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn update_placement(
        &self,
        params: Parameters<UpdatePlacementsParam>,
    ) -> String {
        let mut updates = Vec::with_capacity(params.0.updates.len());
        for update in params.0.updates {
            let start_tick = match arrangement_tick(update.start_beat, update.start_tick, "start") {
                Ok(tick) => tick,
                Err(error) => return validation_err(error),
            };
            let new_start_tick = if update.new_start_beat.is_some()
                || update.new_start_tick.is_some()
            {
                match arrangement_tick(update.new_start_beat, update.new_start_tick, "new_start") {
                    Ok(tick) => Some(tick),
                    Err(error) => return validation_err(error),
                }
            } else {
                None
            };
            if let Some(transpose) = update.transpose_semitones
                && let Err(error) = validate_range("transpose_semitones", transpose, -127.0, 127.0)
            {
                return validation_err(error);
            }
            if let Some(gain) = update.gain
                && let Err(error) = validate_range("gain", gain, 0.0, 2.0)
            {
                return validation_err(error);
            }
            if update.clear_length_override
                && (update.length_beats.is_some() || update.length_ticks.is_some())
            {
                return validation_err(McpBridgeError::Other(
                    "clear_length_override conflicts with length_beats/length_ticks".to_string(),
                ));
            }
            let length_ticks = if update.clear_length_override {
                Some(None)
            } else {
                match arrangement_length(update.length_beats, update.length_ticks) {
                    Ok(Some(length)) => Some(Some(length)),
                    Ok(None) => None,
                    Err(error) => return validation_err(error),
                }
            };
            updates.push(crate::bridge::BridgePlacementUpdate {
                pattern_id: update.pattern_id,
                track_id: update.track_id,
                start: start_tick,
                new_track_id: update.new_track_id,
                new_start: new_start_tick,
                transpose_semitones: update.transpose_semitones,
                gain: update.gain,
                length_ticks,
                loop_mode: update.loop_mode,
            });
        }
        match self.bridge.update_placements(&updates) {
            Ok(result) => to_json(&result),
            Err(error) => format!("Error: {error}"),
        }
    }

    #[tool(
        description = "Build a complete song in one call: creates patterns (with notes and optional automation), tracks, and arrangement placements. \
                       Replaces the current song. Placements use array indices (pattern_index, track_index) since IDs are assigned during creation. \
                       Returns a summary with all assigned IDs.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn set_song(&self, params: Parameters<SetSongParam>) -> String {
        let p = params.0;
        // Validate song name
        if let Err(e) = validate_name("song", &p.name) {
            return validation_err(e);
        }
        // Validate patterns: length, notes, automation
        for (i, pat) in p.patterns.iter().enumerate() {
            if let Err(e) = validate_range("length_beats", pat.length_beats, 0.001, 1024.0) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            for n in &pat.notes {
                if let Err(e) = validate_note_input(n) {
                    return validation_err(McpBridgeError::Other(format!(
                        "pattern[{i}] note: {e}"
                    )));
                }
            }
            if let Some(ref auto) = pat.automation
                && let Err(e) = validate_automation_points_input(auto)
            {
                return validation_err(McpBridgeError::Other(format!(
                    "pattern[{i}] automation: {e}"
                )));
            }
        }
        // Validate track names
        for (i, t) in p.tracks.iter().enumerate() {
            if let Err(e) = validate_name("track", &t.name) {
                return validation_err(McpBridgeError::Other(format!("track[{i}]: {e}")));
            }
        }
        // Validate placement indices and complete placement properties.
        for (i, pl) in p.placements.iter().enumerate() {
            if pl.pattern_index >= p.patterns.len() {
                return validation_err(McpBridgeError::IndexOutOfBounds {
                    name: "pattern_index",
                    index: pl.pattern_index,
                    count: p.patterns.len(),
                });
            }
            if pl.track_index >= p.tracks.len() {
                return validation_err(McpBridgeError::IndexOutOfBounds {
                    name: "track_index",
                    index: pl.track_index,
                    count: p.tracks.len(),
                });
            }
            if let Err(error) = arrangement_tick(pl.start_beat, pl.start_tick, "start") {
                return validation_err(McpBridgeError::Other(format!("placement[{i}]: {error}")));
            }
            if let Some(transpose) = pl.transpose_semitones
                && let Err(error) = validate_range("transpose_semitones", transpose, -127.0, 127.0)
            {
                return validation_err(McpBridgeError::Other(format!("placement[{i}]: {error}")));
            }
            if let Some(gain) = pl.gain
                && let Err(error) = validate_range("gain", gain, 0.0, 2.0)
            {
                return validation_err(McpBridgeError::Other(format!("placement[{i}]: {error}")));
            }
            if let Err(error) = arrangement_length(pl.length_beats, pl.length_ticks) {
                return validation_err(McpBridgeError::Other(format!("placement[{i}]: {error}")));
            }
        }
        let patterns: Vec<_> = p
            .patterns
            .into_iter()
            .map(|pat| crate::bridge::BridgePatternData {
                name: pat.name,
                length_beats: pat.length_beats,
                notes: pat.notes.iter().map(note_input_to_bridge).collect(),
                automation: convert_automation_points(pat.automation),
            })
            .collect();
        let tracks: Vec<_> = p
            .tracks
            .into_iter()
            .map(|t| crate::bridge::BridgeTrackData {
                name: t.name,
                instrument_id: t.instrument_id,
            })
            .collect();
        let mut placements = Vec::with_capacity(p.placements.len());
        for pl in p.placements {
            let start_tick = match arrangement_tick(pl.start_beat, pl.start_tick, "start") {
                Ok(tick) => tick,
                Err(error) => return validation_err(error),
            };
            let length_ticks = match arrangement_length(pl.length_beats, pl.length_ticks) {
                Ok(length) => length,
                Err(error) => return validation_err(error),
            };
            placements.push(crate::bridge::BridgeSongPlacement {
                pattern_index: pl.pattern_index,
                track_index: pl.track_index,
                start: start_tick,
                transpose_semitones: pl.transpose_semitones.unwrap_or(0.0),
                gain: pl.gain.unwrap_or(1.0),
                length_ticks,
                loop_mode: pl.loop_mode.unwrap_or_default(),
            });
        }
        let tempo = p.tempo.unwrap_or(120.0);
        if let Err(e) = validate_range("tempo", tempo, 20.0, 999.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_song(&p.name, tempo, &patterns, &tracks, &placements)
        {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Transport ===

    #[tool(
        description = "Start sequencer playback from the current position. Use seq_seek first to set position.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn seq_play(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.seq_play() {
            Ok(()) => "OK: sequencer playing".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Stop sequencer playback and reset position",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn seq_stop(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.seq_stop() {
            Ok(()) => "OK: sequencer stopped".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Seek the sequencer to a beat position (0.0 = beginning)",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn seq_seek(&self, params: Parameters<SeqSeekParam>) -> String {
        if let Err(e) = validate_range("beat", params.0.beat, 0.0, 9999.0) {
            return format!("Error: {e}");
        }
        match self.bridge.seq_seek(params.0.beat) {
            Ok(()) => format!("OK: seeked to beat {}", params.0.beat),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Batch instrument building ===
}
