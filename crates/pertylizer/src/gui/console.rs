//! Console (text-based) GUI backend.
//!
//! This provides a simple terminal interface for the synthesizer,
//! useful for testing and headless operation.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use crate::audio::AudioHostTrait;
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use synth_core::{MidiNote, NormalizedValue};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, SynthEngine};

/// Console-based GUI backend.
pub struct ConsoleBackend;

impl ConsoleBackend {
    /// Create a new console backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConsoleBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl GuiBackend for ConsoleBackend {
    fn name(&self) -> &'static str {
        "console"
    }

    fn run(
        self: Box<Self>,
        engine: SynthEngine,
        mut handle: EngineHandle,
        mut host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
    ) -> GuiResult<()> {
        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║          Pertylizer - Rust 2024 Edition (Polyphonic)          ║");
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!();

        // Show audio backend info
        println!("✓ Audio backend: {}", host.backend_name());

        // List available devices
        if let Ok(devices) = host.devices() {
            println!("\nAvailable audio devices:");
            for (i, device) in devices.iter().enumerate() {
                println!("  {i}: {} ({:?})", device.name, device.device_type);
            }
        }

        println!("\nStarting audio stream...");
        println!("  Sample rate: {} Hz", config.stream_config.sample_rate.0);
        println!(
            "  Buffer size: {} frames",
            config.stream_config.buffer_size.0
        );
        println!(
            "  Estimated latency: {:.1} ms",
            config
                .stream_config
                .buffer_size
                .latency_at(config.stream_config.sample_rate)
                .as_secs_f64()
                * 1000.0
        );
        println!(
            "  Max polyphony: {} voices",
            config.allocator_config.max_voices
        );

        // Start the audio stream
        match host.start_output(None, &config.stream_config, Box::new(engine)) {
            Ok(info) => {
                println!("✓ Audio stream started");
                println!(
                    "  Actual latency: {:.1} ms",
                    info.output_latency.as_secs_f64() * 1000.0
                );
            }
            Err(e) => {
                eprintln!("✗ Failed to start audio: {e}");
                return Err(e.into());
            }
        }

        print_help();

        let stdin = io::stdin();
        let mut stdout = io::stdout();

        loop {
            print!("> ");
            stdout.flush()?;

            let mut line = String::new();
            if stdin.lock().read_line(&mut line)? == 0 {
                break;
            }

            let parts: Vec<&str> = line.trim().split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "note" | "n" => handle_note(&mut handle, &parts),
                "off" => handle_note_off(&mut handle, &parts),
                "chord" => handle_chord(&mut handle, &parts),
                "vol" | "volume" => handle_volume(&mut handle, &parts),
                "voices" | "v" => println!("Active voices: {}", handle.voice_count()),
                "meters" | "m" => print_meters(&handle),
                "cpu" => println!("CPU usage: {:.1}%", handle.cpu_usage() * 100.0),
                "panic" => {
                    handle.send(EngineCommand::Reset);
                    println!("! All voices killed");
                }
                "latency" => {
                    if let Some(latency) = host.latency() {
                        println!("Output latency: {:.1} ms", latency.as_secs_f64() * 1000.0);
                    } else {
                        println!("No active stream");
                    }
                }
                "quit" | "exit" | "q" => {
                    println!("Stopping audio...");
                    break;
                }
                "help" | "?" => print_help_short(),
                _ => println!("Unknown command. Type 'help' for available commands."),
            }

            // Poll for engine events
            while let Some(event) = handle.poll_event() {
                if let EngineEvent::BufferUnderrun = event {
                    println!("⚠ Buffer underrun!");
                }
            }
        }

        // Clean shutdown
        host.stop()?;
        println!("Goodbye!");

        std::thread::sleep(Duration::from_millis(100));

        Ok(())
    }
}

