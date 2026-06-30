//! Recursive-descent parser: tokens → [`Program`].
//!
//! Follows the grammar documented in `docs/yams.md`. Key behaviours:
//! - **All errors collected** (decision #9): a malformed statement emits a
//!   diagnostic and recovers to the next separator, so later statements still
//!   parse. Lexer diagnostics are threaded in, so callers see one list.
//! - **Comparison operators are non-associative**: `a < b < c` and
//!   `a == b == c` are errors, not silent `(a < b) < c`.
//! - Operator precedence matches the grammar's ten-level ladder.

use crate::ast::{
    ArrayDecl, Assign, BinaryOp, Binding, BodyStmt, Expr, Ident, Local, ModuleRef, OutChannel,
    Output, Program, StateDecl, UnaryOp,
};
use crate::diag::Diagnostic;
use crate::lexer::{Token, TokenKind, lex};
use crate::span::Span;

/// Parse YAMS source into a [`Program`] plus all diagnostics (lexer + parser).
#[must_use]
pub fn parse(src: &str) -> (Program, Vec<Diagnostic>) {
    let (tokens, diags) = lex(src);
    let mut parser = Parser {
        tokens,
        pos: 0,
        diags,
    };
    let program = parser.program();
    (program, parser.diags)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
}

impl Parser {
    // ---- cursor -----------------------------------------------------------

    fn cur_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn cur_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn advance(&mut self) {
        if !self.is_eof() {
            self.pos += 1;
        }
    }

    fn is_eof(&self) -> bool {
        matches!(self.cur_kind(), TokenKind::Eof)
    }

    fn is(&self, kind: &TokenKind) -> bool {
        self.cur_kind() == kind
    }

