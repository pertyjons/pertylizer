//! The YAMS compiler: AST → [`CompiledScript`] (+ the source-input list the
//! engine must fill each block).
//!
//! Responsibilities: name resolution in one namespace, reserved-built-in
//! shadowing checks (decision #5), arity checks, source-slot allocation,
//! duration folding, constant folding (shared with the VM via `Builtin::eval`),
//! `lag` coefficient caching (decision: precompute alpha for a constant time),
//! and the hard caps (decision #8). All errors are collected; any error → no
//! program.

use crate::ast::{BinaryOp, Binding, Expr, Local, Program, UnaryOp};
use crate::diag::Diagnostic;
use crate::lexer::DurationUnit;
use crate::parser::parse;
use crate::span::Span;
use crate::symbols::{
    self, Context, FnKind, Macro, Stateful, constant_value, context_from_name, macro_from_name,
    resolve_fn,
};
use synth_core::script::{
    Builtin, CompiledScript, MAX_INSTRUCTIONS, MAX_LOCALS, MAX_NESTING_DEPTH, MAX_SOURCE_LEN,
    MAX_SOURCES, MAX_STATE, Op, safe_div,
};

/// One value the engine fills into a source register before evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceInput {
    Macro(Macro),
    Context(Context),
    /// A module member address, e.g. `lfo-1.out`. `instance` defaults to 1.
    Module {
        module: String,
        instance: u32,
        member: String,
    },
}

/// A compiled YAMS program: the RT script plus the ordered list of inputs the
/// engine resolves and fills (index = source register).
#[derive(Debug, Clone)]
pub struct CompiledProgram {
    pub script: CompiledScript,
    pub inputs: Vec<SourceInput>,
}

/// Compile-time options supplied by the engine.
#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    /// Control rate in Hz — needed to precompute `lag` coefficients (a `lag`
    /// with a constant time folds its alpha here; recompile on a rate change).
    pub control_rate: f32,
}

impl Default for CompileOptions {
    fn default() -> Self {
        // 48 kHz / 64-sample blocks.
        Self {
            control_rate: 750.0,
        }
    }
}

/// Compile YAMS source into a [`CompiledProgram`], or `None` plus diagnostics.
#[must_use]
pub fn compile(src: &str, opts: &CompileOptions) -> (Option<CompiledProgram>, Vec<Diagnostic>) {
    let (program, diags) = parse(src);
    let mut compiler = Compiler {
        control_rate: opts.control_rate,
        code: Vec::new(),
        constants: Vec::new(),
        inputs: Vec::new(),
        bindings: Vec::new(),
        locals: Vec::new(),
        next_state: 0,
        depth_error: false,
        diags,
    };
    if src.len() > MAX_SOURCE_LEN {
        compiler.error(
            Span::new(0, 0),
            format!(
                "script is too long ({} > {MAX_SOURCE_LEN} bytes)",
                src.len()
            ),
        );
    }
    let result = compiler.run(&program);
    (result, compiler.diags)
}

struct BindingEntry {
    alias: String,
    module: String,
    instance: u32,
    member: String,
}

struct Compiler {
    control_rate: f32,
    code: Vec<Op>,
    constants: Vec<f32>,
    inputs: Vec<SourceInput>,
    bindings: Vec<BindingEntry>,
    locals: Vec<String>,
    next_state: u16,
    depth_error: bool,
    diags: Vec<Diagnostic>,
}

impl Compiler {
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn run(&mut self, program: &Program) -> Option<CompiledProgram> {
        for b in &program.bindings {
            self.register_binding(b);
        }
        for l in &program.locals {
            self.compile_local(l);
        }
        if let Some(out) = &program.output {
            self.compile_expr(&out.expr, 0);
        }

        // Final caps (per-allocation caps were checked as they were hit).
        let fallback = program.output.as_ref().map_or(Span::new(0, 0), |o| o.span);
        if self.code.len() > MAX_INSTRUCTIONS {
            self.error(
                fallback,
                format!(
                    "script is too complex ({} > {MAX_INSTRUCTIONS} instructions)",
                    self.code.len()
                ),
            );
        }

        if self.diags.iter().any(Diagnostic::is_error) {
            return None;
        }
        Some(CompiledProgram {
            script: CompiledScript::new(
                std::mem::take(&mut self.code),
                std::mem::take(&mut self.constants),
                self.inputs.len() as u16,
                self.next_state,
            ),
            inputs: std::mem::take(&mut self.inputs),
        })
    }

    // ---- statements -------------------------------------------------------

