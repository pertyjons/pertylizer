//! Debounced autosave and the startup recovery offer.
//!
//! Autosave never writes to the user's project file — see [`crate::recovery`]
//! for why. This module is only the scheduling and the wiring into the app
//! shell: when to take a snapshot, when to retire one, and what to do on
//! failure.

use std::time::{Duration, Instant, SystemTime};

use super::SynthApp;
use crate::recovery::{RecoveryEntry, RecoveryStore};

/// Minimum gap between snapshot attempts.
///
/// Long enough that a burst of edits (dragging a fader, typing notes) produces
/// one snapshot rather than dozens, short enough that a crash costs at most
/// half a minute of work.
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Scheduling state for the autosave tick.
#[derive(Default)]
pub(super) struct AutosaveState {
    /// The recovery directory, or `None` if it could not be opened — in which
    /// case autosave is simply off. A machine with no writable data directory
    /// should still be able to run the synth.
    store: Option<RecoveryStore>,
    /// When a snapshot was last attempted, successful or not. Failures update
    /// this too, so a persistently failing write retries on the normal interval
    /// rather than on every frame.
    last_attempt: Option<Instant>,
    /// The project state the last successful snapshot captured. Lets an idle
    /// session skip rewriting an identical snapshot every interval.
    snapshotted: Option<crate::dirty::ProjectRevision>,
    /// The project path the snapshot on disk is keyed by, which is *not*
    /// necessarily the current one: the store keys a snapshot by the path the
    /// project had when it was taken, and Save As — or the `.ptz`/`.ptzb`
    /// normalization a save performs when samples appear — moves the current
    /// path out from under it. Retiring by the current path would then leave the
    /// old snapshot behind, and the next launch would offer to recover work that
    /// has already been saved.
    snapshotted_path: Option<std::path::PathBuf>,
    /// Set once a failure has been reported, so a broken recovery directory
    /// does not repeat the same message into the status bar every interval.
    reported_failure: bool,
    /// A snapshot being written on a worker thread, with the state to commit
    /// once it lands.
    ///
    /// Writing happens off the UI thread because a sample-heavy project
    /// serializes to a ZIP with every sample embedded — doing that in the frame
    /// is a visible hitch every interval. At most one is in flight: a second
    /// would race the first for the same file.
    in_flight: Option<InFlightSnapshot>,
}

/// A recovery snapshot being written in the background.
struct InFlightSnapshot {
    /// Delivers the write's outcome exactly once.
    result: std::sync::mpsc::Receiver<Result<(), crate::recovery::RecoveryError>>,
    /// The project state this snapshot captured, recorded only if it succeeds.
    revision: crate::dirty::ProjectRevision,
    /// The path it was keyed by, needed to retire it later.
    path: Option<std::path::PathBuf>,
}

impl AutosaveState {
    /// Open the recovery store. A failure disables autosave for the session and
    /// is logged, not surfaced — nothing the user did caused it and there is
    /// nothing for them to do about it mid-session.
    pub(super) fn new() -> Self {
        let store = match RecoveryStore::open() {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!(
                    target: "pertylizer::recovery",
                    error = %e,
                    "autosave disabled: no usable recovery directory",
                );
                None
            }
        };
        Self {
            store,
            ..Self::default()
        }
    }
}

