//! Tests for `mcp_helper_tests`.

use super::*;
use std::assert_matches;
use synth_core::ModuleType;

fn ap(
    tick: u32,
    value: f32,
    curve: synth_sequencer::CurveType,
) -> synth_sequencer::AutomationPoint {
    synth_sequencer::AutomationPoint {
        tick: synth_sequencer::PatternTick(tick),
        value: synth_core::NormalizedValue::new(value),
        curve,
    }
}

#[test]
fn simplify_drops_collinear_linear_points() {
    use synth_sequencer::CurveType::Linear;
    // A perfectly straight ramp: every interior point is reproduced exactly.
    let pts = [
        ap(0, 0.0, Linear),
        ap(100, 0.25, Linear),
        ap(200, 0.5, Linear),
        ap(300, 0.75, Linear),
        ap(400, 1.0, Linear),
    ];
    let (kept, max_err) = simplify_automation_points(&pts, 0.01);
    assert_eq!(kept.len(), 2, "collinear interior removed: {kept:?}");
    assert_eq!(kept[0].tick.0, 0);
    assert_eq!(kept[1].tick.0, 400);
    assert!(max_err <= 0.01);
}

#[test]
fn simplify_preserves_step_points() {
    use synth_sequencer::CurveType::{Linear, Step};
    // Values are collinear (0, 0.5, 1.0) but the middle point is a Step edge,
    // so it must survive even though a linear fit would drop it.
    let pts = [ap(0, 0.0, Linear), ap(100, 0.5, Step), ap(200, 1.0, Linear)];
    let (kept, _) = simplify_automation_points(&pts, 0.1);
    assert_eq!(kept.len(), 3, "step point must be kept: {kept:?}");
    assert!(
        kept.iter()
            .any(|p| p.tick.0 == 100 && matches!(p.curve, Step))
    );
}

#[test]
fn simplify_respects_tolerance() {
    use synth_sequencer::CurveType::Linear;
    // Middle point sits 0.1 above the 0→1 line at its tick.
    let pts = [
        ap(0, 0.0, Linear),
        ap(100, 0.6, Linear),
        ap(200, 1.0, Linear),
    ];

    // Tight tolerance keeps the deviating point.
    let (kept_tight, _) = simplify_automation_points(&pts, 0.01);
    assert_eq!(kept_tight.len(), 3, "0.1 deviation exceeds 0.01 tol");

    // Loose tolerance drops it, and the reported error is within tolerance.
    let (kept_loose, max_err) = simplify_automation_points(&pts, 0.2);
    assert_eq!(kept_loose.len(), 2, "0.1 deviation within 0.2 tol");
    assert!(
        (0.09..=0.11).contains(&max_err),
        "reported max error ~0.1: {max_err}"
    );
}

#[test]
fn module_category_classifies_known_types() {
    assert_eq!(module_category(ModuleType::Oscillator), "voice");
    assert_eq!(module_category(ModuleType::Reverb), "effect");
    assert_eq!(module_category(ModuleType::Oscilloscope), "visualizer");
}

#[test]
fn densest_window_start_empty_returns_fallback() {
    assert_eq!(densest_window_start(&[], 30.0, 7.5), 7.5);
}

#[test]
fn densest_window_start_picks_the_busiest_cluster() {
    // Two onsets early, then a tight cluster of four near t=100. A 10 s
    // window should anchor on the cluster, not the sparse intro.
    let onsets = [0.0, 5.0, 100.0, 101.0, 102.0, 103.0];
    assert_eq!(densest_window_start(&onsets, 10.0, 0.0), 100.0);
}

#[test]
fn densest_window_start_window_is_half_open() {
    // The window is half-open: an onset exactly `window` after the start is
    // NOT counted (`<`, not `<=`). So [0,10) covers only {0.0}, while
    // [10,20) covers {10.0, 11.0} — the busier span wins at 10.0. (Under
    // closed-interval `<=` semantics the tie would instead resolve to 0.0.)
    let onsets = [0.0, 10.0, 11.0];
    assert_eq!(densest_window_start(&onsets, 10.0, 0.0), 10.0);
}

