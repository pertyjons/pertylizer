//! Cost-per-rendered-second measurement for the offline renderer.
//!
//! Run with `cargo run --release -p pertylizer --bin render_cost`. For every
//! case in the reference corpus it loads the project once, then repeatedly
//! builds a fresh [`OfflineEngineSession`] and renders the case's window,
//! timing **only** the render call.
//!
//! It exists for ADR-0037's proxy measurement: how V1's render cost varies with
//! the block size it processes in. That block size is the compile-time constant
//! [`BUFFER_SIZE`], so a sweep is four builds rather than four runs — and each
//! run stamps every record with the constant it was actually built with, so a
//! mislabelled binary cannot silently become a datapoint.
//!
//! P00A-T003 needs the same cost at more than one operating point, so two
//! sweeps are runs rather than builds:
//!
//! - `--sample-rate <hz>` overrides each case's render rate.
//! - `--polyphony <n>` replaces the corpus with a generated probe holding `n`
//!   notes at once (see [`polyphony_probe`]). It is not a corpus case: it pins
//!   no behaviour, and exists so cost can be read as a function of voice count
//!   without inventing manifest entries nobody would check.
//!
//! # What is timed, and what is not
//!
//! The timed region is one `render_range_with_tail` call. Project loading,
//! engine construction, instrument reconstruction, WAV encoding, and digesting
//! are all outside it. That is deliberate: everything excluded costs roughly the
//! same at every block size, so leaving it in would add a constant to each
//! measurement and compress exactly the ratios the decision is read from.
//!
//! Timing is wall clock on a single-threaded render. The interleaving of builds
//! across rounds — rather than measuring one build to completion and then the
//! next — is the operator's job, and it is what keeps machine drift from being
//! read as a block-size effect.
//!
//! # Output
//!
//! One JSON object per render on stdout, newline-delimited, including the
//! warm-up renders (marked `"warmup": true`) so a discarded measurement is
//! visible rather than absent. Warnings go to stderr.

use std::path::{Path, PathBuf};
use std::time::Instant;

use pertylizer::audio::arrangement_render::{BUFFER_SIZE, OfflineEngineSession};
use pertylizer::audio::wav_format::WavFormat;
use pertylizer::corpus::fixtures::polyphony_probe;
use pertylizer::corpus::{CORPUS_DIR, CorpusManifest, MANIFEST_FILE, RenderSettings};
use pertylizer::render::headless::load_project_file;
use pertylizer::render::{MixSelection, apply_mix_selection, tick_window_from_seconds};
use pertylizer::synth_core::audio::DeviceSampleRate;
use pertylizer::synth_core::{AnalysisScope, Seconds};

/// Renders per case that are discarded before the recorded ones.
const DEFAULT_WARMUP: usize = 1;

/// Recorded renders per case per invocation.
const DEFAULT_REPS: usize = 1;

/// One thing to render: an identifier, a project file, and its render settings.
struct Target {
    id: String,
    input: PathBuf,
    render: RenderSettings,
    /// Voices held at once, for a generated probe. `None` for a corpus case,
    /// whose polyphony is a property of the case rather than of the sweep.
    voices: Option<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::from_args(std::env::args().skip(1))?;
    let corpus_dir = match &options.corpus_dir {
        Some(dir) => dir.clone(),
        None => workspace_root()?.join(CORPUS_DIR),
    };

    let targets = match options.polyphony {
        Some(voices) => vec![probe_target(voices, &options.probe_dir)?],
        None => {
            let manifest = CorpusManifest::load(&corpus_dir.join(MANIFEST_FILE))?;
            manifest
                .cases
                .iter()
                .map(|case| Target {
                    id: case.id.as_str().to_string(),
                    input: case.input_path(&corpus_dir),
                    render: case.render.clone(),
                    voices: None,
                })
                .collect()
        }
    };

