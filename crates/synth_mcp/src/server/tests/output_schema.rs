//! Tests for typed tool output (TODO 6.7).
//!
//! A tool returning `Result<Json<T>, String>` gets its `outputSchema` from the
//! `#[tool]` macro, which infers it by *pattern-matching the return type's
//! syntax*. Nothing checks that the inference succeeded: write the same return
//! type behind an alias and the macro quietly finds no `Json<…>`, publishes no
//! `outputSchema`, and the tool still compiles and still answers with
//! `structuredContent` — a schema silently missing from the catalog. These pin
//! the ones that are converted so that regression is a test failure.
//!
//! Batch reachability is *not* testable from here — `dispatch_tool_inner` needs
//! a live bridge, which lives in the `pertylizer` crate. The guard for a tool
//! dropped from the `dispatch_tools!` table is
//! `pertylizer/tests/mcp_batch_dispatch_coverage.rs`.

use super::*;

fn all_tools() -> Vec<rmcp::model::Tool> {
    SynthMcpServer::build_router(&[]).list_all()
}

/// Tools that deliberately answer prose and publish **no** `outputSchema`.
///
/// The list is the inverse of the one that used to live here — every converted
/// tool by name, which had to be edited on every conversion and whose whole
/// purpose was to notice when someone forgot to edit it. Listing the exceptions
/// instead makes the default correct: a new or converted tool needs no entry, and
/// a tool that answers prose needs a deliberate one with its reason recorded at
/// the handler.
const PROSE_TOOLS: &[&str] = &[
    // Tools whose entire result *is* one document. `Json<T>` fills `content` and
    // `structuredContent` from the same value, so typing one of these ships the
    // document twice for no addressability — 40 KB twice for the YAMS reference,
    // and a measured 258 KB + 276 KB for the project schema. Reasons are recorded
    // at each handler.
    "get_yams_reference",
    "get_project_schema",
];

/// Every tool in the catalog either publishes an `outputSchema` or is a named,
/// reasoned exception — nothing falls between the two by omission.
///
/// This is the guard the old by-name list was trying to be. Its failure mode was
/// silence in the other direction: convert a tool, forget the list, and every
/// check in this file skipped it — the `$ref`-inlining and
/// `required`-vs-`skip_serializing_if` guards included. Anchored to the router,
/// a tool can only be missing a schema on purpose.
#[test]
fn every_tool_either_publishes_a_schema_or_is_listed_as_prose() {
    let undeclared: Vec<String> = all_tools()
        .iter()
        .filter(|t| t.output_schema.is_none())
        .map(|t| t.name.to_string())
        .filter(|name| !PROSE_TOOLS.contains(&name.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "tools publishing no outputSchema and not listed in PROSE_TOOLS \
         (macro inference lost, or a conversion still owed?): {undeclared:?}"
    );
}

/// And the exception list does not outlive its exceptions: a name left here after
/// its tool was converted would silently exempt that tool from every check below.
#[test]
fn the_prose_exceptions_are_all_real() {
    let tools = all_tools();
    for name in PROSE_TOOLS {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("{name} is listed as prose but not registered at all"));
        assert!(
            tool.output_schema.is_none(),
            "{name} publishes an outputSchema now — drop it from PROSE_TOOLS"
        );
    }
}

/// The tools this file actually checks: everything that publishes a schema.
fn schema_publishing_tools() -> Vec<(String, std::sync::Arc<JsonObject>)> {
    all_tools()
        .iter()
        .filter_map(|t| {
            t.output_schema
                .as_ref()
                .map(|s| (t.name.to_string(), std::sync::Arc::clone(s)))
        })
        .collect()
}

/// The published schema must actually describe something. An empty object would
/// satisfy "has a schema" while telling a client nothing.
#[test]
fn a_published_output_schema_describes_its_payload() {
    for (name, schema) in schema_publishing_tools() {
        let described = schema.contains_key("properties")
            || schema.contains_key("items")
            || schema.contains_key("$ref")
            || schema.contains_key("type");
        assert!(
            described,
            "{name}'s outputSchema describes nothing: {schema:?}"
        );
    }
}

