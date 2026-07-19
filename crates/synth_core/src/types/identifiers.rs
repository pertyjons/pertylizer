//! Shared identifier newtypes used across the workspace.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Unique identifier for an instrument.
///
/// A single id namespace shared by the sequencer data model, the engine, the
/// project format, the MCP surface, and the GUI. Each instrument gets a unique
/// id that persists for its lifetime; ids are never reused within a session.
///
/// `InstrumentId(0)` (also [`InstrumentId::FIRST`] and the `Default`) is the
/// first/default instrument.
#[must_use]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[repr(transparent)]
pub struct InstrumentId(u64);

impl InstrumentId {
    /// The default/first instrument id.
    pub const FIRST: Self = Self(0);

    /// Create a new instrument id.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw id value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instrument({})", self.0)
    }
}

impl From<u64> for InstrumentId {
    #[inline]
    fn from(id: u64) -> Self {
        Self(id)
    }
}
