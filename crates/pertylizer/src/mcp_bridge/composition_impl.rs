use super::*;

pub fn generate_chord_impl(
    symbol: &str,
    octave: i32,
    voicing: Option<&str>,
) -> Result<synth_mcp::types::GenerateChordResult, McpBridgeError> {
    use crate::composition::{ChordVoicing, generate_chord};
    use synth_mcp::types::GenerateChordResult;

    let v = match voicing {
        None => ChordVoicing::Close,
        Some(s) => s
            .parse::<ChordVoicing>()
            .map_err(|error| McpBridgeError::Other(error.to_string()))?,
    };
    let generated =
        generate_chord(symbol, octave, v).map_err(|e| McpBridgeError::Other(e.to_string()))?;
    Ok(GenerateChordResult {
        symbol: symbol.to_string(),
        root_pitch_class: generated.root_pitch_class,
        quality: generated.quality.to_string(),
        suffix: generated.suffix.to_string(),
        voicing: generated.voicing.as_str().to_string(),
        notes: generated.notes,
        warnings: generated.warnings,
    })
}

pub fn transpose_notes_impl(
    shared: &McpSharedState,
    pattern_id: PatternId,
    semitones: Semitones,
    scale_tonic: Option<u8>,
    scale_name: Option<&str>,
    tie_break: Option<&str>,
) -> Result<synth_mcp::types::TransposeNotesResult, McpBridgeError> {
    use crate::composition::{ScaleConstraint, transpose_pitches};
    use synth_mcp::types::TransposeNotesResult;

    let tie_break = parse_tie_break(tie_break)?;
    let scale = match (scale_tonic, scale_name) {
        (Some(t), Some(n)) => Some(ScaleConstraint::new(t, n)),
        _ => None,
    };
    let mut warnings = Vec::new();
    if scale_tonic.is_some() ^ scale_name.is_some() {
        warnings.push(
            "scale_tonic and scale_name must both be set to enable scale snapping; ignoring the partial constraint".to_string(),
        );
    }

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(pattern_id)
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    // Snapshot pitches, transpose in place, write back via `note_mut` so note
    // IDs / durations / instruments survive.
    let (ids, mut pitches): (Vec<_>, Vec<u8>) = pattern
        .notes()
        .iter()
        .map(|n| (n.id, n.pitch.as_midi()))
        .unzip();
    #[allow(clippy::cast_possible_truncation)]
    let semitone_steps = semitones.as_f32().round() as i32;
    let result = transpose_pitches(&mut pitches, semitone_steps, scale.as_ref(), tie_break);
    write_back_pitches(pattern, &ids, &pitches);

    Ok(TransposeNotesResult {
        pattern_id,
        semitones,
        notes_in: result.notes_in,
        notes_transposed: result.notes_transposed,
        notes_out_of_range: result.notes_out_of_range,
        notes_snapped_to_scale: result.notes_snapped_to_scale,
        scale_tonic_pitch_class: scale.as_ref().map(|s| s.tonic),
        scale_name: scale.as_ref().map(|s| s.scale_name.to_string()),
        warnings,
    })
}

pub fn quantize_notes_to_scale_impl(
    shared: &McpSharedState,
    pattern_id: PatternId,
    scale_tonic: u8,
    scale_name: &str,
    tie_break: Option<&str>,
) -> Result<synth_mcp::types::QuantizeNotesToScaleResult, McpBridgeError> {
    use crate::composition::{ScaleConstraint, ScaleQuantizeOptions, quantize_pitches_to_scale};
    use synth_mcp::types::QuantizeNotesToScaleResult;

    let tie_break = parse_tie_break(tie_break)?;
    let scale = ScaleConstraint::new(scale_tonic, scale_name);
    let scale_label = scale.scale_name.to_string();
    let scale_pc = scale.tonic;

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(pattern_id)
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    let (ids, mut pitches): (Vec<_>, Vec<u8>) = pattern
        .notes()
        .iter()
        .map(|n| (n.id, n.pitch.as_midi()))
        .unzip();
    let result = quantize_pitches_to_scale(
        &mut pitches,
        &ScaleQuantizeOptions {
            scale: &scale,
            tie_break,
        },
    );
    write_back_pitches(pattern, &ids, &pitches);

    let mean_correction = if result.notes_moved > 0 {
        result.total_correction_semitones as f32 / result.notes_moved as f32
    } else {
        0.0
    };

    Ok(QuantizeNotesToScaleResult {
        pattern_id,
        scale_tonic_pitch_class: scale_pc,
        scale_name: scale_label,
        notes_in: result.notes_in,
        notes_already_in_scale: result.notes_already_in_scale,
        notes_moved: result.notes_moved,
        mean_correction_semitones: mean_correction,
        max_correction_semitones: result.max_correction_semitones,
        warnings: Vec::new(),
    })
}

