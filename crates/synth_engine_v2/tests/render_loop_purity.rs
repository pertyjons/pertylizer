//! The render loop's source may not contain what the render loop may not do.
//!
//! The counting-allocator test covers allocation behaviourally. Three of the four rules
//! it cannot see: a lock, an I/O call, and a logging macro all leave the allocator alone
//! while breaking the audio thread just as thoroughly. This is a **lint-grade** check
//! over a named region of files, each of which exists as its own file so the region needs
//! no exceptions — `prepare` allocates legitimately, and a scan over one file holding
//! both could only be a scan with holes.
//!
//! # The region, and why it grew
//!
//! Phase 1's region was one file: `src/render/hot.rs`. Phase 2 dispatches through a
//! prepared function table, so the loop reaches code that is not in it — and
//! [ADR-0004](../../../plans/v2/decisions/ADR-0004-native-node-representation.md) clause
//! 4 makes that a hard requirement rather than a preference: **every callee reachable
//! from the loop must be enumerable from source**, because the phase's real-time
//! guarantee is a source-level transitive check. A dispatch shape whose callee set could
//! not be enumerated was excluded by that clause regardless of its speed.
//!
//! So the region is the loop plus the kernels, the callee set is the registry, and
//! [`the_kernel_registry_is_closed_and_every_kernel_is_in_the_region`] checks that the
//! registry names all of them and nothing else.
//!
//! What this cannot do is stop someone moving code out of the region to dodge it. That is
//! true of any structural check; what it does stop is the ordinary case, where a helper
//! that locks or logs is added to the hot path without anyone noticing.

use std::path::{Path, PathBuf};

/// The files the real-time rules cover, relative to the crate root.
///
/// `hot.rs` is the loop. `kernels.rs` is everything the loop calls through the prepared
/// function table — and the registry that resolves those pointers is deliberately *not*
/// here, because it runs at admission, allocates, and reads the IR.
const REGION: [&str; 2] = ["src/render/hot.rs", "src/node/kernels.rs"];

fn read_region_file(relative: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every file in the region, as (name, source).
fn region_sources() -> Vec<(&'static str, String)> {
    REGION
        .iter()
        .map(|relative| (*relative, read_region_file(relative)))
        .collect()
}

fn hot_path_source() -> String {
    read_region_file("src/render/hot.rs")
}

/// The file's code lines, with comments and doc comments removed.
///
/// A comment naming a forbidden construct is how the reasons get recorded — this file's
/// own header does it — so scanning them would make the check unusable.
fn code_lines(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim().to_owned()))
        .filter(|(_, line)| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("///")
                && !line.starts_with("//!")
        })
        .collect()
}

