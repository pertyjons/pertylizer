//! Integration tests for `rebuild_instrument_preserve_automation`.
//!
//! Rebuilding an instrument's voice graph must keep automation lanes pointed at
//! still-present modules (counter reset makes same-composition module ids line
//! up) and report — optionally drop — lanes whose target module is gone.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_core::InstrumentId;
use synth_engine::SynthEngine;
use synth_mcp::bridge::*;
use synth_mcp::bridge::{
    BridgeAutomationPointData, BridgeConnectionDef, BridgeInstrumentDef, BridgeModuleDef,
    BridgeParamValue, CurveKind,
};
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_bridge() -> AppSynthBridge {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Rebuild")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));
    AppSynthBridge::new(session, shared, sample_library)
}

/// osc → flt → amp → out, with the filter present so `module:flt:1:cutoff`
/// is a valid automation target.
fn spec_with_filter(instrument_id: Option<InstrumentId>) -> BridgeInstrumentDef {
    BridgeInstrumentDef {
        instrument_id,
        name: "Lead".to_string(),
        midi_channel: None,
        volume: None,
        pan: None,
        modules: vec![
            BridgeModuleDef {
                module_type: "osc".to_string(),
                params: vec![(
                    "waveform".to_string(),
                    BridgeParamValue::Choice("sawtooth".to_string()),
                )],
            },
            BridgeModuleDef {
                module_type: "flt".to_string(),
                params: vec![],
            },
            BridgeModuleDef {
                module_type: "amp".to_string(),
                params: vec![],
            },
            BridgeModuleDef {
                module_type: "out".to_string(),
                params: vec![],
            },
        ],
        connections: vec![
            BridgeConnectionDef {
                from_index: 0,
                from_port: "out".to_string(),
                to_index: 1,
                to_port: "in".to_string(),
            },
            BridgeConnectionDef {
                from_index: 1,
                from_port: "out".to_string(),
                to_index: 2,
                to_port: "in".to_string(),
            },
        ],
    }
}

/// Same chain minus the filter: osc → amp → out. Rebuilding to this orphans any
/// `flt` automation.
fn spec_without_filter(instrument_id: Option<InstrumentId>) -> BridgeInstrumentDef {
    BridgeInstrumentDef {
        instrument_id,
        name: "Lead".to_string(),
        midi_channel: None,
        volume: None,
        pan: None,
        modules: vec![
            BridgeModuleDef {
                module_type: "osc".to_string(),
                params: vec![],
            },
            BridgeModuleDef {
                module_type: "amp".to_string(),
                params: vec![],
            },
            BridgeModuleDef {
                module_type: "out".to_string(),
                params: vec![],
            },
        ],
        connections: vec![],
    }
}

fn script_spec(instrument_id: Option<InstrumentId>) -> BridgeInstrumentDef {
    BridgeInstrumentDef {
        instrument_id,
        name: "Script Lead".to_string(),
        midi_channel: None,
        volume: None,
        pan: None,
        modules: vec![BridgeModuleDef {
            module_type: "scr".to_string(),
            params: vec![],
        }],
        connections: vec![],
    }
}

/// Build the instrument + a pattern with one `module:flt:1:cutoff` lane.
/// Returns (bridge, instrument_id, pattern_id).
fn setup() -> (AppSynthBridge, InstrumentId, synth_sequencer::PatternId) {
    let bridge = build_bridge();
    let built = bridge
        .build_instrument(&spec_with_filter(None))
        .expect("build instrument");
    let inst = built.instrument_id;

    let pattern_id = bridge.create_pattern("P1", 4.0).expect("create pattern");
    bridge
        .add_automation_points(
            pattern_id,
            &[BridgeAutomationPointData {
                param: "module:flt:1:cutoff".to_string(),
                instrument_id: inst,
                beat: 0.0,
                value: 0.5,
                curve: CurveKind::Linear,
                curve_strength: None,
            }],
        )
        .expect("add automation point");
    (bridge, inst, pattern_id)
}

