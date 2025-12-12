//! Comprehensive analysis of tracker files for import compatibility.
//!
//! Analyzes all tracker files in a directory and reports on:
//! - Effects/commands used
//! - Envelope features
//! - Sample properties
//! - Unsupported or unusual features
//!
//! Usage: cargo run --example analyze_all_trackers -- /path/to/music/directory

#![allow(clippy::expect_used)] // This is a CLI tool, expect is fine
#![allow(clippy::manual_range_contains)] // Readability preference
#![allow(clippy::unnecessary_to_owned)] // Convenience in analysis tool
#![allow(clippy::manual_flatten)] // Explicit iteration style

use std::collections::{HashMap, HashSet};
use std::path::Path;

use xmrs::effect::{GlobalEffect, TrackEffect};
use xmrs::import::amiga::amiga_module::AmigaModule;
use xmrs::import::s3m::s3m_module::S3mModule;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::prelude::{FrequencyType, InstrumentType, Module, SampleDataType};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = if args.len() < 2 {
        "/home/per/Musik"
    } else {
        &args[1]
    };

    println!("{}", "=".repeat(80));
    println!("TRACKER FILE ANALYSIS REPORT");
    println!("Directory: {}", dir);
    println!("{}", "=".repeat(80));

    let mut all_track_effects: HashMap<String, HashSet<String>> = HashMap::new();
    let mut all_global_effects: HashMap<String, HashSet<String>> = HashMap::new();
    let mut envelope_features: HashMap<String, u32> = HashMap::new();
    let mut sample_features: HashMap<String, u32> = HashMap::new();
    let mut file_stats: Vec<FileStats> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();

    // Find all tracker files
    let entries: Vec<_> = std::fs::read_dir(dir)
        .expect("Failed to read directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            if !path.is_file() {
                return false;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            matches!(ext.to_lowercase().as_str(), "mod" | "xm" | "s3m")
                || name.to_lowercase().starts_with("mod.")
        })
        .collect();

    println!("\nFound {} tracker files\n", entries.len());

    for entry in &entries {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        match analyze_file(&path) {
            Ok(stats) => {
                // Collect track effects
                for effect in &stats.track_effects_used {
                    all_track_effects
                        .entry(effect.clone())
                        .or_default()
                        .insert(file_name.to_string());
                }

                // Collect global effects
                for effect in &stats.global_effects_used {
                    all_global_effects
                        .entry(effect.clone())
                        .or_default()
                        .insert(file_name.to_string());
                }

                // Collect envelope features
                for feature in &stats.envelope_features {
                    *envelope_features.entry(feature.clone()).or_insert(0) += 1;
                }

                // Collect sample features
                for feature in &stats.sample_features {
                    *sample_features.entry(feature.clone()).or_insert(0) += 1;
                }

                file_stats.push(stats);
            }
            Err(e) => {
                errors.push((file_name.to_string(), e));
            }
        }
    }

    // Print report
    println!("\n{}", "=".repeat(80));
    println!("TRACK EFFECTS USED");
    println!("{}", "=".repeat(80));

    let mut effects_sorted: Vec<_> = all_track_effects.iter().collect();
    effects_sorted.sort_by_key(|(k, _)| *k);

    for (effect, files) in effects_sorted {
        let supported = is_track_effect_supported(effect);
        let status = if supported { "✓" } else { "✗" };
        println!("{} {} (used in {} files)", status, effect, files.len());
        if !supported && files.len() <= 5 {
            println!("    Files: {:?}", files.iter().collect::<Vec<_>>());
        }
    }

    println!("\n{}", "=".repeat(80));
    println!("GLOBAL EFFECTS USED");
    println!("{}", "=".repeat(80));

    let mut global_sorted: Vec<_> = all_global_effects.iter().collect();
    global_sorted.sort_by_key(|(k, _)| *k);

    for (effect, files) in global_sorted {
        let supported = is_global_effect_supported(effect);
        let status = if supported { "✓" } else { "✗" };
        println!("{} {} (used in {} files)", status, effect, files.len());
    }

    println!("\n{}", "=".repeat(80));
    println!("ENVELOPE FEATURES");
    println!("{}", "=".repeat(80));

    let mut env_sorted: Vec<_> = envelope_features.iter().collect();
    env_sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (feature, count) in env_sorted {
        let supported = is_envelope_feature_supported(feature);
        let status = if supported { "✓" } else { "✗" };
        println!("{} {} ({} files)", status, feature, count);
    }

    println!("\n{}", "=".repeat(80));
    println!("SAMPLE FEATURES");
    println!("{}", "=".repeat(80));

    let mut sample_sorted: Vec<_> = sample_features.iter().collect();
    sample_sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (feature, count) in sample_sorted {
        let supported = is_sample_feature_supported(feature);
        let status = if supported { "✓" } else { "✗" };
        println!("{} {} ({} files)", status, feature, count);
    }

    println!("\n{}", "=".repeat(80));
    println!("FILE STATISTICS");
    println!("{}", "=".repeat(80));

    // Summary stats
    let total_files = file_stats.len();
    let total_instruments: usize = file_stats.iter().map(|s| s.instrument_count).sum();
    let total_samples: usize = file_stats.iter().map(|s| s.sample_count).sum();
    let total_patterns: usize = file_stats.iter().map(|s| s.pattern_count).sum();
    let max_channels = file_stats
        .iter()
        .map(|s| s.channel_count)
        .max()
        .unwrap_or(0);

    println!("Total files analyzed: {}", total_files);
    println!("Total instruments: {}", total_instruments);
    println!("Total samples: {}", total_samples);
    println!("Total patterns: {}", total_patterns);
    println!("Max channels in single file: {}", max_channels);

    // Files with unusual features
    println!("\n--- Files with >16 channels ---");
    for stats in &file_stats {
        if stats.channel_count > 16 {
            println!("  {} ({} channels)", stats.file_name, stats.channel_count);
        }
    }

    println!("\n--- Files with panning envelope ---");
    for stats in &file_stats {
        if stats
            .envelope_features
            .contains(&"Panning envelope enabled".to_string())
        {
            println!("  {}", stats.file_name);
        }
    }

    println!("\n--- Files with auto-vibrato ---");
    for stats in &file_stats {
        if stats
            .envelope_features
            .contains(&"Auto-vibrato".to_string())
        {
            println!("  {}", stats.file_name);
        }
    }

    println!("\n--- Files with multi-sample instruments ---");
    for stats in &file_stats {
        if stats
            .sample_features
            .contains(&"Multi-sample instrument (keymap)".to_string())
        {
            println!("  {}", stats.file_name);
        }
    }

    // Errors
    if !errors.is_empty() {
        println!("\n{}", "=".repeat(80));
        println!("PARSE ERRORS");
        println!("{}", "=".repeat(80));
        for (file, error) in &errors {
            println!("  {}: {}", file, error);
        }
    }

    // Print unsupported features summary
    println!("\n{}", "=".repeat(80));
    println!("UNSUPPORTED FEATURES SUMMARY");
    println!("{}", "=".repeat(80));

    let unsupported_track: Vec<_> = all_track_effects
        .keys()
        .filter(|e| !is_track_effect_supported(e))
        .collect();
    let unsupported_global: Vec<_> = all_global_effects
        .keys()
        .filter(|e| !is_global_effect_supported(e))
        .collect();
    let unsupported_env: Vec<_> = envelope_features
        .keys()
        .filter(|e| !is_envelope_feature_supported(e))
        .collect();
    let unsupported_sample: Vec<_> = sample_features
        .keys()
        .filter(|e| !is_sample_feature_supported(e))
        .collect();

    println!("\nUnsupported track effects ({}):", unsupported_track.len());
    for e in &unsupported_track {
        let count = all_track_effects.get(*e).map(|s| s.len()).unwrap_or(0);
        println!("  - {} ({} files)", e, count);
    }

    println!(
        "\nUnsupported global effects ({}):",
        unsupported_global.len()
    );
    for e in &unsupported_global {
        let count = all_global_effects.get(*e).map(|s| s.len()).unwrap_or(0);
        println!("  - {} ({} files)", e, count);
    }

    println!(
        "\nUnsupported envelope features ({}):",
        unsupported_env.len()
    );
    for e in &unsupported_env {
        let count = envelope_features.get(*e).unwrap_or(&0);
        println!("  - {} ({} files)", e, count);
    }

    println!(
        "\nUnsupported sample features ({}):",
        unsupported_sample.len()
    );
    for e in &unsupported_sample {
        let count = sample_features.get(*e).unwrap_or(&0);
        println!("  - {} ({} files)", e, count);
    }

    // Priority recommendations
    println!("\n{}", "=".repeat(80));
    println!("IMPLEMENTATION PRIORITY RECOMMENDATIONS");
    println!("{}", "=".repeat(80));

    println!("\nHIGH PRIORITY (affects many files):");
    for e in &unsupported_track {
        let count = all_track_effects.get(*e).map(|s| s.len()).unwrap_or(0);
        if count >= 10 {
            println!("  - {} ({} files)", e, count);
        }
    }

    println!("\nMEDIUM PRIORITY (affects some files):");
    for e in &unsupported_track {
        let count = all_track_effects.get(*e).map(|s| s.len()).unwrap_or(0);
        if count >= 3 && count < 10 {
            println!("  - {} ({} files)", e, count);
        }
    }
}

