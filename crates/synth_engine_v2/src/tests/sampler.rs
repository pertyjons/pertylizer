//! The one-zone sampler on the prepared map/zone contract — ADR-0026, `P06-S005`.
//!
//! Every playback claim is an **exact** comparison of a render against an oracle written
//! from V1's law in this file: the rate as V1's frequency ratio, the read as V1's two-tap
//! interpolation with its loop wrap and region clamp, the downmix as V1's half-sum, the
//! fade as a linear ramp over V1's 512 frames. A tolerance would hide the rounding the
//! record says is a defect.

use crate::compile::{RenderConfig, compile};
use crate::diagnostics::CompileError;
use crate::ir::{
    ExecutionScope, GraphIr, IrError, IrNodeKind, NodeId, NoteProducerDeclaration,
    PlanDeclarations, PortId, SignalDomain, StealingPolicy,
};
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, GainFactor, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::sample::{
    KeyRange, LoopRegion, PlayDirection, PlayMode, PlaybackRegion, PreparedSample,
    SUSTAIN_FADE_FRAMES, SampleFrame, SampleMap, SampleMapRef, SampleRef, SampleZone,
    VelocityRange,
};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const SAMPLER: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const Q: u64 = QUANTUM_FRAMES as u64;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const FRAMES: usize = 4_096;

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("a harness profile")
}

/// A deterministic mono sample: every value distinct and none dyadic, so a wrong frame, a
/// wrong tap or a wrong weight reads as a wrong number rather than a coincidence.
fn mono_frames() -> Vec<f32> {
    (0..FRAMES)
        .map(|i| ((i * 7_919) % 1_000) as f32 / 1_000.0 - 0.5)
        .collect()
}

fn mono_sample() -> PreparedSample {
    PreparedSample::prepare(
        mono_frames(),
        ChannelLayout::Mono,
        SampleRate::new(48_000.0).expect("valid rate"),
    )
    .expect("the fixture prepares")
}

fn region(start: u32, end: u32) -> PlaybackRegion {
    PlaybackRegion::new(SampleFrame::new(start), SampleFrame::new(end)).expect("a region")
}

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("a keyboard position")
}

/// The one zone every fixture shares unless a test narrows it: the whole keyboard, every
/// velocity, root C4, no fine tuning, the whole sample, no loop, unity gain.
fn zone(sample: SampleRef, loop_region: Option<LoopRegion>) -> SampleZone {
    let zone = SampleZone::new(sample, key(60), region(0, FRAMES as u32));
    match loop_region {
        Some(loop_region) => zone.looping(loop_region),
        None => zone,
    }
}

/// What the sampler is authored with, beyond its zone.
#[derive(Clone, Copy)]
struct Authored {
    play_mode: PlayMode,
    direction: PlayDirection,
    start_offset: NormalizedLevel,
    velocity_sensitivity: NormalizedLevel,
}

const PLAIN: Authored = Authored {
    play_mode: PlayMode::Sustain,
    direction: PlayDirection::Forward,
    start_offset: NormalizedLevel::ZERO,
    velocity_sensitivity: NormalizedLevel::FULL,
};

/// The smallest real sampler voice: sampler → amplifier ← envelope, into the output. The
/// envelope plays the voice and is authored to be transparent — attack and release zero,
/// sustain full, velocity ignored — so the render **is** the sampler's output.
fn voice(samples: Vec<PreparedSample>, map: SampleMap, authored: Authored) -> GraphIr {
    voice_wired(samples, map, authored, false)
}

/// The same voice with the sampler feeding the output **directly**, past the amplifier
/// the envelope drives: what the envelope's own release would silence — the sampler's
/// fade, a one-shot playing out — is then audible on its own. The envelope still plays
/// the voice and the sampler still starts on its trigger; only the audio path differs.
fn voice_direct(samples: Vec<PreparedSample>, map: SampleMap, authored: Authored) -> GraphIr {
    voice_wired(samples, map, authored, true)
}

