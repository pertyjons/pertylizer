//! Keyboard routing: which keystrokes reach the piano, and which reach the
//! application.
//!
//! The bug class this guards against is a keystroke being interpreted twice.
//! Before the shortcut dispatcher existed, the computer-keyboard piano matched
//! bare letters with no regard for focus or modifiers, so every `Ctrl+S` played
//! a C♯ on its way to saving and typing into a text field played chords.

// The shortcut dispatcher is part of the egui shell; a headless build has no
// `gui` module to test.
#![cfg(feature = "gui-egui")]

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use pertylizer::gui::input::KEY_MAP;
use pertylizer::gui::shortcuts::{self, AppShortcut, InputGate};

/// Nothing is claiming the keyboard.
fn open() -> InputGate {
    InputGate {
        text_focused: false,
        modal_open: false,
        widget_focused: false,
    }
}

/// One frame's raw input holding a single key press.
fn key_press(binding: KeyboardShortcut) -> egui::RawInput {
    egui::RawInput {
        events: vec![egui::Event::Key {
            key: binding.logical_key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: binding.modifiers,
        }],
        modifiers: binding.modifiers,
        ..Default::default()
    }
}

/// Run one egui pass over `input` and return what the dispatcher produced.
fn dispatch(input: egui::RawInput, gate: InputGate) -> Vec<AppShortcut> {
    let ctx = egui::Context::default();
    ctx.begin_pass(input);
    let fired = shortcuts::pressed(&ctx, gate);
    let _ = ctx.end_pass();
    fired
}

/// The standard editing chords all use letters the piano layout also claims.
/// Every one of them must be reachable without sounding a note — this test
/// establishes the premise the rest of the file rests on.
#[test]
fn the_standard_editing_chords_are_all_piano_letters() {
    for key in [Key::C, Key::V, Key::X, Key::Z, Key::S, Key::N] {
        assert!(
            KEY_MAP.iter().any(|(mapped, _)| *mapped == key),
            "{key:?} is expected to be a piano key",
        );
    }
}

/// Every application shortcut carries a modifier, except transport, which is
/// bound to a key the piano does not use. Either way none can be mistaken for
/// note entry.
#[test]
fn no_application_shortcut_can_be_mistaken_for_a_note() {
    for shortcut in AppShortcut::ALL {
        let binding = shortcut.binding();
        let bare = binding.modifiers == Modifiers::NONE;
        let is_piano_key = KEY_MAP.iter().any(|(key, _)| *key == binding.logical_key);
        assert!(
            !bare || !is_piano_key,
            "{shortcut:?} is an unmodified piano key and would double as a note",
        );
    }
}

/// Transport must be the bare spacebar — the universal binding, and the one
/// that has to work from every view.
#[test]
fn transport_is_the_bare_spacebar() {
    let binding = AppShortcut::TogglePlayback.binding();
    assert_eq!(binding.logical_key, Key::Space);
    assert_eq!(binding.modifiers, Modifiers::NONE);
}

/// A focused text field must silence both paths: typing "Snare" into a pattern
/// name must not save the project or play a chord.
#[test]
fn a_focused_text_field_silences_all_keyboard_handling() {
    let gate = InputGate {
        text_focused: true,
        modal_open: false,
        widget_focused: false,
    };
    assert!(!gate.allows_piano_keys());
    assert!(!gate.allows_app_shortcuts());
}

/// A modal owns input outright; nothing behind it may react.
#[test]
fn a_modal_dialog_silences_all_keyboard_handling() {
    let gate = InputGate {
        text_focused: false,
        modal_open: true,
        widget_focused: false,
    };
    assert!(!gate.allows_piano_keys());
    assert!(!gate.allows_app_shortcuts());
}

/// With nothing claiming the keyboard, both paths are live.
#[test]
fn an_unclaimed_keyboard_reaches_both_paths() {
    assert!(open().allows_piano_keys());
    assert!(open().allows_app_shortcuts());
}

