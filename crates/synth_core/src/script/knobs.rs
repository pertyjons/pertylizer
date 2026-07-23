//! Per-voice knob store for a script module (`Script` / `AudioScript`).
//!
//! A fixed-size, heap-free store sized to the [`SCRIPT_MAX_PARAMS`] ceiling so
//! that installing an edited script ([`remap`](ScriptParams::remap)) reindexes
//! it **in place** with no audio-thread allocation. The descriptor lists only the
//! `len` *active* knobs, never 32 empty ones, so the cap stays invisible in the
//! GUI / save / MCP surface. Each active slot holds its interned name, current
//! (stored / automated) value, and this block's accumulated mod-offset.

use crate::PortName;
use crate::module_traits::ParameterDescriptor;
use crate::params::{Param, ScriptParam};
use crate::script::{SCRIPT_MAX_PARAMS, ScriptParamDecl};

/// A script module's user-knob store: `len` active knobs in fixed 32-slot arrays.
#[derive(Debug, Clone)]
pub struct ScriptParams {
    /// Number of active knobs — slots `0..len` are live.
    len: usize,
    /// Slot → interned knob name (matched by `set`/`get`/`effective` on the
    /// audio thread via a lock-free `u32` compare).
    names: [PortName; SCRIPT_MAX_PARAMS],
    /// Slot → the knob's cached `'static` name string, so `set_mod_offset`'s
    /// `&str` target match is a lock-free `str` compare (never a per-block
    /// `PortName::as_str` intern-pool read-lock).
    name_strs: [&'static str; SCRIPT_MAX_PARAMS],
    /// Slot → stored (saved) knob value.
    values: [f32; SCRIPT_MAX_PARAMS],
    /// Slot → transient sequencer-automation override, replacing the stored value
    /// while set (`Some`); cleared on transport stop. The base value is never
    /// mutated by automation, so a project saved mid-playback keeps its knob.
    overrides: [Option<f32>; SCRIPT_MAX_PARAMS],
    /// Slot → this block's accumulated mod-matrix offset (cleared each block).
    offsets: [f32; SCRIPT_MAX_PARAMS],
}

impl ScriptParams {
    /// An empty store (no active knobs).
    #[must_use]
    pub fn new() -> Self {
        Self {
            len: 0,
            // Slots `>= len` are never read, so any placeholder is fine.
            names: [PortName::IN; SCRIPT_MAX_PARAMS],
            name_strs: [""; SCRIPT_MAX_PARAMS],
            values: [0.0; SCRIPT_MAX_PARAMS],
            overrides: [None; SCRIPT_MAX_PARAMS],
            offsets: [0.0; SCRIPT_MAX_PARAMS],
        }
    }

    /// Reindex the store from a script's declared knobs, **in place** and
    /// alloc-free: a knob whose name survives keeps its current value (editing a
    /// script must not reset unrelated knobs); a new knob takes its decl default.
    /// Offsets are zeroed (they re-accumulate next block). Real-time safe — a
    /// bounded ≤`SCRIPT_MAX_PARAMS`² name scan over fixed arrays, no heap.
    pub fn remap(&mut self, decls: &[ScriptParamDecl]) {
        let n = decls.len().min(SCRIPT_MAX_PARAMS);
        let old_len = self.len;
        // Compute the new values into a stack temp first: writing `values[i]` in
        // place could clobber a survivor another slot still needs to read (e.g. two
        // knobs swap positions across an edit).
        let mut new_values = [0.0f32; SCRIPT_MAX_PARAMS];
        for (i, decl) in decls.iter().take(n).enumerate() {
            let survivor = (0..old_len)
                .find(|&j| self.names[j] == decl.name)
                .map(|j| self.values[j]);
            new_values[i] = survivor.unwrap_or(decl.default);
        }
        for (i, decl) in decls.iter().take(n).enumerate() {
            self.names[i] = decl.name;
            self.name_strs[i] = decl.name_str;
        }
        self.values = new_values;
        self.overrides = [None; SCRIPT_MAX_PARAMS];
        self.offsets = [0.0; SCRIPT_MAX_PARAMS];
        self.len = n;
    }

