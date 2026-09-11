//! `P08-S004`: sends, return buses and the bus graph, from a saved project's side.
//!
//! The V2 crate holds every law bit for bit (`tests/sends.rs`); what is held here is the
//! lowering: which saved objects become which nodes in which scopes, what V1 drops silently
//! and V2 refuses by name, and that the corpus's send case no longer names its send.
use super::*;
use synth_engine_v2::ir::{ExecutionScope, IrNodeKind, PortId};
use synth_sequencer::{
    AutoInstrumentParam, AutomationLane, AutomationPoint, AutomationTarget, CurveType, PatternTick,
    ReturnBusId, ReturnSend, TrackParam, TrackSend,
};

use super::phase8::{
    corpus_inserts, corpus_with_inserts, instrument_with, notes_song, peak, project_profile,
    render, unity_master,
};
use crate::lowering::buses::{bus_slots, lower_buses};
use crate::lowering::identity::{BusSlot, InstrumentSlot, MASTER_MIX};
use crate::lowering::render::{OutputPolicy, smoke_render_project};
use crate::project::{GlobalProjectState, ReturnBusEffectsState};

/// The corpus's delay, fully wet, as a return's one effect: a return effect at a mix below
/// one would pass the summed sends through undelayed beside the line.
fn delay_effect() -> ModuleState {
    let mut delay = corpus_inserts().instruments[0]
        .patch
        .modules
        .iter()
        .find(|module| module.id == "dly-1")
        .cloned()
        .expect("the corpus insert case has a delay");
    delay
        .parameters
        .insert("mix".to_owned(), ParamValue::Float(1.0));
    delay
}

/// One instrument on the corpus patch, one track with four notes and one send into one
/// return that runs the corpus delay: dry beside a delayed wet path.
pub(super) fn returned_project(
    level: f32,
    pre_fader: bool,
) -> (
    Vec<crate::patch::InstrumentState>,
    synth_sequencer::Song,
    GlobalProjectState,
) {
    let mut song = four_note_song();
    let bus = song.create_return_bus("Wet");
    for track in song.tracks_mut() {
        track.sends.push(TrackSend {
            target: bus,
            level: synth_core::NormalizedValue::new(level),
            pre_fader,
            enabled: true,
        });
    }
    let global = GlobalProjectState {
        return_bus_effects: vec![ReturnBusEffectsState {
            id: bus.0,
            effects: vec![delay_effect()],
        }],
        ..unity_master()
    };
    (vec![instrument_with(0, 1.0)], song, global)
}

fn refusals(rendered: &super::super::render::SmokeRender) -> Vec<&LoweringDiagnostic> {
    rendered
        .diagnostics
        .iter()
        .filter(|d| d.severity() == Severity::Refused)
        .collect()
}

fn names(rendered: &super::super::render::SmokeRender, needle: &str) -> bool {
    rendered.diagnostics.iter().any(|d| match d.reason() {
        LoweringReason::OwnedByLaterPhase { capability, .. } => capability.contains(needle),
        LoweringReason::UnresolvedEndpoint { spelling } => spelling.contains(needle),
        LoweringReason::UnsupportedParameterValue { value } => value.contains(needle),
        _ => false,
    })
}

/// The corpus's send case: the send is lowered and no longer named; what stays refused is
/// the return's reverb; the master compressor is supported.
#[test]
fn the_corpus_send_case_no_longer_names_its_send() {
    let project = corpus_project("sends-returns-master");
    let rendered = smoke_render_project(
        &project.instruments,
        &project.song,
        &project.global,
        project_profile(),
        FrameCount::new(4_800),
        OutputPolicy::Parity,
    );
    assert!(!names(&rendered, "send"), "{:?}", rendered.diagnostics);
    let refused = refusals(&rendered);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(matches!(
        (refused[0].subject(), refused[0].reason()),
        (
            ProjectSubject::ReturnBusModule { .. },
            LoweringReason::UnsupportedModuleType {
                module_type: ModuleType::Reverb
            }
        )
    ));
    // Removing the supported master leaves the same refusal — by type, on the return's
    // own effect — and still nothing names the send.
    let mut without_master = project.global.clone();
    without_master.master_effects.clear();
    let rendered = smoke_render_project(
        &project.instruments,
        &project.song,
        &without_master,
        project_profile(),
        FrameCount::new(4_800),
        OutputPolicy::Parity,
    );
    assert!(!names(&rendered, "send"), "{:?}", rendered.diagnostics);
    let refused = refusals(&rendered);
    assert!(refused.iter().all(|d| matches!(
        (d.subject(), d.reason()),
        (
            ProjectSubject::ReturnBusModule { bus, .. },
            LoweringReason::UnsupportedModuleType {
                module_type: ModuleType::Reverb
            }
        ) if *bus == ReturnBusId::new(0)
    )));
    assert_eq!(refused.len(), 1, "{refused:?}");
}

