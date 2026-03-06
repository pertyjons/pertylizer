//! Clipboard system for copy/paste of modules in the patch editor.
//!
//! Stores module states and their internal connections for pasting
//! as new instances with the same parameters.

use crate::patch::{ConnectionState, ModuleState};

/// Default paste offset in pixels (right and down from original position).
const PASTE_OFFSET: f32 = 50.0;

/// Clipboard contents for copy/paste of modules.
pub(crate) struct ModuleClipboard {
    /// Copied module states (with parameters and positions).
    modules: Vec<ModuleState>,
    /// Connections between the copied modules (internal only).
    connections: Vec<ConnectionState>,
    /// Reference position for paste offset (center of copied selection).
    reference_pos: (f32, f32),
}

impl ModuleClipboard {
    /// Create a new empty clipboard.
    pub(crate) fn new() -> Self {
        Self {
            modules: Vec::new(),
            connections: Vec::new(),
            reference_pos: (0.0, 0.0),
        }
    }

    /// Check if the clipboard has no content.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Copy modules and their internal connections into the clipboard.
    ///
    /// Only connections where both endpoints are in the selection are kept.
    pub(crate) fn copy_modules(
        &mut self,
        modules: &[ModuleState],
        connections: &[ConnectionState],
    ) {
        self.modules = modules.to_vec();
        self.connections = connections.to_vec();

        // Compute center of selection as reference position
        if modules.is_empty() {
            self.reference_pos = (0.0, 0.0);
        } else {
            let (sum_x, sum_y) = modules.iter().fold((0.0_f32, 0.0_f32), |(sx, sy), m| {
                (sx + m.position.0, sy + m.position.1)
            });
            #[allow(clippy::cast_precision_loss)]
            let count = modules.len() as f32;
            self.reference_pos = (sum_x / count, sum_y / count);
        }
    }

    /// Access the clipboard contents.
    ///
    /// Returns (modules, connections, reference_pos).
    #[must_use]
    pub(crate) fn contents(&self) -> (&[ModuleState], &[ConnectionState], (f32, f32)) {
        (&self.modules, &self.connections, self.reference_pos)
    }

    /// Get the default paste offset.
    #[must_use]
    pub(crate) fn paste_offset() -> f32 {
        PASTE_OFFSET
    }
}
