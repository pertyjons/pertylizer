//! Experimental Sound Core V2 render core.
//!
//! This crate started as Phase 1 of the Core V2 migration and owns the render-core
//! contracts Phase 0A accepted — the time and quantum types, the host profile and its
//! admission, a compiler IR, and a renderer that splits any caller block into the fixed
//! internal quantum. Phase 2 adds graph validation over a declared port table. Phase 3
//! begins with a compiled-event scheduler over engine-domain `SampleTime`. The crate does
//! not connect to projects, does not compile V1 patches, and does not map or schedule live
//! hardware events.
//!
//! # It can be deleted
//!
//! No other workspace crate depends on this one, and this one depends on no GUI, MCP,
//! OSC, CPAL, filesystem, or project-loading crate. Deleting the directory and its
//! workspace entry removes it without affecting V1 behaviour or any public API. Two
//! manifest tests assert both halves, because "experimental" is a claim about coupling
//! and coupling is what rots first.
//!
//! # What the contracts are, and where they live
//!
//! | Concern | Module | Contract |
//! |---------|--------|----------|
//! | Time, quantum, epoch | [`time`] | ADR-0001, ADR-0032, ADR-0037 |
//! | Typed quantities | [`quantities`] | `HOST-INV-018` |
//! | The profile | [`profile`] | Host-profile specification |
//! | The IR | [`ir`] | Master plan, Phase 1 work list |
//! | The node registry | [`node`] | ADR-0004 |
//! | Admission | [`compile`] | `HOST-INV-006`, `HOST-INV-007`, `HOST-INV-015` |
//! | The report | [`report`] | `HOST-INV-006` |
//! | Diagnostics | [`diagnostics`] | ADR-0001 clause 16, ADR-0032 clauses 19-21 |
//! | The prepared plan | [`plan`] | Master plan, layer boundaries |
//! | Graph validation | [`validate`] | Master plan, Phase 2 work list; ADR-0002, ADR-0033 |
//! | The buffer arena | `arena` (private) | ADR-0005 |
//! | Rendering | [`render`] | ADR-0001 clauses 4-9, 11-14, 16 |
//! | Compiled scheduling | [`schedule`] | ADR-0001 clause 16; ADR-0032 clauses 17-21 |
//! | The stream's off-thread half | [`stream`] | ADR-0050 clause 9 |
//! | Transport activation | [`transport`] | ADR-0050 |
//! | Offline rendering | [`offline`] | ADR-0001 clauses 9-10 |
//!
//! # The quantum
//!
//! [`time::QUANTUM_FRAMES`] is 64, and since P02-T010 that is final rather than
//! provisional: ADR-0037's V1 proxy was inconclusive, and EVD-0012 re-measured the real
//! renderer at 32, 64, 128 and 256 frames across five builds that render bit-identical
//! audio. On the voice path this crate compiles, 64 costs 6% over 256 against a 15%
//! threshold and 32 costs 5.5% against a 2% one. The restriction that came with the
//! provisional value — no hand-unrolled kernel, no `Q`-specific buffer layout, no test
//! asserting a control rate in hertz — is discharged with it.
//!
//! # A worked example
//!
//! ```
//! use synth_engine_v2::compile::{RenderConfig, compile};
//! use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
//! use synth_engine_v2::offline::render_offline;
//! use synth_engine_v2::profile::HostProfile;
//! use synth_engine_v2::quantities::{Amplitude, ChannelLayout, SampleRate};
//! use synth_engine_v2::time::{FrameCount, PlanPosition};
//!
//! let profile = HostProfile::harness(
//!     SampleRate::new(48_000.0)?,
//!     FrameCount::new(512),
//!     ChannelLayout::Mono,
//! )?;
//!
//! let source = NodeId::new(1);
//! let output = NodeId::new(2);
//! let ir = GraphIr::builder()
//!     .node(
//!         source,
//!         IrNodeKind::Constant { level: Amplitude::new(0.25)? },
//!         ExecutionScope::Global,
//!     )
//!     .node(output, IrNodeKind::Output, ExecutionScope::Global)
//!     .connect((source, PortId::FIRST), (output, PortId::FIRST), SignalDomain::Audio)
//!     .build()?;
//!
//! let outcome = compile(&ir, &RenderConfig::new(profile));
//! // Every field is reported whether or not a plan came out.
//! assert_eq!(outcome.report().rows().len(), synth_engine_v2::report::ResourceField::COUNT);
//!
//! let plan = outcome.into_plan()?;
//! let rendered = render_offline(plan, FrameCount::new(128), PlanPosition::ZERO, &[])?;
//! assert_eq!(rendered.len(), 128);
//! assert!(rendered.iter().all(|sample| (*sample - 0.25).abs() < f32::EPSILON));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod admit;
mod arena;
pub mod compile;
pub mod diagnostics;
pub mod identity;
pub mod ir;
pub mod node;
pub mod offline;
pub mod plan;
pub mod profile;
pub mod publish;
pub mod quantities;
pub mod render;
pub mod report;
pub mod schedule;
pub mod session;
pub mod stream;
pub mod tempo;
pub mod time;
pub mod transport;
pub mod validate;

#[cfg(test)]
#[path = "tests/render_allocation.rs"]
mod render_allocation;

#[cfg(test)]
#[path = "tests/arena_reuse.rs"]
mod arena_reuse;

#[cfg(test)]
#[path = "tests/kernels.rs"]
mod kernel_tests;