    fn name_taken(&self, name: &str) -> bool {
        self.bindings.iter().any(|b| b.alias == name) || self.locals.iter().any(|l| l == name)
    }

    fn register_binding(&mut self, b: &Binding) {
        let name = &b.alias.name;
        if symbols::is_reserved(name) {
            self.error(b.alias.span, format!("cannot shadow built-in `{name}`"));
        } else if self.name_taken(name) {
            self.error(b.alias.span, format!("duplicate name `{name}`"));
        }
        self.bindings.push(BindingEntry {
            alias: name.clone(),
            module: b.address.module.clone(),
            instance: b.address.instance.unwrap_or(1),
            member: b.address.member.clone(),
        });
    }

    fn compile_local(&mut self, l: &Local) {
        let name = &l.name.name;
        if symbols::is_reserved(name) {
            self.error(l.name.span, format!("cannot shadow built-in `{name}`"));
        } else if self.name_taken(name) {
            self.error(l.name.span, format!("duplicate name `{name}`"));
        }
        self.compile_expr(&l.expr, 0);
        let slot = self.locals.len() as u16;
        self.code.push(Op::StoreLocal(slot));
        self.locals.push(name.clone());
        if self.locals.len() > MAX_LOCALS {
            self.error(l.span, format!("too many locals (max {MAX_LOCALS})"));
        }
    }

    // ---- expressions ------------------------------------------------------

    fn compile_expr(&mut self, e: &Expr, depth: usize) {
        if depth > MAX_NESTING_DEPTH {
            if !self.depth_error {
                self.error(
                    e.span(),
                    format!("expression nesting too deep (max {MAX_NESTING_DEPTH})"),
                );
                self.depth_error = true;
            }
            self.emit_const(0.0);
            return;
        }
        if let Some(v) = const_eval(e) {
            self.emit_const(v);
            return;
        }
        match e {
            Expr::Var { name, span } => self.resolve_var(name, *span),
            Expr::Unary { op, rhs, .. } => {
                self.compile_expr(rhs, depth + 1);
                self.code.push(match op {
                    UnaryOp::Neg => Op::Neg,
                    UnaryOp::Not => Op::Not,
                });
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                self.compile_expr(lhs, depth + 1);
                self.compile_expr(rhs, depth + 1);
                self.code.push(binary_op(*op));
            }
            Expr::Ternary {
                cond, then, els, ..
            } => {
                self.compile_expr(cond, depth + 1);
                self.compile_expr(then, depth + 1);
                self.compile_expr(els, depth + 1);
                self.code.push(Op::Select);
            }
            Expr::Call { name, args, span } => self.compile_call(name, args, *span, depth),
            // Number is always constant (handled above); Error was already
            // reported by the parser. Emit 0 to keep the stack balanced.
            Expr::Number { value, unit, .. } => self.emit_const(fold_duration(*value, *unit)),
            Expr::Error { .. } => self.emit_const(0.0),
        }
    }

    fn resolve_var(&mut self, name: &str, span: Span) {
        if let Some(slot) = self.locals.iter().position(|n| n == name) {
            self.code.push(Op::LoadLocal(slot as u16));
            return;
        }
        if let Some(b) = self.bindings.iter().find(|b| b.alias == name) {
            let input = SourceInput::Module {
                module: b.module.clone(),
                instance: b.instance,
                member: b.member.clone(),
            };
            self.push_source(input);
            return;
        }
        if let Some(m) = macro_from_name(name) {
            self.push_source(SourceInput::Macro(m));
            return;
        }
        if let Some(ctx) = context_from_name(name) {
            self.push_source(SourceInput::Context(ctx));
            return;
        }
        self.error(span, format!("unknown identifier `{name}`"));
        self.emit_const(0.0);
    }

    fn compile_call(&mut self, name: &str, args: &[Expr], span: Span, depth: usize) {
        let Some(spec) = resolve_fn(name) else {
            self.error(span, format!("unknown function `{name}`"));
            self.emit_const(0.0);
            return;
        };
        let n = args.len();
        let arity_ok = n >= spec.min_arity && n <= spec.max_arity
            // `rand` accepts 0 or 2, never 1.
            && !(matches!(spec.kind, FnKind::Stateful(Stateful::Rand)) && n == 1);
        if !arity_ok {
            self.error(span, format!("wrong number of arguments to `{name}`"));
            for arg in args {
                self.compile_expr(arg, depth + 1);
            }
            self.emit_const(0.0);
            return;
        }
        match spec.kind {
            FnKind::Stateless(builtin) => {
                for arg in args {
                    self.compile_expr(arg, depth + 1);
                }
                self.code.push(Op::Call(builtin));
            }
            FnKind::Stateful(kind) => self.compile_stateful(kind, args, span, depth),
        }
    }