/// **Every** payload roots at an object, lists included.
///
/// A bare `Vec<T>` would root at an array, which `structuredContent` permits
/// only under spec `2026-07-28`. rmcp negotiates down to whatever version a
/// client asks for and echoes it back, so answering an array over a
/// `2025-06-18` handshake breaks the agreement the server just made — and a
/// client validating against it rejects the whole result. `Listing<T>` wraps
/// them, and this holds the rule for the whole catalog rather than for the one
/// tool that happened to be checked.
#[test]
fn every_output_schema_roots_at_an_object() {
    let non_object: Vec<(String, String)> = schema_publishing_tools()
        .into_iter()
        .filter_map(|(name, schema)| {
            let root = schema
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            (root != "object").then_some((name, root))
        })
        .collect();
    assert!(
        non_object.is_empty(),
        "outputSchemas not rooted at an object: {non_object:?}"
    );
}

/// The wrapper is real rather than incidental: a list tool's schema is an
/// object whose `items` property is the array.
#[test]
fn a_list_returning_tool_wraps_its_array_in_an_object() {
    let tools = all_tools();
    let schema = tools
        .iter()
        .find(|t| t.name == "list_tracks")
        .and_then(|t| t.output_schema.clone())
        .expect("list_tracks publishes an outputSchema");
    assert_eq!(schema.get("type").and_then(|t| t.as_str()), Some("object"));
    assert_eq!(
        schema["properties"]["items"]["type"].as_str(),
        Some("array"),
        "the payload should be under `items`: {schema:?}"
    );
}

/// `build_router` inlines `$ref`s so a client that does not resolve them still
/// sees concrete shapes. Output schemas need it as badly as input ones — a
/// `Json<Vec<T>>` tool would otherwise publish `items: {"$ref": "#/$defs/T"}`,
/// i.e. `Array<unknown>` — so pin that the inlining covers them too.
///
/// A surviving `$ref` is only a bug when it dangles. `inline_schema_refs`
/// deliberately leaves one in place for a *reference cycle* (a recursive result
/// type would otherwise expand forever) and keeps `$defs` as the resolution
/// target in that case — so the assertion is "every `$ref` still resolves",
/// not "no `$ref` at all". Without that distinction the first recursive result
/// type fails this test for producing a correct schema.
#[test]
fn a_published_output_schema_has_its_refs_inlined() {
    /// Collect every `#/$defs/…` / `#/definitions/…` name still referenced.
    fn collect_refs(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    if k == "$ref"
                        && let Some(name) = v.as_str().and_then(|r| {
                            r.strip_prefix("#/$defs/")
                                .or_else(|| r.strip_prefix("#/definitions/"))
                        })
                    {
                        out.push(name.to_string());
                    }
                    collect_refs(v, out);
                }
            }
            serde_json::Value::Array(items) => {
                for v in items {
                    collect_refs(v, out);
                }
            }
            _ => {}
        }
    }

    let mut dangling = Vec::new();
    for (name, schema) in schema_publishing_tools() {
        let defs = schema
            .get("$defs")
            .or_else(|| schema.get("definitions"))
            .and_then(serde_json::Value::as_object);
        let mut refs = Vec::new();
        // Skip the `$defs` block itself: refs *inside* a retained definition
        // point at siblings that are present by construction.
        for (key, value) in schema.as_ref() {
            if key != "$defs" && key != "definitions" {
                collect_refs(value, &mut refs);
            }
        }
        for r in refs {
            if defs.is_none_or(|d| !d.contains_key(&r)) {
                dangling.push(format!("{name}: #/$defs/{r}"));
            }
        }
    }
    assert!(
        dangling.is_empty(),
        "outputSchemas carrying a $ref with no definition to resolve it: {dangling:?}"
    );
}

