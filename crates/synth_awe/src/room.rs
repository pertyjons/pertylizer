//! Room geometry and material definitions for the Acoustic World Engine.

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};
use synth_core::{Hertz, NormalizedValue};

use crate::types::{CubicMeters, Meters, MetersPerSecond, SquareMeters};

/// Speed of sound in air at room temperature (m/s).
pub const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);

/// Shape of the simulated room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RoomShape {
    /// Rectangular room with dimensions in meters.
    Box {
        /// Length in meters (x-axis).
        length: Meters,
        /// Width in meters (y-axis).
        width: Meters,
        /// Height in meters (z-axis).
        height: Meters,
    },

    /// Cylindrical room (pipeline/tunnel mode).
    Cylinder {
        /// Radius in meters.
        radius: Meters,
        /// Length in meters (along cylinder axis).
        length: Meters,
    },

    /// L-shaped room (two connected rectangles).
    LShape {
        /// First section: length along x-axis (meters).
        length_a: Meters,
        /// First section: width along y-axis (meters).
        width_a: Meters,
        /// Second section: length along x-axis (meters).
        length_b: Meters,
        /// Second section: width along y-axis (meters).
        width_b: Meters,
        /// Height of both sections (meters).
        height: Meters,
    },

    /// Spherical room — all dimensions equal to diameter.
    Sphere {
        /// Radius in meters.
        radius: Meters,
    },

    /// Dome (half-sphere) — height = radius, width/length = diameter.
    Dome {
        /// Radius in meters.
        radius: Meters,
    },

    /// Open tube (no end caps) — cylindrical with no end reflections.
    Tube {
        /// Radius in meters.
        radius: Meters,
        /// Length in meters.
        length: Meters,
    },
}

impl RoomShape {
    /// Default room: 8m x 5m x 3m.
    pub const DEFAULT: Self = Self::Box {
        length: Meters::new(8.0),
        width: Meters::new(5.0),
        height: Meters::new(3.0),
    };

    /// Default cylinder: radius 1m, length 20m (tunnel).
    pub const DEFAULT_CYLINDER: Self = Self::Cylinder {
        radius: Meters::new(1.0),
        length: Meters::new(20.0),
    };

    /// Default L-shape: two connected rectangles.
    pub const DEFAULT_LSHAPE: Self = Self::LShape {
        length_a: Meters::new(8.0),
        width_a: Meters::new(5.0),
        length_b: Meters::new(6.0),
        width_b: Meters::new(4.0),
        height: Meters::new(3.0),
    };

    /// Default sphere: radius 5m.
    pub const DEFAULT_SPHERE: Self = Self::Sphere {
        radius: Meters::new(5.0),
    };

    /// Default dome: radius 6m.
    pub const DEFAULT_DOME: Self = Self::Dome {
        radius: Meters::new(6.0),
    };

    /// Default tube: radius 1.5m, length 30m.
    pub const DEFAULT_TUBE: Self = Self::Tube {
        radius: Meters::new(1.5),
        length: Meters::new(30.0),
    };

