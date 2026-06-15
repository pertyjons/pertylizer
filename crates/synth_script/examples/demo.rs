//! Quick YAMS demo: format + compile + evaluate a few snippets, then show what
//! compilation errors look like. Run with `cargo run -p synth_script --example demo`.

use synth_core::script::{EvalContext, RegisterFile};
use synth_script::symbols::{Context, Macro};
use synth_script::{CompileOptions, CompiledProgram, LineCol, SourceInput, compile, format};

/// Demo values fed into each source register.
fn fill(inp: &SourceInput) -> f32 {
    match inp {
        SourceInput::Macro(Macro::Velocity) => 0.8,
        SourceInput::Macro(Macro::ModWheel) => 0.5,
        SourceInput::Macro(_) => 0.0,
        SourceInput::Context(Context::Sr) => 750.0,
        SourceInput::Context(_) => 0.0,
        SourceInput::Module { module, member, .. } if module == "lfo" && member == "out" => 1.0,
        SourceInput::Module { .. } => 0.0,
    }
}

fn eval_once(prog: &CompiledProgram) -> f32 {
    let sources: Vec<f32> = prog.inputs.iter().map(fill).collect();
    let mut regs = RegisterFile::new(0, 0x5EED);
    prog.script
        .eval(&sources, &mut regs, &EvalContext::new(750.0))
}

fn show_valid(src: &str) {
    println!("┌─ source");
    for line in src.lines() {
        println!("│  {line}");
    }
    match format(src) {
        Ok(canon) => {
            print!("├─ yamsfmt\n");
            for line in canon.lines() {
                println!("│  {line}");
            }
        }
        Err(_) => println!("├─ yamsfmt: (parse error)"),
    }
    let (prog, _diags) = compile(src, &CompileOptions::default());
    match prog {
        Some(p) => {
            let inputs: Vec<String> = p
                .inputs
                .iter()
                .map(|i| match i {
                    SourceInput::Macro(m) => format!("{m:?}"),
                    SourceInput::Context(c) => format!("{c:?}"),
                    SourceInput::Module {
                        module,
                        instance,
                        member,
                    } => format!("{module}-{instance}.{member}"),
                })
                .collect();
            println!(
                "├─ compiled: {} instrs, {} const, sources [{}]",
                p.script.code().len(),
                p.script.constants().len(),
                inputs.join(", ")
            );
            println!("└─ eval → {:.4}\n", eval_once(&p));
        }
        None => println!("└─ compile failed\n"),
    }
}

fn show_errors(src: &str) {
    println!("source: {src:?}");
    let (_prog, diags) = compile(src, &CompileOptions::default());
    for d in diags.iter().filter(|d| d.is_error()) {
        let lc = LineCol::resolve(src, d.span.start);
        println!("  error [{}:{}] {}", lc.line, lc.col, d.message);
    }
    println!();
}

fn main() {
    println!("=================  VALID PROGRAMS  =================\n");
    show_valid("out = velocity * 0.6");
    show_valid("src lfo = lfo-1.out\nout = lfo * 0.45 + velocity * 0.6");
    show_valid("src lfo = lfo-1.out\nout = velocity > 0.8 ? lfo : lfo * 0.2");
    show_valid("let depth = lerp(0.2, 1.0, velocity)\nout = clamp(depth, 0, 1)");
    // messy input → canonical formatting (and note: 1 + 2 * 3 is NOT folded by fmt)
    show_valid("src   lfo=lfo-1.out\nout=lfo*0.45+velocity*0.6   # wobble");

    println!("=================  STATEFUL (lag over 6 blocks)  =================\n");
    {
        let src = "out = lag(velocity, 50ms)";
        println!("source: {src}  (velocity = 0.8)");
        let (prog, _) = compile(src, &CompileOptions::default());
        let p = prog.expect("compiles");
        let sources: Vec<f32> = p.inputs.iter().map(fill).collect();
        let mut regs = RegisterFile::new(0, 0x5EED);
        let ctx = EvalContext::new(750.0);
        print!("  out per block: ");
        for _ in 0..6 {
            print!("{:.4}  ", p.script.eval(&sources, &mut regs, &ctx));
        }
        println!("\n");
    }

    println!("=================  COMPILE ERRORS  =================\n");
    show_errors("out = a < b < c");
    show_errors("let sin = 2\nout = sin");
    show_errors("out = foo + 1");
    show_errors("out = clamp(1, 2)");
    show_errors("out = rand(5)");
    show_errors("src lfo = lfo-1.out");
    show_errors("src velocity = lfo-1.out\nout = 0");
    show_errors("out = 1 +");
}
