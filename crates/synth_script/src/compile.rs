//! The YAMS compiler: AST → [`CompiledScript`] (+ the source-input list the
//! engine must fill each block).
//!
//! Responsibilities: name resolution in one namespace, reserved-built-in
//! shadowing checks (decision #5), arity checks, source-slot allocation,
//! duration folding, constant folding (shared with the VM via `Builtin::eval`),
//! `lag` coefficient caching (decision: precompute alpha for a constant time),
//! and the hard caps (decision #8). All errors are collected; any error → no
//! program.

use crate::ast::{ArrayDecl, BinaryOp, Binding, Expr, Local, Program, UnaryOp};
use crate::diag::Diagnostic;
use crate::lexer::DurationUnit;
use crate::parser::parse;
use crate::span::Span;
use crate::symbols::{
    self, Context, FnKind, Macro, Stateful, constant_value, context_from_name, macro_from_name,
    resolve_fn,
};
use synth_core::script::{
    BoundScript, Builtin, CompiledScript, MAX_ARRAY_STORAGE, MAX_ARRAYS, MAX_INSTRUCTIONS,
    MAX_LOCALS, MAX_NESTING_DEPTH, MAX_SOURCE_LEN, MAX_SOURCES, MAX_STATE, Op, ScriptContext,
    ScriptInput, safe_div,
};
use synth_core::{MacroSource, SrcAddr};

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

impl CompiledProgram {
    /// Bind this program to engine-resolvable addresses, producing the RT-side
    /// [`BoundScript`] the voice runs. `source` is the canonical YAMS text kept
    /// with it for persistence and inspection. A module input whose prefix is not
    /// a known module type binds to [`ScriptInput::Zero`] (reads `0.0`) rather
    /// than failing — disable-and-keep, consistent with Step 1's dangling-address
    /// policy (decision #3).
    #[must_use]
    pub fn into_bound(self, source: String) -> BoundScript {
        let inputs = self.inputs.iter().map(input_to_runtime).collect();
        BoundScript::new(self.script, inputs, source)
    }
}

/// Map one compiler source input to the runtime [`ScriptInput`] the voice fills.
fn input_to_runtime(input: &SourceInput) -> ScriptInput {
    match input {
        SourceInput::Macro(m) => ScriptInput::Source(SrcAddr::Macro(macro_to_runtime(*m))),
        SourceInput::Context(c) => ScriptInput::Context(context_to_runtime(*c)),
        SourceInput::Module {
            module,
            instance,
            member,
        } => {
            // Reconstruct the canonical address and let `SrcAddr::parse` resolve
            // the prefix → module type (the same parser the scalar path uses), so
            // an unknown prefix degrades to a zero register instead of erroring.
            let addr = format!("{module}-{instance}.{member}");
            SrcAddr::parse(&addr).map_or(ScriptInput::Zero, ScriptInput::Source)
        }
    }
}

fn macro_to_runtime(m: Macro) -> MacroSource {
    match m {
        Macro::Velocity => MacroSource::Velocity,
        Macro::ModWheel => MacroSource::ModWheel,
        Macro::Aftertouch => MacroSource::Aftertouch,
        Macro::PitchBend => MacroSource::PitchBend,
        Macro::Note => MacroSource::NoteNumber,
        Macro::PolyAt => MacroSource::PolyAftertouch,
    }
}

