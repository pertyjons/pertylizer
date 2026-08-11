//! The published contract, checked over the real protocol (TODO §6.7).
//!
//! Every other MCP test calls `dispatch_tool_for_test`, which is the *batch*
//! path: it returns one string and never builds a `CallToolResult`. So nothing
//! checked the three things a client actually receives against each other —
//! the `outputSchema` in `tools/list`, the `structuredContent` in the reply, and
//! the `is_error` flag — even though §6.7 is entirely about that triple.
//!
//! This drives the shipped binary over stdio JSON-RPC, one session, and asserts
//! for one tool per reply family that the payload validates against the schema
//! its own catalog entry advertises. A green `cargo test` has been compatible
//! with an empty `tools/list` before (see the `--headless` probe recipe), which
//! is the other reason to talk to the real thing.

#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// One JSON-RPC exchange with a freshly-spawned headless server.
///
/// **One request at a time, each answered before the next is sent.** The server
/// handles calls concurrently, so writing the whole script up front and reading
/// afterwards would let a later call overtake an earlier one — and these tests
/// depend on order: they create a pattern and then address it by id. Waiting for
/// each reply is what makes the sequence a sequence.
fn talk(requests: &[Value]) -> Vec<Value> {
    talk_with_protocol("2025-06-18", requests)
}

/// [`talk`], negotiating a specific protocol revision — some fields exist only in
/// the newer ones.
fn talk_with_protocol(protocol_version: &str, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pertylizer"))
        .arg("--headless")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the headless server");

    // `take()`, not `as_mut()`: the session ends when the server sees EOF on
    // stdin, and that only happens once this handle is *dropped*. Borrowing it
    // leaves the pipe open inside `child`, and every spawned server hangs.
    let mut stdin = child.stdin.take().expect("server stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("server stdout"));
    let mut replies = Vec::with_capacity(requests.len());

    let read_reply = |stdout: &mut BufReader<_>, for_id: &Value| -> Option<Value> {
        // Skip anything that is not the answer to `for_id` — notifications, and
        // any reply that arrives out of order.
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).ok()? == 0 {
                return None;
            }
            if let Ok(message) = serde_json::from_str::<Value>(&line)
                && message.get("id") == Some(for_id)
            {
                return Some(message);
            }
        }
    };

    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":protocol_version,"capabilities":{},
            "clientInfo":{"name":"wire-contract","version":"1.0"}}})
    )
    .expect("write initialize");
    read_reply(&mut stdout, &json!(0)).expect("the server answers initialize");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .expect("write initialized");

    for request in requests {
        writeln!(stdin, "{request}").expect("write request");
        let id = request["id"].clone();
        match read_reply(&mut stdout, &id) {
            Some(reply) => replies.push(reply),
            None => break,
        }
    }

    drop(stdin);
    let _ = child.wait();
    replies
}

/// The reply to request `id`, or a panic naming what came back instead.
fn reply(replies: &[Value], id: u64) -> &Value {
    replies
        .iter()
        .find(|m| m["id"] == json!(id))
        .and_then(|m| m.get("result"))
        .unwrap_or_else(|| panic!("no result for id {id} in {replies:#?}"))
}

/// A tool's published `outputSchema`, or `None` when it publishes none.
fn output_schema(catalog: &Value, tool: &str) -> Option<Value> {
    catalog["tools"]
        .as_array()
        .expect("tools/list returns an array")
        .iter()
        .find(|t| t["name"] == json!(tool))
        .unwrap_or_else(|| panic!("{tool} is not in the catalog"))
        .get("outputSchema")
        .cloned()
}

/// Assert a reply's `structuredContent` satisfies the schema its tool publishes.
///
/// This is the check that only exists on the wire: `structuredContent` is
/// produced by rmcp's serialization of the handler's value, and `outputSchema`
/// by schemars' reflection over the same type, and nothing in between forces
/// them to agree. `ParamTypeInfo::unit` once shipped a payload that violated its
/// own advertised schema exactly this way.
fn assert_payload_matches_schema(catalog: &Value, tool: &str, result: &Value) {
    let schema =
        output_schema(catalog, tool).unwrap_or_else(|| panic!("{tool} publishes no outputSchema"));
    let payload = result
        .get("structuredContent")
        .unwrap_or_else(|| panic!("{tool} declares an outputSchema but sent no structuredContent"));
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("{tool}'s published outputSchema is not valid: {e}"));
    if let Err(error) = validator.validate(payload) {
        panic!("{tool}'s payload violates its own outputSchema: {error}\npayload: {payload}");
    }
}

