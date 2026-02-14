//! Room geometry and material definitions for the Acoustic World Engine.

use serde::{Deserialize, Serialize};

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
}

impl RoomShape {
    /// Default room: 8m x 5m x 3m.
    pub const DEFAULT: Self = Self::Box {
        length: 8.0,
        width: 5.0,
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
        }
    }

    /// Axial room modes (fundamental frequencies for each axis).
    ///
    /// Returns (length_mode, width_mode, height_mode) in Hz.
    /// Formula: f = c / (2 * L), where c = 343 m/s (speed of sound).
    #[must_use]
    pub fn axial_modes(self) -> (f32, f32, f32) {
        const SPEED_OF_SOUND: f32 = 343.0;
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
    fn test_material_average() {
        let m = Material::CONCRETE;
        let avg = m.average_absorption();
        assert!(avg > 0.0 && avg < 1.0);
    }
}
