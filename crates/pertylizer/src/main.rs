//! Pertylizer — a modular synthesizer.
//!
//! This is the main entry point for Pertylizer.
//! It supports multiple GUI backends which can be selected via
//! command-line arguments or feature flags.
//!
//! # Usage
//!
//! ```bash
//! # Run with default GUI (egui)
//! cargo run
//!
//! # Run with console interface
//! cargo run -- --gui console
//!
//! # Run with GUI + MCP server on port 9850
//! cargo run --features mcp
//!
//! # Run headless MCP server (stdio)
//! cargo run --features mcp -- --mcp
//!
//! # Or compile with only console support
//! cargo run --no-default-features --features gui-console
//! ```

use std::env;

use pertylizer::audio::{self, AudioHostTrait, BufferSize, ChannelCount, SampleRate, StreamConfig};
use pertylizer::gui::{GuiType, SynthGuiConfig, create_backend, print_available_backends};
use pertylizer::synth_core::VoiceCount;
use pertylizer::synth_engine::{AllocationMode, AllocatorConfig, SynthEngine};

/// Command-line action to perform.
enum CliAction {
    /// Run the GUI (default behavior).
    RunGui(GuiType),
    /// List available backends and exit.
    ListBackends,
    /// Run headless MCP server on stdio (no GUI).
    #[cfg(feature = "mcp")]
    HeadlessMcp,
}

/// Default MCP HTTP port.
#[cfg(feature = "mcp")]
const MCP_PORT: u16 = 9850;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Pertylizer v{} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE"),
    );

    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();
    let action = parse_args(&args)?;

    match action {
        CliAction::ListBackends => {
            print_available_backends();
        }
        CliAction::RunGui(gui_type) => {
            run_gui(gui_type)?;
        }
        #[cfg(feature = "mcp")]
        CliAction::HeadlessMcp => {
            run_headless_mcp()?;
        }
    }

    Ok(())
}

fn run_gui(gui_type: GuiType) -> Result<(), Box<dyn std::error::Error>> {
    // Load persistent settings
    let settings = pertylizer::io::AppSettings::load();

    // Create the synth engine with 8-voice polyphony
    let allocator_config = AllocatorConfig {
        max_voices: VoiceCount::OCTO,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (engine, handle) = SynthEngine::with_config(allocator_config.clone());

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
        let counter = std::sync::Arc::clone(&shared.mcp_active_sessions);
        let shared_for_flag = std::sync::Arc::clone(&shared);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .unwrap_or_else(|e| panic!("Failed to create tokio runtime: {e}"));
            rt.block_on(async {
                shared_for_flag
                    .mcp_listening
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = synth_mcp::serve_http(bridge, MCP_PORT, Some(counter)).await {
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
        settings,
    };

    // Create and run the selected GUI backend
    println!("Starting {} GUI...", gui_type.name());

    let backend = create_backend(gui_type)?;
    backend.run(engine, handle, host, gui_config)?;

    Ok(())
}

/// Run headless MCP server on stdio (no GUI, audio still plays).
#[cfg(feature = "mcp")]
fn run_headless_mcp() -> Result<(), Box<dyn std::error::Error>> {
    let allocator_config = AllocatorConfig {
        max_voices: VoiceCount::OCTO,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (engine, handle) = SynthEngine::with_config(allocator_config);

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

fn parse_args(args: &[String]) -> Result<CliAction, Box<dyn std::error::Error>> {
    let mut gui_type: Option<GuiType> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--gui" | "-g" => {
                if i + 1 < args.len() {
                    let gui_name = &args[i + 1];
                    gui_type = Some(GuiType::from_arg(gui_name).ok_or_else(|| {
                        eprintln!("Unknown GUI type: {}", gui_name);
                        print_available_backends();
                        "Invalid GUI type"
                    })?);
                    i += 1;
                } else {
                    return Err("--gui requires an argument".into());
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--list-backends" => {
                return Ok(CliAction::ListBackends);
            }
            "--console" | "-c" => {
                gui_type = Some(GuiType::Console);
            }
            #[cfg(feature = "mcp")]
            "--mcp" => {
                return Ok(CliAction::HeadlessMcp);
            }
            _ => {
                if args[i].starts_with('-') {
                    eprintln!("Unknown option: {}", args[i]);
                    print_help();
                    std::process::exit(1);
                }
            }
        }
        i += 1;
    }

    // Default to egui if available, otherwise console
    Ok(CliAction::RunGui(gui_type.unwrap_or_else(default_gui_type)))
}

fn default_gui_type() -> GuiType {
    #[cfg(feature = "gui-egui")]
    {
        GuiType::Egui
    }
    #[cfg(all(not(feature = "gui-egui"), feature = "gui-console"))]
    {
        GuiType::Console
    }
    #[cfg(not(any(feature = "gui-egui", feature = "gui-console")))]
    {
        compile_error!("At least one GUI backend must be enabled");
    }
}

fn print_help() {
    println!("Pertylizer");
    println!();
    println!("USAGE:");
    println!("    pertylizer [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -g, --gui <TYPE>       Select GUI backend (egui, console)");
    println!("    -c, --console          Shortcut for --gui console");
    #[cfg(feature = "mcp")]
    println!("    --mcp                  Run headless MCP server on stdio");
    println!("    --list-backends        List available GUI backends");
    println!("    -h, --help             Print this help message");
    println!();
    println!("KEYBOARD (in GUI mode):");
    println!("    Z-M                    Play notes (C3-B3)");
    println!("    Q-I                    Play notes (C4-C5)");
    println!("    2,3,5,6,7              Black keys");
    println!("    -/+                    Shift octave");
    println!();
    println!("EXAMPLES:");
    println!("    pertylizer                      # Run with graphical interface");
    println!("    pertylizer --gui console        # Run with text interface");
    #[cfg(feature = "mcp")]
    println!("    pertylizer --mcp                # Run headless MCP server");
}
