//! Interactive multi-segment envelope editor.

use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use synth_core::{BipolarValue, NormalizedValue, Seconds};

use super::controls::{EnvelopeCurveDirection, envelope_curve_after_vertical_drag};
use crate::gui::theme::theme;

pub(crate) const MAX_MSEG_SEGMENTS: usize = 16;
const MAX_MSEG_SEGMENTS_U8: u8 = 16;
const CURVE_STEPS: usize = 16;
const HIT_RADIUS: f32 = 12.0;
const MIN_SEGMENT_TIME: f32 = 0.001;
const MAX_SEGMENT_TIME: Seconds = Seconds::new(60.0);

/// Index of a segment in the fixed-size MSEG backing store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MsegSegmentIndex(u8);

impl MsegSegmentIndex {
    #[must_use]
    pub(crate) fn new(index: u8) -> Self {
        Self(index.min(MAX_MSEG_SEGMENTS_U8 - 1))
    }

    #[must_use]
    pub(crate) const fn as_u8(self) -> u8 {
        self.0
    }

    #[must_use]
    fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

/// Number of active segments in an MSEG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MsegSegmentCount(u8);

impl MsegSegmentCount {
    #[must_use]
    pub(crate) fn new(count: u8) -> Self {
        Self(count.clamp(1, MAX_MSEG_SEGMENTS_U8))
    }

    #[must_use]
    pub(crate) const fn as_u8(self) -> u8 {
        self.0
    }

    #[must_use]
    fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

/// Inclusive loop bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MsegLoopRegion {
    pub(crate) start: MsegSegmentIndex,
    pub(crate) end: MsegSegmentIndex,
}

/// Editable values for one segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MsegSegment {
    pub(crate) time: Seconds,
    pub(crate) level: NormalizedValue,
    pub(crate) curve: BipolarValue,
}

impl Default for MsegSegment {
    fn default() -> Self {
        Self {
            time: Seconds::new(0.1),
            level: NormalizedValue::MIN,
            curve: BipolarValue::CENTER,
        }
    }
}

/// Fields changed during one editor frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MsegChanges {
    times: [bool; MAX_MSEG_SEGMENTS],
    levels: [bool; MAX_MSEG_SEGMENTS],
    curves: [bool; MAX_MSEG_SEGMENTS],
}

impl MsegChanges {
    #[must_use]
    pub(crate) fn time_changed(self, index: MsegSegmentIndex) -> bool {
        self.times[index.as_usize()]
    }

    #[must_use]
    pub(crate) fn level_changed(self, index: MsegSegmentIndex) -> bool {
        self.levels[index.as_usize()]
    }

    #[must_use]
    pub(crate) fn curve_changed(self, index: MsegSegmentIndex) -> bool {
        self.curves[index.as_usize()]
    }

