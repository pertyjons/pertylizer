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

use std::env;

use pertylizer::audio::{self, AudioHostTrait, BufferSize, ChannelCount, SampleRate, StreamConfig};
use pertylizer::gui::{SynthGuiConfig, create_backend};
use pertylizer::synth_core::VoiceCount;
use pertylizer::synth_engine::{AllocationMode, AllocatorConfig, SynthEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialise tracing FIRST. Writer is locked to stderr so --headless mode
    // (which speaks JSON-RPC over stdout) is not corrupted by log output.
    // Default filter keeps our crates at info and silences chatty deps; the
    // RUST_LOG env var overrides if set, e.g.
    //   RUST_LOG=synth_mcp=debug,pertylizer=info
    init_tracing();

    // stderr so stdout stays a clean JSON-RPC channel in --headless mode.
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

    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    #[cfg(feature = "mcp")]
    if args.iter().any(|a| a == "--headless") {
        return run_headless_mcp();
    }

    run_gui()
}

fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

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
    let (_osc_telemetry, osc_shared) = if args.iter().any(|a| a == "--no-osc") {
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
        std::thread::spawn(move || {
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
        sample_rate: SampleRate::DVD_QUALITY,
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
        sample_rate: SampleRate::DVD_QUALITY,
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

/// Install a `tracing` subscriber that writes to stderr.
///
/// Headless mode uses stdout for JSON-RPC, so logs MUST go to stderr to avoid
/// corrupting the MCP protocol stream. Default filter shows `info` for our
/// crates and silences chatty dependencies; override with `RUST_LOG`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn,pertylizer=info,synth_mcp=info,synth_engine=info")
    });
    let result = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
    if let Err(e) = result {
        eprintln!("warning: failed to install tracing subscriber: {e}");
    }
}

fn print_help() {
    println!("Pertylizer — a modular synthesizer");
    println!();
    println!("USAGE:");
    println!("    pertylizer [OPTIONS]");
    println!();
    println!("OPTIONS:");
    #[cfg(feature = "mcp")]
    println!("    --headless    Run without GUI (MCP server on stdio)");
    #[cfg(feature = "osc")]
    println!("    --no-osc      Disable OSC telemetry output (enabled by default)");
    println!("    -h, --help    Print this help message");
    println!();
    println!("KEYBOARD:");
    println!("    Z-M           Play notes (C3-B3)");
    println!("    Q-I           Play notes (C4-C5)");
    println!("    2,3,5,6,7     Black keys");
    println!("    -/+           Shift octave");
}
