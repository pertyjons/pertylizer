//! The whole `pertylizer render` run, from arguments to receipt.
//!
//! Load, resolve the mix, resolve the window, render, write, report — in that
//! order, with every argument checked against the loaded project *before* the
//! render starts. A ten-second render must not run before the arguments are
//! known to be good.

use std::path::PathBuf;

use synth_core::{AnalysisScope, Seconds};

use super::headless::LoadedProject;
use super::mix::MixSelection;
use super::receipt::{
    AudioInfo, FileDigest, MixInfo, MixTrack, PROTOCOL_VERSION, RenderReceipt, RendererInfo,
};
use super::wav::{WavRenderRequest, render_window_to_wav, tick_window_from_seconds};
use super::{
    MAX_RENDER_BYTES, MAX_RENDER_SAMPLE_RATE, MAX_TAIL_SECONDS, MIN_RENDER_SAMPLE_RATE, RenderError,
};
use crate::audio::arrangement_render::MAX_RENDER_SECONDS;
use crate::audio::wav_format::WavFormat;

/// Channels the offline renderer always produces. Only used to size the
/// allocation check in [`RenderCommand::validate`].
const RENDER_CHANNELS: u32 = 2;

/// Bytes per sample in the renderer's own buffer. The engine renders `f32`
/// whatever format the file is written in, so this is fixed even though
/// [`WavFormat::bytes_per_sample`] is not.
const RENDER_BUFFER_BYTES_PER_SAMPLE: u32 = 4;

/// One render invocation, already parsed but not yet validated against a
/// project.
#[derive(Debug, Clone)]
pub struct RenderCommand {
    /// Project, bundle, or patch file to render. Never written back.
    pub input: PathBuf,
    /// Destination WAV.
    pub output: PathBuf,
    /// Sample rate to render at.
    pub sample_rate: u32,
    /// Sample format the output WAV is encoded in.
    pub format: WavFormat,
    /// How much of the arrangement to render, from tick 0.
    pub seconds: Seconds,
    /// Extra audio captured after the transport stops, for reverb and delay
    /// tails.
    pub tail: Seconds,
    /// Where the receipt goes. `None` prints it on stdout.
    pub result_json: Option<PathBuf>,
    /// Which tracks to solo and mute. Empty means the full mix.
    pub mix: MixSelection,
    /// The invocation, for the receipt.
    pub argv: Vec<String>,
}

impl RenderCommand {
    /// Reject arguments that are wrong on their own terms, before touching the
    /// filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidDuration`],
    /// [`RenderError::DurationTooLong`], [`RenderError::InvalidSampleRate`], or
    /// [`RenderError::RenderTooLarge`].
    fn validate(&self) -> Result<(), RenderError> {
        validate_render_bounds(self.seconds, self.tail, self.sample_rate)
    }

    /// The signal chain a render command reconstructs.
    ///
    /// Unlike the analyzers — which default to the dry instrument sum so a
    /// measurement is not coloured by the master chain — this command exists to
    /// produce the file the project *sounds* like, so master and return effects
    /// are always included. There is deliberately no flag to turn them off in
    /// version 1: nothing has asked for one, and a render that silently omitted
    /// half the mix would be the worse default to have shipped.
    fn scope(&self) -> AnalysisScope {
        AnalysisScope {
            master_effects: true,
            return_effects: true,
            render_sample_rate: synth_core::audio::DeviceSampleRate::new(self.sample_rate),
        }
    }
}

