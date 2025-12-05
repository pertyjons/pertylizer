//! Global theme settings for the synthesizer GUI.
//!
//! This module provides centralized configuration for colors, fonts, sizes,
//! and spacing used throughout the UI. All widgets and GUI elements should
//! read from the global THEME instance.
//!
//! The theme can be modified at runtime for live customization.
//!
//! Uses parking_lot::RwLock for deadlock-free locking.

use eframe::egui::{Color32, FontId};
use parking_lot::RwLock;

/// Global theme instance - use `theme()` to read, `set_theme()` to modify.
static THEME: RwLock<Theme> = RwLock::new(Theme::dark());

/// Get a read reference to the current theme.
///
/// This uses parking_lot's RwLock which is:
/// - Deadlock-free (no poisoning)
/// - Faster than std::sync::RwLock
/// - Never panics on lock contention
pub fn theme() -> parking_lot::RwLockReadGuard<'static, Theme> {
    THEME.read()
}

/// Try to get a read reference without blocking.
/// Returns None if the lock is currently held for writing.
pub fn try_theme() -> Option<parking_lot::RwLockReadGuard<'static, Theme>> {
    THEME.try_read()
}

/// Modify the current theme.
pub fn with_theme_mut<F: FnOnce(&mut Theme)>(f: F) {
    let mut theme = THEME.write();
    f(&mut theme);
}

/// Set the entire theme.
pub fn set_theme(new_theme: Theme) {
    let mut theme = THEME.write();
    *theme = new_theme;
}

/// Complete theme configuration.
#[derive(Debug, Clone)]
pub struct Theme {
    pub colors: Colors,
    pub fonts: Fonts,
    pub sizes: Sizes,
    pub spacing: Spacing,
}

impl Theme {
    /// Default dark theme.
    pub const fn dark() -> Self {
        Self {
            colors: Colors::dark(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
        }
    }

    /// Light theme variant.
    pub const fn light() -> Self {
        Self {
            colors: Colors::light(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
        }
    }
}

/// Color palette.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    // Backgrounds
    pub bg_dark: Color32,
    pub bg_panel: Color32,
    pub bg_module: Color32,
    pub bg_widget: Color32,

    // Text
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_dim: Color32,

    // Accent colors
    pub accent_primary: Color32,
    pub accent_cyan: Color32,
    pub accent_green: Color32,
    pub accent_red: Color32,
    pub accent_purple: Color32,
    pub accent_yellow: Color32,
    pub accent_orange: Color32,

    // Meters
    pub meter_green: Color32,
    pub meter_yellow: Color32,
    pub meter_red: Color32,

    // Ports
    pub port_audio: Color32,
    pub port_control: Color32,
    pub port_gate: Color32,
    pub port_midi: Color32,

    // Cables
    pub cable_audio: Color32,
    pub cable_control: Color32,
    pub cable_gate: Color32,

    // Borders
    pub border: Color32,
    pub border_selected: Color32,
}

impl Colors {
    pub const fn dark() -> Self {
        Self {
            // Backgrounds
            bg_dark: Color32::from_rgb(20, 22, 28),
            bg_panel: Color32::from_rgb(32, 36, 44),
            bg_module: Color32::from_rgb(40, 44, 52),
            bg_widget: Color32::from_rgb(50, 55, 65),

            // Text
            text_primary: Color32::from_rgb(240, 240, 245),
            text_secondary: Color32::from_rgb(180, 180, 190),
            text_dim: Color32::from_rgb(120, 120, 130),

            // Accent colors
            accent_primary: Color32::from_rgb(255, 140, 50),
            accent_cyan: Color32::from_rgb(80, 200, 220),
            accent_green: Color32::from_rgb(100, 220, 100),
            accent_red: Color32::from_rgb(255, 80, 80),
            accent_purple: Color32::from_rgb(180, 100, 255),
            accent_yellow: Color32::from_rgb(255, 220, 80),
            accent_orange: Color32::from_rgb(255, 140, 50),

            // Meters
            meter_green: Color32::from_rgb(100, 220, 100),
            meter_yellow: Color32::from_rgb(255, 220, 80),
            meter_red: Color32::from_rgb(255, 80, 80),

            // Ports
            port_audio: Color32::from_rgb(100, 200, 255),
            port_control: Color32::from_rgb(255, 180, 80),
            port_gate: Color32::from_rgb(100, 255, 100),
            port_midi: Color32::from_rgb(255, 100, 255),

            // Cables
            cable_audio: Color32::from_rgb(80, 180, 230),
            cable_control: Color32::from_rgb(230, 160, 60),
            cable_gate: Color32::from_rgb(80, 230, 80),

            // Borders
            border: Color32::from_rgb(60, 65, 75),
            border_selected: Color32::from_rgb(100, 180, 255),
        }
    }

