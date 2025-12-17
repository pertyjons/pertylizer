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
//! # Debug import a tracker file
//! cargo run -- --debug-import /path/to/song.xm
//!
//! # Or compile with only console support
//! cargo run --no-default-features --features gui-console
//! ```

use std::env;
use std::path::Path;

use modular_synth::audio::{
    self, AudioHostTrait, BufferSize, ChannelCount, SampleRate, StreamConfig,
};
use modular_synth::gui::{GuiType, SynthGuiConfig, create_backend, print_available_backends};
use modular_synth::io::import::{SongImporter, TrackerImporter};
use modular_synth::synth_core::VoiceCount;
use modular_synth::synth_engine::{AllocationMode, AllocatorConfig, SynthEngine};

/// Command-line action to perform.
enum CliAction {
    /// Run the GUI (default behavior).
    RunGui(GuiType),
    /// Import a tracker file and print debug information.
    DebugImport(String),
    /// List available backends and exit.
    ListBackends,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();
    let action = parse_args(&args)?;

    match action {
        CliAction::DebugImport(path) => {
            run_debug_import(&path)?;
        }
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
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--debug-import" | "-d" => {
                if i + 1 < args.len() {
                    return Ok(CliAction::DebugImport(args[i + 1].clone()));
                }
                return Err("--debug-import requires a file path".into());
            }
            "--gui" | "-g" => {
                if i + 1 < args.len() {
                    let gui_name = &args[i + 1];
                    let gui_type = GuiType::from_arg(gui_name).ok_or_else(|| {
                        eprintln!("Unknown GUI type: {}", gui_name);
                        print_available_backends();
                        "Invalid GUI type"
                    })?;
                    return Ok(CliAction::RunGui(gui_type));
                }
                return Err("--gui requires an argument".into());
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--list-backends" => {
                return Ok(CliAction::ListBackends);
            }
            "--console" | "-c" => {
                return Ok(CliAction::RunGui(GuiType::Console));
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
    Ok(CliAction::RunGui(default_gui_type()))
}

fn run_debug_import(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== DEBUG XM IMPORT ===");
    println!("File: {}\n", path);

    let importer = TrackerImporter::new();
    let imported = importer.import(Path::new(path))?;

    println!("=== IMPORT SUCCESS ===");
    println!("Song name: '{}'", imported.song.name);
    println!(
        "Default tempo: {} BPM",
        imported.song.default_tempo.as_f32()
    );
    println!("Samples: {}", imported.samples.len());
    println!("Instruments: {}", imported.instruments.len());
    println!("Patterns: {}", imported.song.pattern_count());

    println!("\n=== INSTRUMENTS ===");
    for (idx, inst) in imported.instruments.iter().enumerate() {
        let sample_info = inst
            .sample_index
            .map(|i| format!("sample[{}]", i))
            .unwrap_or_else(|| "NO SAMPLE".to_string());
        println!(
            "  [{:02}] '{}' - {} (vol={:.2})",
            idx,
            inst.name,
            sample_info,
            inst.global_volume.as_f32()
        );
    }

    println!("\n=== SAMPLES ===");
    for (idx, smp) in imported.samples.iter().enumerate() {
        let loop_info = smp
            .loop_info
            .as_ref()
            .map(|li| {
                format!(
                    "loop={} ({:.2}-{:.2})",
                    li.enabled, li.loop_start, li.loop_end
                )
            })
            .unwrap_or_else(|| "no loop".to_string());
        println!(
            "  [{:02}] {} frames, {:?} - {}",
            idx,
            smp.len().0,
            smp.channels,
            loop_info
        );
    }

    println!("\n=== ARRANGEMENT ===");
    for (idx, placement) in imported.song.arrangement().iter().take(10).enumerate() {
        println!(
            "  [{}] Pattern {:?} at tick {}",
            idx, placement.pattern_id, placement.start.0
        );
    }
    if imported.song.arrangement().len() > 10 {
        println!("  ... ({} more)", imported.song.arrangement().len() - 10);
    }

    println!("\n=== FIRST PATTERN ===");
    let first_arrangement = imported.song.arrangement().first();
    let first_pattern = first_arrangement.and_then(|p| imported.song.pattern(p.pattern_id));
    if let Some(pattern) = first_pattern {
        println!("Pattern ID: {:?}", pattern.id);
        println!("Length: {} ticks", pattern.length.0);
        println!("Tracks: {}", pattern.num_tracks());
        println!("Notes: {}", pattern.notes().len());
        println!("Effect-only events: {}", pattern.effect_events().len());

        println!("\nFirst 20 notes:");
        for (i, note) in pattern.notes().iter().take(20).enumerate() {
            let duration_info = note
                .duration
                .map(|d| format!(", dur={}", d.0))
                .unwrap_or_default();
            println!(
                "  [{:02}] tick={}, {} (MIDI {}), vel={:.2}, inst={:?}, track={:?}{}",
                i,
                note.start.0,
                note.pitch,
                note.pitch.as_midi(),
                note.velocity.as_f32(),
                note.instrument,
                note.track,
                duration_info
            );
        }
    }

    // Check for potential issues
    println!("\n=== POTENTIAL ISSUES ===");

    // Check if instruments have samples
    let instruments_without_samples = imported
        .instruments
        .iter()
        .filter(|inst| inst.sample_index.is_none())
        .count();
    if instruments_without_samples > 0 {
        println!(
            "WARNING: {} instruments have no sample!",
            instruments_without_samples
        );
    }

    // Check if any notes reference non-existent instruments
    if let Some(pattern) = imported.song.patterns().next() {
        let max_inst = imported.instruments.len();
        let bad_refs: Vec<_> = pattern
            .notes()
            .iter()
            .filter(|n| n.instrument.0 as usize >= max_inst)
            .collect();
        if !bad_refs.is_empty() {
            println!(
                "WARNING: {} notes reference instruments >= {} (out of bounds)!",
                bad_refs.len(),
                max_inst
            );
            for note in bad_refs.iter().take(5) {
                println!(
                    "  - Note at tick {}: inst={:?}",
                    note.start.0, note.instrument
                );
            }
        }
    }

    // Check sample sizes
    let empty_samples = imported.samples.iter().filter(|s| s.len().0 == 0).count();
    if empty_samples > 0 {
        println!("WARNING: {} samples are empty!", empty_samples);
    }

    // No warnings
    if instruments_without_samples == 0 && empty_samples == 0 {
        println!("No issues detected.");
    }

    println!("\n=== DONE ===");
    Ok(())
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
    println!("Modular Synthesizer v0.6.0");
    println!();
    println!("USAGE:");
    println!("    modular-synth [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -g, --gui <TYPE>       Select GUI backend (egui, console)");
    println!("    -c, --console          Shortcut for --gui console");
    println!("    -d, --debug-import <FILE>");
    println!("                           Import tracker file and print debug info");
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
    println!("    modular-synth -d song.xm           # Debug import tracker file");
}
