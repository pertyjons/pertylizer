//! Resolving a saved project's identities into the typed identities V2 admits.
//!
//! Phase 4's work list requires that "V2 IR must only contain stable typed identities after
//! lowering". A saved project does not have those. It addresses a module by the string
//! `"osc-1"` and a port by the string `"out"`, and both spellings live in
//! [`crate::patch::ConnectionState`], which stores a pair of `(String, String)` tuples. This
//! module is where those strings stop.
//!
//! # Why the mapping is a table rather than an encoding
//!
//! A [`NodeId`] could be derived arithmetically from a [`ModuleId`] — the type prefix and the
//! instance number would fit a `u32` between them. That is rejected for one reason: the
//! derivation would be one-way in practice, and a diagnostic that reports a `NodeId` must be
//! able to name the **project object** it came from, which the Phase 4 exit gate requires in
//! as many words. A table answers both directions by construction.
//!
//! # Why assignment is arithmetic rather than by rank
//!
//! `AGENTS.md` forbids collection position as identity, and forbids it twice over: not only
//! must reordering the array change nothing, a stable identity must also stay distinct from
//! an ordering position. An earlier revision sorted by [`ModuleId`] and assigned ranks, which
//! satisfies the first and fails the second — inserting a module that sorts first shifts every
//! rank behind it, so an unrelated insertion silently repoints every other identity. An
//! independent review caught it.
//!
//! The address is therefore **computed from the identity alone**: the module type's position
//! in its own declaration paired with the instance number, which is exactly what a
//! [`ModuleId`] is. Nothing about the patch's contents enters, so adding, removing or
//! reordering modules leaves every other address where it was. The assigned number is an
//! address inside one plan; it is never persisted and never compared across two lowerings.

use std::collections::BTreeMap;

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine_v2::ir::NodeId;
use thiserror::Error;

use crate::patch::ModuleState;

/// A saved identity that could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// A module's saved `id` string is not a well-formed module identity.
    ///
    /// Carried as the original spelling plus the parser's own message, because the
    /// diagnostic has to name the project object and `"osc-"` names nothing on its own.
    #[error("module id {spelling:?} does not parse: {reason}")]
    UnparsableModule {
        /// The `id` field exactly as the project spells it.
        spelling: String,
        /// Why `ModuleId`'s parser refused it.
        reason: String,
    },

    /// Two modules in one patch claim one identity.
    ///
    /// Refused rather than resolved to whichever came last: a connection naming `"osc-1"`
    /// would then reach a node chosen by array order, which is exactly the positional
    /// identity this module exists to remove.
    #[error("module {id} is declared twice in one patch")]
    DuplicateModule {
        /// The repeated identity.
        id: ModuleId,
    },

    /// A module's `id` string and its `module_type` field disagree.
    ///
    /// A saved module states its type twice — once inside the id, as the prefix `ModuleId`
    /// parses, and once in its own field. `"osc-1"` typed `Filter` is neither, and admitting
    /// it would let the two halves of lowering disagree about what the node is: the address
    /// would come from the prefix and the node kind from the field. V1's own loader already
    /// refuses this shape, so accepting it here would lose a diagnostic the project already
    /// has.
    #[error("module {spelling:?} is declared as {declared:?} but its id names {named:?}")]
    TypeMismatch {
        /// The `id` field exactly as the project spells it.
        spelling: String,
        /// The type the `module_type` field declares.
        declared: ModuleType,
        /// The type the id's prefix names.
        named: ModuleType,
    },
}

/// The two-way mapping between a patch's saved module identities and one plan's node
/// identities.
///
/// Built once per instrument graph. Every later stage of lowering addresses nodes through
/// this and never through a string.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ResolvedIdentities {
    /// Saved identity to plan address. `BTreeMap` rather than `HashMap` because the
    /// iteration order is what assigns the addresses, and it has to be the same on every
    /// run for the plan to be deterministic.
    to_node: BTreeMap<ModuleId, NodeId>,
    /// Plan address back to saved identity, so a diagnostic can name the project object.
    to_module: BTreeMap<NodeId, ModuleId>,
}

/// The plan address a module identity computes to.
///
/// Injective, and a function of the identity alone — the patch's contents do not enter, which
/// is what makes an address survive an unrelated module being added or removed.
///
/// The two halves cannot collide. `ModuleType` is a fieldless enum of 75 variants, so its
/// discriminant needs seven bits and is shifted clear of the instance number, which is a
/// `u16` and occupies exactly the low sixteen. The cast is of an enum discriminant rather
/// than of a domain quantity, and it is exact for the same reason.
fn address_of(id: ModuleId) -> NodeId {
    NodeId::new(((id.module_type as u32) << 16) | u32::from(id.instance))
}

impl ResolvedIdentities {
    /// Resolve every module in one patch.
    ///
    /// The whole patch is resolved before anything is lowered, so a graph is never half
    /// built when an unparsable identity is found.
    pub fn resolve(modules: &[ModuleState]) -> Result<Self, IdentityError> {
        let mut parsed = BTreeMap::new();
        for module in modules {
            let id: ModuleId =
                module
                    .id
                    .parse()
                    .map_err(|reason| IdentityError::UnparsableModule {
                        spelling: module.id.clone(),
                        reason,
                    })?;
            if id.module_type != module.module_type {
                return Err(IdentityError::TypeMismatch {
                    spelling: module.id.clone(),
                    declared: module.module_type,
                    named: id.module_type,
                });
            }
            if parsed.insert(id, ()).is_some() {
                return Err(IdentityError::DuplicateModule { id });
            }
        }

        let mut to_node = BTreeMap::new();
        let mut to_module = BTreeMap::new();
        for id in parsed.keys() {
            let node = address_of(*id);
            to_node.insert(*id, node);
            to_module.insert(node, *id);
        }
        Ok(Self { to_node, to_module })
    }

    /// The plan address of a saved module identity, if the patch declared it.
    ///
    /// `None` is the answer a connection naming a module the patch does not contain gets,
    /// and the caller turns it into a diagnostic rather than skipping the edge.
    #[must_use]
    pub fn node_for(&self, id: ModuleId) -> Option<NodeId> {
        self.to_node.get(&id).copied()
    }

    /// The saved module identity behind a plan address.
    ///
    /// This is the direction a diagnostic needs: the exit gate requires an unsupported
    /// target to be named as a project object rather than as a plan-internal number.
    #[must_use]
    pub fn module_for(&self, node: NodeId) -> Option<ModuleId> {
        self.to_module.get(&node).copied()
    }

    /// How many modules were resolved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.to_node.len()
    }

    /// Whether the patch declared no modules at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to_node.is_empty()
    }

    /// Every resolved pair, in plan-address order.
    pub fn pairs(&self) -> impl Iterator<Item = (ModuleId, NodeId)> + '_ {
        self.to_module.iter().map(|(node, id)| (*id, *node))
    }
}