    pub const fn light() -> Self {
        Self {
            // Backgrounds - inverted/lighter
            bg_dark: Color32::from_rgb(240, 242, 245),
            bg_panel: Color32::from_rgb(225, 228, 235),
            bg_module: Color32::from_rgb(210, 215, 225),
            bg_widget: Color32::from_rgb(195, 200, 210),

            // Text - darker for light bg
            text_primary: Color32::from_rgb(30, 30, 35),
            text_secondary: Color32::from_rgb(70, 70, 80),
            text_dim: Color32::from_rgb(110, 110, 120),

            // Accent colors - slightly adjusted for light bg
            accent_primary: Color32::from_rgb(230, 120, 30),
            accent_cyan: Color32::from_rgb(50, 170, 190),
            accent_green: Color32::from_rgb(70, 180, 70),
            accent_red: Color32::from_rgb(220, 60, 60),
            accent_purple: Color32::from_rgb(150, 80, 220),
            accent_yellow: Color32::from_rgb(220, 180, 40),
            accent_orange: Color32::from_rgb(230, 120, 30),

            // Meters
            meter_green: Color32::from_rgb(70, 180, 70),
            meter_yellow: Color32::from_rgb(220, 180, 40),
            meter_red: Color32::from_rgb(220, 60, 60),

            // Ports
            port_audio: Color32::from_rgb(70, 160, 220),
            port_control: Color32::from_rgb(220, 150, 50),
            port_gate: Color32::from_rgb(70, 200, 70),
            port_midi: Color32::from_rgb(200, 70, 200),

            // Cables
            cable_audio: Color32::from_rgb(60, 150, 200),
            cable_control: Color32::from_rgb(200, 140, 40),
            cable_gate: Color32::from_rgb(60, 180, 60),

            // Borders
            border: Color32::from_rgb(150, 155, 165),
            border_selected: Color32::from_rgb(70, 140, 220),
        }
    }
}

/// Font settings.
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    /// Large font size (titles, headings).
    pub size_large: f32,
    /// Normal font size (labels, parameters).
    pub size_normal: f32,
    /// Small font size (values, hints, port labels).
    pub size_small: f32,
}

impl Fonts {
    pub const fn default_fonts() -> Self {
        Self {
            size_large: 24.0,
            size_normal: 14.0,
            size_small: 10.0,
        }
    }

    /// Get a FontId for large text (titles, headings).
    pub fn large(&self) -> FontId {
        FontId::proportional(self.size_large)
    }

    /// Get a FontId for normal text (labels, parameters).
    pub fn normal(&self) -> FontId {
        FontId::proportional(self.size_normal)
    }

    /// Get a FontId for small text (values, hints, port labels).
    pub fn small(&self) -> FontId {
        FontId::proportional(self.size_small)
    }
}

/// Size settings for widgets.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    /// Default knob diameter.
    pub knob_size: f32,
    /// Small knob diameter.
    pub knob_size_small: f32,
    /// Large knob diameter.
    pub knob_size_large: f32,
    /// Height reserved for knob label.
    pub knob_label_height: f32,

    /// Port circle diameter.
    pub port_size: f32,

    /// Module minimum width.
    pub module_min_width: f32,
    /// Module minimum height.
    pub module_min_height: f32,

    /// Meter width.
    pub meter_width: f32,
    /// Meter height.
    pub meter_height: f32,

    /// Oscilloscope default width.
    pub oscilloscope_width: f32,
    /// Oscilloscope default height.
    pub oscilloscope_height: f32,

    /// ADSR curve width.
    pub adsr_width: f32,
    /// ADSR curve height.
    pub adsr_height: f32,

    /// Cable thickness.
    pub cable_thickness: f32,
}

