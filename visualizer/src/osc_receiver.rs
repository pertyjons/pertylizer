//! Background-threaded UDP OSC receiver system for Bevy.
//!
//! A dedicated daemon thread owns the UDP socket in blocking mode, decodes
//! incoming OSC packets, maintains an internal telemetry snapshot, and
//! forwards it through a bounded crossbeam channel. The Bevy system drains
//! the channel each frame and copies the latest state into `SynthTelemetry`,
//! keeping the game loop free of any socket or decoding work.
//!
//! The background thread also sends `/viz/pong` replies so the sender knows
//! a client is connected (enables full telemetry).

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, bounded};
use rosc::{OscMessage, OscPacket, OscType, encoder};

use synth_osc_protocol::addresses;

use crate::telemetry::{EXPECTED_PROTOCOL_VERSION, MAX_FFT_BANDS, NoteOnEvent, SynthTelemetry};

/// How often to send `/viz/pong` back to the sender.
const PONG_INTERVAL_SECS: f32 = synth_osc_protocol::PONG_REPLY_INTERVAL_SECS;

/// Maximum number of packets buffered between the reader thread and the
/// Bevy system. If the channel is full, new snapshots are silently dropped
/// (we only care about the latest data).
const CHANNEL_CAPACITY: usize = 100;

/// Decoded telemetry snapshot sent from the background thread to the main
/// thread via the crossbeam channel. The background thread maintains a
/// persistent copy, updates it as OSC messages arrive, and sends clones.
#[derive(Clone)]
struct OscSnapshot {
    // -- Protocol metadata --
    protocol_version: i32,
    sample_rate: f32,
    update_rate_hz: f32,

    // -- Sequence --
    seq: i32,

    // -- Audio levels --
    rms: [f32; 2],
    peak: [f32; 2],

    // -- FFT spectrum --
    fft: [f32; MAX_FFT_BANDS],
    centroid_hz: f32,
    flux: f32,
    fft_freqs: [f32; MAX_FFT_BANDS],
    fft_bin_count: usize,

    // -- Note events --
    last_note_on: Option<NoteOnEvent>,
    /// Note-on events accumulated since the last successful channel send.
    note_events: Vec<NoteOnEvent>,

    // -- Transport --
    playing: bool,
    tempo: f32,
    beat_position: f32,
    beat_phase: f32,

    // -- MIDI CC --
    last_cc: Option<(u8, f32, u8)>,
    pitch_bend: f32,
    aftertouch: f32,

    // -- Engine --
    voice_count: u32,
    cpu: f32,
    event_drops: u32,

    // -- One-shot requests (cleared after successful send) --
    camera_mode_request: Option<String>,
}

impl Default for OscSnapshot {
    fn default() -> Self {
        Self {
            protocol_version: 0,
            sample_rate: 0.0,
            update_rate_hz: 0.0,
            seq: 0,
            rms: [0.0; 2],
            peak: [0.0; 2],
            fft: [0.0; MAX_FFT_BANDS],
            centroid_hz: 0.0,
            flux: 0.0,
            fft_freqs: [0.0; MAX_FFT_BANDS],
            fft_bin_count: synth_osc_protocol::NUM_FFT_BANDS,
            last_note_on: None,
            note_events: Vec::new(),
            playing: false,
            tempo: 120.0,
            beat_position: 0.0,
            beat_phase: 0.0,
            last_cc: None,
            pitch_bend: 0.0,
            aftertouch: 0.0,
            voice_count: 0,
            cpu: 0.0,
            event_drops: 0,
            camera_mode_request: None,
        }
    }
}

/// Resource holding the channel receiver. The background thread owns the
/// socket; the main thread never touches it.
#[derive(Resource)]
pub struct OscSocket {
    /// Receives decoded `OscSnapshot` from the background thread.
    rx: Receiver<OscSnapshot>,
}

/// Plugin that sets up the OSC receiver.
pub struct OscReceiverPlugin;

impl Plugin for OscReceiverPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_osc_socket)
            .add_systems(PreUpdate, receive_osc);
    }
}

