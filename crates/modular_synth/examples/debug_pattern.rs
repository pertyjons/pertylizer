//! Debug a specific pattern to find volume/pitch issues.
//!
//! Usage: cargo run --release --example debug_pattern -- /path/to/file.xm [position]

#![allow(clippy::expect_used)]

use std::env;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::pitch::Pitch;

fn pitch_to_string(note: &Pitch) -> String {
    match note {
        Pitch::None => "...".to_string(),
        Pitch::Off => "===".to_string(),
        _ => format!("{:?}", note),
    }
}

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/per/Musik/joli_suspiria.xm".to_string());
    let target_position: usize = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    let data = std::fs::read(&path).expect("read file");
    let xm = XmModule::load(&data).expect("parse xm");

    println!("=== DEBUG XM PATTERN AT POSITION {} ===", target_position);
    println!("File: {}", path);
    println!("Song: {}", xm.header.name);
    println!("Default speed: {}", xm.header.default_tempo);
    println!("Default BPM: {}", xm.header.default_bpm);
    println!("Channels: {}", xm.header.number_of_channels);
    println!("Patterns: {}", xm.pattern.len());
    println!("Instruments: {}", xm.instrument.len());
    println!();

    // Show song order
    println!("=== SONG ORDER (first 15) ===");
    for (i, pat_idx) in xm.pattern_order.iter().take(15).enumerate() {
        let marker = if i == target_position { " <-- TARGET" } else { "" };
        println!("  Position {:02}: Pattern {:02}{}", i, pat_idx, marker);
    }
    println!();

    // Get the pattern index
    if target_position >= xm.pattern_order.len() {
        println!("ERROR: Position {} is out of bounds (max: {})",
                 target_position, xm.pattern_order.len() - 1);
        return;
    }

    let actual_pattern = xm.pattern_order[target_position] as usize;
    println!("Position {} uses Pattern {}", target_position, actual_pattern);

    if actual_pattern >= xm.pattern.len() {
        println!("ERROR: Pattern {} doesn't exist", actual_pattern);
        return;
    }

    let xm_pattern = &xm.pattern[actual_pattern];
    println!("Pattern {} has {} rows", actual_pattern, xm_pattern.pattern.len());
    println!();

    // Show instruments used in this pattern
    println!("=== INSTRUMENTS USED ===");
    let mut instr_used: Vec<usize> = Vec::new();
    for row in &xm_pattern.pattern {
        for slot in row {
            if let Some(inst) = slot.instrument {
                if !instr_used.contains(&inst) {
                    instr_used.push(inst);
                }
            }
        }
    }
    instr_used.sort();

    for inst_num in &instr_used {
        if *inst_num < xm.instrument.len() {
            let inst = &xm.instrument[*inst_num];
            let sample_count = inst.sample.len();
            let sample_info = if sample_count == 0 {
                "NO SAMPLES".to_string()
            } else {
                format!("{} sample(s)", sample_count)
            };
            println!("  Inst {:02}: '{}' - {}", inst_num, inst.header.name, sample_info);
        } else {
            println!("  Inst {:02}: OUT OF BOUNDS!", inst_num);
        }
    }
    println!();

    // Analyze all rows
    println!("=== PATTERN {} FULL DATA ===", actual_pattern);

    for (row_idx, row) in xm_pattern.pattern.iter().enumerate() {
        let mut row_output = format!("R{:02}:", row_idx);
        let mut has_data = false;

        for (ch, slot) in row.iter().enumerate() {
            let has_note = !matches!(slot.note, Pitch::None);
            let has_instr = slot.instrument.is_some();
            let has_vol = slot.volume != 0;
            let has_fx = slot.effect_type != 0 || slot.effect_parameter != 0;

            if has_note || has_instr || has_vol || has_fx {
                has_data = true;

                let note_str = pitch_to_string(&slot.note);
                let instr_str = slot.instrument
                    .map(|i| format!("{:02}", i))
                    .unwrap_or_else(|| "..".to_string());

                // Volume column decode
                let vol_str = if slot.volume == 0 {
                    "..".to_string()
                } else if slot.volume >= 0x10 && slot.volume <= 0x50 {
                    format!("v{:02}", slot.volume - 0x10)  // Set volume
                } else if slot.volume >= 0x60 && slot.volume < 0x70 {
                    format!("d{:X}", slot.volume & 0x0F)   // Vol slide down
                } else if slot.volume >= 0x70 && slot.volume < 0x80 {
                    format!("u{:X}", slot.volume & 0x0F)   // Vol slide up
                } else if slot.volume >= 0x80 && slot.volume < 0x90 {
                    format!("fd{:X}", slot.volume & 0x0F)  // Fine vol down
                } else if slot.volume >= 0x90 && slot.volume < 0xA0 {
                    format!("fu{:X}", slot.volume & 0x0F)  // Fine vol up
                } else if slot.volume >= 0xA0 && slot.volume < 0xB0 {
                    format!("vs{:X}", slot.volume & 0x0F)  // Vibrato speed
                } else if slot.volume >= 0xB0 && slot.volume < 0xC0 {
                    format!("vd{:X}", slot.volume & 0x0F)  // Vibrato depth
                } else if slot.volume >= 0xC0 && slot.volume < 0xD0 {
                    format!("p{:X}", slot.volume & 0x0F)   // Set pan
                } else if slot.volume >= 0xD0 && slot.volume < 0xE0 {
                    format!("pl{:X}", slot.volume & 0x0F)  // Pan slide left
                } else if slot.volume >= 0xE0 && slot.volume < 0xF0 {
                    format!("pr{:X}", slot.volume & 0x0F)  // Pan slide right
                } else if slot.volume >= 0xF0 {
                    format!("M{:X}", slot.volume & 0x0F)   // Tone porta
                } else {
                    format!("?{:02X}", slot.volume)
                };

                // Effect decode
                let fx_str = if slot.effect_type != 0 || slot.effect_parameter != 0 {
                    format!("{:X}{:02X}", slot.effect_type, slot.effect_parameter)
                } else {
                    "...".to_string()
                };

                row_output.push_str(&format!(
                    " [{:02}|{} {} {} {}]",
                    ch, note_str, instr_str, vol_str, fx_str
                ));
            }
        }

        if has_data {
            println!("{}", row_output);
        }
    }

    // Focus on volume effects
    println!();
    println!("=== VOLUME-RELATED EFFECTS ===");

    for (row_idx, row) in xm_pattern.pattern.iter().enumerate() {
        for (ch, slot) in row.iter().enumerate() {
            // Check for volume column effects
            if slot.volume >= 0x60 {
                println!("  R{:02} Ch{:02}: Volume column 0x{:02X}", row_idx, ch, slot.volume);
            }

            // Check for volume-related effects
            match slot.effect_type {
                0x0A => println!("  R{:02} Ch{:02}: A{:02X} Volume slide", row_idx, ch, slot.effect_parameter),
                0x0C => println!("  R{:02} Ch{:02}: C{:02X} Set volume", row_idx, ch, slot.effect_parameter),
                0x10 => println!("  R{:02} Ch{:02}: G{:02X} Set global volume", row_idx, ch, slot.effect_parameter),
                0x11 => println!("  R{:02} Ch{:02}: H{:02X} Global volume slide", row_idx, ch, slot.effect_parameter),
                _ => {}
            }
        }
    }

    // Check for tempo/speed changes
    println!();
    println!("=== TEMPO/SPEED CHANGES ===");
    for (row_idx, row) in xm_pattern.pattern.iter().enumerate() {
        for (ch, slot) in row.iter().enumerate() {
            if slot.effect_type == 0x0F {
                let param = slot.effect_parameter;
                if param < 0x20 {
                    println!("  R{:02} Ch{:02}: F{:02X} Set speed to {}", row_idx, ch, param, param);
                } else {
                    println!("  R{:02} Ch{:02}: F{:02X} Set BPM to {}", row_idx, ch, param, param);
                }
            }
        }
    }

    // Check for patterns leading into this one
    println!();
    println!("=== CONTEXT: Previous patterns ===");
    for pos in 0..=target_position {
        let pat = xm.pattern_order[pos] as usize;
        println!("  Position {}: Pattern {}", pos, pat);
    }
}
