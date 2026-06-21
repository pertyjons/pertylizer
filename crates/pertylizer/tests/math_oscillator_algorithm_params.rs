//! Regression test for Gap 4: `get_module_type_info("mth")` must document what
//! the math oscillator's generic `param_a`/`param_b`/`param_c` knobs mean for
//! each algorithm, so an agent can select knob values without blind probing.

use synth_core::ModuleType;
use synth_mcp::types::ModuleTypeInfo;

/// `build_module_type_info` is private; reach the same data through the public
/// search entry point, which returns full `ModuleTypeInfo` records.
fn math_oscillator_info() -> ModuleTypeInfo {
    let result = pertylizer::mcp_bridge::search_module_types(None, None, None, Some("math"));
    result
        .modules
        .into_iter()
        .find(|m| m.type_key == ModuleType::MathOscillator.prefix())
        .expect("math oscillator should be discoverable by the `math` query")
}

#[test]
fn math_oscillator_exposes_algorithm_parameters() {
    let info = math_oscillator_info();
    let table = info
        .algorithm_parameters
        .expect("math oscillator must carry an algorithm_parameters table");

    // sine_fm: param_a is the FM index, param_b the ratio — the exact mapping
    // the plan calls out as previously requiring blind probing.
    let sine_fm = &table["sine_fm"];
    let a_name = sine_fm["param_a"]["name"].as_str().unwrap_or_default();
    let b_name = sine_fm["param_b"]["name"].as_str().unwrap_or_default();
    assert!(
        a_name.to_lowercase().contains("index"),
        "sine_fm param_a should be the modulation index, got {a_name:?}"
    );
    assert!(
        b_name.to_lowercase().contains("ratio"),
        "sine_fm param_b should be the modulation ratio, got {b_name:?}"
    );

    // Every algorithm the plan names must be documented with all three knobs.
    for algo in [
        "sine_fm",
        "metallic",
        "wave_folder",
        "feedback_fm",
        "phase_dist",
        "vosim",
    ] {
        let entry = table
            .get(algo)
            .unwrap_or_else(|| panic!("missing algorithm entry for {algo}"));
        for knob in ["param_a", "param_b", "param_c"] {
            let name = entry[knob]["name"].as_str().unwrap_or_default();
            let desc = entry[knob]["description"].as_str().unwrap_or_default();
            assert!(
                !name.is_empty() && !desc.is_empty(),
                "{algo}.{knob} must have a name and description"
            );
        }
    }
}

#[test]
fn ordinary_module_has_no_algorithm_parameters() {
    // A plain oscillator's knobs don't change role with an algorithm, so the
    // field must be absent (None) rather than an empty object.
    let result = pertylizer::mcp_bridge::search_module_types(None, None, None, Some("ring mod"));
    let ring = result
        .modules
        .iter()
        .find(|m| m.type_key == "rng")
        .expect("ring mod should be discoverable");
    assert!(
        ring.algorithm_parameters.is_none(),
        "non-math modules must not carry algorithm_parameters"
    );
}
