//! What compilation and rendering say when something is wrong.
//!
//! Three rules shape this module:
//!
//! - **A refusal names the field, both amounts, and the responsible object**
//!   (`HOST-INV-007`). A diagnostic a reader cannot act on is the failure mode this
//!   whole contract exists to remove.
//! - **A caller precondition returns a `Result`, not a `debug_assert`.** A debug
//!   assertion compiles out of the build that runs, so it cannot define release
//!   behaviour; it may only supplement one.
//! - **Every counter reaches the structured diagnostics report.** `LIMIT-0013`'s
//!   drop counters existed for years and reached no consumer at all, which is the
//!   specific failure the report is the control against.

use thiserror::Error;

use crate::ir::{IrError, IrObject};
use crate::profile::ProfileError;
use crate::quantities::ChannelLayout;
use crate::report::{ResourceAmount, ResourceField};
use crate::time::{FrameCount, TimeError};

/// Why a plan was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum CompileError {
    /// A plan asked for more than a render limit allows.
    ///
    /// Admission never truncates, clamps, or drops to make a plan fit: exceeding a
    /// limit never rewrites authored data, so the only outcomes are a prepared plan
    /// and this.
    #[error(
        "{field} exceeded: {requested} requested, {available} available, contributed most by \
         {responsible}"
    )]
    LimitExceeded {
        /// Which limit.
        field: ResourceField,
        /// What the plan asked for.
        requested: ResourceAmount,
        /// What the profile allows.
        available: ResourceAmount,
        /// The authored object that contributed most to the request.
        responsible: IrObject,
    },

    /// Several signals are patched into one input.
    ///
    /// Summing them is a fan-in policy, and choosing one belongs to Phase 2's graph
    /// validation. Refusing is the honest move: silently taking the first would be
    /// exactly the quiet reduction this contract exists to remove.
    #[error("{edges} edges reach {node} {port}; a fan-in policy is Phase 2's, so this is refused")]
    UnsupportedFanIn {
        /// The node whose input is over-subscribed.
        node: crate::ir::NodeId,
        /// Which input.
        port: crate::ir::PortId,
        /// How many edges reach it.
        edges: u32,
    },

    /// A plan declares more than one output.
    ///
    /// Phase 1 renders one. Taking the first and ignoring the rest would be a silent
    /// choice about which output the plan has.
    #[error("the plan declares {outputs} output nodes; this phase renders one")]
    MultipleOutputs {
        /// How many outputs the plan declares.
        outputs: u32,
    },

    /// An edge reaches an output port this phase does not have.
    ///
    /// The alternative is worse than a refusal: lowering only reads the first port, so
    /// such an edge would compile and render silence with nothing said.
    #[error("{node} {port} is not an output port this phase renders")]
    UnsupportedOutputPort {
        /// The output node.
        node: crate::ir::NodeId,
        /// The port the edge reached.
        port: crate::ir::PortId,
    },

    /// The IR could not be read.
    #[error("the plan could not be read: {0}")]
    Ir(#[from] IrError),

    /// A report row compared two different units, which is an internal defect.
    ///
    /// It is surfaced rather than swallowed because a mismatch means the comparison
    /// that decides admission did not happen for that field.
    #[error("internal: the report row for {field} compares mismatched units")]
    ReportUnitMismatch {
        /// The field whose row is malformed.
        field: ResourceField,
    },

    /// Preparation could not issue a stream epoch.
    #[error("preparation failed: {0}")]
    Time(#[from] TimeError),

    /// The profile itself was invalid.
    #[error("profile invalid: {0}")]
    Profile(#[from] ProfileError),
}

/// Something worth saying about a plan that was nevertheless admitted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompileWarning {
    /// An advisory budget was exceeded.
    ///
    /// `HOST-INV-015`: compilation continues. The predicted and permitted values
    /// both travel with the warning, because a warning that does not say by how
    /// much is a warning nobody can act on.
    AdvisoryBudgetExceeded {
        /// Which advisory field.
        field: ResourceField,
        /// What was predicted.
        predicted: ResourceAmount,
        /// What the budget permits.
        permitted: ResourceAmount,
        /// The object contributing most to the prediction.
        contributor: IrObject,
    },
}

impl std::fmt::Display for CompileWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AdvisoryBudgetExceeded {
                field,
                predicted,
                permitted,
                contributor,
            } => write!(
                f,
                "{field} is advisory and was exceeded: {predicted} predicted against {permitted} \
                 permitted, contributed most by {contributor}"
            ),
        }
    }
}