#[test]
fn masking_conflict_score_flags_identical_noise_floor_as_perfect() {
    // Two tracks parked at the renderer noise floor have identical, tiny
    // band energies, so the raw overlap/max ratio normalizes to ~1.0 — the
    // exact false positive MASKING_SILENCE_FLOOR_DBFS gates out before
    // scoring. This documents the failure mode the floor exists to prevent.
    let n = 2e-7_f32;
    let bands = masking_pair_bands(
        &synth_mcp::types::AnalyzeEnergyBands {
            sub: n,
            low: n,
            mid: n,
            high: n,
        },
        &synth_mcp::types::AnalyzeEnergyBands {
            sub: n,
            low: n,
            mid: n,
            high: n,
        },
    );
    assert!(
        masking_conflict_score(&bands) > 0.99,
        "identical noise-floor tracks score as a near-perfect conflict"
    );
}

#[test]
fn masking_silence_floor_excludes_noise_keeps_audible() {
    // The partition keeps a track when its soloed RMS is >= the floor. A
    // track at the renderer noise floor (~-85 dBFS) or reported silence
    // (-200.0) is dropped; the boundary (-60) and normal levels are kept.
    let audible = |rms_dbfs: f32| rms_dbfs >= MASKING_SILENCE_FLOOR_DBFS;
    for (rms, expected) in [
        (-200.0_f32, false),
        (-85.0, false),
        (-60.0, true),
        (-20.0, true),
    ] {
        assert_eq!(audible(rms), expected, "rms {rms} dBFS");
    }
}

#[test]
fn brief_catalog_entries_are_well_formed() {
    use crate::module_factory::ALL_MODULE_TYPES;

    assert!(!ALL_MODULE_TYPES.is_empty());
    for &mt in ALL_MODULE_TYPES.iter() {
        let key = mt.prefix();
        let cat = module_category(mt);
        assert!(!key.is_empty(), "{mt:?} has an empty type_key");
        assert!(!mt.name().is_empty(), "{mt:?} has an empty name");
        assert!(
            matches!(cat, "voice" | "effect" | "visualizer"),
            "{mt:?} has unexpected category {cat}"
        );
        // The gui_only flag mirrors is_visualizer, and today that is exactly
        // the "visualizer" category — keep the two in lockstep.
        assert_eq!(
            mt.is_visualizer(),
            cat == "visualizer",
            "{mt:?}: gui_only/category disagree"
        );
    }
}

#[test]
fn port_alias_table() {
    assert_eq!(port_alias("output"), Some("out"));
    assert_eq!(port_alias("input"), Some("in"));
    assert_eq!(port_alias("out"), None);
    assert_eq!(port_alias("cutoff_cv"), None);
}

#[test]
fn resolve_port_name_matches_exact_and_aliases() {
    let ports = vec![
        PortDescriptor::audio_output("out", "Out"),
        PortDescriptor::audio_input("in", "In"),
    ];
    // Exact match wins.
    assert_eq!(
        resolve_port_name(&ports, "out", PortDirection::Output)
            .unwrap()
            .as_str(),
        "out"
    );
    // output -> out / input -> in aliases resolve to the canonical name.
    assert_eq!(
        resolve_port_name(&ports, "output", PortDirection::Output)
            .unwrap()
            .as_str(),
        "out"
    );
    assert_eq!(
        resolve_port_name(&ports, "input", PortDirection::Input)
            .unwrap()
            .as_str(),
        "in"
    );
    // Right name, wrong direction is rejected (an output is not an input).
    assert!(resolve_port_name(&ports, "out", PortDirection::Input).is_err());
    // Unknown port fails and the error lists the available names.
    let err = resolve_port_name(&ports, "bogus", PortDirection::Output).unwrap_err();
    assert!(
        err.contains("out"),
        "error should list available ports: {err}"
    );
}

