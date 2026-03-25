//! Error types for sample operations.

use std::path::PathBuf;

use thiserror::Error;

/// Errors that can occur during sample operations.
#[derive(Debug, Error)]
pub enum SampleError {
    /// WAV file I/O error.
    #[error("WAV I/O error: {0}")]
    Hound(#[from] hound::Error),

    /// File not found.
    #[error("file not found: {path}")]
    FileNotFound { path: PathBuf },

    /// Unsupported format.
    #[error("unsupported WAV format: {reason}")]
    UnsupportedFormat { reason: String },

    /// Invalid sample rate.
    #[error("invalid sample rate: {0}")]
    InvalidSampleRate(u32),

    /// General I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