/// A field that serde skips when empty must not be `required` in the published
/// schema. schemars only drops a field from `required` when it sees a serde
/// `default`; `skip_serializing_if` alone leaves it required, and the response
/// then violates the very schema the tool advertises — which a client that
/// validates `structuredContent` rejects. `ParamTypeInfo::unit` is the case that
/// actually shipped broken (most parameters have no unit).
#[test]
fn a_skipped_when_empty_field_is_not_required() {
    let tools = all_tools();
    let schema = tools
        .iter()
        .find(|t| t.name == "get_module_type_info")
        .and_then(|t| t.output_schema.clone())
        .expect("get_module_type_info publishes an outputSchema");
    let required = schema
        .get("properties")
        .and_then(|p| p.get("parameters"))
        .and_then(|p| p.get("items"))
        .and_then(|i| i.get("required"))
        .and_then(serde_json::Value::as_array)
        .expect("the parameter item schema lists its required fields");
    for skipped in ["unit", "description"] {
        assert!(
            !required.iter().any(|r| r.as_str() == Some(skipped)),
            "'{skipped}' is skipped when empty but published as required: {required:?}"
        );
    }
}

/// A mutating tool answers with its prose *and* the values that prose was built
/// from. The prose half stays byte-identical to what `batch_msg` produces,
/// because that is what every existing caller reads.
#[test]
fn an_action_reply_carries_the_prose_and_its_parts() {
    let mut items = Mutations::new();
    items.named("osc-1".to_string());
    items.named("flt-1".to_string());
    items.failed("amp: no such module".to_string());
    let reply = items.reply("modules added");

    let prose = batch_msg(
        2,
        "modules added",
        &["osc-1".to_string(), "flt-1".to_string()],
        &["amp: no such module".to_string()],
    );
    assert_eq!(reply_text(&reply), prose, "the text must not change");
    assert!(
        prose.contains("(osc-1, flt-1)") && prose.contains("1 failed"),
        "sanity: the prose is the flattened form: {prose}"
    );

    let structured = reply
        .structured_content
        .as_ref()
        .expect("a structured half is attached");
    assert_eq!(structured["message"], serde_json::json!(prose));
    let entries = structured["items"].as_array().expect("per-item results");
    assert_eq!(entries.len(), 3, "one entry per requested item");
    assert_eq!(entries[0]["index"], serde_json::json!(0));
    assert_eq!(entries[0]["value"], serde_json::json!("osc-1"));
    assert!(entries[0].get("error").is_none());
    assert_eq!(entries[2]["index"], serde_json::json!(2));
    assert_eq!(
        entries[2]["error"],
        serde_json::json!("amp: no such module")
    );
    assert!(entries[2].get("value").is_none());
}

/// The point of the exercise, and the part a flat `{ok_count, details, errors}`
/// could not express: which *requested item* each result belongs to.
///
/// Three parallel lists could say "three worked and here are two complaints".
/// They could not say which two, because nothing tied a message to the item that
/// produced it — so a caller with five items and two failures still had to read
/// the sentence to find out where to look.
#[test]
fn a_failure_names_the_item_it_belongs_to() {
    let mut items = Mutations::new();
    items.named("osc-1".to_string()); // index 0
    items.failed("mth: no such module type".to_string()); // index 1
    items.named("amp-1".to_string()); // index 2
    let reply = items.reply("modules added");
    let structured = reply.structured_content.expect("structured half");
    let entries = structured["items"].as_array().expect("per-item results");

    let failed: Vec<u64> = entries
        .iter()
        .filter(|e| e.get("error").is_some())
        .filter_map(|e| e["index"].as_u64())
        .collect();
    assert_eq!(failed, vec![1], "the middle item is the one that failed");
    // And the ids are addressable rather than parsed out of "(osc-1, amp-1)".
    assert_eq!(entries[0]["value"], serde_json::json!("osc-1"));
    assert_eq!(entries[2]["value"], serde_json::json!("amp-1"));
}

