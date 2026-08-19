//! The resource ledger's `file:line` citations have to keep pointing at what
//! they claim.
//!
//! `plans/v2/PROCESS.md` makes inventories or evidence the authority for claims
//! about current V1 behaviour because a behaviour someone read and a behaviour
//! someone assumed look identical in prose. A line citation does not guarantee
//! that the claim stays true: the ledger's own fix to `LIMIT-0004` added lines to
//! `render/mod.rs` and silently broke the five neighbouring entries, and the bug
//! fixes that came out of the use-site audit then broke citations into
//! `state.rs` and `synth_engine.rs` that the audit had just repaired.
//!
//! Drift is invisible to a reader: a stale line number still resolves, it just
//! lands on a comment or a closing brace, and nothing says so. These tests make
//! it fail instead of rot.

use std::path::{Path, PathBuf};

/// Repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn ledger() -> String {
    let path = repo_root().join("plans/v2/inventories/resource-limits.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// One `path.rs:line` citation, with the entry that made it.
struct Citation {
    entry: String,
    path: String,
    line: usize,
    /// Whether this line may be checked for drift.
    ///
    /// The closing end of a cited range is legitimately a `}` — citing a
    /// struct's span means citing the brace that ends it — so ranges contribute
    /// their endpoints to the existence check but only their start to the drift
    /// check.
    strict: bool,
}

/// Extract every citation from every cell of every ledger row.
///
/// All columns are scanned, not just the enforcement site: the overflow-behaviour
/// and disposition cells cite lines too, and an earlier version of this test that
/// read only the site column let a stale `LIMIT-0024` overflow citation pass.
///
/// Compound forms are expanded, because the ledger uses all of them:
/// `` `path.rs:75-78` ``, `` `path.rs:308,532,619` ``, and a bare `` `:199` ``
/// continuation that inherits the path named earlier in the same cell.
fn citations() -> Vec<Citation> {
    let text = ledger();
    let mut out = Vec::new();
    let mut orphans: Vec<String> = Vec::new();

    for row in text.lines().filter(|l| l.contains(".rs:")) {
        let cells: Vec<&str> = row.split('|').collect();
        // Inventory rows name their entry in the first cell; the truncation
        // register and the audit table put it in backticks, and prose names
        // none. Whatever is found becomes the label in a failure message.
        let entry = cells
            .get(1)
            .map(|c| c.trim().trim_matches('`').to_owned())
            .filter(|c| c.starts_with("LIMIT-"))
            .unwrap_or_else(|| {
                row.split('`')
                    .find(|t| t.starts_with("LIMIT-"))
                    .unwrap_or("(prose)")
                    .to_owned()
            });

        for cell in cells
            .iter()
            .skip(1)
            .chain(std::iter::once(&row).take(usize::from(cells.len() < 2)))
        {
            // A path holds until the next path in the same cell, so `:199`
            // after `crates/a/b.rs:178` refers to b.rs.
            let mut current: Option<String> = None;

            for chunk in cell.split('`') {
                let (path, spec) = match chunk.split_once(".rs:") {
                    Some((p, rest)) if !p.is_empty() => {
                        // A bare file name is resolved against the tree. It is
                        // an error for it to be ambiguous: `cpal_backend.rs:78`
                        // followed by `:114` made the second line inherit an
                        // entirely different file, and the citation still read
                        // as precise.
                        current = Some(resolve(&format!("{p}.rs")));
                        (current.clone(), rest)
                    }
                    // A continuation is `:123`, not prose that happens to start
                    // with a colon — `LIMIT-0003`'s "`: sized from `" is not a
                    // citation and must not be reported as an orphan.
                    _ if chunk.starts_with(':')
                        && chunk[1..].starts_with(|c: char| c.is_ascii_digit()) =>
                    {
                        (current.clone(), &chunk[1..])
                    }
                    _ => continue,
                };
                let Some(path) = path else {
                    // A `:123` continuation with no path named before it in the
                    // same cell. Silently skipping these is how `LIMIT-0015`'s
                    // `:83` went unchecked; a citation nobody can resolve is the
                    // failure this file exists to catch, so it is named.
                    orphans.push(format!(
                        "{entry} cites `{chunk}` with no path named before it"
                    ));
                    continue;
                };

                for (line, strict) in expand_line_spec(spec) {
                    out.push(Citation {
                        entry: entry.clone(),
                        path: path.clone(),
                        line,
                        strict,
                    });
                }
            }
        }
    }

    assert!(
        orphans.is_empty(),
        "citations with no resolvable path:\n  {}",
        orphans.join("\n  ")
    );
    assert!(
        out.len() > 100,
        "expected the ledger to carry many citations, found {} — the parser or \
         the table shape has changed",
        out.len()
    );
    out
}

/// Turn a cited path into one repo-relative path, or leave it for the existence
/// check to report.
///
/// A path containing `/` is taken as repo-relative. A bare file name is resolved
/// by searching `crates/`; a unique hit is used, and anything else is returned
/// unchanged so the existence test names it rather than this helper guessing.
fn resolve(cited: &str) -> String {
    if cited.contains('/') {
        return cited.to_owned();
    }
    let mut hits = Vec::new();
    let mut stack = vec![repo_root().join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(cited)
                && let Ok(rel) = path.strip_prefix(repo_root())
            {
                hits.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    if hits.len() == 1 {
        hits.remove(0)
    } else {
        cited.to_owned()
    }
}

/// Expand `75-78`, `308,532,619`, or `139` into the lines they name.
///
/// A range is expanded to its endpoints rather than every line between them, and
/// only the start is marked strict: the interior and the closing end of a cited
/// block are allowed to be blank or punctuation.
fn expand_line_spec(spec: &str) -> Vec<(usize, bool)> {
    let head: String = spec
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '-'))
        .collect();
    let mut lines = Vec::new();

    for part in head.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((a, b)) => {
                if let Ok(n) = a.trim().parse::<usize>() {
                    lines.push((n, true));
                }
                if let Ok(n) = b.trim().parse::<usize>() {
                    lines.push((n, false));
                }
            }
            None => {
                if let Ok(n) = part.parse::<usize>() {
                    lines.push((n, true));
                }
            }
        }
    }
    lines
}

