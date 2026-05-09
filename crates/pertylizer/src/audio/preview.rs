//! Offline audio preview rendering for MCP.
//!
//! Renders a single note on an instrument to an in-memory f32 buffer or WAV.
//! The render path is split from WAV encoding so it can be reused by the
//! `analyze_note` MCP tool, which runs offline analysis on the f32 buffer
//! directly without needing WAV bytes.

use std::sync::Arc;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, MidiNote, Velocity};
use synth_engine::commands::{EffectType, PortId};
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::{EngineCommand, SynthEngine};

use synth_mcp::error::McpBridgeError;
use synth_mcp::types::AudioPreview;

use crate::session::SynthSession;

/// Sample rate for preview rendering (44.1 kHz — sufficient for previews).
const PREVIEW_SAMPLE_RATE: u32 = 44100;

/// Buffer size in frames per render call.
const BUFFER_SIZE: usize = 256;

/// Output of [`render_note_to_buffer`] — the raw f32 audio plus metadata.
pub struct RenderedNote {
    /// Stereo-interleaved f32 samples (L0, R0, L1, R1, ...).
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Total duration including tail, in seconds.
    pub duration_seconds: f32,
    /// Channel count (always 2).
    pub channels: u16,
    /// Effective MIDI note that was actually played, after the patch's
    /// `octave_offset` was applied. The caller may want to know this for
    /// reporting, e.g. the `analyze_note` tool returns it as `note_played`.
    pub effective_note: MidiNote,
}

