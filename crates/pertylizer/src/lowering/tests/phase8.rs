//! `P08-S002`: a whole project through one plan.
//!
//! Every instrument as its own channel into one master, V1's stages in V1's order, solo
//! across instruments, the track stage, the master volume, and the output policy — held by
//! renders of two instruments against renders of each alone, and by the refusals the slice
//! names.
use super::*;
use synth_engine_v2::ir::{ExecutionScope, IrNodeKind};
use synth_engine_v2::quantities::{EventCount, HeldNoteCount};
use synth_sequencer::{Duration, PatternTick, Pitch, Tick, Velocity};

use crate::lowering::identity::{
    InstrumentSlot, MASTER_CLAMP, MASTER_MIX, MASTER_OUTPUT, MASTER_TRIM,
};
use crate::lowering::render::{OutputPolicy, smoke_render_project};

/// The corpus patch as a second instrument, by identity, with its own strip.
fn instrument_with(id: u64, volume: f32) -> crate::patch::InstrumentState {
    let (modules, connections) = corpus_patch("sine");
    let mut saved = saved_instrument(modules, connections);
    saved.id = synth_engine::instrument::InstrumentId::new(id);
    saved.name = format!("Instrument {id}");
    saved.volume = synth_core::Gain::new(volume);
    saved
}

/// `four_note_song` for instrument 0, plus a second track for instrument `second` whose one
/// note overlaps the first instrument's first note, so the project holds two notes at once.
fn two_instrument_song(second: u64) -> synth_sequencer::Song {
    let mut song = four_note_song();
    let track = song.create_track("second");
    song.track_mut(track).expect("resolves").instrument =
        synth_engine::instrument::InstrumentId::new(second);
    let pattern = song.create_pattern(Duration(3840));
    {
        let pattern = song.pattern_mut(pattern).expect("resolves");
        let id = pattern.add_note(
            PatternTick(480),
            Pitch::new(67).expect("a keyboard position"),
            Velocity::new(0.6),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(720));
        }
    }
    assert!(song.place_pattern(pattern, track, Tick::ZERO));
    song
}

/// The harness profile with eight times the engine's default event partition.
///
/// The engine's default `session_event_share` is 24, and ADR-0051's catch-up charges one
/// row per parameter address in the plan: one lowered instrument's addresses fit and two do
/// not, so a whole project needs the roomier partition ADR-0054 reselects. Everything but
/// the events group is the default.
fn project_profile() -> HostProfile {
    project_profile_at(harness_profile())
}

/// [`project_profile`] over another base, for a rate the fixture chooses.
fn project_profile_at(base: HostProfile) -> HostProfile {
    use synth_engine_v2::profile::{EventLimits, ProducerShares, RenderLimits};
    let defaults = RenderLimits::engine_defaults(base.capabilities()).expect("defaults");
    let events = defaults.events();
    let shares = events.shares();
    let scale = |count: EventCount| EventCount::limit(count.get() * 8).expect("a capacity");
    let shares = ProducerShares::new(
        scale(shares.compiled_event_share()),
        scale(shares.authored_runtime_event_share()),
        scale(shares.live_event_share()),
        scale(shares.session_event_share()),
        scale(shares.internal_event_share()),
        scale(shares.release_event_share()),
        scale(shares.release_hold_capacity()),
    )
    .expect("scaled shares");
    let events = EventLimits::new(
        scale(events.max_events_per_quantum()),
        events.max_note_expansion_per_tick(),
        scale(events.max_scheduled_events_in_flight()),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("scaled events");
    let limits = RenderLimits::new(
        defaults.stream(),
        defaults.graph(),
        defaults.voices(),
        events,
        defaults.observation(),
        defaults.mixing(),
        defaults.memory(),
        defaults.script(),
        defaults.recording(),
        defaults.cost(),
    )
    .expect("consistent limits");
    HostProfile::new(base.capabilities(), limits).expect("a consistent profile")
}

fn unity_master() -> crate::project::GlobalProjectState {
    crate::project::GlobalProjectState {
        master_volume: synth_core::Gain::UNITY,
        ..Default::default()
    }
}

fn render(
    instruments: &[crate::patch::InstrumentState],
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
    policy: OutputPolicy,
) -> super::super::render::SmokeRender {
    smoke_render_project(
        instruments,
        song,
        global,
        project_profile(),
        FrameCount::new(4_800),
        policy,
    )
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()))
}

