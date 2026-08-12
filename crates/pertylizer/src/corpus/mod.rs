//! The Sound Core V2 reference render corpus.
//!
//! Phase 0A of the Core V2 migration needs a fixed set of V1 renders to measure
//! V2 against. This module owns the machine-readable description of that set —
//! which projects, rendered how, and what each case claims V2 must preserve or
//! is allowed to change. The audio itself is never stored: a case names an input
//! and the exact render settings, so any revision can regenerate the reference
//! bytes and a receipt can be diffed against them.
//!
//! The manifest lives at `corpus/v2-reference/manifest.json` and the projects it
//! names sit next to it. Everything in it is either an input to a render or a
//! claim about the render's meaning; nothing is a measurement, because a
//! measurement belongs in an evidence record that cites this manifest.
//!
//! # Why the model is in the binary crate
//!
//! Loading a case has to reject anything the renderer itself would reject —
//! otherwise the corpus can declare a render that cannot be produced, and the
//! failure surfaces hours later in a batch run. [`CorpusManifest::validate`]
//! therefore calls the same bounds check `pertylizer render` applies to its own
//! arguments.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use synth_core::Seconds;

use crate::audio::wav_format::WavFormat;
use crate::render::{RenderError, TrackSelector};
use synth_core::audio::DeviceSampleRate;

pub mod fixtures;

/// Version of the manifest's shape.
///
/// Adding an optional field does not change it; removing or re-meaning anything
/// does. A loader refuses a manifest that declares any other version rather than
/// silently reading fields that no longer mean what they did.
pub const MANIFEST_VERSION: u32 = 1;

/// Repository-relative location of the corpus directory.
///
/// Stated once here so a test, the generator, and a future harness cannot each
/// carry their own copy and drift apart.
pub const CORPUS_DIR: &str = "corpus/v2-reference";

/// File name of the manifest inside [`CORPUS_DIR`].
pub const MANIFEST_FILE: &str = "manifest.json";

/// A corpus case's stable identifier, `CORPUS-NNNN`.
///
/// Its own type rather than a `String` because it is a domain reference, not
/// text: it is cited from evidence records, review notes, and commit messages,
/// and [`CorpusManifest::case`] looks cases up by it. A bare `String` parameter
/// accepts a title, a file name, or a claim id just as happily.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CorpusCaseId(String);

/// A behaviour claim's stable identifier.
///
/// Separate from [`CorpusCaseId`] despite sharing a shape, because the two are
/// not interchangeable: a claim id names one sentence inside a case, and passing
/// one where the other belongs is exactly the confusion an identifier exists to
/// prevent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BehaviorClaimId(String);

impl CorpusCaseId {
    /// The shape a malformed case identifier failed to match.
    pub const SHAPE: &'static str = "CORPUS-NNNN";

    /// Wrap `id` **without** checking its shape.
    ///
    /// `pub(crate)` and named for what it skips, following the codebase's
    /// existing `VoiceCount::new_unchecked` convention. The validated way in is
    /// [`FromStr`] or [`TryFrom<String>`]; this exists only for the `const`
    /// [`fixtures::FIXTURES`](super::corpus::fixtures::FIXTURES) table, whose
    /// literals a `String`-backed newtype cannot hold and which
    /// `fixtures_and_cases_refer_to_each_other` checks against the manifest.
    #[must_use]
    pub(crate) fn new_unchecked(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is `CORPUS-` followed by exactly four digits.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        series_digits(&self.0, "CORPUS").is_some_and(|digits| digits.is_empty())
    }
}

impl BehaviorClaimId {
    /// The shape a malformed claim identifier failed to match.
    pub const SHAPE: &'static str = "CORPUS-NNNN-<suffix>";

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is a well-formed case identifier followed by `-` and a
    /// non-empty suffix.
    ///
    /// The case part is checked rather than ignored so a claim always says which
    /// case it belongs to — that is the whole reason the ids are nested.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        series_digits(&self.0, "CORPUS")
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|suffix| !suffix.is_empty())
    }
}

/// An identifier that does not match its series' shape.
#[derive(Debug, thiserror::Error)]
#[error("{value:?} is not a well-formed identifier: expected {expected}")]
pub struct MalformedIdentifier {
    /// The text that was rejected.
    pub value: String,
    /// The shape it had to match.
    pub expected: &'static str,
}

impl TryFrom<String> for CorpusCaseId {
    type Error = MalformedIdentifier;

    /// Validated at the boundary, so a malformed identifier cannot reach the
    /// rest of the model. [`CorpusManifest::validate`] checks the same rule
    /// again — that check is now defence in depth for identifiers built in code
    /// rather than deserialized, and it is the one that can name the entry.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let id = Self(value);
        if id.is_well_formed() {
            Ok(id)
        } else {
            Err(MalformedIdentifier {
                value: id.0,
                expected: Self::SHAPE,
            })
        }
    }
}

