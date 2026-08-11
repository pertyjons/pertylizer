//! Integration tests for `batch_execute`'s `dry_run` and `rollback` modes.
//!
//! `dry_run` must validate every operation without mutating any state.
//! `rollback` must snapshot the project before the batch and restore it if any
//! operation fails, so the batch is all-or-nothing.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_mcp::SynthMcpServer;
use synth_mcp::bridge::*;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

/// Build a headless server sharing `song` so tests can read the live song state.
fn build_server() -> (SynthMcpServer, Arc<synth_sequencer::SharedSong>) {
    let (server, song, _shared) = build_server_with_shared();
    (server, song)
}

/// The same server, plus the shared state — for the one test that has to act as a
/// second client.
fn build_server_with_shared() -> (
    SynthMcpServer,
    Arc<synth_sequencer::SharedSong>,
    Arc<McpSharedState>,
) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Original")));
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
    let bridge = AppSynthBridge::new(session, Arc::clone(&shared), sample_library);
    let server = SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>);
    (server, song, shared)
}

#[tokio::test]
async fn dry_run_validates_without_mutating() {
    let (server, song) = build_server();

    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } },
                { "tool": "not_a_real_tool", "params": {} }
            ],
            "dry_run": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["dry_run"], true);
    // The valid op validates; the bogus tool is reported unknown.
    assert_eq!(v["succeeded"], 1);
    assert_eq!(v["failed"], 1);
    assert!(
        v["results"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("dry_run OK"),
        "valid op should report dry_run OK, got {}",
        v["results"][0]
    );
    // A validated op ran nothing, so it has no payload to carry.
    assert!(v["results"][0].get("structured").is_none());
    assert_eq!(v["results"][0]["status"], "success");
    assert_eq!(v["results"][1]["status"], "failure");
    // Nothing executed: the song name is untouched.
    assert_eq!(song.read().name, "Original");
}

#[tokio::test]
async fn rollback_restores_project_on_failure() {
    let (server, song) = build_server();

    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } },
                { "tool": "not_a_real_tool", "params": {} }
            ],
            "rollback": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["rolled_back"], true);
    // rollback implies stop-on-error: the first op ran, the second failed, then
    // the whole batch was undone — so the name is back to its pre-batch value.
    assert_eq!(
        song.read().name,
        "Original",
        "a failed rollback batch must restore the project"
    );
}

/// All-or-nothing means *all*: an operation that half-applied has left the
/// project changed in a way the caller did not ask to keep.
///
/// This is the case the contract used to lose. A `partial` op counted as
/// succeeded, so `rollback: true` neither stopped nor restored, `failed` read 0,
/// and the batch reported success while the project sat half-changed — the exact
/// promise `rollback` exists to make.
#[tokio::test]
async fn rollback_restores_when_an_operation_only_half_applies() {
    let (server, song) = build_server();
    // One real pattern to rename, so the op has something that *can* succeed.
    let created = server
        .batch_execute_for_test(serde_json::json!({ "operations": [
            { "tool": "create_pattern", "params": { "patterns": [{ "name": "Keep", "length_beats": 4 }] } }
        ] }))
        .await;
    let created: serde_json::Value = serde_json::from_str(&created).expect("batch result is JSON");
    let pattern_id = created["results"][0]["structured"]["items"][0]["id"]
        .as_u64()
        .expect("the created pattern id");

    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } },
                { "tool": "rename_pattern", "params": { "items": [
                    { "pattern_id": pattern_id, "name": "Renamed" },
                    { "pattern_id": 9999, "name": "No such pattern" }
                ] } }
            ],
            "rollback": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["results"][1]["status"], "partial", "sanity: {resp}");
    assert_eq!(
        v["partial"], 1,
        "the half-applied op is counted as such: {resp}"
    );
    assert_eq!(
        v["rolled_back"], true,
        "a partial operation must trip the rollback it was promised: {resp}"
    );
    assert_eq!(
        song.read().name,
        "Original",
        "and the restore must actually undo the batch"
    );
    // One op *had* completed before the restore, which is why the whole-batch
    // verdict cannot be read off the counters — see the wire test for the
    // `is_error` this produces.
    assert_eq!(v["succeeded"], 1, "sanity: one op had completed: {resp}");
}