#[derive(Debug)]
struct FileStats {
    file_name: String,
    #[allow(dead_code)]
    format: String,
    channel_count: usize,
    instrument_count: usize,
    sample_count: usize,
    pattern_count: usize,
    track_effects_used: HashSet<String>,
    global_effects_used: HashSet<String>,
    envelope_features: HashSet<String>,
    sample_features: HashSet<String>,
}

fn analyze_file(path: &Path) -> Result<FileStats, String> {
    let data = std::fs::read(path).map_err(|e| format!("Read error: {}", e))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();
    let is_mod_prefix = file_name.to_lowercase().starts_with("mod.");

    let (module, format) = match ext.as_str() {
        "xm" => {
            let xm = XmModule::load(&data).map_err(|e| format!("XM parse: {:?}", e))?;
            (xm.to_module(), "XM")
        }
        "mod" => {
            let amiga = AmigaModule::load(&data).map_err(|e| format!("MOD parse: {:?}", e))?;
            (amiga.to_module(), "MOD")
        }
        "s3m" => {
            let s3m = S3mModule::load(&data).map_err(|e| format!("S3M parse: {:?}", e))?;
            (s3m.to_module(), "S3M")
        }
        _ if is_mod_prefix => {
            let amiga = AmigaModule::load(&data).map_err(|e| format!("MOD parse: {:?}", e))?;
            (amiga.to_module(), "MOD")
        }
        _ => return Err(format!("Unknown format: {}", ext)),
    };

    analyze_module(&module, &file_name, format)
}

