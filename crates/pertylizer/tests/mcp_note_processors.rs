//! MCP-level round-trip for the note-processor rack (NP7).
//!
//! Exercises the `SynthBridge` pass-through: add/list/set/remove processors on
//! a pattern's rack via their externally-tagged JSON, and freeze the rack to
//! concrete notes. The expansion semantics themselves are covered in the
//! `synth_sequencer` crate; here we only check the MCP plumbing round-trips.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use serde_json::json;
use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_bridge() -> AppSynthBridge {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("Headless")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));
    AppSynthBridge::new(session, shared, sample_library)
}

#[test]
fn note_processor_rack_round_trip_via_bridge() {
    let bridge = build_bridge();
    let pid = bridge
        .create_pattern("phrase", 4.0)
        .expect("create pattern");
    // One source note for the chord/arp to act on.
    bridge.add_note(pid, 60, 0.0, 1.0, 100).expect("add note");

    // Add a chord then an arpeggiator — added out of chain order; the rack
    // must sort them canonically (chord stage 1 before arp stage 2).
    // Partial configs work thanks to #[serde(default)]: an empty Arpeggiator
    // is the default (Up, 16th, 1 octave); an empty Chord is a major triad.
    let arp_idx = bridge
        .add_note_processor(pid, json!({ "Arpeggiator": {} }))
        .expect("add arp");
    let chord_idx = bridge
        .add_note_processor(pid, json!({ "Chord": {} }))
        .expect("add chord");
    assert_eq!(arp_idx, 0, "arp inserted first");
    assert_eq!(chord_idx, 0, "chord inserts before the arp (lower stage)");

    let procs = bridge.list_note_processors(pid).expect("list");
    assert_eq!(procs.len(), 2);
    assert_eq!(procs[0].kind, "chord");
    assert_eq!(procs[0].stage, 1);
    assert_eq!(procs[1].kind, "arpeggiator");
    assert_eq!(procs[1].stage, 2);

    // The reported count surfaces in the pattern overview too.
    let pat = bridge
        .list_patterns()
        .unwrap()
        .into_iter()
        .find(|p| p.id == pid)
        .unwrap();
    assert_eq!(pat.processor_count, 2);

    // The config round-trips: feeding a listed processor's config straight back
    // through set_note_processor is accepted.
    let chord_config = procs[0].config.clone();
    bridge
        .set_note_processor(pid, 0, chord_config)
        .expect("set with the round-tripped config");

    // Invalid JSON is rejected, not panicked.
    assert!(
        bridge
            .add_note_processor(pid, json!({ "NotARealKind": {} }))
            .is_err(),
        "unknown processor kind must error"
    );
    // A typo'd config field is rejected (deny_unknown_fields) rather than
    // silently producing a default processor.
    assert!(
        bridge
            .add_note_processor(pid, json!({ "Chord": { "intervls": [0, 3, 7] } }))
            .is_err(),
        "misspelled config field must error"
    );
    // Replacing a processor with a different chain stage is rejected (it would
    // break the locked rack order); index 0 is the chord (stage 1).
    assert!(
        bridge
            .set_note_processor(pid, 0, json!({ "Arpeggiator": {} }))
            .is_err(),
        "cross-stage replacement must error"
    );
    // Out-of-range index errors.
    assert!(bridge.remove_note_processor(pid, 9).is_err());
    assert!(
        bridge
            .set_note_processor(pid, 9, json!({ "Humanize": {} }))
            .is_err()
    );

    // Remove the arp, leaving just the chord.
    bridge.remove_note_processor(pid, 1).expect("remove arp");
    assert_eq!(bridge.list_note_processors(pid).unwrap().len(), 1);

    // Freeze the rack: the chord bakes the source C into a C-major triad and
    // clears the rack.
    let note_count = bridge.freeze_note_processors(pid).expect("freeze");
    assert_eq!(note_count, 3, "C major triad baked");
    assert!(bridge.list_note_processors(pid).unwrap().is_empty());
    assert_eq!(bridge.list_notes(pid).unwrap().len(), 3);
}

