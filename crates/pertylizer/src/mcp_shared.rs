//! Shared state between the MCP server and the GUI.
//!
//! `McpSharedState` is created in `main.rs` and shared via `Arc` to both
//! the `AppSynthBridge` (MCP side) and `SynthApp` (GUI side).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::patch::{Author, Patch};
use crate::project::ProjectFile;
use synth_mcp::McpSessionRegistry;
use synth_sequencer::Song;

/// UI-refresh notification produced when MCP (or any non-GUI caller)
/// applies a project to the engine. The GUI consumes this on its next
/// frame to rebuild its UI mirrors via `refresh_ui_from_project` /
/// `refresh_ui_after_reset`. Stays `None` after a save — saves don't
/// change engine state so the UI has nothing to refresh.
pub enum ProjectRefresh {
    /// A project was loaded — the GUI must rebuild every UI mirror
    /// (instruments, patch editor canvases, keyboard octave, …).
    Loaded(Box<ProjectFile>),
    /// The project was reset to empty — equivalent to loading an empty
    /// `ProjectFile`. The GUI rebuilds its mirrors against the empty
    /// state.
    Reset,
}

/// Captured mix-bus baseline for `compare_mix_before_after`. Held in-session
/// (not persisted to the project) so a later `compare` call can A/B against it.
/// The render settings are stored alongside the metrics so the comparison
/// re-renders the same window + signal chain.
#[derive(Clone)]
pub struct MixBaseline {
    /// Caller-supplied label identifying what the baseline represents.
    pub label: String,
    /// Mix-bus metrics captured at baseline time.
    pub metrics: synth_mcp::MixBusMetrics,
    /// Render duration the baseline was captured with.
    pub duration_seconds: f32,
    /// Start tick the baseline was captured from.
    pub start_tick: Option<u64>,
    /// Signal-chain scope the baseline was captured with (reused on compare).
    pub scope: synth_core::AnalysisScope,
}