fn analyze_module(module: &Module, file_name: &str, format: &str) -> Result<FileStats, String> {
    let mut track_effects_used = HashSet::new();
    let mut global_effects_used = HashSet::new();
    let mut envelope_features = HashSet::new();
    let mut sample_features = HashSet::new();

    // Analyze frequency type
    match module.frequency_type {
        FrequencyType::LinearFrequencies => {
            sample_features.insert("Linear frequencies".to_string());
        }
        FrequencyType::AmigaFrequencies => {
            sample_features.insert("Amiga frequencies".to_string());
        }
    }

    // Analyze instruments
    let mut sample_count = 0;
    for inst in &module.instrument {
        if let InstrumentType::Default(instr) = &inst.instr_type {
            // Count samples
            sample_count += instr.sample.iter().filter(|s| s.is_some()).count();

            // Volume envelope
            let ve = &instr.volume_envelope;
            if ve.enabled {
                envelope_features.insert("Volume envelope enabled".to_string());
                if ve.point.len() > 2 {
                    envelope_features.insert(format!("Volume envelope {} points", ve.point.len()));
                }
                if ve.sustain_enabled {
                    envelope_features.insert("Volume envelope sustain".to_string());
                }
                if ve.loop_enabled {
                    envelope_features.insert("Volume envelope loop".to_string());
                }
            }

            // Panning envelope
            let pe = &instr.pan_envelope;
            if pe.enabled {
                envelope_features.insert("Panning envelope enabled".to_string());
                if pe.sustain_enabled {
                    envelope_features.insert("Panning envelope sustain".to_string());
                }
                if pe.loop_enabled {
                    envelope_features.insert("Panning envelope loop".to_string());
                }
            }

            // Volume fadeout
            if instr.volume_fadeout > 0.0 {
                envelope_features.insert("Volume fadeout".to_string());
            }

            // Auto-vibrato
            if instr.vibrato.speed > 0.0 || instr.vibrato.depth > 0.0 {
                envelope_features.insert("Auto-vibrato".to_string());
            }

            // Sample features
            for sample_opt in &instr.sample {
                if let Some(sample) = sample_opt {
                    // Loop type
                    match sample.loop_flag {
                        xmrs::sample::LoopType::No => {}
                        xmrs::sample::LoopType::Forward => {
                            sample_features.insert("Forward loop".to_string());
                        }
                        xmrs::sample::LoopType::PingPong => {
                            sample_features.insert("Ping-pong loop".to_string());
                        }
                    }

                    // Sample format
                    if let Some(ref data) = sample.data {
                        match data {
                            SampleDataType::Mono8(_) => {
                                sample_features.insert("8-bit mono samples".to_string());
                            }
                            SampleDataType::Mono16(_) => {
                                sample_features.insert("16-bit mono samples".to_string());
                            }
                            SampleDataType::Stereo8(_) => {
                                sample_features.insert("8-bit stereo samples".to_string());
                            }
                            SampleDataType::Stereo16(_) => {
                                sample_features.insert("16-bit stereo samples".to_string());
                            }
                            SampleDataType::StereoFloat(_) => {
                                sample_features.insert("Float stereo samples".to_string());
                            }
                        }
                    }

                    // Finetune/relative pitch
                    if sample.relative_pitch != 0 {
                        sample_features.insert("Relative pitch adjustment".to_string());
                    }

                    // Panning
                    if (sample.panning - 0.5).abs() > 0.01 {
                        sample_features.insert("Sample panning".to_string());
                    }
                }
            }

            // Sample map (multi-sample instruments)
            let unique_samples: HashSet<_> = instr.sample_for_pitch.iter().collect();
            if unique_samples.len() > 1 {
                sample_features.insert("Multi-sample instrument (keymap)".to_string());
            }
        }
    }

    // Analyze patterns for effects
    for pattern in &module.pattern {
        for row in pattern {
            for unit in row {
                // Track effects
                for effect in &unit.effects {
                    let effect_name = describe_track_effect(effect);
                    track_effects_used.insert(effect_name);
                }

                // Global effects
                for effect in &unit.global_effects {
                    let effect_name = describe_global_effect(effect);
                    global_effects_used.insert(effect_name);
                }
            }
        }
    }

    Ok(FileStats {
        file_name: file_name.to_string(),
        format: format.to_string(),
        channel_count: module.get_num_channels(),
        instrument_count: module.instrument.len(),
        sample_count,
        pattern_count: module.pattern.len(),
        track_effects_used,
        global_effects_used,
        envelope_features,
        sample_features,
    })
}

