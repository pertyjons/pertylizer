//! OSC telemetry sender for Pertylizer.
//!
//! Polls the engine's shared atomic state at ~30 Hz and sends OSC bundles
//! via UDP. Drains note events from a lock-free ring buffer at the same rate.

pub mod addresses;
pub mod config;
mod sender;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use synth_engine::{EngineState, NoteEvent};

pub use config::OscConfig;

/// OSC telemetry sender. Owns the background sender thread.
pub struct OscTelemetry {
    config: OscConfig,
    stop_flag: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl OscTelemetry {
    /// Create a new OSC telemetry sender (not yet started).
    pub fn new(config: OscConfig) -> Self {
        Self {
            config,
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
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

        self.thread_handle = Some(std::thread::spawn(move || {
            sender::run(&config, &engine_state, event_consumer, &stop_flag);
        }));
    }

    /// Stop the sender thread gracefully.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for OscTelemetry {
    fn drop(&mut self) {
        self.stop();
    }
}
