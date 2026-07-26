//! Tests for `automation_target_input_tests`.

use super::*;

#[test]
fn module_target_renders_canonical_dsl_string() {
    let t = AutomationTargetInput::Module {
        module_type: "flt".to_string(),
        instance: 1,
        param_id: "cutoff".into(),
    };
    assert_eq!(t.to_target_string(), "module:flt:1:cutoff");
}

#[test]
fn instrument_target_renders_macro_name() {
    let t = AutomationTargetInput::Instrument {
        param: "FilterCutoff".to_string(),
    };
    assert_eq!(t.to_target_string(), "FilterCutoff");
}

fn point(param: Option<&str>, target: Option<AutomationTargetInput>) -> AutomationPointInput {
    AutomationPointInput {
        param: param.map(str::to_string),
        target,
        instrument_id: None,
        beat: 0.0,
        value: 0.5,
        curve: None,
        curve_strength: None,
    }
}

#[test]
fn effective_target_prefers_structured_over_string() {
    let p = point(
        Some("Volume"),
        Some(AutomationTargetInput::Module {
            module_type: "flt".to_string(),
            instance: 2,
            param_id: "resonance".into(),
        }),
    );
    assert_eq!(p.effective_target(), "module:flt:2:resonance");
}

#[test]
fn effective_target_falls_back_to_param_string() {
    assert_eq!(point(Some("Pan"), None).effective_target(), "Pan");
    assert_eq!(point(None, None).effective_target(), "");
}

#[test]
fn validate_automation_point_requires_a_target() {
    assert!(validate_automation_point(&point(None, None)).is_err());
    assert!(validate_automation_point(&point(Some("Volume"), None)).is_ok());
}