/// The first exit bullet: two channels on one patch definition keep their own faders,
/// voices and tails. The project is the sum of each instrument alone, exactly, with two
/// notes sounding at once across the two.
#[test]
fn two_instruments_on_one_patch_are_two_channels_summed_exactly() {
    let first = instrument_with(0, 1.0);
    let second = instrument_with(1, 0.5);
    let song = two_instrument_song(1);
    let global = unity_master();
    let both = render(
        &[first.clone(), second.clone()],
        &song,
        &global,
        OutputPolicy::Headroom,
    );
    assert!(both.is_audible(), "{:?}", both.diagnostics);
    let alone_first = render(&[first], &song, &global, OutputPolicy::Headroom);
    let alone_second = render(&[second], &song, &global, OutputPolicy::Headroom);
    assert!(alone_first.is_audible() && alone_second.is_audible());
    assert_eq!(both.samples.len(), alone_first.samples.len());
    assert_eq!(both.samples.len(), alone_second.samples.len());
    // The master sum seeds with the lower identity's cable and accumulates the other, and
    // the trim at unity and no clipper leave the sum as it is: each sample is exactly the
    // first alone plus the second alone.
    for ((b, f), s) in both
        .samples
        .iter()
        .zip(&alone_first.samples)
        .zip(&alone_second.samples)
    {
        assert_eq!(b.to_bits(), (f + s).to_bits(), "{b} != {f} + {s}");
    }
    // Both sound at once where the second's note overlaps the first's, and the second at
    // half the fader is quieter than the first: the faders are each channel's own.
    assert!(peak(&alone_second.samples) < peak(&alone_first.samples));
    let overlap = &both.samples[(12_000 * 2)..(18_000 * 2)];
    assert!(overlap.iter().any(|s| s.abs() > 0.0));
    // Two notes at once across the project, so the voice scope is instantiated twice.
    assert_eq!(both.lowered_events, EventCount::measured(8 + 2));
}

/// V1's instrument solo, across the project: a soloed instrument silences every unsoloed
/// one — the render is the soloed instrument alone — and an unsoloed project is the sum.
#[test]
fn an_instrument_soloed_elsewhere_silences_this_one() {
    let first = instrument_with(0, 1.0);
    let mut second = instrument_with(1, 1.0);
    let song = two_instrument_song(1);
    let global = unity_master();
    second.solo = true;
    let soloed = render(
        &[first.clone(), second.clone()],
        &song,
        &global,
        OutputPolicy::Parity,
    );
    assert!(soloed.is_audible(), "{:?}", soloed.diagnostics);
    second.solo = false;
    let alone_second = render(
        std::slice::from_ref(&second),
        &song,
        &global,
        OutputPolicy::Parity,
    );
    // Values rather than bits: a muted channel contributes `0.0`, and `0.0 + x` is `x` in
    // value for every `x`, while a `-0.0` sample would differ in its sign bit alone.
    assert_eq!(soloed.samples.len(), alone_second.samples.len());
    assert!(
        soloed.samples == alone_second.samples,
        "the soloed render is the second alone"
    );
    // The unsoloed first is muted, not dropped: its notes are still lowered.
    assert_eq!(soloed.lowered_events, EventCount::measured(8 + 2));
    // And nothing names the solo: silence V1 renders is silence rendered, not a mark. What
    // is named is Phase 8's two stages per instrument and the first's four notes through
    // one gate, exactly as without the solo.
    let unsoloed = render(&[first, second], &song, &global, OutputPolicy::Parity);
    assert_eq!(soloed.diagnostics, unsoloed.diagnostics);
    assert!(soloed.diagnostics.iter().all(|d| !matches!(
        d.reason(),
        LoweringReason::OwnedByLaterPhase { capability, .. }
            if capability.contains("solo") || capability.contains("mute")
    )));
}

