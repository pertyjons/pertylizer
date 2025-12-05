//! Visual state types for GUI rendering.
//!
//! This module provides types for animating and rendering module states,
//! cables, and meters in the GUI.

use std::collections::HashMap;
use std::f32::consts::PI;

use super::connectivity::{ModuleConnectivityStatus, PortVisualState};
use super::shared_state::ModuleStateSnapshot;

/// Visual style for rendering a module.
#[derive(Debug, Clone)]
pub struct ModuleStyle {
    /// Background opacity (0.0 - 1.0).
    pub background_opacity: f32,
    /// Border color as RGBA.
    pub border_color: [f32; 4],
    /// Glow color as RGBA.
    pub glow_color: [f32; 4],
    /// Whether to show error overlay.
    pub error_overlay: bool,
    /// Pulse intensity for activity animation (0.0 - 1.0).
    pub pulse_intensity: f32,
}

impl Default for ModuleStyle {
    fn default() -> Self {
        Self {
            background_opacity: 1.0,
            border_color: [0.5, 0.5, 0.5, 1.0],
            glow_color: [0.0, 0.0, 0.0, 0.0],
            error_overlay: false,
            pulse_intensity: 0.0,
        }
    }
}

/// Visual state for a module in the GUI.
#[derive(Debug, Clone)]
pub struct ModuleVisualState {
    /// Current connectivity status.
    pub connectivity: ModuleConnectivityStatus,

    // Animation states
    /// Target opacity based on connectivity.
    pub opacity_target: f32,
    /// Current animated opacity.
    pub opacity_current: f32,
    /// Glow intensity for "live" indication.
    pub glow_intensity: f32,
    /// Pulse phase for activity animation.
    pub pulse_phase: f32,

    /// Port states.
    pub port_states: HashMap<String, PortVisualState>,

    /// Current error (if any).
    pub error: Option<String>,
    /// Time remaining for error flash animation.
    pub error_flash_time: f32,

    /// Whether module is selected.
    pub selected: bool,
    /// Whether module is hovered.
    pub hovered: bool,
}

impl Default for ModuleVisualState {
    fn default() -> Self {
        Self {
            connectivity: ModuleConnectivityStatus::Disconnected,
            opacity_target: 0.4,
            opacity_current: 0.4,
            glow_intensity: 0.0,
            pulse_phase: 0.0,
            port_states: HashMap::new(),
            error: None,
            error_flash_time: 0.0,
            selected: false,
            hovered: false,
        }
    }
}

impl ModuleVisualState {
    /// Create a new module visual state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update visual state from a module snapshot.
    pub fn update_from_snapshot(&mut self, snapshot: &ModuleStateSnapshot, dt: f32) {
        self.connectivity = snapshot.connectivity;

        // Calculate target opacity based on connectivity
        self.opacity_target = snapshot.connectivity.opacity();

        // Lerp current opacity toward target
        let lerp_speed = 5.0;
        self.opacity_current += (self.opacity_target - self.opacity_current) * lerp_speed * dt;

        // Glow for live modules
        if snapshot.connectivity == ModuleConnectivityStatus::Connected {
            self.glow_intensity = 0.3 + snapshot.cpu_usage.min(1.0) * 0.5;
        } else {
            self.glow_intensity *= 0.95; // Fade out
        }

        // Pulse animation based on output level
        let max_output = snapshot
            .output_levels
            .values()
            .cloned()
            .fold(0.0f32, f32::max);

        if max_output > 0.01 {
            self.pulse_phase = (self.pulse_phase + dt * 10.0 * max_output) % (2.0 * PI);
        }

        // Update port states
        for (port_name, &count) in &snapshot.input_connection_counts {
            self.port_states
                .entry(port_name.clone())
                .or_default()
                .connected = count > 0;
            self.port_states
                .get_mut(port_name)
                .unwrap()
                .connection_count = count;
        }
        for (port_name, &count) in &snapshot.output_connection_counts {
            self.port_states
                .entry(port_name.clone())
                .or_default()
                .connected = count > 0;
            self.port_states
                .get_mut(port_name)
                .unwrap()
                .connection_count = count;
        }

        // Update output levels in port states
        for (port_name, &level) in &snapshot.output_levels {
            if let Some(port_state) = self.port_states.get_mut(port_name) {
                port_state.update_level(level, 0.9);
            }
        }

        // Error flash decay
        if self.error_flash_time > 0.0 {
            self.error_flash_time -= dt;
            if self.error_flash_time <= 0.0 {
                self.error = None;
            }
        }
    }