/// Shared state for communication between MCP bridge and GUI.
pub struct McpSharedState {
    /// Patch queued for loading by MCP (consumed by GUI each frame).
    pub pending_patch: Mutex<Option<(Patch, String)>>,
    /// Current UI layout snapshot (written by GUI, read by MCP).
    pub ui_layout: Mutex<UiLayoutData>,
    /// Shared song data for sequencer (read/written by MCP, read by engine).
    pub song: Arc<synth_sequencer::SharedSong>,
    /// Whether the MCP HTTP server is listening.
    pub mcp_listening: AtomicBool,
    /// Registry of active MCP sessions with client identity info.
    pub mcp_sessions: McpSessionRegistry,
    /// Auto-layout requested by MCP (consumed by GUI each frame).
    ///
    /// Stays a plain `AtomicBool` rather than riding `gui_revision`
    /// because auto-layout has view-gated latching semantics: a request
    /// issued while the user is in a non-Rack view must wait until the
    /// Rack view is drawn before it consumes the flag. A one-shot
    /// revision bump would either fire too early or race with the
    /// view switch.
    pub pending_auto_layout: AtomicBool,
    /// Serializes concurrent project I/O (save vs. load, two MCP clients
    /// racing). Held for the duration of one apply.
    pub project_io_lock: parking_lot::Mutex<()>,
    /// Bumped on every successful project apply (load / reset). The GUI
    /// observes increments to detect "something changed; refresh".
    pub project_revision: AtomicU64,
    /// The globals a project carries that live only in the GUI, published for the
    /// non-GUI save and snapshot paths to read. `None` until the GUI publishes
    /// (headless runs never do).
    ///
    /// Without this, an MCP `save_project` wrote `glide_time: 0` and
    /// `octave_offset: 0` over whatever the user had set — the fields simply were
    /// not reachable from here — and a rollback restored those zeros.
    pub gui_globals: Mutex<Option<GuiGlobals>>,
    /// Bumped once per mutating MCP tool call, across every session sharing this
    /// state.
    ///
    /// `batch_execute`'s rollback exists to undo *its own* writes. It restores a
    /// whole-project snapshot, so anything another client wrote after the snapshot
    /// was taken would be undone along with it — silently, and with the other
    /// client having been told its write succeeded. Comparing this counter tells a
    /// rollback whether the state it is about to overwrite is still the state its
    /// own operations produced.
    pub mutation_seq: AtomicU64,
    /// Bumped whenever MCP writes a one-shot GUI-mirror payload
    /// (`pending_patch`). Same fast-path pattern as
    /// `project_revision`: the GUI checks this atomic at the top of each
    /// frame and only locks the slot mutexes when it actually changed,
    /// so idle frames pay only one `Acquire` load instead of two mutex
    /// acquisitions.
    pub gui_revision: AtomicU64,
    /// UI-refresh queue populated by MCP/non-GUI loads, drained by the
    /// GUI on each frame. `None` between events.
    pub pending_project_refresh: Mutex<Option<ProjectRefresh>>,
    /// Path of the most recently loaded project, for the GUI title bar
    /// and "Save" → existing-path detection. Cleared on `new_project`.
    pub last_loaded_project_path: Mutex<Option<PathBuf>>,
    /// Result of the most recent project I/O op (load / save / new) so
    /// the GUI can surface errors in the status line. `None` before any
    /// op has run.
    pub last_project_io_status: Mutex<Option<Result<String, String>>>,
    /// Project author / composer metadata. Both GUI ("Project → Edit
    /// metadata…") and MCP (future `set_project_author`) read and write
    /// this directly. `None` when not set — file-save then emits a
    /// missing `author` field.
    pub author: Arc<Mutex<Option<Author>>>,
    /// In-session mix-bus baseline captured by `compare_mix_before_after`.
    /// `None` until a `capture` call stores one; a later `compare` call reads
    /// it to compute deltas. Transient — never written to the project file.
    pub mix_baseline: Mutex<Option<MixBaseline>>,
    /// Queue of waiting "give me the project the GUI would save" requests,
    /// each carrying its own one-shot reply channel. Non-GUI save and
    /// rollback-snapshot paths enqueue here so the persisted project carries
    /// the per-instrument UI metadata (module positions, groups, canvas size,
    /// visualizer modules) that an engine reconstruction cannot know.
    ///
    /// A queue rather than a single slot: two simultaneous callers must not
    /// consume each other's project, and a caller that timed out must not
    /// leave a stale project behind for the next request to mistake as its
    /// own. See [`Self::request_gui_project`] /
    /// [`Self::service_gui_project_requests`].
    gui_project_requests: Mutex<Vec<mpsc::SyncSender<Box<ProjectFile>>>>,
    /// Idle-frame fast path for the queue above: the GUI checks this atomic
    /// once per frame and only locks the queue mutex when a request is
    /// actually waiting (same pattern as `project_revision` / `gui_revision`).
    gui_project_requests_pending: AtomicBool,
    /// True once a GUI has attached to this shared state. When false
    /// (headless MCP server, render CLI, integration tests), nothing will
    /// ever answer a project request, so requesters skip the bounded wait
    /// and fall back to engine reconstruction immediately.
    gui_attached: AtomicBool,
    /// Wakes the GUI event loop so a queued project request is serviced on
    /// the next frame even when the window is unfocused. Installed by the
    /// GUI at attach time (an `egui::Context::request_repaint` closure).
    gui_repaint: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl McpSharedState {
    #[must_use]
    pub fn new() -> Self {
        Self::with_song(synth_engine::shared_song(Song::new("Untitled")))
    }

    /// Create with a pre-existing shared Song (so GUI and MCP share the same instance).
    #[must_use]
    pub fn with_song(song: Arc<synth_sequencer::SharedSong>) -> Self {
        Self {
            pending_patch: Mutex::new(None),
            ui_layout: Mutex::new(UiLayoutData::default()),
            gui_globals: Mutex::new(None),
            mutation_seq: AtomicU64::new(0),
            song,
            mcp_listening: AtomicBool::new(false),
            mcp_sessions: McpSessionRegistry::new(),
            pending_auto_layout: AtomicBool::new(false),
            project_io_lock: parking_lot::Mutex::new(()),
            project_revision: AtomicU64::new(0),
            gui_revision: AtomicU64::new(0),
            pending_project_refresh: Mutex::new(None),
            last_loaded_project_path: Mutex::new(None),
            last_project_io_status: Mutex::new(None),
            author: Arc::new(Mutex::new(None)),
            mix_baseline: Mutex::new(None),
            gui_project_requests: Mutex::new(Vec::new()),
            gui_project_requests_pending: AtomicBool::new(false),
            gui_attached: AtomicBool::new(false),
            gui_repaint: Mutex::new(None),
        }
    }

    /// Mark this shared state as having a live GUI and install its wake-up
    /// hook. Cheap to call every frame: after the first call it is a single
    /// atomic load, and `make_repaint` is never invoked again.
    pub fn attach_gui_once(&self, make_repaint: impl FnOnce() -> Box<dyn Fn() + Send + Sync>) {
        if self.gui_attached.load(Ordering::Acquire) {
            return;
        }
        *self.gui_repaint.lock().unwrap_or_else(|e| e.into_inner()) = Some(make_repaint());
        self.gui_attached.store(true, Ordering::Release);
    }

    /// Mark the GUI as gone again (eframe `on_exit`). Restores the headless
    /// fast path — later requests decline immediately instead of burning the
    /// full timeout against an event loop that will never frame — and wakes
    /// any requester already waiting (clearing the queue drops its reply
    /// sender, which disconnects its `recv_timeout` right away).
    pub fn detach_gui(&self) {
        self.gui_attached.store(false, Ordering::Release);
        *self.gui_repaint.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.gui_project_requests_pending
            .store(false, Ordering::Release);
        self.gui_project_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// True once a GUI has attached (and not detached). A `false` from
    /// `request_gui_project` combined with `true` here means the GUI failed
    /// to answer in time — the caller may want to log that its fallback
    /// degrades fidelity, which the headless fallback does not.
    #[must_use]
    pub fn is_gui_attached(&self) -> bool {
        self.gui_attached.load(Ordering::Acquire)
    }

    /// True when at least one project-snapshot request is waiting. GUI-side
    /// peek so the frame can re-reconcile its mirrors with the session
    /// *before* running the build in [`Self::service_gui_project_requests`]
    /// (a request that arrived mid-frame would otherwise be answered from
    /// mirrors reconciled at the top of the frame).
    #[must_use]
    pub fn has_pending_gui_project_requests(&self) -> bool {
        self.gui_project_requests_pending.load(Ordering::Acquire)
    }

    /// Ask the GUI for the project it would itself save (engine state plus the
    /// UI metadata overlay), waiting at most `timeout` for an answer.
    ///
    /// Returns `None` — and the caller falls back to engine reconstruction —
    /// when no GUI has attached (headless), or when the attached GUI does not
    /// answer in time (minimised and not repainting, wedged, shutting down).
    /// The GUI is woken via the repaint hook so an unfocused window still
    /// services the request on its next frame.
    ///
    /// The caller's timeout must exceed the engine snapshot barrier inside the
    /// GUI's own builder (`project_apply`'s command-drain wait): the GUI may
    /// legitimately spend that entire budget before replying, and an equal
    /// nested deadline would turn every such save into a false fallback.
    pub fn request_gui_project(&self, timeout: Duration) -> Option<Box<ProjectFile>> {
        if !self.gui_attached.load(Ordering::Acquire) {
            return None;
        }
        let (reply, receive) = mpsc::sync_channel(1);
        self.gui_project_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(reply);
        // Push before set: `service_gui_project_requests` clears the flag
        // before draining, so a request whose flag-set is observed always has
        // its channel in the queue by then.
        self.gui_project_requests_pending
            .store(true, Ordering::Release);
        // Re-check *after* the push, or the entry-check above races
        // `detach_gui`: a requester that read `true` just before the GUI went
        // away would push into the queue detach had already cleared and then
        // block for the whole timeout — precisely the shutdown stall
        // `detach_gui` exists to prevent. Both the push and detach's `clear()`
        // run under the queue mutex, so they are ordered: either detach
        // cleared after our push (our sender is dropped and `recv_timeout`
        // wakes at once), or it cleared before it — in which case its
        // `gui_attached` store happened before this load and we bail here.
        if !self.gui_attached.load(Ordering::Acquire) {
            return None;
        }
        if let Some(repaint) = self
            .gui_repaint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            repaint();
        }
        receive.recv_timeout(timeout).ok()
    }

    /// GUI-side, once per frame: answer every queued project request.
    ///
    /// `build` runs at most once per call, and only when a requester is
    /// actually waiting; its result is moved to the last waiter and cloned
    /// only for any earlier ones (so the common single-requester case pays no
    /// clone). Sending to a requester that already timed out (and dropped its
    /// receiver) fails harmlessly. Call this **after** the frame has
    /// reconciled GUI mirrors with the session, so a module just added over
    /// MCP is present in the answered project.
    pub fn service_gui_project_requests(&self, build: impl FnOnce() -> ProjectFile) {
        // Clear the flag before draining: requesters push their channel first
        // and set the flag second, so any push this drain misses has its
        // flag-set ordered after this clear and is picked up next frame.
        if !self
            .gui_project_requests_pending
            .swap(false, Ordering::AcqRel)
        {
            return;
        }
        let mut waiters = std::mem::take(
            &mut *self
                .gui_project_requests
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let Some(last) = waiters.pop() else {
            return;
        };
        let project = Box::new(build());
        for waiter in waiters {
            let _ = waiter.try_send(project.clone());
        }
        let _ = last.try_send(project);
    }

    /// Check if the MCP server is listening.
    pub fn is_listening(&self) -> bool {
        self.mcp_listening.load(Ordering::Relaxed)
    }

    /// Get the number of active MCP sessions.
    pub fn active_sessions(&self) -> usize {
        self.mcp_sessions.active_count()
    }
}

impl Default for McpSharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of the current UI layout (positions, sizes, parameters).
#[derive(Debug, Clone, Default)]
pub struct UiLayoutData {
    /// Name of the currently loaded patch.
    pub patch_name: String,
    /// All modules with their visual state.
    pub modules: Vec<ModuleLayout>,
    /// All connections between modules.
    pub connections: Vec<ConnectionLayout>,
    /// Window size (width, height).
    pub window_size: (f32, f32),
}

/// Project-level state that exists only in the GUI.
///
/// The engine holds instruments, modules and the song; these three are the
/// GUI's, and a project file carries them. Anything building a project outside
/// the GUI has to be told them or it writes defaults.
#[derive(Debug, Clone, Copy)]
pub struct GuiGlobals {
    /// Global glide / portamento time.
    pub glide_time: synth_core::Seconds,
    /// Keyboard octave offset.
    pub octave_offset: i32,
    /// The instrument the GUI has selected, which a project reopens on.
    pub active_instrument_id: Option<synth_core::InstrumentId>,
}

/// Visual state of a single module in the UI.
#[derive(Debug, Clone)]
pub struct ModuleLayout {
    /// Module ID string (e.g. "osc-1").
    pub id: String,
    /// Module type name (e.g. "Oscillator").
    pub module_type: String,
    /// Human-readable display name (e.g. "Osc 1").
    pub name: String,
    /// Position in the workspace (x, y).
    pub position: (f32, f32),
    /// Rendered size (width, height).
    pub size: (f32, f32),
    /// Parameters as (name, display_value) pairs.
    pub parameters: Vec<(String, String)>,
}

/// A connection between two module ports.
#[derive(Debug, Clone)]
pub struct ConnectionLayout {
    /// Source module ID.
    pub from_module: String,
    /// Source port name.
    pub from_port: String,
    /// Destination module ID.
    pub to_module: String,
    /// Destination port name.
    pub to_port: String,
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// An empty project whose song name marks where it was built, so a test
    /// can tell a GUI-answered project from an engine fallback.
    fn marker_project(name: &str) -> ProjectFile {
        ProjectFile::new(
            Vec::new(),
            0,
            None,
            synth_sequencer::Song::new(name),
            crate::project::GlobalProjectState::default(),
        )
    }

    fn attached() -> Arc<McpSharedState> {
        let shared = Arc::new(McpSharedState::new());
        shared.attach_gui_once(|| Box::new(|| {}));
        shared
    }

    /// Drive `service_gui_project_requests` like a GUI frame loop until every
    /// spawned requester has returned, then hand back their results.
    fn service_until_finished<T: Send + 'static>(
        shared: &Arc<McpSharedState>,
        requesters: Vec<std::thread::JoinHandle<T>>,
        build: impl Fn() -> ProjectFile,
    ) -> Vec<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while requesters.iter().any(|r| !r.is_finished()) {
            assert!(
                Instant::now() < deadline,
                "requesters never finished — the reply channel is broken"
            );
            shared.service_gui_project_requests(&build);
            std::thread::sleep(Duration::from_millis(1));
        }
        requesters
            .into_iter()
            .map(|r| r.join().expect("requester thread panicked"))
            .collect()
    }

