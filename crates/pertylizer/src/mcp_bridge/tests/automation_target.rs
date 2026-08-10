//! Tests for `automation_target_tests`.

use super::*;
use std::assert_matches;
use synth_engine::ModuleId;
use synth_sequencer::{AutomationTarget, InstrumentId};

/// A module set with a single Filter at instance 1 (the common test graph).
fn flt1() -> Vec<ModuleId> {
    vec![ModuleId::new(synth_core::ModuleType::Filter, 1)]
}

#[test]
fn build_module_target_parses_and_validates() {
    let t = build_automation_target("module:flt:1:cutoff", InstrumentId::new(3), &flt1()).unwrap();
    assert_eq!(
        t,
        AutomationTarget::Module {
            instrument: InstrumentId::new(3),
            module_type: synth_core::ModuleType::Filter,
            instance: 1,
            param_id: "cutoff".into(),
        }
    );
}

#[test]
fn build_module_target_accepts_dash_and_full_name_forms() {
    let expected = AutomationTarget::Module {
        instrument: InstrumentId::new(3),
        module_type: synth_core::ModuleType::Filter,
        instance: 1,
        param_id: "cutoff".into(),
    };
    // Dash form mirrors the ModuleId rendering used by every other tool.
    assert_eq!(
        build_automation_target("module:flt-1:cutoff", InstrumentId::new(3), &flt1()).unwrap(),
        expected
    );
    // Full type name (snake_case) instead of the 3-letter prefix.
    assert_eq!(
        build_automation_target("module:filter:1:cutoff", InstrumentId::new(3), &flt1()).unwrap(),
        expected
    );
    // Full name + dash form together.
    assert_eq!(
        build_automation_target("module:filter-1:cutoff", InstrumentId::new(3), &flt1()).unwrap(),
        expected
    );
}

#[test]
fn build_module_target_handles_hyphenated_multiword_type_name() {
    // A hyphenated multi-word type name ("ladder-filter" — which
    // parse_module_type accepts) must not be mis-split on its internal '-';
    // the instance is the trailing token. Both colon and dash instance
    // separators must resolve to the same target.
    let ldr = vec![ModuleId::new(synth_core::ModuleType::LadderFilter, 1)];
    let expected = AutomationTarget::Module {
        instrument: InstrumentId::new(3),
        module_type: synth_core::ModuleType::LadderFilter,
        instance: 1,
        param_id: "cutoff".into(),
    };
    assert_eq!(
        build_automation_target("module:ladder-filter:1:cutoff", InstrumentId::new(3), &ldr)
            .unwrap(),
        expected
    );
    assert_eq!(
        build_automation_target("module:ladder-filter-1:cutoff", InstrumentId::new(3), &ldr)
            .unwrap(),
        expected
    );
}

#[test]
fn build_instrument_target_still_works() {
    // Instrument-level params ignore the module set.
    let t = build_automation_target("FilterCutoff", InstrumentId::new(2), &[]).unwrap();
    assert_matches!(t, AutomationTarget::Instrument { .. });
}

#[test]
fn build_module_target_rejects_non_automatable_and_invalid() {
    let m = flt1();
    // "type" is the FilterMode choice param — excluded from the allowlist.
    assert!(build_automation_target("module:flt:1:type", InstrumentId::new(0), &m).is_err());
    // Unknown param / module type / instance-string / arity.
    assert!(build_automation_target("module:flt:1:nope", InstrumentId::new(0), &m).is_err());
    assert!(build_automation_target("module:zzz:1:cutoff", InstrumentId::new(0), &m).is_err());
    assert!(build_automation_target("module:flt:x:cutoff", InstrumentId::new(0), &m).is_err());
    assert!(build_automation_target("module:flt:1", InstrumentId::new(0), &m).is_err());
}

/// The rejecting lookup is holding the descriptor that lists every name it
/// would have accepted, so a near miss costs nothing to report — and saves the
/// caller a `get_module_type_info` round trip.
#[test]
fn an_unknown_param_names_the_one_that_was_meant() {
    let m = flt1();
    let err = build_automation_target("module:flt:1:cutof", InstrumentId::new(0), &m)
        .expect_err("'cutof' is not a filter parameter");
    let msg = err.to_string();
    assert!(
        msg.contains("'cutoff'"),
        "expected the near miss to be named; got: {msg}"
    );
}

/// A wrong suggestion is worse than none: a parameter name with no relation to
/// anything on the module must be rejected without a guess.
#[test]
fn an_unrecognizable_param_suggests_nothing() {
    let m = flt1();
    let err = build_automation_target("module:flt:1:nope", InstrumentId::new(0), &m)
        .expect_err("'nope' is not a filter parameter");
    let msg = err.to_string();
    assert!(
        !msg.contains("Did you mean"),
        "expected no suggestion; got: {msg}"
    );
}

