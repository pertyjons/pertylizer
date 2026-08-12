//! Integration tests for the GUI project-snapshot request queue behind the MCP
//! save path.
//!
//! An MCP `save_project` used to rebuild the project from engine state alone,
//! silently flattening everything that lives only in the GUI (module
//! positions, groups, canvas size, visualizer modules). The bridge now asks
//! the GUI for the project *it* would save, over a request queue in
//! `McpSharedState`, and only falls back to engine reconstruction when no GUI
//! answers. These tests drive both sides of that seam with a simulated GUI
//! responder; the request/reply lifecycle itself (concurrency, timeout, late
//! replies) is unit-tested in `mcp_shared`.

#![cfg(feature = "mcp")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use synth_engine::SynthEngine;
use synth_mcp::bridge::ProjectBridge;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::project::{GlobalProjectState, ProjectFile};
use pertylizer::session::SynthSession;

fn build_bridge() -> (AppSynthBridge, Arc<McpSharedState>) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("EngineBuilt")));
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
    (bridge, shared)
}

/// A project that could only have come from the simulated GUI responder.
fn gui_marker_project() -> ProjectFile {
    ProjectFile::new(
        Vec::new(),
        0,
        None,
        Song::new("FROM-GUI"),
        GlobalProjectState::default(),
    )
}

/// A background thread standing in for the GUI frame loop: services snapshot
/// requests until told to stop.
struct FakeGui {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeGui {
    fn spawn(shared: &Arc<McpSharedState>) -> Self {
        shared.attach_gui_once(|| Box::new(|| {}));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = Arc::clone(shared);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    shared.service_gui_project_requests(gui_marker_project);
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeGui {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// With a GUI attached and answering, an MCP save writes the GUI's own project
/// build — the file on disk is the responder's project, not an engine
/// reconstruction.
#[test]
fn an_mcp_save_persists_the_gui_built_project() {
    let (bridge, shared) = build_bridge();
    let _gui = FakeGui::spawn(&shared);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("layout.ptz");
    bridge
        .save_project(path.to_str().expect("utf-8 temp path"))
        .expect("save should succeed");

    let saved = ProjectFile::load(&path).expect("saved project should load");
    assert_eq!(
        saved.song.name, "FROM-GUI",
        "the MCP save must persist the GUI-answered project, not an engine rebuild"
    );
}

/// With no GUI attached (headless server, tests), the save still succeeds via
/// engine reconstruction — the fallback the render CLI and every headless
/// caller depends on.
#[test]
fn a_headless_mcp_save_falls_back_to_the_engine_build() {
    let (bridge, _shared) = build_bridge();

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("headless.ptz");
    bridge
        .save_project(path.to_str().expect("utf-8 temp path"))
        .expect("headless save should succeed");

    let saved = ProjectFile::load(&path).expect("saved project should load");
    assert_eq!(
        saved.song.name, "EngineBuilt",
        "with no GUI attached the engine-built project is written"
    );
}
