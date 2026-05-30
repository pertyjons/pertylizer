//! Project file format for saving and loading complete synth projects.
//!
//! A project contains all instruments (each with their full patch/module graph),
//! the sequencer song, and global state. This is the top-level persistence format.

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::patch::{Author, AwePresetFile, InstrumentState, ModuleState, Patch, PatchError};
use synth_core::{BipolarValue, Gain, Seconds, Semitones};
use synth_engine::instrument::InstrumentId;
use synth_sequencer::Song;

/// Top-level project file container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectFile {
    /// Always `"project"` — distinguishes from patch files.
    pub file_type: String,
    /// Format version (currently `"1.0"`).
    pub version: String,
    /// All instruments with their patches.
    pub instruments: Vec<InstrumentState>,
    /// Which instrument was active (focused) when saved.
    pub active_instrument_id: u64,
    /// Author / composer of this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Complete sequencer song (patterns, tracks, arrangement).
    pub song: Song,
    /// Global settings not tied to any instrument.
    #[serde(default)]
    pub global: GlobalProjectState,
}

/// Global project state that isn't per-instrument.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GlobalProjectState {
    /// Master volume (0.0–1.0).
    #[serde(default = "default_master_volume")]
    #[schemars(with = "f32")]
    pub master_volume: Gain,
    /// Keyboard octave offset.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds.
    #[serde(default)]
    #[schemars(with = "f32")]
    pub glide_time: Seconds,
    /// AWE (Acoustic World Engine) state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe: Option<synth_awe::AweState>,
    /// Full AWE preset with metadata, embedded in project when enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe_preset: Option<AwePresetFile>,
    /// Free-text description of the current AWE state (room's acoustic
    /// character). Mirrors `AwePresetFile.description` when a preset is
    /// loaded but persists separately so MCP-written descriptions on
    /// unsaved rooms survive a project save / load round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe_description: Option<String>,
    /// Effect chains on return busses (the busses themselves — id/name/fader —
    /// live in the `Song`; only the engine-side effect chain is captured here).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub return_bus_effects: Vec<ReturnBusEffectsState>,
    /// Master-bus effect chain (the final chain applied to the full mix), in
    /// processing order. Engine-side runtime state, captured here for save/load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub master_effects: Vec<ModuleState>,
}

/// The effect chain on one return bus, in processing order, for persistence.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReturnBusEffectsState {
    /// Return bus id (matches a `ReturnBus` in the song).
    pub id: u16,
    /// Effects in chain order (each a module type + parameter values).
    pub effects: Vec<ModuleState>,
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
            awe_preset: None,
            awe_description: None,
            return_bus_effects: Vec::new(),
            master_effects: Vec::new(),
        }
    }
}

impl ProjectFile {
    /// The `.pertyproj` on-disk format version. Bumped **only** when the project
    /// file format itself changes — unlike the app's `CARGO_PKG_VERSION`, which
    /// moves every release. This is the value external tooling should pin to
    /// detect a format change (see `get_project_schema`).
    pub const FORMAT_VERSION: &'static str = "1.0";

    /// Create a new project file with the given instruments and song.
    pub fn new(
        instruments: Vec<InstrumentState>,
        active_instrument_id: u64,
        author: Option<Author>,
        song: Song,
        global: GlobalProjectState,
    ) -> Self {
        Self {
            file_type: "project".to_string(),
            version: Self::FORMAT_VERSION.to_string(),
            instruments,
            active_instrument_id,
            author,
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

/// Result of auto-detecting and loading a file.
pub enum LoadedFile {
    /// A single-instrument patch file.
    Patch(Patch),
    /// A full project file with multiple instruments and song.
    Project(Box<ProjectFile>),
    /// An AWE preset file.
    AwePreset(AwePresetFile),
    /// A ZIP bundle (contains project + samples). Path stored for deferred loading.
    Bundle(PathBuf),
}

/// The file extension a project should be saved with given whether the
/// sample library is non-empty: `"zip"` for bundles (sample-embedded),
/// `"json"` otherwise. Lets the filename on disk tell the truth about
/// the format inside.
#[must_use]
pub fn project_extension(has_samples: bool) -> &'static str {
    if has_samples { "zip" } else { "json" }
}

/// Return `path` with its extension forced to match the save format
/// for `has_samples`. Idempotent.
#[must_use]
pub fn normalize_project_path(path: &Path, has_samples: bool) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension(project_extension(has_samples));
    p
}

/// Read a file, auto-detect whether it's a ZIP bundle, patch, project, or AWE preset,
/// and parse it.
///
/// Detects ZIP bundles via magic bytes (`PK\x03\x04`), then falls back to JSON parsing
/// with `"file_type"` discriminator. Falls back to patch format.
pub fn load_file(path: impl AsRef<Path>) -> Result<LoadedFile, PatchError> {
    // Check for ZIP bundle format first
    if crate::bundle::is_zip_file(path.as_ref()) {
        return Ok(LoadedFile::Bundle(path.as_ref().to_path_buf()));
    }

    let content = fs::read_to_string(path.as_ref()).map_err(|e| PatchError::Io(e.to_string()))?;

    // Check for discriminator field
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
        match value.get("file_type").and_then(|v| v.as_str()) {
            Some("project") => {
                let project =
                    serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
                return Ok(LoadedFile::Project(Box::new(project)));
            }
            Some("awe_preset") => {
                let preset =
                    serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
                return Ok(LoadedFile::AwePreset(preset));
            }
            _ => {}
        }
    }

    let patch = serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
    Ok(LoadedFile::Patch(patch))
}

/// Get the default projects directory based on the platform.
///
/// - Linux: `~/.local/share/pertylizer/projects`
/// - macOS: `~/Library/Application Support/pertylizer/projects`
/// - Windows: `%APPDATA%\pertylizer\projects`
pub(crate) fn default_projects_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not determine home directory".to_string())?;
    Ok(base.join("pertylizer").join("projects"))
}

/// Get the default AWE presets directory based on the platform.
///
/// - Linux: `~/.local/share/pertylizer/awe_presets`
/// - macOS: `~/Library/Application Support/pertylizer/awe_presets`
/// - Windows: `%APPDATA%\pertylizer\awe_presets`
pub(crate) fn default_awe_presets_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not determine home directory".to_string())?;
    Ok(base.join("pertylizer").join("awe_presets"))
}

/// Create a default instrument state for instrument 0 with an empty patch.
///
/// Used when creating a new project to have a single instrument ready to use.
#[must_use]
pub fn default_instrument_state() -> InstrumentState {
    InstrumentState {
        id: InstrumentId::FIRST,
        name: "Init".to_string(),
        channel: 1,
        volume: Gain::UNITY,
        pan: BipolarValue::CENTER,
        muted: false,
        solo: false,
        key_range: (0, 127),
        transpose: Semitones::ZERO,
        oversampling: 1,
        category: 0,
        description: String::new(),
        color: None,
        allocation_mode: synth_engine::voice_allocator::AllocationMode::default(),
        stealing_strategy: synth_engine::voice_allocator::StealingStrategy::default(),
        max_voices: synth_core::VoiceCount::OCTO,
        velocity_amp_sensitivity: synth_core::NormalizedValue::MAX,
        velocity_filter_sensitivity: synth_core::NormalizedValue::MIN,
        sidechain_source_id: None,
        patch: Patch::new("Init"),
    }
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
