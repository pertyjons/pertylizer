//! OSC telemetry configuration.

use std::net::{Ipv4Addr, SocketAddr};

/// OSC telemetry sender configuration.
#[derive(Debug, Clone)]
pub struct OscConfig {
    /// Target address for OSC packets.
    pub target: SocketAddr,
    /// State update rate in Hz (RMS, peak, transport).
    pub update_rate_hz: f32,
    /// How often to send `/synth/meta` (seconds, 0 = only on start).
    pub meta_interval_secs: f32,
    /// Seconds without `/viz/pong` before entering idle mode (skip FFT, reduce rate).
    pub idle_timeout_secs: f32,
}

impl OscConfig {
    /// Build a config from the network parts loaded from `pertylizer.toml`,
    /// keeping the remaining intervals at their protocol defaults.
    #[must_use]
    pub fn from_parts(multicast_group: Ipv4Addr, port: u16, update_rate_hz: f32) -> Self {
        Self {
            target: SocketAddr::from((multicast_group, port)),
            update_rate_hz,
            ..Self::default()
        }
    }
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            target: SocketAddr::from((
                synth_osc_protocol::DEFAULT_MULTICAST_GROUP,
                synth_osc_protocol::DEFAULT_OSC_PORT,
            )),
            update_rate_hz: synth_osc_protocol::DEFAULT_UPDATE_RATE_HZ,
            meta_interval_secs: synth_osc_protocol::DEFAULT_META_INTERVAL_SECS,
            idle_timeout_secs: synth_osc_protocol::DEFAULT_IDLE_TIMEOUT_SECS,
        }
    }
}
