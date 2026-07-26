//! Tests for `summarize_params_tests`.

use super::*;
use serde_json::json;

#[test]
fn batch_execute_reports_op_count() {
    let p = json!({ "operations": [ {"tool": "a"}, {"tool": "b"}, {"tool": "c"} ] });
    assert_eq!(summarize_value(&p), "3 ops");
}

#[test]
fn array_shaped_tool_reports_field_len() {
    let p = json!({ "instruments": [ {}, {} ] });
    assert_eq!(summarize_value(&p), "instruments=2");
}

#[test]
fn single_target_reports_scalar_fields() {
    // BTreeMap key order (default serde_json) is alphabetical.
    let p = json!({ "parameter": "cutoff", "value": 800 });
    assert_eq!(summarize_value(&p), "parameter=cutoff, value=800");
}

#[test]
fn caps_scalar_fields_at_three() {
    let p = json!({ "a": 1, "b": 2, "c": 3, "d": 4 });
    // Alphabetical order, first three only.
    assert_eq!(summarize_value(&p), "a=1, b=2, c=3");
}

#[test]
fn long_summary_is_truncated_with_ellipsis() {
    let long = "x".repeat(200);
    let p = json!({ "name": long });
    let out = summarize_value(&p);
    assert!(
        out.chars().count() <= 60,
        "got {} chars",
        out.chars().count()
    );
    assert!(out.ends_with('…'));
}

#[test]
fn non_object_falls_back_to_json() {
    let p = json!("hello");
    assert_eq!(summarize_value(&p), "\"hello\"");
}
