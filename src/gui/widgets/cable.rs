//! Cable drawing utilities.

use eframe::egui::{Color32, Pos2, Stroke};

/// Draw a cable between two points.
pub fn draw_cable(painter: &eframe::egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    // Bezier curve for natural cable look
    let control_offset = (to.x - from.x).abs() * 0.5;
    let ctrl1 = Pos2::new(from.x + control_offset, from.y);
    let ctrl2 = Pos2::new(to.x - control_offset, to.y);

    // Approximate bezier with line segments
    let segments = 32;
    let mut prev = from;

    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        let x = mt3 * from.x + 3.0 * mt2 * t * ctrl1.x + 3.0 * mt * t2 * ctrl2.x + t3 * to.x;
        let y = mt3 * from.y + 3.0 * mt2 * t * ctrl1.y + 3.0 * mt * t2 * ctrl2.y + t3 * to.y;
        let curr = Pos2::new(x, y);

        painter.line_segment([prev, curr], Stroke::new(3.0, color));
        prev = curr;
    }
}