/// The catalog says how long it may be cached, and only where the field exists.
///
/// `#[tool_handler]` started filling these in rmcp 3.1.2 as `ttlMs: 0` +
/// `cacheScope: public` — "immediately stale, anyone may serve it" — for the
/// largest response this server has. Both halves are wrong for a catalog fixed at
/// router construction, so `list_tools` is overridden; this pins that the override
/// is the one in effect, in both directions.
#[test]
fn the_catalog_advertises_a_cache_lifetime() {
    // A revision that has the fields.
    let modern = talk_with_protocol(
        "2026-07-28",
        &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})],
    );
    let catalog = reply(&modern, 1);
    assert_eq!(
        catalog["ttlMs"],
        json!(600_000),
        "a fixed catalog is worth keeping, and 0 would mean the opposite: {catalog:?}"
    );
    assert_eq!(
        catalog["cacheScope"],
        json!("private"),
        "the list is this instance's — `disabled_tools` can differ between processes"
    );

    // A revision that does not: the fields must be absent rather than defaulted.
    let older = talk(&[json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})]);
    let catalog = reply(&older, 1);
    assert!(
        catalog.get("ttlMs").is_none() && catalog.get("cacheScope").is_none(),
        "2025-06-18 has no cache hints to send: {catalog:?}"
    );
}

/// A batched op never carries its payload twice.
///
/// Readers were fixed once (`DispatchedOp::payload` sends no message), and the
/// five tools that publish a bespoke schema then reintroduced it: they moved to
/// the action arm so their verdict could travel, and that arm put
/// `to_json(payload)` in `message` beside the identical `structured`. Their
/// payloads are the largest of any mutator's, so it was the worst place to lose it.
#[test]
fn a_batched_payload_is_not_also_sent_as_text() {
    let replies = talk(&[json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"batch_execute","arguments":{"operations":[
            // A reader.
            {"tool":"list_instruments","params":{}},
            // A bespoke-schema tool whose payload says it is incomplete.
            {"tool":"build_instrument","params":{"instruments":[{
                "name":"ZZ wire",
                "modules":[{"module_type":"osc","params":{"nope":1}}]}]}},
            // A plain mutator, whose sentence is not its payload.
            {"tool":"set_song_name","params":{"name":"ZZ wire"}}]}}})]);

    let ops = reply(&replies, 1)["structuredContent"]["results"]
        .as_array()
        .expect("per-op results")
        .clone();

    for op in &ops[..2] {
        assert!(
            op["structured"].is_object(),
            "sanity: this op has a payload: {op}"
        );
        assert!(
            op.get("message").is_none(),
            "{} sent its payload as text as well: {op}",
            op["tool"]
        );
    }
    assert_eq!(ops[1]["status"], json!("partial"), "sanity: {}", ops[1]);

    // The mutator keeps its sentence: it summarises rather than restates.
    assert!(
        ops[2]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("OK:")),
        "a confirmation is not a duplicate payload: {}",
        ops[2]
    );
}

/// The harness can fail.
///
/// A schema validator that accepts everything passes every test in this file
/// while checking nothing, and the failure would look exactly like success — so
/// prove a real payload's schema rejects a wrong payload before trusting the
/// assertions that follow.
#[test]
fn the_schema_check_rejects_a_wrong_payload() {
    let replies = talk(&[json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})]);
    let schema = output_schema(reply(&replies, 1), "list_instruments")
        .expect("list_instruments publishes an outputSchema");
    let validator = jsonschema::validator_for(&schema).expect("a valid schema");

    assert!(
        validator.validate(&json!({"items": []})).is_ok(),
        "an empty listing is the real shape"
    );
    assert!(
        validator
            .validate(&json!({"items": "not an array"}))
            .is_err(),
        "the validator must actually reject a mistyped payload"
    );
    assert!(
        validator.validate(&json!({})).is_err(),
        "`items` is required, so its absence must fail"
    );
}