    /// Volume of the room in cubic meters.
    pub fn volume(self) -> CubicMeters {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => CubicMeters::new(length.as_f32() * width.as_f32() * height.as_f32()),
            Self::Cylinder { radius, length } => {
                CubicMeters::new(PI * radius.as_f32() * radius.as_f32() * length.as_f32())
            }
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => CubicMeters::new(
                (length_a.as_f32() * width_a.as_f32() + length_b.as_f32() * width_b.as_f32())
                    * height.as_f32(),
            ),
            Self::Sphere { radius } => CubicMeters::new(
                (4.0 / 3.0) * PI * radius.as_f32() * radius.as_f32() * radius.as_f32(),
            ),
            Self::Dome { radius } => CubicMeters::new(
                (2.0 / 3.0) * PI * radius.as_f32() * radius.as_f32() * radius.as_f32(),
            ),
            Self::Tube { radius, length } => {
                CubicMeters::new(PI * radius.as_f32() * radius.as_f32() * length.as_f32())
            }
        }
    }

    /// Total surface area in square meters.
    pub fn surface_area(self) -> SquareMeters {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => SquareMeters::new(
                2.0 * (length.as_f32() * width.as_f32()
                    + length.as_f32() * height.as_f32()
                    + width.as_f32() * height.as_f32()),
            ),
            Self::Cylinder { radius, length } => {
                SquareMeters::new(2.0 * PI * radius.as_f32() * (radius.as_f32() + length.as_f32()))
            }
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => {
                let floor_ceiling = 2.0
                    * (length_a.as_f32() * width_a.as_f32() + length_b.as_f32() * width_b.as_f32());
                let walls = 2.0
                    * height.as_f32()
                    * (length_a.as_f32() + width_a.as_f32() + length_b.as_f32() + width_b.as_f32());
                SquareMeters::new(floor_ceiling + walls)
            }
            Self::Sphere { radius } => {
                SquareMeters::new(4.0 * PI * radius.as_f32() * radius.as_f32())
            }
            Self::Dome { radius } => {
                SquareMeters::new(3.0 * PI * radius.as_f32() * radius.as_f32())
            }
            Self::Tube { radius, length } => {
                SquareMeters::new(2.0 * PI * radius.as_f32() * length.as_f32())
            }
        }
    }

    /// Length of the room (x-axis) in meters.
    pub fn length(self) -> Meters {
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
    pub fn width(self) -> Meters {
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
    pub fn height(self) -> Meters {
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
    pub fn axial_modes(self) -> (Hertz, Hertz, Hertz) {
        match self {
            Self::Box {
                length,
                width,
                height,
            } => (
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * length.as_f32())),
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * width.as_f32())),
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * height.as_f32())),
            ),
            Self::Cylinder { radius, length } => {
                let diameter = radius * 2.0;
                (
                    Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * length.as_f32())),
                    Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * diameter.as_f32())),
                    Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * diameter.as_f32())),
                )
            }
            Self::LShape {
                length_a,
                width_a,
                length_b,
                width_b,
                height,
            } => (
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * (length_a + length_b).as_f32())),
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * width_a.max(width_b).as_f32())),
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * height.as_f32())),
            ),
            Self::Sphere { radius } => {
                // All modes coincide at c / (4r)
                let mode = Hertz::new(SPEED_OF_SOUND.as_f32() / (4.0 * radius.as_f32()));
                (mode, mode, mode)
            }
            Self::Dome { radius } => (
                Hertz::new(SPEED_OF_SOUND.as_f32() / (4.0 * radius.as_f32())), // length mode (diameter)
                Hertz::new(SPEED_OF_SOUND.as_f32() / (4.0 * radius.as_f32())), // width mode (diameter)
                Hertz::new(SPEED_OF_SOUND.as_f32() / (2.0 * radius.as_f32())), // height mode (radius)
            ),
            Self::Tube { radius, length } => (
                Hertz::new(SPEED_OF_SOUND.as_f32() / length.as_f32()), // open-open length mode: c/L
                Hertz::new(SPEED_OF_SOUND.as_f32() / (4.0 * radius.as_f32())), // radial mode
                Hertz::new(SPEED_OF_SOUND.as_f32() / (4.0 * radius.as_f32())), // radial mode
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
    pub absorption_low: NormalizedValue,
    /// Mid-frequency absorption coefficient.
    pub absorption_mid: NormalizedValue,
    /// High-frequency absorption coefficient.
    pub absorption_high: NormalizedValue,
    /// Diffusion coefficient (0.0 = specular, 1.0 = fully diffuse).
    pub diffusion: NormalizedValue,
}

