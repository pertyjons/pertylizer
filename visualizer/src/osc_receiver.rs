//! Non-blocking UDP OSC receiver system for Bevy.
//!
//! Receives OSC bundles from Pertylizer and sends `/viz/pong` replies
//! so the sender knows a client is connected (enables full telemetry).

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use bevy::prelude::*;
use rosc::{OscMessage, OscPacket, OscType, encoder};

use synth_osc_protocol::addresses;

use crate::telemetry::{EXPECTED_PROTOCOL_VERSION, MAX_FFT_BANDS, NoteOnEvent, SynthTelemetry};

/// How often to send `/viz/pong` back to the sender.
const PONG_INTERVAL_SECS: f32 = synth_osc_protocol::PONG_REPLY_INTERVAL_SECS;

/// UDP socket resource for receiving OSC packets.
#[derive(Resource)]
pub struct OscSocket {
    socket: UdpSocket,
    buf: Vec<u8>,
    /// Address of the last sender (to reply with pong).
    sender_addr: Option<SocketAddr>,
    /// Last time we sent a pong.
    last_pong_sent: Instant,
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
    socket
        .set_nonblocking(true)
        .expect("Failed to set non-blocking");

    println!("OSC receiver: listening on {bind_addr}");

    commands.insert_resource(OscSocket {
        socket,
        buf: vec![0u8; 65_536],
        sender_addr: None,
        last_pong_sent: Instant::now(),
    });
}

fn receive_osc(
    time: Res<Time>,
    mut socket: ResMut<OscSocket>,
    mut telemetry: ResMut<SynthTelemetry>,
) {
    // Clear pending events from previous frame before processing new packets
    telemetry.pending_note_events.clear();

    let mut received_any = false;

    // Drain all pending UDP packets (non-blocking)
    let sock = &mut *socket;
    while let Ok((size, addr)) = sock.socket.recv_from(&mut sock.buf) {
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&sock.buf[..size]) {
            handle_packet(&packet, &mut telemetry);
            sock.sender_addr = Some(addr);
            received_any = true;
        }
    }

    // Send /viz/pong back to the sender periodically
    if received_any {
        let now = Instant::now();
        if now.duration_since(sock.last_pong_sent).as_secs_f32() >= PONG_INTERVAL_SECS
            && let Some(addr) = sock.sender_addr
        {
            send_pong(&sock.socket, addr);
            sock.last_pong_sent = now;
        }
    }

    let dt = time.delta_secs();
    if received_any {
        telemetry.stale_frames = 0;
        telemetry.stale_seconds = 0.0;
    } else {
        telemetry.stale_frames = telemetry.stale_frames.saturating_add(1);
        telemetry.stale_seconds += dt;
    }
    telemetry.note_age_frames = telemetry.note_age_frames.saturating_add(1);
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

fn handle_packet(packet: &OscPacket, telemetry: &mut SynthTelemetry) {
    match packet {
        OscPacket::Message(msg) => handle_message(msg, telemetry),
        OscPacket::Bundle(bundle) => {
            for p in &bundle.content {
                handle_packet(p, telemetry);
            }
        }
    }
}

fn handle_message(msg: &OscMessage, telemetry: &mut SynthTelemetry) {
    match msg.addr.as_str() {
        addresses::META => {
            if let [
                OscType::Int(version),
                OscType::Float(sr),
                OscType::Float(rate),
            ] = msg.args.as_slice()
            {
                telemetry.protocol_version = *version;
                telemetry.sample_rate = *sr;
                telemetry.update_rate_hz = *rate;

                // Protocol version check (warn once)
                if *version != EXPECTED_PROTOCOL_VERSION && !telemetry.version_warned {
                    eprintln!(
                        "WARNING: OSC protocol version mismatch (got {version}, expected {EXPECTED_PROTOCOL_VERSION}). \
                         Some telemetry may be incompatible."
                    );
                    telemetry.version_warned = true;
                }
            }
        }

        addresses::META_SEQ => {
            if let Some(OscType::Int(seq)) = msg.args.first() {
                telemetry.seq = *seq;
            }
        }

        addresses::META_FFT_FREQS => {
            let count = msg.args.len().min(MAX_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    telemetry.fft_freqs[i] = *v;
                }
            }
            telemetry.fft_bin_count = count;
        }

        addresses::AUDIO_RMS => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                telemetry.rms = [*l, *r];
            }
        }

        addresses::AUDIO_PEAK => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                telemetry.peak = [*l, *r];
            }
        }

        addresses::AUDIO_FFT => {
            let count = msg.args.len().min(MAX_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    telemetry.fft[i] = *v;
                }
            }
            // Clear any leftover bins beyond received count
            for bin in &mut telemetry.fft[count..MAX_FFT_BANDS] {
                *bin = 0.0;
            }
            telemetry.fft_bin_count = count;
        }

        addresses::AUDIO_CENTROID => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                telemetry.centroid_hz = *v;
            }
        }

        addresses::AUDIO_FLUX => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                telemetry.flux = *v;
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
                telemetry.last_note_on = Some(event);
                telemetry.note_age_frames = 0;
                telemetry.pending_note_events.push(event);
            }
        }

        addresses::EVENT_NOTE_OFF => {
            // Could track note-off for sustained visuals; for now just ignore
        }

        addresses::EVENT_CC => {
            if let [OscType::Int(cc), OscType::Float(value), OscType::Int(channel)] =
                msg.args.as_slice()
            {
                let cc_num = *cc as u8;
                // Pitch bend is sent as CC 128, aftertouch as CC 129
                match cc_num {
                    128 => telemetry.pitch_bend = (*value * 2.0 - 1.0).clamp(-1.0, 1.0),
                    129 => telemetry.aftertouch = value.clamp(0.0, 1.0),
                    _ => telemetry.last_cc = Some((cc_num, *value, *channel as u8)),
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
                telemetry.playing = *playing != 0;
                telemetry.tempo = *tempo;
                telemetry.beat_position = *beats;
            }
        }

        addresses::TRANSPORT_PHASE => {
            if let Some(OscType::Float(phase)) = msg.args.first() {
                telemetry.beat_phase = *phase;
            }
        }

        addresses::ENGINE_VOICE_COUNT => {
            if let Some(OscType::Int(count)) = msg.args.first() {
                telemetry.voice_count = *count as u32;
            }
        }

        addresses::ENGINE_CPU => {
            if let Some(OscType::Float(cpu)) = msg.args.first() {
                telemetry.cpu = *cpu;
            }
        }

        addresses::ENGINE_EVENT_DROPS => {
            if let Some(OscType::Int(drops)) = msg.args.first() {
                telemetry.event_drops = *drops as u32;
            }
        }

        // Ignore ping and unknown addresses
        _ => {}
    }
}
