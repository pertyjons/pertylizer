//! Persistent application settings.
//!
//! Stored as JSON at the platform-appropriate config directory:
//! - Linux: `~/.local/share/pertylizer/settings.json`
//! - macOS: `~/Library/Application Support/pertylizer/settings.json`
//! - Windows: `%APPDATA%\pertylizer\settings.json`

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::patch::Author;

#[cfg(feature = "gui-egui")]
use crate::gui::theme::ThemePreset;

/// Application settings that persist across sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Graphics theme.
    #[cfg(feature = "gui-egui")]
    pub theme: ThemePreset,

    /// Selected monospace font family name (one of the bundled fonts). `None`
    /// uses the built-in default. Resolved via `gui::egui_backend::resolve_font`,
    /// so an unknown/removed font falls back gracefully.
    #[cfg(feature = "gui-egui")]
    pub font: Option<String>,

    /// Composer / author information.
    pub author: Author,

    /// Directory preferences.
    pub directories: DirectorySettings,

    /// Main window state.
    pub window: WindowSettings,

    /// Recently opened project file paths (newest first, max 10).
    #[serde(default)]
    pub recent_projects: Vec<PathBuf>,

    /// Warning message from the last load attempt (e.g. corrupt config file).
    /// Populated when settings fail to load and defaults are used instead.
    /// Not serialized — only lives for the current session.
    #[serde(skip)]
    pub load_warning: Option<String>,
}

/// Directory preferences for file dialogs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectorySettings {
    /// Custom patches directory (None = platform default).
    pub patches_dir: Option<PathBuf>,
    /// Last directory used for opening patches.
    pub last_open_dir: Option<PathBuf>,
    /// Last directory used for saving patches.
    pub last_save_dir: Option<PathBuf>,
    /// Custom projects directory (None = platform default).
    pub projects_dir: Option<PathBuf>,
    /// Last directory used for opening/saving projects.
    pub last_project_dir: Option<PathBuf>,
}

/// Persisted window geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
    /// Window X position (if known).
    pub x: Option<i32>,
    /// Window Y position (if known).
    pub y: Option<i32>,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            x: None,
            y: None,
        }
    }
}

impl AppSettings {
    /// Load settings from disk, falling back to defaults on any error.
    ///
    /// When loading fails (missing file, parse error, etc.), the returned
    /// `AppSettings` will have [`Self::load_warning`] set with a description
    /// of what went wrong. The GUI can check this field on startup to show a
    /// user-visible notification.
    #[must_use]
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(s) => s,
            Err(e) => {
                let warning = format!("Could not load settings, using defaults: {e}");
                eprintln!("Settings: {warning}");
                Self {
                    load_warning: Some(warning),
                    ..Self::default()
                }
            }
        }
    }

    /// Try to load settings from the config file.
    fn try_load() -> Result<Self, String> {
        let path = settings_path().map_err(|e| format!("no config dir: {e}"))?;
        let data =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&data).map_err(|e| format!("parse error: {e}"))
    }

    /// Save settings to disk (fire-and-forget convenience wrapper).
    ///
    /// Logs to stderr on failure. Use `Self::try_save` if the caller
    /// needs to surface the error in the UI.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            let path = settings_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            eprintln!("Settings: failed to save to {path}: {e}");
        }
    }

    /// Save settings to disk, returning any error to the caller.
    ///
    /// Prefer this when the caller can surface failures in the UI
    /// (e.g. a toast notification or status bar message).
    pub fn try_save_checked(&self) -> Result<(), String> {
        self.try_save()
    }

    /// Maximum number of recent projects to remember.
    const MAX_RECENT_PROJECTS: usize = 10;

    /// Add a project path to the front of the recent projects list.
    ///
    /// Deduplicates and caps at `Self::MAX_RECENT_PROJECTS`.
    pub fn add_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(Self::MAX_RECENT_PROJECTS);
    }

    /// Remove a stale entry from the recent projects list.
    pub fn remove_recent_project(&mut self, path: &Path) {
        self.recent_projects.retain(|p| p != path);
    }

    /// Clear all recent projects.
    pub fn clear_recent_projects(&mut self) {
        self.recent_projects.clear();
    }

    /// Try to save settings to the config file.
    fn try_save(&self) -> Result<(), String> {
        let path = settings_path().map_err(|e| format!("no config dir: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        // Settings are rewritten on nearly every user action (recent projects,
        // last-used directories). A truncating write that dies midway would
        // leave unparseable JSON and silently reset the user's preferences on
        // the next launch.
        crate::io::atomic::write(&path, json.as_bytes()).map_err(|e| format!("write: {e}"))
    }
}

/// Platform-appropriate settings file path.
pub fn settings_path() -> Result<PathBuf, &'static str> {
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .ok_or("could not determine home directory")?;
    Ok(base.join("pertylizer").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings_roundtrip() {
        let settings = AppSettings::default();
        let json = serde_json::to_string_pretty(&settings).expect("serialize");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.window.width, 1280);
        assert_eq!(parsed.window.height, 800);
        assert!(parsed.author.name.is_empty());
    }

    #[test]
    fn test_recent_projects_add_dedup_cap() {
        let mut settings = AppSettings::default();
        assert!(settings.recent_projects.is_empty());

        settings.add_recent_project(PathBuf::from("/a.json"));
        settings.add_recent_project(PathBuf::from("/b.json"));
        assert_eq!(settings.recent_projects.len(), 2);
        assert_eq!(settings.recent_projects[0], PathBuf::from("/b.json"));

        // Re-adding moves to front
        settings.add_recent_project(PathBuf::from("/a.json"));
        assert_eq!(settings.recent_projects.len(), 2);
        assert_eq!(settings.recent_projects[0], PathBuf::from("/a.json"));

        // Cap at MAX_RECENT_PROJECTS
        for i in 0..15 {
            settings.add_recent_project(PathBuf::from(format!("/proj_{i}.json")));
        }
        assert_eq!(
            settings.recent_projects.len(),
            AppSettings::MAX_RECENT_PROJECTS,
        );
    }

    #[test]
    fn test_recent_projects_remove_and_clear() {
        let mut settings = AppSettings::default();
        settings.add_recent_project(PathBuf::from("/a.json"));
        settings.add_recent_project(PathBuf::from("/b.json"));
        settings.remove_recent_project(Path::new("/a.json"));
        assert_eq!(settings.recent_projects.len(), 1);
        assert_eq!(settings.recent_projects[0], PathBuf::from("/b.json"));

        settings.clear_recent_projects();
        assert!(settings.recent_projects.is_empty());
    }

    #[test]
    fn test_recent_projects_missing_field_uses_default() {
        // JSON without recent_projects should deserialize via #[serde(default)]
        let json = r#"{"window": {"width": 800, "height": 600}}"#;
        let parsed: AppSettings = serde_json::from_str(json).expect("deserialize");
        assert!(parsed.recent_projects.is_empty());
    }

    #[test]
    fn test_partial_json_uses_defaults() {
        let json = r#"{"author": {"name": "Test"}}"#;
        let parsed: AppSettings = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.author.name, "Test");
        assert!(parsed.author.email.is_empty());
        assert_eq!(parsed.window.width, 1280);
    }
}
