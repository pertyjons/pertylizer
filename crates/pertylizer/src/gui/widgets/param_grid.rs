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
use egui_remixicon::icons as ri;

use synth_core::{ChoiceOption, ModuleDescriptor, ParamKind, ParameterDescriptor, WidgetHint};

use super::{Knob, WaveformSelector, WaveformType};
use crate::gui::theme::theme;

/// A parameter the user changed this frame: its descriptor plus the new value.
/// The value is stored as the parameter's raw `f32` (a 0-based index for choice
/// parameters); apply it with `descriptor.id.with_f32(value)`.
pub type ParamChange<'d> = (&'d ParameterDescriptor, f32);

/// How a parameter participates in the Mod Matrix, for the per-knob marker
/// (S1.5a/b). Mirrors the three-state module-header badge so the per-parameter
/// marker reads as a precise zoom-in of the module roll-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModRole {
    /// The parameter's value is read as a modulation source.
    Source,
    /// A routing modulates this parameter.
    Destination,
    /// Both at once.
    Both,
}

impl ModRole {
    /// Build from `(is_source, is_destination)` flags — the module-header badge's
    /// roll-up shape. `None` when the element participates in neither.
    #[must_use]
    pub fn from_flags(is_source: bool, is_destination: bool) -> Option<Self> {
        match (is_source, is_destination) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Source),
            (false, true) => Some(Self::Destination),
            (false, false) => None,
        }
    }

    /// Icon glyph. Shared with the module-header badge so the per-knob marker and
    /// the module roll-up never disagree on which arrow means which role.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Source => ri::ARROW_RIGHT_UP_LINE,
            Self::Destination => ri::ARROW_LEFT_DOWN_LINE,
            Self::Both => ri::ARROW_LEFT_RIGHT_LINE,
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Source => "Mod Matrix source\nThis parameter's value drives a Mod Matrix slot.",
            Self::Destination => {
                "Mod Matrix destination\nA Mod Matrix slot modulates this parameter."
            }
            Self::Both => "Mod Matrix\nThis parameter is both a source and a destination.",
        }
    }
}

/// Draw the inline Mod Matrix marker for a parameter — a small purple icon (the
/// same accent + glyphs as the module-header badge) with an explanatory tooltip.
fn draw_mod_marker(ui: &mut Ui, role: ModRole) {
    ui.label(
        egui::RichText::new(role.glyph())
            .size(theme().fonts.size_small)
            .color(theme().colors.accent_purple),
    )
    .on_hover_text(role.tooltip());
}

/// Draw a parameter's name label with its optional Mod Matrix marker folded into
/// the *same* widget as a trailing atom. A free-standing `ui.label` + marker
/// (the old pattern) leaves egui to vertically centre two separate widgets whose
/// text sizes differ, so the marker's baseline could drift from the name; as one
/// `AtomLayout` they share a baseline and a single `icon_spacing` gap. The role
/// tooltip is applied to the whole cell when a marker is present.
fn labeled_param(ui: &mut Ui, param: &ParameterDescriptor, role: Option<ModRole>) {
    let name = egui::RichText::new(&param.name)
        .size(theme().fonts.size_normal)
        .color(theme().colors.text_secondary);
    let Some(role) = role else {
        ui.label(name);
        return;
    };
    let marker = egui::RichText::new(role.glyph())
        .size(theme().fonts.size_small)
        .color(theme().colors.accent_purple);
    egui::AtomLayout::new((name, marker))
        .show(ui)
        .response
        .on_hover_text(role.tooltip());
}

/// Which widget group the auto-renderer draws a parameter in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderGroup {
    Waveform,
    Slider,
    Dropdown,
    Toggle,
    Knob,
}

