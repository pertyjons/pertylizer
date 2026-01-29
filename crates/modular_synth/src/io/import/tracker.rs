//! Tracker file import using the xmrs crate.
//!
//! Supports MOD, XM, and S3M formats.

use std::path::Path;
use std::sync::Arc;

use xmrs::import::amiga::amiga_module::AmigaModule;
use xmrs::import::s3m::s3m_module::S3mModule;
use xmrs::import::xm::xmmodule::XmModule;
use xmrs::period_helper::FrequencyType;
use xmrs::prelude::{
    InstrumentType, Module, Pattern as XmrsPattern, Sample as XmrsSample, SampleDataType, TrackUnit,
};
use xmrs::sample::LoopType as XmrsLoopType;

use xmrs::effect::{GlobalEffect, TrackEffect};

use super::{
    ImportError, ImportResult, ImportedAdsr, ImportedInstrument, ImportedSong, SongImporter,
};
use synth_core::{
    BipolarValue, Bpm, ChannelMode, Gain, MidiNote, NormalizedValue, Sample, SampleLoopInfo,
    SampleRate, SampleValue, Seconds, VoiceCount, VoiceIndex,
};
use synth_sequencer::effects::EffectCommand;
use synth_sequencer::pitch::{Pitch, Velocity};
use synth_sequencer::time::Duration;
use synth_sequencer::track::TrackMode;
use synth_sequencer::tracker_pattern::{Cell, TrackerPattern};
use synth_sequencer::{
    PatternId, RowCount, RowIndex, SeqInstrumentId, Song, Tick, TicksPerRow, TrackCount, TrackId,
    TrackIndex,
};

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

    // Check for "mod." prefix (e.g., "mod.echoing" instead of "echoing.mod")
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    let is_mod_prefix = file_name.starts_with("mod.");

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
        _ if is_mod_prefix => {
            // Handle "mod.filename" format (common on Amiga)
            let amiga = AmigaModule::load(data)
                .map_err(|e| ImportError::Parse(format!("MOD parse error: {e}")))?;
            Ok(amiga.to_module())
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
    // XM modules have two tempo parameters:
    // - default_bpm: The BPM value (typically 125)
    // - default_tempo: The "speed" value (ticks per row, typically 6)
    // The actual playback speed depends on both values.
    let bpm = module.default_bpm as f32;
    let speed = module.default_tempo as u8;
    song.default_tempo = Bpm::new(bpm.clamp(60.0, 300.0));
    song.default_tracker_speed = speed.max(1); // Prevent division by zero

    // Extract samples from instruments
    let samples = extract_samples(&module, module.frequency_type)?;

    // Extract instrument metadata with envelope info
    let instruments = extract_instruments(&module, bpm);

    // Get number of channels
    let num_channels = module.get_num_channels();

    // Create a track per channel for tracker-style mono-per-channel playback
    // Each channel gets a fixed voice via TrackMode::MonoVoice
    let mut track_ids = Vec::with_capacity(num_channels);
    for ch in 0..num_channels {
        let track_id = song.create_track(format!("Ch{}", ch + 1));
        // Set mono-voice mode: channel i uses voice i
        if let Some(track) = song.track_mut(track_id) {
            track.mode = TrackMode::MonoVoice(VoiceIndex::new(ch as u8));
        }
        track_ids.push(track_id);
    }

    // Use the first track for pattern placement (patterns span all channels)
    let main_track_id = track_ids.first().copied().unwrap_or(TrackId::new(0));

    // Convert patterns to TrackerPattern format
    let mut pattern_ids = Vec::new();
    for (pat_idx, pattern_data) in module.pattern.iter().enumerate() {
        let tracker_pattern =
            convert_pattern_to_tracker(pattern_data, pat_idx, num_channels, f32::from(speed))?;
        let pattern_id = song.add_tracker_pattern(tracker_pattern);
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
        let tracker_pattern = song
            .tracker_pattern(pattern_id)
            .ok_or_else(|| ImportError::InvalidData("TrackerPattern not found".to_string()))?;

        // Calculate length from TrackerPattern
        let length_ticks = tracker_pattern.length_ticks();
        // Convert PatternTick to Duration (both are u32)
        let length = Duration(length_ticks.0);

        song.place_pattern(pattern_id, main_track_id, current_tick);
        current_tick = Tick(current_tick.0 + length.0 as u64);
    }

    // Calculate minimum voice count needed (one per channel)
    let min_voices = Some(VoiceCount::new(num_channels.max(1) as u8));

    Ok(ImportedSong {
        song,
        samples,
        instruments,
        min_voices,
    })
}

