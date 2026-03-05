//! Non-blocking UDP OSC receiver system for Bevy.

use std::net::UdpSocket;

use bevy::prelude::*;
use rosc::{OscMessage, OscPacket, OscType};

use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

/// UDP socket resource for receiving OSC packets.
#[derive(Resource)]
pub struct OscSocket {
    socket: UdpSocket,
    buf: Vec<u8>,
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
    });
}

fn receive_osc(mut socket: ResMut<OscSocket>, mut telemetry: ResMut<SynthTelemetry>) {
    let mut received_any = false;

    // Drain all pending UDP packets (non-blocking)
    let sock = &mut *socket;
    while let Ok(size) = sock.socket.recv(&mut sock.buf) {
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&sock.buf[..size]) {
            handle_packet(&packet, &mut telemetry);
            received_any = true;
        }
    }

    telemetry.stale_frames = if received_any {
        0
    } else {
        telemetry.stale_frames.saturating_add(1)
    };
    telemetry.note_age_frames = telemetry.note_age_frames.saturating_add(1);
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
            if let [OscType::Int(version), OscType::Float(sr), OscType::Float(rate)] =
                msg.args.as_slice()
            {
                telemetry.protocol_version = *version;
                telemetry.sample_rate = *sr;
                telemetry.update_rate_hz = *rate;
            }
        }

        "/synth/meta/seq" => {
            if let Some(OscType::Int(seq)) = msg.args.first() {
                telemetry.seq = *seq;
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

        "/synth/event/note_on" => {
            if let [OscType::Int(note), OscType::Int(vel), OscType::Int(ch)] =
                msg.args.as_slice()
            {
                telemetry.last_note_on = Some((*note as u8, *vel as u8, *ch as u8));
                telemetry.note_age_frames = 0;
            }
        }

        "/synth/event/note_off" => {
            // Could track note-off for sustained visuals; for now just ignore
        }

        "/synth/transport/state" => {
            if let [OscType::Int(playing), OscType::Float(tempo), OscType::Float(beats)] =
                msg.args.as_slice()
            {
                telemetry.playing = *playing != 0;
                telemetry.tempo = *tempo;
                telemetry.beat_position = *beats;
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

        _ => {}
    }
}
