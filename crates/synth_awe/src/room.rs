//! Room geometry and material definitions for the Acoustic World Engine.

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

/// Speed of sound in air at room temperature (m/s).
pub const SPEED_OF_SOUND: f32 = 343.0;

/// Shape of the simulated room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RoomShape {
    /// Rectangular room with dimensions in meters.
    Box {
        /// Length in meters (x-axis).
        length: f32,
        /// Width in meters (y-axis).
        width: f32,
        /// Height in meters (z-axis).
        height: f32,
    },

    /// Cylindrical room (pipeline/tunnel mode).
    Cylinder {
        /// Radius in meters.
        radius: f32,
        /// Length in meters (along cylinder axis).
        length: f32,
    },

    /// L-shaped room (two connected rectangles).
    LShape {
        /// First section: length along x-axis (meters).
        length_a: f32,
        /// First section: width along y-axis (meters).
        width_a: f32,
        /// Second section: length along x-axis (meters).
        length_b: f32,
        /// Second section: width along y-axis (meters).
        width_b: f32,
        /// Height of both sections (meters).
        height: f32,
    },

    /// Spherical room — all dimensions equal to diameter.
    Sphere {
        /// Radius in meters.
        radius: f32,
    },

    /// Dome (half-sphere) — height = radius, width/length = diameter.
    Dome {
        /// Radius in meters.
        radius: f32,
    },

    /// Open tube (no end caps) — cylindrical with no end reflections.
    Tube {
        /// Radius in meters.
        radius: f32,
        /// Length in meters.
        length: f32,
    },
}

impl RoomShape {
    /// Default room: 8m x 5m x 3m.
    pub const DEFAULT: Self = Self::Box {
        length: 8.0,
        width: 5.0,
        height: 3.0,
    };

    /// Default cylinder: radius 1m, length 20m (tunnel).
    pub const DEFAULT_CYLINDER: Self = Self::Cylinder {
        radius: 1.0,
        length: 20.0,
    };

    /// Default L-shape: two connected rectangles.
    pub const DEFAULT_LSHAPE: Self = Self::LShape {
        length_a: 8.0,
        width_a: 5.0,
        length_b: 6.0,
        width_b: 4.0,
        height: 3.0,
    };

    /// Default sphere: radius 5m.
    pub const DEFAULT_SPHERE: Self = Self::Sphere { radius: 5.0 };

    /// Default dome: radius 6m.
    pub const DEFAULT_DOME: Self = Self::Dome { radius: 6.0 };

    /// Default tube: radius 1.5m, length 30m.
    pub const DEFAULT_TUBE: Self = Self::Tube {
        radius: 1.5,
        length: 30.0,
    };