/// The output policy: under parity a sum above full scale is soft-clipped per channel and
/// clamped at the output, and declining it preserves the headroom in float.
#[test]
fn the_parity_policy_clamps_at_full_scale_and_headroom_preserves_the_sum() {
    // Both faders at V1's mixer maximum, both instruments sounding at once.
    let instruments = [instrument_with(0, 2.0), instrument_with(1, 2.0)];
    let song = two_instrument_song(1);
    let global = unity_master();
    let parity = render(&instruments, &song, &global, OutputPolicy::Parity);
    let headroom = render(&instruments, &song, &global, OutputPolicy::Headroom);
    assert!(parity.is_audible() && headroom.is_audible());
    assert!(
        peak(&headroom.samples) > 1.0,
        "the fixture must sum past full scale: {}",
        peak(&headroom.samples)
    );
    assert!(peak(&parity.samples) <= 1.0);
    assert!(
        parity.samples.iter().any(|s| s.abs() == 1.0),
        "and the clamp holds it there"
    );
    // The two renders differ before the clamp too: each channel is soft-clipped at 0.8 under
    // parity, so a frame the headroom render holds between 0.8 and 1.0 is lower under parity.
    let shaped = headroom
        .samples
        .iter()
        .zip(&parity.samples)
        .filter(|(h, p)| h.abs() > 0.8 && h.abs() < 1.0 && p.abs() < h.abs())
        .count();
    assert!(
        shaped > 0,
        "V1's per-channel clipper shapes the sum below full scale"
    );
    assert_eq!(parity.diagnostics, headroom.diagnostics);
}

