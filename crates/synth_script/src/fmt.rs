//! `yamsfmt` — the canonical formatter (decision #7, #11).
//!
//! One canonical form, no config, no opt-out. The formatter renders from the
//! AST, so it is **not** an optimizer (it preserves `2 * 3`, never folds) and it
//! emits the *minimal necessary* parentheses rather than preserving the
//! author's — full author-paren + lossless-trivia preservation is the deferred
//! rowan-CST path (decision #11). Comments are reattached by line: an own-line
//! comment leads the following statement (a leading comment block is always
//! followed by exactly one blank line before its statement); a trailing comment
//! stays on its code line and never forces a blank line.
//!
//! `format` is idempotent: `format(format(x)) == format(x)`.
//!
//! **v1 limitations** (follow-ups, not silent): long statements are **not**
//! wrapped — each renders on a single line (the spec's 80-column wrapping is
//! unimplemented); and a comment in the *middle* of a multi-line input
//! statement is relocated rather than kept in place (the CST would fix both).

use crate::ast::{BinaryOp, Expr, ModuleRef, Program, UnaryOp};
use crate::diag::Diagnostic;
use crate::parser::parse;

/// Format YAMS source into its canonical form, or return the diagnostics if it
/// does not parse (the caller keeps the raw text, per the spec).
///
/// # Errors
/// Returns the collected diagnostics when the source has any parse error.
pub fn format(src: &str) -> Result<String, Vec<Diagnostic>> {
    let (program, diags) = parse(src);
    if diags.iter().any(Diagnostic::is_error) {
        return Err(diags);
    }
    let comments = scan_comments(src);
    let lines = LineMap::new(src);
    Ok(render(&program, &comments, &lines))
}

// ---- comments + line mapping ----------------------------------------------

struct Comment {
    line: usize,
    own_line: bool,
    text: String,
}

/// Scan `#` comments. With no string/char literals in YAMS, every `#` begins a
/// comment that runs to end of line. Text is canonicalized to `# <content>`.
fn scan_comments(src: &str) -> Vec<Comment> {
    let bytes = src.as_bytes();
    let mut comments = Vec::new();
    let mut line = 0usize;
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
                line_start = i;
            }
            b'#' => {
                let own_line = src[line_start..i].trim().is_empty();
                let end = src[i..].find('\n').map_or(src.len(), |n| i + n);
                let content = src[i + 1..end].trim();
                let text = if content.is_empty() {
                    "#".to_string()
                } else {
                    format!("# {content}")
                };
                comments.push(Comment {
                    line,
                    own_line,
                    text,
                });
                i = end;
            }
            _ => i += 1,
        }
    }
    comments
}

/// Maps a byte offset to its 0-based line number.
struct LineMap {
    starts: Vec<usize>,
}