fn voice_wired(
    samples: Vec<PreparedSample>,
    map: SampleMap,
    authored: Authored,
    direct: bool,
) -> GraphIr {
    let mut builder = GraphIr::builder();
    for sample in samples {
        builder = builder.sample(sample);
    }
    builder = builder
        .sample_map(map)
        .node(
            SAMPLER,
            IrNodeKind::Sampler {
                map: SampleMapRef::new(0),
                level: Amplitude::UNITY,
                velocity_sensitivity: authored.velocity_sensitivity,
                start_offset: authored.start_offset,
                play_mode: authored.play_mode,
                direction: authored.direction,
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        );
    builder = if direct {
        builder.connect(
            (SAMPLER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
    } else {
        builder
            .connect(
                (SAMPLER, PortId::FIRST),
                (AMPLIFIER, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (AMPLIFIER, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
    };
    builder
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("12-TET prepares"),
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(1),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(1),
            stealing: StealingPolicy::None,
            ..PlanDeclarations::default()
        })
        .build()
        .expect("the fixture builds")
}

fn plain_voice() -> GraphIr {
    voice(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        PLAIN,
    )
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

fn refusal(ir: &GraphIr) -> CompileError {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect_err("the plan is refused")
}

fn struck(
    plan: &CompiledPlan,
    key_raw: u8,
    velocity: NoteVelocity,
    at: u64,
    length: u64,
) -> Vec<PlanEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    vec![
        PlanEvent::new(
            PlanPosition::new(at),
            CompiledPayload::NoteOn {
                slot,
                key: key(key_raw),
                velocity,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(at + length),
            CompiledPayload::NoteOff {
                slot,
                key: key(key_raw),
            },
        ),
    ]
}

fn note(plan: &CompiledPlan, key_raw: u8, at: u64, length: u64) -> Vec<PlanEvent> {
    struck(plan, key_raw, NoteVelocity::FULL, at, length)
}

fn render(plan: &CompiledPlan, notes: &[Vec<PlanEvent>], quanta: u64) -> Vec<f32> {
    render_reporting(plan, notes, quanta).0
}

fn render_reporting(plan: &CompiledPlan, notes: &[Vec<PlanEvent>], quanta: u64) -> (Vec<f32>, u64) {
    let mut events: Vec<PlanEvent> = notes.iter().flatten().copied().collect();
    events.sort_by_key(|event| event.position());
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let mut out = drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        ((quanta + 1) * Q) as usize,
    );
    // The stream's output carry is one quantum behind the plan (ADR-0001's alignment),
    // so plan position `p` is output frame `p + Q`; the oracles index plan positions.
    let out = out.split_off(Q as usize);
    (out, renderer.diagnostics().notes_outside_zone())
}

fn drive(
    scheduler: &mut CompiledEventScheduler,
    renderer: &mut PreparedRenderer,
    arbiter: &mut PublicationArbiter,
    frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let this = BLOCK.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(renderer, arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
    out
}

/// V1's rate for a key over a root under twelve-tone equal temperament: the two
/// frequencies as `f32`, widened and divided — `set_voice_pitch`'s own arithmetic.
fn v1_rate(played: u8, root: u8) -> f64 {
    f64::from(synth_core::Hertz::from_midi(played).as_f32())
        / f64::from(synth_core::Hertz::from_midi(root).as_f32())
}

/// V1's two-tap read of interleaved frames at a fractional position: the second tap
/// wraps into the loop's start at the loop's end and clamps at the region's end, each
/// channel interpolated and a stereo pair halved.
fn v1_read(
    frames: &[f32],
    width: usize,
    end: usize,
    looping: Option<(usize, usize)>,
    position: f64,
) -> f32 {
    let whole = position.floor();
    let frac = (position - whole) as f32;
    let index = whole as usize;
    let mut next = index + 1;
    match looping {
        Some((loop_start, loop_end)) if next >= loop_end => next = loop_start,
        _ if next >= end => next = index,
        _ => {}
    }
    let mut read = 0.0_f32;
    for channel in 0..width {
        let s0 = frames[index * width + channel];
        let s1 = frames[next * width + channel];
        read += s0 + (s1 - s0) * frac;
    }
    if width > 1 { read * 0.5 } else { read }
}

fn fade(frames_left: u32) -> f32 {
    frames_left as f32 / SUSTAIN_FADE_FRAMES as f32
}

#[test]
fn a_note_at_the_root_plays_the_sample_at_its_recorded_rate_and_fades_on_release() {
    // ADR-0026 clauses 6 and 7: at the root the ratio is exactly one, so every output frame
    // is the sample's own frame; the off edge in `Sustain` mode starts V1's 512-frame linear
    // fade, after which the sampler is silent.
    let plan = admit(&voice_direct(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        PLAIN,
    ));
    let frames = mono_frames();
    let length = 20 * Q;
    let out = render(&plan, &[note(&plan, 60, 0, length)], 30);
    let fade_end = length as usize + SUSTAIN_FADE_FRAMES as usize;
    for (k, actual) in out.iter().copied().enumerate() {
        let expected = if k < length as usize {
            frames[k]
        } else if k < fade_end {
            frames[k] * fade(SUSTAIN_FADE_FRAMES - (k - length as usize) as u32)
        } else {
            0.0
        };
        assert_eq!(actual, expected, "frame {k}");
    }
}

#[test]
fn a_key_above_the_root_reads_at_v1s_frequency_ratio() {
    // ADR-0026 clause 6: the rate is the resolved frequency over the root's — V1's
    // `played / root` in `f64` over two `f32` frequencies — and the read at each fractional
    // position is V1's two-tap interpolation. A fifth above the root is not a dyadic ratio,
    // so every frame's weight is a real fraction and a wrong tap or a rounded rate fails.
    let plan = admit(&plain_voice());
    let frames = mono_frames();
    let rate = v1_rate(67, 60);
    assert!(
        rate > 1.49 && rate < 1.51,
        "a fifth is about three halves, got {rate}"
    );
    let out = render(&plan, &[note(&plan, 67, 0, 40 * Q)], 40);
    let mut position = 0.0_f64;
    for (k, actual) in out.iter().copied().enumerate() {
        let expected = if position >= FRAMES as f64 {
            0.0
        } else {
            v1_read(&frames, 1, FRAMES, None, position)
        };
        assert_eq!(actual, expected, "frame {k} at position {position}");
        if position < FRAMES as f64 {
            position += rate;
        }
    }
}

#[test]
fn a_stereo_sample_is_summed_to_mono_as_v1_sums_it() {
    // ADR-0026 clause 7: `(left + right) × 0.5`, per frame, after each channel's own read.
    let left = mono_frames();
    let right: Vec<f32> = left.iter().map(|value| -value * 0.75).collect();
    let interleaved: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [*l, *r])
        .collect();
    let sample = PreparedSample::prepare(
        interleaved.clone(),
        ChannelLayout::Stereo,
        SampleRate::new(48_000.0).expect("valid rate"),
    )
    .expect("the stereo fixture prepares");
    let plan = admit(&voice(
        vec![sample],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        PLAIN,
    ));
    let out = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 20);
    for (k, actual) in out.iter().copied().enumerate() {
        let expected = v1_read(&interleaved, 2, FRAMES, None, k as f64);
        assert_eq!(actual, expected, "frame {k}");
        assert_eq!(
            actual,
            (left[k] + right[k]) * 0.5,
            "frame {k}, as the half-sum"
        );
    }
}

#[test]
fn one_shot_ignores_the_off_edge_and_plays_the_region_out() {
    // ADR-0026 clause 7: `OneShot` plays to the region's end whatever the release does;
    // the envelope's gate is transparent here, so the release changes nothing audible.
    let plan = admit(&voice_direct(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        Authored {
            play_mode: PlayMode::OneShot,
            ..PLAIN
        },
    ));
    let frames = mono_frames();
    let out = render(&plan, &[note(&plan, 60, 0, Q)], 80);
    for (k, actual) in out.iter().copied().enumerate() {
        let expected = if k < FRAMES { frames[k] } else { 0.0 };
        assert_eq!(actual, expected, "frame {k}");
    }
}

#[test]
fn loop_mode_repeats_the_loop_while_held_and_fades_on_release_still_looping() {
    // ADR-0026 clause 7: the loop region repeats, the second tap wraps into the loop's
    // start at its end, and the fade runs over the still-looping read as V1's does.
    let loop_region = LoopRegion::new(
        SampleFrame::new(100),
        SampleFrame::new(250),
        region(0, FRAMES as u32),
    )
    .expect("a loop inside the region");
    let plan = admit(&voice_direct(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), Some(loop_region))]),
        Authored {
            play_mode: PlayMode::Loop,
            ..PLAIN
        },
    ));
    let frames = mono_frames();
    let length = 30 * Q;
    // A whole tone above the root, so every position is fractional and the second tap's
    // wrap into the loop's start carries weight: at integer positions it would weigh
    // nothing and a clamp there would be invisible.
    let rate = v1_rate(62, 60);
    let out = render(&plan, &[note(&plan, 62, 0, length)], 40);
    let fade_end = length as usize + SUSTAIN_FADE_FRAMES as usize;
    let mut position = 0.0_f64;
    for (k, actual) in out.iter().copied().enumerate() {
        let read = v1_read(&frames, 1, FRAMES, Some((100, 250)), position);
        let expected = if k < length as usize {
            read
        } else if k < fade_end {
            read * fade(SUSTAIN_FADE_FRAMES - (k - length as usize) as u32)
        } else {
            0.0
        };
        assert_eq!(actual, expected, "frame {k} at position {position}");
        position += rate;
        while position >= 250.0 {
            position -= 150.0;
        }
    }
    assert!(
        out[length as usize - 1] != 0.0 && out[fade_end + 8] == 0.0,
        "the loop sounded to the release and the fade ended it"
    );
}

