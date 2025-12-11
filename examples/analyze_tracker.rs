//! Analyze tracker files (MOD/XM/S3M) to debug import issues.
//!
//! Usage: cargo run --example analyze_tracker -- /path/to/file.mod
//!        cargo run --example analyze_tracker -- /path/to/file.xm
//!        cargo run --example analyze_tracker -- /path/to/file.s3m

use std::path::Path;

use xmrs::import::amiga::amiga_module::AmigaModule;
use xmrs::import::s3m::s3m_module::S3mModule;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::prelude::{InstrumentType, Module, SampleDataType};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <tracker_file>", args[0]);
        eprintln!("Supported formats: .mod, .xm, .s3m");
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    println!("Analyzing: {}", path.display());
    println!("{}", "=".repeat(60));

    // Load the file
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to read file: {}", e);
            std::process::exit(1);
        }
    };

    println!("File size: {} bytes", data.len());

    // Detect format from extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    let module: Module = match ext.as_str() {
        "xm" => {
            println!("Format: XM (Fast Tracker II Extended Module)");
            match XmModule::load(&data) {
                Ok(xm) => xm.to_module(),
                Err(e) => {
                    eprintln!("Failed to parse XM: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        "mod" => {
            println!("Format: MOD (Amiga ProTracker Module)");
            match AmigaModule::load(&data) {
                Ok(amiga) => amiga.to_module(),
                Err(e) => {
                    eprintln!("Failed to parse MOD: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        "s3m" => {
            println!("Format: S3M (Scream Tracker 3 Module)");
            match S3mModule::load(&data) {
                Ok(s3m) => s3m.to_module(),
                Err(e) => {
                    eprintln!("Failed to parse S3M: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Unknown file extension: {}", ext);
            eprintln!("Supported formats: .mod, .xm, .s3m");
            std::process::exit(1);
        }
    };

    analyze_module(&module);
}

fn analyze_module(module: &Module) {
    println!("\n=== Module Info ===");
    println!("Name: {}", module.name);
    println!("Default BPM: {}", module.default_bpm);
    println!("Default tempo/speed: {}", module.default_tempo);
    println!("Number of channels: {}", module.get_num_channels());
    println!("Number of patterns: {}", module.pattern.len());
    println!("Number of instruments: {}", module.instrument.len());

    // Count instruments with samples
    let instruments_with_samples = module
        .instrument
        .iter()
        .filter(|i| matches!(&i.instr_type, InstrumentType::Default(d) if d.sample.iter().any(|s| s.is_some())))
        .count();
    println!("Instruments with samples: {}", instruments_with_samples);

    println!("\n=== Instruments ===");
    for (idx, inst) in module.instrument.iter().enumerate() {
        println!("\n[Instrument {}] {}", idx, inst.name);

        match &inst.instr_type {
            InstrumentType::Default(instr) => {
                let sample_count = instr.sample.iter().filter(|s| s.is_some()).count();
                println!("  Samples: {}", sample_count);
                println!("  Global volume: {:.2}", instr.global_volume);
                println!("  Volume fadeout: {:.4}", instr.volume_fadeout);
                println!("  Default pan: {:.2}", instr.default_pan);

                // Volume envelope
                let ve = &instr.volume_envelope;
                println!("  Volume envelope:");
                println!("    Enabled: {}", ve.enabled);
                println!("    Points: {}", ve.point.len());
                if !ve.point.is_empty() {
                    for (i, p) in ve.point.iter().enumerate() {
                        println!("      [{}] frame={}, value={:.2}", i, p.frame, p.value);
                    }
                }
                println!("    Sustain enabled: {}", ve.sustain_enabled);
                println!("    Sustain point: {}", ve.sustain_start_point);
                println!("    Loop enabled: {}", ve.loop_enabled);

                // Print sample details
                for (smp_idx, sample_opt) in instr.sample.iter().enumerate() {
                    if let Some(sample) = sample_opt {
                        println!("  [Sample {}] {}", smp_idx, sample.name);
                        let data_len = sample
                            .data
                            .as_ref()
                            .map(|d| match d {
                                SampleDataType::Mono8(v) => v.len(),
                                SampleDataType::Mono16(v) => v.len(),
                                SampleDataType::Stereo8(v) => v.len(),
                                SampleDataType::Stereo16(v) => v.len(),
                                SampleDataType::StereoFloat(v) => v.len(),
                            })
                            .unwrap_or(0);
                        println!("    Data length: {} samples", data_len);
                        println!("    Volume: {:.2}", sample.volume);
                        println!("    Loop type: {:?}", sample.loop_flag);
                        println!(
                            "    Loop start: {}, length: {}",
                            sample.loop_start, sample.loop_length
                        );
                        println!("    Relative pitch: {}", sample.relative_pitch);
                    }
                }
            }
            InstrumentType::Empty => {
                println!("  (Empty instrument)");
            }
            _ => {
                println!("  (Other instrument type)");
            }
        }
    }

    // Check pattern order
    println!("\n=== Pattern Order ===");
    if let Some(order) = module.pattern_order.first() {
        println!("Order: {:?}", &order[..order.len().min(32)]);
        if order.len() > 32 {
            println!("  ... ({} total)", order.len());
        }
    }

    // Check first pattern for notes
    println!("\n=== First Pattern Notes ===");
    if let Some(pattern) = module.pattern.first() {
        let mut note_count = 0;
        for (row_idx, row) in pattern.iter().enumerate() {
            for (ch_idx, unit) in row.iter().enumerate() {
                if !unit.note.is_none() && !unit.note.is_keyoff() {
                    if note_count < 20 {
                        println!(
                            "  Row {}, Ch {}: note={:?}, inst={:?}, vel={:.2}",
                            row_idx, ch_idx, unit.note, unit.instrument, unit.velocity
                        );
                    }
                    note_count += 1;
                }
            }
        }
        println!("Total notes in first pattern: {}", note_count);
    }

    // Summary statistics
    println!("\n=== Summary ===");
    let total_samples: usize = module
        .instrument
        .iter()
        .filter_map(|i| match &i.instr_type {
            InstrumentType::Default(d) => Some(d.sample.iter().filter(|s| s.is_some()).count()),
            _ => None,
        })
        .sum();

    let total_sample_data: usize = module
        .instrument
        .iter()
        .filter_map(|i| match &i.instr_type {
            InstrumentType::Default(d) => Some(
                d.sample
                    .iter()
                    .filter_map(|s| s.as_ref())
                    .filter_map(|s| {
                        s.data.as_ref().map(|d| match d {
                            SampleDataType::Mono8(v) => v.len(),
                            SampleDataType::Mono16(v) => v.len() * 2,
                            SampleDataType::Stereo8(v) => v.len(),
                            SampleDataType::Stereo16(v) => v.len() * 2,
                            SampleDataType::StereoFloat(v) => v.len() * 4,
                        })
                    })
                    .sum::<usize>(),
            ),
            _ => None,
        })
        .sum();

    let total_notes: usize = module
        .pattern
        .iter()
        .map(|p| {
            p.iter()
                .flat_map(|row| row.iter())
                .filter(|u| !u.note.is_none() && !u.note.is_keyoff())
                .count()
        })
        .sum();

    println!("Total samples: {}", total_samples);
    println!("Total sample data: {} bytes", total_sample_data);
    println!("Total notes across all patterns: {}", total_notes);
}
