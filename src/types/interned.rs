//! Interned strings for zero-allocation port name handling.
//!
//! Port names are repeated frequently during audio processing.
//! Interning them avoids repeated String allocations.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Global intern pool for port names.
static INTERN_POOL: LazyLock<RwLock<InternPool>> = LazyLock::new(|| {
    RwLock::new(InternPool::new())
});

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
        // Pre-intern common port names
        pool.intern("in");
        pool.intern("out");
        pool.intern("in_l");
        pool.intern("in_r");
        pool.intern("out_l");
        pool.intern("out_r");
        pool.intern("freq");
        pool.intern("freq_cv");
        pool.intern("gate");
        pool.intern("cutoff_cv");
        pool.intern("resonance_cv");
        pool.intern("pwm");
        pool.intern("fm");
        pool.intern("pm");
        pool.intern("sync");
        pool.intern("level");
        pool.intern("pan");
        pool.intern("rate_cv");
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PortName(u32);

impl PortName {
    /// Intern a string, returning a PortName that can be copied freely.
    pub fn intern(s: &str) -> Self {
        let id = INTERN_POOL.write().unwrap().intern(s);
        Self(id)
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        INTERN_POOL
            .read()
            .unwrap()
            .get(self.0)
            .unwrap_or("")
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

// Common port names as constants (interned at first use)
impl PortName {
    /// Standard input port.
    pub fn input() -> Self { Self::intern("in") }
    /// Standard output port.
    pub fn output() -> Self { Self::intern("out") }
    /// Left input port.
    pub fn input_left() -> Self { Self::intern("in_l") }
    /// Right input port.
    pub fn input_right() -> Self { Self::intern("in_r") }
    /// Left output port.
    pub fn output_left() -> Self { Self::intern("out_l") }
    /// Right output port.
    pub fn output_right() -> Self { Self::intern("out_r") }
    /// Frequency port.
    pub fn freq() -> Self { Self::intern("freq") }
    /// Frequency CV port.
    pub fn freq_cv() -> Self { Self::intern("freq_cv") }
    /// Gate port.
    pub fn gate() -> Self { Self::intern("gate") }
    /// Cutoff CV port.
    pub fn cutoff_cv() -> Self { Self::intern("cutoff_cv") }
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
    fn test_common_ports() {
        assert_eq!(PortName::input().as_str(), "in");
        assert_eq!(PortName::output().as_str(), "out");
        assert_eq!(PortName::input_left().as_str(), "in_l");
    }
}
