//! Project file format for saving and loading complete synth projects.
//!
//! A project contains all instruments (each with their full patch/module graph),
//! the sequencer song, and global state. This is the top-level persistence format.

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::patch::{Author, InstrumentState, ModuleState, Patch, PatchError};
use synth_core::{BipolarValue, Gain, Seconds, Semitones};
use synth_engine::instrument::InstrumentId;
use synth_sequencer::Song;

/// Top-level project file container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectFile {
    /// Always `"project"` — distinguishes from patch files.
    pub file_type: String,
    /// Format version (currently `"1.1"`).
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
            return_bus_effects: Vec::new(),
            master_effects: Vec::new(),
        }
    }
}

impl ProjectFile {
    /// The `.ptz` on-disk format version. Bumped **only** when the project
    /// file format itself changes — unlike the app's `CARGO_PKG_VERSION`, which
    /// moves every release. This is the value external tooling should pin to
    /// detect a format change (see `get_project_schema`).
    pub const FORMAT_VERSION: &'static str = "1.1";

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
    Patch(Box<Patch>),
    /// A full project file with multiple instruments and song.
    Project(Box<ProjectFile>),
    /// A ZIP bundle (contains project + samples). Path stored for deferred loading.
    Bundle(PathBuf),
}

/// The file extension a project should be saved with given whether the
/// sample library is non-empty: `"zip"` for bundles (sample-embedded),
/// `"ptz"` otherwise. Lets the filename on disk tell the truth about
/// the format inside.
#[must_use]
pub fn project_extension(has_samples: bool) -> &'static str {
    if has_samples { "zip" } else { "ptz" }
}

/// Extensions accepted as-is for a plain-JSON (sample-free) project. `.ptz` is
/// the Pertylizer project extension; `.json` is kept for backward
/// compatibility. Both hold identical content — the loader auto-detects the
/// format by magic bytes / `file_type`, not by extension.
const PLAIN_PROJECT_EXTENSIONS: &[&str] = &["ptz", "json"];

/// Return `path` with an extension appropriate for the save format.
///
/// - **Bundle** (`has_samples`): forced to `.zip`, because a sample-embedded
///   project is a ZIP archive and the filename must not claim otherwise.
/// - **Plain project** (no samples): a caller-supplied `.ptz` or `.json` is
///   **preserved** rather than silently rewritten; any other (or missing)
///   extension is normalized to the default `.ptz`.
///
/// Idempotent.
#[must_use]
pub fn normalize_project_path(path: &Path, has_samples: bool) -> PathBuf {
    if has_samples {
        let mut p = path.to_path_buf();
        p.set_extension("zip");
        return p;
    }
    let keep = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            PLAIN_PROJECT_EXTENSIONS
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext))
        });
    if keep {
        path.to_path_buf()
    } else {
        let mut p = path.to_path_buf();
        p.set_extension(project_extension(false));
        p
    }
}

/// Read a file, auto-detect whether it's a ZIP bundle, patch, or project,
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
        warn_if_legacy_awe(&value, path.as_ref());
        if value.get("file_type").and_then(|v| v.as_str()) == Some("project") {
            let project =
                serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
            return Ok(LoadedFile::Project(Box::new(project)));
        }
    }

    let patch = serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?;
    Ok(LoadedFile::Patch(Box::new(patch)))
}

/// True if a parsed save file still carries state from the removed Acoustic
/// World Engine — a non-null `global.awe` block (projects) or `settings.awe`
/// block (patches). The field no longer deserializes, so the key is silently
/// dropped on load; this detects it from the raw JSON.
#[must_use]
pub(crate) fn value_has_legacy_awe(value: &serde_json::Value) -> bool {
    ["global", "settings"].iter().any(|parent| {
        value
            .get(parent)
            .and_then(|p| p.get("awe"))
            .is_some_and(|awe| !awe.is_null())
    })
}