/// The project lowers into V1's chain per instrument — scaler and balance per voice, the
/// channel and the clipper on the sum — into one master sum, trim, clamp and output, with
/// the saved output module lowering to no node of its own.
#[test]
fn the_chain_is_v1s_order_into_one_master() {
    use super::super::graph::{GraphAccumulator, InstrumentStages, Sink, lower_instrument_into};
    let (modules, connections) = corpus_patch("sine");
    let mut graph = GraphAccumulator::default();
    let slot = InstrumentSlot::of(instrument()).expect("fits");
    let stages = InstrumentStages {
        velocity: Some(synth_engine_v2::quantities::NormalizedLevel::FULL),
        track: Some(super::super::graph::TrackStage {
            level: synth_engine_v2::quantities::Amplitude::new(0.5).expect("finite"),
            pan: synth_engine_v2::controller::BipolarLevel::new(-0.25).expect("in range"),
            muted: false,
        }),
        channel: Some(super::super::graph::ChannelStrip {
            fader: synth_engine_v2::quantities::Amplitude::UNITY,
            pan: synth_engine_v2::controller::BipolarLevel::ZERO,
            muted: false,
        }),
        soft_clip: true,
    };
    let lowered = lower_instrument_into(
        &mut graph,
        instrument(),
        &modules,
        &connections,
        stages,
        &super::super::modulation::SongModulators::default(),
        Sink::Node(MASTER_MIX),
    );
    assert!(!lowered.refused, "{:?}", lowered.diagnostics);
    for (id, kind, scope) in [
        (MASTER_MIX, IrNodeKind::Mix, ExecutionScope::Global),
        (
            MASTER_TRIM,
            IrNodeKind::Trim {
                level: synth_engine_v2::quantities::Amplitude::UNITY,
            },
            ExecutionScope::Global,
        ),
        (MASTER_CLAMP, IrNodeKind::HardClamp, ExecutionScope::Global),
        (MASTER_OUTPUT, IrNodeKind::Output, ExecutionScope::Global),
    ] {
        graph
            .node(id, kind, scope)
            .expect("the master's addresses are free");
    }
    for pair in [MASTER_MIX, MASTER_TRIM, MASTER_CLAMP, MASTER_OUTPUT].windows(2) {
        graph.connect(
            (pair[0], synth_engine_v2::ir::PortId::FIRST),
            (pair[1], synth_engine_v2::ir::PortId::FIRST),
            synth_engine_v2::ir::SignalDomain::Audio,
        );
    }
    let ir = graph
        .build(
            super::super::graph::voice_tuning().expect("prepares"),
            super::super::graph::plan_declarations(HeldNoteCount::measured(1), EventCount::NONE),
        )
        .expect("builds");
    let chain = [
        slot.voice_output_scaler(),
        slot.balance(),
        slot.channel(),
        slot.soft_clip(),
        MASTER_MIX,
    ];
    for pair in chain.windows(2) {
        assert!(
            ir.edges()
                .iter()
                .any(|edge| edge.from().0 == pair[0] && edge.to().0 == pair[1]),
            "{} must feed {}",
            pair[0],
            pair[1]
        );
    }
    let scope_of = |id| ir.node(id).map(|node| node.scope()).expect("present");
    assert_eq!(scope_of(slot.voice_output_scaler()), ExecutionScope::Voice);
    assert_eq!(
        scope_of(slot.balance()),
        ExecutionScope::Voice,
        "V1 applies the track control per voice"
    );
    assert_eq!(scope_of(slot.channel()), ExecutionScope::Channel);
    assert_eq!(scope_of(slot.soft_clip()), ExecutionScope::Channel);
    match ir.node(slot.balance()).map(|node| node.kind()) {
        Some(IrNodeKind::Balance { level, pan, muted }) => {
            assert_eq!(level.as_f32(), 0.5);
            assert_eq!(pan.as_f32(), -0.25);
            assert!(!muted);
        }
        other => panic!("{other:?}"),
    }
    // One output, the master's: the saved output module lowered to no node.
    let outputs: Vec<_> = ir
        .nodes()
        .iter()
        .filter(|node| matches!(node.kind(), IrNodeKind::Output))
        .map(|node| node.id())
        .collect();
    assert_eq!(outputs, vec![MASTER_OUTPUT]);
    // And the cable the patch draws into its output module enters the scaler.
    let out_module = ModuleId::new(ModuleType::StereoOutput, 1);
    assert!(
        ir.node(lowered.identities.node_for(out_module).expect("resolved"))
            .is_none()
    );
    assert!(
        ir.edges()
            .iter()
            .any(|edge| edge.to().0 == slot.voice_output_scaler()
                && lowered.identities.module_for(edge.from().0).is_some())
    );
    let plan = compile(&ir, &RenderConfig::new(harness_profile()))
        .into_plan()
        .expect("compiles");
    assert_eq!(
        plan.channels().len(),
        1,
        "one channel per instrument, the balance is not one"
    );
}

/// The corpus's shared-instrument project — one instrument on two tracks at differing
/// faders, V1's per-voice gain — is refused by name until ADR-0034; the same two tracks at
/// equal faders lower to one stage.
#[test]
fn a_shared_instrument_with_differing_track_controls_is_refused_by_name() {
    let project = corpus_project("shared-instrument-tracks");
    let rendered = smoke_render_project(
        &project.instruments,
        &project.song,
        &project.global,
        project_profile(),
        FrameCount::new(4_800),
        OutputPolicy::Parity,
    );
    assert!(rendered.samples.is_empty());
    assert!(
        rendered.diagnostics.iter().any(|d| matches!(
            (d.severity(), d.subject(), d.reason()),
            (
                Severity::Refused,
                ProjectSubject::Track { .. },
                LoweringReason::OwnedByLaterPhase { capability, owner }
            ) if capability.contains("differing track controls") && owner.contains("ADR-0034")
        )),
        "{:?}",
        rendered.diagnostics
    );
    // Equal faders: one stage, and the project lowers. `CORPUS-0010` places two simultaneous
    // note streams through the one instrument, which the one-gate rule still refuses; the
    // point here is which refusal names it.
    let mut song = project.song.clone();
    let faders: Vec<_> = song.tracks().map(|track| track.id).collect();
    for id in faders {
        song.track_mut(id).expect("resolves").volume = synth_core::NormalizedValue::MAX;
    }
    let equal = smoke_render_project(
        &project.instruments,
        &song,
        &project.global,
        project_profile(),
        FrameCount::new(4_800),
        OutputPolicy::Parity,
    );
    assert!(
        !equal.diagnostics.iter().any(|d| matches!(
            d.reason(),
            LoweringReason::OwnedByLaterPhase { owner, .. } if owner.contains("ADR-0034")
        )),
        "{:?}",
        equal.diagnostics
    );
}

