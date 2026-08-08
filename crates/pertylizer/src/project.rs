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

    /// Save the project to a JSON file, replacing any existing file atomically.
    ///
    /// Serialization runs first so a project that cannot be encoded fails
    /// before the destination is touched, and the write goes through a
    /// temp-then-rename so a disk error cannot truncate the last good save.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content =
            serde_json::to_string_pretty(self).map_err(|e| PatchError::Serialize(e.to_string()))?;
        crate::io::atomic::write(path.as_ref(), content.as_bytes()).map_err(PatchError::from)
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
/// sample library is non-empty: [`BUNDLE_EXTENSION`] for bundles
/// (sample-embedded), `"ptz"` otherwise. Lets the filename on disk tell the
/// truth about the format inside.
#[must_use]
pub fn project_extension(has_samples: bool) -> &'static str {
    if has_samples { BUNDLE_EXTENSION } else { "ptz" }
}

/// The extension of a sample-embedded project: a ZIP archive whose payload is
/// a `.ptz` project, so the name states both halves.
///
/// [`Path::extension`] only ever sees the `zip` half — `song.ptz.zip` has stem
/// `song.ptz` — which is why [`normalize_project_path`] strips this by name
/// before re-applying it. Without that, normalizing an already-normalized
/// bundle path would append a second `.ptz.zip`.
pub const BUNDLE_EXTENSION: &str = "ptz.zip";

/// Extensions accepted as-is for a plain-JSON (sample-free) project. `.ptz` is
/// the Pertylizer project extension; `.json` is kept for backward
/// compatibility. Both hold identical content — the loader auto-detects the
/// format by magic bytes / `file_type`, not by extension.
const PLAIN_PROJECT_EXTENSIONS: &[&str] = &["ptz", "json"];

/// Return `path` with an extension appropriate for the save format.
///
/// - **Bundle** (`has_samples`): forced to `.ptz.zip`, because a
///   sample-embedded project is a ZIP archive around a `.ptz` project and the
///   filename must not claim otherwise.
/// - **Plain project** (no samples): a caller-supplied `.ptz` or `.json` is
///   **preserved** rather than silently rewritten; any other (or missing)
///   extension is normalized to the default `.ptz`.
///
/// Idempotent, in both directions: a bundle path that loses its samples
/// normalizes to `.ptz` rather than growing a `.ptz.ptz`.
#[must_use]
pub fn normalize_project_path(path: &Path, has_samples: bool) -> PathBuf {
    if has_samples {
        return set_project_extension(path, BUNDLE_EXTENSION);
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
        set_project_extension(path, project_extension(false))
    }
}

/// `path` with its extension replaced by `extension`, treating a trailing
/// [`BUNDLE_EXTENSION`] as the single extension it means to be.
///
/// [`Path::set_extension`] replaces everything after the *last* dot, so on
/// `song.ptz.zip` it would leave the `.ptz` behind as part of the stem. The
/// two-dot suffix is therefore stripped by name first; every other case is a
/// single extension `set_extension` handles correctly.
fn set_project_extension(path: &Path, extension: &str) -> PathBuf {
    let stripped = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(strip_bundle_extension)
        .map(|base| path.with_file_name(base));
    let mut out = stripped.unwrap_or_else(|| path.to_path_buf());
    out.set_extension(extension);
    out
}

