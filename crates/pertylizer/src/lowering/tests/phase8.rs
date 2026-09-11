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
pub(super) fn instrument_with(id: u64, volume: f32) -> crate::patch::InstrumentState {
    let (modules, connections) = corpus_patch("sine");
    let mut saved = saved_instrument(modules, connections);
    saved.id = synth_engine::instrument::InstrumentId::new(id);
    saved.name = format!("Instrument {id}");
    saved.volume = synth_core::Gain::new(volume);
    saved
}

/// `four_note_song` for instrument 0, plus a second track for instrument `second` whose one
/// note overlaps the first instrument's first note, so the project holds two notes at once.
pub(super) fn two_instrument_song(second: u64) -> synth_sequencer::Song {
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
/// ADR-0051's catch-up charges one row per parameter address in the plan, and the engine's
/// first provisional `session_event_share` (24) admitted one lowered instrument's addresses
/// and not two. EVD-0021 reselected it (`P08-S004`), and the survey behind that record is
/// measured under this partition so that admission cannot censor what it counts; the
/// fixtures keep it for the same reason. Everything but the events group is the default.
pub(super) fn project_profile() -> HostProfile {
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

pub(super) fn unity_master() -> crate::project::GlobalProjectState {
    crate::project::GlobalProjectState {
        master_volume: synth_core::Gain::UNITY,
        ..Default::default()
    }
}

pub(super) fn render(
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

pub(super) fn peak(samples: &[f32]) -> f32 {
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
    let mut instruments = [instrument_with(0, 2.0), instrument_with(1, 2.0)];
    for saved in &mut instruments {
        saved.pan = synth_core::BipolarValue::new(-1.0);
        for module in &mut saved.patch.modules {
            if module.module_type == ModuleType::StereoOutput {
                module
                    .parameters
                    .insert("pan".to_owned(), ParamValue::Float(-1.0));
            }
        }
    }
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
        headroom: false,
        sends: Vec::new(),
    };
    let lowered = lower_instrument_into(
        &mut graph,
        instrument(),
        &modules,
        &connections,
        &[],
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
    assert_eq!(
        scope_of(slot.channel()),
        ExecutionScope::Channel(slot.channel_tag())
    );
    assert_eq!(
        scope_of(slot.soft_clip()),
        ExecutionScope::Channel(slot.channel_tag())
    );
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
    assert!(matches!(
        ir.node(lowered.identities.node_for(out_module).expect("resolved"))
            .expect("terminal")
            .kind(),
        IrNodeKind::VoiceOutput { .. }
    ));
    assert_eq!(
        scope_of(lowered.identities.node_for(out_module).expect("resolved")),
        ExecutionScope::Voice
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

// ---------------------------------------------------------------------------------------
// `P08-S003` — inserts: the first native effects with latency and tail.
// ---------------------------------------------------------------------------------------

/// The corpus's insert-chain project, `CORPUS-0005`: distortion into delay on a held fifth.
pub(super) fn corpus_inserts() -> crate::project::ProjectFile {
    corpus_project("instrument-inserts")
}

/// One track on instrument 0 playing `(tick, pitch, duration)` notes.
pub(super) fn notes_song(notes: &[(u32, u8, u32)]) -> synth_sequencer::Song {
    let mut song = synth_sequencer::Song::new("inserts");
    song.default_tempo = synth_core::Bpm::new(120.0);
    let pattern = song.create_pattern(Duration(3840));
    let track = song.create_track("track");
    song.track_mut(track).expect("resolves").instrument =
        synth_engine::instrument::InstrumentId::new(0);
    {
        let pattern = song.pattern_mut(pattern).expect("resolves");
        for (tick, pitch, duration) in notes {
            let id = pattern.add_note(
                PatternTick(*tick),
                Pitch::new(*pitch).expect("a keyboard position"),
                Velocity::new(0.8),
            );
            if let Some(note) = pattern.note_mut(id) {
                note.duration = Some(Duration(*duration));
            }
        }
    }
    assert!(song.place_pattern(pattern, track, Tick::ZERO));
    song
}

/// The corpus project with only the named inserts kept, in that order.
pub(super) fn corpus_with_inserts(kept: &[&str]) -> crate::project::ProjectFile {
    let mut project = corpus_inserts();
    let patch = &mut project.instruments[0].patch;
    patch
        .modules
        .retain(|m| !m.module_type.is_effect() || kept.contains(&m.id.as_str()));
    patch.settings.effect_chain_order = kept.iter().map(|k| (*k).to_owned()).collect();
    project
}

pub(super) fn render_project(
    project: &crate::project::ProjectFile,
    song: &synth_sequencer::Song,
    tail: u64,
    policy: OutputPolicy,
) -> crate::lowering::render::SmokeRender {
    smoke_render_project(
        &project.instruments,
        song,
        &project.global,
        project_profile(),
        FrameCount::new(tail),
        policy,
    )
}

pub(super) fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt()
}

/// The corpus's insert chain lowers and renders, under the roomier event partition and,
/// since EVD-0021 reselected the session share (`P08-S004`), under the engine's default
/// profile too: its catch-up addresses number 29, which the first provisional share of 24
/// refused by name and the reselected share admits.
#[test]
fn the_corpus_insert_chain_lowers_and_renders_under_the_default_share_too() {
    let project = corpus_inserts();
    let rendered = render_project(&project, &project.song, 96_000, OutputPolicy::Parity);
    assert!(
        !rendered
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "{:?}",
        rendered.diagnostics
    );
    assert!(rendered.is_audible());
    assert!(
        !rendered.diagnostics.iter().any(|d| matches!(
            d.subject(),
            ProjectSubject::InsertChain { .. }
        ) || matches!(
            (d.subject(), d.reason()),
            (ProjectSubject::Module { module, .. }, _) if module.module_type.is_effect()
        )),
        "nothing about the inserts is marked: {:?}",
        rendered.diagnostics
    );

    let default = smoke_render_project(
        &project.instruments,
        &project.song,
        &project.global,
        harness_profile(),
        FrameCount::new(4_800),
        OutputPolicy::Parity,
    );
    assert!(default.is_audible(), "{:?}", default.diagnostics);
    assert_eq!(
        default.samples,
        rendered.samples[..default.samples.len()],
        "the share changes no sample; the longer render only adds tail"
    );
    let session = default
        .report
        .as_ref()
        .and_then(|report| report.row(synth_engine_v2::report::ResourceField::SessionEventShare))
        .expect("the session row");
    assert_eq!(
        session.requested(),
        synth_engine_v2::report::ResourceAmount::Events(EventCount::measured(29))
    );
    assert_eq!(
        session.available(),
        synth_engine_v2::report::ResourceAmount::Events(EventCount::limit(128).expect("positive"))
    );
}

/// The inserts sit between the balance and the channel, in the order's order, in the
/// instrument scope, with V1's values as their authored bases.
#[test]
fn inserts_sit_between_the_balance_and_the_channel_in_the_orders_order() {
    use super::super::graph::{GraphAccumulator, InstrumentStages, Sink, lower_instrument_into};
    let project = corpus_inserts();
    let patch = &project.instruments[0].patch;
    let mut graph = GraphAccumulator::default();
    let slot = InstrumentSlot::of(instrument()).expect("fits");
    let stages = InstrumentStages {
        velocity: Some(synth_engine_v2::quantities::NormalizedLevel::FULL),
        track: Some(super::super::graph::TrackStage {
            level: synth_engine_v2::quantities::Amplitude::UNITY,
            pan: synth_engine_v2::controller::BipolarLevel::ZERO,
            muted: false,
        }),
        channel: Some(super::super::graph::ChannelStrip {
            fader: synth_engine_v2::quantities::Amplitude::UNITY,
            pan: synth_engine_v2::controller::BipolarLevel::ZERO,
            muted: false,
        }),
        soft_clip: true,
        headroom: false,
        sends: Vec::new(),
    };
    let lowered = lower_instrument_into(
        &mut graph,
        instrument(),
        &patch.modules,
        &patch.connections,
        &patch.settings.effect_chain_order,
        stages,
        &super::super::modulation::SongModulators::default(),
        Sink::Node(MASTER_MIX),
    );
    assert!(!lowered.refused, "{:?}", lowered.diagnostics);
    for (id, kind, scope) in [
        (MASTER_MIX, IrNodeKind::Mix, ExecutionScope::Global),
        (MASTER_OUTPUT, IrNodeKind::Output, ExecutionScope::Global),
    ] {
        graph.node(id, kind, scope).expect("free");
    }
    graph.connect(
        (MASTER_MIX, synth_engine_v2::ir::PortId::FIRST),
        (MASTER_OUTPUT, synth_engine_v2::ir::PortId::FIRST),
        synth_engine_v2::ir::SignalDomain::Audio,
    );
    let ir = graph
        .build(
            super::super::graph::voice_tuning().expect("prepares"),
            super::super::graph::plan_declarations(HeldNoteCount::measured(1), EventCount::NONE),
        )
        .expect("builds");
    let dst = lowered
        .identities
        .node_for("dst-1".parse().expect("parses"))
        .expect("addressed");
    let dly = lowered
        .identities
        .node_for("dly-1".parse().expect("parses"))
        .expect("addressed");
    let chain = [
        slot.balance(),
        dst,
        dly,
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
    let node = |id| ir.node(id).expect("present");
    assert_eq!(node(dst).scope(), ExecutionScope::InstrumentInstance);
    assert_eq!(node(dly).scope(), ExecutionScope::InstrumentInstance);
    let level = |v: f32| synth_engine_v2::quantities::NormalizedLevel::new(v).expect("a level");
    assert_eq!(
        node(dst).kind(),
        IrNodeKind::Distortion {
            drive: level(0.7),
            tone: level(0.8),
            mix: level(1.0),
        }
    );
    assert_eq!(
        node(dly).kind(),
        IrNodeKind::Delay {
            time_left: synth_engine_v2::quantities::DelayTime::new(0.25).expect("in range"),
            time_right: synth_engine_v2::quantities::DelayTime::new(0.25).expect("in range"),
            feedback: synth_engine_v2::quantities::DelayFeedback::new(0.45).expect("in range"),
            mix: level(0.5),
            tone: level(0.4),
        }
    );
}

/// The order describes the patch's effects exactly once each, by parsed identity
/// (`CORPUS-0005-C1`, ADR-0021): an omitted effect, an unknown entry, a repeat spelled two
/// ways, a voice module and a non-identity are each refused naming the problem.
#[test]
fn an_insert_order_that_does_not_describe_the_effects_exactly_once_is_refused_by_name() {
    for (order, expected) in [
        (
            vec!["dst-1"],
            "dly-1 is an effect module the insert order omits",
        ),
        (
            vec!["dst-1", "dly-1", "rev-3"],
            "rev-3, which is not an effect module of the patch",
        ),
        (vec!["dst-1", "dst-01", "dly-1"], "dst-1 twice"),
        (
            vec!["dst-1", "dly-1", "osc-1"],
            "osc-1, which is not an effect module of the patch",
        ),
        (
            vec!["dst-1", "dly-1", "banana"],
            "\"banana\", which is not a module identity",
        ),
        (vec![], "is an effect module the insert order omits"),
    ] {
        let mut project = corpus_inserts();
        project.instruments[0].patch.settings.effect_chain_order =
            order.iter().map(|o| (*o).to_owned()).collect();
        let rendered = render_project(&project, &project.song, 4_800, OutputPolicy::Parity);
        assert!(rendered.samples.is_empty(), "{order:?}");
        assert!(
            rendered.diagnostics.iter().any(|d| matches!(
                (d.severity(), d.subject(), d.reason()),
                (Severity::Refused, ProjectSubject::InsertChain { .. }, LoweringReason::InsertOrder { problem })
                    if problem.contains(expected)
            )),
            "{order:?}: expected {expected:?}, got {:?}",
            rendered.diagnostics
        );
    }
}

/// A mode V2 has not carried — a distortion type other than soft clip, a delay mode other
/// than mono, a tempo-synced time — is refused naming the parameter.
#[test]
fn an_insert_mode_v2_has_not_carried_is_refused_by_name() {
    for (module, key, value, expected) in [
        (
            "dst-1",
            "type",
            ParamValue::Choice("hard_clip".to_owned()),
            "distortion mode",
        ),
        (
            "dly-1",
            "mode",
            ParamValue::Choice("ping_pong".to_owned()),
            "delay mode",
        ),
        (
            "dly-1",
            "tempo_sync",
            ParamValue::Float(1.0),
            "tempo-synced",
        ),
    ] {
        let mut project = corpus_inserts();
        let saved = project.instruments[0]
            .patch
            .modules
            .iter_mut()
            .find(|m| m.id == module)
            .expect("the corpus has it");
        saved.parameters.insert(key.to_owned(), value);
        let rendered = render_project(&project, &project.song, 4_800, OutputPolicy::Parity);
        assert!(rendered.samples.is_empty(), "{module}.{key}");
        assert!(
            rendered.diagnostics.iter().any(|d| matches!(
                (d.severity(), d.subject(), d.reason()),
                (Severity::Refused, ProjectSubject::Parameter { parameter, .. }, LoweringReason::OwnedByLaterPhase { capability, .. })
                    if parameter == key && capability.contains(expected)
            )),
            "{module}.{key}: {:?}",
            rendered.diagnostics
        );
    }
    // And an effect V2 has no kind for at all is refused as an unsupported type.
    let mut project = corpus_inserts();
    let mut reverb = crate::patch::ModuleState {
        id: "rev-1".to_owned(),
        module_type: ModuleType::Reverb,
        position: crate::patch::Position::new(0.0, 0.0),
        description: String::new(),
        parameters: std::collections::BTreeMap::new(),
        scripts: std::collections::BTreeMap::new(),
    };
    reverb
        .parameters
        .insert("mix".to_owned(), ParamValue::Float(0.3));
    project.instruments[0].patch.modules.push(reverb);
    project.instruments[0]
        .patch
        .settings
        .effect_chain_order
        .push("rev-1".to_owned());
    let rendered = render_project(&project, &project.song, 4_800, OutputPolicy::Parity);
    assert!(rendered.diagnostics.iter().any(|d| matches!(
        (d.severity(), d.reason()),
        (
            Severity::Refused,
            LoweringReason::UnsupportedModuleType {
                module_type: ModuleType::Reverb
            }
        )
    )));
}

/// V1's `time` link macro, as V1 loads it: a side without its own key takes the macro, a side
/// with one keeps it — `time` sorts before `time_left` in the saved map, so the side's key is
/// applied last. `delay_time_macro_yields_to_the_side_key_in_the_offline_renderer` measures
/// that order on V1; this holds the lowerer to the same reading.
#[test]
fn a_delays_time_macro_applies_where_a_side_key_is_absent() {
    use super::super::graph::{GraphAccumulator, InstrumentStages, Sink, lower_instrument_into};
    let lowered_delay = |edit: &dyn Fn(&mut std::collections::BTreeMap<String, ParamValue>)| {
        let mut project = corpus_inserts();
        let patch = &mut project.instruments[0].patch;
        let delay = patch
            .modules
            .iter_mut()
            .find(|m| m.id == "dly-1")
            .expect("the corpus has it");
        edit(&mut delay.parameters);
        let mut graph = GraphAccumulator::default();
        let lowered = lower_instrument_into(
            &mut graph,
            instrument(),
            &patch.modules,
            &patch.connections,
            &patch.settings.effect_chain_order,
            InstrumentStages::default(),
            &super::super::modulation::SongModulators::default(),
            Sink::OwnOutput,
        );
        assert!(!lowered.refused, "{:?}", lowered.diagnostics);
        let node = lowered
            .identities
            .node_for("dly-1".parse().expect("parses"))
            .expect("addressed");
        let ir = graph
            .build(
                super::super::graph::voice_tuning().expect("prepares"),
                super::super::graph::plan_declarations(
                    HeldNoteCount::measured(1),
                    EventCount::NONE,
                ),
            )
            .expect("builds");
        match ir.node(node).expect("present").kind() {
            IrNodeKind::Delay {
                time_left,
                time_right,
                ..
            } => (time_left.as_f32(), time_right.as_f32()),
            other => panic!("{other:?}"),
        }
    };
    // Only the macro: both sides take it.
    assert_eq!(
        lowered_delay(&|p| {
            p.remove("time_left");
            p.remove("time_right");
            p.insert("time".to_owned(), ParamValue::Float(0.5));
        }),
        (0.5, 0.5)
    );
    // The macro and one side: the side keeps its own, the other takes the macro.
    assert_eq!(
        lowered_delay(&|p| {
            p.remove("time_right");
            p.insert("time".to_owned(), ParamValue::Float(0.5));
        }),
        (0.25, 0.5)
    );
    // Neither: V1's declared defaults per side, which differ.
    assert_eq!(
        lowered_delay(&|p| {
            p.remove("time_left");
            p.remove("time_right");
        }),
        (0.375, 0.5)
    );
}

/// `CORPUS-0005-P4`: the chain runs in the order's order — the reverse is another signal.
#[test]
fn reversing_the_insert_order_renders_other_bits() {
    let project = corpus_inserts();
    let forward = render_project(&project, &project.song, 48_000, OutputPolicy::Parity);
    let mut reversed = corpus_inserts();
    reversed.instruments[0]
        .patch
        .settings
        .effect_chain_order
        .reverse();
    let backward = render_project(&reversed, &reversed.song, 48_000, OutputPolicy::Parity);
    assert!(forward.is_audible() && backward.is_audible());
    assert_ne!(forward.samples, backward.samples);
    let difference: Vec<f32> = forward
        .samples
        .iter()
        .zip(&backward.samples)
        .map(|(a, b)| a - b)
        .collect();
    assert!(
        rms(&difference) > 0.1 * rms(&forward.samples),
        "the reversal is a different signal, not a rounding"
    );
}

/// `CORPUS-0005-P1` and `P2`, each isolated as the corpus isolates them: with the distortion
/// alone the dyad rendered whole differs from the two notes rendered apart and summed,
/// because the chain shapes the **sum** of the voices; with the delay alone likewise, because
/// the two notes share one line and meet in the `tanh` on its feedback write; and with no
/// insert the two renders agree to rounding — the null control that proves both notes sound
/// and the rest of the chain is linear under the headroom policy.
#[test]
fn the_chain_runs_on_the_voice_sum_and_its_state_is_shared_across_voices() {
    let dyad = notes_song(&[(0, 48, 720), (0, 55, 720)]);
    let low = notes_song(&[(0, 48, 720)]);
    let high = notes_song(&[(0, 55, 720)]);
    let measure = |kept: &[&str]| {
        let project = corpus_with_inserts(kept);
        let whole = render_project(&project, &dyad, 48_000, OutputPolicy::Headroom);
        let a = render_project(&project, &low, 48_000, OutputPolicy::Headroom);
        let b = render_project(&project, &high, 48_000, OutputPolicy::Headroom);
        assert!(
            whole.is_audible() && a.is_audible() && b.is_audible(),
            "{kept:?}"
        );
        assert_eq!(whole.samples.len(), a.samples.len());
        assert_eq!(whole.samples.len(), b.samples.len());
        let summed: Vec<f32> = a
            .samples
            .iter()
            .zip(&b.samples)
            .map(|(x, y)| x + y)
            .collect();
        let difference: Vec<f32> = whole
            .samples
            .iter()
            .zip(&summed)
            .map(|(w, s)| w - s)
            .collect();
        rms(&difference) / rms(&whole.samples)
    };
    let null = measure(&[]);
    let distortion_only = measure(&["dst-1"]);
    let delay_only = measure(&["dly-1"]);
    assert!(null < 1e-5, "the empty chain is linear to rounding: {null}");
    assert!(
        distortion_only > 1e-2,
        "the distortion shapes the sum, not each voice: {distortion_only}"
    );
    assert!(
        delay_only > 1e-4,
        "the delay's line is shared, not per voice: {delay_only}"
    );
}

/// Overlapping notes of distinct keys lower up to the instrument's voice count — V1's default
/// eight — and one more is refused naming the note, because V1 steals there and V2 would play
/// every note; a tie counts as held. Found by an independent read of the lifted overlap rule.
#[test]
fn more_notes_held_at_once_than_the_instruments_voices_are_refused_by_name() {
    let project = corpus_with_inserts(&["dst-1", "dly-1"]);
    let chord = |count: u8| {
        let notes: Vec<(u32, u8, u32)> = (0..count).map(|i| (0, 48 + i, 720)).collect();
        notes_song(&notes)
    };
    let eight = render_project(&project, &chord(8), 4_800, OutputPolicy::Parity);
    assert!(
        !eight
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused),
        "{:?}",
        eight.diagnostics
    );
    assert!(eight.is_audible());
    let nine = render_project(&project, &chord(9), 4_800, OutputPolicy::Parity);
    assert!(nine.samples.is_empty());
    assert!(
        nine.diagnostics.iter().any(|d| matches!(
            (d.severity(), d.subject(), d.reason()),
            (Severity::Refused, ProjectSubject::Note { .. }, LoweringReason::OwnedByLaterPhase { capability, .. })
                if capability.contains("more notes held at once than the instrument's voices")
        )),
        "{:?}",
        nine.diagnostics
    );
    // A ninth note starting at the tick the first ends is still held at that tick.
    let mut notes: Vec<(u32, u8, u32)> = (0..8).map(|i| (0, 48 + i, 720)).collect();
    notes.push((720, 60, 720));
    let tie = render_project(&project, &notes_song(&notes), 4_800, OutputPolicy::Parity);
    assert!(
        tie.samples.is_empty(),
        "a tie counts as held, as the declared peak counts it"
    );
}

/// `CORPUS-0005-P3`: the delay's repeats outlast the notes that produced them — the render
/// past the last note's release is signal with the delay and silence without it.
#[test]
fn the_delays_state_outlives_the_notes_that_produced_it() {
    // One short note at 2 s: released by 2.35 s with the corpus's 0.1 s release.
    let song = notes_song(&[(1920, 60, 240)]);
    let with = render_project(
        &corpus_with_inserts(&["dly-1"]),
        &song,
        96_000,
        OutputPolicy::Parity,
    );
    let without = render_project(
        &corpus_with_inserts(&[]),
        &song,
        96_000,
        OutputPolicy::Parity,
    );
    assert!(with.is_audible() && without.is_audible());
    let from = 2 * (2.6 * 48_000.0) as usize;
    let to = 2 * (3.5 * 48_000.0) as usize;
    assert!(without.samples.len() >= to && with.samples.len() >= to);
    assert!(
        without.samples[from..to].iter().all(|s| *s == 0.0),
        "without the delay the release has ended"
    );
    assert!(
        with.samples[from..to].iter().any(|s| s.abs() > 1e-4),
        "the delay's repeats continue past the release"
    );
}

/// V2's two insert kinds against V1's own modules, run over the same widened input: the
/// oracle is `synth_modules::effects::{Delay, Distortion}` itself, not a transcription.
#[test]
fn v2s_delay_and_distortion_are_v1s_modules_bit_for_bit() {
    use synth_core::{DelayMode, DelayParam, DistortionParam, Param, ProcessContext};
    use synth_engine_v2::ir::{ExecutionScope, GraphIr, NodeId, PortId, SignalDomain};
    use synth_engine_v2::offline::render_offline;
    use synth_engine_v2::time::PlanPosition;

    const SOURCE: NodeId = NodeId::new(1);
    const INSERT: NodeId = NodeId::new(2);
    const OUTPUT: NodeId = NodeId::new(3);
    const FRAMES: u64 = 65_536;
    let rate = 48_000.0_f32;
    let level = |v: f32| synth_engine_v2::quantities::NormalizedLevel::new(v).expect("a level");
    let source = IrNodeKind::Sine {
        frequency: synth_engine_v2::quantities::Frequency::new(220.0).expect("finite"),
        amplitude: synth_engine_v2::quantities::Amplitude::new(0.8).expect("finite"),
    };
    let render = |insert: Option<IrNodeKind>| {
        let mut builder = GraphIr::builder()
            .node(SOURCE, source, ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
        match insert {
            Some(kind) => {
                builder = builder
                    .node(INSERT, kind, ExecutionScope::Global)
                    .connect(
                        (SOURCE, PortId::FIRST),
                        (INSERT, PortId::FIRST),
                        SignalDomain::Audio,
                    )
                    .connect(
                        (INSERT, PortId::FIRST),
                        (OUTPUT, PortId::FIRST),
                        SignalDomain::Audio,
                    );
            }
            None => {
                builder = builder.connect(
                    (SOURCE, PortId::FIRST),
                    (OUTPUT, PortId::FIRST),
                    SignalDomain::Audio,
                );
            }
        }
        let ir = builder.build().expect("readable");
        let plan = compile(&ir, &RenderConfig::new(harness_profile()))
            .into_plan()
            .expect("admits");
        render_offline(plan, FrameCount::new(FRAMES), PlanPosition::ZERO, &[]).expect("renders")
    };
    let input = render(None);
    let context = ProcessContext {
        sample_rate: synth_core::SampleRate::new(rate),
        samples: synth_core::SampleCount::new(FRAMES as usize),
        ..Default::default()
    };
    let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();

    let (mut v1, _) = crate::module_factory::create_effect(ModuleType::Delay).expect("V1 has it");
    v1.set_sample_rate(synth_core::SampleRate::new(rate));
    for param in [
        Param::Delay(DelayParam::Mode(DelayMode::Mono)),
        Param::Delay(DelayParam::TimeLeft(synth_core::Seconds::new(0.25))),
        Param::Delay(DelayParam::TimeRight(synth_core::Seconds::new(0.1234))),
        Param::Delay(DelayParam::Feedback(synth_core::NormalizedValue::new(0.45))),
        Param::Delay(DelayParam::Mix(synth_core::NormalizedValue::new(0.5))),
        Param::Delay(DelayParam::Damping(synth_core::NormalizedValue::new(0.4))),
        Param::Delay(DelayParam::TempoSync(false)),
    ] {
        v1.set_param(param);
    }
    let mut expected = vec![0.0_f32; input.len()];
    v1.process(&input, &mut expected, &context);
    let v2 = render(Some(IrNodeKind::Delay {
        time_left: synth_engine_v2::quantities::DelayTime::new(0.25).expect("in range"),
        time_right: synth_engine_v2::quantities::DelayTime::new(0.1234).expect("in range"),
        feedback: synth_engine_v2::quantities::DelayFeedback::new(0.45).expect("in range"),
        mix: level(0.5),
        tone: level(0.4),
    }));
    assert_eq!(bits(&v2), bits(&expected), "delay");
    assert_ne!(bits(&v2), bits(&input));

    let (mut v1, _) =
        crate::module_factory::create_effect(ModuleType::Distortion).expect("V1 has it");
    v1.set_sample_rate(synth_core::SampleRate::new(rate));
    for param in [
        Param::Distortion(DistortionParam::Mode(synth_core::DistortionMode::SoftClip)),
        Param::Distortion(DistortionParam::Drive(synth_core::NormalizedValue::new(
            0.7,
        ))),
        Param::Distortion(DistortionParam::Tone(synth_core::NormalizedValue::new(0.8))),
        Param::Distortion(DistortionParam::Mix(synth_core::NormalizedValue::new(1.0))),
    ] {
        v1.set_param(param);
    }
    let mut expected = vec![0.0_f32; input.len()];
    v1.process(&input, &mut expected, &context);
    let v2 = render(Some(IrNodeKind::Distortion {
        drive: level(0.7),
        tone: level(0.8),
        mix: level(1.0),
    }));
    assert_eq!(bits(&v2), bits(&expected), "distortion");
    assert_ne!(bits(&v2), bits(&input));
}

/// The compressor oracle is the existing V1 effect, including its initial zero-dB
/// envelope and rectified sidechain high-pass. Both engines receive the same samples.
#[test]
fn v2_compressor_matches_v1_with_internal_external_and_unpatched_detectors() {
    use synth_core::{
        AudioEffect, CompressorParam, Decibels, Hertz, Milliseconds, Param, ProcessContext, Ratio,
    };
    use synth_engine_v2::dynamics::{CompressorDetector, CompressorSettings};
    use synth_engine_v2::ir::{GraphIr, NodeId, PortId, SignalDomain};
    use synth_engine_v2::quantities::{Amplitude, CutoffFrequency, Frequency, NormalizedLevel};
    use synth_engine_v2::time::PlanPosition;
    let main = NodeId::new(1);
    let detector = NodeId::new(2);
    let effect = NodeId::new(3);
    let out = NodeId::new(4);
    let source = |frequency| IrNodeKind::Sine {
        frequency: Frequency::new(frequency).expect("frequency"),
        amplitude: Amplitude::new(0.8).expect("level"),
    };
    let render = |settings: Option<CompressorSettings>, patched: bool, frequency: f32| {
        let mut b = GraphIr::builder()
            .node(main, source(frequency), ExecutionScope::Global)
            .node(out, IrNodeKind::Output, ExecutionScope::Global);
        if let Some(settings) = settings {
            b = b
                .node(
                    effect,
                    IrNodeKind::Compressor { settings },
                    ExecutionScope::Global,
                )
                .connect(
                    (main, PortId::FIRST),
                    (effect, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .connect(
                    (effect, PortId::FIRST),
                    (out, PortId::FIRST),
                    SignalDomain::Audio,
                );
            if patched {
                b = b
                    .node(detector, source(73.0), ExecutionScope::Global)
                    .connect(
                        (detector, PortId::FIRST),
                        (effect, PortId::new(1)),
                        SignalDomain::Audio,
                    );
            }
        } else {
            b = b.connect(
                (main, PortId::FIRST),
                (out, PortId::FIRST),
                SignalDomain::Audio,
            );
        }
        let plan = compile(
            &b.build().expect("graph"),
            &RenderConfig::new(harness_profile()),
        )
        .into_plan()
        .expect("plan");
        synth_engine_v2::offline::render_offline(
            plan,
            FrameCount::new(2048),
            PlanPosition::ZERO,
            &[],
        )
        .expect("render")
    };
    let main_samples = render(None, false, 880.0);
    let side_samples = render(None, false, 73.0);
    for (external, patched, cutoff) in [
        (false, false, 80.0),
        (true, false, 80.0),
        (true, true, 20.0),
        (true, true, 180.0),
    ] {
        let settings = CompressorSettings::new(
            Decibels::new(-23.0),
            Ratio::new(5.0),
            Milliseconds::new(0.5),
            Milliseconds::new(70.0),
            Decibels::new(3.0),
            NormalizedLevel::new(0.7).expect("mix"),
            if external {
                CompressorDetector::External {
                    cutoff: CutoffFrequency::new(cutoff).expect("cutoff"),
                }
            } else {
                CompressorDetector::Internal
            },
        )
        .expect("settings");
        let mut v1 = synth_modules::effects::Compressor::new();
        for p in [
            CompressorParam::Threshold(Decibels::new(-23.0)),
            CompressorParam::Ratio(Ratio::new(5.0)),
            CompressorParam::Attack(Milliseconds::new(0.5)),
            CompressorParam::Release(Milliseconds::new(70.0)),
            CompressorParam::Makeup(Decibels::new(3.0)),
            CompressorParam::Mix(synth_core::NormalizedValue::new(0.7)),
            CompressorParam::SidechainEnabled(external),
            CompressorParam::SidechainFilter(Hertz::new(cutoff)),
        ] {
            v1.set_param(Param::Compressor(p));
        }
        if patched {
            v1.set_sidechain_input(&side_samples);
        }
        let mut expected = vec![0.0; main_samples.len()];
        v1.process(
            &main_samples,
            &mut expected,
            &ProcessContext {
                sample_rate: synth_core::SampleRate::new(48_000.0),
                samples: synth_core::SampleCount::new(2048),
                ..Default::default()
            },
        );
        assert_eq!(
            render(Some(settings), patched, 880.0),
            expected,
            "external {external}, patched {patched}, cutoff {cutoff}"
        );
        assert_ne!(
            expected, main_samples,
            "comparison must exercise processing"
        );
    }
}

#[test]
fn project_sidechain_resolves_the_source_channel_and_marks_current_quantum_timing() {
    let source = instrument_with(0, 0.4);
    let mut destination = instrument_with(1, 0.6);
    destination.sidechain_source_id = Some(0);
    let mut compressor = module("cmp-1", ModuleType::Compressor);
    compressor
        .parameters
        .insert("sidechain".into(), crate::patch::ParamValue::Float(1.0));
    destination.patch.modules.push(compressor);
    destination
        .patch
        .settings
        .effect_chain_order
        .push("cmp-1".into());
    let result = smoke_render_project(
        &[source, destination],
        &two_instrument_song(1),
        &crate::project::GlobalProjectState::default(),
        harness_profile(),
        FrameCount::new(4096),
        OutputPolicy::Parity,
    );
    assert!(!result.samples.is_empty(), "{:?}", result.diagnostics);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| matches!(d.reason(), LoweringReason::SidechainTiming))
    );
    assert_eq!(
        result.fidelity(),
        Fidelity::UnsupportedScope,
        "intentional timing is not a parity claim"
    );
}

#[test]
fn voice_amplifier_and_terminating_output_match_actual_v1_modules() {
    use synth_core::{
        AudioBuffer, InputPorts, MixerParam, Param, PolyModule, PortName, ProcessContext,
    };
    use synth_engine_v2::controller::BipolarLevel;
    use synth_engine_v2::ir::{GraphIr, NodeId, PortId, SignalDomain};
    use synth_engine_v2::output::OutputLimiting;
    use synth_engine_v2::quantities::{Amplitude, NormalizedLevel};
    use synth_engine_v2::time::PlanPosition;
    let context = ProcessContext {
        samples: synth_core::SampleCount::new(64),
        ..ProcessContext::default()
    };
    for input in [-3.0, -0.2, 0.0, 0.7, 4.0] {
        let mut audio = AudioBuffer::new(64);
        for n in 0..64 {
            audio[n] = input;
        }
        for pan in [-1.0, -0.37, 0.0, 0.63, 1.0] {
            let render = |kind| {
                let ir = GraphIr::builder()
                    .node(
                        NodeId::new(1),
                        IrNodeKind::Constant {
                            level: Amplitude::new(input).expect("level"),
                        },
                        ExecutionScope::Global,
                    )
                    .node(NodeId::new(2), kind, ExecutionScope::Global)
                    .node(NodeId::new(3), IrNodeKind::Output, ExecutionScope::Global)
                    .connect(
                        (NodeId::new(1), PortId::FIRST),
                        (NodeId::new(2), PortId::FIRST),
                        SignalDomain::Audio,
                    )
                    .connect(
                        (NodeId::new(2), PortId::FIRST),
                        (NodeId::new(3), PortId::FIRST),
                        SignalDomain::Audio,
                    )
                    .build()
                    .expect("graph");
                let plan = compile(&ir, &RenderConfig::new(harness_profile()))
                    .into_plan()
                    .expect("plan");
                synth_engine_v2::offline::render_offline(
                    plan,
                    FrameCount::new(64),
                    PlanPosition::ZERO,
                    &[],
                )
                .expect("render")
            };
            let mut amp = synth_modules::Amplifier::new();
            amp.set_param(Param::Amplifier(synth_core::AmplifierParam::Pan(
                synth_core::BipolarValue::new(pan),
            )));
            let mut outputs =
                std::collections::HashMap::from([(PortName::OUT, AudioBuffer::new(64))]);
            amp.process(
                InputPorts::new(&[(PortName::IN, &audio)]),
                &mut outputs,
                &context,
            );
            let expected: Vec<f32> = outputs[&PortName::OUT]
                .as_slice()
                .iter()
                .flat_map(|v| [*v; 2])
                .collect();
            assert_eq!(
                render(IrNodeKind::VoiceAmplifier {
                    pan: BipolarLevel::new(pan).expect("pan")
                }),
                expected,
                "amp input {input} pan {pan}"
            );
            for master in [0.0, 0.8, 1.0] {
                for (enabled, muted) in [(true, false), (false, false), (true, true)] {
                    let mut terminal = synth_modules::StereoOutput::new();
                    for p in [
                        Param::Mixer(MixerParam::Master(synth_core::Gain::new(master))),
                        Param::Amplifier(synth_core::AmplifierParam::Pan(
                            synth_core::BipolarValue::new(pan),
                        )),
                        Param::Mixer(MixerParam::Limit(enabled)),
                        Param::Mixer(MixerParam::Mute(muted)),
                    ] {
                        terminal.set_param(p);
                    }
                    terminal.process(
                        InputPorts::new(&[(PortName::IN, &audio)]),
                        &mut std::collections::HashMap::new(),
                        &context,
                    );
                    let actual = render(IrNodeKind::VoiceOutput {
                        master: NormalizedLevel::new(master).expect("master"),
                        pan: BipolarLevel::new(pan).expect("pan"),
                        muted,
                        limiting: if enabled {
                            OutputLimiting::SoftKnee
                        } else {
                            OutputLimiting::HardClamp
                        },
                    });
                    assert_eq!(
                        actual,
                        &terminal.get_output()[..128],
                        "output input {input} pan {pan} master {master} limit {enabled} mute {muted}"
                    );
                }
            }
        }
    }
}

#[test]
fn saved_master_inserts_keep_order_before_volume_and_refuse_bad_identities() {
    use synth_core::ProcessContext;
    let saved = instrument_with(0, 1.0);
    let song = four_note_song();
    let base = render(
        std::slice::from_ref(&saved),
        &song,
        &unity_master(),
        OutputPolicy::Headroom,
    );
    let distortion = corpus_inserts().instruments[0]
        .patch
        .modules
        .iter()
        .find(|module| module.module_type == ModuleType::Distortion)
        .expect("distortion")
        .clone();
    let compressor = module("cmp-1", ModuleType::Compressor);
    let mut outcomes = Vec::new();
    for effects in [
        [distortion.clone(), compressor.clone()],
        [compressor.clone(), distortion.clone()],
    ] {
        let mut global = unity_master();
        global.master_volume = synth_core::Gain::new(0.4);
        global.master_effects = effects.to_vec();
        let actual = render(
            std::slice::from_ref(&saved),
            &song,
            &global,
            OutputPolicy::Headroom,
        );
        assert!(actual.is_audible(), "{:?}", actual.diagnostics);
        let mut expected = base.samples.clone();
        for effect in &effects {
            let (mut v1, descriptor) =
                crate::module_factory::create_effect(effect.module_type).expect("effect");
            v1.set_sample_rate(synth_core::SampleRate::new(48_000.0));
            for (key, value) in &effect.parameters {
                let declaration = descriptor.find_parameter(key).expect("parameter");
                v1.set_param(value.to_param(declaration));
            }
            let mut next = vec![0.0; expected.len()];
            v1.process(
                &expected,
                &mut next,
                &ProcessContext {
                    sample_rate: synth_core::SampleRate::new(48_000.0),
                    samples: synth_core::SampleCount::new(expected.len() / 2),
                    ..ProcessContext::default()
                },
            );
            expected = next;
        }
        for sample in &mut expected {
            *sample *= 0.4;
        }
        assert_eq!(actual.samples, expected);
        outcomes.push(actual.samples);
    }
    assert_ne!(outcomes[0], outcomes[1], "insert order must be audible");
    for effects in [
        vec![compressor.clone(), compressor.clone()],
        vec![module("dly-1", ModuleType::Compressor)],
    ] {
        let mut global = unity_master();
        global.master_effects = effects;
        let actual = render(
            std::slice::from_ref(&saved),
            &song,
            &global,
            OutputPolicy::Parity,
        );
        assert!(actual.samples.is_empty());
        assert!(
            actual
                .diagnostics
                .iter()
                .any(|d| d.subject() == &ProjectSubject::MasterChain
                    && d.severity() == Severity::Refused)
        );
    }
}