    for target in &targets {
        let id = &target.id;
        let project = load_project_file(&target.input)?;
        for line in project.report.diagnostic_lines() {
            eprintln!("{id}: {line}");
        }

        {
            let mut song = project.song.write();
            let applied = apply_mix_selection(
                &mut song,
                &MixSelection {
                    solo: target.render.solo_tracks.clone(),
                    mute: target.render.mute_tracks.clone(),
                },
            )?;
            for warning in &applied.warnings {
                eprintln!("{id}: {warning}");
            }
        }

        let window = tick_window_from_seconds(&project.song, None, target.render.seconds)?;
        // The override applies to the rate the render runs at, not to the rate
        // the case declares: the manifest stays the authority for what the case
        // is, and the sweep records the rate it actually used.
        let sample_rate = options
            .sample_rate
            .map_or(target.render.sample_rate, DeviceSampleRate::new);
        let scope = AnalysisScope {
            master_effects: true,
            return_effects: true,
            render_sample_rate: sample_rate,
        };

        for rep in 0..(options.warmup + options.reps) {
            let warmup = rep < options.warmup;

            // A fresh session per render, not a reused one: a reused session
            // skips the warm-up block and clears the previous render's voices
            // first, so its second render is not the same work as its first.
            let (mut engine, warnings) = OfflineEngineSession::new_with_scope(
                &project.session,
                &project.sample_library,
                scope,
            )?;
            for warning in &warnings {
                eprintln!("{id}: {warning}");
            }

            let started = Instant::now();
            let rendered = engine.render_range_with_tail(
                &project.song,
                window.start().0,
                window.end().0,
                target.render.tail_seconds,
            )?;
            let elapsed = started.elapsed();

            for warning in &rendered.warnings {
                eprintln!("{id}: {warning}");
            }

            let frames = rendered.samples.len() / usize::from(rendered.channels.max(1));
            let seconds = f64::from(rendered.duration_seconds);
            let elapsed_ns = elapsed.as_nanos();
            // Guard the divisor rather than assume it: a case whose window
            // rounds to nothing would otherwise report an infinite cost as if
            // it were a measurement.
            let cost = if seconds > 0.0 {
                Some(elapsed.as_secs_f64() / seconds)
            } else {
                None
            };

            println!(
                "{{\"block_size\":{},\"case\":\"{}\",\"voices\":{},\"rep\":{rep},\
                 \"warmup\":{warmup},\"elapsed_ns\":{elapsed_ns},\
                 \"rendered_seconds\":{seconds},\"cost_per_rendered_second\":{},\
                 \"sample_rate\":{},\"frames\":{frames},\"peak\":{},\"rms\":{}}}",
                BUFFER_SIZE,
                id,
                target
                    .voices
                    .map_or_else(|| "null".to_string(), |v| v.to_string()),
                cost.map_or_else(|| "null".to_string(), |value| format!("{value}")),
                rendered.sample_rate,
                peak(&rendered.samples),
                rms(&rendered.samples),
            );
        }
    }

    Ok(())
}

/// Largest absolute sample in the buffer.
///
/// Recorded next to the timing because renders at different block sizes are
/// *not the same audio* — V1 dispatches sequencer events on block boundaries —
/// and a measurement that could not show that would look like one that had
/// verified it.
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |max, s| max.max(s.abs()))
}

/// Root mean square of the whole buffer, for the same reason as [`peak`]: it
/// moves when the rendered audio moves, and peak alone can miss a change that
/// leaves the loudest sample where it was.
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// The probe's rendered window, chosen to match CORPUS-0001's so a polyphony
/// row and a corpus row are the same length of audio.
const PROBE_SECONDS: f32 = 2.0;

/// The probe's release/effect tail, for the same reason.
const PROBE_TAIL_SECONDS: f32 = 1.0;

