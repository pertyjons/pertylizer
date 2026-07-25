//! The `Script` / `AudioScript` module's user-declared knob parameter.
//!
//! Unlike every other `*Param` (a fixed enum of named knobs), a script knob's
//! name is **declared by the program** (`param drive = 0.5`), so it cannot be a
//! compile-time enum variant. It rides as an interned [`PortName`] (a `Copy`
//! `u32` handle) plus its current value, which keeps [`Param`](super::Param)
//! `Copy` and makes `ModuleParam::name` a zero-allocation `&'static str` — the
//! intern pool leaks every string, so [`PortName::as_str`] is already `'static`.

use serde::{Deserialize, Serialize};

use crate::PortName;
use crate::module_traits::{ModuleParam, ParamKind, ParameterUnit, ResponseCurve};

/// A `Script`-module parameter. One variant today — a user-declared knob.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ScriptParam {
    /// A knob declared by the program (`param <name> = <default> [min, max]`).
    /// Carries the interned param name and its current value.
    Knob(PortName, f32),
}

impl ModuleParam for ScriptParam {
    /// Two knobs are the same kind **iff they share the interned name** — this
    /// drives descriptor-matching, so a module with several script knobs must not
    /// collapse them all onto the first descriptor entry.
    fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Knob(a, _), Self::Knob(b, _)) => a == b,
        }
    }

    /// The knob's display name — its interned identifier, returned as a
    /// `'static str` (the intern pool never frees, so this needs no allocation
    /// and no per-module name table).
    fn name(&self) -> &'static str {
        match self {
            Self::Knob(name, _) => name.as_str(),
        }
    }

    /// The stored knob value.
    fn as_f32(&self) -> f32 {
        match self {
            Self::Knob(_, v) => *v,
        }
    }

    /// Keep the interned name, replace only the value. This is what makes the
    /// MCP / automation value path free: the setter looks up the descriptor by
    /// `type_id`, takes its `id` (`Param::Script(Knob(name, default))`), and calls
    /// `.with_f32(value)`.
    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Knob(name, _) => Self::Knob(*name, value),
        }
    }

    /// A knob is a continuous f32 value.
    fn kind(&self) -> ParamKind {
        ParamKind::Continuous
    }

    /// v1 carries no unit (a later optional `param … unit` keyword can map a
    /// recognized token to the enum; anything else stays `None`).
    fn unit(&self) -> ParameterUnit {
        ParameterUnit::None
    }

    /// Linear response by default (v1 defers curve declaration).
    fn default_curve(&self) -> ResponseCurve {
        ResponseCurve::Linear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Param;

    #[test]
    fn same_kind_compares_the_interned_name() {
        let drive = PortName::intern("drive");
        let cutoff = PortName::intern("cutoff");
        // Same name, different value → same kind (descriptor matching).
        assert!(ScriptParam::Knob(drive, 0.5).same_kind(&ScriptParam::Knob(drive, 0.9)));
        // Different name → different kind (never collapse two knobs onto one).
        assert!(!ScriptParam::Knob(drive, 0.5).same_kind(&ScriptParam::Knob(cutoff, 0.5)));
    }

    #[test]
    fn with_f32_keeps_name_replaces_value() {
        let name = PortName::intern("amt");
        let q = ScriptParam::Knob(name, 0.1).with_f32(0.75);
        assert_eq!(q, ScriptParam::Knob(name, 0.75));
        assert_eq!(q.name(), "amt");
        assert_eq!(q.as_f32(), 0.75);
    }

    #[test]
    fn aggregate_param_stays_copy_and_delegates() {
        // `Param` must stay `Copy` — the knob name rides as an interned handle.
        fn assert_copy<T: Copy>(_: T) {}
        let p = Param::Script(ScriptParam::Knob(PortName::intern("drive"), 0.5));
        assert_copy(p);
        assert_eq!(p.name(), "drive");
        assert_eq!(p.as_f32(), 0.5);
        assert_eq!(p.with_f32(0.25).as_f32(), 0.25);
        assert!(p.same_kind(&Param::Script(ScriptParam::Knob(
            PortName::intern("drive"),
            9.0
        ))));
        assert!(!p.same_kind(&Param::Script(ScriptParam::Knob(
            PortName::intern("other"),
            0.5
        ))));
    }
}
