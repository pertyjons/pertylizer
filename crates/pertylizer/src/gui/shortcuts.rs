//! One place that decides what a keystroke means.
//!
//! Keyboard handling used to be spread across the views, each reading raw key
//! state from the egui context. Two problems followed from that, and this module
//! exists to fix both.
//!
//! **The computer-keyboard piano ate everything.** It matched bare letters with
//! no regard for focus or modifiers, so `Ctrl+S` played a C♯ on its way to
//! saving, `Ctrl+Z` played a C, and typing a pattern name into a text field
//! played a chord. See [`InputGate`].
//!
//! **Application shortcuts were per-view.** Save had no binding at all, and the
//! spacebar only started playback inside the sequencer editors. Views each
//! implemented their own copies, which could disagree. [`AppShortcut`] is now
//! the single table, so a menu can render the same binding it dispatches
//! ([`AppShortcut::label`]).
//!
//! # Ordering
//!
//! Application shortcuts are dispatched *before* view input and use
//! [`egui::InputState::consume_shortcut`], which removes the event. A view that
//! also binds the key therefore never sees it, and the two cannot both fire.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

/// An application-wide command that a keystroke can trigger.
///
/// Deliberately small: these are the actions that must work identically from
/// every view. View-local editing (note entry, module selection) stays with the
/// view that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppShortcut {
    /// Save the current project, prompting for a path if it has none.
    Save,
    /// Save the current project under a new name.
    SaveAs,
    /// Start a new, empty project.
    New,
    /// Open an existing project.
    Open,
    /// Undo the last action.
    Undo,
    /// Redo the last undone action.
    Redo,
    /// Start or pause playback.
    TogglePlayback,
}

impl AppShortcut {
    /// Every application shortcut, in menu order.
    pub const ALL: [Self; 7] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Undo,
        Self::Redo,
        Self::TogglePlayback,
    ];

    /// The key combination that triggers this command.
    ///
    /// `Modifiers::COMMAND` is the platform command key — Cmd on macOS, Ctrl
    /// elsewhere — so the bindings read correctly on every platform without a
    /// per-platform table.
    pub const fn binding(self) -> KeyboardShortcut {
        match self {
            Self::Save => KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
            // Shift+Cmd+S rather than a separate key, matching the convention
            // every other application uses for "same action, new target".
            Self::SaveAs => {
                KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S)
            }
            Self::New => KeyboardShortcut::new(Modifiers::COMMAND, Key::N),
            Self::Open => KeyboardShortcut::new(Modifiers::COMMAND, Key::O),
            Self::Undo => KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
            Self::Redo => KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
            // Bare space, the universal transport binding. Safe because the
            // gate below stops it reaching here while text is focused.
            Self::TogglePlayback => KeyboardShortcut::new(Modifiers::NONE, Key::Space),
        }
    }

    /// The command's name for a menu entry.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Save => "Save Project",
            Self::SaveAs => "Save Project As...",
            Self::New => "New Project",
            Self::Open => "Open Project...",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::TogglePlayback => "Play / Pause",
        }
    }
}

/// Whether keyboard input should reach the application at all this frame, and
/// which parts of it.
///
/// The distinction that matters: a focused text field must swallow *everything*
/// (a bare `S` is a character, `Cmd+S` in a text field still means save on most
/// platforms but is not worth the ambiguity), whereas a modal dialog owns input
/// entirely — nothing behind it should react.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputGate {
    /// A text field has keyboard focus.
    pub text_focused: bool,
    /// A modal dialog is open and owns input.
    pub modal_open: bool,
    /// Some widget — a button, a slider — holds keyboard focus.
    ///
    /// Separate from `text_focused` because it blocks less: a focused button
    /// owns the *unmodified* keys it activates on (Space, Enter), but has no
    /// claim on `Cmd+S`. See [`InputGate::allows_unmodified_shortcuts`].
    pub widget_focused: bool,
}

impl InputGate {
    /// Read the gate from the egui context and the app's own modal flags.
    ///
    /// `modal_open` cannot be derived from egui: our dialogs are ordinary
    /// windows that happen to be treated as modal by convention, so the caller
    /// supplies it.
    pub fn new(ctx: &egui::Context, modal_open: bool) -> Self {
        Self {
            text_focused: ctx.text_edit_focused(),
            modal_open,
            widget_focused: ctx.memory(|m| m.focused().is_some()),
        }
    }

    /// Whether shortcuts bound to a *bare* key may fire.
    ///
    /// Transport is the bare spacebar, and the dispatcher consumes keys before
    /// any widget sees them — so without this check, tabbing to a button and
    /// pressing Space would start playback instead of pressing the button, and
    /// the button could not be activated from the keyboard at all. A focused
    /// widget gets first claim on unmodified keys; modified ones (`Cmd+S` and
    /// friends) are unaffected, since no widget binds those.
    #[must_use]
    pub const fn allows_unmodified_shortcuts(self) -> bool {
        self.allows_app_shortcuts() && !self.widget_focused
    }

    /// Whether application shortcuts should be dispatched.
    ///
    /// A modal owns input outright. Text focus blocks them too: `Cmd+N` while
    /// renaming a pattern should not discard the project.
    #[must_use]
    pub const fn allows_app_shortcuts(self) -> bool {
        !self.modal_open && !self.text_focused
    }

    /// Whether the computer-keyboard piano should produce notes.
    ///
    /// Identical to [`Self::allows_app_shortcuts`] today, but kept separate
    /// because they answer different questions and will diverge the moment
    /// either grows a condition of its own.
    #[must_use]
    pub const fn allows_piano_keys(self) -> bool {
        !self.modal_open && !self.text_focused
    }
}

