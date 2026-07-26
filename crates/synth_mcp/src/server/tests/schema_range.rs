//! Tests for `schema_range_tests`.

use super::*;

/// `#[schemars(range(...))]` on fixed-range numeric MCP fields must surface
/// machine-readable `minimum`/`maximum` in the generated JSON schema, not
/// just prose bounds in the description.
#[test]
fn fixed_range_fields_expose_min_max_in_schema() {
    // note/velocity: u8 with range(0, 127); channel: Option<u8> range(1, 16).
    let schema = serde_json::to_value(schemars::schema_for!(NoteOnInput))
        .expect("NoteOnInput schema serializes");
    let props = &schema["properties"];

    let midi_note = &schema["$defs"]["MidiNote"];
    assert_eq!(midi_note["maximum"], serde_json::json!(127), "note max");
    assert_eq!(midi_note["minimum"], serde_json::json!(0), "note min");
    assert_eq!(
        props["velocity"]["maximum"],
        serde_json::json!(127),
        "velocity max"
    );
    assert_eq!(
        props["velocity"]["minimum"],
        serde_json::json!(0),
        "velocity min"
    );
    let midi_channel = &schema["$defs"]["MidiChannel"];
    assert_eq!(
        midi_channel["maximum"],
        serde_json::json!(16),
        "channel max"
    );
    assert_eq!(midi_channel["minimum"], serde_json::json!(1), "channel min");
}

#[test]
fn batch_item_schemas_are_concrete() {
    fn schema_text<T: schemars::JsonSchema>() -> String {
        serde_json::to_string(&schemars::schema_for!(T)).unwrap()
    }

    let pattern = schema_text::<CreatePatternsParam>();
    assert!(pattern.contains("length_beats") && pattern.contains("start_beat"));
    let notes = schema_text::<AddNotesParam>();
    assert!(notes.contains("pitch") && notes.contains("duration_beats"));
    let tracks = schema_text::<CreateTracksParam>();
    assert!(tracks.contains("instrument_id"));
    let placements = schema_text::<PlacePatternsParam>();
    assert!(placements.contains("pattern_id") && placements.contains("track_id"));

    let note_modules = schema_text::<AddNoteGraphModuleParam>();
    assert!(
        note_modules.contains("graph_id")
            && note_modules.contains("ProbabilityGate")
            && note_modules.contains("NoteDelay")
    );
    let note_connections = schema_text::<ConnectNoteGraphParam>();
    assert!(note_connections.contains("from") && note_connections.contains("to_input"));
    let mod_nodes = schema_text::<AddModGraphNodeParam>();
    assert!(
        mod_nodes.contains("graph_id")
            && mod_nodes.contains("Macro")
            && mod_nodes.contains("Target")
    );
    let mod_connections = schema_text::<ConnectModGraphParam>();
    assert!(mod_connections.contains("from_port") && mod_connections.contains("to_port"));
}

#[test]
fn inline_schema_refs_makes_array_item_schemas_concrete() {
    // Precondition: the raw schema references its array item type via a
    // `$ref` into `$defs` — the shape that renders as `Array<unknown>` in
    // clients that don't resolve `$ref`.
    let raw = serde_json::to_value(schemars::schema_for!(ClearAutomationLaneParam))
        .expect("schema serializes");
    let raw_obj = raw.as_object().expect("schema is an object").clone();
    assert!(
        serde_json::to_string(&raw_obj).unwrap().contains("$ref"),
        "precondition: schemars emits $ref for nested item types"
    );

    // After inlining, no `$ref`/`$defs` remain and the array item schema is
    // a concrete object exposing the required fields.
    let inlined = serde_json::Value::Object(inline_schema_refs(&raw_obj));
    let text = serde_json::to_string(&inlined).unwrap();
    assert!(!text.contains("$ref"), "all refs inlined: {text}");
    assert!(!text.contains("$defs"), "defs dropped once inlined: {text}");

    let item = &inlined["properties"]["items"]["items"];
    assert_eq!(item["type"], "object", "array item is a concrete object");
    let props = &item["properties"];
    assert!(props.get("pattern_id").is_some(), "pattern_id visible");
    assert!(
        props.get("instrument_id").is_some(),
        "instrument_id visible"
    );
    assert!(props.get("target").is_some(), "target visible");
}

#[test]
fn inline_schema_refs_is_a_noop_without_defs() {
    // A schema with no `$defs` (all-primitive fields) is returned unchanged.
    let raw =
        serde_json::to_value(schemars::schema_for!(ProjectPathParam)).expect("schema serializes");
    let obj = raw.as_object().expect("object").clone();
    assert!(!serde_json::to_string(&obj).unwrap().contains("$defs"));
    assert_eq!(inline_schema_refs(&obj), obj);
}
