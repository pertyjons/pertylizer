//! Detailed XM file analyzer to debug import issues.
//!
//! Usage: cargo run --example analyze_xm_detailed -- /path/to/file.xm

use std::collections::HashSet;
use std::path::Path;

use xmrs::import::xm::xmmodule::XmModule;
use xmrs::prelude::*;
use xmrs::sample::SampleDataType;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Usage: analyze_xm_detailed <file.xm>");
    let data = std::fs::read(&path).expect("Failed to read file");

    let xm = XmModule::load(&data).expect("Failed to parse XM");
    let module = xm.to_module();

    println!("=== MODULE INFO ===");
    println!("Name: {}", module.name);
    println!("Frequency Type: {:?}", module.frequency_type);
    println!("BPM: {}", module.default_bpm);
    println!("Speed (ticks/row): {}", module.default_tempo);
    println!("Channels: {}", module.get_num_channels());
    println!("Instruments: {}", module.instrument.len());
    println!("Patterns: {}", module.pattern.len());
    println!();

    // Envelope tick rate calculation
    let tick_rate = module.default_bpm as f32 * 2.0 / 5.0;
    println!("Envelope tick rate: {:.1} ticks/sec", tick_rate);
    println!("Secs per tick: {:.4}", 1.0 / tick_rate);
    println!();

    for (inst_idx, inst) in module.instrument.iter().enumerate() {
        if let InstrumentType::Default(instr) = &inst.instr_type {
            let sample_count = instr.sample.iter().filter(|s| s.is_some()).count();
            if sample_count == 0 {
                continue;
            }

            println!("=== INSTRUMENT {} ===", inst_idx);
            println!("Name: '{}'", inst.name);
            println!("Global Volume: {:.3}", instr.global_volume);
            println!("Default Pan: {:.3} (0=L, 0.5=C, 1=R)", instr.default_pan);
            println!("Volume Fadeout: {:.6}", instr.volume_fadeout);
            println!("Sample count: {}", sample_count);

            // Volume envelope
            let env = &instr.volume_envelope;
            println!("\nVolume Envelope:");
            println!("  Enabled: {}", env.enabled);
            if !env.point.is_empty() {
                println!("  Points:");
                for (i, p) in env.point.iter().enumerate() {
                    let time_secs = p.frame as f32 / tick_rate;
                    println!(
                        "    [{:2}] frame={:4}, value={:.3} ({:.3}s)",
                        i, p.frame, p.value, time_secs
                    );
                }
            }
            println!(
                "  Sustain: {} (point {}->{})",
                env.sustain_enabled, env.sustain_start_point, env.sustain_end_point
            );
            println!(
                "  Loop: {} (point {}->{})",
                env.loop_enabled, env.loop_start_point, env.loop_end_point
            );

            // sample_for_pitch analysis
            let unique_samples: HashSet<_> =
                instr.sample_for_pitch.iter().filter_map(|x| *x).collect();
            println!("\nsample_for_pitch:");
            println!("  Unique sample indices used: {:?}", unique_samples);
            if unique_samples.len() > 1 {
                println!("  WARNING: Multi-sample instrument - different samples for different notes!");
                // Show mapping
                let mut current_sample = instr.sample_for_pitch[0];
                let mut range_start = 0;
                for (i, &sample_idx) in instr.sample_for_pitch.iter().enumerate() {
                    if sample_idx != current_sample || i == 119 {
                        if current_sample.is_some() {
                            println!(
                                "    Notes {}-{}: Sample {}",
                                range_start,
                                i - 1,
                                current_sample.unwrap_or(0)
                            );
                        }
                        current_sample = sample_idx;
                        range_start = i;
                    }
                }
            }

            // Samples
            println!("\nSamples:");
            for (smp_idx, sample_opt) in instr.sample.iter().enumerate() {
                if let Some(sample) = sample_opt {
                    println!("  [{}] '{}'", smp_idx, sample.name);
                    println!("      relative_pitch: {} semitones", sample.relative_pitch);
                    println!("      finetune: {:.4} ({:.1} cents)", sample.finetune, sample.finetune * 100.0);
                    println!("      volume: {:.3}", sample.volume);
                    println!("      panning: {:.3}", sample.panning);
                    println!("      loop_flag: {:?}", sample.loop_flag);

                    if let Some(data) = &sample.data {
                        let (len, typ, channels) = match data {
                            SampleDataType::Mono8(v) => (v.len(), "Mono8", 1),
                            SampleDataType::Mono16(v) => (v.len(), "Mono16", 1),
                            SampleDataType::Stereo8(v) => (v.len(), "Stereo8", 2),
                            SampleDataType::Stereo16(v) => (v.len(), "Stereo16", 2),
                            SampleDataType::StereoFloat(v) => (v.len(), "StereoFloat", 2),
                        };
                        let frame_count = len / channels;
                        println!("      data: {} samples, {} frames ({})", len, frame_count, typ);

                        // Show sample data stats to verify encoding
                        let (min, max, first_10) = match data {
                            SampleDataType::Mono8(v) => {
                                let min = v.iter().min().copied().unwrap_or(0);
                                let max = v.iter().max().copied().unwrap_or(0);
                                let first: Vec<i8> = v.iter().take(10).copied().collect();
                                (min as i32, max as i32, format!("{:?}", first))
                            }
                            SampleDataType::Mono16(v) => {
                                let min = v.iter().min().copied().unwrap_or(0);
                                let max = v.iter().max().copied().unwrap_or(0);
                                let first: Vec<i16> = v.iter().take(10).copied().collect();
                                (min as i32, max as i32, format!("{:?}", first))
                            }
                            _ => (0, 0, "stereo".to_string()),
                        };
                        println!("      data range: {} to {}", min, max);
                        println!("      first samples: {}", first_10);

                        println!(
                            "      loop_start: {} loop_length: {}",
                            sample.loop_start, sample.loop_length
                        );
                        if sample.loop_length > 0 {
                            let loop_start_norm = sample.loop_start as f32 / frame_count as f32;
                            let loop_end_norm =
                                (sample.loop_start + sample.loop_length) as f32 / frame_count as f32;
                            println!(
                                "      -> loop normalized: {:.4} - {:.4} ({}% - {}%)",
                                loop_start_norm,
                                loop_end_norm,
                                (loop_start_norm * 100.0) as i32,
                                (loop_end_norm * 100.0) as i32
                            );

                            // Check if loop extends past sample end
                            let loop_end = sample.loop_start + sample.loop_length;
                            if loop_end > frame_count as u32 {
                                println!("      WARNING: Loop end ({}) > frame count ({})!", loop_end, frame_count);
                            }
                        }

                        // Our pitch calculation based on frequency type
                        let base_rate = match module.frequency_type {
                            xmrs::period_helper::FrequencyType::LinearFrequencies => 8363.0_f32,
                            xmrs::period_helper::FrequencyType::AmigaFrequencies => {
                                let note = 48.0_f32;
                                let period = 6848.0 * (-0.0578 * note).exp() + 0.2782;
                                7_093_789.2 / (period * 2.0)
                            }
                        };
                        let finetune_adj = 2.0_f32.powf(sample.finetune / 12.0);
                        let sample_rate = base_rate * finetune_adj;
                        let root_midi =
                            (60 + sample.relative_pitch as i16).clamp(0, 127) as u8;
                        println!("      -> Base rate ({:?}): {:.1} Hz", module.frequency_type, base_rate);
                        println!("      -> Calculated sample_rate: {:.1} Hz", sample_rate);
                        println!(
                            "      -> Calculated root_note: {} (MIDI {})",
                            note_name(root_midi),
                            root_midi
                        );
                    }
                }
            }
            println!();
        }
    }

    // Show first pattern notes
    if let Some(pattern) = module.pattern.first() {
        println!("=== FIRST PATTERN NOTES (first 16 rows) ===");
        for (row_idx, row) in pattern.iter().take(16).enumerate() {
            let notes: Vec<String> = row.iter().enumerate().map(|(ch, unit)| {
                if unit.note.is_none() {
                    "---".to_string()
                } else if unit.note.is_keyoff() {
                    "===".to_string()
                } else {
                    let note_val = unit.note as u8;
                    format!("{}", note_name(note_val))
                }
            }).collect();
            println!("  Row {:2}: {}", row_idx, notes.join(" | "));
        }
    }
}

fn note_name(midi: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = midi / 12;
    let note = (midi % 12) as usize;
    format!("{}-{}", names[note], octave)
}
