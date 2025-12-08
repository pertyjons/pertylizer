//! MIDI input handling using midir.
//!
//! This module provides a hardware abstraction layer for MIDI input.
//! It converts raw MIDI bytes to domain types immediately upon receipt,
//! then forwards them as EngineCommands to the audio thread.
//!
//! ## Architecture
//!
//! The MIDI layer is purely an I/O adapter:
//! - The SynthEngine remains ignorant of MIDI hardware
//! - All MIDI input is converted to EngineCommand immediately
//! - Visual feedback routes via EngineEvent (NoteTriggered/NoteReleased)
//!
//! ## Type Safety
//!
//! Raw MIDI bytes (u8) are converted to domain types at the parsing layer:
//! - Velocity (0-127) -> NormalizedValue (0.0-1.0)
//! - Pitch bend (0-16383) -> BipolarValue (-1.0 to 1.0)
//! - Channel (0-15) -> MidiChannel

use midir::{Ignore, MidiInput, MidiInputConnection};
use thiserror::Error;

use crate::engine::CommandSender;
use crate::engine::commands::EngineCommand;
use crate::engine::instrument::MidiChannel;
use crate::types::{BipolarValue, MidiNote, NormalizedValue};

/// MIDI message status bytes (high nibble).
mod status {
    pub const NOTE_OFF: u8 = 0x80;
    pub const NOTE_ON: u8 = 0x90;
    pub const POLY_AFTERTOUCH: u8 = 0xA0;
    pub const CONTROL_CHANGE: u8 = 0xB0;
    pub const CHANNEL_PRESSURE: u8 = 0xD0;
    pub const PITCH_BEND: u8 = 0xE0;
}

/// MIDI CC numbers.
mod cc {
    pub const MOD_WHEEL: u8 = 1;
    pub const ALL_NOTES_OFF: u8 = 123;
}

/// Parse raw MIDI bytes into an EngineCommand.
///
/// CRITICAL: Handles the MIDI quirk where NoteOn with velocity 0 is treated as NoteOff.
///
/// Returns None for unhandled MIDI messages (SysEx, clock, etc.)
pub fn parse_midi(bytes: &[u8]) -> Option<EngineCommand> {
    if bytes.is_empty() {
        return None;
    }

    let status = bytes[0];
    let message_type = status & 0xF0;
    let channel_raw = status & 0x0F;

    // Create MidiChannel from raw byte (0-indexed)
    let channel = MidiChannel::from_zero_indexed(channel_raw)?;

    match message_type {
        status::NOTE_ON if bytes.len() >= 3 => {
            let note = MidiNote::new(bytes[1] & 0x7F);
            let velocity_raw = bytes[2] & 0x7F;

            // CRITICAL: NoteOn with velocity 0 is actually NoteOff
            if velocity_raw == 0 {
                Some(EngineCommand::NoteOff { note, channel })
            } else {
                // Convert 0-127 to 0.0-1.0
                let velocity = NormalizedValue::new(velocity_raw as f32 / 127.0);
                Some(EngineCommand::NoteOn {
                    note,
                    velocity,
                    channel,
                })
            }
        }

        status::NOTE_OFF if bytes.len() >= 3 => {
            let note = MidiNote::new(bytes[1] & 0x7F);
            // Release velocity is ignored (bytes[2])
            Some(EngineCommand::NoteOff { note, channel })
        }

        status::POLY_AFTERTOUCH if bytes.len() >= 3 => {
            let note = MidiNote::new(bytes[1] & 0x7F);
            let pressure = bytes[2] & 0x7F;
            let value = NormalizedValue::new(pressure as f32 / 127.0);
            Some(EngineCommand::PolyAftertouch {
                note,
                value,
                channel,
            })
        }

        status::CONTROL_CHANGE if bytes.len() >= 3 => {
            let cc_number = bytes[1] & 0x7F;
            let cc_value = bytes[2] & 0x7F;

            match cc_number {
                cc::MOD_WHEEL => {
                    let value = NormalizedValue::new(cc_value as f32 / 127.0);
                    Some(EngineCommand::ModWheel { value, channel })
                }
                cc::ALL_NOTES_OFF => Some(EngineCommand::AllNotesOff),
                // TODO: Add more CC handling as needed (sustain, expression, etc.)
                _ => None,
            }
        }

        status::CHANNEL_PRESSURE if bytes.len() >= 2 => {
            let pressure = bytes[1] & 0x7F;
            let value = NormalizedValue::new(pressure as f32 / 127.0);
            Some(EngineCommand::Aftertouch { value, channel })
        }

        status::PITCH_BEND if bytes.len() >= 3 => {
            // Pitch bend is 14-bit: LSB (data1) + MSB (data2)
            let lsb = bytes[1] as u16 & 0x7F;
            let msb = bytes[2] as u16 & 0x7F;
            let raw_value = (msb << 7) | lsb; // 0-16383, center at 8192

            // Convert to bipolar: -1.0 to +1.0
            // 0 -> -1.0, 8192 -> 0.0, 16383 -> +1.0
            let normalized = (raw_value as f32 - 8192.0) / 8192.0;
            let value = BipolarValue::new(normalized);

            Some(EngineCommand::PitchBend { value, channel })
        }

        _ => None,
    }
}