fn describe_track_effect(effect: &TrackEffect) -> String {
    match effect {
        TrackEffect::Arpeggio { half1, half2 } => format!("Arpeggio ({}, {})", half1, half2),
        TrackEffect::ChannelVolume(_) => "ChannelVolume (set volume)".to_string(),
        TrackEffect::ChannelVolumeSlide { fine, .. } => {
            if *fine {
                "ChannelVolumeSlide (fine)".to_string()
            } else {
                "ChannelVolumeSlide".to_string()
            }
        }
        TrackEffect::Glissando(_) => "Glissando".to_string(),
        TrackEffect::InstrumentFineTune(_) => "InstrumentFineTune".to_string(),
        TrackEffect::InstrumentNewNoteAction(_) => "InstrumentNewNoteAction".to_string(),
        TrackEffect::InstrumentPanningEnvelopePosition(_) => {
            "InstrumentPanningEnvelopePosition".to_string()
        }
        TrackEffect::InstrumentPanningEnvelope(_) => "InstrumentPanningEnvelope".to_string(),
        TrackEffect::InstrumentPitchEnvelope(_) => "InstrumentPitchEnvelope".to_string(),
        TrackEffect::InstrumentSampleOffset(_) => "InstrumentSampleOffset".to_string(),
        TrackEffect::InstrumentSurround(_) => "InstrumentSurround".to_string(),
        TrackEffect::InstrumentVolumeEnvelopePosition(_) => {
            "InstrumentVolumeEnvelopePosition".to_string()
        }
        TrackEffect::InstrumentVolumeEnvelope(_) => "InstrumentVolumeEnvelope".to_string(),
        TrackEffect::NoteRetrig { .. } => "NoteRetrig".to_string(),
        TrackEffect::NoteCut { .. } => "NoteCut".to_string(),
        TrackEffect::NoteDelay(_) => "NoteDelay".to_string(),
        TrackEffect::NoteFadeOut { .. } => "NoteFadeOut".to_string(),
        TrackEffect::NoteOff { .. } => "NoteOff".to_string(),
        TrackEffect::Panbrello { .. } => "Panbrello".to_string(),
        TrackEffect::PanbrelloWaveform { .. } => "PanbrelloWaveform".to_string(),
        TrackEffect::Panning(_) => "Panning".to_string(),
        TrackEffect::PanningSlide { fine, .. } => {
            if *fine {
                "PanningSlide (fine)".to_string()
            } else {
                "PanningSlide".to_string()
            }
        }
        TrackEffect::Portamento(speed) => {
            if *speed > 0.0 {
                "PortamentoUp".to_string()
            } else if *speed < 0.0 {
                "PortamentoDown".to_string()
            } else {
                "Portamento".to_string()
            }
        }
        TrackEffect::TonePortamento(_) => "TonePortamento (glide)".to_string(),
        TrackEffect::Tremolo { .. } => "Tremolo".to_string(),
        TrackEffect::TremoloWaveform { .. } => "TremoloWaveform".to_string(),
        TrackEffect::Tremor { .. } => "Tremor".to_string(),
        TrackEffect::Vibrato { .. } => "Vibrato".to_string(),
        TrackEffect::VibratoSpeed(_) => "VibratoSpeed".to_string(),
        TrackEffect::VibratoDepth(_) => "VibratoDepth".to_string(),
        TrackEffect::VibratoWaveform { .. } => "VibratoWaveform".to_string(),
        TrackEffect::Volume { .. } => "Volume (set volume)".to_string(),
        TrackEffect::VolumeSlide { fine, .. } => {
            if *fine {
                "VolumeSlide (fine)".to_string()
            } else {
                "VolumeSlide".to_string()
            }
        }
    }
}

