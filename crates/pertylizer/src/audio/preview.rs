//! Offline audio preview rendering for MCP.
//!
//! Renders a single note on an instrument to an in-memory WAV buffer,
//! used by the `preview_note` MCP tool to return audio content.

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

/// Render a note preview for the given instrument.
///
/// Snapshots the instrument's current module graph and parameters from the
/// live engine state, creates a fresh offline engine, loads the patch,
/// plays the note, and returns WAV data.
pub fn render_note_preview(
    session: &SynthSession,
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration_ms: u32,
    tail_ms: u32,
) -> Result<AudioPreview, McpBridgeError> {
    // Validate instrument exists
    if !session.instrument_exists(instrument_id) {
        return Err(McpBridgeError::InstrumentNotFound(instrument_id.as_u64()));
    }

    // Snapshot modules, connections, and parameters from the live engine
    let engine_state = session.state();
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
    let _ = tmp_session.add_instrument_with_id(InstrumentId::FIRST, "Preview");
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
    let mut buffer = vec![0.0f32; BUFFER_SIZE * channels];
    let init_context = AudioCallbackContext {
        sample_rate: hw_sample_rate,
        frames: BUFFER_SIZE,
        channels: channels as u16,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: 0.0,
    };
    engine.process(&mut buffer, &init_context);

    // Calculate frame counts
    let note_frames = (f64::from(duration_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
    let tail_frames = (f64::from(tail_ms) / 1000.0 * f64::from(PREVIEW_SAMPLE_RATE)) as u64;
    let total_frames = note_frames + tail_frames;
    let total_seconds = total_frames as f32 / PREVIEW_SAMPLE_RATE as f32;

    // Create in-memory WAV writer (16-bit for smaller size)
    let spec = hound::WavSpec {
        channels: channels as u16,
        sample_rate: PREVIEW_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav_cursor = std::io::Cursor::new(Vec::with_capacity(
        total_frames as usize * channels * 2 + 44,
    ));
    let mut writer = hound::WavWriter::new(&mut wav_cursor, spec)
        .map_err(|e| McpBridgeError::Other(format!("WAV writer error: {e}")))?;

    // Send note on
    handle.send_blocking(EngineCommand::NoteOn {
        note,
        velocity,
        channel: MidiChannel::CH1,
    });

    let mut frames_written: u64 = 0;
    let mut note_off_sent = false;

    while frames_written < total_frames {
        let remaining = (total_frames - frames_written) as usize;
        let this_buffer = remaining.min(BUFFER_SIZE);
        let sample_count = this_buffer * channels;

        buffer.fill(0.0);

        let context = AudioCallbackContext {
            sample_rate: hw_sample_rate,
            frames: this_buffer,
            channels: channels as u16,
            stream_time: frames_written as f64 / f64::from(PREVIEW_SAMPLE_RATE),
            sample_position: frames_written,
            output_latency: 0.0,
        };

        engine.process(&mut buffer[..sample_count], &context);

        // Send note off after the specified duration
        if !note_off_sent && frames_written >= note_frames {
            handle.send_blocking(EngineCommand::NoteOff {
                note,
                channel: MidiChannel::CH1,
            });
            note_off_sent = true;
        }

        // Write 16-bit samples
        for &sample in &buffer[..sample_count] {
            let clamped = sample.clamp(-1.0, 1.0);
            #[allow(clippy::cast_possible_truncation)]
            let int_val = (clamped * f32::from(i16::MAX)) as i16;
            writer
                .write_sample(int_val)
                .map_err(|e| McpBridgeError::Other(format!("WAV write error: {e}")))?;
        }

        frames_written += this_buffer as u64;
    }

    // Finalize WAV
    writer
        .finalize()
        .map_err(|e| McpBridgeError::Other(format!("WAV finalize error: {e}")))?;
    let wav_data = wav_cursor.into_inner();

    Ok(AudioPreview {
        wav_data,
        sample_rate: PREVIEW_SAMPLE_RATE,
        duration_seconds: total_seconds,
    })
}
