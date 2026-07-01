//! Shared "module" framing — the rounded, accent-tinted panel look used by
//! patch-editor modules and mixer channel strips.
//!
//! Keeping the frame and header in one place means both views stay visually in
//! sync: tweak the tint strength, corner radius, or header gradient here and it
//! propagates everywhere.

use eframe::egui::{self, Color32, Sense, Ui, Vec2};

use super::theme;

/// Blend `tint` into `base` by `amount` (clamped to `0..=1`), preserving the
/// base alpha. `amount == 0` returns `base`, `amount == 1` returns `tint`.
#[must_use]
pub fn blend_rgb(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let inv = 1.0 - amount;
    Color32::from_rgba_unmultiplied(
        (f32::from(base.r()) * inv + f32::from(tint.r()) * amount).round() as u8,
        (f32::from(base.g()) * inv + f32::from(tint.g()) * amount).round() as u8,
        (f32::from(base.b()) * inv + f32::from(tint.b()) * amount).round() as u8,
        base.a(),
    )
}

/// Builder for the shared module-style [`egui::Frame`]: rounded corners, an
/// accent-tinted fill, and an accent stroke that brightens and thickens when
/// selected. Build the final frame with [`ModuleFrame::build`].
#[derive(Clone, Copy)]
pub struct ModuleFrame {
    base_fill: Color32,
    accent: Color32,
    selected: bool,
    opacity: f32,
    inner_margin: f32,
}

impl ModuleFrame {
    /// Start a frame tinted with `accent`. Defaults: theme module background,
    /// unselected, fully opaque, 8 pt inner margin.
    #[must_use]
    pub fn new(accent: Color32) -> Self {
        Self {
            base_fill: theme().colors.bg_module,
            accent,
            selected: false,
            opacity: 1.0,
            inner_margin: 8.0,
        }
    }

    /// Override the fill colour the accent is blended into (defaults to the
    /// theme module background).
    #[must_use]
    pub fn base_fill(mut self, fill: Color32) -> Self {
        self.base_fill = fill;
        self
    }

    /// Mark the frame as selected (brighter, 2 pt stroke and stronger tint).
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Dim the whole frame (fill) by `opacity` — used for orphaned or
    /// disconnected modules. The accent passed to [`Self::new`] is expected to
    /// already carry the same dimming for the stroke.
    #[must_use]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Override the inner margin (defaults to 8 pt).
    #[must_use]
    pub fn inner_margin(mut self, margin: f32) -> Self {
        self.inner_margin = margin;
        self
    }

    /// Build the [`egui::Frame`]. `style` is the UI's global style, used as the
    /// base for `Frame::window` (shadow, etc.).
    pub fn build(&self, style: &egui::Style) -> egui::Frame {
        let tint_strength = if self.selected { 0.12 } else { 0.06 };
        let fill =
            blend_rgb(self.base_fill, self.accent, tint_strength).gamma_multiply(self.opacity);
        egui::Frame::window(style)
            .corner_radius(6.0)
            .inner_margin(self.inner_margin)
            .stroke(egui::Stroke::new(
                if self.selected { 2.0 } else { 1.0 },
                if self.selected {
                    self.accent
                } else {
                    self.accent.gamma_multiply(0.5)
                },
            ))
            .fill(fill)
    }
}

/// Draw the module header row inside a [`ModuleFrame`]: a left-to-right accent
/// gradient wash, a 4 pt accent indicator bar, the title, and a trailing
/// `actions` closure for buttons. Ends with a separator.
///
/// Must be called as the first content inside the frame (it reaches up by the
/// window margin to paint the gradient flush with the frame's top edge).
///
/// When `title_clickable` is set the title senses clicks and the returned
/// [`egui::Response`] lets callers drive click-to-rename; otherwise the title
/// is a static, non-selectable label and the response can be ignored.
pub fn draw_module_header<F>(
    ui: &mut Ui,
    accent_color: Color32,
    title: &str,
    hover_text: Option<String>,
    title_clickable: bool,
    actions: F,
) -> egui::Response
where
    F: FnOnce(&mut Ui),
{
    let margin = ui.spacing().window_margin;
    let module_rect = ui.max_rect();
    let header_top = ui.cursor().min.y - f32::from(margin.top);
    let header_rect = egui::Rect::from_min_max(
        egui::pos2(module_rect.left() - f32::from(margin.left), header_top),
        egui::pos2(
            module_rect.right() + f32::from(margin.right),
            header_top + 34.0,
        ),
    );
    let tint = accent_color.gamma_multiply(0.14);
    let transparent = Color32::TRANSPARENT;
    // Strong at the top-left, fading down and to the right.
    fill_gradient_quad(
        ui.painter(),
        header_rect,
        [tint, transparent, transparent, tint.gamma_multiply(0.35)],
    );

    let title_response = ui
        .horizontal(|ui| {
            // Accent color indicator
            let (rect, _) = ui.allocate_exact_size(Vec2::new(4.0, 16.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, accent_color);

            // Title. When `title_clickable` it senses clicks so callers can
            // drive click-to-rename; otherwise it's a static, non-selectable
            // label (no hover/selection affordance).
            let label = egui::Label::new(
                egui::RichText::new(title)
                    .strong()
                    .size(theme().fonts.size_normal)
                    .color(accent_color),
            );
            let label = if title_clickable {
                label.sense(Sense::click())
            } else {
                label.selectable(false)
            };
            let mut response = ui.add(label);
            if let Some(hover) = hover_text {
                response = response.on_hover_text(hover);
            }

            // Interactive controls are pinned to the header's right edge; the
            // title keeps the left slack. `right_aligned_row` lays them out
            // right-to-left, so callers emit their actions in reverse reading
            // order (rightmost widget first) and every module header — patch,
            // mixer, group — shares one alignment.
            super::controls::right_aligned_row(ui, actions);
            response
        })
        .inner;

    ui.separator();
    ui.add_space(theme().spacing.xxs);
    title_response
}

/// Draw a module's bottom **status bar** — the foot mirror of
/// [`draw_module_header`].
///
/// Detaches from the body with a leading separator, then runs `content` in a
/// horizontal row. Like the header, this owns only the frame chrome; the actual
/// badges come from the caller (`controls.rs` widgets).
pub fn draw_module_footer<F>(ui: &mut Ui, content: F)
where
    F: FnOnce(&mut Ui),
{
    ui.add_space(theme().spacing.xxs);
    ui.separator();
    ui.horizontal(content);
}

/// Paint a four-corner gradient quad into `painter`. Colors are the corner tints
/// in `[top_left, top_right, bottom_right, bottom_left]` order. Shared by the
/// module header and footer washes so their mesh construction can't drift.
fn fill_gradient_quad(painter: &egui::Painter, rect: egui::Rect, [tl, tr, br, bl]: [Color32; 4]) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), tl);
    mesh.colored_vertex(rect.right_top(), tr);
    mesh.colored_vertex(rect.right_bottom(), br);
    mesh.colored_vertex(rect.left_bottom(), bl);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
