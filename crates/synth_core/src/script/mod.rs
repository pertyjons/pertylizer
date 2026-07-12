//! YAMS (*Yet Another Modulation Script*) real-time runtime.
//!
//! This module holds only the audio-thread half of the Control Script layer:
//! the bytecode instruction set, the immutable [`CompiledScript`], and the
//! allocation-free stack-machine evaluator. The non-RT toolchain that *builds*
//! a `CompiledScript` from source text (lexer, parser, compiler, formatter)
//! lives in the `synth_script` crate. See `docs/yams.md`.

pub mod bound;
pub mod bytecode;
pub mod eval;
pub mod host;

pub use bound::{AudioInputChannel, BoundScript, NoteField, ScriptContext, ScriptInput};
pub use bytecode::{
    Builtin, CompiledScript, MAX_ARRAY_STORAGE, MAX_ARRAYS, MAX_INSTRUCTIONS, MAX_LOCALS,
    MAX_NESTING_DEPTH, MAX_SOURCE_LEN, MAX_SOURCES, MAX_STACK, MAX_STATE, Op, finite_or_zero,
    safe_clamp, safe_div,
};
pub use eval::{AudioBindings, EvalContext, NoteOutputs, RegisterFile};
pub use host::{SCRIPT_HOST_SLOTS, SCRIPT_PRNG_SEED, ScriptHost};
