//! Tests for `mod_matrix_routing_tests`.

use super::*;
use synth_core::{
    BipolarValue, DestAddr, ModDestination, ModMatrixGridSize, ModMatrixParam, ModSource,
    ModuleType, SrcAddr,
};
use synth_engine::ModuleStateSnapshot;
use synth_engine::instrument::InstrumentId;

fn stub_module(mt: ModuleType, instance: u16) -> ModuleStateSnapshot {
    ModuleStateSnapshot::new(
        ModuleId::new(mt, instance),
        InstrumentId::new(1),
        mt,
        format!("{}-{}", mt.prefix(), instance),
    )
}

/// The "instrument will be silent" diagnostic must recognise every audio
/// generator as a sound source — including the voice modules (regression:
/// the old hand-maintained whitelist omitted VoiceSynth / VocalTract and so
/// flagged every voice patch as silent).
#[test]
fn is_sound_source_covers_generators_and_voices() {
    for mt in [
        ModuleType::Oscillator,
        ModuleType::Noise,
        ModuleType::GranularOsc,
        ModuleType::Sampler,
        ModuleType::AmFormant,
        ModuleType::VoiceSynth,
        ModuleType::VocalTract,
        ModuleType::Fof,
        ModuleType::MechanicalNoise,
    ] {
        assert!(is_sound_source(mt), "{mt:?} should count as a sound source");
    }
    for mt in [
        ModuleType::Lfo,
        ModuleType::Filter,
        ModuleType::Envelope,
        ModuleType::Amplifier,
        ModuleType::StereoOutput,
        ModuleType::Reverb,
    ] {
        assert!(
            !is_sound_source(mt),
            "{mt:?} should not count as a sound source"
        );
    }
}

fn matrix_snapshot(matrix_instance: u16, params: Vec<Param>) -> ModuleStateSnapshot {
    let mut snap = stub_module(ModuleType::ModMatrix, matrix_instance);
    snap.parameters = params;
    snap
}

/// The routings report surfaces a slot's YAMS control script (S2.3a): the
/// `script` text rides alongside the addresses, and a slot that has *only* a
/// script (no scalar source/dest) is still reported — the script owns the value.
#[test]
fn routings_report_includes_control_script() {
    // A scripted slot that also has a destination.
    let mut mmx = matrix_snapshot(
        1,
        vec![
            Param::ModMatrix(ModMatrixParam::SlotDestination(
                0,
                Some(DestAddr::new(ModuleType::Filter, 1, "cutoff")),
            )),
            Param::ModMatrix(ModMatrixParam::SlotEnabled(0, true)),
        ],
    );
    mmx.scripts
        .insert("1".to_string(), "out = velocity * 0.5".to_string());
    let routings = collect_mod_matrix_routings(&mmx.parameters, &mmx.scripts);
    assert_eq!(routings.len(), 1);
    assert_eq!(routings[0].slot, 1);
    assert_eq!(routings[0].destination, "flt-1.cutoff");
    assert_eq!(routings[0].script.as_deref(), Some("out = velocity * 0.5"));

    // A slot with ONLY a script (no source/dest) is still surfaced.
    let mut only = matrix_snapshot(1, vec![]);
    only.scripts.insert("3".to_string(), "out = 1".to_string());
    let r = collect_mod_matrix_routings(&only.parameters, &only.scripts);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].slot, 3);
    assert_eq!(r[0].script.as_deref(), Some("out = 1"));
    // A script-free slot reports no script.
    assert!(r.iter().all(|x| x.slot != 1));
}

/// The report echoes the stored absolute address faithfully — no
/// report-time positional remap (legacy positional ids are upgraded at
/// *load* instead; see `patch::upgrade_legacy_mod_matrix`). An instrument on
/// non-canonical global instances (env-5/env-6, flt-3) resolves its routing
/// and badge sets to those exact instances.
#[test]
fn report_echoes_absolute_addresses_for_noncanonical_instances() {
    let modules = vec![
        stub_module(ModuleType::Envelope, 5),
        stub_module(ModuleType::Envelope, 6),
        stub_module(ModuleType::Filter, 3),
        matrix_snapshot(
            2,
            vec![
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    0,
                    Some(SrcAddr::module(ModuleType::Envelope, 6, "out")),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    0,
                    Some(DestAddr::new(ModuleType::Filter, 3, "cutoff")),
                )),
                Param::ModMatrix(ModMatrixParam::SlotAmount(0, BipolarValue::new(0.9))),
                Param::ModMatrix(ModMatrixParam::SlotEnabled(0, true)),
            ],
        ),
    ];

    let idx = InstrumentModuleIndex::from_snapshots(&modules);
    let last = modules.last().unwrap();
    let routings = collect_mod_matrix_routings(&last.parameters, &last.scripts);
    assert_eq!(routings.len(), 1);
    let r = &routings[0];
    assert_eq!(r.source, "env-6.out");
    assert_eq!(r.destination, "flt-3.cutoff");
    assert!((r.amount - 0.9).abs() < 1e-4);
    assert!(r.enabled);

    // Badge sets resolve the exact addressed instances — these feed the
    // virtual "matrix" port surfaced on the actual module instances.
    let sources = collect_mod_matrix_sources(&modules, &idx);
    let destinations = collect_mod_matrix_destinations(&modules, &idx);
    assert!(
        sources.contains(&ModuleId::new(ModuleType::Envelope, 6)),
        "sources={sources:?}"
    );
    assert!(
        destinations.contains(&ModuleId::new(ModuleType::Filter, 3)),
        "destinations={destinations:?}"
    );
    // The unrelated global env-2 is NOT pulled in.
    assert!(!sources.contains(&ModuleId::new(ModuleType::Envelope, 2)));
}

