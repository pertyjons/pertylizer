//! Pertylizer — a modular synthesizer.
//!
//! # Usage
//!
//! ```bash
//! # Run with GUI (MCP + OSC telemetry enabled by default; ports from
//! # pertylizer.toml next to the executable, defaults 9850 / 9000)
//! cargo run
//!
//! # Run headless (no GUI, MCP server on stdio)
//! cargo run -- --headless
//!
//! # Run without OSC telemetry
//! cargo run -- --no-osc
//! ```

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[cfg(any(feature = "gui-egui", feature = "mcp"))]
use pertylizer::audio::{
    self, AudioHostTrait, BufferSize, ChannelCount, DeviceSampleRate, StreamConfig,
};
#[cfg(feature = "gui-egui")]
use pertylizer::gui::{SynthGuiConfig, create_backend};
#[cfg(any(feature = "gui-egui", feature = "mcp"))]
use pertylizer::synth_core::VoiceCount;
#[cfg(any(feature = "gui-egui", feature = "mcp"))]
use pertylizer::synth_engine::{AllocationMode, AllocatorConfig, SynthEngine};

/// Everything the binary accepts.
///
/// One parser for the whole surface. The flags below used to be matched by
/// hand against `env::args()`, with a separately maintained `print_help` and no
/// rejection of anything unrecognised — so a misspelled `--headles` silently
/// started the GUI. Deriving both the parse and the help from this struct means
/// they cannot drift, and an unknown argument now exits non-zero.
///
/// Fields are `#[cfg]`-gated on the feature that gives them meaning, so the
/// help text a build prints describes that build rather than the full set.
#[derive(Debug, Parser)]
// `long_about = None` keeps the rationale in the doc comment above out of
// `--help`; without it clap promotes the whole comment into the long help.
#[command(
    name = "pertylizer",
    version,
    about = "Pertylizer — a modular synthesizer",
    long_about = None,
    after_help = KEYBOARD_HELP
)]
struct Cli {
    /// Run without GUI (MCP server on stdio).
    #[cfg(feature = "mcp")]
    #[arg(long)]
    headless: bool,

    /// Disable OSC telemetry output (enabled by default).
    #[cfg(feature = "osc")]
    #[arg(long)]
    no_osc: bool,

    /// What to do. Omitted launches the GUI, as it always has.
    #[command(subcommand)]
    command: Option<Command>,
}

/// The binary's non-GUI modes.
#[derive(Debug, Subcommand)]
enum Command {
    /// Render a saved project to a WAV file and exit.
    Render(RenderArgs),
}

/// Version-1 arguments of `pertylizer render`.
///
/// The mix flags are the whole mix state: whatever the project saved is
/// cleared, and exactly what is asked for here is applied. That keeps a render
/// a function of this command plus the input's content, which is what makes the
/// receipt worth reading.
#[derive(Debug, clap::Args)]
struct RenderArgs {
    /// Contract version this invocation speaks.
    #[arg(long, default_value_t = pertylizer::render::PROTOCOL_VERSION)]
    protocol_version: u32,

    /// Project, bundle, or patch file to render. Never written to.
    #[arg(long, value_name = "FILE")]
    input: PathBuf,

    /// Destination WAV.
    #[arg(long, value_name = "FILE")]
    output: PathBuf,

    /// Sample rate to render at.
    #[arg(long, default_value_t = 44_100, value_name = "HZ")]
    sample_rate: u32,

    /// Sample format of the output WAV. Integer depths are 8, 16, 24, and 32i;
    /// 32f is 32-bit float, which is lossless against the renderer's own
    /// output.
    #[arg(long = "bit-depth", default_value = "32f", value_name = "DEPTH")]
    format: pertylizer::render::WavFormat,

    /// How much of the arrangement to render, from the start.
    #[arg(long, default_value_t = 10.0, value_name = "SECONDS")]
    seconds: f32,

    /// Extra audio to capture after the transport stops, for reverb and delay
    /// tails.
    #[arg(long, default_value_t = 0.0, value_name = "SECONDS")]
    tail_seconds: f32,

    /// Where to write the JSON result. Omitted prints it on stdout.
    #[arg(long, value_name = "FILE")]
    result_json: Option<PathBuf>,

