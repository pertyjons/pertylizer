//! Interned strings for zero-allocation port name handling.
//!
//! Port names are repeated frequently during audio processing.
//! Interning them avoids repeated String allocations.
//!
//! # Thread Safety
//!
//! Pre-defined port names are cached as static constants and require
//! NO LOCKING in the audio thread. Only dynamic string interning
//! (for custom port names) requires a write lock, which should only
//! happen during module initialization, never in `process()`.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ============================================================================
// PRE-DEFINED PORT NAME IDS (compile-time constants)
// ============================================================================

// These are the IDs assigned during InternPool::new() initialization.
// They are guaranteed stable because the pool always initializes in the same order.
const ID_IN: u32 = 0;
const ID_OUT: u32 = 1;
const ID_IN_L: u32 = 2;
const ID_IN_R: u32 = 3;
const ID_OUT_L: u32 = 4;
const ID_OUT_R: u32 = 5;
const ID_FREQ: u32 = 6;
const ID_FREQ_CV: u32 = 7;
const ID_GATE: u32 = 8;
const ID_CUTOFF_CV: u32 = 9;
const ID_RESONANCE_CV: u32 = 10;
const ID_PWM: u32 = 11;
const ID_FM: u32 = 12;
const ID_PM: u32 = 13;
const ID_SYNC: u32 = 14;
const ID_LEVEL: u32 = 15;
const ID_PAN: u32 = 16;
const ID_RATE_CV: u32 = 17;
const ID_CV: u32 = 18;
const ID_PAN_CV: u32 = 19;
const ID_LEFT: u32 = 20;
const ID_RIGHT: u32 = 21;
const ID_VELOCITY: u32 = 22;
// Additional commonly used ports
const ID_PITCH: u32 = 23;
const ID_PITCH_CV: u32 = 24;
const ID_ACCENT: u32 = 25;
// Retrigger port
const ID_RETRIGGER: u32 = 26;
// Mixer input ports (for zero-allocation mixer processing)
const ID_IN1: u32 = 27;
const ID_IN2: u32 = 28;
const ID_IN3: u32 = 29;
const ID_IN4: u32 = 30;
const ID_IN5: u32 = 31;
const ID_IN6: u32 = 32;
const ID_IN7: u32 = 33;
const ID_IN8: u32 = 34;

/// Global intern pool for port names.
static INTERN_POOL: LazyLock<RwLock<InternPool>> = LazyLock::new(|| RwLock::new(InternPool::new()));

struct InternPool {
    // Store leaked 'static strings - safe because we never deallocate them
    strings: Vec<&'static str>,
    lookup: HashMap<&'static str, u32>,
}

impl InternPool {
    fn new() -> Self {
        let mut pool = Self {
            strings: Vec::with_capacity(64),
            lookup: HashMap::with_capacity(64),
        };
        // Pre-intern common port names - ORDER MATTERS! Must match ID_* constants above.
        pool.intern("in"); // 0
        pool.intern("out"); // 1
        pool.intern("in_l"); // 2
        pool.intern("in_r"); // 3
        pool.intern("out_l"); // 4
        pool.intern("out_r"); // 5
        pool.intern("freq"); // 6
        pool.intern("freq_cv"); // 7
        pool.intern("gate"); // 8
        pool.intern("cutoff_cv"); // 9
        pool.intern("resonance_cv"); // 10
        pool.intern("pwm"); // 11
        pool.intern("fm"); // 12
        pool.intern("pm"); // 13
        pool.intern("sync"); // 14
        pool.intern("level"); // 15
        pool.intern("pan"); // 16
        pool.intern("rate_cv"); // 17
        pool.intern("cv"); // 18
        pool.intern("pan_cv"); // 19
        pool.intern("left"); // 20
        pool.intern("right"); // 21
        pool.intern("velocity"); // 22
        // Additional commonly used ports
        pool.intern("pitch"); // 23
        pool.intern("pitch_cv"); // 24
        pool.intern("accent"); // 25
        // Mixer input ports
        pool.intern("in1"); // 26
        pool.intern("in2"); // 27
        pool.intern("in3"); // 28
        pool.intern("in4"); // 29
        pool.intern("in5"); // 30
        pool.intern("in6"); // 31
        pool.intern("in7"); // 32
        pool.intern("in8"); // 33
        pool
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.lookup.get(s) {
            return id;
        }
        let id = self.strings.len() as u32;
        // Leak the boxed string to get a 'static reference.
        // This is intentional for string interning - strings are never freed.
        let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
        self.lookup.insert(leaked, id);
        self.strings.push(leaked);
        id
    }

    fn get(&self, id: u32) -> Option<&'static str> {
        self.strings.get(id as usize).copied()
    }
}

