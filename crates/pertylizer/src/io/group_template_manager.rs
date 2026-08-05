//! Group template file management.
//!
//! Provides high-level operations for managing group template files:
//! - Finding and creating the templates directory
//! - Listing available templates
//! - Loading and saving templates

use std::fs;
use std::path::{Path, PathBuf};

use crate::patch::{GroupCategory, GroupTemplate, PatchError, categorized_group_templates};

/// Where a group template comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupTemplateSource {
    /// A built-in template, referenced by its index in the flat list.
    BuiltIn(usize),
    /// A user-saved template on disk.
    File(PathBuf),
}

/// Information about a group template without loading full content.
#[derive(Debug, Clone)]
pub struct GroupTemplateInfo {
    /// Where this template comes from.
    pub source: GroupTemplateSource,
    /// Template name.
    pub name: String,
    /// Optional description if available.
    pub description: Option<String>,
    /// Optional category.
    pub category: Option<GroupCategory>,
    /// File modification time (None for built-in templates).
    pub modified: Option<std::time::SystemTime>,
}

/// Manages group template files on disk.
pub struct GroupTemplateManager {
    templates_dir: PathBuf,
}

impl GroupTemplateManager {
    /// Create a new template manager with the default templates directory.
    ///
    /// The default directory is:
    /// - Linux: `~/.local/share/pertylizer/group-templates`
    /// - macOS: `~/Library/Application Support/pertylizer/group-templates`
    /// - Windows: `%APPDATA%\pertylizer\group-templates`
    pub fn new() -> Result<Self, PatchError> {
        let templates_dir = Self::default_templates_dir()?;
        Self::with_directory(templates_dir)
    }

    /// Create a template manager with a custom directory.
    pub fn with_directory(templates_dir: PathBuf) -> Result<Self, PatchError> {
        if !templates_dir.exists() {
            fs::create_dir_all(&templates_dir).map_err(|e| {
                PatchError::Io(format!("Failed to create group templates directory: {e}"))
            })?;
        }

        Ok(Self { templates_dir })
    }

    /// Get the default templates directory based on the platform.
    pub fn default_templates_dir() -> Result<PathBuf, PatchError> {
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| PatchError::Io("Could not determine home directory".to_string()))?;

        Ok(base.join("pertylizer").join("group-templates"))
    }

    /// Get the templates directory path.
    pub fn templates_dir(&self) -> &Path {
        &self.templates_dir
    }

    /// List all available templates.
    ///
    /// Returns template info for all `.json` files in the templates directory.
    pub fn list_templates(&self) -> Result<Vec<GroupTemplateInfo>, PatchError> {
        let mut templates = Vec::new();

        let entries = fs::read_dir(&self.templates_dir).map_err(|e| {
            PatchError::Io(format!("Failed to read group templates directory: {e}"))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(info) = self.get_template_info(&path)
            {
                templates.push(info);
            }
        }

        // Sort by modification time (newest first)
        templates.sort_by(|a, b| match (&b.modified, &a.modified) {
            (Some(b_time), Some(a_time)) => b_time.cmp(a_time),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        });

        Ok(templates)
    }

    /// Get info about a specific template file without fully loading it.
    pub fn get_template_info(&self, path: &Path) -> Result<GroupTemplateInfo, PatchError> {
        let metadata = fs::metadata(path)
            .map_err(|e| PatchError::Io(format!("Failed to read file metadata: {e}")))?;

        let content = fs::read_to_string(path)
            .map_err(|e| PatchError::Io(format!("Failed to read template file: {e}")))?;

        let (name, description, category) = Self::extract_template_metadata(&content, path);

        Ok(GroupTemplateInfo {
            source: GroupTemplateSource::File(path.to_path_buf()),
            name,
            description,
            category,
            modified: metadata.modified().ok(),
        })
    }

    /// List all available templates: built-in first, then user-saved files.
    pub fn list_all_templates(&self) -> Vec<GroupTemplateInfo> {
        let mut all = Vec::new();

        // Built-in templates
        let mut index = 0;
        for (category, templates) in categorized_group_templates() {
            for t in templates {
                all.push(GroupTemplateInfo {
                    source: GroupTemplateSource::BuiltIn(index),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    category: Some(category),
                    modified: None,
                });
                index += 1;
            }
        }

        // User-saved templates from disk
        if let Ok(file_templates) = self.list_templates() {
            all.extend(file_templates);
        }

        all
    }

    /// Load a group template from a file path.
    pub fn load_template(&self, path: &Path) -> Result<GroupTemplate, PatchError> {
        let content = fs::read_to_string(path)
            .map_err(|e| PatchError::Io(format!("Failed to read template file: {e}")))?;

        serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save a group template to disk.
    pub fn save_template(&self, template: &GroupTemplate) -> Result<PathBuf, PatchError> {
        let filename = Self::sanitize_filename(&template.name);
        let path = self.templates_dir.join(format!("{filename}.json"));

        let content = serde_json::to_string_pretty(template)
            .map_err(|e| PatchError::Serialize(e.to_string()))?;
        crate::io::atomic::write(&path, content.as_bytes())
            .map_err(|e| PatchError::Io(format!("Failed to write template: {e}")))?;

        Ok(path)
    }

    /// Extract name/description/category from template JSON without full parsing.
    fn extract_template_metadata(
        content: &str,
        path: &Path,
    ) -> (String, Option<String>, Option<GroupCategory>) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                });

            let description = value
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);

            let category = value
                .get("category")
                .and_then(|v| v.as_str())
                .and_then(|s| match s {
                    "voice" => Some(GroupCategory::Voice),
                    "effect" => Some(GroupCategory::Effect),
                    "utility" => Some(GroupCategory::Utility),
                    "tutorial" => Some(GroupCategory::Tutorial),
                    _ => None,
                });

            (name, description, category)
        } else {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            (name, None, None)
        }
    }

    /// Sanitize a name for use as a filename.
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

impl Default for GroupTemplateManager {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            templates_dir: PathBuf::from("group-templates"),
        })
    }
}