/// The declared simultaneous notes are the project's peak across instruments, floored at
/// one and generous at a tie.
#[test]
fn the_declared_notes_are_the_projects_peak_across_instruments() {
    use super::super::performance::peak_concurrency;
    let ids = [
        synth_engine::instrument::InstrumentId::new(0),
        synth_engine::instrument::InstrumentId::new(1),
    ];
    // The fixture: the second instrument's note overlaps the first's.
    let rate = SampleRate::new(48_000.0).expect("a real rate");
    assert_eq!(
        peak_concurrency(&ids, &two_instrument_song(1), rate),
        HeldNoteCount::measured(2)
    );
    // One instrument, separated notes: one.
    assert_eq!(
        peak_concurrency(&ids[..1], &four_note_song(), rate),
        HeldNoteCount::measured(1)
    );
    // No notes at all: still one, the floor.
    assert_eq!(
        peak_concurrency(&ids, &song_with(120.0, &[], None), rate),
        HeldNoteCount::measured(1)
    );
    // Back to back across instruments — one ending where the other begins — counts two: the
    // events at one sample keep their emission order, so the note-on may be presented
    // before the release that would have freed the index.
    let mut song = four_note_song();
    let track = song.create_track("second");
    song.track_mut(track).expect("resolves").instrument = ids[1];
    let pattern = song.create_pattern(Duration(3840));
    {
        let pattern = song.pattern_mut(pattern).expect("resolves");
        let id = pattern.add_note(
            PatternTick(720),
            Pitch::new(67).expect("a keyboard position"),
            Velocity::new(0.6),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(240));
        }
    }
    assert!(song.place_pattern(pattern, track, Tick::ZERO));
    assert_eq!(
        peak_concurrency(&ids, &song, rate),
        HeldNoteCount::measured(2)
    );
    // And that project renders.
    let rendered = render(
        &[instrument_with(0, 1.0), instrument_with(1, 1.0)],
        &song,
        &unity_master(),
        OutputPolicy::Parity,
    );
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
}

/// Three or more instruments saved out of identity order are a marked summation-order
/// difference; two, or three in order, are not.
#[test]
fn instruments_out_of_identity_order_are_marked_at_three() {
    let song = four_note_song();
    let global = unity_master();
    let names_order = |rendered: &super::super::render::SmokeRender| {
        rendered.diagnostics.iter().any(|d| {
            matches!(
                d.reason(),
                LoweringReason::OwnedByLaterPhase { capability, .. }
                    if capability.contains("identity order")
            )
        })
    };
    let (a, b, c) = (
        instrument_with(0, 1.0),
        instrument_with(1, 1.0),
        instrument_with(2, 1.0),
    );
    assert!(!names_order(&render(
        &[b.clone(), a.clone()],
        &song,
        &global,
        OutputPolicy::Parity
    )));
    assert!(!names_order(&render(
        &[a.clone(), b.clone(), c.clone()],
        &song,
        &global,
        OutputPolicy::Parity
    )));
    let marked = render(&[c, a, b], &song, &global, OutputPolicy::Parity);
    assert!(marked.is_audible());
    assert!(names_order(&marked));
}