impl SynthApp {
    /// Take a recovery snapshot if enough time has passed and there is
    /// something new to protect. Called once per frame.
    pub(super) fn tick_autosave(&mut self) {
        if self.autosave.store.is_none() || !self.is_dirty() {
            return;
        }
        // A pending recovery offer means the snapshot on disk is the previous
        // session's unsaved work. Snapshotting now would overwrite it with the
        // untouched document the user is looking at — destroying exactly what
        // we are about to offer them.
        if self.recovery_prompt.is_some() {
            return;
        }

        // One write at a time: a second would race the first for the same
        // file, and the interval is far longer than any write should take.
        if self.autosave.in_flight.is_some() {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.autosave.last_attempt
            && now.duration_since(last) < AUTOSAVE_INTERVAL
        {
            return;
        }

        // Nothing has changed since the last snapshot, so the one on disk is
        // already an accurate copy. (The project is still dirty relative to the
        // *manual* save — that is a different baseline.)
        let revision = self.current_revision();
        if self.autosave.snapshotted == Some(revision) {
            return;
        }

        self.autosave.last_attempt = Some(now);
        self.write_recovery_snapshot(revision);
    }

    /// Capture the project on the UI thread and write it on a worker.
    ///
    /// Only the capture has to happen here — it reads shared state the UI
    /// thread owns. Serialization and the file write, which for a sample-heavy
    /// project means a ZIP with every sample in it, go to a worker so the frame
    /// does not stall.
    fn write_recovery_snapshot(&mut self, revision: crate::dirty::ProjectRevision) {
        let Some(store) = self.autosave.store.clone() else {
            return;
        };
        let project = self.create_project_from_app();
        let name = self.project_display_name();
        let path = self.current_project_path.clone();
        // Snapshot the samples too, or recovering a sampler project would hand
        // back a song whose audio is gone. Cloning the library is cheap: the
        // audio is `Arc`-shared, so this copies metadata and bumps refcounts.
        let library = self.sample_library.read().ok().map(|lib| lib.clone());

        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        std::thread::Builder::new()
            .name("pertylizer-autosave".to_string())
            .spawn(move || {
                let outcome = store
                    .write_snapshot(
                        &project,
                        library.as_ref(),
                        worker_path.as_deref(),
                        &name,
                        SystemTime::now(),
                    )
                    .map(|_| ());
                // A closed receiver means the app is shutting down; the
                // snapshot is written either way, which is the point.
                let _ = sender.send(outcome);
            })
            .map_or_else(
                |e| {
                    tracing::warn!(
                        target: "pertylizer::recovery",
                        error = %e,
                        "could not spawn the autosave worker",
                    );
                },
                |_handle| {
                    self.autosave.in_flight = Some(InFlightSnapshot {
                        result: receiver,
                        revision,
                        path,
                    });
                },
            );
    }

    /// Commit a finished background snapshot. Called once per frame.
    pub(super) fn poll_autosave(&mut self) {
        let Some(in_flight) = &self.autosave.in_flight else {
            return;
        };
        let outcome = match in_flight.result.try_recv() {
            Ok(outcome) => outcome,
            // Still writing.
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            // The worker died without reporting; treat it as a failure so the
            // slot is freed and the next interval retries.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(
                crate::recovery::RecoveryError::Write("the autosave worker stopped".to_string()),
            ),
        };
        let Some(in_flight) = self.autosave.in_flight.take() else {
            return;
        };

        match outcome {
            Ok(()) => {
                self.autosave.snapshotted = Some(in_flight.revision);
                self.autosave.snapshotted_path = in_flight.path;
                self.autosave.reported_failure = false;
                tracing::debug!(target: "pertylizer::recovery", "recovery snapshot written");
            }
            Err(e) => {
                // Deliberately does not touch dirty state: work that could not
                // be snapshotted is *more* at risk, not less.
                tracing::warn!(
                    target: "pertylizer::recovery",
                    error = %e,
                    "recovery snapshot failed",
                );
                if !self.autosave.reported_failure {
                    self.autosave.reported_failure = true;
                    self.dialog_state
                        .set_status(format!("Autosave failed (your work is unsaved): {e}"));
                }
            }
        }
    }

    /// Drop the recovery snapshot for the current project.
    ///
    /// Called when the work it protected is no longer at risk — after a
    /// successful manual save, and when the user chooses to discard changes.
    /// Leaving it would make the next launch offer to recover work the user has
    /// already saved or deliberately thrown away.
    /// Retires the snapshot under the path it was *written* with as well as the
    /// current one — Save As and the bundle-extension normalization both move
    /// the current path after the snapshot was keyed.
    pub(super) fn retire_recovery_snapshot(&mut self) {
        // While an offer is pending, the snapshot on disk is the previous
        // session's unsaved work and nobody has answered for it yet. Starting a
        // new project or saving this one behind the prompt must not delete it —
        // the same reason `tick_autosave` refuses to overwrite it.
        if self.recovery_prompt.is_some() {
            return;
        }
        if let Some(store) = &self.autosave.store {
            store.retire(self.current_project_path.as_deref());
            if self.autosave.snapshotted_path != self.current_project_path {
                store.retire(self.autosave.snapshotted_path.as_deref());
            }
        }
        self.autosave.snapshotted = None;
        self.autosave.snapshotted_path = None;
    }

