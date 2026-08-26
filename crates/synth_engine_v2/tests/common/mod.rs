//! Shared fixtures for the Phase 1 contract tests.
//!
//! Compiled into every test binary that declares `mod common`, so a helper only one
//! binary uses is not dead code — it is unused *here*.
#![allow(dead_code, reason = "each test binary uses a subset of these fixtures")]

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PlanDeclarations, PortId, SignalDomain,
};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::{HostProfile, RenderLimits};
use synth_engine_v2::quantities::{ChannelLayout, SampleRate};
use synth_engine_v2::time::FrameCount;

/// The source node every fixture patches into the output.
pub const SOURCE: NodeId = NodeId::new(1);
/// The output node.
pub const OUTPUT: NodeId = NodeId::new(2);

/// A rate that is certainly valid.
pub fn rate(hz: f32) -> SampleRate {
    SampleRate::new(hz).expect("test rate is valid")
}

/// A harness profile at 48 kHz with the given maximum block and layout.
pub fn profile(block: u64, layout: ChannelLayout) -> HostProfile {
    HostProfile::harness(rate(48_000.0), FrameCount::new(block), layout)
        .expect("the default harness profile is valid")
}

/// The default limits for a profile, as a starting point for overriding one group.
pub fn defaults_for(profile: &HostProfile) -> RenderLimits {
    RenderLimits::engine_defaults(profile.capabilities()).expect("defaults are valid")
}

/// A plan with one source patched into an output.
pub fn source_plan(kind: IrNodeKind) -> GraphIr {
    GraphIr::builder()
        .node(SOURCE, kind, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a source into an output is a readable plan")
}

/// A plan that declares what it needs but has no nodes.
pub fn declaring(declarations: PlanDeclarations) -> GraphIr {
    GraphIr::builder()
        .declaring(declarations)
        .build()
        .expect("declarations alone are a readable plan")
}

/// Admit a plan, expecting it to fit.
pub fn admit(ir: &GraphIr, profile: HostProfile) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("the plan fits this profile")
}

/// Admit a plan, expecting a refusal.
pub fn refuse(ir: &GraphIr, profile: HostProfile) -> CompileError {
    compile(ir, &RenderConfig::new(profile))
        .into_plan()
        .expect_err("the plan must be refused")
}

/// Declarations for a plan that plays notes from one compiled source.
///
/// A plan that starts notes must say who starts them: ADR-0046 partitions hold entitlements
/// across admitted note-on producers and ADR-0047 partitions identity ranges across a
/// superset of those, and neither partition can be computed from a plan that names none.
/// Compiled sources declare no hold — their releases use plan entitlements.
pub fn compiled_notes(simultaneous: u32) -> synth_engine_v2::ir::PlanDeclarations {
    synth_engine_v2::ir::PlanDeclarations {
        note_producers: vec![synth_engine_v2::ir::NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(simultaneous),
            simultaneous_holds: synth_engine_v2::quantities::EventCount::NONE,
        }],
        ..synth_engine_v2::ir::PlanDeclarations::default()
    }
}
