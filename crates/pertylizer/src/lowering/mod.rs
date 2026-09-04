//! The Phase 4 legacy-project lowerer.
//!
//! Lowers a saved project into the experimental Core V2 render core's IR, without touching
//! the GUI or any live engine state. The master plan puts this **outside** the V2 core crate
//! and lets it depend on the current project, sequencer, module and session types; V2 itself
//! depends on none of them and does not know this module exists.
//!
//! # Why it lives behind a non-default feature
//!
//! `synth_engine_v2`'s `crate_boundary` test carries the Phase 1 exit gate's claim that the
//! experimental crate "can be deleted without affecting V1 behavior or public APIs". Phase 4
//! is the first consumer that is not a measurement harness, so that claim needs a boundary
//! rather than a deletion. The boundary is the non-default `v2-lowering` feature: with
//! default features the workspace still has **no** normal or build edge to the experimental
//! crate — measured with `cargo tree --edges normal --invert`, not assumed — so a shipping
//! build is exactly what it was, and V1 remains the default renderer as the exit gate
//! requires.
//!
//! # What it does not do
//!
//! It loads no files. Samples and other assets reach V2 as already-prepared immutable data,
//! per the work list, so nothing here opens a path. It does not write projects: the save and
//! load format is untouched and V2 is a consumer only.
//!
//! # What it cannot represent yet
//!
//! **A saved note's own pitch and velocity reach the render**, and `P04-R001`'s precondition
//! is discharged: the work list's "before rendering the first saved pitched note, close
//! P03-R003 with minimum typed pitch and velocity payload semantics" is met, so a saved note
//! renders here rather than being refused.
//!
//! What is still unrepresented is **how V1 composes velocity**. V1 applies one saved velocity
//! twice — once through the envelope's own sensitivity and again through the voice output's —
//! and V2 applies it as one scale on the envelope. The work list says closing that residual
//! "does not decide Phase 6's tuning or expression-composition model", so the outcome is
//! marked [`diagnostics::Fidelity::UnsupportedScope`] and the A/B path refuses to compare it
//! for parity. That refusal is the fails-closed mechanism the phase-exit rule requires.

pub mod diagnostics;
pub mod graph;
pub mod identity;
pub mod performance;
pub mod render;

#[cfg(test)]
mod tests;

pub use diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};
pub use graph::{LoweredGraph, lower_voice_patch};
pub use identity::{IdentityError, ResolvedIdentities};
pub use performance::{LoweredPerformance, lower_performance};
pub use render::{SmokeRender, smoke_render};
