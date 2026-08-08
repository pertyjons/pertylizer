//! The machine-readable result a `pertylizer render` run emits.
//!
//! The receipt exists so a consumer can tell, after the fact, exactly what was
//! rendered: which build did it, which bytes went in, which bytes came out, and
//! what the effective mix was. Everything in it is either an input to the render
//! or a measurement of its output — nothing is inferred.

use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::RenderError;

/// Version of the command contract and of this receipt's shape.
///
/// A caller passes `--protocol-version` and gets it echoed back. Adding an
/// optional flag or an extra receipt field does not change it; removing or
/// re-meaning anything does.
pub const PROTOCOL_VERSION: u32 = 1;

/// The complete result of one render.
#[derive(Debug, Clone, Serialize)]
pub struct RenderReceipt {
    /// Contract version this receipt speaks.
    pub protocol_version: u32,
    /// Which build produced it.
    pub renderer: RendererInfo,
    /// The project that was rendered.
    pub input: FileDigest,
    /// The WAV that was written.
    pub output: FileDigest,
    /// What the audio is.
    pub audio: AudioInfo,
    /// Which tracks were audible, and why.
    pub mix: MixInfo,
    /// What the project loader reported — instrument counts, and whatever else
    /// it summarises about the file it just applied, with a count of anything
    /// it could not reconstruct appended.
    ///
    /// A one-line human summary. Each thing the load lost is also listed
    /// individually at the head of `warnings`, which is the field to read
    /// programmatically.
    pub load_summary: String,
    /// Everything non-fatal the load, the mix resolution and the render had to
    /// say. Empty when all three were clean.
    pub warnings: Vec<String>,
    /// The invocation, argument by argument.
    ///
    /// An argv array rather than a shell string: it round-trips paths
    /// containing spaces, quotes, or newlines with no quoting rules to get
    /// wrong, and it is what a caller re-executing this would pass anyway.
    pub command: Vec<String>,
}

/// Identity of the build that rendered.
#[derive(Debug, Clone, Serialize)]
pub struct RendererInfo {
    /// Crate version, e.g. `0.316.0`.
    pub version: &'static str,
    /// UTC build date.
    pub build_date: &'static str,
    /// Commit the build came from, absent when built without git.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<&'static str>,
    /// Branch the build came from, absent when built without git.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<&'static str>,
    /// Whether the working tree had uncommitted changes at build time.
    ///
    /// `Some(true)` means the commit above does **not** identify the source
    /// that produced this binary. `None` means the build could not tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
}

impl RendererInfo {
    /// Read the build stamp `build.rs` embedded.
    #[must_use]
    pub fn current() -> Self {
        /// Empty means "git was unavailable at build time", not "empty value".
        fn non_empty(value: &'static str) -> Option<&'static str> {
            (!value.is_empty()).then_some(value)
        }

        Self {
            version: env!("CARGO_PKG_VERSION"),
            build_date: env!("BUILD_DATE"),
            commit: non_empty(env!("GIT_COMMIT_HASH")),
            branch: non_empty(env!("GIT_BRANCH")),
            dirty: match env!("GIT_DIRTY") {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            },
        }
    }
}

/// A file identified by its content, not just by its name.
#[derive(Debug, Clone, Serialize)]
pub struct FileDigest {
    /// Absolute path when it could be resolved, lossily converted to text.
    ///
    /// A `PathBuf` here would make the whole receipt *fail to serialize* on a
    /// path that is not valid UTF-8 — after the WAV had already been written.
    /// JSON cannot carry arbitrary bytes anyway, so the lossy form is the
    /// honest one; the digest below is what actually identifies the file.
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the whole file.
    pub sha256: String,
}

impl FileDigest {
    /// Hash `path` in streaming fashion.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Digest`] if the file cannot be opened or read.
    pub fn of(path: &Path) -> Result<Self, RenderError> {
        let digest = |source| RenderError::Digest {
            path: path.to_path_buf(),
            source,
        };
        let mut file = std::fs::File::open(path).map_err(digest)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut bytes = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(digest)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            bytes += read as u64;
        }
        let sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            path: super::headless::absolute(path)
                .to_string_lossy()
                .into_owned(),
            bytes,
            sha256,
        })
    }
}

/// What the rendered audio actually is.
#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    /// Sample rate the render ran at, which is what the WAV declares.
    pub sample_rate: u32,
    /// Channels in the WAV.
    pub channels: u16,
    /// Bits per sample the WAV header declares.
    pub bit_depth: u16,
    /// `"int"` or `"float"`. Recorded next to `bit_depth` because the two
    /// together are what identifies the encoding: 32 alone is ambiguous.
    pub sample_format: &'static str,
    /// Frames (samples per channel) written.
    pub frames: u64,
    /// Length in seconds, tail included.
    pub duration_seconds: f32,
    /// Requested window length, tail excluded.
    pub requested_seconds: f32,
    /// Extra audio rendered after the transport stopped.
    pub tail_seconds: f32,
    /// Absolute song ticks the window covered, `start` inclusive.
    pub start_tick: u64,
    /// Absolute song ticks the window covered, `end` exclusive.
    pub end_tick: u64,
    /// Largest absolute sample amplitude. 0.0 means the render was silent.
    pub peak: f32,
}

/// The effective mix, resolved.
#[derive(Debug, Clone, Serialize)]
pub struct MixInfo {
    /// Track ids the `--solo-track` flags resolved to.
    pub soloed: Vec<u16>,
    /// Track ids the `--mute-track` flags resolved to.
    pub muted: Vec<u16>,
    /// The tracks that were actually heard, in display order.
    pub audible: Vec<MixTrack>,
}

/// One audible track, named as well as identified so a receipt is readable
/// without the project next to it.
#[derive(Debug, Clone, Serialize)]
pub struct MixTrack {
    /// The track's stable id.
    pub id: u16,
    /// The track's name at render time.
    pub name: String,
}

impl RenderReceipt {
    /// Serialize as pretty-printed JSON with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Receipt`] if serialization fails.
    pub fn to_json(&self) -> Result<Vec<u8>, RenderError> {
        let mut json = serde_json::to_vec_pretty(self).map_err(RenderError::Receipt)?;
        json.push(b'\n');
        Ok(json)
    }
}