impl TryFrom<String> for BehaviorClaimId {
    type Error = MalformedIdentifier;

    /// See [`CorpusCaseId::try_from`].
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let id = Self(value);
        if id.is_well_formed() {
            Ok(id)
        } else {
            Err(MalformedIdentifier {
                value: id.0,
                expected: Self::SHAPE,
            })
        }
    }
}

impl std::str::FromStr for CorpusCaseId {
    type Err = MalformedIdentifier;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_string())
    }
}

impl std::str::FromStr for BehaviorClaimId {
    type Err = MalformedIdentifier;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_string())
    }
}

impl From<CorpusCaseId> for String {
    fn from(id: CorpusCaseId) -> Self {
        id.0
    }
}

impl From<BehaviorClaimId> for String {
    fn from(id: BehaviorClaimId) -> Self {
        id.0
    }
}

impl fmt::Display for CorpusCaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for BehaviorClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A case's external PRNG seed, present or deliberately absent.
///
/// A wrapper rather than a bare `Option<u64>` because serde's derive treats an
/// `Option` field as implicitly optional: with the field typed `Option<u64>` a
/// manifest that simply omits `seed` parses happily as `None`, and "this case
/// has no seed" becomes indistinguishable from "somebody forgot to write one".
/// Wrapping it in a struct makes the key required while `null` stays a legal —
/// and, for now, the only correct — answer.
///
/// Deliberately **not** `#[serde(transparent)]`, which would defeat the whole
/// point: serde fills a missing field by handing the type a deserializer that
/// implements exactly one method, `deserialize_option`, and a transparent
/// wrapper forwards straight to it and yields `None`. A plain newtype derive
/// asks for `deserialize_newtype_struct`, which that deserializer does not
/// answer, so the field becomes genuinely required. JSON encodes a newtype
/// struct as its inner value either way, so the on-disk form is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seed(Option<u64>);

impl Seed {
    /// The case declares no seed. V1's answer for every case.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    /// The case renders with this seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(Some(seed))
    }

    /// The seed, if there is one.
    #[must_use]
    pub const fn get(self) -> Option<u64> {
        self.0
    }
}

/// The kind of V1 behaviour a case exercises.
///
/// The variants are exactly the bounded corpus the master plan's Phase 0A work
/// list enumerates. They are a closed set on purpose: the manifest is required
/// to account for every one of them, either with a case or with a recorded gap,
/// so a category cannot quietly disappear from the corpus as it grows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, strum::EnumIter,
)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusCategory {
    /// Oscillator into filter into an envelope-gated amplifier — the minimal
    /// path every later category builds on.
    SubtractiveVoice,
    /// A voice whose output is not the same in both channels: stereo
    /// oscillator, keyboard panner, or spatial panner.
    StereoOrSpatialVoice,
    /// A patch whose sound source is a sample rather than a synthesised
    /// waveform.
    SamplerPatch,
    /// A patch that routes modulation through the Mod Matrix.
    ModMatrixPatch,
    /// A patch driven by a YAMS control script (control-rate).
    YamsControlPatch,
    /// A patch driven by a YAMS AudioScript node (audio-rate).
    YamsAudioScriptPatch,
    /// More simultaneous notes than the instrument has voices, so the allocator
    /// has to steal.
    PolyphonicVoiceStealing,
    /// An instrument carrying its own insert effect chain.
    InstrumentInserts,
    /// Track sends into return busses, and a master effect chain.
    SendsReturnsMaster,
    /// An arrangement whose tempo changes during the rendered window.
    TempoMapArrangement,
    /// One patch or instrument referenced from more than one place.
    SharedPatchOrInstrument,
}

impl CorpusCategory {
    /// Every category, in declaration order.
    pub fn all() -> impl Iterator<Item = Self> {
        use strum::IntoEnumIterator;
        Self::iter()
    }
}

/// How a V1-to-V2 audio difference is to be read.
///
/// These are the four classes of the master plan's audio comparison policy. A
/// claim carries one so that a difference is never reported as generic error:
/// the class is what tells a reviewer whether a deviation is a regression or the
/// point of the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComparisonClass {
    /// Same DSP, same timing, same inputs — the outputs are expected to match
    /// sample for sample within the stated tolerance.
    ExactParity,
    /// Same audible behaviour, but a changed operation order or an explicit
    /// interpolation makes the numbers drift.
    FeatureParity,
    /// V2 deliberately does something else, and the new rule is the thing under
    /// test.
    IntentionalCorrection,
    /// V2 does not implement this yet. Reported as unsupported, never rendered
    /// silently with the behaviour missing.
    UnsupportedScope,
}

