//! P09-S001: simulated output conformance for IO-INV-001..004 and IO-INV-006.
//! IO-INV-005's input clocks/capture remain unavailable, not implicitly qualified.

mod common;

use common::{rate, source_plan};
use synth_engine_v2::host::*;
use synth_engine_v2::ir::IrNodeKind;
use synth_engine_v2::quantities::{Amplitude, ChannelLayout};
use synth_engine_v2::render::AudioBlockMut;
use synth_engine_v2::time::{FrameCount, SampleTime};

fn id(value: &str) -> EndpointId {
    EndpointId::new(value.to_owned()).unwrap()
}

fn format(hz: f32, layout: ChannelLayout) -> OutputFormat {
    OutputFormat {
        rate: rate(hz),
        layout,
    }
}

fn backend() -> SimulatedBackend {
    SimulatedBackend {
        endpoints: vec![SimulatedEndpoint {
            id: id("output-a"),
            display_name: "Speakers".to_owned(),
            format: format(48_000.0, ChannelLayout::Mono),
            callback_bound: CallbackBound::Guaranteed(FrameCount::new(256)),
            open_succeeds: true,
        }],
        default_output: Some(id("output-a")),
    }
}

fn request() -> OutputRequest {
    OutputRequest::new(
        EndpointSelection::Exact(id("output-a")),
        format(48_000.0, ChannelLayout::Mono),
    )
}

fn graph() -> synth_engine_v2::ir::GraphIr {
    source_plan(IrNodeKind::Constant {
        level: Amplitude::new(0.25).unwrap(),
    })
}

fn prepare(host: &mut SimulatedHost) -> ConnectionGeneration {
    let generation = host.begin(request()).unwrap();
    host.prepare(generation, &backend(), &graph()).unwrap();
    generation
}

fn active() -> (SimulatedHost, ConnectionGeneration) {
    let mut host = SimulatedHost::new();
    let generation = prepare(&mut host);
    host.activate(generation).unwrap();
    (host, generation)
}

fn callback(host: &mut SimulatedHost, generation: ConnectionGeneration, frames: usize) -> Vec<f32> {
    let mut samples = vec![9.0; frames];
    host.callback(
        generation,
        AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).unwrap(),
    )
    .unwrap();
    samples
}

#[test]
fn readiness_means_owned_configuration_and_activation_requires_explicit_start() {
    let mut host = SimulatedHost::new();
    assert!(host.active().is_none());
    assert!(host.last_valid_plan().is_none());
    let generation = host.begin(request()).unwrap();
    assert_eq!(host.candidate().unwrap().state, ConnectionState::Preparing);
    assert!(host.candidate().unwrap().identity.is_none());
    assert!(matches!(
        host.activate(generation),
        Err(HostError::WrongState)
    ));
    host.prepare(generation, &backend(), &graph()).unwrap();
    let candidate = host.candidate().unwrap().clone();
    assert_eq!(candidate.state, ConnectionState::Ready);
    assert!(candidate.identity.is_some());
    assert!(host.active().is_none());
    assert!(host.last_valid_plan().is_none());
    host.activate(generation).unwrap();
    assert_eq!(host.active(), Some(&candidate));
    assert_eq!(callback(&mut host, generation, 128), vec![0.0; 128]);
    host.start(generation).unwrap();
    let audio = callback(&mut host, generation, 256);
    assert!(audio.contains(&0.25));
    let ack = host.active().unwrap().identity;
    let clock = host.active().unwrap().clock;
    host.stop(generation).unwrap();
    assert_eq!(callback(&mut host, generation, 256), vec![0.0; 256]);
    assert_eq!(host.active().unwrap().identity, ack);
    assert_eq!(host.active().unwrap().clock, clock);
}