    /// Set an error with flash animation.
    pub fn set_error(&mut self, error: String, flash_duration: f32) {
        self.error = Some(error);
        self.error_flash_time = flash_duration;
    }

    /// Clear any error.
    pub fn clear_error(&mut self) {
        self.error = None;
        self.error_flash_time = 0.0;
    }

    /// Get the visual style for rendering.
    pub fn get_style(&self) -> ModuleStyle {
        // Base border color from connectivity
        let border_color = match self.connectivity {
            ModuleConnectivityStatus::Disconnected => [0.5, 0.5, 0.5, 0.5],
            ModuleConnectivityStatus::PartiallyConnected => [0.8, 0.6, 0.2, 0.8],
            ModuleConnectivityStatus::Connected => [0.2, 0.8, 0.4, 1.0],
            ModuleConnectivityStatus::Bypassed => [0.6, 0.6, 0.6, 0.7],
        };

        // Selection/hover overlay
        let border_color = if self.selected {
            [0.3, 0.6, 1.0, 1.0]
        } else if self.hovered {
            [
                border_color[0] * 1.2,
                border_color[1] * 1.2,
                border_color[2] * 1.2,
                border_color[3],
            ]
        } else {
            border_color
        };

        ModuleStyle {
            background_opacity: self.opacity_current,
            border_color,
            glow_color: [0.3, 0.7, 1.0, self.glow_intensity],
            error_overlay: self.error.is_some(),
            pulse_intensity: (self.pulse_phase.sin() * 0.5 + 0.5) * 0.2,
        }
    }

    /// Get port visual state.
    pub fn get_port_state(&self, port_name: &str) -> Option<&PortVisualState> {
        self.port_states.get(port_name)
    }
}

/// A 2D point for cable rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    /// Create a new point.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Calculate distance to another point.
    pub fn distance(&self, other: &Point) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }

    /// Linear interpolation to another point.
    pub fn lerp(&self, other: &Point, t: f32) -> Point {
        Point {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }
}

/// Visual state for a connection cable.
#[derive(Debug, Clone)]
pub struct CableVisualState {
    /// Start position.
    pub from_pos: Point,
    /// End position.
    pub to_pos: Point,
    /// Current signal level (0.0 - 1.0+).
    pub signal_level: f32,
    /// Animation phase for signal flow direction.
    pub flow_phase: f32,
    /// Cable color as RGBA.
    pub color: [f32; 4],
    /// Whether this cable is selected.
    pub selected: bool,
    /// Whether this cable is hovered.
    pub hovered: bool,
}

impl CableVisualState {
    /// Create a new cable visual state.
    pub fn new(from_pos: Point, to_pos: Point) -> Self {
        Self {
            from_pos,
            to_pos,
            signal_level: 0.0,
            flow_phase: 0.0,
            color: [0.3, 0.7, 0.9, 0.6],
            selected: false,
            hovered: false,
        }
    }

    /// Update cable animation.
    pub fn update(&mut self, signal_level: f32, dt: f32) {
        // Smooth signal level
        self.signal_level = self.signal_level * 0.9 + signal_level * 0.1;

        // Animate flow based on signal presence
        if self.signal_level > 0.001 {
            self.flow_phase = (self.flow_phase + dt * self.signal_level * 5.0) % 1.0;
        }

        // Update color based on signal level
        let intensity = self.signal_level.sqrt().min(1.0);
        self.color = [
            0.3 + intensity * 0.5, // More red at high levels
            0.7 - intensity * 0.3,
            0.9,
            0.6 + intensity * 0.4,
        ];

        // Highlight if selected/hovered
        if self.selected {
            self.color = [0.3, 0.6, 1.0, 1.0];
        } else if self.hovered {
            self.color[3] = (self.color[3] + 0.2).min(1.0);
        }
    }

    /// Calculate a point along a bezier curve.
    pub fn bezier_point(&self, t: f32) -> Point {
        let control1 = Point::new(self.from_pos.x + 50.0, self.from_pos.y);
        let control2 = Point::new(self.to_pos.x - 50.0, self.to_pos.y);

        // Cubic bezier: B(t) = (1-t)³P0 + 3(1-t)²tP1 + 3(1-t)t²P2 + t³P3
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        Point {
            x: mt3 * self.from_pos.x
                + 3.0 * mt2 * t * control1.x
                + 3.0 * mt * t2 * control2.x
                + t3 * self.to_pos.x,
            y: mt3 * self.from_pos.y
                + 3.0 * mt2 * t * control1.y
                + 3.0 * mt * t2 * control2.y
                + t3 * self.to_pos.y,
        }
    }