impl ComparisonClass {
    /// Whether this class may appear in a case's `preserve` list.
    ///
    /// Preservation is a claim that the behaviour survives, so only the two
    /// parity classes can express it. A "preserved" intentional correction is a
    /// contradiction, and it is the kind that reads as reasonable in review.
    #[must_use]
    pub fn is_preservation(self) -> bool {
        matches!(self, Self::ExactParity | Self::FeatureParity)
    }

    /// Whether this class may appear in a case's `change` list. The exact
    /// complement of [`Self::is_preservation`].
    #[must_use]
    pub fn is_change(self) -> bool {
        !self.is_preservation()
    }
}

/// What a case expects of two renders of the same input by the same build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Determinism {
    /// Two renders must produce identical bytes. Every V1 case is this, and a
    /// case that is not would first have to explain why.
    BitExact,
}

/// Everything a render of one case needs, beyond the input file.
///
/// The fields are the `pertylizer render` arguments that change the audio.
/// Deliberately not the whole argument list: an output path or a receipt path
/// belongs to the run, not to the case.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSettings {
    /// Sample rate to render at.
    pub sample_rate: DeviceSampleRate,
    /// Sample format of the reference WAV. `Float32` for every case, because an
    /// integer depth quantizes the very differences the comparison measures.
    pub bit_depth: WavFormat,
    /// How much of the arrangement to render, from its start.
    pub seconds: Seconds,
    /// Extra audio captured after the transport stops, for reverb and delay
    /// tails.
    pub tail_seconds: Seconds,
    /// Tracks to solo, by id or by unique name. Empty means the full mix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solo_tracks: Vec<TrackSelector>,
    /// Tracks to mute, by id or by unique name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute_tracks: Vec<TrackSelector>,
}

/// One statement about what V2 must do with a behaviour this case exercises.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorClaim {
    /// Stable identifier, unique across the whole manifest, so a review note or
    /// a test name can cite one claim rather than "the third bullet".
    pub id: BehaviorClaimId,
    /// The behaviour, stated so that a render can contradict it.
    pub behavior: String,
    /// How a difference here is to be read.
    pub class: ComparisonClass,
    /// Why the change is intended. Required on a `change` claim and absent on a
    /// `preserve` one — preserving V1 needs no justification, changing it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// One corpus entry: an input, how to render it, and what its render means.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusCase {
    /// Stable identifier, `CORPUS-NNNN`.
    pub id: CorpusCaseId,
    /// One-line human title.
    pub title: String,
    /// Which of the master plan's categories this case covers.
    pub category: CorpusCategory,
    /// What this case is here to exercise, in a sentence or two.
    pub purpose: String,
    /// Input project, relative to the directory holding the manifest.
    pub input: String,
    /// Lowercase hex SHA-256 of the input file.
    ///
    /// The corpus is only a baseline if its inputs are pinned: an edited project
    /// silently rebaselines every measurement taken against it. The digest turns
    /// that into a loud failure.
    pub sha256: String,
    /// Render settings for the reference render.
    pub render: RenderSettings,
    /// External PRNG seed for the render, if one is ever supplied.
    ///
    /// `None` on every current case, and that is a statement rather than an
    /// omission: V1 has no render-level seed. Every random-family module derives
    /// its state from its voice index and module instance number
    /// (`PolyModule::set_seed`), so a render is reproducible without one and
    /// there is nothing to record. The field exists because a V2 that introduces
    /// an explicit seed must not be able to change what a case renders without
    /// changing the case.
    ///
    /// The key is **required** and every current case spells it `null`. Neither
    /// `serde(default)` nor `skip_serializing_if` is used, deliberately: with
    /// either one, "this case has no seed" and "somebody forgot to write a seed"
    /// are the same file. A missing key is a parse error, so the answer is
    /// written down rather than inferred from an absence.
    pub seed: Seed,
    /// What two renders of this case by one build must produce.
    pub determinism: Determinism,
    /// Behaviour V2 is intended to preserve.
    pub preserve: Vec<BehaviorClaim>,
    /// Behaviour V2 is intended to change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change: Vec<BehaviorClaim>,
}

impl CorpusCase {
    /// Absolute path of this case's input, given the directory holding the
    /// manifest.
    #[must_use]
    pub fn input_path(&self, corpus_dir: &Path) -> PathBuf {
        corpus_dir.join(&self.input)
    }
}

/// A category the corpus does not cover yet, and why.
///
/// A gap is recorded rather than left implicit so that reading the manifest
/// answers "what is missing" without reading the master plan alongside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedCategory {
    /// The uncovered category.
    pub category: CorpusCategory,
    /// Why there is no case yet, and what adding one needs.
    pub why_absent: String,
}