#[test]
fn preparing_and_failed_candidates_preserve_the_active_plan_and_request() {
    let (mut host, generation) = active();
    host.start(generation).unwrap();
    assert!(matches!(
        host.begin(request()),
        Err(HostError::TransportRunning)
    ));
    host.stop(generation).unwrap();
    let old = host.active().unwrap().clone();
    let candidate = host.begin(request()).unwrap();
    assert_eq!(host.active(), Some(&old));
    let mut refused = backend();
    refused.endpoints[0].open_succeeds = false;
    assert!(matches!(
        host.prepare(candidate, &refused, &graph()),
        Err(HostError::OpenFailed)
    ));
    assert_eq!(
        host.candidate().unwrap().failure,
        Some(HostFailure::OpenFailed)
    );
    assert_eq!(
        host.candidate().unwrap().state,
        ConnectionState::Unavailable
    );
    assert_eq!(host.active(), Some(&old));
    assert_eq!(
        host.last_valid_plan().unwrap().id(),
        old.identity.unwrap().plan
    );
    assert_eq!(host.selected_request(), Some(&request()));
    host.start(generation).unwrap();
    assert!(callback(&mut host, generation, 256).contains(&0.25));
}

#[test]
fn compile_failure_cannot_publish_a_partial_candidate() {
    let (mut host, _) = active();
    let plan = host.last_valid_plan().unwrap().id();
    let candidate = host.begin(request()).unwrap();
    let invalid = common::declaring(synth_engine_v2::ir::PlanDeclarations {
        events_per_quantum: synth_engine_v2::quantities::EventCount::measured(u32::MAX),
        ..Default::default()
    });
    assert!(matches!(
        host.prepare(candidate, &backend(), &invalid),
        Err(HostError::Compile(_))
    ));
    assert_eq!(
        host.candidate().unwrap().failure,
        Some(HostFailure::Compilation)
    );
    assert!(host.candidate().unwrap().identity.is_none());
    assert!(matches!(
        host.activate(candidate),
        Err(HostError::WrongState)
    ));
    assert_eq!(host.last_valid_plan().unwrap().id(), plan);
}

#[test]
fn loss_needs_no_final_callback_and_reconnection_never_starts_playback() {
    for final_callback in [false, true] {
        let (mut host, old) = active();
        host.start(old).unwrap();
        callback(&mut host, old, 256);
        let plan = host.last_valid_plan().unwrap().id();
        let epoch = host.active().unwrap().identity.unwrap().epoch;
        host.device_lost(old).unwrap();
        assert_eq!(host.active().unwrap().state, ConnectionState::Quiescing);
        let clock = host.active().unwrap().clock;
        if final_callback {
            assert_eq!(callback(&mut host, old, 256), vec![0.0; 256]);
            assert_eq!(host.active().unwrap().clock, clock);
        }
        let new = prepare(&mut host);
        assert_ne!(new, old);
        assert!(matches!(
            host.activate(new),
            Err(HostError::AwaitingQuiescence)
        ));
        assert_eq!(host.last_valid_plan().unwrap().id(), plan);
        host.acknowledge_quiescence(old).unwrap();
        assert_eq!(host.active().unwrap().state, ConnectionState::Unavailable);
        assert!(host.active().unwrap().identity.is_none());
        assert_eq!(host.last_valid_plan().unwrap().id(), plan);
        host.activate(new).unwrap();
        assert_eq!(host.active().unwrap().state, ConnectionState::Ready);
        assert_ne!(host.active().unwrap().identity.unwrap().epoch, epoch);
        assert_eq!(host.active().unwrap().clock, SampleTime::ZERO);
        assert_eq!(callback(&mut host, new, 256), vec![0.0; 256]);
    }
}

