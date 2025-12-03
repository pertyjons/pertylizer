//! Simple MIDI test to verify midir is working correctly.
//!
//! Run with: cargo run --example midi_test

use std::io::{stdin, stdout, Write};
use std::error::Error;

use midir::{Ignore, MidiInput};

fn main() -> Result<(), Box<dyn Error>> {
    println!("=== MIDI Input Test ===\n");

    let mut midi_in = MidiInput::new("midi-test")?;
    midi_in.ignore(Ignore::None); // Don't ignore any messages

    let ports = midi_in.ports();
    println!("Available MIDI input ports ({}):", ports.len());

    if ports.is_empty() {
        println!("  (no ports found)");
        println!("\nTroubleshooting:");
        println!("  - Is your MIDI device connected?");
        println!("  - On Linux, check: aconnect -l");
        println!("  - On Linux, you may need to install: alsa-utils");
        return Ok(());
    }

    for (i, port) in ports.iter().enumerate() {
        let name = midi_in.port_name(port).unwrap_or_else(|_| "Unknown".to_string());
        println!("  [{}] {}", i, name);
    }

    println!("\nSelect port number (or press Enter for port 0): ");
    stdout().flush()?;

    let mut input = String::new();
    stdin().read_line(&mut input)?;
    let port_num: usize = input.trim().parse().unwrap_or(0);

    if port_num >= ports.len() {
        println!("Invalid port number");
        return Ok(());
    }

    let port = &ports[port_num];
    let port_name = midi_in.port_name(port)?;

    println!("\nConnecting to: {}", port_name);
    println!("Play notes on your MIDI device. Press Enter to exit.\n");

    // Connect and start receiving
    let _conn = midi_in.connect(
        port,
        "midi-test-input",
        |timestamp, message, _| {
            print!("[{:>10}] ", timestamp);
            for byte in message {
                print!("{:02X} ", byte);
            }

            // Decode message type
            if !message.is_empty() {
                let status = message[0];
                let msg_type = status & 0xF0;
                let channel = status & 0x0F;

                let description = match msg_type {
                    0x80 => format!("Note Off ch={} note={} vel={}", channel + 1, message.get(1).unwrap_or(&0), message.get(2).unwrap_or(&0)),
                    0x90 => {
                        let vel = message.get(2).unwrap_or(&0);
                        if *vel == 0 {
                            format!("Note Off (vel=0) ch={} note={}", channel + 1, message.get(1).unwrap_or(&0))
                        } else {
                            format!("Note On ch={} note={} vel={}", channel + 1, message.get(1).unwrap_or(&0), vel)
                        }
                    },
                    0xA0 => format!("Poly AT ch={}", channel + 1),
                    0xB0 => format!("CC ch={} cc={} val={}", channel + 1, message.get(1).unwrap_or(&0), message.get(2).unwrap_or(&0)),
                    0xC0 => format!("Program ch={}", channel + 1),
                    0xD0 => format!("Channel AT ch={}", channel + 1),
                    0xE0 => format!("Pitch Bend ch={}", channel + 1),
                    0xF0 => "System".to_string(),
                    _ => "Unknown".to_string(),
                };
                print!(" - {}", description);
            }
            println!();
            let _ = stdout().flush();
        },
        (),
    )?;

    println!("Connected! Waiting for MIDI input...\n");

    // Wait for Enter
    let mut input = String::new();
    stdin().read_line(&mut input)?;

    println!("Closing connection.");
    Ok(())
}
