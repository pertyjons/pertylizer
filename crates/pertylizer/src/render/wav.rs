//! Render one tick window of a song to a 32-bit float WAV.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use synth_core::Seconds;
use synth_mcp::AnalysisScope;
use synth_sequencer::{SharedSong, Tick};

use super::RenderError;
use crate::audio::arrangement_render::OfflineEngineSession;
use crate::audio::preview::SharedSampleLibrary;
use crate::session::SynthSession;

/// An absolute, non-empty song range: `start` inclusive, `end` exclusive.
///
/// Constructed only through [`TickWindow::new`] or
/// [`tick_window_from_seconds`], both of which reject `end <= start`, so a
/// window that reaches the renderer always covers at least one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TickWindow {
    start: Tick,
    end: Tick,
}

impl TickWindow {
    /// Build a window from an absolute tick range.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyWindow`] if `end` is not strictly after
    /// `start`.
    pub fn new(start: Tick, end: Tick) -> Result<Self, RenderError> {
        if end.0 <= start.0 {
            return Err(RenderError::EmptyWindow {
                start: start.0,
                end: end.0,
            });
        }
        Ok(Self { start, end })
    }

    /// First tick of the window (inclusive).
    pub fn start(self) -> Tick {
        self.start
    }

    /// One past the last tick of the window (exclusive).
    pub fn end(self) -> Tick {
        self.end
    }
}

/// Resolve `duration` starting at `start_tick` (default 0) into an absolute
/// tick window, using `song`'s tempo map so tempo changes inside the range are
/// honoured.
///
/// This is only the tempo arithmetic. Policy — a default duration, an upper
/// bound on how long one render may be — belongs to the caller, because the
/// analyzers and the render command do not agree on it.
///
/// # Errors
///
/// Returns [`RenderError::EmptyWindow`] if the duration rounds to zero ticks at
/// the song's tempo.
pub fn tick_window_from_seconds(
    song: &Arc<SharedSong>,
    start_tick: Option<Tick>,
    duration: Seconds,
) -> Result<TickWindow, RenderError> {
    let start = start_tick.unwrap_or(Tick(0));
    let end = {
        let song = song.read();
        let start_seconds = song.tick_to_seconds(start);
        let target_seconds = start_seconds + f64::from(duration.as_f32());
        song.seconds_to_tick(target_seconds)
    };
    TickWindow::new(start, end)
}

/// Everything one render needs: which song, which window, how much tail, which
/// optional signal stages, and where the WAV goes.
///
/// The song arrives as a handle rather than as shared application state so a
/// caller can pass a clone it has muted, soloed, or otherwise adjusted without
/// touching the live project.
pub struct WavRenderRequest<'a> {
    /// Sequencer source to render.
    pub song: &'a Arc<SharedSong>,
    /// Absolute range to render.
    pub window: TickWindow,
    /// Extra audio rendered after the transport stops at the window's end, so
    /// reverb and delay tails are captured rather than cut off.
    pub tail: Seconds,
    /// Which optional signal stages to reconstruct, and at what sample rate.
    pub scope: AnalysisScope,
    /// Destination WAV. Missing parent directories are created.
    pub output_path: &'a Path,
}

/// What a completed render produced. Describes the file on disk.
#[derive(Debug, Clone)]
pub struct WavRenderOutcome {
    /// The written file, canonicalized when possible.
    pub path: PathBuf,
    /// Sample rate the render actually ran at.
    pub sample_rate: u32,
    /// Channel count of the written file.
    pub channels: u16,
    /// Rendered length in seconds, tail included.
    pub duration_seconds: f32,
    /// Frames (samples per channel) written.
    pub frames: u64,
    /// Largest absolute sample amplitude in the buffer; 0.0 for silence.
    pub peak: f32,
    /// Non-fatal notes about the engine setup and the render itself.
    pub warnings: Vec<String>,
}