#[test]
fn stale_callbacks_errors_commands_and_completion_cannot_change_the_replacement() {
    let (mut host, old) = active();
    host.shutdown(old).unwrap();
    host.acknowledge_quiescence(old).unwrap();
    let canceled = host.begin(request()).unwrap();
    let new = prepare(&mut host);
    assert!(matches!(
        host.prepare(canceled, &backend(), &graph()),
        Err(HostError::StaleGeneration)
    ));
    host.activate(new).unwrap();
    let before = host.active().unwrap().clone();
    assert!(matches!(
        host.device_lost(old),
        Err(HostError::StaleGeneration)
    ));
    assert!(matches!(host.start(old), Err(HostError::StaleGeneration)));
    assert!(matches!(host.stop(old), Err(HostError::StaleGeneration)));
    assert!(matches!(
        host.shutdown(old),
        Err(HostError::StaleGeneration)
    ));
    assert!(matches!(
        host.acknowledge_quiescence(old),
        Err(HostError::StaleGeneration)
    ));
    assert!(matches!(
        host.activate(old),
        Err(HostError::StaleGeneration)
    ));
    let mut samples = [9.0; 64];
    assert_eq!(
        host.callback(
            old,
            AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).unwrap()
        ),
        Err(CallbackError::StaleGeneration)
    );
    assert_eq!(samples, [0.0; 64]);
    assert_eq!(host.active(), Some(&before));
}

#[test]
fn changing_rate_layout_or_capacity_prepares_a_fresh_epoch() {
    for (hz, layout, maximum) in [
        (44_100.0, ChannelLayout::Mono, 256),
        (48_000.0, ChannelLayout::Stereo, 256),
        (48_000.0, ChannelLayout::Mono, 128),
    ] {
        let (mut host, old) = active();
        let epoch = host.active().unwrap().identity.unwrap().epoch;
        let mut changed = backend();
        changed.endpoints[0].format = format(hz, layout);
        changed.endpoints[0].callback_bound = CallbackBound::Guaranteed(FrameCount::new(maximum));
        let requested =
            OutputRequest::new(EndpointSelection::Exact(id("output-a")), format(hz, layout));
        let new = host.begin(requested).unwrap();
        host.prepare(new, &changed, &graph()).unwrap();
        host.shutdown(old).unwrap();
        host.acknowledge_quiescence(old).unwrap();
        assert_eq!(host.active().unwrap().state, ConnectionState::Stopped);
        host.activate(new).unwrap();
        assert_ne!(host.active().unwrap().identity.unwrap().epoch, epoch);
        let plan = host.last_valid_plan().unwrap();
        assert_eq!(plan.sample_rate(), rate(hz));
        assert_eq!(plan.channel_layout(), layout);
        assert_eq!(plan.maximum_block_size(), FrameCount::new(maximum));
    }
}

#[test]
fn unknown_bound_refuses_even_with_an_estimate_or_requested_ceiling() {
    for estimate in [None, Some(FrameCount::new(256))] {
        let mut host = SimulatedHost::new();
        let generation = host
            .begin(
                request()
                    .with_buffer_preference(FrameCount::new(256))
                    .unwrap(),
            )
            .unwrap();
        let mut unknown = backend();
        unknown.endpoints[0].callback_bound = CallbackBound::Unknown { estimate };
        assert!(matches!(
            host.prepare(generation, &unknown, &graph()),
            Err(HostError::UnknownCallbackBound)
        ));
        assert_eq!(
            host.candidate().unwrap().failure,
            Some(HostFailure::UnknownCallbackBound)
        );
        assert!(host.candidate().unwrap().identity.is_none());
    }
    assert!(matches!(
        request().with_buffer_preference(FrameCount::ZERO),
        Err(HostError::ZeroBuffer)
    ));
    assert!(matches!(
        EndpointId::new("  ".to_owned()),
        Err(HostError::EmptyEndpoint)
    ));
}

