//! State transitions owned by confirmation dialogs.

use std::path::PathBuf;

/// Action deferred until the user responds to the unsaved-changes dialog.
pub(super) enum PendingAction {
    /// Create a new project.
    NewProject,
    /// Open a project via file dialog.
    OpenProject,
    /// Load a specific project file.
    LoadProject(PathBuf),
    /// Quit the application.
    Quit,
}

/// State for the unsaved-changes confirmation dialog.
#[derive(Default)]
pub(super) struct UnsavedChangesDialog {
    /// Whether the dialog is currently visible.
    pub(super) open: bool,
    /// The action to perform once the user responds.
    pub(super) pending_action: Option<PendingAction>,
}
