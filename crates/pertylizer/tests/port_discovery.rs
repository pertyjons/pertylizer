//! Regression coverage for live MCP module-port discovery.

use synth_core::ModuleType;

#[test]
fn live_module_discovery_exposes_semantic_port_metadata() {
    let result = pertylizer::mcp_bridge::search_module_types(None, None, None, Some("lfo"));
    let lfo = result
        .modules
        .iter()
        .find(|module| module.type_key == ModuleType::Lfo.prefix())
        .expect("LFO should be discoverable");

    let output = lfo
        .output_ports
        .iter()
        .find(|port| port.name == "out")
        .expect("LFO output should be discoverable");
    assert_eq!(output.label, "Out");
    assert_eq!(output.signal_type, "control");
    assert_eq!(output.value_domain.id, "bipolar");
    assert_eq!(output.value_domain.nominal_min, Some(-1.0));
    assert_eq!(output.value_domain.nominal_max, Some(1.0));
    assert_eq!(output.value_domain.unit.as_deref(), Some("normalized"));
    assert!(
        output.value_domain.accepted_values.contains("finite f32"),
        "LFO output should document accepted numeric values"
    );
    assert!(
        output.description.contains("LFO signal"),
        "LFO output description should explain its signal"
    );

    let retrigger = lfo
        .input_ports
        .iter()
        .find(|port| port.name == "retrigger")
        .expect("LFO retrigger input should be discoverable");
    assert_eq!(retrigger.label, "Retrig");
    assert_eq!(retrigger.signal_type, "gate");
    assert_eq!(retrigger.value_domain.id, "gate");
    assert_eq!(retrigger.value_domain.nominal_min, Some(0.0));
    assert_eq!(retrigger.value_domain.nominal_max, Some(1.0));
    assert!(
        retrigger.value_domain.accepted_values.contains("> 0.5"),
        "gate input should expose its threshold"
    );
    assert!(
        !retrigger.description.is_empty(),
        "LFO retrigger description should be exposed"
    );
}
