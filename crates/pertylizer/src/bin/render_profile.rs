//! Memory and per-block timing baseline for the offline renderer.
//!
//! Run with `cargo run --release -p pertylizer --bin render_profile`. It is the
//! other half of P00A-T003: [`render_cost`](../render_cost/index.html) measures
//! throughput, and this measures the two quantities a throughput number cannot
//! contain.
//!
//! # Memory
//!
//! Resident set size, read from `/proc/self/status`, around three phases:
//!
//! - **load** — reading the project and building the live-but-silent engine;
//! - **prepare** — building one [`OfflineEngineSession`], which is where the
//!   voice pools, module graphs, and effect chains are allocated;
//! - **render** — the render call itself, which should need only the output
//!   buffer.
//!
//! `VmHWM` — peak RSS since the process started — is reported alongside, since
//! a per-phase delta cannot show a peak inside the phase.
//!
//! **This is the kernel's view, not the allocator's**, and the difference
//! matters when reading the numbers. RSS counts pages the process holds, so a
//! phase that allocates and frees leaves its pages behind in the allocator's
//! free lists and reads as retained; a phase that reuses pages an earlier phase
//! freed reads as allocating nothing. Every per-phase figure is therefore an
//! upper bound on what the phase kept and a lower bound on what it touched.
//!
//! A counting global allocator would give exact figures, and was written first.
//! It needs `unsafe`, which this workspace denies, so it would need an explicit
//! exemption rather than a quiet `allow`. Page-granular truth beat that trade
//! for a baseline whose subject is megabytes.
//!
//! # Timing
//!
//! `OfflineEngineSession::start_block_timings` records wall-clock nanoseconds
//! per `process` call. What matters is the distribution, not the mean: a real
//! time callback fails on its worst block, not its average one. The report gives
//! p50, p99, and max, and divides them by the block's own real-time budget —
//! `frames / sample_rate` — so the number reads as "this block used 3% of the
//! time it would have had".
//!
//! This is still an offline measurement with no host, no device, and no
//! competing thread, so it is a **lower bound** on what a live callback would
//! cost. It is recorded as one, not as a headroom claim.

use std::path::PathBuf;

use pertylizer::audio::arrangement_render::OfflineEngineSession;
use pertylizer::corpus::{CORPUS_DIR, CorpusManifest, MANIFEST_FILE};
use pertylizer::render::headless::load_project_file;
use pertylizer::render::{MixSelection, apply_mix_selection, tick_window_from_seconds};
use pertylizer::synth_core::AnalysisScope;

// ---------------------------------------------------------------------------
// Resident set
// ---------------------------------------------------------------------------

/// A field of `/proc/self/status` whose value is in kibibytes, as bytes.
///
/// Linux only, deliberately: a portable RSS needs a dependency, and a
/// dependency added for a baseline measurement is a dependency the release has
/// to carry. On any other platform every memory figure is reported as `null`
/// rather than as zero, so a missing measurement cannot be read as a small one.
fn status_bytes(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with(field))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib.saturating_mul(1024))
}

/// Current resident set size in bytes.
fn resident_bytes() -> Option<u64> {
    status_bytes("VmRSS:")
}

/// Peak resident set size since the process started, in bytes.
fn peak_resident_bytes() -> Option<u64> {
    status_bytes("VmHWM:")
}

/// RSS at one instant, as a signed value so a difference cannot underflow.
fn rss_or_zero() -> i64 {
    resident_bytes().unwrap_or(0) as i64
}

