//! Tests for `batch_msg_tests`.

use super::*;

#[test]
fn full_success_leads_with_ok() {
    assert_eq!(batch_msg(3, "widgets set", &[], &[]), "OK: 3 widgets set");
}

#[test]
fn partial_success_leads_with_ok_and_lists_failures() {
    let msg = batch_msg(2, "widgets set", &[], &["boom".to_string()]);
    assert!(msg.starts_with("OK: 2 widgets set"), "got: {msg}");
    assert!(msg.contains("1 failed: boom"), "got: {msg}");
}

#[test]
fn total_failure_leads_with_error_not_ok() {
    // Every item failed: the message must not read as success to a caller
    // that gates on a leading "Error:".
    let msg = batch_msg(0, "widgets set", &[], &["boom".to_string()]);
    assert!(msg.starts_with("Error: 0 widgets set"), "got: {msg}");
    assert!(
        !msg.starts_with("OK:"),
        "total failure must not lead with OK: {msg}"
    );
    assert!(msg.contains("1 failed: boom"), "got: {msg}");
}

#[test]
fn empty_batch_leads_with_ok() {
    // Nothing attempted, nothing failed — still a benign success.
    assert_eq!(batch_msg(0, "widgets set", &[], &[]), "OK: 0 widgets set");
}

/// The verdict comes from the items, not from the sentence.
///
/// The three cases that used to be read out of prose and JSON shapes — total
/// failure, partial success, full success — are properties of `MutationResult`
/// now, so this asserts the predicate itself rather than a wording.
#[test]
fn the_outcome_is_read_from_the_items() {
    let ok = |index: usize| MutationItem::<String> {
        index,
        value: None,
        error: None,
    };
    let bad = |index: usize| MutationItem::<String> {
        index,
        value: None,
        error: Some("boom".to_string()),
    };
    let result = |items: Vec<MutationItem<String>>| MutationResult {
        message: String::new(),
        items,
    };

    assert_eq!(result(vec![]).outcome(), ToolOutcome::Success);
    assert_eq!(result(vec![ok(0), ok(1)]).outcome(), ToolOutcome::Success);
    assert_eq!(result(vec![ok(0), bad(1)]).outcome(), ToolOutcome::Partial);
    assert_eq!(result(vec![bad(0), bad(1)]).outcome(), ToolOutcome::Failure);

    // The distinction the old string predicate kept getting wrong: a partial
    // success is not a failure, so it must not trip the rollback gate.
    assert_eq!(result(vec![ok(0), bad(1)]).ok_count(), 1);
}

/// A reply carries its verdict on the wire, so nothing downstream re-reads text.
#[test]
fn a_reply_states_its_own_verdict() {
    // Total failure: flagged at the source, by the code holding the items.
    let total = action_failed("Error: nope");
    assert_eq!(total.is_error, Some(true));
    assert_eq!(
        reply_outcome(total.structured_content.as_ref(), true),
        ToolOutcome::Failure
    );

    // Partial: an error *and* work that landed. Not flagged — a rollback here
    // would discard the items that succeeded.
    let mut items = Mutations::new();
    items.named("osc-1".to_string());
    items.failed("mth: no such module type".to_string());
    let partial = items.reply("modules added");
    assert_eq!(partial.is_error, Some(false));
    assert_eq!(
        reply_outcome(partial.structured_content.as_ref(), false),
        ToolOutcome::Partial
    );

    // A reader's payload has no per-item results, so it reads as success.
    let listing = serde_json::json!({"items": [{"id": 1}]});
    assert_eq!(reply_outcome(Some(&listing), false), ToolOutcome::Success);

    // And `is_error` always wins, whatever the payload looks like.
    assert_eq!(reply_outcome(Some(&listing), true), ToolOutcome::Failure);
}