    #[must_use]
    fn any_changed(self) -> bool {
        self.times.iter().any(|changed| *changed)
            || self.levels.iter().any(|changed| *changed)
            || self.curves.iter().any(|changed| *changed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragTarget {
    Node(MsegSegmentIndex),
    Curve(MsegSegmentIndex),
}

#[derive(Debug, Clone, Copy)]
struct DragState {
    target: DragTarget,
    pointer_origin: Pos2,
    segment_origin: MsegSegment,
    next_time_origin: Option<Seconds>,
    display_time: Seconds,
    curve_direction: EnvelopeCurveDirection,
}

/// Graphical MSEG editor with draggable segment endpoints and curve handles.
pub(crate) struct MsegEditor<'a> {
    segments: &'a mut [MsegSegment; MAX_MSEG_SEGMENTS],
    segment_count: MsegSegmentCount,
    sustain_segment: MsegSegmentIndex,
    loop_region: Option<MsegLoopRegion>,
    accent_color: Color32,
    width: f32,
    height: f32,
}

impl<'a> MsegEditor<'a> {
    #[must_use]
    pub(crate) fn new(
        segments: &'a mut [MsegSegment; MAX_MSEG_SEGMENTS],
        segment_count: MsegSegmentCount,
        sustain_segment: MsegSegmentIndex,
        loop_region: Option<MsegLoopRegion>,
    ) -> Self {
        Self {
            segments,
            segment_count,
            sustain_segment,
            loop_region,
            accent_color: theme().colors.accent_green,
            width: 320.0,
            height: 150.0,
        }
    }

    #[must_use]
    pub(crate) fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    #[must_use]
    pub(crate) fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    #[must_use]
    pub(crate) fn show(self, ui: &mut Ui) -> Option<MsegChanges> {
        let id = ui.id().with("mseg_editor");
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), Sense::click_and_drag());
        super::controls::expose(&response, egui::WidgetType::Other, "MSEG editor", None);

        let t = theme();
        let painter = ui.painter();
        painter.rect_filled(rect, t.style.corner_radius, t.colors.bg_widget);
        painter.rect_stroke(
            rect,
            t.style.corner_radius,
            Stroke::new(t.style.border_width, t.colors.text_dim.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );

        let inner = rect.shrink2(Vec2::new(8.0, 10.0));
        draw_grid(painter, inner);

        let active_count = self.segment_count.as_usize();
        let display_time = display_duration(self.segments, active_count);
        let node_positions =
            segment_node_positions(self.segments, active_count, inner, display_time);
        let curve_positions =
            curve_handle_positions(self.segments, active_count, inner, display_time);

        if let Some(region) = self.loop_region {
            draw_loop_region(
                painter,
                inner,
                self.segments,
                active_count,
                display_time,
                region,
                self.accent_color,
            );
        }
        draw_sustain_marker(
            painter,
            inner,
            &node_positions,
            active_count,
            self.sustain_segment,
        );
        draw_curves(
            painter,
            inner,
            self.segments,
            active_count,
            display_time,
            self.accent_color,
        );

        let to_local = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .map(|transform| transform.inverse());
        let to_local_pos =
            |position: Pos2| to_local.map_or(position, |transform| transform * position);
        let pointer = ui
            .input(|input| input.pointer.hover_pos())
            .map(to_local_pos);
        let dragging: Option<DragState> = ui.memory(|memory| memory.data.get_temp(id));
        let hovered = pointer.and_then(|position| {
            nearest_target(position, &node_positions, &curve_positions, active_count)
        });

        draw_handles(
            ui,
            painter,
            self.segments,
            active_count,
            &node_positions,
            &curve_positions,
            hovered,
            dragging.map(|state| state.target),
            self.accent_color,
        );

        if response.drag_started()
            && let (Some(target), Some(pointer_origin)) = (hovered, pointer)
        {
            let index = target_index(target);
            let next_time_origin = (index.as_usize() + 1 < active_count)
                .then(|| self.segments[index.as_usize() + 1].time);
            let curve_direction = segment_curve_direction(self.segments, index);
            ui.memory_mut(|memory| {
                memory.data.insert_temp(
                    id,
                    DragState {
                        target,
                        pointer_origin,
                        segment_origin: self.segments[index.as_usize()],
                        next_time_origin,
                        display_time,
                        curve_direction,
                    },
                );
            });
        }

        let mut changes = MsegChanges::default();
        if let Some(drag) = dragging
            && let Some(position) = ui
                .input(|input| input.pointer.interact_pos())
                .map(to_local_pos)
        {
            apply_drag(
                self.segments,
                active_count,
                inner,
                drag,
                position,
                &mut changes,
            );
        }

        if response.drag_stopped() {
            ui.memory_mut(|memory| memory.data.remove::<DragState>(id));
        }
        if hovered.is_some() || dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        if changes.any_changed() {
            Some(changes)
        } else {
            None
        }
    }
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect) {
    let color = theme().colors.text_dim.gamma_multiply(0.14);
    for step in 1..5 {
        let fraction = step as f32 / 5.0;
        let x = egui::lerp(rect.x_range(), fraction);
        let y = egui::lerp(rect.y_range(), fraction);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, color),
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, color),
        );
    }
}

fn display_duration(segments: &[MsegSegment; MAX_MSEG_SEGMENTS], active_count: usize) -> Seconds {
    let total = segments[..active_count]
        .iter()
        .map(|segment| segment.time.as_f32())
        .sum::<f32>();
    Seconds::new(total.max(MIN_SEGMENT_TIME))
}

fn time_to_x(time: Seconds, display_time: Seconds, rect: egui::Rect) -> f32 {
    rect.left() + (time.as_f32() / display_time.as_f32()) * rect.width()
}

fn level_to_y(level: NormalizedValue, rect: egui::Rect) -> f32 {
    rect.bottom() - level.as_f32() * rect.height()
}