/// The corpus, as loaded from disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusManifest {
    /// Shape version. Must equal [`MANIFEST_VERSION`].
    pub manifest_version: u32,
    /// What the corpus is for, in the file itself, so it is readable without
    /// this module.
    pub description: String,
    /// The cases, in identifier order.
    pub cases: Vec<CorpusCase>,
    /// Categories with no case yet.
    pub planned: Vec<PlannedCategory>,
}

/// A manifest that cannot be trusted as a baseline.
///
/// Every variant names the case or category at fault, because a corpus failure
/// is read by whoever did not write the entry.
#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    /// The manifest file could not be read.
    #[error("cannot read the corpus manifest {path}: {source}")]
    Read {
        /// The manifest that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The manifest is not valid JSON, or does not match the model.
    #[error("cannot parse the corpus manifest {path}: {source}")]
    Parse {
        /// The manifest that could not be parsed.
        path: PathBuf,
        /// What serde said.
        source: serde_json::Error,
    },
    /// The manifest declares a shape this build does not speak.
    #[error("corpus manifest version {found}, but this build speaks version {expected}")]
    Version {
        /// The version the file declares.
        found: u32,
        /// The version this build implements.
        expected: u32,
    },
    /// Two entries share an identifier.
    #[error("{what} {id} appears more than once")]
    DuplicateId {
        /// Which kind of entry collided, e.g. `case`.
        what: &'static str,
        /// The repeated identifier.
        id: String,
    },
    /// An identifier does not follow its series' shape.
    #[error("{what} identifier {id:?} does not match {expected}")]
    MalformedId {
        /// Which kind of entry was malformed.
        what: &'static str,
        /// The identifier as written.
        id: String,
        /// The shape it had to have.
        expected: &'static str,
    },
    /// A claim identifier does not name the case it sits in.
    #[error(
        "case {case}: claim identifier {claim} belongs to a different case — a claim id is its \
         case's id plus a suffix, so that a citation says where to look"
    )]
    ClaimOutsideCase {
        /// The case the claim sits in.
        case: CorpusCaseId,
        /// The claim whose identifier names another case.
        claim: BehaviorClaimId,
    },
    /// A digest is not a SHA-256.
    ///
    /// Caught here rather than at [`CorpusManifest::verify_inputs`] because a
    /// truncated or typo'd digest surfaces there as "the input changed", which
    /// sends whoever reads it looking at the project instead of at the manifest.
    #[error("case {case}: {sha256:?} is not a lowercase hex SHA-256")]
    MalformedDigest {
        /// The case at fault.
        case: CorpusCaseId,
        /// The digest as written.
        sha256: String,
    },
    /// An input path escapes the corpus directory or is absolute.
    #[error("case {case}: input {input:?} must be a relative path inside the corpus directory")]
    InputOutsideCorpus {
        /// The case at fault.
        case: CorpusCaseId,
        /// The path as written.
        input: String,
    },
    /// A claim's class does not fit the list it appears in.
    #[error("case {case}: claim {claim} is in the {list} list but its class is {class:?}")]
    ClaimClass {
        /// The case at fault.
        case: CorpusCaseId,
        /// The claim at fault.
        claim: BehaviorClaimId,
        /// Which list it appeared in, `preserve` or `change`.
        list: &'static str,
        /// The class it declared.
        class: ComparisonClass,
    },
    /// A change claim does not say why the change is intended.
    #[error("case {case}: change claim {claim} has no rationale")]
    MissingRationale {
        /// The case at fault.
        case: CorpusCaseId,
        /// The claim at fault.
        claim: BehaviorClaimId,
    },
    /// A preserve claim carries a rationale, which belongs on a change.
    #[error("case {case}: preserve claim {claim} carries a rationale, which only a change takes")]
    UnexpectedRationale {
        /// The case at fault.
        case: CorpusCaseId,
        /// The claim at fault.
        claim: BehaviorClaimId,
    },
    /// A case claims nothing, so rendering it proves nothing.
    #[error("case {case} declares no preserved behaviour")]
    NoPreservedBehavior {
        /// The case at fault.
        case: CorpusCaseId,
    },
    /// The render settings would be refused by `pertylizer render`.
    #[error("case {case}: {source}")]
    RenderSettings {
        /// The case at fault.
        case: CorpusCaseId,
        /// Why the renderer would refuse it.
        source: Box<RenderError>,
    },
    /// A category is neither covered by a case nor recorded as a gap.
    #[error(
        "category {category:?} is neither covered by a case nor listed as planned — \
         every category the master plan names must be accounted for"
    )]
    UnaccountedCategory {
        /// The category nothing mentions.
        category: CorpusCategory,
    },
    /// A category is both covered and listed as a gap.
    #[error("category {category:?} is listed as planned but case {case} already covers it")]
    PlannedButCovered {
        /// The contradictory category.
        category: CorpusCategory,
        /// A case that covers it.
        case: CorpusCaseId,
    },
}

