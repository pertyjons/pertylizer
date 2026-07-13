//! Lexer: source text → tokens with byte spans.
//!
//! Whitespace and comments are skipped — the compiler path does not need
//! trivia (the formatter, decision #11, walks a lossless CST instead). Errors
//! are collected into diagnostics, never fatal: an unknown character emits a
//! diagnostic and lexing resumes at the next character.

use crate::diag::Diagnostic;
use crate::span::Span;

/// A time-unit suffix on a numeric literal. Folds to seconds at compile time
/// (`ms` ×0.001, `s` ×1) — the lexer keeps the raw value + unit so the
/// formatter can preserve `50ms` rather than `0.05`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationUnit {
    Millis,
    Seconds,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords.
    Src,
    Let,
    Out,
    Arr,
    /// `state` — declares a persistent, author-addressable difference-equation
    /// cell (custom IIR/feedback memory).
    State,
    /// `param` — declares a user-facing knob (`param drive = 0.5`), Script /
    /// AudioScript modules only.
    Param,
    // Identifier (not a keyword).
    Ident(String),
    /// A double-quoted string literal (`"Drive"`), used for optional `param`
    /// display label / tooltip. No escape sequences (v1).
    Str(String),
    /// Numeric literal. `unit` is set for durations; `is_int` is true for a
    /// bare non-negative integer (used for module instance numbers like the
    /// `1` in `lfo-1`).
    Number {
        value: f64,
        unit: Option<DurationUnit>,
        is_int: bool,
    },
    // Punctuation / operators.
    Question,
    Colon,
    OrOr,
    AndAnd,
    EqEq,
    BangEq,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Caret,
    Eq,
    LParen,
    RParen,
    Comma,
    Dot,
    LBracket,
    RBracket,
    /// A run of newlines and/or semicolons (the `separator` production), with
    /// interleaved inline whitespace/comments folded in.
    Sep,
    /// End of input — always the final token.
    Eof,
}

