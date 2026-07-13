//! The YAMS compiler: AST → [`CompiledScript`] (+ the source-input list the
//! engine must fill each block).
//!
//! Responsibilities: name resolution in one namespace, reserved-built-in
//! shadowing checks (decision #5), arity checks, source-slot allocation,
//! duration folding, constant folding (shared with the VM via `Builtin::eval`),
//! `lag` coefficient caching (decision: precompute alpha for a constant time),
//! and the hard caps (decision #8). All errors are collected; any error → no
//! program.

use crate::ast::{
    ArrayDecl, Assign, BinaryOp, Binding, BodyStmt, Expr, Local, OutChannel, Output, ParamDecl,
    Program, StateDecl, UnaryOp,
};
use crate::diag::Diagnostic;
use crate::lexer::DurationUnit;
use crate::parser::parse;
use crate::span::Span;
use crate::symbols::{
    self, Context, FnKind, Macro, Stateful, audio_in_channel, constant_value, context_from_name,
    macro_from_name, note_field, note_input, resolve_fn,
};
use synth_core::script::{
    AudioInputChannel, BoundScript, Builtin, CompiledScript, MAX_ARRAY_STORAGE, MAX_ARRAYS,
    MAX_INSTRUCTIONS, MAX_LOCALS, MAX_NESTING_DEPTH, MAX_SOURCE_LEN, MAX_SOURCES, MAX_STATE,
    NoteField, Op, SCRIPT_MAX_PARAMS, ScriptContext, ScriptInput, ScriptParamDecl, safe_div,
};
use synth_core::{MacroSource, PortName, SrcAddr};

/// One value the engine fills into a source register before evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceInput {
    Macro(Macro),
    Context(Context),
    /// A per-sample audio input (`in` / `in_l` / `in_r`) — audio-rate scripts
    /// only; the engine overwrites this register each sample.
    AudioIn(AudioInputChannel),
    /// An incoming-note field (`note_pitch` / `note_vel` / `note_dur` / `tick`) —
    /// `note_event` scripts only; the consumer fills it from the note event.
    NoteField(NoteField),
    /// A note-event `Value` modulation input `in1..in4` — `note_event` scripts
    /// only, indexed `0..3`.
    NoteInput(u8),
    /// A control-ports CV input `in1..in4` — the control-ports `Script` module
    /// only, indexed `0..3`. The voice fills it from the wired incoming graph
    /// connection to the matching `in{N}` port.
    ControlIn(u8),
    /// A user-declared `param` knob, read in-script by its interned name. The
    /// voice fills it from the host module's stored knob value each block.
    LocalParam(PortName),
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
    /// User-declared `param` knobs (empty for a program with none). Carried onto
    /// the [`BoundScript`] so the host module builds its descriptor + knob store.
    pub params: Vec<ScriptParamDecl>,
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
        BoundScript::new(self.script, inputs, source).with_params(self.params)
    }
}

/// Map one compiler source input to the runtime [`ScriptInput`] the voice fills.
fn input_to_runtime(input: &SourceInput) -> ScriptInput {
    match input {
        SourceInput::Macro(m) => ScriptInput::Source(SrcAddr::Macro(macro_to_runtime(*m))),
        SourceInput::Context(c) => ScriptInput::Context(context_to_runtime(*c)),
        SourceInput::AudioIn(ch) => ScriptInput::AudioIn(*ch),
        SourceInput::NoteField(field) => ScriptInput::NoteField(*field),
        SourceInput::NoteInput(idx) => ScriptInput::NoteInput(*idx),
        SourceInput::ControlIn(idx) => ScriptInput::ControlIn(*idx),
        SourceInput::LocalParam(name) => ScriptInput::LocalParam(*name),
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
        Context::Sr => ScriptContext::Sr,
        Context::NoteHz => ScriptContext::NoteHz,
        Context::Beat => ScriptContext::Beat,
        Context::BarPhase => ScriptContext::BarPhase,
        Context::Tempo => ScriptContext::Tempo,
        Context::Playing => ScriptContext::Playing,
        Context::FirstSample => ScriptContext::FirstSample,
    }
}

/// Compile-time options supplied by the engine.
#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    /// Control rate in Hz — needed to precompute `lag` coefficients (a `lag`
    /// with a constant time folds its alpha here; recompile on a rate change).
    /// At audio rate this is the sample rate (used the same way for `lag`).
    pub control_rate: f32,
    /// Compile for the audio-rate [`AudioScript`](synth_core) module. Enables the
    /// audio-only grammar — audio-in sources (`in`/`in_l`/`in_r`), the
    /// `first_sample` one-shot, and `out.left`/`out.right` multi-out — which are
    /// compile errors in a control-rate (Mod Matrix / Script module) script.
    pub audio_rate: bool,
    /// Compile for the `note_event` (NoteScriptTransform) dialect. Enables the
    /// note-event grammar — the note-field reads (`note_pitch`/`note_vel`/
    /// `note_dur`/`tick`) and `Value` inputs (`in1..in4`), and the `out.pitch`/
    /// `out.vel`/`out.dur`/`out.gate` field writes — which are compile errors in
    /// any other dialect. Mutually exclusive with [`Self::audio_rate`] in
    /// practice (a script targets exactly one module kind).
    pub note_event: bool,
    /// Compile for the control-ports `Script` module. Enables the numbered CV
    /// port grammar — `in1..in4` reads (as [`SourceInput::ControlIn`], distinct
    /// from the `note_event` `in1..in4`) and `out1..out4` writes — which are
    /// compile errors in any other dialect. A bare `out` still means `out1`.
    /// Mutually exclusive with [`Self::audio_rate`]/[`Self::note_event`] in
    /// practice; the Mod Matrix's control `scr` scripts leave this `false` and
    /// keep their single-`out`, no-ports contract.
    pub control_ports: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        // 48 kHz / 64-sample blocks, control-rate (Mod Matrix `scr` script).
        Self {
            control_rate: 750.0,
            audio_rate: false,
            note_event: false,
            control_ports: false,
        }
    }
}