fn setup_osc_socket(mut commands: Commands) {
    let bind_addr = format!("0.0.0.0:{}", synth_osc_protocol::DEFAULT_OSC_PORT);
    let socket = UdpSocket::bind(&bind_addr).unwrap_or_else(|e| {
        panic!(
            "Failed to bind OSC port {}: {e}",
            synth_osc_protocol::DEFAULT_OSC_PORT
        )
    });

    let (tx, rx) = bounded(CHANNEL_CAPACITY);

    // Spawn a daemon thread that owns the socket, decodes OSC, and
    // forwards telemetry snapshots through the channel.
    std::thread::Builder::new()
        .name("osc-reader".into())
        .spawn(move || {
            socket
                .set_nonblocking(false)
                .expect("Failed to set socket to blocking mode");

            let mut buf = vec![0u8; 65_536];
            let mut state = OscSnapshot::default();
            let mut version_warned = false;
            let mut last_pong_sent = Instant::now();

            loop {
                match socket.recv_from(&mut buf) {
                    Ok((size, addr)) => {
                        if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) {
                            handle_packet(&packet, &mut state, &mut version_warned);

                            // Send pong reply periodically
                            let now = Instant::now();
                            if now.duration_since(last_pong_sent).as_secs_f32()
                                >= PONG_INTERVAL_SECS
                            {
                                send_pong(&socket, addr);
                                last_pong_sent = now;
                            }

                            // Forward snapshot to main thread
                            match tx.try_send(state.clone()) {
                                Ok(()) => {
                                    // Clear one-shot data after successful send
                                    state.note_events.clear();
                                    state.camera_mode_request = None;
                                }
                                Err(_) => {
                                    // Channel full — note events accumulate for next send
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // If the channel is disconnected the main app has
                        // shut down; exit the thread quietly.
                        if tx.is_empty() && e.kind() == std::io::ErrorKind::Other {
                            break;
                        }
                    }
                }
            }
        })
        .expect("Failed to spawn OSC reader thread");

    println!("OSC receiver: listening on {bind_addr}");

    commands.insert_resource(OscSocket { rx });
}

fn receive_osc(
    time: Res<Time>,
    socket: Res<OscSocket>,
    mut telemetry: ResMut<SynthTelemetry>,
    mut camera_state: ResMut<crate::visuals::camera::CameraState>,
) {
    // Clear pending events from previous frame before processing new snapshots
    telemetry.pending_note_events.clear();

    let mut received_any = false;
    let mut latest: Option<OscSnapshot> = None;

    // Drain all snapshots forwarded by the background thread
    while let Ok(snapshot) = socket.rx.try_recv() {
        // Collect note events from every snapshot (not just the latest)
        telemetry.pending_note_events.extend(&snapshot.note_events);

        // Handle camera mode request from any snapshot
        if let Some(ref mode_name) = snapshot.camera_mode_request {
            if let Some(mode) = crate::visuals::camera::CameraMode::from_name(mode_name) {
                camera_state.osc_requested_mode = Some(mode);
            } else {
                warn!("Unknown camera mode via OSC: {mode_name}");
            }
        }

        latest = Some(snapshot);
        received_any = true;
    }

    // Apply the latest snapshot's field values to the telemetry resource
    if let Some(snap) = latest {
        telemetry.protocol_version = snap.protocol_version;
        telemetry.sample_rate = snap.sample_rate;
        telemetry.update_rate_hz = snap.update_rate_hz;
        telemetry.seq = snap.seq;
        telemetry.rms = snap.rms;
        telemetry.peak = snap.peak;
        telemetry.fft = snap.fft;
        telemetry.centroid_hz = snap.centroid_hz;
        telemetry.flux = snap.flux;
        telemetry.fft_freqs = snap.fft_freqs;
        telemetry.fft_bin_count = snap.fft_bin_count;
        telemetry.last_note_on = snap.last_note_on;
        telemetry.playing = snap.playing;
        telemetry.tempo = snap.tempo;
        telemetry.beat_position = snap.beat_position;
        telemetry.beat_phase = snap.beat_phase;
        telemetry.last_cc = snap.last_cc;
        telemetry.pitch_bend = snap.pitch_bend;
        telemetry.aftertouch = snap.aftertouch;
        telemetry.voice_count = snap.voice_count;
        telemetry.cpu = snap.cpu;
        telemetry.event_drops = snap.event_drops;
    }

    let dt = time.delta_secs();
    if received_any {
        telemetry.stale_frames = 0;
        telemetry.stale_seconds = 0.0;
        telemetry.note_age_frames = 0;
    } else {
        telemetry.stale_frames = telemetry.stale_frames.saturating_add(1);
        telemetry.stale_seconds += dt;
        telemetry.note_age_frames = telemetry.note_age_frames.saturating_add(1);
    }
}

/// Send a `/viz/pong` reply to the sender.
fn send_pong(socket: &UdpSocket, addr: SocketAddr) {
    let packet = OscPacket::Message(OscMessage {
        addr: addresses::VIZ_PONG.to_owned(),
        args: vec![],
    });
    if let Ok(bytes) = encoder::encode(&packet) {
        let _ = socket.send_to(&bytes, addr);
    }
}

fn handle_packet(packet: &OscPacket, state: &mut OscSnapshot, version_warned: &mut bool) {
    match packet {
        OscPacket::Message(msg) => handle_message(msg, state, version_warned),
        OscPacket::Bundle(bundle) => {
            for p in &bundle.content {
                handle_packet(p, state, version_warned);
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn handle_message(msg: &OscMessage, state: &mut OscSnapshot, version_warned: &mut bool) {
    match msg.addr.as_str() {
        addresses::META => {
            if let [
                OscType::Int(version),
                OscType::Float(sr),
                OscType::Float(rate),
            ] = msg.args.as_slice()
            {
                state.protocol_version = *version;
                state.sample_rate = *sr;
                state.update_rate_hz = *rate;

                if *version != EXPECTED_PROTOCOL_VERSION && !*version_warned {
                    eprintln!(
                        "WARNING: OSC protocol version mismatch (got {version}, expected {EXPECTED_PROTOCOL_VERSION}). \
                         Some telemetry may be incompatible."
                    );
                    *version_warned = true;
                }
            }
        }

        addresses::META_SEQ => {
            if let Some(OscType::Int(seq)) = msg.args.first() {
                state.seq = *seq;
            }
        }

        addresses::META_FFT_FREQS => {
            let count = msg.args.len().min(MAX_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    state.fft_freqs[i] = *v;
                }
            }
            state.fft_bin_count = count;
        }

        addresses::AUDIO_RMS => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                state.rms = [*l, *r];
            }
        }

        addresses::AUDIO_PEAK => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                state.peak = [*l, *r];
            }
        }

        addresses::AUDIO_FFT => {
            let count = msg.args.len().min(MAX_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    state.fft[i] = *v;
                }
            }
            for bin in &mut state.fft[count..MAX_FFT_BANDS] {
                *bin = 0.0;
            }
            state.fft_bin_count = count;
        }

        addresses::AUDIO_CENTROID => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                state.centroid_hz = *v;
            }
        }

        addresses::AUDIO_FLUX => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                state.flux = *v;
            }
        }

        addresses::EVENT_NOTE_ON => {
            if let [
                OscType::Int(note),
                OscType::Int(vel),
                OscType::Int(instrument_id),
                OscType::Int(category),
            ] = msg.args.as_slice()
            {
                let event = NoteOnEvent {
                    midi_note: *note as u8,
                    velocity: *vel as u8,
                    instrument_id: *instrument_id as u32,
                    category: synth_osc_protocol::InstrumentCategory::from_u8(*category as u8),
                };
                state.last_note_on = Some(event);
                state.note_events.push(event);
            }
        }

        addresses::EVENT_NOTE_OFF => {}

        addresses::EVENT_CC => {
            if let [
                OscType::Int(cc),
                OscType::Float(value),
                OscType::Int(channel),
            ] = msg.args.as_slice()
            {
                let cc_num = *cc as u8;
                match cc_num {
                    128 => state.pitch_bend = (*value * 2.0 - 1.0).clamp(-1.0, 1.0),
                    129 => state.aftertouch = value.clamp(0.0, 1.0),
                    _ => state.last_cc = Some((cc_num, *value, *channel as u8)),
                }
            }
        }

        addresses::TRANSPORT_STATE => {
            if let [
                OscType::Int(playing),
                OscType::Float(tempo),
                OscType::Float(beats),
            ] = msg.args.as_slice()
            {
                state.playing = *playing != 0;
                state.tempo = *tempo;
                state.beat_position = *beats;
            }
        }

        addresses::TRANSPORT_PHASE => {
            if let Some(OscType::Float(phase)) = msg.args.first() {
                state.beat_phase = *phase;
            }
        }

        addresses::ENGINE_VOICE_COUNT => {
            if let Some(OscType::Int(count)) = msg.args.first() {
                state.voice_count = *count as u32;
            }
        }

        addresses::ENGINE_CPU => {
            if let Some(OscType::Float(cpu)) = msg.args.first() {
                state.cpu = *cpu;
            }
        }

        addresses::ENGINE_EVENT_DROPS => {
            if let Some(OscType::Int(drops)) = msg.args.first() {
                state.event_drops = *drops as u32;
            }
        }

        addresses::VIZ_CAMERA_MODE => {
            if let Some(OscType::String(mode_name)) = msg.args.first() {
                state.camera_mode_request = Some(mode_name.clone());
            }
        }

        _ => {}
    }
}
