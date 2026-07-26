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

#[test]
fn result_is_failure_flags_prose_and_json_total_failure() {
    // Prose leaders.
    assert!(result_is_failure("Error: nope"));
    assert!(result_is_failure(&batch_msg(
        0,
        "x",
        &[],
        &["e".to_string()]
    )));
    assert!(!result_is_failure("OK: 2 x; 1 failed: e"));
    assert!(!result_is_failure("OK: 3 x"));

    // batch_json: total failure (no successes) is a failure; partial/full is not.
    let total_fail = batch_json("created", &Vec::<u64>::new(), &["boom".to_string()]);
    assert!(result_is_failure(&total_fail), "got: {total_fail}");
    let partial = batch_json("created", &[1_u64], &["boom".to_string()]);
    assert!(
        !result_is_failure(&partial),
        "partial success is not a failure"
    );
    let full = batch_json("created", &[1_u64, 2], &[]);
    assert!(!result_is_failure(&full), "full success is not a failure");

    // A non-batch JSON blob with an empty errors list is not a failure.
    assert!(!result_is_failure(r#"{"created":[],"errors":[]}"#));

    // Bridge BatchResult: only a total failure trips rollback. Partial
    // success remains a successful tool call with per-item errors.
    assert!(result_is_failure(
        r#"{"total":2,"succeeded":0,"failed":2,"items":[]}"#
    ));
    assert!(!result_is_failure(
        r#"{"total":2,"succeeded":1,"failed":1,"items":[]}"#
    ));
}
