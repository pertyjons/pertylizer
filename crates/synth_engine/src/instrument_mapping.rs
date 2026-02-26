//! Bidirectional mapping between sequencer and engine instrument IDs.
//!
//! `SeqInstrumentId(u16)` is compact and used in pattern/note data.
//! `InstrumentId(u64)` is unique and stable for the engine's lifetime.
//!
//! Convention: `SeqInstrumentId(X)` maps to `InstrumentId(X)` (identity).
//! The mapping formalises this so `route_sequencer_events` doesn't rely
//! on fragile vec-index casting.

use synth_sequencer::SeqInstrumentId;

use crate::instrument::InstrumentId;

/// Bidirectional map between sequencer and engine instrument IDs.
///
/// Kept on `SynthEngine` (audio thread) and rebuilt whenever instruments
/// are added or removed.  Look-ups are O(n) where n ≤ 16 in practice.
#[derive(Debug, Clone, Default)]
pub struct InstrumentMapping {
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    seq: SeqInstrumentId,
    engine: InstrumentId,
}

impl InstrumentMapping {
    /// Create an empty mapping.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(16),
        }
    }

    /// Register a mapping.  If the `seq_id` already exists it is replaced.
    pub fn insert(&mut self, seq_id: SeqInstrumentId, engine_id: InstrumentId) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.seq == seq_id) {
            e.engine = engine_id;
        } else {
            self.entries.push(Entry {
                seq: seq_id,
                engine: engine_id,
            });
        }
    }

    /// Remove any entry that references the given engine ID.
    pub fn remove_by_engine_id(&mut self, engine_id: InstrumentId) {
        self.entries.retain(|e| e.engine != engine_id);
    }

    /// Look up the engine `InstrumentId` for a sequencer ID.
    #[must_use]
    pub fn engine_id(&self, seq_id: SeqInstrumentId) -> Option<InstrumentId> {
        self.entries
            .iter()
            .find(|e| e.seq == seq_id)
            .map(|e| e.engine)
    }

    /// Look up the sequencer ID for an engine `InstrumentId`.
    #[must_use]
    pub fn seq_id(&self, engine_id: InstrumentId) -> Option<SeqInstrumentId> {
        self.entries
            .iter()
            .find(|e| e.engine == engine_id)
            .map(|e| e.seq)
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the mapping is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_lookup() {
        let mut map = InstrumentMapping::new();
        map.insert(SeqInstrumentId(0), InstrumentId::FIRST);
        map.insert(SeqInstrumentId(1), InstrumentId::new(1));

        assert_eq!(map.engine_id(SeqInstrumentId(0)), Some(InstrumentId::FIRST));
        assert_eq!(
            map.engine_id(SeqInstrumentId(1)),
            Some(InstrumentId::new(1))
        );
        assert_eq!(map.engine_id(SeqInstrumentId(99)), None);
    }

    #[test]
    fn test_reverse_lookup() {
        let mut map = InstrumentMapping::new();
        map.insert(SeqInstrumentId(0), InstrumentId::FIRST);
        map.insert(SeqInstrumentId(5), InstrumentId::new(5));

        assert_eq!(map.seq_id(InstrumentId::FIRST), Some(SeqInstrumentId(0)));
        assert_eq!(map.seq_id(InstrumentId::new(5)), Some(SeqInstrumentId(5)));
        assert_eq!(map.seq_id(InstrumentId::new(99)), None);
    }

    #[test]
    fn test_remove_by_engine_id() {
        let mut map = InstrumentMapping::new();
        map.insert(SeqInstrumentId(0), InstrumentId::FIRST);
        map.insert(SeqInstrumentId(1), InstrumentId::new(1));

        map.remove_by_engine_id(InstrumentId::new(1));

        assert_eq!(map.engine_id(SeqInstrumentId(1)), None);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_replace_existing() {
        let mut map = InstrumentMapping::new();
        map.insert(SeqInstrumentId(0), InstrumentId::FIRST);
        map.insert(SeqInstrumentId(0), InstrumentId::new(99));

        assert_eq!(
            map.engine_id(SeqInstrumentId(0)),
            Some(InstrumentId::new(99))
        );
        assert_eq!(map.len(), 1);
    }
}
