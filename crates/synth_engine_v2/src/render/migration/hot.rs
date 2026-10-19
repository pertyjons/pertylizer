//! Two-pass preflight, small Copy records and history-owner transfers. No final drop.
use super::{GlobalMigrationError, PreparedGlobalMigration, PreparedRenderer};
use crate::time::{QUANTUM_FRAMES, SampleTime};

impl PreparedGlobalMigration {
    /// Validate every record before mutating either renderer or the replacement anchor.
    pub(crate) fn transfer_into(
        &self,
        old: &mut PreparedRenderer,
        new: &mut PreparedRenderer,
    ) -> Result<(), GlobalMigrationError> {
        self.preflight_into(old, new)?;
        self.install_into(old, new);
        Ok(())
    }

    pub(crate) fn preflight_into(
        &self,
        old: &PreparedRenderer,
        new: &PreparedRenderer,
    ) -> Result<(), GlobalMigrationError> {
        if old.plan.id() != self.old || new.plan.id() != self.new {
            return Err(GlobalMigrationError::Origin);
        }
        if old.carry_frames != 0
            || new.clock != SampleTime::ZERO
            || new.carry_frames != QUANTUM_FRAMES as usize
        {
            return Err(GlobalMigrationError::Boundary);
        }
        if self.nodes.len() != self.destinations.len() {
            return Err(GlobalMigrationError::Storage);
        }
        let mut previous = None;
        for (destination, _) in &self.destinations {
            if previous.is_some_and(|previous| previous >= *destination) {
                return Err(GlobalMigrationError::Storage);
            }
            previous = Some(*destination);
        }
        let mut previous = None;
        for pair in &self.nodes {
            if previous.is_some_and(|previous| previous >= pair.old) {
                return Err(GlobalMigrationError::Storage);
            }
            previous = Some(pair.old);
            if self.destinations.get(pair.inverse_index) != Some(&(pair.new, pair.old)) {
                return Err(GlobalMigrationError::Storage);
            }
            old.node_states
                .get(pair.old.index())
                .ok_or(GlobalMigrationError::Storage)?;
            new.node_states
                .get(pair.new.index())
                .ok_or(GlobalMigrationError::Storage)?;
            let source = old
                .history
                .get(pair.old.index())
                .ok_or(GlobalMigrationError::Storage)?;
            let destination = new
                .history
                .get(pair.new.index())
                .ok_or(GlobalMigrationError::Storage)?;
            if source.len() != pair.history_len || destination.len() != pair.history_len {
                return Err(GlobalMigrationError::Storage);
            }
        }
        for pair in &self.parameters {
            old.parameter_slots
                .get(pair.old.index())
                .ok_or(GlobalMigrationError::Storage)?;
            new.parameter_slots
                .get(pair.new.index())
                .ok_or(GlobalMigrationError::Storage)?;
        }
        Ok(())
    }

    /// The immutable descriptor and both extents were checked without mutation.
    pub(crate) fn install_into(&self, old: &mut PreparedRenderer, new: &mut PreparedRenderer) {
        // No fallible operation follows this point. Extents stay fixed, and the
        // checked bijection ensures every history owner moves exactly once. The
        // outgoing retirement receives fresh buffers; no payload is copied or freed.
        // Each index below was checked above. Swapping Box owners never changes
        // either table's length; the immutable map cannot change between passes.
        for pair in &self.nodes {
            new.node_states[pair.new.index()] = old.node_states[pair.old.index()];
            core::mem::swap(
                &mut old.history[pair.old.index()],
                &mut new.history[pair.new.index()],
            );
        }
        for pair in &self.parameters {
            new.parameter_slots[pair.new.index()] = old.parameter_slots[pair.old.index()];
        }
    }
}