    /// Solo a track, by id or by unique name. Repeatable.
    #[arg(long, value_name = "ID|NAME")]
    solo_track: Vec<pertylizer::render::TrackSelector>,

    /// Mute a track, by id or by unique name. Repeatable.
    #[arg(long, value_name = "ID|NAME")]
    mute_track: Vec<pertylizer::render::TrackSelector>,
}

/// Trailing help section. Not a set of flags, so it rides `after_help` rather
/// than being invented as arguments nobody can pass.
const KEYBOARD_HELP: &str = "\
KEYBOARD:
    Z-M           Play notes (C3-B3)
    Q-I           Play notes (C4-C5)
    2,3,5,6,7     Black keys
    -/+           Shift octave";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialise tracing FIRST. Writer is locked to stderr so --headless mode
    // (which speaks JSON-RPC over stdout) is not corrupted by log output.
    // Default filter keeps our crates at info and silences chatty deps; the
    // RUST_LOG env var overrides if set, e.g.
    //   RUST_LOG=synth_mcp=debug,pertylizer=info
    // The capture-layer clone flows to the GUI app; headless drops it.
    #[cfg(feature = "gui-egui")]
    let activity_log = init_tracing();
    #[cfg(not(feature = "gui-egui"))]
    init_tracing();

    // Install the crash-diagnostics panic hook now that tracing is up, so a
    // panic on any thread is logged and dumped to a crash report file.
    pertylizer::panic_hook::install();

    // Parsed before the banner and the thread pool, so `--help` and a rejected
    // argument print exactly one thing and exit without starting anything.
    let cli = Cli::parse();

    // stderr so stdout stays a clean JSON-RPC channel in --headless mode, and a
    // clean JSON channel for `render` without --result-json.
    eprintln!(
        "Pertylizer v{} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE"),
    );

    // Leave at least one CPU for the tokio executor: offline analyzers spawn
    // rayon-parallel renders that otherwise pin every core and starve the MCP
    // server's SSE keep-alive, which axum/hyper aborts after ~15 s — dropping
    // the active client session. Floor at 2 so 2-core CI/VM hosts still
    // parallelise per-track renders (mild oversubscription is benign for the
    // short analyzer bursts; tokio is otherwise idle during the render).
    let rayon_threads = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(2))
        .unwrap_or(2);
    if let Err(e) = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global()
    {
        tracing::warn!(error = %e, "rayon global thread pool was already initialised");
    }

    // `--headless` and a subcommand are two different things to do. Taking the
    // subcommand and ignoring the flag would let `pertylizer --headless render
    // …` look like it started an MCP server while it rendered and exited.
    // Printed and exited rather than returned: `main`'s error path `Debug`-
    // prints, and `Custom { kind: Other, error: "…" }` is not a usage message.
    #[cfg(feature = "mcp")]
    if cli.headless && cli.command.is_some() {
        eprintln!("error: --headless runs the MCP server and cannot be combined with a subcommand");
        std::process::exit(1);
    }

    if let Some(Command::Render(args)) = &cli.command {
        // Nothing here draws or listens; drop the GUI's log clone.
        #[cfg(feature = "gui-egui")]
        drop(activity_log);
        return run_render(args);
    }

    #[cfg(feature = "mcp")]
    if cli.headless {
        // No GUI to render the log; drop the capture clone.
        #[cfg(feature = "gui-egui")]
        drop(activity_log);
        return run_headless_mcp();
    }

    #[cfg(feature = "gui-egui")]
    {
        run_gui(activity_log, &cli)
    }
    #[cfg(not(feature = "gui-egui"))]
    {
        Err(std::io::Error::other(
            "this build has no GUI; enable `gui-egui` or run a build with `mcp` and --headless",
        )
        .into())
    }
}

/// Render one project and exit.
///
/// Failures print their message, then exit non-zero. Returning the error to
/// `main` instead would `Debug`-print it — `MixSelection(UnknownTrackId(42))`
/// rather than "no track with id 42 in this project" — which is no use to
/// whoever generated the command line. The cause chain is not walked because
/// every error here already names its source in its own message.
fn run_render(args: &RenderArgs) -> Result<(), Box<dyn std::error::Error>> {
    let Err(error) = render_project(args) else {
        return Ok(());
    };
    eprintln!("error: {error}");
    std::process::exit(1);
}