/// Sender trait for MIDI commands.
///
/// This abstraction allows testing without a real CommandSender.
#[allow(dead_code)] // Designed for test mocking
pub trait MidiCommandSender: Send + 'static {
    fn send(&self, command: EngineCommand) -> bool;
}

impl MidiCommandSender for CommandSender {
    fn send(&self, command: EngineCommand) -> bool {
        CommandSender::send(self, command)
    }
}

/// Manages MIDI input connections.
///
/// The MidiHandler owns the midir connection and forwards parsed MIDI
/// messages to the engine via a CommandSender.
pub struct MidiHandler {
    /// The active MIDI input connection, if any.
    connection: Option<MidiInputConnection<()>>,
    /// Name of the connected port.
    port_name: Option<String>,
    /// Command sender for reconnection.
    sender: CommandSender,
}

impl MidiHandler {
    /// Create a new MidiHandler and connect to the first available MIDI port.
    ///
    /// The CommandSender is cloned into the midir callback closure to satisfy
    /// Send requirements across threads.
    pub fn new(sender: CommandSender) -> Self {
        let mut handler = Self {
            connection: None,
            port_name: None,
            sender,
        };

        let ports = Self::list_ports();

        // Prefer hardware ports (skip "Midi Through" virtual ports)
        let preferred_port = ports
            .iter()
            .find(|p| !p.to_lowercase().contains("midi through"))
            .or(ports.first());

        if let Some(port) = preferred_port {
            println!("MIDI: Auto-selecting '{}'", port);
            let _ = handler.connect_to(port);
        } else {
            println!("MIDI: No ports available");
        }

        handler
    }

    /// Connect to a specific MIDI port by name.
    ///
    /// Disconnects from any current port first.
    pub fn connect_to(&mut self, port_name: &str) -> Result<(), MidiError> {
        // Disconnect current connection
        self.connection = None;
        self.port_name = None;

        println!("MIDI: Connecting to '{}'...", port_name);

        let mut midi_in = MidiInput::new("modular-synth").map_err(MidiError::Init)?;
        midi_in.ignore(Ignore::None);

        let ports = midi_in.ports();
        let port = ports
            .iter()
            .find(|p| {
                midi_in
                    .port_name(p)
                    .map(|n| n == port_name)
                    .unwrap_or(false)
            })
            .ok_or(MidiError::PortNotFound)?;

        let sender = self.sender.clone();
        let connection = midi_in
            .connect(
                port,
                "modular-synth-input",
                move |_timestamp, message, _| {
                    if let Some(command) = parse_midi(message) {
                        let _ = sender.send(command);
                    }
                },
                (),
            )
            .map_err(MidiError::Connect)?;

        println!("MIDI: Connected to '{}'", port_name);

        self.connection = Some(connection);
        self.port_name = Some(port_name.to_string());

        Ok(())
    }

