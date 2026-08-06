//! Loading a saved project into an engine with no audio device and no GUI.
//!
//! The offline renderer reads the *live* engine's instrument snapshots, so a
//! project has to be really loaded — commands sent, drained, and mirrored into
//! shared state — before anything can be rendered. In the application that
//! happens because the audio callback runs concurrently with the load. Here
//! there is no callback, so this module supplies one: the load runs on a worker
//! thread while the caller's thread drives `SynthEngine::process` in a tight
//! loop.
//!
//! That ordering is not just convenience. The engine's command ring is bounded
//! and a push onto a full ring is *dropped*, so a large project loaded against
//! a stalled engine can silently lose commands. Draining concurrently — with no
//! real-time sleep between blocks, unlike the null audio backend — keeps the
//! ring far from full. The load is then complete when the engine has drained as
//! many commands as the loader enqueued, which
//! [`CommandSync`](synth_engine::shared_state::CommandSync) reports exactly, so
//! nothing here waits on a timer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::SynthEngine;
use synth_sequencer::{SharedSong, Song};

use super::RenderError;
use crate::audio::preview::SharedSampleLibrary;
use crate::session::SynthSession;

/// Block size the load-drain loop runs at. Only the drain rate depends on it;
/// the offline render builds its own engine at its own sample rate.
const DRAIN_FRAMES: usize = 256;

/// Sample rate for the same throwaway stream.
const DRAIN_SAMPLE_RATE: u32 = 44_100;

/// Blocks processed after the queue is empty, so work a command defers to the
/// next block (voice teardown, graph rebuilds) also completes.
const SETTLE_BLOCKS: usize = 32;

/// How long the load may take before it is called stuck.
///
/// Wall-clock rather than a block count: the loop spins at whatever rate the
/// machine manages, so a block budget would be a limit on CPU speed rather than
/// on progress, and would fail a large project on a fast machine while letting
/// a hung one spin on a slow one. Generous, because it is a deadlock backstop
/// and not a performance target.
const LOAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// A loaded project held in a live-but-silent engine, ready to render.
///
/// The engine is kept alive for exactly as long as this value: the offline
/// renderer snapshots its instruments each time it builds a render session, so
/// dropping it early would leave nothing to render.
pub struct LoadedProject {
    /// Never read: held only so the loaded instrument state stays alive. The
    /// offline renderer builds its *own* engine from `session`'s snapshots, so
    /// dropping this one early would leave nothing to snapshot.
    _engine: SynthEngine,
    /// Kept so the command channel behind `session` stays connected.
    _handle: synth_engine::EngineHandle,
    /// Module lifecycle owner the offline renderer snapshots.
    pub session: Arc<SynthSession>,
    /// The loaded song. Mutate this to apply a mix selection.
    pub song: Arc<SharedSong>,
    /// Samples the project's instruments reference.
    pub sample_library: SharedSampleLibrary,
    /// The loader's own summary line (instrument counts, skipped modules, …).
    pub summary: String,
}

/// The callback context the drain loop pretends to be called with.
fn drain_context() -> AudioCallbackContext {
    AudioCallbackContext {
        sample_rate: synth_core::audio::DeviceSampleRate::new(DRAIN_SAMPLE_RATE),
        frames: DRAIN_FRAMES,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    }
}