#[test]
fn endpoint_identity_and_explicit_fallback_are_observable() {
    let mut host = SimulatedHost::new();
    let mut changed = backend();
    changed.endpoints[0].id = id("output-b"); // Same display name, different identity.
    changed.endpoints[0].format = format(44_100.0, ChannelLayout::Stereo);
    let refused = host.begin(request()).unwrap();
    assert!(matches!(
        host.prepare(refused, &changed, &graph()),
        Err(HostError::EndpointUnavailable)
    ));
    let mut permitted = request();
    permitted.fallback_endpoints.push(id("output-b"));
    let refused = host.begin(permitted.clone()).unwrap();
    assert!(matches!(
        host.prepare(refused, &changed, &graph()),
        Err(HostError::UnpermittedFormat)
    ));
    permitted.fallback_formats.push(changed.endpoints[0].format);
    let generation = host.begin(permitted.clone()).unwrap();
    host.prepare(generation, &changed, &graph()).unwrap();
    let actual = host.candidate().unwrap().negotiated.as_ref().unwrap();
    assert_eq!(actual.endpoint, id("output-b"));
    assert!(actual.endpoint_substituted && actual.format_substituted);
    assert_eq!(host.selected_request(), Some(&permitted));
    host.shutdown(generation).unwrap();
    host.acknowledge_quiescence(generation).unwrap();
    changed.endpoints.push(changed.endpoints[0].clone());
    let generation = host.begin(permitted).unwrap();
    assert!(matches!(
        host.prepare(generation, &changed, &graph()),
        Err(HostError::AmbiguousEndpoint)
    ));
}

#[test]
fn explicit_system_default_resolves_by_identity() {
    let mut host = SimulatedHost::new();
    let generation = host
        .begin(OutputRequest::new(
            EndpointSelection::SystemDefault,
            request().format,
        ))
        .unwrap();
    host.prepare(generation, &backend(), &graph()).unwrap();
    assert_eq!(
        host.candidate()
            .unwrap()
            .negotiated
            .as_ref()
            .unwrap()
            .endpoint,
        id("output-a")
    );
}

#[test]
fn callback_partitions_preserve_audio_and_oversize_is_terminal_in_every_build_mode() {
    let sine = source_plan(IrNodeKind::Sine {
        frequency: synth_engine_v2::quantities::Frequency::new(440.0).unwrap(),
        amplitude: Amplitude::new(0.25).unwrap(),
    });
    let make_host = || {
        let mut host = SimulatedHost::new();
        let generation = host.begin(request()).unwrap();
        host.prepare(generation, &backend(), &sine).unwrap();
        host.activate(generation).unwrap();
        (host, generation)
    };
    let (mut whole, g1) = make_host();
    whole.start(g1).unwrap();
    let expected = callback(&mut whole, g1, 256);
    let (mut split, g2) = make_host();
    split.start(g2).unwrap();
    let epoch = split.active().unwrap().identity.unwrap().epoch;
    let actual: Vec<_> = [1, 63, 17, 100, 75]
        .into_iter()
        .flat_map(|frames| callback(&mut split, g2, frames))
        .collect();
    assert_eq!(actual, expected);
    assert!(actual.windows(2).any(|pair| pair[0] != pair[1]));
    assert_eq!(split.active().unwrap().clock, whole.active().unwrap().clock);
    assert_eq!(split.active().unwrap().identity.unwrap().epoch, epoch);

    for running in [false, true] {
        let (mut host, generation) = active();
        if running {
            host.start(generation).unwrap();
        }
        let mut samples = [9.0; 257];
        assert!(matches!(
            host.callback(
                generation,
                AudioBlockMut::new(&mut samples, 257, ChannelLayout::Mono).unwrap()
            ),
            Err(CallbackError::Render(_))
        ));
        assert_eq!(samples, [0.0; 257]);
        let fault = host.active().unwrap().clone();
        assert!(fault.needs_reprepare);
        assert_eq!(fault.state, ConnectionState::Quiescing);
        assert_eq!(fault.last_callback, Some(FrameCount::new(257)));
        // No telemetry reader is needed for fault retention; arbitrary later callbacks
        // remain silent and cannot advance or clear the fault.
        for _ in 0..100 {
            assert_eq!(callback(&mut host, generation, 64), vec![0.0; 64]);
            assert_eq!(host.active().unwrap().failure, fault.failure);
            assert_eq!(host.active().unwrap().clock, fault.clock);
        }
        assert!(matches!(host.start(generation), Err(HostError::WrongState)));
    }
}