/// An instrument identity past the address space is refused by name, not folded into
/// another's addresses.
#[test]
fn an_instrument_beyond_the_address_space_is_refused_by_name() {
    let rendered = render(
        &[instrument_with(127, 1.0)],
        &four_note_song(),
        &unity_master(),
        OutputPolicy::Parity,
    );
    assert!(rendered.samples.is_empty());
    assert!(
        rendered.diagnostics.iter().any(|d| matches!(
            (d.severity(), d.reason()),
            (Severity::Refused, LoweringReason::UnresolvedEndpoint { spelling })
                if spelling.contains("addressable range")
        )),
        "{:?}",
        rendered.diagnostics
    );
}

/// The velocity macro and the inserted channel no longer share an address: a patch reading
/// velocity through its Mod Matrix lowers beside its channel, which `P08-S001`'s reserved
/// addresses refused as a duplicate node.
#[test]
fn a_velocity_macro_lowers_beside_the_channel() {
    let (mut modules, connections) = corpus_patch("sawtooth");
    modules.push(mod_matrix(
        "mmx-1",
        &[(synth_core::MacroSource::Velocity.id(), "flt-1.cutoff", 0.5)],
    ));
    let saved = saved_instrument(modules, connections);
    let rendered = render(
        std::slice::from_ref(&saved),
        &four_note_song(),
        &unity_master(),
        OutputPolicy::Parity,
    );
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
}

/// A note whose two edges round to one sample — a one-tick note at 8 kHz and 1000 BPM —
/// keeps its on edge before its own release and renders, where ordering every release
/// before every note-on at one sample aborted the render as an unmatched release. An
/// independent read found it.
#[test]
fn a_note_whose_edges_round_to_one_sample_renders() {
    use synth_engine_v2::offline::render_offline;
    use synth_engine_v2::time::PlanPosition;
    let (modules, connections) = corpus_patch("sawtooth");
    let rate = SampleRate::new(8_000.0).expect("a real rate");
    let song = song_with(1000.0, &[(1, 1), (400, 1)], None);
    let (plan, performance) = lowered_performance_at(&modules, &connections, &song, rate);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    assert_eq!(performance.events.len(), 4);
    let first_two: Vec<_> = performance.events[..2]
        .iter()
        .map(|event| event.time())
        .collect();
    assert_eq!(
        first_two[0], first_two[1],
        "the fixture's two edges share a sample"
    );
    let frames = FrameCount::new(performance.frames.as_u64() + 800);
    let rendered = render_offline(plan, frames, PlanPosition::ZERO, &performance.events);
    assert!(rendered.is_ok(), "{rendered:?}");
}

