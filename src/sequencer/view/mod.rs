//! View helpers for the sequencer.
//!
//! These structures are for rendering patterns in different views (tracker, piano roll, etc.)
//! They are NOT part of the core data model.
//!
//! ## View Adapter Pattern
//!
//! The view layer uses the View Adapter pattern to separate data from presentation:
//! - `render::ColumnType` - defines column types for the tracker
//! - `render::render_cell_text()` - converts TrackCell to display text
//! - `render::cell_color()` - determines cell colors
//!
//! This allows the same data to be rendered differently (tracker vs piano roll).

pub mod input;
#[cfg(feature = "gui-egui")]
pub mod render;
pub mod state;
pub mod tracker;

pub use input::*;
#[cfg(feature = "gui-egui")]
pub use render::*;
pub use state::*;
pub use tracker::*;
