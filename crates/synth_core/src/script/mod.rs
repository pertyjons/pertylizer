//! YAMS (*Yet Another Modulation Script*) real-time runtime.
//!
//! This module holds only the audio-thread half of the Control Script layer:
//! the bytecode instruction set, the immutable [`CompiledScript`], and the
//! allocation-free stack-machine evaluator. The non-RT toolchain that *builds*
//! a `CompiledScript` from source text (lexer, parser, compiler, formatter)
//! lives in the `synth_script` crate. See `plans/yams-grammar.md`.

pub mod bytecode;
pub mod eval;

pub use bytecode::{
    Builtin, CompiledScript, MAX_INSTRUCTIONS, MAX_LOCALS, MAX_NESTING_DEPTH, MAX_SOURCE_LEN,
    MAX_SOURCES, MAX_STACK, MAX_STATE, Op,
};
pub use eval::{EvalContext, RegisterFile};
