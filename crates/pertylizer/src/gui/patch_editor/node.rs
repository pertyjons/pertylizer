//! Module-body rendering for the patch editor.
//!
//! Holds the `PatchEditor` methods that render a module's body (the three-column
//! IN / content / OUT layout and the per-module vertical port columns) plus the
//! large descriptor-driven parameter renderer (`draw_module_panel_params`).
//! Extracted verbatim from `patch_editor.rs` — no behavior change.

use eframe::egui::{self, Color32, Rect, Ui, Vec2};
use egui_remixicon::icons as ri;

use synth_core::{ModuleCategory, ModuleDescriptor, PortName};
use synth_engine::{EngineHandle, ModuleId};

use crate::audio::input::InputState;
use crate::gui::module_panel::{ModulePanelState, PortPosition};
use crate::gui::theme::theme;
use crate::gui::widgets::{CaptionTone, ModMarkers, WidgetPortDirection, caption, expose};

use crate::gui::node_canvas;

use super::{
    AudioInputAction, AudioInputSnapshot, EFFECT_CHAIN_AMBER, ModAddrCatalog, ModuleBodyCtx,
    PanelParamsResult, PatchAnalysis, PatchEditor, PatchEditorResult, PatchPort, PortRenderInfo,
    ScriptDepGraph, convert_port_type, draw_audio_script_module_grid, draw_mod_matrix_grid,
    draw_script_module_grid, draw_visualizer_display, trim_sweep_to_complete_cycles,
};

