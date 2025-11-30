//! Noise generator parameter types.

use serde::{Deserialize, Serialize};

/// Noise generator parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoiseParam {
    /// Noise type/color (White, Pink, Brown, Blue, Violet)
    Type,
    /// Output level (0.0 to 1.0)
    Level,
}
