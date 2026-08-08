//! What a project load could not reconstruct.
//!
//! Applying a saved project to the engine is lossy in ways the file cannot
//! express: a module id whose prefix does not match its type, an effect type
//! this build does not have, a cable to a module that failed to add, a sample
//! the library never received. Every one of those is recoverable — the rest of
//! the project still loads — which is exactly why they used to be dropped with
//! a bare `continue` or an `eprintln!`.
//!
//! The problem with dropping them is not the missing message. It is that the
//! load then *reports success*: the GUI says "Loaded project", `load_project`
//! returns a clean summary, and a headless render writes a receipt with an
//! empty warning list over a mix that is missing an effect. A caller cannot
//! tell a faithful load from a partial one.
//!
//! So every skip produces a [`ProjectApplyDiagnostic`] instead, carrying enough
//! to act on: *where* in the project ([`ProjectPath`]), *what kind* of failure
//! ([`DiagnosticCode`], stable enough to match on), and a human message. The
//! collection is returned to the caller, which decides how to show it.
//!
//! Severity is deliberately coarse. [`Severity::Warning`] means the object was
//! skipped and the rest of the project is coherent; [`Severity::Error`] is
//! reserved for a load that cannot produce a coherent project at all. Today
//! everything here is a warning — a load that cannot continue returns `Err`
//! from the apply function rather than a diagnostic — but the distinction is
//! carried so a future non-fatal-but-serious case has somewhere to go.

use std::fmt;

use synth_core::{InstrumentId, ModuleType};
use synth_engine::ModuleId;
use synth_sequencer::ReturnBusId;

/// How badly a diagnostic affects the loaded project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The object was skipped; the rest of the project loaded coherently.
    Warning,
    /// The project could not be reconstructed coherently.
    Error,
}

impl Severity {
    /// Lowercase, stable identifier for logs and machine-read output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of thing went wrong, as a stable identifier.
///
/// An enum rather than a free-form string so a caller can match on the cause
/// without parsing prose, and so a new failure mode has to be named here rather
/// than smuggled in as a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    /// A module id that does not parse as `<prefix>-<instance>`, or whose
    /// prefix names a different module type than the object claims.
    InvalidModuleId,
    /// A module or effect type this build cannot construct.
    UnsupportedModuleType,
    /// The module could not be added to the instrument's graph.
    ModuleAddFailed,
    /// A parameter the module rejected or does not have.
    ParameterRejected,
    /// A cable that could not be created.
    ConnectionFailed,
    /// A script that could not be parsed, compiled, or installed.
    ScriptRejected,
    /// A per-instance description the engine mirror refused.
    DescriptionRejected,
    /// A sampler module bound to a sample the library does not hold.
    SampleMissing,
    /// The instrument's graph could not be cleared before applying the patch.
    GraphResetFailed,
}

impl DiagnosticCode {
    /// Stable `kebab-case` identifier, safe to match on from outside the crate.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidModuleId => "invalid-module-id",
            Self::UnsupportedModuleType => "unsupported-module-type",
            Self::ModuleAddFailed => "module-add-failed",
            Self::ParameterRejected => "parameter-rejected",
            Self::ConnectionFailed => "connection-failed",
            Self::ScriptRejected => "script-rejected",
            Self::DescriptionRejected => "description-rejected",
            Self::SampleMissing => "sample-missing",
            Self::GraphResetFailed => "graph-reset-failed",
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where in the saved project an object lives, as a dotted path.
///
/// Built through the constructors rather than formatted at each call site, so
/// the same object is named identically wherever it is reported and a reader
/// can map a diagnostic back to the file by eye. The shape mirrors the
/// serialized project: `global.master_effects[0]`,
/// `instruments[id=1].patch.modules[osc-2]`.
///
/// A bare `[n]` subscript is an array index; a subscript written `[id=n]` is an
/// identifier, because instruments and return busses are addressed by an id
/// that is not their position in the file — an instrument with id 12 can be the
/// first entry in the array, and `instruments[12]` would send a reader to the
/// wrong object (or off the end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPath(String);

impl ProjectPath {
    /// A slot in the master effect chain, by its index in the saved array.
    #[must_use]
    pub fn master_effect(index: usize) -> Self {
        Self(format!("global.master_effects[{index}]"))
    }

