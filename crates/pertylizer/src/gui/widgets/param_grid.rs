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

use super::{CaptionTone, Knob, WaveformSelector, WaveformType, caption};
use crate::gui::theme::theme;

/// A parameter the user changed this frame: its descriptor plus the new value.
/// The value is stored as the parameter's raw `f32` (a 0-based index for choice
/// parameters); apply it with `descriptor.id.with_f32(value)`.
pub type ParamChange<'d> = (&'d ParameterDescriptor, f32);

/// One modulation marker shown beside a parameter, port, macro, or module. Each
/// kind is a distinct glyph+colour so the three source kinds (Mod Matrix / Script
/// / AudioScript) and the Mod Matrix destination are all visually separable. A
/// single element can carry several at once — see [`ModMarkers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModMarker {
    /// A Mod Matrix slot reads this element as its source.
    MatrixSource,
    /// A control-rate Script module (`ModuleType::Script`) reads this as a source.
    ScriptSource,
    /// An audio-rate AudioScript module (`ModuleType::AudioScript`) reads this.
    AudioScriptSource,
    /// A Mod Matrix slot modulates this parameter (destination).
    MatrixDest,
}

impl ModMarker {
    /// Icon glyph. The two script source kinds share one `ƒx` glyph and are told
    /// apart by colour; the Mod Matrix uses directional arrows.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::MatrixSource => ri::ARROW_RIGHT_UP_LINE,
            Self::ScriptSource | Self::AudioScriptSource => ri::FUNCTION_LINE,
            Self::MatrixDest => ri::ARROW_LEFT_DOWN_LINE,
        }
    }

    /// The fixed corner of a widget box this marker paints in, as `(anchor point,
    /// alignment)`. Each kind always uses the same corner, so a glyph's *position*
    /// is a second cue to its kind on top of its colour, and two markers can never
    /// collide. The two Mod Matrix arrows point out of their corner (↗ top-right,
    /// ↙ bottom-left); the scripts take the remaining corners (Script top-left,
    /// AudioScript bottom-right).
    ///
    /// With `outside` the glyph is nudged just past the corner *horizontally* so it
    /// clears a round widget body (knobs) on the left/right, while growing
    /// *vertically inward* — the top pair sits just below the top edge and the
    /// bottom pair just above the bottom edge (clear of the label under a knob),
    /// rather than floating above/below the cell. `outside == false` tucks the
    /// glyph fully inside the corner (ports, whose fixed box has no spare room).
    #[must_use]
    pub fn corner(self, rect: egui::Rect, outside: bool) -> (egui::Pos2, egui::Align2) {
        use egui::{Align2, vec2};
        const G: f32 = 1.0;
        match (self, outside) {
            (Self::MatrixSource, false) => (rect.right_top(), Align2::RIGHT_TOP),
            (Self::ScriptSource, false) => (rect.left_top(), Align2::LEFT_TOP),
            (Self::AudioScriptSource, false) => (rect.right_bottom(), Align2::RIGHT_BOTTOM),
            (Self::MatrixDest, false) => (rect.left_bottom(), Align2::LEFT_BOTTOM),
            (Self::MatrixSource, true) => (rect.right_top() + vec2(G, 0.0), Align2::LEFT_TOP),
            (Self::ScriptSource, true) => (rect.left_top() + vec2(-G, 0.0), Align2::RIGHT_TOP),
            (Self::AudioScriptSource, true) => {
                (rect.right_bottom() + vec2(G, 0.0), Align2::LEFT_BOTTOM)
            }
            (Self::MatrixDest, true) => (rect.left_bottom() + vec2(-G, 0.0), Align2::RIGHT_BOTTOM),
        }
    }

    /// Accent colour — purple for the Mod Matrix, teal for a Script, yellow for an
    /// AudioScript.
    #[must_use]
    pub fn color(self) -> Color32 {
        let c = &theme().colors;
        match self {
            Self::MatrixSource | Self::MatrixDest => c.accent_purple,
            Self::ScriptSource => c.accent_cyan,
            Self::AudioScriptSource => c.accent_yellow,
        }
    }

    /// Per-element hover text (a parameter, port or macro that participates).
    #[must_use]
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::MatrixSource => "Mod Matrix source\nRead as a source by a Mod Matrix slot.",
            Self::ScriptSource => "Script source\nRead as a source by a Script module.",
            Self::AudioScriptSource => {
                "AudioScript source\nRead as a source by an AudioScript module."
            }
            Self::MatrixDest => {
                "Mod Matrix destination\nA Mod Matrix slot modulates this parameter."
            }
        }
    }

    /// Module-level hover text for the bottom status-bar badge (the roll-up of a
    /// whole module's participation, not a single parameter).
    #[must_use]
    pub fn module_tooltip(self) -> &'static str {
        match self {
            Self::MatrixSource => {
                "Mod Matrix source\nThis module drives one or more Mod Matrix slots."
            }
            Self::ScriptSource => "Script source\nA Script module reads a value from this module.",
            Self::AudioScriptSource => {
                "AudioScript source\nAn AudioScript module reads a value from this module."
            }
            Self::MatrixDest => {
                "Mod Matrix destination\nA Mod Matrix slot modulates a parameter on this module."
            }
        }
    }
}