impl TokenKind {
    /// The identifier-or-keyword spelling, if this token is one. Returns the
    /// text for [`TokenKind::Ident`] and for the keyword tokens (`src`, `let`,
    /// `out`) — the parser uses this to accept a keyword spelling as a member
    /// name (e.g. the very common port `lfo-1.out`).
    #[must_use]
    pub fn ident_text(&self) -> Option<&str> {
        match self {
            Self::Ident(name) => Some(name),
            Self::Src => Some("src"),
            Self::Let => Some("let"),
            Self::Out => Some("out"),
            Self::Arr => Some("arr"),
            Self::State => Some("state"),
            Self::Param => Some("param"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Tokenize `src`, returning the tokens (always ending in [`TokenKind::Eof`])
/// and any diagnostics collected along the way.
#[must_use]
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    Lexer {
        src,
        bytes: src.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        diags: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn run(mut self) -> (Vec<Token>, Vec<Diagnostic>) {
        loop {
            self.skip_inline();
            let start = self.pos;
            let Some(c) = self.byte_at(self.pos) else {
                self.push(TokenKind::Eof, start, self.pos);
                break;
            };
            match c {
                b'\n' | b';' => self.lex_separator(start),
                b'0'..=b'9' => self.lex_number(start),
                b'"' => self.lex_string(start),
                c if is_ident_start(c) => self.lex_ident(start),
                c => self.lex_symbol(start, c),
            }
        }
        (self.tokens, self.diags)
    }

    /// Skip spaces, tabs, carriage returns, and `#` comments (to end of line,
    /// without consuming the terminating newline — that is a separator).
    fn skip_inline(&mut self) {
        loop {
            match self.byte_at(self.pos) {
                Some(b' ' | b'\t' | b'\r') => self.pos += 1,
                Some(b'#') => {
                    while !matches!(self.byte_at(self.pos), None | Some(b'\n')) {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn lex_separator(&mut self, start: usize) {
        // Track the end of the last actual separator char so the token span
        // covers the separator run only — not the trailing inline whitespace /
        // comments that `skip_inline` skips past while looking for more.
        let mut end = self.pos;
        loop {
            while matches!(self.byte_at(self.pos), Some(b'\n' | b';')) {
                self.pos += 1;
                end = self.pos;
            }
            self.skip_inline();
            if !matches!(self.byte_at(self.pos), Some(b'\n' | b';')) {
                break;
            }
        }
        self.push(TokenKind::Sep, start, end);
    }

    fn lex_ident(&mut self, start: usize) {
        while matches!(self.byte_at(self.pos), Some(c) if is_ident_continue(c)) {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        let kind = match text {
            "src" => TokenKind::Src,
            "let" => TokenKind::Let,
            "out" => TokenKind::Out,
            "arr" => TokenKind::Arr,
            "state" => TokenKind::State,
            "param" => TokenKind::Param,
            _ => TokenKind::Ident(text.to_string()),
        };
        self.push(kind, start, self.pos);
    }

    /// Lex a double-quoted string literal (`"Drive"`) for an optional `param`
    /// label / tooltip. No escapes and no multi-line strings (v1): a newline or
    /// EOF before the closing quote is an "unterminated string" error, but a
    /// best-effort [`TokenKind::Str`] with the text so far is still emitted so the
    /// rest of the program keeps parsing (decision #9: collect, don't abort).
    fn lex_string(&mut self, start: usize) {
        self.pos += 1; // opening quote
        let content_start = self.pos;
        while let Some(c) = self.byte_at(self.pos) {
            match c {
                b'"' => {
                    let text = self.src[content_start..self.pos].to_string();
                    self.pos += 1; // closing quote
                    self.push(TokenKind::Str(text), start, self.pos);
                    return;
                }
                b'\n' => break,
                _ => self.pos += 1,
            }
        }
        // Unterminated (newline or EOF before the closing quote).
        let text = self.src[content_start..self.pos].to_string();
        self.diags.push(Diagnostic::error(
            Span::new(start as u32, self.pos as u32),
            "unterminated string literal",
        ));
        self.push(TokenKind::Str(text), start, self.pos);
    }

    fn lex_number(&mut self, start: usize) {
        while matches!(self.byte_at(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        let mut is_int = true;
        // Fractional part — only if '.' is immediately followed by a digit, so
        // `1.out` lexes as `1` `.` `out` (an address), not `1.` `out`.
        if self.byte_at(self.pos) == Some(b'.')
            && matches!(self.byte_at(self.pos + 1), Some(b'0'..=b'9'))
        {
            is_int = false;
            self.pos += 1;
            while matches!(self.byte_at(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        // Exponent — only when a digit (optionally signed) follows, so `2exp`
        // lexes as `2` `exp`, not a broken exponent.
        if matches!(self.byte_at(self.pos), Some(b'e' | b'E')) && self.exponent_follows() {
            is_int = false;
            self.pos += 1;
            if matches!(self.byte_at(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.byte_at(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let num_end = self.pos;
        let unit = self.lex_unit();
        if unit.is_some() {
            is_int = false;
        }
        let value = self.src[start..num_end].parse::<f64>().unwrap_or_else(|_| {
            self.diags.push(Diagnostic::error(
                Span::new(start as u32, num_end as u32),
                "invalid number literal",
            ));
            0.0
        });
        self.push(
            TokenKind::Number {
                value,
                unit,
                is_int,
            },
            start,
            self.pos,
        );
    }

    /// Whether an exponent (`e`/`E` already at `pos`) is followed by a digit,
    /// possibly with a leading sign.
    fn exponent_follows(&self) -> bool {
        match self.byte_at(self.pos + 1) {
            Some(b'0'..=b'9') => true,
            Some(b'+' | b'-') => matches!(self.byte_at(self.pos + 2), Some(b'0'..=b'9')),
            _ => false,
        }
    }

    /// Try to consume a `ms`/`s` duration suffix at `pos`. Only matches when
    /// the suffix is not followed by an identifier char, so `2sin` stays
    /// `2` `sin`.
    fn lex_unit(&mut self) -> Option<DurationUnit> {
        let followed_by_ident = |b: Option<u8>| matches!(b, Some(c) if is_ident_continue(c));
        if self.byte_at(self.pos) == Some(b'm')
            && self.byte_at(self.pos + 1) == Some(b's')
            && !followed_by_ident(self.byte_at(self.pos + 2))
        {
            self.pos += 2;
            return Some(DurationUnit::Millis);
        }
        if self.byte_at(self.pos) == Some(b's') && !followed_by_ident(self.byte_at(self.pos + 1)) {
            self.pos += 1;
            return Some(DurationUnit::Seconds);
        }
        None
    }

    fn lex_symbol(&mut self, start: usize, c: u8) {
        let two = self.byte_at(self.pos + 1);
        let two_char = match (c, two) {
            (b'|', Some(b'|')) => Some(TokenKind::OrOr),
            (b'&', Some(b'&')) => Some(TokenKind::AndAnd),
            (b'=', Some(b'=')) => Some(TokenKind::EqEq),
            (b'!', Some(b'=')) => Some(TokenKind::BangEq),
            (b'<', Some(b'=')) => Some(TokenKind::Le),
            (b'>', Some(b'=')) => Some(TokenKind::Ge),
            _ => None,
        };
        if let Some(kind) = two_char {
            self.pos += 2;
            self.push(kind, start, self.pos);
            return;
        }
        let single = match c {
            b'?' => Some(TokenKind::Question),
            b':' => Some(TokenKind::Colon),
            b'<' => Some(TokenKind::Lt),
            b'>' => Some(TokenKind::Gt),
            b'+' => Some(TokenKind::Plus),
            b'-' => Some(TokenKind::Minus),
            b'*' => Some(TokenKind::Star),
            b'/' => Some(TokenKind::Slash),
            b'%' => Some(TokenKind::Percent),
            b'!' => Some(TokenKind::Bang),
            b'^' => Some(TokenKind::Caret),
            b'=' => Some(TokenKind::Eq),
            b'(' => Some(TokenKind::LParen),
            b')' => Some(TokenKind::RParen),
            b',' => Some(TokenKind::Comma),
            b'.' => Some(TokenKind::Dot),
            b'[' => Some(TokenKind::LBracket),
            b']' => Some(TokenKind::RBracket),
            _ => None,
        };
        if let Some(kind) = single {
            self.pos += 1;
            self.push(kind, start, self.pos);
            return;
        }
        // Unknown character, or a lone `|` / `&`. Emit a diagnostic and skip
        // exactly one character so lexing can resume.
        let next = start + self.char_len_at(start);
        self.pos = next;
        let message = match c {
            b'|' => "expected '||'",
            b'&' => "expected '&&'",
            _ => "unexpected character",
        };
        self.diags.push(Diagnostic::error(
            Span::new(start as u32, next as u32),
            message,
        ));
    }

    fn byte_at(&self, i: usize) -> Option<u8> {
        self.bytes.get(i).copied()
    }

    /// UTF-8 length in bytes of the character starting at `pos`.
    fn char_len_at(&self, pos: usize) -> usize {
        self.src[pos..].chars().next().map_or(1, char::len_utf8)
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            span: Span::new(start as u32, end as u32),
        });
    }
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Token kinds only (drop spans) for terse assertions.
    fn kinds(src: &str) -> Vec<TokenKind> {
        let (toks, diags) = lex(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        toks.into_iter().map(|t| t.kind).collect()
    }

    fn num(value: f64, unit: Option<DurationUnit>, is_int: bool) -> TokenKind {
        TokenKind::Number {
            value,
            unit,
            is_int,
        }
    }

    #[test]
    fn simple_expression() {
        assert_eq!(
            kinds("out = lfo * 0.45"),
            vec![
                TokenKind::Out,
                TokenKind::Eq,
                TokenKind::Ident("lfo".to_string()),
                TokenKind::Star,
                num(0.45, None, false),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn duration_suffixes() {
        assert_eq!(
            kinds("50ms"),
            vec![num(50.0, Some(DurationUnit::Millis), false), TokenKind::Eof]
        );
        assert_eq!(
            kinds("2s"),
            vec![num(2.0, Some(DurationUnit::Seconds), false), TokenKind::Eof]
        );
    }

    #[test]
    fn s_suffix_does_not_swallow_identifier() {
        // `2sin(x)` must not read `s` as a unit.
        assert_eq!(
            kinds("2sin"),
            vec![
                num(2.0, None, true),
                TokenKind::Ident("sin".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn module_address_keeps_the_dot() {
        // `lfo-1.out` → ident, minus, int 1, dot, `out` keyword — the dot must
        // survive. `out` lexes as the keyword token; the parser accepts it as a
        // member name via `ident_text` (see `keyword_member_text`).
        assert_eq!(
            kinds("lfo-1.out"),
            vec![
                TokenKind::Ident("lfo".to_string()),
                TokenKind::Minus,
                num(1.0, None, true),
                TokenKind::Dot,
                TokenKind::Out,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn keyword_member_text() {
        // The parser reads `.out` as the port named "out" through `ident_text`.
        assert_eq!(TokenKind::Out.ident_text(), Some("out"));
        assert_eq!(
            TokenKind::Ident("cutoff".to_string()).ident_text(),
            Some("cutoff")
        );
        assert_eq!(TokenKind::Plus.ident_text(), None);
    }

    #[test]
    fn exponent_vs_identifier() {
        assert_eq!(kinds("1e3"), vec![num(1000.0, None, false), TokenKind::Eof]);
        assert_eq!(
            kinds("2exp"),
            vec![
                num(2.0, None, true),
                TokenKind::Ident("exp".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn array_decl_and_index_tokens() {
        assert_eq!(
            kinds("arr seq = [0, 0.5]"),
            vec![
                TokenKind::Arr,
                TokenKind::Ident("seq".to_string()),
                TokenKind::Eq,
                TokenKind::LBracket,
                num(0.0, None, true),
                TokenKind::Comma,
                num(0.5, None, false),
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
        assert_eq!(
            kinds("seq[i]"),
            vec![
                TokenKind::Ident("seq".to_string()),
                TokenKind::LBracket,
                TokenKind::Ident("i".to_string()),
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn two_char_operators() {
        assert_eq!(
            kinds("<= >= == != && ||"),
            vec![
                TokenKind::Le,
                TokenKind::Ge,
                TokenKind::EqEq,
                TokenKind::BangEq,
                TokenKind::AndAnd,
                TokenKind::OrOr,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn separator_run_collapses_blank_lines_and_comments() {
        // Two statements separated by blank lines and a comment → one Sep.
        let kinds = kinds("out = 1\n\n   # a comment\n\nlet x = 2");
        let seps = kinds.iter().filter(|k| **k == TokenKind::Sep).count();
        assert_eq!(seps, 1, "blank lines + comment should fold to a single Sep");
    }

    #[test]
    fn semicolon_is_a_separator() {
        assert_eq!(
            kinds("a;b"),
            vec![
                TokenKind::Ident("a".to_string()),
                TokenKind::Sep,
                TokenKind::Ident("b".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn separator_span_excludes_trailing_whitespace() {
        // `a;  b` — the Sep span must cover only `;`, not the two spaces after.
        let (toks, _diags) = lex("a;  b");
        let sep = toks
            .iter()
            .find(|t| t.kind == TokenKind::Sep)
            .expect("a separator");
        assert_eq!(sep.span, Span::new(1, 2));
    }

    #[test]
    fn comment_to_end_of_line_is_skipped() {
        assert_eq!(
            kinds("out = 1 # trailing"),
            vec![
                TokenKind::Out,
                TokenKind::Eq,
                num(1.0, None, true),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn unknown_character_reports_and_recovers() {
        let (toks, diags) = lex("a @ b");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "unexpected character");
        // Lexing recovered: both identifiers are present.
        let idents = toks
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Ident(_)))
            .count();
        assert_eq!(idents, 2);
    }

    #[test]
    fn lone_pipe_suggests_or() {
        let (_toks, diags) = lex("a | b");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "expected '||'");
    }
}