    /// Volume of the room in cubic meters.
    #[must_use]
    pub fn volume(self) -> f32 {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => length * width * height,
            Self::Cylinder { radius, length } => PI * radius * radius * length,
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => (length_a * width_a + length_b * width_b) * height,
            Self::Sphere { radius } => (4.0 / 3.0) * PI * radius * radius * radius,
            Self::Dome { radius } => (2.0 / 3.0) * PI * radius * radius * radius,
            Self::Tube { radius, length } => PI * radius * radius * length,
        }
    }

    /// Total surface area in square meters.
    #[must_use]
    pub fn surface_area(self) -> f32 {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => 2.0 * (length * width + length * height + width * height),
            Self::Cylinder { radius, length } => 2.0 * PI * radius * (radius + length),
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => {
                let floor_ceiling = 2.0 * (length_a * width_a + length_b * width_b);
                let walls = 2.0 * height * (length_a + width_a + length_b + width_b);
                floor_ceiling + walls
            }
            Self::Sphere { radius } => 4.0 * PI * radius * radius,
            Self::Dome { radius } => 3.0 * PI * radius * radius,
            Self::Tube { radius, length } => 2.0 * PI * radius * length,
        }
    }

    /// Length of the room (x-axis) in meters.
    #[must_use]
    pub fn length(self) -> f32 {
        match self {
            Self::Box { length, .. }
            | Self::Cylinder { length, .. }
            | Self::Tube { length, .. } => length,
            Self::LShape {
                length_a, length_b, ..
            } => length_a + length_b,
            Self::Sphere { radius } | Self::Dome { radius } => radius * 2.0,
        }
    }

    /// Width of the room (y-axis) in meters.
    #[must_use]
    pub fn width(self) -> f32 {
        match self {
            Self::Box { width, .. } => width,
            Self::Cylinder { radius, .. } | Self::Tube { radius, .. } => radius * 2.0,
            Self::LShape {
                width_a, width_b, ..
            } => width_a.max(width_b),
            Self::Sphere { radius } | Self::Dome { radius } => radius * 2.0,
        }
    }

    /// Height of the room (z-axis) in meters.
    #[must_use]
    pub fn height(self) -> f32 {
        match self {
            Self::Box { height, .. } | Self::LShape { height, .. } => height,
            Self::Cylinder { radius, .. } | Self::Tube { radius, .. } => radius * 2.0,
            Self::Sphere { radius } => radius * 2.0,
            Self::Dome { radius } => radius,
        }
    }

    /// Axial room modes (fundamental frequencies for each axis).
    ///
    /// Returns (length_mode, width_mode, height_mode) in Hz.
    /// Formula: f = c / (2 * L), where c = 343 m/s (speed of sound).
    #[must_use]
    pub fn axial_modes(self) -> (f32, f32, f32) {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => (
                SPEED_OF_SOUND / (2.0 * length),
                SPEED_OF_SOUND / (2.0 * width),
                SPEED_OF_SOUND / (2.0 * height),
            ),
            Self::Cylinder { radius, length } => {
                let diameter = radius * 2.0;
                (
                    SPEED_OF_SOUND / (2.0 * length),
                    SPEED_OF_SOUND / (2.0 * diameter),
                    SPEED_OF_SOUND / (2.0 * diameter),
                )
            }
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => (
                SPEED_OF_SOUND / (2.0 * (length_a + length_b)),
                SPEED_OF_SOUND / (2.0 * width_a.max(width_b)),
                SPEED_OF_SOUND / (2.0 * height),
            ),
            Self::Sphere { radius } => {
                // All modes coincide at c / (4r)
                let mode = SPEED_OF_SOUND / (4.0 * radius);
                (mode, mode, mode)
            }
            Self::Dome { radius } => (
                SPEED_OF_SOUND / (4.0 * radius), // length mode (diameter)
                SPEED_OF_SOUND / (4.0 * radius), // width mode (diameter)
                SPEED_OF_SOUND / (2.0 * radius), // height mode (radius)
            ),
            Self::Tube { radius, length } => (
                SPEED_OF_SOUND / length,         // open-open length mode: c/L
                SPEED_OF_SOUND / (4.0 * radius), // radial mode
                SPEED_OF_SOUND / (4.0 * radius), // radial mode
            ),
        }
    }
}

impl Default for RoomShape {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Surface material with frequency-dependent absorption.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// Low-frequency absorption coefficient (0.0 = fully reflective, 1.0 = fully absorptive).
    pub absorption_low: f32,
    /// Mid-frequency absorption coefficient.
    pub absorption_mid: f32,
    /// High-frequency absorption coefficient.
    pub absorption_high: f32,
    /// Diffusion coefficient (0.0 = specular, 1.0 = fully diffuse).
    pub diffusion: f32,
}

impl Material {
    /// Hard concrete walls.
    pub const CONCRETE: Self = Self {
        absorption_low: 0.01,
        absorption_mid: 0.02,
        absorption_high: 0.04,
        diffusion: 0.1,
    };

    /// Wood paneling.
    pub const WOOD: Self = Self {
        absorption_low: 0.15,
        absorption_mid: 0.10,
        absorption_high: 0.07,
        diffusion: 0.3,
    };

    /// Glass windows.
    pub const GLASS: Self = Self {
        absorption_low: 0.18,
        absorption_mid: 0.06,
        absorption_high: 0.04,
        diffusion: 0.05,
    };

    /// Metal surface.
    pub const METAL: Self = Self {
        absorption_low: 0.01,
        absorption_mid: 0.01,
        absorption_high: 0.02,
        diffusion: 0.05,
    };

