//! dialog flow responsibilities for the egui application.

use super::*;

impl SynthApp {
    pub(super) fn show_dialogs(&mut self, ctx: &egui::Context) {
        // Recovery offer first: it decides which document the user is in, so it
        // must be answered before anything that acts on the current one.
        self.show_recovery_prompt(ctx);

        // Unsaved changes confirmation dialog
        self.show_unsaved_changes_dialog(ctx);

        // Update dialog state (clears expired status messages)
        self.dialog_state.update();

        // Settings dialog
        match show_settings_dialog(
            ctx,
            &mut self.dialog_state.show_settings,
            &mut self.dialog_state.current_theme,
            &mut self.settings,
        ) {
            SettingsAction::LiveChange | SettingsAction::SaveAndClose => {
                self.settings.save();
            }
            SettingsAction::CloseWithoutSave => {
                // Discard unsaved edits by reloading from disk
                self.settings = AppSettings::load();
            }
            SettingsAction::None => {}
        }

        // About dialog
        show_about_dialog(ctx, &mut self.dialog_state.show_about);

        // Export WAV dialog
        if self.dialog_state.show_export_wav {
            let mut open = self.dialog_state.show_export_wav;
            match crate::gui::export_dialog::show_export_dialog(
                ctx,
                &mut self.dialog_state.export_state,
                &mut open,
            ) {
                crate::gui::export_dialog::ExportDialogResult::Completed(msg) => {
                    self.dialog_state.set_status(msg);
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::Failed(msg) => {
                    self.dialog_state
                        .set_status(format!("Export failed: {msg}"));
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::Closed => {
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::None => {}
            }
            self.dialog_state.show_export_wav = open;

            // Handle deferred export start (avoids borrow conflict)
            if self.dialog_state.export_state.wants_export {
                let project = self.create_project_from_app();
                self.dialog_state
                    .export_state
                    .begin_export(project, std::sync::Arc::clone(&self.sample_library));
            }
        }

        // Load built-in patch dialog
        match show_load_patch_dialog(
            ctx,
            &mut self.dialog_state.show_load_patch,
            &mut self.dialog_state.load_patch_search,
        ) {
            LoadPatchResult::LoadBuiltin(patch) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.dialog_state
                    .set_status(format!("Loaded: {}", patch.name));
            }
            LoadPatchResult::Cancelled | LoadPatchResult::None => {}
        }

        // Group template browser dialog
        if self.dialog_state.show_group_templates {
            let manager = GroupTemplateManager::default();
            let templates = manager.list_all_templates();
            match show_group_template_browser(
                ctx,
                &mut self.dialog_state.show_group_templates,
                &templates,
                &mut self.dialog_state.group_template_search,
                &mut self.dialog_state.group_template_selected,
            ) {
                GroupTemplateBrowserResult::Selected(source) => {
                    match Self::resolve_group_template(&manager, &source) {
                        Ok(template) => {
                            self.insert_group_template(&template);
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Error loading template: {e}"));
                        }
                    }
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::Browse => {
                    let initial_dir = self.resolve_group_templates_dir();
                    self.dialog_state.open_file_dialog(
                        FileDialogMode::OpenGroupTemplate,
                        None,
                        initial_dir.as_deref(),
                    );
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::Cancelled => {
                    self.dialog_state.group_template_drop_pos = None;
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::None => {}
            }
        }

        // Save group template dialog
        if self.dialog_state.show_save_group_template {
            match show_save_group_template_dialog(
                ctx,
                &mut self.dialog_state.show_save_group_template,
                &mut self.dialog_state.group_template_save_name,
                &mut self.dialog_state.group_template_save_description,
                &mut self.dialog_state.group_template_save_category,
            ) {
                SaveGroupTemplateResult::Save => {
                    let group_id = self.dialog_state.group_template_save_group;
                    if let (Some(group_id), Some(editor)) =
                        (group_id, self.active_patch_editor_ref())
                    {
                        let author = self.settings.author.name.trim();
                        let author = if author.is_empty() {
                            None
                        } else {
                            Some(author.to_string())
                        };
                        let description = self
                            .dialog_state
                            .group_template_save_description
                            .trim()
                            .to_string();
                        let description = if description.is_empty() {
                            None
                        } else {
                            Some(description)
                        };
                        let template = editor.build_group_template(
                            group_id,
                            Some(self.dialog_state.group_template_save_category),
                            description,
                            author,
                        );
                        if let Some(template) = template {
                            let manager = GroupTemplateManager::default();
                            match manager.save_template(&template) {
                                Ok(path) => {
                                    self.dialog_state
                                        .set_status(format!("Saved template: {}", path.display()));
                                }
                                Err(e) => {
                                    self.dialog_state
                                        .set_status(format!("Error saving template: {e}"));
                                }
                            }
                        }
                    }
                    self.dialog_state.group_template_save_group = None;
                }
                SaveGroupTemplateResult::Cancelled => {
                    self.dialog_state.group_template_save_group = None;
                }
                SaveGroupTemplateResult::None => {}
            }
        }

        // File dialog (open/save/import)
        if let Some(result) = self.dialog_state.update_file_dialog(ctx) {
            match result {
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenPatch)) => {
                    // Remember the directory for next time
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_open_dir = Some(parent.to_path_buf());
                        self.settings.save();
                    }
                    match Patch::load(&path) {
                        Ok(patch) => {
                            self.load_patch_data(&patch);
                            self.current_patch_name = patch.name.clone();
                            self.current_patch_path = Some(path.clone());
                            self.dialog_state
                                .set_status(format!("Loaded: {}", path.display()));
                        }
                        Err(e) => {
                            self.dialog_state.set_status(format!("Error loading: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::SavePatch)) => {
                    // Remember the directory for next time
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_save_dir = Some(parent.to_path_buf());
                        self.settings.save();
                    }
                    if let Some(patch) = self.create_patch_from_rack() {
                        if let Err(e) = patch.save(&path) {
                            self.dialog_state.set_status(format!("Error saving: {e}"));
                        } else {
                            // Extract name from path
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("patch")
                                .to_string();
                            self.current_patch_name = name;
                            self.current_patch_path = Some(path.clone());
                            self.dialog_state
                                .set_status(format!("Saved: {}", path.display()));
                        }
                    }
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenProject)) => {
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_project_dir = Some(parent.to_path_buf());
                    }
                    // Smart open: auto-detect and load in a single read
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
                        Ok(LoadedFile::Bundle(bundle_path)) => {
                            match self.load_bundle_file(&bundle_path) {
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
                            }
                        }
                        Err(e) => {
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Error reading file: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::SaveProject)) => {
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_project_dir = Some(parent.to_path_buf());
                    }
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
                        }
                        Err(e) => {
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Error saving project: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::ExportWav)) => {
                    // User picked an export path — store it and open the export dialog
                    self.dialog_state.export_state.export_path = Some(path);
                    self.dialog_state.show_export_wav = true;
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenGroupTemplate)) => {
                    let manager = GroupTemplateManager::default();
                    match manager.load_template(&path) {
                        Ok(template) => {
                            self.insert_group_template(&template);
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Error loading template: {e}"));
                        }
                    }
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::ImportSample)) => {
                    let target_rate = synth_core::audio::DeviceSampleRate::DVD_QUALITY;
                    match synth_sampler::load_wav(&path, target_rate) {
                        Ok(sample) => {
                            let name = sample.meta.name.clone();
                            let imported = self
                                .sample_library
                                .write()
                                .ok()
                                .map(|mut lib| lib.add(sample));
                            if let Some(id) = imported {
                                self.sample_view_state.selected_sample = Some(id);
                                self.sample_view_state.invalidate_peaks();
                                crate::undo::SampleUndo {
                                    undo: &mut self.undo_manager,
                                    coalescer: &mut self.drag_coalescer,
                                }
                                .record_import(&self.sample_library, id);
                            }
                            self.dialog_state
                                .set_status(format!("Imported sample: {name}"));
                        }
                        Err(e) => {
                            self.dialog_state.set_status(format!("Import failed: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::ExportSample)) => {
                    if let Some(id) = self.sample_view_state.selected_sample
                        && let Ok(lib) = self.sample_library.read()
                        && let Some(sample) = lib.get(id)
                    {
                        let save_path = if path.extension().is_none() {
                            path.with_extension("wav")
                        } else {
                            path
                        };
                        match synth_sampler::save_wav(
                            sample,
                            &save_path,
                            synth_sampler::BitDepth::Int16,
                        ) {
                            Ok(()) => {
                                self.dialog_state
                                    .set_status(format!("Exported: {}", save_path.display()));
                            }
                            Err(e) => {
                                self.dialog_state.set_status(format!("Export failed: {e}"));
                            }
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::ExportActivityLog)) => {
                    let save_path = if path.extension().is_none() {
                        path.with_extension("txt")
                    } else {
                        path
                    };
                    let text = crate::gui::activity_log_view::format_export(&self.activity_log);
                    match std::fs::write(&save_path, text) {
                        Ok(()) => {
                            self.dialog_state.set_status(format!(
                                "Activity log exported: {}",
                                save_path.display()
                            ));
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Activity log export failed: {e}"));
                        }
                    }
                }
                _ => {}
            }
        }

        // Status message toast
        show_status_toast(ctx, &mut self.dialog_state);
    }
}
