//! Modular Synthesizer Application
//!
//! This is the main entry point for the modular synthesizer.
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
//! # Or compile with only console support
//! cargo run --no-default-features --features gui-console
//! ```

use std::env;

use modular_synth::audio::{
    self, AudioHostTrait, BufferSize, ChannelCount, SampleRate, StreamConfig,
};
use modular_synth::gui::{GuiType, SynthGuiConfig, create_backend, print_available_backends};
use modular_synth::synth_core::VoiceCount;
use modular_synth::synth_engine::{AllocationMode, AllocatorConfig, SynthEngine};

/// Command-line action to perform.
enum CliAction {
    /// Run the GUI (default behavior).
    RunGui(GuiType),
    /// List available backends and exit.
    ListBackends,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    }

    Ok(())
}

fn run_gui(gui_type: GuiType) -> Result<(), Box<dyn std::error::Error>> {
    // Create the synth engine with 8-voice polyphony
    let allocator_config = AllocatorConfig {
        max_voices: VoiceCount::OCTO,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (engine, handle) = SynthEngine::with_config(allocator_config.clone());

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
        title: "Modular Synthesizer".to_string(),
        width: 1280,
        height: 800,
        allocator_config,
        stream_config,
    };

    // Create and run the selected GUI backend
    println!("Starting {} GUI...", gui_type.name());

    let backend = create_backend(gui_type)?;
    backend.run(engine, handle, host, gui_config)?;

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
    println!("Modular Synthesizer");
    println!();
    println!("USAGE:");
    println!("    modular-synth [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -g, --gui <TYPE>       Select GUI backend (egui, console)");
    println!("    -c, --console          Shortcut for --gui console");
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
    println!("    modular-synth                      # Run with graphical interface");
    println!("    modular-synth --gui console        # Run with text interface");
}