/// Reject a window that is wrong on its own terms, independently of any project.
///
/// Split out of [`RenderCommand::validate`] so that anything declaring a render
/// ahead of time — the reference corpus, above all — is held to exactly the
/// bounds the command applies, instead of carrying a second copy that drifts. A
/// corpus case whose settings fail here would otherwise render fine in review
/// and fail in a batch run hours later.
///
/// # Errors
///
/// Returns [`RenderError::InvalidDuration`], [`RenderError::DurationTooLong`],
/// [`RenderError::InvalidSampleRate`], or [`RenderError::RenderTooLarge`].
pub(crate) fn validate_render_bounds(
    seconds: Seconds,
    tail: Seconds,
    sample_rate: u32,
) -> Result<(), RenderError> {
    let seconds = seconds.as_f32();
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(RenderError::InvalidDuration {
            what: "--seconds",
            bound: "positive",
            value: seconds,
        });
    }
    // The shared renderer clamps a longer range and warns. Refusing here
    // instead means a harness that asked for ten minutes finds out, rather
    // than comparing five minutes of audio against ten and calling it a
    // regression.
    if seconds > MAX_RENDER_SECONDS {
        return Err(RenderError::DurationTooLong {
            what: "--seconds",
            requested: seconds,
            maximum: MAX_RENDER_SECONDS,
        });
    }
    let tail = tail.as_f32();
    if !tail.is_finite() || tail < 0.0 {
        return Err(RenderError::InvalidDuration {
            what: "--tail-seconds",
            bound: "non-negative",
            value: tail,
        });
    }
    if tail > MAX_TAIL_SECONDS {
        return Err(RenderError::DurationTooLong {
            what: "--tail-seconds",
            requested: tail,
            maximum: MAX_TAIL_SECONDS,
        });
    }
    if !(MIN_RENDER_SAMPLE_RATE..=MAX_RENDER_SAMPLE_RATE).contains(&sample_rate) {
        return Err(RenderError::InvalidSampleRate(sample_rate));
    }
    // Each argument above is fine on its own; their product is what the
    // renderer allocates in one `Vec`, and an over-large allocation aborts
    // the process instead of returning an error. Every factor is already
    // bounded by the checks above, so the product cannot overflow.
    //
    // Measured against the *render* buffer, which is `f32` whatever
    // `--bit-depth` writes: the allocation this guards is the one the
    // renderer makes before any encoding happens. A narrower output format
    // makes the file smaller, not the buffer.
    let bytes = (f64::from(seconds + tail)
        * f64::from(sample_rate)
        * f64::from(RENDER_CHANNELS)
        * f64::from(RENDER_BUFFER_BYTES_PER_SAMPLE))
    .ceil() as u64;
    if bytes > MAX_RENDER_BYTES {
        return Err(RenderError::RenderTooLarge {
            seconds,
            tail,
            sample_rate,
            bytes,
            maximum: MAX_RENDER_BYTES,
        });
    }
    Ok(())
}

/// Run `command` end to end and return the receipt describing what happened.
///
/// The input file is only ever read. The mix flags are applied to the loaded
/// in-memory song and never saved back.
///
/// # Errors
///
/// Returns whichever [`RenderError`] the step that failed produced. Any failure
/// before the WAV write leaves no output files at all — except that a
/// `--result-json` directory is created up front, so that its creation is not
/// what a finished render fails on; both writes go through
/// [`crate::io::atomic`], so neither can leave a truncated file behind.
pub fn run_render_command(command: &RenderCommand) -> Result<RenderReceipt, RenderError> {
    command.validate()?;

    // Checked here so a missing or unreadable input is reported as a load
    // failure, which is what it is, rather than as a digest failure below.
    if !command.input.is_file() {
        return Err(RenderError::ProjectLoad {
            path: command.input.clone(),
            message: "not a readable file".to_string(),
        });
    }
    // Rendering over the input would destroy the project and still produce a
    // receipt that looks clean, because the input digest was taken first.
    //
    // `path_identity`, not `absolute`: neither output exists yet, and
    // `canonicalize` fails on a path with no file behind it — so `out.wav` and
    // `./out.wav` would compare unequal and both be written, one over the
    // other.
    let input_identity = super::headless::path_identity(&command.input);
    for (what, path) in [
        ("--output", Some(&command.output)),
        ("--result-json", command.result_json.as_ref()),
    ] {
        if path.is_some_and(|p| super::headless::path_identity(p) == input_identity) {
            return Err(RenderError::OutputOverwritesInput {
                what,
                path: command.input.clone(),
            });
        }
    }
    if command.result_json.as_ref().is_some_and(|json| {
        super::headless::path_identity(json) == super::headless::path_identity(&command.output)
    }) {
        return Err(RenderError::OutputCollision {
            path: command.output.clone(),
        });
    }
    // Before the render, not after it: a five-minute render must not be spent
    // only to fail on a `--result-json` directory that was never creatable.
    if let Some(parent) = command
        .result_json
        .as_deref()
        .and_then(std::path::Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| RenderError::CreateOutputDir {
            dir: parent.to_path_buf(),
            source,
        })?;
    }
    // Digest the input before loading it, so the receipt names the bytes that
    // were actually read rather than whatever the path points at afterwards.
    let input = FileDigest::of(&command.input)?;

    // Held until the end of this function: the render below snapshots the
    // engine's live instruments, so the loaded engine has to outlive it.
    let project = super::headless::load_project_file(&command.input)?;
    let mut warnings = Vec::new();

    // First, and before the render: an object the load could not reconstruct
    // changes what the WAV contains. A receipt whose only warnings came from
    // the mix and the render would be describing a file it had mis-measured.
    warnings.extend(project.report.diagnostic_lines());

    let applied = {
        let mut song = project.song.write();
        super::mix::apply_mix_selection(&mut song, &command.mix)?
    };
    warnings.extend(applied.warnings.iter().cloned());

    let window = tick_window_from_seconds(&project.song, None, command.seconds)?;

    let audible = audible_tracks(&project, &applied.audible);

    let outcome = render_window_to_wav(
        &project.session,
        &project.sample_library,
        &WavRenderRequest {
            song: &project.song,
            window,
            tail: command.tail,
            scope: command.scope(),
            format: command.format,
            output_path: &command.output,
        },
    )?;
    warnings.extend(outcome.warnings.iter().cloned());

    // A render of nothing exits zero and produces a perfectly valid WAV, so
    // nothing else tells a harness that it just measured silence.
    //
    // The audible-set check comes first and does not look at `peak`, because
    // `peak == 0.0` does not detect this case: the master chain emits a small
    // residual even with every track muted, so a fully silenced mix comes back
    // at a peak of ~1e-4 rather than at zero. Whether any track is audible is a
    // structural fact, and it covers both ways in: a mix selection that muted
    // everything, and an input with no arrangement to play (a bare patch file
    // loads one instrument and no tracks).
    if audible.is_empty() {
        warnings.push(
            "no track in this project is audible — the render contains no instrument signal"
                .to_string(),
        );
    } else if outcome.peak == 0.0 {
        warnings.push(
            "the render is silent (peak 0.0): the window covers no sounding notes".to_string(),
        );
    }

    // An integer format has no headroom: everything past ±1.0 was clamped to
    // full scale on the way out, and unlike the float default that loss is in
    // the file for good. The receipt's `peak` records what the engine produced,
    // so it is the only place the overshoot survives — say so rather than
    // leaving a harness to compare `peak` against `sample_format` itself.
    if outcome.format != WavFormat::Float32 && outcome.peak > 1.0 {
        warnings.push(format!(
            "the render clipped: peak {:.3} exceeds full scale and {} output clamped it — \
             lower the mix or render at --bit-depth 32f",
            outcome.peak,
            outcome.format.label()
        ));
    }

    let output = FileDigest::of(&outcome.path)?;

    let receipt = RenderReceipt {
        protocol_version: PROTOCOL_VERSION,
        renderer: RendererInfo::current(),
        input,
        output,
        audio: AudioInfo {
            sample_rate: outcome.sample_rate,
            channels: outcome.channels,
            bit_depth: outcome.format.bits_per_sample(),
            sample_format: outcome.format.encoding(),
            frames: outcome.frames,
            duration_seconds: outcome.duration_seconds,
            requested_seconds: command.seconds.as_f32(),
            tail_seconds: command.tail.as_f32(),
            start_tick: window.start().0,
            end_tick: window.end().0,
            peak: outcome.peak,
        },
        load_summary: project.report.summary_with_diagnostics(),
        mix: MixInfo {
            soloed: applied.soloed.iter().map(|id| id.0).collect(),
            muted: applied.muted.iter().map(|id| id.0).collect(),
            audible,
        },
        warnings,
        command: command.argv.clone(),
    };

    if let Some(path) = &command.result_json {
        let json = receipt.to_json()?;
        // The directory was created up front, before the render.
        crate::io::atomic::write(path, &json).map_err(|source| RenderError::WriteReceipt {
            path: path.clone(),
            source,
        })?;
    }

    Ok(receipt)
}