/// `GridSize` is vestigial: every configured slot is reported regardless of
/// the old grid dimension (the matrix is a flat list now, not an N×N grid).
#[test]
fn grid_size_does_not_limit_configured_slots() {
    let modules = [
        stub_module(ModuleType::Lfo, 1),
        stub_module(ModuleType::Envelope, 1),
        stub_module(ModuleType::Oscillator, 1),
        stub_module(ModuleType::Amplifier, 1),
        matrix_snapshot(
            1,
            vec![
                Param::ModMatrix(ModMatrixParam::GridSize(ModMatrixGridSize::Grid1x1)),
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    0,
                    SrcAddr::from_mod_source(ModSource::Lfo(0)),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    0,
                    DestAddr::from_mod_destination(ModDestination::OscPitch(0)),
                )),
                // Slot 5 would be past a 1×1 grid — it must still appear.
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    4,
                    SrcAddr::from_mod_source(ModSource::Envelope(0)),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    4,
                    DestAddr::from_mod_destination(ModDestination::AmpLevel(0)),
                )),
            ],
        ),
    ];

    let last = modules.last().unwrap();
    let routings = collect_mod_matrix_routings(&last.parameters, &last.scripts);
    assert_eq!(routings.len(), 2);
    assert_eq!(routings[0].slot, 1);
    assert_eq!(routings[0].source, "lfo-1.out");
    assert_eq!(routings[0].destination, "osc-1.pitch");
    assert_eq!(routings[1].slot, 5);
    assert_eq!(routings[1].source, "env-1.out");
    assert_eq!(routings[1].destination, "amp-1.level");
}

/// Disabled slots should not contribute to either the source or
/// destination sets — otherwise toggling a slot off would still leave
/// the badge / `"matrix"` port lingering.
#[test]
fn disabled_slots_drop_from_both_sets() {
    let modules = vec![
        stub_module(ModuleType::Lfo, 1),
        stub_module(ModuleType::Oscillator, 1),
        matrix_snapshot(
            1,
            vec![
                Param::ModMatrix(ModMatrixParam::GridSize(ModMatrixGridSize::Grid1x1)),
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    0,
                    SrcAddr::from_mod_source(ModSource::Lfo(0)),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    0,
                    DestAddr::from_mod_destination(ModDestination::OscPitch(0)),
                )),
                Param::ModMatrix(ModMatrixParam::SlotEnabled(0, false)),
            ],
        ),
    ];

    let idx = InstrumentModuleIndex::from_snapshots(&modules);
    assert!(collect_mod_matrix_sources(&modules, &idx).is_empty());
    assert!(collect_mod_matrix_destinations(&modules, &idx).is_empty());
}

/// Non-module sources like Velocity / Mod Wheel have no positional
/// resolution; the helper falls back to the source's static `id()`
/// string.
#[test]
fn implicit_sources_use_source_id_fallback() {
    let modules = [
        stub_module(ModuleType::Amplifier, 1),
        stub_module(ModuleType::Filter, 1),
        matrix_snapshot(
            1,
            vec![
                Param::ModMatrix(ModMatrixParam::GridSize(ModMatrixGridSize::Grid2x2)),
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    0,
                    SrcAddr::from_mod_source(ModSource::Velocity),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    0,
                    DestAddr::from_mod_destination(ModDestination::AmpLevel(0)),
                )),
                Param::ModMatrix(ModMatrixParam::SlotSource(
                    1,
                    SrcAddr::from_mod_source(ModSource::ModWheel),
                )),
                Param::ModMatrix(ModMatrixParam::SlotDestination(
                    1,
                    DestAddr::from_mod_destination(ModDestination::FilterCutoff(0)),
                )),
            ],
        ),
    ];

    let last = modules.last().unwrap();
    let routings = collect_mod_matrix_routings(&last.parameters, &last.scripts);
    assert_eq!(routings.len(), 2);
    assert_eq!(routings[0].source, "velocity");
    assert_eq!(routings[1].source, "mod_wheel");
}
