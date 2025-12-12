//! Debug tools for audio engine analysis.
//!
//! This module provides debugging utilities for analyzing the synth engine
//! without real-time playback. Enable with `--features debug-tools`.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ SignalProbe     - Inspect sample values at specific points  │
//! │ GraphDebugger   - Verify module connections                 │
//! │ SequencerDebugger - Step through songs tick-by-tick        │
//! │ VoiceDebugger   - Understand voice allocation               │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use modular_synth::debug::{GraphDebugger, SignalProbe};
//!
//! // Verify connections
//! let dbg = GraphDebugger::new(&voice.graph);
//! assert!(dbg.path_exists(osc_id, amp_id));
//!
//! // Record signal values
//! let mut probe = SignalProbe::new();
//! probe.add_probe(envelope_id, "out");
//! voice.process_with_probe(&mut probe, &context);
//! let stats = probe.stats(&envelope_id, "out").unwrap();
//! assert!(!stats.is_silent);
//! ```

mod graph_debugger;
#[cfg(test)]
mod integration_tests;
mod sequencer_debugger;
mod signal_probe;
mod voice_debugger;

pub use graph_debugger::{ConnectionInfo, GraphDebugger, PathStep, PathTrace};
pub use sequencer_debugger::{
    ActiveNoteInfo, EventSource, PlayState, SequencerDebugger, SequencerSnapshot, TimestampedEvent,
};
pub use signal_probe::{ProbePoint, SignalComparison, SignalProbe, SignalStats};
pub use voice_debugger::{ReleaseReason, VoiceDebugger, VoiceEvent, VoiceInfo, VoiceSnapshot};