impl Material {
    /// Hard concrete walls — hårt, mörkt, kallt.
    pub const CONCRETE: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.05),
        absorption_mid: NormalizedValue::new_unchecked(0.08),
        absorption_high: NormalizedValue::new_unchecked(0.15),
        diffusion: NormalizedValue::new_unchecked(0.15),
    };

    /// Wood paneling — varmt, balanserat.
    pub const WOOD: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.20),
        absorption_mid: NormalizedValue::new_unchecked(0.18),
        absorption_high: NormalizedValue::new_unchecked(0.25),
        diffusion: NormalizedValue::new_unchecked(0.35),
    };

    /// Glass windows — ljust men tunt i basen.
    pub const GLASS: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.35),
        absorption_mid: NormalizedValue::new_unchecked(0.12),
        absorption_high: NormalizedValue::new_unchecked(0.08),
        diffusion: NormalizedValue::new_unchecked(0.05),
    };

    /// Metal surface — ultraljust, ringande.
    pub const METAL: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.02),
        absorption_mid: NormalizedValue::new_unchecked(0.03),
        absorption_high: NormalizedValue::new_unchecked(0.05),
        diffusion: NormalizedValue::new_unchecked(0.05),
    };

    /// Fabric / curtains — mycket mörkt.
    pub const FABRIC: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.10),
        absorption_mid: NormalizedValue::new_unchecked(0.45),
        absorption_high: NormalizedValue::new_unchecked(0.75),
        diffusion: NormalizedValue::new_unchecked(0.75),
    };

    /// Ceramic tile — hårt, ljust, kliniskt.
    pub const TILE: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.03),
        absorption_mid: NormalizedValue::new_unchecked(0.04),
        absorption_high: NormalizedValue::new_unchecked(0.08),
        diffusion: NormalizedValue::new_unchecked(0.10),
    };

    /// Polished marble — varmare hårdyta.
    pub const MARBLE: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.08),
        absorption_mid: NormalizedValue::new_unchecked(0.10),
        absorption_high: NormalizedValue::new_unchecked(0.20),
        diffusion: NormalizedValue::new_unchecked(0.20),
    };

    /// Ice / frozen surfaces — krispigt, märkbar HF-absorption.
    pub const ICE: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.06),
        absorption_mid: NormalizedValue::new_unchecked(0.15),
        absorption_high: NormalizedValue::new_unchecked(0.30),
        diffusion: NormalizedValue::new_unchecked(0.15),
    };

    /// Thick carpet — dött, absorberar allt.
    pub const CARPET: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.30),
        absorption_mid: NormalizedValue::new_unchecked(0.60),
        absorption_high: NormalizedValue::new_unchecked(0.90),
        diffusion: NormalizedValue::new_unchecked(0.85),
    };

    /// Water-lined chamber — mumlande, medium-mörkt.
    pub const WATER: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.15),
        absorption_mid: NormalizedValue::new_unchecked(0.30),
        absorption_high: NormalizedValue::new_unchecked(0.55),
        diffusion: NormalizedValue::new_unchecked(0.25),
    };

    /// Perfectly reflective void.
    pub const VOID: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.0),
        absorption_mid: NormalizedValue::new_unchecked(0.0),
        absorption_high: NormalizedValue::new_unchecked(0.0),
        diffusion: NormalizedValue::new_unchecked(0.0),
    };

    /// Prism-like surface with extreme HF absorption and high diffusion.
    pub const PRISM: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.05),
        absorption_mid: NormalizedValue::new_unchecked(0.20),
        absorption_high: NormalizedValue::new_unchecked(0.92),
        diffusion: NormalizedValue::new_unchecked(0.95),
    };

    /// Plasma sheen with strong LF damping and bright tail.
    pub const PLASMA: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.40),
        absorption_mid: NormalizedValue::new_unchecked(0.25),
        absorption_high: NormalizedValue::new_unchecked(0.85),
        diffusion: NormalizedValue::new_unchecked(0.90),
    };

    /// Membrane walls: absorbs lows more than highs (non-physical).
    pub const MEMBRANE: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.80),
        absorption_mid: NormalizedValue::new_unchecked(0.30),
        absorption_high: NormalizedValue::new_unchecked(0.08),
        diffusion: NormalizedValue::new_unchecked(0.40),
    };

    /// Nanogel: ultra-absorbent but highly diffusive.
    pub const NANOGEL: Self = Self {
        absorption_low: NormalizedValue::new_unchecked(0.10),
        absorption_mid: NormalizedValue::new_unchecked(0.55),
        absorption_high: NormalizedValue::new_unchecked(0.95),
        diffusion: NormalizedValue::new_unchecked(1.0),
    };

    /// Average absorption across all frequency bands.
    #[must_use]
    pub fn average_absorption(self) -> NormalizedValue {
        let avg = (self.absorption_low.as_f32()
            + self.absorption_mid.as_f32()
            + self.absorption_high.as_f32())
            / 3.0;
        NormalizedValue::new(avg)
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
        assert!((room.volume().as_f32() - 120.0).abs() < 0.01); // 8 * 5 * 3 = 120
    }

    #[test]
    fn test_room_surface_area() {
        let room = RoomShape::DEFAULT;
        // 2*(8*5 + 8*3 + 5*3) = 2*(40+24+15) = 158
        assert!((room.surface_area().as_f32() - 158.0).abs() < 0.01);
    }

    #[test]
    fn test_axial_modes() {
        let room = RoomShape::DEFAULT;
        let (fl, fw, fh) = room.axial_modes();
        // 343 / (2*8) = 21.4375
        assert!((fl.as_f32() - 21.4375).abs() < 0.01);
        // 343 / (2*5) = 34.3
        assert!((fw.as_f32() - 34.3).abs() < 0.01);
        // 343 / (2*3) ≈ 57.167
        assert!((fh.as_f32() - 57.167).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_volume() {
        let room = RoomShape::DEFAULT_CYLINDER;
        // PI * 1^2 * 20 ≈ 62.83
        assert!((room.volume().as_f32() - 62.83).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_surface_area() {
        let room = RoomShape::DEFAULT_CYLINDER;
        // 2 * PI * 1 * (1 + 20) ≈ 131.95
        assert!((room.surface_area().as_f32() - 131.95).abs() < 0.01);
    }

    #[test]
    fn test_lshape_volume() {
        let room = RoomShape::DEFAULT_LSHAPE;
        // (8*5 + 6*4) * 3 = (40+24) * 3 = 192
        assert!((room.volume().as_f32() - 192.0).abs() < 0.01);
    }

    #[test]
    fn test_cylinder_axial_modes() {
        let room = RoomShape::DEFAULT_CYLINDER;
        let (fl, fw, fh) = room.axial_modes();
        // length mode: 343 / (2*20) = 8.575
        assert!((fl.as_f32() - 8.575).abs() < 0.01);
        // radial modes (diameter = 2): 343 / (2*2) = 85.75
        assert!((fw.as_f32() - 85.75).abs() < 0.01);
        assert!((fh.as_f32() - 85.75).abs() < 0.01);
    }

    #[test]
    fn test_lshape_dimensions() {
        let room = RoomShape::DEFAULT_LSHAPE;
        // length = 8 + 6 = 14
        assert!((room.length().as_f32() - 14.0).abs() < 0.01);
        // width = max(5, 4) = 5
        assert!((room.width().as_f32() - 5.0).abs() < 0.01);
        // height = 3
        assert!((room.height().as_f32() - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_material_average() {
        let m = Material::CONCRETE;
        let avg = m.average_absorption();
        assert!(avg.as_f32() > 0.0 && avg.as_f32() < 1.0);
    }

    #[test]
    fn test_sphere_volume() {
        let room = RoomShape::DEFAULT_SPHERE;
        // 4/3 * PI * 5^3 ≈ 523.60
        assert!((room.volume().as_f32() - 523.60).abs() < 0.1);
    }

    #[test]
    fn test_sphere_surface_area() {
        let room = RoomShape::DEFAULT_SPHERE;
        // 4 * PI * 5^2 ≈ 314.16
        assert!((room.surface_area().as_f32() - 314.16).abs() < 0.1);
    }

    #[test]
    fn test_sphere_dimensions() {
        let room = RoomShape::DEFAULT_SPHERE;
        assert!((room.length().as_f32() - 10.0).abs() < 0.01);
        assert!((room.width().as_f32() - 10.0).abs() < 0.01);
        assert!((room.height().as_f32() - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_sphere_axial_modes() {
        let room = RoomShape::DEFAULT_SPHERE;
        let (fl, fw, fh) = room.axial_modes();
        // All modes = 343 / (4*5) = 17.15
        assert!((fl.as_f32() - 17.15).abs() < 0.01);
        assert!((fw.as_f32() - 17.15).abs() < 0.01);
        assert!((fh.as_f32() - 17.15).abs() < 0.01);
    }

    #[test]
    fn test_dome_volume() {
        let room = RoomShape::DEFAULT_DOME;
        // 2/3 * PI * 6^3 ≈ 452.39
        assert!((room.volume().as_f32() - 452.39).abs() < 0.1);
    }

    #[test]
    fn test_dome_surface_area() {
        let room = RoomShape::DEFAULT_DOME;
        // 3 * PI * 6^2 ≈ 339.29
        assert!((room.surface_area().as_f32() - 339.29).abs() < 0.1);
    }

    #[test]
    fn test_dome_dimensions() {
        let room = RoomShape::DEFAULT_DOME;
        assert!((room.length().as_f32() - 12.0).abs() < 0.01);
        assert!((room.width().as_f32() - 12.0).abs() < 0.01);
        assert!((room.height().as_f32() - 6.0).abs() < 0.01); // height = radius
    }

    #[test]
    fn test_tube_volume() {
        let room = RoomShape::DEFAULT_TUBE;
        // PI * 1.5^2 * 30 ≈ 212.06
        assert!((room.volume().as_f32() - 212.06).abs() < 0.1);
    }

    #[test]
    fn test_tube_surface_area() {
        let room = RoomShape::DEFAULT_TUBE;
        // 2 * PI * 1.5 * 30 ≈ 282.74 (no end caps)
        assert!((room.surface_area().as_f32() - 282.74).abs() < 0.1);
    }

    #[test]
    fn test_tube_dimensions() {
        let room = RoomShape::DEFAULT_TUBE;
        assert!((room.length().as_f32() - 30.0).abs() < 0.01);
        assert!((room.width().as_f32() - 3.0).abs() < 0.01);
        assert!((room.height().as_f32() - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_tube_axial_modes() {
        let room = RoomShape::DEFAULT_TUBE;
        let (fl, fw, fh) = room.axial_modes();
        // length mode: 343 / 30 ≈ 11.43 (open-open)
        assert!((fl.as_f32() - 11.43).abs() < 0.01);
        // radial modes: 343 / (4*1.5) ≈ 57.17
        assert!((fw.as_f32() - 57.17).abs() < 0.01);
        assert!((fh.as_f32() - 57.17).abs() < 0.01);
    }
}