#[test]
fn a_start_offset_seeks_into_the_region_as_v1_does() {
    // ADR-0026 clause 7: V1 starts at `crop_start + (crop_end − crop_start) × offset` when
    // the offset is above `0.001`, and at the start otherwise.
    let plan = admit(&voice(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        Authored {
            start_offset: NormalizedLevel::new(0.25).expect("in range"),
            ..PLAIN
        },
    ));
    let frames = mono_frames();
    let out = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 8);
    let from = (FRAMES as f64 * 0.25) as usize;
    for (k, actual) in out.iter().copied().enumerate() {
        assert_eq!(actual, frames[from + k], "frame {k}");
    }
    let threshold = admit(&voice(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        Authored {
            start_offset: NormalizedLevel::new(0.0005).expect("in range"),
            ..PLAIN
        },
    ));
    let out = render(&threshold, &[note(&threshold, 60, 0, 30 * Q)], 2);
    assert_eq!(
        out[..Q as usize],
        frames[..Q as usize],
        "below the threshold, no seek"
    );
}

#[test]
fn velocity_scales_the_output_under_v1s_law() {
    // ADR-0026 clause 8: `(1 − s) + s × v`, the sampler's own destination and sensitivity.
    // With the envelope ignoring velocity the render isolates the sampler's factor.
    let frames = mono_frames();
    for (s, v) in [(1.0_f32, 0.5_f32), (0.0, 0.5), (0.5, 0.25)] {
        let plan = admit(&voice(
            vec![mono_sample()],
            SampleMap::new(vec![zone(SampleRef::new(0), None)]),
            Authored {
                velocity_sensitivity: NormalizedLevel::new(s).expect("in range"),
                ..PLAIN
            },
        ));
        let out = render(
            &plan,
            &[struck(&plan, 60, NoteVelocity::saturating(v), 0, 30 * Q)],
            4,
        );
        let scale = (1.0 - s) + s * v;
        for (k, actual) in out.iter().copied().enumerate() {
            assert_eq!(actual, frames[k] * scale, "s {s} v {v} frame {k}");
        }
    }
}

