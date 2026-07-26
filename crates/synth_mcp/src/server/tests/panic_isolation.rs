//! Tests for `panic_isolation_tests`.

use super::*;

#[tokio::test]
async fn passes_through_a_non_panicking_result() {
    let out = run_catching_panic(0, "test_tool", async { 42_u32 }).await;
    assert_eq!(out, Ok(42));
}

#[tokio::test]
async fn recovers_a_str_panic_into_an_err_with_the_message() {
    let out: Result<(), String> =
        run_catching_panic(0, "test_tool", async { panic!("boom") }).await;
    assert_eq!(out, Err("boom".to_string()));
}

#[tokio::test]
async fn recovers_a_string_panic_with_formatted_message() {
    let out: Result<(), String> =
        run_catching_panic(0, "test_tool", async { panic!("bad value {}", 7) }).await;
    assert_eq!(out, Err("bad value 7".to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn recovers_a_panic_from_inside_block_in_place() {
    // The real failure mode: a panic raised inside a synchronous
    // `block_in_place` closure must unwind into this poll and be caught,
    // not propagate up to the worker thread.
    let out: Result<(), String> = run_catching_panic(0, "test_tool", async {
        tokio::task::block_in_place(|| panic!("inside block_in_place"));
    })
    .await;
    assert_eq!(out, Err("inside block_in_place".to_string()));
}