    fn compile_stateful(&mut self, kind: Stateful, args: &[Expr], span: Span, depth: usize) {
        let base = self.alloc_state(kind.state_cells(), span);
        let d = depth + 1;
        match kind {
            Stateful::Lag => {
                self.compile_expr(&args[0], d); // x
                self.emit_lag_alpha(&args[1], d);
                self.code.push(Op::Lag(base));
            }
            Stateful::Slew => {
                self.compile_expr(&args[0], d);
                self.compile_expr(&args[1], d);
                self.compile_expr(&args[2], d);
                self.code.push(Op::Slew(base));
            }
            Stateful::Sah => {
                self.compile_expr(&args[0], d);
                self.compile_expr(&args[1], d);
                self.code.push(Op::Sah(base));
            }
            Stateful::Accum => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Accum(base));
            }
            Stateful::Delta => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Delta(base));
            }
            Stateful::Phasor => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Phasor(base));
            }
            Stateful::Edge => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Edge(base));
            }
            Stateful::Counter => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Counter(base));
            }
            Stateful::Rand => {
                if args.len() == 2 {
                    self.compile_expr(&args[0], d);
                    self.compile_expr(&args[1], d);
                } else {
                    self.emit_const(0.0);
                    self.emit_const(1.0);
                }
                self.code.push(Op::Rand);
            }
            Stateful::White => {
                self.emit_const(-1.0);
                self.emit_const(1.0);
                self.code.push(Op::Rand);
            }
        }
    }

    /// Emit the `lag` smoothing coefficient onto the stack. If the time argument
    /// is a constant, alpha is precomputed here (coefficient caching); otherwise
    /// `alpha = 1 - exp(-1 / (sr * t))` is emitted as runtime bytecode.
    fn emit_lag_alpha(&mut self, time: &Expr, depth: usize) {
        if let Some(t) = const_eval(time) {
            self.emit_const(self.lag_alpha(t));
            return;
        }
        // Runtime: 1 - exp(-1 / (t * sr)).  Stack already has x below.
        self.emit_const(1.0);
        self.compile_expr(time, depth + 1);
        self.push_source(SourceInput::Context(Context::Sr));
        self.code.push(Op::Mul); // t * sr
        self.code.push(Op::Div); // 1 / (t * sr)
        self.code.push(Op::Neg);
        self.code.push(Op::Call(Builtin::Exp));
        self.code.push(Op::Neg);
        self.emit_const(1.0);
        self.code.push(Op::Add); // 1 - exp(...)
    }

    fn lag_alpha(&self, t_seconds: f32) -> f32 {
        if self.control_rate <= 0.0 || t_seconds <= 0.0 {
            return 1.0;
        }
        (1.0 - (-1.0 / (self.control_rate * t_seconds)).exp()).clamp(0.0, 1.0)
    }

    // ---- emit helpers -----------------------------------------------------

    fn emit_const(&mut self, v: f32) {
        let idx = self.intern_const(v);
        self.code.push(Op::PushConst(idx));
    }

    fn intern_const(&mut self, v: f32) -> u16 {
        if let Some(i) = self
            .constants
            .iter()
            .position(|&c| c == v || (c.is_nan() && v.is_nan()))
        {
            return i as u16;
        }
        self.constants.push(v);
        (self.constants.len() - 1) as u16
    }

    fn push_source(&mut self, input: SourceInput) {
        let slot = if let Some(i) = self.inputs.iter().position(|x| *x == input) {
            i as u16
        } else {
            self.inputs.push(input);
            let i = (self.inputs.len() - 1) as u16;
            if self.inputs.len() > MAX_SOURCES {
                self.error(
                    Span::new(0, 0),
                    format!("too many sources (max {MAX_SOURCES})"),
                );
            }
            i
        };
        self.code.push(Op::PushSource(slot));
    }

    fn alloc_state(&mut self, cells: u16, span: Span) -> u16 {
        let base = self.next_state;
        self.next_state = self.next_state.saturating_add(cells);
        if self.next_state as usize > MAX_STATE {
            self.error(span, format!("too much state (max {MAX_STATE} cells)"));
        }
        base
    }
}

// ---- constant folding (shared scalar math via Builtin::eval) ---------------

