//! Project file format for saving and loading complete synth projects.
//!
//! A project contains all instruments (each with their full patch/module graph),
//! the sequencer song, and global state. This is the top-level persistence format.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::patch::{InstrumentState, Patch, PatchError};
use synth_core::{Gain, Seconds};
use synth_sequencer::Song;

/// Top-level project file container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Always `"project"` — distinguishes from patch files.
    pub file_type: String,
    /// Format version (currently `"1.0"`).
    pub version: String,
    /// All instruments with their patches.
    pub instruments: Vec<InstrumentState>,
    /// Which instrument was active (focused) when saved.
    pub active_instrument_id: u64,
    /// Complete sequencer song (patterns, tracks, arrangement).
    pub song: Song,
    /// Global settings not tied to any instrument.
    #[serde(default)]
    pub global: GlobalProjectState,
}

/// Global project state that isn't per-instrument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalProjectState {
    /// Master volume (0.0–1.0).
    #[serde(default = "default_master_volume")]
    pub master_volume: Gain,
    /// Keyboard octave offset.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds.
    #[serde(default)]
    pub glide_time: Seconds,
    /// AWE (Acoustic World Engine) state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe: Option<synth_awe::AweState>,
}

fn default_master_volume() -> Gain {
    Gain::new(0.8)
}

impl Default for GlobalProjectState {
    fn default() -> Self {
        Self {
            master_volume: Gain::new(0.8),
            octave_offset: 0,
            glide_time: Seconds::ZERO,
            awe: None,
        }
    }
}

impl ProjectFile {
    /// Create a new project file with the given instruments and song.
    pub fn new(
        instruments: Vec<InstrumentState>,
        active_instrument_id: u64,
        song: Song,
        global: GlobalProjectState,
    ) -> Self {
        Self {
            file_type: "project".to_string(),
            version: "1.0".to_string(),
            instruments,
            active_instrument_id,
            song,
            global,
        }
    }

    /// Load a project from a JSON file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| PatchError::Io(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save the project to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content =
            serde_json::to_string_pretty(self).map_err(|e| PatchError::Serialize(e.to_string()))?;
        fs::write(path.as_ref(), content).map_err(|e| PatchError::Io(e.to_string()))
    }
}

/// Result of auto-detecting and loading a JSON file.
pub enum LoadedFile {
    /// A single-instrument patch file.
    Patch(Patch),
    /// A full project file with multiple instruments and song.
    Project(ProjectFile),
}

/// Read a JSON file once, auto-detect whether it's a patch or project, and parse it.
///
/// Looks for `"file_type": "project"` to distinguish project files from patches.
/// This avoids the double read/parse that would occur from calling
/// `detect_file_type` followed by `load`.
pub fn load_file(path: impl AsRef<Path>) -> Result<LoadedFile, PatchError> {
    let content = fs::read_to_string(path.as_ref()).map_err(|e| PatchError::Io(e.to_string()))?;

    // Check for the project discriminator field
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
        && value.get("file_type").and_then(|v| v.as_str()) == Some("project")
    {
        let project =
            serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
        return Ok(LoadedFile::Project(project));
    }

    let patch = serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
    Ok(LoadedFile::Patch(patch))
}

/// Get the default projects directory based on the platform.
///
/// - Linux: `~/.local/share/modular-synth/projects`
/// - macOS: `~/Library/Application Support/modular-synth/projects`
/// - Windows: `%APPDATA%\modular-synth\projects`
pub(crate) fn default_projects_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not determine home directory".to_string())?;
    Ok(base.join("modular-synth").join("projects"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_project_state_default() {
        let state = GlobalProjectState::default();
        assert!((state.master_volume.as_f32() - 0.8).abs() < f32::EPSILON);
        assert_eq!(state.octave_offset, 0);
        assert!((state.glide_time.as_f32()).abs() < f32::EPSILON);
        assert!(state.awe.is_none());
    }
}