impl PatchEditor {
    /// Draw a module's body below its header. Global modules (effects /
    /// visualizers) get a full-width informational layout; normal modules get the
    /// three-column layout (IN ports | parameter panel | OUT ports). Parameter and
    /// audio-input changes are forwarded onto `result`.
    pub(super) fn draw_module_body(
        &mut self,
        ui: &mut Ui,
        ctx: ModuleBodyCtx,
        handle: &EngineHandle,
        result: &mut PatchEditorResult,
    ) {
        let ModuleBodyCtx {
            module_id,
            descriptor,
            accent_color,
            connected_ports,
            analysis,
            mod_catalog,
            script_graph,
            effect_chain_order,
            audio_input_snapshot,
        } = ctx;

        // Width the header row established. Used to stretch the three-column body
        // so the OUT port column reaches the panel's right edge instead of
        // floating mid-panel when the body content is narrower than the header
        // (e.g. the Script module's compact slot list under a wide title bar).
        let header_width = ui.min_rect().width();

        // Check if this is a global module (no ports to show in columns)
        let is_global = matches!(
            descriptor.category,
            ModuleCategory::Effect | ModuleCategory::Visualizer
        );

        if is_global {
            // Global modules: full-width content, no port columns
            let is_effect = descriptor.category == ModuleCategory::Effect;
            if is_effect {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} Effect Chain", ri::FLASHLIGHT_FILL))
                            .size(11.0)
                            .color(theme().colors.text_dim),
                    );

                    // Chain position badge
                    if let Some(chain_pos) =
                        effect_chain_order.iter().position(|id| *id == module_id)
                    {
                        let chain_label = format!("#{}", chain_pos + 1);
                        ui.label(
                            egui::RichText::new(chain_label)
                                .size(10.0)
                                .color(EFFECT_CHAIN_AMBER),
                        );
                    }
                });
                ui.label(
                    egui::RichText::new("Applied automatically after voice mixing")
                        .size(9.0)
                        .color(theme().colors.text_dim.gamma_multiply(0.7)),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} Visualizer",
                            egui_remixicon::icons::SPECTRUM_FILL
                        ))
                        .size(11.0)
                        .color(theme().colors.text_dim),
                    );
                });
                ui.label(
                    egui::RichText::new("Displays final output signal")
                        .size(9.0)
                        .color(theme().colors.text_dim.gamma_multiply(0.7)),
                );
            }
            ui.separator();

            if let Some(panel_state) = self.panels.get_mut(&module_id) {
                let vis_buffer = handle.get_visualization_buffer(module_id);
                let panel_result = draw_module_panel_params(
                    ui,
                    panel_state,
                    descriptor,
                    accent_color,
                    vis_buffer,
                    analysis,
                    mod_catalog,
                    script_graph,
                    &self.sample_list,
                    audio_input_snapshot,
                );
                for param in panel_result.param_changes {
                    result.param_changes.push((module_id, param));
                }
                if panel_result.audio_input_action.is_some() {
                    result.audio_input_action = panel_result.audio_input_action;
                }
                for (slot, src) in panel_result.mod_script_actions {
                    result.mod_script_actions.push((module_id, slot, src));
                }
            }
        } else {
            // Normal modules: three-column layout (IN ports | content | OUT ports)
            let col_w = theme().sizes.port_column_width;

            ui.horizontal(|ui| {
                // Tighten the gaps between the port columns and the content band.
                // The default item spacing (8 px) on both sides eats enough width
                // to drop a Medium module below three knobs per row; 4 px keeps the
                // content band wide enough for its bucket's intended knob count
                // while still separating the ports.
                ui.spacing_mut().item_spacing.x = 4.0;
                let spacing_x = ui.spacing().item_spacing.x;

                // Size the content column to exactly the space between the two port
                // columns so IN + content + OUT span the header width, anchoring OUT
                // to the right edge. `set_width` (not `set_min_width`) is deliberate:
                // the OUT column is drawn *after* the content, so it isn't reserved
                // yet — with only a minimum, `available_width()` inside the content
                // would run all the way to the row's right edge (an extra
                // `col_w + spacing`), and fill-width widgets (sliders, the knob-grid
                // column count) would overrun into the OUT port column. Capping the
                // width keeps them inside the content band; fixed content that is
                // genuinely wider still overflows and pushes OUT out as before.
                let content_w = (header_width - 2.0 * col_w - 2.0 * spacing_x).max(0.0);

                // Left port column (IN) - fixed width
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    self.draw_port_column(
                        ui,
                        module_id,
                        descriptor,
                        WidgetPortDirection::Input,
                        connected_ports,
                        analysis,
                    );
                });

                // Content column - stretched to anchor OUT at the edge
                ui.vertical(|ui| {
                    ui.set_width(content_w);
                    if let Some(panel_state) = self.panels.get_mut(&module_id) {
                        let vis_buffer = handle.get_visualization_buffer(module_id);
                        let panel_result = draw_module_panel_params(
                            ui,
                            panel_state,
                            descriptor,
                            accent_color,
                            vis_buffer,
                            analysis,
                            mod_catalog,
                            script_graph,
                            &self.sample_list,
                            audio_input_snapshot,
                        );
                        for param in panel_result.param_changes {
                            result.param_changes.push((module_id, param));
                        }
                        if panel_result.audio_input_action.is_some() {
                            result.audio_input_action = panel_result.audio_input_action;
                        }
                        for (slot, src) in panel_result.mod_script_actions {
                            result.mod_script_actions.push((module_id, slot, src));
                        }
                    }
                });

                // Right port column (OUT) - fixed width
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    self.draw_port_column(
                        ui,
                        module_id,
                        descriptor,
                        WidgetPortDirection::Output,
                        connected_ports,
                        analysis,
                    );
                });
            });
        }
    }

    pub(super) fn draw_port_column(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        direction: WidgetPortDirection,
        connected_ports: &[PortName],
        analysis: &PatchAnalysis,
    ) {
        use synth_core::PortDirection as CorePortDirection;

        let core_dir = match direction {
            WidgetPortDirection::Input => CorePortDirection::Input,
            WidgetPortDirection::Output => CorePortDirection::Output,
        };

        let ports: Vec<PortRenderInfo> = descriptor
            .ports
            .iter()
            .filter(|p| p.direction == core_dir)
            .map(|p| PortRenderInfo {
                module_id,
                port_name: p.name,
                label: p.label.clone(),
                description: p.description.clone(),
                port_type: convert_port_type(p.port_type),
                is_connected: connected_ports.contains(&p.name),
                // Only output ports can be read as a modulation source.
                markers: match direction {
                    WidgetPortDirection::Output => {
                        analysis.markers_for_port(module_id, p.name.as_str())
                    }
                    WidgetPortDirection::Input => ModMarkers::default(),
                },
            })
            .collect();

        let pending_info = self.pending_wire_source();
        // Cycle-blocked targets were computed once for this frame; the highlight
        // just looks each module up.
        let cycle_blocked = &self.drag_cycle_blocked;
        let port_positions = &mut self.port_positions;
        let wire_events = &mut self.wire_events;
        Self::draw_port_column_with(
            ui,
            direction,
            &ports,
            pending_info,
            cycle_blocked,
            |port, center, response| {
                port_positions.insert(
                    (module_id, port.port_name),
                    PortPosition {
                        module_id,
                        port_name: port.port_name,
                        position: center,
                        port_type: port.port_type,
                        direction,
                    },
                );
                // Feed the shared wire FSM (drag-to-connect / click-click).
                let pp = PatchPort {
                    module: module_id,
                    port: port.port_name,
                    direction,
                    port_type: port.port_type,
                };
                node_canvas::push_port_event(wire_events, response, pp, center);
            },
        );
    }
}

