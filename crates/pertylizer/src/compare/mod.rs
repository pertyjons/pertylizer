//! Headless comparison of two rendered WAVs.
//!
//! The other half of the Phase 0A baseline. The [reference
//! corpus](crate::corpus) says what to render and what each render is claimed to
//! mean; this says, in numbers, how two renders of one case differ. Together
//! they are what makes "V2 reproduces V1" a checkable statement rather than an
//! opinion.
//!
//! # What it does not do
//!
//! It does not decide whether a difference is acceptable. There is no pass, no
//! fail, and no tolerance anywhere in this module. Whether 0.4 dB in the 2-5 kHz
//! band is a regression or the intended consequence of a decision depends on the
//! corpus case's `preserve` and `change` claims and on the ADR behind them, and a
//! tool that answered it here would be encoding a judgement it has no basis for
//! — and hiding it inside a constant.
//!
//! It also does not resample, time-align, or gain-match. Every measurement is
//! taken on the files as they are; where that makes a metric meaningless the
//! metric is omitted and a warning says so. A comparison that quietly
//! normalized its inputs would report agreement it had manufactured.
//!
//! # Layout
//!
//! - [`signal`] decodes a WAV to interleaved `f32`;
//! - [`metrics`] holds each measurement and how it is derived;
//! - [`report`] is the versioned JSON shape;
//! - [`command`] runs one comparison end to end.

use std::path::PathBuf;

pub mod command;
pub mod metrics;
pub mod report;
pub mod signal;

pub use command::{CompareCommand, run_compare_command};
pub use report::{COMPARE_PROTOCOL_VERSION, ComparisonReport};
pub use signal::Signal;

/// A failure somewhere in the load-measure-write sequence.
#[derive(Debug, thiserror::Error)]
pub enum CompareError {
    /// A WAV could not be opened or parsed.
    #[error("cannot read {path}: {source}")]
    ReadWav {
        /// The file that could not be read.
        path: PathBuf,
        /// What `hound` said.
        source: hound::Error,
    },
    /// A WAV in a format this reader does not decode.
    #[error("{path} is {description}, which this build cannot decode")]
    UnsupportedWav {
        /// The file that could not be decoded.
        path: PathBuf,
        /// The format it declared.
        description: String,
    },
    /// A WAV with no audio in it.
    #[error("{path} contains no audio frames — there is nothing to compare")]
    EmptyWav {
        /// The empty file.
        path: PathBuf,
    },
    /// A WAV too large to decode into memory.
    ///
    /// Raised from the header, before anything is allocated, so an over-large
    /// input is an error rather than the process abort the allocation would be.
    #[error(
        "{path} decodes to {bytes} bytes, over the {maximum}-byte maximum — compare a shorter \
         window, or a lower sample rate"
    )]
    SignalTooLarge {
        /// The file that was too large.
        path: PathBuf,
        /// Bytes it would have decoded to.
        bytes: u64,
        /// The most one signal may decode to.
        maximum: u64,
    },
    /// Both inputs are the same file.
    #[error(
        "--reference and --candidate are both {path} — comparing a file with itself reports no \
         difference whatever the renderer did"
    )]
    SameFile {
        /// The path both arguments named.
        path: PathBuf,
    },
    /// The report would be written over one of the inputs.
    #[error("--result-json is one of the inputs — writing the report would destroy it")]
    ReportOverwritesInput,
    /// A file could not be read for its content digest.
    #[error(transparent)]
    Digest(#[from] crate::render::RenderError),
    /// The report could not be serialized.
    #[error("cannot serialize the comparison report: {0}")]
    Serialize(#[source] serde_json::Error),
    /// The report's directory does not exist and could not be created.
    #[error("cannot create directory {dir} for the comparison report: {source}")]
    CreateOutputDir {
        /// The directory that could not be created.
        dir: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The report could not be written.
    #[error("cannot write the comparison report to {path}: {source}")]
    WriteReport {
        /// The report path that was being written.
        path: PathBuf,
        /// The underlying write error.
        source: crate::io::atomic::AtomicWriteError,
    },
}