#[test]
fn note_processor_accepts_stringified_json_blob() {
    // §2 fix: LLM clients commonly send the processor as a stringified-JSON
    // object (a `Value::String`) rather than a raw object. Both add and set must
    // tolerate that — the externally-tagged enum would otherwise read the whole
    // string as the variant tag and fail with `unknown variant`.
    let bridge = build_bridge();
    let pid = bridge.create_pattern("blob", 4.0).expect("create pattern");
    bridge.add_note(pid, 60, 0.0, 1.0, 100).expect("add note");

    // A stringified object is accepted by add.
    let blob = json!(serde_json::to_string(&json!({ "Chord": {} })).unwrap());
    let idx = bridge
        .add_note_processor(pid, blob)
        .expect("add must tolerate a stringified-JSON blob");
    assert_eq!(bridge.list_note_processors(pid).unwrap()[idx].kind, "chord");

    // The same blob form is accepted by the in-place set (the §2 repro): same
    // chain stage, so it is not rejected for reordering.
    let set_blob = json!(serde_json::to_string(&json!({ "Chord": {} })).unwrap());
    bridge
        .set_note_processor(pid, idx, set_blob)
        .expect("set must tolerate a stringified-JSON blob");

    // A genuine non-JSON string still surfaces a clear error, not a panic.
    assert!(
        bridge
            .add_note_processor(pid, json!("not json at all"))
            .is_err(),
        "a non-JSON string must still error cleanly"
    );
}

#[test]
fn chiptune_arp_round_trips_via_bridge() {
    // The chiptune payoff: a Custom arp with a sub-grid rate, a bare-array
    // offset list, and legato must round-trip through the MCP pass-through with
    // no tool-signature change (externally-tagged JSON straight into serde).
    let bridge = build_bridge();
    let pid = bridge.create_pattern("chip", 4.0).expect("create pattern");
    bridge.add_note(pid, 60, 0.0, 1.0, 100).expect("add note");

    bridge
        .add_note_processor(
            pid,
            json!({ "Arpeggiator": {
                "mode": "Custom",
                "rate": { "Ticks": 40 },
                "legato": true,
                "custom": [0, 4, 7]
            }}),
        )
        .expect("add chiptune arp");

    let procs = bridge.list_note_processors(pid).expect("list");
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].kind, "arpeggiator");
    let cfg = &procs[0].config["Arpeggiator"];
    assert_eq!(cfg["mode"], "Custom");
    assert_eq!(cfg["rate"]["Ticks"], 40, "sub-grid rate round-trips");
    assert_eq!(cfg["legato"], true);
    assert_eq!(cfg["custom"], json!([0, 4, 7]), "offsets as a bare array");

    // The listed config feeds straight back through set (full round-trip).
    bridge
        .set_note_processor(pid, 0, procs[0].config.clone())
        .expect("round-trip the listed config");

    // The frame-locked MilliHz variant parses too (50 Hz PAL).
    bridge
        .set_note_processor(
            pid,
            0,
            json!({ "Arpeggiator": {
                "mode": "Custom",
                "rate": { "MilliHz": 50_000 },
                "legato": true,
                "custom": [0, 12, 7, 4]
            }}),
        )
        .expect("frame-locked MilliHz arp accepted");
    let procs = bridge.list_note_processors(pid).expect("list again");
    assert_eq!(procs[0].config["Arpeggiator"]["rate"]["MilliHz"], 50_000);
}

#[test]
fn set_and_clear_note_ornament_via_bridge() {
    let bridge = build_bridge();
    let pid = bridge
        .create_pattern("phrase", 4.0)
        .expect("create pattern");
    let note = bridge.add_note(pid, 60, 0.0, 1.0, 100).expect("add note");

    // A fresh note has no ornament.
    let listed = bridge.list_notes(pid).unwrap();
    assert!(listed[0].ornament.is_none());

    // Set a flam (partial config; #[serde(default)] fills the rest).
    bridge
        .set_note_ornament(pid, note.id, Some(json!({ "count": 2, "spacing": 60 })))
        .expect("set ornament");
    let listed = bridge.list_notes(pid).unwrap();
    let orn = listed[0]
        .ornament
        .as_ref()
        .expect("ornament surfaced in reader");
    assert_eq!(orn["count"], 2);
    assert_eq!(orn["spacing"], 60);

    // A typo'd field is rejected (deny_unknown_fields).
    assert!(
        bridge
            .set_note_ornament(pid, note.id, Some(json!({ "kount": 3 })))
            .is_err(),
        "misspelled ornament field must error"
    );

    // null clears it.
    bridge
        .set_note_ornament(pid, note.id, Some(serde_json::Value::Null))
        .expect("clear ornament with null");
    assert!(bridge.list_notes(pid).unwrap()[0].ornament.is_none());

    // None also clears (and unknown note errors).
    assert!(bridge.set_note_ornament(pid, 999, None).is_err());
}

#[test]
fn note_processor_tools_reject_unknown_pattern() {
    let bridge = build_bridge();
    assert!(bridge.list_note_processors(999).is_err());
    assert!(
        bridge
            .add_note_processor(999, json!({ "Humanize": {} }))
            .is_err()
    );
    assert!(bridge.freeze_note_processors(999).is_err());
}