impl LineMap {
    fn new(src: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    fn line_of(&self, offset: u32) -> usize {
        self.starts
            .partition_point(|&s| s <= offset as usize)
            .saturating_sub(1)
    }
}

// ---- rendering ------------------------------------------------------------

enum Stmt {
    Binding {
        sline: usize,
        eline: usize,
        text: String,
    },
    Body {
        sline: usize,
        eline: usize,
        text: String,
    },
}

fn render(program: &Program, comments: &[Comment], lines: &LineMap) -> String {
    // Build the canonical statement list with original source line ranges (for
    // comment attachment) and rendered text.
    let mut stmts: Vec<Stmt> = Vec::new();
    for b in &program.bindings {
        stmts.push(Stmt::Binding {
            sline: lines.line_of(b.span.start),
            eline: lines.line_of(b.span.end),
            text: format!("src {} = {}", b.alias.name, render_address(&b.address)),
        });
    }
    let body_start = stmts.len();
    for l in &program.locals {
        stmts.push(Stmt::Body {
            sline: lines.line_of(l.span.start),
            eline: lines.line_of(l.span.end),
            text: format!("let {} = {}", l.name.name, render_expr(&l.expr)),
        });
    }
    if let Some(o) = &program.output {
        stmts.push(Stmt::Body {
            sline: lines.line_of(o.span.start),
            eline: lines.line_of(o.span.end),
            text: format!("out = {}", render_expr(&o.expr)),
        });
    }

    let mut out: Vec<String> = Vec::new();
    let mut next = 0usize; // index into comments (sorted by line)
    for (i, stmt) in stmts.iter().enumerate() {
        let (Stmt::Binding { sline, eline, text } | Stmt::Body { sline, eline, text }) = stmt;
        let (sline, eline) = (*sline, *eline);
        // Exactly one blank line between the `src` header and the body.
        if i == body_start && body_start > 0 {
            out.push(String::new());
        }
        // Own-line comments that precede this statement, then exactly one blank
        // line separating the comment block from the statement it documents.
        let mut led_by_comment = false;
        while next < comments.len() && comments[next].own_line && comments[next].line < sline {
            out.push(comments[next].text.clone());
            next += 1;
            led_by_comment = true;
        }
        if led_by_comment {
            out.push(String::new());
        }
        // The statement, plus a trailing comment if one sits on its last line.
        let mut line = text.clone();
        if next < comments.len() && !comments[next].own_line && comments[next].line == eline {
            line.push_str("  ");
            line.push_str(&comments[next].text);
            next += 1;
        }
        out.push(line);
    }
    // Any remaining own-line comments (after the last statement).
    while next < comments.len() {
        out.push(comments[next].text.clone());
        next += 1;
    }

    let mut result = out.join("\n");
    result.push('\n');
    result
}

fn render_address(addr: &ModuleRef) -> String {
    match addr.instance {
        Some(n) => format!("{}-{}.{}", addr.module, n, addr.member),
        None => format!("{}.{}", addr.module, addr.member),
    }
}

fn render_expr(e: &Expr) -> String {
    match e {
        Expr::Number { value, unit, .. } => render_number(*value, *unit),
        Expr::Var { name, .. } => name.clone(),
        Expr::Call { name, args, .. } => {
            let rendered: Vec<String> = args.iter().map(render_expr).collect();
            format!("{name}({})", rendered.join(", "))
        }
        Expr::Unary { op, rhs, .. } => {
            let sym = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "!",
            };
            // Unary binds at level 8; parenthesize a looser-binding operand.
            format!("{sym}{}", child(rhs, 8, false, true))
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            let p = prec(e);
            let left_assoc = !matches!(op, BinaryOp::Pow);
            let ls = child(lhs, p, true, left_assoc);
            let rs = child(rhs, p, false, left_assoc);
            format!("{ls} {} {rs}", binary_symbol(*op))
        }
        Expr::Ternary {
            cond, then, els, ..
        } => {
            // Ternary is the loosest level; a same-or-looser cond needs parens.
            format!(
                "{} ? {} : {}",
                child(cond, 1, true, true),
                render_expr(then),
                render_expr(els)
            )
        }
        Expr::Error { .. } => "0".to_string(),
    }
}

/// Render `e` as a child at the given parent precedence, adding parentheses
/// only when precedence/associativity require them.
fn child(e: &Expr, parent_prec: u8, is_left: bool, parent_left_assoc: bool) -> String {
    let cp = prec(e);
    let needs = cp < parent_prec
        || (cp == parent_prec && if parent_left_assoc { !is_left } else { is_left });
    let s = render_expr(e);
    if needs { format!("({s})") } else { s }
}

fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Ternary { .. } => 1,
        Expr::Binary { op, .. } => match op {
            BinaryOp::Or => 2,
            BinaryOp::And => 3,
            BinaryOp::Eq | BinaryOp::Ne => 4,
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => 5,
            BinaryOp::Add | BinaryOp::Sub => 6,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 7,
            BinaryOp::Pow => 9,
        },
        Expr::Unary { .. } => 8,
        Expr::Number { .. } | Expr::Var { .. } | Expr::Call { .. } | Expr::Error { .. } => 10,
    }
}

fn binary_symbol(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        BinaryOp::Pow => "^",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Gt => ">",
        BinaryOp::Le => "<=",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
    }
}

