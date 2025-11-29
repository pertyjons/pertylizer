//! Macros for reducing boilerplate in newtype implementations.
//!
//! These macros automate the implementation of common arithmetic traits
//! for the domain-specific types in this module.

/// Implement `Add` and `Sub` for a newtype (T + T = T, T - T = T).
#[macro_export]
macro_rules! impl_additive {
    ($type:ty) => {
        impl std::ops::Add for $type {
            type Output = Self;

            #[inline]
            fn add(self, rhs: Self) -> Self::Output {
                Self(self.0 + rhs.0)
            }
        }

        impl std::ops::Sub for $type {
            type Output = Self;

            #[inline]
            fn sub(self, rhs: Self) -> Self::Output {
                Self(self.0 - rhs.0)
            }
        }
    };
}

/// Implement `Mul<f32>`, `f32 * T`, and `Div<f32>` for scaling a newtype.
#[macro_export]
macro_rules! impl_scaling {
    ($type:ty) => {
        impl std::ops::Mul<f32> for $type {
            type Output = Self;

            #[inline]
            fn mul(self, rhs: f32) -> Self::Output {
                Self(self.0 * rhs)
            }
        }

        impl std::ops::Mul<$type> for f32 {
            type Output = $type;

            #[inline]
            fn mul(self, rhs: $type) -> Self::Output {
                <$type>::new(self * rhs.0)
            }
        }

        impl std::ops::Div<f32> for $type {
            type Output = Self;

            #[inline]
            fn div(self, rhs: f32) -> Self::Output {
                Self(self.0 / rhs)
            }
        }
    };
}

/// Implement `Div<T>` for a newtype that returns f32 (ratio).
#[macro_export]
macro_rules! impl_ratio {
    ($type:ty) => {
        impl std::ops::Div<$type> for $type {
            type Output = f32;

            #[inline]
            fn div(self, rhs: $type) -> Self::Output {
                self.0 / rhs.0
            }
        }
    };
}

/// Implement `From<f32>` and `From<T> for f32` conversions.
#[macro_export]
macro_rules! impl_float_conversions {
    ($type:ty) => {
        impl From<f32> for $type {
            #[inline]
            fn from(value: f32) -> Self {
                Self(value)
            }
        }

        impl From<$type> for f32 {
            #[inline]
            fn from(value: $type) -> Self {
                value.0
            }
        }
    };
}

/// Implement `From<f32>` with clamping via `new()` and `From<T> for f32`.
#[macro_export]
macro_rules! impl_float_conversions_clamped {
    ($type:ty) => {
        impl From<f32> for $type {
            #[inline]
            fn from(value: f32) -> Self {
                Self::new(value)
            }
        }

        impl From<$type> for f32 {
            #[inline]
            fn from(value: $type) -> Self {
                value.0
            }
        }
    };
}

/// Implement all standard arithmetic traits for a simple newtype.
/// Combines: impl_additive!, impl_scaling!, impl_ratio!, impl_float_conversions!
#[macro_export]
macro_rules! impl_newtype_arithmetic {
    ($type:ty) => {
        $crate::impl_additive!($type);
        $crate::impl_scaling!($type);
        $crate::impl_ratio!($type);
        $crate::impl_float_conversions!($type);
    };
}

// Note: Macros are exported via #[macro_export] and available via crate::macro_name!