/// The one-instrument lowering has no balance stage and, without the channel, no place for
/// an instrument volume lane: a lane on either is refused by name rather than dropped as
/// inert. An independent read found both dropped.
#[test]
fn a_standalone_lowering_refuses_a_lane_on_a_stage_it_did_not_insert() {
    use synth_sequencer::{
        AutoInstrumentParam, AutomationLane, AutomationPoint, AutomationTarget, PatternTick,
        TrackParam,
    };
    let (modules, connections) = corpus_patch("sawtooth");
    let with_lane = |target: AutomationTarget| {
        let mut song = four_note_song();
        let pattern = placed_pattern(&song);
        let mut lane = AutomationLane::new(target);
        lane.add_point(AutomationPoint::new(
            PatternTick(0),
            synth_core::NormalizedValue::new(0.5),
        ));
        song.pattern_mut(pattern)
            .expect("resolves")
            .add_automation_lane(lane);
        song
    };
    let refusal = |performance: &super::super::performance::LoweredPerformance| {
        performance
            .diagnostics
            .iter()
            .find_map(|d| match (d.severity(), d.reason()) {
                (Severity::Refused, LoweringReason::OwnedByLaterPhase { capability, .. }) => {
                    Some(*capability)
                }
                _ => None,
            })
    };
    // A track lane on the playing track, with no balance stage in the plan.
    let song = with_lane(AutomationTarget::Track {
        track: None,
        param: TrackParam::Volume,
    });
    let (_, performance) = lowered_performance(&modules, &connections, &song);
    assert!(
        refusal(&performance).is_some_and(|c| c.contains("without the track's balance stage")),
        "{:?}",
        performance.diagnostics
    );
    // An instrument volume lane, with the channel: lowered, one write and no restore.
    let song = with_lane(AutomationTarget::Instrument {
        instrument: instrument(),
        param: AutoInstrumentParam::Volume,
    });
    let (plan, performance) = lowered_performance(&modules, &connections, &song);
    assert!(!performance.refused(), "{:?}", performance.diagnostics);
    assert_eq!(performance.events.len(), 8 + 1);
    // The same lane against targets resolved without the channel: refused by name.
    let lowered = super::super::graph::lower_voice_patch(
        instrument(),
        &modules,
        &connections,
        EventCount::NONE,
    );
    let bare = super::super::performance::AutomationTargets::resolve(&lowered.identities);
    let gate = lowered
        .identities
        .pairs()
        .find(|(id, _)| id.module_type == ModuleType::Envelope)
        .map(|(_, node)| node)
        .expect("an envelope");
    let performance = super::super::performance::lower_performance(
        instrument(),
        "Subtractive Voice",
        &song,
        &plan,
        gate,
        &bare,
        SampleRate::new(48_000.0).expect("a real rate"),
    );
    assert!(
        refusal(&performance).is_some_and(|c| c.contains("without the instrument's channel")),
        "{:?}",
        performance.diagnostics
    );
}

/// Two ticks that round to one sample are one sample to the minter: at 8 kHz and 1000 BPM,
/// ticks 1 and 2 are both sample 1, so an instrument's note ending at tick 1 and another's
/// beginning at tick 2 meet at one sample, where the first instrument's note-on is presented
/// before the second's release. Counted over sample positions the declaration is two and the
/// project renders; counted over ticks it was one and the render was refused. An
/// independent read found it.
#[test]
fn concurrency_is_counted_over_sample_positions_not_ticks() {
    use super::super::performance::peak_concurrency;
    let ids = [
        synth_engine::instrument::InstrumentId::new(0),
        synth_engine::instrument::InstrumentId::new(1),
    ];
    let rate = SampleRate::new(8_000.0).expect("a real rate");
    // Instrument 0: ticks 2..20 on the fixture's track; instrument 1: ticks 0..1.
    let mut song = song_with(1000.0, &[(2, 18)], None);
    let track = song.create_track("second");
    song.track_mut(track).expect("resolves").instrument = ids[1];
    let pattern = song.create_pattern(Duration(3840));
    {
        let pattern = song.pattern_mut(pattern).expect("resolves");
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(67).expect("a keyboard position"),
            Velocity::new(0.6),
        );
        if let Some(note) = pattern.note_mut(id) {
            note.duration = Some(Duration(1));
        }
    }
    assert!(song.place_pattern(pattern, track, Tick::ZERO));
    assert_eq!(
        peak_concurrency(&ids, &song, rate),
        HeldNoteCount::measured(2)
    );
    let profile = project_profile_at(
        HostProfile::harness(
            rate,
            FrameCount::new(512),
            synth_engine_v2::quantities::ChannelLayout::Stereo,
        )
        .expect("a harness profile"),
    );
    let rendered = smoke_render_project(
        &[instrument_with(0, 1.0), instrument_with(1, 1.0)],
        &song,
        &unity_master(),
        profile,
        FrameCount::new(800),
        OutputPolicy::Parity,
    );
    assert!(rendered.is_audible(), "{:?}", rendered.diagnostics);
    assert_eq!(rendered.lowered_events, EventCount::measured(4));
}