impl CorpusManifest {
    /// Read and validate the manifest at `path`.
    ///
    /// Validation is not optional and not a separate call: a manifest that
    /// loaded but did not validate would be used, and every consumer would have
    /// to remember to check it.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] for an unreadable or unparseable file, a version
    /// this build does not speak, or any of the consistency rules in
    /// [`Self::validate`].
    pub fn load(path: &Path) -> Result<Self, CorpusError> {
        let text = std::fs::read_to_string(path).map_err(|source| CorpusError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let manifest: Self = serde_json::from_str(&text).map_err(|source| CorpusError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Serialize as pretty-printed JSON with a trailing newline.
    ///
    /// The generator writes the manifest back after refreshing digests, so this
    /// has to produce the same bytes the file already holds for an unchanged
    /// corpus — which it does, because every map in the model is ordered.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut json = serde_json::to_vec_pretty(self)?;
        json.push(b'\n');
        Ok(json)
    }

    /// The case with this identifier.
    #[must_use]
    pub fn case(&self, id: &CorpusCaseId) -> Option<&CorpusCase> {
        self.cases.iter().find(|case| &case.id == id)
    }

    /// Check every rule the corpus depends on to be a baseline.
    ///
    /// Deliberately does not touch the filesystem: a caller that only wants to
    /// read the metadata should not need the projects checked out. Digest and
    /// existence checks are [`Self::verify_inputs`].
    ///
    /// # Errors
    ///
    /// Returns the first [`CorpusError`] found.
    pub fn validate(&self) -> Result<(), CorpusError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(CorpusError::Version {
                found: self.manifest_version,
                expected: MANIFEST_VERSION,
            });
        }

        let mut case_ids = BTreeSet::new();
        let mut claim_ids = BTreeSet::new();
        for case in &self.cases {
            // Unreachable for a *deserialized* manifest — `TryFrom<String>`
            // rejects a malformed identifier before it can reach a field. It
            // stays because that is not the only way a manifest gets built:
            // `CorpusCaseId::new_unchecked` exists for the `const` fixture
            // table, and this is the check that catches a typo there.
            if !case.id.is_well_formed() {
                return Err(CorpusError::MalformedId {
                    what: "case",
                    id: case.id.to_string(),
                    expected: CorpusCaseId::SHAPE,
                });
            }
            if !case_ids.insert(case.id.as_str()) {
                return Err(CorpusError::DuplicateId {
                    what: "case",
                    id: case.id.to_string(),
                });
            }
            // `is_absolute` is not enough on its own: on Windows it is false for
            // a rooted-but-prefixless path like `\projects\x`, while `Path::join`
            // still treats that as rooted and throws the corpus directory away.
            // Rejecting `RootDir` and `Prefix` components closes that, so the
            // rule means the same thing on every platform the build targets.
            let input = Path::new(&case.input);
            if input.is_absolute()
                || input.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err(CorpusError::InputOutsideCorpus {
                    case: case.id.clone(),
                    input: case.input.clone(),
                });
            }
            if !is_sha256_hex(&case.sha256) {
                return Err(CorpusError::MalformedDigest {
                    case: case.id.clone(),
                    sha256: case.sha256.clone(),
                });
            }
            crate::render::validate_render_bounds(
                case.render.seconds,
                case.render.tail_seconds,
                case.render.sample_rate.as_u32(),
            )
            .map_err(|source| CorpusError::RenderSettings {
                case: case.id.clone(),
                source: Box::new(source),
            })?;

            if case.preserve.is_empty() {
                return Err(CorpusError::NoPreservedBehavior {
                    case: case.id.clone(),
                });
            }
            for (list, claims, accepts) in [
                (
                    "preserve",
                    &case.preserve,
                    ComparisonClass::is_preservation as fn(ComparisonClass) -> bool,
                ),
                ("change", &case.change, ComparisonClass::is_change),
            ] {
                for claim in claims {
                    if !claim.id.is_well_formed() {
                        return Err(CorpusError::MalformedId {
                            what: "claim",
                            id: claim.id.to_string(),
                            expected: BehaviorClaimId::SHAPE,
                        });
                    }
                    // A claim id is its case's id plus a suffix, so a citation
                    // says where to look. Without this a claim can name a case
                    // it does not sit in, and the reader is sent to the wrong
                    // entry by an identifier that looks perfectly well formed.
                    if !claim
                        .id
                        .as_str()
                        .starts_with(&format!("{}-", case.id.as_str()))
                    {
                        return Err(CorpusError::ClaimOutsideCase {
                            case: case.id.clone(),
                            claim: claim.id.clone(),
                        });
                    }
                    if !claim_ids.insert(claim.id.as_str()) {
                        return Err(CorpusError::DuplicateId {
                            what: "claim",
                            id: claim.id.to_string(),
                        });
                    }
                    if !accepts(claim.class) {
                        return Err(CorpusError::ClaimClass {
                            case: case.id.clone(),
                            claim: claim.id.clone(),
                            list,
                            class: claim.class,
                        });
                    }
                    match (list, claim.rationale.is_some()) {
                        ("change", false) => {
                            return Err(CorpusError::MissingRationale {
                                case: case.id.clone(),
                                claim: claim.id.clone(),
                            });
                        }
                        ("preserve", true) => {
                            return Err(CorpusError::UnexpectedRationale {
                                case: case.id.clone(),
                                claim: claim.id.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        // A gap is recorded once. Listed twice, the second entry's reason is
        // invisible — `any` stops at the first — and the coverage table a reader
        // builds by counting `planned` says one more category is uncovered than
        // there are categories left to cover.
        let mut planned_categories = BTreeSet::new();
        for gap in &self.planned {
            if !planned_categories.insert(gap.category) {
                return Err(CorpusError::DuplicateId {
                    what: "planned category",
                    id: format!("{:?}", gap.category),
                });
            }
        }

        // Every category the master plan names is either exercised or recorded
        // as a gap. This is what keeps the corpus honest as it grows: a new
        // category cannot be added to the enum and then forgotten.
        for category in CorpusCategory::all() {
            let covering = self.cases.iter().find(|case| case.category == category);
            let planned = self.planned.iter().any(|gap| gap.category == category);
            match (covering, planned) {
                (Some(case), true) => {
                    return Err(CorpusError::PlannedButCovered {
                        category,
                        case: case.id.clone(),
                    });
                }
                (None, false) => return Err(CorpusError::UnaccountedCategory { category }),
                _ => {}
            }
        }

        Ok(())
    }

    /// Check that every case's input exists and still hashes to its recorded
    /// digest.
    ///
    /// Separate from [`Self::validate`] because it reads files: a caller
    /// inspecting the metadata should not pay for it, and a caller about to
    /// render must.
    ///
    /// # Errors
    ///
    /// Returns one message per failing case. An empty vector means every input
    /// is present and unchanged.
    #[must_use]
    pub fn verify_inputs(&self, corpus_dir: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        for case in &self.cases {
            let path = case.input_path(corpus_dir);
            match crate::render::receipt::FileDigest::of(&path) {
                Err(error) => problems.push(format!("{}: {error}", case.id)),
                Ok(digest) if digest.sha256 != case.sha256 => problems.push(format!(
                    "{}: {} has digest {} but the manifest records {} — the input changed, so \
                     every baseline measured against it is stale",
                    case.id, case.input, digest.sha256, case.sha256
                )),
                Ok(_) => {}
            }
        }
        problems
    }
}

/// Whether `digest` is 64 lowercase hex digits, the shape
/// [`FileDigest`](crate::render::receipt::FileDigest) writes.
fn is_sha256_hex(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// If `id` begins `<prefix>-NNNN` with exactly four digits, everything after
/// those digits; otherwise `None`.
///
/// Returning the remainder rather than a bool is what lets one rule serve both
/// identifier series: a case id is this with an empty remainder, and a claim id
/// is this with a `-suffix`.
fn series_digits<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = id.strip_prefix(prefix)?.strip_prefix('-')?;
    let (digits, tail) = rest.split_at_checked(4)?;
    digits.bytes().all(|b| b.is_ascii_digit()).then_some(tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed claim identifier, through the validated path.
    fn claim_id(id: &str) -> BehaviorClaimId {
        id.parse().expect("a well-formed claim id")
    }

    /// A minimal valid manifest, so each test below can break exactly one rule.
    fn manifest() -> CorpusManifest {
        CorpusManifest {
            manifest_version: MANIFEST_VERSION,
            description: "test".to_string(),
            cases: vec![CorpusCase {
                id: CorpusCaseId::new_unchecked("CORPUS-0001"),
                title: "Case".to_string(),
                category: CorpusCategory::SubtractiveVoice,
                purpose: "purpose".to_string(),
                input: "projects/case.ptz".to_string(),
                sha256: "0".repeat(64),
                render: RenderSettings {
                    sample_rate: DeviceSampleRate::new(44_100),
                    bit_depth: WavFormat::Float32,
                    seconds: Seconds::new(2.0),
                    tail_seconds: Seconds::new(0.5),
                    solo_tracks: Vec::new(),
                    mute_tracks: Vec::new(),
                },
                seed: Seed::none(),
                determinism: Determinism::BitExact,
                preserve: vec![BehaviorClaim {
                    id: claim_id("CORPUS-0001-P1"),
                    behavior: "behaviour".to_string(),
                    class: ComparisonClass::ExactParity,
                    rationale: None,
                }],
                change: Vec::new(),
            }],
            planned: CorpusCategory::all()
                .filter(|c| *c != CorpusCategory::SubtractiveVoice)
                .map(|category| PlannedCategory {
                    category,
                    why_absent: "not yet".to_string(),
                })
                .collect(),
        }
    }

    /// The fixture the other tests mutate has to pass on its own, or every
    /// assertion below would pass for the wrong reason.
    #[test]
    fn the_baseline_fixture_validates() {
        manifest().validate().expect("fixture should validate");
    }

    /// A manifest is only a baseline if every category is accounted for.
    /// Dropping a gap entry must fail rather than shrink the corpus silently.
    #[test]
    fn an_unaccounted_category_is_rejected() {
        let mut m = manifest();
        m.planned.pop();
        assert!(matches!(
            m.validate(),
            Err(CorpusError::UnaccountedCategory { .. })
        ));
    }

    /// Claiming a category is both covered and planned is a contradiction, and
    /// the covered side would win silently.
    #[test]
    fn a_category_cannot_be_both_covered_and_planned() {
        let mut m = manifest();
        m.planned.push(PlannedCategory {
            category: CorpusCategory::SubtractiveVoice,
            why_absent: "contradiction".to_string(),
        });
        assert!(matches!(
            m.validate(),
            Err(CorpusError::PlannedButCovered { .. })
        ));
    }

    /// A category listed as a gap twice hides the second entry's reason and
    /// makes the manifest's own coverage count disagree with the enum.
    #[test]
    fn a_category_cannot_be_listed_as_planned_twice() {
        let mut m = manifest();
        let duplicate = m.planned[0].clone();
        m.planned.push(duplicate);
        assert!(matches!(m.validate(), Err(CorpusError::DuplicateId { .. })));
    }

    /// An intentional correction listed as preserved reads as reasonable in
    /// review and means the opposite of what it says.
    #[test]
    fn a_correction_cannot_be_listed_as_preserved() {
        let mut m = manifest();
        m.cases[0].preserve[0].class = ComparisonClass::IntentionalCorrection;
        assert!(matches!(m.validate(), Err(CorpusError::ClaimClass { .. })));
    }

    /// The exit gate requires every intentional change to be named, which a
    /// change with no rationale is not.
    #[test]
    fn a_change_needs_a_rationale() {
        let mut m = manifest();
        m.cases[0].change.push(BehaviorClaim {
            id: claim_id("CORPUS-0001-C1"),
            behavior: "changed".to_string(),
            class: ComparisonClass::IntentionalCorrection,
            rationale: None,
        });
        assert!(matches!(
            m.validate(),
            Err(CorpusError::MissingRationale { .. })
        ));
    }

    /// Two claims sharing an id make a citation ambiguous, which is the one
    /// thing the id exists to prevent.
    #[test]
    fn claim_identifiers_are_unique_across_the_manifest() {
        let mut m = manifest();
        let duplicate = m.cases[0].preserve[0].clone();
        m.cases[0].preserve.push(duplicate);
        assert!(matches!(m.validate(), Err(CorpusError::DuplicateId { .. })));
    }

    /// A case that would be refused by `pertylizer render` must be refused
    /// here, hours earlier and with the case named.
    #[test]
    fn render_settings_are_held_to_the_render_commands_bounds() {
        let mut m = manifest();
        m.cases[0].render.sample_rate = DeviceSampleRate::new(1);
        assert!(matches!(
            m.validate(),
            Err(CorpusError::RenderSettings { .. })
        ));
    }

    /// An input reachable outside the corpus directory is not pinned by the
    /// corpus, so its digest guarantees nothing.
    #[test]
    fn an_input_may_not_escape_the_corpus_directory() {
        let mut m = manifest();
        m.cases[0].input = "../../assets/examples/projects/Neon Horizon.ptz".to_string();
        assert!(matches!(
            m.validate(),
            Err(CorpusError::InputOutsideCorpus { .. })
        ));
    }

    /// A digest that is not a SHA-256 pins nothing, and left to `verify_inputs`
    /// it reads as "the input changed" — which points the reader at the wrong
    /// file.
    #[test]
    fn a_digest_that_is_not_a_sha256_is_rejected() {
        for bad in ["", "0".repeat(63).as_str(), "g".repeat(64).as_str(), "ABC"] {
            let mut m = manifest();
            m.cases[0].sha256 = bad.to_string();
            assert!(
                matches!(m.validate(), Err(CorpusError::MalformedDigest { .. })),
                "{bad:?} should not pass as a digest"
            );
        }
    }

    /// A rooted input escapes the corpus directory on Windows even though
    /// `is_absolute` calls it relative, so the component check has to catch it.
    #[test]
    fn a_rooted_input_is_rejected_whatever_the_platform_thinks_of_it() {
        let mut m = manifest();
        m.cases[0].input = "/etc/passwd".to_string();
        assert!(matches!(
            m.validate(),
            Err(CorpusError::InputOutsideCorpus { .. })
        ));
    }

    /// A claim whose identifier names a different case sends a reader to the
    /// wrong entry, and does it with an id that passes every shape check.
    #[test]
    fn a_claim_must_be_identified_by_its_own_case() {
        let mut m = manifest();
        m.cases[0].preserve[0].id = claim_id("CORPUS-0002-P1");
        assert!(matches!(
            m.validate(),
            Err(CorpusError::ClaimOutsideCase { .. })
        ));
    }

    /// The seed key is required, so that "this case has no seed" and "somebody
    /// forgot to write one" cannot be the same file. A missing key is a parse
    /// error; an explicit `null` is the answer.
    #[test]
    fn the_seed_key_is_required_and_null_is_an_answer() {
        let mut value = serde_json::to_value(manifest()).expect("serialize");
        assert!(
            value["cases"][0]
                .get("seed")
                .is_some_and(serde_json::Value::is_null),
            "a seedless case must serialize an explicit null, not omit the key"
        );

        value["cases"][0]
            .as_object_mut()
            .expect("a case object")
            .remove("seed");
        let error = serde_json::from_value::<CorpusManifest>(value)
            .expect_err("a missing seed key must not parse");
        assert!(error.to_string().contains("seed"), "{error}");
    }

    /// A misspelled optional field is the failure this format exists to prevent:
    /// `sead`, or `solo_track` for `solo_tracks`, would otherwise be dropped in
    /// silence and the case would render with settings nobody chose.
    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let mut value = serde_json::to_value(manifest()).expect("serialize");
        value["cases"][0]["render"]
            .as_object_mut()
            .expect("a render object")
            .insert("solo_track".to_string(), serde_json::json!(["Lead"]));
        let error = serde_json::from_value::<CorpusManifest>(value)
            .expect_err("an unknown field must not parse");
        assert!(error.to_string().contains("solo_track"), "{error}");
    }

    /// A malformed identifier is refused where it enters, so it cannot reach the
    /// rest of the model at all. `validate` checks the same rule again, for
    /// identifiers built in code rather than deserialized.
    #[test]
    fn a_malformed_identifier_is_refused_at_the_boundary() {
        let mut value = serde_json::to_value(manifest()).expect("serialize");
        value["cases"][0]["id"] = serde_json::json!("CORPUS-1");
        let error = serde_json::from_value::<CorpusManifest>(value)
            .expect_err("a short case id must not parse");
        assert!(error.to_string().contains("CORPUS-NNNN"), "{error}");

        assert!(CorpusCaseId::try_from("CORPUS-0001".to_string()).is_ok());
        assert!(CorpusCaseId::try_from("CORPUS-1".to_string()).is_err());
        assert!(BehaviorClaimId::try_from("CORPUS-0001-P1".to_string()).is_ok());
        assert!(BehaviorClaimId::try_from("CORPUS-0001".to_string()).is_err());
    }

    /// A manifest written for another shape must be refused, not read field by
    /// field until something happens to be missing.
    #[test]
    fn a_foreign_manifest_version_is_refused() {
        let mut m = manifest();
        m.manifest_version = MANIFEST_VERSION + 1;
        assert!(matches!(m.validate(), Err(CorpusError::Version { .. })));
    }

    /// The identifier shape is checked, not assumed: `CORPUS-1` sorts and reads
    /// differently from `CORPUS-0001`.
    #[test]
    fn case_identifiers_must_carry_four_digits() {
        let case = |id: &str| id.parse::<CorpusCaseId>().is_ok();
        assert!(case("CORPUS-0001"));
        assert!(!case("CORPUS-1"));
        assert!(!case("CORPUS-00001"));
        assert!(!case("CORPUS-000a"));
        assert!(!case("EVD-0001"));
        // A claim id is a case id plus a suffix, and neither shape accepts the
        // other — which is the whole point of them being separate types.
        let claim = |id: &str| id.parse::<BehaviorClaimId>().is_ok();
        assert!(claim("CORPUS-0001-P1"));
        assert!(!claim("CORPUS-0001"), "a bare case id is not a claim id");
        assert!(!claim("CORPUS-0001-"), "an empty suffix names nothing");
        assert!(!case("CORPUS-0001-P1"), "a claim id is not a case id");
    }

    /// The manifest round-trips through JSON unchanged, which is what lets the
    /// generator rewrite digests without reformatting the file.
    #[test]
    fn the_manifest_round_trips_through_json() {
        let original = manifest();
        let json = original.to_json().expect("serialize");
        let reparsed: CorpusManifest = serde_json::from_slice(&json).expect("parse");
        assert_eq!(
            reparsed.to_json().expect("reserialize"),
            json,
            "a round trip must be byte-stable or the generator would churn the file"
        );
    }
}
