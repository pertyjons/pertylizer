//! Piano keyboard component for the modular synthesizer.
//!
//! Provides a piano keyboard that dynamically adjusts its number of octaves
//! to fit the available width. Can be played with mouse and shows the
//! currently active octave range for computer keyboard input.

use crate::gui::theme::theme;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;
use synth_core::{MidiNote, Velocity};

/// Width of a single white key in pixels.
const WHITE_KEY_WIDTH: f32 = 24.0;
/// Number of white keys per octave (C D E F G A B).
const WHITE_KEYS_PER_OCTAVE: u32 = 7;

/// Result of keyboard interaction
#[derive(Debug, Clone, Default)]
pub struct KeyboardEvent {
    /// Note that was pressed (MidiNote for type safety)
    pub note_on: Option<MidiNote>,
    /// Notes that were released
    pub note_off: Vec<MidiNote>,
    pub octave_change: i32,
}

/// Piano keyboard widget with dynamic octave count
pub struct PianoKeyboard {
    /// Currently pressed keys (raw MIDI note -> velocity 0.0-1.0)
    pressed_keys: HashMap<u8, f32>,
    /// Keys pressed by mouse (need to track separately for release)
    mouse_pressed_keys: HashMap<u8, bool>,
    /// Current octave offset for computer keyboard (-2 to +4)
    octave_offset: i32,
}

impl PianoKeyboard {
    /// Create a new piano keyboard
    pub fn new() -> Self {
        Self {
            pressed_keys: HashMap::new(),
            mouse_pressed_keys: HashMap::new(),
            octave_offset: -1, // Start at C3 range
        }
    }

    /// Get the current octave offset
    pub fn octave_offset(&self) -> i32 {
        self.octave_offset
    }

    /// Set the current octave offset
    pub fn set_octave_offset(&mut self, offset: i32) {
        self.octave_offset = offset.clamp(-2, 4);
    }

    /// Mark a note as pressed with velocity (for visual feedback)
    pub fn set_note_on(&mut self, note: MidiNote, velocity: Velocity) {
        self.pressed_keys.insert(note.as_u8(), velocity.as_f32());
    }

    /// Mark a note as released
    pub fn set_note_off(&mut self, note: MidiNote) {
        self.pressed_keys.remove(&note.as_u8());
    }

    /// Check if a note is pressed
    pub fn is_note_pressed(&self, note: MidiNote) -> bool {
        self.pressed_keys.contains_key(&note.as_u8())
    }

    /// Get the velocity of a pressed note (0.0 if not pressed)
    pub fn get_velocity(&self, note: MidiNote) -> f32 {
        self.pressed_keys.get(&note.as_u8()).copied().unwrap_or(0.0)
    }

    /// Clear all pressed notes
    pub fn clear_pressed(&mut self) {
        self.pressed_keys.clear();
        self.mouse_pressed_keys.clear();
    }

    /// Calculate how many octaves fit in the given width.
    /// Returns a value between 2 and 8.
    #[must_use]
    pub fn octaves_for_width(available_width: f32) -> u32 {
        let max_octaves =
            (available_width / (WHITE_KEYS_PER_OCTAVE as f32 * WHITE_KEY_WIDTH)) as u32;
        max_octaves.clamp(2, 8)
    }

    /// Calculate the exact pixel width the piano needs for the given number of octaves.
    #[must_use]
    pub fn width_for_octaves(octaves: u32) -> f32 {
        let white_keys = octaves * WHITE_KEYS_PER_OCTAVE + 1; // +1 for final C
        white_keys as f32 * WHITE_KEY_WIDTH
    }

    /// Returns true if the given MIDI note is a black key
    fn is_black_key(note: u8) -> bool {
        matches!(note % 12, 1 | 3 | 6 | 8 | 10)
    }

    /// Get the octave highlight range (start note, end note) based on octave offset
    fn get_highlight_range(&self) -> (u8, u8) {
        let base_note = (48i32 + self.octave_offset * 12).clamp(0, 127) as u8;
        let end_note = (base_note as u16 + 24).min(127) as u8;
        (base_note, end_note)
    }

