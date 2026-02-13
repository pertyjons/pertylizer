//! Piano keyboard component for the modular synthesizer.
//!
//! Provides an 88-key piano keyboard (A0-C8) that can be played with mouse
//! and shows the currently active octave range for computer keyboard input.

use crate::gui::theme::theme;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;
use synth_core::{MidiNote, Velocity};

/// MIDI note number for A0 (lowest note on 88-key piano)
pub const PIANO_LOW_NOTE: u8 = MidiNote::A0.as_u8();
/// MIDI note number for C8 (highest note on 88-key piano)
pub const PIANO_HIGH_NOTE: u8 = MidiNote::C8.as_u8();
/// Total number of keys on the piano
pub const TOTAL_KEYS: u8 = PIANO_HIGH_NOTE - PIANO_LOW_NOTE + 1;

/// Result of keyboard interaction
#[derive(Debug, Clone, Default)]
pub struct KeyboardEvent {
    /// Note that was pressed (MidiNote for type safety)
    pub note_on: Option<MidiNote>,
    /// Notes that were released
    pub note_off: Vec<MidiNote>,
    pub octave_change: i32,
}

/// Piano keyboard widget with 88 keys
pub struct PianoKeyboard {
    /// Currently pressed keys (raw MIDI note -> velocity 0.0-1.0)
    /// Uses u8 internally for efficient HashMap operations
    pressed_keys: HashMap<u8, f32>,
    /// Keys pressed by mouse (need to track separately for release)
    mouse_pressed_keys: HashMap<u8, bool>,
    /// Current octave offset for computer keyboard (-2 to +4)
    octave_offset: i32,
    /// Scroll position (horizontal offset in pixels)
    scroll_offset: f32,
}

