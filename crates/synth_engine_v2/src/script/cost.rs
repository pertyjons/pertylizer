//! Static VM workload advisory, not a CPU-time prediction.
use super::{ScriptDomain, ScriptProgram};
use crate::{ir::ExecutionScope, profile::HostProfile, quantities::VoiceCount};

/// One bytecode dispatch or one ScaleSnap candidate comparison.
/// Transcendental dispatches count once; this unit deliberately promises no time bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct VmWorkUnits(u64);
impl VmWorkUnits {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Work at the configured polyphony ceiling, even when this plan admits fewer voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct AudioScriptCost {
    pub node: crate::ir::NodeId,
    pub per_evaluation: VmWorkUnits,
    pub per_quantum: VmWorkUnits,
    pub voices: VoiceCount,
    pub warning_threshold: VmWorkUnits,
}
impl ScriptProgram {
    /// A directly countable workload, including each bounded table-search candidate.
    pub fn vm_work_per_evaluation(&self) -> VmWorkUnits {
        VmWorkUnits(self.code.code().iter().fold(0_u64, |work, op| {
            let candidates = match op {
                synth_core::script::Op::ScaleSnap { len, .. } => 3 * u64::from(*len),
                _ => 0,
            };
            work.saturating_add(1 + candidates)
        }))
    }
    pub fn audio_cost(
        &self,
        scope: ExecutionScope,
        profile: &HostProfile,
    ) -> Option<AudioScriptCost> {
        if !matches!(self.domain, ScriptDomain::Audio(_)) {
            return None;
        }
        let voices = if scope == ExecutionScope::Voice {
            profile.limits().voices().maximum_voices_per_instrument()
        } else {
            VoiceCount::measured(1)
        };
        let per_evaluation = self.vm_work_per_evaluation();
        Some(AudioScriptCost {
            node: self.node(),
            per_evaluation,
            voices,
            per_quantum: VmWorkUnits(
                per_evaluation
                    .0
                    .saturating_mul(u64::from(crate::time::QUANTUM_FRAMES))
                    .saturating_mul(u64::from(voices.get())),
            ),
            // More per-quantum work than one maximum-sized straight-line Control program
            // is the declared advisory threshold; it is not an admission ceiling.
            warning_threshold: VmWorkUnits(u64::from(
                profile
                    .limits()
                    .script()
                    .max_instructions_per_program()
                    .get(),
            )),
        })
    }
}
