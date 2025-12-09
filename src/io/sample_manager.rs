//! Sample loading and caching for audio samples.
//!
//! The `SampleManager` lives in the GUI thread and handles loading
//! WAV files from disk. Loaded samples are cached and can be shared
//! with the audio engine via `Arc<Sample>`.
//!
//! # Thread Safety
//!
//! - Loading happens in the GUI thread (blocking I/O is OK)
//! - Samples are immutable and shared via `Arc<Sample>`
//! - The audio engine only receives `Arc<Sample>`, never file paths

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::types::{ChannelMode, MidiNote, Sample, SampleRate, SampleValue};

/// Errors that can occur when loading samples.
#[derive(Debug, Error)]
pub enum SampleError {
    /// File not found.
    #[error("Sample file not found: {0}")]
    NotFound(PathBuf),

    /// I/O error reading the file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Error decoding WAV file.
    #[error("WAV decode error: {0}")]
    WavDecode(#[from] hound::Error),

    /// Unsupported sample format.
    #[error("Unsupported sample format: {0}")]
    UnsupportedFormat(String),

    /// Unsupported channel count.
    #[error("Unsupported channel count: {0} (only mono and stereo supported)")]
    UnsupportedChannels(u16),
}

/// Result type for sample operations.
pub type SampleResult<T> = Result<T, SampleError>;

/// Manages loading and caching of audio samples.
///
/// Call `load()` to load a WAV file, which returns an `Arc<Sample>`.
/// The sample is cached by path, so subsequent loads return the same Arc.
#[derive(Default)]
pub struct SampleManager {
    /// Cache of loaded samples, keyed by absolute path.
    cache: HashMap<PathBuf, Arc<Sample>>,
}

impl SampleManager {
    /// Create a new sample manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a sample from disk, returning a cached version if available.
    ///
    /// # Arguments
    /// * `path` - Path to the WAV file
    ///
    /// # Returns
    /// An `Arc<Sample>` that can be safely sent to the audio engine.
    pub fn load(&mut self, path: impl AsRef<Path>) -> SampleResult<Arc<Sample>> {
        let path = path.as_ref();

        // Canonicalize path for consistent caching
        let canonical = path
            .canonicalize()
            .map_err(|_| SampleError::NotFound(path.to_path_buf()))?;

        // Return cached sample if available
        if let Some(sample) = self.cache.get(&canonical) {
            return Ok(Arc::clone(sample));
        }

        // Load and cache the sample
        let sample = Arc::new(load_wav(&canonical)?);
        self.cache.insert(canonical, Arc::clone(&sample));

        Ok(sample)
    }

    /// Check if a sample is already cached.
    #[must_use]
    pub fn is_cached(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        if let Ok(canonical) = path.canonicalize() {
            self.cache.contains_key(&canonical)
        } else {
            false
        }
    }

    /// Get the number of cached samples.
    #[must_use]
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    /// Clear the cache.
    ///
    /// Note: This doesn't invalidate existing `Arc<Sample>` references,
    /// but future loads will re-read from disk.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Remove a specific sample from the cache.
    pub fn remove(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if let Ok(canonical) = path.canonicalize() {
            self.cache.remove(&canonical);
        }
    }
}

/// Load a WAV file and convert to our internal sample format.
fn load_wav(path: &Path) -> SampleResult<Sample> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    // Validate channel count
    let channels = match spec.channels {
        1 => ChannelMode::Mono,
        2 => ChannelMode::Stereo,
        n => return Err(SampleError::UnsupportedChannels(n)),
    };

    // Extract file name for the sample name
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(String::from)
        .unwrap_or_else(|| "unknown".to_string());

    // Detect root note from filename
    let root_note = detect_root_note(&name);

    // Convert samples to f32 in range -1.0 to 1.0
    let data = convert_samples(reader, spec)?;

    Ok(Sample::new(
        name,
        data,
        channels,
        SampleRate::new(spec.sample_rate as f32),
    )
    .with_root_note(root_note))
}

