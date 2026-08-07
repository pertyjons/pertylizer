//! Offline render core.
//!
//! The work that the MCP `render_to_wav` tool and the headless `pertylizer
//! render` command both do: resolve a tick window against a song, build an
//! offline engine session, render that window, and write the buffer to a WAV in
//! the caller's chosen [`WavFormat`].
//!
//! It lives here rather than in `mcp_bridge` so a caller does not have to speak
//! a protocol to render a file. Nothing in this module's inputs, outputs, or
//! errors mentions an MCP session, [`crate::mcp_shared::McpSharedState`], or
//! `McpBridgeError`; the song arrives as a plain `SharedSong` handle and
//! failures arrive as [`RenderError`].
//!
//! Two seams into the existing code are deliberately left alone:
//!
//! - [`synth_mcp::AnalysisScope`] is still the scope type. It is plain
//!   configuration data (which optional signal stages to reconstruct, and at
//!   what sample rate) with no protocol semantics, and it is what
//!   [`crate::audio::arrangement_render`] already takes. Relocating it out of
//!   `synth_mcp` is a separate cleanup, not part of extracting this core.
//! - The offline renderer still fails with `McpBridgeError`, so
//!   [`RenderError::Render`] carries its rendered message rather than the value.
//!   Re-typing all of `arrangement_render` would dwarf this extraction.

use std::path::PathBuf;

pub mod command;
pub mod headless;
pub mod mix;
pub mod receipt;
pub mod wav;

pub use crate::audio::wav_format::WavFormat;
pub use command::{RenderCommand, run_render_command};
pub use mix::{AppliedMix, MixSelection, MixSelectionError, TrackSelector, apply_mix_selection};
pub use receipt::{PROTOCOL_VERSION, RenderReceipt};
pub use wav::{
    TickWindow, WavRenderOutcome, WavRenderRequest, render_window_to_wav, tick_window_from_seconds,
};

/// Lowest sample rate the render command accepts, below telephone quality.
pub const MIN_RENDER_SAMPLE_RATE: u32 = 8_000;

/// Highest sample rate the render command accepts.
pub const MAX_RENDER_SAMPLE_RATE: u32 = 384_000;

/// Longest tail the render command accepts.
///
/// Matches the bound the `render_to_wav` MCP tool already validates against.
/// Nothing downstream clamps a tail — it is added on top of the window and its
/// frame count goes straight into a `Vec::with_capacity` — so without this a
/// `--tail-seconds 100000` aborts the process on the allocation instead of
/// returning an error.
pub const MAX_TAIL_SECONDS: f32 = 30.0;

/// Largest interleaved audio buffer one render may allocate, in bytes.
///
/// `--seconds` alone does not bound memory. The renderer sizes one
/// `Vec::with_capacity((seconds + tail) * sample_rate * channels)` of `f32`,
/// so the 300-second maximum means 105 MB at 44.1 kHz but 921 MB at the
/// 384 kHz `--sample-rate` ceiling — and that allocation aborts the process
/// rather than returning an error, exactly the failure [`MAX_TAIL_SECONDS`]
/// exists to prevent. The bound is on the product, which is the thing that
/// actually costs memory.
pub const MAX_RENDER_BYTES: u64 = 512 * 1024 * 1024;

