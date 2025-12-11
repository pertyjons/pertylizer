//! Tracker file import using the xmrs crate.
//!
//! Supports MOD, XM, and S3M formats.

use std::path::Path;
use std::sync::Arc;

use xmrs::import::amiga::amiga_module::AmigaModule;
use xmrs::import::s3m::s3m_module::S3mModule;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::prelude::{
    InstrumentType, Module, Pattern as XmrsPattern, Sample as XmrsSample, SampleDataType, TrackUnit,
};
use xmrs::sample::LoopType as XmrsLoopType;

use super::{ImportError, ImportResult, ImportedSong, SongImporter};
use crate::sequencer::pattern::RowResolution;
use crate::sequencer::pitch::{Pitch, Velocity};
use crate::sequencer::time::Duration;
use crate::sequencer::{PatternId, SeqInstrumentId, Song, Tick, TrackId};
use crate::types::{Bpm, ChannelMode, MidiNote, Sample, SampleLoopInfo, SampleRate, SampleValue};

/// Importer for tracker files (MOD, XM, S3M).
pub struct TrackerImporter;

impl TrackerImporter {
    /// Create a new tracker importer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for TrackerImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl SongImporter for TrackerImporter {
    fn name(&self) -> &str {
        "Tracker (MOD/XM/S3M)"
    }

    fn extensions(&self) -> &[&str] {
        &["mod", "xm", "s3m"]
    }

    fn import(&self, path: &Path) -> ImportResult<ImportedSong> {
        // Read file
        let data = std::fs::read(path)?;

        // Try to load as XM/MOD/S3M using xmrs
        let module = load_tracker_module(&data, path)?;

        // Convert to our Song format
        convert_module_to_song(module, path)
    }
}

/// Load a tracker module from binary data.
fn load_tracker_module(data: &[u8], path: &Path) -> ImportResult<Module> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    match ext.as_str() {
        "xm" => {
            let xm = XmModule::load(data)
                .map_err(|e| ImportError::Parse(format!("XM parse error: {e}")))?;
            Ok(xm.to_module())
        }
        "mod" => {
            let amiga = AmigaModule::load(data)
                .map_err(|e| ImportError::Parse(format!("MOD parse error: {e}")))?;
            Ok(amiga.to_module())
        }
        "s3m" => {
            let s3m = S3mModule::load(data)
                .map_err(|e| ImportError::Parse(format!("S3M parse error: {e}")))?;
            Ok(s3m.to_module())
        }
        _ => Err(ImportError::UnsupportedFormat(format!(
            "Unknown extension: {ext}"
        ))),
    }
}

/// Convert an xmrs Module to our Song format.
fn convert_module_to_song(module: Module, path: &Path) -> ImportResult<ImportedSong> {
    let mut song = Song::new(&module.name);

    // Set author from file name if module doesn't have one
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        song.author = stem.to_string();
    }

    // Set tempo from module defaults
    let bpm = module.default_bpm as f32;
    let speed = module.default_tempo as f32;
    // Tracker tempo formula: actual BPM = (bpm * 2.5) / speed
    // Standard: speed=6, bpm=125 -> 125 * 2.5 / 6 = 52 BPM
    // But for simplicity, we use the BPM value directly
    song.default_tempo = Bpm::new(bpm.clamp(60.0, 300.0));

    // Extract samples from instruments
    let samples = extract_samples(&module)?;

    // Get number of channels
    let num_channels = module.get_num_channels();

    // Create a single track for simplicity (tracker patterns span all channels)
    let track_id = song.create_track("Main");

    // Convert patterns
    let mut pattern_ids = Vec::new();
    for (pat_idx, pattern_data) in module.pattern.iter().enumerate() {
        let pattern_id = convert_pattern(&mut song, pattern_data, pat_idx, num_channels, speed)?;
        pattern_ids.push(pattern_id);
    }

    // Create arrangement from pattern order (use first subsong)
    let mut current_tick = Tick(0);
    let pattern_order = module.pattern_order.first().cloned().unwrap_or_default();
    for order_idx in pattern_order {
        if order_idx >= pattern_ids.len() || order_idx == 255 {
            // 255 is often "end of song" marker
            continue;
        }

        let pattern_id = pattern_ids[order_idx];
        let pattern = song
            .pattern(pattern_id)
            .ok_or_else(|| ImportError::InvalidData("Pattern not found".to_string()))?;

        let length = pattern.length;

        song.place_pattern(pattern_id, track_id, current_tick);
        current_tick = Tick(current_tick.0 + length.0 as u64);
    }

    Ok(ImportedSong { song, samples })
}