/// Load a project, bundle, or patch file into a fresh headless engine.
///
/// # Errors
///
/// Returns [`RenderError::ProjectLoad`] if the file is missing, cannot be
/// parsed, or the loader rejects it, and [`RenderError::LoadDidNotSettle`] if
/// the engine never finishes draining what the load enqueued.
pub fn load_project_file(path: &Path) -> Result<LoadedProject, RenderError> {
    let (mut engine, handle) = SynthEngine::new();
    engine.on_stream_start(&synth_core::StreamInfo {
        sample_rate: synth_core::audio::DeviceSampleRate::new(DRAIN_SAMPLE_RATE),
        buffer_size: synth_core::BufferSize::new(DRAIN_FRAMES as u32),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    });

    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let song = synth_engine::shared_song(Song::new("Untitled"));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    // Attach the shared song before the load, exactly as the application does,
    // so the sequencer and the loader are talking about the same instance.
    handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });

    let command_sync = Arc::clone(&handle.state.command_sync);
    let loader = {
        let load_path = path.to_path_buf();
        let load_session = Arc::clone(&session);
        let load_song = Arc::clone(&song);
        let load_library = Arc::clone(&sample_library);
        std::thread::Builder::new()
            .name("render-load".to_string())
            .spawn(move || {
                crate::project_apply::load_file_into_engine(
                    &load_path,
                    &load_session,
                    &load_song,
                    &load_library,
                )
            })
            .map_err(|source| RenderError::ProjectLoad {
                path: path.to_path_buf(),
                message: format!("cannot start the loader thread: {source}"),
            })?
    };

    let mut buffer = vec![0.0_f32; DRAIN_FRAMES * 2];
    let context = drain_context();
    let deadline = std::time::Instant::now() + LOAD_DEADLINE;
    // Two conditions, in this order: the loader must have stopped enqueueing
    // before "everything enqueued has been drained" means anything.
    while !loader.is_finished() || command_sync.processed() < command_sync.enqueued() {
        buffer.fill(0.0);
        engine.process(&mut buffer, &context);
        if std::time::Instant::now() > deadline {
            return Err(RenderError::LoadDidNotSettle {
                path: path.to_path_buf(),
                enqueued: command_sync.enqueued(),
                processed: command_sync.processed(),
            });
        }
    }

    let summary = loader
        .join()
        .map_err(|_| RenderError::ProjectLoad {
            path: path.to_path_buf(),
            message: "the loader thread panicked".to_string(),
        })?
        .map_err(|message| RenderError::ProjectLoad {
            path: path.to_path_buf(),
            message,
        })?;

    for _ in 0..SETTLE_BLOCKS {
        buffer.fill(0.0);
        engine.process(&mut buffer, &context);
    }

    Ok(LoadedProject {
        _engine: engine,
        _handle: handle,
        session,
        song,
        sample_library,
        summary,
    })
}

/// Absolute form of `path` when it can be resolved, else `path` unchanged.
///
/// Used for the receipt, where a relative path would not identify the file to
/// anyone reading it from elsewhere.
#[must_use]
pub fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Identity of `path` for "are these two arguments the same file?" checks,
/// defined for files that do not exist yet.
///
/// [`absolute`] cannot answer that question for an output: `canonicalize`
/// fails on a path with no file behind it, so it falls back to the argument
/// verbatim and `out.wav` compares unequal to `./out.wav`. Both would then be
/// written, one over the other. Canonicalizing the *directory* and re-joining
/// the file name resolves `.`, `..`, and symlinked directories before the
/// comparison, which is what makes the guard hold for a file that is about to
/// be created.
#[must_use]
pub fn path_identity(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match (std::fs::canonicalize(parent), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        // Neither the file nor its directory exists — nothing better to compare
        // than what was typed. The write will fail on its own terms anyway.
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard against `--output` and `--result-json` naming one file has to
    /// hold before either exists, which is exactly when `canonicalize` cannot
    /// help.
    #[test]
    fn spellings_of_one_not_yet_existing_file_share_an_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = dir.path().join("out.wav");
        let dotted = dir.path().join(".").join("out.wav");
        let round_trip = dir.path().join("sub").join("..").join("out.wav");
        std::fs::create_dir(dir.path().join("sub")).expect("subdir");

        assert!(!plain.exists(), "the point is that it does not exist yet");
        assert_eq!(path_identity(&plain), path_identity(&dotted));
        assert_eq!(path_identity(&plain), path_identity(&round_trip));
    }

    /// Different files must stay different, or the guard would refuse valid
    /// invocations.
    #[test]
    fn distinct_files_keep_distinct_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_ne!(
            path_identity(&dir.path().join("a.wav")),
            path_identity(&dir.path().join("b.wav"))
        );
    }
}
