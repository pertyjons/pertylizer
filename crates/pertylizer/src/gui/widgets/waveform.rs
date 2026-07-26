//! Waveform selector widget with visual preview.

use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// The *shapes* this widget can draw a preview of — a rendering concern only.
/// A waveform's name is not here: it comes from the module descriptor's
/// [`ChoiceOption`](synth_core::module_traits::ChoiceOption) (or the toggle
/// parameter's own name) via [`from_id`](Self::from_id), so the button label, its
/// tooltip, and the caption under the selector are all one string. `Noise` is
/// reachable only through the waveform-bit toggles (the SID oscillator) —
/// standalone noise generation lives in the NoiseGenerator module.
#[derive(Clone, Copy, PartialEq)]
pub enum WaveformType {
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
    DsfSaw,
    Noise,
    SampleAndHold,
    SmoothRandom,
}

impl WaveformType {
    /// Map a `ChoiceOption.id` string to a waveform variant for visual
    /// rendering. `"pulse25"` shares the 25%-duty `Pulse` visualisation.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "pulse" | "pulse25" => Some(Self::Pulse),
            "dsf_saw" => Some(Self::DsfSaw),
            "noise" => Some(Self::Noise),
            "sample_and_hold" => Some(Self::SampleAndHold),
            "smooth_random" => Some(Self::SmoothRandom),
            _ => None,
        }
    }

    /// Generate sample points for visualization (0 to 1 normalized x, -1 to 1 y)
    pub fn sample(&self, x: f32) -> f32 {
        match self {
            Self::Sine => (x * std::f32::consts::TAU).sin(),
            Self::Triangle => {
                if x < 0.5 {
                    4.0 * x - 1.0
                } else {
                    3.0 - 4.0 * x
                }
            }
            Self::Sawtooth => 2.0 * x - 1.0,
            Self::Square => {
                if x < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Pulse => {
                if x < 0.25 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::DsfSaw => {
                // Band-limited saw: softer transitions than raw sawtooth
                let phase = x * std::f32::consts::TAU;
                let mut sum = 0.0_f32;
                for k in 1..=6 {
                    sum += (k as f32 * phase).sin() / k as f32;
                }
                sum * (2.0 / std::f32::consts::PI)
            }
            Self::Noise => {
                // Deterministic pseudo-random zigzag (a pure hash of x, so the
                // preview never flickers between frames). rem_euclid keeps the
                // hash in [0, 1) — fract() goes negative for negative inputs,
                // which pushed the line outside the button.
                let h = (x * 12.9898).sin() * 43758.547;
                (h.rem_euclid(1.0) * 2.0 - 1.0) * 0.8
            }
            Self::SampleAndHold => {
                let step = (x * 6.0).floor();
                let h = (step * 12.9898).sin() * 43758.547;
                (h.rem_euclid(1.0) * 2.0 - 1.0) * 0.8
            }
            Self::SmoothRandom => {
                let position = x * 4.0;
                let step = position.floor();
                let fraction = position - step;
                let random_at = |index: f32| {
                    let h = (index * 12.9898).sin() * 43758.547;
                    (h.rem_euclid(1.0) * 2.0 - 1.0) * 0.8
                };
                let from = random_at(step);
                let to = random_at(step + 1.0);
                let smooth = fraction * fraction * (3.0 - 2.0 * fraction);
                from + (to - from) * smooth
            }
        }
    }
}

/// Paint one waveform-preview button (shared by the exclusive
/// [`WaveformSelector`] and the multi-select waveform-bit toggles): a rounded
/// box with the waveform drawn inside, accent-lit while `active`. `label` is the
/// descriptor's name for this choice — the widget never names a waveform itself.
pub(crate) fn waveform_button(
    ui: &mut Ui,
    waveform: WaveformType,
    label: &str,
    active: bool,
    accent: Color32,
) -> egui::Response {
    let button_size = Vec2::new(40.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(button_size, Sense::click());

    let bg_color = if active {
        accent.gamma_multiply(0.3)
    } else if response.hovered() {
        theme().colors.bg_widget.gamma_multiply(1.3)
    } else {
        theme().colors.bg_widget
    };
    let stroke_color = if active {
        accent
    } else {
        theme().colors.text_dim
    };
    ui.painter().rect(
        rect,
        4.0,
        bg_color,
        Stroke::new(1.0, stroke_color),
        egui::StrokeKind::Outside,
    );

    let line_color = if active {
        accent
    } else {
        theme().colors.text_secondary
    };
    draw_waveform_preview(ui.painter(), rect.shrink(4.0), waveform, line_color);

    // Expose to AccessKit / the egui-inspection MCP so a driver can pick a
    // waveform by name instead of by pixel.
    super::controls::expose(&response, egui::WidgetType::Button, label, None);

    response.on_hover_text(label.to_owned())
}

/// Draw a waveform's preview line inside `rect`.
fn draw_waveform_preview(
    painter: &egui::Painter,
    rect: Rect,
    waveform: WaveformType,
    color: Color32,
) {
    let samples = 32;
    let mut points = Vec::with_capacity(samples);
    for i in 0..samples {
        #[allow(clippy::cast_precision_loss)]
        let x_norm = i as f32 / (samples - 1) as f32;
        let y_norm = waveform.sample(x_norm);
        let x = rect.left() + x_norm * rect.width();
        let y = rect.center().y - y_norm * rect.height() * 0.4;
        points.push(egui::Pos2::new(x, y));
    }
    if points.len() >= 2 {
        painter.add(Shape::line(points, Stroke::new(1.5, color)));
    }
}

/// One choice offered by a [`WaveformSelector`]: the shape to preview and the
/// descriptor's name for it (the button tooltip and AccessKit label).
pub struct WaveformChoice {
    pub shape: WaveformType,
    pub label: String,
}

/// Exclusive waveform selector: a row of preview buttons, one per choice the
/// module's descriptor advertises.
pub struct WaveformSelector<'a> {
    selected: &'a mut usize,
    choices: Vec<WaveformChoice>,
    accent_color: Color32,
}

impl<'a> WaveformSelector<'a> {
    /// The choices are required rather than defaulted: only the descriptor knows
    /// which waveforms a module actually supports, and what it calls them.
    pub fn new(selected: &'a mut usize, choices: Vec<WaveformChoice>) -> Self {
        Self {
            selected,
            choices,
            accent_color: theme().colors.accent_orange,
        }
    }

    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    /// Show the waveform selector and return true if selection changed
    pub fn show(self, ui: &mut Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            // A small deliberate gap between the waveform icons, owned by the
            // widget so it doesn't depend on the surrounding layout's spacing.
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, choice) in self.choices.iter().enumerate() {
                let is_selected = i == *self.selected;
                if waveform_button(
                    ui,
                    choice.shape,
                    &choice.label,
                    is_selected,
                    self.accent_color,
                )
                .clicked()
                {
                    *self.selected = i;
                    changed = true;
                }
            }
        });
        changed
    }
}
