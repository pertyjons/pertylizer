//! OSC telemetry sender for Pertylizer.
//!
//! Polls the engine's shared atomic state at ~30 Hz and sends OSC bundles
//! via UDP. Drains note events from a lock-free ring buffer at the same rate.
//!
//! Supports idle mode: when no visualizer responds with `/viz/pong`, the sender
//! skips FFT computation and sends only meta beacons.

pub mod addresses;
pub mod config;
mod sender;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::JoinHandle;

use synth_engine::{EngineState, NoteEvent};

pub use config::OscConfig;

/// OSC connection status visible to the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OscStatus {
    /// OSC is disabled (`--no-osc`).
    Off = 0,
    /// Sender is running but no client has responded.
    Idle = 1,
    /// A visualizer client is actively responding with `/viz/pong`.
    Connected = 2,
}

impl From<u8> for OscStatus {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Idle,
            2 => Self::Connected,
            _ => Self::Off,
        }
    }
}

/// Shared state readable by the GUI thread.
#[derive(Clone)]
pub struct OscSharedState {
    status: Arc<AtomicU8>,
}

impl OscSharedState {
    /// Current OSC status.
    #[must_use]
    pub fn status(&self) -> OscStatus {
        OscStatus::from(self.status.load(Ordering::Relaxed))
    }
}

/// OSC telemetry sender. Owns the background sender thread.
pub struct OscTelemetry {
    config: OscConfig,
    stop_flag: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    thread_handle: Option<JoinHandle<()>>,
}

impl OscTelemetry {
    /// Create a new OSC telemetry sender (not yet started).
    pub fn new(config: OscConfig) -> Self {
        Self {
            config,
            stop_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(AtomicU8::new(OscStatus::Idle as u8)),
            thread_handle: None,
        }
    }

    /// Get a shared state handle for GUI status display.
    #[must_use]
    pub fn shared_state(&self) -> OscSharedState {
        OscSharedState {
            status: Arc::clone(&self.status),
        }
    }

    /// Start the sender thread. Reads from the shared engine state
    /// (including `master_scope` for FFT spectrum) and drains note events.
    pub fn start(
        &mut self,
        engine_state: Arc<EngineState>,
        event_consumer: ringbuf::HeapCons<NoteEvent>,
    ) {
        if self.thread_handle.is_some() {
            return; // Already running
        }

        let config = self.config.clone();
        let stop_flag = Arc::clone(&self.stop_flag);
        let status = Arc::clone(&self.status);

        match std::thread::Builder::new()
            .name("osc-telemetry".to_string())
            .spawn(move || {
                sender::run(&config, &engine_state, event_consumer, &stop_flag, &status);
            }) {
            Ok(handle) => self.thread_handle = Some(handle),
            Err(_) => {
                // Couldn't spawn the sender thread — leave telemetry off rather
                // than crash; the caller sees status stay `Off`.
                self.status.store(OscStatus::Off as u8, Ordering::Relaxed);
            }
        }
    }

    /// Stop the sender thread gracefully.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.status.store(OscStatus::Off as u8, Ordering::Relaxed);
    }
}

impl Drop for OscTelemetry {
    fn drop(&mut self) {
        self.stop();
    }
}
