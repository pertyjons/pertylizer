//! YAMS (*Yet Another Modulation Script*) — the non-real-time front-end for the
//! Control Script layer (Step 2 of `plans/control-script-plan.md`).
//!
//! This crate holds the language toolchain that runs on the UI/MCP thread:
//! lexer, parser, resolver, bytecode compiler, and the `yamsfmt` formatter. It
//! compiles source text into a `CompiledScript` (the real-time type, defined in
//! `synth_core`) which the audio thread evaluates. See `plans/yams-grammar.md`
//! for the language specification.

pub mod diag;
pub mod lexer;
pub mod span;

pub use diag::{Diagnostic, Severity};
pub use span::{LineCol, Span};
