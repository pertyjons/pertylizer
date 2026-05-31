//! Integration tests for `batch_execute`'s `dry_run` and `rollback` modes.
//!
//! `dry_run` must validate every operation without mutating any state.
//! `rollback` must snapshot the project before the batch and restore it if any
//! operation fails, so the batch is all-or-nothing.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
use synth_mcp::SynthMcpServer;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

/// Build a headless server sharing `song` so tests can read the live song state.
fn build_server() -> (SynthMcpServer, Arc<PlRwLock<Song>>) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("Original")));
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
    let bridge = AppSynthBridge::new(session, shared, sample_library);
    let server = SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>);
    (server, song)
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
        v["results"][0]["result"]
            .as_str()
            .unwrap()
            .contains("dry_run OK"),
        "valid op should report dry_run OK, got {}",
        v["results"][0]["result"]
    );
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
