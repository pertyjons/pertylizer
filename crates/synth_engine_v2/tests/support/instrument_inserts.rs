//! `CORPUS-0005`'s saved `instrument-inserts` project as `pertylizer`'s project lowering
//! builds it, transcribed through `synth_engine_v2`'s public builder for EVD-0026.
//!
//! `synth_engine_v2` cannot load a project and no shipping crate may enable the mixed
//! rehearsal (ADR-0078 clause 1), so this source file bridges the
//! saved graph. `pertylizer`'s `lowering::render::evd_0026` includes it and requires its
//! ordinary and mixed lowerings of the pinned project to equal [`ir`] exactly, and its
//! lowered events to equal [`events_at`]. The two-source host uses the same graph;
//! both test binaries check [`SOURCE_SHA256`] against the project file.
//!
//! The graph uses the parity output policy and does not depend on block size or sample
//! rate. Four independent literal timelines pin the candidate sample rates.

use synth_engine_v2::{
    controller::BipolarLevel,
    ir::{
        ChannelTag, ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration,
        PlanDeclarations, PortId, SignalDomain,
    },
    output::OutputLimiting,
    plan::NoteSlot,
    quantities::{
        Amplitude, ChannelLayout, CutoffFrequency, DelayFeedback, DelayTime, EventCount, Frequency,
        HeldNoteCount, KeyIdentity, NormalizedLevel, NoteVelocity, Resonance, SampleRate, Seconds,
    },
    schedule::CompiledPayload,
    tuning::PreparedTuning,
};

/// The saved project, relative to the repository root.
pub const SOURCE: &str = "corpus/v2-reference/projects/instrument-inserts.ptz";

/// The saved project's SHA-256, as `corpus/v2-reference/manifest.json` pins it.
pub const SOURCE_SHA256: &str = "1d55668d424e71cb49d23c5da46f7f35541b8c16669dc1399a8ef28fefe12dbc";

/// The voice patch: `osc-1`, `flt-1`, `env-1`, `amp-1` and `out-1`.
pub const OSCILLATOR: NodeId = NodeId::new(1);
pub const FILTER: NodeId = NodeId::new(262_145);
/// The one node a note plays.
pub const ENVELOPE: NodeId = NodeId::new(327_681);
pub const AMPLIFIER: NodeId = NodeId::new(458_753);
pub const VOICE_OUTPUT: NodeId = NodeId::new(589_825);
/// The insert chain, in the saved order: distortion, then delay.
pub const DELAY: NodeId = NodeId::new(655_361);
pub const DISTORTION: NodeId = NodeId::new(786_433);
/// The stages the lowering inserts: velocity and the track's balance per voice, then the
/// instrument's channel and V1's channel-stage clipper.
pub const VELOCITY: NodeId = NodeId::new(16_711_680);
pub const BALANCE: NodeId = NodeId::new(16_711_681);
pub const CHANNEL: NodeId = NodeId::new(16_711_682);
pub const CHANNEL_CLIP: NodeId = NodeId::new(16_711_683);
/// The master: sum, trim, V1's output clamp, the meter and the plan's output.
pub const MASTER_MIX: NodeId = NodeId::new(2_147_418_112);
pub const MASTER_TRIM: NodeId = NodeId::new(2_147_418_113);
pub const MASTER_CLAMP: NodeId = NodeId::new(2_147_418_114);
pub const MASTER_OUTPUT: NodeId = NodeId::new(2_147_418_115);
pub const MASTER_METER: NodeId = NodeId::new(2_147_418_116);
/// The reserved explicit mono fold, after the stereo master tap.
pub const MASTER_MONO: NodeId = NodeId::new(2_147_418_117);

/// The compiled producer the lowering declares: the arrangement's peak of two
/// simultaneous notes, and no release holds.
pub const COMPILED_NOTES: u32 = 2;

/// The live range EVD-0026's mixed lowering appends: four notes and four holds.
///
/// A harness input rather than a property of the project, which says nothing about a live
/// keyboard. EVD-0026's frozen sound script still has to fix the live span.
pub fn live() -> NoteProducerDeclaration {
    NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(4),
        simultaneous_holds: EventCount::measured(4),
    }
}

fn level(value: f32) -> NormalizedLevel {
    NormalizedLevel::new(value).expect("a transcribed level")
}

fn centre() -> BipolarLevel {
    BipolarLevel::new(0.0).expect("a transcribed pan")
}

/// The lowered graph. `None` is the ordinary lowering, one compiled producer; `Some`
/// appends that live producer after it and adds its notes to the plan's held notes, which
/// is the only difference the mixed lowering is meant to make.
pub fn ir(live: Option<NoteProducerDeclaration>) -> GraphIr {
    ir_with_layout(live, ChannelLayout::Stereo)
}