/// Map a [`WidgetHint`] to its auto-render group, or `None` when the module
/// draws the parameter itself (`EnvelopeEditor` / `PianoKeyboard` / `WaveEditor`)
/// or it is explicitly `Hidden`.
///
/// The `match` is intentionally exhaustive with no wildcard: every value hint
/// must map to a group so it can never silently vanish from the grid (the
/// `PanKnob` / `XYPad` bug), and adding a new `WidgetHint` becomes a *compile
/// error* here until it is classified. Hints without a bespoke widget fall back
/// to a knob (a descriptor-driven `Knob` already honours range/curve/unit, so a
/// bipolar `PanKnob` renders correctly).
fn render_group(hint: WidgetHint) -> Option<RenderGroup> {
    match hint {
        WidgetHint::WaveformSelector => Some(RenderGroup::Waveform),
        WidgetHint::Slider
        | WidgetHint::TimeSlider
        | WidgetHint::PercentSlider
        | WidgetHint::DecibelSlider => Some(RenderGroup::Slider),
        WidgetHint::Dropdown => Some(RenderGroup::Dropdown),
        WidgetHint::Toggle => Some(RenderGroup::Toggle),
        WidgetHint::Knob
        | WidgetHint::FrequencySlider
        | WidgetHint::PanKnob
        | WidgetHint::XYPad => Some(RenderGroup::Knob),
        WidgetHint::EnvelopeEditor
        | WidgetHint::PianoKeyboard
        | WidgetHint::WaveEditor
        | WidgetHint::Hidden => None,
    }
}

/// Kind-aware render group for a descriptor (Phase 6). A `Bool`-kind parameter
/// that renders at all is always a checkbox (Toggle) regardless of its widget hint
/// — defensive so a bool can never be drawn as a 0–1 slider/knob. A `Hidden`
/// (custom-UI) bool stays hidden. Otherwise the widget hint decides.
fn param_render_group(p: &ParameterDescriptor) -> Option<RenderGroup> {
    let base = render_group(p.widget_hint);
    if p.kind == ParamKind::Bool && base.is_some() {
        return Some(RenderGroup::Toggle);
    }
    base
}

