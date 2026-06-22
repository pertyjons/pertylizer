//! YAMS (*Yet Another Modulation Script*) — the non-real-time front-end for the
//! Control Script layer (the Mod Matrix modulation expression language; see
//! `docs/yams.md`).
//!
//! This crate holds the language toolchain that runs on the UI/MCP thread:
//! lexer, parser, resolver, bytecode compiler, and the `yamsfmt` formatter. It
//! compiles source text into a `CompiledScript` (the real-time type, defined in
//! `synth_core`) which the audio thread evaluates. See `docs/yams.md`
//! for the language specification.

pub mod ast;
pub mod compile;
pub mod diag;
pub mod fmt;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod symbols;

pub use compile::{CompileOptions, CompiledProgram, SourceInput, compile};
pub use diag::{Diagnostic, Severity};
pub use fmt::format;
pub use parser::parse;
pub use span::{LineCol, Span};

/// The YAMS language reference (Markdown), embedded into the binary at compile
/// time from `assets/yams.md`. This crate owns the language, so it owns the
/// canonical reference; consumers (e.g. the `get_yams_reference` MCP tool) serve
/// this constant rather than reading the file at runtime. The repo's
/// `docs/yams.md` is a symlink to the same file for documentation browsing.
pub const REFERENCE: &str = include_str!("../assets/yams.md");