/// The same saved graph with an explicit terminal fold for a requested mono sink.
pub fn ir_with_layout(live: Option<NoteProducerDeclaration>, layout: ChannelLayout) -> GraphIr {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(COMPILED_NOTES),
        simultaneous_holds: EventCount::NONE,
    };
    let held = live.map_or(0, |live| live.simultaneous_notes.get());
    let channel = ExecutionScope::Channel(ChannelTag::new(0));
    let audio = |builder: synth_engine_v2::ir::GraphIrBuilder, from: NodeId, to: NodeId| {
        builder.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        )
    };
    let mut builder = GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Saw {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::UNITY,
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(2_400.0).expect("a transcribed cutoff"),
                resonance: Resonance::new(0.588_235_26).expect("a transcribed resonance"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.005).expect("finite"),
                decay: Seconds::new(0.18).expect("finite"),
                sustain: level(0.0),
                release: Seconds::new(0.1).expect("finite"),
                velocity_sensitivity: level(1.0),
            },
            ExecutionScope::Voice,
        )
        .node(
            AMPLIFIER,
            IrNodeKind::VoiceAmplifier { pan: centre() },
            ExecutionScope::Voice,
        )
        .node(
            VOICE_OUTPUT,
            IrNodeKind::VoiceOutput {
                master: level(1.0),
                pan: centre(),
                muted: false,
                limiting: OutputLimiting::SoftKnee,
            },
            ExecutionScope::Voice,
        )
        .node(
            DELAY,
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.25).expect("a transcribed time"),
                time_right: DelayTime::new(0.25).expect("a transcribed time"),
                feedback: DelayFeedback::new(0.45).expect("a transcribed feedback"),
                mix: level(0.5),
                tone: level(0.4),
            },
            ExecutionScope::InstrumentInstance,
        )
        .node(
            DISTORTION,
            IrNodeKind::Distortion {
                drive: level(0.7),
                tone: level(0.8),
                mix: level(1.0),
            },
            ExecutionScope::InstrumentInstance,
        )
        .node(
            VELOCITY,
            IrNodeKind::StereoVelocityScaler {
                sensitivity: level(1.0),
            },
            ExecutionScope::Voice,
        )
        .node(
            BALANCE,
            IrNodeKind::Balance {
                level: Amplitude::UNITY,
                pan: centre(),
                muted: false,
            },
            ExecutionScope::Voice,
        )
        .node(
            CHANNEL,
            IrNodeKind::Channel {
                fader: Amplitude::UNITY,
                pan: centre(),
                muted: false,
            },
            channel,
        )
        .node(CHANNEL_CLIP, IrNodeKind::SoftClip, channel)
        .node(MASTER_MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .node(
            MASTER_TRIM,
            IrNodeKind::Trim {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(MASTER_CLAMP, IrNodeKind::HardClamp, ExecutionScope::Global)
        .node(MASTER_OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .node(
            MASTER_METER,
            IrNodeKind::StereoMonitor,
            ExecutionScope::Global,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, PortId::new(1)),
            SignalDomain::Control,
        );
    // The audio edges in the order the lowering connects them; an edge's identity is its
    // position.
    for (from, to) in [
        (AMPLIFIER, VOICE_OUTPUT),
        (OSCILLATOR, FILTER),
        (FILTER, AMPLIFIER),
        (VOICE_OUTPUT, VELOCITY),
        (VELOCITY, BALANCE),
        (BALANCE, DISTORTION),
        (DISTORTION, DELAY),
        (DELAY, CHANNEL),
        (CHANNEL, CHANNEL_CLIP),
        (CHANNEL_CLIP, MASTER_MIX),
        (MASTER_MIX, MASTER_TRIM),
        (MASTER_TRIM, MASTER_CLAMP),
        (MASTER_CLAMP, MASTER_METER),
    ] {
        builder = audio(builder, from, to);
    }
    if layout == ChannelLayout::Mono {
        builder = builder.node(
            MASTER_MONO,
            IrNodeKind::StereoToMono,
            ExecutionScope::Global,
        );
        builder = audio(builder, MASTER_METER, MASTER_MONO);
        builder = audio(builder, MASTER_MONO, MASTER_OUTPUT);
    } else {
        builder = audio(builder, MASTER_METER, MASTER_OUTPUT);
    }
    builder
        .tuning(
            ExecutionScope::Voice,
            PreparedTuning::equal_temperament().expect("twelve-tone equal temperament"),
        )
        .declaring(PlanDeclarations {
            note_producers: std::iter::once(compiled).chain(live).collect(),
            held_notes: HeldNoteCount::measured(COMPILED_NOTES + held),
            // The arrangement's peak: the dyad's two edges in one quantum.
            events_per_quantum: EventCount::measured(2),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("the transcribed graph builds")
}

/// The pinned project's 120 BPM timeline: a held fifth from the song start, then one
/// short note. The four literal tables are independent of the production tempo mapper:
/// the first release is 3/8 seconds, the later onset 1 second and its release 9/8 seconds,
/// rounded once, half away from zero. This pins both half-frame releases at 44.1 kHz.
pub fn events_at(slot: NoteSlot, sample_rate: SampleRate) -> [(u64, CompiledPayload); 6] {
    let [dyad_off, later_on, later_off] = match sample_rate.as_f32() {
        44_100.0 => [16_538, 44_100, 49_613],
        48_000.0 => [18_000, 48_000, 54_000],
        96_000.0 => [36_000, 96_000, 108_000],
        192_000.0 => [72_000, 192_000, 216_000],
        rate => panic!("rate {rate} is outside the transcribed candidate envelope"),
    };
    let velocity = NoteVelocity::new(0.755_905_5).expect("a transcribed velocity");
    let key = |raw| KeyIdentity::new(raw).expect("a transcribed key");
    let on = |raw| CompiledPayload::NoteOn {
        slot,
        key: key(raw),
        velocity,
    };
    let off = |raw| CompiledPayload::NoteOff {
        slot,
        key: key(raw),
    };
    [
        (0, on(48)),
        (0, on(55)),
        (dyad_off, off(48)),
        (dyad_off, off(55)),
        (later_on, on(60)),
        (later_off, off(60)),
    ]
}
