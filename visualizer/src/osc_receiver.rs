//! Non-blocking UDP OSC receiver system for Bevy.
//!
//! Receives OSC bundles from Pertylizer and sends `/viz/pong` replies
//! so the sender knows a client is connected (enables full telemetry).

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use bevy::prelude::*;
use rosc::{OscMessage, OscPacket, OscType, encoder};

use crate::telemetry::{EXPECTED_PROTOCOL_VERSION, NUM_FFT_BANDS, SynthTelemetry};

/// How often to send `/viz/pong` back to the sender.
const PONG_INTERVAL_SECS: f32 = 2.0;

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
    let socket = UdpSocket::bind("0.0.0.0:9000").expect("Failed to bind OSC port 9000");
    socket
        .set_nonblocking(true)
        .expect("Failed to set non-blocking");

    println!("OSC receiver: listening on 0.0.0.0:9000");

    commands.insert_resource(OscSocket {
        socket,
        buf: vec![0u8; 8192],
        sender_addr: None,
        last_pong_sent: Instant::now(),
    });
}

fn receive_osc(mut socket: ResMut<OscSocket>, mut telemetry: ResMut<SynthTelemetry>) {
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

    telemetry.stale_frames = if received_any {
        0
    } else {
        telemetry.stale_frames.saturating_add(1)
    };
    telemetry.note_age_frames = telemetry.note_age_frames.saturating_add(1);
}

/// Send a `/viz/pong` reply to the sender.
fn send_pong(socket: &UdpSocket, addr: SocketAddr) {
    let packet = OscPacket::Message(OscMessage {
        addr: "/viz/pong".to_owned(),
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
        "/synth/meta" => {
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

        "/synth/meta/seq" => {
            if let Some(OscType::Int(seq)) = msg.args.first() {
                telemetry.seq = *seq;
            }
        }

        "/synth/meta/fft_freqs" => {
            let count = msg.args.len().min(NUM_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    telemetry.fft_freqs[i] = *v;
                }
            }
        }

        "/synth/audio/rms" => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                telemetry.rms = [*l, *r];
            }
        }

        "/synth/audio/peak" => {
            if let [OscType::Float(l), OscType::Float(r)] = msg.args.as_slice() {
                telemetry.peak = [*l, *r];
            }
        }

        "/synth/audio/fft" => {
            let count = msg.args.len().min(NUM_FFT_BANDS);
            for (i, arg) in msg.args.iter().enumerate().take(count) {
                if let OscType::Float(v) = arg {
                    telemetry.fft[i] = *v;
                }
            }
        }

        "/synth/audio/centroid" => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                telemetry.centroid_hz = *v;
            }
        }

        "/synth/audio/flux" => {
            if let Some(OscType::Float(v)) = msg.args.first() {
                telemetry.flux = *v;
            }
        }

        "/synth/event/note_on" => {
            if let [
                OscType::Int(note),
                OscType::Int(vel),
                OscType::Int(instrument_id),
                OscType::Int(category),
            ] = msg.args.as_slice()
            {
                let event = (
                    *note as u8,
                    *vel as u8,
                    *instrument_id as u32,
                    *category as u8,
                );
                telemetry.last_note_on = Some(event);
                telemetry.note_age_frames = 0;
                telemetry.pending_note_events.push(event);
            }
        }

        "/synth/event/note_off" => {
            // Could track note-off for sustained visuals; for now just ignore
        }

        "/synth/event/cc" => {
            // CC events available for future visual effects
        }

        "/synth/transport/state" => {
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

        "/synth/transport/phase" => {
            if let Some(OscType::Float(phase)) = msg.args.first() {
                telemetry.beat_phase = *phase;
            }
        }

        "/synth/engine/voice_count" => {
            if let Some(OscType::Int(count)) = msg.args.first() {
                telemetry.voice_count = *count as u32;
            }
        }

        "/synth/engine/cpu" => {
            if let Some(OscType::Float(cpu)) = msg.args.first() {
                telemetry.cpu = *cpu;
            }
        }

        "/synth/engine/event_drops" => {
            if let Some(OscType::Int(drops)) = msg.args.first() {
                telemetry.event_drops = *drops as u32;
            }
        }

        // Ignore ping and unknown addresses
        _ => {}
    }
}