/// Extract samples from module instruments.
fn extract_samples(module: &Module) -> ImportResult<Vec<Arc<Sample>>> {
    let mut samples = Vec::new();

    for (inst_idx, instrument) in module.instrument.iter().enumerate() {
        // InstrDefault has a sample vector - match on the instrument type
        if let InstrumentType::Default(instr) = &instrument.instr_type {
            for (smp_idx, sample_opt) in instr.sample.iter().enumerate() {
                if let Some(xmrs_sample) = sample_opt {
                    let sample = convert_sample(xmrs_sample, inst_idx, smp_idx)?;
                    samples.push(Arc::new(sample));
                }
            }
        }
    }

    Ok(samples)
}

/// Convert an xmrs Sample to our Sample type.
fn convert_sample(
    xmrs_sample: &XmrsSample,
    inst_idx: usize,
    smp_idx: usize,
) -> ImportResult<Sample> {
    let name = if xmrs_sample.name.is_empty() {
        format!("Inst{inst_idx:02X}_Smp{smp_idx:02X}")
    } else {
        xmrs_sample.name.clone()
    };

    // Convert sample data to f32
    let (data, channels) = match &xmrs_sample.data {
        Some(sample_data) => convert_sample_data(sample_data)?,
        None => (Vec::new(), ChannelMode::Mono),
    };

    // Tracker samples typically use 8363 Hz as base rate for C-4
    // relative_pitch adjusts this
    let base_rate = 8363.0_f32;
    let rate_multiplier = 2.0_f32.powf(f32::from(xmrs_sample.relative_pitch) / 12.0);
    let sample_rate = SampleRate::new(base_rate * rate_multiplier);

    // Calculate root note from relative_pitch
    // relative_pitch = 0 means C-4 (MIDI 60)
    let root_midi = (60 + i16::from(xmrs_sample.relative_pitch)).clamp(0, 127) as u8;
    let root_note = MidiNote::new(root_midi);

    // Calculate sample length in frames
    let frame_count = data.len() / channels.channel_count();

    // Extract loop information
    let loop_info = if matches!(
        xmrs_sample.loop_flag,
        XmrsLoopType::Forward | XmrsLoopType::PingPong
    ) && xmrs_sample.loop_length > 0
        && frame_count > 0
    {
        let loop_start = xmrs_sample.loop_start as f32 / frame_count as f32;
        let loop_end =
            (xmrs_sample.loop_start + xmrs_sample.loop_length) as f32 / frame_count as f32;
        Some(SampleLoopInfo {
            loop_start: loop_start.clamp(0.0, 1.0),
            loop_end: loop_end.clamp(0.0, 1.0),
            enabled: true,
            ping_pong: matches!(xmrs_sample.loop_flag, XmrsLoopType::PingPong),
        })
    } else {
        None
    };

    // Extract default volume (xmrs uses 0.0-1.0)
    let default_volume = xmrs_sample.volume;

    let mut sample = Sample::new(name, data, channels, sample_rate).with_root_note(root_note);

    if let Some(loop_info) = loop_info {
        sample = sample.with_loop_info(loop_info);
    }

    sample = sample.with_default_volume(default_volume);

    Ok(sample)
}

/// Convert xmrs sample data to Vec<SampleValue>.
fn convert_sample_data(data: &SampleDataType) -> ImportResult<(Vec<SampleValue>, ChannelMode)> {
    match data {
        SampleDataType::Mono8(samples) => {
            let converted: Vec<SampleValue> = samples
                .iter()
                .map(|&s| SampleValue::new(f32::from(s) / 128.0))
                .collect();
            Ok((converted, ChannelMode::Mono))
        }
        SampleDataType::Mono16(samples) => {
            let converted: Vec<SampleValue> = samples
                .iter()
                .map(|&s| SampleValue::new(f32::from(s) / 32768.0))
                .collect();
            Ok((converted, ChannelMode::Mono))
        }
        SampleDataType::Stereo8(samples) => {
            // Stereo samples are interleaved
            let converted: Vec<SampleValue> = samples
                .iter()
                .map(|&s| SampleValue::new(f32::from(s) / 128.0))
                .collect();
            Ok((converted, ChannelMode::Stereo))
        }
        SampleDataType::Stereo16(samples) => {
            let converted: Vec<SampleValue> = samples
                .iter()
                .map(|&s| SampleValue::new(f32::from(s) / 32768.0))
                .collect();
            Ok((converted, ChannelMode::Stereo))
        }
        SampleDataType::StereoFloat(samples) => {
            let converted: Vec<SampleValue> =
                samples.iter().map(|&s| SampleValue::new(s)).collect();
            Ok((converted, ChannelMode::Stereo))
        }
    }
}