/// Each reply family, over the wire, against its own published schema.
#[test]
fn every_reply_family_matches_its_published_schema() {
    let replies = talk(&[
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
        // A reader whose payload is a list.
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"list_instruments","arguments":{}}}),
        // A mutator that succeeds, with values to name.
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
               "params":{"name":"create_instrument","arguments":{"names":["Pad","Bass"]}}}),
        // A mutator where every item fails: `Ok` payload, total failure.
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
               "params":{"name":"import_sample","arguments":{"samples":[
                   {"path":"/nonexistent/a.wav"},{"path":"/nonexistent/b.wav"}]}}}),
        // A document reader, which publishes no schema by design.
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call",
               "params":{"name":"get_yams_reference","arguments":{}}}),
        // The orchestrator, carrying other tools' payloads as data.
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call",
               "params":{"name":"batch_execute","arguments":{"operations":[
                   {"tool":"list_instruments","params":{}},
                   {"tool":"get_song_info","params":{}}]}}}),
    ]);

    let catalog = reply(&replies, 1);
    assert_eq!(
        catalog["tools"].as_array().map(Vec::len),
        Some(219),
        "the catalog itself must arrive — a compile-green server has shipped an \
         empty tools/list before"
    );

    for (id, tool) in [
        (2, "list_instruments"),
        (3, "create_instrument"),
        (4, "import_sample"),
        (6, "batch_execute"),
    ] {
        assert_payload_matches_schema(catalog, tool, reply(&replies, id));
    }

    // A document tool publishes no schema, so it must not send a structured half
    // either — the two halves of that decision have to agree.
    let document = reply(&replies, 5);
    assert!(
        output_schema(catalog, "get_yams_reference").is_none(),
        "get_yams_reference is a PROSE_TOOLS exception"
    );
    assert!(
        document.get("structuredContent").is_none(),
        "a tool with no outputSchema must not answer structuredContent"
    );
    assert!(
        !document["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the document itself still arrives as text"
    );
}

/// A *failure* reply must not violate the schema its tool publishes either.
///
/// The five tools that publish a bespoke `outputSchema` answered their error and
/// pre-flight paths with the shared mutation envelope — `{message, items}` — which
/// satisfies none of the fields those schemas require. A client that validates
/// `structuredContent` (this file does, on the success replies) would reject the
/// reply and lose the error message with it.
///
/// The rule that holds instead: a reply may omit `structuredContent`, but if it
/// carries one it must match what the catalog promised. The spec only constrains
/// the field when it is present.
#[test]
fn an_error_reply_never_contradicts_its_published_schema() {
    let replies = talk(&[
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
        // Bridge refusal: no such pattern.
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"freeze_pattern","arguments":{"pattern_id":9999}}}),
        // Pre-flight refusal: a placement pointing past the patterns array.
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
               "params":{"name":"set_song","arguments":{"name":"S","patterns":[],"tracks":[],
                   "placements":[{"pattern_index":0,"track_index":0,"start_beat":0}]}}}),
        // Validation refusal inside a bespoke-schema tool.
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
               "params":{"name":"build_instrument","arguments":{"instruments":[
                   {"name":"","modules":[]}]}}}),
    ]);

    let catalog = reply(&replies, 1);
    for (id, tool) in [
        (2, "freeze_pattern"),
        (3, "set_song"),
        (4, "build_instrument"),
    ] {
        let result = reply(&replies, id);
        assert_eq!(
            result["isError"],
            json!(true),
            "{tool} should have refused: {result}"
        );
        assert!(
            !result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .is_empty(),
            "{tool}'s reason must survive as text: {result}"
        );
        if result.get("structuredContent").is_some() {
            // Allowed — but then it has to be the payload the catalog promised.
            assert_payload_matches_schema(catalog, tool, result);
        }
    }
}

