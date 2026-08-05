//! Input handling for the GUI.
//!
//! This module handles keyboard input for playing notes via the computer keyboard,
//! using a piano-style layout.

use std::collections::HashMap;

use eframe::egui;

use crate::gui::keyboard::PianoKeyboard;
use synth_core::MidiNote;
use synth_engine::EngineHandle;
use synth_engine::instrument::MidiChannelSelection;

/// Computer keyboard to MIDI note mapping.
///
/// Uses a piano-style layout where:
/// - Lower row (Z-M) maps to C3-B3
/// - Upper row (Q-U, with number keys for sharps) maps to C4-B4
pub const KEY_MAP: &[(egui::Key, u8)] = &[
    // Lower row: Z-M = C3-B3
    (egui::Key::Z, 48), // C3
    (egui::Key::S, 49), // C#3
    (egui::Key::X, 50), // D3
    (egui::Key::D, 51), // D#3
    (egui::Key::C, 52), // E3
    (egui::Key::V, 53), // F3
    (egui::Key::G, 54), // F#3
    (egui::Key::B, 55), // G3
    (egui::Key::H, 56), // G#3
    (egui::Key::N, 57), // A3
    (egui::Key::J, 58), // A#3
    (egui::Key::M, 59), // B3
    // Upper row: Q-U = C4-B4
    (egui::Key::Q, 60),    // C4 (Middle C)
    (egui::Key::Num2, 61), // C#4
    (egui::Key::W, 62),    // D4
    (egui::Key::Num3, 63), // D#4
    (egui::Key::E, 64),    // E4
    (egui::Key::R, 65),    // F4
    (egui::Key::Num5, 66), // F#4
    (egui::Key::T, 67),    // G4
    (egui::Key::Num6, 68), // G#4
    (egui::Key::Y, 69),    // A4
    (egui::Key::Num7, 70), // A#4
    (egui::Key::U, 71),    // B4
    (egui::Key::I, 72),    // C5
];

/// Input state collected from egui context.
///
/// This struct captures the keyboard state for a single frame,
/// allowing the main handler to process it without holding a reference
/// to the egui input.
struct KeyboardInputState {
    pressed: Vec<u8>,
    released: Vec<u8>,
    octave_down: bool,
    octave_up: bool,
}

impl KeyboardInputState {
    fn collect(ctx: &egui::Context, octave_offset: i32) -> Self {
        let mut pressed = Vec::new();
        let mut released = Vec::new();
        let mut octave_down = false;
        let mut octave_up = false;

        ctx.input(|input| {
            // A held command/ctrl modifier means the keystroke is a shortcut,
            // not a note. Without this check every `Cmd+S` also played a C♯,
            // `Cmd+Z` a C, `Cmd+V` an F — the letters the piano layout happens
            // to use are exactly the ones the standard editing chords use.
            //
            // Note-*offs* are still collected: a note that started before the
            // modifier went down has to be able to stop.
            let modified = input.modifiers.command || input.modifiers.ctrl || input.modifiers.alt;

            for (key, base_note) in KEY_MAP {
                let note_i32 = *base_note as i32 + octave_offset * 12;
                if !(0..=127).contains(&note_i32) {
                    continue;
                }
                let note = note_i32 as u8;

                if !modified && input.key_pressed(*key) {
                    pressed.push(note);
                }
                if input.key_released(*key) {
                    released.push(note);
                }
            }

            octave_down = !modified && input.key_pressed(egui::Key::Minus);
            octave_up = !modified && input.key_pressed(egui::Key::Plus);
        });

        Self {
            pressed,
            released,
            octave_down,
            octave_up,
        }
    }
}

/// Send note-offs for every note the computer keyboard is still holding.
///
/// Called when input stops reaching the piano — a text field takes focus, a
/// modal opens, the window loses focus, the view changes. Without it the key
/// release lands somewhere else and the note sustains forever, which is the
/// most audible bug in the whole input path.
pub fn release_all_keyboard_notes(
    handle: &mut EngineHandle,
    pressed_keys: &mut HashMap<u8, bool>,
    active_channel: MidiChannelSelection,
) {
    for (note, held) in pressed_keys.iter_mut() {
        if *held {
            handle.note_off_channel(MidiNote::new(*note), active_channel);
            *held = false;
        }
    }
}

/// Handle keyboard input for playing notes.
///
/// This function processes computer keyboard input and converts it to MIDI note
/// events, supporting octave shifting and proper note-on/note-off handling.
///
/// # Arguments
///
/// * `ctx` - The egui context for reading input
/// * `handle` - The engine handle for sending note events
/// * `keyboard` - The piano keyboard state for octave offset
/// * `pressed_keys` - Map tracking which keys are currently pressed
/// * `active_channel` - The MIDI channel to send notes to
pub fn handle_keyboard_input(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    keyboard: &mut PianoKeyboard,
    pressed_keys: &mut HashMap<u8, bool>,
    active_channel: MidiChannelSelection,
) {
    let octave_offset = keyboard.octave_offset();

    // Collect input state first (avoids borrow issues)
    let input_state = KeyboardInputState::collect(ctx, octave_offset);

    // Process note-ons
    for note in input_state.pressed {
        if !pressed_keys.get(&note).copied().unwrap_or(false) {
            handle.note_on_channel(
                MidiNote::new(note),
                synth_core::Velocity::new(0.8),
                active_channel,
            );
            pressed_keys.insert(note, true);
        }
    }

    // Process note-offs
    for note in input_state.released {
        handle.note_off_channel(MidiNote::new(note), active_channel);
        pressed_keys.insert(note, false);
    }

    // Octave shift
    if input_state.octave_down && octave_offset > -2 {
        keyboard.set_octave_offset(octave_offset - 1);
    }
    if input_state.octave_up && octave_offset < 4 {
        keyboard.set_octave_offset(octave_offset + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_map_coverage() {
        // Verify we have keys for two octaves worth of notes
        assert_eq!(KEY_MAP.len(), 25);

        // Check that notes are in valid MIDI range
        for (_, note) in KEY_MAP {
            assert!(*note <= 127);
        }

        // Check we have middle C
        assert!(KEY_MAP.iter().any(|(_, note)| *note == 60));
    }

    #[test]
    fn test_key_map_no_duplicates() {
        let mut notes: Vec<u8> = KEY_MAP.iter().map(|(_, n)| *n).collect();
        let original_len = notes.len();
        notes.sort_unstable();
        notes.dedup();
        assert_eq!(notes.len(), original_len, "Duplicate notes in KEY_MAP");
    }
}
