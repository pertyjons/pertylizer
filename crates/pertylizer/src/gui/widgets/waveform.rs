//! Waveform selector widget with visual preview.

use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Types of waveforms to display.
/// Note: Noise waveforms have been moved to the dedicated NoiseGenerator module.
#[derive(Clone, Copy, PartialEq)]
pub enum WaveformType {
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
    DsfSaw,
}

impl WaveformType {
    /// Get all standard oscillator waveforms
    pub fn all() -> Vec<Self> {
        vec![
            Self::Sine,
            Self::Triangle,
            Self::Sawtooth,
            Self::Square,
            Self::Pulse,
            Self::DsfSaw,
        ]
    }

    /// Get the name of the waveform
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Saw",
            Self::Square => "Square",
            Self::Pulse => "Pulse",
            Self::DsfSaw => "DSF Saw",
        }
    }

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
        }
    }
}

/// Waveform selector widget with visual preview.
/// Shows a row of waveform buttons with visual representation of each waveform.
pub struct WaveformSelector<'a> {
    selected: &'a mut usize,
    waveforms: Vec<WaveformType>,
    accent_color: Color32,
}

impl<'a> WaveformSelector<'a> {
    pub fn new(selected: &'a mut usize) -> Self {
        Self {
            selected,
            waveforms: WaveformType::all(),
            accent_color: theme().colors.accent_orange,
        }
    }

    pub fn waveforms(mut self, waveforms: Vec<WaveformType>) -> Self {
        self.waveforms = waveforms;
        self
    }

    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    /// Show the waveform selector and return true if selection changed
    pub fn show(self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let button_size = Vec2::new(40.0, 30.0);

        ui.horizontal(|ui| {
            // A small deliberate gap between the waveform icons, owned by the
            // widget so it doesn't depend on the surrounding layout's spacing.
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, waveform) in self.waveforms.iter().enumerate() {
                let is_selected = i == *self.selected;

                let (rect, response) = ui.allocate_exact_size(button_size, Sense::click());

                if response.clicked() {
                    *self.selected = i;
                    changed = true;
                }

                // Draw button background
                let bg_color = if is_selected {
                    self.accent_color.gamma_multiply(0.3)
                } else if response.hovered() {
                    theme().colors.bg_widget.gamma_multiply(1.3)
                } else {
                    theme().colors.bg_widget
                };

                let stroke_color = if is_selected {
                    self.accent_color
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

                // Draw waveform preview
                let inner_rect = rect.shrink(4.0);
                self.draw_waveform(ui.painter(), inner_rect, *waveform, is_selected);

                // Tooltip with name
                response.on_hover_text(waveform.name());
            }
        });

        changed
    }

    fn draw_waveform(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        waveform: WaveformType,
        is_selected: bool,
    ) {
        let color = if is_selected {
            self.accent_color
        } else {
            theme().colors.text_secondary
        };

        let samples = 32;
        let mut points = Vec::with_capacity(samples);

        for i in 0..samples {
            let x_norm = i as f32 / (samples - 1) as f32;
            let y_norm = waveform.sample(x_norm);

            let x = rect.left() + x_norm * rect.width();
            let y = rect.center().y - y_norm * rect.height() * 0.4;

            points.push(egui::Pos2::new(x, y));
        }

        // Draw the waveform as single optimized line shape
        if points.len() >= 2 {
            painter.add(Shape::line(points, Stroke::new(1.5, color)));
        }
    }
}
