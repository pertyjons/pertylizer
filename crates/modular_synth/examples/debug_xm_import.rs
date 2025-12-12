//! Debug XM import to find why playback is silent.
//!
//! Usage: cargo run --release --example debug_xm_import

#![allow(clippy::expect_used)]

use modular_synth::io::import::{SongImporter, TrackerImporter};
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/per/Musik/joli_untouched.xm".to_string());

    println!("=== DEBUG XM IMPORT ===");
    println!("File: {}\n", path);

    let importer = TrackerImporter::new();
    match importer.import(Path::new(&path)) {
        Ok(imported) => {
            println!("=== IMPORT SUCCESS ===");
            println!("Song name: '{}'", imported.song.name);
            println!(
                "Default tempo: {} BPM",
                imported.song.default_tempo.as_f32()
            );
            println!("Samples extracted: {}", imported.samples.len());
            println!("Instruments extracted: {}", imported.instruments.len());
            println!("Patterns: {}", imported.song.pattern_count());

            println!("\n=== INSTRUMENTS ===");
            for (idx, inst) in imported.instruments.iter().enumerate() {
                let sample_info = inst
                    .sample_index
                    .map(|i| format!("sample[{}]", i))
                    .unwrap_or_else(|| "NO SAMPLE".to_string());
                println!(
                    "  [{:02}] '{}' - {} (global_vol={})",
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

            println!("\n=== FIRST PATTERN IN ARRANGEMENT (Pattern 0) ===");
            // Get the first pattern from the arrangement, not from the HashMap
            let first_arrangement = imported.song.arrangement().first();
            let first_pattern = first_arrangement.and_then(|p| imported.song.pattern(p.pattern_id));
            if let Some(pattern) = first_pattern {
                println!("Pattern ID: {:?}", pattern.id);
                println!("Pattern name: '{}'", pattern.name);
                println!("Pattern length: {} ticks", pattern.length.0);
                println!("Num tracks: {}", pattern.num_tracks());
                println!("Notes in pattern: {}", pattern.notes().len());

                println!("\nFirst 20 notes:");
                for (i, note) in pattern.notes().iter().take(20).enumerate() {
                    println!(
                        "  [{:02}] tick={}, pitch={} (MIDI {}), vel={:.2}, inst={:?}, track={:?}",
                        i,
                        note.start.0,
                        note.pitch,
                        note.pitch.as_midi(),
                        note.velocity.as_f32(),
                        note.instrument,
                        note.track
                    );
                }
            }

            // Check for potential issues
            println!("\n=== POTENTIAL ISSUES ===");

            // Check if instruments have samples
            let instruments_without_samples = imported
                .instruments
                .iter()
                .enumerate()
                .filter(|(_, inst)| inst.sample_index.is_none())
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

            println!("\n=== DONE ===");
        }
        Err(e) => {
            println!("IMPORT FAILED: {}", e);
        }
    }
}