/// Pair each audible track id with the name it had at render time, so a receipt
/// reads without the project next to it.
fn audible_tracks(project: &LoadedProject, ids: &[synth_sequencer::TrackId]) -> Vec<MixTrack> {
    let song = project.song.read();
    ids.iter()
        .map(|id| MixTrack {
            id: id.0,
            name: song
                .track(*id)
                .map_or_else(String::new, |track| track.name.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_RENDER_BYTES, MAX_RENDER_SAMPLE_RATE, MAX_RENDER_SECONDS, MAX_TAIL_SECONDS,
        RENDER_BUFFER_BYTES_PER_SAMPLE, RENDER_CHANNELS,
    };

    /// [`MAX_RENDER_BYTES`] is a backstop, not a reachable check: the duration,
    /// tail, and rate bounds together cap one render below it. That is a
    /// relationship between four independent constants, so it is pinned rather
    /// than assumed — raising any of them fails here, which is the moment to
    /// re-check whether the allocation guard is armed again.
    ///
    /// A unit test rather than an integration one, because
    /// `MAX_RENDER_SECONDS` is an internal renderer bound and publishing it to
    /// let a test read it would be exposing an implementation detail as
    /// supported API.
    #[test]
    fn the_other_bounds_cannot_reach_the_size_budget() {
        let largest = f64::from(MAX_RENDER_SECONDS + MAX_TAIL_SECONDS)
            * f64::from(MAX_RENDER_SAMPLE_RATE)
            * f64::from(RENDER_CHANNELS)
            * f64::from(RENDER_BUFFER_BYTES_PER_SAMPLE);
        assert!(
            largest <= MAX_RENDER_BYTES as f64,
            "the bounds now reach {largest} bytes against a {MAX_RENDER_BYTES}-byte budget: \
             the size guard is reachable again, so \
             `validation_does_not_depend_on_the_output_format` should go back to driving both \
             formats into `RenderTooLarge`"
        );
    }
}