impl PianoKeyboard {
    /// Create a new piano keyboard
    pub fn new() -> Self {
        Self {
            pressed_keys: HashMap::new(),
            mouse_pressed_keys: HashMap::new(),
            octave_offset: -1,    // Start at C3 range
            scroll_offset: 300.0, // Start scrolled to show middle C area
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

    /// Returns true if the given MIDI note is a black key
    fn is_black_key(note: u8) -> bool {
        matches!(note % 12, 1 | 3 | 6 | 8 | 10)
    }

    /// Get the x position of a key relative to the start of the keyboard
    fn key_x_position(note: u8, white_key_width: f32) -> f32 {
        // Count white keys from A0 (note 21) to this note
        let mut white_count = 0;
        for n in PIANO_LOW_NOTE..note {
            if !Self::is_black_key(n) {
                white_count += 1;
            }
        }

        if Self::is_black_key(note) {
            // Black key: position between adjacent white keys
            let prev_white_x = white_count as f32 * white_key_width;
            prev_white_x - white_key_width * 0.3
        } else {
            // White key
            white_count as f32 * white_key_width
        }
    }

    /// Get the octave highlight range (start note, end note) based on octave offset
    fn get_highlight_range(&self) -> (u8, u8) {
        // Computer keyboard maps to 2 octaves starting at base_note
        let base_note = (48i32 + self.octave_offset * 12).clamp(0, 127) as u8;
        let end_note = (base_note as u16 + 24).min(127) as u8;
        (base_note, end_note)
    }

    /// Draw the piano keyboard and handle interaction
    /// Returns events that occurred (note on/off, octave changes)
    pub fn show(&mut self, ui: &mut egui::Ui) -> KeyboardEvent {
        let mut event = KeyboardEvent::default();

        // Header with controls
        ui.horizontal(|ui| {
            ui.label(RichText::new("KEYBOARD").color(theme().colors.text_dim));
            ui.separator();

            // Octave display and controls
            ui.label(
                RichText::new(format!("Octave: {:+}", self.octave_offset))
                    .color(theme().colors.text_secondary),
            );
            if ui.small_button("-").clicked() && self.octave_offset > -2 {
                self.octave_offset -= 1;
                event.octave_change = -1;
            }
            if ui.small_button("+").clicked() && self.octave_offset < 4 {
                self.octave_offset += 1;
                event.octave_change = 1;
            }

            ui.separator();

            // Scroll to highlight button
            if ui
                .small_button("Center")
                .on_hover_text("Scroll to active octave")
                .clicked()
            {
                let (start, _) = self.get_highlight_range();
                // Use a reasonable key width for centering (actual width calculated below)
                self.scroll_offset = Self::key_x_position(start, 24.0) - 50.0;
            }
        });

        ui.add_space(4.0);

        // Piano keyboard with horizontal scrolling
        let available = ui.available_size();
        let keyboard_height = 100.0;

        // Count total white keys
        let total_white_keys: u32 = (PIANO_LOW_NOTE..=PIANO_HIGH_NOTE)
            .filter(|&n| !Self::is_black_key(n))
            .count() as u32;

        // Key dimensions - scale to fill available width, with min size
        let inner_width = available.x - 10.0; // account for shrink(5.0) padding
        let min_key_width = 14.0;
        let white_key_width = (inner_width / total_white_keys as f32).max(min_key_width);
        let white_key_height = keyboard_height - 10.0;
        let black_key_width = white_key_width * 0.583; // proportional (14/24)
        let black_key_height = 55.0;

        let total_keyboard_width = total_white_keys as f32 * white_key_width;
        let fits_all = total_keyboard_width <= inner_width;

        // Center offset when all keys fit
        let center_offset = if fits_all {
            (inner_width - total_keyboard_width) / 2.0
        } else {
            0.0
        };

        // Allocate space for the keyboard
        let (outer_rect, _outer_response) =
            ui.allocate_exact_size(Vec2::new(available.x, keyboard_height), Sense::hover());

        // Clamp scroll offset (no scroll when all keys fit)
        let max_scroll = if fits_all {
            0.0
        } else {
            (total_keyboard_width - outer_rect.width() + 20.0).max(0.0)
        };
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);

        // Create a clip rect for the keyboard area
        let inner_rect = outer_rect.shrink(5.0);

        // Handle scroll with mouse wheel (only when not all keys fit)
        if !fits_all && ui.rect_contains_pointer(outer_rect) {
            ui.input(|input| {
                let scroll_delta = input.smooth_scroll_delta.x - input.smooth_scroll_delta.y * 2.0;
                self.scroll_offset = (self.scroll_offset - scroll_delta).clamp(0.0, max_scroll);
            });
        }

        // Get highlight range for active octave
        let (highlight_start, highlight_end) = self.get_highlight_range();

        // Allocate response for the inner area for click detection
        let inner_response = ui.allocate_rect(inner_rect, Sense::click_and_drag());
        let hover_pos = inner_response.hover_pos();

        // Now get painter after all allocations are done
        let painter = ui.painter();
        painter.rect_filled(outer_rect, 4.0, theme().colors.bg_dark);
        painter.with_clip_rect(inner_rect).rect_filled(
            inner_rect,
            2.0,
            Color32::from_rgb(15, 16, 20),
        );

        // Store which note was clicked this frame
        let mut clicked_note: Option<u8> = None;
        let mut hovered_black_key: Option<u8> = None;

        // First pass: detect black key hover (they have priority)
        for note in PIANO_LOW_NOTE..=PIANO_HIGH_NOTE {
            if !Self::is_black_key(note) {
                continue;
            }

            let key_x = Self::key_x_position(note, white_key_width) - self.scroll_offset
                + center_offset
                + inner_rect.left();
            let key_rect = Rect::from_min_size(
                Pos2::new(key_x, inner_rect.top() + 2.0),
                Vec2::new(black_key_width, black_key_height),
            );

            if let Some(pos) = hover_pos
                && key_rect.contains(pos)
            {
                hovered_black_key = Some(note);
            }
        }

        // Draw white keys first (so black keys are on top)
        for note in PIANO_LOW_NOTE..=PIANO_HIGH_NOTE {
            if Self::is_black_key(note) {
                continue;
            }

            let key_x = Self::key_x_position(note, white_key_width) - self.scroll_offset
                + center_offset
                + inner_rect.left();

            // Skip keys outside visible area
            if key_x + white_key_width < inner_rect.left() || key_x > inner_rect.right() {
                continue;
            }

            let key_rect = Rect::from_min_size(
                Pos2::new(key_x, inner_rect.top() + 2.0),
                Vec2::new(white_key_width - 1.0, white_key_height),
            );

            let velocity = self.pressed_keys.get(&note).copied();
            let is_in_highlight = note >= highlight_start && note < highlight_end;
            let is_hovered = hover_pos
                .map(|p| key_rect.contains(p) && hovered_black_key.is_none())
                .unwrap_or(false);

            // Determine fill color (velocity modulates brightness)
            let fill = if let Some(vel) = velocity {
                // Intensity: 0.4 base + 0.6 * velocity (soft notes visible, hard notes bright)
                let intensity = 0.4 + (0.6 * vel);
                theme().colors.accent_orange.gamma_multiply(intensity)
            } else if is_hovered {
                Color32::from_rgb(230, 230, 235)
            } else if is_in_highlight {
                Color32::from_rgb(255, 245, 230) // Slight orange tint for highlight
            } else {
                Color32::from_rgb(250, 250, 252)
            };

            // Draw the key
            painter
                .with_clip_rect(inner_rect)
                .rect_filled(key_rect, 2.0, fill);

            // Draw highlight indicator line at bottom for active octave
            if is_in_highlight {
                let indicator_rect = Rect::from_min_size(
                    Pos2::new(key_x, inner_rect.top() + white_key_height - 4.0),
                    Vec2::new(white_key_width - 1.0, 4.0),
                );
                painter.with_clip_rect(inner_rect).rect_filled(
                    indicator_rect,
                    0.0,
                    theme().colors.accent_orange.gamma_multiply(0.7),
                );
            }

            // Draw key border
            painter.with_clip_rect(inner_rect).rect_stroke(
                key_rect,
                2.0,
                Stroke::new(1.0, Color32::from_rgb(150, 150, 155)),
                egui::StrokeKind::Inside,
            );

            // Draw note name on C keys
            if note % 12 == 0 {
                let octave = (note / 12) as i32 - 1;
                let label = format!("C{}", octave);
                let text_pos = Pos2::new(key_x + 3.0, inner_rect.top() + white_key_height - 16.0);
                painter.with_clip_rect(inner_rect).text(
                    text_pos,
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(9.0),
                    Color32::from_rgb(100, 100, 110),
                );
            }

            // Handle click
            if is_hovered && inner_response.is_pointer_button_down_on() {
                clicked_note = Some(note);
            }
        }

        // Draw black keys on top
        for note in PIANO_LOW_NOTE..=PIANO_HIGH_NOTE {
            if !Self::is_black_key(note) {
                continue;
            }

            let key_x = Self::key_x_position(note, white_key_width) - self.scroll_offset
                + center_offset
                + inner_rect.left();

            // Skip keys outside visible area
            if key_x + black_key_width < inner_rect.left() || key_x > inner_rect.right() {
                continue;
            }

            let key_rect = Rect::from_min_size(
                Pos2::new(key_x, inner_rect.top() + 2.0),
                Vec2::new(black_key_width, black_key_height),
            );

            let velocity = self.pressed_keys.get(&note).copied();
            let is_in_highlight = note >= highlight_start && note < highlight_end;
            let is_hovered = hover_pos.map(|p| key_rect.contains(p)).unwrap_or(false);

            // Determine fill color (velocity modulates brightness)
            let fill = if let Some(vel) = velocity {
                // Intensity: 0.4 base + 0.6 * velocity (soft notes visible, hard notes bright)
                let intensity = 0.4 + (0.6 * vel);
                theme().colors.accent_orange.gamma_multiply(intensity)
            } else if is_hovered {
                Color32::from_rgb(60, 60, 65)
            } else if is_in_highlight {
                Color32::from_rgb(50, 45, 40) // Slight orange tint for highlight
            } else {
                Color32::from_rgb(30, 32, 38)
            };

            // Draw the key
            painter
                .with_clip_rect(inner_rect)
                .rect_filled(key_rect, 2.0, fill);

            // Draw highlight indicator line at bottom for active octave
            if is_in_highlight {
                let indicator_rect = Rect::from_min_size(
                    Pos2::new(key_x, inner_rect.top() + black_key_height - 3.0),
                    Vec2::new(black_key_width, 3.0),
                );
                painter.with_clip_rect(inner_rect).rect_filled(
                    indicator_rect,
                    0.0,
                    theme().colors.accent_orange.gamma_multiply(0.7),
                );
            }

            // Handle click
            if is_hovered && inner_response.is_pointer_button_down_on() {
                clicked_note = Some(note);
            }
        }

        // Handle mouse press/release
        if let Some(note) = clicked_note
            && !self.mouse_pressed_keys.get(&note).copied().unwrap_or(false)
        {
            // New note pressed (use default velocity 0.8 for mouse clicks)
            self.mouse_pressed_keys.insert(note, true);
            self.pressed_keys.insert(note, 0.8);
            event.note_on = Some(MidiNote::new(note));
        }

        // Release notes when mouse is released or leaves the keyboard
        if inner_response.drag_stopped()
            || (!inner_response.hovered() && inner_response.clicked_elsewhere())
        {
            for (&note, &pressed) in self.mouse_pressed_keys.iter() {
                if pressed {
                    event.note_off.push(MidiNote::new(note));
                }
            }
            // Clear mouse pressed state but keep the visual pressed state
            // (it will be managed by the external pressed_keys updates)
            for pressed in self.mouse_pressed_keys.values_mut() {
                *pressed = false;
            }
        } else if !inner_response.is_pointer_button_down_on() {
            // Mouse button released while still over keyboard
            for (&note, &pressed) in self.mouse_pressed_keys.iter() {
                if pressed {
                    event.note_off.push(MidiNote::new(note));
                }
            }
            for pressed in self.mouse_pressed_keys.values_mut() {
                *pressed = false;
            }
        }

        // Draw scroll indicators if not at edges (only when scrolling is needed)
        if !fits_all && self.scroll_offset > 0.0 {
            // Left arrow indicator
            let arrow_rect = Rect::from_min_size(
                outer_rect.left_top() + Vec2::new(2.0, keyboard_height / 2.0 - 10.0),
                Vec2::new(15.0, 20.0),
            );
            painter.rect_filled(arrow_rect, 3.0, theme().colors.bg_panel.gamma_multiply(0.8));
            painter.text(
                arrow_rect.center(),
                egui::Align2::CENTER_CENTER,
                "◀",
                egui::FontId::proportional(12.0),
                theme().colors.text_dim,
            );
        }

        if !fits_all && self.scroll_offset < max_scroll {
            // Right arrow indicator
            let arrow_rect = Rect::from_min_size(
                outer_rect.right_top() + Vec2::new(-17.0, keyboard_height / 2.0 - 10.0),
                Vec2::new(15.0, 20.0),
            );
            painter.rect_filled(arrow_rect, 3.0, theme().colors.bg_panel.gamma_multiply(0.8));
            painter.text(
                arrow_rect.center(),
                egui::Align2::CENTER_CENTER,
                "▶",
                egui::FontId::proportional(12.0),
                theme().colors.text_dim,
            );
        }

        event
    }
}

impl Default for PianoKeyboard {
    fn default() -> Self {
        Self::new()
    }
}