/// Why a render call could not be served.
///
/// Every variant is returned **before** renderer state or output is mutated, except
/// the two stream-contract faults, which are terminal for the epoch and say so:
/// they silence the output, invalidate the carries, and publish `needs_reprepare`.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum RenderError {
    /// The caller delivered more frames than the profile's maximum block.
    ///
    /// ADR-0021 part 3's terminal stream-contract fault: output silence, both
    /// carries invalidated, `needs_reprepare` published, nothing allocated. The
    /// engine makes no claim that the old input epoch continues, so recovery is
    /// re-preparation.
    #[error(
        "callback of {frames} exceeds maximum_block_size {maximum}; the stream contract is broken \
         and the epoch is over"
    )]
    OversizedCallback {
        /// What the caller delivered.
        frames: FrameCount,
        /// What the profile admits.
        maximum: FrameCount,
    },

    /// The output slice does not match the frames and layout it claims.
    #[error(
        "output buffer holds {samples} samples, but {frames} frames of {layout} needs {needed}"
    )]
    OutputBufferShape {
        /// Samples the caller provided.
        samples: usize,
        /// Frames the caller asked for.
        frames: usize,
        /// The stream's layout.
        layout: ChannelLayout,
        /// Samples that layout requires for those frames.
        needed: usize,
    },

    /// One quantum was presented with more events than it admits.
    ///
    /// Rejected before anything is mutated. Phase 1 and Phase 2 do **not** defer:
    /// `HOST-INV-021` is deferred to Phase 3, which owns the ingress streams and the
    /// bounded deferred store that a deferral mechanism needs. Until then an
    /// over-full quantum is a caller contract violation, and the one rule that binds
    /// meanwhile is negative — nothing may allocate to absorb it.
    #[error(
        "quantum {quantum_index} was presented with {requested} events against \
         max_events_per_quantum {available}"
    )]
    QuantumEventOverflow {
        /// Which quantum, absolutely.
        quantum_index: u64,
        /// Events due in it.
        requested: u32,
        /// Events it admits.
        available: u32,
    },

    /// The span holds more events than any one call could admit.
    ///
    /// Checked **before** the span is scanned, and that ordering is the point. The
    /// per-quantum tally cannot bound this call's work on its own: an event discarded for
    /// a stale epoch or a distant timestamp never reaches the tally, so a producer could
    /// present a million of them and the audio thread would walk every one. The span's
    /// *total* is still not a per-quantum limit — one call may validly cover several
    /// quanta — but it has an absolute ceiling, which is the capacity times the quanta a
    /// call can render.
    #[error("the span presents {presented} events, above the {admissible} a single call can admit")]
    EventSpanTooLarge {
        /// Events the caller presented.
        presented: usize,
        /// The most any one call can admit.
        admissible: usize,
    },

    /// An event is stamped for a quantum this call does not render.
    ///
    /// Phase 1's event input is a **prevalidated bounded span** covering the quanta
    /// this call renders. Holding an event for a later call would require the
    /// deferred store Phase 3 owns, and dropping it silently is what ADR-0001 clause
    /// 16 forbids — so the span's contract is enforced instead of quietly bent.
    #[error(
        "event at sample {position} falls in quantum {event_quantum}, outside the quanta \
         {first_quantum}..={last_quantum} this call renders"
    )]
    EventOutsideCallSpan {
        /// The event's render position.
        position: u64,
        /// The quantum it falls in.
        event_quantum: u64,
        /// First quantum this call renders.
        first_quantum: u64,
        /// Last quantum this call renders.
        last_quantum: u64,
    },

    /// The render clock cannot advance.
    ///
    /// ADR-0032 clause 28's terminal fault, with the same shape as the oversized
    /// callback: silence, `needs_reprepare`, a counted diagnostic, and no panic —
    /// because a clock that cannot advance can no longer place an event, and
    /// continuing would break the monotonicity every other clause rests on.
    #[error("the render clock is exhausted: {0}")]
    ClockExhausted(TimeError),

    /// The stream is over and the renderer is waiting to be re-prepared.
    #[error("this stream needs re-preparation before it can render again")]
    NeedsReprepare,
}

/// The counters a phase exit review inspects.
///
/// This is the report ADR-0021 requires drop counts to reach and ADR-0001 clause 16
/// requires the late counter to reach. It exists because of a specific failure:
/// `LIMIT-0013`'s per-priority drop counters were recorded as "published on OSC" for
/// years, and the use-site audit found `get_dropped_counts` has no caller at all
/// while the OSC feed publishes a different ring's counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct DiagnosticsReport {
    late_events: u64,
    stale_epoch_events: u64,
    out_of_horizon_events: u64,
    pre_epoch_clamps: u64,
    arrival_stamped_events: u64,
    oversized_callback_faults: u64,
    clock_exhaustion_faults: u64,
    needs_reprepare: bool,
}

impl DiagnosticsReport {
    /// Events whose timestamp fell in an already-rendered quantum.
    ///
    /// ADR-0001 clause 16: such an event is clamped to the first not-yet-rendered
    /// quantum boundary and **counted**; it is never applied retroactively and never
    /// silently dropped. The counter's trigger is a *condition*, not a cause — it
    /// does not ask who was at fault.
    pub const fn late_events(&self) -> u64 {
        self.late_events
    }