/// Extract samples from module instruments.
fn extract_samples(module: &Module, freq_type: FrequencyType) -> ImportResult<Vec<Arc<Sample>>> {
    let mut samples = Vec::new();

    for (inst_idx, instrument) in module.instrument.iter().enumerate() {
        // InstrDefault has a sample vector - match on the instrument type
        if let InstrumentType::Default(instr) = &instrument.instr_type {
            for (smp_idx, sample_opt) in instr.sample.iter().enumerate() {
                if let Some(xmrs_sample) = sample_opt {
                    let sample = convert_sample(xmrs_sample, inst_idx, smp_idx, freq_type)?;
                    samples.push(Arc::new(sample));
                }
            }
        }
    }

    Ok(samples)
}

/// Extract instrument metadata with envelope info.
///
/// Extracts both the ADSR approximation (for legacy compatibility) and
/// the raw envelope points (for MultiPointEnvelope).
fn extract_instruments(module: &Module, bpm: f32) -> Vec<ImportedInstrument> {
    let mut sample_offset = 0usize;

    module
        .instrument
        .iter()
        .enumerate()
        .map(|(idx, inst)| {
            match &inst.instr_type {
                InstrumentType::Default(instr) => {
                    // Convert envelope to ADSR (legacy compatibility)
                    let volume_envelope =
                        convert_envelope_to_adsr(&instr.volume_envelope, instr.volume_fadeout, bpm);

                    // Extract raw envelope points for MultiPointEnvelope
                    // xmrs uses usize for frame, we convert to u16 (safe for tracker formats)
                    #[allow(clippy::cast_possible_truncation)]
                    let envelope_points: Vec<(u16, f32)> = instr
                        .volume_envelope
                        .point
                        .iter()
                        .map(|p| (p.frame as u16, p.value))
                        .collect();

                    // Extract sustain point
                    let envelope_sustain = if instr.volume_envelope.sustain_enabled
                        && instr.volume_envelope.sustain_start_point < envelope_points.len()
                    {
                        #[allow(clippy::cast_possible_truncation)]
                        Some(instr.volume_envelope.sustain_start_point as u8)
                    } else {
                        None
                    };

                    // Extract loop region
                    let envelope_loop = if instr.volume_envelope.loop_enabled
                        && instr.volume_envelope.loop_start_point < envelope_points.len()
                        && instr.volume_envelope.loop_end_point < envelope_points.len()
                    {
                        #[allow(clippy::cast_possible_truncation)]
                        Some((
                            instr.volume_envelope.loop_start_point as u8,
                            instr.volume_envelope.loop_end_point as u8,
                        ))
                    } else {
                        None
                    };

                    // Find first sample index for this instrument
                    // Samples are flattened across all instruments, so we track offset
                    let sample_count = instr.sample.iter().filter(|s| s.is_some()).count();
                    let first_sample_index = sample_offset;
                    let sample_index = if sample_count > 0 {
                        sample_offset += sample_count;
                        Some(first_sample_index)
                    } else {
                        None
                    };

                    // Extract keymap (sample_for_pitch) if instrument has multiple samples
                    // The keymap maps MIDI note 0-119 to sample index within this instrument
                    let sample_keymap = if sample_count > 1 {
                        // Check if keymap is used (not all same value)
                        let unique_samples: std::collections::HashSet<_> =
                            instr.sample_for_pitch.iter().filter_map(|&s| s).collect();
                        if unique_samples.len() > 1 {
                            // Convert to our format: vec of sample indices
                            Some(
                                instr
                                    .sample_for_pitch
                                    .iter()
                                    .map(|&s| s.unwrap_or(0))
                                    .collect(),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // Convert pan from 0.0-1.0 to -1.0 to 1.0
                    let default_pan = BipolarValue::new(instr.default_pan * 2.0 - 1.0);

                    ImportedInstrument {
                        name: inst.name.clone(),
                        sample_index,
                        sample_keymap,
                        sample_count,
                        volume_envelope,
                        envelope_points,
                        envelope_sustain,
                        envelope_loop,
                        fadeout: instr.volume_fadeout,
                        global_volume: Gain::new(instr.global_volume),
                        default_pan,
                    }
                }
                _ => ImportedInstrument {
                    name: format!("Instrument {}", idx + 1),
                    sample_index: None,
                    sample_keymap: None,
                    sample_count: 0,
                    volume_envelope: ImportedAdsr::default(),
                    envelope_points: Vec::new(),
                    envelope_sustain: None,
                    envelope_loop: None,
                    fadeout: 0.0,
                    global_volume: Gain::UNITY,
                    default_pan: BipolarValue::CENTER,
                },
            }
        })
        .collect()
}

/// Convert XM envelope points to ADSR parameters.
///
/// XM envelope format:
/// - Points with (frame, value) where frame is in ticks
/// - Tick rate ≈ BPM * 2 / 5 Hz (at standard speed=6)
/// - Sustain point = where envelope holds during note-on
fn convert_envelope_to_adsr(
    envelope: &xmrs::envelope::Envelope,
    volume_fadeout: f32,
    bpm: f32,
) -> ImportedAdsr {
    if !envelope.enabled || envelope.point.is_empty() {
        return ImportedAdsr::default();
    }

    // XM tick rate: at BPM=125, speed=6: 125*2/5 = 50 ticks/second
    let tick_rate = (bpm * 2.0 / 5.0).max(1.0);
    let secs_per_tick = 1.0 / tick_rate;

    // Find peak (highest value)
    let peak_idx = envelope
        .point
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.value
                .partial_cmp(&b.value)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    let peak_point = &envelope.point[peak_idx];

    // Attack = time to peak
    let attack_secs = if peak_idx > 0 && peak_point.frame > 0 {
        (peak_point.frame as f32 * secs_per_tick).max(0.001)
    } else {
        0.01 // Minimal attack
    };

    // Sustain point and decay
    let (decay_secs, sustain_level) =
        if envelope.sustain_enabled && envelope.sustain_start_point < envelope.point.len() {
            let sustain_point = &envelope.point[envelope.sustain_start_point];
            let level = sustain_point.value.clamp(0.0, 1.0);

            // Decay = time from peak to sustain
            let decay = if envelope.sustain_start_point > peak_idx {
                let decay_frames = sustain_point.frame.saturating_sub(peak_point.frame);
                (decay_frames as f32 * secs_per_tick).max(0.001)
            } else {
                0.1 // Default decay
            };
            (decay, level)
        } else {
            (0.1, 1.0) // Defaults
        };

    // Release based on fadeout
    // XM fadeout: value represents how fast volume decreases
    // Higher value = faster fade = shorter release
    let release_secs = if volume_fadeout > 0.001 {
        // Invert: high fadeout = short release
        (1.0 / volume_fadeout).clamp(0.01, 10.0)
    } else {
        0.3 // Default release
    };

    ImportedAdsr {
        enabled: true,
        attack: Seconds::new(attack_secs),
        decay: Seconds::new(decay_secs),
        sustain: NormalizedValue::new(sustain_level),
        release: Seconds::new(release_secs),
    }
}

/// Convert an xmrs Sample to our Sample type.
fn convert_sample(
    xmrs_sample: &XmrsSample,
    inst_idx: usize,
    smp_idx: usize,
    freq_type: FrequencyType,
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

    // Calculate base frequency based on frequency type (Linear vs Amiga)
    // - Linear mode: Uses 8363 Hz as C-4 rate with standard 2^(semitones/12) scaling
    // - Amiga mode: Uses period-based calculations from Paula chipset
    //
    // For Amiga mode, C-4 frequency ≈ 8287 Hz (slightly different from Linear's 8363 Hz)
    // We calculate the C-4 frequency for the given mode and use that as base.
    let base_rate = match freq_type {
        FrequencyType::LinearFrequencies => 8363.0_f32,
        FrequencyType::AmigaFrequencies => {
            // Amiga C-4 calculation:
            // period = 6848.0 * e^(-0.0578 * 48) + 0.2782 ≈ 428.46
            // frequency = 7093789.2 / (period * 2) ≈ 8278 Hz
            // Using xmrs's actual formula for accuracy
            let note = 48.0_f32; // C-4 in xmrs notation
            let period = 6848.0 * (-0.0578 * note).exp() + 0.2782;
            7_093_789.2 / (period * 2.0)
        }
    };

    // Apply finetune adjustment (-1..1, roughly -1 to +1 semitones)
    // We bake finetune into the sample_rate to achieve fine pitch adjustment.
    // The playback engine uses: rate = sample_rate * 2^((target_note - root_note) / 12)
    let finetune_adjustment = 2.0_f32.powf(xmrs_sample.finetune / 12.0);
    let sample_rate = SampleRate::new(base_rate * finetune_adjustment);

    // Root note from relative_pitch
    // relative_pitch = 0 means C-4 (MIDI 60), +12 means C-5, etc.
    let root_midi = (60 + i16::from(xmrs_sample.relative_pitch)).clamp(0, 127) as u8;
    let root_note = MidiNote::new(root_midi);

    // Calculate sample length in frames
    let frame_count = data.len() / channels.channel_count();

    // Extract loop information with exact sample positions (no precision loss)
    let loop_info = if matches!(
        xmrs_sample.loop_flag,
        XmrsLoopType::Forward | XmrsLoopType::PingPong
    ) && xmrs_sample.loop_length > 0
        && frame_count > 0
    {
        // Use exact u32 positions from xmrs to avoid f32 precision loss
        let loop_start = xmrs_sample.loop_start.min(frame_count as u32);
        let loop_end = (xmrs_sample.loop_start + xmrs_sample.loop_length).min(frame_count as u32);
        Some(SampleLoopInfo {
            loop_start,
            loop_end,
            enabled: true,
            ping_pong: matches!(xmrs_sample.loop_flag, XmrsLoopType::PingPong),
        })
    } else {
        None
    };

    // Extract default volume and panning (xmrs uses 0.0-1.0)
    let default_volume = xmrs_sample.volume;
    let default_panning = xmrs_sample.panning; // 0.0 = left, 1.0 = right

    let mut sample = Sample::new(name, data, channels, sample_rate).with_root_note(root_note);

    if let Some(loop_info) = loop_info {
        sample = sample.with_loop_info(loop_info);
    }

    sample = sample
        .with_default_volume(default_volume)
        .with_default_panning(default_panning);

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

/// Convert an xmrs pattern to TrackerPattern format.
///
/// This creates a proper TrackerPattern with explicit rows and cells,
/// following tracker semantics:
/// - Each track maps 1:1 to a voice
/// - NoteOff cells are ONLY created for explicit key-off markers
/// - New notes on same track implicitly cut previous notes (handled by engine)
/// - Instrument 0 means "inherit previous instrument"
#[allow(clippy::cast_possible_truncation)]
fn convert_pattern_to_tracker(
    pattern_data: &XmrsPattern,
    pat_idx: usize,
    num_channels: usize,
    speed: f32,
) -> ImportResult<TrackerPattern> {
    let num_rows = pattern_data.len();
    if num_rows == 0 {
        // Empty pattern - create minimal
        let mut pattern = TrackerPattern::new(
            PatternId::new(pat_idx as u32),
            TrackCount::new(num_channels as u8),
            RowCount::new(1),
        );
        pattern.set_name(format!("Pattern {pat_idx:02X}"));
        return Ok(pattern);
    }

    // Convert tracker speed to song ticks per row
    // Tracker speed 6 at 24 ticks/beat = 240 song ticks/row (at 960 PPQN)
    let ticks_per_row = TicksPerRow::from_tracker_speed(speed as u8);

    let mut pattern = TrackerPattern::new(
        PatternId::new(pat_idx as u32),
        TrackCount::new(num_channels as u8),
        RowCount::new(num_rows as u16),
    );
    pattern.set_name(format!("Pattern {pat_idx:02X}"));
    pattern.set_ticks_per_row(ticks_per_row);

    // Track state per channel (for instrument/volume memory)
    let mut channel_state: Vec<TrackerChannelState> =
        vec![TrackerChannelState::default(); num_channels];

    // Convert each row
    for (row_idx, row_data) in pattern_data.iter().enumerate() {
        let row_index = RowIndex::new(row_idx as u16);

        for (channel_idx, track_unit) in row_data.iter().enumerate() {
            if channel_idx >= num_channels {
                break;
            }

            let track_index = TrackIndex::new(channel_idx as u8);
            let state = &mut channel_state[channel_idx];

            // Process the track unit and get the cell
            let (cell, effects) = process_track_unit_to_cell(track_unit, state);

            // Set cell if not empty
            if !cell.is_empty() {
                pattern.set_cell(track_index, row_index, cell);
            }

            // Add effects if any
            if !effects.is_empty() {
                pattern.set_effects(track_index, row_index, effects);
            }
        }
    }

    Ok(pattern)
}

/// Channel state for TrackerPattern conversion.
#[derive(Clone, Default)]
struct TrackerChannelState {
    /// Last instrument used (0 = none yet).
    last_instrument: usize,
    /// Volume (0.0-1.0), defaults to 1.0 (full volume) as per tracker convention.
    last_volume: f32,
    /// Last portamento target (for tone portamento continuation).
    last_porta_target: Option<Pitch>,
    /// Last vibrato settings.
    last_vibrato_speed: u8,
    last_vibrato_depth: u8,
    /// Last tremolo settings.
    last_tremolo_speed: u8,
    last_tremolo_depth: u8,
}

/// Process a track unit and return a Cell and effects for TrackerPattern.
///
/// Key differences from Pattern-based processing:
/// - Returns Cell instead of creating Note
/// - NoteOff only for explicit key-off (not implicit cuts)
/// - Instrument 0 → None (inherit previous)
#[allow(clippy::cast_possible_truncation)]
fn process_track_unit_to_cell(
    unit: &TrackUnit,
    state: &mut TrackerChannelState,
) -> (Cell, Vec<EffectCommand>) {
    // Check for instrument change
    // Note: xmrs uses 0-indexed instruments
    let has_new_instrument = unit.instrument.is_some();
    if let Some(inst) = unit.instrument {
        state.last_instrument = inst;
    }

    // Use the velocity from the track unit (0.0-1.0)
    if unit.velocity > 0.0 {
        state.last_volume = unit.velocity;
    }

    // Convert effects
    let mut effects = Vec::new();

    // Process track effects
    for effect in &unit.effects {
        if let Some(cmd) = convert_track_effect_for_tracker(effect, state) {
            effects.push(cmd);
        }
    }

    // Process global effects
    for effect in &unit.global_effects {
        if let Some(cmd) = convert_global_effect(effect) {
            effects.push(cmd);
        }
    }

    // Check for explicit key-off (=== in tracker)
    // CRITICAL: Only create NoteOff for explicit key-off markers!
    if unit.note.is_keyoff() {
        return (Cell::note_off(), effects);
    }

    // Check for no note (effect-only row or empty)
    if unit.note.is_none() {
        return (Cell::Empty, effects);
    }

    // Get the note value as u8 (C0=0, C1=12, etc.)
    let note_value = unit.note as u8;

    // xmrs Pitch values match MIDI directly (C0=0, C1=12, etc.)
    // Shift up by 12 for tracker convention (C0 = MIDI 12)
    let midi_note = note_value.saturating_add(12).min(127);

    let Some(pitch) = Pitch::new(midi_note) else {
        // Invalid pitch - treat as empty
        return (Cell::Empty, effects);
    };

    // Update portamento target for tone portamento
    state.last_porta_target = Some(pitch);

    // Determine instrument
    // - If unit has explicit instrument, use it
    // - If no instrument specified, use None (inherit from previous)
    let instrument = if has_new_instrument {
        Some(SeqInstrumentId(state.last_instrument as u16))
    } else {
        // No instrument specified on this row - inherit
        // The engine will use the last instrument for this voice
        None
    };

    // Use the channel's current volume
    let velocity = if state.last_volume < 1.0 {
        Some(Velocity::new(state.last_volume))
    } else {
        None // Default velocity
    };

    (Cell::note_full(pitch, instrument, velocity), effects)
}

/// Convert an xmrs track effect for TrackerPattern.
/// Similar to convert_track_effect but uses TrackerChannelState.
#[allow(clippy::cast_possible_truncation)]
fn convert_track_effect_for_tracker(
    effect: &TrackEffect,
    state: &mut TrackerChannelState,
) -> Option<EffectCommand> {
    match effect {
        // === Pitch effects ===
        TrackEffect::Arpeggio { half1, half2 } => Some(EffectCommand::Arpeggio {
            x: *half1 as u8,
            y: *half2 as u8,
        }),

        TrackEffect::Portamento(speed) => {
            let scaled = (speed.abs() * 16.0).min(255.0) as u8;
            if *speed > 0.0 {
                Some(EffectCommand::PortamentoUp(scaled))
            } else if *speed < 0.0 {
                Some(EffectCommand::PortamentoDown(scaled))
            } else {
                None
            }
        }

        TrackEffect::TonePortamento(speed) => {
            let scaled = (speed * 16.0).min(255.0) as u8;
            Some(EffectCommand::TonePortamento {
                speed: scaled,
                target: state.last_porta_target,
            })
        }

        TrackEffect::Vibrato { speed, depth } => {
            // xmrs normalizes speed to 0-1 using 64 as denominator, depth uses 16
            // xmrs applies parameter memory, so continuation effects have resolved values
            if *speed > 0.0 {
                state.last_vibrato_speed = (speed * 64.0).min(15.0) as u8;
            }
            if *depth > 0.0 {
                state.last_vibrato_depth = (depth * 16.0).min(15.0) as u8;
            }
            Some(EffectCommand::Vibrato {
                speed: state.last_vibrato_speed,
                depth: state.last_vibrato_depth,
            })
        }

        TrackEffect::VibratoSpeed(speed) => {
            // xmrs normalizes speed to 0-1 using 64 as denominator
            state.last_vibrato_speed = (speed * 64.0).min(15.0) as u8;
            Some(EffectCommand::Vibrato {
                speed: state.last_vibrato_speed,
                depth: state.last_vibrato_depth,
            })
        }

        TrackEffect::VibratoDepth(depth) => {
            state.last_vibrato_depth = (depth * 16.0).min(15.0) as u8;
            Some(EffectCommand::Vibrato {
                speed: state.last_vibrato_speed,
                depth: state.last_vibrato_depth,
            })
        }

        TrackEffect::VibratoWaveform { waveform, .. } => {
            Some(EffectCommand::VibratoWaveform(convert_waveform(waveform)))
        }

        TrackEffect::Glissando(enabled) => Some(EffectCommand::Glissando(*enabled)),

        TrackEffect::InstrumentFineTune(tune) => {
            let cents = (tune * 100.0).clamp(-128.0, 127.0) as i8;
            Some(EffectCommand::FineTune(cents))
        }

        // === Volume effects ===
        TrackEffect::Volume { value, .. } => {
            let vol = (value * 64.0).min(64.0) as u8;
            Some(EffectCommand::SetVolume(vol))
        }

        TrackEffect::ChannelVolume(value) => {
            let vol = (value * 64.0).min(64.0) as u8;
            Some(EffectCommand::SetVolume(vol))
        }

        TrackEffect::VolumeSlide { speed, fine } => {
            let scaled = (speed.abs() * 16.0).min(15.0) as u8;
            if *fine {
                if *speed > 0.0 {
                    Some(EffectCommand::FineVolumeSlide {
                        up: scaled,
                        down: 0,
                    })
                } else {
                    Some(EffectCommand::FineVolumeSlide {
                        up: 0,
                        down: scaled,
                    })
                }
            } else if *speed > 0.0 {
                Some(EffectCommand::VolumeSlide {
                    up: scaled,
                    down: 0,
                })
            } else {
                Some(EffectCommand::VolumeSlide {
                    up: 0,
                    down: scaled,
                })
            }
        }

        TrackEffect::ChannelVolumeSlide { speed, fine } => {
            let scaled = (speed.abs() * 16.0).min(15.0) as u8;
            if *fine {
                if *speed > 0.0 {
                    Some(EffectCommand::FineVolumeSlide {
                        up: scaled,
                        down: 0,
                    })
                } else {
                    Some(EffectCommand::FineVolumeSlide {
                        up: 0,
                        down: scaled,
                    })
                }
            } else if *speed > 0.0 {
                Some(EffectCommand::VolumeSlide {
                    up: scaled,
                    down: 0,
                })
            } else {
                Some(EffectCommand::VolumeSlide {
                    up: 0,
                    down: scaled,
                })
            }
        }

        TrackEffect::Tremolo { speed, depth } => {
            // xmrs normalizes speed to 0-1 using 64 as denominator, depth uses 16
            // xmrs applies parameter memory, so continuation effects have resolved values
            if *speed > 0.0 {
                state.last_tremolo_speed = (speed * 64.0).min(15.0) as u8;
            }
            if *depth > 0.0 {
                state.last_tremolo_depth = (depth * 16.0).min(15.0) as u8;
            }
            Some(EffectCommand::Tremolo {
                speed: state.last_tremolo_speed,
                depth: state.last_tremolo_depth,
            })
        }

        TrackEffect::TremoloWaveform { waveform, .. } => {
            Some(EffectCommand::TremoloWaveform(convert_waveform(waveform)))
        }

        // === Panning effects ===
        TrackEffect::Panning(pan) => {
            let p = (pan * 255.0).min(255.0) as u8;
            Some(EffectCommand::SetPanning(p))
        }

        TrackEffect::PanningSlide { speed, .. } => {
            let scaled = (speed.abs() * 16.0).min(15.0) as u8;
            if *speed < 0.0 {
                Some(EffectCommand::PanningSlide {
                    left: scaled,
                    right: 0,
                })
            } else {
                Some(EffectCommand::PanningSlide {
                    left: 0,
                    right: scaled,
                })
            }
        }

        // === Sample/playback effects ===
        TrackEffect::InstrumentSampleOffset(offset) => {
            // xmrs gives us offset * 256 (absolute sample position).
            // We need the raw 0-255 parameter value where 255 = 100% of sample.
            // TrackerSampleOffset::as_normalized() divides by 256 to get 0.0-1.0.
            #[allow(clippy::cast_possible_truncation)]
            let raw_param = ((*offset) / 256).min(255) as u16;
            Some(EffectCommand::SampleOffset(raw_param))
        }

        TrackEffect::NoteRetrig {
            speed,
            volume_modifier,
        } => {
            let vol_change = match volume_modifier {
                xmrs::effect::NoteRetrigOperator::None => 0i8,
                xmrs::effect::NoteRetrigOperator::Sum(v) => (v * 16.0).clamp(-128.0, 127.0) as i8,
                xmrs::effect::NoteRetrigOperator::Mul(v) => {
                    ((v - 1.0) * 16.0).clamp(-128.0, 127.0) as i8
                }
            };
            Some(EffectCommand::Retrigger {
                interval: *speed as u8,
                volume_change: vol_change,
            })
        }

        TrackEffect::NoteCut { tick, .. } => Some(EffectCommand::NoteCut(*tick as u8)),

        TrackEffect::NoteDelay(ticks) => Some(EffectCommand::NoteDelay(*ticks as u8)),

        TrackEffect::NoteFadeOut { tick, .. } => Some(EffectCommand::NoteFadeOut(*tick as u8)),

        // === Envelope effects (not directly supported yet) ===
        TrackEffect::InstrumentVolumeEnvelope(_)
        | TrackEffect::InstrumentVolumeEnvelopePosition(_)
        | TrackEffect::InstrumentPanningEnvelope(_)
        | TrackEffect::InstrumentPanningEnvelopePosition(_)
        | TrackEffect::InstrumentPitchEnvelope(_)
        | TrackEffect::InstrumentNewNoteAction(_)
        | TrackEffect::InstrumentSurround(_) => None,

        // === Unsupported effects ===
        TrackEffect::Panbrello { .. }
        | TrackEffect::PanbrelloWaveform { .. }
        | TrackEffect::NoteOff { .. }
        | TrackEffect::Tremor { .. } => None,
    }
}

/// Convert an xmrs global effect to our EffectCommand.
#[allow(clippy::cast_possible_truncation)]
fn convert_global_effect(effect: &GlobalEffect) -> Option<EffectCommand> {
    match effect {
        GlobalEffect::Bpm(bpm) => Some(EffectCommand::SetTempo(*bpm as u16)),

        GlobalEffect::Speed(speed) => Some(EffectCommand::SetSpeed(*speed as u8)),

        GlobalEffect::PatternBreak(row) => Some(EffectCommand::PatternBreak(*row as u8)),

        GlobalEffect::PositionJump(pos) => Some(EffectCommand::PatternJump(*pos as u8)),

        GlobalEffect::PatternLoop(count) => Some(EffectCommand::PatternLoop {
            count: *count as u8,
        }),

        GlobalEffect::PatternDelay { quantity, .. } => {
            Some(EffectCommand::PatternDelay(*quantity as u8))
        }

        GlobalEffect::Volume(vol) => {
            // Global volume - convert to SetVolume for now
            let v = (vol * 64.0).min(64.0) as u8;
            Some(EffectCommand::SetVolume(v))
        }

        GlobalEffect::VolumeSlide { speed, fine } => {
            let scaled = (speed.abs() * 16.0).min(15.0) as u8;
            if *fine {
                if *speed > 0.0 {
                    Some(EffectCommand::FineVolumeSlide {
                        up: scaled,
                        down: 0,
                    })
                } else {
                    Some(EffectCommand::FineVolumeSlide {
                        up: 0,
                        down: scaled,
                    })
                }
            } else if *speed > 0.0 {
                Some(EffectCommand::VolumeSlide {
                    up: scaled,
                    down: 0,
                })
            } else {
                Some(EffectCommand::VolumeSlide {
                    up: 0,
                    down: scaled,
                })
            }
        }

        // === Unsupported ===
        GlobalEffect::BpmSlide(_) | GlobalEffect::MidiMacro(_) => None,
    }
}

/// Convert xmrs waveform to our EffectWaveform.
fn convert_waveform(
    waveform: &xmrs::waveform::Waveform,
) -> synth_sequencer::effects::EffectWaveform {
    use synth_sequencer::effects::EffectWaveform;
    use xmrs::waveform::Waveform;
    match waveform {
        Waveform::Sine | Waveform::TranslatedSine => EffectWaveform::Sine,
        Waveform::RampDown | Waveform::TranslatedRampDown | Waveform::TranslatedRampUp => {
            EffectWaveform::Ramp
        }
        Waveform::Square | Waveform::TranslatedSquare => EffectWaveform::Square,
        Waveform::Random => EffectWaveform::Random,
    }
}