/// Detect root note from filename.
///
/// Matches patterns like:
/// - "Piano_C3", "kick_A#2", "pad-Eb4"
/// - "C3_piano", "A#2-kick"
///
/// Returns C4 (MIDI 60) as default if no note is found.
fn detect_root_note(filename: &str) -> MidiNote {
    // Try to find note patterns in the filename
    let filename_upper = filename.to_uppercase();

    // Look for note names followed by optional accidental and octave
    // Patterns: C3, C#3, Cb3, etc.
    for word in filename_upper.split(|c: char| !c.is_alphanumeric()) {
        if let Some(note) = parse_note_name(word) {
            return note;
        }
    }

    // Also try to find embedded note patterns like "Piano_C3_loud"
    let chars: Vec<char> = filename_upper.chars().collect();
    for i in 0..chars.len() {
        // Look for pattern: note letter, optional accidental, digit
        if let Some(note) = try_parse_note_at(&chars, i) {
            return note;
        }
    }

    MidiNote::C4
}

/// Try to parse a note name like "C3", "A#4", "Eb2".
fn parse_note_name(s: &str) -> Option<MidiNote> {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return None;
    }

    // First char must be A-G
    let base = match chars[0] {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };

    let mut idx = 1;

    // Check for accidental
    let modifier = if idx < chars.len() {
        match chars[idx] {
            '#' => {
                idx += 1;
                1
            }
            'B' if idx + 1 < chars.len() && chars[idx + 1].is_ascii_digit() => {
                // 'B' followed by digit could be note B, not flat
                0
            }
            'B' => {
                idx += 1;
                -1
            }
            _ => 0,
        }
    } else {
        0
    };

    // Parse octave (can be negative or multiple digits)
    if idx >= chars.len() {
        return None;
    }

    let octave_str: String = chars[idx..].iter().collect();
    let octave: i32 = octave_str.parse().ok()?;

    // MIDI note: (octave + 1) * 12 + base + modifier
    // C4 = 60, so C-1 = 0
    let midi_note = (octave + 1) * 12 + base + modifier;

    if (0..=127).contains(&midi_note) {
        Some(MidiNote::new(midi_note as u8))
    } else {
        None
    }
}

/// Try to parse a note starting at position i in the character array.
fn try_parse_note_at(chars: &[char], i: usize) -> Option<MidiNote> {
    if i >= chars.len() {
        return None;
    }

    // Check for note letter
    let base = match chars[i] {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };

    let mut idx = i + 1;

    // Check for accidental
    let modifier = if idx < chars.len() {
        match chars[idx] {
            '#' => {
                idx += 1;
                1
            }
            'B' if idx + 1 < chars.len() && chars[idx + 1].is_ascii_digit() => 0,
            _ => 0,
        }
    } else {
        return None;
    };

    // Must have at least one digit for octave
    if idx >= chars.len() || !chars[idx].is_ascii_digit() {
        return None;
    }

    // Parse octave (handle negative octaves too)
    let mut octave_chars = Vec::new();
    while idx < chars.len() && (chars[idx].is_ascii_digit() || chars[idx] == '-') {
        octave_chars.push(chars[idx]);
        idx += 1;
    }

    let octave_str: String = octave_chars.into_iter().collect();
    let octave: i32 = octave_str.parse().ok()?;

    let midi_note = (octave + 1) * 12 + base + modifier;

    if (0..=127).contains(&midi_note) {
        Some(MidiNote::new(midi_note as u8))
    } else {
        None
    }
}