impl Sizes {
    pub const fn default_sizes() -> Self {
        Self {
            knob_size: 72.0,
            knob_size_small: 56.0,
            knob_size_large: 88.0,
            knob_label_height: 24.0,

            port_size: 12.0,

            module_min_width: 180.0,
            module_min_height: 100.0,

            meter_width: 80.0,
            meter_height: 100.0,

            oscilloscope_width: 160.0,
            oscilloscope_height: 80.0,

            adsr_width: 140.0,
            adsr_height: 50.0,

            cable_thickness: 2.5,
        }
    }
}

/// Spacing and padding settings.
#[derive(Debug, Clone, Copy)]
pub struct Spacing {
    /// Default padding inside panels.
    pub panel_padding: f32,
    /// Space between widgets.
    pub widget_spacing: f32,
    /// Space between knobs.
    pub knob_spacing: f32,
    /// Space between sections.
    pub section_spacing: f32,
    /// Space between label and widget.
    pub label_spacing: f32,
    /// Module grid spacing for auto-layout.
    pub module_grid_x: f32,
    pub module_grid_y: f32,
}

impl Spacing {
    pub const fn default_spacing() -> Self {
        Self {
            panel_padding: 8.0,
            widget_spacing: 4.0,
            knob_spacing: 6.0,
            section_spacing: 12.0,
            label_spacing: 4.0,
            module_grid_x: 210.0,
            module_grid_y: 320.0,
        }
    }
}

// ============================================================================
// Legacy compatibility - re-export colors as constants for gradual migration
// ============================================================================

/// Legacy color constants for backwards compatibility.
/// Prefer using `theme().colors.xxx` for new code.
pub mod colors {
    use super::Color32;

    // These delegate to the dark theme defaults for const compatibility
    pub const BG_DARK: Color32 = Color32::from_rgb(20, 22, 28);
    pub const BG_PANEL: Color32 = Color32::from_rgb(32, 36, 44);
    pub const BG_MODULE: Color32 = Color32::from_rgb(40, 44, 52);
    pub const BG_WIDGET: Color32 = Color32::from_rgb(50, 55, 65);

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(240, 240, 245);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(180, 180, 190);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(120, 120, 130);

    pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 140, 50);
    pub const ACCENT_CYAN: Color32 = Color32::from_rgb(80, 200, 220);
    pub const ACCENT_GREEN: Color32 = Color32::from_rgb(100, 220, 100);
    pub const ACCENT_RED: Color32 = Color32::from_rgb(255, 80, 80);
    pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(180, 100, 255);
    pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(255, 220, 80);

    pub const METER_GREEN: Color32 = Color32::from_rgb(100, 220, 100);
    pub const METER_YELLOW: Color32 = Color32::from_rgb(255, 220, 80);
    pub const METER_RED: Color32 = Color32::from_rgb(255, 80, 80);

    pub const PORT_AUDIO: Color32 = Color32::from_rgb(100, 200, 255);
    pub const PORT_CONTROL: Color32 = Color32::from_rgb(255, 180, 80);
    pub const PORT_GATE: Color32 = Color32::from_rgb(100, 255, 100);
    pub const PORT_MIDI: Color32 = Color32::from_rgb(255, 100, 255);

    pub const CABLE_AUDIO: Color32 = Color32::from_rgb(80, 180, 230);
    pub const CABLE_CONTROL: Color32 = Color32::from_rgb(230, 160, 60);
    pub const CABLE_GATE: Color32 = Color32::from_rgb(80, 230, 80);

    pub const BORDER: Color32 = Color32::from_rgb(60, 65, 75);
}