pub fn quantize_notes_to_grid_impl(
    shared: &McpSharedState,
    pattern_id: PatternId,
    grid_ticks: u32,
    strength: Option<f32>,
    swing: Option<f32>,
    humanize_ticks: Option<u32>,
    humanize_seed: Option<u64>,
) -> Result<synth_mcp::types::QuantizeNotesToGridResult, McpBridgeError> {
    use crate::composition::{GridQuantizeOptions, NoteTiming, quantize_grid};
    use synth_mcp::types::QuantizeNotesToGridResult;

    let strength_val = strength.unwrap_or(1.0).clamp(0.0, 1.0);
    let swing_val = swing.unwrap_or(0.0).clamp(0.0, 1.0);
    let humanize_val = humanize_ticks.unwrap_or(0);
    let seed_val = humanize_seed.unwrap_or(0);

    let mut song = shared.song.write();
    let pattern = song
        .pattern_mut(pattern_id)
        .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

    let length_ticks = pattern.length.0;
    let (ids, mut timings): (Vec<_>, Vec<NoteTiming>) = pattern
        .notes()
        .iter()
        .map(|n| {
            (
                n.id,
                NoteTiming {
                    start_tick: n.start.0,
                },
            )
        })
        .unzip();
    let result = quantize_grid(
        &mut timings,
        &GridQuantizeOptions {
            grid_ticks,
            pattern_length_ticks: length_ticks,
            strength: strength_val,
            swing: swing_val,
            humanize_ticks: humanize_val,
            seed: seed_val,
        },
    );

    // Goes through `move_note` so the pattern's start-tick sort invariant is
    // preserved — writing `note_mut().start = ...` would silently break it.
    for (id, timing) in ids.iter().zip(timings.iter()) {
        pattern.move_note(*id, synth_sequencer::PatternTick(timing.start_tick));
    }

    let mean_delta = if result.notes_moved > 0 {
        result.total_delta_ticks as f32 / result.notes_moved as f32
    } else {
        0.0
    };

    let mut warnings = Vec::new();
    if result.disabled {
        warnings.push("grid_ticks was 0; no changes applied".to_string());
    }

    Ok(QuantizeNotesToGridResult {
        pattern_id,
        grid_ticks,
        strength: strength_val,
        swing: swing_val,
        humanize_ticks: humanize_val,
        humanize_seed: seed_val,
        notes_in: result.notes_in,
        notes_moved: result.notes_moved,
        mean_delta_ticks: mean_delta,
        max_delta_ticks: result.max_delta_ticks,
        pattern_length_ticks: length_ticks,
        warnings,
    })
}

fn write_back_pitches(
    pattern: &mut synth_sequencer::Pattern,
    ids: &[synth_sequencer::NoteId],
    pitches: &[u8],
) {
    for (id, new_pitch) in ids.iter().zip(pitches.iter()) {
        if let Some(note) = pattern.note_mut(*id)
            && let Some(p) = synth_sequencer::Pitch::new(*new_pitch)
        {
            note.pitch = p;
        }
    }
}

fn parse_tie_break(s: Option<&str>) -> Result<crate::composition::ScaleTieBreak, McpBridgeError> {
    use crate::composition::ScaleTieBreak;
    match s {
        None => Ok(ScaleTieBreak::NearestUp),
        Some(raw) => raw
            .parse::<ScaleTieBreak>()
            .map_err(|error| McpBridgeError::Other(error.to_string())),
    }
}
