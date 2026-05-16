//! Symbolic composition helpers used by the Group-B MCP tools.
//!
//! All operations are pure note manipulations — no audio rendering, no
//! engine snapshot. They share the chord/scale theory tables from
//! [`crate::harmony`] so chord identification and chord generation stay
//! consistent (the parser uses the same suffix table that the identifier
//! emits).
//!
//! - [`chord_gen::generate_chord`] — symbol → MIDI notes with voicing.
//! - [`transpose::transpose_pitches`] — shift pitches by semitones with an
//!   optional scale-snap.
//! - [`quantize_scale::quantize_pitches_to_scale`] — snap pitches to the
//!   nearest scale degree.
//! - [`quantize_grid::quantize_grid`] — snap start ticks to a grid with
//!   optional swing and humanization.

pub mod chord_gen;
pub mod quantize_grid;
pub mod quantize_scale;
pub mod transpose;

pub use chord_gen::{ChordVoicing, GenerateChordError, GeneratedChord, generate_chord};
pub use quantize_grid::{GridQuantizeOptions, GridQuantizeResult, NoteTiming, quantize_grid};
pub use quantize_scale::{
    ScaleConstraint, ScaleQuantizeOptions, ScaleQuantizeResult, ScaleTieBreak,
    quantize_pitches_to_scale,
};
pub use transpose::{TransposeResult, transpose_pitches};
