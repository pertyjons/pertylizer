//! The whole `pertylizer compare` run, from arguments to report.
//!
//! Load both files, measure everything measurable, say why anything else was
//! not, and write the report. Nothing here decides whether a difference is
//! acceptable — that judgement belongs to the corpus case's preserve/change
//! claims and to the evidence record citing them, and a tool that pronounced a
//! verdict would have to encode a tolerance it has no basis for.

use std::path::{Path, PathBuf};

use super::CompareError;
use super::metrics::{
    EnvelopeDifference, LevelDifference, LoudnessDifference, PitchDifference, SampleDifference,
    SpectrumDifference, StereoDifference, TimingDifference,
};
use super::report::{COMPARE_PROTOCOL_VERSION, ComparedFile, ComparisonReport};
use super::signal::{Signal, SignalInfo};
use crate::render::receipt::{FileDigest, RendererInfo};

/// One comparison invocation, already parsed.
#[derive(Debug, Clone)]
pub struct CompareCommand {
    /// The render everything is measured against. Only ever read.
    pub reference: PathBuf,
    /// The render being measured. Only ever read.
    pub candidate: PathBuf,
    /// Where the report goes. `None` prints it on stdout.
    pub result_json: Option<PathBuf>,
    /// The invocation, for the report.
    pub argv: Vec<String>,
}