/// A closed gate must produce no commands whatever was pressed. This is the
/// property that keeps `Cmd+N` in a text field from discarding the project.
#[test]
fn no_shortcut_dispatches_through_a_closed_gate() {
    for gate in [
        InputGate {
            text_focused: true,
            modal_open: false,
            widget_focused: false,
        },
        InputGate {
            text_focused: false,
            modal_open: true,
            widget_focused: false,
        },
    ] {
        for shortcut in AppShortcut::ALL {
            let fired = dispatch(key_press(shortcut.binding()), gate);
            assert!(
                fired.is_empty(),
                "{shortcut:?} fired through a closed gate {gate:?}",
            );
        }
    }
}

/// End-to-end through a real egui context: pressing a binding produces that
/// command, and only that command.
#[test]
fn pressing_a_binding_dispatches_exactly_that_command() {
    for shortcut in AppShortcut::ALL {
        let binding = shortcut.binding();
        assert_eq!(
            dispatch(key_press(binding), open()),
            vec![shortcut],
            "{shortcut:?} ({:?} + {:?}) should dispatch alone",
            binding.modifiers,
            binding.logical_key,
        );
    }
}

/// Save and Save As share the `S` key and differ only by Shift. Pressing the
/// plain form must not also trigger the shifted one, or a Save would open a
/// file dialog.
#[test]
fn a_shifted_binding_does_not_also_fire_its_unshifted_twin() {
    for (plain, shifted) in [
        (AppShortcut::Save, AppShortcut::SaveAs),
        (AppShortcut::Undo, AppShortcut::Redo),
    ] {
        for (pressed, other) in [(plain, shifted), (shifted, plain)] {
            let fired = dispatch(key_press(pressed.binding()), open());
            assert!(fired.contains(&pressed), "{pressed:?} must fire");
            assert!(
                !fired.contains(&other),
                "{pressed:?} must not also fire {other:?}",
            );
        }
    }
}

/// A dispatched shortcut is consumed, so a view binding the same key cannot
/// also act on it. Without this, `Cmd+Z` could undo twice.
#[test]
fn a_dispatched_shortcut_is_consumed() {
    let binding = AppShortcut::Undo.binding();
    let ctx = egui::Context::default();
    ctx.begin_pass(key_press(binding));

    let fired = shortcuts::pressed(&ctx, open());
    assert_eq!(fired, vec![AppShortcut::Undo]);
    // What a view would do afterwards.
    let still_available = ctx.input_mut(|i| i.consume_shortcut(&binding));

    let _ = ctx.end_pass();
    assert!(
        !still_available,
        "the key must be gone once the dispatcher has acted on it",
    );
}

/// An unbound keystroke must dispatch nothing — the dispatcher must not swallow
/// keys the views need.
#[test]
fn an_unbound_key_dispatches_nothing() {
    let binding = KeyboardShortcut::new(Modifiers::NONE, Key::F9);
    assert!(dispatch(key_press(binding), open()).is_empty());
}

/// A bare piano letter must reach note entry rather than being eaten as a
/// command — the dispatcher must not over-claim.
#[test]
fn a_bare_piano_letter_dispatches_no_command() {
    for key in [Key::Z, Key::S, Key::C, Key::V, Key::X, Key::N] {
        let binding = KeyboardShortcut::new(Modifiers::NONE, key);
        assert!(
            dispatch(key_press(binding), open()).is_empty(),
            "bare {key:?} must stay available for note entry",
        );
    }
}

/// A focused widget must keep the bare spacebar: the dispatcher consumes keys
/// before widget code runs, so claiming Space here would make a tabbed-to
/// button impossible to activate from the keyboard.
#[test]
fn transport_yields_the_spacebar_to_a_focused_widget() {
    let focused = InputGate {
        text_focused: false,
        modal_open: false,
        widget_focused: true,
    };
    let fired = dispatch(key_press(AppShortcut::TogglePlayback.binding()), focused);
    assert!(
        fired.is_empty(),
        "Space must stay available for the focused widget, got {fired:?}",
    );
}

/// A focused widget has no claim on modified shortcuts, so saving still works
/// while a fader has focus.
#[test]
fn modified_shortcuts_still_fire_while_a_widget_is_focused() {
    let focused = InputGate {
        text_focused: false,
        modal_open: false,
        widget_focused: true,
    };
    assert_eq!(
        dispatch(key_press(AppShortcut::Save.binding()), focused),
        vec![AppShortcut::Save],
    );
}