    /// The core round trip: a requester gets exactly the project the GUI-side
    /// service built.
    #[test]
    fn a_serviced_request_returns_the_gui_built_project() {
        let shared = attached();
        let s = Arc::clone(&shared);
        let requester = std::thread::spawn(move || s.request_gui_project(Duration::from_secs(10)));
        let mut results =
            service_until_finished(&shared, vec![requester], || marker_project("FROM-GUI"));
        let project = results
            .pop()
            .expect("one requester")
            .expect("request should be answered");
        assert_eq!(project.song.name, "FROM-GUI");
    }

    /// Two simultaneous callers each get their own reply — the failure mode a
    /// single shared slot had (one caller consuming the other's project).
    #[test]
    fn two_concurrent_requesters_each_get_a_reply() {
        let shared = attached();
        let requesters: Vec<_> = (0..2)
            .map(|_| {
                let s = Arc::clone(&shared);
                std::thread::spawn(move || s.request_gui_project(Duration::from_secs(10)))
            })
            .collect();
        let results = service_until_finished(&shared, requesters, || marker_project("SHARED"));
        assert_eq!(results.len(), 2);
        for result in results {
            let project = result.expect("both requests should be answered");
            assert_eq!(project.song.name, "SHARED");
        }
    }

    /// A timed-out request must not poison the queue: the late answer to its
    /// dead channel is discarded, and the next request gets a fresh reply
    /// rather than the stale project.
    #[test]
    fn a_late_answer_after_timeout_is_discarded() {
        let shared = attached();
        // Nobody services, so this times out.
        assert!(
            shared
                .request_gui_project(Duration::from_millis(20))
                .is_none()
        );
        // The late service finds the dead channel and must neither panic nor
        // leave anything behind.
        shared.service_gui_project_requests(|| marker_project("STALE"));

        let s = Arc::clone(&shared);
        let requester = std::thread::spawn(move || s.request_gui_project(Duration::from_secs(10)));
        let mut results =
            service_until_finished(&shared, vec![requester], || marker_project("FRESH"));
        let project = results
            .pop()
            .expect("one requester")
            .expect("second request should be answered");
        assert_eq!(project.song.name, "FRESH");
    }