#[test]
fn a_note_outside_the_zone_plays_nothing_and_is_counted() {
    // ADR-0026 clause 2: a range is the zone's, an unmatched key or velocity selects it
    // not, the sampler plays nothing for that note, and the report counts it.
    let narrow = SampleZone::new(SampleRef::new(0), key(60), region(0, FRAMES as u32))
        .selected_by_keys(KeyRange::new(key(60), key(64)).expect("a range"))
        .selected_by_velocities(
            VelocityRange::new(NoteVelocity::saturating(0.5), NoteVelocity::FULL).expect("a range"),
        );
    let plan = admit(&voice(
        vec![mono_sample()],
        SampleMap::new(vec![narrow]),
        PLAIN,
    ));
    let (inside, counted) = render_reporting(&plan, &[note(&plan, 62, 0, 4 * Q)], 6);
    assert!(inside.iter().any(|v| *v != 0.0) && counted == 0);
    let (above, counted) = render_reporting(&plan, &[note(&plan, 65, 0, 4 * Q)], 6);
    assert!(
        above.iter().all(|v| *v == 0.0),
        "a key above the range plays nothing"
    );
    assert_eq!(counted, 1);
    let (soft, counted) = render_reporting(
        &plan,
        &[struck(&plan, 62, NoteVelocity::saturating(0.25), 0, 4 * Q)],
        6,
    );
    assert!(
        soft.iter().all(|v| *v == 0.0),
        "a velocity below the range plays nothing"
    );
    assert_eq!(counted, 1);
}

