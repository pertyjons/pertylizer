//! project flow responsibilities for the egui application.

use super::*;

impl SynthApp {
    /// Resolve the initial directory for the Open file dialog.
    ///
    /// Priority: last open dir > custom patches dir > default patches dir.
    pub(super) fn resolve_open_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_open_dir
            .clone()
            .or_else(|| self.settings.directories.patches_dir.clone())
            .or_else(|| PatchManager::default_patches_dir().ok())
    }

    /// Resolve the initial directory for the Save file dialog.
    ///
    /// Priority: last save dir > custom patches dir > default patches dir.
    pub(super) fn resolve_save_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_save_dir
            .clone()
            .or_else(|| self.settings.directories.patches_dir.clone())
            .or_else(|| PatchManager::default_patches_dir().ok())
    }

    /// Resolve the initial directory for the Group Template file dialog.
    pub(super) fn resolve_group_templates_dir(&self) -> Option<PathBuf> {
        GroupTemplateManager::default_templates_dir().ok()
    }

    /// Resolve a group template source to the actual template data.
    pub(super) fn resolve_group_template(
        manager: &GroupTemplateManager,
        source: &crate::io::GroupTemplateSource,
    ) -> Result<crate::patch::GroupTemplate, crate::patch::PatchError> {
        match source {
            crate::io::GroupTemplateSource::BuiltIn(index) => {
                let all = crate::group_templates::builtin_group_templates();
                all.into_iter().nth(*index).ok_or_else(|| {
                    crate::patch::PatchError::Io(format!(
                        "Built-in template index {index} out of range"
                    ))
                })
            }
            crate::io::GroupTemplateSource::File(path) => manager.load_template(path),
        }
    }

    /// Insert a group template at the last remembered drop position.
    pub(super) fn insert_group_template(&mut self, template: &crate::patch::GroupTemplate) {
        let drop_pos = self
            .dialog_state
            .group_template_drop_pos
            .take()
            .unwrap_or(Pos2::new(100.0, 100.0));
        let Some(active_id) = self.active_instrument_id else {
            return;
        };
        let session = self.session.clone();
        let (handle, instruments) = (&mut self.handle, &mut self.instruments);
        if let Some(editor) = instruments
            .iter_mut()
            .find(|i| i.id == active_id)
            .map(|i| &mut i.patch_editor)
        {
            match patch_bridge::insert_group_template(
                template, drop_pos, editor, &session, handle, active_id,
            ) {
                Ok(_) => {
                    self.dialog_state
                        .set_status(format!("Inserted template: {}", template.name));
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Template insert failed: {e}"));
                }
            }
        }
    }

    /// Load a patch into the active instrument's rack view.
    /// If no instrument is active, one is auto-created so the user can load a
    /// patch into a fresh project without first clicking "+ New Instrument".
    pub(super) fn load_patch_data(&mut self, patch: &Patch) {
        self.mark_dirty();
        // A loaded patch belongs in the rack — don't strand the user on Home.
        if self.active_view == AppView::Home {
            self.active_view = AppView::Rack;
        }

        // Delegate to patch_bridge for the main loading logic
        // Load into the active instrument's patch editor (auto-create if none)
        let active_id = match self.active_instrument_id {
            Some(id) => id,
            None => match self.add_new_instrument() {
                Some(id) => id,
                None => {
                    self.dialog_state
                        .set_status("Cannot load patch: failed to create instrument".to_string());
                    return;
                }
            },
        };
        let Some(instrument) = self.instruments.iter_mut().find(|i| i.id == active_id) else {
            self.dialog_state
                .set_status("Cannot load patch: active instrument missing".to_string());
            return;
        };

        // Clear visualization buffers only for the active instrument's modules
        for module_id in instrument.patch_editor.module_ids() {
            self.handle.visualization_buffers.remove(&module_id);
        }

        // Update instrument name to match the loaded patch (both UI and engine)
        instrument.name = patch.name.clone();
        self.handle.send(EngineCommand::RenameInstrument {
            instrument_id: active_id,
            name: patch.name.clone(),
        });

        patch_bridge::load_patch(
            patch,
            &mut instrument.patch_editor,
            &self.session,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.glide_time,
            active_id,
            false,
        );

        // Restore canvas size hint so the scroll area matches the original layout
        if let Some(cs) = patch.settings.canvas_size {
            instrument
                .patch_editor
                .set_min_canvas_size(eframe::egui::Vec2::new(cs.width, cs.height));
        }

        tracing::info!(target: "pertylizer::patch", "loaded patch '{}'", patch.name);
    }

    /// Reconcile GUI state with session: detect modules added/removed by MCP.
    ///
    /// Loops over ALL instruments (not just the active one) so that modules
    /// and connections created by MCP on any instrument appear immediately
    /// when the user switches to that instrument.
    #[cfg(feature = "mcp")]
    pub(super) fn reconcile_with_session(&mut self) {
        self.reconcile_instruments();

        // Engine-side graph version bumps on any mutation (module add/remove,
        // connection add/remove, parameter set). If it hasn't moved since last
        // frame, nothing observable has changed — skip the clone-every-module
        // sync.
        let graph_version = self.session.state().shared_graph.version();
        if !self.mcp_sync.graph.observe(graph_version) {
            return;
        }

        let instrument_ids: Vec<InstrumentId> = self.instruments.iter().map(|i| i.id).collect();

        for inst_id in instrument_ids {
            let session_modules = self.session.all_modules_for_instrument(inst_id);
            let engine_connections = self
                .session
                .state()
                .shared_graph
                .get_connections_for_instrument(inst_id);

            let Some(patch_editor) = self
                .instruments
                .iter_mut()
                .find(|i| i.id == inst_id)
                .map(|i| &mut i.patch_editor)
            else {
                continue;
            };

            let editor_ids: std::collections::HashSet<ModuleId> =
                patch_editor.module_ids().into_iter().collect();
            let session_ids: std::collections::HashSet<ModuleId> =
                session_modules.keys().copied().collect();

            // Modules added by MCP (in session but not in editor)
            let to_add: Vec<(ModuleId, synth_core::ModuleDescriptor)> = session_ids
                .difference(&editor_ids)
                .filter_map(|id| session_modules.get(id).map(|d| (*id, d.clone())))
                .collect();

            // Modules removed by MCP (in editor but not in session)
            let to_remove: Vec<ModuleId> = editor_ids
                .difference(&session_ids)
                .copied()
                // Skip visualizers — they are GUI-only and not tracked by session
                .filter(|id| {
                    !patch_editor
                        .module_descriptor(*id)
                        .is_some_and(|d| d.category == synth_core::ModuleCategory::Visualizer)
                })
                .collect();

            // Check connections too
            let editor_connections = patch_editor.connections().to_vec();
            let new_connections: Vec<synth_engine::graph::Connection> = engine_connections
                .iter()
                .filter_map(|snap| {
                    let conn = synth_engine::graph::Connection::new(
                        snap.from_module,
                        snap.from_port,
                        snap.to_module,
                        snap.to_port,
                    );
                    if !editor_connections.contains(&conn) {
                        Some(conn)
                    } else {
                        None
                    }
                })
                .collect();

            let module_snapshots = self
                .session
                .state()
                .shared_graph
                .get_modules_for_instrument(inst_id);

            // Mirror engine parameters into existing panels so MCP-driven
            // `set_parameter` reaches GUI dropdowns/knobs.
            for snap in &module_snapshots {
                if editor_ids.contains(&snap.id) {
                    patch_editor.sync_module_params(snap.id, &snap.parameters);
                    patch_editor.sync_module_scripts(snap.id, &snap.scripts);
                    patch_editor.sync_module_description(snap.id, &snap.description);
                }
            }

            if to_add.is_empty() && to_remove.is_empty() && new_connections.is_empty() {
                continue;
            }

            for (module_id, descriptor) in to_add {
                let position = eframe::egui::Pos2::new(100.0, 100.0);
                patch_editor.add_module_at(module_id, descriptor, position);

                if let Some(snap) = module_snapshots.iter().find(|s| s.id == module_id) {
                    // Mirror via the same path as the per-frame sync so the panel
                    // gets BOTH the f32 cache and the address mirror (slot_addrs).
                    // The plain per-param f32 copy used here before left slot_addrs
                    // empty, and the version-gated sync above never re-ran for a
                    // freshly-added module — so a Mod Matrix created via MCP showed
                    // "(none)" in its pickers forever.
                    patch_editor.sync_module_params(module_id, &snap.parameters);
                    patch_editor.sync_module_scripts(module_id, &snap.scripts);
                    patch_editor.sync_module_description(module_id, &snap.description);
                }
            }

            for module_id in to_remove {
                patch_editor.remove_module(module_id);
            }

            for conn in new_connections {
                patch_editor.add_connection(conn);
            }
        }
    }

    /// Reconcile GUI instrument list with engine's `instrument_snapshots`.
    ///
    /// Detects instruments added or removed by MCP and syncs the GUI.
    #[cfg(feature = "mcp")]
    pub(super) fn reconcile_instruments(&mut self) {
        let snapshots = self.session.list_instruments();

        let gui_ids: std::collections::HashSet<InstrumentId> =
            self.instruments.iter().map(|i| i.id).collect();
        let engine_ids: std::collections::HashSet<InstrumentId> =
            snapshots.iter().map(|s| s.id).collect();

        // Instruments added by MCP (in engine but not in GUI)
        for snap in &snapshots {
            if !gui_ids.contains(&snap.id) {
                let channel =
                    snap.midi_channel
                        .map_or(synth_engine::MidiChannelSelection::OMNI, |channel| {
                            synth_engine::MidiChannelSelection::from_zero_indexed(
                                channel.as_index(),
                            )
                            .unwrap_or(synth_engine::MidiChannelSelection::CH1)
                        });
                let mut ui_inst = InstrumentUiState::new(snap.id, &snap.name).with_channel(channel);
                ui_inst.volume = snap.volume;
                ui_inst.pan = snap.pan;
                ui_inst.muted = snap.muted;
                ui_inst.solo = snap.solo;
                self.instruments.push(ui_inst);
            }
        }

        // Remove GUI instruments that the engine no longer has, BUT only if
        // their PatchEditor is empty. During project loading, engine processes
        // AddInstrument commands asynchronously — GUI instruments with loaded
        // modules may not yet appear in shared state. Keeping them prevents
        // reconciliation from destroying loaded PatchEditors.
        self.instruments
            .retain(|i| engine_ids.contains(&i.id) || !i.patch_editor.module_ids().is_empty());

        // If active instrument was removed, switch to first available
        if self
            .active_instrument_id
            .is_some_and(|id| !self.instruments.iter().any(|i| i.id == id))
        {
            self.active_instrument_id = self.instruments.first().map(|i| i.id);
            self.handle
                .set_focused_instrument(self.active_instrument_id);
        }

        // Update metadata for existing instruments from the engine snapshot.
        // MCP is the source of truth for these fields when changed, so we
        // mirror them here so create_project_from_app (which reads from the
        // GUI-side InstrumentUiState) sees the latest values at save time.
        for snap in &snapshots {
            if let Some(ui_inst) = self.instruments.iter_mut().find(|i| i.id == snap.id) {
                ui_inst.name = snap.name.clone();
                if !ui_inst.muted {
                    ui_inst.volume = snap.volume;
                }
                ui_inst.pan = snap.pan;
                ui_inst.muted = snap.muted;
                ui_inst.solo = snap.solo;
                ui_inst.channel =
                    snap.midi_channel
                        .map_or(synth_engine::MidiChannelSelection::OMNI, |channel| {
                            synth_engine::MidiChannelSelection::from_zero_indexed(
                                channel.as_index(),
                            )
                            .unwrap_or(ui_inst.channel)
                        });
                ui_inst.description = snap.description.clone();
                ui_inst.patch_description = snap.patch_description.clone().unwrap_or_default();
                ui_inst.color = snap.color.clone();
                ui_inst.category = snap.category;
                ui_inst.sidechain_source_id = snap.sidechain_source_id;
            }
        }
    }

    /// Write the current UI layout to MCP shared state (called each frame).
    #[cfg(feature = "mcp")]
    pub(super) fn write_mcp_layout(&self, ctx: &egui::Context) {
        let Some(shared) = &self.mcp_shared else {
            return;
        };
        let Some(patch_editor) = self.active_patch_editor_ref() else {
            return;
        };

        let rect = ctx.input(|i| i.viewport_rect());
        let window_size = (rect.width(), rect.height());

        let mut modules = Vec::new();
        for module_id in patch_editor.module_ids() {
            if let Some((descriptor, pos, param_values)) = patch_editor.get_module_data(module_id) {
                let panel_size = patch_editor
                    .module_panel_size(module_id)
                    .unwrap_or(egui::Vec2::new(250.0, 200.0));
                let params: Vec<(String, String)> = param_values
                    .iter()
                    .map(|(name, val)| (name.clone(), format!("{val:.3}")))
                    .collect();
                modules.push(crate::mcp_shared::ModuleLayout {
                    id: module_id.to_string(),
                    module_type: descriptor.type_id.as_str().to_string(),
                    name: descriptor.name.clone(),
                    position: (pos.x, pos.y),
                    size: (panel_size.x, panel_size.y),
                    parameters: params,
                });
            }
        }

        let connections: Vec<crate::mcp_shared::ConnectionLayout> = patch_editor
            .connections()
            .iter()
            .map(|c| crate::mcp_shared::ConnectionLayout {
                from_module: c.from_module.to_string(),
                from_port: c.from_port.to_string(),
                to_module: c.to_module.to_string(),
                to_port: c.to_port.to_string(),
            })
            .collect();

        if let Ok(mut layout) = shared.ui_layout.lock() {
            *layout = crate::mcp_shared::UiLayoutData {
                patch_name: self.current_patch_name.clone(),
                modules,
                connections,
                window_size,
            };
        }
    }

    /// Reset the active instrument to a new empty patch.
    /// Clears all modules and adds a default StereoOutput for immediate sound.
    pub(super) fn reset_to_new_patch(&mut self) {
        self.mark_dirty();

        // 1. Clear active instrument's GUI state
        let Some(active_id) = self.active_instrument_id else {
            return;
        };

        // Remove visualizer buffers before clearing (session doesn't track these)
        if let Some(patch_editor) = self
            .instruments
            .iter()
            .find(|i| i.id == active_id)
            .map(|i| &i.patch_editor)
        {
            for module_id in patch_editor.module_ids() {
                let category = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.category);
                if matches!(category, Some(ModuleCategory::Visualizer)) {
                    let _ = self.handle.send_blocking(EngineCommand::RemoveVisualizer {
                        instrument_id: Some(active_id),
                        id: module_id,
                    });
                    self.handle.remove_visualization_buffer(module_id);
                }
            }
        }

        // Clear all non-visualizer modules via session.
        // Failure is non-fatal during load — the instrument may already be empty.
        if let Err(e) = self.session.clear_graph(active_id) {
            eprintln!("clear_graph({active_id:?}) failed during load: {e}");
        }

        // Clear the patch editor GUI state
        if let Some(editor) = self.active_patch_editor() {
            editor.clear();
        }
        self.handle.visualization_buffers.clear();

        // 2. Reset keyboard state
        self.keyboard = PianoKeyboard::new();
        self.glide_time = synth_core::Seconds::new(0.0);

        // 3. Add default StereoOutput so user gets sound immediately
        self.add_stereo_output_module();

        // 4. Update patch name
        self.current_patch_name = "New Patch".to_string();
    }

    /// Create a patch from current rack state.
    pub(super) fn create_patch_from_rack(&self) -> Option<Patch> {
        let active_id = self.active_instrument_id?;
        let instrument = self.instruments.iter().find(|i| i.id == active_id)?;
        let engine_state = Some((self.session.state().as_ref(), active_id));
        patch_bridge::create_patch_from_rack(
            &instrument.name,
            &self.settings.author,
            &instrument.patch_editor,
            &self.keyboard,
            &self.handle,
            self.glide_time,
            engine_state,
        )
    }

    // ------------------------------------------------------------------
    // Project save/load
    // ------------------------------------------------------------------

    /// Build a `ProjectFile` from the current application state — the
    /// canonical engine-side builder plus a GUI overlay for fields the
    /// engine doesn't track (color, module positions, group metadata,
    /// canvas size, visualizer modules).
    pub(super) fn create_project_from_app(&self) -> ProjectFile {
        let opts = self.build_save_options();
        let mut project = crate::project_apply::build_project_from_engine(
            &self.session,
            &self.song,
            &self.sample_library,
            opts,
        );
        // Snapshot ordering may differ from UI-state ordering; honour
        // whichever active id the GUI thinks it has so the file matches
        // the user's focused instrument.
        project.active_instrument_id = self.active_instrument_id.map_or(0, |id| id.as_u64());
        self.overlay_ui_metadata(&mut project);
        project
    }

    /// Build save options from current GUI state (author, glide, octave).
    /// MCP uses the same `ProjectBuildOptions` type with its own field
    /// sources (`McpSharedState`); this is the GUI's mirror.
    pub(super) fn build_save_options(&self) -> crate::project_apply::ProjectBuildOptions {
        let author = if self.current_project_author.is_empty() {
            None
        } else {
            Some(self.current_project_author.clone())
        };

        crate::project_apply::ProjectBuildOptions {
            author,
            glide_time: Some(self.glide_time),
            octave_offset: Some(self.keyboard.octave_offset()),
        }
    }

    /// Overlay GUI-only patch metadata onto a project built from engine
    /// state: instrument color, module positions, group metadata,
    /// canvas size, and visualizer modules (the engine doesn't track
    /// them — they live only in `PatchEditor`).
    pub(super) fn overlay_ui_metadata(&self, project: &mut ProjectFile) {
        use std::collections::HashSet;
        for inst_state in &mut project.instruments {
            let Some(ui_inst) = self.instruments.iter().find(|i| i.id == inst_state.id) else {
                continue;
            };

            inst_state.color = ui_inst.color.clone();

            let patch = &mut inst_state.patch;

            let content_size = ui_inst.patch_editor.content_size();
            patch.settings.canvas_size = Some(crate::patch::CanvasSize::new(
                content_size.x,
                content_size.y,
            ));
            patch.groups = ui_inst.patch_editor.group_states();

            // Overwrite default `(0, 0)` positions on engine-tracked
            // modules with the canvas positions from the patch editor.
            for module in &mut patch.modules {
                if let Ok(module_id) = module.id.parse::<ModuleId>()
                    && let Some((_, pos, _)) = ui_inst.patch_editor.get_module_data(module_id)
                {
                    module.position = crate::patch::Position::new(pos.x, pos.y);
                }
            }

            // Append visualizer modules that exist in the PatchEditor
            // but not in the engine (apply_project skips visualizers,
            // so build_project_from_engine doesn't emit them either).
            let engine_ids: HashSet<String> = patch.modules.iter().map(|m| m.id.clone()).collect();
            for module_id in ui_inst.patch_editor.module_ids() {
                let id_str = module_id.to_string();
                if engine_ids.contains(&id_str) {
                    continue;
                }
                let Some((descriptor, position, gui_params)) =
                    ui_inst.patch_editor.get_module_data(module_id)
                else {
                    continue;
                };
                let name_to_type_id: std::collections::HashMap<&str, &str> = descriptor
                    .parameters
                    .iter()
                    .map(|p| (p.name.as_str(), p.type_id.as_str()))
                    .collect();
                let mut param_map = std::collections::BTreeMap::new();
                for (display_name, value) in &gui_params {
                    let key = name_to_type_id
                        .get(display_name.as_str())
                        .map_or_else(|| display_name.clone(), |tid| (*tid).to_string());
                    param_map.insert(key, crate::patch::ParamValue::Float(*value));
                }
                patch.modules.push(crate::patch::ModuleState {
                    id: id_str,
                    module_type: module_id.module_type,
                    position: crate::patch::Position::new(position.x, position.y),
                    description: String::new(),
                    parameters: param_map,
                    scripts: std::collections::BTreeMap::new(),
                });
            }
        }
    }

    /// Load a ZIP bundle project file with embedded samples.
    pub(super) fn load_bundle_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        let (project, sample_count) = {
            let mut lib = self
                .sample_library
                .write()
                .map_err(|_| "Failed to acquire sample library lock".to_string())?;
            let project = crate::bundle::load_bundle(path, &mut lib).map_err(|e| format!("{e}"))?;
            (project, lib.len())
        };
        self.apply_and_refresh_project(&project);
        Ok(format!(
            "Bundle loaded: {} ({sample_count} samples)",
            path.display()
        ))
    }

    /// Rebuild the GUI's UI mirrors against `project`. Assumes engine
    /// state has already been written via `project_apply::apply_project`;
    /// this only touches `InstrumentUiState`, `PatchEditor` canvases,
    /// visualization buffers, glide / keyboard widgets, and the
    /// active-instrument bookkeeping.
    pub(super) fn refresh_ui_from_project(&mut self, project: &ProjectFile) {
        // 1. Tear down old UI mirrors. The engine instruments are already
        //    gone (apply_project ran tear_down_all_instruments); we just
        //    need to drop the GUI's visualization buffers and the
        //    `InstrumentUiState` vector.
        let old_ids: Vec<InstrumentId> = self.instruments.iter().map(|i| i.id).collect();
        for inst_id in &old_ids {
            self.remove_visualizers_for_instrument(*inst_id);
        }
        self.instruments.clear();
        self.active_instrument_id = None;
        self.handle.visualization_buffers.clear();

        // 2. Build fresh InstrumentUiState per instrument and populate the
        //    patch editor canvases. Engine state is already loaded; the
        //    patch_editor population reads descriptors from `session`.
        for inst_state in &project.instruments {
            let inst_id = inst_state.id;

            let channel = if inst_state.channel == 0 {
                MidiChannelSelection::OMNI
            } else {
                MidiChannelSelection::from_one_indexed(inst_state.channel)
                    .unwrap_or(MidiChannelSelection::CH1)
            };
            let mut ui_inst =
                InstrumentUiState::new(inst_id, &inst_state.name).with_channel(channel);

            patch_bridge::populate_editor_from_patch(
                &inst_state.patch,
                &mut ui_inst.patch_editor,
                &self.session,
                &mut self.handle,
                inst_id,
            );

            if let Some(cs) = inst_state.patch.settings.canvas_size {
                ui_inst
                    .patch_editor
                    .set_min_canvas_size(eframe::egui::Vec2::new(cs.width, cs.height));
            }

            ui_inst.volume = inst_state.volume;
            ui_inst.pan = inst_state.pan;
            ui_inst.muted = inst_state.muted;
            ui_inst.solo = inst_state.solo;
            ui_inst.key_range = synth_engine::instrument::KeyRange::new(
                synth_core::MidiNote::new(inst_state.key_range.0),
                synth_core::MidiNote::new(inst_state.key_range.1),
            );
            ui_inst.transpose = inst_state.transpose;
            ui_inst.oversampling = match inst_state.oversampling {
                2 => synth_dsp::OversamplingFactor::X2,
                4 => synth_dsp::OversamplingFactor::X4,
                _ => synth_dsp::OversamplingFactor::X1,
            };
            ui_inst.category = synth_engine::InstrumentCategory::from_u8(inst_state.category);
            ui_inst.description = inst_state.description.clone();
            ui_inst.color = inst_state.color.clone();
            ui_inst.allocation_mode = inst_state.allocation_mode;
            ui_inst.stealing_strategy = inst_state.stealing_strategy;
            ui_inst.unison_detune = inst_state.unison_detune;
            ui_inst.unison_spread = inst_state.unison_spread;
            ui_inst.max_voices = inst_state.max_voices;
            ui_inst.velocity_amp_sensitivity = inst_state.velocity_amp_sensitivity;
            ui_inst.velocity_filter_sensitivity = inst_state.velocity_filter_sensitivity;
            ui_inst.sidechain_source_id = inst_state.sidechain_source_id.map(InstrumentId::new);
            ui_inst.patch_description = inst_state.patch.description.clone().unwrap_or_default();

            self.instruments.push(ui_inst);
        }

        // 3. Global UI mirrors.
        self.current_project_author = project
            .author
            .clone()
            .unwrap_or_else(|| self.settings.author.clone());
        // Mirror the loaded author into the MCP-shared slot so an
        // MCP-side save reads the loaded value, not a stale one.
        #[cfg(feature = "mcp")]
        if let Some(shared) = &self.mcp_shared {
            *shared.author.lock().unwrap_or_else(|e| e.into_inner()) = project.author.clone();
        }
        self.glide_time = project.global.glide_time;
        self.keyboard
            .set_octave_offset(project.global.octave_offset);

        // 4. Active-instrument bookkeeping. `apply_project` already sent
        //    `SetFocusedInstrument` to the engine; we only mirror it
        //    locally. (The instrument-ID counter lives in `SynthSession`, which
        //    `apply_project` already bumped past every loaded ID.)
        let target_id = InstrumentId::new(project.active_instrument_id);
        self.active_instrument_id = if self.instruments.iter().any(|i| i.id == target_id) {
            Some(target_id)
        } else {
            self.instruments.first().map(|i| i.id)
        };
    }

    /// Refresh UI mirrors after `project_apply::reset_to_new_project`
    /// has cleared the engine. Equivalent to a refresh against an empty
    /// `ProjectFile`.
    pub(super) fn refresh_ui_after_reset(&mut self) {
        let empty = ProjectFile::new(
            Vec::new(),
            0,
            None,
            synth_sequencer::Song::new("Untitled"),
            GlobalProjectState::default(),
        );
        self.refresh_ui_from_project(&empty);
    }

    /// Load a project file, replacing all current state. Wipes the
    /// sample library first — plain-JSON projects carry no embedded
    /// samples, and leaving stale data behind silently switches the
    /// next save to bundle format and re-embeds it.
    pub(super) fn load_project_data(&mut self, project: ProjectFile) {
        crate::project_apply::clear_sample_library(&self.sample_library);
        self.apply_and_refresh_project(&project);
    }

    /// Push `project` to the engine and rebuild GUI mirrors. Caller is
    /// responsible for the sample library — `load_project_data` clears
    /// it (plain-JSON path), `load_bundle_file` leaves it populated
    /// from `bundle::load_bundle`.
    pub(super) fn apply_and_refresh_project(&mut self, project: &ProjectFile) {
        self.undo_manager.clear();
        self.sample_view_state.invalidate_peaks();
        // Note-graph ids restart per project, so stale per-id canvas state
        // (positions, cameras, a mid-gesture undo baseline) must not leak
        // into the loaded project's graphs.
        self.note_grid_view_state = Default::default();
        self.mod_grid_view_state = Default::default();
        // Force a Mod Grid runtime rebuild on the next frame: the loaded pool's
        // generation is unrelated to the previous song's, so don't trust the
        // cached value to differ.
        self.mod_grid_version.invalidate();
        if let Err(e) = crate::project_apply::apply_project(
            project,
            &self.session,
            &self.song,
            &self.sample_library,
        ) {
            tracing::warn!(target: "pertylizer::project", error = %e, "apply_project failed during GUI load");
        }
        self.refresh_ui_from_project(project);
        let project_name = {
            let song = self.song.read();
            if song.name.is_empty() {
                "Untitled".to_string()
            } else {
                song.name.clone()
            }
        };
        tracing::info!(target: "pertylizer::project", "loaded project '{project_name}'");
        // A loaded project belongs in the rack — don't strand the user on the
        // Home welcome screen. Leave any other explicit view choice intact.
        if self.active_view == AppView::Home {
            self.active_view = AppView::Rack;
        }
    }

    /// Reset to a new empty project, clearing all instruments and song data.
    pub(super) fn reset_to_new_project(&mut self) {
        self.undo_manager.clear();
        if let Err(e) = crate::project_apply::reset_to_new_project(
            &self.session,
            &self.song,
            &self.sample_library,
        ) {
            tracing::warn!(target: "pertylizer::project", error = %e, "reset_to_new_project failed");
        }
        tracing::info!(target: "pertylizer::project", "new project");
        self.sample_view_state.invalidate_peaks();
        self.note_grid_view_state = Default::default();
        self.mod_grid_view_state = Default::default();
        // Force a Mod Grid runtime rebuild (empties the runtime for the new,
        // graph-less project).
        self.mod_grid_version.invalidate();
        self.refresh_ui_after_reset();
        self.current_project_path = None;
        self.current_patch_name = "Init".to_string();
        self.current_patch_path = None;
        self.mark_saved();
        // A fresh, empty project lands on the Home welcome screen.
        self.active_view = AppView::Home;
    }

    /// Load a project from a recent-projects path.
    pub(super) fn load_recent_project(&mut self, path: PathBuf) {
        match project::load_file(&path) {
            Ok(LoadedFile::Project(proj)) => {
                self.load_project_data(*proj);
                self.current_project_path = Some(path.clone());
                self.mark_saved();
                self.settings.add_recent_project(path.clone());
                self.settings.save();
                self.dialog_state
                    .set_status(format!("Project loaded: {}", path.display()));
            }
            Ok(LoadedFile::Patch(patch)) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.current_patch_path = Some(path.clone());
                self.mark_saved();
                self.settings.add_recent_project(path.clone());
                self.settings.save();
                self.dialog_state
                    .set_status(format!("Loaded patch: {}", path.display()));
            }
            Ok(LoadedFile::Bundle(bundle_path)) => match self.load_bundle_file(&bundle_path) {
                Ok(msg) => {
                    self.current_project_path = Some(path.clone());
                    self.mark_saved();
                    self.settings.add_recent_project(path.clone());
                    self.settings.save();
                    self.dialog_state.set_status(msg);
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Bundle load error: {e}"));
                }
            },
            Err(e) => {
                self.settings.remove_recent_project(&path);
                self.settings.save();
                self.dialog_state.set_status(format!("Error loading: {e}"));
            }
        }
    }

    /// Save the current project (returns true on success).
    pub(super) fn save_current_project(&mut self) -> bool {
        if let Some(path) = self.current_project_path.clone() {
            let proj = self.create_project_from_app();
            let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
            let path = crate::project::normalize_project_path(&path, has_samples);
            let save_result = if has_samples {
                if let Ok(lib) = self.sample_library.read() {
                    crate::bundle::save_bundle(&proj, &lib, &path)
                } else {
                    proj.save(&path)
                }
            } else {
                proj.save(&path)
            };
            match save_result {
                Ok(()) => {
                    self.current_project_path = Some(path.clone());
                    self.mark_saved();
                    self.settings.add_recent_project(path.clone());
                    self.settings.save();
                    self.dialog_state
                        .set_status(format!("Project saved: {}", path.display()));
                    true
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Error saving project: {e}"));
                    false
                }
            }
        } else {
            let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
            let default_name =
                format!("project.{}", crate::project::project_extension(has_samples));
            let initial_dir = self.resolve_project_dir();
            self.dialog_state.open_file_dialog(
                FileDialogMode::SaveProject,
                Some(&default_name),
                initial_dir.as_deref(),
            );
            false
        }
    }

    /// Show the unsaved-changes confirmation dialog.
    pub(super) fn show_unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        if !self.unsaved_dialog.open {
            return;
        }
        let mut close = false;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. Save before continuing?");
                ui.add_space(theme().spacing.md);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        let saved = self.save_current_project();
                        if saved {
                            self.execute_pending_action(ctx);
                        }
                        close = true;
                    }
                    if ui.button("Don't Save").clicked() {
                        self.mark_saved();
                        self.execute_pending_action(ctx);
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.unsaved_dialog.open = false;
            self.unsaved_dialog.pending_action = None;
        }
    }

    /// Execute the pending action from the unsaved-changes dialog.
    pub(super) fn execute_pending_action(&mut self, ctx: &egui::Context) {
        let action = self.unsaved_dialog.pending_action.take();
        match action {
            Some(PendingAction::NewProject) => {
                self.reset_to_new_project();
                self.dialog_state
                    .set_status("New project created".to_string());
            }
            Some(PendingAction::OpenProject) => {
                let initial_dir = self.resolve_project_dir();
                self.dialog_state.open_file_dialog(
                    FileDialogMode::OpenProject,
                    None,
                    initial_dir.as_deref(),
                );
            }
            Some(PendingAction::LoadProject(path)) => {
                self.load_recent_project(path);
            }
            Some(PendingAction::Quit) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
    }

    /// Remove all visualizer modules for a given instrument.
    pub(super) fn remove_visualizers_for_instrument(&mut self, inst_id: InstrumentId) {
        if let Some(inst) = self.instruments.iter().find(|i| i.id == inst_id) {
            let vis_ids: Vec<_> = inst
                .patch_editor
                .module_ids()
                .into_iter()
                .filter(|mid| {
                    inst.patch_editor
                        .module_descriptor(*mid)
                        .map(|d| d.category)
                        == Some(synth_core::ModuleCategory::Visualizer)
                })
                .collect();
            for module_id in vis_ids {
                let _ = self.handle.send_blocking(EngineCommand::RemoveVisualizer {
                    instrument_id: Some(inst_id),
                    id: module_id,
                });
                self.handle.remove_visualization_buffer(module_id);
            }
        }
    }

    /// Resolve the initial directory for the Project file dialog.
    ///
    /// Priority: last project dir > custom projects dir > default projects dir.
    pub(super) fn resolve_project_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_project_dir
            .clone()
            .or_else(|| self.settings.directories.projects_dir.clone())
            .or_else(|| project::default_projects_dir().ok())
    }
}