/// Render `request` and write the result to `request.output_path`.
///
/// Builds one [`OfflineEngineSession`] from the live instruments in `session`,
/// renders the window plus its tail, and writes a 32-bit float WAV. The live
/// project is never mutated; the caller decides what `request.song` contains.
///
/// # Errors
///
/// Returns [`RenderError::Render`] if the offline engine cannot be built or the
/// render fails, [`RenderError::CreateOutputDir`] if the destination directory
/// cannot be created, and [`RenderError::WriteWav`] if encoding or writing
/// fails.
pub fn render_window_to_wav(
    session: &SynthSession,
    sample_library: &SharedSampleLibrary,
    request: &WavRenderRequest<'_>,
) -> Result<WavRenderOutcome, RenderError> {
    // Before the render, not just before the write: a directory that cannot be
    // created is an argument problem, and finding out after five minutes of
    // rendering costs those five minutes for nothing.
    if let Some(parent) = request
        .output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| RenderError::CreateOutputDir {
            dir: parent.to_path_buf(),
            source,
        })?;
    }

    let (mut engine_session, mut warnings) =
        OfflineEngineSession::new_with_scope(session, sample_library, request.scope)?;
    let rendered = engine_session.render_range_with_tail(
        request.song,
        request.window.start().0,
        request.window.end().0,
        request.tail,
    )?;
    warnings.extend(rendered.warnings.iter().cloned());

    let peak = crate::audio::export::write_interleaved_wav_f32(
        request.output_path,
        &rendered.samples,
        rendered.sample_rate,
        rendered.channels,
    )
    .map_err(|source| RenderError::WriteWav {
        path: request.output_path.to_path_buf(),
        source,
    })?;

    let frames = rendered.samples.len() as u64 / u64::from(rendered.channels.max(1));
    // Report the absolute path the caller can actually read back; canonicalize
    // only succeeds now that the file exists, so fall back to the input.
    let path = std::fs::canonicalize(request.output_path)
        .unwrap_or_else(|_| request.output_path.to_path_buf());

    Ok(WavRenderOutcome {
        path,
        sample_rate: rendered.sample_rate,
        channels: rendered.channels,
        duration_seconds: rendered.duration_seconds,
        frames,
        peak,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zero-length range is the one thing every caller must be stopped from
    /// handing to the renderer: it would otherwise reach `render_range_with_tail`
    /// and fail deep inside the engine instead of at the argument.
    #[test]
    fn an_empty_range_is_rejected() {
        assert!(matches!(
            TickWindow::new(Tick(480), Tick(480)),
            Err(RenderError::EmptyWindow {
                start: 480,
                end: 480
            })
        ));
        assert!(matches!(
            TickWindow::new(Tick(480), Tick(0)),
            Err(RenderError::EmptyWindow { .. })
        ));
    }

    #[test]
    fn a_forward_range_round_trips() {
        let window = TickWindow::new(Tick(96), Tick(1920)).expect("forward range is valid");
        assert_eq!(window.start(), Tick(96));
        assert_eq!(window.end(), Tick(1920));
    }

    /// The default start is tick 0, and the duration is measured from there
    /// through the song's tempo rather than assumed to be a fixed tick count.
    #[test]
    fn seconds_resolve_through_the_song_tempo() {
        let song = synth_engine::shared_song(synth_sequencer::Song::new("Untitled"));
        let window = tick_window_from_seconds(&song, None, Seconds::new(2.0))
            .expect("two seconds is a non-empty window");
        assert_eq!(window.start(), Tick(0));

        let expected = song.read().seconds_to_tick(2.0);
        assert_eq!(window.end(), expected);
    }

    /// A duration too short to cover a single tick has to fail here, not
    /// produce a zero-sample WAV that looks like a successful render.
    #[test]
    fn a_duration_below_one_tick_is_rejected() {
        let song = synth_engine::shared_song(synth_sequencer::Song::new("Untitled"));
        assert!(matches!(
            tick_window_from_seconds(&song, None, Seconds::ZERO),
            Err(RenderError::EmptyWindow { .. })
        ));
    }
}