#[test]
fn a_map_of_two_zones_and_a_direction_not_built_are_refused_by_name() {
    // ADR-0026 clauses 2 and 7: refused, not played from the first zone or forwards.
    let two = voice(
        vec![mono_sample()],
        SampleMap::new(vec![
            zone(SampleRef::new(0), None),
            zone(SampleRef::new(0), None),
        ]),
        PLAIN,
    );
    assert!(matches!(
        refusal(&two),
        CompileError::MapBeyondOneZone {
            node: SAMPLER,
            zones: 2
        }
    ));
    for direction in [PlayDirection::Reverse, PlayDirection::PingPong] {
        let ir = voice(
            vec![mono_sample()],
            SampleMap::new(vec![zone(SampleRef::new(0), None)]),
            Authored { direction, ..PLAIN },
        );
        assert!(matches!(
            refusal(&ir),
            CompileError::DirectionNotBuilt { node: SAMPLER, direction: d } if d == direction
        ));
    }
}

#[test]
fn a_region_past_the_sample_and_a_dangling_reference_are_refused_at_construction() {
    // ADR-0026 clause 1: validated where the sample is, before any record is prepared.
    let past = SampleZone::new(SampleRef::new(0), key(60), region(0, FRAMES as u32 + 1));
    let built = GraphIr::builder()
        .sample(mono_sample())
        .sample_map(SampleMap::new(vec![past]))
        .build();
    assert!(matches!(
        built,
        Err(IrError::RegionOutsideSample { zone: 0, .. })
    ));
    let dangling = GraphIr::builder()
        .sample_map(SampleMap::new(vec![zone(SampleRef::new(3), None)]))
        .build();
    assert!(matches!(
        dangling,
        Err(IrError::UnknownSample { zone: 0, .. })
    ));
    let unmapped = GraphIr::builder()
        .node(
            SAMPLER,
            IrNodeKind::Sampler {
                map: SampleMapRef::new(0),
                level: Amplitude::UNITY,
                velocity_sensitivity: NormalizedLevel::FULL,
                start_offset: NormalizedLevel::ZERO,
                play_mode: PlayMode::Sustain,
                direction: PlayDirection::Forward,
            },
            ExecutionScope::Voice,
        )
        .build();
    assert!(matches!(
        unmapped,
        Err(IrError::UnknownSampleMap { node: SAMPLER, .. })
    ));
    assert!(matches!(
        LoopRegion::new(SampleFrame::new(10), SampleFrame::new(300), region(0, 200)),
        Err(crate::sample::SampleError::LoopOutsideRegion { .. })
    ));
    assert!(PlaybackRegion::new(SampleFrame::new(5), SampleFrame::new(5)).is_err());
    assert!(KeyRange::new(key(64), key(60)).is_err());
    assert!(
        PreparedSample::prepare(
            vec![0.0, f32::NAN],
            ChannelLayout::Mono,
            SampleRate::new(48_000.0).expect("valid rate")
        )
        .is_err()
    );
    assert!(
        PreparedSample::prepare(
            vec![0.0, 0.1, 0.2],
            ChannelLayout::Stereo,
            SampleRate::new(48_000.0).expect("valid rate")
        )
        .is_err(),
        "three samples are not stereo frames"
    );
}

#[test]
fn equal_samples_are_held_once_and_the_charge_is_what_the_plan_holds() {
    // ADR-0026 clause 3: two references to one content are one entry, compared by the
    // frames; the IR's charge is that entry's bytes plus a slot per sampler, which is what
    // the compiled plan carries.
    let ir = voice(
        vec![mono_sample(), mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(1), None)]),
        PLAIN,
    );
    assert_eq!(ir.samples().len(), 2, "the IR holds both references");
    let plan = admit(&ir);
    assert_eq!(
        plan.prepared_samples().len(),
        1,
        "the plan holds the content once"
    );
    let held: u64 = plan
        .prepared_samples()
        .iter()
        .map(PreparedSample::prepared_bytes)
        .sum::<u64>()
        + size_of::<crate::plan::SampleSlot>() as u64;
    assert_eq!(ir.sample_bytes(), held);
    assert!(held > (FRAMES * size_of::<f32>()) as u64);
    let frames = mono_frames();
    let out = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 2);
    assert_eq!(
        out[..Q as usize],
        frames[..Q as usize],
        "the second reference plays"
    );
}