/// How many modifier keys a binding requires.
///
/// Used to order matching, see [`pressed`].
const fn modifier_count(modifiers: Modifiers) -> u8 {
    modifiers.alt as u8
        + modifiers.ctrl as u8
        + modifiers.shift as u8
        + modifiers.mac_cmd as u8
        + modifiers.command as u8
}

/// The application shortcuts pressed this frame.
///
/// Consuming the shortcut removes the event, so a view that binds the same key
/// does not also fire. Returns nothing at all when the gate is closed, which is
/// what keeps `Cmd+S` in a text field from saving.
///
/// # Why the ordering matters
///
/// `consume_shortcut` does **not** require an exact modifier match: `Cmd+S`
/// matches while Shift is also held. Tested in menu order, `Cmd+S` therefore
/// swallows `Shift+Cmd+S`, and Save As could never fire — Save would run and
/// silently overwrite instead of opening the file dialog.
///
/// So bindings are tried most-specific first, by modifier count. A shifted
/// variant is always more specific than its base, so it always gets first
/// refusal, and consuming it leaves nothing for the base to match.
pub fn pressed(ctx: &egui::Context, gate: InputGate) -> Vec<AppShortcut> {
    if !gate.allows_app_shortcuts() {
        return Vec::new();
    }
    let mut by_specificity = AppShortcut::ALL;
    by_specificity
        .sort_by_key(|shortcut| std::cmp::Reverse(modifier_count(shortcut.binding().modifiers)));

    let allow_unmodified = gate.allows_unmodified_shortcuts();
    ctx.input_mut(|input| {
        by_specificity
            .into_iter()
            .filter(|shortcut| {
                let binding = shortcut.binding();
                // A focused widget owns bare keys; leave the event for it
                // rather than consuming it here.
                if binding.modifiers == Modifiers::NONE && !allow_unmodified {
                    return false;
                }
                input.consume_shortcut(&binding)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_gate_blocks_everything() {
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
            InputGate {
                text_focused: true,
                modal_open: true,
                widget_focused: false,
            },
        ] {
            assert!(!gate.allows_app_shortcuts(), "{gate:?}");
            assert!(!gate.allows_piano_keys(), "{gate:?}");
        }
    }

    #[test]
    fn an_open_gate_allows_everything() {
        let gate = InputGate {
            text_focused: false,
            modal_open: false,
            widget_focused: false,
        };
        assert!(gate.allows_app_shortcuts());
        assert!(gate.allows_piano_keys());
        assert!(gate.allows_unmodified_shortcuts());
    }

    /// Two commands sharing a binding would make one of them unreachable.
    #[test]
    fn every_binding_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for shortcut in AppShortcut::ALL {
            let binding = shortcut.binding();
            assert!(
                seen.insert((binding.modifiers, binding.logical_key)),
                "{shortcut:?} duplicates another binding",
            );
        }
    }

    /// Save/Save As and Undo/Redo are distinguished only by Shift; if that were
    /// ever dropped, one would shadow the other.
    #[test]
    fn shifted_variants_differ_from_their_base_binding() {
        for (base, shifted) in [
            (AppShortcut::Save, AppShortcut::SaveAs),
            (AppShortcut::Undo, AppShortcut::Redo),
        ] {
            let base = base.binding();
            let shifted = shifted.binding();
            assert_eq!(base.logical_key, shifted.logical_key);
            assert_ne!(
                base.modifiers, shifted.modifiers,
                "the shifted variant must not share the base modifiers",
            );
            assert!(shifted.modifiers.shift);
            assert!(!base.modifiers.shift);
        }
    }

    /// The piano's letter keys must never be reachable as a bare application
    /// shortcut, or the two would collide on every keystroke.
    #[test]
    fn no_application_shortcut_is_an_unmodified_letter() {
        for shortcut in AppShortcut::ALL {
            let binding = shortcut.binding();
            let is_bare = binding.modifiers == Modifiers::NONE;
            let is_letter = crate::gui::input::KEY_MAP
                .iter()
                .any(|(key, _)| *key == binding.logical_key);
            assert!(
                !(is_bare && is_letter),
                "{shortcut:?} is a bare piano key and would collide with note entry",
            );
        }
    }

    /// Every command needs a name for the menu to show next to its binding.
    #[test]
    fn every_shortcut_has_a_label() {
        for shortcut in AppShortcut::ALL {
            assert!(!shortcut.label().is_empty(), "{shortcut:?}");
        }
    }

    /// A focused button must keep its Space activation. The dispatcher runs
    /// before widget code and consumes keys, so without this a tabbed-to button
    /// could not be pressed from the keyboard at all — Space would start
    /// playback instead.
    #[test]
    fn a_focused_widget_keeps_bare_keys() {
        let gate = InputGate {
            text_focused: false,
            modal_open: false,
            widget_focused: true,
        };
        assert!(
            !gate.allows_unmodified_shortcuts(),
            "a focused widget owns unmodified keys",
        );
        assert!(
            gate.allows_app_shortcuts(),
            "modified shortcuts are unaffected — no widget binds Cmd+S",
        );
    }

    /// With nothing focused, bare-key shortcuts are live.
    #[test]
    fn bare_keys_reach_the_app_when_nothing_is_focused() {
        let gate = InputGate {
            text_focused: false,
            modal_open: false,
            widget_focused: false,
        };
        assert!(gate.allows_unmodified_shortcuts());
    }
}