/// A post-fader send into a delayed return renders the wet path beside the dry one: the
/// return muted alone changes the render, and so does the send at zero — and those two
/// absences are the same render, bit for bit (`CORPUS-0004-P1`'s shape).
#[test]
fn a_post_fader_send_into_a_delayed_return_renders_wet_beside_dry() {
    let (instruments, song, global) = returned_project(0.6, false);
    let full = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(full.is_audible(), "{:?}", full.diagnostics);
    assert!(refusals(&full).is_empty(), "{:?}", full.diagnostics);

    let mut muted_return = song.clone();
    muted_return
        .return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .mute = true;
    let dry_by_mute = render(&instruments, &muted_return, &global, OutputPolicy::Parity);

    let (_, silent_send, _) = returned_project(0.0, false);
    let dry_by_send = render(&instruments, &silent_send, &global, OutputPolicy::Parity);

    assert_ne!(full.samples, dry_by_mute.samples, "the return contributes");
    assert_ne!(full.samples, dry_by_send.samples, "the send contributes");
    assert_eq!(
        dry_by_mute.samples, dry_by_send.samples,
        "a muted return and a send at zero are the same absence"
    );
    // The wet path is delayed: the corpus delay's 0.25 s line at 48 kHz is 12 000 frames,
    // and the first note starts at frame 0, so the two renders agree exactly until the
    // line's first read.
    let first_difference = full
        .samples
        .iter()
        .zip(&dry_by_mute.samples)
        .position(|(a, b)| a != b)
        .expect("the wet path differs somewhere");
    assert!(
        first_difference / 2 >= 12_000,
        "the wet path arrives after the delay line, not before: frame {}",
        first_difference / 2
    );
}

