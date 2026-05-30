//! Descriptor-driven parameter widgets shared by the patch editor's module
//! panels and the mixer's return-bus inserts.
//!
//! Both callers render the *same* controls — waveform selectors, sliders, choice
//! dropdowns, toggles and knobs — grouped by each parameter's [`WidgetHint`]. The
//! only things that differ between them are where a parameter's current value
//! comes from (the patch editor caches it per module; the mixer reads it from the
//! engine snapshot) and what a change does, so those are passed in as a getter
//! closure and read back from the returned change list. Keeping the rendering
//! here means a tweak to, say, the time-slider formatting shows up in both views.

use eframe::egui::{self, Color32, Ui, Vec2};

use synth_core::{ChoiceOption, ModuleDescriptor, ParameterDescriptor, WidgetHint};

use super::{Knob, WaveformSelector, WaveformType};
use crate::gui::theme::theme;

/// A parameter the user changed this frame: its descriptor plus the new value.
/// The value is stored as the parameter's raw `f32` (a 0-based index for choice
/// parameters); apply it with `descriptor.id.with_f32(value)`.
pub type ParamChange<'d> = (&'d ParameterDescriptor, f32);

/// Render every parameter of `descriptor` as the widget its [`WidgetHint`] asks
/// for, and return the ones the user changed this frame.
///
/// `get` supplies a parameter's current value; `choice_visible` filters dropdown
/// choices (the patch editor hides mod-matrix targets that aren't wired up — the
/// mixer shows everything by returning `true`).
pub fn draw_parameter_grid<'d>(
    ui: &mut Ui,
    descriptor: &'d ModuleDescriptor,
    accent: Color32,
    get: impl Fn(&ParameterDescriptor) -> f32,
    choice_visible: impl Fn(&ParameterDescriptor, &ChoiceOption) -> bool,
) -> Vec<ParamChange<'d>> {
    let mut changes = Vec::new();

    let by_hint = |hints: &[WidgetHint]| -> Vec<&'d ParameterDescriptor> {
        descriptor
            .parameters
            .iter()
            .filter(|p| hints.contains(&p.widget_hint))
            .collect()
    };
    let waveform_params = by_hint(&[WidgetHint::WaveformSelector]);
    let slider_params = by_hint(&[WidgetHint::Slider, WidgetHint::TimeSlider]);
    let dropdown_params = by_hint(&[WidgetHint::Dropdown]);
    let toggle_params = by_hint(&[WidgetHint::Toggle]);
    let knob_params = by_hint(&[WidgetHint::Knob, WidgetHint::FrequencySlider]);

    // Waveform selectors first (most prominent).
    for param in &waveform_params {
        let Some(choices) = &param.choices else {
            continue;
        };
        // Only show buttons for the waveforms this module actually supports.
        let waveforms: Vec<WaveformType> = choices
            .iter()
            .filter_map(|c| WaveformType::from_id(&c.id))
            .collect();
        if waveforms.is_empty() {
            continue;
        }
        let mut selected = (get(param).round() as usize).min(waveforms.len() - 1);
        ui.label(
            egui::RichText::new(&param.name)
                .size(theme().fonts.size_normal)
                .color(theme().colors.text_secondary),
        );
        if WaveformSelector::new(&mut selected)
            .waveforms(waveforms)
            .accent_color(accent)
            .show(ui)
        {
            changes.push((*param, selected as f32));
        }
        if let Some(choice) = choices.get(selected) {
            ui.label(
                egui::RichText::new(&choice.name)
                    .size(theme().fonts.size_small)
                    .color(theme().colors.text_dim),
            );
        }
    }

    // Sliders (logarithmic with a seconds suffix for time parameters).
    for param in &slider_params {
        let mut value = get(param);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );
            ui.add_space(4.0);
            let is_time = matches!(param.widget_hint, WidgetHint::TimeSlider);
            let slider = if is_time && param.range.min > 0.0 {
                egui::Slider::new(&mut value, param.range.min..=param.range.max)
                    .logarithmic(true)
                    .suffix("s")
                    .min_decimals(3)
                    .max_decimals(3)
            } else {
                egui::Slider::new(&mut value, param.range.min..=param.range.max)
                    .min_decimals(2)
                    .max_decimals(2)
            };
            if ui.add(slider).changed() {
                changes.push((*param, value));
            }
        });
    }

    // Choice dropdowns (the stored value is the 0-based choice index).
    for param in &dropdown_params {
        let Some(choices) = &param.choices else {
            continue;
        };
        let current = get(param);
        let mut selected = current.round() as usize;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );
            let text = choices
                .get(selected)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".into());
            let combo =
                egui::ComboBox::from_id_salt(format!("{}-{}", descriptor.type_id.0, param.name))
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        for (i, choice) in choices.iter().enumerate() {
                            if !choice_visible(param, choice) {
                                continue;
                            }
                            let mut resp = ui.selectable_label(selected == i, &choice.name);
                            if let Some(desc) = &choice.description {
                                resp = resp.on_hover_text(desc);
                            }
                            if resp.clicked() {
                                selected = i;
                            }
                        }
                    });
            // Hovering the closed combo shows the active choice's description.
            if let Some(desc) = choices.get(selected).and_then(|c| c.description.as_ref()) {
                combo.response.on_hover_text(desc);
            }
        });
        if selected as f32 != current.round() {
            changes.push((*param, selected as f32));
        }
    }

    // Toggles, packed onto one row.
    if !toggle_params.is_empty() {
        ui.horizontal(|ui| {
            for param in &toggle_params {
                let mut checked = get(param) > 0.5;
                if ui.checkbox(&mut checked, &param.name).changed() {
                    changes.push((*param, if checked { 1.0 } else { 0.0 }));
                }
            }
        });
    }

    // Knobs, in rows.
    if !knob_params.is_empty() {
        ui.add_space(4.0);
        changes.extend(draw_knobs(ui, &knob_params, accent, &get));
    }

    changes
}

/// Render a set of parameters as rotary knobs in rows, returning the ones the
/// user changed. Exposed separately so the envelope panel — which draws its ADSR
/// with a dedicated editor — can still render its remaining knob parameters.
pub fn draw_knobs<'d>(
    ui: &mut Ui,
    params: &[&'d ParameterDescriptor],
    accent: Color32,
    get: impl Fn(&ParameterDescriptor) -> f32,
) -> Vec<ParamChange<'d>> {
    /// Cap per row so wide panels still read as a grid rather than one long line.
    const MAX_PER_ROW: usize = 5;

    let mut changes = Vec::new();
    let knob_size = theme().sizes.knob_size;
    let cell_size = Vec2::new((knob_size + 18.0).max(54.0), knob_size + 18.0);

    // Wrap to the available width so the grid fits narrow panels (e.g. the
    // mixer's return inserts) instead of overflowing a fixed 5-per-row.
    let spacing = 5.0;
    let per_row = ((ui.available_width() + spacing) / (cell_size.x + spacing))
        .floor()
        .max(1.0) as usize;
    let per_row = per_row.min(MAX_PER_ROW);

    for chunk in params.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(5.0, 6.0);
            for param in chunk {
                ui.allocate_ui_with_layout(
                    cell_size,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let current = get(param);
                        let mut value = current;
                        Knob::from_descriptor(&mut value, param)
                            .size(knob_size)
                            .accent_color(accent)
                            .show(ui);
                        if (value - current).abs() > f32::EPSILON {
                            changes.push((*param, value));
                        }
                    },
                );
            }
        });
    }

    changes
}