    /// Consume the current token if it equals `kind` (unit variants only).
    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.is(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn error_here(&mut self, message: impl Into<String>) {
        let span = self.cur_span();
        self.error(span, message);
    }

    /// Skip tokens until a separator or EOF, then consume the separator. Used
    /// to resynchronize after a statement-level parse error.
    fn recover_to_sep(&mut self) {
        while !matches!(self.cur_kind(), TokenKind::Sep | TokenKind::Eof) {
            self.advance();
        }
        self.eat(&TokenKind::Sep);
    }

    fn skip_seps(&mut self) {
        while self.eat(&TokenKind::Sep) {}
    }

    /// Consume the statement terminator: a separator, or EOF (a missing final
    /// newline is tolerated). Anything else is an error + recovery.
    fn expect_terminator(&mut self) {
        match self.cur_kind() {
            TokenKind::Sep => self.advance(),
            TokenKind::Eof => {}
            _ => {
                self.error_here("expected end of statement");
                self.recover_to_sep();
            }
        }
    }

    fn eat_ident(&mut self) -> Option<Ident> {
        if let TokenKind::Ident(name) = self.cur_kind() {
            let id = Ident {
                name: name.clone(),
                span: self.cur_span(),
            };
            self.advance();
            Some(id)
        } else {
            None
        }
    }

    /// Like [`Self::eat_ident`] but also accepts keyword spellings (`out`,
    /// `src`, `let`) — used for member names so `lfo-1.out` works.
    fn eat_member(&mut self) -> Option<Ident> {
        if let Some(text) = self.cur_kind().ident_text() {
            let id = Ident {
                name: text.to_string(),
                span: self.cur_span(),
            };
            self.advance();
            Some(id)
        } else {
            None
        }
    }

    // ---- program / statements --------------------------------------------

    fn program(&mut self) -> Program {
        self.skip_seps();
        let (bindings, arrays, states) = self.parse_header();
        let (body, outputs) = self.parse_body();
        self.skip_seps();
        if !self.is_eof() {
            self.error_here("unexpected token after the 'out' statement");
            while !self.is_eof() {
                self.advance();
            }
        }
        Program {
            bindings,
            arrays,
            states,
            body,
            outputs,
        }
    }

    /// Parse the header section: `src` bindings, `arr` const-table declarations,
    /// and `state` cell declarations, in any order, until the body (`let` /
    /// assignment / `out`) begins.
    fn parse_header(&mut self) -> (Vec<Binding>, Vec<ArrayDecl>, Vec<StateDecl>) {
        let mut bindings = Vec::new();
        let mut arrays = Vec::new();
        let mut states = Vec::new();
        loop {
            match self.cur_kind() {
                TokenKind::Src => match self.parse_binding() {
                    Some(b) => bindings.push(b),
                    None => self.recover_to_sep(),
                },
                TokenKind::Arr => match self.parse_array_decl() {
                    Some(a) => arrays.push(a),
                    None => self.recover_to_sep(),
                },
                TokenKind::State => match self.parse_state_decl() {
                    Some(s) => states.push(s),
                    None => self.recover_to_sep(),
                },
                _ => break,
            }
        }
        (bindings, arrays, states)
    }

    /// `state <name> = <expr>` — declare a persistent state cell.
    fn parse_state_decl(&mut self) -> Option<StateDecl> {
        let start = self.cur_span();
        self.advance(); // `state`
        let Some(name) = self.eat_ident() else {
            self.error_here("expected a name after 'state'");
            return None;
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' in state declaration");
            return None;
        }
        let init = self.expr();
        let span = start.to(init.span());
        self.expect_terminator();
        Some(StateDecl { name, init, span })
    }

    fn parse_array_decl(&mut self) -> Option<ArrayDecl> {
        let start = self.cur_span();
        self.advance(); // `arr`
        let Some(name) = self.eat_ident() else {
            self.error_here("expected an array name after 'arr'");
            return None;
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' in arr declaration");
            return None;
        }
        if !self.eat(&TokenKind::LBracket) {
            self.error_here("expected '[' to begin the array elements");
            return None;
        }
        let mut elements = Vec::new();
        if !self.is(&TokenKind::RBracket) {
            loop {
                elements.push(self.expr());
                if self.eat(&TokenKind::Comma) {
                    continue;
                }
                break;
            }
        }
        let end = self.cur_span();
        if !self.eat(&TokenKind::RBracket) {
            self.error_here("expected ',' or ']' in array elements");
            return None;
        }
        let span = start.to(end);
        self.expect_terminator();
        Some(ArrayDecl {
            name,
            elements,
            span,
        })
    }

    fn parse_binding(&mut self) -> Option<Binding> {
        let start = self.cur_span();
        self.advance(); // `src`
        let Some(alias) = self.eat_ident() else {
            self.error_here("expected an alias name after 'src'");
            return None;
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' in src binding");
            return None;
        }
        let address = self.parse_address()?;
        let span = start.to(address.span);
        self.expect_terminator();
        Some(Binding {
            alias,
            address,
            span,
        })
    }

    fn parse_address(&mut self) -> Option<ModuleRef> {
        let Some(module) = self.eat_ident() else {
            self.error_here("expected a module name");
            return None;
        };
        let mut instance = None;
        if self.eat(&TokenKind::Minus) {
            match self.cur_kind() {
                TokenKind::Number {
                    value,
                    is_int: true,
                    ..
                } => {
                    instance = Some(*value as u32);
                    self.advance();
                }
                _ => {
                    self.error_here("expected an integer instance number after '-'");
                    return None;
                }
            }
        }
        if !self.eat(&TokenKind::Dot) {
            self.error_here("expected '.' before the member name");
            return None;
        }
        let Some(member) = self.eat_member() else {
            self.error_here("expected a member (port or param) name");
            return None;
        };
        let span = module.span.to(member.span);
        Some(ModuleRef {
            module: module.name,
            instance,
            member: member.name,
            span,
        })
    }

    /// Parse the body: an ordered sequence of `let` locals and `<name> = expr`
    /// state assignments, terminated by one or more `out` statements (a single
    /// mono `out`, or `out.left` / `out.right`). Order is preserved (it matters
    /// for state).
    fn parse_body(&mut self) -> (Vec<BodyStmt>, Vec<Output>) {
        let mut body = Vec::new();
        loop {
            match self.cur_kind() {
                TokenKind::Let => match self.parse_local() {
                    Some(l) => body.push(BodyStmt::Local(l)),
                    None => self.recover_to_sep(),
                },
                // A bare identifier at statement level is a `state` assignment
                // (`s = expr`); nothing else starts with an identifier here.
                TokenKind::Ident(_) => match self.parse_assign() {
                    Some(a) => body.push(BodyStmt::Assign(a)),
                    None => self.recover_to_sep(),
                },
                _ => break,
            }
        }
        let mut outputs = Vec::new();
        while matches!(self.cur_kind(), TokenKind::Out) {
            match self.parse_output() {
                Some(o) => outputs.push(o),
                None => self.recover_to_sep(),
            }
        }
        if outputs.is_empty() {
            self.error_here("expected 'out = …'");
        }
        (body, outputs)
    }

    /// `<name> = <expr>` — a state-cell assignment statement.
    fn parse_assign(&mut self) -> Option<Assign> {
        let Some(name) = self.eat_ident() else {
            self.error_here("expected a name");
            return None;
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' (a bare statement must be a `state` assignment)");
            return None;
        }
        let expr = self.expr();
        let span = name.span.to(expr.span());
        self.expect_terminator();
        Some(Assign { name, expr, span })
    }

    fn parse_local(&mut self) -> Option<Local> {
        let start = self.cur_span();
        self.advance(); // `let`
        let Some(name) = self.eat_ident() else {
            self.error_here("expected a name after 'let'");
            return None;
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' in let binding");
            return None;
        }
        let expr = self.expr();
        let span = start.to(expr.span());
        self.expect_terminator();
        Some(Local { name, expr, span })
    }

    fn parse_output(&mut self) -> Option<Output> {
        let start = self.cur_span();
        self.advance(); // `out`
        // Optional `.left` / `.right` channel selector (stereo multi-out). A bare
        // `out` is mono. The compiler rejects channel outputs in a control script.
        let channel = if self.eat(&TokenKind::Dot) {
            match self.eat_member() {
                Some(id) if id.name == "left" => OutChannel::Left,
                Some(id) if id.name == "right" => OutChannel::Right,
                Some(id) => {
                    self.error(id.span, "expected `left` or `right` after `out.`");
                    OutChannel::Mono
                }
                None => {
                    self.error_here("expected `left` or `right` after `out.`");
                    return None;
                }
            }
        } else {
            OutChannel::Mono
        };
        if !self.eat(&TokenKind::Eq) {
            self.error_here("expected '=' in 'out' statement");
            return None;
        }
        let expr = self.expr();
        let span = start.to(expr.span());
        // Outputs need a terminator now that several may follow one another.
        self.expect_terminator();
        Some(Output {
            channel,
            expr,
            span,
        })
    }

    // ---- expressions ------------------------------------------------------

    fn expr(&mut self) -> Expr {
        self.ternary()
    }

    fn ternary(&mut self) -> Expr {
        let cond = self.logic_or();
        if self.eat(&TokenKind::Question) {
            let then = self.expr();
            if !self.eat(&TokenKind::Colon) {
                self.error_here("expected ':' in ternary");
            }
            let els = self.ternary();
            let span = cond.span().to(els.span());
            return Expr::Ternary {
                cond: Box::new(cond),
                then: Box::new(then),
                els: Box::new(els),
                span,
            };
        }
        cond
    }

    fn logic_or(&mut self) -> Expr {
        let mut lhs = self.logic_and();
        while self.eat(&TokenKind::OrOr) {
            let rhs = self.logic_and();
            lhs = binary(BinaryOp::Or, lhs, rhs);
        }
        lhs
    }

    fn logic_and(&mut self) -> Expr {
        let mut lhs = self.equality();
        while self.eat(&TokenKind::AndAnd) {
            let rhs = self.equality();
            lhs = binary(BinaryOp::And, lhs, rhs);
        }
        lhs
    }

    fn equality(&mut self) -> Expr {
        let lhs = self.relational();
        let op = match self.cur_kind() {
            TokenKind::EqEq => Some(BinaryOp::Eq),
            TokenKind::BangEq => Some(BinaryOp::Ne),
            _ => None,
        };
        let Some(op) = op else { return lhs };
        self.advance();
        let rhs = self.relational();
        let node = binary(op, lhs, rhs);
        if matches!(self.cur_kind(), TokenKind::EqEq | TokenKind::BangEq) {
            self.chained_comparison_error(|k| matches!(k, TokenKind::EqEq | TokenKind::BangEq));
        }
        node
    }

    fn relational(&mut self) -> Expr {
        let lhs = self.additive();
        let op = match self.cur_kind() {
            TokenKind::Lt => Some(BinaryOp::Lt),
            TokenKind::Gt => Some(BinaryOp::Gt),
            TokenKind::Le => Some(BinaryOp::Le),
            TokenKind::Ge => Some(BinaryOp::Ge),
            _ => None,
        };
        let Some(op) = op else { return lhs };
        self.advance();
        let rhs = self.additive();
        let node = binary(op, lhs, rhs);
        if is_relational(self.cur_kind()) {
            self.chained_comparison_error(is_relational);
        }
        node
    }

    /// Report a non-associative comparison chain and consume the rest of the
    /// chain (so the remaining operands are not re-reported as stray tokens).
    fn chained_comparison_error(&mut self, is_op: fn(&TokenKind) -> bool) {
        self.error_here("comparison operators cannot be chained; parenthesize");
        while is_op(self.cur_kind()) {
            self.advance();
            // Discard the extra operand to keep recovering.
            let _ = self.additive();
        }
    }

    fn additive(&mut self) -> Expr {
        let mut lhs = self.multiplicative();
        loop {
            let op = match self.cur_kind() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.multiplicative();
            lhs = binary(op, lhs, rhs);
        }
        lhs
    }

    fn multiplicative(&mut self) -> Expr {
        let mut lhs = self.unary();
        loop {
            let op = match self.cur_kind() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => break,
            };
            self.advance();
            let rhs = self.unary();
            lhs = binary(op, lhs, rhs);
        }
        lhs
    }