fn handle_note(handle: &mut EngineHandle, parts: &[&str]) {
    if let Some(&note_str) = parts.get(1) {
        if let Ok(note) = note_str.parse::<u8>() {
            let velocity = parts
                .get(2)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.8);
            handle.note_on(MidiNote(note), NormalizedValue::new(velocity));
            let freq = 440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0);
            println!("♪ Note on: {note} ({freq:.1} Hz), velocity: {velocity}");
            println!("  Active voices: {}", handle.voice_count());
        } else {
            println!("Invalid note number");
        }
    } else {
        println!("Usage: note <0-127> [velocity]");
    }
}

fn handle_note_off(handle: &mut EngineHandle, parts: &[&str]) {
    if let Some(&note_str) = parts.get(1) {
        if let Ok(note) = note_str.parse::<u8>() {
            handle.note_off(MidiNote(note));
            println!("♪ Note off: {note}");
        }
    } else {
        handle.send(EngineCommand::AllNotesOff);
        println!("♪ All notes off");
    }
    println!("  Active voices: {}", handle.voice_count());
}

fn handle_chord(handle: &mut EngineHandle, parts: &[&str]) {
    if parts.len() >= 3 {
        if let Ok(root) = parts[1].parse::<u8>() {
            let chord_type = parts[2];
            let notes: Vec<u8> = match chord_type {
                "maj" | "major" => vec![root, root + 4, root + 7],
                "min" | "minor" => vec![root, root + 3, root + 7],
                "7" | "dom7" => vec![root, root + 4, root + 7, root + 10],
                "maj7" => vec![root, root + 4, root + 7, root + 11],
                "min7" => vec![root, root + 3, root + 7, root + 10],
                "dim" => vec![root, root + 3, root + 6],
                "aug" => vec![root, root + 4, root + 8],
                _ => {
                    println!("Unknown chord type. Use: maj, min, 7, maj7, min7, dim, aug");
                    return;
                }
            };

            for note in &notes {
                handle.note_on(MidiNote(*note), NormalizedValue::new(0.7));
            }
            println!("♪ Chord: {} {} ({:?})", root, chord_type, notes);
            println!("  Active voices: {}", handle.voice_count());
        }
    } else {
        println!("Usage: chord <root_note> <type>");
        println!("  Types: maj, min, 7, maj7, min7, dim, aug");
    }
}

fn handle_volume(handle: &mut EngineHandle, parts: &[&str]) {
    if let Some(&vol_str) = parts.get(1) {
        if let Ok(vol) = vol_str.parse::<f32>() {
            handle.set_master_volume(synth_core::Gain::new(vol));
            println!("Volume: {:.0}%", vol * 100.0);
        }
    } else {
        println!("Current volume: {:.0}%", handle.master_volume() * 100.0);
    }
}

fn print_meters(handle: &EngineHandle) {
    let (peak_l, peak_r) = handle.peak_meters();
    let (rms_l, rms_r) = handle.rms_meters();
    println!("Peak: L={peak_l:>6.3} R={peak_r:>6.3}");
    println!("RMS:  L={rms_l:>6.3} R={rms_r:>6.3}");
    println!("Voices: {}", handle.voice_count());
}

fn print_help() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                        Commands                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  note <midi_note> [velocity]  - Play a note (0-127)          ║");
    println!("║  off [note]                   - Note off (or all)            ║");
    println!("║  chord <root> <type>          - Play chord (maj/min/7)       ║");
    println!("║  vol <0.0-1.0>                - Set master volume            ║");
    println!("║  voices                       - Show active voice count      ║");
    println!("║  meters                       - Show current meters          ║");
    println!("║  cpu                          - Show CPU usage               ║");
    println!("║  panic                        - Kill all voices              ║");
    println!("║  quit                         - Exit                         ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}

fn print_help_short() {
    println!("Commands:");
    println!("  note <n> [vel]  - Play MIDI note");
    println!("  off [note]      - Note off");
    println!("  chord <n> <t>   - Play chord");
    println!("  vol <0-1>       - Volume");
    println!("  voices          - Voice count");
    println!("  meters          - Audio levels");
    println!("  cpu             - CPU usage");
    println!("  panic           - Kill all");
    println!("  quit            - Exit");
}