/// Run `command` end to end and return the report.
///
/// Neither input is ever written to. The report is the only output.
///
/// # Errors
///
/// Returns whichever [`CompareError`] the step that failed produced. A failure
/// to load either input happens before any measurement, so a failed run leaves
/// no partial report.
pub fn run_compare_command(command: &CompareCommand) -> Result<ComparisonReport, CompareError> {
    // Refused up front rather than measured: comparing a file with itself
    // produces a page of zeroes that reads exactly like "V2 reproduces V1", and
    // the one keystroke between the two paths is the easiest mistake to make in
    // a batch script. Compared by identity, not by content — two files with the
    // same bytes are a legitimate and interesting comparison.
    if crate::render::headless::path_identity(&command.reference)
        == crate::render::headless::path_identity(&command.candidate)
    {
        return Err(CompareError::SameFile {
            path: command.reference.clone(),
        });
    }
    if command.result_json.as_ref().is_some_and(|json| {
        let json = crate::render::headless::path_identity(json);
        json == crate::render::headless::path_identity(&command.reference)
            || json == crate::render::headless::path_identity(&command.candidate)
    }) {
        return Err(CompareError::ReportOverwritesInput);
    }

    let reference_digest = FileDigest::of(&command.reference)?;
    let candidate_digest = FileDigest::of(&command.candidate)?;
    let reference = Signal::load(&command.reference)?;
    let candidate = Signal::load(&command.candidate)?;

    let mut warnings = Vec::new();
    if reference.sample_rate != candidate.sample_rate {
        warnings.push(format!(
            "sample rates differ ({} Hz against {} Hz) — the sample and spectrum comparisons are \
             skipped, and every time-domain measurement is being read off two different grids",
            reference.sample_rate, candidate.sample_rate
        ));
    }
    if reference.channels != candidate.channels {
        warnings.push(format!(
            "channel counts differ ({} against {}) — the stereo comparison is skipped and the \
             mono-summed metrics average over a different number of channels on each side",
            reference.channels, candidate.channels
        ));
    }
    if reference.frames() != candidate.frames() {
        warnings.push(format!(
            "lengths differ ({} frames against {}) — every metric is measured over its own \
             signal's full length, so a difference here colours all of them",
            reference.frames(),
            candidate.frames()
        ));
    }

    let samples = SampleDifference::measure(&reference, &candidate);
    if samples.is_none() {
        warnings.push(
            "no sample-by-sample comparison: it needs the same sample rate, channel count, and \
             length on both sides"
                .to_string(),
        );
    }
    let timing = TimingDifference::measure(&reference, &candidate);
    match &timing {
        None => warnings
            .push("no timing comparison: one of the signals is silent or too short".to_string()),
        // The section is present but one field inside it is not, and the module
        // contract is that *every* absence carries a warning — an unexplained
        // missing field reads as "this build does not emit it" rather than as
        // "the material could not answer it".
        Some(timing) if timing.envelope_lag_ms.is_none() => warnings.push(
            "no envelope alignment lag: the reference's envelope is too featureless for a \
             cross-correlation peak to mean anything (a sustained tone looks the same at every \
             offset); the onset delta is still measured"
                .to_string(),
        ),
        Some(_) => {}
    }
    let envelope = EnvelopeDifference::measure(&reference, &candidate);
    match &envelope {
        None => warnings
            .push("no envelope comparison: one of the signals is silent or too short".to_string()),
        // Same contract as the alignment lag above: a field that is present in
        // one report and missing from another must say which of the two it is.
        Some(envelope) if envelope.correlation.is_none() => warnings.push(
            "no envelope correlation: one of the envelopes is too featureless to correlate, so \
             the figure would come out of window-boundary ripple; the landmarks and the level \
             difference are still measured"
                .to_string(),
        ),
        Some(_) => {}
    }
    let stereo = StereoDifference::measure(&reference, &candidate);
    match &stereo {
        None => {
            warnings.push("no stereo comparison: it needs two channels on both sides".to_string())
        }
        Some(stereo) if stereo.correlation_delta.is_none() => warnings.push(
            "no stereo correlation: a channel on one side is silent or constant, and a Pearson \
             figure over it would assert \"uncorrelated\" where the truth is \"not measured\"; \
             the per-channel energies are still reported"
                .to_string(),
        ),
        Some(_) => {}
    }
    let spectrum = SpectrumDifference::measure(&reference, &candidate);
    if spectrum.is_none() {
        warnings.push(
            "no spectrum comparison: it needs one sample rate on both sides and enough audio for \
             an analysis frame"
                .to_string(),
        );
    }
    let loudness = LoudnessDifference::measure(&reference, &candidate);
    if loudness.delta_lu.is_none() {
        warnings.push(
            "no loudness difference: at least one side is silent or shorter than the standard's \
             400 ms block"
                .to_string(),
        );
    }

    let pitch = PitchDifference::measure(&reference, &candidate);
    if pitch.delta_cents.is_none() {
        warnings.push(
            "no pitch interval: at least one side has no detectable fundamental, which is what an \
             unpitched or percussive render looks like"
                .to_string(),
        );
    }
    if pitch.drift.is_none() {
        warnings.push(
            "no pitch drift: no analysis window is voiced on both sides, so there is no window \
             pair whose interval could be averaged"
                .to_string(),
        );
    }

    let report = ComparisonReport {
        protocol_version: COMPARE_PROTOCOL_VERSION,
        comparer: RendererInfo::current(),
        files_identical: reference_digest.sha256 == candidate_digest.sha256,
        // Both halves of an entry name the file the same way — the digest's
        // absolute path. Letting the `audio` section carry the argument as typed
        // would put two spellings of one file in one report, and a reader
        // diffing two runs from different working directories would see them
        // disagree about a file neither of them moved.
        reference: ComparedFile {
            audio: SignalInfo::of(&reference, Path::new(&reference_digest.path)),
            path: reference_digest.path,
            bytes: reference_digest.bytes,
            sha256: reference_digest.sha256,
        },
        candidate: ComparedFile {
            audio: SignalInfo::of(&candidate, Path::new(&candidate_digest.path)),
            path: candidate_digest.path,
            bytes: candidate_digest.bytes,
            sha256: candidate_digest.sha256,
        },
        level: LevelDifference::measure(&reference, &candidate),
        samples,
        timing,
        pitch,
        envelope,
        stereo,
        spectrum,
        loudness,
        warnings,
        command: command.argv.clone(),
    };

    if let Some(path) = &command.result_json {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| CompareError::CreateOutputDir {
                dir: parent.to_path_buf(),
                source,
            })?;
        }
        crate::io::atomic::write(path, &report.to_json()?).map_err(|source| {
            CompareError::WriteReport {
                path: path.clone(),
                source,
            }
        })?;
    }

    Ok(report)
}
