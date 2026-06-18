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

pub use bound::{BoundScript, ScriptContext, ScriptInput};
pub use bytecode::{
    Builtin, CompiledScript, MAX_INSTRUCTIONS, MAX_LOCALS, MAX_NESTING_DEPTH, MAX_SOURCE_LEN,
    MAX_SOURCES, MAX_STACK, MAX_STATE, Op, finite_or_zero, safe_clamp, safe_div,
};
pub use eval::{EvalContext, RegisterFile};