#[test]
fn a_note_sent_to_the_sampler_itself_is_refused() {
    // ADR-0026 clause 5: the sampler is never a note's address.
    let plan = admit(&plain_voice());
    assert!(plan.resolve_note(SAMPLER).is_none());
    assert!(plan.resolve_note(ENVELOPE).is_some());
}

#[test]
fn the_second_note_on_the_voice_restarts_the_read() {
    // ADR-0026 clause 9: one player state per instance, reset by the on edge. The second
    // note's first frame is the sample's first frame again, not a continuation.
    let plan = admit(&plain_voice());
    let frames = mono_frames();
    let first = 10 * Q;
    let second = first + Q;
    let out = render(
        &plan,
        &[note(&plan, 60, 0, first), note(&plan, 60, second, 20 * Q)],
        24,
    );
    assert_eq!(
        out[second as usize], frames[0],
        "the second note starts over"
    );
    assert_eq!(
        out[second as usize + 7],
        frames[7],
        "and reads the sample from its start"
    );
}

#[test]
fn the_kernel_alone_plays_from_the_on_edge() {
    // The kernel against a hand-built record and a hand-built quantum of controls, apart
    // from the renderer's plumbing: an on edge at frame 0 with the root's frequency reads
    // the sample from its first frame at rate one.
    use crate::node::kernels::{
        InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, SAMPLER_PITCH, SAMPLER_TRIGGER,
        SAMPLER_VELOCITY, TimedControl, sampler,
    };
    use crate::quantities::{Frequency, ParameterValue};
    use crate::time::QuantumOffset;
    let plan = admit(&plain_voice());
    let samples = [mono_sample()];
    let prepared = PreparedNode::Sampler {
        sample: crate::plan::SampleSlot::new(plan.id(), 0),
        keys: KeyRange::FULL,
        velocities: VelocityRange::FULL,
        root: key(60),
        root_frequency: Frequency::new(261.62555).expect("finite"),
        fine_factor: 1.0,
        region: region(0, FRAMES as u32),
        loop_region: None,
        gain: GainFactor::UNITY,
        level: Amplitude::UNITY,
        velocity_sensitivity: NormalizedLevel::FULL,
        start_offset: NormalizedLevel::ZERO,
        play_mode: PlayMode::Sustain,
    };
    let mut state = NodeState::initial(&prepared);
    let controls = [
        TimedControl {
            offset: QuantumOffset::ZERO,
            control: SAMPLER_PITCH,
            value: ParameterValue::from_frequency(Frequency::new(261.62555).expect("finite")),
        },
        TimedControl {
            offset: QuantumOffset::ZERO,
            control: SAMPLER_VELOCITY,
            value: ParameterValue::from_note_velocity(NoteVelocity::saturating(0.5)),
        },
        TimedControl {
            offset: QuantumOffset::ZERO,
            control: SAMPLER_TRIGGER,
            value: ParameterValue::ONE,
        },
    ];
    let ramps = [1.0_f32; 2 * QUANTUM_FRAMES as usize];
    let mut out = vec![0.0_f32; QUANTUM_FRAMES as usize];
    let mut io = NodeIo {
        out: &mut out,
        channels: ChannelLayout::Mono,
        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
        position: None,
        controls: &controls,
        ramps: &ramps,
        samples: &samples,
    };
    sampler(&prepared, &mut state, &mut io);
    // The velocity control is decoded and applied: at sensitivity one the scale is `v`.
    let expected: Vec<f32> = mono_frames()[..QUANTUM_FRAMES as usize]
        .iter()
        .map(|frame| frame * 0.5)
        .collect();
    assert_eq!(out, expected, "the whole quantum, at half velocity");
}

