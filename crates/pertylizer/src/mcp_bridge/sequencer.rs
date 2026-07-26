use super::*;

impl synth_mcp::bridge::SequencerBridge for AppSynthBridge {
    fn get_song_info(&self) -> Result<SongInfo, McpBridgeError> {
        let (loop_enabled, loop_start, loop_end) = self.session.transport_loop_state();
        let song = self.shared.song.read();
        let ts = song.default_time_signature;
        Ok(SongInfo {
            name: song.name.clone(),
            author: song.author.clone(),
            description: song.description.clone(),
            tempo: song.default_tempo.as_f32(),
            time_signature: format!("{}/{}", ts.numerator, ts.denominator),
            length_seconds: song.length_seconds(),
            pattern_count: song.pattern_count(),
            track_count: song.track_count(),
            transport_loop_enabled: loop_enabled,
            transport_loop_start_beats: ticks_to_beats_u64(loop_start.0),
            transport_loop_end_beats: ticks_to_beats_u64(loop_end.0),
            tempo_map: tempo_points(&song),
        })
    }

    fn set_transport_loop(
        &self,
        start_beats: f32,
        end_beats: f32,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        if enabled && end_beats <= start_beats {
            return Err(McpBridgeError::Other(format!(
                "transport loop: end_beats ({end_beats}) must be > start_beats ({start_beats})"
            )));
        }
        let start = synth_sequencer::Tick(u64::from(beats_to_ticks(start_beats)));
        let end = synth_sequencer::Tick(u64::from(beats_to_ticks(end_beats)));
        self.session
            .set_transport_loop(start, end, enabled)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn clear_transport_loop(&self) -> Result<(), McpBridgeError> {
        self.session
            .set_transport_loop(
                synth_sequencer::Tick::ZERO,
                synth_sequencer::Tick::ZERO,
                false,
            )
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_song_tempo(&self, bpm: Bpm) -> Result<(), McpBridgeError> {
        {
            let mut song = self.shared.song.write();
            song.default_tempo = bpm;
        }
        // Also update engine transport tempo
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(bpm))
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetTempo",
            });
        }
        Ok(())
    }

    fn set_tempo_at(&self, points: &[(Tick, f32, bool)]) -> Result<(), McpBridgeError> {
        // The engine reads the tempo map live via `Song::tempo_at` each tick,
        // so mutating it under the shared-song lock is enough — no
        // `EngineCommand::SetTempo` (that is only for the global default).
        let mut song = self.shared.song.write();
        for &(tick, bpm, ramp) in points {
            song.set_tempo_ramp_at(tick, synth_core::Bpm::new(bpm), ramp);
        }
        Ok(())
    }

    fn remove_tempo_at(&self, ticks: &[Tick]) -> Result<usize, McpBridgeError> {
        let mut song = self.shared.song.write();
        let removed = ticks
            .iter()
            .filter(|&&tick| song.remove_tempo_change(tick))
            .count();
        Ok(removed)
    }

    fn get_tempo_map(&self) -> Result<Vec<TempoPoint>, McpBridgeError> {
        let song = self.shared.song.read();
        Ok(tempo_points(&song))
    }

    fn set_song_name(&self, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.name = name.to_string();
        Ok(())
    }

    // === Sequencer: Patterns ===

    fn list_patterns(&self) -> Result<Vec<PatternInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        let mut patterns: Vec<PatternInfo> = song
            .patterns()
            .map(|p| PatternInfo {
                id: p.id,
                name: p.name.clone(),
                description: p.description.clone(),
                length_beats: ticks_to_beats(p.length.0),
                note_count: p.note_count(),
            })
            .collect();
        patterns.sort_by_key(|p| p.id);
        Ok(patterns)
    }

    fn create_pattern(&self, name: &str, length_beats: f32) -> Result<PatternId, McpBridgeError> {
        if length_beats <= 0.0 {
            return Err(McpBridgeError::Other(format!(
                "length_beats must be positive, got {length_beats}"
            )));
        }
        let mut song = self.shared.song.write();
        let duration = synth_sequencer::Duration(beats_to_ticks(length_beats));
        let id = song.create_pattern(duration);
        if let Some(pattern) = song.pattern_mut(id) {
            pattern.name = name.to_string();
        }
        Ok(id)
    }

    fn delete_pattern(&self, pattern_id: PatternId) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let id = pattern_id;
        song.delete_pattern(id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        Ok(())
    }

    // === Sequencer: Notes ===

    fn list_notes(&self, pattern_id: PatternId) -> Result<Vec<NoteInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        let id = pattern_id;
        let pattern = song
            .pattern(id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        Ok(pattern.notes().iter().map(note_to_info).collect())
    }

    fn add_note(
        &self,
        pattern_id: PatternId,
        pitch: MidiNote,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
    ) -> Result<NoteInfo, McpBridgeError> {
        if start_beat < 0.0 {
            return Err(McpBridgeError::Other(format!(
                "start_beat must be >= 0, got {start_beat}"
            )));
        }
        if duration_beats <= 0.0 {
            return Err(McpBridgeError::Other(format!(
                "duration_beats must be positive, got {duration_beats}"
            )));
        }
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let p = synth_sequencer::Pitch::new(pitch.as_u8()).ok_or_else(|| {
            McpBridgeError::Other(format!("invalid pitch {pitch}, must be 0-127"))
        })?;
        let start = synth_sequencer::PatternTick(beats_to_ticks(start_beat));
        let vel = synth_core::Velocity::from_midi(velocity);

        let note = synth_sequencer::Note::new(
            synth_sequencer::NoteId(0), // will be reassigned by insert_note
            start,
            p,
            vel,
        )
        .with_duration(synth_sequencer::Duration(beats_to_ticks(duration_beats)));

        let note_id = pattern.insert_note(note);
        // Read back the inserted note to return full info
        Ok(pattern.note(note_id).map(note_to_info).unwrap_or(NoteInfo {
            id: note_id,
            pitch,
            pitch_name: p.to_string(),
            start_beat,
            duration_beats,
            velocity,
            ornament: None,
            note_graph: None,
        }))
    }

    fn remove_note(&self, pattern_id: PatternId, note_id: NoteId) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let nid = note_id;
        pattern
            .remove_note(nid)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;
        Ok(())
    }

    fn update_note(
        &self,
        pattern_id: PatternId,
        note_id: NoteId,
        pitch: Option<MidiNote>,
        start_beat: Option<f32>,
        duration_beats: Option<f32>,
        velocity: Option<u8>,
    ) -> Result<NoteInfo, McpBridgeError> {
        if let Some(s) = start_beat
            && s < 0.0
        {
            return Err(McpBridgeError::Other(format!(
                "start_beat must be >= 0, got {s}"
            )));
        }
        if let Some(d) = duration_beats
            && d <= 0.0
        {
            return Err(McpBridgeError::Other(format!(
                "duration_beats must be positive, got {d}"
            )));
        }
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let nid = note_id;
        let note = pattern
            .note_mut(nid)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;

        if let Some(p) = pitch {
            if let Some(new_pitch) = synth_sequencer::Pitch::new(p.as_u8()) {
                note.pitch = new_pitch;
            } else {
                return Err(McpBridgeError::Other(format!(
                    "invalid pitch {p}, must be 0-127"
                )));
            }
        }
        if let Some(s) = start_beat {
            note.start = synth_sequencer::PatternTick(beats_to_ticks(s));
        }
        if let Some(d) = duration_beats {
            note.duration = Some(synth_sequencer::Duration(beats_to_ticks(d)));
        }
        if let Some(v) = velocity {
            note.velocity = synth_core::Velocity::from_midi(v);
        }

        Ok(note_to_info(note))
    }

    // === Sequencer: pattern freeze ===

    fn freeze_pattern(&self, pattern_id: PatternId) -> Result<(usize, u32), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bpm = song.tempo_at(synth_sequencer::Tick(0));
        let pid = pattern_id;
        if song.pattern(pid).is_none() {
            return Err(McpBridgeError::PatternNotFound(pattern_id));
        }
        // Song::freeze_pattern bakes a bound note graph first (graph-over-rack
        // precedence), else per-note ornaments + note-scope articulation.
        let stats = song.freeze_pattern(pid, bpm);
        Ok((stats.notes, stats.dropped))
    }

    // === Sequencer: Note Grid (pooled note-processing graphs) ===

    fn list_note_graphs(&self) -> Result<Vec<NoteGraphInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        Ok(song
            .note_graphs()
            .map(|g| note_graph_info(&song, g))
            .collect())
    }

    fn get_note_graph(&self, graph_id: NoteGraphId) -> Result<NoteGraphDetail, McpBridgeError> {
        let song = self.shared.song.read();
        let gid = graph_id;
        let graph = song
            .note_graph(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        // Modules in processing (topological) order, falling back to id order if
        // the derived order is empty (e.g. a freshly loaded, un-rebuilt graph).
        let ordered: Vec<synth_sequencer::NoteModuleId> = if graph.processing_order.is_empty() {
            graph.nodes.keys().copied().collect()
        } else {
            graph.processing_order.clone()
        };
        let modules = ordered
            .iter()
            .filter_map(|id| graph.nodes.get(id).map(|cfg| (id, cfg)))
            .map(|(id, cfg)| {
                Ok(NoteGraphModuleInfo {
                    id: *id,
                    kind: cfg.kind().to_string(),
                    description: graph.node_descriptions.get(id).cloned().unwrap_or_default(),
                    config: serde_json::to_value(cfg)
                        .map_err(|e| McpBridgeError::Other(e.to_string()))?,
                })
            })
            .collect::<Result<Vec<_>, McpBridgeError>>()?;
        let connections = graph
            .connections
            .iter()
            .map(|c| NoteGraphConnectionInfo {
                from: c.from,
                to: c.to,
                port: note_port_to_str(c.port).to_string(),
                to_input: c.to_input,
            })
            .collect();
        Ok(NoteGraphDetail {
            info: note_graph_info(&song, graph),
            modules,
            connections,
        })
    }

    fn create_note_graph(
        &self,
        name: String,
        description: Option<String>,
        color: Option<String>,
    ) -> Result<NoteGraphId, McpBridgeError> {
        if name.trim().is_empty() {
            return Err(McpBridgeError::EmptyName { kind: "note graph" });
        }
        let description = description.unwrap_or_default();
        let len = description.chars().count();
        if len > MAX_MODULE_DESCRIPTION_LEN {
            return Err(McpBridgeError::DescriptionTooLong {
                len,
                max: MAX_MODULE_DESCRIPTION_LEN,
            });
        }
        let color = match color {
            Some(hex) => Some(synth_sequencer::TrackColor::from_hex(&hex).ok_or_else(|| {
                McpBridgeError::Other(format!("invalid color '{hex}' (expected #rrggbb)"))
            })?),
            None => None,
        };
        let mut song = self.shared.song.write();
        let gid = song.create_note_graph(name);
        if let Some(graph) = song.note_graph_mut(gid) {
            graph.description = description;
            graph.color = color;
        }
        Ok(gid)
    }

    fn duplicate_note_graph(&self, graph_id: NoteGraphId) -> Result<NoteGraphId, McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        song.duplicate_note_graph(gid)
            .map(|graph| graph.id)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))
    }

    fn delete_note_graph(&self, graph_id: NoteGraphId) -> Result<usize, McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let usage = song.note_graph_usage(gid);
        song.remove_note_graph(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        Ok(usage)
    }

    fn add_note_graph_module(
        &self,
        graph_id: NoteGraphId,
        module: serde_json::Value,
        description: Option<String>,
    ) -> Result<NoteModuleId, McpBridgeError> {
        let description = validated_description(description)?;
        let config = parse_note_module(module)?;
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .note_graph_mut(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        let module_id = graph.next_module_id();
        graph
            .try_insert_node(module_id, config)
            .map_err(|e| McpBridgeError::Other(e.to_string()))?;
        if !description.is_empty() {
            graph.node_descriptions.insert(module_id, description);
        }
        // A serde-built `NoteScriptTransform` carries only its `source` (the
        // compiled program is `#[serde(skip)]`), so compile it now or the node
        // would be silently pass-through.
        crate::project_apply::recompile_graph_scripts(graph);
        Ok(module_id)
    }

    fn set_note_graph_module(
        &self,
        graph_id: NoteGraphId,
        module_id: NoteModuleId,
        module: serde_json::Value,
        description: Option<String>,
    ) -> Result<(), McpBridgeError> {
        let description = description
            .map(|value| validated_description(Some(value)))
            .transpose()?;
        let config = parse_note_module(module)?;
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .note_graph_mut(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        let mid = module_id;
        if !graph.nodes.contains_key(&mid) {
            return Err(McpBridgeError::NoteGraphModuleNotFound {
                graph_id,
                module_id,
            });
        }
        // Replacing keeps the id and its connections; validation rolls back on
        // an edit that would orphan an existing edge.
        graph
            .try_insert_node(mid, config)
            .map_err(|e| McpBridgeError::Other(e.to_string()))?;
        if let Some(description) = description {
            if description.is_empty() {
                graph.node_descriptions.remove(&mid);
            } else {
                graph.node_descriptions.insert(mid, description);
            }
        }
        // Compile a replaced `NoteScriptTransform`'s program (serde drops it).
        crate::project_apply::recompile_graph_scripts(graph);
        Ok(())
    }

    fn set_note_graph_metadata(
        &self,
        graph_id: NoteGraphId,
        name: Option<String>,
        description: Option<String>,
        color: Option<String>,
    ) -> Result<(), McpBridgeError> {
        let description = description
            .map(|value| validated_description(Some(value)))
            .transpose()?;
        if name.as_ref().is_some_and(|value| value.trim().is_empty()) {
            return Err(McpBridgeError::EmptyName { kind: "note graph" });
        }
        let color = color
            .map(|hex| {
                if hex.is_empty() {
                    Ok(None)
                } else {
                    synth_sequencer::TrackColor::from_hex(&hex)
                        .map(Some)
                        .ok_or_else(|| {
                            McpBridgeError::Other(format!(
                                "invalid color '{hex}' (expected #rrggbb or empty to clear)"
                            ))
                        })
                }
            })
            .transpose()?;
        let mut song = self.shared.song.write();
        let graph = song
            .note_graph_mut(graph_id)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        if let Some(name) = name {
            graph.name = name;
        }
        if let Some(description) = description {
            graph.description = description;
        }
        if let Some(color) = color {
            graph.color = color;
        }
        Ok(())
    }

    fn set_note_graph_script(
        &self,
        graph_id: NoteGraphId,
        module_id: NoteModuleId,
        source: String,
    ) -> Result<String, McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .note_graph_mut(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        let mid = module_id;
        let node = graph
            .nodes
            .get_mut(&mid)
            .ok_or(McpBridgeError::NoteGraphModuleNotFound {
                graph_id,
                module_id,
            })?;
        let synth_sequencer::NoteModuleConfig::NoteScriptTransform(transform) = node else {
            return Err(McpBridgeError::Other(format!(
                "module {module_id} on graph {graph_id} is not a NoteScriptTransform"
            )));
        };
        // The source is always persisted; the compile result only decides whether
        // a program is installed (empty / failing sources are valid pass-through).
        transform.source = source;
        if transform.source().trim().is_empty() {
            transform.set_compiled(None);
            return Ok(format!(
                "Script cleared on module {module_id} (graph {graph_id}) — passes notes through"
            ));
        }
        match crate::session::compile_note_event_script(transform.source()) {
            Ok(program) => {
                transform.set_compiled(Some(program));
                Ok(format!(
                    "Script compiled and installed on module {module_id} (graph {graph_id})"
                ))
            }
            Err(e) => {
                transform.set_compiled(None);
                Ok(format!(
                    "Script saved on module {module_id} (graph {graph_id}) but did not compile \
                     (passes notes through): {e}"
                ))
            }
        }
    }

    fn remove_note_graph_module(
        &self,
        graph_id: NoteGraphId,
        module_id: NoteModuleId,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .note_graph_mut(gid)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        let mid = module_id;
        graph
            .remove_node(mid)
            .map(|_| ())
            .ok_or(McpBridgeError::NoteGraphModuleNotFound {
                graph_id,
                module_id,
            })
    }

    fn connect_note_graph(
        &self,
        graph_id: NoteGraphId,
        from: NoteModuleId,
        to: NoteModuleId,
        port: String,
        to_input: u8,
    ) -> Result<(), McpBridgeError> {
        let port = parse_note_port(&port)?;
        let mut song = self.shared.song.write();
        let graph = song
            .note_graph_mut(graph_id)
            .ok_or(McpBridgeError::NoteGraphNotFound(graph_id))?;
        let connection = synth_sequencer::NoteConnection {
            from,
            to,
            port,
            to_input,
        };
        graph
            .try_connect(connection)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn set_pattern_note_graph(
        &self,
        pattern_id: PatternId,
        graph_id: Option<NoteGraphId>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        // Reject a dangling reference up-front so binding is never silently dry.
        if let Some(id) = graph_id {
            let gid = id;
            if song.note_graph(gid).is_none() {
                return Err(McpBridgeError::NoteGraphNotFound(id));
            }
        }
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.set_note_graph(graph_id);
        Ok(())
    }

    fn set_note_note_graph(
        &self,
        pattern_id: PatternId,
        note_id: NoteId,
        graph_id: Option<NoteGraphId>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        // Reject a dangling reference up-front so the binding is never silently dry.
        if let Some(id) = graph_id {
            let gid = id;
            if song.note_graph(gid).is_none() {
                return Err(McpBridgeError::NoteGraphNotFound(id));
            }
        }
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let note = pattern
            .note_mut(note_id)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;
        note.note_graph = graph_id;
        Ok(())
    }

    // === Sequencer: Mod Grid (pooled control-rate modulator graphs) ===

    fn list_mod_graphs(&self) -> Result<Vec<ModGraphInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        Ok(song.mod_graphs().map(mod_graph_info).collect())
    }

    fn get_mod_graph(&self, graph_id: ModGraphId) -> Result<ModGraphDetail, McpBridgeError> {
        let song = self.shared.song.read();
        let gid = graph_id;
        let graph = song
            .mod_graph(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        let nodes = graph
            .nodes
            .iter()
            .map(|(id, cfg)| {
                Ok(ModGraphNodeInfo {
                    id: *id,
                    kind: cfg.kind().to_string(),
                    description: graph.node_descriptions.get(id).cloned().unwrap_or_default(),
                    config: serde_json::to_value(cfg)
                        .map_err(|e| McpBridgeError::Other(e.to_string()))?,
                })
            })
            .collect::<Result<Vec<_>, McpBridgeError>>()?;
        let connections = graph
            .connections
            .iter()
            .map(|c| ModGraphConnectionInfo {
                from: c.from,
                from_port: c.from_port.clone(),
                to: c.to,
                to_port: c.to_port.clone(),
            })
            .collect();
        Ok(ModGraphDetail {
            info: mod_graph_info(graph),
            nodes,
            connections,
        })
    }

    fn create_mod_graph(
        &self,
        name: String,
        description: Option<String>,
        color: Option<String>,
        scope: Option<String>,
    ) -> Result<ModGraphId, McpBridgeError> {
        if name.trim().is_empty() {
            return Err(McpBridgeError::EmptyName { kind: "mod graph" });
        }
        let description = description.unwrap_or_default();
        let len = description.chars().count();
        if len > MAX_MODULE_DESCRIPTION_LEN {
            return Err(McpBridgeError::DescriptionTooLong {
                len,
                max: MAX_MODULE_DESCRIPTION_LEN,
            });
        }
        let scope = parse_mod_graph_scope(scope.as_deref())?;
        let color = color
            .map(|hex| {
                synth_sequencer::TrackColor::from_hex(&hex).ok_or_else(|| {
                    McpBridgeError::Other(format!("invalid color '{hex}' (expected #rrggbb)"))
                })
            })
            .transpose()?;
        let mut song = self.shared.song.write();
        let gid = song.create_mod_graph(name);
        song.set_mod_graph_scope(gid, scope);
        if let Some(graph) = song.mod_graph_mut(gid) {
            graph.description = description;
            graph.color = color;
        }
        Ok(gid)
    }

    fn duplicate_mod_graph(&self, graph_id: ModGraphId) -> Result<ModGraphId, McpBridgeError> {
        let mut song = self.shared.song.write();
        song.duplicate_mod_graph(graph_id)
            .map(|graph| graph.id)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))
    }

    fn delete_mod_graph(&self, graph_id: ModGraphId) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        song.remove_mod_graph(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        Ok(())
    }

    fn set_mod_graph_scope(
        &self,
        graph_id: ModGraphId,
        scope: String,
    ) -> Result<(), McpBridgeError> {
        let scope = parse_mod_graph_scope(Some(scope.as_str()))?;
        let mut song = self.shared.song.write();
        let gid = graph_id;
        if song.set_mod_graph_scope(gid, scope) {
            Ok(())
        } else {
            Err(McpBridgeError::ModGraphNotFound(graph_id))
        }
    }

    fn assign_mod_graph(
        &self,
        graph_id: ModGraphId,
        tracks: Vec<TrackId>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        if let Some(track_id) = tracks
            .iter()
            .copied()
            .find(|track_id| song.track(*track_id).is_none())
        {
            return Err(McpBridgeError::TrackNotFound(track_id));
        }
        let gid = graph_id;
        if song.assign_mod_graph(gid, &tracks) {
            Ok(())
        } else {
            Err(McpBridgeError::ModGraphNotFound(graph_id))
        }
    }

    fn add_mod_graph_node(
        &self,
        graph_id: ModGraphId,
        node: serde_json::Value,
        description: Option<String>,
    ) -> Result<ModNodeId, McpBridgeError> {
        let description = validated_description(description)?;
        let config = parse_mod_node(node)?;
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .mod_graph_mut(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        let node_id = graph.next_node_id();
        graph
            .try_insert_node(node_id, config)
            .map_err(|e| McpBridgeError::Other(e.to_string()))?;
        if !description.is_empty() {
            graph.node_descriptions.insert(node_id, description);
        }
        Ok(node_id)
    }

    fn set_mod_graph_metadata(
        &self,
        graph_id: ModGraphId,
        name: Option<String>,
        description: Option<String>,
        color: Option<String>,
    ) -> Result<(), McpBridgeError> {
        let description = description
            .map(|value| validated_description(Some(value)))
            .transpose()?;
        if name.as_ref().is_some_and(|value| value.trim().is_empty()) {
            return Err(McpBridgeError::EmptyName { kind: "mod graph" });
        }
        let color = color
            .map(|hex| {
                if hex.is_empty() {
                    Ok(None)
                } else {
                    synth_sequencer::TrackColor::from_hex(&hex)
                        .map(Some)
                        .ok_or_else(|| {
                            McpBridgeError::Other(format!(
                                "invalid color '{hex}' (expected #rrggbb or empty to clear)"
                            ))
                        })
                }
            })
            .transpose()?;
        let mut song = self.shared.song.write();
        let graph = song
            .mod_graph_mut(graph_id)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        if let Some(name) = name {
            graph.name = name;
        }
        if let Some(description) = description {
            graph.description = description;
        }
        if let Some(color) = color {
            graph.color = color;
        }
        Ok(())
    }

    fn remove_mod_graph_node(
        &self,
        graph_id: ModGraphId,
        node_id: ModNodeId,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .mod_graph_mut(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        graph
            .remove_node(node_id)
            .ok_or(McpBridgeError::ModGraphNodeNotFound { graph_id, node_id })?;
        Ok(())
    }

    fn connect_mod_graph(
        &self,
        graph_id: ModGraphId,
        from: ModNodeId,
        from_port: String,
        to: ModNodeId,
        to_port: String,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .mod_graph_mut(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        validate_mod_connection_ports(graph, from, &from_port, to, &to_port)?;
        let cable = synth_sequencer::ModConnection::new(from, from_port, to, to_port);
        graph
            .try_connect(cable)
            .map_err(|e| McpBridgeError::Other(e.to_string()))
    }

    fn disconnect_mod_graph(
        &self,
        graph_id: ModGraphId,
        from: ModNodeId,
        from_port: String,
        to: ModNodeId,
        to_port: String,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .mod_graph_mut(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        let cable = synth_sequencer::ModConnection::new(from, from_port, to, to_port);
        if graph.disconnect(&cable) {
            Ok(())
        } else {
            Err(McpBridgeError::Other(format!(
                "no cable {from}.{} → {to}.{} in graph {graph_id}",
                cable.from_port, cable.to_port
            )))
        }
    }

    fn set_mod_graph_node(
        &self,
        graph_id: ModGraphId,
        node_id: ModNodeId,
        node: serde_json::Value,
        description: Option<String>,
    ) -> Result<(), McpBridgeError> {
        let description = description
            .map(|value| validated_description(Some(value)))
            .transpose()?;
        let config = parse_mod_node(node)?;
        let mut song = self.shared.song.write();
        let gid = graph_id;
        let graph = song
            .mod_graph_mut(gid)
            .ok_or(McpBridgeError::ModGraphNotFound(graph_id))?;
        let nid = node_id;
        // Edit-in-place, not create: the node must already exist.
        if !graph.nodes.contains_key(&nid) {
            return Err(McpBridgeError::ModGraphNodeNotFound { graph_id, node_id });
        }
        // `try_insert_node` replaces the config, keeps the id and its cables, and
        // re-validates (rolling back on rejection).
        let mut candidate = graph.clone();
        candidate
            .try_insert_node(nid, config)
            .map_err(|e| McpBridgeError::Other(e.to_string()))?;
        for cable in &candidate.connections {
            validate_mod_connection_ports(
                &candidate,
                cable.from,
                &cable.from_port,
                cable.to,
                &cable.to_port,
            )?;
        }
        *graph = candidate;
        if let Some(description) = description {
            if description.is_empty() {
                graph.node_descriptions.remove(&nid);
            } else {
                graph.node_descriptions.insert(nid, description);
            }
        }
        Ok(())
    }

    fn list_mod_targets(
        &self,
        graph_id: Option<ModGraphId>,
    ) -> Result<Vec<ModTargetInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        let mut out = Vec::new();
        for graph in song.mod_graphs() {
            if let Some(want) = graph_id
                && graph.id != want
            {
                continue;
            }
            for (node_id, cfg) in &graph.nodes {
                if let synth_sequencer::ModNodeConfig::Target(t) = cfg {
                    out.push(ModTargetInfo {
                        graph_id: graph.id,
                        graph_name: graph.name.clone(),
                        node_id: *node_id,
                        target: t.target.display_name(),
                        amount: t.amount,
                    });
                }
            }
        }
        Ok(out)
    }

    fn set_note_ornament(
        &self,
        pattern_id: PatternId,
        note_id: NoteId,
        ornament: Option<serde_json::Value>,
    ) -> Result<(), McpBridgeError> {
        // null / omitted clears the ornament; otherwise parse the Ornament.
        let parsed: Option<synth_sequencer::Ornament> = match ornament {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                serde_json::from_value(value)
                    .map_err(|e| McpBridgeError::Other(format!("invalid ornament: {e}")))?,
            ),
        };
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let note = pattern
            .note_mut(note_id)
            .ok_or(McpBridgeError::NoteNotFound(note_id))?;
        note.ornament = parsed;
        Ok(())
    }

    // === Sequencer: Tracks ===

    fn list_tracks(&self) -> Result<Vec<TrackInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        let mut tracks: Vec<TrackInfo> = song
            .tracks()
            .map(|t| TrackInfo {
                id: t.id,
                name: t.name.clone(),
                description: t.description.clone(),
                color: t.color.to_hex(),
                instrument_id: Some(t.instrument),
                volume: t.volume,
                // Convert normalized (0.0..1.0) to bipolar (-1.0..1.0) for MCP API
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
                sends: t
                    .sends
                    .iter()
                    .map(|s| synth_mcp::SendInfo {
                        target: s.target,
                        level: s.level,
                        pre_fader: s.pre_fader,
                        enabled: s.enabled,
                    })
                    .collect(),
            })
            .collect();
        tracks.sort_by_key(|t| t.id);
        Ok(tracks)
    }

    fn create_track(
        &self,
        name: &str,
        instrument_id: Option<InstrumentId>,
    ) -> Result<TrackId, McpBridgeError> {
        let mut song = self.shared.song.write();
        let id = song.create_track(name);
        if let Some(inst_id) = instrument_id
            && let Some(track) = song.track_mut(id)
        {
            track.instrument = inst_id;
        }
        Ok(id)
    }

    // === Sequencer: Arrangement ===

    fn place_pattern(&self, data: &BridgePlacementData) -> Result<(), McpBridgeError> {
        let pid = data.pattern_id;
        let tid = data.track_id;
        let tick = data.start.tick();

        let placement_end = {
            let mut song = self.shared.song.write();
            let pattern_length = song
                .pattern(pid)
                .ok_or(McpBridgeError::PatternNotFound(data.pattern_id))?
                .length;
            if song.track(tid).is_none() {
                return Err(McpBridgeError::TrackNotFound(data.track_id));
            }
            if song
                .arrangement()
                .iter()
                .any(|placement| placement.track_id == tid && placement.start == tick)
            {
                return Err(McpBridgeError::Other(format!(
                    "placement already exists on track {} at tick {}",
                    data.track_id,
                    data.start.tick()
                )));
            }
            let placement = placement_from_bridge(data);
            let placement_end = placement.end(pattern_length);
            if !song.insert_placement(placement) {
                return Err(McpBridgeError::Other(
                    "placement could not be inserted".to_string(),
                ));
            }
            placement_end
        };

        self.auto_extend_transport_loop(placement_end);
        Ok(())
    }

    fn remove_placement(
        &self,
        pattern_id: PatternId,
        track_id: TrackId,
        start_tick: Tick,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let tid = track_id;
        let tick = start_tick;
        if song.remove_placement(pid, tid, tick) {
            Ok(())
        } else {
            Err(McpBridgeError::Other(format!(
                "placement not found: pattern {pattern_id}, track {track_id}, tick {}",
                tick.0
            )))
        }
    }

    fn list_arrangement(&self) -> Result<Vec<PlacementInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        Ok(song
            .arrangement()
            .iter()
            .filter_map(|p| {
                let pattern_length = song.pattern(p.pattern_id)?.length;
                let effective_length = p.effective_length(pattern_length);
                Some(PlacementInfo {
                    pattern_id: p.pattern_id,
                    track_id: p.track_id,
                    start_beat: ticks_to_beats_u64(p.start.0),
                    start_tick: p.start,
                    transpose_semitones: p.transpose.as_f32(),
                    gain: p.gain.as_f32(),
                    length_beats: p.length_override.map(|length| ticks_to_beats(length.0)),
                    length_ticks: p.length_override.map(|length| length.0),
                    loop_mode: p.loop_mode,
                    effective_length_beats: ticks_to_beats(effective_length.0),
                    effective_length_ticks: effective_length.0,
                    end_tick: p.end(pattern_length),
                })
            })
            .collect())
    }

    // === Sequencer: Batch operations ===

    fn add_notes(
        &self,
        pattern_id: PatternId,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            match try_insert_note_into_pattern(pattern, n) {
                Ok(note_id) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: Some(note_id),
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(err) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(err),
                    });
                }
            }
        }

        Ok(BatchResult {
            total: notes.len(),
            succeeded,
            failed: notes.len() - succeeded,
            items,
        })
    }

    fn update_notes(
        &self,
        pattern_id: PatternId,
        updates: &[BridgeNoteUpdate],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut items = Vec::with_capacity(updates.len());
        let mut succeeded = 0usize;

        for (i, u) in updates.iter().enumerate() {
            let nid = u.note_id;
            if let Some(note) = pattern.note_mut(nid) {
                if let Some(p) = u.pitch {
                    if let Some(new_pitch) = synth_sequencer::Pitch::new(p.as_u8()) {
                        note.pitch = new_pitch;
                    } else {
                        items.push(BatchItemResult {
                            index: i,
                            success: false,
                            id: None,
                            error: Some(format!("invalid pitch value: {p}")),
                        });
                        continue;
                    }
                }
                if let Some(s) = u.start_beat {
                    note.start = synth_sequencer::PatternTick(beats_to_ticks(s));
                }
                if let Some(d) = u.duration_beats {
                    note.duration = Some(synth_sequencer::Duration(beats_to_ticks(d)));
                }
                if let Some(v) = u.velocity {
                    note.velocity = synth_core::Velocity::from_midi(v);
                }
                items.push(BatchItemResult {
                    index: i,
                    success: true,
                    id: None,
                    error: None,
                });
                succeeded += 1;
            } else {
                items.push(BatchItemResult {
                    index: i,
                    success: false,
                    id: None,
                    error: Some(format!("note not found: {}", u.note_id)),
                });
            }
        }

        Ok(BatchResult {
            total: updates.len(),
            succeeded,
            failed: updates.len() - succeeded,
            items,
        })
    }

    fn replace_notes(
        &self,
        pattern_id: PatternId,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        pattern.clear_notes();

        let mut items = Vec::with_capacity(notes.len());
        let mut succeeded = 0usize;

        for (i, n) in notes.iter().enumerate() {
            match try_insert_note_into_pattern(pattern, n) {
                Ok(note_id) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: Some(note_id),
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(err) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(err),
                    });
                }
            }
        }

        Ok(BatchResult {
            total: notes.len(),
            succeeded,
            failed: notes.len() - succeeded,
            items,
        })
    }

    fn clear_pattern(&self, pattern_id: PatternId) -> Result<usize, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        let count = pattern.note_count();
        pattern.clear_notes();
        Ok(count)
    }

    fn create_patterns(
        &self,
        patterns: &[BridgePatternData],
    ) -> Result<BatchResult, McpBridgeError> {
        // Validate module automation targets against the real graphs; build the
        // cache before taking the song lock.
        let module_cache = self.module_id_cache(
            patterns
                .iter()
                .flat_map(|p| p.automation.iter().map(|pt| pt.instrument_id)),
        );

        let mut song = self.shared.song.write();

        let mut items = Vec::with_capacity(patterns.len());
        let mut succeeded = 0usize;

        for (i, p) in patterns.iter().enumerate() {
            let duration = synth_sequencer::Duration(beats_to_ticks(p.length_beats));
            let id = song.create_pattern(duration);
            let mut skipped_automation = 0usize;
            if let Some(pattern) = song.pattern_mut(id) {
                pattern.name = p.name.clone();
                for n in &p.notes {
                    insert_note_into_pattern(pattern, n);
                }
                skipped_automation =
                    insert_automation_into_pattern(pattern, &p.automation, &module_cache);
            }
            // The pattern is created either way; surface skipped automation as a
            // warning rather than dropping it silently.
            let error = (skipped_automation > 0).then(|| {
                format!(
                    "{skipped_automation} automation point(s) skipped (unknown or invalid target)"
                )
            });
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(u64::from(id.0)),
                error,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: patterns.len(),
            succeeded,
            failed: patterns.len() - succeeded,
            items,
        })
    }

    fn create_tracks(&self, tracks: &[BridgeTrackData]) -> Result<BatchResult, McpBridgeError> {
        let mut song = self.shared.song.write();

        let mut items = Vec::with_capacity(tracks.len());
        let mut succeeded = 0usize;

        for (i, t) in tracks.iter().enumerate() {
            let id = song.create_track(&t.name);
            if let Some(inst_id) = t.instrument_id
                && let Some(track) = song.track_mut(id)
            {
                track.instrument = inst_id;
            }
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: Some(u64::from(id.0)),
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total: tracks.len(),
            succeeded,
            failed: tracks.len() - succeeded,
            items,
        })
    }

    fn place_patterns(
        &self,
        placements: &[BridgePlacementData],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut items = Vec::with_capacity(placements.len());
        let mut succeeded = 0usize;
        let mut max_end = synth_sequencer::Tick::ZERO;

        {
            let mut song = self.shared.song.write();

            for (i, p) in placements.iter().enumerate() {
                let pid = p.pattern_id;
                let tid = p.track_id;
                let tick = p.start.tick();

                let Some(pattern_length) = song.pattern(pid).map(|p| p.length) else {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("pattern not found: {}", p.pattern_id)),
                    });
                    continue;
                };
                if song.track(tid).is_none() {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("track not found: {}", p.track_id)),
                    });
                    continue;
                }

                if song
                    .arrangement()
                    .iter()
                    .any(|placement| placement.track_id == tid && placement.start == tick)
                {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!(
                            "placement already exists on track {} at tick {}",
                            p.track_id,
                            p.start.tick()
                        )),
                    });
                    continue;
                }
                let placement = placement_from_bridge(p);
                let placement_end = placement.end(pattern_length);
                if !song.insert_placement(placement) {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some("placement could not be inserted".to_string()),
                    });
                    continue;
                }
                if placement_end.0 > max_end.0 {
                    max_end = placement_end;
                }
                items.push(BatchItemResult {
                    index: i,
                    success: true,
                    id: None,
                    error: None,
                });
                succeeded += 1;
            }
        }

        if succeeded > 0 {
            self.auto_extend_transport_loop(max_end);
        }

        Ok(BatchResult {
            total: placements.len(),
            succeeded,
            failed: placements.len() - succeeded,
            items,
        })
    }

    fn update_placements(
        &self,
        updates: &[BridgePlacementUpdate],
    ) -> Result<BatchResult, McpBridgeError> {
        let mut song = self.shared.song.write();
        let mut items = Vec::with_capacity(updates.len());
        let mut succeeded = 0usize;
        let mut max_end = synth_sequencer::Tick::ZERO;

        for (index, update) in updates.iter().enumerate() {
            let pattern_id = update.pattern_id;
            let track_id = update.track_id;
            let start = update.start.tick();
            let Some(mut replacement) = song
                .arrangement()
                .iter()
                .find(|placement| {
                    placement.pattern_id == pattern_id
                        && placement.track_id == track_id
                        && placement.start == start
                })
                .cloned()
            else {
                items.push(BatchItemResult {
                    index,
                    success: false,
                    id: None,
                    error: Some("placement not found".to_string()),
                });
                continue;
            };
            if let Some(new_track_id) = update.new_track_id {
                replacement.track_id = new_track_id;
            }
            if let Some(new_start) = update.new_start {
                replacement.start = new_start.tick();
            }
            if let Some(transpose) = update.transpose_semitones {
                replacement.transpose = synth_core::Semitones::new(transpose);
            }
            if let Some(gain) = update.gain {
                replacement.gain = synth_core::Gain::new(gain);
            }
            if let Some(length_ticks) = update.length_ticks {
                replacement.length_override = length_ticks.map(synth_sequencer::Duration);
            }
            if let Some(loop_mode) = update.loop_mode {
                replacement.loop_mode = loop_mode;
            }
            let replacement_end = song
                .pattern(replacement.pattern_id)
                .map(|pattern| replacement.end(pattern.length));
            if song.update_placement(pattern_id, track_id, start, replacement) {
                succeeded += 1;
                if let Some(end) = replacement_end
                    && end.0 > max_end.0
                {
                    max_end = end;
                }
                items.push(BatchItemResult {
                    index,
                    success: true,
                    id: None,
                    error: None,
                });
            } else {
                items.push(BatchItemResult {
                    index,
                    success: false,
                    id: None,
                    error: Some("invalid target track or occupied target position".to_string()),
                });
            }
        }
        drop(song);
        if succeeded > 0 {
            self.auto_extend_transport_loop(max_end);
        }
        Ok(BatchResult {
            total: updates.len(),
            succeeded,
            failed: updates.len() - succeeded,
            items,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn set_song(
        &self,
        name: &str,
        tempo: f32,
        patterns: &[BridgePatternData],
        tracks: &[BridgeTrackData],
        placements: &[BridgeSongPlacement],
    ) -> Result<SetSongResult, McpBridgeError> {
        // Validate module automation targets against the real graphs; build the
        // cache before taking the song lock.
        let module_cache = self.module_id_cache(
            patterns
                .iter()
                .flat_map(|p| p.automation.iter().map(|pt| pt.instrument_id)),
        );

        let mut song = self.shared.song.write();

        // Replace the entire song
        *song = synth_sequencer::Song::new(name).with_tempo(synth_core::Bpm::new(tempo));

        let mut errors = Vec::new();
        let mut total_notes = 0usize;

        // Create patterns with notes
        let mut pattern_ids: Vec<PatternId> = Vec::with_capacity(patterns.len());
        for (i, p) in patterns.iter().enumerate() {
            let duration = synth_sequencer::Duration(beats_to_ticks(p.length_beats));
            let id = song.create_pattern(duration);
            if let Some(pattern) = song.pattern_mut(id) {
                pattern.name = p.name.clone();
                for n in &p.notes {
                    insert_note_into_pattern(pattern, n);
                    total_notes += 1;
                }
                let skipped = insert_automation_into_pattern(pattern, &p.automation, &module_cache);
                if skipped > 0 {
                    errors.push(format!(
                        "pattern[{i}] '{}': {skipped} automation point(s) skipped (unknown or invalid target)",
                        p.name
                    ));
                }
            } else {
                errors.push(format!("failed to access pattern[{i}] after creation"));
            }
            pattern_ids.push(id);
        }

        // Create tracks
        let mut track_ids: Vec<TrackId> = Vec::with_capacity(tracks.len());
        for t in tracks {
            let id = song.create_track(&t.name);
            if let Some(inst_id) = t.instrument_id
                && let Some(track) = song.track_mut(id)
            {
                track.instrument = inst_id;
            }
            track_ids.push(id);
        }

        // Create arrangement placements (index-based → real IDs)
        let mut placements_created = 0usize;
        for (i, pl) in placements.iter().enumerate() {
            if pl.pattern_index >= pattern_ids.len() {
                errors.push(format!(
                    "placement[{i}]: pattern_index {} out of range (have {})",
                    pl.pattern_index,
                    pattern_ids.len()
                ));
                continue;
            }
            if pl.track_index >= track_ids.len() {
                errors.push(format!(
                    "placement[{i}]: track_index {} out of range (have {})",
                    pl.track_index,
                    track_ids.len()
                ));
                continue;
            }

            let pid = pattern_ids[pl.pattern_index];
            let tid = track_ids[pl.track_index];
            let data = BridgePlacementData {
                pattern_id: pid,
                track_id: tid,
                start: pl.start,
                transpose_semitones: pl.transpose_semitones,
                gain: pl.gain,
                length_ticks: pl.length_ticks,
                loop_mode: pl.loop_mode,
            };
            let placement = placement_from_bridge(&data);
            if song.arrangement().iter().any(|existing| {
                existing.track_id == placement.track_id && existing.start == placement.start
            }) {
                errors.push(format!(
                    "placement[{i}]: target track {} tick {} is occupied",
                    tid.0,
                    pl.start.tick()
                ));
            } else if song.insert_placement(placement) {
                placements_created += 1;
            } else {
                errors.push(format!("placement[{i}]: placement could not be inserted"));
            }
        }

        // Also update engine transport tempo
        drop(song);
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetTempo(synth_core::Bpm::new(tempo)))
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetTempo",
            });
        }

        Ok(SetSongResult {
            patterns_created: pattern_ids.len(),
            tracks_created: track_ids.len(),
            notes_added: total_notes,
            placements_created,
            pattern_ids,
            track_ids,
            errors,
        })
    }

    // === Sequencer: Transport ===

    fn seq_play(&self) -> Result<(), McpBridgeError> {
        if self.session.command_sender().send(EngineCommand::Play) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "play" })
        }
    }

    fn seq_stop(&self) -> Result<(), McpBridgeError> {
        if self.session.command_sender().send(EngineCommand::Stop) {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "stop" })
        }
    }

    fn seq_seek(&self, beat: f32) -> Result<(), McpBridgeError> {
        let tick = synth_sequencer::Tick(u64::from(beats_to_ticks(beat)));
        if self
            .session
            .command_sender()
            .send(EngineCommand::Seek { tick })
        {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed { command: "seek" })
        }
    }
}
