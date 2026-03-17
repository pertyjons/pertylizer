//! Pertylizer — a modular synthesizer.
//!
//! # Usage
//!
//! ```bash
//! # Run with GUI (MCP on port 9850 + OSC telemetry enabled by default)
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

/// Default MCP HTTP port.
#[cfg(feature = "mcp")]
const MCP_PORT: u16 = 9850;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Pertylizer v{} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE"),
    );

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
        let mut osc = synth_osc::OscTelemetry::new(synth_osc::OscConfig::default());
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
    let song = std::sync::Arc::new(std::sync::RwLock::new(synth_sequencer::Song::new(
        "Untitled",
    )));

    // Send the shared Song to the engine so sequencer shares the same instance
    let _ = handle
        .command_sender()
        .send(pertylizer::synth_engine::EngineCommand::SetSong {
            song: std::sync::Arc::clone(&song),
        });

    // Start MCP HTTP server in background (if feature enabled)
    #[cfg(feature = "mcp")]
    let mcp_shared = {
        let shared = std::sync::Arc::new(pertylizer::mcp_shared::McpSharedState::with_song(
            std::sync::Arc::clone(&song),
        ));
        let bridge = std::sync::Arc::new(pertylizer::mcp_bridge::AppSynthBridge::new(
            std::sync::Arc::clone(&session),
            std::sync::Arc::clone(&shared),
        ));
        let registry = shared.mcp_sessions.clone();
        let shared_for_flag = std::sync::Arc::clone(&shared);
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("Failed to create tokio runtime for MCP: {e}");
                    return;
                }
            };
            rt.block_on(async {
                shared_for_flag
                    .mcp_listening
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = synth_mcp::serve_http(bridge, MCP_PORT, Some(registry)).await {
                    eprintln!("MCP server error: {e}");
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
    };

    let backend = create_backend();
    backend.run(engine, handle, host, gui_config)?;

    Ok(())
}

/// Run headless MCP server on stdio (no GUI, audio still plays).
#[cfg(feature = "mcp")]
fn run_headless_mcp() -> Result<(), Box<dyn std::error::Error>> {
    let (engine, handle) = SynthEngine::new();

    // Start audio
    let mut host: Box<dyn AudioHostTrait> = match audio::default_host() {
        Ok(h) => {
            eprintln!("✓ Audio backend: {}", h.backend_name());
            Box::new(h)
        }
        Err(e) => {
            eprintln!("⚠ Could not initialize audio: {e}");
            Box::new(audio::null_host())
        }
    };

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
    let bridge = std::sync::Arc::new(pertylizer::mcp_bridge::AppSynthBridge::new(
        std::sync::Arc::clone(&session),
        std::sync::Arc::clone(&shared),
    ));

    // Run MCP on stdio (blocking)
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(synth_mcp::serve_stdio(bridge))?;

    // Keep host alive until MCP exits
    host.stop()?;

    Ok(())
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