    /// Draw the keyboard header (KEYBOARD label, octave controls, Center button).
    pub fn show_header(&mut self, ui: &mut egui::Ui) -> i32 {
        let mut octave_change = 0;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Octave: {:+}", self.octave_offset))
                    .color(theme().colors.text_secondary),
            );
            if ui.small_button("-").clicked() && self.octave_offset > -2 {
                self.octave_offset -= 1;
                octave_change = -1;
            }
            if ui.small_button("+").clicked() && self.octave_offset < 4 {
                self.octave_offset += 1;
                octave_change = 1;
            }
        });
        octave_change
    }

    /// Draw the piano keys and handle interaction.
    /// The number of visible octaves is determined by available width.
    #[allow(clippy::too_many_lines)]
    pub fn show_keys(&mut self, ui: &mut egui::Ui) -> KeyboardEvent {
        let mut event = KeyboardEvent::default();

        let available = ui.available_size();
        let keyboard_height = 100.0;

        let white_key_width = WHITE_KEY_WIDTH;
        let white_key_height = keyboard_height - 10.0;
        let black_key_width = 14.0;
        let black_key_height = 55.0;

        // Calculate how many octaves fit
        let num_octaves = Self::octaves_for_width(available.x);

        // Center the visible range around middle C (C4 = MIDI 60)
        // Each octave starts at C, so we want roughly num_octaves/2 below C4
        let center_octave = 4i32; // C4
        let half = num_octaves as i32 / 2;
        let start_octave = (center_octave - half).max(1); // C1 minimum
        let end_octave = start_octave + num_octaves as i32; // exclusive

        // MIDI note range: C of start_octave to C of end_octave
        let low_note = ((start_octave + 1) * 12) as u8; // C1=24, C2=36, etc.
        let high_note = (((end_octave + 1) * 12) as u8).min(108); // cap at C8

        // Count white keys in our range
        let white_key_count: u32 = (low_note..=high_note)
            .filter(|&n| !Self::is_black_key(n))
            .count() as u32;

        let total_keyboard_width = white_key_count as f32 * white_key_width;

        // Allocate space (no extra padding — parent controls margins)
        let (outer_rect, _) =
            ui.allocate_exact_size(Vec2::new(available.x, keyboard_height), Sense::hover());
        let inner_rect = outer_rect;

        // Center the keyboard
        let center_offset = ((inner_rect.width() - total_keyboard_width) / 2.0).max(0.0);

        let (highlight_start, highlight_end) = self.get_highlight_range();

        let inner_response = ui.allocate_rect(inner_rect, Sense::click_and_drag());
        let hover_pos = inner_response.hover_pos();

        let painter = ui.painter();
        painter.rect_filled(outer_rect, 4.0, theme().colors.bg_dark);
        painter.with_clip_rect(inner_rect).rect_filled(
            inner_rect,
            2.0,
            Color32::from_rgb(15, 16, 20),
        );

        let mut clicked_note: Option<u8> = None;
        let mut hovered_black_key: Option<u8> = None;

        // Helper: x position of a note relative to low_note
        let key_x = |note: u8| -> f32 {
            let white_count: u32 =
                (low_note..note).filter(|&n| !Self::is_black_key(n)).count() as u32;

            if Self::is_black_key(note) {
                white_count as f32 * white_key_width - white_key_width * 0.3
                    + center_offset
                    + inner_rect.left()
            } else {
                white_count as f32 * white_key_width + center_offset + inner_rect.left()
            }
        };

        // First pass: detect black key hover (they have priority)
        for note in low_note..=high_note {
            if !Self::is_black_key(note) {
                continue;
            }
            let kx = key_x(note);
            let key_rect = Rect::from_min_size(
                Pos2::new(kx, inner_rect.top() + 2.0),
                Vec2::new(black_key_width, black_key_height),
            );
            if let Some(pos) = hover_pos
                && key_rect.contains(pos)
            {
                hovered_black_key = Some(note);
            }
        }

        // Draw white keys
        for note in low_note..=high_note {
            if Self::is_black_key(note) {
                continue;
            }

            let kx = key_x(note);
            let key_rect = Rect::from_min_size(
                Pos2::new(kx, inner_rect.top() + 2.0),
                Vec2::new(white_key_width - 1.0, white_key_height),
            );

            let velocity = self.pressed_keys.get(&note).copied();
            let is_in_highlight = note >= highlight_start && note < highlight_end;
            let is_hovered = hover_pos
                .map(|p| key_rect.contains(p) && hovered_black_key.is_none())
                .unwrap_or(false);

            let fill = if let Some(vel) = velocity {
                let intensity = 0.4 + (0.6 * vel);
                theme().colors.accent_orange.gamma_multiply(intensity)
            } else if is_hovered {
                Color32::from_rgb(230, 230, 235)
            } else if is_in_highlight {
                Color32::from_rgb(255, 245, 230)
            } else {
                Color32::from_rgb(250, 250, 252)
            };

            painter
                .with_clip_rect(inner_rect)
                .rect_filled(key_rect, 2.0, fill);

            if is_in_highlight {
                let indicator_rect = Rect::from_min_size(
                    Pos2::new(kx, inner_rect.top() + white_key_height - 4.0),
                    Vec2::new(white_key_width - 1.0, 4.0),
                );
                painter.with_clip_rect(inner_rect).rect_filled(
                    indicator_rect,
                    0.0,
                    theme().colors.accent_orange.gamma_multiply(0.7),
                );
            }

            painter.with_clip_rect(inner_rect).rect_stroke(
                key_rect,
                2.0,
                Stroke::new(1.0, Color32::from_rgb(150, 150, 155)),
                egui::StrokeKind::Inside,
            );

            // Note name on C keys
            if note % 12 == 0 {
                let octave = (note / 12) as i32 - 1;
                let label = format!("C{octave}");
                let text_pos = Pos2::new(kx + 3.0, inner_rect.top() + white_key_height - 16.0);
                painter.with_clip_rect(inner_rect).text(
                    text_pos,
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(9.0),
                    Color32::from_rgb(100, 100, 110),
                );
            }

            if is_hovered && inner_response.is_pointer_button_down_on() {
                clicked_note = Some(note);
            }
        }

        // Draw black keys
        for note in low_note..=high_note {
            if !Self::is_black_key(note) {
                continue;
            }

            let kx = key_x(note);
            let key_rect = Rect::from_min_size(
                Pos2::new(kx, inner_rect.top() + 2.0),
                Vec2::new(black_key_width, black_key_height),
            );

            let velocity = self.pressed_keys.get(&note).copied();
            let is_in_highlight = note >= highlight_start && note < highlight_end;
            let is_hovered = hover_pos.map(|p| key_rect.contains(p)).unwrap_or(false);

            let fill = if let Some(vel) = velocity {
                let intensity = 0.4 + (0.6 * vel);
                theme().colors.accent_orange.gamma_multiply(intensity)
            } else if is_hovered {
                Color32::from_rgb(60, 60, 65)
            } else if is_in_highlight {
                Color32::from_rgb(50, 45, 40)
            } else {
                Color32::from_rgb(30, 32, 38)
            };

            painter
                .with_clip_rect(inner_rect)
                .rect_filled(key_rect, 2.0, fill);

            if is_in_highlight {
                let indicator_rect = Rect::from_min_size(
                    Pos2::new(kx, inner_rect.top() + black_key_height - 3.0),
                    Vec2::new(black_key_width, 3.0),
                );
                painter.with_clip_rect(inner_rect).rect_filled(
                    indicator_rect,
                    0.0,
                    theme().colors.accent_orange.gamma_multiply(0.7),
                );
            }

            if is_hovered && inner_response.is_pointer_button_down_on() {
                clicked_note = Some(note);
            }
        }

        // Handle mouse press/release
        if let Some(note) = clicked_note
            && !self.mouse_pressed_keys.get(&note).copied().unwrap_or(false)
        {
            self.mouse_pressed_keys.insert(note, true);
            self.pressed_keys.insert(note, 0.8);
            event.note_on = Some(MidiNote::new(note));
        }

        let should_release = inner_response.drag_stopped()
            || (!inner_response.hovered() && inner_response.clicked_elsewhere())
            || !inner_response.is_pointer_button_down_on();

        if should_release {
            for (&note, &pressed) in self.mouse_pressed_keys.iter() {
                if pressed {
                    event.note_off.push(MidiNote::new(note));
                }
            }
            for pressed in self.mouse_pressed_keys.values_mut() {
                *pressed = false;
            }
        }

        event
    }
}

impl Default for PianoKeyboard {
    fn default() -> Self {
        Self::new()
    }
}