/// The DSL's parser, its near-miss hint and `automation_target_info`'s renderer
/// are three views of one name list per lane kind. A variant that renders to a
/// string the parser rejects is a lane `list_automation_lanes` reports and no
/// other automation tool can address — the exact drift the shared name lists
/// exist to prevent — so pin the round trip over *every* variant rather than
/// over the handful a hand-written test would name.
#[test]
fn every_lane_name_round_trips_through_the_dsl() {
    use synth_sequencer::{AutoInstrumentParam, GlobalParam, TrackParam};

    let instrument = InstrumentId::new(0);
    let mut targets: Vec<AutomationTarget> = Vec::new();
    targets.extend(
        AutoInstrumentParam::ALL
            .iter()
            .map(|&param| AutomationTarget::Instrument { instrument, param }),
    );
    targets.extend(
        TrackParam::ALL
            .iter()
            .map(|&param| AutomationTarget::Track { track: None, param }),
    );
    targets.push(AutomationTarget::Global(GlobalParam::MasterVolume));

    for target in targets {
        let (rendered, _, _) = automation_target_info(&target);
        assert_eq!(
            build_automation_target(&rendered, instrument, &[]).ok(),
            Some(target.clone()),
            "{target:?} renders to {rendered:?}, which the DSL parser rejects"
        );
    }
}

/// The DSL's param names are case-sensitive, so a case slip is the likeliest
/// way to get one wrong — and the one a hint recovers most cheaply, since the
/// ranking is case-insensitive.
#[test]
fn an_unknown_dsl_param_names_the_one_that_was_meant() {
    for (target, expected) in [
        ("track:volume", "'Volume'"),
        ("track:Volme", "'Volume'"),
        ("global:mastervolume", "'MasterVolume'"),
        ("filtercutoff", "'FilterCutoff'"),
    ] {
        let err = build_automation_target(target, InstrumentId::new(0), &flt1())
            .expect_err("the DSL is case-sensitive, so this must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains(expected),
            "target {target} should suggest {expected}; got: {msg}"
        );
    }
}

/// An unknown module type in a `module:` target names the type that was meant,
/// keyed the way the DSL wants it written.
#[test]
fn an_unknown_dsl_module_type_names_the_key() {
    let err = build_automation_target("module:lim:1:threshold", InstrumentId::new(0), &flt1())
        .expect_err("'lim' is not a module type key");
    let msg = err.to_string();
    assert!(
        msg.contains("'lmt' (Limiter)"),
        "expected the near miss to be named; got: {msg}"
    );
}

/// Nonsense stays nonsense — no guess.
#[test]
fn an_unrecognizable_dsl_token_suggests_nothing() {
    for target in ["track:zzz", "global:zzz", "zzz"] {
        let err = build_automation_target(target, InstrumentId::new(0), &flt1())
            .expect_err("'zzz' is not a param");
        let msg = err.to_string();
        assert!(
            !msg.contains("Did you mean"),
            "target {target} must not guess; got: {msg}"
        );
    }
}

#[test]
fn build_module_target_rejects_missing_instance() {
    // Instrument has flt-4 / flt-5 but no flt-1: a flt:1 target must be
    // rejected rather than creating a silently-dead lane.
    let modules = vec![
        ModuleId::new(synth_core::ModuleType::Filter, 4),
        ModuleId::new(synth_core::ModuleType::Filter, 5),
    ];
    let err = build_automation_target("module:flt:1:cutoff", InstrumentId::new(1), &modules)
        .expect_err("a flt-1 target must be rejected when the instrument has no flt-1");
    // The error must name the instrument and point at instrument_id — the
    // module often exists on *another* instrument and instrument_id defaults
    // to 0, so a bare "no such module" sent callers chasing a phantom bug.
    let msg = err.to_string();
    assert!(
        msg.contains("instrument 1") && msg.contains("instrument_id"),
        "missing-instance error must name the instrument and mention instrument_id; got: {msg}"
    );
    // The instances that do exist are accepted.
    assert!(build_automation_target("module:flt:4:cutoff", InstrumentId::new(1), &modules).is_ok());
}

#[test]
fn module_target_info_round_trips_through_build() {
    let target = AutomationTarget::Module {
        instrument: InstrumentId::new(5),
        module_type: synth_core::ModuleType::Filter,
        instance: 2,
        param_id: "resonance".into(),
    };
    let (name, inst, scope) = automation_target_info(&target);
    assert_eq!(inst, Some(InstrumentId::new(5)));
    assert_eq!(scope, "module");
    let modules = vec![ModuleId::new(synth_core::ModuleType::Filter, 2)];
    let rebuilt = build_automation_target(&name, inst.unwrap(), &modules).unwrap();
    assert_eq!(rebuilt, target);
}

#[test]
fn track_and_global_targets_round_trip_through_build() {
    use synth_sequencer::{GlobalParam, TrackId, TrackParam};
    let cases = [
        AutomationTarget::Track {
            track: None,
            param: TrackParam::Pitch,
        },
        AutomationTarget::Track {
            track: Some(TrackId(3)),
            param: TrackParam::Volume,
        },
        AutomationTarget::Global(GlobalParam::MasterVolume),
    ];
    let expected_scope = ["track", "track", "global"];
    for (target, want_scope) in cases.iter().zip(expected_scope) {
        let (name, inst, scope) = automation_target_info(target);
        assert_eq!(scope, want_scope, "scope for {name}");
        // instrument_id is meaningless for track/global; build ignores it.
        let rebuilt = build_automation_target(&name, inst.unwrap_or_default(), &[]).unwrap();
        assert_eq!(&rebuilt, target, "round trip for {name}");
    }
}