/// The embedded `.ptz` schema that `get_project_schema` ships is valid
/// JSON and a well-formed JSON Schema document (has `$schema`, an object
/// root, and `properties`). Guards against the tool returning a truncated or
/// corrupt artifact, and pins the bytes external tools diff against.
#[test]
fn embedded_project_schema_is_valid_and_well_formed() {
    let schema: serde_json::Value =
        serde_json::from_str(PROJECT_SCHEMA_JSON).expect("embedded schema must be valid JSON");
    assert!(
        schema.get("$schema").is_some(),
        "schema must carry a $schema dialect declaration"
    );
    assert_eq!(
        schema.get("type").and_then(|t| t.as_str()),
        Some("object"),
        "project schema root must be an object"
    );
    assert!(
        schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some_and(|p| !p.is_empty()),
        "schema must declare project properties"
    );
    // The build version paired with the schema is non-empty.
    assert!(!env!("CARGO_PKG_VERSION").is_empty());
}

/// The build-time env vars behind `get_version` are well-formed: the
/// timestamp is a full ISO 8601 / RFC 3339 UTC instant
/// ("YYYY-MM-DDTHH:MM:SSZ"), and the git fields are either absent together
/// or a 40-hex hash with a branch and a 0/1 dirty flag (dev builds always
/// run inside the repo, so expect the latter).
#[test]
fn version_info_build_env_is_well_formed() {
    let ts = env!("BUILD_TIMESTAMP");
    assert_eq!(ts.len(), "YYYY-MM-DDTHH:MM:SSZ".len(), "timestamp: {ts}");
    assert!(ts.ends_with('Z'), "timestamp: {ts}");

    let hash = env!("GIT_COMMIT_HASH");
    if hash.is_empty() {
        assert!(env!("GIT_BRANCH").is_empty());
        assert!(env!("GIT_DIRTY").is_empty());
    } else {
        assert_eq!(hash.len(), 40, "commit hash: {hash}");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!env!("GIT_BRANCH").is_empty());
        assert!(matches!(env!("GIT_DIRTY"), "0" | "1"));
    }
}

#[test]
fn add_note_applies_legato_and_relative_glide() {
    let mut pattern = synth_sequencer::Pattern::new(
        synth_sequencer::PatternId::new(0),
        synth_sequencer::Duration(3840),
    );
    let data = BridgeNoteData {
        pitch: MidiNote::C4,
        start_beat: 0.0,
        duration_beats: 1.0,
        velocity: 100,
        legato: true,
        glide: Some(BridgeGlide {
            from_semitones: Some(-12.0),
            from_pitch: None,
            time_ms: 80.0,
            stepped: true,
        }),
        expression: None,
    };
    let id = try_insert_note_into_pattern(&mut pattern, &data).unwrap();
    let note = pattern.note(NoteId::new(id)).unwrap();
    assert!(note.legato);
    let g = note.glide.expect("glide applied");
    assert_eq!(
        g.from,
        synth_sequencer::GlideFrom::Semitones(synth_core::Semitones::new(-12.0))
    );
    assert!((g.time.as_f32() - 80.0).abs() < f32::EPSILON);
    assert_eq!(g.interp, synth_sequencer::GlideInterp::Stepped);
}

