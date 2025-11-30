//! Sub-oscillator parameter types.

use serde::{Deserialize, Serialize};

/// Sub-oscillator parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubOscParam {
    /// Waveform selection (Sine, Square, Pulse25)
    Waveform,
    /// Octave transposition (-1 or -2)
    Octave,
    /// Output level (0.0 to 1.0)
    Level,
}