/// Render a note on the given instrument into an in-memory f32 buffer.
///
/// Snapshots the instrument's current module graph and parameters from the
/// live engine state, creates a fresh offline engine, loads the patch, plays
/// the note, and returns the rendered stereo-interleaved f32 samples (no WAV
/// encoding).
pub fn render_note_to_buffer(
    session: &SynthSession,
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
) -> Result<RenderedNote, McpBridgeError> {
    // Validate instrument exists
    if !session.instrument_exists(instrument_id) {
        return Err(McpBridgeError::InstrumentNotFound(instrument_id.as_u64()));
    }

    // Snapshot modules, connections, and parameters from the live engine
    let engine_state = session.state();

    // Mirror the on-screen keyboard's octave shift (applied when the patch
    // was loaded). Without this, a bass patch with `octave_offset = -2`
    // would preview two octaves higher than what the user hears live.
    let octave_offset = engine_state.get_octave_offset(instrument_id);
    let effective_note = if octave_offset == 0 {
        note
    } else {
        let shifted = i32::from(note.as_u8()) + octave_offset * 12;
        MidiNote::new(shifted.clamp(0, 127) as u8)
    };

    let modules = engine_state
        .shared_graph
        .get_modules_for_instrument(instrument_id);
    let connections = engine_state
        .shared_graph
        .get_connections_for_instrument(instrument_id);

    if modules.is_empty() {
        return Err(McpBridgeError::Other(
            "Instrument has no modules — nothing to render".to_string(),
        ));
    }

    // Create a fresh offline engine
    let (mut engine, mut handle) = SynthEngine::new();

    // Create a temporary session and instrument for loading modules
    let tmp_session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    tmp_session
        .add_instrument_with_id(InstrumentId::FIRST, "Preview")
        .map_err(|e| McpBridgeError::Other(format!("Failed to create preview instrument: {e}")))?;
    tmp_session.reset_counters_for_instrument(InstrumentId::FIRST);

    // Load modules into the offline engine
    for module_snap in &modules {
        // Skip visualizer modules (they need GUI)
        if module_snap.module_type.is_visualizer() {
            continue;
        }

        let module_id = module_snap.id;
        let descriptor = match tmp_session.add_module_with_id(
            InstrumentId::FIRST,
            module_id,
            module_id.module_type,
        ) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Preview: failed to add module {module_id}: {e}");
                continue;
            }
        };

        // Apply parameters from the live engine snapshot
        let effect_type = EffectType::from_module_type(module_id.module_type);
        for desc_param in &descriptor.parameters {
            // Find matching parameter in the snapshot by kind
            if let Some(ep) = module_snap
                .parameters
                .iter()
                .find(|p| p.same_kind(&desc_param.id))
            {
                let param = desc_param.id.with_f32(ep.as_f32());
                if let Some(et) = effect_type {
                    handle.send_blocking(EngineCommand::SetEffectParameter {
                        instrument_id: Some(InstrumentId::FIRST),
                        effect_type: et,
                        param,
                    });
                } else {
                    handle.send_blocking(EngineCommand::SetModuleParameter {
                        instrument_id: Some(InstrumentId::FIRST),
                        module_id,
                        param,
                    });
                }
            }
        }
    }

    // Load connections
    for conn in &connections {
        // Skip connections involving visualizer modules
        if conn.from_module.module_type.is_visualizer()
            || conn.to_module.module_type.is_visualizer()
        {
            continue;
        }
        handle.send_blocking(EngineCommand::Connect {
            instrument_id: Some(InstrumentId::FIRST),
            from: PortId::new(conn.from_module, conn.from_port),
            to: PortId::new(conn.to_module, conn.to_port),
        });
    }

    // Enable the instrument
    handle.send_blocking(EngineCommand::SetInstrumentEnabled {
        instrument_id: InstrumentId::FIRST,
        enabled: true,
    });
    handle.send_blocking(EngineCommand::SetInstrumentMidiChannel {
        instrument_id: InstrumentId::FIRST,
        channel: MidiChannel::CH1,
    });

    // Set up stream
    let hw_sample_rate = HwSampleRate(PREVIEW_SAMPLE_RATE);
    let stream_info = synth_core::StreamInfo {
        sample_rate: hw_sample_rate,
        buffer_size: synth_core::BufferSize(BUFFER_SIZE as u32),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    // Warm up: process one buffer to let engine initialize
    let channels: usize = 2;
    let mut block = vec![0.0f32; BUFFER_SIZE * channels];
    // Use a sentinel position for the warmup block so the engine does not see
    // a duplicate sample_position 0 when real rendering starts.
    let init_context = AudioCallbackContext {
        sample_rate: hw_sample_rate,
        frames: BUFFER_SIZE,
        channels: channels as u16,
        stream_time: 0.0,
        sample_position: u64::MAX,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &init_context);

    // Calculate frame counts
    let note_frames = (f64::from(duration_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
    let tail_frames = (f64::from(tail_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
    let total_frames = note_frames + tail_frames;

    if total_frames == 0 {
        return Err(McpBridgeError::Other(
            "Total render duration is 0 — duration_ms and tail_ms must not both be 0".to_string(),
        ));
    }
    let total_seconds = total_frames as f32 / PREVIEW_SAMPLE_RATE as f32;

    // Pre-allocate the output buffer for the entire render.
    let mut samples: Vec<f32> = Vec::with_capacity(total_frames as usize * channels);

    // Send note on
    handle.send_blocking(EngineCommand::NoteOn {
        note: effective_note,
        velocity,
        channel: MidiChannel::CH1,
    });

    let mut frames_written: u64 = 0;
    let mut note_off_sent = false;

    while frames_written < total_frames {
        let remaining = (total_frames - frames_written) as usize;
        let this_buffer = remaining.min(BUFFER_SIZE);
        let sample_count = this_buffer * channels;

        block.fill(0.0);

        let context = AudioCallbackContext {
            sample_rate: hw_sample_rate,
            frames: this_buffer,
            channels: channels as u16,
            stream_time: frames_written as f64 / f64::from(PREVIEW_SAMPLE_RATE),
            sample_position: frames_written,
            output_latency: synth_core::Seconds::ZERO,
        };

        engine.process(&mut block[..sample_count], &context);

        // Send note off after the specified duration
        if !note_off_sent && frames_written >= note_frames {
            handle.send_blocking(EngineCommand::NoteOff {
                note: effective_note,
                channel: MidiChannel::CH1,
            });
            note_off_sent = true;
        }

        // Append this block's samples to the output buffer.
        samples.extend_from_slice(&block[..sample_count]);

        frames_written += this_buffer as u64;
    }

    Ok(RenderedNote {
        samples,
        sample_rate: PREVIEW_SAMPLE_RATE,
        duration_seconds: total_seconds,
        channels: channels as u16,
        effective_note,
    })
}

/// Encode a stereo-interleaved (or mono) f32 buffer as a 16-bit signed WAV.
///
/// Samples outside ±1.0 are clamped before conversion. The resulting bytes
/// are a complete in-memory WAV file (header + PCM data).
pub fn encode_buffer_as_wav(
    buffer: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<Vec<u8>, McpBridgeError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav_cursor = std::io::Cursor::new(Vec::with_capacity(buffer.len() * 2 + 44));
    let mut writer = hound::WavWriter::new(&mut wav_cursor, spec)
        .map_err(|e| McpBridgeError::Other(format!("WAV writer error: {e}")))?;

    for &sample in buffer {
        let clamped = sample.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let int_val = (clamped * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(int_val)
            .map_err(|e| McpBridgeError::Other(format!("WAV write error: {e}")))?;
    }

    writer
        .finalize()
        .map_err(|e| McpBridgeError::Other(format!("WAV finalize error: {e}")))?;
    Ok(wav_cursor.into_inner())
}

/// Render a note preview for the given instrument and return WAV bytes.
///
/// Thin wrapper that calls [`render_note_to_buffer`] and then encodes the
/// resulting f32 buffer with [`encode_buffer_as_wav`]. Existing callers
/// (e.g. the `preview_note` MCP tool) keep using this function unchanged.
pub fn render_note_preview(
    session: &SynthSession,
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
) -> Result<AudioPreview, McpBridgeError> {
    let rendered =
        render_note_to_buffer(session, instrument_id, note, velocity, duration_ms, tail_ms)?;

    let wav_data =
        encode_buffer_as_wav(&rendered.samples, rendered.sample_rate, rendered.channels)?;

    Ok(AudioPreview {
        wav_data,
        sample_rate: rendered.sample_rate,
        duration_seconds: rendered.duration_seconds,
    })
}
