//! Shared "module" framing — the rounded, accent-tinted panel look used by
//! patch-editor modules and mixer channel strips.
//!
//! Keeping the frame and header in one place means both views stay visually in
//! sync: tweak the tint strength, corner radius, or header gradient here and it
//! propagates everywhere.

use eframe::egui::{self, Color32, Sense, Ui, Vec2};
use synth_core::ModuleWidth;

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

/// Rack-led module-card composition: frame, fixed content width, header,
/// domain-specific body, and an optional footer/status bar.
///
/// This owns module chrome only. Canvas behavior (positioning, dragging,
/// selection, wiring, and size measurement) remains with the calling view.
/// Consequently the same card can be used by Rack, Note, Mod, and mixer views
/// without coupling their domain state or interaction models.
#[derive(Clone, Copy)]
pub(crate) struct ModuleCard {
    frame: ModuleFrame,
    accent: Color32,
    width: Option<ModuleCardWidth>,
}

#[derive(Clone, Copy)]
enum ModuleCardWidth {
    Bucket(ModuleWidth),
    BodyBucket(ModuleWidth),
}

/// Derived dimensions for a card whose semantic width describes its body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModuleCardGeometry {
    pub(crate) outer_width: f32,
    pub(crate) content_width: f32,
    pub(crate) body_width: f32,
}

impl ModuleCardGeometry {
    pub(crate) fn ported(width: ModuleWidth, inner_margin: f32) -> Self {
        let sizes = theme().sizes;
        let inner_margin = inner_margin.max(0.0);
        let body_width = (width.module_px() - 2.0 * inner_margin).max(0.0);
        let port_chrome = 2.0 * (sizes.port_column_width + sizes.module_port_gap);
        let content_width = body_width + port_chrome;
        Self {
            outer_width: content_width + 2.0 * inner_margin,
            content_width,
            body_width,
        }
    }
}

/// Content context supplied while rendering a [`ModuleCard`].
pub(crate) struct ModuleCardUi<'a> {
    ui: &'a mut Ui,
    accent: Color32,
}