/// A total failure still leads with `"Error:"` in its prose — callers and scripts
/// read that — but the flag no longer depends on it: `mutation_reply` stamps
/// `is_error` from the items themselves.
#[test]
fn a_total_failure_states_it_in_both_halves() {
    let mut items = Mutations::new();
    items.failed("a: nope".to_string());
    items.failed("b: nope".to_string());
    let reply = items.reply("modules added");

    let text = reply_text(&reply);
    assert!(text.starts_with("Error:"), "got: {text}");
    assert_eq!(
        reply.is_error,
        Some(true),
        "the verdict is stamped where the items are known, not recovered from prose"
    );
}

/// Every action tool answers against one shared schema, and it must describe the
/// two fields the reply actually carries.
#[test]
fn the_action_schema_describes_the_reply() {
    let schema = action_output_schema();
    let props = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("the action schema has properties");
    for field in ["message", "items"] {
        assert!(props.contains_key(field), "schema is missing {field}");
    }
    // `ok_count` is derived from `items`, deliberately not stored: a counter beside
    // the entries it counts is one more thing that can disagree with them.
    assert!(
        !props.contains_key("ok_count"),
        "ok_count must not come back as a field: {props:?}"
    );
}

/// And it stays *one* document across the 112 routes that publish it.
///
/// `MutationResult<T>` `$ref`s its per-item type into `$defs`, which is the
/// condition `build_router` rewrites-and-re-`Arc`s on. `action_output_schema`
/// therefore inlines its own refs; if that stops happening the `OnceLock` still
/// works and nothing fails — the router just quietly holds 112 copies again.
#[test]
fn the_action_schema_is_shared_not_copied_per_route() {
    let schema = action_output_schema();
    assert!(
        !schema_has_defs(&schema),
        "build_router would rewrite this per route: {schema:?}"
    );
    // Inlined, not merely absent: the item schema is concrete.
    let item = schema
        .get("properties")
        .and_then(|p| p.get("items"))
        .and_then(|i| i.get("items"))
        .and_then(serde_json::Value::as_object)
        .expect("the items array has a concrete item schema");
    assert!(
        !item.contains_key("$ref") && item.contains_key("properties"),
        "the per-item schema must be inlined, not a $ref: {item:?}"
    );

    let router_copies: Vec<_> = SynthMcpServer::build_router(&[])
        .list_all()
        .into_iter()
        .filter_map(|t| t.output_schema)
        .filter(|s| **s == *schema)
        .collect();
    assert!(
        router_copies.len() > 100,
        "expected the action tools to publish this schema, got {}",
        router_copies.len()
    );
    assert!(
        router_copies
            .iter()
            .all(|s| std::sync::Arc::ptr_eq(s, &schema)),
        "every action route must share the one allocation"
    );
}

/// A partial success is not a failure — the distinction the old string predicate
/// kept getting wrong, in three separate forms.
#[test]
fn a_partial_success_is_not_a_failure() {
    let mut items = Mutations::new();
    items.named("osc-1".to_string());
    items.named("flt-1".to_string());
    items.named("amp-1".to_string());
    items.failed("mth: no such module type".to_string());
    let partial = items.reply("modules added");

    assert_eq!(
        partial.is_error,
        Some(false),
        "a partial success must not be flagged, or a rollback batch would discard \
         the three modules that landed"
    );
    assert_eq!(
        reply_outcome(partial.structured_content.as_ref(), false),
        ToolOutcome::Partial
    );

    // Nothing landed: that one *is* a failure.
    let mut items = Mutations::new();
    items.failed("mth: no such module type".to_string());
    let total = items.reply("modules added");
    assert_eq!(total.is_error, Some(true));
    assert_eq!(
        reply_outcome(total.structured_content.as_ref(), true),
        ToolOutcome::Failure
    );
}
