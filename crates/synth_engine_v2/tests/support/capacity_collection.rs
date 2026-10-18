//! Opt-in collection only after the reviewed workload/method commit. Ordinary
//! quality gates compile this harness and run its controls, but collect no numbers.
use super::*;
use crate::frozen_capacity::{
    FrozenError, FrozenHostConfig, FrozenObservations, reference_host, select_host,
};
use std::{process::Command, time::Duration};
use synth_engine_v2::quantities::PreparedBytes;

#[path = "mono_capacity_collection.rs"]
mod mono;

const HOST_HEAP_BOUND: PreparedBytes = PreparedBytes::measured(512 * 1_024 * 1_024);

#[derive(Clone, Copy)]
enum RunKind {
    Reference,
    Selected,
}

impl RunKind {
    fn label(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Selected => "selected",
        }
    }
    fn unavailable(self) -> &'static str {
        match self {
            Self::Reference => "Inconclusive",
            Self::Selected => "Not supported",
        }
    }
}

fn git(argument: &str) -> String {
    let output = Command::new("git")
        .args(argument.split_whitespace())
        .output()
        .expect("git state");
    assert!(output.status.success(), "git {argument}");
    String::from_utf8(output.stdout).expect("git UTF-8")
}

fn environment(record: &str, checksums_path: &str) -> (String, String) {
    if cfg!(debug_assertions) {
        panic!("collection requires --release");
    }
    assert_eq!(
        std::env::var(format!("PERTYLIZER_{record}_COLLECT")).as_deref(),
        Ok("1")
    );
    let freeze = std::env::var(format!("PERTYLIZER_{record}_FREEZE_REVISION"))
        .expect("explicit independently reviewed freeze revision");
    assert_eq!(
        git("rev-parse HEAD").trim(),
        freeze,
        "collection must run on the specified reviewed freeze commit"
    );
    assert!(
        git("status --porcelain --untracked-files=normal").is_empty(),
        "commit and independently review the workload/method before collection"
    );
    let root = git("rev-parse --show-toplevel");
    let checksums = Command::new("sha256sum")
        .current_dir(root.trim())
        .args(["--check", checksums_path])
        .output()
        .expect("frozen workload checksums");
    assert!(
        checksums.status.success(),
        "{}{}",
        String::from_utf8_lossy(&checksums.stdout),
        String::from_utf8_lossy(&checksums.stderr)
    );
    let status = std::fs::read_to_string("/proc/self/status").expect("Linux process status");
    let cpu = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .expect("affinity")
        .trim();
    assert!(
        cpu.parse::<usize>().is_ok(),
        "pin to one recorded logical CPU"
    );
    let governor_path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor");
    let governor = std::fs::read_to_string(&governor_path).expect("recorded fixed governor");
    assert_eq!(
        governor.trim(),
        "performance",
        "the frozen host requires the performance governor"
    );
    let clock =
        std::fs::read_to_string("/sys/devices/system/clocksource/clocksource0/current_clocksource")
            .expect("recorded Linux clock source");
    println!(
        "revision={}cpu={cpu} governor={} clock={} timer=std::time::Instant",
        git("rev-parse HEAD"),
        governor.trim(),
        clock.trim()
    );
    (governor_path, governor)
}

fn assert_resources(joined: &Joined) {
    assert!(joined.source_peaks.iter().all(|peak| peak.get() <= 128));
    let owners = [
        ("initial", joined.retired_reset[0].bridge().transfer()),
        ("successor", joined.teardown.transfer()),
    ];
    for (owner, teardown) in owners {
        let usage = teardown
            .resource_usage()
            .expect("representable observations");
        let used = usage.renderer;
        let granted = usage.renderer_storage;
        assert_eq!(used.rejected_control_writes, EventCount::NONE);
        assert!(used.timed_controls_required <= granted.timed_control_entries);
        assert!(used.event_payloads <= granted.event_entries);
        assert!(used.seeded_rows <= granted.parameter_rows);
        assert!(used.adoption_gate_entries <= granted.adoption_gate_entries);
        println!(
            "owner={owner} publication={:?} resources={usage:?}",
            teardown.publication_usage()
        );
    }
    println!(
        "source_peaks={:?} stage_peaks={:?} raw_input_peaks={:?}",
        joined.source_peaks, joined.stage_peaks, joined.raw_input_peaks
    );
    assert_eq!(joined.queue_owners.len(), 2);
    for (owner, queues) in ["initial", "successor"].iter().zip(&joined.queue_owners) {
        println!(
            "owner={owner} commands={:?} results={:?}",
            queues.commands.get(),
            queues.results.get()
        );
    }
}

fn path_max(
    callbacks: &[callback_probe::Observation],
    boundaries: impl Iterator<Item = SampleTime>,
) -> Duration {
    boundaries
        .filter_map(|boundary| {
            // Priming is callback zero; sample boundary zero follows it at index one.
            let index = usize::try_from(boundary.as_u64() / 64).expect("bounded index") + 1;
            (index >= WARMUP_CALLBACKS).then_some(callbacks[index].duration)
        })
        .max()
        .expect("the required path ran")
}

