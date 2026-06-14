//! Compile diagnostics.
//!
//! Per the YAMS design (decision #9) all errors are collected and reported with
//! spans rather than bailing on the first one — better for an editor and for
//! LLM authoring. Lexer, parser, resolver, and compiler all push into the same
//! `Vec<Diagnostic>`.

use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    #[must_use]
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            severity: Severity::Error,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            severity: Severity::Warning,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error)
    }
}
