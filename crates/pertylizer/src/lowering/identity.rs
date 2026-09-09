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
//! The address is therefore **computed from the identity alone**: the instrument's own
//! identity, then the module type's position in its own declaration paired with the instance
//! number, which is exactly what a [`ModuleId`] is. Nothing about the patch's contents
//! enters, so adding, removing or reordering modules — or instruments — leaves every other
//! address where it was. The assigned number is an address inside one plan; it is never
//! persisted and never compared across two lowerings.
//!
//! # The address space, since a plan holds a whole project (`P08-S002`)
//!
//! A [`NodeId`] is thirty-two bits, laid out so that no two lowered objects can meet:
//!
//! - bit 31 set is a **Mod Grid node**, global to the song (`modulation::grid_node_address`);
//! - bits 24–30 are the **instrument slot**, the instrument's persisted identity itself,
//!   which must therefore be below [`InstrumentSlot::MASTER`] — a project naming a higher
//!   identity is refused by name rather than folded into another's addresses;
//! - bits 16–22 are the **module type**, whose seventy-odd variants never reach `0xFF`, so
//!   a module-type field of `0xFF` marks a node the lowerer **inserts** for that instrument
//!   — the velocity scaler, the balance stage, the channel, the clipper, the macro sources —
//!   in the low sixteen bits;
//! - the slot [`InstrumentSlot::MASTER`] with the inserted-node mark is the **master**: the
//!   sum, the trim, the clamp and the plan's one output.
//!
//! Before this, the inserted channel sat at `0xFFFF_0001`, which was also the velocity
//! macro's address: a patch reading velocity through its Mod Matrix could not lower beside a
//! channel. The per-instrument range closes that by construction.

use std::collections::BTreeMap;

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine::instrument::InstrumentId;
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

    /// An instrument's persisted identity lies outside the address space's instrument field.
    ///
    /// Refused rather than hashed or ranked into it: a rank would repoint every other
    /// instrument's addresses when one is added, and a hash could meet another's.
    #[error("instrument {instrument} lies outside the addressable range of {limit} instruments")]
    InstrumentOutOfRange {
        /// The identity that does not fit.
        instrument: InstrumentId,
        /// How many identities do.
        limit: u64,
    },
}

/// One instrument's place in the plan's address space: its persisted identity, checked to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct InstrumentSlot(u8);

impl InstrumentSlot {
    /// The slot the master nodes share; no instrument may take it.
    pub const MASTER: Self = Self(0x7F);

    /// Where the instrument field sits in a [`NodeId`].
    const SHIFT: u32 = 24;
    /// The module-type field value that marks a node the lowerer inserts.
    const INSERTED: u32 = 0x00FF_0000;

    /// An instrument's slot, or the refusal naming it.
    pub fn of(instrument: InstrumentId) -> Result<Self, IdentityError> {
        let limit = u64::from(Self::MASTER.0);
        match u8::try_from(instrument.as_u64()) {
            Ok(slot) if u64::from(slot) < limit => Ok(Self(slot)),
            _ => Err(IdentityError::InstrumentOutOfRange { instrument, limit }),
        }
    }

    /// The address of one of this instrument's saved modules.
    fn module(self, id: ModuleId) -> NodeId {
        NodeId::new(
            ((self.0 as u32) << Self::SHIFT)
                | ((id.module_type as u32) << 16)
                | u32::from(id.instance),
        )
    }

    /// The address of a node the lowerer inserts for this instrument, by tag.
    const fn inserted(self, tag: u16) -> NodeId {
        NodeId::new(((self.0 as u32) << Self::SHIFT) | Self::INSERTED | tag as u32)
    }

    /// The voice-output velocity stage (ADR-0059).
    pub const fn voice_output_scaler(self) -> NodeId {
        self.inserted(0)
    }

    /// The balance stage V1's track control lowers to (`P08-S002`).
    pub const fn balance(self) -> NodeId {
        self.inserted(1)
    }

    /// The mix channel (`P08-S001`).
    pub const fn channel(self) -> NodeId {
        self.inserted(2)
    }

    /// V1's channel-stage soft clipper (`P08-S002`).
    pub const fn soft_clip(self) -> NodeId {
        self.inserted(3)
    }

    /// One of V1's six Mod Matrix macro sources (`P07-S004`), by its one-based tag.
    pub const fn macro_source(self, tag: u16) -> NodeId {
        self.inserted(0x10 + tag)
    }
}

/// The master sum every instrument's channel feeds (`P08-S002`).
pub const MASTER_MIX: NodeId = InstrumentSlot::MASTER.inserted(0);
/// The master trim, V1's master volume (`P08-S002`).
pub const MASTER_TRIM: NodeId = InstrumentSlot::MASTER.inserted(1);
/// V1's output clamp, under the parity policy (`P08-S002`).
pub const MASTER_CLAMP: NodeId = InstrumentSlot::MASTER.inserted(2);
/// The plan's one output (`P08-S002`).
pub const MASTER_OUTPUT: NodeId = InstrumentSlot::MASTER.inserted(3);

/// The two-way mapping between a patch's saved module identities and one plan's node
/// identities.
///
/// Built once per instrument graph. Every later stage of lowering addresses nodes through
/// this and never through a string.
#[derive(Debug, Clone)]
#[must_use]
pub struct ResolvedIdentities {
    /// The instrument every address below belongs to.
    slot: InstrumentSlot,
    /// Saved identity to plan address. `BTreeMap` rather than `HashMap` because the
    /// iteration order is what assigns the addresses, and it has to be the same on every
    /// run for the plan to be deterministic.
    to_node: BTreeMap<ModuleId, NodeId>,
    /// Plan address back to saved identity, so a diagnostic can name the project object.
    to_module: BTreeMap<NodeId, ModuleId>,
}

impl Default for ResolvedIdentities {
    /// An empty table for the first instrument slot, which is what a refused resolution
    /// hands back beside its diagnostic.
    fn default() -> Self {
        Self {
            slot: InstrumentSlot(0),
            to_node: BTreeMap::new(),
            to_module: BTreeMap::new(),
        }
    }
}

impl ResolvedIdentities {
    /// Resolve every module in one instrument's patch.
    ///
    /// The whole patch is resolved before anything is lowered, so a graph is never half
    /// built when an unparsable identity is found. The instrument's own identity is the
    /// address's high field, checked to fit first.
    pub fn resolve(
        instrument: InstrumentId,
        modules: &[ModuleState],
    ) -> Result<Self, IdentityError> {
        let slot = InstrumentSlot::of(instrument)?;
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
            let node = slot.module(*id);
            to_node.insert(*id, node);
            to_module.insert(node, *id);
        }
        Ok(Self {
            slot,
            to_node,
            to_module,
        })
    }

    /// The instrument's slot, for the nodes the lowerer inserts beside its modules.
    pub const fn slot(&self) -> InstrumentSlot {
        self.slot
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
