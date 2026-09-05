//! EVD-0013's harness: the equivalence pair, its controls, and the two
//! measurements `pertylizer compare` cannot make.
//!
//! Three subcommands, run in the order the record's *Method* states:
//!
//! ```text
//! evd_0013_equivalence fixtures <dir>   # write the V1 projects
//! evd_0013_equivalence v2 <dir>         # render the V2 arms to WAV
//! evd_0013_equivalence measure <dir>    # C1, C2, E2b and E4's V1 half
//! ```
//!
//! The V1 renders happen between the first and the last, through
//! `pertylizer render` — the shipped offline path, and the one the corpus
//! digests are taken through. E1, E2a, E3a and E3b come from
//! `pertylizer compare` and are not computed here.
//!
//! # Why the V2 arm is built here rather than in `synth_engine_v2`
//!
//! It needs a WAV writer and the V1 fixture builder, neither of which belongs in
//! that crate. `synth_engine_v2` is a dev-dependency of this package for exactly
//! this reason and for EVD-0014's, which needs both engines in one binary.

use std::path::{Path, PathBuf};

use pertylizer::audio::wav_format::WavFormat;
use pertylizer::compare::Signal;
use pertylizer::corpus::fixtures::equivalence_probe;
use pertylizer::synth_core::Hertz;
use pertylizer::synth_sequencer::{PatternTick, Pitch};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, Resonance, SampleRate,
    Seconds,
};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime};

/// The corpus render rate, and the rate both arms' filter coefficients are
/// derived against.
const SAMPLE_RATE: u32 = 44_100;

/// The whole render: the corpus window for `CORPUS-0001`, 2 s plus a 1 s tail.
const TOTAL_FRAMES: u64 = 132_300;

/// Where the note is released. The V1 fixture's note is 2 880 ticks long, and at
/// the default 120 BPM with 960 ticks to the quarter that is exactly 1.5 s — so
/// attack, decay, sustain and release all complete inside the visible range and
/// the tail is silence.
const NOTE_OFF_FRAME: u64 = 66_150;

/// V1's `BUFFER_SIZE`, and therefore the block E4's V1 half is quantised to.
const BLOCK: u64 = 256;

/// The fixture's filter corner.
const CORNER_HZ: f32 = 1_000.0;

/// Control **C2**'s corner: far above the fundamental, so the stage is
/// near-transparent and what the level ratio measures is the gain staging.
const NULL_CORNER_HZ: f32 = 20_000.0;

/// E3a's six points: one per octave band, every one an exact octave of A so
/// `Frequency::from_midi`'s `powf` is exact and both arms are at one frequency
/// by construction.
const SWEEP: [(u8, f32); 6] = [
    (45, 110.0),
    (57, 220.0),
    (69, 440.0),
    (81, 880.0),
    (93, 1_760.0),
    (105, 3_520.0),
];

/// The fixture's own pitch: A4, E3a's third point.
const BASE_MIDI: u8 = 69;

/// E4's V1 family spans ticks 0 to this. One tick is 22.968 75 samples at the
/// default tempo, so 256 samples is about 11.1 ticks and this covers more than
/// two blocks — enough for the step function to show two risers.
const OFFSET_TICKS: u32 = 25;

/// The envelope, identical in both arms.
const ATTACK_S: f32 = 0.010;
const DECAY_S: f32 = 0.100;
const SUSTAIN: f32 = 0.700;
const RELEASE_S: f32 = 0.200;

/// V2's sine amplitude, which cancels V1's chain gain.
///
/// That gain is **three** equal-power centre pans, not two: `Amplifier::process`
/// writes its mono `OUT` as the mean of two panned channels, `StereoOutput`
/// pans again per channel, and then `SynthEngine::process` applies the
/// instrument fader's own `Gain::from_pan`
/// (`crates/synth_engine/src/synth_engine.rs:3835`). In `f32` the product is
/// 0.353 553 354 740 142 8, or `0x3eb504f2`.
///
/// **Control C2 is what found the third one.** An earlier draft of EVD-0013
/// derived two, set this to 0.5, and the gain null came back at +3.008 dB —
/// almost exactly one pan. The residual after the correction is the oscillator
/// approximation's +0.0029 dB and nothing else.
const V2_AMPLITUDE: f32 = 0.353_553_35;