    fn unary(&mut self) -> Expr {
        let op = match self.cur_kind() {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Bang => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.cur_span();
            self.advance();
            let rhs = self.unary();
            let span = start.to(rhs.span());
            return Expr::Unary {
                op,
                rhs: Box::new(rhs),
                span,
            };
        }
        self.power()
    }

    fn power(&mut self) -> Expr {
        let base = self.primary();
        if self.eat(&TokenKind::Caret) {
            // Right-associative: the exponent is a full `unary`.
            let exp = self.unary();
            return binary(BinaryOp::Pow, base, exp);
        }
        base
    }

    fn primary(&mut self) -> Expr {
        let span = self.cur_span();
        match self.cur_kind() {
            TokenKind::Number { value, unit, .. } => {
                let (value, unit) = (*value, *unit);
                self.advance();
                Expr::Number { value, unit, span }
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                if self.eat(&TokenKind::LParen) {
                    let (args, end) = self.parse_args();
                    Expr::Call {
                        name,
                        args,
                        span: span.to(end),
                    }
                } else if self.eat(&TokenKind::LBracket) {
                    let index = self.expr();
                    let end = self.cur_span();
                    if !self.eat(&TokenKind::RBracket) {
                        self.error_here("expected ']' to close the index");
                    }
                    Expr::Index {
                        name,
                        index: Box::new(index),
                        span: span.to(end),
                    }
                } else {
                    Expr::Var { name, span }
                }
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.expr();
                if !self.eat(&TokenKind::RParen) {
                    self.error_here("expected ')'");
                }
                // Grouping is structural in the tree; return the inner node.
                inner
            }
            _ => {
                self.error_here("expected an expression");
                Expr::Error { span }
            }
        }
    }

