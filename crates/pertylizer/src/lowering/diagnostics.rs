//! What the lowerer says about a project it could not fully represent.
//!
//! The Phase 4 exit gate requires that "unsupported modules and targets produce structured
//! diagnostics naming the project object and reason". Both halves are types here rather than
//! a formatted string, so that a caller can act on a reason without parsing prose and a
//! subject can always be traced back to something the user authored.
//!
//! # Two severities, and why the weaker one is not a warning
//!
//! [`Severity::Refused`] stops the lowering. [`Severity::Unrepresented`] does not, but it is
//! **not** advisory: it sets [`Fidelity::UnsupportedScope`] on the outcome, and
//! `PROCESS.md`'s phase-exit rule requires the implemented behaviour to fail closed rather
//! than accept and silently ignore an unsupported case. A marked outcome is refused by any
//! parity comparison; see [`Fidelity`].

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{NoteId, PatternId, ReturnBusId, TrackId};

/// The project object a diagnostic is about.
///
/// Every variant names something a user authored and can find. A plan-internal address is
/// never a subject: `ResolvedIdentities::module_for` exists to turn one back into a module.
/// `#[non_exhaustive]` because this list grows with every phase that lowers more of a
/// project, and each addition would otherwise break an exhaustive match in any consumer. The
/// attribute is added now, while the only consumer is this crate's own tests, rather than
/// after the first break it would have caused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ProjectSubject {
    /// The project as a whole.
    Project,
    /// One instrument.
    Instrument {
        /// Its persisted identity.
        instrument: InstrumentId,
        /// Its name, so the diagnostic reads the way the user's project does.
        name: String,
    },
    /// One module inside one instrument's voice patch.
    Module {
        /// The owning instrument.
        instrument: InstrumentId,
        /// The module, by its persisted identity.
        module: ModuleId,
    },
    /// One parameter of one module.
    Parameter {
        /// The owning instrument.
        instrument: InstrumentId,
        /// The owning module.
        module: ModuleId,
        /// The parameter's saved key.
        parameter: String,
    },
    /// One connection inside one instrument's voice patch.
    ///
    /// The endpoints are the saved spellings rather than resolved identities, because the
    /// commonest reason to diagnose a connection is that one of them did not resolve.
    Connection {
        /// The owning instrument.
        instrument: InstrumentId,
        /// Source module and port, as the project spells them.
        from: (String, String),
        /// Destination module and port, as the project spells them.
        to: (String, String),
    },
    /// One pattern.
    Pattern {
        /// Its identity.
        pattern: PatternId,
        /// Its name.
        name: String,
    },
    /// One note inside one pattern.
    Note {
        /// The owning pattern.
        pattern: PatternId,
        /// The note's identity within it.
        note: NoteId,
    },
    /// One arrangement track.
    Track {
        /// Its identity.
        track: TrackId,
        /// Its name.
        name: String,
    },
    /// One return bus's effect chain.
    ReturnBus {
        /// The bus the song declares.
        ///
        /// The domain newtype rather than the `u16` the project file stores: a bus identity
        /// and a chain position are both small integers, and only the type keeps one from
        /// being passed where the other belongs.
        bus: ReturnBusId,
    },
    /// The master effect chain.
    MasterChain,
}

