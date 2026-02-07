//! Analyze tracker files using our internal representation (Cell/EffectCommand).
//!
//! This shows what actually gets imported and played back, after conversion
//! from the raw xmrs format through our import pipeline.
//!
//! For raw xmrs representation, use `analyze_tracker_raw` instead.
//!
//! Usage: cargo run --example analyze_tracker -- /path/to/file.xm

use std::path::Path;

use modular_synth::io::import::import_song;
use synth_sequencer::effects::EffectCommand;
use synth_sequencer::ids::{RowIndex, TrackIndex};
use synth_sequencer::pitch::Pitch;
use synth_sequencer::tracker_pattern::{Cell, TrackerPattern};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <tracker_file>", args[0]);
        eprintln!("Supported formats: .mod, .xm, .s3m");
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    println!("Analyzing (internal representation): {}", path.display());
    println!("{}", "=".repeat(60));

    let imported = match import_song(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Import failed: {e}");
            std::process::exit(1);
        }
    };

    let song = &imported.song;

    // Song metadata
    println!("Song: {}", song.name);
    println!("Author: {}", song.author);
    println!("Default BPM: {}", song.default_tempo.as_f32());
    println!("Default speed: {}", song.default_tracker_speed);
    println!("Frequency mode: {:?}", song.tracker_frequency_mode);
    println!("Tracks: {}", song.track_count());
    println!(
        "Patterns: {} tracker + {} piano",
        song.tracker_pattern_count(),
        song.pattern_count()
    );
    println!("Samples: {}", imported.samples.len());
    println!("Instruments: {}", imported.instruments.len());
    println!();

    // Instrument defaults
    let defaults = song.all_instrument_defaults();
    if !defaults.is_empty() {
        println!("=== Instrument Defaults ===");
        let mut ids: Vec<_> = defaults.keys().collect();
        ids.sort_by_key(|id| id.0);
        for id in ids {
            if let Some(d) = defaults.get(id) {
                let name = imported
                    .instruments
                    .get(id.0 as usize)
                    .map_or("?", |i| &i.name);
                println!(
                    "  Inst {:02X}: vol={:.3} pan={:+.3}  \"{}\"",
                    id.0,
                    d.volume.as_f32(),
                    d.panning.as_f32(),
                    name
                );
            }
        }
        println!();
    }

    // Arrangement (pattern order)
    let arrangement = song.arrangement();
    println!("=== Arrangement ({} placements) ===", arrangement.len());
    for (i, placement) in arrangement.iter().enumerate() {
        println!(
            "  {:02X}: Pattern {:?} at tick {}",
            i, placement.pattern_id, placement.start.0
        );
    }
    println!();

    // Collect and sort patterns by ID
    let mut patterns: Vec<&TrackerPattern> = song.tracker_patterns().collect();
    patterns.sort_by_key(|p| p.id().0);

    // Print each pattern
    for pattern in &patterns {
        print_pattern(pattern);
        println!();
    }
}

/// Print a single tracker pattern in grid format.
fn print_pattern(pattern: &TrackerPattern) {
    let num_tracks = pattern.num_tracks().as_u8() as usize;
    let num_rows = pattern.num_rows().as_u16() as usize;

    println!(
        "=== Pattern {:02X}: \"{}\" ({} rows, {} tracks, {} ticks/row) ===",
        pattern.id().0,
        pattern.name(),
        num_rows,
        num_tracks,
        pattern.ticks_per_row().as_u32(),
    );

    // Header
    print!("    |");
    for ch in 0..num_tracks {
        print!(" T{:<2}            |", ch + 1);
    }
    println!();

    // Separator
    print!("----|");
    for _ in 0..num_tracks {
        print!("-----------------|");
    }
    println!();

    // Rows
    for row_idx in 0..num_rows {
        let ri = RowIndex::new(row_idx as u16);
        print!("{row_idx:02X}  |");
        for ch in 0..num_tracks {
            let ti = TrackIndex::new(ch as u8);
            let row = pattern.get(ti, ri);
            print!(" {} |", format_cell(&row.cell, &row.effects));
        }
        println!();
    }
}

