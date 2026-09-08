//! Typed controller addresses and note-source vocabulary (`P07-S004`).

use crate::plan::{CompiledPlan, ParameterSlot};
use crate::quantities::{ParameterValue, QuantityError};

/// A MIDI continuous-controller number, including CC 1 (the mod wheel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct MidiController(u8);
impl MidiController {
    /// Reject numbers outside MIDI's seven-bit domain.
    pub const fn new(value: u8) -> Result<Self, QuantityError> {
        if value < 128 {
            Ok(Self(value))
        } else {
            Err(QuantityError::OutOfRange {
                quantity: "MidiController",
                value: value as u64,
                maximum: 127,
            })
        }
    }
    /// The protocol number.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// A controller source's semantic role; adapter-side MIDI routing belongs to Phase 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerKind {
    /// MIDI CC 1.
    ModWheel,
    /// Channel pressure.
    Aftertouch,
    /// Normalized bipolar channel bend; edge depth supplies the range in semitones.
    PitchBend,
    /// A continuous controller.
    MidiCc(MidiController),
}

/// A value in the closed bipolar interval.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct BipolarLevel(f32);
impl BipolarLevel {
    /// The neutral value.
    pub const ZERO: Self = Self(0.0);
    /// Reject nonfinite or out-of-domain input.
    pub fn new(value: f32) -> Result<Self, QuantityError> {
        if !value.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "BipolarLevel",
                value,
            });
        }
        if !(-1.0..=1.0).contains(&value) {
            return Err(QuantityError::OutsideInterval {
                quantity: "BipolarLevel",
                value,
                minimum: -1.0,
                maximum: 1.0,
            });
        }
        Ok(Self(value))
    }
    /// The normalized value.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
    pub(crate) fn parameter(self) -> ParameterValue {
        ParameterValue::saturating(self.0)
    }
}

/// A note's control-rate source, independent of its sample-positioned magnitudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteSource {
    /// The struck velocity.
    Velocity,
    /// The key divided by 127, as V1's macro reads it.
    NoteNumber,
    /// Polyphonic pressure, initially zero.
    Pressure,
    /// Release velocity, initially zero.
    ReleaseVelocity,
}

/// An expression addressed to one live occurrence. Release velocity is published before
/// that occurrence's off edge, at the same position, so it can address the live identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteExpression {
    /// Polyphonic pressure.
    Pressure(crate::quantities::NormalizedLevel),
    /// Velocity of the release edge.
    ReleaseVelocity(crate::quantities::NormalizedLevel),
}
impl NoteExpression {
    pub(crate) const fn source(self) -> NoteSource {
        match self {
            Self::Pressure(_) => NoteSource::Pressure,
            Self::ReleaseVelocity(_) => NoteSource::ReleaseVelocity,
        }
    }
    pub(crate) const fn value(self) -> ParameterValue {
        match self {
            Self::Pressure(v) | Self::ReleaseVelocity(v) => ParameterValue::from_level(v),
        }
    }
}

/// An admitted controller address. Construction checks the kind's declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ControllerSlot {
    slot: ParameterSlot,
    kind: ControllerKind,
}
impl ControllerSlot {
    /// The underlying source parameter, for automation of its resting value.
    pub const fn parameter(self) -> ParameterSlot {
        self.slot
    }
    /// The controller that this source represents.
    pub const fn kind(self) -> ControllerKind {
        self.kind
    }
    /// A replacement-layer write. `None` removes the replacement and reveals automation.
    pub fn change(self, value: Option<BipolarLevel>) -> Result<ControllerChange, QuantityError> {
        if let Some(value) = value
            && self.kind != ControllerKind::PitchBend
            && value.as_f32() < 0.0
        {
            return Err(QuantityError::OutsideInterval {
                quantity: "unipolar controller",
                value: value.as_f32(),
                minimum: 0.0,
                maximum: 1.0,
            });
        }
        Ok(ControllerChange {
            slot: self,
            value: value.map(BipolarLevel::parameter),
        })
    }
}

/// A controller write validated against its destination, ready for existing event queues.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ControllerChange {
    slot: ControllerSlot,
    value: Option<ParameterValue>,
}
impl ControllerChange {
    /// The declared source that receives this replacement.
    pub const fn slot(self) -> ControllerSlot {
        self.slot
    }
    pub(crate) const fn value(self) -> Option<ParameterValue> {
        self.value
    }
}
impl CompiledPlan {
    /// Resolve only a controller source. Ordinary parameters have no controller layer.
    pub fn resolve_controller(&self, node: crate::ir::NodeId) -> Option<ControllerSlot> {
        let slot = self.resolve_parameter(node, crate::ir::parameters::SOURCE_VALUE)?;
        let target = self.parameter_targets().get(slot.index())?;
        if !target.controller {
            return None;
        }
        let crate::node::kernels::PreparedNode::Controller { kind } =
            self.prepared_for_node(target.node)?
        else {
            return None;
        };
        Some(ControllerSlot { slot, kind: *kind })
    }
}

/// An activation's complete controller-source layers, charged as one catch-up event.
/// Only stream preparation constructs this: ordinary writers cannot install a controller
/// layer on a parameter whose declaration does not support one.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ControllerRestore {
    pub(crate) slot: ControllerSlot,
    pub(crate) override_value: ParameterValue,
    pub(crate) controller: Option<ParameterValue>,
}