    /// Parse a `(`-opened argument list up to and including `)`. Returns the
    /// args and the span of the closing token (or the error site).
    fn parse_args(&mut self) -> (Vec<Expr>, Span) {
        let mut args = Vec::new();
        if !self.is(&TokenKind::RParen) {
            loop {
                args.push(self.expr());
                if self.eat(&TokenKind::Comma) {
                    continue;
                }
                break;
            }
        }
        let end = self.cur_span();
        if !self.eat(&TokenKind::RParen) {
            self.error_here("expected ',' or ')' in argument list");
        }
        (args, end)
    }
}

fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span().to(rhs.span());
    Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

fn is_relational(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Lt | TokenKind::Gt | TokenKind::Le | TokenKind::Ge
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Expr;

    fn parse_ok(src: &str) -> Program {
        let (program, diags) = parse(src);
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        program
    }

    fn errors(src: &str) -> Vec<String> {
        let (_program, diags) = parse(src);
        diags
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| d.message)
            .collect()
    }

    fn out_expr(program: &Program) -> &Expr {
        &program.outputs.first().expect("an output").expr
    }

    #[test]
    fn binding_with_instance() {
        let p = parse_ok("src lfo = lfo-1.out\nout = lfo");
        assert_eq!(p.bindings.len(), 1);
        let b = &p.bindings[0];
        assert_eq!(b.alias.name, "lfo");
        assert_eq!(b.address.module, "lfo");
        assert_eq!(b.address.instance, Some(1));
        assert_eq!(b.address.member, "out");
    }

    #[test]
    fn binding_without_instance_defaults_later() {
        let p = parse_ok("src o = osc.detune\nout = o");
        assert_eq!(p.bindings[0].address.instance, None);
        assert_eq!(p.bindings[0].address.member, "detune");
    }

    #[test]
    fn locals_and_output() {
        let p = parse_ok("let depth = 0.5\nout = depth");
        assert_eq!(p.body.len(), 1);
        let BodyStmt::Local(l) = &p.body[0] else {
            panic!("expected a local");
        };
        assert_eq!(l.name.name, "depth");
        assert!(matches!(out_expr(&p), Expr::Var { .. }));
    }

    #[test]
    fn state_decl_and_assignment_parse() {
        // `state` is a header declaration; `s = expr` is an ordered body statement.
        let p = parse_ok("state s = 0\nlet x = s + 1\ns = x\nout = s");
        assert_eq!(p.states.len(), 1);
        assert_eq!(p.states[0].name.name, "s");
        assert_eq!(p.body.len(), 2);
        assert!(matches!(p.body[0], BodyStmt::Local(_)));
        let BodyStmt::Assign(a) = &p.body[1] else {
            panic!("expected an assignment");
        };
        assert_eq!(a.name.name, "s");
    }

    #[test]
    fn precedence_mul_binds_tighter_than_add() {
        // 1 + 2 * 3 → Add(1, Mul(2, 3))
        let p = parse_ok("out = 1 + 2 * 3");
        let Expr::Binary { op, rhs, .. } = out_expr(&p) else {
            panic!("expected binary");
        };
        assert_eq!(*op, BinaryOp::Add);
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }

    #[test]
    fn power_is_right_associative() {
        // 2 ^ 3 ^ 2 → Pow(2, Pow(3, 2))
        let p = parse_ok("out = 2 ^ 3 ^ 2");
        let Expr::Binary { op, rhs, .. } = out_expr(&p) else {
            panic!("expected binary");
        };
        assert_eq!(*op, BinaryOp::Pow);
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::Pow,
                ..
            }
        ));
    }

    #[test]
    fn unary_neg_binds_looser_than_power() {
        // -a ^ 2 → Neg(Pow(a, 2))
        let p = parse_ok("out = -a ^ 2");
        let Expr::Unary { op, rhs, .. } = out_expr(&p) else {
            panic!("expected unary");
        };
        assert_eq!(*op, UnaryOp::Neg);
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::Pow,
                ..
            }
        ));
    }

    #[test]
    fn ternary_is_right_associative() {
        // a ? b : c ? d : e → a ? b : (c ? d : e)
        let p = parse_ok("out = a ? b : c ? d : e");
        let Expr::Ternary { els, .. } = out_expr(&p) else {
            panic!("expected ternary");
        };
        assert!(matches!(**els, Expr::Ternary { .. }));
    }

    #[test]
    fn call_with_args() {
        let p = parse_ok("out = clamp(a, 0, 1)");
        let Expr::Call { name, args, .. } = out_expr(&p) else {
            panic!("expected call");
        };
        assert_eq!(name, "clamp");
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn mixed_comparison_levels_allowed() {
        // a < b == c → (a < b) == c — different levels, not chaining.
        let p = parse_ok("out = a < b == c");
        let Expr::Binary { op, lhs, .. } = out_expr(&p) else {
            panic!("expected binary");
        };
        assert_eq!(*op, BinaryOp::Eq);
        assert!(matches!(
            **lhs,
            Expr::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
    }

    #[test]
    fn chained_relational_is_an_error() {
        let errs = errors("out = a < b < c");
        assert!(
            errs.iter().any(|e| e.contains("cannot be chained")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn chained_equality_is_an_error() {
        let errs = errors("out = a == b == c");
        assert!(
            errs.iter().any(|e| e.contains("cannot be chained")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn multi_out_channels_parse() {
        // `out.left` / `out.right` parse into channel-tagged Output statements.
        let p = parse_ok("out.left = a\nout.right = b");
        assert_eq!(p.outputs.len(), 2);
        assert_eq!(p.outputs[0].channel, OutChannel::Left);
        assert_eq!(p.outputs[1].channel, OutChannel::Right);
        // A bare `out` is mono.
        let m = parse_ok("out = a");
        assert_eq!(m.outputs[0].channel, OutChannel::Mono);
    }

    #[test]
    fn bad_output_forms_are_errors() {
        // A bare identifier after `out` (no dot) is not a channel — it's a syntax
        // error (the old "named output" form is gone).
        assert!(!errors("out foo = 1").is_empty());
        // An unknown channel after the dot is rejected.
        assert!(
            errors("out.middle = 1")
                .iter()
                .any(|e| e.contains("left` or `right"))
        );
    }

    #[test]
    fn missing_output_is_an_error() {
        let errs = errors("src lfo = lfo-1.out");
        assert!(errs.iter().any(|e| e.contains("out")), "got: {errs:?}");
    }

    #[test]
    fn non_integer_instance_is_an_error() {
        let errs = errors("src x = lfo-1.5\nout = x");
        assert!(
            errs.iter().any(|e| e.contains("integer instance")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn recovers_after_a_bad_binding() {
        // First binding is malformed (no '='); the second + output still parse.
        let (program, diags) = parse("src a lfo-1.out\nsrc b = osc.out\nout = b");
        assert!(diags.iter().any(Diagnostic::is_error));
        assert_eq!(program.bindings.len(), 1);
        assert_eq!(program.bindings[0].alias.name, "b");
        assert!(!program.outputs.is_empty());
    }

    #[test]
    fn array_decl_parses() {
        let p = parse_ok("arr seq = [0, 0.5, 1, -0.3]\nout = seq[0]");
        assert_eq!(p.arrays.len(), 1);
        assert_eq!(p.arrays[0].name.name, "seq");
        assert_eq!(p.arrays[0].elements.len(), 4);
        let Expr::Index { name, .. } = out_expr(&p) else {
            panic!("expected index");
        };
        assert_eq!(name, "seq");
    }

    #[test]
    fn empty_array_decl_parses() {
        // Emptiness is a compile-time error, not a parse error.
        let p = parse_ok("arr e = []\nout = 0");
        assert!(p.arrays[0].elements.is_empty());
    }

    #[test]
    fn index_with_complex_expression() {
        let p = parse_ok("arr s = [1, 2]\nout = s[floor(beat) % 2]");
        let Expr::Index { name, index, .. } = out_expr(&p) else {
            panic!("expected index");
        };
        assert_eq!(name, "s");
        assert!(matches!(
            **index,
            Expr::Binary {
                op: BinaryOp::Rem,
                ..
            }
        ));
    }

    #[test]
    fn header_mixes_src_and_arr() {
        let p = parse_ok("src lfo = lfo-1.out\narr s = [1, 2]\nout = lfo + s[0]");
        assert_eq!(p.bindings.len(), 1);
        assert_eq!(p.arrays.len(), 1);
    }

    #[test]
    fn unterminated_array_is_an_error() {
        let errs = errors("arr s = [1, 2\nout = 0");
        assert!(!errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn worked_example_full_program() {
        let p = parse_ok(
            "src lfo = lfo-1.out\nsrc env = env-2.out\nlet depth = lerp(0.2, 1.0, velocity)\nout = clamp(env * depth + lfo * 0.1, 0, 1)",
        );
        assert_eq!(p.bindings.len(), 2);
        assert_eq!(p.body.len(), 1);
        assert!(matches!(out_expr(&p), Expr::Call { .. }));
    }
}