    /// Drop all knobs (a cleared script).
    pub fn clear(&mut self) {
        self.len = 0;
        self.overrides = [None; SCRIPT_MAX_PARAMS];
        self.offsets = [0.0; SCRIPT_MAX_PARAMS];
    }

    /// The number of active knobs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the store has no active knobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Store `value` on the knob named `name`, returning whether it matched an
    /// active knob.
    pub fn set(&mut self, name: PortName, value: f32) -> bool {
        match self.slot_of(name) {
            Some(i) => {
                self.values[i] = value;
                true
            }
            None => false,
        }
    }

    /// The stored value of the knob named `name` (ignoring the mod-offset).
    #[must_use]
    pub fn get(&self, name: PortName) -> Option<f32> {
        self.slot_of(name).map(|i| self.values[i])
    }

    /// The active knobs as `Param`s, for [`PolyModule::get_params`]. Each is a
    /// `Param::Script(ScriptParam::Knob(name, value))` at its current value.
    #[must_use]
    pub fn as_params(&self) -> Vec<Param> {
        (0..self.len)
            .map(|i| Param::Script(ScriptParam::Knob(self.names[i], self.values[i])))
            .collect()
    }

    /// Add a mod-matrix offset onto the knob whose descriptor `type_id` is
    /// `target`, returning whether it matched. Matched by a lock-free `str`
    /// compare against each active knob's cached `'static` name — no
    /// `PortName::as_str` read-lock on the audio thread.
    pub fn add_offset(&mut self, target: &str, value: f32) -> bool {
        for i in 0..self.len {
            if self.name_strs[i] == target {
                self.offsets[i] += value;
                return true;
            }
        }
        false
    }

    /// Clear every knob's accumulated mod-offset (once per block).
    pub fn clear_offsets(&mut self) {
        self.offsets[..self.len].fill(0.0);
    }

    /// Set a transient sequencer-automation override on the knob named `name`,
    /// returning whether it matched. The stored base value is untouched (a project
    /// saved mid-playback keeps it); [`clear_overrides`](Self::clear_overrides)
    /// reverts to base on transport stop.
    pub fn set_override(&mut self, name: PortName, value: f32) -> bool {
        match self.slot_of(name) {
            Some(i) => {
                self.overrides[i] = Some(value);
                true
            }
            None => false,
        }
    }

    /// Drop every knob's automation override (revert to the stored base value).
    pub fn clear_overrides(&mut self) {
        self.overrides[..self.len].fill(None);
    }

    /// The effective value of the knob named `name`: the automation override (if
    /// set) or else the stored base, plus this block's accumulated mod-offset.
    /// `None` if no such active knob.
    #[must_use]
    pub fn effective(&self, name: PortName) -> Option<f32> {
        self.slot_of(name)
            .map(|i| self.overrides[i].unwrap_or(self.values[i]) + self.offsets[i])
    }