/// `name` without a trailing `.ptz.zip`, or `None` if it has none.
///
/// Case-insensitive, matching the [`PLAIN_PROJECT_EXTENSIONS`] comparison. A
/// name that is *only* the suffix yields `None` rather than an empty stem,
/// which would make the caller rewrite the parent directory instead of a file.
fn strip_bundle_extension(name: &str) -> Option<&str> {
    let split = name.len().checked_sub(BUNDLE_EXTENSION.len() + 1)?;
    let (base, tail) = name.split_at_checked(split)?;
    let suffix = tail.strip_prefix('.')?;
    (!base.is_empty() && suffix.eq_ignore_ascii_case(BUNDLE_EXTENSION)).then_some(base)
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
#[cfg(feature = "gui-egui")]
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
    fn project_extension_picks_ptz_zip_for_samples_ptz_otherwise() {
        assert_eq!(project_extension(true), "ptz.zip");
        assert_eq!(project_extension(false), "ptz");
    }

    /// A minimal but real project for the save-path tests below.
    fn sample_project(name: &str) -> ProjectFile {
        ProjectFile::new(
            Vec::new(),
            0,
            None,
            Song::new(name),
            GlobalProjectState::default(),
        )
    }

    /// First save of a `.ptz` project: the file appears and reloads.
    #[test]
    fn saves_a_new_ptz_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fresh.ptz");

        sample_project("fresh").save(&path).expect("save");

        let loaded = ProjectFile::load(&path).expect("load");
        assert_eq!(loaded.song.name, "fresh");
    }

    /// Overwriting must fully replace the previous project, not leave a tail of
    /// the older, longer file behind (the classic truncating-write corruption).
    #[test]
    fn overwriting_a_ptz_project_replaces_it_completely() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.ptz");
        sample_project("a project with a considerably longer name")
            .save(&path)
            .expect("first save");

        sample_project("short").save(&path).expect("second save");

        let loaded = ProjectFile::load(&path).expect("load");
        assert_eq!(loaded.song.name, "short");
    }

    /// The data-safety guarantee: a save that fails must leave the user's last
    /// good project exactly as it was, rather than truncating it first.
    ///
    /// Unix-only *mechanism*, not a Unix-only guarantee: Windows largely
    /// ignores the readonly attribute on a directory, so the temp file is
    /// created anyway and the save never fails. The portable halves are
    /// [`crate::io::atomic`]'s payload-failure tests and the
    /// destination-is-a-directory case below.
    #[cfg(unix)]
    #[test]
    fn a_failed_save_preserves_the_previous_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.ptz");
        sample_project("last good save").save(&path).expect("seed");
        let before = fs::read(&path).expect("read seed");

        // Make the directory read-only so creating the temp file fails — the
        // same class of failure as a full disk or a revoked permission.
        let mut perms = fs::metadata(dir.path()).expect("metadata").permissions();
        perms.set_readonly(true);
        fs::set_permissions(dir.path(), perms).expect("set dir read-only");

        let result = sample_project("would-be corruption").save(&path);

        // Restore write access before any assertion so the tempdir can clean up
        // even when an assertion below fails.
        let mut perms = fs::metadata(dir.path()).expect("metadata").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(dir.path(), perms).expect("restore dir perms");

        assert!(result.is_err(), "save into a read-only directory must fail");
        assert_eq!(
            fs::read(&path).expect("read after failed save"),
            before,
            "the previous project must survive a failed save",
        );
        assert_eq!(
            ProjectFile::load(&path).expect("reload").song.name,
            "last good save",
        );
    }

    /// The same guarantee at the replace step, on every platform: a path that
    /// is a directory cannot be renamed over anywhere, so the save must fail
    /// and leave that directory exactly as it was — not empty it, and not
    /// abandon the scratch file next to it.
    #[test]
    fn a_save_that_cannot_replace_the_destination_leaves_it_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.ptz");
        fs::create_dir(&path).expect("destination directory");
        fs::write(path.join("keep.txt"), b"untouched").expect("marker");

        let result = sample_project("would-be corruption").save(&path);

        assert!(result.is_err(), "saving onto a directory must fail");
        assert!(path.is_dir(), "the destination directory must survive");
        assert_eq!(
            fs::read(path.join("keep.txt")).expect("read marker"),
            b"untouched",
            "the destination's contents must be untouched",
        );

        let strays: Vec<_> = fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| *p != path)
            .collect();
        assert!(strays.is_empty(), "temp file left behind: {strays:?}");
    }

    #[test]
    fn load_file_accepts_legacy_awe_project_gracefully() {
        // A real example project with an injected legacy AWE block: it must still
        // load (the unknown `awe` key is ignored) and be flagged by the detector
        // that drives the load-time warning.
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/examples/projects/all-modules-reference.ptz");
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
    fn normalize_project_path_forces_ptz_zip_for_bundles() {
        // Samples present → always `.ptz.zip`, whatever the user typed (a
        // bundle is a ZIP archive around a project; the filename must not
        // claim otherwise).
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.json"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.ptz"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
        assert_eq!(
            normalize_project_path(Path::new("/songs/track"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
        // A bare `.zip` is upgraded rather than left half-named.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.zip"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
    }

    /// The two-dot suffix is the reason `set_extension` alone is not enough:
    /// it replaces only what follows the *last* dot, so re-normalizing an
    /// already-correct bundle path would keep stacking `.ptz`.
    #[test]
    fn normalize_project_path_does_not_stack_the_bundle_suffix() {
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.ptz.zip"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
        // Case-insensitive, like the plain-extension comparison.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.PTZ.ZIP"), true),
            PathBuf::from("/songs/track.ptz.zip")
        );
        // Dropping every sample takes the same path back down to `.ptz`
        // instead of producing `track.ptz.ptz`.
        assert_eq!(
            normalize_project_path(Path::new("/songs/track.ptz.zip"), false),
            PathBuf::from("/songs/track.ptz")
        );
    }

    /// A name that is *only* the suffix has no stem to keep, and stripping it
    /// would leave the parent directory as the thing being renamed.
    #[test]
    fn normalize_project_path_does_not_consume_the_parent_directory() {
        let normalized = normalize_project_path(Path::new("/songs/.ptz.zip"), true);
        assert_eq!(normalized.parent(), Some(Path::new("/songs")));
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