#[test]
fn add_note_absolute_glide_pitch_takes_precedence() {
    let mut pattern = synth_sequencer::Pattern::new(
        synth_sequencer::PatternId::new(0),
        synth_sequencer::Duration(3840),
    );
    let data = BridgeNoteData {
        pitch: MidiNote::new(67),
        start_beat: 0.0,
        duration_beats: 1.0,
        velocity: 100,
        legato: false,
        glide: Some(BridgeGlide {
            from_semitones: Some(-2.0), // ignored when from_pitch is set
            from_pitch: Some(60),
            time_ms: 100.0,
            stepped: false,
        }),
        expression: None,
    };
    let id = try_insert_note_into_pattern(&mut pattern, &data).unwrap();
    let note = pattern.note(NoteId::new(id)).unwrap();
    assert!(!note.legato);
    let g = note.glide.expect("glide applied");
    assert_eq!(
        g.from,
        synth_sequencer::GlideFrom::Pitch(synth_sequencer::Pitch::new(60).unwrap())
    );
    assert_eq!(g.interp, synth_sequencer::GlideInterp::Continuous);
}

#[test]
fn add_note_applies_expression_block() {
    let mut pattern = synth_sequencer::Pattern::new(
        synth_sequencer::PatternId::new(0),
        synth_sequencer::Duration(3840),
    );
    let data = BridgeNoteData {
        pitch: MidiNote::C4,
        start_beat: 0.0,
        duration_beats: 1.0,
        velocity: 100,
        legato: false,
        glide: None,
        expression: Some(BridgeExpression {
            accent: Some(1.5),
            gate: Some(0.5),
            ghost: true,
            probability: Some(0.8),
            vibrato: Some(synth_mcp::bridge::BridgeVibrato {
                depth: 0.4,
                rate: 6.0,
                delay_ms: 20.0,
                shape: Some("triangle".to_owned()),
            }),
        }),
    };
    let id = try_insert_note_into_pattern(&mut pattern, &data).unwrap();
    let note = pattern.note(NoteId::new(id)).unwrap();
    let e = note.expression.expect("expression applied");
    assert_eq!(e.accent, Some(1.5));
    assert!((e.gate.unwrap().as_f32() - 0.5).abs() < f32::EPSILON);
    assert!(e.ghost);
    assert!((e.probability.unwrap().as_f32() - 0.8).abs() < f32::EPSILON);
    let v = e.vibrato.expect("vibrato applied");
    assert!((v.depth.as_f32() - 0.4).abs() < f32::EPSILON);
    assert_eq!(v.shape, synth_sequencer::VibratoShape::Triangle);
}

#[test]
fn add_note_empty_expression_collapses_to_none() {
    let mut pattern = synth_sequencer::Pattern::new(
        synth_sequencer::PatternId::new(0),
        synth_sequencer::Duration(3840),
    );
    // An expression block with no fields set is semantically "no expression".
    let data = BridgeNoteData {
        pitch: MidiNote::C4,
        start_beat: 0.0,
        duration_beats: 1.0,
        velocity: 100,
        legato: false,
        glide: None,
        expression: Some(BridgeExpression::default()),
    };
    let id = try_insert_note_into_pattern(&mut pattern, &data).unwrap();
    assert!(pattern.note(NoteId::new(id)).unwrap().expression.is_none());
}

#[test]
fn parse_module_type_accepts_prefix_name_display_and_camel() {
    // Prefix (the type key list_module_types advertises).
    assert_eq!(parse_module_type("ldr"), Some(ModuleType::LadderFilter));
    // snake_case name.
    assert_eq!(
        parse_module_type("ladder_filter"),
        Some(ModuleType::LadderFilter)
    );
    // Display name (what list_module_types puts in `name`).
    assert_eq!(
        parse_module_type("Ladder Filter"),
        Some(ModuleType::LadderFilter)
    );
    // CamelCase + a display name that diverges from the identifier.
    assert_eq!(
        parse_module_type("LadderFilter"),
        Some(ModuleType::LadderFilter)
    );
    assert_eq!(parse_module_type("LPC Vocoder"), Some(ModuleType::Vocoder));
    assert_eq!(parse_module_type("vocoder"), Some(ModuleType::Vocoder));
    // Unknown.
    assert_eq!(parse_module_type("nonexistent"), None);
}

