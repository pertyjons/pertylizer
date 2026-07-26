#![forbid(unsafe_code)]

//! Runtime configuration loaded from a `pertylizer.toml` file.
//!
//! Both the main application (`pertylizer`) and the standalone visualizer read
//! this file so they agree on the MCP and OSC ports. The file is optional: a
//! development build run via `cargo` (with no file present) falls back to
//! built-in defaults. A shipped release places `pertylizer.toml` next to the
//! executable.
//!
//! Search order: next to the current executable, then the current working
//! directory. The first existing file wins. Unknown or missing fields fall
//! back to defaults, so a partial file is valid.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// File name searched for next to the executable, then in the CWD.
pub const CONFIG_FILE_NAME: &str = "pertylizer.toml";

/// Default MCP HTTP port. Defined here (not in `synth_osc_protocol`, which is
/// OSC-only) so it is the single source of truth for both binaries.
pub const DEFAULT_MCP_PORT: u16 = 9850;

/// Top-level runtime configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// MCP server settings.
    pub mcp: McpConfig,
    /// OSC telemetry network settings.
    pub osc: OscNetConfig,
}

/// MCP server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// HTTP port the MCP server binds to (`http://127.0.0.1:<port>/mcp`).
    pub port: u16,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_MCP_PORT,
        }
    }
}

/// OSC telemetry network settings (shared by sender and visualizer).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OscNetConfig {
    /// UDP port for OSC telemetry.
    pub port: u16,
    /// Multicast group the telemetry is sent to / received from.
    pub multicast_group: Ipv4Addr,
    /// Telemetry update rate in Hz.
    pub update_rate_hz: f32,
}

impl Default for OscNetConfig {
    fn default() -> Self {
        Self {
            port: synth_osc_protocol::DEFAULT_OSC_PORT,
            multicast_group: synth_osc_protocol::DEFAULT_MULTICAST_GROUP,
            update_rate_hz: synth_osc_protocol::DEFAULT_UPDATE_RATE_HZ,
        }
    }
}

impl RuntimeConfig {
    /// Load configuration, falling back to defaults on any error.
    ///
    /// Prints a one-line note to stderr describing where the config was loaded
    /// from (or that a parse/read error caused defaults to be used).
    #[must_use]
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(Some((cfg, path))) => {
                eprintln!("Config: loaded {}", path.display());
                cfg
            }
            Ok(None) => Self::default(),
            Err(e) => {
                eprintln!("Config: could not load {CONFIG_FILE_NAME}, using defaults: {e}");
                Self::default()
            }
        }
    }

    /// `Ok(Some(..))` when a file was found and parsed, `Ok(None)` when no file
    /// exists in any search location, `Err` on a read or parse error.
    fn try_load() -> Result<Option<(Self, PathBuf)>, String> {
        let Some(path) = Self::find_config_file() else {
            return Ok(None);
        };
        let data =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let cfg: Self =
            toml::from_str(&data).map_err(|e| format!("{}: parse error: {e}", path.display()))?;
        Ok(Some((cfg, path)))
    }

    /// First existing `pertylizer.toml`: next to the executable, then the CWD.
    fn find_config_file() -> Option<PathBuf> {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let candidate = dir.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_protocol_constants() {
        let c = RuntimeConfig::default();
        assert_eq!(c.mcp.port, DEFAULT_MCP_PORT);
        assert_eq!(c.osc.port, synth_osc_protocol::DEFAULT_OSC_PORT);
        assert_eq!(
            c.osc.multicast_group,
            synth_osc_protocol::DEFAULT_MULTICAST_GROUP
        );
    }

    #[test]
    fn empty_file_is_all_defaults() {
        let c: RuntimeConfig = toml::from_str("").expect("parse");
        assert_eq!(c.mcp.port, DEFAULT_MCP_PORT);
        assert_eq!(c.osc.port, synth_osc_protocol::DEFAULT_OSC_PORT);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let c: RuntimeConfig = toml::from_str("[mcp]\nport = 9999\n").expect("parse");
        assert_eq!(c.mcp.port, 9999);
        // [osc] omitted entirely → defaults preserved.
        assert_eq!(c.osc.port, synth_osc_protocol::DEFAULT_OSC_PORT);
        assert_eq!(
            c.osc.multicast_group,
            synth_osc_protocol::DEFAULT_MULTICAST_GROUP
        );
    }

    #[test]
    fn full_file_round_trips() {
        let s = "[mcp]\nport = 1234\n\
                 [osc]\nport = 5678\nmulticast_group = \"239.1.2.3\"\nupdate_rate_hz = 60.0\n";
        let c: RuntimeConfig = toml::from_str(s).expect("parse");
        assert_eq!(c.mcp.port, 1234);
        assert_eq!(c.osc.port, 5678);
        assert_eq!(c.osc.multicast_group, Ipv4Addr::new(239, 1, 2, 3));
        assert!((c.osc.update_rate_hz - 60.0).abs() < f32::EPSILON);
    }
}