/// Compile YAMS source into a [`CompiledProgram`], or `None` plus diagnostics.
#[must_use]
pub fn compile(src: &str, opts: &CompileOptions) -> (Option<CompiledProgram>, Vec<Diagnostic>) {
    let (program, diags) = parse(src);
    let mut compiler = Compiler {
        control_rate: opts.control_rate,
        audio_rate: opts.audio_rate,
        note_event: opts.note_event,
        control_ports: opts.control_ports,
        code: Vec::new(),
        constants: Vec::new(),
        inputs: Vec::new(),
        bindings: Vec::new(),
        arrays: Vec::new(),
        states: Vec::new(),
        params: Vec::new(),
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

/// A declared `state` cell: its name and the state-register index it owns.
struct StateEntry {
    name: String,
    cell: u16,
}

struct Compiler {
    control_rate: f32,
    audio_rate: bool,
    note_event: bool,
    control_ports: bool,
    code: Vec<Op>,
    constants: Vec<f32>,
    inputs: Vec<SourceInput>,
    bindings: Vec<BindingEntry>,
    arrays: Vec<ArrayEntry>,
    states: Vec<StateEntry>,
    /// Declared `param` knobs, in source order — the runtime decls carried onto
    /// the compiled program, and the resolution table for in-script param reads.
    params: Vec<ScriptParamDecl>,
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
        // Declared `state` cells are allocated before the body compiles, so a
        // forward read/write of `s` resolves to a stable cell index (and any
        // body `lag`/`phasor` cells are allocated after them).
        for s in &program.states {
            self.register_state(s);
        }
        // Declared `param` knobs are registered before the body so an in-script
        // read of a param name resolves to its source register.
        for p in &program.params {
            self.register_param(p);
        }
        for stmt in &program.body {
            match stmt {
                BodyStmt::Local(l) => self.compile_local(l),
                BodyStmt::Assign(a) => self.compile_assign(a),
            }
        }
        self.compile_outputs(&program.outputs);

        // Final caps (per-allocation caps were checked as they were hit).
        let fallback = program.outputs.first().map_or(Span::new(0, 0), |o| o.span);
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
            params: std::mem::take(&mut self.params),
        })
    }

    // ---- statements -------------------------------------------------------

    fn name_taken(&self, name: &str) -> bool {
        self.bindings.iter().any(|b| b.alias == name)
            || self.arrays.iter().any(|a| a.name == name)
            || self.states.iter().any(|s| s.name == name)
            || self.locals.iter().any(|l| l == name)
            || self.params.iter().any(|p| p.name_str == name)
    }

    /// Emit a diagnostic if `name` shadows a built-in or collides with an
    /// existing declaration. The single guard shared by every declaration site
    /// (`src` / `arr` / `state` / `let`), so the rules and wording stay in lockstep.
    fn check_unique_name(&mut self, name: &str, span: Span) {
        if symbols::is_reserved(name)
            || (self.note_event && symbols::is_note_event_reserved(name))
            || (self.control_ports && control_ports_reserves(name))
        {
            self.error(span, format!("cannot shadow built-in `{name}`"));
        } else if self.name_taken(name) {
            self.error(span, format!("duplicate name `{name}`"));
        }
    }

    /// Register a `state s = <const>` declaration: allocate one persistent cell
    /// and record `name → cell`. The initializer must fold to `0` (cells reset to
    /// 0 on note-on; a non-zero seed comes from `first_sample` at audio rate).
    fn register_state(&mut self, s: &StateDecl) {
        let name = &s.name.name;
        self.check_unique_name(name, s.name.span);
        match const_eval(&s.init) {
            Some(0.0) => {}
            Some(_) => self.error(
                s.init.span(),
                "state cells initialize to 0 (note-on resets all state); seed a non-zero value from `first_sample` in an audio-rate script",
            ),
            None => self.error(s.init.span(), "state initializer must be a constant"),
        }
        let cell = self.alloc_state(1, s.span);
        self.states.push(StateEntry {
            name: name.clone(),
            cell,
        });
    }

    /// Register a `param <name> = <default> [ [min, max] ] [ "label" ] [ "tooltip" ]`
    /// knob. Script / AudioScript modules only — a control-rate Mod Matrix `scr`
    /// script has no knobs. The default and optional range bounds must const-fold;
    /// the name is interned to a [`PortName`] (off the audio thread) and its
    /// `'static` string cached so audio-thread mod-offset matching never locks.
    fn register_param(&mut self, p: &ParamDecl) {
        let name = &p.name.name;
        // Dialect gate: knobs belong to the two script *modules*, not the Mod Matrix.
        if !self.control_ports && !self.audio_rate {
            self.error(
                p.name.span,
                "`param` knobs are only available in a Script or AudioScript program",
            );
            return;
        }
        self.check_unique_name(name, p.name.span);
        if self.params.len() >= SCRIPT_MAX_PARAMS {
            self.error(p.span, format!("too many params (max {SCRIPT_MAX_PARAMS})"));
            return;
        }
        // The default (and optional range bounds) must fold to constants.
        let Some(default) = const_eval(&p.default) else {
            self.error(p.default.span(), "param default must be a constant");
            return;
        };
        let (min, max) = match &p.range {
            Some((lo, hi)) => match (const_eval(lo), const_eval(hi)) {
                (Some(lo), Some(hi)) => (lo, hi),
                _ => {
                    self.error(p.span, "param range bounds must be constants");
                    return;
                }
            },
            None => (0.0, 1.0),
        };
        let interned = PortName::intern(name);
        self.params.push(ScriptParamDecl {
            name: interned,
            name_str: interned.as_str(),
            default,
            min,
            max,
            label: p.label.clone(),
            tooltip: p.tooltip.clone(),
        });
    }

    fn register_binding(&mut self, b: &Binding) {
        let name = &b.alias.name;
        self.check_unique_name(name, b.alias.span);
        if self.note_event {
            // A note-event script transforms one note event; it has no module
            // graph to sample, so a `src x = module.member` binding would read 0
            // at runtime. Reject it at the source rather than silently zero it.
            self.error(
                b.alias.span,
                "module references are not available in a note-event script",
            );
            return;
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
        self.check_unique_name(name, a.name.span);
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
        self.check_unique_name(name, l.name.span);
        self.compile_expr(&l.expr, 0);
        let slot = self.locals.len() as u16;
        self.code.push(Op::StoreLocal(slot));
        self.locals.push(name.clone());
        if self.locals.len() > MAX_LOCALS {
            self.error(l.span, format!("too many locals (max {MAX_LOCALS})"));
        }
    }

    /// Compile a `state`-cell assignment `s = expr` → evaluate the expression and
    /// pop it into the cell (`Op::StoreState`). Stack-neutral, like `let`. Only a
    /// declared `state` name is assignable.
    fn compile_assign(&mut self, a: &Assign) {
        let name = &a.name.name;
        let Some(cell) = self.states.iter().find(|s| s.name == *name).map(|s| s.cell) else {
            if symbols::is_reserved(name)
                || (self.note_event && symbols::is_note_event_reserved(name))
                || (self.control_ports && control_ports_reserves(name))
            {
                self.error(a.name.span, format!("cannot assign to built-in `{name}`"));
            } else if self.name_taken(name) {
                self.error(
                    a.name.span,
                    format!("`{name}` is not a `state` cell; only `state` cells can be assigned"),
                );
            } else {
                self.error(
                    a.name.span,
                    format!(
                        "assignment to undeclared `{name}`; declare it with `state {name} = 0`"
                    ),
                );
            }
            // Still compile the RHS so nested errors in it are reported; the
            // emitted code is discarded (an error path never yields a program).
            self.compile_expr(&a.expr, 0);
            return;
        };
        self.compile_expr(&a.expr, 0);
        self.code.push(Op::StoreState(cell));
    }

    /// Compile the program's output statement(s), validating the channel mix.
    ///
    /// Control-rate: exactly one mono `out` (a channel output or a second `out`
    /// is an error). Audio-rate: either one mono `out` (the
    /// [`eval_block`](synth_core) fallback duplicates it to both channels) or up
    /// to two channel outputs `out.left`/`out.right` (no duplicate channel, not
    /// mixed with a mono `out`). `note_event`: one or more of `out.pitch`/
    /// `out.vel`/`out.dur`/`out.gate` (no mono, no stereo). A mono `out` leaves
    /// its value on the value stack; every other output emits `Op::StoreOut`
    /// into its slot (`0..3`).
    fn compile_outputs(&mut self, outputs: &[Output]) {
        self.validate_output_channels(outputs);
        for o in outputs {
            self.compile_expr(&o.expr, 0);
            match o.channel {
                // A bare `out` leaves its value on the stack (the VM reads the top
                // as the mono result and duplicates it to both channels).
                OutChannel::Mono => {}
                // Audio left / note pitch share output slot 0; right / vel slot 1.
                // A script is a single dialect, so the slot is unambiguous.
                OutChannel::Left | OutChannel::Pitch => self.code.push(Op::StoreOut(0)),
                OutChannel::Right | OutChannel::Vel => self.code.push(Op::StoreOut(1)),
                OutChannel::Dur => self.code.push(Op::StoreOut(2)),
                OutChannel::Gate => self.code.push(Op::StoreOut(3)),
                // Numbered CV output `out1..out4` → slot `0..3`.
                OutChannel::Out(slot) => self.code.push(Op::StoreOut(slot)),
            }
        }
    }

    /// Validate the set of output statements for the active dialect, emitting a
    /// diagnostic per violation (the emission itself stays simple).
    fn validate_output_channels(&mut self, outputs: &[Output]) {
        // Reject any output member that does not belong to the active dialect.
        for o in outputs {
            if !self.channel_allowed(o.channel) {
                self.error(o.span, channel_reject_msg(o.channel));
            }
        }
        // Audio-rate: a mono `out` and a stereo channel `out` cannot be mixed.
        if self.audio_rate
            && outputs.iter().any(|o| o.channel == OutChannel::Mono)
            && let Some(o) = outputs
                .iter()
                .find(|o| matches!(o.channel, OutChannel::Left | OutChannel::Right))
        {
            self.error(
                o.span,
                "use a single `out` OR `out.left`/`out.right`, not both",
            );
        }
        // Control-ports: a bare `out` is sugar for `out1`, so both target slot 0 —
        // writing both is a duplicate (the second would silently win).
        if self.control_ports
            && outputs.iter().any(|o| o.channel == OutChannel::Mono)
            && let Some(o) = outputs.iter().find(|o| o.channel == OutChannel::Out(0))
        {
            self.error(
                o.span,
                "use `out` OR `out1` (they are the same port), not both",
            );
        }
        // Each channel may appear at most once (a later one would silently win).
        for (channel, label) in [
            (OutChannel::Mono, "out"),
            (OutChannel::Left, "out.left"),
            (OutChannel::Right, "out.right"),
            (OutChannel::Pitch, "out.pitch"),
            (OutChannel::Vel, "out.vel"),
            (OutChannel::Dur, "out.dur"),
            (OutChannel::Gate, "out.gate"),
            (OutChannel::Out(0), "out1"),
            (OutChannel::Out(1), "out2"),
            (OutChannel::Out(2), "out3"),
            (OutChannel::Out(3), "out4"),
        ] {
            self.dup_channel_error(outputs, channel, label);
        }
    }

    /// Whether an output member is legal in the active dialect. A bare mono `out`
    /// is the control/audio result but is *not* a `note_event` output (that
    /// dialect must name a field); stereo channels are audio-only; note fields
    /// are `note_event`-only.
    fn channel_allowed(&self, channel: OutChannel) -> bool {
        match channel {
            OutChannel::Mono => !self.note_event,
            OutChannel::Left | OutChannel::Right => self.audio_rate,
            OutChannel::Pitch | OutChannel::Vel | OutChannel::Dur | OutChannel::Gate => {
                self.note_event
            }
            OutChannel::Out(_) => self.control_ports,
        }
    }

    /// Report the second (and later) occurrence of a duplicated output channel.
    fn dup_channel_error(&mut self, outputs: &[Output], channel: OutChannel, label: &str) {
        for o in outputs.iter().filter(|o| o.channel == channel).skip(1) {
            self.error(o.span, format!("duplicate `{label}`"));
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
        // A bare `state` name reads the cell's current value (`Op::LoadState`).
        if let Some(cell) = self.states.iter().find(|s| s.name == name).map(|s| s.cell) {
            self.code.push(Op::LoadState(cell));
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
        // A declared `param` knob reads its own source register (the voice fills
        // it from the host module's stored knob value each block).
        if let Some(decl) = self.params.iter().find(|p| p.name_str == name) {
            self.push_source(SourceInput::LocalParam(decl.name));
            return;
        }
        if let Some(m) = macro_from_name(name) {
            // Macros and context vars are engine-graph modulation sources with no
            // meaning inside a per-event note script — reject rather than resolve
            // to a runtime 0 (the `note_event` mirror of the audio-only gate).
            if self.note_event {
                self.reject_in_note_event(name, span);
            } else {
                self.push_source(SourceInput::Macro(m));
            }
            return;
        }
        if let Some(ctx) = context_from_name(name) {
            if self.note_event {
                self.reject_in_note_event(name, span);
            } else {
                self.push_source(SourceInput::Context(ctx));
            }
            return;
        }
        // Audio-only identifiers (`in`/`in_l`/`in_r`, `first_sample`) resolve only
        // in an audio-rate script; in a control-rate one they are reserved but
        // produce a clear error rather than a confusing "unknown identifier".
        if let Some(ch) = audio_in_channel(name) {
            self.resolve_audio_only(name, span, SourceInput::AudioIn(ch));
            return;
        }
        if name == "first_sample" {
            self.resolve_audio_only(name, span, SourceInput::Context(Context::FirstSample));
            return;
        }
        // Note-event identifiers (`note_pitch`/…, `in1..in4`) resolve only in a
        // `note_event` script; reserved-but-erroring elsewhere (like the audio ones).
        if let Some(field) = note_field(name) {
            self.resolve_note_only(name, span, SourceInput::NoteField(field));
            return;
        }
        if let Some(idx) = note_input(name) {
            // `in1..in4` names both a note-event `Value` input and a control-ports
            // CV input; the active dialect decides which (or neither).
            if self.control_ports {
                self.push_source(SourceInput::ControlIn(idx));
            } else if self.note_event {
                self.push_source(SourceInput::NoteInput(idx));
            } else {
                self.error(
                    span,
                    format!("`{name}` is only available in a note-event or control-ports script"),
                );
                self.emit_const(0.0);
            }
            return;
        }
        // A numbered CV input past the 4-port ceiling in a control-ports script
        // (`in5`, `in0`) gets a clear ceiling error instead of "unknown identifier".
        if self.control_ports && matches!(symbols::input_port_index(name), Some(Err(()))) {
            self.error(span, "only in1..in4 are available (max 4 CV inputs)");
            self.emit_const(0.0);
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

    /// Resolve an audio-only identifier, gating it behind `audio_rate`: push its
    /// source register in an audio-rate script, or emit a clear error (and a
    /// placeholder `0`) in a control-rate one. Shared by the audio-in and
    /// `first_sample` resolution paths.
    fn resolve_audio_only(&mut self, name: &str, span: Span, src: SourceInput) {
        if self.audio_rate {
            self.push_source(src);
        } else {
            self.error(
                span,
                format!("`{name}` is only available in an audio-rate script"),
            );
            self.emit_const(0.0);
        }
    }

    /// Resolve a note-event-only identifier, gating it behind `note_event`: push
    /// its source register in a `note_event` script, or emit a clear error (and a
    /// placeholder `0`) in any other dialect. Shared by the note-field and
    /// `in1..in4` resolution paths (the `note_event` mirror of
    /// [`resolve_audio_only`](Self::resolve_audio_only)).
    fn resolve_note_only(&mut self, name: &str, span: Span, src: SourceInput) {
        if self.note_event {
            self.push_source(src);
        } else {
            self.error(
                span,
                format!("`{name}` is only available in a note-event script"),
            );
            self.emit_const(0.0);
        }
    }

    /// Reject an engine-graph modulation source (macro / context var) used inside
    /// a `note_event` script, where it has no meaning and would otherwise resolve
    /// to a runtime `0` with a green "compiled" status. Emits a placeholder `0`.
    fn reject_in_note_event(&mut self, name: &str, span: Span) {
        self.error(
            span,
            format!("`{name}` is not available in a note-event script"),
        );
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

/// The diagnostic for an output member used in the wrong dialect. A bare mono
/// `out` only reaches here in `note_event` mode (it is allowed everywhere else).
fn channel_reject_msg(channel: OutChannel) -> String {
    match channel {
        OutChannel::Left | OutChannel::Right => {
            "`out.left`/`out.right` (stereo output) is only available in an audio-rate script"
                .to_string()
        }
        OutChannel::Pitch | OutChannel::Vel | OutChannel::Dur | OutChannel::Gate => {
            "`out.pitch`/`out.vel`/`out.dur`/`out.gate` is only available in a note-event script"
                .to_string()
        }
        OutChannel::Mono => {
            "a note-event script writes `out.pitch`/`out.vel`/`out.dur`/`out.gate`, not a bare `out`"
                .to_string()
        }
        OutChannel::Out(_) => {
            "`out1`..`out4` (numbered CV output) is only available in a control-ports (Script module) script"
                .to_string()
        }
    }
}

/// Whether `name` is a control-ports reserved port token (`in1..in4` /
/// `out1..out4`). Those names belong to the `Script` module's CV ports, so a
/// control-ports script cannot bind (`src`/`let`/`state`/`arr`) or assign over
/// them. The caller ANDs this with its `control_ports` flag.
fn control_ports_reserves(name: &str) -> bool {
    symbols::note_input(name).is_some() || matches!(symbols::output_port_index(name), Some(Ok(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::script::{EvalContext, NoteOutputs, RegisterFile};

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
            SourceInput::Context(_)
            | SourceInput::AudioIn(_)
            | SourceInput::NoteField(_)
            | SourceInput::NoteInput(_)
            | SourceInput::ControlIn(_)
            | SourceInput::LocalParam(_) => 0.0,
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
    fn cr_and_sr_are_distinct_context_vars() {
        // `cr` is the control rate (`sample_rate / block_size`); `sr` is the audio
        // sample rate. Both resolve, to *different* source registers, so a script
        // can read either without one aliasing the other.
        compile_ok("out = cr");
        let prog = compile_ok("out = sr");
        assert_eq!(
            prog.inputs,
            vec![SourceInput::Context(Context::Sr)],
            "`sr` resolves to the audio sample-rate context var"
        );
        // Reading `sr` must pick up the Sr fill, not the Cr fill.
        let out = eval(&prog, |inp| match inp {
            SourceInput::Context(Context::Sr) => 48_000.0,
            SourceInput::Context(Context::Cr) => 750.0,
            _ => 0.0,
        });
        assert!(approx(out, 48_000.0), "`sr` read the wrong register: {out}");
    }

    #[test]
    fn note_hz_resolves_to_the_voice_frequency_var() {
        // `note_hz` is the voice's playing frequency; it must resolve to its own
        // context register, distinct from the raw `note` macro.
        let prog = compile_ok("out = note_hz");
        assert_eq!(prog.inputs, vec![SourceInput::Context(Context::NoteHz)]);
        let out = eval(&prog, |inp| match inp {
            SourceInput::Context(Context::NoteHz) => 220.0,
            SourceInput::Macro(Macro::Note) => 57.0,
            _ => 0.0,
        });
        assert!(approx(out, 220.0), "`note_hz` read the wrong source: {out}");
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

    // ---- Phase 4 step 2a: `state` cells, assignment, `tanh` ---------------

    /// Evaluate a program over `n` blocks, filling sources via `fill` each block,
    /// against a single persistent `RegisterFile` — so state carries across blocks.
    fn eval_n(prog: &CompiledProgram, fill: impl Fn(&SourceInput) -> f32, n: usize) -> f32 {
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
    fn state_cell_is_a_manual_accumulator() {
        // `state s` declares one cell; `s = s + velocity` accumulates across blocks.
        let prog = compile_ok("state s = 0\ns = s + velocity\nout = s");
        assert_eq!(prog.script.state_count(), 1);
        // velocity = 0.5 each block → 0.5, 1.0, 1.5 after three blocks.
        assert!(approx(eval_n(&prog, |_| 0.5, 3), 1.5));
    }

    #[test]
    fn state_read_returns_prior_value_until_assignment() {
        // Reading `s` before its assignment in the same block sees last block's
        // value (the IIR ordering guarantee): let prev = s; s = velocity; out = prev.
        let prog = compile_ok("state s = 0\nlet prev = s\ns = velocity\nout = prev");
        // Block 1: prev = 0 (cold). Block 2: prev = velocity from block 1.
        let sources: Vec<f32> = prog.inputs.iter().map(|_| 0.7).collect();
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        assert!(approx(prog.script.eval(&sources, &mut regs, &ctx), 0.0));
        assert!(approx(prog.script.eval(&sources, &mut regs, &ctx), 0.7));
    }

    #[test]
    fn state_is_stack_neutral_and_chains() {
        // Two state cells updated in sequence; the program must stay balanced.
        let prog = compile_ok("state a = 0\nstate b = 0\na = a + 1\nb = b + a\nout = b");
        assert_eq!(prog.script.state_count(), 2);
        // a: 1,2,3 ; b: 1,3,6.
        assert!(approx(eval_n(&prog, |_| 0.0, 3), 6.0));
    }

    #[test]
    fn assigning_a_non_state_is_an_error() {
        assert!(
            errors("let x = 1\nx = 2\nout = x")
                .iter()
                .any(|e| e.contains("not a `state` cell"))
        );
        assert!(
            errors("s = 1\nout = 0")
                .iter()
                .any(|e| e.contains("undeclared"))
        );
        assert!(
            errors("velocity = 1\nout = 0")
                .iter()
                .any(|e| e.contains("built-in"))
        );
    }

    #[test]
    fn state_keyword_and_nonzero_init_are_guarded() {
        // `state` is a keyword, so it cannot be used as a `let` name at all (a
        // parse error — stronger than the reserved-identifier shadow check).
        assert!(!errors("let state = 1\nout = state").is_empty());
        assert!(
            errors("state s = 5\nout = s")
                .iter()
                .any(|e| e.contains("initialize to 0"))
        );
        assert!(
            errors("state s = velocity\nout = s")
                .iter()
                .any(|e| e.contains("must be a constant"))
        );
    }

    #[test]
    fn state_counts_against_the_cap() {
        // 16 state cells is exactly MAX_STATE; a 17th overflows.
        let ok = (0..16)
            .map(|i| format!("state s{i} = 0"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            compile(&format!("{ok}\nout = s0"), &CompileOptions::default())
                .0
                .is_some()
        );
        let over = (0..17)
            .map(|i| format!("state s{i} = 0"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            errors(&format!("{over}\nout = s0"))
                .iter()
                .any(|e| e.contains("too much state"))
        );
    }

    #[test]
    fn tanh_folds_and_evaluates() {
        // Stateless builtin: a constant argument folds at compile time.
        assert!(approx(eval(&compile_ok("out = tanh(0)"), |_| 0.0), 0.0));
        // Dynamic argument evaluates at runtime: tanh(velocity).
        let prog = compile_ok("out = tanh(velocity)");
        assert!(approx(eval(&prog, |_| 1.0), 1.0_f32.tanh()));
        // `tanh` is reserved (cannot be shadowed).
        assert!(
            errors("let tanh = 1\nout = 0")
                .iter()
                .any(|e| e.contains("shadow"))
        );
    }

    // ---- Phase 4 step 2b: audio-rate grammar ------------------------------

    fn audio_opts() -> CompileOptions {
        CompileOptions {
            audio_rate: true,
            ..CompileOptions::default()
        }
    }

    fn compile_audio_ok(src: &str) -> CompiledProgram {
        let (prog, diags) = compile(src, &audio_opts());
        let errs: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errs.is_empty(), "unexpected errors: {errs:?}");
        prog.expect("a compiled audio program")
    }

    fn audio_errors(src: &str) -> Vec<String> {
        compile(src, &audio_opts())
            .1
            .into_iter()
            .filter(Diagnostic::is_error)
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn audio_sources_are_gated_behind_audio_rate() {
        // In a control-rate script the audio identifiers are reserved-but-unusable.
        for name in ["in", "in_l", "in_r"] {
            assert!(
                errors(&format!("out = {name}"))
                    .iter()
                    .any(|e| e.contains("audio-rate")),
                "`{name}` should be control-rate-rejected"
            );
        }
        assert!(
            errors("out = first_sample")
                .iter()
                .any(|e| e.contains("audio-rate"))
        );
        // At audio rate they compile to the right source kinds.
        let prog = compile_audio_ok("out = in_l + in_r * first_sample");
        assert!(
            prog.inputs
                .contains(&SourceInput::AudioIn(AudioInputChannel::Left))
        );
        assert!(
            prog.inputs
                .contains(&SourceInput::AudioIn(AudioInputChannel::Right))
        );
        assert!(
            prog.inputs
                .contains(&SourceInput::Context(Context::FirstSample))
        );
        // `in` aliases the left channel.
        let mono = compile_audio_ok("out = in");
        assert!(
            mono.inputs
                .contains(&SourceInput::AudioIn(AudioInputChannel::Left))
        );
    }

    #[test]
    fn multi_out_is_gated_and_emits_store_audio_out() {
        // Control-rate rejects channel outputs.
        assert!(
            errors("out.left = velocity")
                .iter()
                .any(|e| e.contains("audio-rate"))
        );
        // Audio-rate emits StoreOut(0) and StoreOut(1).
        let prog = compile_audio_ok("out.left = in_l\nout.right = in_r");
        assert!(prog.script.code().contains(&Op::StoreOut(0)));
        assert!(prog.script.code().contains(&Op::StoreOut(1)));
        // Mixing mono and channel, or duplicating a channel, is an error.
        assert!(
            audio_errors("out = in\nout.left = in")
                .iter()
                .any(|e| e.contains("not both"))
        );
        assert!(
            audio_errors("out.left = in\nout.left = in")
                .iter()
                .any(|e| e.contains("duplicate"))
        );
    }

    #[test]
    fn audio_waveshaper_round_trips_through_eval_block() {
        use synth_core::script::AudioBindings;
        // Compile a real audio DSP program and run it through eval_block, building
        // the per-sample bindings from the compiled input list (as the module will).
        let prog = compile_audio_ok("out = tanh(in * 4)");
        let drive_reg = prog
            .inputs
            .iter()
            .position(|i| *i == SourceInput::AudioIn(AudioInputChannel::Left))
            .expect("an `in` source register") as u16;
        let bindings = AudioBindings {
            in_left: Some(drive_reg),
            ..Default::default()
        };

        let input: Vec<f32> = vec![0.0, 0.1, -0.2, 0.5];
        let mut sources = vec![0.0; prog.inputs.len()];
        let mut regs = RegisterFile::new(0, SEED);
        let mut l = vec![0.0; input.len()];
        let mut r = vec![0.0; input.len()];
        prog.script.eval_block(
            &mut sources,
            &bindings,
            &input,
            &input,
            &mut l,
            &mut r,
            &mut regs,
            &EvalContext::audio(48_000.0),
            true,
        );
        for i in 0..input.len() {
            let expected = (input[i] * 4.0).tanh();
            assert!(approx(l[i], expected), "sample {i}: {} != {expected}", l[i]);
            assert!(approx(r[i], expected), "mono duplicated to right");
        }
    }

    // ---- Phase 7 B1: `note_event` dialect ---------------------------------

    fn note_opts() -> CompileOptions {
        CompileOptions {
            note_event: true,
            ..CompileOptions::default()
        }
    }

    fn compile_note_ok(src: &str) -> CompiledProgram {
        let (prog, diags) = compile(src, &note_opts());
        let errs: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errs.is_empty(), "unexpected errors: {errs:?}");
        prog.expect("a compiled note-event program")
    }

    fn note_errors(src: &str) -> Vec<String> {
        compile(src, &note_opts())
            .1
            .into_iter()
            .filter(Diagnostic::is_error)
            .map(|d| d.message)
            .collect()
    }

    /// Compile+run a note-event program, filling each source via `fill`.
    fn eval_note(prog: &CompiledProgram, fill: impl Fn(&SourceInput) -> f32) -> NoteOutputs {
        let sources: Vec<f32> = prog.inputs.iter().map(fill).collect();
        let mut regs = RegisterFile::new(0, SEED);
        prog.script.eval_note(&sources, &mut regs)
    }

    #[test]
    fn note_event_transposes_pitch_and_passes_the_rest_through() {
        // (a) `out.pitch = note_pitch + 12` → pitch Some(+12), others None.
        let prog = compile_note_ok("out.pitch = note_pitch + 12");
        assert_eq!(prog.inputs, vec![SourceInput::NoteField(NoteField::Pitch)]);
        let outs = eval_note(&prog, |inp| match inp {
            SourceInput::NoteField(NoteField::Pitch) => 60.0,
            _ => 0.0,
        });
        assert_eq!(outs.pitch, Some(72.0));
        assert_eq!(outs.vel, None);
        assert_eq!(outs.dur, None);
        assert_eq!(outs.gate, None);
    }

    #[test]
    fn note_event_reads_note_vel_and_a_value_input() {
        // (b) `out.vel = note_vel * (0.5 + 0.5 * in1)` reads note_vel and in1.
        let prog = compile_note_ok("out.vel = note_vel * (0.5 + 0.5 * in1)");
        assert!(
            prog.inputs
                .contains(&SourceInput::NoteField(NoteField::Vel))
        );
        assert!(prog.inputs.contains(&SourceInput::NoteInput(0)));
        // note_vel = 0.8, in1 = 1.0 → 0.8 * (0.5 + 0.5) = 0.8.
        let outs = eval_note(&prog, |inp| match inp {
            SourceInput::NoteField(NoteField::Vel) => 0.8,
            SourceInput::NoteInput(0) => 1.0,
            _ => 0.0,
        });
        assert!(outs.vel.is_some_and(|v| approx(v, 0.8)));
        // in1 = -1 → 0.8 * (0.5 - 0.5) = 0.
        let muted = eval_note(&prog, |inp| match inp {
            SourceInput::NoteField(NoteField::Vel) => 0.8,
            SourceInput::NoteInput(0) => -1.0,
            _ => 0.0,
        });
        assert!(muted.vel.is_some_and(|v| approx(v, 0.0)));
        assert_eq!(muted.pitch, None);
    }

    #[test]
    fn note_event_in1_through_in4_map_to_indices_0_through_3() {
        let prog = compile_note_ok("out.pitch = in1 + in2 + in3 + in4");
        for idx in 0u8..4 {
            assert!(
                prog.inputs.contains(&SourceInput::NoteInput(idx)),
                "missing in{}",
                idx + 1
            );
        }
    }

    #[test]
    fn note_event_unwritten_fields_are_none() {
        // (c) pass-through: writing only `out.gate` leaves the rest None.
        let prog = compile_note_ok("out.gate = 1");
        let outs = eval_note(&prog, |_| 0.0);
        assert_eq!(outs.gate, Some(1.0));
        assert_eq!(outs.pitch, None);
        assert_eq!(outs.vel, None);
        assert_eq!(outs.dur, None);
    }

    #[test]
    fn note_event_identifiers_error_without_the_dialect() {
        // (d) note_event reads/writes are errors when note_event = false.
        for name in ["note_pitch", "note_vel", "note_dur", "tick", "in1", "in4"] {
            assert!(
                errors(&format!("out = {name}"))
                    .iter()
                    .any(|e| e.contains("note-event")),
                "`{name}` should be rejected in a control-rate script"
            );
        }
        assert!(
            errors("out.pitch = 60")
                .iter()
                .any(|e| e.contains("note-event")),
            "`out.pitch` should be rejected in a control-rate script"
        );
    }

    #[test]
    fn note_event_rejects_audio_channels_and_bare_out() {
        // (e) `out.left` is a compile error in a note_event (non-audio) script.
        assert!(
            note_errors("out.left = 1")
                .iter()
                .any(|e| e.contains("audio-rate")),
            "`out.left` must be rejected in a note-event script"
        );
        // A bare mono `out` has no meaning in the note-event dialect.
        assert!(
            note_errors("out = note_pitch")
                .iter()
                .any(|e| e.contains("not a bare")),
            "a bare `out` must be rejected in a note-event script"
        );
    }

    #[test]
    fn note_event_names_reserved_only_in_note_event_dialect() {
        // In a note-event script the note identifiers cannot be shadowed…
        for name in ["note_pitch", "note_vel", "note_dur", "tick", "in1"] {
            assert!(
                note_errors(&format!("let {name} = 1\nout.pitch = 0"))
                    .iter()
                    .any(|e| e.contains("shadow")),
                "`{name}` must be reserved inside a note-event script"
            );
            // …but in every OTHER dialect they are ordinary bindable identifiers,
            // so pre-existing mod-matrix/audio scripts using them keep compiling
            // (regression fix: they were briefly reserved in all dialects).
            assert!(
                errors(&format!("let {name} = 1\nout = {name}")).is_empty(),
                "`{name}` must be bindable in a control-rate script"
            );
        }
    }

    #[test]
    fn note_event_rejects_engine_graph_sources() {
        // Macros, context vars, and module references are engine-graph modulation
        // sources with no meaning in a per-event note script — they must be a
        // compile error, not a silent runtime 0.
        for (src, kind) in [
            ("out.vel = note_vel * velocity", "macro"),
            ("out.vel = note_vel * tempo", "context var"),
            (
                "src x = lfo-1.out\nout.pitch = note_pitch + x",
                "module ref",
            ),
        ] {
            assert!(
                note_errors(src)
                    .iter()
                    .any(|e| e.contains("not available in a note-event")
                        || e.contains("module references are not available")),
                "note-event {kind} must be rejected: {src}"
            );
        }
    }

    #[test]
    fn note_event_duplicate_field_is_an_error() {
        assert!(
            note_errors("out.pitch = 1\nout.pitch = 2")
                .iter()
                .any(|e| e.contains("duplicate"))
        );
    }

    // ---- control-ports dialect (the `Script` module) ----------------------

    fn cp_opts() -> CompileOptions {
        CompileOptions {
            control_ports: true,
            ..CompileOptions::default()
        }
    }

    fn compile_cp_ok(src: &str) -> CompiledProgram {
        let (prog, diags) = compile(src, &cp_opts());
        let errs: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errs.is_empty(), "unexpected errors: {errs:?}");
        prog.expect("a compiled control-ports program")
    }

    fn cp_errors(src: &str) -> Vec<String> {
        compile(src, &cp_opts())
            .1
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message.clone())
            .collect()
    }

    /// Run a compiled control-ports program's `eval_multi`, filling each source
    /// register from `fill` (`in1..in4` arrive as `SourceInput::ControlIn`).
    fn eval_cp(prog: &CompiledProgram, fill: impl Fn(&SourceInput) -> f32) -> [Option<f32>; 4] {
        let sources: Vec<f32> = prog.inputs.iter().map(&fill).collect();
        let mut regs = RegisterFile::new(0, SEED);
        prog.script
            .eval_multi(&sources, &mut regs, &EvalContext::new(CR))
    }

    #[test]
    fn control_ports_out_emit_store_out_slots() {
        // out1..out4 map to StoreOut(0..3).
        let prog = compile_cp_ok("out1 = 0.1\nout2 = 0.2\nout3 = 0.3\nout4 = 0.4");
        for slot in 0u8..4 {
            assert!(
                prog.script.code().contains(&Op::StoreOut(slot)),
                "missing StoreOut({slot})"
            );
        }
    }

    #[test]
    fn control_ports_in_reads_control_in_registers() {
        let prog = compile_cp_ok("out1 = in1 * in2\nout2 = in3 + in4");
        for idx in 0u8..4 {
            assert!(
                prog.inputs.contains(&SourceInput::ControlIn(idx)),
                "missing in{}",
                idx + 1
            );
        }
        // in1=0.5, in2=0.4, in3=0.25, in4=0.1 → out1=0.2, out2=0.35; out3/out4 None.
        let outs = eval_cp(&prog, |inp| match inp {
            SourceInput::ControlIn(0) => 0.5,
            SourceInput::ControlIn(1) => 0.4,
            SourceInput::ControlIn(2) => 0.25,
            SourceInput::ControlIn(3) => 0.1,
            _ => 0.0,
        });
        assert!(outs[0].is_some_and(|v| approx(v, 0.2)));
        assert!(outs[1].is_some_and(|v| approx(v, 0.35)));
        assert_eq!(outs[2], None);
        assert_eq!(outs[3], None);
    }

    #[test]
    fn control_ports_bare_out_is_out1() {
        // A bare `out` yields slot 0 (out1) via the eval_multi fallback.
        let prog = compile_cp_ok("out = velocity * 0.5");
        let outs = eval_cp(&prog, |_| 1.0);
        assert!(outs[0].is_some_and(|v| approx(v, 0.5)));
        assert_eq!(outs[1], None);
        // `out` and `out1` are the same port — writing both is an error.
        assert!(
            cp_errors("out = 1\nout1 = 2")
                .iter()
                .any(|e| e.contains("same port"))
        );
    }

    #[test]
    fn control_ports_one_program_shares_locals_across_outputs() {
        // One program computes a value once and feeds it to several outputs (the
        // in-module chaining the 8-slot rack used to need `scr-1.out1` plumbing for).
        let prog = compile_cp_ok("let p = in1 * 2\nout1 = p\nout2 = p + 1");
        let outs = eval_cp(&prog, |inp| match inp {
            SourceInput::ControlIn(0) => 0.3,
            _ => 0.0,
        });
        assert!(outs[0].is_some_and(|v| approx(v, 0.6)));
        assert!(outs[1].is_some_and(|v| approx(v, 1.6)));
    }

    #[test]
    fn control_ports_numbered_ports_gated_to_the_dialect() {
        // out1/in1 are control-ports-only: rejected in a control-rate `scr` script.
        assert!(
            errors("out1 = 0")
                .iter()
                .any(|e| e.contains("control-ports"))
        );
        assert!(
            errors("out = in1")
                .iter()
                .any(|e| e.contains("note-event or control-ports"))
        );
        // Conversely, audio channels / note fields are rejected in control-ports.
        assert!(
            cp_errors("out.left = 0")
                .iter()
                .any(|e| e.contains("audio-rate"))
        );
    }

    #[test]
    fn control_ports_reject_past_the_ceiling() {
        assert!(
            cp_errors("out5 = 0")
                .iter()
                .any(|e| e.contains("max 4 CV outputs"))
        );
        assert!(
            cp_errors("out = in5")
                .iter()
                .any(|e| e.contains("max 4 CV inputs"))
        );
    }

    #[test]
    fn control_ports_port_names_are_reserved_only_in_this_dialect() {
        // in1..in4 / out1..out4 cannot be shadowed in a control-ports script…
        assert!(
            cp_errors("let in1 = 1\nout = in1")
                .iter()
                .any(|e| e.contains("shadow"))
        );
        assert!(
            cp_errors("let out1 = 1\nout = 0")
                .iter()
                .any(|e| e.contains("shadow"))
        );
        // …but `in1` stays an ordinary bindable local in a control-rate `scr` script.
        assert!(errors("let in1 = 1\nout = in1").is_empty());
    }

    // ---- `param` knob declarations ----------------------------------------

    #[test]
    fn param_declares_a_knob_read_as_a_local_param() {
        let prog = compile_cp_ok("param drive = 0.5\nout1 = in1 * drive");
        assert_eq!(prog.params.len(), 1);
        let d = &prog.params[0];
        assert_eq!(d.name_str, "drive");
        assert!(approx(d.default, 0.5));
        assert!(approx(d.min, 0.0) && approx(d.max, 1.0));
        assert_eq!(d.label, None);
        assert_eq!(d.tooltip, None);
        // The body reads it as a LocalParam source register.
        assert!(
            prog.inputs
                .iter()
                .any(|i| matches!(i, SourceInput::LocalParam(n) if *n == d.name))
        );
    }

    #[test]
    fn param_range_label_and_tooltip_parse() {
        let prog = compile_cp_ok(
            "param cutoff = 1000 [20, 20000] \"Cutoff\" \"filter cutoff\"\nout1 = cutoff",
        );
        let d = &prog.params[0];
        assert!(approx(d.default, 1000.0));
        assert!(approx(d.min, 20.0) && approx(d.max, 20000.0));
        assert_eq!(d.label.as_deref(), Some("Cutoff"));
        assert_eq!(d.tooltip.as_deref(), Some("filter cutoff"));
    }

    #[test]
    fn param_is_gated_to_script_modules() {
        // A plain control-rate `scr` script has no knobs.
        assert!(
            errors("param drive = 0.5\nout = drive")
                .iter()
                .any(|e| e.contains("only available in a Script or AudioScript"))
        );
        // Both the audio-rate and control-ports dialects allow it.
        assert_eq!(
            compile_audio_ok("param drive = 0.5\nout = in * drive")
                .params
                .len(),
            1
        );
    }

    #[test]
    fn param_default_and_range_must_be_constant() {
        assert!(
            cp_errors("param drive = velocity\nout1 = drive")
                .iter()
                .any(|e| e.contains("must be a constant"))
        );
        assert!(
            cp_errors("param drive = 0.5 [velocity, 1]\nout1 = drive")
                .iter()
                .any(|e| e.contains("bounds must be constants"))
        );
    }

    #[test]
    fn duplicate_param_name_is_an_error() {
        assert!(
            cp_errors("param drive = 0.1\nparam drive = 0.2\nout1 = drive")
                .iter()
                .any(|e| e.contains("duplicate"))
        );
    }

    #[test]
    fn param_count_is_capped() {
        let decls = (0..=SCRIPT_MAX_PARAMS)
            .map(|i| format!("param p{i} = 0"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            cp_errors(&format!("{decls}\nout1 = p0"))
                .iter()
                .any(|e| e.contains("too many params"))
        );
    }
}
