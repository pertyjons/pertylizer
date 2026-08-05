//! Tests for `tool_annotations_tests`.
//!
//! Every tool carries MCP annotation hints so clients can auto-approve reads and
//! warn before destructive calls. A wrong hint is worse than none — a mutating
//! tool marked `readOnlyHint` gets silently auto-approved — so the invariants are
//! locked here rather than left to review.

use super::*;

/// The read-only families. A tool named outside these must not claim to be read
/// only, and every tool inside them must — that is what keeps a newly added
/// `get_*`/`list_*` from shipping unannotated.
const READ_ONLY_PREFIXES: &[&str] = &[
    "get_",
    "list_",
    "analyze_",
    "compare_",
    "search_",
    "find_",
    "check_",
    "lint_",
    "suggest_",
    "validate_",
];

fn all_tools() -> Vec<rmcp::model::Tool> {
    SynthMcpServer::build_router(&[]).list_all()
}

#[test]
fn every_tool_carries_annotations() {
    let missing: Vec<_> = all_tools()
        .into_iter()
        .filter(|t| t.annotations.is_none())
        .map(|t| t.name.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "tools without annotation hints: {missing:?}"
    );
}

#[test]
fn read_only_matches_the_read_only_families() {
    for tool in all_tools() {
        let expected = READ_ONLY_PREFIXES.iter().any(|p| tool.name.starts_with(p));
        let actual = tool
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        assert_eq!(
            actual, expected,
            "{}: read_only_hint should be {expected}",
            tool.name
        );
    }
}

/// `destructiveHint` is only meaningful when `readOnlyHint` is false, so a tool
/// claiming both is a contradiction a client cannot act on.
#[test]
fn no_tool_is_both_read_only_and_destructive() {
    for tool in all_tools() {
        let Some(a) = tool.annotations.as_ref() else {
            continue;
        };
        assert!(
            !(a.read_only_hint.unwrap_or(false) && a.destructive_hint.unwrap_or(false)),
            "{} claims to be both read-only and destructive",
            tool.name
        );
    }
}

/// Deleting, clearing and disconnecting are the irreversible families; none of
/// them may be left looking additive.
#[test]
fn teardown_families_are_marked_destructive() {
    for tool in all_tools() {
        if !tool.name.starts_with("delete_")
            && !tool.name.starts_with("remove_")
            && !tool.name.starts_with("clear_")
            && !tool.name.starts_with("disconnect")
        {
            continue;
        }
        let destructive = tool
            .annotations
            .as_ref()
            .and_then(|a| a.destructive_hint)
            .unwrap_or(false);
        assert!(destructive, "{} should be destructive", tool.name);
    }
}