/// A failure somewhere in the load-validate-render-write sequence.
///
/// Every variant except [`Self::WriteWav`], [`Self::WriteReceipt`], and a
/// [`Self::Digest`] naming the *output* file means nothing was written to the
/// output paths at all. The output digest is taken after the WAV lands, so a
/// failure there leaves a complete WAV and no receipt.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The input file is missing, unreadable, or not a project the loader
    /// accepts.
    #[error("cannot load {path}: {message}")]
    ProjectLoad {
        /// The input that could not be loaded.
        path: PathBuf,
        /// What the loader said.
        message: String,
    },
    /// The engine never finished applying the loaded project. Something is
    /// wrong with the engine, not with the input.
    #[error(
        "loading {path} did not settle: the engine drained {processed} of \
         {enqueued} commands"
    )]
    LoadDidNotSettle {
        /// The input that was being loaded.
        path: PathBuf,
        /// Commands the loader successfully enqueued.
        enqueued: u64,
        /// Commands the engine drained before giving up.
        processed: u64,
    },
    /// An output path is the input path.
    ///
    /// Rendering over the input would destroy the project *and* still produce a
    /// receipt that reads as a clean run, because the input digest is taken
    /// before the write.
    #[error("{what} is the input file {path} — the render would destroy it")]
    OutputOverwritesInput {
        /// Which argument collided, e.g. `--output`.
        what: &'static str,
        /// The input path both arguments named.
        path: PathBuf,
    },
    /// The WAV and the receipt would be written to the same path, so whichever
    /// lands second silently replaces the other.
    #[error("--output and --result-json are both {path}")]
    OutputCollision {
        /// The path both arguments named.
        path: PathBuf,
    },
    /// The mix flags do not resolve against this project. Raised before
    /// anything is rendered.
    #[error(transparent)]
    MixSelection(#[from] MixSelectionError),
    /// A duration that cannot describe a window: negative, zero, or not a
    /// number.
    #[error("{what} must be a finite {bound} number of seconds, got {value}")]
    InvalidDuration {
        /// Which argument was wrong, e.g. `--seconds`.
        what: &'static str,
        /// How it was wrong, e.g. `positive`.
        bound: &'static str,
        /// The value that was rejected.
        value: f32,
    },
    /// A duration longer than one render may produce.
    ///
    /// For `--seconds` the renderer would clamp it and warn; the command
    /// refuses instead, so a caller never compares a truncated render against a
    /// full-length reference and reads the difference as a regression. For
    /// `--tail-seconds` nothing clamps it at all — the tail is added on top of
    /// the window and sized straight into a buffer allocation, so an
    /// unreasonable value aborts the process instead of returning an error.
    #[error("{what} {requested} exceeds the {maximum}-second maximum")]
    DurationTooLong {
        /// Which argument was too long.
        what: &'static str,
        /// The duration that was asked for.
        requested: f32,
        /// The longest that argument may be.
        maximum: f32,
    },
    /// A sample rate nothing can render at.
    #[error(
        "--sample-rate must be between {MIN_RENDER_SAMPLE_RATE} and {MAX_RENDER_SAMPLE_RATE} Hz, got {0}"
    )]
    InvalidSampleRate(u32),
    /// Individually legal durations and sample rate that multiply out to a
    /// buffer the renderer cannot allocate. See [`MAX_RENDER_BYTES`].
    #[error(
        "--seconds {seconds} plus --tail-seconds {tail} at --sample-rate {sample_rate} needs a \
         {bytes}-byte audio buffer, over the {maximum}-byte maximum — lower the sample rate or \
         shorten the window"
    )]
    RenderTooLarge {
        /// The requested window length.
        seconds: f32,
        /// The requested tail.
        tail: f32,
        /// The requested sample rate.
        sample_rate: u32,
        /// Bytes the render would have allocated.
        bytes: u64,
        /// The most it may allocate.
        maximum: u64,
    },
    /// A file could not be read for its content digest.
    #[error("cannot read {path} to digest it: {source}")]
    Digest {
        /// The file that could not be digested.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The receipt could not be serialized.
    #[error("cannot serialize the render receipt: {0}")]
    Receipt(#[source] serde_json::Error),
    /// The receipt could not be written.
    #[error("cannot write the render receipt to {path}: {source}")]
    WriteReceipt {
        /// The receipt path that was being written.
        path: PathBuf,
        /// The underlying write error.
        source: crate::io::atomic::AtomicWriteError,
    },
    /// The requested window covers no song ticks, so there is nothing to
    /// render. Usually a duration that rounds to zero ticks at the song tempo.
    #[error(
        "requested window resolves to an empty tick range \
         (start {start}, end {end}) — check the duration against the song tempo"
    )]
    EmptyWindow {
        /// First tick of the requested window (inclusive).
        start: u64,
        /// Last tick of the requested window (exclusive).
        end: u64,
    },
    /// The offline renderer refused the request or failed mid-render.
    #[error("render failed: {0}")]
    Render(String),
    /// The output file's parent directory does not exist and could not be
    /// created.
    #[error("cannot create directory {dir} for the render output: {source}")]
    CreateOutputDir {
        /// The directory that could not be created.
        dir: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// Encoding or writing the WAV failed.
    #[error("cannot write WAV to {path}: {source}")]
    WriteWav {
        /// The output file that was being written.
        path: PathBuf,
        /// The underlying encoder/I/O error.
        source: crate::audio::export::ExportError,
    },
}

impl From<synth_mcp::error::McpBridgeError> for RenderError {
    fn from(e: synth_mcp::error::McpBridgeError) -> Self {
        Self::Render(e.to_string())
    }
}

impl From<RenderError> for synth_mcp::error::McpBridgeError {
    fn from(e: RenderError) -> Self {
        Self::Other(e.to_string())
    }
}