impl ModuleCardUi<'_> {
    /// Compose a vertically stacked section while retaining access to card
    /// chrome methods inside the closure.
    pub(crate) fn vertical<R>(&mut self, content: impl FnOnce(&mut ModuleCardUi<'_>) -> R) -> R {
        let accent = self.accent;
        self.ui
            .vertical(|ui| content(&mut ModuleCardUi { ui, accent }))
            .inner
    }

    /// Create a stable widget-id scope while retaining the card composition API.
    pub(crate) fn push_id<R>(
        &mut self,
        id_salt: impl egui::AsIdSalt,
        content: impl FnOnce(&mut ModuleCardUi<'_>) -> R,
    ) -> R {
        let accent = self.accent;
        self.ui
            .push_id(id_salt, |ui| content(&mut ModuleCardUi { ui, accent }))
            .inner
    }

    /// Draw the standard Rack-style header.
    pub(crate) fn header<F>(
        &mut self,
        title: &str,
        hover_text: Option<String>,
        title_clickable: bool,
        actions: F,
    ) -> egui::Response
    where
        F: FnOnce(&mut Ui),
    {
        self.header_with_accent(self.accent, title, hover_text, title_clickable, actions)
    }

    /// Draw a header with an accent distinct from the surrounding frame.
    pub(crate) fn header_with_accent<F>(
        &mut self,
        accent: Color32,
        title: &str,
        hover_text: Option<String>,
        title_clickable: bool,
        actions: F,
    ) -> egui::Response
    where
        F: FnOnce(&mut Ui),
    {
        draw_module_header(self.ui, accent, title, hover_text, title_clickable, actions)
    }

    /// Access the body UI for domain-specific content.
    pub(crate) fn body(&mut self) -> &mut Ui {
        self.ui
    }

    /// Draw the optional Rack-style footer/status bar.
    pub(crate) fn footer<F>(&mut self, content: F)
    where
        F: FnOnce(&mut Ui),
    {
        draw_module_footer(self.ui, content);
    }
}

impl ModuleCard {
    /// Start a card using the standard module frame and the given accent.
    #[must_use]
    pub(crate) fn new(accent: Color32) -> Self {
        Self {
            frame: ModuleFrame::new(accent),
            accent,
            width: None,
        }
    }

    /// Mark the card as selected.
    #[must_use]
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.frame = self.frame.selected(selected);
        self
    }

    /// Dim the card fill while preserving its configured accent.
    #[must_use]
    pub(crate) fn opacity(mut self, opacity: f32) -> Self {
        self.frame = self.frame.opacity(opacity);
        self
    }

    /// Override the frame's base fill.
    #[must_use]
    pub(crate) fn base_fill(mut self, fill: Color32) -> Self {
        self.frame = self.frame.base_fill(fill);
        self
    }

    /// Override the frame's inner margin.
    #[must_use]
    pub(crate) fn inner_margin(mut self, margin: f32) -> Self {
        self.frame = self.frame.inner_margin(margin);
        self
    }

    /// Pin the card to one of the shared semantic module-width buckets.
    #[must_use]
    pub(crate) fn module_width(mut self, width: ModuleWidth) -> Self {
        self.width = Some(ModuleCardWidth::Bucket(width));
        self
    }

    /// Size a graph card from a semantic body bucket plus two port columns.
    #[must_use]
    pub(crate) fn body_module_width(mut self, width: ModuleWidth) -> Self {
        self.width = Some(ModuleCardWidth::BodyBucket(width));
        self
    }

    /// Draw the card and compose its header, body, and optional footer in one
    /// sequential closure. One closure is deliberate: immediate-mode views can
    /// safely reuse the same mutable domain state across all three sections.
    pub(crate) fn show<R>(
        self,
        ui: &mut Ui,
        content: impl FnOnce(&mut ModuleCardUi<'_>) -> R,
    ) -> R {
        self.frame
            .build(&ui.global_style())
            .show(ui, |ui| {
                if let Some(content_width) = self.resolved_content_width() {
                    ui.set_width(content_width);
                }
                content(&mut ModuleCardUi {
                    ui,
                    accent: self.accent,
                })
            })
            .inner
    }

    fn resolved_content_width(&self) -> Option<f32> {
        self.width.map(|width| {
            let content_width = match width {
                ModuleCardWidth::Bucket(width) => {
                    width.module_px() - 2.0 * self.frame.inner_margin_value()
                }
                ModuleCardWidth::BodyBucket(width) => {
                    ModuleCardGeometry::ported(width, self.frame.inner_margin_value()).content_width
                }
            };
            content_width.max(0.0)
        })
    }
}

impl ModuleFrame {
    fn inner_margin_value(&self) -> f32 {
        self.inner_margin
    }

    /// Start a frame tinted with `accent`. Defaults: theme module background,
    /// unselected, fully opaque, 8 pt inner margin.
    #[must_use]
    pub fn new(accent: Color32) -> Self {
        Self {
            base_fill: theme().colors.bg_module,
            accent,
            selected: false,
            opacity: 1.0,
            inner_margin: theme().sizes.module_inner_margin,
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
        self.inner_margin = margin.max(0.0);
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
            )
            .truncate();
            let label = if title_clickable {
                label.sense(Sense::click())
            } else {
                label.selectable(false)
            };
            // Lay out the trailing actions first at the right edge, then give
            // the title exactly the remaining width. This keeps close/menu
            // controls visible and truncates long user-defined names in every
            // shared module header instead of letting them overlap the actions.
            ui.allocate_ui_with_layout(
                ui.available_size(),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    actions(ui);
                    let mut response = ui
                        .allocate_ui_with_layout(
                            ui.available_size(),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| ui.add(label),
                        )
                        .inner;
                    if let Some(hover) = hover_text {
                        response = response.on_hover_text(hover);
                    }
                    response
                },
            )
            .inner
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_width_uses_frame_default_margin() {
        let card = ModuleCard::new(Color32::WHITE).module_width(ModuleWidth::Medium);

        assert_eq!(card.resolved_content_width(), Some(240.0));
    }

    #[test]
    fn module_width_tracks_custom_frame_margin() {
        let card = ModuleCard::new(Color32::WHITE)
            .inner_margin(6.0)
            .module_width(ModuleWidth::Medium);

        assert_eq!(card.resolved_content_width(), Some(244.0));
    }

    #[test]
    fn body_module_width_adds_port_columns_and_respects_margin() {
        let card = ModuleCard::new(Color32::WHITE)
            .inner_margin(6.0)
            .body_module_width(ModuleWidth::Medium);

        assert_eq!(card.resolved_content_width(), Some(308.0));
    }

    #[test]
    fn ported_geometry_keeps_outer_content_and_body_in_sync() {
        let geometry = ModuleCardGeometry::ported(ModuleWidth::Medium, 6.0);

        assert_eq!(geometry.outer_width, 320.0);
        assert_eq!(geometry.content_width, 308.0);
        assert_eq!(geometry.body_width, 244.0);
    }

    #[test]
    fn ported_geometry_normalizes_negative_margin() {
        let geometry = ModuleCardGeometry::ported(ModuleWidth::Small, -4.0);

        assert_eq!(geometry.outer_width, 256.0);
        assert_eq!(geometry.content_width, 256.0);
        assert_eq!(geometry.body_width, 192.0);
    }

    #[test]
    fn ported_geometry_expands_outer_width_for_oversized_margin() {
        let geometry = ModuleCardGeometry::ported(ModuleWidth::Small, 120.0);

        assert_eq!(geometry.outer_width, 304.0);
        assert_eq!(geometry.content_width, 64.0);
        assert_eq!(geometry.body_width, 0.0);
    }

    #[test]
    fn unconstrained_card_leaves_content_width_to_parent() {
        let card = ModuleCard::new(Color32::WHITE);

        assert_eq!(card.resolved_content_width(), None);
    }
}
