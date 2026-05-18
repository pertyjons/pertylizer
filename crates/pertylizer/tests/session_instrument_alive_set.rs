//! Regression test for the `apply_example_patch` → `set_instrument_volume`
//! bridge race documented in `docs/TODO.md` §0.1.
//!
//! Before the fix, `instrument_exists` consulted `EngineState::instrument_snapshots`
//! which is only rebuilt on the audio thread after it pops an `EngineCommand`.
//! Inside one `batch_execute` the audio thread had not ticked yet, so a write
//! that validated against the snapshot would fail with "instrument not found"
//! despite the ID having just been handed out.
//!
//! These tests never call `engine.process()` — they exercise the same window
//! the bug lived in: command queued, engine yet to tick.

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;

use pertylizer::session::SynthSession;

#[test]
fn add_instrument_visible_to_validator_before_engine_tick() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let id = session.add_instrument("test").expect("add_instrument");

    // Engine has not processed AddInstrument yet — snapshot is still empty.
    // The synchronous alive-set must already report the ID as existing,
    // or any validator inside the same batch (set_instrument_volume,
    // set_instrument_pan, list_modules, etc.) would falsely reject it.
    assert!(session.instrument_exists(id));
    assert!(session.state().instrument_snapshots.read().is_empty());
}

#[test]
fn remove_instrument_drops_from_alive_set_immediately() {
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
fn add_instrument_with_id_populates_alive_set() {
    let (_engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let id = InstrumentId::new(42);
    session
        .add_instrument_with_id(id, "loaded")
        .expect("add_instrument_with_id");

    assert!(session.instrument_exists(id));
}
