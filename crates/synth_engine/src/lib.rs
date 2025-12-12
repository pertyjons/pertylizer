//! Synth engine with voice allocation, graph processing, and sequencer.
//!
//! This crate provides the core audio engine:
//! - Voice allocation and management
//! - Module graph for signal routing
//! - Parameter system
//! - Effect chains
//! - Command/event system for UI communication

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]

// TODO: Move from src/engine/