/// `is_error` reflects what the handler decided, including for a typed `Ok`.
///
/// The regression this pins: rmcp renders a `Result<Json<T>, String>` success
/// through `CallToolResult::structured`, which hardcodes `is_error: false`. A
/// mutation whose every item failed is exactly that shape, so trusting the flag
/// as rmcp set it reported a total failure as a success.
#[test]
fn a_total_failure_is_flagged_even_when_the_call_returned_ok() {
    let replies = talk(&[
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"import_sample","arguments":{"samples":[
                   {"path":"/nonexistent/a.wav"},{"path":"/nonexistent/b.wav"}]}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"create_instrument","arguments":{"names":["Pad"]}}}),
    ]);

    let failed = reply(&replies, 1);
    assert_eq!(
        failed["isError"],
        json!(true),
        "nothing was imported, so the call failed: {failed}"
    );
    let items = failed["structuredContent"]["items"]
        .as_array()
        .expect("per-item results");
    assert_eq!(items.len(), 2, "one entry per requested sample");
    for (position, item) in items.iter().enumerate() {
        assert_eq!(item["index"], json!(position), "items keep request order");
        assert!(
            item["error"].is_string(),
            "each failure names the item it belongs to: {item}"
        );
    }

    let ok = reply(&replies, 2);
    assert_eq!(ok["isError"], json!(false), "that one worked: {ok}");
    assert!(
        ok["structuredContent"]["items"][0]["value"]["id"].is_number(),
        "a created id is data, not something to parse out of the sentence: {ok}"
    );
}

/// A rolled-back batch is reported as a failure, because none of it stands.
///
/// The counters still show the operation that completed before the restore — it
/// did run — so a client reading `succeeded > 0` would conclude the batch worked.
/// The verdict has to come from the fact that the project was put back.
#[test]
fn a_rolled_back_batch_reports_failure() {
    let replies = talk(&[
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"create_pattern","arguments":{
                   "patterns":[{"name":"Keep","length_beats":4}]}}}),
        // One op completes, the next only half-applies: all-or-nothing means the
        // batch is restored, so nothing it did survives.
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"batch_execute","arguments":{"rollback":true,"operations":[
                   {"tool":"set_song_name","params":{"name":"Changed"}},
                   {"tool":"rename_pattern","params":{"items":[
                       {"pattern_id":0,"name":"Renamed"},
                       {"pattern_id":9999,"name":"No such pattern"}]}}]}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
               "params":{"name":"get_song_info","arguments":{}}}),
    ]);

    let batch = reply(&replies, 2);
    let report = &batch["structuredContent"];
    assert_eq!(report["results"][1]["status"], json!("partial"), "sanity");
    assert_eq!(
        report["rolled_back"],
        json!(true),
        "all-or-nothing was asked for"
    );
    assert_eq!(report["partial"], json!(1));
    assert_eq!(
        batch["isError"],
        json!(true),
        "a batch that was put back did not do what it was asked: {report}"
    );
    assert_eq!(
        reply(&replies, 3)["structuredContent"]["name"],
        json!("Untitled"),
        "and the restore really happened"
    );
}

/// A batched op carries its payload as data, and its own verdict.
#[test]
fn a_batch_op_carries_the_payload_a_direct_call_would() {
    let replies = talk(&[json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
               "params":{"name":"batch_execute","arguments":{"operations":[
                   {"tool":"list_instruments","params":{}},
                   {"tool":"import_sample","params":{"samples":[{"path":"/nonexistent/a.wav"}]}}]}}})]);

    let report = &reply(&replies, 1)["structuredContent"];
    let ops = report["results"].as_array().expect("per-op results");

    assert_eq!(ops[0]["status"], json!("success"));
    assert!(
        ops[0]["structured"]["items"].is_array(),
        "the reader's payload crosses the batch boundary as data, not as a JSON \
         string to parse again: {}",
        ops[0]
    );

    assert_eq!(
        ops[1]["status"],
        json!("failure"),
        "nothing was imported: {}",
        ops[1]
    );
    assert!(
        ops[1]["structured"]["items"][0]["error"].is_string(),
        "and a batched mutation keeps its per-item results: {}",
        ops[1]
    );
    assert_eq!(report["failed"], json!(1));
}