/// Every cited file exists and every cited line is inside it.
///
/// This is the cheap half: it catches a deleted file, a renamed module, and a
/// citation that ran off the end after code was removed.
#[test]
fn every_cited_file_and_line_exists() {
    let root = repo_root();
    let mut bad = Vec::new();

    for c in citations() {
        let full = root.join(&c.path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            bad.push(format!("{} cites missing file {}", c.entry, c.path));
            continue;
        };
        let count = text.lines().count();
        if c.line == 0 || c.line > count {
            bad.push(format!(
                "{} cites {}:{}, which has {count} lines",
                c.entry, c.path, c.line
            ));
        }
    }

    assert!(
        bad.is_empty(),
        "stale ledger citations:\n  {}",
        bad.join("\n  ")
    );
}

/// Every cited line points at something, not at whitespace or lone punctuation.
///
/// This is the half that catches drift. When a citation slips by a few lines it
/// almost always lands on a blank line, a `///` continuation, or a closing
/// brace — every one of the five drifted entries the pass-5 audit found did.
/// A line that is only punctuation cannot be the enforcement site for anything.
#[test]
fn no_citation_points_at_blank_or_punctuation() {
    let root = repo_root();
    let mut bad = Vec::new();

    for c in citations().into_iter().filter(|c| c.strict) {
        let full = root.join(&c.path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue; // covered by the test above
        };
        let Some(line) = text.lines().nth(c.line - 1) else {
            continue;
        };
        let trimmed = line.trim();

        let empty = trimmed.is_empty();
        let punctuation_only = !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|ch| matches!(ch, '{' | '}' | '(' | ')' | '[' | ']' | ',' | ';'));
        // A bare `///` or `//!` with no words is a continuation line, never a site.
        let empty_doc = matches!(trimmed, "///" | "//!" | "//");

        if empty || punctuation_only || empty_doc {
            bad.push(format!(
                "{} cites {}:{}, which is {:?} — the code moved and the citation did not",
                c.entry, c.path, c.line, trimmed
            ));
        }
    }

    assert!(
        bad.is_empty(),
        "ledger citations that have drifted off their site:\n  {}",
        bad.join("\n  ")
    );
}

/// Where the ledger names the identifier a citation points at, that identifier
/// has to be on that line.
///
/// This is the only check here that catches drift onto *valid* code. The shape
/// test above catches a citation that slips onto a brace or a blank line, which
/// is the common case, but a citation that slips onto a neighbouring statement
/// reads as fine — `LIMIT-0014` cited the sequencer buffer's allocation three
/// lines above where it actually is, and every other check passed.
///
/// The annotation form is `` `path.rs:60` (`MAX_TAIL_SECONDS`) ``. Entries that
/// do not use it are not checked by this test, so adopting it is how an entry
/// buys the stronger guarantee.
#[test]
fn named_identifiers_are_on_the_lines_that_cite_them() {
    let root = repo_root();
    let text = ledger();
    let mut checked = 0_usize;
    let mut bad = Vec::new();

    // `path.rs:123` (`IDENT`) — the annotation must follow immediately.
    for line in text.lines().filter(|l| l.contains(".rs:")) {
        let mut rest = line;
        while let Some(pos) = rest.find(".rs:") {
            let (before, after) = rest.split_at(pos);
            let path_start = before.rfind('`').map_or(0, |i| i + 1);
            let path = format!("{}.rs", &before[path_start..]);
            let after = &after[".rs:".len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            let tail = &after[digits.len()..];
            rest = after;

            let Ok(number) = digits.parse::<usize>() else {
                continue;
            };
            // Accept only the exact `` ` (` `` annotation, nothing looser.
            let Some(annotation) = tail.strip_prefix("` (`").and_then(|t| t.split('`').next())
            else {
                continue;
            };

            let full = root.join(resolve(&path));
            let Ok(source) = std::fs::read_to_string(&full) else {
                continue; // reported by the existence test
            };
            let Some(target) = source.lines().nth(number - 1) else {
                continue;
            };

            checked += 1;
            if !target.contains(annotation) {
                bad.push(format!(
                    "{path}:{number} is annotated `{annotation}` but that line is {:?}",
                    target.trim()
                ));
            }
        }
    }

    assert!(
        checked > 0,
        "no annotated citations found — the annotation form has changed"
    );
    assert!(
        bad.is_empty(),
        "citations whose named identifier is not on the cited line:\n  {}",
        bad.join("\n  ")
    );
}