/// Format an optional byte count as JSON.
fn json_bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |b| b.to_string())
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// Percentile by nearest rank, on an already-sorted slice.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::from_args(std::env::args().skip(1))?;
    let corpus_dir = match &options.corpus_dir {
        Some(dir) => dir.clone(),
        None => workspace_root()?.join(CORPUS_DIR),
    };
    let manifest = CorpusManifest::load(&corpus_dir.join(MANIFEST_FILE))?;

    for case in &manifest.cases {
        let id = case.id.as_str();

        let load_start = rss_or_zero();
        let project = load_project_file(&case.input_path(&corpus_dir))?;
        let load_delta = rss_or_zero() - load_start;
        for line in project.report.diagnostic_lines() {
            eprintln!("{id}: {line}");
        }

        {
            let mut song = project.song.write();
            let applied = apply_mix_selection(
                &mut song,
                &MixSelection {
                    solo: case.render.solo_tracks.clone(),
                    mute: case.render.mute_tracks.clone(),
                },
            )?;
            for warning in &applied.warnings {
                eprintln!("{id}: {warning}");
            }
        }

        let window = tick_window_from_seconds(&project.song, None, case.render.seconds)?;
        let sample_rate = options.sample_rate.map_or(
            case.render.sample_rate,
            pertylizer::synth_core::audio::DeviceSampleRate::new,
        );
        let scope = AnalysisScope {
            master_effects: true,
            return_effects: true,
            render_sample_rate: sample_rate,
        };

        for rep in 0..(options.warmup + options.reps) {
            let warmup = rep < options.warmup;

            let prepare_start = rss_or_zero();
            let (mut engine, warnings) = OfflineEngineSession::new_with_scope(
                &project.session,
                &project.sample_library,
                scope,
            )?;
            let prepare_delta = rss_or_zero() - prepare_start;
            for warning in &warnings {
                eprintln!("{id}: {warning}");
            }

            engine.start_block_timings();

            let render_start = rss_or_zero();
            let rendered = engine.render_range_with_tail(
                &project.song,
                window.start().0,
                window.end().0,
                case.render.tail_seconds,
            )?;
            let render_delta = rss_or_zero() - render_start;
            for warning in &rendered.warnings {
                eprintln!("{id}: {warning}");
            }

            let timings = engine.take_block_timings().unwrap_or_default();
            // Only full blocks: the last block of a render is short, and a short
            // block's time is not comparable with a full one's. Both counts are
            // reported so a reader can see how many were dropped.
            let full = timings.frames.iter().copied().max().unwrap_or(0);
            let mut full_nanos: Vec<u64> = timings
                .nanos
                .iter()
                .zip(timings.frames.iter())
                .filter(|(_, frames)| **frames == full)
                .map(|(nanos, _)| *nanos)
                .collect();
            full_nanos.sort_unstable();

            let budget_ns = if full > 0 && sample_rate.as_u32() > 0 {
                f64::from(full) / f64::from(sample_rate.as_u32()) * 1e9
            } else {
                0.0
            };
            let ratio = |ns: u64| {
                if budget_ns > 0.0 {
                    format!("{}", ns as f64 / budget_ns)
                } else {
                    "null".to_string()
                }
            };

            let p50 = percentile(&full_nanos, 50.0);
            let p99 = percentile(&full_nanos, 99.0);
            let max = full_nanos.last().copied().unwrap_or(0);

            println!(
                "{{\"case\":\"{id}\",\"rep\":{rep},\"warmup\":{warmup},\
                 \"sample_rate\":{},\"blocks\":{},\"full_blocks\":{},\"block_frames\":{full},\
                 \"block_budget_ns\":{budget_ns},\
                 \"block_p50_ns\":{p50},\"block_p99_ns\":{p99},\"block_max_ns\":{max},\
                 \"block_p50_ratio\":{},\"block_p99_ratio\":{},\"block_max_ratio\":{},\
                 \"rss_load_delta\":{load_delta},\"rss_prepare_delta\":{prepare_delta},\
                 \"rss_render_delta\":{render_delta},\"rss_bytes\":{},\"rss_peak_bytes\":{}}}",
                sample_rate.as_u32(),
                timings.len(),
                full_nanos.len(),
                ratio(p50),
                ratio(p99),
                ratio(max),
                json_bytes(resident_bytes()),
                json_bytes(peak_resident_bytes()),
            );
        }
    }

    Ok(())
}

/// How many renders to profile, at which rate, and over what.
struct Options {
    warmup: usize,
    reps: usize,
    corpus_dir: Option<PathBuf>,
    sample_rate: Option<u32>,
}

impl Options {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut options = Self {
            warmup: 1,
            reps: 1,
            corpus_dir: None,
            sample_rate: None,
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
                    options.sample_rate = Some(rate);
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