    /// Fabric / curtains.
    pub const FABRIC: Self = Self {
        absorption_low: 0.05,
        absorption_mid: 0.30,
        absorption_high: 0.60,
        diffusion: 0.7,
    };

    /// Ceramic tile.
    pub const TILE: Self = Self {
        absorption_low: 0.01,
        absorption_mid: 0.01,
        absorption_high: 0.02,
        diffusion: 0.1,
    };

    /// Polished marble.
    pub const MARBLE: Self = Self {
        absorption_low: 0.02,
        absorption_mid: 0.03,
        absorption_high: 0.05,
        diffusion: 0.2,
    };

    /// Ice / frozen surfaces.
    pub const ICE: Self = Self {
        absorption_low: 0.02,
        absorption_mid: 0.04,
        absorption_high: 0.08,
        diffusion: 0.15,
    };

    /// Thick carpet.
    pub const CARPET: Self = Self {
        absorption_low: 0.25,
        absorption_mid: 0.55,
        absorption_high: 0.85,
        diffusion: 0.8,
    };

    /// Water-lined chamber.
    pub const WATER: Self = Self {
        absorption_low: 0.06,
        absorption_mid: 0.12,
        absorption_high: 0.40,
        diffusion: 0.2,
    };

    /// Perfectly reflective void.
    pub const VOID: Self = Self {
        absorption_low: 0.0,
        absorption_mid: 0.0,
        absorption_high: 0.0,
        diffusion: 0.0,
    };

    /// Prism-like surface with extreme HF absorption and high diffusion.
    pub const PRISM: Self = Self {
        absorption_low: 0.02,
        absorption_mid: 0.08,
        absorption_high: 0.90,
        diffusion: 0.95,
    };

    /// Plasma sheen with strong LF damping and bright tail.
    pub const PLASMA: Self = Self {
        absorption_low: 0.30,
        absorption_mid: 0.18,
        absorption_high: 0.85,
        diffusion: 0.9,
    };

    /// Membrane walls: absorbs lows more than highs (non-physical).
    pub const MEMBRANE: Self = Self {
        absorption_low: 0.75,
        absorption_mid: 0.22,
        absorption_high: 0.05,
        diffusion: 0.4,
    };

    /// Nanogel: ultra-absorbent but highly diffusive.
    pub const NANOGEL: Self = Self {
        absorption_low: 0.05,
        absorption_mid: 0.50,
        absorption_high: 0.95,
        diffusion: 1.0,
    };

