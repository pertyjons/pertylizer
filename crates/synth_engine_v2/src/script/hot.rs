//! Allocation-free VM invocation over numeric bindings.
use super::{ParameterRead, PreparedInput};
use crate::node::kernels::{self, ControlIndex, InputBuffer, NodeIo, NodeState, PreparedNode};

pub(crate) fn run(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Script { program, rate, .. } = prepared else {
        return;
    };
    let NodeState::Script {
        registers,
        seed,
        reset_pending,
        voice,
    } = state
    else {
        return;
    };
    let Some(program) = io.scripts.programs.get(program.index()) else {
        io.out.fill(0.0);
        return;
    };
    let reset_now = io.controls.iter().any(|control| {
        control.control == ControlIndex::RESET && control.offset == crate::time::QuantumOffset::ZERO
    });
    if *reset_pending || reset_now {
        registers.reset(0, seed.as_u64());
    }
    *reset_pending = io.controls.iter().any(|control| {
        control.control == ControlIndex::RESET && control.offset != crate::time::QuantumOffset::ZERO
    });
    let mut sources = [0.0; synth_core::script::MAX_SOURCES];
    for (input, source) in program.inputs.iter().zip(&mut sources) {
        *source = match *input {
            PreparedInput::Local(index) => kernels::ramp_of(io.ramps, usize::from(index.as_u8()))
                .first()
                .copied()
                .unwrap_or(0.0),
            PreparedInput::Signal(index) => match io.inputs.get(index.index()) {
                Some(InputBuffer::Patched(buffer)) => buffer.first().copied().unwrap_or(0.0),
                _ => 0.0,
            },
            PreparedInput::Constant(value) => value.as_f32(),
            PreparedInput::Parameter {
                slot,
                read,
                per_voice,
            } => {
                let offset = if per_voice { voice.0 as usize } else { 0 };
                io.scripts
                    .parameters
                    .get(slot.index().saturating_add(offset))
                    .map_or(0.0, |values| match read {
                        ParameterRead::Base => values.base.as_f32(),
                        ParameterRead::Automated => values.automated.as_f32(),
                        ParameterRead::PreviousResolved => values.previous.as_f32(),
                    })
            }
        };
    }
    let value = program.code.eval(
        &sources,
        registers,
        &synth_core::script::EvalContext::new(*rate),
    );
    io.out.fill(value);
}