    /// List available MIDI input ports.
    pub fn list_ports() -> Vec<String> {
        match MidiInput::new("modular-synth-probe") {
            Ok(midi_in) => midi_in
                .ports()
                .iter()
                .filter_map(|p| midi_in.port_name(p).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Check if connected to a MIDI port.
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Get the name of the connected port.
    pub fn port_name(&self) -> Option<&str> {
        self.port_name.as_deref()
    }
}

/// MIDI-related errors.
#[derive(Debug, Error)]
pub enum MidiError {
    /// Failed to initialize MIDI subsystem.
    #[error("MIDI init error: {0}")]
    Init(#[from] midir::InitError),
    /// No MIDI ports available.
    #[error("No MIDI input ports available")]
    NoPortsAvailable,
    /// Requested port not found.
    #[error("MIDI port not found")]
    PortNotFound,
    /// Failed to connect to port.
    #[error("MIDI connect error: {0}")]
    Connect(midir::ConnectError<MidiInput>),
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_note_on() {
        // Note On: channel 0, note 60 (C4), velocity 100
        let msg = [0x90, 60, 100];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::NoteOn {
                note,
                velocity,
                channel,
            } => {
                assert_eq!(note, MidiNote::C4);
                assert!((velocity.as_f32() - 100.0 / 127.0).abs() < 0.01);
                assert_eq!(channel.as_zero_indexed(), 0);
            }
            _ => panic!("Expected NoteOn"),
        }
    }

    #[test]
    fn test_parse_note_on_velocity_zero_is_note_off() {
        // Note On with velocity 0 should be parsed as NoteOff (MIDI quirk)
        let msg = [0x90, 64, 0];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::NoteOff { note, channel } => {
                assert_eq!(note, MidiNote::new(64));
                assert_eq!(channel.as_zero_indexed(), 0);
            }
            _ => panic!("Expected NoteOff for velocity 0"),
        }
    }

    #[test]
    fn test_parse_note_off() {
        // Note Off: channel 1, note 72
        let msg = [0x81, 72, 64];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::NoteOff { note, channel } => {
                assert_eq!(note, MidiNote::new(72));
                assert_eq!(channel.as_zero_indexed(), 1);
            }
            _ => panic!("Expected NoteOff"),
        }
    }

    #[test]
    fn test_parse_pitch_bend_center() {
        // Pitch bend centered: 0x00, 0x40 = 8192
        let msg = [0xE0, 0x00, 0x40];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::PitchBend { value, .. } => {
                assert!(value.as_f32().abs() < 0.01, "Center should be ~0.0");
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[test]
    fn test_parse_pitch_bend_max() {
        // Pitch bend max: 0x7F, 0x7F = 16383
        let msg = [0xE0, 0x7F, 0x7F];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::PitchBend { value, .. } => {
                assert!((value.as_f32() - 1.0).abs() < 0.01, "Max should be ~1.0");
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[test]
    fn test_parse_pitch_bend_min() {
        // Pitch bend min: 0x00, 0x00 = 0
        let msg = [0xE0, 0x00, 0x00];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::PitchBend { value, .. } => {
                assert!((value.as_f32() + 1.0).abs() < 0.01, "Min should be ~-1.0");
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[test]
    fn test_parse_mod_wheel() {
        // CC 1 (mod wheel): channel 0, value 64 (half)
        let msg = [0xB0, 1, 64];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::ModWheel { value, channel } => {
                assert!((value.as_f32() - 64.0 / 127.0).abs() < 0.01);
                assert_eq!(channel.as_zero_indexed(), 0);
            }
            _ => panic!("Expected ModWheel"),
        }
    }

    #[test]
    fn test_parse_aftertouch() {
        // Channel pressure: channel 2, pressure 100
        let msg = [0xD2, 100];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::Aftertouch { value, channel } => {
                assert!((value.as_f32() - 100.0 / 127.0).abs() < 0.01);
                assert_eq!(channel.as_zero_indexed(), 2);
            }
            _ => panic!("Expected Aftertouch"),
        }
    }

    #[test]
    fn test_parse_poly_aftertouch() {
        // Polyphonic key pressure: channel 0, note 60, pressure 80
        let msg = [0xA0, 60, 80];
        let cmd = parse_midi(&msg).unwrap();

        match cmd {
            EngineCommand::PolyAftertouch {
                note,
                value,
                channel,
            } => {
                assert_eq!(note, MidiNote::C4);
                assert!((value.as_f32() - 80.0 / 127.0).abs() < 0.01);
                assert_eq!(channel.as_zero_indexed(), 0);
            }
            _ => panic!("Expected PolyAftertouch"),
        }
    }

    #[test]
    fn test_parse_all_notes_off() {
        // CC 123 (all notes off)
        let msg = [0xB0, 123, 0];
        let cmd = parse_midi(&msg).unwrap();

        assert!(matches!(cmd, EngineCommand::AllNotesOff));
    }

    #[test]
    fn test_parse_unknown_cc_returns_none() {
        // Unknown CC number
        let msg = [0xB0, 42, 100];
        assert!(parse_midi(&msg).is_none());
    }

    #[test]
    fn test_parse_empty_returns_none() {
        assert!(parse_midi(&[]).is_none());
    }
}