    /// Headless: with no GUI attached the request declines immediately instead
    /// of burning the full timeout on a reply that can never come.
    #[test]
    fn an_unattached_request_declines_without_waiting() {
        let shared = Arc::new(McpSharedState::new());
        let start = Instant::now();
        assert!(
            shared
                .request_gui_project(Duration::from_secs(10))
                .is_none()
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "an unattached request must not wait for the timeout"
        );
    }

    /// The idle-frame path: with nothing queued, the service call must not
    /// invoke the (expensive) project builder.
    #[test]
    fn service_without_requests_does_not_build() {
        let shared = attached();
        shared
            .service_gui_project_requests(|| panic!("build closure must not run on an idle frame"));
    }

    /// Detach (window closed) must wake a requester already waiting — clearing
    /// the queue drops its reply sender, disconnecting its `recv_timeout` —
    /// and restore the immediate-decline fast path for later requests.
    #[test]
    fn detach_wakes_a_waiting_requester_and_restores_the_fast_path() {
        let shared = attached();
        let s = Arc::clone(&shared);
        let requester = std::thread::spawn(move || {
            let start = Instant::now();
            (
                s.request_gui_project(Duration::from_secs(30)),
                start.elapsed(),
            )
        });
        // Let the requester enqueue before pulling the GUI away.
        let enqueue_deadline = Instant::now() + Duration::from_secs(10);
        while !shared.has_pending_gui_project_requests() {
            assert!(
                Instant::now() < enqueue_deadline,
                "requester never enqueued"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        shared.detach_gui();
        let (result, waited) = requester.join().expect("requester thread panicked");
        assert!(result.is_none(), "a detached GUI cannot answer");
        assert!(
            waited < Duration::from_secs(10),
            "detach must wake the waiter instead of letting it burn the timeout"
        );

        let start = Instant::now();
        assert!(
            shared
                .request_gui_project(Duration::from_secs(10))
                .is_none()
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "after detach, requests must decline without waiting"
        );
    }

    /// The repaint hook installed at attach time is invoked per request, so a
    /// GUI that only repaints on demand still wakes up to answer.
    #[test]
    fn a_request_fires_the_repaint_hook() {
        let shared = Arc::new(McpSharedState::new());
        let repaints = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&repaints);
        shared.attach_gui_once(move || {
            Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            })
        });
        let _ = shared.request_gui_project(Duration::from_millis(10));
        assert_eq!(repaints.load(Ordering::Relaxed), 1);
    }
}
