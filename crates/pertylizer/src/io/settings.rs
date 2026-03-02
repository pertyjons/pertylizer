//! Persistent application settings.
//!
//! Stored as JSON at the platform-appropriate config directory:
//! - Linux: `~/.local/share/pertylizer/settings.json`
//! - macOS: `~/Library/Application Support/pertylizer/settings.json`
//! - Windows: `%APPDATA%\pertylizer\settings.json`

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg(feature = "gui-egui")]
use crate::gui::theme::ThemePreset;

/// Application settings that persist across sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Graphics theme.
    #[cfg(feature = "gui-egui")]
    pub theme: ThemePreset,

    /// Composer / author information.
    pub author: AuthorInfo,

    /// Directory preferences.
    pub directories: DirectorySettings,

    /// Main window state.
    pub window: WindowSettings,
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

/// Author / composer information embedded in saved songs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthorInfo {
    /// Full name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// Website URL.
    pub website: String,
    /// Default license for compositions (e.g. "CC BY 4.0", "All rights reserved").
    pub license: String,
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
    #[must_use]
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Settings: using defaults ({e})");
                Self::default()
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

    /// Save settings to disk.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("Settings: failed to save ({e})");
        }
    }

    /// Try to save settings to the config file.
    fn try_save(&self) -> Result<(), String> {
        let path = settings_path().map_err(|e| format!("no config dir: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("write: {e}"))
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
    fn test_partial_json_uses_defaults() {
        let json = r#"{"author": {"name": "Test"}}"#;
        let parsed: AppSettings = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.author.name, "Test");
        assert!(parsed.author.email.is_empty());
        assert_eq!(parsed.window.width, 1280);
    }
}
