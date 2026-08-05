//! Monotonic edit counters for mutable state.
//!
//! A [`ContentRevision`] answers one question cheaply: *has this changed since
//! the last time I looked?* Owners of mutable state ([`crate::types`] does not
//! define any itself — see `SharedSong` and `SampleLibrary`) bump their revision
//! whenever they are mutated, and observers compare a stored value against the
//! current one instead of deep-comparing or hashing the state.
//!
//! Being a plain counter rather than a content hash means it can report a change
//! that did not alter the content, but never miss one that did — the safe
//! direction for the unsaved-changes prompt that consumes it.

/// A monotonic counter identifying one version of some mutable state.
///
/// Only equality is meaningful across different state owners: two revisions from
/// *different* sources say nothing about each other, and the ordering of raw
/// values is an implementation detail of whoever issues them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[must_use]
pub struct ContentRevision(u64);

impl ContentRevision {
    /// The revision of freshly created, never-mutated state.
    pub const INITIAL: Self = Self(0);

    /// Wrap a raw counter value.
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw counter value, for storing in an atomic or a snapshot.
    #[inline]
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// The next revision in sequence.
    ///
    /// Wraps at `u64::MAX`, which is unreachable in practice: at one edit per
    /// nanosecond it would take over 500 years, and a wrap could at worst make
    /// one comparison miss a change.
    #[inline]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_is_zero_and_default() {
        assert_eq!(ContentRevision::INITIAL, ContentRevision::default());
        assert_eq!(ContentRevision::INITIAL.as_u64(), 0);
    }

    #[test]
    fn next_advances_and_compares_unequal() {
        let first = ContentRevision::INITIAL;
        let second = first.next();
        assert_ne!(first, second);
        assert_eq!(second.as_u64(), 1);
    }

    #[test]
    fn wraps_instead_of_overflowing() {
        assert_eq!(
            ContentRevision::new(u64::MAX).next(),
            ContentRevision::INITIAL,
        );
    }
}