/// A write that landed *before* the batch is part of what the snapshot captured,
/// so it must not read as concurrent.
///
/// The guard predicts the mutation counter — capture value plus one per mutating
/// operation — and refuses to restore when the real one moved further, because the
/// extra movement is somebody else's write and the snapshot predates it. This pins
/// the direction that would break rollback entirely if the arithmetic were off by
/// a baseline: every rollback batch in a session that had ever mutated would
/// refuse.
///
/// The true positive — a write arriving *between* two operations — is not driven
/// from here: the op loop has no yield point between dispatches, so a second task
/// cannot deterministically interleave, and a timing-dependent version of this
/// test would be worse than none. The arithmetic it depends on is one line, shared
/// with the predicate `dispatch_tool` bumps on.
#[tokio::test]
async fn a_write_before_the_batch_does_not_read_as_concurrent() {
    let (server, song, shared) = build_server_with_shared();
    // Another client's earlier call — captured *by* the snapshot, not after it.
    shared
        .mutation_seq
        .fetch_add(3, std::sync::atomic::Ordering::AcqRel);

    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } },
                { "tool": "rename_pattern", "params": { "items": [
                    { "pattern_id": 9999, "name": "No such pattern" }
                ] } }
            ],
            "rollback": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["failed"], 1, "sanity: the batch did fail: {resp}");
    assert_eq!(
        v["rolled_back"], true,
        "the pre-batch write is baseline, so the rollback must still happen: {resp}"
    );
    assert!(
        v["rollback_error"].is_null(),
        "and nothing to report: {resp}"
    );
    assert_eq!(song.read().name, "Original", "the restore happened");
}

/// A typed payload that says it is incomplete counts as incomplete.
///
/// `build_instrument` answers its own type, not a `MutationResult`, and states
/// partiality as `partial_success` inside it. Nothing generic could see that, so
/// a half-built instrument inside a `rollback: true` batch was reported
/// `status: success`, counted as succeeded, and left standing. The verdict
/// travels with the reply now instead of being inferred from the payload's shape.
#[tokio::test]
async fn rollback_restores_a_partially_built_instrument() {
    let (server, song) = build_server();
    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } },
                { "tool": "build_instrument", "params": { "instruments": [{
                    "name": "Half",
                    "modules": [{ "module_type": "osc", "params": { "not_a_real_param": 1.0 } }]
                }] } }
            ],
            "rollback": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(
        v["results"][1]["structured"]["items"][0]["partial_success"], true,
        "sanity: the payload itself says the patch is incomplete: {resp}"
    );
    assert_eq!(
        v["results"][1]["status"], "partial",
        "and the call's verdict has to say so too: {resp}"
    );
    assert_eq!(v["rolled_back"], true, "so all-or-nothing restores: {resp}");
    assert_eq!(song.read().name, "Original", "the restore really happened");
}

/// `stop_on_error` stops on an operation that did not finish, not only on one
/// that did nothing — and the operations it never reached are reported as
/// skipped rather than quietly missing from the count.
#[tokio::test]
async fn stop_on_error_halts_on_a_partial_operation() {
    let (server, _song) = build_server();
    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "rename_pattern", "params": { "items": [
                    { "pattern_id": 9999, "name": "No such pattern" }
                ] } },
                { "tool": "set_song_name", "params": { "name": "Never reached" } }
            ],
            "stop_on_error": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["total"], 2, "`total` counts what was requested: {resp}");
    assert_eq!(v["skipped"], 1, "the unreached op is visible: {resp}");
    assert_eq!(
        v["results"].as_array().map(Vec::len),
        Some(1),
        "and has no fabricated result: {resp}"
    );
}

#[tokio::test]
async fn rollback_commits_when_all_succeed() {
    let (server, song) = build_server();

    let resp = server
        .batch_execute_for_test(serde_json::json!({
            "operations": [
                { "tool": "set_song_name", "params": { "name": "Changed" } }
            ],
            "rollback": true
        }))
        .await;

    let v: serde_json::Value = serde_json::from_str(&resp).expect("batch result is JSON");
    assert_eq!(v["rolled_back"], false);
    assert_eq!(v["failed"], 0);
    // No failure → the change sticks.
    assert_eq!(song.read().name, "Changed");
}