/// Render every parameter of `descriptor` as the widget its [`WidgetHint`] asks
/// for, and return the ones the user changed this frame.
///
/// `get` supplies a parameter's current value; `choice_visible` filters dropdown
/// choices (the patch editor hides mod-matrix targets that aren't wired up — the
/// mixer shows everything by returning `true`). `mod_role` reports whether a
/// parameter participates in the Mod Matrix so a marker is drawn beside it (the
/// mixer returns `None` for every parameter).
pub fn draw_parameter_grid<'d>(
    ui: &mut Ui,
    descriptor: &'d ModuleDescriptor,
    accent: Color32,
    get: impl Fn(&ParameterDescriptor) -> f32,
    choice_visible: impl Fn(&ParameterDescriptor, &ChoiceOption) -> bool,
    mod_role: impl Fn(&ParameterDescriptor) -> Option<ModRole>,
) -> Vec<ParamChange<'d>> {
    let mut changes = Vec::new();

    let by_group = |group: RenderGroup| -> Vec<&'d ParameterDescriptor> {
        descriptor
            .parameters
            .iter()
            .filter(|p| param_render_group(p) == Some(group))
            .collect()
    };
    let waveform_params = by_group(RenderGroup::Waveform);
    let slider_params = by_group(RenderGroup::Slider);
    let dropdown_params = by_group(RenderGroup::Dropdown);
    let toggle_params = by_group(RenderGroup::Toggle);
    let knob_params = by_group(RenderGroup::Knob);

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
        labeled_param(ui, param, mod_role(param));
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
            labeled_param(ui, param, mod_role(param));
            ui.add_space(theme().spacing.xs);
            let is_integer = param.kind == ParamKind::Integer;
            let is_time =
                matches!(param.widget_hint, WidgetHint::TimeSlider) && param.range.min > 0.0;
            let slider = if is_integer {
                // Integer kind: snap to whole numbers, no decimals (Phase 6).
                egui::Slider::new(&mut value, param.range.min..=param.range.max)
                    .step_by(1.0)
                    .min_decimals(0)
                    .max_decimals(0)
            } else if is_time {
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
            // Make the rail fill the remaining row width instead of egui's fixed
            // `slider_width` (~100 px), which otherwise bursts the narrow buckets.
            // egui draws the value as a `DragValue` to the right, so reserve exactly
            // its width — measured from the widest range endpoint formatted as the
            // slider shows it, rather than a fixed guess (time values like "0.258s"
            // are wider than plain decimals and were overflowing the bucket).
            let fmt = |v: f32| {
                if is_integer {
                    format!("{v:.0}")
                } else if is_time {
                    format!("{v:.3}s")
                } else {
                    format!("{v:.2}")
                }
            };
            let font = egui::TextStyle::Body.resolve(ui.style());
            let value_w = [param.range.min, param.range.max]
                .into_iter()
                .map(|v| {
                    ui.painter()
                        .layout_no_wrap(fmt(v), font.clone(), egui::Color32::PLACEHOLDER)
                        .size()
                        .x
                })
                .fold(0.0_f32, f32::max);
            // value text + DragValue padding + the slider→value item spacing.
            let reserve = value_w + 28.0;
            ui.spacing_mut().slider_width = (ui.available_width() - reserve).max(40.0);
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
            labeled_param(ui, param, mod_role(param));
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
                if let Some(role) = mod_role(param) {
                    draw_mod_marker(ui, role);
                }
            }
        });
    }

    // Knobs, in rows.
    if !knob_params.is_empty() {
        ui.add_space(theme().spacing.xs);
        changes.extend(draw_knobs(ui, &knob_params, accent, &get, &mod_role));
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
    mod_role: impl Fn(&ParameterDescriptor) -> Option<ModRole>,
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
                        // A modulated knob shows the marker as a corner glyph (the
                        // same purple language as the label-group markers), kept off
                        // the label so it never widens the fixed-width grid cell.
                        let mut knob = Knob::from_descriptor(&mut value, param)
                            .size(knob_size)
                            .accent_color(accent);
                        if let Some(role) = mod_role(param) {
                            knob = knob.mod_marker(role.glyph(), theme().colors.accent_purple);
                        }
                        knob.show(ui);
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

#[cfg(test)]
mod tests {
    use super::{RenderGroup, param_render_group, render_group};
    use synth_core::WidgetHint;

    #[test]
    fn bool_kind_renders_as_toggle_but_hidden_stays_hidden() {
        use synth_core::params::{MsegParam, OscillatorParam};
        use synth_core::{Hertz, Param, ParameterDescriptor};
        // A Bool with a Knob hint is forced to a checkbox (Toggle).
        let bool_knob =
            ParameterDescriptor::float("loop", Param::Mseg(MsegParam::LoopEnabled(false)), "Loop")
                .widget(WidgetHint::Knob);
        assert_eq!(param_render_group(&bool_knob), Some(RenderGroup::Toggle));
        // A Hidden bool (custom UI) stays hidden — not forced visible.
        let bool_hidden =
            ParameterDescriptor::float("loop", Param::Mseg(MsegParam::LoopEnabled(false)), "Loop")
                .widget(WidgetHint::Hidden);
        assert_eq!(param_render_group(&bool_hidden), None);
        // A non-bool param follows its widget hint.
        let cont = ParameterDescriptor::float(
            "freq",
            Param::Oscillator(OscillatorParam::Frequency(Hertz::new(440.0))),
            "Freq",
        )
        .widget(WidgetHint::Knob);
        assert_eq!(param_render_group(&cont), Some(RenderGroup::Knob));
    }

    #[test]
    fn previously_dropped_hints_now_render() {
        // Regression: these hints mapped to no group and silently vanished from
        // the auto-renderer. PanKnob/XYPad render as knobs, the unit sliders as
        // sliders.
        assert_eq!(render_group(WidgetHint::PanKnob), Some(RenderGroup::Knob));
        assert_eq!(render_group(WidgetHint::XYPad), Some(RenderGroup::Knob));
        assert_eq!(
            render_group(WidgetHint::PercentSlider),
            Some(RenderGroup::Slider)
        );
        assert_eq!(
            render_group(WidgetHint::DecibelSlider),
            Some(RenderGroup::Slider)
        );
    }

    #[test]
    fn module_supplied_and_hidden_hints_are_not_auto_rendered() {
        for hint in [
            WidgetHint::EnvelopeEditor,
            WidgetHint::PianoKeyboard,
            WidgetHint::WaveEditor,
            WidgetHint::Hidden,
        ] {
            assert_eq!(render_group(hint), None, "{hint:?} should not auto-render");
        }
    }

    #[test]
    fn standard_value_hints_map_to_their_widget() {
        assert_eq!(
            render_group(WidgetHint::WaveformSelector),
            Some(RenderGroup::Waveform)
        );
        assert_eq!(render_group(WidgetHint::Slider), Some(RenderGroup::Slider));
        assert_eq!(
            render_group(WidgetHint::TimeSlider),
            Some(RenderGroup::Slider)
        );
        assert_eq!(
            render_group(WidgetHint::Dropdown),
            Some(RenderGroup::Dropdown)
        );
        assert_eq!(render_group(WidgetHint::Toggle), Some(RenderGroup::Toggle));
        assert_eq!(render_group(WidgetHint::Knob), Some(RenderGroup::Knob));
        assert_eq!(
            render_group(WidgetHint::FrequencySlider),
            Some(RenderGroup::Knob)
        );
    }
}