fn segment_node_positions(
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    rect: egui::Rect,
    display_time: Seconds,
) -> [Pos2; MAX_MSEG_SEGMENTS] {
    let mut positions = [Pos2::ZERO; MAX_MSEG_SEGMENTS];
    let mut elapsed = Seconds::ZERO;
    for (index, segment) in segments[..active_count].iter().enumerate() {
        elapsed = elapsed + segment.time;
        positions[index] = Pos2::new(
            time_to_x(elapsed, display_time, rect),
            level_to_y(segment.level, rect),
        );
    }
    positions
}

fn curve_handle_positions(
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    rect: egui::Rect,
    display_time: Seconds,
) -> [Pos2; MAX_MSEG_SEGMENTS] {
    let mut positions = [Pos2::ZERO; MAX_MSEG_SEGMENTS];
    let mut elapsed = Seconds::ZERO;
    let mut start_level = NormalizedValue::MIN;
    for (index, segment) in segments[..active_count].iter().enumerate() {
        let midpoint = elapsed + segment.time * 0.5;
        let level = synth_modules::math::interpolate_with_curve(
            start_level.as_f32(),
            segment.level.as_f32(),
            0.5,
            segment.curve.as_f32(),
        );
        positions[index] = Pos2::new(
            time_to_x(midpoint, display_time, rect),
            level_to_y(NormalizedValue::new(level), rect),
        );
        elapsed = elapsed + segment.time;
        start_level = segment.level;
    }
    positions
}

fn draw_curves(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    display_time: Seconds,
    accent: Color32,
) {
    let mut points = Vec::with_capacity(active_count * CURVE_STEPS + 1);
    let mut elapsed = Seconds::ZERO;
    let mut start_level = NormalizedValue::MIN;
    points.push(Pos2::new(rect.left(), rect.bottom()));
    for segment in &segments[..active_count] {
        for step in 1..=CURVE_STEPS {
            let phase = step as f32 / CURVE_STEPS as f32;
            let level = synth_modules::math::interpolate_with_curve(
                start_level.as_f32(),
                segment.level.as_f32(),
                phase,
                segment.curve.as_f32(),
            );
            let time = elapsed + segment.time * phase;
            points.push(Pos2::new(
                time_to_x(time, display_time, rect),
                level_to_y(NormalizedValue::new(level), rect),
            ));
        }
        elapsed = elapsed + segment.time;
        start_level = segment.level;
    }

    super::controls::paint_envelope_curve_fill(
        painter,
        &points,
        rect.bottom(),
        accent.gamma_multiply(0.12),
    );
    painter.add(Shape::line(points, Stroke::new(2.25, accent)));
}

fn draw_loop_region(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    display_time: Seconds,
    region: MsegLoopRegion,
    accent: Color32,
) {
    let start = region.start.as_usize().min(active_count.saturating_sub(1));
    let end = region
        .end
        .as_usize()
        .clamp(start, active_count.saturating_sub(1));
    let start_time = segments[..start]
        .iter()
        .fold(Seconds::ZERO, |sum, segment| sum + segment.time);
    let end_time = segments[..=end]
        .iter()
        .fold(Seconds::ZERO, |sum, segment| sum + segment.time);
    let loop_rect = egui::Rect::from_x_y_ranges(
        time_to_x(start_time, display_time, rect)..=time_to_x(end_time, display_time, rect),
        rect.y_range(),
    );
    painter.rect_filled(loop_rect, 0.0, accent.gamma_multiply(0.08));
    painter.rect_stroke(
        loop_rect,
        0.0,
        Stroke::new(1.0, accent.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        loop_rect.left_top() + Vec2::new(3.0, 2.0),
        egui::Align2::LEFT_TOP,
        "LOOP",
        theme().fonts.small(),
        accent.gamma_multiply(0.85),
    );
}

fn draw_sustain_marker(
    painter: &egui::Painter,
    rect: egui::Rect,
    node_positions: &[Pos2; MAX_MSEG_SEGMENTS],
    active_count: usize,
    sustain: MsegSegmentIndex,
) {
    let index = sustain.as_usize();
    if index >= active_count {
        return;
    }
    let x = node_positions[index].x;
    let color = theme().colors.accent_yellow;
    painter.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        Stroke::new(1.5, color.gamma_multiply(0.75)),
    );
    painter.text(
        Pos2::new(x - 3.0, rect.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        "S",
        theme().fonts.small(),
        color,
    );
}

fn nearest_target(
    pointer: Pos2,
    nodes: &[Pos2; MAX_MSEG_SEGMENTS],
    curves: &[Pos2; MAX_MSEG_SEGMENTS],
    active_count: usize,
) -> Option<DragTarget> {
    let mut nearest = None;
    let mut nearest_distance = HIT_RADIUS;
    for index in 0..active_count {
        let segment_index = MsegSegmentIndex::new(u8::try_from(index).unwrap_or(15));
        for (position, target) in [
            (nodes[index], DragTarget::Node(segment_index)),
            (curves[index], DragTarget::Curve(segment_index)),
        ] {
            let distance = position.distance(pointer);
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest = Some(target);
            }
        }
    }
    nearest
}