/// The render itself.
///
/// Progress goes to stderr so stdout stays a clean JSON channel: without
/// `--result-json` the receipt is printed there and nothing else is.
fn render_project(args: &RenderArgs) -> Result<(), Box<dyn std::error::Error>> {
    use pertylizer::render::{MixSelection, RenderCommand, run_render_command};

    if args.protocol_version != pertylizer::render::PROTOCOL_VERSION {
        return Err(std::io::Error::other(format!(
            "unsupported --protocol-version {}; this build speaks version {}",
            args.protocol_version,
            pertylizer::render::PROTOCOL_VERSION
        ))
        .into());
    }

    let command = RenderCommand {
        input: args.input.clone(),
        output: args.output.clone(),
        sample_rate: args.sample_rate,
        format: args.format,
        seconds: pertylizer::synth_core::Seconds::new(args.seconds),
        tail: pertylizer::synth_core::Seconds::new(args.tail_seconds),
        result_json: args.result_json.clone(),
        mix: MixSelection {
            solo: args.solo_track.clone(),
            mute: args.mute_track.clone(),
        },
        // `args_os`, not `args`: clap parses paths from `args_os` and happily
        // accepts a non-UTF-8 one, so `env::args()` — which panics mid-iteration
        // on exactly that input — would crash on a command line the parser had
        // already approved.
        argv: std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
    };

    let receipt = run_render_command(&command)?;
    for warning in &receipt.warnings {
        eprintln!("warning: {warning}");
    }
    match &args.result_json {
        Some(path) => eprintln!(
            "✓ Rendered {} frames to {} (receipt: {})",
            receipt.audio.frames,
            receipt.output.path,
            path.display()
        ),
        None => {
            let json = receipt.to_json()?;
            std::io::Write::write_all(&mut std::io::stdout(), &json)?;
        }
    }
    Ok(())
}