/// Draw only the parameters section of a module panel.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_module_panel_params(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    vis_buffer: Option<&synth_engine::visualizers::VisualizationBuffer>,
    analysis: &PatchAnalysis,
    mod_catalog: &ModAddrCatalog,
    script_graph: Option<&ScriptDepGraph>,
    sample_list: &[(u64, String)],
    audio_input_snapshot: &AudioInputSnapshot,
) -> PanelParamsResult {
    use crate::gui::widgets::EnvelopeEditor;
    use synth_core::WidgetHint;

    let mut param_changes = Vec::new();
    let mut audio_input_action = None;
    // Per-knob modulation markers (S1.5a/b): a parameter is marked when a routing
    // targets this module's matching `type_id` (destination) or reads it as a
    // source param — the marker set comes from `markers_for_param`.
    let module_id = state.id;
    let markers =
        |p: &synth_core::ParameterDescriptor| analysis.markers_for_param(module_id, &p.type_id);

    // For Visualizer modules, draw visualization FIRST (before parameters)
    if descriptor.category == ModuleCategory::Visualizer {
        draw_visualizer_display(ui, state, descriptor, vis_buffer, &mut param_changes);
        // Skip regular parameter drawing for visualizers - the display is the main UI
        return PanelParamsResult {
            param_changes,
            audio_input_action: None,
            mod_script_actions: Vec::new(),
        };
    }

    // Special handling for Envelope modules - use interactive EnvelopeEditor
    if descriptor.category == ModuleCategory::Envelope {
        // Get current ADSR values
        let mut attack = state.param_values.get("Attack").copied().unwrap_or(0.01);
        let mut decay = state.param_values.get("Decay").copied().unwrap_or(0.1);
        let mut sustain = state.param_values.get("Sustain").copied().unwrap_or(0.7);
        let mut release = state.param_values.get("Release").copied().unwrap_or(0.3);

        // Get envelope playback position (lock-free)
        let envelope_pos = state.envelope_position.as_ref().map(|buf| buf.get());

        // Draw the interactive envelope editor. Width follows the module's
        // content band (env is Large, MSEG ExtraLarge — both are Envelope-category
        // and share this branch), so the graph fills the panel instead of capping
        // at a fixed 250 px and leaving the wider buckets half-empty.
        ui.add_space(theme().spacing.xs);
        let width = ui.available_width().max(150.0);
        let height = (width * 0.5).clamp(80.0, 140.0);

        let mut editor = EnvelopeEditor::new(&mut attack, &mut decay, &mut sustain, &mut release)
            .accent_color(accent_color)
            .size(width, height)
            .max_time(10.0);

        // Add playback position if available
        if let Some((stage, level)) = envelope_pos {
            editor = editor.playback_position(stage, level);
        }

        if let Some(changes) = editor.show(ui) {
            // Find parameter descriptors and push changes
            for param in &descriptor.parameters {
                if param.name == "Attack" && changes.attack.is_some() {
                    state.param_values.insert("Attack".to_string(), attack);
                    param_changes.push(param.id.with_f32(attack));
                }
                if param.name == "Decay" && changes.decay.is_some() {
                    state.param_values.insert("Decay".to_string(), decay);
                    param_changes.push(param.id.with_f32(decay));
                }
                if param.name == "Sustain" && changes.sustain.is_some() {
                    state.param_values.insert("Sustain".to_string(), sustain);
                    param_changes.push(param.id.with_f32(sustain));
                }
                if param.name == "Release" && changes.release.is_some() {
                    state.param_values.insert("Release".to_string(), release);
                    param_changes.push(param.id.with_f32(release));
                }
            }
        }

        ui.add_space(theme().spacing.xs);

        // Only show knob parameters for Envelope (Vel Sens, curves etc)
        let knob_params: Vec<_> = descriptor
            .parameters
            .iter()
            .filter(|p| matches!(p.widget_hint, WidgetHint::Knob))
            .collect();

        if !knob_params.is_empty() {
            let changes = crate::gui::widgets::draw_knobs(
                ui,
                &knob_params,
                accent_color,
                |p| {
                    state
                        .param_values
                        .get(&p.name)
                        .copied()
                        .unwrap_or(p.range.default)
                },
                markers,
            );
            for (param, value) in changes {
                state.param_values.insert(param.name.clone(), value);
                param_changes.push(param.id.with_f32(value));
            }
        }

        return PanelParamsResult {
            param_changes,
            audio_input_action: None,
            mod_script_actions: Vec::new(),
        };
    }

    // Special handling for Audio Input — monitoring and recording controls
    if descriptor.type_id.0 == "audio_input" {
        let t = theme();
        let input_state = audio_input_snapshot.state;
        let is_monitoring = input_state != InputState::Idle;
        let is_recording = input_state == InputState::Recording;

        // Monitor toggle button
        ui.add_space(t.spacing.xs);
        let monitor_icon = if is_monitoring {
            ri::MIC_FILL
        } else {
            ri::MIC_LINE
        };
        let monitor_color = if is_monitoring {
            t.colors.meter_green
        } else {
            t.colors.text_dim
        };
        if ui
            .button(
                egui::RichText::new(format!("{monitor_icon} Monitor"))
                    .color(monitor_color)
                    .size(11.0),
            )
            .clicked()
        {
            audio_input_action = Some(if is_monitoring {
                AudioInputAction::StopMonitoring
            } else {
                AudioInputAction::StartMonitoring
            });
        }

        // Record button (only enabled when monitoring)
        let rec_icon = if is_recording {
            ri::STOP_FILL
        } else {
            ri::RECORD_CIRCLE_FILL
        };
        let rec_color = if is_recording {
            t.colors.meter_red
        } else if is_monitoring {
            t.colors.text_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .add_enabled(
                is_monitoring,
                egui::Button::new(
                    egui::RichText::new(format!("{rec_icon} Rec"))
                        .color(rec_color)
                        .size(11.0),
                ),
            )
            .clicked()
        {
            audio_input_action = Some(if is_recording {
                AudioInputAction::StopRecording
            } else {
                AudioInputAction::StartRecording
            });
        }

        // Peak level meter
        if is_monitoring {
            let peak = audio_input_snapshot.peak_level;
            let bar_width = ui.available_width().min(120.0);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(bar_width, 6.0), egui::Sense::hover());
            // Read-only peak meter: expose the live level so the MCP can read it.
            expose(
                &response,
                egui::WidgetType::ProgressIndicator,
                "input level",
                Some(f64::from(peak)),
            );
            let painter = ui.painter();
            painter.rect_filled(rect, 2.0, t.colors.bg_dark);
            let fill_w = rect.width() * peak.clamp(0.0, 1.0);
            let fill_color = if peak > 0.9 {
                t.colors.meter_red
            } else if peak > 0.6 {
                t.colors.meter_yellow
            } else {
                t.colors.meter_green
            };
            painter.rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(fill_w, rect.height())),
                2.0,
                fill_color,
            );
            ui.request_repaint();
        }

        // Recording timer
        if is_recording {
            let secs = audio_input_snapshot.recorded_seconds;
            let mins = (secs / 60.0) as u32;
            let remaining = secs - f64::from(mins) * 60.0;
            ui.label(
                egui::RichText::new(format!("{mins}:{remaining:04.1}"))
                    .color(t.colors.meter_red)
                    .monospace()
                    .size(10.0),
            );
        }

        ui.add_space(t.spacing.xs);
    }

    // Special handling for Sampler — sample selector dropdown
    if descriptor.type_id.0 == "sampler" && !sample_list.is_empty() {
        let current_id = state.param_values.get("Sample").copied().unwrap_or(0.0) as u64;
        let current_name = sample_list
            .iter()
            .find(|(id, _)| *id == current_id)
            .map(|(_, name)| name.as_str())
            .unwrap_or("(none)");

        ui.horizontal(|ui| {
            caption(ui, "Sample:", CaptionTone::Color(accent_color));
            egui::ComboBox::from_id_salt(format!("sampler_sample_select_{:?}", state.id))
                .selected_text(current_name)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for &(id, ref name) in sample_list {
                        if ui.selectable_label(id == current_id, name).clicked() {
                            state.param_values.insert("Sample".to_string(), id as f32);
                            param_changes.push(synth_core::Param::sample_select(id));
                        }
                    }
                });
        });
        ui.add_space(theme().spacing.xs);
    }

    // Special handling for Mod Matrix — custom grid rendering
    if descriptor.type_id.0 == "mod_matrix" {
        return draw_mod_matrix_grid(
            ui,
            state,
            descriptor,
            accent_color,
            mod_catalog,
            script_graph,
        );
    }

    // Special handling for the Script module — a list of YAMS slots (one per
    // output port), each opening the shared expression editor.
    if descriptor.type_id.0 == "script" {
        return draw_script_module_grid(ui, state, accent_color, script_graph, mod_catalog);
    }

    // Special handling for the AudioScript module — a single per-sample YAMS
    // program (one stereo slot) rather than 8 control-rate slots.
    if descriptor.type_id.0 == "audio_script" {
        return draw_audio_script_module_grid(ui, state, accent_color, script_graph, mod_catalog);
    }

    // Signal Monitor — draw oscilloscope display above parameters
    if descriptor.type_id.0 == "signal_monitor" {
        let gain = state.param_values.get("Gain").copied().unwrap_or(1.0);
        let trigger_level = state.param_values.get("Trig").copied().unwrap_or(0.5);

        let samples = if let Some(buffer) = vis_buffer {
            buffer.read_sweep().unwrap_or_default()
        } else {
            (0..256)
                .map(|i| {
                    let t = i as f32 / 256.0;
                    (t * std::f32::consts::TAU * 3.0).sin() * 0.5
                })
                .collect()
        };
        let threshold = trigger_level * 2.0 - 1.0;
        let samples = trim_sweep_to_complete_cycles(&samples, threshold);

        let width = ui.available_width().clamp(120.0, 300.0);
        let height = (width * 0.5).clamp(60.0, 120.0);

        crate::gui::widgets::draw_oscilloscope_with_trigger(
            ui,
            samples,
            width,
            height,
            gain,
            theme().colors.accent_cyan,
            Some(trigger_level),
        );

        if vis_buffer.is_none() {
            caption(ui, "No signal", CaptionTone::Dim);
        }
    }

    // Generic descriptor-driven parameter widgets, shared with the mixer's
    // return-bus inserts (see `widgets::draw_parameter_grid`). The patch editor
    // caches values per module and hides mod-matrix targets that aren't wired
    // up; those two concerns are the getter and choice filter, and the shared
    // renderer draws the rest.
    let changes = crate::gui::widgets::draw_parameter_grid(
        ui,
        descriptor,
        accent_color,
        |p| {
            state
                .param_values
                .get(&p.name)
                .copied()
                .unwrap_or(p.range.default)
        },
        // The Mod Matrix has its own picker path (early return above); every other
        // module's choices are always shown.
        |_p, _choice| true,
        markers,
    );
    for (param, value) in changes {
        state.param_values.insert(param.name.clone(), value);
        param_changes.push(param.id.with_f32(value));
    }

    PanelParamsResult {
        param_changes,
        audio_input_action,
        mod_script_actions: Vec::new(),
    }
}