fn target_index(target: DragTarget) -> MsegSegmentIndex {
    match target {
        DragTarget::Node(index) | DragTarget::Curve(index) => index,
    }
}

fn segment_curve_direction(
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    index: MsegSegmentIndex,
) -> EnvelopeCurveDirection {
    let slot = index.as_usize();
    let start_level = slot
        .checked_sub(1)
        .map_or(NormalizedValue::MIN, |previous| segments[previous].level);
    if segments[slot].level >= start_level {
        EnvelopeCurveDirection::Rising
    } else {
        EnvelopeCurveDirection::Falling
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_handles(
    ui: &Ui,
    painter: &egui::Painter,
    segments: &[MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    nodes: &[Pos2; MAX_MSEG_SEGMENTS],
    curves: &[Pos2; MAX_MSEG_SEGMENTS],
    hovered: Option<DragTarget>,
    dragging: Option<DragTarget>,
    accent: Color32,
) {
    for index in 0..active_count {
        let segment_index = MsegSegmentIndex::new(u8::try_from(index).unwrap_or(15));
        let node_target = DragTarget::Node(segment_index);
        let curve_target = DragTarget::Curve(segment_index);
        let node_active = hovered == Some(node_target) || dragging == Some(node_target);
        let curve_active = hovered == Some(curve_target) || dragging == Some(curve_target);

        painter.circle_filled(
            nodes[index],
            if node_active { 6.0 } else { 4.0 },
            if dragging == Some(node_target) {
                Color32::WHITE
            } else {
                accent
            },
        );
        super::controls::paint_envelope_curve_handle(painter, curves[index], accent, curve_active);

        if node_active {
            super::tooltip::draw_tooltip_above(
                ui,
                nodes[index],
                &format!(
                    "{}  •  {:.0}%",
                    segments[index].time,
                    segments[index].level.as_f32() * 100.0
                ),
                accent,
            );
        } else if curve_active {
            super::tooltip::draw_tooltip_above(
                ui,
                curves[index],
                &format!("Curve {:+.2}", segments[index].curve.as_f32()),
                accent,
            );
        }
    }
}

fn apply_drag(
    segments: &mut [MsegSegment; MAX_MSEG_SEGMENTS],
    active_count: usize,
    rect: egui::Rect,
    drag: DragState,
    pointer: Pos2,
    changes: &mut MsegChanges,
) {
    let index = target_index(drag.target);
    let slot = index.as_usize();
    match drag.target {
        DragTarget::Node(_) => {
            let delta = pointer - drag.pointer_origin;
            let requested_delta = delta.x / rect.width() * drag.display_time.as_f32();
            let minimum_delta = MIN_SEGMENT_TIME - drag.segment_origin.time.as_f32();
            let maximum_delta = drag.next_time_origin.map_or_else(
                || MAX_SEGMENT_TIME.as_f32() - drag.segment_origin.time.as_f32(),
                |next_time| next_time.as_f32() - MIN_SEGMENT_TIME,
            );
            let time_delta = requested_delta.clamp(minimum_delta, maximum_delta);
            let new_time = (drag.segment_origin.time.as_f32() + time_delta).max(MIN_SEGMENT_TIME);
            segments[slot].time = Seconds::new(new_time);
            changes.times[slot] = true;

            if slot + 1 < active_count
                && let Some(next_origin) = drag.next_time_origin
            {
                let next_time = (next_origin.as_f32() - time_delta).max(MIN_SEGMENT_TIME);
                segments[slot + 1].time = Seconds::new(next_time);
                changes.times[slot + 1] = true;
            }

            let level = 1.0 - (pointer.y - rect.top()) / rect.height();
            segments[slot].level = NormalizedValue::new(level);
            changes.levels[slot] = true;
        }
        DragTarget::Curve(_) => {
            let delta_y = pointer.y - drag.pointer_origin.y;
            segments[slot].curve = envelope_curve_after_vertical_drag(
                drag.segment_origin.curve,
                delta_y,
                rect.height(),
                drag.curve_direction,
            );
            changes.curves[slot] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_count_is_always_valid() {
        assert_eq!(MsegSegmentCount::new(0).as_u8(), 1);
        assert_eq!(MsegSegmentCount::new(17).as_u8(), 16);
    }

    #[test]
    fn curve_direction_tracks_each_segments_level_change() {
        let mut segments = [MsegSegment::default(); MAX_MSEG_SEGMENTS];
        segments[0].level = NormalizedValue::new(0.25);
        segments[1].level = NormalizedValue::new(0.75);
        segments[2].level = NormalizedValue::new(0.1);

        assert_eq!(
            segment_curve_direction(&segments, MsegSegmentIndex::new(1)),
            EnvelopeCurveDirection::Rising
        );
        assert_eq!(
            segment_curve_direction(&segments, MsegSegmentIndex::new(2)),
            EnvelopeCurveDirection::Falling
        );
    }

    #[test]
    fn node_drag_moves_boundary_and_preserves_adjacent_total() {
        let mut segments = [MsegSegment::default(); MAX_MSEG_SEGMENTS];
        segments[0].time = Seconds::new(0.25);
        segments[1].time = Seconds::new(0.75);
        let segment_origin = segments[0];
        let next_time_origin = segments[1].time;
        let mut changes = MsegChanges::default();
        let rect = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        apply_drag(
            &mut segments,
            2,
            rect,
            DragState {
                target: DragTarget::Node(MsegSegmentIndex::new(0)),
                pointer_origin: Pos2::new(25.0, 50.0),
                segment_origin,
                next_time_origin: Some(next_time_origin),
                display_time: Seconds::new(1.0),
                curve_direction: EnvelopeCurveDirection::Rising,
            },
            Pos2::new(50.0, 25.0),
            &mut changes,
        );

        assert!((segments[0].time.as_f32() - 0.5).abs() < 0.001);
        assert!((segments[1].time.as_f32() - 0.5).abs() < 0.001);
        assert!((segments[0].level.as_f32() - 0.75).abs() < 0.001);
        assert!(changes.time_changed(MsegSegmentIndex::new(0)));
        assert!(changes.time_changed(MsegSegmentIndex::new(1)));
        assert!(changes.level_changed(MsegSegmentIndex::new(0)));
    }

    #[test]
    fn rising_curve_drag_follows_the_pointer() {
        let mut segments = [MsegSegment::default(); MAX_MSEG_SEGMENTS];
        segments[0].level = NormalizedValue::MAX;
        let segment_origin = segments[0];
        let mut changes = MsegChanges::default();
        let rect = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        apply_drag(
            &mut segments,
            1,
            rect,
            DragState {
                target: DragTarget::Curve(MsegSegmentIndex::new(0)),
                pointer_origin: Pos2::new(50.0, 50.0),
                segment_origin,
                next_time_origin: None,
                display_time: Seconds::new(0.1),
                curve_direction: EnvelopeCurveDirection::Rising,
            },
            Pos2::new(50.0, 25.0),
            &mut changes,
        );

        assert_eq!(segments[0].curve, BipolarValue::MIN);
        assert!(changes.curve_changed(MsegSegmentIndex::new(0)));
    }

    #[test]
    fn falling_curve_drag_follows_the_pointer() {
        let mut segments = [MsegSegment::default(); MAX_MSEG_SEGMENTS];
        segments[0].level = NormalizedValue::MAX;
        segments[1].level = NormalizedValue::MIN;
        let segment_origin = segments[1];
        let mut changes = MsegChanges::default();
        let rect = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        apply_drag(
            &mut segments,
            2,
            rect,
            DragState {
                target: DragTarget::Curve(MsegSegmentIndex::new(1)),
                pointer_origin: Pos2::new(50.0, 50.0),
                segment_origin,
                next_time_origin: None,
                display_time: Seconds::new(0.2),
                curve_direction: EnvelopeCurveDirection::Falling,
            },
            Pos2::new(50.0, 25.0),
            &mut changes,
        );

        assert_eq!(segments[1].curve, BipolarValue::MAX);
        assert!(changes.curve_changed(MsegSegmentIndex::new(1)));
    }
}