#[test]
fn the_render_loop_takes_no_lock_and_performs_no_io_or_logging() {
    let forbidden = [
        ("Mutex", "a blocking lock"),
        ("RwLock", "a blocking lock"),
        (".lock(", "a blocking lock"),
        (".read(", "a blocking lock or an I/O read"),
        (".write(", "a blocking lock or an I/O write"),
        ("println!", "logging"),
        ("eprintln!", "logging"),
        ("dbg!", "logging"),
        ("tracing::", "logging"),
        ("log::", "logging"),
        ("std::fs", "filesystem I/O"),
        ("File::", "filesystem I/O"),
        ("std::thread", "scheduling"),
        ("std::time", "a clock read"),
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for (needle, why) in forbidden {
                if line.contains(needle) {
                    found.push(format!(
                        "{file}:{line_number} contains `{needle}` ({why}): {line}"
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not do these things:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_render_loop_contains_no_allocating_construct() {
    let forbidden = [
        "Vec::new",
        "Vec::with_capacity",
        "vec![",
        "to_vec()",
        "String::",
        "format!(",
        "to_owned()",
        "to_string()",
        "Box::new",
        "HashMap",
        "BTreeMap",
        "collect()",
        ".sort_by_key(",
        ".sort_by(",
        ".sort(",
        ".push(",
        ".insert(",
        ".resize(",
        ".extend(",
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for needle in forbidden {
                if line.contains(needle) {
                    found.push(format!("{file}:{line_number} contains `{needle}`: {line}"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not allocate or grow a collection:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_render_loop_cannot_panic_through_an_unchecked_accessor() {
    // `unwrap` and `expect` are forbidden in production code repository-wide; on the
    // audio thread a panic is worse than a wrong sample. The stable sort is banned above
    // for a different reason — `sort_by_key` allocates while `sort_unstable_by_key` does
    // not — and this is where the panicking accessors are caught.
    let forbidden = [
        "unwrap()",
        "unwrap_or_else(",
        "expect(",
        "panic!",
        "unreachable!",
        "assert",
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for needle in forbidden {
                if line.contains(needle) {
                    found.push(format!("{file}:{line_number} contains `{needle}`: {line}"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must not be able to panic:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_check_is_reading_the_file_it_thinks_it_is() {
    // A scan over an empty or renamed file passes every assertion above. This is the
    // control: the file has to be the render loop.
    let source = hot_path_source();
    assert!(source.len() > 2_000, "hot.rs is unexpectedly small");
    for expected in [
        "impl Renderer for PreparedRenderer",
        "fn render_quantum",
        "fn resolve_events",
        "sort_unstable_by_key",
    ] {
        assert!(
            source.contains(expected),
            "hot.rs does not contain `{expected}`; the render loop moved and this check is now \
             scanning the wrong thing"
        );
    }
}

#[test]
fn the_render_loop_makes_no_topology_or_naming_decision() {
    // The Phase 2 gate adds four constructs to the three Phase 1 checked: no port
    // strings, no map lookups, no graph traversal or topology decision, no buffer
    // resizing. `HashMap` and `.resize(` are already banned by the allocation check
    // above, so these are the rest — the identities the compiler is supposed to have
    // turned into slots, and the searches it is supposed to have made unnecessary.
    let forbidden = [
        (
            "PortId",
            "a port identity is a naming decision the compiler owed",
        ),
        ("PortName", "a port string"),
        (
            "NodeId",
            "an identity that should have been compiled to a slot",
        ),
        (
            "ParameterId",
            "an identity that should have been compiled to a slot",
        ),
        ("GraphIr", "graph traversal"),
        ("IrNodeKind", "a topology decision"),
        ("IrEdge", "graph traversal"),
        (
            ".find(",
            "a search where the compiler could have produced an index",
        ),
        (
            ".iter().position(",
            "a search where the compiler could have produced an index",
        ),
    ];

    let mut found = Vec::new();
    for (file, source) in region_sources() {
        for (line_number, line) in code_lines(&source) {
            for (needle, why) in forbidden {
                if line.contains(needle) {
                    found.push(format!(
                        "{file}:{line_number} contains `{needle}` ({why}): {line}"
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the render loop must decide nothing about topology or naming:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn every_call_the_render_loop_makes_is_inside_the_checked_region() {
    // What the scans above cannot do on their own: someone moves a locking helper into
    // another file and calls it from here, and every check still passes. The file's own
    // header said exactly that. This closes it.
    //
    // Every call in this file — method, associated function, or free function, on any
    // receiver, however the line breaks fall — must match either something defined in
    // this file or a name on the allowlist below. The allowlist is not a convenience:
    // each group is justified, and a name that is not on it fails this test until
    // someone justifies it here.
    //
    // **What this cannot do, stated rather than implied.** It matches *names*, not
    // resolved targets: a helper elsewhere called `get` on some other receiver would
    // pass. Closing that needs type resolution, which a source scan does not have. Two
    // things bound the gap. The import check below means a free function cannot be
    // brought in at all, so an escape has to be a method on a type this file already
    // touches. And the counting-allocator test covers the allocation half behaviourally
    // whatever the call is named. What is left is a locking or blocking method on an
    // existing receiver, deliberately named like an accessor — and the allowlist is
    // short enough that a reader checking it would notice.
    let region: Vec<(&str, String)> = region_sources()
        .into_iter()
        .map(|(file, source)| (file, strip_comments(&source)))
        .collect();
    let source: String = region
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let defined: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("fn ")
                .or_else(|| trimmed.strip_prefix("pub fn "))
                .or_else(|| trimmed.strip_prefix("pub const fn "))
                .or_else(|| trimmed.strip_prefix("const fn "))
                .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
                .or_else(|| trimmed.strip_prefix("pub(crate) const fn "))
        })
        .filter_map(|rest| rest.split(['(', '<']).next())
        .map(|name| name.trim().to_owned())
        .collect();

    // Control flow, visibility and attributes, none of which are calls. `pub(crate)` and
    // `#[derive(..)]` are both an identifier followed by a parenthesis and are neither
    // reachable code nor nameable as a function.
    let syntax = [
        "if", "while", "for", "match", "return", "loop", "else", "fn", "let", "move", "pub",
        "derive",
    ];

    // `std` on slices, `Option`, iterators and primitives: bounds-checked accessors,
    // fills, a memmove, saturating integer arithmetic, and the one in-place sort. None
    // allocates, locks, or can panic — `get`/`get_mut` return `Option` precisely so the
    // loop indexes without panicking, and `sort_unstable_by_key` is the non-allocating
    // sort where `sort_by_key` would allocate.
    let std_calls = [
        "get",
        "get_mut",
        "fill",
        "len",
        "is_empty",
        "copied",
        "copy_within",
        "copy_from_slice",
        "iter",
        "enumerate",
        "contains",
        "sort_unstable_by_key",
        "unwrap_or",
        "map_or",
        "ok",
        "is_some_and",
        "try_from",
        "from",
        "ok_or",
        "map_err",
        "default",
        "checked_sub",
        "saturating_mul",
        "saturating_add",
        "saturating_sub",
        "div_ceil",
        "min",
        "max",
        "abs",
        "floor",
        "sin",
        "matches",
        // The kernels' half: in-place iteration over preallocated slices, the borrow
        // split that turns slots into slices, and one sort over a fixed-size array of
        // three entries. `split_at_mut_checked` returns `Option` rather than panicking,
        // and `sort_unstable` is the non-allocating sort.
        "iter_mut",
        "zip",
        "flatten",
        "first",
        "checked_mul",
        "split_at_mut_checked",
        "sort_unstable",
        "size_of",
    ];

    // This crate's own `const fn` accessors and `Copy` constructors: field reads on the
    // prepared plan, the anchor, the clock, the event envelope, and the diagnostics
    // counters, plus `new` on the checked time and count newtypes. Each is a field read
    // or a saturating add.
    let crate_accessors = [
        "new",
        "channel_layout",
        "forward_event_horizon",
        "max_events_per_quantum",
        "maximum_block_size",
        "ops",
        "parameter_targets",
        "sample_rate",
        "id",
        "plan",
        "index",
        "position",
        "time",
        "epoch",
        "envelope",
        "payload",
        "source",
        "is_ingress",
        "is_negative",
        "difference",
        "checked_add",
        "checked_advance_quantum",
        "quantum_index",
        "as_u64",
        "as_i64",
        "as_f32",
        "as_usize",
        "as_slice",
        "into_frequency",
        "into_amplitude",
        "frames",
        "layout",
        "needs_reprepare",
        "set_needs_reprepare",
        "count_late_event",
        "count_stale_epoch_event",
        "count_out_of_horizon_event",
        "count_arrival_stamped_event",
        "count_foreign_slot_event",
        "count_oversized_callback",
        "count_clock_exhaustion",
        // Phase 2's additions: the compiled step's slots, the prepared table, and the
        // one method that moves a control. Each is a field read or an assignment.
        "prepared_nodes",
        "node",
        "out",
        "inputs",
        "in_place_safe",
        "kernel",
        "set_control",
        "bind",
        // The segment counter's own accessors: a field read, a comparison with zero, and
        // a saturating decrement.
        "is_finished",
        "spent",
        // The binding a step resolved at admission, read rather than recomputed.
        "order",
        "bindings",
    ];

    let mut unresolved: Vec<String> = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    for (index, character) in bytes.iter().enumerate() {
        if *character != '(' {
            continue;
        }
        // The identifier immediately before the parenthesis, if any.
        let mut start = index;
        while start > 0
            && bytes
                .get(start - 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            start -= 1;
        }
        if start == index {
            continue;
        }
        let name: String = bytes[start..index].iter().collect();
        if name.chars().next().is_some_and(char::is_uppercase) {
            // A tuple-struct or enum-variant construction, not a call.
            continue;
        }

        // A **module-qualified** call reaches out of this file whatever it is named, and
        // matching only the final identifier would accept `crate::elsewhere::fault(..)`
        // through a local `fn fault`. The qualifier decides: `Type::new(..)` is a
        // constructor on a `Copy` newtype, `module::helper(..)` is code this check
        // cannot see.
        let qualifier: String = bytes[..start]
            .iter()
            .rev()
            .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == ':')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Rust's primitives are types whose names are lowercase, so they are the one
        // qualifier that looks like a module and is not one. `u32::try_from` and
        // `f64::from` are conversions on `Copy` values.
        let primitives = [
            "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
            "f32", "f64", "bool", "char", "str",
        ];
        if qualifier.ends_with("::") {
            let owner = qualifier.trim_end_matches("::").rsplit("::").next();
            let is_module = owner.is_some_and(|segment| {
                !primitives.contains(&segment)
                    && segment
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_lowercase() || first == '_')
            });
            // A qualifier that names a **region module** is not a reach outside the
            // checked region: `kernels::bind` is code this scan reads, and refusing it
            // would mean the loop could not call a kernel at all. Anything else
            // module-qualified is exactly what the scan cannot see.
            let region_module = owner.is_some_and(|segment| REGION_MODULES.contains(&segment));
            let qualified = format!("{qualifier}{name}");
            if is_module && !region_module && !unresolved.contains(&qualified) {
                unresolved.push(qualified);
                continue;
            }
        }
        if syntax.contains(&name.as_str())
            || std_calls.contains(&name.as_str())
            || crate_accessors.contains(&name.as_str())
            || defined.contains(&name)
        {
            continue;
        }
        if !unresolved.contains(&name) {
            unresolved.push(name);
        }
    }

    assert!(
        unresolved.is_empty(),
        "the render loop calls names that are neither defined in hot.rs nor justified in \
         this test's allowlist, so the real-time guarantee does not cover them: {unresolved:?}"
    );

    // A control: the scan has to be finding calls at all.
    assert!(
        defined.iter().any(|name| name == "render_quantum"),
        "the definition scan found no `render_quantum`, so it is not reading this file"
    );
}

#[test]
fn the_render_loop_imports_no_free_function() {
    // The half of reachability a name scan *can* settle. Every `use` in the hot path
    // must name a type, a trait, or a constant — all of which start with an uppercase
    // letter — so a free function cannot be brought into scope to be called. That is the
    // loophole the check above would otherwise leave wide: a lowercase import is the one
    // way to call code elsewhere without a receiver this file already holds.
    // Whole items, not lines: `hot.rs` already has a multiline brace import, and a
    // line-based scan would skip every name inside it — which is precisely where a
    // lowercase helper would be added.
    let source = strip_comments(&hot_path_source()).replace('\n', " ");
    let mut imported_functions = Vec::new();

    for item in source.split(';') {
        let trimmed = item.trim();
        let Some(rest) = trimmed.strip_prefix("use ") else {
            continue;
        };
        let names = rest.replace(['{', '}'], " ");
        for name in names.split([',', ' ']) {
            let Some(last) = name.rsplit("::").next() else {
                continue;
            };
            let last = last.trim();
            if last.is_empty() || last == "self" || last == "super" || last == "crate" {
                continue;
            }
            // A glob is worse than a lowercase name: it brings in whatever the other
            // module has, including free functions whose names collide with the
            // accessors the call scan allows. A **region module** is the exception, and
            // the only one: importing `kernels` is how the loop reaches the function
            // table, and everything in it is scanned by this file.
            if last == "*"
                || (last.chars().next().is_some_and(char::is_lowercase)
                    && !REGION_MODULES.contains(&last))
            {
                imported_functions.push(last.to_owned());
            }
        }
    }

    assert!(
        imported_functions.is_empty(),
        "the render loop imports lowercase names, which are functions or modules rather \
         than types: {imported_functions:?}"
    );

    // The control: a scan that found nothing at all would also pass the assertion above.
    // `hot.rs` imports its types through a multiline brace list, and every one of them
    // has to have been seen.
    let seen = source
        .split(';')
        .filter(|item| item.trim().starts_with("use "))
        .count();
    assert!(
        seen >= 4,
        "the import scan found {seen} `use` items, so it is not reading the imports"
    );
}

/// The modules of the region, as they are named in a `use` or a call qualifier.
///
/// A call into one of these is not a reach outside the checked region: this file reads
/// them. Adding a name here without adding its file to [`REGION`] would be the whole
/// check going quietly hollow, and
/// [`the_region_modules_are_all_scanned`] is what stops that.
const REGION_MODULES: [&str; 1] = ["kernels"];

#[test]
fn the_region_modules_are_all_scanned() {
    // The two lists have to agree, or the call scan would wave through a module nobody
    // reads. `kernels` is `src/node/kernels.rs`; the mapping is by file name.
    for module in REGION_MODULES {
        assert!(
            REGION
                .iter()
                .any(|file| file.ends_with(&format!("/{module}.rs"))),
            "`{module}` is trusted by the call and import scans but is not in the scanned region"
        );
    }
}

#[test]
fn the_kernel_registry_is_closed_and_every_kernel_is_in_the_region() {
    // ADR-0004 clause 4: the callee set has to be enumerable from source. It is the
    // registry, and this is the check that the registry and the kernels are the same
    // set — a kernel nobody registers is dead code, and a registered name that is not a
    // kernel defined in the region is a call the scans above never see.
    let kernels = strip_comments(&read_region_file("src/node/kernels.rs"));
    let registry = strip_comments(&read_region_file("src/node.rs"));

    let defined: Vec<String> = kernels
        .split("pub fn ")
        .skip(1)
        .filter_map(|rest| {
            let name = rest.split(['(', '<']).next()?.trim().to_owned();
            // A kernel is a function with the kernel signature, not merely a public one:
            // `bind` is in this file and is called by the loop directly.
            rest.contains("io: &mut NodeIo<'_>").then_some(name)
        })
        .collect();
    assert!(
        defined.len() > 3,
        "found {} kernels, so this scan is not reading the kernel file",
        defined.len()
    );

    let mut registered: Vec<String> = Vec::new();
    for fragment in registry.split("kernel: kernels::").skip(1) {
        if let Some(name) = fragment.split([',', ' ', '\n']).next() {
            registered.push(name.trim().to_owned());
        }
    }
    assert!(
        !registered.is_empty(),
        "the registry scan found no `kernel: kernels::` entry, so it is not reading node.rs"
    );

    for name in &defined {
        assert!(
            registered.contains(name),
            "`{name}` has the kernel signature but no registry entry, so nothing can schedule it"
        );
    }
    for name in &registered {
        assert!(
            defined.contains(name),
            "the registry names `{name}`, which is not a kernel defined in the scanned region"
        );
    }
}

/// The source with comments and string literals removed.
///
/// Both would otherwise feed the scan: this file's own header names several forbidden
/// constructs, and a diagnostic's message text contains parentheses.
fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let code = line.split("//").next().unwrap_or("");
            let mut out = String::with_capacity(code.len());
            let mut in_string = false;
            let mut escaped = false;
            for character in code.chars() {
                match character {
                    '"' if !escaped => in_string = !in_string,
                    '\\' if in_string => escaped = !escaped,
                    _ => escaped = false,
                }
                if !in_string && character != '"' {
                    out.push(character);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}