/// Warn when a saved file still carries removed-AWE state. The key is silently
/// ignored on load; this tells the user why the room is gone and how to rebuild
/// it, rather than letting the sound quietly change with no explanation.
pub(crate) fn warn_if_legacy_awe(value: &serde_json::Value, path: &Path) {
    if value_has_legacy_awe(value) {
        tracing::warn!(
            target: "pertylizer::project",
            "Loaded '{}' — this file was saved with the removed Acoustic World Engine \
             (AWE); its room/spatial state is ignored. Rebuild spatial audio with the \
             Spatial Panner module plus the Reverb / Convolution / Modal Resonator \
             effects.",
            path.display()
        );
    }
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
        unison_detune: synth_core::Cents::new(10.0),
        unison_spread: synth_core::NormalizedValue::MIN,
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
    }

    #[test]
    fn project_extension_picks_zip_for_samples_ptz_otherwise() {
        assert_eq!(project_extension(true), "zip");
        assert_eq!(project_extension(false), "ptz");
    }

    #[test]
    fn load_file_accepts_legacy_awe_project_gracefully() {
        // A real example project with an injected legacy AWE block: it must still
        // load (the unknown `awe` key is ignored) and be flagged by the detector
        // that drives the load-time warning.
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/examples/projects/all-modules-reference.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&example).expect("read example"))
                .expect("parse example");
        value["global"]["awe"] = serde_json::json!({ "enabled": true });
        assert!(
            value_has_legacy_awe(&value),
            "injected awe must be detected"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("legacy_awe.json");
        fs::write(&path, serde_json::to_string(&value).expect("serialize")).expect("write temp");

        // Runs the warn path and drops the unknown `awe` key without error.
        match load_file(&path).expect("load legacy-awe project") {
            LoadedFile::Project(_) => {}
            _ => panic!("expected a project"),
        }
    }

    #[test]
    fn detects_legacy_awe_state_in_saved_files() {
        // Project: awe under `global`.
        let proj = serde_json::json!({ "global": { "awe": { "enabled": true } } });
        assert!(value_has_legacy_awe(&proj));
        // Patch: awe under `settings`.
        let patch = serde_json::json!({ "settings": { "awe": { "enabled": false } } });
        assert!(value_has_legacy_awe(&patch));
        // Absent or null awe → no warning.
        assert!(!value_has_legacy_awe(&serde_json::json!({ "global": {} })));
        assert!(!value_has_legacy_awe(
            &serde_json::json!({ "global": { "awe": null } })
        ));
        assert!(!value_has_legacy_awe(
            &serde_json::json!({ "master_volume": 0.8 })
        ));
    }

    #[test]
    fn normalize_project_path_forces_zip_for_bundles() {
        // Samples present → always `.zip`, whatever the user typed (a bundle is
        // a ZIP archive; the filename must not claim otherwise).
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.json"), true),
            PathBuf::from("/songs/track.zip")
        );
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.ptz"), true),
            PathBuf::from("/songs/track.zip")
        );
        assert_eq!(
            normalize_project_path(Path::new("/songs/track"), true),
            PathBuf::from("/songs/track.zip")
        );
    }

    #[test]
    fn normalize_project_path_preserves_recognized_plain_extensions() {
        // A caller-supplied `.ptz` is kept, not silently rewritten to `.json` —
        // this was the reported MCP surprise.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.ptz"), false),
            PathBuf::from("/songs/track.ptz")
        );
        // `.json` is still accepted for backward compatibility.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.json"), false),
            PathBuf::from("/songs/track.json")
        );
        // Case-insensitive match on the extension.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.PTZ"), false),
            PathBuf::from("/songs/track.PTZ")
        );
    }

    #[test]
    fn normalize_project_path_defaults_unknown_plain_extensions_to_ptz() {
        // An unrecognized (or missing) extension normalizes to the default.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.zip"), false),
            PathBuf::from("/songs/track.ptz")
        );
        assert_eq!(
            normalize_project_path(Path::new("/songs/track"), false),
            PathBuf::from("/songs/track.ptz")
        );
    }

    #[test]
    fn normalize_project_path_is_idempotent() {
        for has_samples in [true, false] {
            let once = normalize_project_path(Path::new("/songs/track.bin"), has_samples);
            let twice = normalize_project_path(&once, has_samples);
            assert_eq!(once, twice);
        }
    }
}
