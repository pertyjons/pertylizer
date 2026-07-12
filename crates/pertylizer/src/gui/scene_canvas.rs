//! Generic `egui::Scene` node-canvas layer.
//!
//! The pieces of a pannable/zoomable node editor that carry no audio-graph
//! knowledge: the world-space grid, grid snapping, screen→world conversion,
//! the configured `Scene` builder, initial camera framing, and the pinned
//! zoom/fit view controls. Shared by the patch editor and the Note Grid
//! editor so the two canvases stay visually and behaviorally identical.

use eframe::egui::{self, Color32, Pos2, Rect, Ui, Vec2};

use crate::gui::theme::theme;

/// Grid cell size in world units.
pub(crate) const GRID_SIZE: f32 = 32.0;

/// Background grid line color (faint blue-gray).
const GRID_LINE_COLOR: Color32 = Color32::from_rgba_unmultiplied_const(60, 65, 75, 50);

/// Snap a position to the nearest grid point.
#[must_use]
pub(crate) fn snap_to_grid(pos: Pos2) -> Pos2 {
    Pos2::new(
        (pos.x / GRID_SIZE).round() * GRID_SIZE,
        (pos.y / GRID_SIZE).round() * GRID_SIZE,
    )
}

/// Convert a screen-space position to scene/world coordinates. Inside the
/// `egui::Scene` closure the raw `ui.input` pointer is global/screen, but the
/// manual cable / port / group / background hit-tests compare against world
/// coordinates — egui only transforms the pointer for real widget interactions
/// (knobs, buttons), not for these lookups. Returns `screen` unchanged when the
/// layer has no transform (e.g. before the first Scene frame).
#[must_use]
pub(crate) fn screen_to_world(ui: &Ui, screen: Pos2) -> Pos2 {
    ui.ctx()
        .layer_transform_to_global(ui.layer_id())
        .map_or(screen, |t| t.inverse() * screen)
}

/// The canvas `Scene` builder with the shared camera behavior: 0.2–2.0 zoom,
/// RIGHT-drag pan (freeing LEFT-drag on the empty grid for rubber-band /
/// selection gestures; pan is still available via scroll / trackpad).
/// (`egui::Scene` is itself `#[must_use]`, so dropping the builder warns.)
pub(crate) fn scene() -> egui::Scene {
    egui::Scene::new()
        .zoom_range(egui::Rangef::new(0.2, 2.0))
        .drag_pan_buttons(egui::containers::DragPanButtons::SECONDARY)
}

/// Paint the canvas background and world-space grid over `rect`.
pub(crate) fn draw_grid(ui: &Ui, rect: Rect) {
    let painter = ui.painter();

    painter.rect_filled(rect, 0.0, theme().colors.bg_dark);

    let stroke = egui::Stroke::new(1.0, GRID_LINE_COLOR);
    let mut x = rect.left();
    while x < rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            stroke,
        );
        x += GRID_SIZE;
    }
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            stroke,
        );
        y += GRID_SIZE;
    }
}

/// Frame the given node rects (padded) for the first `Scene` frame. Falls back
/// to the viewport at the world origin when the canvas is empty.
#[must_use]
pub(crate) fn initial_scene_rect(
    node_rects: impl IntoIterator<Item = Rect>,
    visible_rect: Rect,
) -> Rect {
    let mut bounds: Option<Rect> = None;
    for r in node_rects {
        bounds = Some(bounds.map_or(r, |b| b.union(r)));
    }
    bounds.map_or_else(
        || Rect::from_min_size(Pos2::ZERO, visible_rect.size()),
        |b| b.expand(80.0),
    )
}

/// A click on the pinned view-control chips.
pub(crate) enum SceneViewAction {
    ZoomOut,
    ZoomIn,
    /// Frame all nodes (the caller supplies the fit bounds lazily).
    Fit,
    /// Reset zoom to 100% around the current view centre.
    OneToOne,
}

/// Draw the small zoom/fit chip row as a screen-space `egui::Area` pinned to
/// the canvas's top-right so it never pans/zooms with the Scene. Returns the
/// clicked action; the caller resolves it via [`apply_view_action`] (so fit
/// bounds are only computed when "Fit" is actually clicked, not every frame).
pub(crate) fn view_controls(ui: &Ui, id: egui::Id, visible_rect: Rect) -> Option<SceneViewAction> {
    let t = theme();
    let inset = Vec2::splat(t.spacing.panel_padding);

    let mut action: Option<SceneViewAction> = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(
            visible_rect.right() - inset.x,
            visible_rect.top() + inset.y,
        ))
        .constrain_to(visible_rect)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(t.colors.bg_panel)
                .stroke(egui::Stroke::new(1.0, t.colors.border))
                .corner_radius(4.0)
                .inner_margin(t.spacing.widget_spacing)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // `icon_button` gives every chip the shared frameless
                        // look plus an AccessKit/MCP label from the tooltip.
                        let dim = t.colors.text_dim;
                        let mut chip = |ui: &mut Ui, label, tip, act: SceneViewAction| {
                            if crate::gui::widgets::icon_button(ui, label, dim, tip).clicked() {
                                action = Some(act);
                            }
                        };
                        chip(ui, "−", "Zoom out", SceneViewAction::ZoomOut);
                        chip(ui, "+", "Zoom in", SceneViewAction::ZoomIn);
                        chip(ui, "Fit", "Fit all nodes in view", SceneViewAction::Fit);
                        chip(ui, "1:1", "Reset zoom to 100%", SceneViewAction::OneToOne);
                    });
                });
        });
    action
}

/// Resolve a [`SceneViewAction`] against the current camera. `scene_rect` is
/// the visible WORLD region, so a *smaller* rect = *higher* zoom; zoom steps
/// keep the current view centre. egui clamps the result to the Scene's zoom
/// range on the next frame.
#[must_use]
pub(crate) fn apply_view_action(
    action: SceneViewAction,
    scene_rect: Rect,
    visible_rect: Rect,
    fit_bounds: impl FnOnce() -> Rect,
) -> Rect {
    match action {
        SceneViewAction::ZoomOut => {
            Rect::from_center_size(scene_rect.center(), scene_rect.size() * 1.25)
        }
        SceneViewAction::ZoomIn => {
            Rect::from_center_size(scene_rect.center(), scene_rect.size() * 0.8)
        }
        SceneViewAction::Fit => fit_bounds(),
        SceneViewAction::OneToOne => {
            Rect::from_center_size(scene_rect.center(), visible_rect.size())
        }
    }
}