fn const_eval(e: &Expr) -> Option<f32> {
    match e {
        Expr::Number { value, unit, .. } => Some(fold_duration(*value, *unit)),
        Expr::Var { name, .. } => constant_value(name),
        Expr::Unary { op, rhs, .. } => {
            let v = const_eval(rhs)?;
            Some(match op {
                UnaryOp::Neg => -v,
                UnaryOp::Not => b2f(v == 0.0),
            })
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            let a = const_eval(lhs)?;
            let b = const_eval(rhs)?;
            Some(fold_binary(*op, a, b))
        }
        Expr::Ternary {
            cond, then, els, ..
        } => {
            let c = const_eval(cond)?;
            let t = const_eval(then)?;
            let f = const_eval(els)?;
            Some(if c != 0.0 { t } else { f })
        }
        Expr::Call { name, args, .. } => {
            let spec = resolve_fn(name)?;
            let FnKind::Stateless(builtin) = spec.kind else {
                return None;
            };
            if args.len() != builtin.arity() || args.len() > 3 {
                return None;
            }
            let mut vals = [0.0f32; 3];
            for (k, arg) in args.iter().enumerate() {
                vals[k] = const_eval(arg)?;
            }
            Some(builtin.eval(&vals[..args.len()]))
        }
        Expr::Error { .. } => None,
    }
}

fn fold_duration(value: f64, unit: Option<DurationUnit>) -> f32 {
    let seconds = match unit {
        Some(DurationUnit::Millis) => value * 0.001,
        Some(DurationUnit::Seconds) | None => value,
    };
    seconds as f32
}

fn fold_binary(op: BinaryOp, a: f32, b: f32) -> f32 {
    match op {
        BinaryOp::Add => a + b,
        BinaryOp::Sub => a - b,
        BinaryOp::Mul => a * b,
        BinaryOp::Div => safe_div(a, b),
        BinaryOp::Rem => {
            if b == 0.0 {
                0.0
            } else {
                a % b
            }
        }
        BinaryOp::Pow => a.powf(b),
        BinaryOp::Eq => b2f(a == b),
        BinaryOp::Ne => b2f(a != b),
        BinaryOp::Lt => b2f(a < b),
        BinaryOp::Gt => b2f(a > b),
        BinaryOp::Le => b2f(a <= b),
        BinaryOp::Ge => b2f(a >= b),
        BinaryOp::And => b2f(a != 0.0 && b != 0.0),
        BinaryOp::Or => b2f(a != 0.0 || b != 0.0),
    }
}

fn binary_op(op: BinaryOp) -> Op {
    match op {
        BinaryOp::Add => Op::Add,
        BinaryOp::Sub => Op::Sub,
        BinaryOp::Mul => Op::Mul,
        BinaryOp::Div => Op::Div,
        BinaryOp::Rem => Op::Rem,
        BinaryOp::Pow => Op::Pow,
        BinaryOp::Eq => Op::Eq,
        BinaryOp::Ne => Op::Ne,
        BinaryOp::Lt => Op::Lt,
        BinaryOp::Gt => Op::Gt,
        BinaryOp::Le => Op::Le,
        BinaryOp::Ge => Op::Ge,
        BinaryOp::And => Op::And,
        BinaryOp::Or => Op::Or,
    }
}

