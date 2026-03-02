//! Patch file management for the synthesizer.
//!
//! Provides high-level operations for managing patch files:
//! - Finding and creating the patches directory
//! - Listing available patches
//! - Loading patch metadata without full parsing

use std::fs;
use std::path::{Path, PathBuf};

use crate::patch::{Patch, PatchError};

/// Information about a patch file without loading full content.
#[derive(Debug, Clone)]
pub struct PatchInfo {
    /// File path.
    pub path: PathBuf,
    /// Patch name (from filename or metadata).
    pub name: String,
    /// Optional description if available.
    pub description: Option<String>,
    /// File modification time.
    pub modified: Option<std::time::SystemTime>,
}

/// Manages patch files on disk.
pub struct PatchManager {
    /// Directory for user patches.
    patches_dir: PathBuf,
}

impl PatchManager {
    /// Create a new patch manager with the default patches directory.
    ///
    /// The default directory is:
    /// - Linux: `~/.local/share/pertylizer/patches`
    /// - macOS: `~/Library/Application Support/pertylizer/patches`
    /// - Windows: `%APPDATA%\pertylizer\patches`
    pub fn new() -> Result<Self, PatchError> {
        let patches_dir = Self::default_patches_dir()?;
        Self::with_directory(patches_dir)
    }

    /// Create a patch manager with a custom directory.
    pub fn with_directory(patches_dir: PathBuf) -> Result<Self, PatchError> {
        // Create directory if it doesn't exist
        if !patches_dir.exists() {
            fs::create_dir_all(&patches_dir).map_err(|e| {
                PatchError::Io(format!("Failed to create patches directory: {}", e))
            })?;
        }

        Ok(Self { patches_dir })
    }

    /// Get the default patches directory based on the platform.
    pub fn default_patches_dir() -> Result<PathBuf, PatchError> {
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| PatchError::Io("Could not determine home directory".to_string()))?;

        Ok(base.join("pertylizer").join("patches"))
    }

    /// Get the patches directory path.
    pub fn patches_dir(&self) -> &Path {
        &self.patches_dir
    }

    /// List all available patches.
    ///
    /// Returns patch info for all `.json` files in the patches directory.
    pub fn list_patches(&self) -> Result<Vec<PatchInfo>, PatchError> {
        let mut patches = Vec::new();

        let entries = fs::read_dir(&self.patches_dir)
            .map_err(|e| PatchError::Io(format!("Failed to read patches directory: {}", e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(info) = self.get_patch_info(&path)
            {
                patches.push(info);
            }
        }

        // Sort by modification time (newest first)
        patches.sort_by(|a, b| match (&b.modified, &a.modified) {
            (Some(b_time), Some(a_time)) => b_time.cmp(a_time),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        });

        Ok(patches)
    }

    /// Get info about a specific patch file without fully loading it.
    pub fn get_patch_info(&self, path: &Path) -> Result<PatchInfo, PatchError> {
        let metadata = fs::metadata(path)
            .map_err(|e| PatchError::Io(format!("Failed to read file metadata: {}", e)))?;

        // Try to read just enough to get the name and description
        let content = fs::read_to_string(path)
            .map_err(|e| PatchError::Io(format!("Failed to read patch file: {}", e)))?;

        // Quick parse to extract name and description
        let (name, description) = Self::extract_patch_metadata(&content, path);

        Ok(PatchInfo {
            path: path.to_path_buf(),
            name,
            description,
            modified: metadata.modified().ok(),
        })
    }

    /// Extract name and description from patch JSON without full parsing.
    fn extract_patch_metadata(content: &str, path: &Path) -> (String, Option<String>) {
        // Try quick JSON value extraction
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    // Fall back to filename
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                });

            let description = value
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);

            (name, description)
        } else {
            // Parse failed, use filename
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            (name, None)
        }
    }

    /// Load a patch from the patches directory by name.
    pub fn load_patch(&self, name: &str) -> Result<Patch, PatchError> {
        let filename = Self::sanitize_filename(name);
        let path = self.patches_dir.join(format!("{}.json", filename));
        Patch::load(&path)
    }

    /// Save a patch to the patches directory.
    #[allow(clippy::similar_names)] // filename vs file - different concepts
    pub fn save_patch(&self, patch: &Patch) -> Result<PathBuf, PatchError> {
        let filename = Self::sanitize_filename(&patch.name);
        let path = self.patches_dir.join(format!("{}.json", filename));
        patch.save(&path)?;
        Ok(path)
    }

    /// Delete a patch from the patches directory.
    pub fn delete_patch(&self, name: &str) -> Result<(), PatchError> {
        let filename = Self::sanitize_filename(name);
        let path = self.patches_dir.join(format!("{}.json", filename));

        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| PatchError::Io(format!("Failed to delete patch: {}", e)))?;
        }

        Ok(())
    }

    /// Check if a patch with the given name exists.
    pub fn patch_exists(&self, name: &str) -> bool {
        let filename = Self::sanitize_filename(name);
        let path = self.patches_dir.join(format!("{}.json", filename));
        path.exists()
    }

    /// Sanitize a name for use as a filename.
    /// Only allows ASCII alphanumeric characters, hyphens, and underscores.
    fn sanitize_filename(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
}

impl Default for PatchManager {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            // Fallback to current directory if default fails
            Self {
                patches_dir: PathBuf::from("patches"),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_patch_manager_creation() {
        let temp = tempdir().unwrap();
        let manager = PatchManager::with_directory(temp.path().to_path_buf()).unwrap();
        assert!(manager.patches_dir().exists());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(PatchManager::sanitize_filename("My Patch!"), "my_patch_");
        assert_eq!(
            PatchManager::sanitize_filename("test-patch_1"),
            "test-patch_1"
        );
        assert_eq!(PatchManager::sanitize_filename("Спейс"), "_____");
    }

    #[test]
    fn test_list_empty_directory() {
        let temp = tempdir().unwrap();
        let manager = PatchManager::with_directory(temp.path().to_path_buf()).unwrap();
        let patches = manager.list_patches().unwrap();
        assert!(patches.is_empty());
    }

    #[test]
    fn test_save_and_load_patch() {
        let temp = tempdir().unwrap();
        let manager = PatchManager::with_directory(temp.path().to_path_buf()).unwrap();

        let patch = Patch::new("Test Patch");
        let saved_path = manager.save_patch(&patch).unwrap();
        assert!(saved_path.exists());

        let loaded = manager.load_patch("Test Patch").unwrap();
        assert_eq!(loaded.name, "Test Patch");
    }

    #[test]
    fn test_patch_exists() {
        let temp = tempdir().unwrap();
        let manager = PatchManager::with_directory(temp.path().to_path_buf()).unwrap();

        assert!(!manager.patch_exists("NonExistent"));

        let patch = Patch::new("Exists");
        manager.save_patch(&patch).unwrap();

        assert!(manager.patch_exists("Exists"));
    }
}