#[cfg(feature = "gui-egui")]
// `cli` is read only for `--no-osc`, and `handle` is only borrowed mutably to
// hand the note-event consumer to the telemetry thread. A build without `osc`
// needs neither, and `-D warnings` would otherwise fail that configuration.
#[cfg_attr(not(feature = "osc"), allow(unused_variables, unused_mut))]
fn run_gui(
    activity_log: pertylizer::activity_log::ActivityLog,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load persistent settings
    let settings = pertylizer::io::AppSettings::load();

    // Load the runtime config (MCP / OSC ports) from pertylizer.toml next to
    // the executable; falls back to defaults when the file is absent.
    #[cfg(any(feature = "mcp", feature = "osc"))]
    let config = synth_config::RuntimeConfig::load();

    // Create the synth engine with 8-voice polyphony
    let allocator_config = AllocatorConfig {
        max_voices: VoiceCount::OCTO,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (engine, mut handle) = SynthEngine::new();

    // Start OSC telemetry by default (disable with --no-osc)
    #[cfg(feature = "osc")]
    let (_osc_telemetry, osc_shared) = if cli.no_osc {
        (None, None)
    } else {
        let mut osc = synth_osc::OscTelemetry::new(synth_osc::OscConfig::from_parts(
            config.osc.multicast_group,
            config.osc.port,
            config.osc.update_rate_hz,
        ));
        let shared = osc.shared_state();
        if let Some(consumer) = handle.take_note_event_consumer() {
            osc.start(std::sync::Arc::clone(&handle.state), consumer);
        }
        (Some(osc), Some(shared))
    };

    // Create the shared session (module lifecycle owner)
    let session = std::sync::Arc::new(pertylizer::session::SynthSession::new(
        handle.command_sender(),
        std::sync::Arc::clone(&handle.state),
    ));

    // Create shared Song (always available, not just MCP)
    let song = synth_engine::shared_song(synth_sequencer::Song::new("Untitled"));

    // Send the shared Song to the engine so sequencer shares the same instance
    let _ = handle
        .command_sender()
        .send(pertylizer::synth_engine::EngineCommand::SetSong {
            song: std::sync::Arc::clone(&song),
        });

    // Create shared sample library (used by GUI and MCP bridge)
    let sample_library =
        std::sync::Arc::new(std::sync::RwLock::new(synth_sampler::SampleLibrary::new()));

    // Start MCP HTTP server in background (if feature enabled)
    #[cfg(feature = "mcp")]
    let mcp_shared = {
        let shared = std::sync::Arc::new(pertylizer::mcp_shared::McpSharedState::with_song(
            std::sync::Arc::clone(&song),
        ));
        let bridge = std::sync::Arc::new(pertylizer::mcp_bridge::AppSynthBridge::new(
            std::sync::Arc::clone(&session),
            std::sync::Arc::clone(&shared),
            std::sync::Arc::clone(&sample_library),
        ));
        let registry = shared.mcp_sessions.clone();
        let shared_for_flag = std::sync::Arc::clone(&shared);
        let mcp_port = config.mcp.port;
        let mcp_thread = std::thread::Builder::new()
            .name("mcp-server".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(error = %e, "MCP: failed to create tokio runtime; MCP disabled");
                        return;
                    }
                };
                rt.block_on(async {
                    shared_for_flag
                        .mcp_listening
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    if let Err(e) = synth_mcp::serve_http(bridge, mcp_port, Some(registry)).await {
                        tracing::error!(error = %e, port = mcp_port, "MCP HTTP server stopped");
                        shared_for_flag
                            .mcp_listening
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            });
        if let Err(e) = mcp_thread {
            tracing::error!(error = %e, "MCP: failed to spawn server thread; MCP disabled");
        }
        shared
    };

    // Create audio host
    let host: Box<dyn AudioHostTrait> = match audio::default_host() {
        Ok(h) => {
            println!("✓ Audio backend: {}", h.backend_name());
            Box::new(h)
        }
        Err(e) => {
            println!("⚠ Could not initialize audio: {e}");
            println!("  Falling back to null backend (no sound)");
            Box::new(audio::null_host())
        }
    };

    // Configure stream
    let stream_config = StreamConfig {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        buffer_size: BufferSize::MEDIUM,
        channels: ChannelCount::Stereo,
    };

    // Create GUI configuration
    let gui_config = SynthGuiConfig {
        title: format!(
            "Pertylizer v{} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("BUILD_DATE"),
        ),
        allocator_config,
        stream_config,
        session,
        song,
        #[cfg(feature = "mcp")]
        mcp_shared: Some(mcp_shared),
        #[cfg(feature = "osc")]
        osc_shared,
        settings,
        sample_library,
        activity_log,
    };

    let backend = create_backend();
    backend.run(engine, handle, host, gui_config)?;

    Ok(())
}

/// Run headless MCP server on stdio (no GUI, audio still plays).
#[cfg(feature = "mcp")]
fn run_headless_mcp() -> Result<(), Box<dyn std::error::Error>> {
    let (engine, handle) = SynthEngine::new();

    // Headless mode runs the engine purely as an offline-render driver for
    // MCP analyzers — no listener, no realtime budget to make. The null host
    // avoids underrun noise the real backend would produce when offline
    // renders saturate the CPU.
    let mut host: Box<dyn AudioHostTrait> = Box::new(audio::null_host());
    eprintln!("✓ Audio backend: {} (headless)", host.backend_name());

    let stream_config = StreamConfig {
        sample_rate: DeviceSampleRate::DVD_QUALITY,
        buffer_size: BufferSize::MEDIUM,
        channels: ChannelCount::Stereo,
    };

    let _stream_info = host.start_output(None, &stream_config, Box::new(engine))?;
    eprintln!("✓ Audio stream started");

    // Create the shared session (module lifecycle owner)
    let session = std::sync::Arc::new(pertylizer::session::SynthSession::new(
        handle.command_sender(),
        std::sync::Arc::clone(&handle.state),
    ));

    let shared = std::sync::Arc::new(pertylizer::mcp_shared::McpSharedState::new());
    // Send the shared Song to the engine so sequencer and MCP share the same instance
    handle
        .command_sender()
        .send(pertylizer::synth_engine::EngineCommand::SetSong {
            song: std::sync::Arc::clone(&shared.song),
        });
    let sample_library =
        std::sync::Arc::new(std::sync::RwLock::new(synth_sampler::SampleLibrary::new()));
    let bridge = std::sync::Arc::new(pertylizer::mcp_bridge::AppSynthBridge::new(
        std::sync::Arc::clone(&session),
        std::sync::Arc::clone(&shared),
        std::sync::Arc::clone(&sample_library),
    ));

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(synth_mcp::serve_stdio(bridge))?;

    host.stop()?;

    Ok(())
}

/// Install a `tracing` subscriber and return the shared [`ActivityLog`] fed by
/// its capture layer.
///
/// Two layers hang off one `Registry`:
/// - **stderr fmt layer** — the existing behaviour, under the `RUST_LOG`
///   `EnvFilter` (default `warn,pertylizer=info,…`). Headless mode uses stdout
///   for JSON-RPC, so logs MUST go to stderr to avoid corrupting the MCP
///   protocol stream.
/// - **capture layer** — writes events into the returned [`ActivityLog`] under
///   its own, deliberately more permissive filter (`…=debug`), so the buffer
///   holds debug-level detail even when stderr stays at `info`. The Home
///   console then decides what to *show*; its Debug toggle would reveal nothing
///   if the capture layer shared the stderr filter.
///
/// The caller hands the returned clone to the GUI app; `--headless` drops it.
#[cfg(feature = "gui-egui")]
fn init_tracing() -> pertylizer::activity_log::ActivityLog {
    use pertylizer::activity_log::{ActivityLog, CaptureLayer};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Layer};

    // Forward the `log` crate (used by egui/eframe) into tracing so their
    // diagnostics — notably egui's id-clash warnings — reach both sinks
    // instead of being silently dropped. Best-effort: ignore if already set.
    let _ = tracing_log::LogTracer::init();

    let stderr_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // `synth_mcp::call=warn` keeps the per-call success line (emitted at info
        // by `call_tool` so the activity console shows every MCP call) OFF stderr
        // — otherwise a busy agent session would spam one line per call. The
        // capture layer below has no such override, so the console still gets it.
        EnvFilter::new("warn,pertylizer=info,synth_mcp=info,synth_mcp::call=warn,synth_engine=info")
    });
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_filter(stderr_filter);

    let activity_log = ActivityLog::new();
    // Fixed (not env-derived) so the buffer always holds the debug detail the
    // panel's Debug toggle reveals, regardless of RUST_LOG.
    let capture_filter = EnvFilter::new("warn,pertylizer=debug,synth_mcp=debug,synth_engine=debug");
    let capture_layer = CaptureLayer::new(activity_log.clone()).with_filter(capture_filter);

    let result = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(capture_layer)
        .try_init();
    if let Err(e) = result {
        eprintln!("warning: failed to install tracing subscriber: {e}");
    }

    activity_log
}

