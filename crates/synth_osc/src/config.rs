//! OSC telemetry configuration.

use std::net::SocketAddr;

/// OSC telemetry sender configuration.
#[derive(Debug, Clone)]
pub struct OscConfig {
    /// Target address for OSC packets.
    pub target: SocketAddr,
    /// State update rate in Hz (RMS, peak, transport).
    pub update_rate_hz: f32,
    /// How often to send `/synth/meta` (seconds, 0 = only on start).
    pub meta_interval_secs: f32,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            target: "127.0.0.1:9000"
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 9000))),
            update_rate_hz: 30.0,
            meta_interval_secs: 5.0,
        }
    }
}
