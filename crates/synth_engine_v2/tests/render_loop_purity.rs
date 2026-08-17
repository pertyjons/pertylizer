//! The render loop's source may not contain what the render loop may not do.
//!
//! The counting-allocator test covers allocation behaviourally. Three of the four rules
//! it cannot see: a lock, an I/O call, and a logging macro all leave the allocator alone
//! while breaking the audio thread just as thoroughly. This is a **lint-grade** check
//! over `src/render/hot.rs`, which exists as its own file so the region needs no
//! exceptions — `prepare` allocates legitimately, and a scan over one file holding both
//! could only be a scan with holes.
//!
//! What it cannot do is stop someone moving code out of the file to dodge it. That is
//! true of any structural check; what it does stop is the ordinary case, where a helper
//! that locks or logs is added to the hot path without anyone noticing.

use std::path::{Path, PathBuf};

fn hot_path_source() -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render/hot.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
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
    let source = hot_path_source();
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
    for (line_number, line) in code_lines(&source) {
        for (needle, why) in forbidden {
            if line.contains(needle) {
                found.push(format!(
                    "hot.rs:{line_number} contains `{needle}` ({why}): {line}"
                ));
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
    let source = hot_path_source();
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
    for (line_number, line) in code_lines(&source) {
        for needle in forbidden {
            if line.contains(needle) {
                found.push(format!("hot.rs:{line_number} contains `{needle}`: {line}"));
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
    let source = hot_path_source();
    let forbidden = [
        "unwrap()",
        "unwrap_or_else(",
        "expect(",
        "panic!",
        "unreachable!",
        "assert",
    ];

    let mut found = Vec::new();
    for (line_number, line) in code_lines(&source) {
        for needle in forbidden {
            if line.contains(needle) {
                found.push(format!("hot.rs:{line_number} contains `{needle}`: {line}"));
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