fn b2f(cond: bool) -> f32 {
    if cond { 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::script::{EvalContext, RegisterFile};

    const SR: f32 = 750.0;
    const SEED: u64 = 0x5EED;

    fn compile_ok(src: &str) -> CompiledProgram {
        let (prog, diags) = compile(src, &CompileOptions::default());
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        prog.expect("a compiled program")
    }

    fn errors(src: &str) -> Vec<String> {
        let (_prog, diags) = compile(src, &CompileOptions::default());
        diags
            .into_iter()
            .filter(Diagnostic::is_error)
            .map(|d| d.message)
            .collect()
    }

    /// Evaluate a program, filling each source via `fill`.
    fn eval(prog: &CompiledProgram, fill: impl Fn(&SourceInput) -> f32) -> f32 {
        let sources: Vec<f32> = prog.inputs.iter().map(fill).collect();
        let mut regs = RegisterFile::new(0, SEED);
        prog.script.eval(&sources, &mut regs, &EvalContext::new(SR))
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn constant_expression_folds() {
        let prog = compile_ok("out = 1 + 2 * 3");
        assert!(prog.inputs.is_empty());
        // Fully folded → a single PushConst.
        assert_eq!(prog.script.code().len(), 1);
        assert!(approx(eval(&prog, |_| 0.0), 7.0));
    }

    #[test]
    fn macro_source_is_read() {
        let prog = compile_ok("out = velocity * 0.6");
        assert_eq!(prog.inputs, vec![SourceInput::Macro(Macro::Velocity)]);
        assert!(approx(eval(&prog, |_| 0.5), 0.3));
    }

    #[test]
    fn the_demo_program() {
        let prog = compile_ok("src lfo = lfo-1.out\nout = lfo * 0.45 + velocity * 0.6");
        let out = eval(&prog, |inp| match inp {
            SourceInput::Module {
                module,
                instance,
                member,
            } if module == "lfo" && *instance == 1 && member == "out" => 1.0,
            SourceInput::Macro(Macro::Velocity) => 1.0,
            _ => 0.0,
        });
        assert!(approx(out, 1.05));
    }

    #[test]
    fn cross_source_multiply() {
        let prog = compile_ok("src lfo = lfo-1.out\nout = lfo * velocity");
        let out = eval(&prog, |inp| match inp {
            SourceInput::Module { .. } => 0.8,
            SourceInput::Macro(_) => 0.5,
            SourceInput::Context(_) => 0.0,
        });
        assert!(approx(out, 0.4));
    }

    #[test]
    fn ternary_is_eager_and_selects() {
        let prog = compile_ok("out = velocity > 0.8 ? 1 : 0");
        assert!(approx(eval(&prog, |_| 0.9), 1.0));
        assert!(approx(eval(&prog, |_| 0.5), 0.0));
    }

    #[test]
    fn local_resolves_before_use() {
        let prog = compile_ok("let depth = lerp(0.2, 1.0, velocity)\nout = depth");
        assert!(approx(eval(&prog, |_| 0.5), 0.6));
    }

    #[test]
    fn lag_with_constant_time_caches_coefficient() {
        // alpha folded; first block from rest = alpha * x with x = 1.
        let prog = compile_ok("out = lag(velocity, 50ms)");
        let alpha = 1.0 - (-1.0f32 / (SR * 0.05)).exp();
        assert!(approx(eval(&prog, |_| 1.0), alpha));
    }

    #[test]
    fn lag_with_dynamic_time_computes_alpha_at_runtime() {
        // A non-constant time emits the `1 - exp(-1/(t*sr))` bytecode; with the
        // same 0.05 s it must match the constant-folded path (guards the Div
        // operand order in the runtime-alpha lowering).
        let prog = compile_ok("out = lag(velocity, mod_wheel)");
        let alpha = 1.0 - (-1.0f32 / (SR * 0.05)).exp();
        let out = eval(&prog, |inp| match inp {
            SourceInput::Macro(Macro::Velocity) => 1.0,
            SourceInput::Macro(Macro::ModWheel) => 0.05,
            SourceInput::Context(Context::Sr) => SR,
            _ => 0.0,
        });
        assert!(
            approx(out, alpha),
            "dynamic lag {out} != constant alpha {alpha}"
        );
    }

    #[test]
    fn shadowing_a_builtin_is_an_error() {
        assert!(
            errors("let sin = 2\nout = sin")
                .iter()
                .any(|e| e.contains("shadow"))
        );
        assert!(
            errors("src velocity = lfo-1.out\nout = 0")
                .iter()
                .any(|e| e.contains("shadow"))
        );
    }

    #[test]
    fn unknown_identifier_and_function() {
        assert!(
            errors("out = foo")
                .iter()
                .any(|e| e.contains("unknown identifier"))
        );
        assert!(
            errors("out = bogus(1)")
                .iter()
                .any(|e| e.contains("unknown function"))
        );
    }

    #[test]
    fn arity_errors() {
        assert!(
            errors("out = clamp(1, 2)")
                .iter()
                .any(|e| e.contains("arguments"))
        );
        assert!(
            errors("out = rand(5)")
                .iter()
                .any(|e| e.contains("arguments"))
        );
    }

    #[test]
    fn state_cap_is_enforced() {
        // 17 accum() calls → 17 state cells > MAX_STATE (16).
        let body = (0..17)
            .map(|_| "accum(velocity)")
            .collect::<Vec<_>>()
            .join(" + ");
        let src = format!("out = {body}");
        assert!(errors(&src).iter().any(|e| e.contains("too much state")));
    }

    #[test]
    fn nesting_cap_is_enforced() {
        let body = vec!["velocity"; 40].join(" + ");
        let src = format!("out = {body}");
        assert!(errors(&src).iter().any(|e| e.contains("nesting too deep")));
    }

    #[test]
    fn duplicate_name_is_an_error() {
        assert!(
            errors("let x = 1\nlet x = 2\nout = x")
                .iter()
                .any(|e| e.contains("duplicate"))
        );
    }
}