fn context_to_runtime(c: Context) -> ScriptContext {
    match c {
        Context::Gate => ScriptContext::Gate,
        Context::GateOn => ScriptContext::GateOn,
        Context::Age => ScriptContext::Age,
        Context::Cr => ScriptContext::Cr,
        Context::Beat => ScriptContext::Beat,
        Context::BarPhase => ScriptContext::BarPhase,
        Context::Tempo => ScriptContext::Tempo,
        Context::Playing => ScriptContext::Playing,
    }
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
        arrays: Vec::new(),
        array_storage: 0,
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

/// A registered `arr` table: its name and where its elements live in the shared
/// constant pool (`constants[base..base+len]`).
struct ArrayEntry {
    name: String,
    base: u16,
    len: u16,
}

struct Compiler {
    control_rate: f32,
    code: Vec<Op>,
    constants: Vec<f32>,
    inputs: Vec<SourceInput>,
    bindings: Vec<BindingEntry>,
    arrays: Vec<ArrayEntry>,
    array_storage: usize,
    locals: Vec<String>,
    next_state: u16,
    depth_error: bool,
    diags: Vec<Diagnostic>,
}

impl Compiler {
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn warn(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::warning(span, message));
    }

    fn run(&mut self, program: &Program) -> Option<CompiledProgram> {
        for b in &program.bindings {
            self.register_binding(b);
        }
        // Arrays are baked into the constant pool *first* so each table occupies
        // a contiguous, never-deduplicated `constants[base..base+len]` slice that
        // `IndexConst` reads directly. Any later `emit_const` appends after them.
        for a in &program.arrays {
            self.register_array(a);
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
        self.bindings.iter().any(|b| b.alias == name)
            || self.arrays.iter().any(|a| a.name == name)
            || self.locals.iter().any(|l| l == name)
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

    fn register_array(&mut self, a: &ArrayDecl) {
        let name = &a.name.name;
        if symbols::is_reserved(name) {
            self.error(a.name.span, format!("cannot shadow built-in `{name}`"));
        } else if self.name_taken(name) {
            self.error(a.name.span, format!("duplicate name `{name}`"));
        }
        if a.elements.is_empty() {
            self.error(
                a.span,
                format!("array `{name}` must have at least one element"),
            );
            return;
        }
        if self.arrays.len() >= MAX_ARRAYS {
            self.error(a.span, format!("too many arrays (max {MAX_ARRAYS})"));
        }
        // Fold each element to a constant and bake it contiguously into the pool.
        let base = self.constants.len() as u16;
        for el in &a.elements {
            let Some(v) = const_eval(el) else {
                self.error(el.span(), "array elements must be constant");
                self.constants.push(0.0);
                continue;
            };
            self.constants.push(v);
        }
        let len = a.elements.len();
        self.array_storage += len;
        if self.array_storage > MAX_ARRAY_STORAGE {
            self.error(
                a.span,
                format!("array storage exceeds {MAX_ARRAY_STORAGE} elements"),
            );
        }
        self.arrays.push(ArrayEntry {
            name: name.clone(),
            base,
            len: len as u16,
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
            Expr::Index { name, index, span } => self.compile_index(name, index, *span, depth),
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
        if self.arrays.iter().any(|a| a.name == name) {
            self.error(
                span,
                format!("arrays cannot be used directly in arithmetic; index it with `{name}[i]`"),
            );
            self.emit_const(0.0);
            return;
        }
        self.error(span, format!("unknown identifier `{name}`"));
        self.emit_const(0.0);
    }

    /// Resolve an array name to its `(base, len)` in the constant pool, if any.
    fn find_array(&self, name: &str) -> Option<(u16, u16)> {
        self.arrays
            .iter()
            .find(|a| a.name == name)
            .map(|a| (a.base, a.len))
    }

    fn compile_index(&mut self, name: &str, index: &Expr, span: Span, depth: usize) {
        let Some((base, len)) = self.find_array(name) else {
            if self.name_taken(name)
                || macro_from_name(name).is_some()
                || context_from_name(name).is_some()
            {
                self.error(span, format!("`{name}` is not an array"));
            } else {
                self.error(span, format!("unknown identifier `{name}`"));
            }
            self.emit_const(0.0);
            return;
        };
        // Warn on a provably-constant out-of-bounds index. The runtime floors
        // then clamps (`IndexConst`), so this is never an error — the read is
        // safe — but a literal/folded index outside `0..=len-1` is almost always
        // an author mistake. Only indices `const_eval` can fold are checked here;
        // `len(t)`, sources, and other dynamic indices fall back to the safe
        // runtime clamp.
        if let Some(i) = const_eval(index).map(f32::floor)
            && (i < 0.0 || i >= f32::from(len))
        {
            let last = len.saturating_sub(1);
            self.warn(
                span,
                format!(
                    "array index {i} is out of bounds for `{name}` (length {len}, valid 0..={last}); it clamps to the nearest element at runtime"
                ),
            );
        }
        self.compile_expr(index, depth + 1);
        self.code.push(Op::IndexConst { base, len });
    }

    fn compile_call(&mut self, name: &str, args: &[Expr], span: Span, depth: usize) {
        // `len(arr)` is not a stack-argument builtin — it folds to a compile-time
        // constant (the array's element count), so it is handled before the
        // normal function table (an array name is never a stack value).
        if name == "len" {
            self.compile_len(args, span);
            return;
        }
        // `table_lin`/`scale_snap` take an array argument that is never a stack
        // value, so they lower to dedicated opcodes carrying the array's pool
        // location — handled before the normal stack-argument function table.
        if name == "table_lin" {
            self.compile_array_fn(name, args, span, depth, 0, |base, len| Op::TableLin {
                base,
                len,
            });
            return;
        }
        if name == "scale_snap" {
            self.compile_array_fn(name, args, span, depth, 1, |base, len| Op::ScaleSnap {
                base,
                len,
            });
            return;
        }
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

    /// Compile `len(arr)` → a constant equal to the array's element count. The
    /// sole argument must be a bare array name (arrays are not stack values).
    fn compile_len(&mut self, args: &[Expr], span: Span) {
        if args.len() != 1 {
            self.error(span, "wrong number of arguments to `len`");
            self.emit_const(0.0);
            return;
        }
        if let Expr::Var { name, .. } = &args[0]
            && let Some((_, len)) = self.find_array(name)
        {
            self.emit_const(f32::from(len));
            return;
        }
        self.error(
            args[0].span(),
            "expected an array name as the argument to `len`",
        );
        self.emit_const(0.0);
    }

    /// Compile an array-taking builtin (`table_lin`, `scale_snap`) to its
    /// dedicated opcode. `arr_arg` is the index of the array-name argument; the
    /// other argument is the numeric operand pushed onto the stack. The opcode
    /// (built by `make_op` from the resolved `base`/`len`) reads the table
    /// directly from the constant pool — the array is never a stack value.
    fn compile_array_fn(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        depth: usize,
        arr_arg: usize,
        make_op: impl Fn(u16, u16) -> Op,
    ) {
        if args.len() != 2 {
            self.error(span, format!("wrong number of arguments to `{name}`"));
            for arg in args {
                self.compile_expr(arg, depth + 1);
            }
            self.emit_const(0.0);
            return;
        }
        let val_arg = 1 - arr_arg;
        let Expr::Var { name: arr_name, .. } = &args[arr_arg] else {
            self.error(
                args[arr_arg].span(),
                format!("`{name}` expects an array name as argument {}", arr_arg + 1),
            );
            self.compile_expr(&args[val_arg], depth + 1);
            self.emit_const(0.0);
            return;
        };
        let Some((base, len)) = self.find_array(arr_name) else {
            self.error(
                args[arr_arg].span(),
                format!("`{arr_name}` is not an array"),
            );
            self.compile_expr(&args[val_arg], depth + 1);
            self.emit_const(0.0);
            return;
        };
        self.compile_expr(&args[val_arg], depth + 1);
        self.code.push(make_op(base, len));
    }

    fn compile_stateful(&mut self, kind: Stateful, args: &[Expr], span: Span, depth: usize) {
        let base = self.alloc_state(kind.state_cells(args.len()), span);
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
                if args.len() == 2 {
                    self.compile_expr(&args[1], d); // reset
                    self.code.push(Op::AccumReset(base));
                } else {
                    self.code.push(Op::Accum(base));
                }
            }
            Stateful::Delta => {
                self.compile_expr(&args[0], d);
                self.code.push(Op::Delta(base));
            }
            Stateful::Phasor => {
                self.compile_expr(&args[0], d); // rate
                if args.len() == 2 {
                    self.compile_expr(&args[1], d); // sync
                    self.code.push(Op::PhasorSync(base));
                } else {
                    self.code.push(Op::Phasor(base));
                }
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
            Stateful::RandSmooth => {
                self.compile_expr(&args[0], d); // rate
                self.code.push(Op::RandSmooth(base));
            }
            Stateful::Pulse => {
                // pulse(div) = edge((floor(beat) % div) == 0): a single-block
                // trigger at the start of every `div`-th beat. The level-based
                // edge trick only works for an integer divisor ≥ 2 — `div == 1`
                // makes `% 1` permanently 0 (one fire, never re-armed), and
                // `0`/fractional divisors degenerate the same way. Reject those
                // when the divisor is a constant rather than fail silently.
                if let Some(v) = const_eval(&args[0])
                    && !(v >= 2.0 && (v - v.floor()).abs() < f32::EPSILON)
                {
                    self.error(
                        span,
                        "pulse(div) needs an integer divisor ≥ 2 (e.g. pulse(4))",
                    );
                }
                self.push_source(SourceInput::Context(Context::Beat));
                self.code.push(Op::Call(Builtin::Floor));
                self.compile_expr(&args[0], d); // div
                self.code.push(Op::Rem);
                self.emit_const(0.0);
                self.code.push(Op::Eq);
                self.code.push(Op::Edge(base));
            }
        }
    }

    /// Emit the `lag` smoothing coefficient onto the stack. If the time argument
    /// is a constant, alpha is precomputed here (coefficient caching); otherwise
    /// `alpha = 1 - exp(-1 / (cr * t))` is emitted as runtime bytecode.
    fn emit_lag_alpha(&mut self, time: &Expr, depth: usize) {
        if let Some(t) = const_eval(time) {
            self.emit_const(self.lag_alpha(t));
            return;
        }
        // Runtime: 1 - exp(-1 / (t * cr)).  Stack already has x below.
        self.emit_const(1.0);
        self.compile_expr(time, depth + 1);
        self.push_source(SourceInput::Context(Context::Cr));
        self.code.push(Op::Mul); // t * cr
        self.code.push(Op::Div); // 1 / (t * cr)
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
        // Indexing needs the compiler's array registry (the constant pool), which
        // a free function cannot see — it is lowered to `IndexConst`, not folded.
        Expr::Index { .. } | Expr::Error { .. } => None,
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

    const CR: f32 = 750.0;
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
        prog.script.eval(&sources, &mut regs, &EvalContext::new(CR))
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
    fn into_bound_maps_each_input_kind() {
        // One module source, one macro, one context var — register order is
        // preserved one-to-one into the runtime input list.
        let src = "src lfo = lfo-1.out\nout = lfo * velocity + gate";
        let prog = compile_ok(src);
        let original = prog.inputs.clone();
        let bound = prog.into_bound(src.to_string());

        assert_eq!(bound.source, src);
        assert_eq!(bound.inputs.len(), original.len());
        for (orig, rt) in original.iter().zip(&bound.inputs) {
            match (orig, rt) {
                (
                    SourceInput::Macro(Macro::Velocity),
                    ScriptInput::Source(SrcAddr::Macro(MacroSource::Velocity)),
                )
                | (
                    SourceInput::Context(Context::Gate),
                    ScriptInput::Context(ScriptContext::Gate),
                ) => {}
                (
                    SourceInput::Module { module, member, .. },
                    ScriptInput::Source(addr @ SrcAddr::Module { .. }),
                ) if module == "lfo" && member == "out" => {
                    assert_eq!(addr.to_address_string(), "lfo-1.out");
                }
                other => panic!("unexpected mapping: {other:?}"),
            }
        }
    }

    #[test]
    fn into_bound_macro_names_map_to_runtime() {
        // `note`/`poly_at` are the two non-identity macro renames.
        let bound = compile_ok("out = note + poly_at").into_bound(String::new());
        assert!(bound.inputs.contains(&ScriptInput::Source(SrcAddr::Macro(
            MacroSource::NoteNumber
        ))));
        assert!(bound.inputs.contains(&ScriptInput::Source(SrcAddr::Macro(
            MacroSource::PolyAftertouch
        ))));
    }

    #[test]
    fn transport_context_vars_compile_and_eval() {
        // The Phase-1 transport sources are context vars: they compile to
        // `Context` inputs and the VM reads them straight from the source slice.
        let prog = compile_ok("out = sin(beat * tau) * playing");
        assert!(prog.inputs.contains(&SourceInput::Context(Context::Beat)));
        assert!(
            prog.inputs
                .contains(&SourceInput::Context(Context::Playing))
        );

        // beat = 0.25 → sin(tau/4) = 1; playing = 1 → out = 1.
        let out = eval(&prog, |inp| match inp {
            SourceInput::Context(Context::Beat) => 0.25,
            SourceInput::Context(Context::Playing) => 1.0,
            _ => 0.0,
        });
        assert!(approx(out, 1.0));
        // playing = 0 mutes regardless of beat.
        let muted = eval(&prog, |inp| match inp {
            SourceInput::Context(Context::Beat) => 0.25,
            _ => 0.0,
        });
        assert!(approx(muted, 0.0));
    }

    #[test]
    fn into_bound_maps_transport_context_vars() {
        let bound = compile_ok("out = bar_phase + tempo + playing").into_bound(String::new());
        assert!(
            bound
                .inputs
                .contains(&ScriptInput::Context(ScriptContext::BarPhase))
        );
        assert!(
            bound
                .inputs
                .contains(&ScriptInput::Context(ScriptContext::Tempo))
        );
        assert!(
            bound
                .inputs
                .contains(&ScriptInput::Context(ScriptContext::Playing))
        );
    }

    #[test]
    fn transport_context_names_are_reserved() {
        // Predefined context vars cannot be shadowed by a `src` binding.
        for name in ["beat", "bar_phase", "tempo", "playing"] {
            let errs = errors(&format!("src {name} = lfo-1.out\nout = {name}"));
            assert!(
                !errs.is_empty(),
                "binding reserved context var `{name}` must be a compile error"
            );
        }
    }

    #[test]
    fn into_bound_unknown_module_prefix_is_zero() {
        // An unknown module prefix is not a compile error (decision #3); it binds
        // to a zero register so the routing is kept and inert, not dropped.
        let bound = compile_ok("src x = zzz-1.out\nout = x").into_bound(String::new());
        assert_eq!(bound.inputs, vec![ScriptInput::Zero]);
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
        let alpha = 1.0 - (-1.0f32 / (CR * 0.05)).exp();
        assert!(approx(eval(&prog, |_| 1.0), alpha));
    }

    #[test]
    fn lag_with_dynamic_time_computes_alpha_at_runtime() {
        // A non-constant time emits the `1 - exp(-1/(t*cr))` bytecode; with the
        // same 0.05 s it must match the constant-folded path (guards the Div
        // operand order in the runtime-alpha lowering).
        let prog = compile_ok("out = lag(velocity, mod_wheel)");
        let alpha = 1.0 - (-1.0f32 / (CR * 0.05)).exp();
        let out = eval(&prog, |inp| match inp {
            SourceInput::Macro(Macro::Velocity) => 1.0,
            SourceInput::Macro(Macro::ModWheel) => 0.05,
            SourceInput::Context(Context::Cr) => CR,
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
    fn control_rate_var_is_cr_not_sr() {
        // The control-rate context var was renamed `sr` → `cr`; the old name is
        // gone (it conventionally reads as "sample rate", an order-of-magnitude trap).
        compile_ok("out = cr");
        assert!(
            errors("out = sr")
                .iter()
                .any(|e| e.contains("unknown identifier")),
            "`sr` must no longer resolve"
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

    fn eval_blocks(prog: &CompiledProgram, fill: impl Fn(&SourceInput) -> f32, n: usize) -> f32 {
        let sources: Vec<f32> = prog.inputs.iter().map(&fill).collect();
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        let mut last = 0.0;
        for _ in 0..n {
            last = prog.script.eval(&sources, &mut regs, &ctx);
        }
        last
    }

    #[test]
    fn synced_phasor_allocates_two_cells_and_resets() {
        // Non-synced phasor stays at one state cell; the synced overload bumps to
        // two (phase + prev-sync) so it never overwrites a neighbour.
        let plain = compile_ok("out = phasor(1)");
        assert_eq!(plain.script.state_count(), 1);
        let synced = compile_ok("out = phasor(1, gate_on)");
        assert_eq!(synced.script.state_count(), 2);

        // gate_on high on the first block resets to 0; otherwise it advances.
        let prog = compile_ok("out = phasor(187.5, gate_on)"); // 187.5/750 = 0.25/block
        let fill = |hi: bool| {
            move |inp: &SourceInput| match inp {
                SourceInput::Context(Context::GateOn) if hi => 1.0,
                _ => 0.0,
            }
        };
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        let src_lo: Vec<f32> = prog.inputs.iter().map(fill(false)).collect();
        let src_hi: Vec<f32> = prog.inputs.iter().map(fill(true)).collect();
        assert!(approx(prog.script.eval(&src_lo, &mut regs, &ctx), 0.25));
        assert!(approx(prog.script.eval(&src_lo, &mut regs, &ctx), 0.5));
        assert!(approx(prog.script.eval(&src_hi, &mut regs, &ctx), 0.0)); // edge reset
    }

    #[test]
    fn synced_accum_allocates_two_cells() {
        assert_eq!(compile_ok("out = accum(1)").script.state_count(), 1);
        assert_eq!(
            compile_ok("out = accum(1, gate_on)").script.state_count(),
            2
        );
    }

    #[test]
    fn rand_smooth_allocates_three_cells_and_wanders() {
        let prog = compile_ok("out = rand_smooth(2)");
        assert_eq!(prog.script.state_count(), 3);
        // Stays in [0,1) and is not flat-zero on cold start.
        let v = eval_blocks(&prog, |_| 0.0, 4);
        assert!((0.0..1.0).contains(&v), "out of range: {v}");
    }

    #[test]
    fn synced_state_counts_against_the_cap() {
        // 8 synced phasors = 16 cells (exactly the cap); a 9th overflows.
        let body = (0..8)
            .map(|_| "phasor(1, gate_on)")
            .collect::<Vec<_>>()
            .join(" + ");
        assert!(
            compile(&format!("out = {body}"), &CompileOptions::default())
                .0
                .is_some()
        );
        let over = (0..9)
            .map(|_| "phasor(1, gate_on)")
            .collect::<Vec<_>>()
            .join(" + ");
        assert!(
            errors(&format!("out = {over}"))
                .iter()
                .any(|e| e.contains("too much state"))
        );
    }

    #[test]
    fn unipolar_bipolar_fold_and_eval() {
        // Both are stateless builtins, so a constant argument folds.
        assert!(approx(
            eval(&compile_ok("out = unipolar(-1)"), |_| 0.0),
            0.0
        ));
        assert!(approx(eval(&compile_ok("out = unipolar(1)"), |_| 0.0), 1.0));
        assert!(approx(eval(&compile_ok("out = bipolar(0)"), |_| 0.0), -1.0));
        assert!(approx(eval(&compile_ok("out = bipolar(1)"), |_| 0.0), 1.0));
        // unipolar(bipolar(x)) is the identity.
        let prog = compile_ok("out = unipolar(bipolar(velocity))");
        assert!(approx(eval(&prog, |_| 0.7), 0.7));
    }

    #[test]
    fn pulse_fires_on_period_beat_edges() {
        // pulse(2) fires once at the start of beats 0, 2, 4, ...
        let prog = compile_ok("out = pulse(2)");
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        let fire = |beat: f32, regs: &mut RegisterFile| {
            let sources: Vec<f32> = prog
                .inputs
                .iter()
                .map(|inp| match inp {
                    SourceInput::Context(Context::Beat) => beat,
                    _ => 0.0,
                })
                .collect();
            prog.script.eval(&sources, regs, &ctx)
        };
        // Sub-beat blocks within beat 0: rising edge on the first, then held.
        assert!(approx(fire(0.0, &mut regs), 1.0)); // entering beat 0 (0 % 2 == 0)
        assert!(approx(fire(0.5, &mut regs), 0.0)); // still beat 0, no new edge
        assert!(approx(fire(1.5, &mut regs), 0.0)); // beat 1: 1 % 2 != 0
        assert!(approx(fire(2.2, &mut regs), 1.0)); // beat 2: 2 % 2 == 0 → fires
        assert!(approx(fire(3.0, &mut regs), 0.0)); // beat 3: no
    }

    #[test]
    fn pulse_rejects_degenerate_constant_divisors() {
        // div=1 (%1 always 0), 0, and fractional all silently fire once with the
        // edge trick — reject them at compile time when the divisor is constant.
        for bad in ["pulse(1)", "pulse(0)", "pulse(0.5)", "pulse(-2)"] {
            assert!(
                errors(&format!("out = {bad}"))
                    .iter()
                    .any(|e| e.contains("integer divisor")),
                "`{bad}` should be rejected"
            );
        }
        // A valid constant and a non-constant divisor both compile.
        assert!(compile_ok("out = pulse(4)").script.state_count() == 1);
        compile_ok("out = pulse(2 + 2)");
    }

    #[test]
    fn table_lin_interpolates_an_array() {
        let prog = compile_ok("arr s = [0, 10, 20]\nout = table_lin(s, beat)");
        let at = |beat: f32| {
            eval(&prog, |inp| match inp {
                SourceInput::Context(Context::Beat) => beat,
                _ => 0.0,
            })
        };
        assert!(approx(at(0.5), 5.0));
        assert!(approx(at(1.5), 15.0));
    }

    #[test]
    fn scale_snap_snaps_to_an_array_scale() {
        let prog = compile_ok("arr maj = [0, 2, 4, 5, 7, 9, 11]\nout = scale_snap(beat, maj)");
        let snap = |p: f32| {
            eval(&prog, |inp| match inp {
                SourceInput::Context(Context::Beat) => p,
                _ => 0.0,
            })
        };
        assert!(approx(snap(3.4), 4.0));
        assert!(approx(snap(11.8), 12.0)); // octave-aware
    }

    #[test]
    fn array_fn_on_non_array_is_an_error() {
        assert!(
            errors("let x = 1\nout = table_lin(x, beat)")
                .iter()
                .any(|e| e.contains("not an array"))
        );
        assert!(
            errors("arr s = [1]\nout = scale_snap(beat, 5)")
                .iter()
                .any(|e| e.contains("expects an array name"))
        );
    }

    #[test]
    fn array_fn_arity_is_checked() {
        assert!(
            errors("arr s = [1, 2]\nout = table_lin(s)")
                .iter()
                .any(|e| e.contains("arguments"))
        );
    }

    #[test]
    fn array_builtins_are_reserved_names() {
        for name in ["table_lin", "scale_snap", "unipolar", "bipolar", "pulse"] {
            assert!(
                errors(&format!("let {name} = 1\nout = 0"))
                    .iter()
                    .any(|e| e.contains("shadow")),
                "`{name}` must be reserved"
            );
        }
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
    fn array_step_sequencer_indexes_and_clamps() {
        // The headline example: a per-parameter step sequencer over `beat`.
        let prog = compile_ok("arr seq = [0, 0.5, 1, 0.25]\nout = seq[floor(beat) % 4]");
        let at = |beat: f32| {
            eval(&prog, |inp| match inp {
                SourceInput::Context(Context::Beat) => beat,
                _ => 0.0,
            })
        };
        assert!(approx(at(0.0), 0.0));
        assert!(approx(at(1.0), 0.5));
        assert!(approx(at(2.5), 1.0)); // floor(2.5) = 2 → seq[2]
        assert!(approx(at(3.0), 0.25));
        assert!(approx(at(4.0), 0.0)); // wraps via % 4
    }

    #[test]
    fn array_out_of_bounds_clamps_without_wrap() {
        // Without an explicit `% len`, an OOB index clamps to the edges.
        let prog = compile_ok("arr s = [10, 20, 30]\nout = s[beat]");
        let at = |beat: f32| {
            eval(&prog, |inp| match inp {
                SourceInput::Context(Context::Beat) => beat,
                _ => 0.0,
            })
        };
        assert!(approx(at(-5.0), 10.0));
        assert!(approx(at(99.0), 30.0));
    }

    #[test]
    fn array_constant_oob_index_warns_but_compiles() {
        // A provably-constant OOB index is a warning, not an error — the runtime
        // clamps safely, so the program still compiles.
        let (prog, diags) = compile("arr t = [1, 2, 3]\nout = t[5]", &CompileOptions::default());
        assert!(
            prog.is_some(),
            "OOB constant index must not be a hard error"
        );
        let warns: Vec<_> = diags
            .iter()
            .filter(|d| !d.is_error() && d.message.contains("out of bounds"))
            .collect();
        assert_eq!(warns.len(), 1, "expected one OOB warning, got {diags:?}");

        // A folded arithmetic index that lands OOB also warns (3 + 4 = 7).
        let (_p, folded) = compile(
            "arr t = [1, 2, 3]\nout = t[3 + 4]",
            &CompileOptions::default(),
        );
        assert!(
            folded
                .iter()
                .any(|d| !d.is_error() && d.message.contains("out of bounds"))
        );

        // A negative constant index warns too.
        let (_p, neg) = compile(
            "arr t = [1, 2, 3]\nout = t[0 - 1]",
            &CompileOptions::default(),
        );
        assert!(neg.iter().any(|d| d.message.contains("out of bounds")));

        // In-bounds constant and dynamic indices must NOT warn.
        for src in [
            "arr t = [1, 2, 3]\nout = t[2]",
            "arr t = [1, 2, 3]\nout = t[beat]",
        ] {
            let (_p, d) = compile(src, &CompileOptions::default());
            assert!(
                !d.iter().any(|x| x.message.contains("out of bounds")),
                "should not warn for `{src}`: {d:?}"
            );
        }
    }

    #[test]
    fn array_with_negative_and_folded_elements() {
        // Elements may be any constant expression (negatives, `pi`, arithmetic).
        let prog = compile_ok("arr s = [-0.3, 1 + 1, pi]\nout = s[1]");
        assert!(approx(eval(&prog, |_| 0.0), 2.0));
    }

    #[test]
    fn len_folds_to_element_count() {
        let prog = compile_ok("arr s = [1, 2, 3, 4, 5]\nout = len(s)");
        assert!(approx(eval(&prog, |_| 0.0), 5.0));
    }

    #[test]
    fn array_elements_are_baked_contiguously() {
        // Two tables plus a stray literal must each occupy a contiguous slice;
        // indexing the second table reads its own elements, not the first's.
        let prog = compile_ok("arr a = [1, 2]\narr b = [7, 8, 9]\nout = b[2] + a[0]");
        assert!(approx(eval(&prog, |_| 0.0), 10.0));
    }

    #[test]
    fn empty_array_is_an_error() {
        assert!(
            errors("arr e = []\nout = 0")
                .iter()
                .any(|e| e.contains("at least one element"))
        );
    }

    #[test]
    fn indexing_a_scalar_is_an_error() {
        assert!(
            errors("let x = 1\nout = x[0]")
                .iter()
                .any(|e| e.contains("not an array"))
        );
    }

    #[test]
    fn array_in_arithmetic_is_an_error() {
        assert!(
            errors("arr s = [1, 2]\nout = s + 1")
                .iter()
                .any(|e| e.contains("cannot be used directly"))
        );
    }

    #[test]
    fn len_of_non_array_is_an_error() {
        assert!(
            errors("let x = 1\nout = len(x)")
                .iter()
                .any(|e| e.contains("expected an array name"))
        );
    }

    #[test]
    fn array_name_collisions_are_errors() {
        assert!(
            errors("arr s = [1]\nlet s = 2\nout = s[0]")
                .iter()
                .any(|e| e.contains("duplicate"))
        );
        assert!(
            errors("arr sin = [1]\nout = sin[0]")
                .iter()
                .any(|e| e.contains("shadow"))
        );
    }

    #[test]
    fn array_storage_cap_is_enforced() {
        // One array of 257 elements > MAX_ARRAY_STORAGE (256).
        let body = (0..257).map(|_| "0").collect::<Vec<_>>().join(", ");
        let src = format!("arr s = [{body}]\nout = s[0]");
        assert!(errors(&src).iter().any(|e| e.contains("array storage")));
    }

    #[test]
    fn too_many_arrays_is_an_error() {
        // 17 single-element arrays > MAX_ARRAYS (16).
        let decls = (0..17)
            .map(|i| format!("arr a{i} = [0]"))
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!("{decls}\nout = a0[0]");
        assert!(errors(&src).iter().any(|e| e.contains("too many arrays")));
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