#[test]
fn rebuild_with_same_modules_preserves_automation() {
    let (bridge, inst, pattern_id) = setup();

    let result = bridge
        .rebuild_instrument_preserve_automation(&spec_with_filter(Some(inst)), false)
        .expect("rebuild");

    assert!(
        result.orphaned_lanes.is_empty(),
        "the filter still exists, so no lane should orphan: {:?}",
        result.orphaned_lanes
    );
    assert!(
        result.preserved_lanes >= 1,
        "the flt:1:cutoff lane should be preserved, got {}",
        result.preserved_lanes
    );
    // The lane is still listed and valid.
    let lanes = bridge
        .list_automation_lanes(pattern_id)
        .expect("list lanes");
    assert!(
        lanes.iter().any(|l| l.target.contains("flt")),
        "filter lane should survive, got {lanes:?}"
    );
}

#[test]
fn rebuild_dropping_filter_orphans_lane_and_can_drop_it() {
    let (bridge, inst, pattern_id) = setup();

    // Rebuild without the filter, reporting only (drop_orphaned = false).
    let report = bridge
        .rebuild_instrument_preserve_automation(&spec_without_filter(Some(inst)), false)
        .expect("rebuild without filter");
    assert_eq!(report.orphaned_lanes.len(), 1, "the flt lane should orphan");
    assert!(report.orphaned_lanes[0].target.contains("flt"));
    assert!(!report.dropped_orphaned);
    // Not dropped yet — the lane is still present (dangling).
    assert_eq!(
        bridge.list_automation_lanes(pattern_id).unwrap().len(),
        1,
        "reporting must not remove the lane"
    );

    // Rebuild again, this time dropping orphans.
    let dropped = bridge
        .rebuild_instrument_preserve_automation(&spec_without_filter(Some(inst)), true)
        .expect("rebuild dropping orphans");
    assert_eq!(dropped.orphaned_lanes.len(), 1);
    assert!(dropped.dropped_orphaned);
    assert!(
        bridge.list_automation_lanes(pattern_id).unwrap().is_empty(),
        "drop_orphaned should remove the dangling lane"
    );
}

#[test]
fn rebuild_requires_existing_instrument_id() {
    let bridge = build_bridge();
    let err = bridge
        .rebuild_instrument_preserve_automation(&spec_with_filter(None), false)
        .expect_err("rebuild without instrument_id should error");
    assert!(
        err.to_string().contains("requires instrument_id"),
        "got {err}"
    );
}

#[test]
fn rebuild_reports_and_drops_lane_when_script_param_is_removed() {
    let bridge = build_bridge();
    let built = bridge
        .build_instrument(&script_spec(None))
        .expect("build script instrument");
    let instrument_id = built.instrument_id;
    bridge
        .set_mod_matrix_script(instrument_id, "scr-1", 1, "param drive = 0.5\nout1 = drive")
        .expect("install script parameter");
    let pattern_id = bridge
        .create_pattern("Script automation", 4.0)
        .expect("create pattern");
    let authored = bridge
        .add_automation_points(
            pattern_id,
            &[BridgeAutomationPointData {
                param: "module:scr:1:drive".to_string(),
                instrument_id,
                beat: 0.0,
                value: 0.75,
                curve: CurveKind::Linear,
                curve_strength: None,
            }],
        )
        .expect("automate declared script parameter");
    assert_eq!(authored.succeeded, 1, "script automation must be authored");

    let result = bridge
        .rebuild_instrument_preserve_automation(&script_spec(Some(instrument_id)), true)
        .expect("rebuild after removing declared parameter");

    assert_eq!(result.orphaned_lanes.len(), 1);
    assert_eq!(result.orphaned_lanes[0].target, "module:scr:1:drive");
    assert!(result.dropped_orphaned);
    assert!(
        bridge
            .list_automation_lanes(pattern_id)
            .expect("list lanes")
            .is_empty()
    );
}
