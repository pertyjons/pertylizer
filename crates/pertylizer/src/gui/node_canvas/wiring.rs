//! Generic drag-to-connect wire FSM.
//!
//! Extracted verbatim (in behaviour) from the Note Grid's hardened
//! implementation and generalised over the caller's port-identity and
//! connection types. A port emits a [`WireEvent`] while its node is drawn; the
//! events, plus the world-space port anchors recorded that frame, are resolved
//! once by [`resolve_wire_events`] against a caller-supplied validator, and each
//! completed connection is handed back through a `commit` sink.
//!
//! `Port` fully identifies one endpoint (its node *and* its port), so the FSM
//! needs no separate node id: the caller's `open_connection` closure owns all
//! semantics (same-node rejection, port-type compatibility, occupancy, cycles).

use std::collections::HashMap;
use std::hash::Hash;

use eframe::egui::{self, Pos2, Rect, Response, Ui};

use crate::gui::scene_canvas;

/// A wire drag/click in progress, anchored at its source port.
#[derive(Clone)]
pub(crate) struct WireDrag<P> {
    /// The source endpoint the wire is being drawn from.
    pub(crate) from: P,
    /// World-space position of the source port — the cable's fixed tail.
    pub(crate) from_pos: Pos2,
    /// Started by an actual drag (completes on pointer release) rather than a
    /// click (completes on a second click).
    pub(crate) armed_by_drag: bool,
}

/// A port interaction gathered while drawing the nodes, resolved afterwards.
#[derive(Clone)]
pub(crate) enum WireEvent<P> {
    /// A drag started on a port (completes on pointer release).
    DragFrom(P, Pos2),
    /// A port was clicked (starts, or completes, a click-click wire).
    Click(P, Pos2),
}

/// Where a dragged wire may land: the port anchors recorded this frame, the
/// canvas bounds a release must fall inside, and the per-port hit radius.
pub(crate) struct DropTargets<'a, P> {
    /// World-space port centres recorded while the nodes were drawn.
    pub(crate) anchors: &'a HashMap<P, Pos2>,
    /// A release outside this world-space rect (over a side panel or header)
    /// never connects.
    pub(crate) world_rect: Rect,
    /// Hit radius (world units) around a port anchor.
    pub(crate) drop_radius: f32,
}

/// Translate a port widget's `response` into a wire event and record it: a
/// primary-button drag starts a wire (completes on release), a plain click
/// starts or completes a click-click wire. Both editors call this from every
/// port they draw so the "which gesture is a wire" rule lives in one place. A
/// secondary-button drag is deliberately ignored (it pans the canvas).
pub(crate) fn push_port_event<P: Copy>(
    events: &mut Vec<WireEvent<P>>,
    response: &Response,
    port: P,
    anchor: Pos2,
) {
    if response.drag_started_by(egui::PointerButton::Primary) {
        events.push(WireEvent::DragFrom(port, anchor));
    } else if response.clicked() {
        events.push(WireEvent::Click(port, anchor));
    }
}

/// Resolve this frame's port interactions against the pending wire, then handle
/// drag-release completion by hit-testing the recorded anchors.
///
/// `open_connection(from, to)` returns the connection a drop from `from` onto
/// `to` would create, or `None` when it is invalid/occupied — the same
/// predicate the port highlight uses, so a drag only completes on a port that
/// would actually accept it. `commit` receives each completed connection.
pub(crate) fn resolve_wire_events<P, C>(
    ui: &Ui,
    pending: &mut Option<WireDrag<P>>,
    targets: DropTargets<'_, P>,
    events: Vec<WireEvent<P>>,
    open_connection: impl Fn(P, P) -> Option<C>,
    mut commit: impl FnMut(C),
) where
    P: Copy + Eq + Hash,
{
    for event in events {
        match event {
            WireEvent::DragFrom(port, pos) => {
                *pending = Some(WireDrag {
                    from: port,
                    from_pos: pos,
                    armed_by_drag: true,
                });
            }
            WireEvent::Click(port, pos) => {
                let completed = pending.as_ref().and_then(|p| open_connection(p.from, port));
                if let Some(connection) = completed {
                    commit(connection);
                    *pending = None;
                } else {
                    // Start (or restart) a click-click wire from this port.
                    *pending = Some(WireDrag {
                        from: port,
                        from_pos: pos,
                        armed_by_drag: false,
                    });
                }
            }
        }
    }

    // A drag-started wire completes (or cancels) on pointer release. The drop
    // point must land on the canvas — a release over a side panel or header must
    // not connect to a nearby (possibly off-screen) anchor.
    if ui.input(|i| i.pointer.primary_released())
        && pending.as_ref().is_some_and(|p| p.armed_by_drag)
    {
        let from = pending.as_ref().map(|p| p.from);
        let target = ui
            .input(|i| i.pointer.interact_pos())
            .map(|screen| scene_canvas::screen_to_world(ui, screen))
            .filter(|world| targets.world_rect.contains(*world))
            .zip(from)
            .and_then(|(world, from)| {
                targets
                    .anchors
                    .iter()
                    .filter(|(port, center)| {
                        center.distance(world) <= targets.drop_radius
                            && open_connection(from, **port).is_some()
                    })
                    .min_by(|(_, a), (_, b)| a.distance(world).total_cmp(&b.distance(world)))
                    .map(|(port, _)| *port)
            });
        if let (Some(drag), Some(port)) = (pending.take(), target)
            && let Some(connection) = open_connection(drag.from, port)
        {
            commit(connection);
        }
    }
}