/// Install the stderr-only tracing subscriber for builds without a GUI.
#[cfg(not(feature = "gui-egui"))]
fn init_tracing() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Layer};

    let _ = tracing_log::LogTracer::init();
    let stderr_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn,pertylizer=info,synth_mcp=info,synth_mcp::call=warn,synth_engine=info")
    });
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_filter(stderr_filter);
    if let Err(error) = tracing_subscriber::registry().with(fmt_layer).try_init() {
        eprintln!("warning: failed to install tracing subscriber: {error}");
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    /// Running with no arguments is how the GUI is launched, so it has to stay
    /// a valid invocation — a required argument here would break every desktop
    /// launcher.
    #[test]
    fn no_arguments_parses() {
        assert!(Cli::try_parse_from(["pertylizer"]).is_ok());
    }

    /// The headline regression: nothing rejected unknown arguments before, so
    /// `--headles` started the GUI as if it had been asked to. A harness
    /// generating command lines needs a typo to fail, not to render the wrong
    /// thing successfully.
    #[test]
    fn an_unknown_argument_is_rejected() {
        let err = Cli::try_parse_from(["pertylizer", "--headles"]).expect_err("must not parse");
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn headless_sets_its_flag() {
        let cli = Cli::try_parse_from(["pertylizer", "--headless"]).expect("parses");
        assert!(cli.headless);
        assert!(
            !Cli::try_parse_from(["pertylizer"])
                .expect("parses")
                .headless
        );
    }

    /// Telemetry is on unless asked otherwise, so the flag's absence must read
    /// as "enabled" rather than defaulting the other way.
    #[cfg(feature = "osc")]
    #[test]
    fn osc_is_on_unless_disabled() {
        assert!(!Cli::try_parse_from(["pertylizer"]).expect("parses").no_osc);
        assert!(
            Cli::try_parse_from(["pertylizer", "--no-osc"])
                .expect("parses")
                .no_osc
        );
    }

    /// A flag whose feature is off must be rejected rather than silently
    /// accepted, or a build would advertise one surface and accept another.
    #[cfg(not(feature = "mcp"))]
    #[test]
    fn headless_is_unknown_without_the_mcp_feature() {
        assert!(Cli::try_parse_from(["pertylizer", "--headless"]).is_err());
    }

    /// clap's own consistency check: conflicting or malformed argument
    /// definitions panic here rather than at the first user invocation.
    #[test]
    fn the_command_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    /// No subcommand still means "launch the GUI", so adding `render` must not
    /// have made a subcommand mandatory.
    #[test]
    fn no_subcommand_is_still_valid() {
        assert!(
            Cli::try_parse_from(["pertylizer"])
                .expect("parses")
                .command
                .is_none()
        );
    }

    #[test]
    fn render_parses_its_version_1_arguments() {
        let cli = Cli::try_parse_from([
            "pertylizer",
            "render",
            "--input",
            "a project.ptz",
            "--output",
            "out.wav",
            "--seconds",
            "10",
            "--tail-seconds",
            "2",
            "--sample-rate",
            "48000",
            "--result-json",
            "result.json",
            "--solo-track",
            "3",
            "--solo-track",
            "Lead",
            "--mute-track",
            "Kick",
        ])
        .expect("parses");
        let Some(Command::Render(args)) = cli.command else {
            panic!("expected the render subcommand");
        };
        assert_eq!(args.input, PathBuf::from("a project.ptz"));
        assert_eq!(args.output, PathBuf::from("out.wav"));
        assert_eq!(args.sample_rate, 48_000);
        assert_eq!(args.seconds, 10.0);
        assert_eq!(args.tail_seconds, 2.0);
        assert_eq!(args.result_json, Some(PathBuf::from("result.json")));
        // Repeatable, and a bare number is an id while anything else is a name.
        assert_eq!(args.solo_track.len(), 2);
        assert_eq!(args.mute_track.len(), 1);
    }

    /// A bare `--bit-depth 32` must be refused rather than silently selecting
    /// the integer format. "32-bit WAV" usually means the float one, so
    /// accepting the ambiguous spelling would quietly hand back the encoding
    /// the caller did not want — and clip it into the bargain.
    #[test]
    fn an_ambiguous_bit_depth_is_refused() {
        let parse = |depth: &str| {
            Cli::try_parse_from([
                "pertylizer",
                "render",
                "--input",
                "a.ptz",
                "--output",
                "out.wav",
                "--bit-depth",
                depth,
            ])
        };
        assert!(parse("32").is_err(), "a bare 32 must not resolve");
        for depth in ["8", "16", "24", "32i", "32f"] {
            assert!(parse(depth).is_ok(), "{depth} must parse");
        }
    }

    /// Both paths are required: rendering without knowing where the audio goes
    /// is not a partial success worth having.
    #[test]
    fn render_requires_an_input_and_an_output() {
        assert!(Cli::try_parse_from(["pertylizer", "render"]).is_err());
        assert!(Cli::try_parse_from(["pertylizer", "render", "--input", "a.ptz"]).is_err());
    }

    /// Defaults exist so the common invocation is short, but they have to be
    /// the documented ones.
    #[test]
    fn render_defaults_match_the_contract() {
        let cli = Cli::try_parse_from([
            "pertylizer",
            "render",
            "--input",
            "a.ptz",
            "--output",
            "b.wav",
        ])
        .expect("parses");
        let Some(Command::Render(args)) = cli.command else {
            panic!("expected the render subcommand");
        };
        assert_eq!(args.protocol_version, pertylizer::render::PROTOCOL_VERSION);
        assert_eq!(args.sample_rate, 44_100);
        assert_eq!(args.seconds, 10.0);
        assert_eq!(args.tail_seconds, 0.0);
        assert_eq!(args.result_json, None);
        assert!(args.solo_track.is_empty() && args.mute_track.is_empty());
    }

    /// A misspelled subcommand must fail like a misspelled flag does, or a
    /// generated command line could quietly launch the GUI instead of
    /// rendering.
    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["pertylizer", "rendr"]).is_err());
    }
}