    /// Get positions of animated flow dots.
    pub fn get_flow_dots(&self, num_dots: usize) -> Vec<Point> {
        if self.signal_level < 0.001 {
            return Vec::new();
        }

        (0..num_dots)
            .map(|i| {
                let t = (self.flow_phase + i as f32 / num_dots as f32) % 1.0;
                self.bezier_point(t)
            })
            .collect()
    }

    /// Get the cable length (approximate).
    pub fn length(&self) -> f32 {
        self.from_pos.distance(&self.to_pos)
    }
}

/// Tiny level meter for module output visualization.
#[derive(Debug, Clone)]
pub struct MiniMeter {
    /// Current level (0.0 - 1.0+).
    pub level: f32,
    /// Peak hold level.
    pub peak: f32,
    /// Time remaining for peak hold.
    pub peak_hold_time: f32,
    /// Meter width in pixels.
    pub width: f32,
    /// Meter height in pixels.
    pub height: f32,
}

impl MiniMeter {
    /// Create a new mini meter.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            level: 0.0,
            peak: 0.0,
            peak_hold_time: 0.0,
            width,
            height,
        }
    }

    /// Update the meter with a new level.
    pub fn update(&mut self, new_level: f32, dt: f32) {
        // Smooth level
        self.level = self.level * 0.8 + new_level * 0.2;

        // Peak hold
        if new_level > self.peak {
            self.peak = new_level;
            self.peak_hold_time = 1.0; // Hold for 1 second
        } else {
            self.peak_hold_time -= dt;
            if self.peak_hold_time <= 0.0 {
                self.peak *= 0.99; // Slow decay
            }
        }
    }

    /// Get the color for the current level.
    pub fn level_color(&self) -> [f32; 4] {
        if self.level > 0.9 {
            [1.0, 0.2, 0.2, 1.0] // Red (clipping)
        } else if self.level > 0.7 {
            [1.0, 0.8, 0.2, 1.0] // Yellow (hot)
        } else {
            [0.2, 0.8, 0.4, 1.0] // Green (normal)
        }
    }

    /// Get level as dB (-60 to 0).
    pub fn level_db(&self) -> f32 {
        if self.level > 0.0001 {
            20.0 * self.level.log10()
        } else {
            -60.0
        }
    }

    /// Get peak as dB.
    pub fn peak_db(&self) -> f32 {
        if self.peak > 0.0001 {
            20.0 * self.peak.log10()
        } else {
            -60.0
        }
    }

    /// Check if currently clipping.
    pub fn is_clipping(&self) -> bool {
        self.level > 1.0 || self.peak > 1.0
    }
}

impl Default for MiniMeter {
    fn default() -> Self {
        Self::new(8.0, 40.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_visual_state() {
        let mut state = ModuleVisualState::new();
        assert_eq!(state.connectivity, ModuleConnectivityStatus::Disconnected);

        state.connectivity = ModuleConnectivityStatus::Connected;
        state.opacity_target = 1.0;
        state.update_from_snapshot(
            &ModuleStateSnapshot::new(
                super::super::commands::ModuleId::new(
                    super::super::typed_params::ModuleType::Oscillator,
                    1,
                ),
                super::super::typed_params::ModuleType::Oscillator,
                "Test".to_string(),
            ),
            0.016,
        );

        let style = state.get_style();
        assert!(style.background_opacity > 0.0);
    }

    #[test]
    fn test_cable_visual_state() {
        let mut cable = CableVisualState::new(Point::new(0.0, 0.0), Point::new(100.0, 100.0));

        cable.update(0.5, 0.016);
        assert!(cable.signal_level > 0.0);
        assert!(cable.flow_phase > 0.0);

        let dots = cable.get_flow_dots(3);
        assert_eq!(dots.len(), 3);
    }

    #[test]
    fn test_mini_meter() {
        let mut meter = MiniMeter::new(10.0, 50.0);

        meter.update(0.8, 0.016);
        assert!(meter.level > 0.0);
        assert!(meter.peak >= meter.level);

        meter.update(0.2, 0.016);
        assert!(meter.peak > meter.level); // Peak should hold
    }

    #[test]
    fn test_point_operations() {
        let p1 = Point::new(0.0, 0.0);
        let p2 = Point::new(3.0, 4.0);

        assert!((p1.distance(&p2) - 5.0).abs() < 0.001);

        let mid = p1.lerp(&p2, 0.5);
        assert!((mid.x - 1.5).abs() < 0.001);
        assert!((mid.y - 2.0).abs() < 0.001);
    }
}
