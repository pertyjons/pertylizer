//! The machine-readable result a `pertylizer compare` run emits.
//!
//! Versioned like the render receipt and for the same reason: the number a
//! harness reads out of this file becomes a baseline in an evidence record, and
//! a silently re-meant field would rebaseline every conclusion drawn from it.

use serde::Serialize;

use crate::render::receipt::RendererInfo;

use super::CompareError;
use super::metrics::{
    EnvelopeDifference, LevelDifference, LoudnessDifference, PitchDifference, SampleDifference,
    SpectrumDifference, StereoDifference,
};
use super::signal::SignalInfo;

/// Version of the compare contract and of this report's shape.
///
/// A caller passes `--protocol-version` and gets it echoed back. Adding an
/// optional field does not change it; removing or re-meaning anything does.
/// Independent of the render command's version: the two contracts change for
/// different reasons and pinning them together would force a bump on one
/// whenever the other moved.
pub const COMPARE_PROTOCOL_VERSION: u32 = 1;

/// Identity of one input, by path and by content.
#[derive(Debug, Clone, Serialize)]
pub struct ComparedFile {
    /// Absolute path when it could be resolved, lossily converted to text.
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the whole file.
    ///
    /// The render determinism digest the master plan asks for: two renders of
    /// one corpus case by one build must produce the same value here. It covers
    /// the file rather than the audio, so a header difference shows up as a
    /// digest difference with `samples.identical` still true — which is the
    /// distinction a determinism claim needs.
    pub sha256: String,
    /// What the decoded audio is.
    pub audio: SignalInfo,
}

/// The complete result of one comparison.
///
/// Every `delta` field anywhere below is candidate minus reference.
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonReport {
    /// Contract version this report speaks.
    pub protocol_version: u32,
    /// Which build produced it.
    pub comparer: RendererInfo,
    /// The reference — the render everything is measured against.
    pub reference: ComparedFile,
    /// The candidate.
    pub candidate: ComparedFile,
    /// Whether the two files are byte-identical.
    pub files_identical: bool,
    /// Peak and RMS on both sides.
    pub level: LevelDifference,
    /// Sample-by-sample agreement. Absent when the two are not on one time base.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<SampleDifference>,
    /// Onsets and overall alignment. Absent when either side has no onset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<TimingSection>,
    /// Fundamental frequency and pitch drift.
    pub pitch: PitchDifference,
    /// Amplitude-envelope landmarks and shape. Absent when either side has no
    /// measurable envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<EnvelopeDifference>,
    /// Stereo image. Absent unless both sides are stereo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo: Option<StereoDifference>,
    /// Per-octave-band spectrum. Absent when the sample rates differ or either
    /// side is too short to analyse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<SpectrumDifference>,
    /// Integrated loudness on both sides.
    pub loudness: LoudnessDifference,
    /// Why anything above is absent, and anything else the comparison had to
    /// say. Empty when every metric was computed.
    ///
    /// Read this before concluding from a report with a missing section: an
    /// absent section and a section full of zeroes mean opposite things.
    pub warnings: Vec<String>,
    /// The invocation, argument by argument.
    pub command: Vec<String>,
}

/// Alias kept so the report's field names read as sections rather than as types.
pub type TimingSection = super::metrics::TimingDifference;

impl ComparisonReport {
    /// Serialize as pretty-printed JSON with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns [`CompareError::Serialize`] if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, CompareError> {
        let mut json = serde_json::to_vec_pretty(self).map_err(CompareError::Serialize)?;
        json.push(b'\n');
        Ok(json)
    }
}