/// The two tallied envelopes say so too.
///
/// `BatchResult` (11 tools — `add_note`, `update_note`, `set_parameter`, …) and
/// `BatchExecResult` both reach the client as `Json<T>`, and
/// `CallToolResult::structured` stamps `is_error: Some(false)` whatever they
/// contain. Without this the batch of ids that *all* failed reports a clean
/// success, does not stop a `stop_on_error` batch, and does not trip a rollback.
#[test]
fn a_tallied_envelope_states_its_verdict_too() {
    let batch = |succeeded: usize, failed: usize| {
        serde_json::to_value(crate::types::BatchResult {
            total: succeeded + failed,
            succeeded,
            failed,
            items: Vec::new(),
        })
        .expect("BatchResult serializes")
    };
    assert_eq!(
        reply_outcome(Some(&batch(0, 2)), false),
        ToolOutcome::Failure,
        "nothing landed"
    );
    assert_eq!(
        reply_outcome(Some(&batch(1, 1)), false),
        ToolOutcome::Partial,
        "a partial batch must not trip the rollback gate"
    );
    assert_eq!(
        reply_outcome(Some(&batch(2, 0)), false),
        ToolOutcome::Success
    );
    assert_eq!(
        reply_outcome(Some(&batch(0, 0)), false),
        ToolOutcome::Success
    );

    // The batch's own verdict is derived from its operations, so the fixture has
    // to carry them: counters that disagree with the entries are exactly what
    // deriving is meant to make impossible.
    let op = |index: usize, status: ToolOutcome| crate::types::BatchExecItemResult {
        index,
        tool: "set_song_name".to_string(),
        status,
        structured: None,
        message: None,
    };
    let exec = |statuses: Vec<ToolOutcome>| {
        let results: Vec<_> = statuses
            .into_iter()
            .enumerate()
            .map(|(index, status)| op(index, status))
            .collect();
        let tally = |wanted: ToolOutcome| results.iter().filter(|o| o.status == wanted).count();
        serde_json::to_value(crate::types::BatchExecResult {
            total: results.len(),
            succeeded: tally(ToolOutcome::Success),
            partial: tally(ToolOutcome::Partial),
            failed: tally(ToolOutcome::Failure),
            skipped: 0,
            dry_run: false,
            rolled_back: false,
            rollback_error: None,
            results,
        })
        .expect("BatchExecResult serializes")
    };
    use ToolOutcome::{Failure, Partial, Success};
    assert_eq!(
        reply_outcome(Some(&exec(vec![Failure, Failure])), false),
        ToolOutcome::Failure
    );
    assert_eq!(
        reply_outcome(Some(&exec(vec![Success, Failure])), false),
        ToolOutcome::Partial
    );
    assert_eq!(
        reply_outcome(Some(&exec(vec![Success, Success])), false),
        ToolOutcome::Success
    );
    // A batch whose every op half-applied is *not* a success: nothing it was
    // asked to do finished, and a stored `failed == 0` used to report it as one.
    assert_eq!(
        reply_outcome(Some(&exec(vec![Partial, Partial])), false),
        ToolOutcome::Failure
    );
    assert_eq!(
        reply_outcome(Some(&exec(vec![Success, Partial])), false),
        ToolOutcome::Partial
    );
}

/// A typed tool's own failure travels the same `Err` channel `dispatch_tools!`
/// uses for a rejection, so the log severity split depends on telling them
/// apart by the wording the macro and the panic guard own.
#[test]
fn a_dispatch_rejection_is_distinguished_from_a_tool_reported_failure() {
    assert!(is_dispatch_rejection("Error: unknown tool 'lst_tracks'"));
    assert!(is_dispatch_rejection(
        "Error: invalid params for 'list_notes': missing field `pattern_id`"
    ));
    assert!(is_dispatch_rejection(
        "Error: tool 'analyze_harmony' panicked: index out of bounds"
    ));
    // What a converted tool returns when the bridge refuses.
    assert!(!is_dispatch_rejection("Error: pattern not found: 7"));
    assert!(!is_dispatch_rejection("Error: invalid module type: nope"));
}
