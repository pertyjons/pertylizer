//! Analyze tracker files (MOD/XM/S3M) to debug import issues.
//!
//! Produces a complete textual representation of a tracker module,
//! including all instruments, envelopes, pattern order, and full
//! pattern data in a tracker-style grid format.
//!
//! Usage: cargo run --example analyze_tracker -- /path/to/file.mod
//!        cargo run --example analyze_tracker -- /path/to/file.xm
//!        cargo run --example analyze_tracker -- /path/to/file.s3m

use std::collections::HashMap;
use std::path::Path;

use xmrs::effect::{GlobalEffect, TrackEffect};
use xmrs::import::amiga::amiga_module::AmigaModule;
use xmrs::import::s3m::s3m_module::S3mModule;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::prelude::{InstrumentType, Module, SampleDataType, TrackUnit};

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

    // Detect format from extension or "mod." prefix
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    let is_mod_prefix = file_name.starts_with("mod.");

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
        _ if is_mod_prefix => {
            println!("Format: MOD (Amiga ProTracker Module) - detected from mod. prefix");
            match AmigaModule::load(&data) {
                Ok(amiga) => amiga.to_module(),
                Err(e) => {
                    eprintln!("Failed to parse MOD: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Unknown file extension: {}", ext);
            eprintln!("Supported formats: .mod, .xm, .s3m, or mod.* prefix");
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
    println!("Frequency type: {:?}", module.frequency_type);
    println!("Number of channels: {}", module.get_num_channels());
    println!("Number of patterns: {}", module.pattern.len());
    println!("Number of instruments: {}", module.instrument.len());

    // Count instruments with samples
    let instruments_with_samples = module
        .instrument
        .iter()
        .filter(|i| {
            matches!(&i.instr_type, InstrumentType::Default(d) if d.sample.iter().any(|s| s.is_some()))
        })
        .count();
    println!("Instruments with samples: {}", instruments_with_samples);

    // === Instruments ===
    println!("\n=== Instruments ===");
    for (idx, inst) in module.instrument.iter().enumerate() {
        println!("\n[Instrument {:02X}] {}", idx, inst.name);

        match &inst.instr_type {
            InstrumentType::Default(instr) => {
                let sample_count = instr.sample.iter().filter(|s| s.is_some()).count();
                println!("  Samples: {}", sample_count);
                println!("  Global volume: {:.2}", instr.global_volume);
                println!("  Volume fadeout: {:.4}", instr.volume_fadeout);
                println!("  Default pan: {:.2}", instr.default_pan);

                // Volume envelope
                print_envelope("Volume", &instr.volume_envelope);

                // Panning envelope
                print_envelope("Panning", &instr.pan_envelope);

                // Keymap summary (if multi-sample)
                let unique_samples: std::collections::HashSet<_> =
                    instr.sample_for_pitch.iter().filter_map(|&s| s).collect();
                if unique_samples.len() > 1 {
                    println!(
                        "  Keymap: {} unique samples across 120 notes",
                        unique_samples.len()
                    );
                    // Show keymap ranges
                    let mut current_sample = instr.sample_for_pitch[0];
                    let mut range_start = 0;
                    for (i, &smp) in instr.sample_for_pitch.iter().enumerate().skip(1) {
                        if smp != current_sample || i == 119 {
                            let end = if i == 119 && smp == current_sample {
                                i
                            } else {
                                i - 1
                            };
                            println!(
                                "    Notes {:3}-{:3} ({}-{}): Sample {:?}",
                                range_start,
                                end,
                                format_midi_note(range_start as u8),
                                format_midi_note(end as u8),
                                current_sample
                            );
                            current_sample = smp;
                            range_start = i;
                        }
                    }
                }

                // Print sample details
                for (smp_idx, sample_opt) in instr.sample.iter().enumerate() {
                    if let Some(sample) = sample_opt {
                        println!("  [Sample {:02X}] {}", smp_idx, sample.name);
                        let data_info = sample.data.as_ref().map(|d| match d {
                            SampleDataType::Mono8(v) => (v.len(), "Mono8"),
                            SampleDataType::Mono16(v) => (v.len(), "Mono16"),
                            SampleDataType::Stereo8(v) => (v.len(), "Stereo8"),
                            SampleDataType::Stereo16(v) => (v.len(), "Stereo16"),
                            SampleDataType::StereoFloat(v) => (v.len(), "StereoF32"),
                        });
                        if let Some((len, fmt)) = data_info {
                            println!("    Data: {} samples ({})", len, fmt);
                        } else {
                            println!("    Data: (none)");
                        }
                        println!("    Volume: {:.2}", sample.volume);
                        println!("    Panning: {:.2}", sample.panning);
                        println!("    Finetune: {:.4}", sample.finetune);
                        println!("    Relative pitch: {}", sample.relative_pitch);
                        println!(
                            "    Loop: {:?}  start={} length={}",
                            sample.loop_flag, sample.loop_start, sample.loop_length
                        );
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

    // === Pattern Order (full) ===
    println!("\n=== Pattern Order ===");
    if let Some(order) = module.pattern_order.first() {
        println!("Length: {}", order.len());
        println!("Order: {:?}", order);
    }

    // === Full Pattern Dump ===
    println!("\n=== Patterns ===");
    for (pat_idx, pattern) in module.pattern.iter().enumerate() {
        print_pattern(pat_idx, pattern, module.get_num_channels());
    }

    // === Effects Summary ===
    print_effects_summary(module);

    // === Summary Statistics ===
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

    let mut total_notes: usize = 0;
    let mut total_keyoffs: usize = 0;
    let mut total_effects: usize = 0;
    for pattern in &module.pattern {
        for row in pattern.iter() {
            for unit in row.iter() {
                if !unit.note.is_none() && !unit.note.is_keyoff() {
                    total_notes += 1;
                }
                if unit.note.is_keyoff() {
                    total_keyoffs += 1;
                }
                total_effects += unit.effects.len() + unit.global_effects.len();
            }
        }
    }

    println!("Total samples: {}", total_samples);
    println!("Total sample data: {} bytes", total_sample_data);
    println!("Total notes across all patterns: {}", total_notes);
    println!("Total key-offs across all patterns: {}", total_keyoffs);
    println!("Total effect commands: {}", total_effects);
}

/// Print a single envelope (volume or panning).
fn print_envelope(name: &str, envelope: &xmrs::envelope::Envelope) {
    println!("  {} envelope:", name);
    println!("    Enabled: {}", envelope.enabled);
    if !envelope.enabled && envelope.point.is_empty() {
        return;
    }
    println!("    Points: {}", envelope.point.len());
    for (i, p) in envelope.point.iter().enumerate() {
        println!("      [{:2}] frame={:5}, value={:.4}", i, p.frame, p.value);
    }
    println!(
        "    Sustain: enabled={}, point={}",
        envelope.sustain_enabled, envelope.sustain_start_point
    );
    println!(
        "    Loop: enabled={}, start={}, end={}",
        envelope.loop_enabled, envelope.loop_start_point, envelope.loop_end_point
    );
}

/// Print a full pattern in tracker grid format.
fn print_pattern(index: usize, pattern: &[Vec<TrackUnit>], num_channels: usize) {
    let num_rows = pattern.len();
    if num_rows == 0 {
        println!("\n--- Pattern {:02X} (empty) ---", index);
        return;
    }

    let actual_channels = pattern.first().map(|row| row.len()).unwrap_or(num_channels);

    println!(
        "\n--- Pattern {:02X} ({} rows, {} channels) ---",
        index, num_rows, actual_channels
    );

    // Grid header
    print!("Row |");
    for ch in 0..actual_channels {
        print!(" Ch {:02}          |", ch);
    }
    println!();

    // Separator
    print!("----|");
    for _ in 0..actual_channels {
        print!("----------------|");
    }
    println!();

    // Rows
    for (row_idx, row) in pattern.iter().enumerate() {
        print!("{:02X}  |", row_idx);
        for (ch_idx, unit) in row.iter().enumerate() {
            if ch_idx >= actual_channels {
                break;
            }
            print!(" {} |", format_cell(unit));
        }
        println!();
    }
}

/// Format a single cell as "NOT IN VL EFFECT" (15 chars).
///
/// Separates Volume (SetVolume) from other effects to match tracker convention
/// where volume column is displayed before the effect column.
fn format_cell(unit: &TrackUnit) -> String {
    let note = format_note(unit);
    let inst = match unit.instrument {
        Some(i) => format!("{:02X}", i),
        None => "..".to_string(),
    };

    // Separate volume (Volume/SetVolume) from other effects, showing volume first
    // to match tracker convention (volume column before effect column).
    let mut vol_str = String::new();
    let mut other_effects = Vec::new();

    for effect in &unit.effects {
        if let TrackEffect::Volume { value, .. } = effect {
            if vol_str.is_empty() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = (value * 64.0).min(64.0) as u8;
                vol_str = format!("{v:02X}");
            } else {
                other_effects.push(format_track_effect(effect));
            }
        } else {
            other_effects.push(format_track_effect(effect));
        }
    }

    // If no Volume effect, show velocity (for notes with explicit instrument)
    if vol_str.is_empty() {
        if unit.velocity > 0.0 && unit.velocity < 1.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let v = (unit.velocity * 255.0) as u8;
            vol_str = format!("{v:02X}");
        } else if unit.velocity >= 1.0 && unit.instrument.is_some() {
            vol_str = "FF".to_string();
        } else {
            vol_str = "..".to_string();
        }
    }

    // Add global effects after track effects
    for effect in &unit.global_effects {
        other_effects.push(format_global_effect(effect));
    }

    let efx_str = if other_effects.is_empty() {
        "....".to_string()
    } else {
        other_effects.join(" ")
    };

    format!("{} {} {} {:<4}", note, inst, vol_str, efx_str)
}

/// Format note/pitch as 3-character string (e.g. "C-5", "C#5", "===", "---").
fn format_note(unit: &TrackUnit) -> String {
    if unit.note.is_none() {
        return "...".to_string();
    }
    if unit.note.is_keyoff() {
        return "===".to_string();
    }

    let note_val = unit.note as u8;
    let octave = note_val / 12;
    let semitone = note_val % 12;

    let note_name = match semitone {
        0 => "C-",
        1 => "C#",
        2 => "D-",
        3 => "D#",
        4 => "E-",
        5 => "F-",
        6 => "F#",
        7 => "G-",
        8 => "G#",
        9 => "A-",
        10 => "A#",
        11 => "B-",
        _ => "??",
    };

    format!("{}{}", note_name, octave)
}

/// Format MIDI note number as note name.
fn format_midi_note(note: u8) -> String {
    let octave = note / 12;
    let semitone = note % 12;
    let name = match semitone {
        0 => "C-",
        1 => "C#",
        2 => "D-",
        3 => "D#",
        4 => "E-",
        5 => "F-",
        6 => "F#",
        7 => "G-",
        8 => "G#",
        9 => "A-",
        10 => "A#",
        11 => "B-",
        _ => "??",
    };
    format!("{}{}", name, octave)
}

/// Format a track effect as a 4-char code.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_track_effect(effect: &TrackEffect) -> String {
    match effect {
        TrackEffect::Arpeggio { half1, half2 } => {
            format!("0{:X}{:X}", (*half1).min(15), (*half2).min(15))
        }
        TrackEffect::Portamento(speed) => {
            let s = (speed.abs() * 16.0).min(255.0) as u8;
            if *speed > 0.0 {
                format!("1{:02X}", s)
            } else if *speed < 0.0 {
                format!("2{:02X}", s)
            } else {
                "100".to_string()
            }
        }
        TrackEffect::TonePortamento(speed) => {
            let s = (speed * 16.0).min(255.0) as u8;
            format!("3{:02X}", s)
        }
        TrackEffect::Vibrato { speed, depth } => {
            let s = (speed * 64.0).min(15.0) as u8;
            let d = (depth * 16.0).min(15.0) as u8;
            format!("4{:X}{:X}", s, d)
        }
        TrackEffect::VolumeSlide { speed, fine } => {
            let s = (speed.abs() * 64.0).min(15.0) as u8;
            let prefix = if *fine { "E" } else { "A" };
            if *speed > 0.0 {
                format!("{}{:X}0", prefix, s)
            } else {
                format!("{}0{:X}", prefix, s)
            }
        }
        TrackEffect::Volume { value, .. } => {
            let v = (value * 64.0).min(64.0) as u8;
            format!("v{:02X}", v)
        }
        TrackEffect::Panning(pan) => {
            let p = (pan * 255.0).min(255.0) as u8;
            format!("8{:02X}", p)
        }
        TrackEffect::PanningSlide { speed, .. } => {
            let s = (speed.abs() * 16.0).min(15.0) as u8;
            if *speed < 0.0 {
                format!("P{:X}0", s)
            } else {
                format!("P0{:X}", s)
            }
        }
        TrackEffect::InstrumentSampleOffset(offset) => {
            let raw = (offset / 256).min(255) as u8;
            format!("9{:02X}", raw)
        }
        TrackEffect::Tremolo { speed, depth } => {
            let s = (speed * 64.0).min(15.0) as u8;
            let d = (depth * 16.0).min(15.0) as u8;
            format!("7{:X}{:X}", s, d)
        }
        TrackEffect::NoteCut { tick, .. } => format!("SC{:X}", (*tick).min(15)),
        TrackEffect::NoteDelay(ticks) => format!("SD{:X}", (*ticks).min(15)),
        TrackEffect::NoteFadeOut { tick, .. } => format!("KF{:X}", (*tick).min(15)),
        TrackEffect::NoteRetrig { speed, .. } => format!("R{:02X}", (*speed).min(255)),
        TrackEffect::Glissando(on) => format!("E3{}", if *on { "1" } else { "0" }),
        TrackEffect::VibratoWaveform { .. } => "E4x".to_string(),
        TrackEffect::TremoloWaveform { .. } => "E7x".to_string(),
        TrackEffect::VibratoSpeed(_) => "vSP".to_string(),
        TrackEffect::VibratoDepth(_) => "vDP".to_string(),
        TrackEffect::ChannelVolume(_) => "cVOL".to_string(),
        TrackEffect::ChannelVolumeSlide { .. } => "cVSL".to_string(),
        TrackEffect::InstrumentFineTune(_) => "E5x".to_string(),
        TrackEffect::InstrumentVolumeEnvelope(on) => {
            format!("Lv{}", if *on { "1" } else { "0" })
        }
        TrackEffect::InstrumentVolumeEnvelopePosition(pos) => {
            format!("L{:02X}", (*pos).min(255))
        }
        TrackEffect::InstrumentPanningEnvelope(on) => {
            format!("Lp{}", if *on { "1" } else { "0" })
        }
        TrackEffect::InstrumentPanningEnvelopePosition(pos) => {
            format!("M{:02X}", (*pos).min(255))
        }
        TrackEffect::InstrumentPitchEnvelope(on) => {
            format!("Lt{}", if *on { "1" } else { "0" })
        }
        TrackEffect::Panbrello { .. } => "Ypq".to_string(),
        TrackEffect::PanbrelloWaveform { .. } => "Ywv".to_string(),
        TrackEffect::NoteOff { tick, .. } => format!("KO{:X}", (*tick).min(15)),
        TrackEffect::Tremor { on_time, off_time } => {
            format!("T{:X}{:X}", (*on_time).min(15), (*off_time).min(15))
        }
        TrackEffect::InstrumentNewNoteAction(_) => "NNA".to_string(),
        TrackEffect::InstrumentSurround(on) => format!("S{}", if *on { "1" } else { "0" }),
    }
}

/// Format a global effect as a 4-char code.
#[allow(clippy::cast_possible_truncation)]
fn format_global_effect(effect: &GlobalEffect) -> String {
    match effect {
        GlobalEffect::Bpm(bpm) => format!("F{:02X}", (*bpm).min(255)),
        GlobalEffect::Speed(speed) => format!("F{:02X}", (*speed).min(31)),
        GlobalEffect::PatternBreak(row) => format!("D{:02X}", (*row).min(255)),
        GlobalEffect::PositionJump(pos) => format!("B{:02X}", (*pos).min(255)),
        GlobalEffect::PatternLoop(count) => format!("E6{:X}", (*count).min(15)),
        GlobalEffect::PatternDelay { quantity, .. } => format!("EE{:X}", (*quantity).min(15)),
        GlobalEffect::Volume(vol) => {
            let v = (vol * 64.0).min(64.0) as u8;
            format!("G{:02X}", v)
        }
        GlobalEffect::VolumeSlide { speed, fine } => {
            let s = (speed.abs() * 64.0).min(15.0) as u8;
            let prefix = if *fine { "gF" } else { "gS" };
            let dir = if *speed > 0.0 { "+" } else { "-" };
            format!("{}{}{:X}", prefix, dir, s)
        }
        GlobalEffect::BpmSlide(delta) => format!("Fb{:+}", delta),
        GlobalEffect::MidiMacro(_) => "Zxx".to_string(),
    }
}

/// Print a summary of which effects are used across all patterns.
fn print_effects_summary(module: &Module) {
    let mut track_effect_counts: HashMap<String, usize> = HashMap::new();
    let mut global_effect_counts: HashMap<String, usize> = HashMap::new();

    for pattern in &module.pattern {
        for row in pattern.iter() {
            for unit in row.iter() {
                for effect in &unit.effects {
                    let name = track_effect_name(effect);
                    *track_effect_counts.entry(name).or_insert(0) += 1;
                }
                for effect in &unit.global_effects {
                    let name = global_effect_name(effect);
                    *global_effect_counts.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    if track_effect_counts.is_empty() && global_effect_counts.is_empty() {
        return;
    }

    println!("\n=== Effects Summary ===");

    if !track_effect_counts.is_empty() {
        println!("Track effects:");
        let mut sorted: Vec<_> = track_effect_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, count) in &sorted {
            println!("  {:30} x{}", name, count);
        }
    }

    if !global_effect_counts.is_empty() {
        println!("Global effects:");
        let mut sorted: Vec<_> = global_effect_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, count) in &sorted {
            println!("  {:30} x{}", name, count);
        }
    }
}

/// Get a human-readable name for a track effect (for summary).
fn track_effect_name(effect: &TrackEffect) -> String {
    match effect {
        TrackEffect::Arpeggio { .. } => "Arpeggio".to_string(),
        TrackEffect::Portamento(s) => {
            if *s > 0.0 {
                "Portamento Up".to_string()
            } else {
                "Portamento Down".to_string()
            }
        }
        TrackEffect::TonePortamento(_) => "Tone Portamento".to_string(),
        TrackEffect::Vibrato { .. } => "Vibrato".to_string(),
        TrackEffect::VibratoSpeed(_) => "Vibrato Speed".to_string(),
        TrackEffect::VibratoDepth(_) => "Vibrato Depth".to_string(),
        TrackEffect::VibratoWaveform { .. } => "Vibrato Waveform".to_string(),
        TrackEffect::Glissando(_) => "Glissando".to_string(),
        TrackEffect::InstrumentFineTune(_) => "Fine Tune".to_string(),
        TrackEffect::Volume { .. } => "Set Volume".to_string(),
        TrackEffect::ChannelVolume(_) => "Channel Volume".to_string(),
        TrackEffect::VolumeSlide { fine: true, .. } => "Fine Volume Slide".to_string(),
        TrackEffect::VolumeSlide { fine: false, .. } => "Volume Slide".to_string(),
        TrackEffect::ChannelVolumeSlide { .. } => "Channel Volume Slide".to_string(),
        TrackEffect::Tremolo { .. } => "Tremolo".to_string(),
        TrackEffect::TremoloWaveform { .. } => "Tremolo Waveform".to_string(),
        TrackEffect::Panning(_) => "Set Panning".to_string(),
        TrackEffect::PanningSlide { .. } => "Panning Slide".to_string(),
        TrackEffect::Panbrello { .. } => "Panbrello".to_string(),
        TrackEffect::PanbrelloWaveform { .. } => "Panbrello Waveform".to_string(),
        TrackEffect::InstrumentSampleOffset(_) => "Sample Offset".to_string(),
        TrackEffect::NoteRetrig { .. } => "Note Retrigger".to_string(),
        TrackEffect::NoteCut { .. } => "Note Cut".to_string(),
        TrackEffect::NoteDelay(_) => "Note Delay".to_string(),
        TrackEffect::NoteFadeOut { .. } => "Note Fade Out".to_string(),
        TrackEffect::NoteOff { .. } => "Note Off".to_string(),
        TrackEffect::Tremor { .. } => "Tremor".to_string(),
        TrackEffect::InstrumentVolumeEnvelope(_) => "Vol Envelope On/Off".to_string(),
        TrackEffect::InstrumentVolumeEnvelopePosition(_) => "Vol Envelope Position".to_string(),
        TrackEffect::InstrumentPanningEnvelope(_) => "Pan Envelope On/Off".to_string(),
        TrackEffect::InstrumentPanningEnvelopePosition(_) => "Pan Envelope Position".to_string(),
        TrackEffect::InstrumentPitchEnvelope(_) => "Pitch Envelope On/Off".to_string(),
        TrackEffect::InstrumentNewNoteAction(_) => "New Note Action".to_string(),
        TrackEffect::InstrumentSurround(_) => "Surround".to_string(),
    }
}

/// Get a human-readable name for a global effect (for summary).
fn global_effect_name(effect: &GlobalEffect) -> String {
    match effect {
        GlobalEffect::Bpm(_) => "Set BPM".to_string(),
        GlobalEffect::Speed(_) => "Set Speed".to_string(),
        GlobalEffect::PatternBreak(_) => "Pattern Break".to_string(),
        GlobalEffect::PositionJump(_) => "Position Jump".to_string(),
        GlobalEffect::PatternLoop(_) => "Pattern Loop".to_string(),
        GlobalEffect::PatternDelay { .. } => "Pattern Delay".to_string(),
        GlobalEffect::Volume(_) => "Global Volume".to_string(),
        GlobalEffect::VolumeSlide { .. } => "Global Volume Slide".to_string(),
        GlobalEffect::BpmSlide(_) => "BPM Slide".to_string(),
        GlobalEffect::MidiMacro(_) => "MIDI Macro".to_string(),
    }
}
