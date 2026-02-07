//! Debug playback: step through a tracker module tick-by-tick and log events.
//!
//! Loads a tracker file, steps through a specific pattern region, and prints
//! every NoteOn/NoteOff/Modulation event for selected tracks.
//!
//! Usage: cargo run --example debug_playback -- <file> [pattern_idx] [tracks]
//!   e.g.: cargo run --example debug_playback -- song.xm 2 12,13,14

use std::path::Path;
use std::sync::{Arc, RwLock};

use modular_synth::io::import::import_song;
use synth_core::{SampleCount, SampleRate};
use synth_engine::SequencerEngine;
use synth_sequencer::SequencerEvent;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} <tracker_file> [pattern_index] [track1,track2,...]",
            args[0]
        );
        eprintln!("  pattern_index: 0-indexed arrangement position (default: 0)");
        eprintln!("  tracks: comma-separated 0-indexed track numbers (default: all)");
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    let pattern_idx: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let track_filter: Option<Vec<u16>> = args
        .get(3)
        .map(|s| s.split(',').filter_map(|t| t.trim().parse().ok()).collect());

    println!("Loading: {}", path.display());
    let imported = match import_song(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Import failed: {e}");
            std::process::exit(1);
        }
    };

    let song = &imported.song;
    println!(
        "Song: {} (BPM={}, speed={})",
        song.name,
        song.default_tempo.as_f32(),
        song.default_tracker_speed
    );
    println!("Frequency mode: {:?}", song.tracker_frequency_mode);
    println!();

    // Find the pattern region to debug
    let arrangement = song.arrangement();
    if pattern_idx >= arrangement.len() {
        eprintln!(
            "Pattern index {pattern_idx} out of range (arrangement has {} entries)",
            arrangement.len()
        );
        std::process::exit(1);
    }

    let placement = &arrangement[pattern_idx];
    let start_tick = placement.start;
    let pattern_id = placement.pattern_id;

    // Get pattern info
    let (ticks_per_row, pattern_end) = if let Some(tp) = song.tracker_pattern(pattern_id) {
        let rows = tp.num_rows().as_u16() as u32;
        let tpr = tp.ticks_per_row().as_u32();
        println!(
            "Pattern {:02X}: {} rows, {} ticks/row = {} total ticks",
            pattern_id.0,
            rows,
            tpr,
            rows * tpr
        );
        println!(
            "Arrangement position {pattern_idx}: tick {} to {}",
            start_tick.0,
            start_tick.0 + u64::from(rows * tpr)
        );
        (u64::from(tpr), start_tick.0 + u64::from(rows * tpr))
    } else {
        eprintln!("Pattern {:?} not found as tracker pattern", pattern_id);
        std::process::exit(1);
    };

    // Show instrument defaults for context
    println!();
    println!("=== Instrument Defaults ===");
    let defaults = song.all_instrument_defaults();
    let mut ids: Vec<_> = defaults.keys().collect();
    ids.sort_by_key(|id| id.0);
    for id in &ids {
        if let Some(d) = defaults.get(id) {
            println!(
                "  Inst {:02X}: vol={:.3} pan={:+.3}",
                id.0,
                d.volume.as_f32(),
                d.panning.as_f32()
            );
        }
    }

    if let Some(ref tracks) = track_filter {
        println!();
        println!("Filtering tracks: {:?}", tracks);
    }

    println!();
    println!("=== Stepping through pattern ===");
    println!();

    // Create engine and step through
    let song_arc = Arc::new(RwLock::new(imported.song));
    let sample_rate = SampleRate::CD_QUALITY;
    let mut engine = SequencerEngine::with_song(Arc::clone(&song_arc), sample_rate);

    // Calculate samples per internal tick
    // At 134 BPM, 44100 Hz: ticks_per_sec = 134/60 * 960 = 2144
    // samples_per_tick = 44100 / 2144 ≈ 20.6
    let bpm = song_arc.read().map_or(120.0, |s| s.default_tempo.as_f32());
    let ticks_per_sec = (bpm / 60.0) * 960.0;
    let samples_per_tick = (sample_rate.as_f32() / ticks_per_sec).ceil() as usize;

    engine.play();

    let mut events = Vec::new();
    let mut last_printed_tick: Option<u64> = None;
    let mut event_count = 0u32;
    let mut modulation_count = 0u32;

    // Step tick by tick until we pass the pattern
    while engine.current_tick().0 < pattern_end {
        events.clear();
        engine.process(SampleCount::new(samples_per_tick), &mut events);

        // Only show events within our pattern region
        for event in &events {
            let tick = event.tick().0;
            if tick < start_tick.0 {
                continue;
            }

            // Filter by track if specified
            if let Some(ref tracks) = track_filter {
                let et = event_track_id(event);
                if let Some(tid) = et {
                    if !tracks.contains(&tid) {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            // Print tick header when it changes
            if last_printed_tick != Some(tick) {
                let pattern_tick = tick.saturating_sub(start_tick.0);
                let row = pattern_tick / ticks_per_row;
                let tick_in_row = pattern_tick % ticks_per_row;
                let tracker_tick = tick_in_row / 40; // SONG_TICKS_PER_TRACKER_TICK
                println!(
                    "--- Tick {} (row {:02X}, tracker tick {}) ---",
                    tick, row, tracker_tick
                );
                last_printed_tick = Some(tick);
            }

            event_count += 1;
            print_event(event);

            if event.is_modulation() {
                modulation_count += 1;
            }
        }
    }

    println!();
    println!("=== Summary ===");
    println!("Events printed: {event_count}");
    println!("  Modulation events: {modulation_count}");
}

fn event_track_id(event: &SequencerEvent) -> Option<u16> {
    match event {
        SequencerEvent::NoteOn { voice_index, .. } => voice_index.map(|v| u16::from(v.as_u8())),
        SequencerEvent::NoteOff { .. } => None,
        SequencerEvent::Modulation { track, .. } => Some(track.0),
        SequencerEvent::Effect { .. } => None,
        SequencerEvent::Parameter { .. } => None,
        SequencerEvent::VoiceOff { voice_index, .. } => Some(u16::from(voice_index.as_u8())),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn print_event(event: &SequencerEvent) {
    match event {
        SequencerEvent::NoteOn {
            pitch,
            velocity,
            instrument,
            effects,
            voice_index,
            sample_offset,
            ..
        } => {
            let voice = voice_index.map_or("?".to_string(), |v| format!("{}", v.as_u8()));
            let offset = if sample_offset.as_f32() > 0.0 {
                format!(" offset={:.3}", sample_offset.as_f32())
            } else {
                String::new()
            };
            println!(
                "  NoteOn  voice={} pitch={} vel={:.3} inst={:02X} effects={:?}{}",
                voice,
                format_pitch(pitch.as_midi()),
                velocity.as_f32(),
                instrument.0,
                effects,
                offset,
            );
        }
        SequencerEvent::NoteOff {
            pitch, instrument, ..
        } => {
            println!(
                "  NoteOff pitch={} inst={:02X}",
                format_pitch(pitch.as_midi()),
                instrument.0,
            );
        }
        SequencerEvent::Modulation {
            track,
            pitch_cents,
            volume,
            panning,
            note_triggered,
            note_cut,
            ..
        } => {
            let mut flags = Vec::new();
            if *note_triggered {
                flags.push("TRIG");
            }
            if *note_cut {
                flags.push("CUT");
            }
            let flags_str = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(","))
            };
            println!(
                "  Mod     T{:02} vol={:.4} pitch={:+.2}ct pan={:+.3}{}",
                track.0,
                volume.as_f32(),
                pitch_cents.as_f32(),
                panning.as_f32(),
                flags_str,
            );
        }
        SequencerEvent::Effect { effect, .. } => {
            println!("  Effect  {:?}", effect);
        }
        SequencerEvent::VoiceOff { voice_index, .. } => {
            println!("  VoiceOff voice={}", voice_index.as_u8());
        }
        SequencerEvent::Parameter { target, value, .. } => {
            println!("  Param   {:?} = {:.3}", target, value);
        }
    }
}

fn format_pitch(midi: u8) -> String {
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