fn describe_global_effect(effect: &GlobalEffect) -> String {
    match effect {
        GlobalEffect::Bpm(_) => "SetBpm".to_string(),
        GlobalEffect::BpmSlide(_) => "BpmSlide".to_string(),
        GlobalEffect::MidiMacro(_) => "MidiMacro".to_string(),
        GlobalEffect::PatternBreak(_) => "PatternBreak".to_string(),
        GlobalEffect::PatternDelay { .. } => "PatternDelay".to_string(),
        GlobalEffect::PatternLoop(_) => "PatternLoop".to_string(),
        GlobalEffect::PositionJump(_) => "PositionJump".to_string(),
        GlobalEffect::Speed(_) => "SetSpeed".to_string(),
        GlobalEffect::Volume(_) => "GlobalVolume".to_string(),
        GlobalEffect::VolumeSlide { fine, .. } => {
            if *fine {
                "GlobalVolumeSlide (fine)".to_string()
            } else {
                "GlobalVolumeSlide".to_string()
            }
        }
    }
}

fn is_track_effect_supported(effect: &str) -> bool {
    // Effects we currently support in our import/playback
    matches!(
        effect,
        "ChannelVolume (set volume)" | "InstrumentSampleOffset"
    )
}

fn is_global_effect_supported(effect: &str) -> bool {
    matches!(
        effect,
        "SetSpeed" | "SetBpm" | "PatternBreak" | "PositionJump"
    )
}

fn is_envelope_feature_supported(feature: &str) -> bool {
    matches!(
        feature,
        "Volume envelope enabled" | "Volume envelope sustain" | "Volume fadeout"
    )
}

fn is_sample_feature_supported(feature: &str) -> bool {
    matches!(
        feature,
        "Forward loop"
            | "Ping-pong loop"
            | "8-bit mono samples"
            | "16-bit mono samples"
            | "8-bit stereo samples"
            | "16-bit stereo samples"
            | "Linear frequencies"
            | "Amiga frequencies"
            | "Relative pitch adjustment"
    )
}
