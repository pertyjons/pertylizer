//! Waveform selector widget with visual preview.

use eframe::egui::{self, Color32, Rect, Sense, Stroke, Ui, Vec2};

use super::colors;

/// Types of waveforms to display.
/// Note: Noise waveforms have been moved to the dedicated NoiseGenerator module.
#[derive(Clone, Copy, PartialEq)]
pub enum WaveformType {
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
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
            Self::Square => if x < 0.5 { 1.0 } else { -1.0 },
            Self::Pulse => if x < 0.25 { 1.0 } else { -1.0 },
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
            accent_color: colors::ACCENT_ORANGE,
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
                    colors::BG_WIDGET.gamma_multiply(1.3)
                } else {
                    colors::BG_WIDGET
                };

                let stroke_color = if is_selected {
                    self.accent_color
                } else {
                    colors::TEXT_DIM
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

    fn draw_waveform(&self, painter: &egui::Painter, rect: Rect, waveform: WaveformType, is_selected: bool) {
        let color = if is_selected {
            self.accent_color
        } else {
            colors::TEXT_SECONDARY
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

        // Draw the waveform line
        for i in 0..points.len() - 1 {
            painter.line_segment(
                [points[i], points[i + 1]],
                Stroke::new(1.5, color),
            );
        }
    }
}