/// Convert an xmrs pattern to our Pattern format.
fn convert_pattern(
    song: &mut Song,
    pattern_data: &XmrsPattern,
    pat_idx: usize,
    num_channels: usize,
    speed: f32,
) -> ImportResult<PatternId> {
    // Pattern data is organized as: pattern[row][channel] = Vec<Vec<TrackUnit>>
    let num_rows = pattern_data.len();
    if num_rows == 0 {
        // Empty pattern - create minimal
        let pattern_id = song.create_pattern(Duration(960)); // One bar
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            pattern.name = format!("Pattern {pat_idx:02X}");
        }
        return Ok(pattern_id);
    }

    // Calculate ticks per row based on speed
    // Standard: speed=6 means 6 ticks per row
    // We use 960 PPQN, so we scale accordingly
    // At speed 6 and BPM 125: one row = 6 ticks = 1/4 beat
    // Our quarter note = 960 ticks, so one tracker row ≈ 960/4 = 240 ticks at speed 6
    let ticks_per_row = (240.0 * speed / 6.0) as u32;
    let pattern_length = Duration(num_rows as u32 * ticks_per_row);

    let pattern_id = song.create_pattern(pattern_length);
    let pattern = song
        .pattern_mut(pattern_id)
        .ok_or_else(|| ImportError::InvalidData("Failed to get pattern".to_string()))?;

    pattern.name = format!("Pattern {pat_idx:02X}");

    // Set up row resolution for tracker view
    pattern.row_resolution = RowResolution::custom(num_rows as u16, ticks_per_row as u16);

    // Track state per channel (for volume/instrument memory)
    let mut channel_state: Vec<ChannelState> = vec![ChannelState::default(); num_channels];

    // Convert each row
    for (row_idx, row_data) in pattern_data.iter().enumerate() {
        let row_tick = crate::sequencer::PatternTick(row_idx as u32 * ticks_per_row);

        for (channel_idx, track_unit) in row_data.iter().enumerate() {
            if channel_idx >= num_channels {
                break;
            }

            // Update channel state and create note if needed
            #[allow(clippy::cast_possible_truncation)]
            let track_id = TrackId::new(channel_idx as u16);
            if let Some(note_event) =
                process_track_unit(track_unit, &mut channel_state[channel_idx], track_id)
            {
                // Create note with track info for mono-per-track behavior
                let note = crate::sequencer::Note::new(
                    crate::sequencer::NoteId(0), // ID will be reassigned by insert_note
                    row_tick,
                    note_event.pitch,
                    note_event.velocity,
                    note_event.instrument,
                )
                .with_track(note_event.track);
                pattern.insert_note(note);
            }
        }
    }

    Ok(pattern_id)
}

/// Channel state for tracking volume/instrument between rows.
#[derive(Default, Clone)]
struct ChannelState {
    last_instrument: usize,
    last_volume: f32, // 0.0-1.0
}

/// A note event to be added to the pattern.
struct NoteEvent {
    pitch: Pitch,
    velocity: Velocity,
    instrument: SeqInstrumentId,
    track: TrackId,
}

/// Process a track unit and return a note event if one should be triggered.
fn process_track_unit(
    unit: &TrackUnit,
    state: &mut ChannelState,
    track: TrackId,
) -> Option<NoteEvent> {
    // Check for instrument change
    if let Some(inst) = unit.instrument {
        state.last_instrument = inst;
    }

    // Use the velocity from the track unit (0.0-1.0)
    if unit.velocity > 0.0 {
        state.last_volume = unit.velocity;
    }

    // Check for note using the Pitch enum
    // Pitch::None and Pitch::Off should not trigger a note
    if unit.note.is_none() || unit.note.is_keyoff() {
        return None;
    }

    // Get the note value as u8 (C0=0, C1=12, etc.)
    let note_value = unit.note as u8;

    // xmrs Pitch values match MIDI directly (C0=0, C1=12, etc.)
    // But tracker convention often has C0 = MIDI 12 (C-1)
    // Let's shift up by 12 to be safe
    let midi_note = note_value.saturating_add(12).min(127);

    let pitch = Pitch::new(midi_note)?;
    let velocity = Velocity::new(state.last_volume.max(0.5)); // Default to 0.5 if no volume set

    #[allow(clippy::cast_possible_truncation)]
    let instrument = SeqInstrumentId(state.last_instrument as u16);

    Some(NoteEvent {
        pitch,
        velocity,
        instrument,
        track,
    })
}