/// Write the polyphony probe for `voices` and describe how to render it.
///
/// The project is generated rather than committed: it pins no behaviour, so a
/// file in the corpus would be a fixture nobody validates. It is rewritten on
/// every run, so a stale file from an earlier build cannot be measured.
fn probe_target(voices: u8, probe_dir: &Path) -> Result<Target, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(probe_dir)?;
    let input = probe_dir.join(format!("polyphony-probe-{voices}.ptz"));
    polyphony_probe(voices).save(&input)?;
    Ok(Target {
        id: format!("POLY-{voices:03}"),
        input,
        render: RenderSettings {
            sample_rate: DeviceSampleRate::CD_QUALITY,
            bit_depth: WavFormat::Float32,
            seconds: Seconds::new(PROBE_SECONDS),
            tail_seconds: Seconds::new(PROBE_TAIL_SECONDS),
            solo_tracks: Vec::new(),
            mute_tracks: Vec::new(),
        },
        voices: Some(voices),
    })
}

/// How many renders to run, at which operating point, and over what.
struct Options {
    warmup: usize,
    reps: usize,
    corpus_dir: Option<PathBuf>,
    /// Render rate override. `None` uses each case's declared rate.
    sample_rate: Option<u32>,
    /// Render the generated polyphony probe at this voice count instead of the
    /// corpus.
    polyphony: Option<u8>,
    /// Where a generated probe project is written.
    probe_dir: PathBuf,
}

impl Options {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut options = Self {
            warmup: DEFAULT_WARMUP,
            reps: DEFAULT_REPS,
            corpus_dir: None,
            sample_rate: None,
            polyphony: None,
            probe_dir: std::env::temp_dir().join("pertylizer-render-cost"),
        };
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--warmup" => options.warmup = next_count(&mut args, "--warmup")?,
                "--reps" => options.reps = next_count(&mut args, "--reps")?,
                "--corpus-dir" => {
                    options.corpus_dir = Some(PathBuf::from(
                        args.next().ok_or("--corpus-dir needs a directory")?,
                    ));
                }
                "--sample-rate" => {
                    let raw = args.next().ok_or("--sample-rate needs a rate in Hz")?;
                    let rate: u32 = raw
                        .parse()
                        .map_err(|e| format!("--sample-rate takes Hz, not {raw:?}: {e}"))?;
                    if rate == 0 {
                        return Err("--sample-rate 0 is not a rate".into());
                    }
                    // The engine ceiling, not an arbitrary bound: above it,
                    // buffers sized from `SampleRate::MAX_SUPPORTED` silently
                    // deliver less than their parameters advertise, so a
                    // measurement taken there would describe degraded DSP
                    // without saying so (`LIMIT-0004`).
                    let ceiling = DeviceSampleRate::MAX_SUPPORTED.as_u32();
                    if rate > ceiling {
                        return Err(format!(
                            "--sample-rate {rate} is above the engine ceiling of {ceiling} Hz"
                        )
                        .into());
                    }
                    options.sample_rate = Some(rate);
                }
                "--polyphony" => {
                    let raw = args.next().ok_or("--polyphony needs a voice count")?;
                    let voices: u8 = raw
                        .parse()
                        .map_err(|e| format!("--polyphony takes 1..=128, not {raw:?}: {e}"))?;
                    if voices == 0 {
                        return Err("--polyphony 0 would render silence".into());
                    }
                    options.polyphony = Some(voices);
                }
                "--probe-dir" => {
                    options.probe_dir =
                        PathBuf::from(args.next().ok_or("--probe-dir needs a directory")?);
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        if options.reps == 0 {
            return Err("--reps 0 would record nothing".into());
        }
        Ok(options)
    }
}

fn next_count(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let raw = args.next().ok_or_else(|| format!("{flag} needs a count"))?;
    raw.parse()
        .map_err(|source| format!("{flag} takes a count, not {raw:?}: {source}").into())
}

/// The workspace root, derived from this crate's manifest directory.
fn workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "cannot locate the workspace root from CARGO_MANIFEST_DIR".into())
}