/// An interned port name - Copy, cheap to compare, no allocation on clone.
///
/// # Performance
///
/// - Comparison is a single `u32` equality check (O(1), no string comparison)
/// - Pre-defined port names (IN, OUT, etc.) are compile-time constants
/// - Custom port names require a one-time interning during initialization
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PortName(u32);

impl PortName {
    // ========================================================================
    // COMPILE-TIME CONSTANTS - NO LOCKING REQUIRED
    // ========================================================================

    /// Standard input port "in".
    pub const IN: Self = Self(ID_IN);
    /// Standard output port "out".
    pub const OUT: Self = Self(ID_OUT);
    /// Left input port "in_l".
    pub const IN_L: Self = Self(ID_IN_L);
    /// Right input port "in_r".
    pub const IN_R: Self = Self(ID_IN_R);
    /// Left output port "out_l".
    pub const OUT_L: Self = Self(ID_OUT_L);
    /// Right output port "out_r".
    pub const OUT_R: Self = Self(ID_OUT_R);
    /// Frequency port "freq".
    pub const FREQ: Self = Self(ID_FREQ);
    /// Frequency CV port "freq_cv".
    pub const FREQ_CV: Self = Self(ID_FREQ_CV);
    /// Gate port "gate".
    pub const GATE: Self = Self(ID_GATE);
    /// Cutoff CV port "cutoff_cv".
    pub const CUTOFF_CV: Self = Self(ID_CUTOFF_CV);
    /// Resonance CV port "resonance_cv".
    pub const RESONANCE_CV: Self = Self(ID_RESONANCE_CV);
    /// PWM port "pwm".
    pub const PWM: Self = Self(ID_PWM);
    /// FM port "fm".
    pub const FM: Self = Self(ID_FM);
    /// PM port "pm".
    pub const PM: Self = Self(ID_PM);
    /// Sync port "sync".
    pub const SYNC: Self = Self(ID_SYNC);
    /// Level port "level".
    pub const LEVEL: Self = Self(ID_LEVEL);
    /// Pan port "pan".
    pub const PAN: Self = Self(ID_PAN);
    /// Rate CV port "rate_cv".
    pub const RATE_CV: Self = Self(ID_RATE_CV);
    /// CV port "cv".
    pub const CV: Self = Self(ID_CV);
    /// Pan CV port "pan_cv".
    pub const PAN_CV: Self = Self(ID_PAN_CV);
    /// Left output port "left".
    pub const LEFT: Self = Self(ID_LEFT);
    /// Right output port "right".
    pub const RIGHT: Self = Self(ID_RIGHT);
    /// Velocity port "velocity".
    pub const VELOCITY: Self = Self(ID_VELOCITY);
    /// Pitch port "pitch".
    pub const PITCH: Self = Self(ID_PITCH);
    /// Pitch CV port "pitch_cv".
    pub const PITCH_CV: Self = Self(ID_PITCH_CV);
    /// Accent port "accent".
    pub const ACCENT: Self = Self(ID_ACCENT);
    /// Retrigger port "retrigger".
    pub const RETRIGGER: Self = Self(ID_RETRIGGER);
    // Mixer input ports "in1" through "in8"
    /// Mixer input port 1 "in1".
    pub const IN1: Self = Self(ID_IN1);
    /// Mixer input port 2 "in2".
    pub const IN2: Self = Self(ID_IN2);
    /// Mixer input port 3 "in3".
    pub const IN3: Self = Self(ID_IN3);
    /// Mixer input port 4 "in4".
    pub const IN4: Self = Self(ID_IN4);
    /// Mixer input port 5 "in5".
    pub const IN5: Self = Self(ID_IN5);
    /// Mixer input port 6 "in6".
    pub const IN6: Self = Self(ID_IN6);
    /// Mixer input port 7 "in7".
    pub const IN7: Self = Self(ID_IN7);
    /// Mixer input port 8 "in8".
    pub const IN8: Self = Self(ID_IN8);
    /// Array of mixer input ports for iteration (IN1 through IN8).
    pub const MIXER_INPUTS: [Self; 8] = [
        Self::IN1,
        Self::IN2,
        Self::IN3,
        Self::IN4,
        Self::IN5,
        Self::IN6,
        Self::IN7,
        Self::IN8,
    ];

    // ========================================================================
    // RUNTIME INTERNING - for custom/dynamic port names only
    // ========================================================================

    /// Intern a string, returning a PortName that can be copied freely.
    ///
    /// **Note:** For standard port names, prefer the constants (e.g., `PortName::IN`)
    /// to avoid any locking overhead.
    ///
    /// # Panics
    /// Panics if the intern pool lock is poisoned.
    #[allow(clippy::unwrap_used)]
    pub fn intern(s: &str) -> Self {
        let id = INTERN_POOL.write().unwrap().intern(s);
        Self(id)
    }

