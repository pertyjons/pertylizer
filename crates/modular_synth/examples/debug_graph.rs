//! Debug tool to analyze the default patch module graph.
//!
//! Usage: cargo run --example debug_graph -p modular_synth

#![allow(clippy::unwrap_used)]

use modular_synth::patch::PatchModuleType;
use modular_synth::patches::default_patch;

fn main() {
    println!("=== Default Patch Graph Debugger ===\n");

    let patch = default_patch();

    println!("Patch: {}", patch.name);
    if let Some(desc) = &patch.description {
        println!("Description: {}", desc);
    }
    println!();

    println!("=== MODULES ({}) ===", patch.modules.len());
    for module in &patch.modules {
        println!(
            "  {} ({:?}) at ({}, {})",
            module.id, module.module_type, module.position.0, module.position.1
        );
        if !module.parameters.is_empty() {
            for (name, value) in &module.parameters {
                println!("    {} = {:?}", name, value);
            }
        }
    }
    println!();

    println!("=== CONNECTIONS ({}) ===", patch.connections.len());
    for conn in &patch.connections {
        println!(
            "  {}.{} -> {}.{}",
            conn.from.0, conn.from.1, conn.to.0, conn.to.1
        );
    }
    println!();

    // Check for common issues
    println!("=== DIAGNOSTICS ===");

    // Check that all connection endpoints exist
    let module_ids: Vec<_> = patch.modules.iter().map(|m| m.id.as_str()).collect();

    for conn in &patch.connections {
        if !module_ids.contains(&conn.from.0.as_str()) {
            println!(
                "ERROR: Connection from non-existent module: {}",
                conn.from.0
            );
        }
        if !module_ids.contains(&conn.to.0.as_str()) {
            println!("ERROR: Connection to non-existent module: {}", conn.to.0);
        }
    }

    // Check for modules without any connections
    for module in &patch.modules {
        let has_input = patch.connections.iter().any(|c| c.to.0 == module.id);
        let has_output = patch.connections.iter().any(|c| c.from.0 == module.id);

        if !has_input && !has_output {
            println!("WARNING: Module {} is isolated (no connections)", module.id);
        }
    }

    // Check signal path from oscillators to output
    println!();
    println!("=== SIGNAL PATH CHECK ===");

    // Find output module
    let output_id = patch
        .modules
        .iter()
        .find(|m| matches!(m.module_type, PatchModuleType::StereoOutput))
        .map(|m| m.id.as_str());

    if let Some(out_id) = output_id {
        println!("Output module: {}", out_id);

        // Trace back from output
        let inputs_to_output: Vec<_> = patch
            .connections
            .iter()
            .filter(|c| c.to.0 == out_id)
            .collect();

        if inputs_to_output.is_empty() {
            println!("ERROR: Output module has NO inputs connected!");
        } else {
            println!("Inputs to output:");
            for conn in inputs_to_output {
                println!("  <- {}.{} (port: {})", conn.from.0, conn.from.1, conn.to.1);
            }
        }
    } else {
        println!("ERROR: No output module found!");
    }

    // Find oscillators
    let oscillators: Vec<_> = patch
        .modules
        .iter()
        .filter(|m| matches!(m.module_type, PatchModuleType::Oscillator))
        .collect();

    println!();
    println!("Oscillators ({}):", oscillators.len());
    for osc in &oscillators {
        println!("  {}", osc.id);
        let outputs: Vec<_> = patch
            .connections
            .iter()
            .filter(|c| c.from.0 == osc.id)
            .collect();
        for conn in outputs {
            println!("    -> {}.{}", conn.to.0, conn.to.1);
        }
    }

    // Find envelopes
    let envelopes: Vec<_> = patch
        .modules
        .iter()
        .filter(|m| matches!(m.module_type, PatchModuleType::Envelope))
        .collect();

    println!();
    println!("Envelopes ({}):", envelopes.len());
    for env in &envelopes {
        println!("  {}", env.id);
        // Check if envelope has gate input
        let gate_input = patch
            .connections
            .iter()
            .find(|c| c.to.0 == env.id && (c.to.1 == "gate" || c.to.1 == "trigger"));
        if gate_input.is_none() {
            println!("    WARNING: No gate/trigger input connected!");
        }
        // Show outputs
        let outputs: Vec<_> = patch
            .connections
            .iter()
            .filter(|c| c.from.0 == env.id)
            .collect();
        for conn in outputs {
            println!("    -> {}.{}", conn.to.0, conn.to.1);
        }
    }

    // Find amplifiers
    let amplifiers: Vec<_> = patch
        .modules
        .iter()
        .filter(|m| matches!(m.module_type, PatchModuleType::Amplifier))
        .collect();

    println!();
    println!("Amplifiers ({}):", amplifiers.len());
    for amp in &amplifiers {
        println!("  {}", amp.id);
        // Check CV input
        let cv_input = patch
            .connections
            .iter()
            .find(|c| c.to.0 == amp.id && c.to.1 == "cv");
        if cv_input.is_none() {
            println!("    WARNING: No CV input connected (will rely on base level)");
        } else if let Some(cv) = cv_input {
            println!("    cv <- {}.{}", cv.from.0, cv.from.1);
        }
        // Check audio input
        let audio_input = patch
            .connections
            .iter()
            .find(|c| c.to.0 == amp.id && c.to.1 == "in");
        if audio_input.is_none() {
            println!("    WARNING: No audio input connected!");
        } else if let Some(inp) = audio_input {
            println!("    in <- {}.{}", inp.from.0, inp.from.1);
        }
    }

    println!();
    println!("=== END ===");
}
