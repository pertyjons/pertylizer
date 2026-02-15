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
        }
    }

    /// Length of the room (x-axis) in meters.
    #[must_use]
    pub fn length(self) -> f32 {
        match self {
            Self::Box { length, .. } | Self::Cylinder { length, .. } => length,
            Self::LShape {
                length_a, length_b, ..
            } => length_a + length_b,
        }
    }

    /// Width of the room (y-axis) in meters.
    #[must_use]
    pub fn width(self) -> f32 {
        match self {
            Self::Box { width, .. } => width,
            Self::Cylinder { radius, .. } => radius * 2.0,
            Self::LShape {
                width_a, width_b, ..
            } => width_a.max(width_b),
        }
    }

    /// Height of the room (z-axis) in meters.
    #[must_use]
    pub fn height(self) -> f32 {
        match self {
            Self::Box { height, .. } | Self::LShape { height, .. } => height,
            Self::Cylinder { radius, .. } => radius * 2.0,
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
}
