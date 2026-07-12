//! Generic node-canvas layer shared by the patch editor and the Note Grid.
//!
//! Phase 5 extracted the leaf widgets both editors paint with (`scene_canvas`,
//! the `ModuleFrame` chrome, `PortWidget`, the `widgets::cable` painters,
//! `ModuleWidth`). This module converges the layer *above* those leaves — the
//! parts the two editors had each reimplemented in parallel:
//!
//! - the drag-to-connect **wire FSM** ([`wiring`]) — pending-wire state, the
//!   per-frame port-interaction events, and drop resolution, extracted from the
//!   Note Grid's hardened implementation (primary-button-only drags,
//!   canvas-confined drops, occupied-port-aware highlighting);
//!
//! Everything here is generic over the caller's node/port/connection types via
//! `Copy + Eq + Hash` bounds and closures, so audio-graph specifics
//! (groups, effect chains, mod-matrix markers, cycle rules, connection types)
//! stay in the calling editor.

pub(crate) mod wiring;

pub(crate) use wiring::{DropTargets, WireDrag, WireEvent, push_port_event, resolve_wire_events};
