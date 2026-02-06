//! Song import system for tracker files (MOD, XM, S3M, IT).
//!
//! This module provides a trait-based architecture for importing
//! various music file formats into the internal `Song` representation.
//!
//! ## Supported Formats
//!
//! - MOD (ProTracker)
//! - XM (FastTracker II)
//! - S3M (Scream Tracker 3)
//! - IT (Impulse Tracker)
//!
//! ## Usage
//!
//! ```ignore
//! use modular_synth::io::import::{SongImporter, TrackerImporter};
//!
//! let importer = TrackerImporter::new();
//! let song = importer.import(Path::new("song.xm"))?;
//! ```

mod tracker;

use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

pub use tracker::TrackerImporter;

use synth_core::{BipolarValue, Gain, NormalizedValue, Sample, Seconds, VoiceCount};
use synth_sequencer::Song;

/// Errors that can occur during song import.
#[derive(Debug, Error)]
pub enum ImportError {
    /// File not found.
    #[error("File not found: {0}")]
    NotFound(String),

    /// I/O error reading the file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Unsupported file format.
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Error parsing the file.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Invalid data in the file.
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Result type for import operations.
pub type ImportResult<T> = Result<T, ImportError>;

/// ADSR envelope parameters from import.
///
/// Maps directly to our Envelope module parameters.
#[derive(Debug, Clone)]
pub struct ImportedAdsr {
    /// Whether the envelope is enabled.
    pub enabled: bool,
    /// Attack time.
    pub attack: Seconds,
    /// Decay time.
    pub decay: Seconds,
    /// Sustain level.
    pub sustain: NormalizedValue,
    /// Release time.
    pub release: Seconds,
}

impl Default for ImportedAdsr {
    fn default() -> Self {
        Self {
            enabled: false,
            attack: Seconds::new(0.01),
            decay: Seconds::new(0.1),
            sustain: NormalizedValue::new(0.7),
            release: Seconds::new(0.3),
        }
    }
}

/// Instrument metadata from import.
#[derive(Debug, Clone)]
pub struct ImportedInstrument {
    /// Instrument name.
    pub name: String,
    /// Index into the samples vector (first sample for this instrument).
    pub sample_index: Option<usize>,
    /// Keymap: maps MIDI note (0-119) to sample index within this instrument's samples.
    /// If None, the instrument uses a single sample for all notes.
    /// If Some, the vec has 120 entries (one per MIDI note 0-119).
    pub sample_keymap: Option<Vec<usize>>,
    /// Number of samples in this instrument (for keymap offset calculation).
    pub sample_count: usize,
    /// Volume envelope (converted to ADSR for legacy compatibility).
    pub volume_envelope: ImportedAdsr,
    /// Raw envelope points for MultiPointEnvelope (frame, value).
    /// Points are in tracker ticks (not seconds).
    pub envelope_points: Vec<(u16, f32)>,
    /// Sustain point index (if envelope has sustain).
    pub envelope_sustain: Option<u8>,
    /// Loop region (start_index, end_index) if envelope loops.
    pub envelope_loop: Option<(u8, u8)>,
    /// Raw panning envelope points for MultiPointEnvelope (frame, value).
    /// Points are in tracker ticks (not seconds). Values 0.0-1.0 where 0.5 = center.
    pub panning_envelope_points: Vec<(u16, f32)>,
    /// Sustain point index for panning envelope.
    pub panning_envelope_sustain: Option<u8>,
    /// Loop region (start_index, end_index) for panning envelope.
    pub panning_envelope_loop: Option<(u8, u8)>,
    /// Fadeout rate (0.0 = no fadeout, 1.0 = instant).
    pub fadeout: f32,
    /// Global instrument volume.
    pub global_volume: Gain,
    /// Default pan position.
    pub default_pan: BipolarValue,
}

impl ImportedInstrument {
    /// Get the sample index for a given MIDI note.
    ///
    /// If the instrument has a keymap, this returns the appropriate sample
    /// based on the note pitch. Otherwise, returns the default sample_index.
    ///
    /// # Arguments
    /// * `note` - MIDI note number (0-127)
    /// * `first_sample_offset` - The offset into the global samples array
    ///   for this instrument's first sample
    ///
    /// # Returns
    /// The global sample index, or None if no sample is available for this note.
    #[must_use]
    pub fn sample_index_for_note(&self, note: u8) -> Option<usize> {
        let first_sample = self.sample_index?;

        if let Some(keymap) = &self.sample_keymap {
            // Use keymap to find sample index within this instrument
            let note_idx = (note as usize).min(119);
            let local_sample_idx = keymap.get(note_idx).copied().unwrap_or(0);
            // Convert to global index
            Some(first_sample + local_sample_idx.min(self.sample_count.saturating_sub(1)))
        } else {
            // No keymap - use first sample for all notes
            Some(first_sample)
        }
    }
}

/// Result of a successful import.
///
/// Contains the song, samples, and instrument metadata.
#[derive(Debug)]
pub struct ImportedSong {
    /// The imported song with patterns and arrangement.
    pub song: Song,
    /// Extracted samples (indexed by sample number).
    pub samples: Vec<Arc<Sample>>,
    /// Instrument metadata with envelope information.
    pub instruments: Vec<ImportedInstrument>,
    /// Minimum voice count required for playback (e.g., tracker channel count).
    /// The engine should resize its voice pool to at least this many voices.
    pub min_voices: Option<VoiceCount>,
}

/// Trait for song importers.
///
/// Implement this trait to add support for new file formats.
pub trait SongImporter {
    /// Get the name of this importer.
    fn name(&self) -> &str;

    /// Get the file extensions this importer supports.
    fn extensions(&self) -> &[&str];

    /// Check if this importer can handle the given file extension.
    fn can_import(&self, path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match ext {
            Some(ext) => self.extensions().iter().any(|&e| e == ext),
            None => false,
        }
    }

    /// Import a song from the given path.
    fn import(&self, path: &Path) -> ImportResult<ImportedSong>;
}

/// Get the appropriate importer for a file.
///
/// Returns `None` if no importer supports the file's extension.
#[must_use]
pub fn get_importer_for(path: &Path) -> Option<Box<dyn SongImporter>> {
    let importers: Vec<Box<dyn SongImporter>> = vec![Box::new(TrackerImporter::new())];

    importers.into_iter().find(|imp| imp.can_import(path))
}

/// Import a song from the given path.
///
/// Automatically selects the appropriate importer based on file extension.
pub fn import_song(path: &Path) -> ImportResult<ImportedSong> {
    let importer = get_importer_for(path).ok_or_else(|| {
        ImportError::UnsupportedFormat(format!(
            "No importer for extension: {}",
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
        ))
    })?;

    importer.import(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_importer_extensions() {
        let importer = TrackerImporter::new();
        assert!(importer.can_import(Path::new("song.mod")));
        assert!(importer.can_import(Path::new("song.MOD")));
        assert!(importer.can_import(Path::new("song.xm")));
        assert!(importer.can_import(Path::new("song.s3m")));
        // IT format not supported by xmrs import feature
        assert!(!importer.can_import(Path::new("song.it")));
        assert!(!importer.can_import(Path::new("song.wav")));
        assert!(!importer.can_import(Path::new("song.mp3")));
    }
}