    /// A slot in one return bus's effect chain, by the bus id and the slot's
    /// index within that bus's chain.
    #[must_use]
    pub fn return_bus_effect(bus: ReturnBusId, index: usize) -> Self {
        Self(format!(
            "global.return_bus_effects[id={}].effects[{index}]",
            bus.0
        ))
    }

    /// The sample library as a whole — for a failure that costs every sampler
    /// its audio rather than one module its sample.
    #[must_use]
    pub fn sample_library() -> Self {
        Self("sample_library".to_string())
    }

    /// An instrument as a whole.
    #[must_use]
    pub fn instrument(instrument: InstrumentId) -> Self {
        Self(instrument_prefix(instrument))
    }

    /// A module inside an instrument's patch, named by its saved id — which is
    /// more useful than an array index when the id is what is wrong.
    #[must_use]
    pub fn instrument_module(instrument: InstrumentId, module_id: &str) -> Self {
        Self(format!(
            "{}.patch.modules[{module_id}]",
            instrument_prefix(instrument)
        ))
    }

    /// A named parameter on a module inside an instrument's patch.
    #[must_use]
    pub fn instrument_module_param(
        instrument: InstrumentId,
        module_id: ModuleId,
        param: &str,
    ) -> Self {
        Self(format!(
            "{}.patch.modules[{module_id}].parameters[{param}]",
            instrument_prefix(instrument)
        ))
    }

    /// A script slot on a module inside an instrument's patch. The slot key is
    /// the saved string rather than a parsed number, because an unparsable key
    /// is one of the things reported here.
    #[must_use]
    pub fn instrument_module_script(
        instrument: InstrumentId,
        module_id: ModuleId,
        slot_key: &str,
    ) -> Self {
        Self(format!(
            "{}.patch.modules[{module_id}].scripts[{slot_key}]",
            instrument_prefix(instrument)
        ))
    }

    /// An entry in an instrument's saved effect-chain order, named by the
    /// entry as written — an entry that does not parse is the case this
    /// reports, so there is nothing else to name it by.
    #[must_use]
    pub fn instrument_effect_chain_order(instrument: InstrumentId, entry: &str) -> Self {
        Self(format!(
            "{}.patch.settings.effect_chain_order[{entry}]",
            instrument_prefix(instrument)
        ))
    }

    /// A cable inside an instrument's patch, named by its endpoints.
    #[must_use]
    pub fn instrument_connection(
        instrument: InstrumentId,
        from: &str,
        from_port: &str,
        to: &str,
        to_port: &str,
    ) -> Self {
        Self(format!(
            "{}.patch.connections[{from}:{from_port} → {to}:{to_port}]",
            instrument_prefix(instrument)
        ))
    }

    /// The path as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `instruments[id=N]` prefix every in-instrument path is built on.
fn instrument_prefix(instrument: InstrumentId) -> String {
    format!("instruments[id={}]", instrument.as_u64())
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One thing the load could not reconstruct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectApplyDiagnostic {
    /// Whether the project is still coherent without this object.
    pub severity: Severity,
    /// What kind of failure this is.
    pub code: DiagnosticCode,
    /// Where in the saved project the object lives.
    pub path: ProjectPath,
    /// What happened, in prose, including any hint at the fix.
    pub message: String,
}

impl ProjectApplyDiagnostic {
    /// A recoverable skip: the object is gone, the project is otherwise whole.
    #[must_use]
    pub fn warning(code: DiagnosticCode, path: ProjectPath, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            path,
            message: message.into(),
        }
    }
}

impl fmt::Display for ProjectApplyDiagnostic {
    /// One line, leading with severity and path so a list of these sorts and
    /// scans usefully: `warning [invalid-module-id] global.master_effects[0]: …`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} [{}] {}: {}",
            self.severity, self.code, self.path, self.message
        )
    }
}

/// Explain an unparsable module id, naming the prefix it should have had.
///
/// A saved id is `<prefix>-<instance>`, and the prefix *is* the module type —
/// so an id whose prefix is wrong does not merely fail to parse, it names
/// nothing. The observed case was a `limiter` effect saved as `lim-1`: plausible
/// to a human, three letters off, and discarded in silence. The caller already
/// knows the type the object claims to be, so the fix can be stated outright
/// rather than left to whoever reads the raw parse error.
#[must_use]
pub fn invalid_module_id_message(id: &str, claimed: ModuleType, parse_error: &str) -> String {
    format!(
        "id '{id}' does not parse as a module id ({parse_error}) — a {} is saved as '{}-<n>'",
        claimed.name(),
        claimed.prefix()
    )
}

