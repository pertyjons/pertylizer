//! View helpers for the sequencer.
//!
//! These structures are for rendering patterns in different views (tracker, piano roll, etc.)
//! They are NOT part of the core data model.

#[cfg(feature = "gui-egui")]
pub mod render;
pub mod state;
pub mod tracker;

#[cfg(feature = "gui-egui")]
pub use render::*;
pub use state::*;
pub use tracker::*;
