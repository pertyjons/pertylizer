//! Allocation-free VM invocation over numeric bindings.
use super::{ParameterRead, PreparedInput, ScriptVoiceId};
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
        ..
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
        *source = input_value(*input, io, 0, *voice, false);
    }
    let value = program.code.eval(
        &sources,
        registers,
        &synth_core::script::EvalContext::new(*rate),
    );
    io.out.fill(value);
}

pub(crate) fn audio(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Script { program, rate, .. } = prepared else {
        return;
    };
    let NodeState::Script {
        registers,
        seed,
        voice,
        first_sample,
        ..
    } = state
    else {
        return;
    };
    let Some(program) = io.scripts.programs.get(program.index()) else {
        io.out.fill(0.0);
        return;
    };
    let mut sources = [0.0; synth_core::script::MAX_SOURCES];
    let channels = io.channels.channels();
    for frame in 0..io.out.len() / channels {
        if io.controls.iter().any(|control| {
            control.control == ControlIndex::RESET && usize::from(control.offset.as_u16()) == frame
        }) {
            registers.reset(0, seed.as_u64());
            *first_sample = true;
        }
        for (input, source) in program.inputs.iter().zip(&mut sources) {
            *source = input_value(*input, io, frame, *voice, *first_sample);
        }
        let values = program.code.eval_multi(
            &sources,
            registers,
            &synth_core::script::EvalContext::audio(*rate),
        );
        let left = values.first().copied().flatten().unwrap_or(0.0);
        let right = values
            .get(1)
            .copied()
            .flatten()
            .unwrap_or(if program.mono_output { left } else { 0.0 });
        if let Some(out) = io.out.get_mut(frame * channels) {
            *out = left;
        }
        if channels == 2
            && let Some(out) = io.out.get_mut(frame * channels + 1)
        {
            *out = right;
        }
        *first_sample = false;
    }
}

fn input_value(
    input: PreparedInput,
    io: &NodeIo<'_>,
    frame: usize,
    voice: ScriptVoiceId,
    first: bool,
) -> f32 {
    match input {
        PreparedInput::Local(index) => kernels::ramp_of(io.ramps, usize::from(index.as_u8()))
            .get(frame)
            .copied()
            .unwrap_or(0.0),
        PreparedInput::Signal(index) => match io.inputs.get(index.index()) {
            Some(InputBuffer::Patched(buffer)) => buffer.first().copied().unwrap_or(0.0),
            _ => 0.0,
        },
        PreparedInput::AudioSignal {
            index,
            layout,
            channel,
        } => match io.inputs.get(index.index()) {
            Some(InputBuffer::Patched(buffer)) => buffer
                .get(frame * layout.channels() + channel.index())
                .copied()
                .unwrap_or(0.0),
            _ => 0.0,
        },
        PreparedInput::FirstSample => f32::from(first),
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
    }
}