/// Explain a module id that parses but names a different type than the object
/// claims — `flt-1` on an effect saved as a `limiter`.
///
/// Reported, not fatal: the effect chains build from the saved type and key on
/// the id opaquely, so such an entry loads correctly. What it costs is every
/// later reader that infers a type from the prefix instead of asking.
///
/// Distinct from [`invalid_module_id_message`] because nothing is malformed
/// here: the id is a perfectly good id *for the wrong module*. The instance is
/// built from the entry's own type, so the file is not describing the module it
/// says it is — and anything downstream that reads the type off the id would be
/// told the wrong one.
#[must_use]
pub fn mismatched_module_id_message(id: &str, claimed: ModuleType, named: ModuleType) -> String {
    format!(
        "id '{id}' names a {} but the entry is a {} — a {} is saved as '{}-<n>'",
        named.name(),
        claimed.name(),
        claimed.name(),
        claimed.prefix()
    )
}

/// The outcome of applying a project: what was loaded, and what was not.
#[derive(Debug, Clone, Default)]
pub struct ProjectApplyReport {
    /// The human-readable success line the load used to return on its own.
    pub summary: String,
    /// Everything the load could not reconstruct, in the order encountered.
    pub diagnostics: Vec<ProjectApplyDiagnostic>,
}

impl ProjectApplyReport {
    /// A report with no diagnostics — a fully faithful load.
    #[must_use]
    pub fn clean(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            diagnostics: Vec::new(),
        }
    }

    /// Whether anything at all was skipped.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Each diagnostic rendered as one line, for a receipt or a log.
    #[must_use]
    pub fn diagnostic_lines(&self) -> Vec<String> {
        self.diagnostics.iter().map(ToString::to_string).collect()
    }

    /// The summary with a count of what was skipped appended, so a caller that
    /// shows a single line cannot show a clean one over a partial load.
    ///
    /// Identical to [`Self::summary`] when nothing was skipped, so the happy
    /// path reads exactly as it did before diagnostics existed.
    ///
    /// At most [`Self::SUMMARY_DIAGNOSTIC_LIMIT`] diagnostics are spelled out;
    /// the rest are counted. This is a one-line human summary and it is handed
    /// straight to an MCP client and to the render receipt — a project loaded
    /// into a build that renamed a parameter can produce a diagnostic per
    /// parameter per module, and joining thousands of them would turn a status
    /// line into a multi-hundred-kilobyte string. The full list is always
    /// available from [`Self::diagnostic_lines`], which is what every caller
    /// that wants them all already uses.
    #[must_use]
    pub fn summary_with_diagnostics(&self) -> String {
        if self.is_clean() {
            return self.summary.clone();
        }
        let shown = self
            .diagnostics
            .iter()
            .take(Self::SUMMARY_DIAGNOSTIC_LIMIT)
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        let elided = self
            .diagnostics
            .len()
            .saturating_sub(Self::SUMMARY_DIAGNOSTIC_LIMIT);
        let tail = if elided > 0 {
            format!("; … and {elided} more")
        } else {
            String::new()
        };
        format!(
            "{} — {} item(s) could not be loaded: {shown}{tail}",
            self.summary,
            self.diagnostics.len(),
        )
    }

    /// How many diagnostics [`Self::summary_with_diagnostics`] spells out
    /// before it starts counting instead.
    pub const SUMMARY_DIAGNOSTIC_LIMIT: usize = 10;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paths mirror the serialized project, because their whole job is to
    /// let a reader find the object in the file.
    #[test]
    fn paths_read_like_the_saved_project() {
        assert_eq!(
            ProjectPath::master_effect(0).as_str(),
            "global.master_effects[0]"
        );
        assert_eq!(
            ProjectPath::return_bus_effect(ReturnBusId(2), 1).as_str(),
            "global.return_bus_effects[id=2].effects[1]"
        );
        assert_eq!(
            ProjectPath::instrument_module(InstrumentId::new(1), "lim-1").as_str(),
            "instruments[id=1].patch.modules[lim-1]"
        );
    }

    /// A module path names the module by its saved id rather than an index:
    /// when the id is the thing that is wrong, an index says nothing.
    #[test]
    fn a_module_param_path_names_module_and_parameter() {
        let module = ModuleId::new(ModuleType::Filter, 1);
        assert_eq!(
            ProjectPath::instrument_module_param(InstrumentId::new(3), module, "cutoff").as_str(),
            "instruments[id=3].patch.modules[flt-1].parameters[cutoff]"
        );
    }

    #[test]
    fn a_connection_path_names_both_endpoints() {
        assert_eq!(
            ProjectPath::instrument_connection(
                InstrumentId::new(1),
                "osc-1",
                "out",
                "flt-1",
                "audio_in"
            )
            .as_str(),
            "instruments[id=1].patch.connections[osc-1:out → flt-1:audio_in]"
        );
    }

    /// The one-line form leads with severity and code so a list of them can be
    /// scanned and grepped.
    #[test]
    fn a_diagnostic_renders_as_one_scannable_line() {
        let diagnostic = ProjectApplyDiagnostic::warning(
            DiagnosticCode::InvalidModuleId,
            ProjectPath::master_effect(0),
            "id 'lim-1' is not a Limiter id (expected prefix 'lmt')",
        );
        assert_eq!(
            diagnostic.to_string(),
            "warning [invalid-module-id] global.master_effects[0]: \
             id 'lim-1' is not a Limiter id (expected prefix 'lmt')"
        );
    }

    /// A clean load must read exactly as it did before diagnostics existed —
    /// otherwise every existing caller's output changes for no reason.
    #[test]
    fn a_clean_report_summary_is_unchanged() {
        let report = ProjectApplyReport::clean("Loaded project: 1 instrument(s)");
        assert!(report.is_clean());
        assert_eq!(
            report.summary_with_diagnostics(),
            "Loaded project: 1 instrument(s)"
        );
    }

    /// A partial load cannot present a clean summary line.
    #[test]
    fn a_partial_report_summary_says_what_was_skipped() {
        let report = ProjectApplyReport {
            summary: "Loaded project: 1 instrument(s)".to_string(),
            diagnostics: vec![ProjectApplyDiagnostic::warning(
                DiagnosticCode::SampleMissing,
                ProjectPath::instrument_module(InstrumentId::new(1), "smp-1"),
                "sample 7 is not in the library",
            )],
        };

        assert!(!report.is_clean());
        let summary = report.summary_with_diagnostics();
        assert!(summary.starts_with("Loaded project: 1 instrument(s) — 1 item(s)"));
        assert!(summary.contains("sample-missing"));
        assert!(summary.contains("smp-1"));
    }

    /// The one-line summary is handed to an MCP client and stamped into a
    /// render receipt, so it must stay a line. A version-skewed project can
    /// reject a parameter per parameter per module; spelling every one of them
    /// out would make the "summary" the largest field in the response.
    #[test]
    fn a_summary_counts_diagnostics_past_the_limit_instead_of_listing_them() {
        let count = ProjectApplyReport::SUMMARY_DIAGNOSTIC_LIMIT + 7;
        let report = ProjectApplyReport {
            summary: "Loaded project".to_string(),
            diagnostics: (0..count)
                .map(|i| {
                    ProjectApplyDiagnostic::warning(
                        DiagnosticCode::ParameterRejected,
                        ProjectPath::master_effect(i),
                        "no such parameter",
                    )
                })
                .collect(),
        };

        let summary = report.summary_with_diagnostics();
        // The true total is still stated, so nothing is hidden.
        assert!(summary.contains(&format!("{count} item(s)")), "{summary}");
        assert!(summary.ends_with("… and 7 more"), "{summary}");
        assert!(
            summary.contains("master_effects[9]") && !summary.contains("master_effects[10]"),
            "exactly the first {} are spelled out: {summary}",
            ProjectApplyReport::SUMMARY_DIAGNOSTIC_LIMIT
        );
        // The full list stays reachable for the callers that want it all.
        assert_eq!(report.diagnostic_lines().len(), count);
    }
}