const ENVELOPE: NodeId = NodeId::new(1);
const SINE: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(9);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let dir = PathBuf::from(args.next().unwrap_or_else(|| ".".to_string()));

    match command.as_str() {
        "fixtures" => write_fixtures(&dir),
        "v2" => render_v2(&dir),
        "measure" => measure(&dir),
        other => {
            Err(format!("unknown subcommand {other:?}; expected fixtures, v2 or measure").into())
        }
    }
}

// ---------------------------------------------------------------------------
// Every arm this record renders, named once so the three subcommands agree
// ---------------------------------------------------------------------------

/// One thing to render in both engines.
struct Arm {
    name: String,
    midi: u8,
    frequency: f32,
    start_tick: u32,
    cutoff_hz: f32,
}

fn arms() -> Vec<Arm> {
    let mut out = vec![
        // The reference fixture: E1, E2a, E2b, E3b.
        Arm {
            name: "aligned".to_string(),
            midi: BASE_MIDI,
            frequency: 440.0,
            start_tick: 0,
            cutoff_hz: CORNER_HZ,
        },
        // Control C2's gain null.
        Arm {
            name: "null".to_string(),
            midi: BASE_MIDI,
            frequency: 440.0,
            start_tick: 0,
            cutoff_hz: NULL_CORNER_HZ,
        },
    ];
    // E3a. `sweep-69` would be `aligned` again, so it is skipped rather than
    // rendered twice; the analysis reads `aligned` for that point.
    for (midi, frequency) in SWEEP {
        if midi == BASE_MIDI {
            continue;
        }
        out.push(Arm {
            name: format!("sweep-{midi}"),
            midi,
            frequency,
            start_tick: 0,
            cutoff_hz: CORNER_HZ,
        });
    }
    // E4's V1 half. Rendered in V2 as well only so the two subcommands stay
    // symmetric; E4 does not read the V2 renders, because V2's half is the
    // crate's own `note_events` test.
    for tick in 0..OFFSET_TICKS {
        out.push(Arm {
            name: format!("offset-{tick:02}"),
            midi: BASE_MIDI,
            frequency: 440.0,
            start_tick: tick,
            cutoff_hz: CORNER_HZ,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

fn write_fixtures(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let projects = dir.join("v1");
    std::fs::create_dir_all(&projects)?;
    std::fs::create_dir_all(dir.join("wav"))?;
    for arm in arms() {
        let path = projects.join(format!("{}.ptz", arm.name));
        let pitch = Pitch::new(arm.midi).ok_or("the sweep's MIDI notes are in range")?;
        equivalence_probe(
            pitch,
            PatternTick(arm.start_tick),
            Hertz::new(arm.cutoff_hz),
        )
        .save(&path)?;
        println!("{}", path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// v2
// ---------------------------------------------------------------------------

/// V2's `voice-mono` graph at a stereo profile — ADR-0041 clause 16's first
/// baseline fixture, which is also EVD-0012's governing shape.
/// The tuning the V2 arm resolves its key through.
///
/// `SOUND-INV-021` puts the key-to-frequency mapping in the plan, so the arm's frequency now
/// arrives through the note rather than through the oscillator's prepared value. That does not
/// change what this record measures: **measured, not assumed** — every one of E3a's six points
/// is an exact octave of A, and `TuningTable::equal_temperament` reproduces 110, 220, 440, 880,
/// 1760 and 3520 Hz bit-for-bit, so both arms still receive one frequency by construction.
fn twelve_tet() -> synth_engine_v2::tuning::PreparedTuning {
    synth_engine_v2::tuning::PreparedTuning::equal_temperament()
        .expect("twelve-tone equal temperament prepares")
}

fn v2_graph(frequency: f32, cutoff_hz: f32) -> GraphIr {
    GraphIr::builder()
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(ATTACK_S).expect("finite"),
                decay: Seconds::new(DECAY_S).expect("finite"),
                sustain: NormalizedLevel::new(SUSTAIN).expect("in range"),
                release: Seconds::new(RELEASE_S).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            SINE,
            IrNodeKind::Sine {
                frequency: Frequency::new(frequency).expect("finite"),
                amplitude: Amplitude::new(V2_AMPLITUDE).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(cutoff_hz).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SINE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, twelve_tet())
        .declaring(synth_engine_v2::ir::PlanDeclarations {
            // A plan that starts notes must say who starts them: the identity range a
            // compiled note-on mints from is partitioned across declared producers.
            note_producers: vec![synth_engine_v2::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(1),
                simultaneous_holds: synth_engine_v2::quantities::EventCount::NONE,
            }],
            ..synth_engine_v2::ir::PlanDeclarations::default()
        })
        .build()
        .expect("the voice path is a readable plan")
}

/// One V2 arm, rendered offline as interleaved stereo `f32`.
fn v2_samples(
    midi: u8,
    frequency: f32,
    cutoff_hz: f32,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let profile = HostProfile::harness(
        SampleRate::new(SAMPLE_RATE as f32).map_err(|e| format!("a valid rate: {e:?}"))?,
        FrameCount::new(BLOCK),
        ChannelLayout::Stereo,
    )
    .map_err(|e| format!("the harness profile is valid: {e:?}"))?;
    let plan = compile(&v2_graph(frequency, cutoff_hz), &RenderConfig::new(profile))
        .into_plan()
        .map_err(|error| format!("the fixture is admissible: {error:?}"))?;
    let slot = plan
        .resolve_note(ENVELOPE)
        .ok_or("the envelope is playable")?;

    // The note starts at plan sample 0 and is released where V1's 2 880-tick
    // note ends. `render_offline` is latency-compensated, so its first output
    // sample is plan sample 0.
    let events = [
        OfflineEvent::new(
            SampleTime::ZERO,
            CompiledPayload::NoteOn {
                slot,
                // **The arm's own key**, not a fixed one. Since `SOUND-INV-021` the note's key
                // resolves through the plan's tuning and reaches the oscillator, so a constant
                // here would render every sweep point at one pitch and E1 and E3a would be
                // comparing arms that are not at the same frequency. An independent review
                // found exactly that.
                key: synth_engine_v2::quantities::KeyIdentity::new(midi)
                    .map_err(|e| format!("the sweep's MIDI notes are keyboard positions: {e:?}"))?,
                velocity: synth_engine_v2::quantities::NoteVelocity::FULL,
            },
        ),
        OfflineEvent::new(
            SampleTime::new(NOTE_OFF_FRAME),
            CompiledPayload::NoteOff {
                slot,
                key: synth_engine_v2::quantities::KeyIdentity::new(midi)
                    .map_err(|e| format!("the sweep's MIDI notes are keyboard positions: {e:?}"))?,
            },
        ),
    ];
    Ok(render_offline(
        plan,
        FrameCount::new(TOTAL_FRAMES),
        PlanPosition::ZERO,
        &events,
    )
    .map_err(|error| format!("the fixture renders: {error:?}"))?)
}

fn render_v2(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let wav = dir.join("wav");
    std::fs::create_dir_all(&wav)?;
    for arm in arms() {
        let samples = v2_samples(arm.midi, arm.frequency, arm.cutoff_hz)?;
        let path = wav.join(format!("v2-{}.wav", arm.name));
        write_wav(&path, &samples)?;
        println!("{}", path.display());

        // Control C1 renders each arm twice. Only the reference fixture and the
        // null need it: a determinism defect would not be arm-specific, and
        // fifty extra renders would say nothing the first two do not.
        if arm.name == "aligned" || arm.name == "null" {
            let repeat = v2_samples(arm.midi, arm.frequency, arm.cutoff_hz)?;
            let path = wav.join(format!("v2-{}-b.wav", arm.name));
            write_wav(&path, &repeat)?;
            println!("{}", path.display());
        }
    }
    Ok(())
}

fn write_wav(path: &Path, interleaved: &[f32]) -> Result<(), Box<dyn std::error::Error>> {
    let spec = WavFormat::Float32.spec(2, SAMPLE_RATE);
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in interleaved {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// measure
// ---------------------------------------------------------------------------

/// The 10 ms window every landmark in this record is read on — E2a's, through
/// `pertylizer compare`, and E2b's, here.
const WINDOW_MS: f32 = pertylizer::audio::analysis::spectrum::ENV_ALIGN_WINDOW_MS;

fn measure(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let wav = dir.join("wav");

    println!("# EVD-0013 harness: C1, C2, E2b and E4's V1 half");
    println!("sample_rate,{SAMPLE_RATE}");
    println!("total_frames,{TOTAL_FRAMES}");
    println!("note_off_frame,{NOTE_OFF_FRAME}");
    println!("envelope_window_ms,{WINDOW_MS}");
    println!();

    control_c1(&wav)?;
    println!();
    control_c2(&wav)?;
    println!();
    measure_e2b(&wav)?;
    println!();
    measure_e4(&wav)?;
    println!();
    attribute_by_region(&wav)?;
    Ok(())
}

/// Where the whole-render difference lives, region by region.
///
/// The band comparison reports one number for the render; this splits it into
/// the segments that produced it, so a difference can be **attributed** rather
/// than merely observed. It is what says the constant per-band offset is the
/// envelope and not the oscillator or the filter — a frequency-independent
/// difference concentrated in the release is a shape the other two causes
/// cannot make.
///
/// Retained rather than computed by hand: these figures are verified premises
/// of ADR-0042, and a premise a reader cannot recompute is an assertion.
fn attribute_by_region(wav: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let v1 = Signal::load(&wav.join("v1-aligned.wav"))?;
    let v2 = Signal::load(&wav.join("v2-aligned.wav"))?;
    let (left, right) = (v1.mono(), v2.mono());
    let rate = v1.sample_rate as f32;

    let regions: [(&str, usize, usize); 4] = [
        // The attack and the decay, up to the first sustain window.
        ("attack+decay", 0, ((ATTACK_S + DECAY_S) * rate) as usize),
        // Wholly inside sustain, on both sides.
        (
            "sustain",
            ((ATTACK_S + DECAY_S + 0.04) * rate) as usize,
            NOTE_OFF_FRAME as usize - 1_000,
        ),
        // The release, from the *nominal* gate. V1's actual gate is earlier —
        // see the own-gate rows below — so this row carries the scheduling
        // difference as well as the curve's.
        (
            "release (shared window)",
            NOTE_OFF_FRAME as usize,
            (NOTE_OFF_FRAME as f32 + RELEASE_S * rate * 1.25) as usize,
        ),
        ("whole render", 0, left.len()),
    ];

    println!("# Attribution: where the whole-render difference lives");
    println!("region,first_frame,last_frame,v1_rms,v2_rms,delta_db");
    for (name, first, last) in regions {
        let last = last.min(left.len()).min(right.len());
        let (Some(a), Some(b)) = (left.get(first..last), right.get(first..last)) else {
            continue;
        };
        let rms = |x: &[f32]| {
            (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
        };
        let (first_rms, second_rms) = (rms(a), rms(b));
        let delta = 20.0 * (second_rms / first_rms).log10();
        println!("{name},{first},{last},{first_rms:.7},{second_rms:.7},{delta:+.4}");
    }

    // The release again, each arm measured from **its own** gate.
    //
    // V1 routes a block's events before rendering it, so a note-off authored at
    // `NOTE_OFF_FRAME` is applied at the start of the block containing it — 102
    // frames earlier at this fixture. A shared window therefore compares V1's
    // release 102 frames in against V2's at its start, and reports the
    // scheduling difference (E4's subject) as though it were curve shape. This
    // row removes it, and the gap between the two rows is how much of the
    // shared-window figure was timing rather than shape.
    let v1_gate = (NOTE_OFF_FRAME - NOTE_OFF_FRAME % BLOCK) as usize;
    let span = (RELEASE_S * rate * 1.25) as usize;
    let rms = |x: &[f32]| {
        (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
    };
    if let (Some(a), Some(b)) = (
        left.get(v1_gate..(v1_gate + span).min(left.len())),
        right.get(NOTE_OFF_FRAME as usize..(NOTE_OFF_FRAME as usize + span).min(right.len())),
    ) {
        let (first_rms, second_rms) = (rms(a), rms(b));
        let delta = 20.0 * (second_rms / first_rms).log10();
        println!(
            "release (own gate: v1 at {v1_gate}, v2 at {NOTE_OFF_FRAME}),,,{first_rms:.7},             {second_rms:.7},{delta:+.4}"
        );
    }
    Ok(())
}

/// **C1 — determinism.** Each arm rendered twice and compared against itself.
/// Exact equality, or nothing downstream means anything.
fn control_c1(wav: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("# C1 determinism: an arm against its own second render");
    println!("control,arm,engine,frames,identical");
    for arm in ["aligned", "null"] {
        for engine in ["v1", "v2"] {
            let first = Signal::load(&wav.join(format!("{engine}-{arm}.wav")))?;
            let second = Signal::load(&wav.join(format!("{engine}-{arm}-b.wav")))?;
            let a = first.mono();
            let b = second.mono();
            let identical =
                a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits());
            println!("C1,{arm},{engine},{},{identical}", a.len());
        }
    }
    Ok(())
}

/// **C2 — the gain null.** With the corner far above the fundamental the filter
/// is near-transparent, so the level inside sustain is the gain staging alone.
/// Must be 0.00 dB within 0.05 dB.
fn control_c2(wav: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("# C2 gain null: sustain-window level, V2 against V1");
    println!("control,reference_rms,candidate_rms,delta_db,within_0p05_db");
    let v1 = Signal::load(&wav.join("v1-null.wav"))?;
    let v2 = Signal::load(&wav.join("v2-null.wav"))?;
    let reference = sustain_level(&v1);
    let candidate = sustain_level(&v2);
    let delta = 20.0 * (candidate / reference).log10();
    println!(
        "C2,{reference:.9},{candidate:.9},{delta:+.5},{}",
        delta.abs() <= 0.05
    );
    Ok(())
}

/// **E2b.** The two landmarks `EnvelopeDifference` does not report: the sustain
/// level, and where the decay reaches it.
fn measure_e2b(wav: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("# E2b: the sustain level and the decay endpoint");
    println!("threshold,engine,sustain_rms,decay_end_ms");
    let mut levels = Vec::new();
    for engine in ["v1", "v2"] {
        let signal = Signal::load(&wav.join(format!("{engine}-aligned.wav")))?;
        let level = sustain_level(&signal);
        let end = decay_end_ms(&signal, level);
        levels.push((level, end));
        println!(
            "E2b,{engine},{level:.9},{}",
            end.map_or("none".to_string(), |ms| format!("{ms:.1}"))
        );
    }
    if let (Some((reference, reference_end)), Some((candidate, candidate_end))) =
        (levels.first(), levels.get(1))
    {
        let delta = 20.0 * (candidate / reference).log10();
        println!("threshold,quantity,delta,limit,met");
        println!(
            "E2b,sustain_level_db,{delta:+.5},0.1,{}",
            delta.abs() <= 0.1
        );
        if let (Some(a), Some(b)) = (reference_end, candidate_end) {
            let shift = b - a;
            println!(
                "E2b,decay_end_ms,{shift:+.1},{WINDOW_MS},{}",
                shift.abs() <= WINDOW_MS
            );
        }
    }
    Ok(())
}

/// **E4's V1 half.** Every onset difference a multiple of 256, and distinct
/// ticks inside one block sharing an onset.
///
/// Differenced within V1's own family, where the DSP latency is one constant:
/// `Oscillator::note_on` re-seeds every unison phase, and with `uni_phase` at
/// 0.0 the seed is `Phase::ZERO`, so every note in the family starts from the
/// same phase.
fn measure_e4(wav: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("# E4 V1 half: onset against authored tick");
    println!("threshold,tick,onset_frame,delta_from_tick_0,multiple_of_256");
    let mut baseline: Option<usize> = None;
    let mut deltas = Vec::new();
    for tick in 0..OFFSET_TICKS {
        let signal = Signal::load(&wav.join(format!("v1-offset-{tick:02}.wav")))?;
        let Some(onset) = first_crossing(&signal) else {
            println!("E4,{tick},none,,");
            continue;
        };
        let base = *baseline.get_or_insert(onset);
        let delta = onset as i64 - base as i64;
        let aligned = delta.rem_euclid(BLOCK as i64) == 0;
        deltas.push((tick, delta));
        println!("E4,{tick},{onset},{delta},{aligned}");
    }

    let all_aligned = deltas
        .iter()
        .all(|(_, delta)| delta.rem_euclid(BLOCK as i64) == 0);
    let distinct: std::collections::BTreeSet<i64> = deltas.iter().map(|(_, d)| *d).collect();
    println!("threshold,quantity,value,met");
    println!("E4,every_delta_multiple_of_256,{all_aligned},{all_aligned}");
    println!(
        "E4,distinct_onsets_over_{OFFSET_TICKS}_ticks,{},{}",
        distinct.len(),
        distinct.len() < OFFSET_TICKS as usize
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The two estimators, named rather than improvised
// ---------------------------------------------------------------------------

/// The mean of the envelope windows lying wholly inside sustain.
///
/// From attack plus decay plus one window, to the window before the gate falls.
/// Over the 10 ms RMS envelope `pertylizer compare` uses, not over raw samples:
/// a filtered sine crosses every level near zero once per cycle, so "the level"
/// is not a quantity until an estimator says what it means.
fn sustain_level(signal: &Signal) -> f32 {
    let envelope =
        pertylizer::audio::analysis::rms_envelope(&signal.mono(), signal.sample_rate, WINDOW_MS);
    let window_frames = WINDOW_MS / 1000.0 * signal.sample_rate as f32;
    let first =
        ((ATTACK_S + DECAY_S) * signal.sample_rate as f32 / window_frames).ceil() as usize + 1;
    let last = (NOTE_OFF_FRAME as f32 / window_frames).floor() as usize;
    let slice = envelope.get(first..last.min(envelope.len())).unwrap_or(&[]);
    if slice.is_empty() {
        return 0.0;
    }
    slice.iter().sum::<f32>() / slice.len() as f32
}

/// The first envelope window after the peak whose level is within 0.5% of
/// `sustain`.
fn decay_end_ms(signal: &Signal, sustain: f32) -> Option<f32> {
    let envelope =
        pertylizer::audio::analysis::rms_envelope(&signal.mono(), signal.sample_rate, WINDOW_MS);
    let peak_index = envelope
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(index, _)| index)?;
    envelope
        .get(peak_index..)?
        .iter()
        .position(|level| (level - sustain).abs() <= sustain * 0.005)
        .map(|offset| (peak_index + offset) as f32 * WINDOW_MS)
}

/// The first frame whose absolute value exceeds 10⁻⁴ of this render's own later
/// peak.
///
/// The threshold does not have to be principled, only identical across the
/// renders of one engine — which it is, since it is taken relative to each
/// render's own peak and those renders differ only in where an identical note
/// sits.
fn first_crossing(signal: &Signal) -> Option<usize> {
    let mono = signal.mono();
    let peak = mono.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
    if peak <= 0.0 {
        return None;
    }
    let threshold = peak * 1.0e-4;
    mono.iter().position(|x| x.abs() > threshold)
}