/// Why the lowerer produced a diagnostic.
///
/// A closed enum rather than a message: the A/B path has to decide whether an outcome may be
/// compared for parity, and that decision reads reasons, not strings.
/// `#[non_exhaustive]` for the reason [`ProjectSubject`] is: the set of things a project can
/// ask for that V2 cannot yet represent is open, and shrinks phase by phase.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum LoweringReason {
    /// The module's type has no node kind in V2's registry.
    UnsupportedModuleType {
        /// The type the project authored.
        module_type: ModuleType,
    },
    /// The module's type maps to a V2 node kind, but this parameter value selects behaviour
    /// that kind does not have — a `sawtooth` on a kind that is a sine, for instance.
    UnsupportedParameterValue {
        /// The value the project authored, as it is spelled.
        value: String,
    },
    /// A connection endpoint names a module the patch does not declare.
    UnresolvedEndpoint {
        /// The endpoint spelling that did not resolve.
        spelling: String,
    },
    /// A connection endpoint names a port the destination node kind does not declare.
    UnknownPort {
        /// The port name the project authored.
        port: String,
    },
    /// Both ports exist, but one carries a signal the other does not accept.
    ///
    /// Separate from [`Self::UnknownPort`] because that variant's `port` field means "the
    /// port name the project authored", and a caller highlighting the cable needs that name
    /// intact. Folding an explanation into it would leave the field carrying prose that no
    /// longer matches anything in the project.
    DomainMismatch {
        /// The destination port name the project authored.
        port: String,
        /// What the destination accepts.
        expected: &'static str,
        /// What the source carries.
        found: &'static str,
    },
    /// The project asks for behaviour a later phase owns.
    ///
    /// Distinct from an unsupported type: nothing is missing from the registry, and the
    /// obstacle is a decision or a phase boundary rather than a node kind.
    OwnedByLaterPhase {
        /// What the project asked for.
        capability: &'static str,
        /// Which phase owns supplying it.
        owner: &'static str,
    },
}

/// Whether a diagnostic stopped the lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Severity {
    /// Lowering stopped. No plan was produced.
    Refused,
    /// Lowering continued, and the result does not represent what the subject asked for.
    ///
    /// Never advisory: it forces [`Fidelity::UnsupportedScope`].
    Unrepresented,
}

/// One structured thing the lowerer has to say.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct LoweringDiagnostic {
    subject: ProjectSubject,
    reason: LoweringReason,
    severity: Severity,
}

impl LoweringDiagnostic {
    /// A diagnostic that stops the lowering.
    pub const fn refused(subject: ProjectSubject, reason: LoweringReason) -> Self {
        Self {
            subject,
            reason,
            severity: Severity::Refused,
        }
    }

    /// A diagnostic that lets lowering continue but marks the outcome.
    pub const fn unrepresented(subject: ProjectSubject, reason: LoweringReason) -> Self {
        Self {
            subject,
            reason,
            severity: Severity::Unrepresented,
        }
    }

    /// What the diagnostic is about.
    pub const fn subject(&self) -> &ProjectSubject {
        &self.subject
    }

    /// Why it was produced.
    pub const fn reason(&self) -> &LoweringReason {
        &self.reason
    }

    /// Whether it stopped the lowering.
    pub const fn severity(&self) -> Severity {
        self.severity
    }
}

/// Whether an outcome may be compared against V1 for parity.
///
/// # Why this is on the outcome rather than left to the caller
///
/// `P04-R001` requires that no render carrying an unrepresented note payload can be presented
/// as faithful. A diagnostic list alone does not achieve that — a caller may render, ignore
/// the list, and report parity, which is the exact failure `PROCESS.md` calls "accepting and
/// silently ignoring the unsupported case". Carrying the verdict on the outcome means the
/// comparison path reads one value and refuses, and a caller that wants to compare has to
/// delete the refusal rather than merely forget the diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Fidelity {
    /// Everything the project asked for is represented. A parity comparison is admissible.
    Faithful,
    /// Something is not represented. A parity comparison is **refused**.
    UnsupportedScope,
}

impl Fidelity {
    /// The verdict a set of diagnostics implies.
    ///
    /// Derived rather than set, so the two cannot disagree: an outcome carrying an
    /// `Unrepresented` diagnostic and claiming `Faithful` is not constructible.
    pub fn of(diagnostics: &[LoweringDiagnostic]) -> Self {
        if diagnostics.is_empty() {
            Self::Faithful
        } else {
            Self::UnsupportedScope
        }
    }

    /// Whether a parity comparison may read this outcome.
    #[must_use]
    pub const fn admits_parity_comparison(self) -> bool {
        matches!(self, Self::Faithful)
    }
}
