//! Error types for the MCP bridge.

/// Errors that can occur in the MCP bridge.
#[derive(Debug, thiserror::Error)]
pub enum McpBridgeError {
    /// Instrument not found.
    #[error("instrument not found: {0}")]
    InstrumentNotFound(u64),

    /// Module not found.
    #[error("module not found: {0}")]
    ModuleNotFound(String),

    /// Parameter not found.
    #[error("parameter not found: {0}")]
    ParameterNotFound(String),

    /// Command failed to send.
    #[error("command send failed")]
    CommandSendFailed,

    /// Example patch not found.
    #[error("example patch not found: {0}")]
    PatchNotFound(String),

    /// Invalid module type.
    #[error("invalid module type: {0}")]
    InvalidModuleType(String),

    /// Port not found on module.
    #[error("port not found: {port} on module {module}")]
    PortNotFound { module: String, port: String },

    /// Pattern not found.
    #[error("pattern not found: {0}")]
    PatternNotFound(u32),

    /// Note not found.
    #[error("note not found: {0}")]
    NoteNotFound(u64),

    /// Track not found.
    #[error("track not found: {0}")]
    TrackNotFound(u16),

    /// Song lock poisoned.
    #[error("song lock poisoned")]
    SongLockPoisoned,
}