/// Convert WAV samples to our internal f32 format.
fn convert_samples(
    mut reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    spec: hound::WavSpec,
) -> SampleResult<Vec<SampleValue>> {
    match (spec.sample_format, spec.bits_per_sample) {
        // Integer formats
        (hound::SampleFormat::Int, 8) => {
            let samples: hound::Result<Vec<i8>> = reader.samples().collect();
            Ok(samples?
                .into_iter()
                .map(|s| SampleValue::new(f32::from(s) / 128.0))
                .collect())
        }
        (hound::SampleFormat::Int, 16) => {
            let samples: hound::Result<Vec<i16>> = reader.samples().collect();
            Ok(samples?
                .into_iter()
                .map(|s| SampleValue::new(f32::from(s) / 32768.0))
                .collect())
        }
        (hound::SampleFormat::Int, 24) => {
            let samples: hound::Result<Vec<i32>> = reader.samples().collect();
            Ok(samples?
                .into_iter()
                .map(|s| SampleValue::new(s as f32 / 8_388_608.0))
                .collect())
        }
        (hound::SampleFormat::Int, 32) => {
            let samples: hound::Result<Vec<i32>> = reader.samples().collect();
            Ok(samples?
                .into_iter()
                .map(|s| SampleValue::new(s as f32 / 2_147_483_648.0))
                .collect())
        }
        // Float format
        (hound::SampleFormat::Float, 32) => {
            let samples: hound::Result<Vec<f32>> = reader.samples().collect();
            Ok(samples?.into_iter().map(SampleValue::new).collect())
        }
        // Unsupported
        (format, bits) => Err(SampleError::UnsupportedFormat(format!(
            "{format:?} {bits}-bit"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    /// Create a test WAV file with a simple sine wave.
    fn create_test_wav() -> NamedTempFile {
        let temp = NamedTempFile::new().unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(temp.path(), spec).unwrap();
        for i in 0..1000 {
            let sample = (i as f32 * 0.1).sin();
            let amplitude = (sample * 32767.0) as i16;
            writer.write_sample(amplitude).unwrap();
        }
        writer.finalize().unwrap();

        temp
    }

    #[test]
    fn test_load_sample() {
        let temp = create_test_wav();
        let mut manager = SampleManager::new();

        let sample = manager.load(temp.path()).unwrap();

        assert_eq!(
            sample.name.as_str(),
            temp.path().file_stem().unwrap().to_str().unwrap()
        );
        assert_eq!(sample.channels, ChannelMode::Mono);
        assert!(!sample.is_empty());
    }

    #[test]
    fn test_cache() {
        let temp = create_test_wav();
        let mut manager = SampleManager::new();

        let sample1 = manager.load(temp.path()).unwrap();
        let sample2 = manager.load(temp.path()).unwrap();

        // Should return the same Arc
        assert!(Arc::ptr_eq(&sample1, &sample2));
        assert_eq!(manager.cached_count(), 1);
    }

    #[test]
    fn test_not_found() {
        let mut manager = SampleManager::new();
        let result = manager.load("/nonexistent/path.wav");

        assert!(matches!(result, Err(SampleError::NotFound(_))));
    }

    #[test]
    fn test_clear_cache() {
        let temp = create_test_wav();
        let mut manager = SampleManager::new();

        manager.load(temp.path()).unwrap();
        assert_eq!(manager.cached_count(), 1);

        manager.clear_cache();
        assert_eq!(manager.cached_count(), 0);
    }

    #[test]
    fn test_detect_root_note_basic() {
        // Standard patterns
        assert_eq!(detect_root_note("Piano_C3"), MidiNote::new(48));
        assert_eq!(detect_root_note("kick_C4"), MidiNote::C4);
        assert_eq!(detect_root_note("A4_string"), MidiNote::new(69));

        // With sharp
        assert_eq!(detect_root_note("Piano_C#3"), MidiNote::new(49));
        assert_eq!(detect_root_note("F#4_pad"), MidiNote::new(66));

        // Default to C4 when no note found
        assert_eq!(detect_root_note("kick_drum"), MidiNote::C4);
        assert_eq!(detect_root_note("ambient_sound"), MidiNote::C4);
    }

    #[test]
    fn test_detect_root_note_edge_cases() {
        // Various octaves
        assert_eq!(detect_root_note("bass_C1"), MidiNote::new(24));
        assert_eq!(detect_root_note("high_C7"), MidiNote::new(96));

        // Note at start
        assert_eq!(detect_root_note("C4_piano"), MidiNote::C4);
        assert_eq!(detect_root_note("G#2_bass"), MidiNote::new(44));
    }
}
