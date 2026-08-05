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
    pub style: WidgetStyle,
}

impl Theme {
    /// Default dark theme - synth-focused.
    pub const fn dark() -> Self {
        Self {
            colors: Colors::dark(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Light theme variant.
    pub const fn light() -> Self {
        Self {
            colors: Colors::light(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Vintage theme - Moog/ARP-inspired warm tones.
    pub const fn vintage() -> Self {
        Self {
            colors: Colors::vintage(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Neon theme - 80s synthwave style.
    pub const fn neon() -> Self {
        Self {
            colors: Colors::neon(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Studio theme - professional, neutral.
    pub const fn studio() -> Self {
        Self {
            colors: Colors::studio(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Dracula theme - popular dark theme.
    pub const fn dracula() -> Self {
        Self {
            colors: Colors::dracula(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Monokai theme - classic from Sublime.
    pub const fn monokai() -> Self {
        Self {
            colors: Colors::monokai(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }

    /// Solarized Dark theme.
    pub const fn solarized_dark() -> Self {
        Self {
            colors: Colors::solarized_dark(),
            fonts: Fonts::default_fonts(),
            sizes: Sizes::default_sizes(),
            spacing: Spacing::default_spacing(),
            style: WidgetStyle::default_style(),
        }
    }
}

/// Available theme presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemePreset {
    /// Default dark synth theme with orange accents.
    #[default]
    Dark,
    /// Light theme for well-lit environments.
    Light,
    /// Vintage Moog/ARP-inspired warm tones.
    Vintage,
    /// 80s synthwave neon colors.
    Neon,
    /// Professional neutral studio theme.
    Studio,
    /// Popular Dracula theme with purple accents.
    Dracula,
    /// Classic Monokai from Sublime Text.
    Monokai,
    /// Solarized Dark - low contrast classic.
    SolarizedDark,
}

impl ThemePreset {
    /// All available theme presets.
    pub const ALL: &'static [Self] = &[
        Self::Dark,
        Self::Light,
        Self::Vintage,
        Self::Neon,
        Self::Studio,
        Self::Dracula,
        Self::Monokai,
        Self::SolarizedDark,
    ];

    /// Get the display name for this preset.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Vintage => "Vintage",
            Self::Neon => "Neon",
            Self::Studio => "Studio",
            Self::Dracula => "Dracula",
            Self::Monokai => "Monokai",
            Self::SolarizedDark => "Solarized Dark",
        }
    }

    /// Get the Theme for this preset.
    pub const fn theme(&self) -> Theme {
        match self {
            Self::Dark => Theme::dark(),
            Self::Light => Theme::light(),
            Self::Vintage => Theme::vintage(),
            Self::Neon => Theme::neon(),
            Self::Studio => Theme::studio(),
            Self::Dracula => Theme::dracula(),
            Self::Monokai => Theme::monokai(),
            Self::SolarizedDark => Theme::solarized_dark(),
        }
    }

    /// Apply this preset as the current theme.
    pub fn apply(&self) {
        set_theme(self.theme());
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
    /// Whether this is a dark palette (light text on a dark background).
    ///
    /// Derived from the panel background's perceived luminance instead of a
    /// hand-maintained flag, so a new preset cannot forget to declare itself.
    /// egui has to be told this truthfully: it drives `Visuals::dark_mode` and
    /// the [`egui::Theme`] we pin as the theme preference, which in turn decides
    /// the colour of the OS window decorations.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        // Rec. 601 luma, scaled by 1000 to stay in integer arithmetic.
        let c = self.bg_panel;
        let luma = 299 * u32::from(c.r()) + 587 * u32::from(c.g()) + 114 * u32::from(c.b());
        luma < 128 * 1000
    }

    /// Default dark theme - synth-focused with deep backgrounds and warm accents.
    pub const fn dark() -> Self {
        Self {
            // Backgrounds - darker, more of a "rack" feel
            bg_dark: Color32::from_rgb(12, 12, 16),
            bg_panel: Color32::from_rgb(22, 24, 28),
            bg_module: Color32::from_rgb(32, 34, 38),
            bg_widget: Color32::from_rgb(42, 44, 50),

            // Text - varm vit
            text_primary: Color32::from_rgb(230, 225, 220),
            text_secondary: Color32::from_rgb(160, 155, 150),
            text_dim: Color32::from_rgb(90, 88, 85),

            // Accent - klassiska synth-färger
            accent_primary: Color32::from_rgb(255, 100, 50), // Moog-orange
            accent_cyan: Color32::from_rgb(0, 255, 200),     // Teal/cyan LED
            accent_green: Color32::from_rgb(50, 255, 100),   // Grön LED
            accent_red: Color32::from_rgb(255, 50, 50),      // Röd LED
            accent_purple: Color32::from_rgb(200, 100, 255), // Lila
            accent_yellow: Color32::from_rgb(255, 200, 50),  // Gul LED
            accent_orange: Color32::from_rgb(255, 120, 30),  // Orange

            // Meters - VU-meter stil
            meter_green: Color32::from_rgb(50, 220, 80),
            meter_yellow: Color32::from_rgb(255, 200, 50),
            meter_red: Color32::from_rgb(255, 50, 50),

            // Ports - tydliga signaltyper
            port_audio: Color32::from_rgb(60, 180, 255), // Blå = audio
            port_control: Color32::from_rgb(255, 160, 60), // Orange = CV
            port_gate: Color32::from_rgb(50, 255, 120),  // Grön = gate/trigger
            port_midi: Color32::from_rgb(255, 80, 200),  // Rosa = MIDI

            // Cables - mättade
            cable_audio: Color32::from_rgb(80, 160, 255),
            cable_control: Color32::from_rgb(255, 140, 40),
            cable_gate: Color32::from_rgb(60, 230, 100),

            // Borders
            border: Color32::from_rgb(50, 52, 58),
            border_selected: Color32::from_rgb(255, 140, 60), // Orange markering
        }
    }

    /// Light theme variant.
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

    /// Vintage theme - Moog/ARP-inspired warm tones.
    pub const fn vintage() -> Self {
        Self {
            bg_dark: Color32::from_rgb(25, 22, 18),
            bg_panel: Color32::from_rgb(35, 32, 28),
            bg_module: Color32::from_rgb(50, 45, 38),
            bg_widget: Color32::from_rgb(65, 58, 48),

            text_primary: Color32::from_rgb(230, 220, 200),
            text_secondary: Color32::from_rgb(180, 170, 150),
            text_dim: Color32::from_rgb(120, 110, 95),

            accent_primary: Color32::from_rgb(255, 90, 30),
            accent_cyan: Color32::from_rgb(100, 180, 160),
            accent_green: Color32::from_rgb(80, 200, 80),
            accent_red: Color32::from_rgb(230, 60, 50),
            accent_purple: Color32::from_rgb(160, 100, 180),
            accent_yellow: Color32::from_rgb(240, 190, 50),
            accent_orange: Color32::from_rgb(255, 120, 40),

            meter_green: Color32::from_rgb(80, 200, 80),
            meter_yellow: Color32::from_rgb(240, 190, 50),
            meter_red: Color32::from_rgb(230, 60, 50),

            port_audio: Color32::from_rgb(80, 160, 220),
            port_control: Color32::from_rgb(240, 160, 60),
            port_gate: Color32::from_rgb(80, 220, 100),
            port_midi: Color32::from_rgb(220, 100, 180),

            cable_audio: Color32::from_rgb(70, 150, 200),
            cable_control: Color32::from_rgb(220, 140, 50),
            cable_gate: Color32::from_rgb(70, 200, 90),

            border: Color32::from_rgb(55, 50, 42),
            border_selected: Color32::from_rgb(255, 130, 50),
        }
    }

    /// Neon theme - 80s synthwave style.
    pub const fn neon() -> Self {
        Self {
            bg_dark: Color32::from_rgb(10, 8, 24),
            bg_panel: Color32::from_rgb(18, 14, 36),
            bg_module: Color32::from_rgb(28, 22, 52),
            bg_widget: Color32::from_rgb(40, 32, 68),

            text_primary: Color32::from_rgb(240, 230, 255),
            text_secondary: Color32::from_rgb(180, 170, 210),
            text_dim: Color32::from_rgb(110, 100, 140),

            accent_primary: Color32::from_rgb(255, 0, 150), // Hot pink
            accent_cyan: Color32::from_rgb(0, 255, 255),    // Cyan
            accent_green: Color32::from_rgb(0, 255, 128),   // Neon green
            accent_red: Color32::from_rgb(255, 50, 80),
            accent_purple: Color32::from_rgb(180, 0, 255), // Purple
            accent_yellow: Color32::from_rgb(255, 255, 0),
            accent_orange: Color32::from_rgb(255, 100, 0),

            meter_green: Color32::from_rgb(0, 255, 128),
            meter_yellow: Color32::from_rgb(255, 255, 0),
            meter_red: Color32::from_rgb(255, 50, 80),

            port_audio: Color32::from_rgb(0, 200, 255),
            port_control: Color32::from_rgb(255, 100, 0),
            port_gate: Color32::from_rgb(0, 255, 128),
            port_midi: Color32::from_rgb(255, 0, 200),

            cable_audio: Color32::from_rgb(0, 180, 230),
            cable_control: Color32::from_rgb(255, 80, 0),
            cable_gate: Color32::from_rgb(0, 230, 110),

            border: Color32::from_rgb(50, 40, 80),
            border_selected: Color32::from_rgb(255, 0, 200),
        }
    }

    /// Studio theme - professional, neutral colors.
    pub const fn studio() -> Self {
        Self {
            bg_dark: Color32::from_rgb(32, 32, 34),
            bg_panel: Color32::from_rgb(42, 42, 45),
            bg_module: Color32::from_rgb(52, 52, 56),
            bg_widget: Color32::from_rgb(62, 62, 68),

            text_primary: Color32::from_rgb(210, 210, 215),
            text_secondary: Color32::from_rgb(150, 150, 158),
            text_dim: Color32::from_rgb(100, 100, 108),

            accent_primary: Color32::from_rgb(90, 160, 230), // Blue
            accent_cyan: Color32::from_rgb(80, 200, 200),
            accent_green: Color32::from_rgb(80, 200, 100),
            accent_red: Color32::from_rgb(230, 80, 80),
            accent_purple: Color32::from_rgb(160, 100, 220),
            accent_yellow: Color32::from_rgb(230, 200, 80),
            accent_orange: Color32::from_rgb(230, 140, 60),

            meter_green: Color32::from_rgb(80, 200, 100),
            meter_yellow: Color32::from_rgb(230, 200, 80),
            meter_red: Color32::from_rgb(230, 80, 80),

            port_audio: Color32::from_rgb(80, 160, 230),
            port_control: Color32::from_rgb(230, 160, 60),
            port_gate: Color32::from_rgb(80, 200, 100),
            port_midi: Color32::from_rgb(200, 100, 200),

            cable_audio: Color32::from_rgb(70, 150, 210),
            cable_control: Color32::from_rgb(210, 140, 50),
            cable_gate: Color32::from_rgb(70, 180, 90),

            border: Color32::from_rgb(55, 55, 60),
            border_selected: Color32::from_rgb(100, 170, 240),
        }
    }

    /// Dracula theme - popular dark theme with purple accents.
    pub const fn dracula() -> Self {
        Self {
            bg_dark: Color32::from_rgb(40, 42, 54), // #282a36
            bg_panel: Color32::from_rgb(50, 52, 66),
            bg_module: Color32::from_rgb(60, 62, 78),
            bg_widget: Color32::from_rgb(68, 71, 90), // #44475a

            text_primary: Color32::from_rgb(248, 248, 242), // #f8f8f2
            text_secondary: Color32::from_rgb(200, 200, 195),
            text_dim: Color32::from_rgb(140, 140, 135),

            accent_primary: Color32::from_rgb(255, 121, 198), // #ff79c6 pink
            accent_cyan: Color32::from_rgb(139, 233, 253),    // #8be9fd
            accent_green: Color32::from_rgb(80, 250, 123),    // #50fa7b
            accent_red: Color32::from_rgb(255, 85, 85),       // #ff5555
            accent_purple: Color32::from_rgb(189, 147, 249),  // #bd93f9
            accent_yellow: Color32::from_rgb(241, 250, 140),  // #f1fa8c
            accent_orange: Color32::from_rgb(255, 184, 108),  // #ffb86c

            meter_green: Color32::from_rgb(80, 250, 123),
            meter_yellow: Color32::from_rgb(241, 250, 140),
            meter_red: Color32::from_rgb(255, 85, 85),

            port_audio: Color32::from_rgb(139, 233, 253),
            port_control: Color32::from_rgb(255, 184, 108),
            port_gate: Color32::from_rgb(80, 250, 123),
            port_midi: Color32::from_rgb(255, 121, 198),

            cable_audio: Color32::from_rgb(120, 210, 230),
            cable_control: Color32::from_rgb(235, 165, 95),
            cable_gate: Color32::from_rgb(70, 230, 110),

            border: Color32::from_rgb(68, 71, 90),
            border_selected: Color32::from_rgb(189, 147, 249),
        }
    }

    /// Monokai theme - classic from Sublime Text.
    pub const fn monokai() -> Self {
        Self {
            bg_dark: Color32::from_rgb(39, 40, 34), // #272822
            bg_panel: Color32::from_rgb(49, 50, 44),
            bg_module: Color32::from_rgb(59, 60, 54),
            bg_widget: Color32::from_rgb(69, 70, 64),

            text_primary: Color32::from_rgb(248, 248, 242), // #f8f8f2
            text_secondary: Color32::from_rgb(200, 200, 195),
            text_dim: Color32::from_rgb(140, 140, 135),

            accent_primary: Color32::from_rgb(249, 38, 114), // #f92672 pink/red
            accent_cyan: Color32::from_rgb(102, 217, 239),   // #66d9ef
            accent_green: Color32::from_rgb(166, 226, 46),   // #a6e22e
            accent_red: Color32::from_rgb(249, 38, 114),
            accent_purple: Color32::from_rgb(174, 129, 255), // #ae81ff
            accent_yellow: Color32::from_rgb(230, 219, 116), // #e6db74
            accent_orange: Color32::from_rgb(253, 151, 31),  // #fd971f

            meter_green: Color32::from_rgb(166, 226, 46),
            meter_yellow: Color32::from_rgb(230, 219, 116),
            meter_red: Color32::from_rgb(249, 38, 114),

            port_audio: Color32::from_rgb(102, 217, 239),
            port_control: Color32::from_rgb(253, 151, 31),
            port_gate: Color32::from_rgb(166, 226, 46),
            port_midi: Color32::from_rgb(174, 129, 255),

            cable_audio: Color32::from_rgb(90, 200, 220),
            cable_control: Color32::from_rgb(230, 135, 25),
            cable_gate: Color32::from_rgb(150, 210, 40),

            border: Color32::from_rgb(59, 60, 54),
            border_selected: Color32::from_rgb(249, 38, 114),
        }
    }

    /// Solarized Dark theme - classic low-contrast dark theme.
    pub const fn solarized_dark() -> Self {
        Self {
            bg_dark: Color32::from_rgb(0, 43, 54),  // #002b36
            bg_panel: Color32::from_rgb(7, 54, 66), // #073642
            bg_module: Color32::from_rgb(7, 54, 66),
            bg_widget: Color32::from_rgb(88, 110, 117), // #586e75

            text_primary: Color32::from_rgb(131, 148, 150), // #839496
            text_secondary: Color32::from_rgb(101, 123, 131),
            text_dim: Color32::from_rgb(88, 110, 117),

            accent_primary: Color32::from_rgb(203, 75, 22), // #cb4b16 orange
            accent_cyan: Color32::from_rgb(42, 161, 152),   // #2aa198
            accent_green: Color32::from_rgb(133, 153, 0),   // #859900
            accent_red: Color32::from_rgb(220, 50, 47),     // #dc322f
            accent_purple: Color32::from_rgb(108, 113, 196), // #6c71c4
            accent_yellow: Color32::from_rgb(181, 137, 0),  // #b58900
            accent_orange: Color32::from_rgb(203, 75, 22),

            meter_green: Color32::from_rgb(133, 153, 0),
            meter_yellow: Color32::from_rgb(181, 137, 0),
            meter_red: Color32::from_rgb(220, 50, 47),

            port_audio: Color32::from_rgb(42, 161, 152),
            port_control: Color32::from_rgb(203, 75, 22),
            port_gate: Color32::from_rgb(133, 153, 0),
            port_midi: Color32::from_rgb(108, 113, 196),

            cable_audio: Color32::from_rgb(38, 148, 140),
            cable_control: Color32::from_rgb(185, 68, 20),
            cable_gate: Color32::from_rgb(120, 140, 0),

            border: Color32::from_rgb(7, 54, 66),
            border_selected: Color32::from_rgb(38, 139, 210), // #268bd2
        }
    }
}

/// Font settings.
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    /// Large font size (titles).
    pub size_large: f32,
    /// Heading font size (section headers).
    pub size_heading: f32,
    /// Normal font size (labels, parameters).
    pub size_normal: f32,
    /// Small font size (values, hints, port labels).
    pub size_small: f32,
}

impl Fonts {
    pub const fn default_fonts() -> Self {
        Self {
            size_large: 24.0,
            size_heading: 16.0,
            size_normal: 14.0,
            size_small: 10.0,
        }
    }

    /// Get a FontId for large text (titles).
    pub fn large(&self) -> FontId {
        FontId::proportional(self.size_large)
    }

    /// Get a FontId for section headings.
    pub fn heading(&self) -> FontId {
        FontId::proportional(self.size_heading)
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
    /// Width of the port column (left/right side of module).
    pub port_column_width: f32,
    /// Vertical spacing between ports in a column.
    pub port_vertical_spacing: f32,
    /// Horizontal gap between a port column and the module body.
    pub module_port_gap: f32,
    /// Standard module-card inner margin.
    pub module_inner_margin: f32,
    /// Compact module-card inner margin used by graph nodes.
    pub module_compact_inner_margin: f32,
    /// Minimum width for module content area (between port columns).
    pub module_content_min_width: f32,

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
            knob_size: 36.0,
            knob_size_small: 28.0,
            knob_size_large: 48.0,
            knob_label_height: 12.0,

            port_size: 12.0,
            port_column_width: 28.0,
            port_vertical_spacing: 24.0,
            module_port_gap: 4.0,
            module_inner_margin: 8.0,
            module_compact_inner_margin: 6.0,
            module_content_min_width: 100.0,

            module_min_width: 180.0,
            module_min_height: 80.0,

            meter_width: 60.0,
            meter_height: 80.0,

            oscilloscope_width: 120.0,
            oscilloscope_height: 60.0,

            adsr_width: 120.0,
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

    // --- Generic t-shirt gap scale for explicit `add_space` / layout gaps. ---
    /// 2 px — hairline gap.
    pub xxs: f32,
    /// 4 px — tight gap between closely related items.
    pub xs: f32,
    /// 6 px — small gap.
    pub sm: f32,
    /// 8 px — standard gap between groups.
    pub md: f32,
    /// 12 px — gap between sections.
    pub lg: f32,
    /// 16 px — large separation.
    pub xl: f32,

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
            xxs: 2.0,
            xs: 4.0,
            sm: 6.0,
            md: 8.0,
            lg: 12.0,
            xl: 16.0,
            module_grid_x: 210.0,
            module_grid_y: 320.0,
        }
    }
}

/// Widget styling parameters.
#[derive(Debug, Clone, Copy)]
pub struct WidgetStyle {
    // Rounded corners
    /// Default corner radius for panels and modules.
    pub corner_radius: f32,
    /// Smaller corner radius for widgets and buttons.
    pub corner_radius_small: f32,

    // Line widths
    /// Default border width.
    pub border_width: f32,
    /// Thick border width for selection and emphasis.
    pub border_width_thick: f32,

    // Knob styling
    /// Width of the arc stroke on knobs.
    pub knob_arc_width: f32,
    /// Size of the position indicator on knobs.
    pub knob_indicator_size: f32,
    /// Mouse drag sensitivity for knobs (value change per pixel).
    pub knob_sensitivity: f32,
    /// Number of segments in knob arc (for smooth rendering).
    pub knob_arc_segments: usize,

    // Slider styling
    /// Height of the slider rail/track.
    pub slider_rail_height: f32,
    /// Radius of the slider handle.
    pub slider_handle_radius: f32,

    // Meter styling
    /// Gap between meter segments.
    pub meter_segment_gap: f32,
    /// Number of segments in meters.
    pub meter_segments: usize,

    // Cable styling
    /// Curvature factor for cables (0.0 = straight, 1.0 = very curved).
    pub cable_curvature: f32,
}

impl WidgetStyle {
    /// Default widget style.
    pub const fn default_style() -> Self {
        Self {
            corner_radius: 4.0,
            corner_radius_small: 2.0,
            border_width: 1.0,
            border_width_thick: 2.0,
            knob_arc_width: 3.0,
            knob_indicator_size: 3.0,
            knob_sensitivity: 0.005,
            knob_arc_segments: 32,
            slider_rail_height: 6.0,
            slider_handle_radius: 7.0,
            meter_segment_gap: 1.0,
            meter_segments: 20,
            cable_curvature: 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Colors, ThemePreset};

    /// `Colors::is_dark` classifies every preset the way its name implies. It is
    /// what `setup_custom_style` feeds to egui as `Visuals::dark_mode` and as the
    /// theme preference the OS window decorations follow, so a preset whose
    /// palette drifts across the luminance threshold must fail here first.
    #[test]
    fn presets_are_classified_by_luminance() {
        for preset in ThemePreset::ALL {
            let expected_dark = !matches!(preset, ThemePreset::Light);
            assert_eq!(
                preset.theme().colors.is_dark(),
                expected_dark,
                "{} classified as {}",
                preset.name(),
                if expected_dark { "light" } else { "dark" }
            );
        }
    }

    /// The threshold itself, independent of the shipped palettes.
    #[test]
    fn is_dark_follows_the_panel_background() {
        let mut colors = Colors::dark();
        assert!(colors.is_dark());

        colors.bg_panel = Colors::light().bg_panel;
        assert!(!colors.is_dark());
    }
}