    /// Get the string representation.
    ///
    /// # Panics
    /// Panics if the intern pool lock is poisoned.
    #[allow(clippy::unwrap_used)]
    pub fn as_str(&self) -> &'static str {
        INTERN_POOL.read().unwrap().get(self.0).unwrap_or("")
    }

    // ========================================================================
    // DEPRECATED METHODS - use constants instead
    // ========================================================================

    /// Standard input port.
    #[deprecated(since = "0.34.3", note = "Use PortName::IN constant instead")]
    pub fn input() -> Self {
        Self::IN
    }
    /// Standard output port.
    #[deprecated(since = "0.34.3", note = "Use PortName::OUT constant instead")]
    pub fn output() -> Self {
        Self::OUT
    }
    /// Left input port.
    #[deprecated(since = "0.34.3", note = "Use PortName::IN_L constant instead")]
    pub fn input_left() -> Self {
        Self::IN_L
    }
    /// Right input port.
    #[deprecated(since = "0.34.3", note = "Use PortName::IN_R constant instead")]
    pub fn input_right() -> Self {
        Self::IN_R
    }
    /// Left output port.
    #[deprecated(since = "0.34.3", note = "Use PortName::OUT_L constant instead")]
    pub fn output_left() -> Self {
        Self::OUT_L
    }
    /// Right output port.
    #[deprecated(since = "0.34.3", note = "Use PortName::OUT_R constant instead")]
    pub fn output_right() -> Self {
        Self::OUT_R
    }
    /// Frequency port.
    #[deprecated(since = "0.34.3", note = "Use PortName::FREQ constant instead")]
    pub fn freq() -> Self {
        Self::FREQ
    }
    /// Frequency CV port.
    #[deprecated(since = "0.34.3", note = "Use PortName::FREQ_CV constant instead")]
    pub fn freq_cv() -> Self {
        Self::FREQ_CV
    }
    /// Gate port.
    #[deprecated(since = "0.34.3", note = "Use PortName::GATE constant instead")]
    pub fn gate() -> Self {
        Self::GATE
    }
    /// Cutoff CV port.
    #[deprecated(since = "0.34.3", note = "Use PortName::CUTOFF_CV constant instead")]
    pub fn cutoff_cv() -> Self {
        Self::CUTOFF_CV
    }
}

impl PartialEq<&str> for PortName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl std::fmt::Display for PortName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for PortName {
    fn from(s: &str) -> Self {
        Self::intern(s)
    }
}

impl From<String> for PortName {
    fn from(s: String) -> Self {
        Self::intern(&s)
    }
}

impl From<&String> for PortName {
    fn from(s: &String) -> Self {
        Self::intern(s)
    }
}

impl From<PortName> for String {
    fn from(p: PortName) -> Self {
        p.as_str().to_string()
    }
}

impl Serialize for PortName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PortName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::intern(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_same_string() {
        let a = PortName::intern("test");
        let b = PortName::intern("test");
        assert_eq!(a, b);
    }

    #[test]
    fn test_intern_different_strings() {
        let a = PortName::intern("foo");
        let b = PortName::intern("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn test_as_str() {
        let name = PortName::intern("my_port");
        assert_eq!(name.as_str(), "my_port");
    }

    #[test]
    fn test_common_port_constants() {
        // Test that constants work and match the expected strings
        assert_eq!(PortName::IN.as_str(), "in");
        assert_eq!(PortName::OUT.as_str(), "out");
        assert_eq!(PortName::IN_L.as_str(), "in_l");
        assert_eq!(PortName::IN_R.as_str(), "in_r");
        assert_eq!(PortName::OUT_L.as_str(), "out_l");
        assert_eq!(PortName::OUT_R.as_str(), "out_r");
        assert_eq!(PortName::GATE.as_str(), "gate");
        assert_eq!(PortName::CV.as_str(), "cv");
    }

    #[test]
    fn test_constant_equals_interned() {
        // Constants should equal runtime-interned versions
        assert_eq!(PortName::IN, PortName::intern("in"));
        assert_eq!(PortName::OUT, PortName::intern("out"));
        assert_eq!(PortName::GATE, PortName::intern("gate"));
    }

    #[test]
    #[allow(deprecated)]
    fn test_deprecated_methods_still_work() {
        assert_eq!(PortName::input(), PortName::IN);
        assert_eq!(PortName::output(), PortName::OUT);
        assert_eq!(PortName::input_left(), PortName::IN_L);
    }
}