/// The lowered graph has V1's shape: the send in the channel's scope reading what the
/// strip reads and entering the return's entry; the return's entry, delay, strip and
/// clipper in the bus's scope, in that order, into the master.
#[test]
fn the_bus_graph_is_lowered_in_v1s_shape_and_scopes() {
    use super::super::graph::{GraphAccumulator, InstrumentStages, Sink, lower_instrument_into};
    let (instruments, song, global) = returned_project(0.6, false);
    let saved = &instruments[0];
    let mut diagnostics = Vec::new();
    let slots = bus_slots(&song, &mut diagnostics).expect("one return");
    let sends = crate::lowering::buses::instrument_sends(
        saved,
        &song,
        &slots,
        harness_profile().limits().mixing().max_sends_per_channel(),
        false,
        &mut diagnostics,
    )
    .expect("one send");
    assert_eq!(sends.len(), 1);
    let mut graph = GraphAccumulator::default();
    let slot = InstrumentSlot::of(saved.id).expect("fits");
    let bus = BusSlot::of(ReturnBusId::new(0)).expect("fits");
    let stages = InstrumentStages {
        velocity: Some(synth_engine_v2::quantities::NormalizedLevel::FULL),
        track: Some(super::super::graph::TrackStage {
            level: synth_engine_v2::quantities::Amplitude::UNITY,
            pan: synth_engine_v2::controller::BipolarLevel::ZERO,
            muted: false,
        }),
        channel: Some(super::super::graph::ChannelStrip {
            fader: synth_engine_v2::quantities::Amplitude::new(0.7).expect("finite"),
            pan: synth_engine_v2::controller::BipolarLevel::new(0.2).expect("in range"),
            muted: false,
        }),
        soft_clip: true,
        headroom: false,
        sends,
    };
    let lowered = lower_instrument_into(
        &mut graph,
        saved.id,
        &saved.patch.modules,
        &saved.patch.connections,
        &saved.patch.settings.effect_chain_order,
        stages,
        &super::super::modulation::SongModulators::default(),
        Sink::Node(MASTER_MIX),
    );
    assert!(!lowered.refused, "{:?}", lowered.diagnostics);
    assert!(!lower_buses(
        &mut graph,
        &song,
        &global,
        OutputPolicy::Parity,
        &slots,
        MASTER_MIX,
        &mut diagnostics,
    ));
    graph
        .node(MASTER_MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .expect("free");
    let ir = graph
        .build(
            super::super::graph::voice_tuning().expect("12-TET"),
            super::super::graph::plan_declarations(
                synth_engine_v2::quantities::HeldNoteCount::measured(1),
                synth_engine_v2::quantities::EventCount::NONE,
            ),
        )
        .expect("builds");
    let kind = |id| ir.node(id).map(|node| node.kind()).expect("present");
    let scope = |id| ir.node(id).map(|node| node.scope()).expect("present");
    let feeds = |from, to| {
        ir.edges()
            .iter()
            .any(|edge| edge.from() == (from, PortId::FIRST) && edge.to() == (to, PortId::FIRST))
    };
    let source_of = |to| {
        ir.edges()
            .iter()
            .find(|edge| edge.to() == (to, PortId::FIRST))
            .map(|edge| edge.from().0)
    };

    // The send: the channel's scope, the strip's bases and the send's level, reading what
    // the strip reads, into the return's entry.
    let send = slot.send(0);
    assert_eq!(scope(send), ExecutionScope::Channel(slot.channel_tag()));
    match kind(send) {
        IrNodeKind::PostFaderSend {
            fader,
            pan,
            muted,
            level,
        } => {
            assert_eq!(fader.as_f32(), 0.7);
            assert_eq!(pan.as_f32(), 0.2);
            assert!(!muted);
            assert_eq!(level.as_f32(), 0.6);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(source_of(send), source_of(slot.channel()));
    assert_eq!(source_of(send), Some(slot.balance()));
    assert!(feeds(send, bus.entry()));

    // The bus: entry, the corpus delay at its own address, the strip with V1's return
    // fader, pan and mute, the clipper, into the master; every node in the bus's scope.
    let delay = bus
        .module(synth_engine::ModuleId {
            module_type: ModuleType::Delay,
            instance: 1,
        })
        .expect("fits");
    for (from, to) in [
        (bus.entry(), delay),
        (delay, bus.strip()),
        (bus.strip(), bus.soft_clip()),
        (bus.soft_clip(), MASTER_MIX),
    ] {
        assert!(feeds(from, to), "{from} must feed {to}");
    }
    for node in [bus.entry(), delay, bus.strip(), bus.soft_clip()] {
        assert_eq!(scope(node), ExecutionScope::Bus(bus.tag()), "{node}");
    }
    assert!(matches!(kind(bus.entry()), IrNodeKind::Mix));
    assert!(matches!(kind(delay), IrNodeKind::Delay { .. }));
    assert!(matches!(
        kind(bus.strip()),
        IrNodeKind::Channel { fader, pan, muted: false }
            if fader == synth_engine_v2::quantities::Amplitude::UNITY && pan.as_f32() == 0.0
    ));
    assert!(matches!(kind(bus.soft_clip()), IrNodeKind::SoftClip));
    // And nothing else enters the master: the channel's clipper and the bus's.
    let into_master: Vec<_> = ir
        .edges()
        .iter()
        .filter(|edge| edge.to().0 == MASTER_MIX)
        .map(|edge| edge.from().0)
        .collect();
    assert_eq!(into_master, vec![slot.soft_clip(), bus.soft_clip()]);
}

/// A pre-fader send lowers to a `Send` reading before the fader, muted where the
/// instrument is not audible; a muted instrument's send therefore taps nothing, as V1's.
#[test]
fn a_pre_fader_send_reads_before_the_fader_and_a_muted_instrument_sends_nothing() {
    let (instruments, song, global) = returned_project(0.6, true);
    let full = render(&instruments, &song, &global, OutputPolicy::Headroom);
    assert!(full.is_audible(), "{:?}", full.diagnostics);
    // The fader does not reach a pre-fader tap. The wet path alone is the render at a
    // fader of zero, since the dry path is then exactly nothing; under the headroom policy
    // the master is one float sum in identity order — the dry channel seeded, the return
    // accumulated — so at any fader the render is the dry path alone plus that same wet
    // path, bit for bit.
    let mut muted_return = song.clone();
    muted_return
        .return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .mute = true;
    let wet_alone = render(
        &[instrument_with(0, 0.0)],
        &song,
        &global,
        OutputPolicy::Headroom,
    );
    assert!(wet_alone.is_audible(), "{:?}", wet_alone.diagnostics);
    for fader in [1.0_f32, 0.5] {
        let full = render(
            &[instrument_with(0, fader)],
            &song,
            &global,
            OutputPolicy::Headroom,
        );
        let dry = render(
            &[instrument_with(0, fader)],
            &muted_return,
            &global,
            OutputPolicy::Headroom,
        );
        let composed: Vec<f32> = dry
            .samples
            .iter()
            .zip(&wet_alone.samples)
            .map(|(d, w)| d + w)
            .collect();
        assert_eq!(full.samples, composed, "fader {fader}");
    }
    // A post-fader tap does read it: at a fader of zero its wet path is nothing at all.
    let (_, post_song, post_global) = returned_project(0.6, false);
    let post_wet_alone = render(
        &[instrument_with(0, 0.0)],
        &post_song,
        &post_global,
        OutputPolicy::Headroom,
    );
    assert!(post_wet_alone.samples.iter().all(|s| *s == 0.0));

    // A muted instrument: V1 taps nothing from a channel that is not audible, pre-fader or
    // post-fader, so the whole render is silence.
    let mut muted = instrument_with(0, 1.0);
    muted.muted = true;
    let silent = render(&[muted], &song, &global, OutputPolicy::Parity);
    assert!(refusals(&silent).is_empty(), "{:?}", silent.diagnostics);
    assert!(silent.samples.iter().all(|s| *s == 0.0));
}

/// What V1 drops silently is refused by name: more resolved sends than the cap — counted
/// **before** zero-level sends are dropped, since a zero-level send occupies one of V1's
/// slots — a send into a return the song does not declare, a return's send into itself,
/// and a return whose identity is past the address space.
#[test]
fn sends_are_refused_by_name_where_v1_dropped_them() {
    // Sixteen resolved sends at zero and one audible seventeenth: V1 keeps the sixteen
    // zeros and drops the audible one; refused here naming the cap and LIMIT-0024.
    let (instruments, mut song, global) = returned_project(0.6, false);
    for track in song.tracks_mut() {
        let audible = track.sends.pop().expect("the fixture's send");
        for _ in 0..16 {
            track.sends.push(TrackSend {
                level: synth_core::NormalizedValue::MIN,
                ..audible
            });
        }
        track.sends.push(audible);
    }
    let over = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(over.samples.is_empty());
    assert!(names(&over, "LIMIT-0024"), "{:?}", over.diagnostics);
    assert!(names(&over, "17 resolved sends"), "{:?}", over.diagnostics);
    // Sixteen resolved, the audible one among them, lower.
    for track in song.tracks_mut() {
        track.sends.remove(0);
    }
    let at_cap = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(at_cap.is_audible(), "{:?}", at_cap.diagnostics);
    // The fifteen zero-level sends occupied V1's slots and lower to nothing: the plan holds
    // the one audible send.
    let sends = at_cap
        .report
        .as_ref()
        .and_then(|report| report.row(synth_engine_v2::report::ResourceField::MaxSendsPerChannel))
        .map(|row| row.requested())
        .expect("the sends row");
    assert_eq!(
        sends,
        synth_engine_v2::report::ResourceAmount::Sends(
            synth_engine_v2::quantities::SendCount::measured(1)
        )
    );

    // A send into a return the song does not declare.
    let (instruments, mut song, global) = returned_project(0.6, false);
    for track in song.tracks_mut() {
        track.sends[0].target = ReturnBusId::new(7);
    }
    let dangling = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(dangling.samples.is_empty());
    assert!(
        names(&dangling, "which the song does not declare"),
        "{:?}",
        dangling.diagnostics
    );

    // A return's send into itself.
    let (instruments, mut song, global) = returned_project(0.6, false);
    song.return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .sends
        .push(ReturnSend::new(
            ReturnBusId::new(0),
            synth_core::NormalizedValue::new(0.5),
        ));
    let looped = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(looped.samples.is_empty());
    assert!(names(&looped, "into itself"), "{:?}", looped.diagnostics);

    // A return's send into a return the song does not declare.
    let (instruments, mut song, global) = returned_project(0.6, false);
    song.return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .sends
        .push(ReturnSend::new(
            ReturnBusId::new(9),
            synth_core::NormalizedValue::new(0.5),
        ));
    let dangling = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(dangling.samples.is_empty());
    assert!(names(&dangling, "which the song does not declare"));

    // A return past the address space.
    let (instruments, mut song, global) = returned_project(0.6, false);
    song.return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .id = ReturnBusId::new(256);
    for track in song.tracks_mut() {
        track.sends[0].target = ReturnBusId::new(256);
    }
    let past = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(past.samples.is_empty());
    assert!(names(&past, "addressable range"), "{:?}", past.diagnostics);
}

/// V1 keys its send lists by instrument and takes the last assigned track's list, an empty
/// track included: two assigned tracks with differing lists are refused naming ADR-0034,
/// and two with equal lists lower as one.
#[test]
fn an_instrument_assigned_to_two_tracks_with_differing_sends_is_refused_by_name() {
    let (instruments, mut song, global) = returned_project(0.6, false);
    // A second, note-free track on the same instrument with no sends: V1 would render dry.
    let empty = song.create_track("empty");
    song.track_mut(empty).expect("resolves").instrument = instruments[0].id;
    let differing = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(differing.samples.is_empty());
    assert!(
        differing.diagnostics.iter().any(|d| matches!(
            (d.severity(), d.subject(), d.reason()),
            (
                Severity::Refused,
                ProjectSubject::Track { track, .. },
                LoweringReason::OwnedByLaterPhase { capability, owner }
            ) if *track == empty && capability.contains("differing sends") && owner.contains("ADR-0034")
        )),
        "{:?}",
        differing.diagnostics
    );
    // The same list on both: one list.
    let sends = song
        .tracks()
        .next()
        .expect("the playing track")
        .sends
        .clone();
    song.track_mut(empty).expect("resolves").sends = sends;
    let equal = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(equal.is_audible(), "{:?}", equal.diagnostics);
    assert!(refusals(&equal).is_empty());
}

/// A return soloed elsewhere keeps the unsoloed return off the master and leaves its
/// bus-to-bus send flowing, as V1's return solo gates the master sum alone.
#[test]
fn a_return_soloed_elsewhere_drops_the_master_cable_and_keeps_the_bus_send() {
    let (instruments, mut song, mut global) = returned_project(0.6, false);
    let second = song.create_return_bus("Second");
    global.return_bus_effects.push(ReturnBusEffectsState {
        id: second.0,
        effects: Vec::new(),
    });
    // The first return sends into the second at unity.
    song.return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .sends
        .push(ReturnSend::new(second, synth_core::NormalizedValue::MAX));
    let both = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(both.is_audible(), "{:?}", both.diagnostics);

    // Solo the second: the first's output reaches the master through the second alone.
    song.return_bus_mut(second).expect("declared").solo = true;
    let soloed = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(refusals(&soloed).is_empty(), "{:?}", soloed.diagnostics);
    let mut diagnostics = Vec::new();
    let slots = bus_slots(&song, &mut diagnostics).expect("two returns");
    let mut graph = super::super::graph::GraphAccumulator::default();
    assert!(!lower_buses(
        &mut graph,
        &song,
        &global,
        OutputPolicy::Parity,
        &slots,
        MASTER_MIX,
        &mut diagnostics,
    ));
    graph
        .node(MASTER_MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .expect("free");
    let ir = graph
        .build(
            super::super::graph::voice_tuning().expect("12-TET"),
            super::super::graph::plan_declarations(
                synth_engine_v2::quantities::HeldNoteCount::measured(1),
                synth_engine_v2::quantities::EventCount::NONE,
            ),
        )
        .expect("builds");
    let first = BusSlot::of(ReturnBusId::new(0)).expect("fits");
    let second_slot = BusSlot::of(second).expect("fits");
    let feeds = |from, to| {
        ir.edges()
            .iter()
            .any(|edge| edge.from().0 == from && edge.to().0 == to)
    };
    assert!(
        !feeds(first.soft_clip(), MASTER_MIX),
        "soloed out of the master"
    );
    assert!(feeds(first.soft_clip(), first.send(0).expect("fits")));
    assert!(feeds(first.send(0).expect("fits"), second_slot.entry()));
    assert!(feeds(second_slot.soft_clip(), MASTER_MIX));
    // The dry channel is not gated by a return solo, and the first return still sounds
    // through the second: the soloed render differs from both the unsoloed one and the
    // dry one.
    let mut muted = song.clone();
    muted
        .return_bus_mut(ReturnBusId::new(0))
        .expect("declared")
        .mute = true;
    muted.return_bus_mut(second).expect("declared").mute = true;
    let dry = render(&instruments, &muted, &global, OutputPolicy::Parity);
    assert_ne!(soloed.samples, both.samples);
    assert_ne!(soloed.samples, dry.samples);
}

/// A return running an effect V2 has not carried is still refused, by type and by the
/// return's subject; a return with no effect and no send lowers as V1 creates it.
#[test]
fn a_return_with_an_uncarried_effect_is_refused_by_type_and_an_empty_one_lowers() {
    let (instruments, song, mut global) = returned_project(0.6, false);
    global.return_bus_effects[0].effects = vec![module("rev-1", ModuleType::Reverb)];
    let refused = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(refused.samples.is_empty());
    assert!(refused.diagnostics.iter().any(|d| matches!(
        (d.subject(), d.reason()),
        (
            ProjectSubject::ReturnBusModule { .. },
            LoweringReason::UnsupportedModuleType {
                module_type: ModuleType::Reverb
            }
        )
    )));

    let mut song = four_note_song();
    let _ = song.create_return_bus("Idle");
    let idle = render(
        &[instrument_with(0, 1.0)],
        &song,
        &unity_master(),
        OutputPolicy::Parity,
    );
    assert!(idle.is_audible(), "{:?}", idle.diagnostics);
    let plain = render(
        &[instrument_with(0, 1.0)],
        &four_note_song(),
        &unity_master(),
        OutputPolicy::Parity,
    );
    assert_eq!(
        idle.samples, plain.samples,
        "an unfed return adds exactly nothing to the master"
    );
}

/// Two instruments on one patch, two tracks, each with a post-fader send into its own
/// return, both returns running the corpus delay: the survey's second synthetic fixture
/// (EVD-0021), and the shape the independent-sends test moves one send of.
pub(super) fn two_returned_project(
    first_level: f32,
) -> (
    Vec<crate::patch::InstrumentState>,
    synth_sequencer::Song,
    GlobalProjectState,
) {
    let mut song = super::phase8::two_instrument_song(1);
    let a = song.create_return_bus("A");
    let b = song.create_return_bus("B");
    let tracks: Vec<_> = song.tracks().map(|track| track.id).collect();
    song.track_mut(tracks[0])
        .expect("resolves")
        .sends
        .push(TrackSend::new(
            a,
            synth_core::NormalizedValue::new(first_level),
        ));
    song.track_mut(tracks[1])
        .expect("resolves")
        .sends
        .push(TrackSend::new(b, synth_core::NormalizedValue::new(0.5)));
    let global = GlobalProjectState {
        return_bus_effects: vec![
            ReturnBusEffectsState {
                id: a.0,
                effects: vec![delay_effect()],
            },
            ReturnBusEffectsState {
                id: b.0,
                effects: vec![delay_effect()],
            },
        ],
        ..unity_master()
    };
    (
        vec![instrument_with(0, 0.8), instrument_with(1, 0.8)],
        song,
        global,
    )
}

/// The first exit bullet's send clause from the project's side: two instruments on one
/// patch, each with its own send into its own return; moving one instrument's send leaves
/// the other route bit for bit.
#[test]
fn two_instruments_on_one_patch_keep_independent_sends() {
    let song_with_sends = |first_level: f32| {
        let mut song = super::phase8::two_instrument_song(1);
        let a = song.create_return_bus("A");
        let b = song.create_return_bus("B");
        let tracks: Vec<_> = song.tracks().map(|track| track.id).collect();
        song.track_mut(tracks[0])
            .expect("resolves")
            .sends
            .push(TrackSend::new(
                a,
                synth_core::NormalizedValue::new(first_level),
            ));
        song.track_mut(tracks[1])
            .expect("resolves")
            .sends
            .push(TrackSend::new(b, synth_core::NormalizedValue::new(0.5)));
        (song, a, b)
    };
    let global = |a: ReturnBusId, b: ReturnBusId| GlobalProjectState {
        return_bus_effects: vec![
            ReturnBusEffectsState {
                id: a.0,
                effects: vec![delay_effect()],
            },
            ReturnBusEffectsState {
                id: b.0,
                effects: vec![delay_effect()],
            },
        ],
        ..unity_master()
    };
    let instruments = [instrument_with(0, 0.8), instrument_with(1, 0.8)];
    // B's route alone: A muted.
    let route_b = |first_level: f32| {
        let (mut song, a, b) = song_with_sends(first_level);
        song.return_bus_mut(a).expect("declared").mute = true;
        let rendered = render(&instruments, &song, &global(a, b), OutputPolicy::Headroom);
        assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
        rendered.samples
    };
    assert_eq!(
        route_b(0.6),
        route_b(0.2),
        "B's route does not read A's send"
    );
    // And A's route does move.
    let route_a = |first_level: f32| {
        let (mut song, a, b) = song_with_sends(first_level);
        song.return_bus_mut(b).expect("declared").mute = true;
        render(&instruments, &song, &global(a, b), OutputPolicy::Headroom).samples
    };
    assert_ne!(route_a(0.6), route_a(0.2));
}

/// V1 sums the master's returns in its Kahn order and V2 in identity order: three returns
/// where a bus-to-bus send reorders the walk are a marked difference on the master, and
/// three sends into one return out of V1's order are one on that return.
#[test]
fn a_return_order_v1_walks_differently_is_marked_at_three() {
    let order_mark = |rendered: &super::super::render::SmokeRender, project: bool| {
        rendered.diagnostics.iter().any(|d| {
            matches!(
                (d.subject(), d.reason()),
                (subject, LoweringReason::OwnedByLaterPhase { capability, .. })
                    if capability.contains("Kahn order")
                        && (matches!(subject, ProjectSubject::Project) == project)
            )
        })
    };
    let (instruments, mut song, mut global) = returned_project(0.6, false);
    let b = song.create_return_bus("B");
    let c = song.create_return_bus("C");
    for id in [b, c] {
        global.return_bus_effects.push(ReturnBusEffectsState {
            id: id.0,
            effects: Vec::new(),
        });
    }
    // A, B, C with no bus-to-bus send: V1's order is index order, which is identity order.
    let plain = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(plain.is_audible(), "{:?}", plain.diagnostics);
    assert!(!order_mark(&plain, true));
    // C sends into A: V1 walks B, C, A; V2 sums A, B, C.
    song.return_bus_mut(c)
        .expect("declared")
        .sends
        .push(ReturnSend::new(
            ReturnBusId::new(0),
            synth_core::NormalizedValue::new(0.5),
        ));
    let walked = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(walked.is_audible(), "{:?}", walked.diagnostics);
    assert!(order_mark(&walked, true), "{:?}", walked.diagnostics);
    assert!(
        !order_mark(&walked, false),
        "two sends into A sum the same either way"
    );

    // Three instruments out of identity order each sending into one return: marked on the
    // return, beside the master's own mark.
    let mut song = four_note_song();
    let wet = song.create_return_bus("Wet");
    for track in song.tracks_mut() {
        track
            .sends
            .push(TrackSend::new(wet, synth_core::NormalizedValue::new(0.5)));
    }
    for id in [2_u64, 1] {
        let track = song.create_track("more");
        song.track_mut(track).expect("resolves").instrument =
            synth_engine::instrument::InstrumentId::new(id);
        song.track_mut(track)
            .expect("resolves")
            .sends
            .push(TrackSend::new(wet, synth_core::NormalizedValue::new(0.5)));
    }
    let global = GlobalProjectState {
        return_bus_effects: vec![ReturnBusEffectsState {
            id: wet.0,
            effects: Vec::new(),
        }],
        ..unity_master()
    };
    let out_of_order = render(
        &[
            instrument_with(2, 1.0),
            instrument_with(0, 1.0),
            instrument_with(1, 1.0),
        ],
        &song,
        &global,
        OutputPolicy::Parity,
    );
    assert!(out_of_order.is_audible(), "{:?}", out_of_order.diagnostics);
    assert!(
        order_mark(&out_of_order, false),
        "{:?}",
        out_of_order.diagnostics
    );
}

/// An instrument volume lane fans out to the channel's post-fader sends: a lane holding one
/// value from the first tick renders as the same value saved on the instrument, wet path
/// included, bit for bit.
#[test]
fn an_instrument_volume_lane_reaches_the_post_fader_send() {
    let (instruments, mut song, global) = returned_project(0.6, false);
    let pattern = song
        .arrangement()
        .first()
        .expect("the fixture places one pattern")
        .pattern_id;
    let mut lane = AutomationLane::new(AutomationTarget::Instrument {
        instrument: instruments[0].id,
        param: AutoInstrumentParam::Volume,
    });
    lane.add_point(
        AutomationPoint::new(PatternTick(0), synth_core::NormalizedValue::new(0.5))
            .with_curve(CurveType::Step),
    );
    song.pattern_mut(pattern)
        .expect("resolves")
        .add_automation_lane(lane);
    let by_lane = render(&instruments, &song, &global, OutputPolicy::Parity);
    assert!(by_lane.is_audible(), "{:?}", by_lane.diagnostics);
    let (_, plain_song, _) = returned_project(0.6, false);
    let by_fader = render(
        &[instrument_with(0, 0.5)],
        &plain_song,
        &global,
        OutputPolicy::Parity,
    );
    assert_eq!(by_lane.samples, by_fader.samples);
    // The control: the same lane on a project whose send does not see it would differ.
    let at_unity = render(&instruments, &plain_song, &global, OutputPolicy::Parity);
    assert_ne!(by_lane.samples, at_unity.samples);
}

/// A track mute lane zeroes the voices before the instrument's inserts, so an insert
/// delay's tail still feeds the send after the mute lands — V1's behaviour, which a
/// channel-mute gate on the send would have broken.
#[test]
fn a_track_mute_lane_leaves_the_insert_delays_tail_feeding_the_send() {
    let wet_after_mute = |project: &mut crate::project::ProjectFile| {
        let mut song = notes_song(&[(0, 60, 480)]);
        let bus = song.create_return_bus("Wet");
        for track in song.tracks_mut() {
            track
                .sends
                .push(TrackSend::new(bus, synth_core::NormalizedValue::MAX));
        }
        project.global.return_bus_effects = vec![ReturnBusEffectsState {
            id: bus.0,
            effects: Vec::new(),
        }];
        let pattern = song.arrangement().first().expect("placed").pattern_id;
        // Mute the track at tick 960 — half a second at 120 BPM, after the note.
        let mut lane = AutomationLane::new(AutomationTarget::Track {
            track: None,
            param: TrackParam::Mute,
        });
        // Unmuted from the first tick — a lane holds its first point's value before it —
        // and muted from tick 960.
        lane.add_point(
            AutomationPoint::new(PatternTick(0), synth_core::NormalizedValue::MIN)
                .with_curve(CurveType::Step),
        );
        lane.add_point(
            AutomationPoint::new(PatternTick(960), synth_core::NormalizedValue::MAX)
                .with_curve(CurveType::Step),
        );
        song.pattern_mut(pattern)
            .expect("resolves")
            .add_automation_lane(lane);
        let full = smoke_render_project(
            &project.instruments,
            &song,
            &project.global,
            project_profile(),
            FrameCount::new(48_000),
            OutputPolicy::Parity,
        );
        assert!(full.is_audible(), "{:?}", full.diagnostics);
        let mut muted_return = song.clone();
        muted_return.return_bus_mut(bus).expect("declared").mute = true;
        let dry = smoke_render_project(
            &project.instruments,
            &muted_return,
            &project.global,
            project_profile(),
            FrameCount::new(48_000),
            OutputPolicy::Parity,
        );
        let wet: Vec<f32> = full
            .samples
            .iter()
            .zip(&dry.samples)
            .map(|(f, d)| f - d)
            .collect();
        // The mute lands at 24 000 frames; the delay's 0.25 s line still rings into the send
        // for a while after it.
        peak(&wet[2 * 24_000 + 2 * 1_000..2 * 30_000])
    };
    assert!(
        wet_after_mute(&mut corpus_with_inserts(&["dly-1"])) > 0.0,
        "the delay's tail feeds the send past the mute"
    );
    // The control: without the insert, the muted voices leave the send nothing to tap.
    assert_eq!(
        wet_after_mute(&mut corpus_with_inserts(&[])),
        0.0,
        "with no insert the mute silences the send"
    );
}