    /// Events discarded because their epoch is not the renderer's.
    ///
    /// ADR-0032 clause 20. Without this an event stamped microseconds before a
    /// re-preparation would be applied against a clock that restarted at zero,
    /// producing a timing error bounded only by how long the previous stream ran.
    pub const fn stale_epoch_events(&self) -> u64 {
        self.stale_epoch_events
    }

    /// Ingress events rejected for being stamped beyond the forward horizon.
    ///
    /// ADR-0032 clause 21. Holding one would pin a queue slot for an unbounded time.
    /// It binds ingress provenance only: a compiled list spans the whole piece, and
    /// measuring it against this horizon would reject most of a song.
    pub const fn out_of_horizon_events(&self) -> u64 {
        self.out_of_horizon_events
    }

    /// Ingress timestamps clamped up to the start of the epoch.
    ///
    /// ADR-0032 clause 22's counter. **Phase 1 publishes it and never increments
    /// it**, and that is deliberate rather than an omission: the envelope carries an
    /// unsigned `SampleTime`, so a pre-epoch stamp is unrepresentable by the time it
    /// reaches the renderer. The clamp belongs to the ingress mapper, which is Phase
    /// 3's, along with both of the tests that separate this counter from the late
    /// one.
    pub const fn pre_epoch_clamps(&self) -> u64 {
        self.pre_epoch_clamps
    }

    /// Events whose provenance is `Arrival`.
    ///
    /// ADR-0032 clause 19: an adapter with no hardware timestamp must declare its
    /// arrival-time fallback, and that declaration reaches this report. The
    /// uncertainty is *reported*, never used to move an event — an adapter may not
    /// compensate its own unmeasured error.
    pub const fn arrival_stamped_events(&self) -> u64 {
        self.arrival_stamped_events
    }

    /// Callbacks larger than the profile's maximum block.
    pub const fn oversized_callback_faults(&self) -> u64 {
        self.oversized_callback_faults
    }

    /// Times the render clock could not advance.
    pub const fn clock_exhaustion_faults(&self) -> u64 {
        self.clock_exhaustion_faults
    }

    /// Whether the stream is waiting to be re-prepared.
    pub const fn needs_reprepare(&self) -> bool {
        self.needs_reprepare
    }

    pub(crate) fn count_late_event(&mut self) {
        self.late_events = self.late_events.saturating_add(1);
    }

    pub(crate) fn count_stale_epoch_event(&mut self) {
        self.stale_epoch_events = self.stale_epoch_events.saturating_add(1);
    }

    pub(crate) fn count_out_of_horizon_event(&mut self) {
        self.out_of_horizon_events = self.out_of_horizon_events.saturating_add(1);
    }

    pub(crate) fn count_arrival_stamped_event(&mut self) {
        self.arrival_stamped_events = self.arrival_stamped_events.saturating_add(1);
    }

    pub(crate) fn count_oversized_callback(&mut self) {
        self.oversized_callback_faults = self.oversized_callback_faults.saturating_add(1);
    }

    pub(crate) fn count_clock_exhaustion(&mut self) {
        self.clock_exhaustion_faults = self.clock_exhaustion_faults.saturating_add(1);
    }

    pub(crate) fn set_needs_reprepare(&mut self) {
        self.needs_reprepare = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero_and_saturate_rather_than_wrap() {
        let mut report = DiagnosticsReport::default();
        assert_eq!(report.late_events(), 0);
        assert!(!report.needs_reprepare());

        report.count_late_event();
        assert_eq!(report.late_events(), 1);

        report.late_events = u64::MAX;
        report.count_late_event();
        assert_eq!(
            report.late_events(),
            u64::MAX,
            "a counter must saturate: wrapping to zero would read as 'nothing happened'"
        );
    }

    #[test]
    fn the_pre_epoch_counter_exists_and_stays_at_zero_in_this_phase() {
        // Published, never incremented here. Phase 3's ingress mapper is what can
        // observe a pre-zero stamp at all.
        let report = DiagnosticsReport::default();
        assert_eq!(report.pre_epoch_clamps(), 0);
    }

    #[test]
    fn a_limit_refusal_names_the_field_both_amounts_and_the_object() {
        use crate::ir::NodeId;
        use crate::quantities::NodeCount;

        let error = CompileError::LimitExceeded {
            field: ResourceField::MaxNodes,
            requested: ResourceAmount::Nodes(NodeCount::measured(5)),
            available: ResourceAmount::Nodes(NodeCount::measured(4)),
            responsible: IrObject::Node(NodeId::new(3)),
        };
        let rendered = error.to_string();
        for expected in ["max_nodes", "5 nodes", "4 nodes", "node 3"] {
            assert!(
                rendered.contains(expected),
                "a refusal must name {expected}; it said {rendered:?}"
            );
        }
    }
}
