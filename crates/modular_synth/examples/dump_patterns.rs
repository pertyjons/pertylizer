//! Dump XM patterns in tracker-style format.
//!
//! Usage: cargo run --example dump_patterns -- /path/to/file.xm [num_patterns] [start_channel] [end_channel]
//!
//! Examples:
//!   cargo run --example dump_patterns -- file.xm 5          # First 5 patterns, channels 0-7
//!   cargo run --example dump_patterns -- file.xm 3 12 15    # First 3 patterns, channels 12-15

#![allow(clippy::expect_used)]

use std::env;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::pitch::Pitch;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| "test.xm".to_string());
    let num_patterns: usize = env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let start_channel: usize = env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let end_channel: Option<usize> = env::args().nth(4).and_then(|s| s.parse().ok());

    let data = std::fs::read(&path).expect("Failed to read file");
    let xm = XmModule::load(&data).expect("Failed to parse XM");

    println!("=== {} ===", xm.header.name.trim());
    println!("Channels: {}", xm.header.number_of_channels);
    println!("Patterns: {}", xm.pattern.len());
    println!(
        "BPM: {}, Speed: {}",
        xm.header.default_bpm, xm.header.default_tempo
    );
    println!();

    for (pat_idx, pattern) in xm.pattern.iter().take(num_patterns).enumerate() {
        let num_rows = pattern.pattern.len();
        let num_channels = if num_rows > 0 {
            pattern.pattern[0].len()
        } else {
            0
        };

        // Determine channel range to display
        let ch_start = start_channel.min(num_channels);
        let ch_end = end_channel.unwrap_or(ch_start + 8).min(num_channels);

        println!(
            "╔══════════════════════════════════════════════════════════════════════════════════════════════════════════════╗"
        );
        println!(
            "║ Pattern {:02X} ({} rows, {} channels) - showing channels {}-{}",
            pat_idx,
            num_rows,
            num_channels,
            ch_start,
            ch_end - 1
        );
        println!(
            "╠══════════════════════════════════════════════════════════════════════════════════════════════════════════════╣"
        );

        // Print header with channel numbers
        print!("║ Row │");
        for ch in ch_start..ch_end {
            print!("  Ch{:02}  │", ch);
        }
        println!();
        println!(
            "╟─────┼─────────────────────────────────────────────────────────────────────────────────────────────────────────╢"
        );

        for (row_idx, row) in pattern.pattern.iter().enumerate() {
            print!("║ {:3} │", row_idx);

            for ch in ch_start..ch_end {
                if let Some(slot) = row.get(ch) {
                    let note_str = format_note(slot.note);
                    let inst_str = if let Some(inst) = slot.instrument {
                        format!("{:02X}", inst)
                    } else {
                        "--".to_string()
                    };
                    let vol_str = format_volume(slot.volume);
                    let effect_str = format_effect(slot.effect_type, slot.effect_parameter);

                    print!(" {}{} {} {} │", note_str, inst_str, vol_str, effect_str);
                }
            }
            println!();
        }
        println!(
            "╚══════════════════════════════════════════════════════════════════════════════════════════════════════════════╝"
        );
        println!();
    }
}

fn format_note(pitch: Pitch) -> String {
    if pitch.is_none() {
        "--- ".to_string()
    } else if pitch.is_keyoff() {
        "=== ".to_string()
    } else {
        let note_val = pitch as u8;
        let names = [
            "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
        ];
        let octave = note_val / 12;
        let note = (note_val % 12) as usize;
        format!("{}{} ", names[note], octave)
    }
}

#[allow(clippy::manual_range_contains)]
fn format_volume(vol: u8) -> String {
    if vol == 0 {
        "---".to_string()
    } else if vol >= 0x10 && vol <= 0x50 {
        // Set volume (0x10-0x50 = volume 0-64)
        format!("v{:02X}", vol - 0x10)
    } else if vol >= 0x60 && vol < 0x70 {
        // Volume slide down
        format!("d{:02}", vol & 0x0F)
    } else if vol >= 0x70 && vol < 0x80 {
        // Volume slide up
        format!("u{:02}", vol & 0x0F)
    } else if vol >= 0x80 && vol < 0x90 {
        // Fine volume down
        format!("D{:02}", vol & 0x0F)
    } else if vol >= 0x90 && vol < 0xA0 {
        // Fine volume up
        format!("U{:02}", vol & 0x0F)
    } else if vol >= 0xA0 && vol < 0xB0 {
        // Set vibrato speed
        format!("S{:02}", vol & 0x0F)
    } else if vol >= 0xB0 && vol < 0xC0 {
        // Vibrato
        format!("V{:02}", vol & 0x0F)
    } else if vol >= 0xC0 && vol < 0xD0 {
        // Set panning
        format!("p{:02X}", (vol & 0x0F) << 4)
    } else if vol >= 0xD0 && vol < 0xE0 {
        // Panning slide left
        format!("<{:02}", vol & 0x0F)
    } else if vol >= 0xE0 && vol < 0xF0 {
        // Panning slide right
        format!(">{:02}", vol & 0x0F)
    } else if vol >= 0xF0 {
        // Tone portamento
        format!("M{:02}", vol & 0x0F)
    } else {
        format!("{:03}", vol)
    }
}

fn format_effect(effect_type: u8, param: u8) -> String {
    if effect_type == 0 && param == 0 {
        return "---".to_string();
    }

    let effect_char = match effect_type {
        0 => '0',  // Arpeggio
        1 => '1',  // Portamento up
        2 => '2',  // Portamento down
        3 => '3',  // Tone portamento
        4 => '4',  // Vibrato
        5 => '5',  // Tone porta + volume slide
        6 => '6',  // Vibrato + volume slide
        7 => '7',  // Tremolo
        8 => '8',  // Set panning
        9 => '9',  // Sample offset
        10 => 'A', // Volume slide
        11 => 'B', // Position jump
        12 => 'C', // Set volume
        13 => 'D', // Pattern break
        14 => 'E', // Extended effects
        15 => 'F', // Set tempo/speed
        16 => 'G', // Set global volume
        17 => 'H', // Global volume slide
        20 => 'K', // Key off
        21 => 'L', // Set envelope position
        25 => 'P', // Panning slide
        27 => 'R', // Retrigger
        29 => 'T', // Tremor
        33 => 'X', // Extra fine portamento
        _ => '?',
    };

    format!("{}{:02X}", effect_char, param)
}