    /// A name for the project, for the recovery prompt to show.
    fn project_display_name(&self) -> String {
        let song_name = self.song.read().name.clone();
        if !song_name.is_empty() {
            return song_name;
        }
        self.current_project_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string()
    }

    /// Look for work left behind by a previous run and, if there is any, arm
    /// the recovery prompt.
    pub(super) fn check_for_recoverable_work(&mut self) {
        let Some(store) = &self.autosave.store else {
            return;
        };
        if let Some(entry) = store.find_recoverable(SystemTime::now()) {
            tracing::info!(
                target: "pertylizer::recovery",
                project = %entry.meta.project_name,
                "found recoverable work from a previous session",
            );
            self.recovery_prompt = Some(entry);
        }
    }

    /// Load a recovered snapshot as an **unsaved** document.
    ///
    /// The project path is restored so a later save goes back to the right
    /// file, but the clean baseline deliberately is not: the recovered content
    /// is by definition not what is on disk, so it must still be marked as
    /// having unsaved changes.
    pub(super) fn accept_recovery(&mut self, entry: &RecoveryEntry) {
        // A snapshot of a project with samples is a bundle, and has to come back
        // through the bundle loader or every sample is dropped on the floor.
        let loaded = if entry.load_is_bundle() {
            self.load_bundle_file(&entry.snapshot_path)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            entry
                .load()
                .map(|project| self.load_project_data(project))
                .map_err(|e| e.to_string())
        };

        match loaded {
            Ok(()) => {
                self.current_project_path = entry.meta.project_path.clone();
                // The load re-applied the project; mark it unsaved, because
                // recovered work has not been saved anywhere the user can see.
                self.mark_dirty();
                self.dialog_state.set_status(format!(
                    "Recovered unsaved work from '{}' — save to keep it",
                    entry.meta.project_name,
                ));
            }
            Err(e) => {
                self.dialog_state
                    .set_status(format!("Could not recover: {e}"));
            }
        }
    }

    /// Discard the offered snapshot at the user's request.
    pub(super) fn decline_recovery(&mut self, entry: &RecoveryEntry) {
        if let Some(store) = &self.autosave.store {
            store.retire(entry.meta.project_path.as_deref());
            // An untitled snapshot is keyed by a fixed id rather than a path,
            // so `retire` cannot find it from the metadata alone.
            let _ = std::fs::remove_file(&entry.snapshot_path);
            let _ = std::fs::remove_file(entry.snapshot_path.with_extension("json"));
        }
    }

    /// Ask whether to restore work found from a previous session.
    ///
    /// Shown modally at startup because the answer decides what document the
    /// user is about to work in — continuing behind it would mean editing the
    /// wrong one.
    pub(super) fn show_recovery_prompt(&mut self, ctx: &egui::Context) {
        let Some(entry) = self.recovery_prompt.clone() else {
            return;
        };
        let mut decision: Option<bool> = None;

        egui::Window::new("Recover Unsaved Work")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Pertylizer closed with unsaved changes to '{}'.",
                    entry.meta.project_name,
                ));
                ui.label("Restore them?");
                ui.add_space(super::theme().spacing.md);
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        decision = Some(false);
                    }
                });
            });

        match decision {
            Some(true) => {
                self.accept_recovery(&entry);
                self.recovery_prompt = None;
            }
            Some(false) => {
                self.decline_recovery(&entry);
                self.recovery_prompt = None;
            }
            None => {}
        }
    }
}
