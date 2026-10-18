//! EVD-0027: fresh mono controls and collection after the explicit-fold method freeze.
use super::*;
use synth_engine_v2::quantities::SampleRate;

fn mono_profile(rate: SampleRate) -> Result<HostProfile, FrozenError> {
    if ![44_100.0, 48_000.0, 96_000.0, 192_000.0].contains(&rate.as_f32()) {
        return Err(
            crate::capacity_selection_controls::capacity_selection::CandidateRefused::Rate(rate)
                .into(),
        );
    }
    Ok(HostProfile::harness(
        rate,
        FrameCount::new(64),
        ChannelLayout::Mono,
    )?)
}

#[test]
fn mono_candidate_has_its_own_fixed_layout_and_refuses_rates_outside_the_frozen_envelope() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = mono_profile(SampleRate::new(hz).expect("rate")).expect("candidate");
        assert_eq!(profile.capabilities().channel_layout(), ChannelLayout::Mono);
        assert_eq!(
            profile.capabilities().maximum_block_size(),
            FrameCount::new(64)
        );
    }
    for hz in [8_000.0, 22_050.0, 48_001.0] {
        assert!(matches!(
            mono_profile(SampleRate::new(hz).expect("engine rate")),
            Err(FrozenError::Candidate(_))
        ));
    }
}

#[test]
fn mono_reference_runs_all_functional_controls_and_the_complete_script_without_collection() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let base = mono_profile(SampleRate::new(hz).expect("rate")).expect("candidate");
        let config = crate::frozen_capacity::reference_host_for_profile(base).expect("reference");
        super::super::super::capacity_controls::controls(config.profile, &|| config.setup());
        crate::raw_reserved_key_control(&config, 64, false);
        let (_transcript, _joined, callbacks) =
            sustained_run_with(config.profile, &|| config.setup(), false, "Inconclusive");
        assert!(
            callbacks.is_empty(),
            "functional preparation does not collect timing"
        );
    }
}

#[test]
#[ignore = "explicit EVD-0027 release collection; requires reviewed revision, pinned CPU and performance governor"]
fn collect_mono_reference_then_selected_at_all_four_rates() {
    let (governor_path, governor) = environment(
        "EVD_0027",
        "plans/v2/evidence/phase-09/EVD-0027-workload-SHA256SUMS.txt",
    );
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let base = mono_profile(SampleRate::new(hz).expect("rate"))
            .unwrap_or_else(|error| panic!("Inconclusive: mono envelope: {error}"));
        let reference = crate::frozen_capacity::reference_host_for_profile(base)
            .unwrap_or_else(|error| panic!("Inconclusive: mono reference: {error}"));
        let observations = collect(RunKind::Reference, &reference);
        let selected = select_host(base, &observations)
            .unwrap_or_else(|error| panic!("Inconclusive: mono selected admission: {error}"));
        let rerun = collect(RunKind::Selected, &selected);
        let reselection = select_host(base, &rerun)
            .unwrap_or_else(|error| panic!("Inconclusive: mono stability admission: {error}"));
        assert_eq!(
            selected, reselection,
            "Not supported: mono selected profile is not stable"
        );
    }
    assert_eq!(
        std::fs::read_to_string(governor_path).expect("governor after run"),
        governor
    );
}