#[test]
fn curve_from_kind_maps_strength_only_for_exponential() {
    use synth_mcp::bridge::CurveKind;
    use synth_sequencer::{CurveStrength, CurveType};
    assert_eq!(
        curve_from_kind(
            CurveKind::Linear,
            Some(CurveStrength::new(40).unwrap_or_default()),
        ),
        CurveType::Linear
    );
    assert_eq!(curve_from_kind(CurveKind::Step, None), CurveType::Step);
    assert_eq!(
        curve_from_kind(
            CurveKind::SCurve,
            Some(CurveStrength::new(7).unwrap_or_default()),
        ),
        CurveType::SCurve
    );
    let negative = CurveStrength::new(-30).unwrap_or_default();
    assert_eq!(
        curve_from_kind(CurveKind::Exponential, Some(negative)),
        CurveType::Exponential(negative)
    );
    // Missing strength defaults to 0.
    assert_eq!(
        curve_from_kind(CurveKind::Exponential, None),
        CurveType::Exponential(CurveStrength::ZERO)
    );
}

/// Returns the Filter descriptor and the type_id of its first choice param.
fn filter_choice_param() -> (synth_core::ParameterDescriptor, String) {
    let desc =
        crate::module_factory::get_descriptor(ModuleType::Filter).expect("filter descriptor");
    let choice = desc
        .parameters
        .iter()
        .find(|p| p.choices.is_some())
        .expect("filter has a choice param")
        .clone();
    (choice.clone(), choice.type_id.clone())
}

#[test]
fn resolve_param_value_numbers_and_bools_pass_through() {
    use synth_mcp::bridge::BridgeParamValue;
    assert_eq!(
        resolve_param_value(&BridgeParamValue::Number(440.0), None, "frequency").unwrap(),
        440.0
    );
    assert_eq!(
        resolve_param_value(&BridgeParamValue::Bool(true), None, "on").unwrap(),
        1.0
    );
    assert_eq!(
        resolve_param_value(&BridgeParamValue::Bool(false), None, "on").unwrap(),
        0.0
    );
}

#[test]
fn resolve_param_value_resolves_choice_by_id_and_name() {
    use synth_mcp::bridge::BridgeParamValue;
    let (pd, _) = filter_choice_param();
    let choices = pd.choices.as_ref().unwrap();
    let first = &choices[0];
    // Match by id (case-insensitive) → index 0.
    let by_id = resolve_param_value(
        &BridgeParamValue::Choice(first.id.to_uppercase()),
        Some(&pd),
        &pd.name,
    )
    .unwrap();
    assert_eq!(by_id, 0.0);
    // Match by display name → index 0.
    let by_name = resolve_param_value(
        &BridgeParamValue::Choice(first.name.clone()),
        Some(&pd),
        &pd.name,
    )
    .unwrap();
    assert_eq!(by_name, 0.0);
}

#[test]
fn resolve_param_value_rejects_unknown_choice() {
    use synth_mcp::bridge::BridgeParamValue;
    let (pd, _) = filter_choice_param();
    let err = resolve_param_value(
        &BridgeParamValue::Choice("definitely_not_a_choice".to_string()),
        Some(&pd),
        &pd.name,
    )
    .unwrap_err();
    assert_matches!(err, McpBridgeError::InvalidChoice { .. });
}

#[test]
fn resolve_param_value_rejects_string_for_numeric_param() {
    use synth_mcp::bridge::BridgeParamValue;
    let desc =
        crate::module_factory::get_descriptor(ModuleType::Filter).expect("filter descriptor");
    let numeric = desc
        .parameters
        .iter()
        .find(|p| p.choices.is_none())
        .expect("filter has a numeric param");
    let err = resolve_param_value(
        &BridgeParamValue::Choice("sawtooth".to_string()),
        Some(numeric),
        &numeric.name,
    )
    .unwrap_err();
    assert_matches!(err, McpBridgeError::InvalidChoice { .. });
}