    fn slot_of(&self, name: PortName) -> Option<usize> {
        (0..self.len).find(|&i| self.names[i] == name)
    }
}

impl Default for ScriptParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a modulatable [`ParameterDescriptor`] for one declared knob at its
/// current `value`. The `type_id` is the knob's interned name (the persistence
/// key and mod-matrix / cross-script address); the display name is its `label`
/// or, failing that, the identifier; the tooltip becomes the description.
#[must_use]
pub fn knob_descriptor(decl: &ScriptParamDecl, value: f32) -> ParameterDescriptor {
    let id = Param::Script(ScriptParam::Knob(decl.name, value));
    let label = decl
        .label
        .clone()
        .unwrap_or_else(|| decl.name_str.to_string());
    let mut d = ParameterDescriptor::float(decl.name_str, id, label)
        .range(decl.min, decl.max)
        .default(decl.default)
        .unit(decl.unit)
        .modulatable(true);
    if let Some(tip) = &decl.tooltip {
        d = d.description(tip.clone());
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, default: f32) -> ScriptParamDecl {
        let n = PortName::intern(name);
        ScriptParamDecl {
            name: n,
            name_str: n.as_str(),
            default,
            min: 0.0,
            max: 1.0,
            label: None,
            tooltip: None,
            unit: crate::ParameterUnit::None,
        }
    }

    #[test]
    fn remap_keeps_survivors_and_defaults_new() {
        let mut s = ScriptParams::new();
        let drive = PortName::intern("drive");
        s.remap(&[decl("drive", 0.5), decl("amt", 0.2)]);
        assert_eq!(s.len(), 2);
        assert!(s.set(drive, 0.9));
        // Re-install with drive kept + a new knob: drive keeps 0.9, gain defaults.
        s.remap(&[decl("drive", 0.5), decl("gain", 0.3)]);
        assert_eq!(s.get(drive), Some(0.9));
        assert_eq!(s.get(PortName::intern("gain")), Some(0.3));
        // The dropped knob is gone.
        assert_eq!(s.get(PortName::intern("amt")), None);
    }

    #[test]
    fn set_get_and_effective_with_offset() {
        let mut s = ScriptParams::new();
        let drive = PortName::intern("drive");
        s.remap(&[decl("drive", 0.5)]);
        assert_eq!(s.get(drive), Some(0.5));
        assert!(s.set(drive, 0.7));
        assert_eq!(s.effective(drive), Some(0.7));
        assert!(s.add_offset("drive", 0.1));
        assert!((s.effective(drive).unwrap() - 0.8).abs() < 1e-6);
        s.clear_offsets();
        assert_eq!(s.effective(drive), Some(0.7));
        // Unknown knob: no match, no panic.
        assert!(!s.set(PortName::intern("nope"), 1.0));
        assert!(!s.add_offset("nope", 1.0));
    }

    #[test]
    fn automation_override_layers_over_base_and_reverts() {
        let mut s = ScriptParams::new();
        let drive = PortName::intern("drive");
        s.remap(&[decl("drive", 0.5)]);
        s.set(drive, 0.6);
        // The override replaces the base for `effective`; the base is untouched
        // (so a project saved mid-playback keeps its knob).
        assert!(s.set_override(drive, 0.9));
        assert_eq!(s.effective(drive), Some(0.9));
        assert_eq!(s.get(drive), Some(0.6));
        // A mod-offset stacks on top of the override.
        s.add_offset("drive", 0.05);
        assert!((s.effective(drive).unwrap() - 0.95).abs() < 1e-6);
        // Clearing the override reverts to base (+ the still-present offset).
        s.clear_overrides();
        assert!((s.effective(drive).unwrap() - 0.65).abs() < 1e-6);
    }

    #[test]
    fn knob_descriptor_carries_range_label_tooltip() {
        let n = PortName::intern("cutoff");
        let d = ScriptParamDecl {
            name: n,
            name_str: n.as_str(),
            default: 1000.0,
            min: 20.0,
            max: 20000.0,
            label: Some("Cutoff".to_string()),
            tooltip: Some("filter".to_string()),
            unit: crate::ParameterUnit::Hertz,
        };
        let pd = knob_descriptor(&d, 1500.0);
        assert_eq!(pd.type_id, "cutoff");
        assert_eq!(pd.name, "Cutoff");
        assert_eq!(pd.description, "filter");
        assert_eq!(pd.unit, crate::ParameterUnit::Hertz);
        assert!(pd.modulatable);
        assert!((pd.range.min - 20.0).abs() < 1e-6);
        assert!((pd.range.max - 20000.0).abs() < 1e-6);
        assert!((pd.range.default - 1000.0).abs() < 1e-6);
        // The id is a Script knob param carrying the current value + its name.
        assert!((pd.id.as_f32() - 1500.0).abs() < 1e-6);
        assert_eq!(pd.id.name(), "cutoff");
    }
}