/// The set of modulation markers on one element. An element can be several source
/// kinds at once (read by a Mod Matrix *and* a Script, say), so this is a set of
/// independent flags rather than a single role. All-false means "not wired".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModMarkers {
    pub matrix_source: bool,
    pub script_source: bool,
    pub audio_script_source: bool,
    pub matrix_dest: bool,
}

impl ModMarkers {
    /// `true` when no marker is active (draw nothing).
    #[must_use]
    pub fn is_empty(self) -> bool {
        !(self.matrix_source || self.script_source || self.audio_script_source || self.matrix_dest)
    }

    /// The active markers in a stable draw order (sources first, then destination).
    pub fn iter(self) -> impl Iterator<Item = ModMarker> {
        [
            self.matrix_source.then_some(ModMarker::MatrixSource),
            self.script_source.then_some(ModMarker::ScriptSource),
            self.audio_script_source
                .then_some(ModMarker::AudioScriptSource),
            self.matrix_dest.then_some(ModMarker::MatrixDest),
        ]
        .into_iter()
        .flatten()
    }
}

/// Draw the inline modulation markers for a toggle parameter — small coloured
/// glyphs, each with its own hover tooltip. No-op when the set is empty.
fn draw_mod_markers_inline(ui: &mut Ui, markers: ModMarkers) {
    for m in markers.iter() {
        caption(ui, m.glyph(), CaptionTone::Color(m.color())).on_hover_text(m.tooltip());
    }
}

/// Draw a parameter's name label with its modulation markers folded into the
/// *same* widget as trailing atoms. A free-standing `ui.label` + markers leaves
/// egui to vertically centre separate widgets whose text sizes differ, so a
/// marker's baseline could drift from the name; as one `AtomLayout` they share a
/// baseline and gap. Each active marker's tooltip is joined onto the whole cell.
fn labeled_param(ui: &mut Ui, param: &ParameterDescriptor, markers: ModMarkers) {
    let name = egui::RichText::new(&param.name)
        .size(theme().fonts.size_normal)
        .color(theme().colors.text_secondary);
    if markers.is_empty() {
        ui.label(name);
        return;
    }
    let mut atoms = egui::Atoms::new(name);
    for m in markers.iter() {
        atoms.push_right(
            egui::RichText::new(m.glyph())
                .size(theme().fonts.size_small)
                .color(m.color()),
        );
    }
    let tip = markers
        .iter()
        .map(ModMarker::tooltip)
        .collect::<Vec<_>>()
        .join("\n\n");
    egui::AtomLayout::new(atoms)
        .show(ui)
        .response
        .on_hover_text(tip);
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
/// mixer shows everything by returning `true`). `markers` reports which modulation
/// markers a parameter carries so they are drawn beside it (the mixer returns an
/// empty set for every parameter).
pub fn draw_parameter_grid<'d>(
    ui: &mut Ui,
    descriptor: &'d ModuleDescriptor,
    accent: Color32,
    get: impl Fn(&ParameterDescriptor) -> f32,
    choice_visible: impl Fn(&ParameterDescriptor, &ChoiceOption) -> bool,
    markers: impl Fn(&ParameterDescriptor) -> ModMarkers,
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
        labeled_param(ui, param, markers(param));
        if WaveformSelector::new(&mut selected)
            .waveforms(waveforms)
            .accent_color(accent)
            .show(ui)
        {
            changes.push((*param, selected as f32));
        }
        if let Some(choice) = choices.get(selected) {
            caption(ui, &choice.name, CaptionTone::Dim);
        }
    }

    // Sliders (logarithmic with a seconds suffix for time parameters).
    for param in &slider_params {
        let mut value = get(param);
        ui.horizontal(|ui| {
            labeled_param(ui, param, markers(param));
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
            labeled_param(ui, param, markers(param));
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
                draw_mod_markers_inline(ui, markers(param));
            }
        });
    }

    // Knobs, in rows.
    if !knob_params.is_empty() {
        ui.add_space(theme().spacing.xs);
        changes.extend(draw_knobs(ui, &knob_params, accent, &get, &markers));
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
    markers: impl Fn(&ParameterDescriptor) -> ModMarkers,
) -> Vec<ParamChange<'d>> {
    /// Cap per row so wide panels still read as a grid rather than one long line.
    const MAX_PER_ROW: usize = 5;

    let mut changes = Vec::new();
    let knob_size = theme().sizes.knob_size;
    let cell_size = Vec2::new((knob_size + 18.0).max(54.0), knob_size + 18.0);

    // Wrap to the available width so the grid fits narrow panels (e.g. the
    // mixer's return inserts) instead of overflowing a fixed 5-per-row.
    let spacing = 5.0;
    // Greedy: fill each row to the panel's capacity. With the content band now
    // sized to its ModuleWidth bucket (see `draw_module_body`), the capacity
    // matches the intended knobs-per-row (Medium 3, Large 4, …), so filling the
    // first row reads correctly (e.g. 7 knobs on a Large → 4+3, 4 knobs on a
    // Medium → 3+1) without a separate balancing pass.
    let per_row = (((ui.available_width() + spacing) / (cell_size.x + spacing))
        .floor()
        .max(1.0) as usize)
        .min(MAX_PER_ROW);

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
                        // A modulated knob shows its markers as corner glyphs, kept
                        // off the label so they never widen the fixed-width grid cell.
                        let knob = Knob::from_descriptor(&mut value, param)
                            .size(knob_size)
                            .accent_color(accent)
                            .mod_markers(markers(param));
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
