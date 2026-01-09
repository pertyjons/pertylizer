//! Debug XM effect values to diagnose import issues.
//!
//! Usage: cargo run --example debug_effects -- /path/to/file.xm [pattern] [channel]

#![allow(clippy::expect_used)]

use std::env;
use xmrs::effect::TrackEffect;
use xmrs::import::xm::xmmodule::XmModule;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| "test.xm".to_string());
    let target_pattern: Option<usize> = env::args().nth(2).and_then(|s| s.parse().ok());
    let target_channel: Option<usize> = env::args().nth(3).and_then(|s| s.parse().ok());

    let data = std::fs::read(&path).expect("read file");

    // Parse raw XM to see effect bytes
    let xm = XmModule::load(&data).expect("parse xm");

    println!("=== Raw XM pattern data (before xmrs conversion) ===");
    println!(
        "Targeting pattern {:?}, channel {:?}\n",
        target_pattern, target_channel
    );

    // Access raw pattern data - xm.pattern is Vec<XmPattern>
    // Each XmPattern has .pattern which is Vec<Vec<PatternSlot>>
    for (pat_idx, xm_pattern) in xm.pattern.iter().enumerate() {
        if target_pattern.is_some() && target_pattern != Some(pat_idx) {
            continue;
        }

        println!("Pattern {} ({} rows):", pat_idx, xm_pattern.pattern.len());

        for (row_idx, row) in xm_pattern.pattern.iter().enumerate().take(20) {
            for (ch, slot) in row.iter().enumerate() {
                if target_channel.is_some() && target_channel != Some(ch) {
                    continue;
                }

                // Check if this slot has effect 4 (vibrato) or volume vibrato (0xBx)
                let has_vibrato_effect = slot.effect_type == 4;
                let has_vibrato_volume = slot.volume >= 0xB0 && slot.volume < 0xC0;

                if has_vibrato_effect || has_vibrato_volume {
                    println!(
                        "  Row {:2} Ch {:2}: effect_type={:X} effect_param=0x{:02X} ({}{:X}) vol=0x{:02X}",
                        row_idx,
                        ch,
                        slot.effect_type,
                        slot.effect_parameter,
                        slot.effect_parameter >> 4,
                        slot.effect_parameter & 0xF,
                        slot.volume
                    );
                }
            }
        }
        println!();
    }

    // Also show xmrs interpretation
    let module = xm.to_module();

    println!("=== xmrs interpretation ===\n");

    for (pat_idx, pattern) in module.pattern.iter().enumerate() {
        if target_pattern.is_some() && target_pattern != Some(pat_idx) {
            continue;
        }

        println!("Pattern {}:", pat_idx);
        for (row_idx, row) in pattern.iter().enumerate().take(20) {
            for (ch, slot) in row.iter().enumerate() {
                if target_channel.is_some() && target_channel != Some(ch) {
                    continue;
                }

                for effect in &slot.effects {
                    if let TrackEffect::Vibrato { speed, depth } = effect {
                        // Try different scaling factors
                        let speed_16 = (*speed * 16.0) as u8;
                        let speed_64 = (*speed * 64.0) as u8;
                        let depth_16 = (*depth * 16.0) as u8;

                        println!(
                            "  Row {:2} Ch {:2}: Vibrato speed={:.6} depth={:.6}",
                            row_idx, ch, speed, depth
                        );
                        println!(
                            "                 -> *16: 4{:X}{:X}, *64 speed: 4{:X}{:X}",
                            speed_16 & 0xF,
                            depth_16 & 0xF,
                            speed_64 & 0xF,
                            depth_16 & 0xF
                        );
                    }
                }
            }
        }
        println!();
    }
}