#[test]
fn candidate_loss_invalidates_readiness_and_fences_resources_without_changing_the_active_plan() {
    for final_callback in [false, true] {
        let (mut host, old) = active();
        let before = host.active().unwrap().clone();
        let candidate = prepare(&mut host);
        assert_eq!(callback(&mut host, candidate, 128), vec![0.0; 128]);
        assert_eq!(host.candidate().unwrap().clock, SampleTime::ZERO);
        assert_eq!(host.active(), Some(&before));
        host.device_lost(candidate).unwrap();
        assert_eq!(host.candidate().unwrap().state, ConnectionState::Quiescing);
        assert_eq!(
            host.candidate().unwrap().failure,
            Some(HostFailure::DeviceLost)
        );
        assert!(matches!(
            host.activate(candidate),
            Err(HostError::WrongState)
        ));
        assert!(matches!(
            host.begin(request()),
            Err(HostError::AwaitingQuiescence)
        ));
        if final_callback {
            assert_eq!(callback(&mut host, candidate, 128), vec![0.0; 128]);
        }
        host.acknowledge_quiescence(candidate).unwrap();
        assert_eq!(
            host.candidate().unwrap().state,
            ConnectionState::Unavailable
        );
        assert_eq!(host.active(), Some(&before));
        assert_eq!(
            host.last_valid_plan().unwrap().id(),
            before.identity.unwrap().plan
        );
        let replacement = prepare(&mut host);
        let ready = host.candidate().unwrap().clone();
        assert!(matches!(
            host.device_lost(candidate),
            Err(HostError::StaleGeneration)
        ));
        assert_eq!(host.candidate(), Some(&ready));
        host.shutdown(old).unwrap();
        host.acknowledge_quiescence(old).unwrap();
        host.activate(replacement).unwrap();
        assert_eq!(host.active().unwrap().state, ConnectionState::Ready);
    }
}

#[test]
fn loss_or_cancellation_during_preparation_rejects_late_readiness() {
    for lost in [false, true] {
        let mut host = SimulatedHost::new();
        let candidate = host.begin(request()).unwrap();
        if lost {
            host.device_lost(candidate).unwrap();
        } else {
            host.shutdown(candidate).unwrap();
        }
        assert!(matches!(
            host.prepare(candidate, &backend(), &graph()),
            Err(HostError::WrongState)
        ));
        assert!(matches!(
            host.activate(candidate),
            Err(HostError::WrongState)
        ));
        assert!(host.last_valid_plan().is_none());
        let replacement = prepare(&mut host);
        assert_ne!(replacement, candidate);
    }
}

#[test]
fn candidate_callbacks_are_silent_and_configuration_faults_prevent_activation() {
    for oversized in [false, true] {
        let mut host = SimulatedHost::new();
        let candidate = prepare(&mut host);
        let frames = if oversized { 257 } else { 128 };
        let layout = if oversized {
            ChannelLayout::Mono
        } else {
            ChannelLayout::Stereo
        };
        let mut samples = vec![9.0; frames * layout.channels()];
        assert!(matches!(
            host.callback(
                candidate,
                AudioBlockMut::new(&mut samples, frames, layout).unwrap()
            ),
            Err(CallbackError::Render(_))
        ));
        assert!(samples.iter().all(|sample| *sample == 0.0));
        assert_eq!(host.candidate().unwrap().state, ConnectionState::Quiescing);
        assert!(matches!(
            host.activate(candidate),
            Err(HostError::WrongState)
        ));
        host.acknowledge_quiescence(candidate).unwrap();
        assert!(host.last_valid_plan().is_none());
    }
}

#[test]
fn an_enumerated_primary_failure_does_not_silently_try_a_fallback() {
    let mut host = SimulatedHost::new();
    let mut devices = backend();
    let mut fallback = devices.endpoints[0].clone();
    fallback.id = id("output-b");
    devices.endpoints.push(fallback);
    devices.endpoints[0].open_succeeds = false;
    let mut requested = request();
    requested.fallback_endpoints.push(id("output-b"));
    let generation = host.begin(requested).unwrap();
    assert!(matches!(
        host.prepare(generation, &devices, &graph()),
        Err(HostError::OpenFailed)
    ));
}