/// Format a cell + effects as a fixed-width string.
///
/// Format: "NOT IN VV EFX1 EFX2" (up to 16 chars)
/// - NOT: note (3 chars: "C-4", "===", "---")
/// - IN: instrument (2 chars: hex or "..")
/// - VV: volume from SetVolume effect (2 chars: hex or "..")
/// - EFX: remaining effects (variable)
fn format_cell(cell: &Cell, effects: &[EffectCommand]) -> String {
    let (note, inst, vel) = match cell {
        Cell::Empty => ("---".to_string(), "..".to_string(), None),
        Cell::NoteOff => ("===".to_string(), "..".to_string(), None),
        Cell::Note {
            pitch,
            instrument,
            velocity,
        } => (
            format_pitch(*pitch),
            instrument.map_or("..".to_string(), |i| format!("{:02X}", i.0)),
            *velocity,
        ),
    };

    // Separate volume (SetVolume) from other effects, showing volume first
    // to match tracker convention (volume column before effect column).
    let mut vol_str = String::new();
    let mut other_effects = Vec::new();

    for effect in effects {
        if let EffectCommand::SetVolume(v) = effect {
            if vol_str.is_empty() {
                vol_str = format!("{v:02X}");
            } else {
                // Multiple SetVolume — show extras as effects
                other_effects.push(format_effect(effect));
            }
        } else {
            other_effects.push(format_effect(effect));
        }
    }

    // If no SetVolume effect, check cell velocity
    if vol_str.is_empty() {
        if let Some(v) = vel {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let vol_byte = (v.as_f32() * 64.0).min(64.0) as u8;
            vol_str = format!("{vol_byte:02X}");
        } else {
            vol_str = "..".to_string();
        }
    }

    // Build effects string
    let efx_str = if other_effects.is_empty() {
        "....".to_string()
    } else {
        other_effects.join(" ")
    };

    format!("{note} {inst} {vol_str} {efx_str:<4}")
}

/// Format a Pitch as "C-4", "C#4", etc.
fn format_pitch(pitch: Pitch) -> String {
    let midi = pitch.as_midi();
    let octave = midi / 12;
    let semitone = midi % 12;
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
    format!("{name}{octave}")
}

/// Format an EffectCommand as a short code.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_effect(effect: &EffectCommand) -> String {
    match effect {
        EffectCommand::Arpeggio { x, y } => format!("0{x:X}{y:X}"),
        EffectCommand::PortamentoUp(s) => format!("1{s:02X}"),
        EffectCommand::PortamentoDown(s) => format!("2{s:02X}"),
        EffectCommand::TonePortamento { speed, .. } => format!("3{speed:02X}"),
        EffectCommand::Vibrato { speed, depth } => format!("4{speed:X}{depth:X}"),
        EffectCommand::VibratoWaveform(_) => "E4x".to_string(),
        EffectCommand::Glissando(on) => format!("E3{}", u8::from(*on)),
        EffectCommand::FineTune(c) => format!("E5{:02X}", *c as u8),
        EffectCommand::SetVolume(v) => format!("C{v:02X}"),
        EffectCommand::VolumeSlide { up, down } => {
            if *up > 0 {
                format!("A{up:X}0")
            } else {
                format!("A0{down:X}")
            }
        }
        EffectCommand::FineVolumeSlide { up, down } => {
            if *up > 0 {
                format!("EA{up:X}")
            } else {
                format!("EB{down:X}")
            }
        }
        EffectCommand::Tremolo { speed, depth } => format!("7{speed:X}{depth:X}"),
        EffectCommand::TremoloWaveform(_) => "E7x".to_string(),
        EffectCommand::SetPanning(p) => format!("8{p:02X}"),
        EffectCommand::PanningSlide { left, right } => {
            if *left > 0 {
                format!("P{left:X}0")
            } else {
                format!("P0{right:X}")
            }
        }
        EffectCommand::SampleOffset(o) => format!("9{:02X}", (*o >> 8) as u8),
        EffectCommand::Retrigger { interval, .. } => format!("R{interval:02X}"),
        EffectCommand::NoteCut(t) => format!("SC{t:X}"),
        EffectCommand::NoteDelay(t) => format!("SD{t:X}"),
        EffectCommand::NoteFadeOut(t) => format!("KF{t:X}"),
        EffectCommand::Reverse => "REV".to_string(),
        EffectCommand::SetTempo(t) => format!("F{t:02X}"),
        EffectCommand::SetSpeed(s) => format!("F{s:02X}"),
        EffectCommand::PatternDelay(r) => format!("EE{r:X}"),
        EffectCommand::PatternLoop { count } => format!("E6{count:X}"),
        EffectCommand::PatternJump(p) => format!("B{p:02X}"),
        EffectCommand::PatternBreak(r) => format!("D{r:02X}"),
    }
}