fn render_number(value: f64, unit: Option<crate::ast::DurationUnit>) -> String {
    use crate::ast::DurationUnit;
    // f64 Display already gives the canonical spelling: leading zero present,
    // no trailing zeros, integers without a fraction. The value is raw (50 for
    // `50ms`), so the unit suffix is just appended.
    let n = format!("{value}");
    match unit {
        Some(DurationUnit::Millis) => format!("{n}ms"),
        Some(DurationUnit::Seconds) => format!("{n}s"),
        None => n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format(src).expect("formats")
    }

    fn idempotent(src: &str) {
        let once = fmt(src);
        let twice = fmt(&once);
        assert_eq!(
            once, twice,
            "not idempotent:\n--once--\n{once}\n--twice--\n{twice}"
        );
    }

    #[test]
    fn canonical_spacing_and_blank_line() {
        let out = fmt("src lfo=lfo-1.out\nout=lfo*0.45+velocity*0.6");
        assert_eq!(
            out,
            "src lfo = lfo-1.out\n\nout = lfo * 0.45 + velocity * 0.6\n"
        );
    }

    #[test]
    fn minimal_parens() {
        // Required parens kept; single-atom parens already dropped by the parser.
        assert_eq!(fmt("out = (a + b) * c"), "out = (a + b) * c\n");
        assert_eq!(fmt("out = a * b + c"), "out = a * b + c\n");
        assert_eq!(fmt("out = (a) + b"), "out = a + b\n");
        assert_eq!(fmt("out = a - (b - c)"), "out = a - (b - c)\n");
    }

    #[test]
    fn power_and_unary() {
        assert_eq!(fmt("out = 2^3^2"), "out = 2 ^ 3 ^ 2\n");
        assert_eq!(fmt("out = (2^3)^2"), "out = (2 ^ 3) ^ 2\n");
        assert_eq!(fmt("out = -a^2"), "out = -a ^ 2\n");
        assert_eq!(fmt("out = -(a+b)"), "out = -(a + b)\n");
    }

    #[test]
    fn number_canonicalization() {
        assert_eq!(fmt("out = 0.50"), "out = 0.5\n");
        assert_eq!(fmt("out = 1.0"), "out = 1\n");
        assert_eq!(
            fmt("out = lag(velocity, 50ms)"),
            "out = lag(velocity, 50ms)\n"
        );
    }

    #[test]
    fn does_not_constant_fold() {
        // The formatter is not an optimizer (decision #7).
        assert_eq!(fmt("out = 1 + 2 * 3"), "out = 1 + 2 * 3\n");
    }

    #[test]
    fn ternary_and_calls() {
        assert_eq!(
            fmt("out = velocity>0.8?lfo:lfo*0.2"),
            "out = velocity > 0.8 ? lfo : lfo * 0.2\n"
        );
        assert_eq!(fmt("out = clamp(a,0,1)"), "out = clamp(a, 0, 1)\n");
    }

    #[test]
    fn address_without_instance() {
        assert_eq!(
            fmt("src o=osc.detune\nout=o"),
            "src o = osc.detune\n\nout = o\n"
        );
    }

    #[test]
    fn own_line_comment_leads_statement() {
        // A leading own-line comment is always followed by one blank line.
        let out = fmt("# header\nout = 1");
        assert_eq!(out, "# header\n\nout = 1\n");
    }

    #[test]
    fn comment_block_gets_exactly_one_blank_line() {
        // One or more own-line comments -> exactly one blank line before the
        // statement they document (no matter how many comment lines).
        let out = fmt("# line one\n# line two\nout = velocity");
        assert_eq!(out, "# line one\n# line two\n\nout = velocity\n");
    }

    #[test]
    fn trailing_comment_stays_on_line() {
        let out = fmt("out = velocity   #the velocity");
        assert_eq!(out, "out = velocity  # the velocity\n");
    }

    #[test]
    fn trailing_comment_does_not_force_blank_line() {
        // A comment on a code line stays on that line and does NOT insert a blank
        // before the next statement -- only own-line comment blocks do.
        let out = fmt("let a = 1 # first\nout = a");
        assert_eq!(out, "let a = 1  # first\nout = a\n");
    }

    #[test]
    fn comment_canonicalizes_and_is_idempotent() {
        idempotent("#header\nsrc lfo=lfo-1.out\nout=lfo # trail");
    }

    #[test]
    fn idempotent_on_examples() {
        idempotent(
            "src lfo=lfo-1.out\nsrc env=env-2.out\nlet depth=lerp(0.2,1.0,velocity)\nout=clamp(env*depth+lfo*0.1,0,1)",
        );
        idempotent("out = velocity > 0.8 ? lfo : lfo * 0.2");
        idempotent("out = -a ^ 2 + (b - c) * d");
    }

    #[test]
    fn parse_error_is_reported() {
        assert!(format("out = a < b < c").is_err());
        assert!(format("src lfo = lfo-1.out").is_err()); // missing out
    }
}