fn collect(kind: RunKind, config: &FrozenHostConfig) -> FrozenObservations {
    let label = kind.label();
    println!("begin={label} config={config:?}");
    super::super::capacity_controls::controls(config.profile, &|| config.setup());
    crate::raw_reserved_key_control(config, 64, false);
    let mut result = None;
    // Include the entire serial lifecycle and all evidence buffers. Nested scopes
    // may overstate the peak; the whole-host claim remains a conservative upper bound.
    let lifecycle = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        allocation_counter::measure(|| {
            result = Some(sustained_run_with(
                config.profile,
                &|| config.setup(),
                true,
                kind.unavailable(),
            ));
        })
    }));
    let heap = lifecycle.unwrap_or_else(|failure| {
        eprintln!(
            "{label} lifecycle failed; preparation refusals are Inconclusive; see the original runtime diagnostic"
        );
        std::panic::resume_unwind(failure)
    });
    let (transcript, joined, callbacks) = result.expect("completed lifecycle");
    assert_eq!(callbacks.len(), TOTAL_CALLBACKS);
    for (index, observation) in callbacks.iter().enumerate() {
        assert_eq!(observation.allocations.count_total, 0, "callback {index}");
        assert_eq!(observation.allocations.count_current, 0, "callback {index}");
        assert_eq!(observation.allocations.bytes_current, 0, "callback {index}");
        println!(
            "callback={index} elapsed_ns={}",
            observation.duration.as_nanos()
        );
    }
    assert!(
        heap.bytes_max <= HOST_HEAP_BOUND.get(),
        "whole-host requested heap: {heap:?}"
    );
    assert_resources(&joined);
    let observations = FrozenObservations::from_joined(&joined)
        .unwrap_or_else(|error| panic!("Inconclusive: incomplete observations: {error}"));
    crate::frozen_capacity::validate_frozen_bounds(&observations).unwrap_or_else(|error| {
        let classification = match error {
            FrozenError::FrozenBound { .. } => "Not supported",
            _ => "Inconclusive",
        };
        panic!("{classification}: frozen demand validation: {error}")
    });
    config
        .validate_observations(&observations)
        .expect("observations fit admitted queues and shares");
    println!("kind={label} observations={observations:?} requested_heap_upper_bound={heap:?}");
    let rate = config.profile.capabilities().sample_rate();
    let mut timed: Vec<_> = callbacks[WARMUP_CALLBACKS..]
        .iter()
        .map(|entry| entry.duration)
        .collect();
    assert_eq!(timed.len(), EXERCISE_CALLBACKS);
    let maximum = timed.iter().copied().max().expect("timed callbacks");
    let percentile = callback_probe::p999(&mut timed).expect("nearest-rank p99.9");
    let empty_floor = (0..10_000)
        .map(|_| {
            callback_probe::observe(|| std::hint::black_box(()))
                .1
                .duration
        })
        .min()
        .expect("timer floor");
    let deadline = callback_probe::budget(rate);
    println!(
        "kind={label} rate={rate:?} max={maximum:?} p999={percentile:?} empty_floor={empty_floor:?} deadline={deadline:?}"
    );
    println!(
        "paths seek_or_loop={:?} loop={:?} stop={:?} start={:?} panic={:?} reset={:?} pressure={:?}",
        path_max(
            &callbacks,
            transcript.activations.iter().map(|entry| entry.boundary)
        ),
        path_max(
            &callbacks,
            transcript
                .activations
                .iter()
                .filter(|entry| entry.looped)
                .map(|entry| entry.boundary)
        ),
        path_max(&callbacks, transcript.stops.iter().copied()),
        path_max(&callbacks, transcript.starts.iter().copied()),
        path_max(
            &callbacks,
            transcript.panics.iter().map(|entry| entry.executed_at())
        ),
        path_max(&callbacks, transcript.resets.iter().map(|_| at(6_004 * 64))),
        path_max(&callbacks, transcript.pressure.iter().copied())
    );
    let classification = kind.unavailable();
    assert!(
        maximum <= deadline,
        "{classification}: {label} raw maximum exceeds the frozen callback budget"
    );
    observations
}

#[test]
#[ignore = "explicit EVD-0026 release collection; requires specified reviewed revision, pinned CPU and performance governor"]
fn collect_stereo_reference_then_selected_at_all_four_rates() {
    let (governor_path, governor) = environment(
        "EVD_0026",
        "plans/v2/evidence/phase-09/EVD-0026-workload-SHA256SUMS.txt",
    );
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let base = sustained_profile(hz);
        let reference = reference_host(base.capabilities().sample_rate())
            .unwrap_or_else(|error| panic!("Inconclusive: P_ref admission: {error}"));
        let observations = collect(RunKind::Reference, &reference);
        let selected = select_host(base, &observations)
            .unwrap_or_else(|error| panic!("Inconclusive: selected admission: {error}"));
        let rerun = collect(RunKind::Selected, &selected);
        // Use the same fixed selection rule on the rerun, without silently changing
        // its profile. Growth that would demand larger capacities rejects stability.
        let reselection = select_host(base, &rerun)
            .unwrap_or_else(|error| panic!("Inconclusive: selected stability admission: {error}"));
        assert_eq!(
            selected, reselection,
            "Not supported: selected profile is not stable"
        );
    }
    assert_eq!(
        std::fs::read_to_string(governor_path).expect("governor after run"),
        governor
    );
}
