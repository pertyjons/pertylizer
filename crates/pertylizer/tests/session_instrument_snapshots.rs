//! Regression tests for synchronous instrument control snapshots.
//!
//! These tests never call `engine.process()`: accepted commands must publish
//! their read-side state before the DSP mutation reaches the audio thread.

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;

use pertylizer::session::SynthSession;

#[test]
fn add_instrument_visible_to_validator_before_engine_tick() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let id = session.add_instrument("test").expect("add_instrument");

    assert!(session.instrument_exists(id));
    assert_eq!(session.state().instrument_snapshots.read().len(), 1);
}

#[test]
fn remove_instrument_drops_from_snapshot_immediately() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let id = session.add_instrument("test").expect("add_instrument");
    assert!(session.instrument_exists(id));

    session.remove_instrument(id).expect("remove_instrument");

    assert!(!session.instrument_exists(id));
}

#[test]
fn unknown_instrument_id_is_not_alive() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    assert!(!session.instrument_exists(InstrumentId::new(999)));
}

#[test]
fn add_instrument_with_id_populates_snapshot() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let id = InstrumentId::new(42);
    session
        .add_instrument_with_id(id, "loaded")
        .expect("add_instrument_with_id");

    assert!(session.instrument_exists(id));
}