#[test]
fn a_regions_start_is_where_the_read_and_the_offset_begin() {
    // ADR-0026 clause 7: the seek is `start + (end − start) × offset`, and with no offset
    // the read begins at the region's start — an implementation computing `(end − start)
    // × offset` alone passes every test whose region starts at zero, which an independent
    // read pointed out.
    let frames = mono_frames();
    for (offset, from) in [(0.0_f32, 1_000_usize), (0.25, 1_500)] {
        let zone = SampleZone::new(SampleRef::new(0), key(60), region(1_000, 3_000));
        let plan = admit(&voice(
            vec![mono_sample()],
            SampleMap::new(vec![zone]),
            Authored {
                start_offset: NormalizedLevel::new(offset).expect("in range"),
                ..PLAIN
            },
        ));
        let out = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 4);
        for (k, actual) in out.iter().copied().enumerate() {
            assert_eq!(actual, frames[from + k], "offset {offset} frame {k}");
        }
    }
}

#[test]
fn a_sample_at_another_rate_and_loop_mode_without_a_loop_are_refused_by_name() {
    // Two refusals an independent read asked for. A sample recorded at a rate other than
    // the stream's would play mis-pitched — V1's speed formula never read a source rate
    // either — so it is refused rather than substituted; and `Loop` over a zone with no
    // loop has nothing to repeat, so it is refused rather than played as `Sustain`.
    use crate::diagnostics::PreparationFault;
    let other_rate = PreparedSample::prepare(
        mono_frames(),
        ChannelLayout::Mono,
        SampleRate::new(44_100.0).expect("valid rate"),
    )
    .expect("the sample prepares at its own rate");
    let mismatched = voice(
        vec![other_rate],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        PLAIN,
    );
    assert!(matches!(
        refusal(&mismatched),
        CompileError::NodeNotPreparable {
            node: SAMPLER,
            fault: PreparationFault::SampleRateMismatch { .. }
        }
    ));
    let unlooped = voice(
        vec![mono_sample()],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        Authored {
            play_mode: PlayMode::Loop,
            ..PLAIN
        },
    );
    assert!(matches!(
        refusal(&unlooped),
        CompileError::NodeNotPreparable {
            node: SAMPLER,
            fault: PreparationFault::LoopWithoutRegion
        }
    ));
}

#[test]
fn a_sample_no_sampler_reaches_is_not_prepared_into_the_plan() {
    // ADR-0026 clause 3 and the charge: the IR holds two different samples and the one
    // zone plays the first, so the plan prepares one and the charge is that one. An
    // independent read found every IR sample prepared, paid for or not.
    let other: Vec<f32> = mono_frames().iter().map(|v| v * 0.5).collect();
    let unreached = PreparedSample::prepare(
        other,
        ChannelLayout::Mono,
        SampleRate::new(48_000.0).expect("valid rate"),
    )
    .expect("the second sample prepares");
    let ir = voice(
        vec![mono_sample(), unreached],
        SampleMap::new(vec![zone(SampleRef::new(0), None)]),
        PLAIN,
    );
    let plan = admit(&ir);
    assert_eq!(plan.prepared_samples().len(), 1);
    assert_eq!(
        plan.prepared_samples()[0].frames(),
        mono_frames().as_slice()
    );
    let held: u64 = plan
        .prepared_samples()
        .iter()
        .map(PreparedSample::prepared_bytes)
        .sum::<u64>()
        + size_of::<crate::plan::SampleSlot>() as u64;
    assert_eq!(ir.sample_bytes(), held);
}

#[test]
fn a_fine_tune_scales_the_rate_by_v1s_factor() {
    // ADR-0026 clause 6: the rate carries `2^(fine_cents / 1200)`, V1's own factor, on top
    // of the frequency ratio; at the root that factor alone is the rate.
    let zone = SampleZone::new(SampleRef::new(0), key(60), region(0, FRAMES as u32))
        .tuned_by(crate::quantities::Cents::new(50.0).expect("finite"));
    let plan = admit(&voice(
        vec![mono_sample()],
        SampleMap::new(vec![zone]),
        PLAIN,
    ));
    let frames = mono_frames();
    let rate = 2.0_f64.powf(50.0 / 1_200.0);
    let out = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 8);
    let mut position = 0.0_f64;
    for (k, actual) in out.iter().copied().enumerate() {
        assert_eq!(
            actual,
            v1_read(&frames, 1, FRAMES, None, position),
            "frame {k} at position {position}"
        );
        position += rate;
    }
}