    /// Average absorption across all frequency bands.
    #[must_use]
    pub fn average_absorption(self) -> f32 {
        (self.absorption_low + self.absorption_mid + self.absorption_high) / 3.0
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::CONCRETE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_default() {
        let room = RoomShape::default();
        assert!((room.volume() - 120.0).abs() < 0.01); // 8 * 5 * 3 = 120
    }

    #[test]
    fn test_room_surface_area() {
        let room = RoomShape::DEFAULT;
        // 2*(8*5 + 8*3 + 5*3) = 2*(40+24+15) = 158
        assert!((room.surface_area() - 158.0).abs() < 0.01);
    }

    #[test]
    fn test_axial_modes() {
        let room = RoomShape::DEFAULT;
        let (fl, fw, fh) = room.axial_modes();
        // 343 / (2*8) = 21.4375
        assert!((fl - 21.4375).abs() < 0.01);
        // 343 / (2*5) = 34.3
        assert!((fw - 34.3).abs() < 0.01);
        // 343 / (2*3) ≈ 57.167
        assert!((fh - 57.167).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_volume() {
        let room = RoomShape::DEFAULT_CYLINDER;
        // PI * 1^2 * 20 ≈ 62.83
        assert!((room.volume() - 62.83).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_surface_area() {
        let room = RoomShape::DEFAULT_CYLINDER;
        // 2 * PI * 1 * (1 + 20) ≈ 131.95
        assert!((room.surface_area() - 131.95).abs() < 0.01);
    }

    #[test]
    fn test_lshape_volume() {
        let room = RoomShape::DEFAULT_LSHAPE;
        // (8*5 + 6*4) * 3 = (40+24) * 3 = 192
        assert!((room.volume() - 192.0).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_axial_modes() {
        let room = RoomShape::DEFAULT_CYLINDER;
        let (fl, fw, fh) = room.axial_modes();
        // length mode: 343 / (2*20) = 8.575
        assert!((fl - 8.575).abs() < 0.01);
        // radial modes (diameter = 2): 343 / (2*2) = 85.75
        assert!((fw - 85.75).abs() < 0.01);
        assert!((fh - 85.75).abs() < 0.01);
    }

    #[test]
    fn test_lshape_dimensions() {
        let room = RoomShape::DEFAULT_LSHAPE;
        // length = 8 + 6 = 14
        assert!((room.length() - 14.0).abs() < 0.01);
        // width = max(5, 4) = 5
        assert!((room.width() - 5.0).abs() < 0.01);
        // height = 3
        assert!((room.height() - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_material_average() {
        let m = Material::CONCRETE;
        let avg = m.average_absorption();
        assert!(avg > 0.0 && avg < 1.0);
    }

    #[test]
    fn test_sphere_volume() {
        let room = RoomShape::DEFAULT_SPHERE;
        // 4/3 * PI * 5^3 ≈ 523.60
        assert!((room.volume() - 523.60).abs() < 0.1);
    }

    #[test]
    fn test_sphere_surface_area() {
        let room = RoomShape::DEFAULT_SPHERE;
        // 4 * PI * 5^2 ≈ 314.16
        assert!((room.surface_area() - 314.16).abs() < 0.1);
    }

    #[test]
    fn test_sphere_dimensions() {
        let room = RoomShape::DEFAULT_SPHERE;
        assert!((room.length() - 10.0).abs() < 0.01);
        assert!((room.width() - 10.0).abs() < 0.01);
        assert!((room.height() - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_sphere_axial_modes() {
        let room = RoomShape::DEFAULT_SPHERE;
        let (fl, fw, fh) = room.axial_modes();
        // All modes = 343 / (4*5) = 17.15
        assert!((fl - 17.15).abs() < 0.01);
        assert!((fw - 17.15).abs() < 0.01);
        assert!((fh - 17.15).abs() < 0.01);
    }

    #[test]
    fn test_dome_volume() {
        let room = RoomShape::DEFAULT_DOME;
        // 2/3 * PI * 6^3 ≈ 452.39
        assert!((room.volume() - 452.39).abs() < 0.1);
    }

    #[test]
    fn test_dome_surface_area() {
        let room = RoomShape::DEFAULT_DOME;
        // 3 * PI * 6^2 ≈ 339.29
        assert!((room.surface_area() - 339.29).abs() < 0.1);
    }

    #[test]
    fn test_dome_dimensions() {
        let room = RoomShape::DEFAULT_DOME;
        assert!((room.length() - 12.0).abs() < 0.01);
        assert!((room.width() - 12.0).abs() < 0.01);
        assert!((room.height() - 6.0).abs() < 0.01); // height = radius
    }

    #[test]
    fn test_tube_volume() {
        let room = RoomShape::DEFAULT_TUBE;
        // PI * 1.5^2 * 30 ≈ 212.06
        assert!((room.volume() - 212.06).abs() < 0.1);
    }

    #[test]
    fn test_tube_surface_area() {
        let room = RoomShape::DEFAULT_TUBE;
        // 2 * PI * 1.5 * 30 ≈ 282.74 (no end caps)
        assert!((room.surface_area() - 282.74).abs() < 0.1);
    }

    #[test]
    fn test_tube_dimensions() {
        let room = RoomShape::DEFAULT_TUBE;
        assert!((room.length() - 30.0).abs() < 0.01);
        assert!((room.width() - 3.0).abs() < 0.01);
        assert!((room.height() - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_tube_axial_modes() {
        let room = RoomShape::DEFAULT_TUBE;
        let (fl, fw, fh) = room.axial_modes();
        // length mode: 343 / 30 ≈ 11.43 (open-open)
        assert!((fl - 11.43).abs() < 0.01);
        // radial modes: 343 / (4*1.5) ≈ 57.17
        assert!((fw - 57.17).abs() < 0.01);
        assert!((fh - 57.17).abs() < 0.01);
    }
}
