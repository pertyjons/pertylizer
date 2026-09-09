//! One bounded in-process smoke render of a saved project through V2.
//!
//! This is the second half of the first Phase 4 slice: the lowerer can represent a bounded
//! subset, and this is what proves it by rendering. Deliberately narrow — one project, one
//! in-memory buffer, no job contract, no streaming, no cancellation. ADR-0028 may remain
//! `Deferred` for exactly this scope and must be `Accepted` before any of those.
//!
//! # A whole project through one plan (`P08-S002`)
//!
//! Every instrument the project holds lowers into one graph: its voice patch in the voice
//! scope, then V1's stages in V1's order — the velocity scaler and the track's balance per
//! voice, the instrument's channel and V1's channel-stage clipper on the voice sum — into
//! one master sum, the master volume as a trim, V1's output clamp, and the plan's one
//! output. The voice scope is instantiated once per note the arrangement holds open at once
//! across the project, every instrument's chain in every instance, so a note lands on any
//! free instance and the voice sum carries only the instances that sound (`SOUND-INV-025`).
//!
//! Solo is V1's: a track soloed anywhere silences every unsoloed track's notes before they
//! are lowered, and an instrument soloed anywhere mutes every unsoloed instrument's channel.
//! The lowering specification's open question — a solo *elsewhere*, unseeable from one
//! instrument — closes here, because the whole project is the input.
//!
//! # What the render is not
//!
//! Not faithful, and it says so: what remains named is Phase 8's — the amplifier's pan stage
//! and the terminating node's stages — so the outcome's [`Fidelity`] is
//! [`Fidelity::UnsupportedScope`] and a parity comparison is refused. The audio is evidence
//! that the lowering, admission, scheduling and rendering path connects end to end — not
//! evidence that it matches V1.
//!
//! V1 remains the default renderer for the GUI, MCP, CLI and releases. Nothing here is
//! reachable without the non-default `v2-lowering` feature ADR-0056 selects.

use synth_core::ModuleType;
use synth_engine::instrument::InstrumentId;
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::NodeId;
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::EventCount;
use synth_engine_v2::time::{FrameCount, PlanPosition};

use synth_engine_v2::ir::{ExecutionScope, IrNodeKind, PortId, SignalDomain};
use synth_engine_v2::quantities::Amplitude;

use super::diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};
use super::graph::{
    ChannelStrip, GraphAccumulator, InstrumentStages, Sink, lower_instrument_into,
    plan_declarations, voice_tuning,
};
use super::identity::{MASTER_CLAMP, MASTER_MIX, MASTER_OUTPUT, MASTER_TRIM};
use super::performance::{
    AutomationTargets, InstrumentPerformance, lower_project_performance, peak_concurrency,
    playing_tracks, project_peak,
};
use crate::patch::InstrumentState;

/// What the project as a whole asks for that V2 cannot do.
///
/// Read from `global` and the song's buses rather than from the instrument, and that is the
/// point: an earlier revision looked only at the instrument's voice patch, so a project with a
/// reverb on a return bus and a compressor on the master lowered as if neither existed. A
/// survey of every saved project in the repository found it — `sends-returns-master` counted
/// as eligible for a subset that cannot render either stage.
fn project_diagnostics(
    instruments: &[InstrumentState],
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
) -> Vec<LoweringDiagnostic> {
    let mut diagnostics = Vec::new();

    // V1 sums its instruments into the master in the order the project lists them; V2 sums
    // the master's cables in ascending identity (`SOUND-INV-008`), which is the instruments'
    // identity order. A float sum of three or more terms depends on its order, so a project
    // whose list is not in identity order is a marked difference, not a translation. Two
    // terms sum the same either way.
    let in_identity_order = instruments
        .windows(2)
        .all(|pair| pair[0].id.as_u64() < pair[1].id.as_u64());
    if instruments.len() >= 3 && !in_identity_order {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "three or more instruments saved out of identity order, which V1 \
                             sums in list order and V2 in identity order",
                owner: "the first A/B consumer, under the corpus's intentional-correction class",
            },
        ));
    }

    // Every saved project-global field, dispositioned once, by the same mechanism as
    // `instrument_state_dispositions`: destructured **without `..`**, so a field added to
    // `GlobalProjectState` is a compile error here rather than a silent difference. The type
    // is this crate's own, so nothing stops the destructure; an earlier revision read it field
    // by field and recorded the gap as an open question, which an independent review read as
    // the invariant promising a mechanism it did not have.
    let crate::project::GlobalProjectState {
        // Lowered onto the master trim by `master_trim` (`P08-S002`), within V1's own bound.
        master_volume: _,
        // The live keyboard's octave. It shifts what a played key sounds as, and nothing in
        // either engine's arrangement playback reads it: `audio::preview` reads the
        // *instrument's* own `octave_offset`, which the instrument destructure dispositions.
        octave_offset: _,
        // Reported below: an expression stage V2 does not apply.
        glide_time,
        // Refused below, effect by effect: a second signal path and a stage on everything.
        return_bus_effects,
        master_effects,
    } = global;

    // A master chain is audible processing on everything. V2 has no master bus at all.
    for (position, effect) in master_effects.iter().enumerate() {
        let _ = position;
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::MasterChain,
            LoweringReason::UnsupportedModuleType {
                module_type: effect.module_type,
            },
        ));
    }

    // A return bus is a second signal path. V2 renders one graph into one output.
    for bus in return_bus_effects {
        for effect in &bus.effects {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::ReturnBus {
                    bus: synth_sequencer::ReturnBusId::new(bus.id),
                },
                LoweringReason::UnsupportedModuleType {
                    module_type: effect.module_type,
                },
            ));
        }
    }

    // A send routes a track's audio to one of those buses, so it is the same absence seen
    // from the track's side. Reported separately because it is the object the user drew.
    for track in song.tracks() {
        // A send that contributes nothing is not routing V2 has to refuse. Two ways to
        // contribute nothing, and both are documented V1 behaviour: a **disabled** send is a
        // non-destructive bypass that keeps its level and tap point, and a send at **zero
        // level** is multiplied by that zero. Refusing either would reject a dry project for
        // settings that do not sound, while an acoustically identical one passed.
        let sends_audio = track
            .sends
            .iter()
            .any(|send| send.enabled && send.level != synth_core::NormalizedValue::MIN);
        if sends_audio {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Track {
                    track: track.id,
                    name: track.name.clone(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "a send into a return bus",
                    owner: "Phase 8, with the mixer and bus model",
                },
            ));
        }
    }

    // A Mod Grid graph is a control-rate modulator V1's offline renderer installs before the
    // engine applies it to track and instrument controls. Since `P07-S003` it is lowered in
    // `modulation::lower_mod_grid`, from the instrument's side: the instances V1's own builder
    // returns become global-scope modulator nodes and edges into this instrument's modules,
    // and the shapes V2 does not carry are refused there by name. It is asked after the
    // instrument's dispositions, in `smoke_render`, because its routes name the instrument.

    // A placed pattern's automation is lowered since `P07-S002b`, in `performance`: each
    // lane V1 runs is classified there over every placement, before any note filtering —
    // V1 executes a pattern's automation whether or not that track's notes are audible, and
    // a lane names its own instrument — and the classes V2 does not carry are refused there
    // by name.

    // The global glide is a stage V2 does not apply. It changes what V1 renders without
    // stopping the lowering, so it is reported rather than refused.
    if *glide_time != synth_core::Seconds::ZERO {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "a global glide time",
                owner: "Phase 6, with the expression model",
            },
        ));
    }
    diagnostics
}

/// Whether lowering may continue after a disposition pass.
enum Continue {
    Yes,
    No,
}

/// Every saved instrument field, with its disposition stated exactly once.
///
/// # Why this is a destructuring rather than a list of `if`s
///
/// The pattern below names **every** field and uses no `..`, so adding a field to
/// `InstrumentState` is a compile error here rather than a silent difference in the render.
/// That mechanism is the point. Five independent reviews of this phase each found another saved
/// field the lowerer never read — the project's global state, then the track's fader and pan,
/// then the instrument's key range and transpose, then its oversampling and unison — and a
/// prose claim that "every asymmetry is represented or refused" could not have stopped the
/// sixth. A field's disposition is now a thing the compiler asks for.
///
/// # How each disposition was chosen
///
/// `tests/offline_instrument_settings.rs` **measures** which of these fields reach V1's offline
/// renderer at all, one field per test, by rendering a project twice and comparing the bytes.
/// The dispositions below cite that evidence rather than a reading of the engine: a field that
/// changes V1's audio is refused or reported here, and a field measured inert is not.
fn instrument_state_dispositions(
    saved: &crate::patch::InstrumentState,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Continue {
    let crate::patch::InstrumentState {
        // Represented: the identity every diagnostic subject and the lowered plan are built on.
        id,
        name,
        // Not read by the offline arrangement path: a track names its instrument by id, and
        // `arrangement_render` never consults the channel. It is MIDI routing for live input.
        channel: _,
        // The instrument's channel stage, lowered onto a mix channel by `channel_strip`
        // (`P08-S001`): the fader within V1's own mixer range, the pan under V1's law and the
        // mute, so a silenced project renders the silence V1 renders rather than being
        // refused.
        volume: _,
        pan: _,
        muted: _,
        // One instrument is this input's whole world, so a solo **elsewhere** cannot be seen
        // from here — recorded as an input-shape limit in `spec-project-lowering-and-fidelity`
        // rather than pretended away. A solo on *this* instrument silences nothing of its own.
        solo: _,
        // Note input, applied by `Instrument::note_on_expr` before a voice exists: the range
        // suppresses notes outside it and the transpose moves every note, dropping one it takes
        // off the keyboard. Both change *which notes sound*, so both are refused.
        key_range,
        transpose,
        // Changes the anti-aliasing of everything the voice does, measured by
        // `oversampling_reaches_the_offline_renderer`.
        oversampling,
        // Metadata. Never reaches audio in either engine.
        category: _,
        description: _,
        color: _,
        // Voice allocation, which Phase 6 owns. Measured to reach the offline renderer, and
        // reachable here even without overlapping gates: a release still ringing under the next
        // note needs a second V1 voice, where V2 retriggers its one gate. Unison lives under
        // `allocation_mode`, so refusing a non-default mode is what makes the detune and spread
        // beside it unreachable — they are refused with it rather than separately.
        allocation_mode,
        stealing_strategy,
        unison_detune,
        unison_spread,
        max_voices,
        // Carried by `P04-R001`'s composition marker, which names both sensitivities and the
        // composition V2 does not have. Raised where the notes are, not here.
        // ADR-0059: V1's voice-output velocity stage, lowered to a velocity scaler where the
        // patch is lowered below, read from the saved instrument there. Its sibling is set on
        // the voice and read by nothing in V1's DSP — V1's own dead field — so it lowers to
        // nothing and is not a fidelity mark.
        velocity_amp_sensitivity: _,
        // Measured **inert in V1** by `velocity_filter_sensitivity_is_inert_in_v1`, which is a
        // characterization test that fails the day someone implements it.
        velocity_filter_sensitivity: _,
        // Ducking driven by another instrument, which needs the mixer Phase 8 owns.
        sidechain_source_id,
        // Represented: this is what the voice graph is lowered from.
        patch: _,
    } = saved;

    let subject = || ProjectSubject::Instrument {
        instrument: *id,
        name: name.clone(),
    };

    // Read through V1's own boundary rather than compared as a tuple. `project_apply` builds
    // the range with `KeyRange::new(MidiNote::new(lo), MidiNote::new(hi))`, which **swaps**
    // reversed endpoints and clamps a value above 127, so a saved `(127, 0)` and a saved
    // `(0, 255)` are both the full keyboard to V1. Comparing the serialized tuple would refuse
    // two projects V1 treats as neutral — an independent review found that.
    let reconstructed = synth_engine::instrument::KeyRange::new(
        synth_core::MidiNote::new(key_range.0),
        synth_core::MidiNote::new(key_range.1),
    );
    if reconstructed != synth_engine::instrument::KeyRange::default() {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "an instrument key range, which V1 uses to suppress notes outside \
                             it before a voice is allocated",
                owner: "Phase 6, with the instrument runtime",
            },
        ));
        return Continue::No;
    }
    // Decided from V1's **effective** value, not the stored one. `MidiNote::transpose` uses
    // `semitones.as_f32().round()`, so a saved `0.4` moves no note and is acoustically neutral;
    // refusing it would reject a project V1 plays exactly as an untransposed one. A non-finite
    // or out-of-range value does not round to zero and is still refused.
    if transpose.as_f32().round() != 0.0 {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "an instrument transpose, which V1 applies to every note and which \
                             drops one it moves off the keyboard",
                owner: "Phase 6, with the instrument runtime",
            },
        ));
        return Continue::No;
    }

    // The allocator settings travel together and are refused together, because unison is a
    // mode rather than a field: the detune and the spread mean nothing outside it.
    // Read from V1's own declaration of its defaults rather than transcribed here, exactly as
    // the module's clamps are: if `default_instrument_state` changes, this moves with it.
    let defaults = crate::project::default_instrument_state();
    if *allocation_mode != defaults.allocation_mode
        || *stealing_strategy != defaults.stealing_strategy
        || *max_voices != defaults.max_voices
    {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a voice-allocation setting — the mode, the stealing strategy or \
                             the voice count — which decides what V1 does when a release still \
                             rings under the next note",
                owner: "Phase 6, with the voice allocator",
            },
        ));
        return Continue::No;
    }
    // Unreachable while the mode is `Polyphonic`, and asserted rather than assumed: if the
    // refusal above ever narrows, this stops the pair from becoming silent again.
    debug_assert!(
        *allocation_mode == defaults.allocation_mode,
        "unison detune {unison_detune:?} and spread {unison_spread:?} are only reachable \
         under a non-default allocation mode, which is refused above"
    );

    if sidechain_source_id.is_some() {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a sidechain source, which ducks this instrument on what another \
                             one plays",
                owner: "Phase 8, with the mixer model",
            },
        ));
        return Continue::No;
    }

    // Reported rather than refused: the notes are unchanged and only their timbre is.
    // Mapped through V1's own `1 | 2 | 4` reading, where every other value is `X1`, so a saved
    // `3` is neutral to V1 and must be neutral here.
    // V1's own decoder, called rather than copied: `2` and `4` are factors and every other
    // value — `0` and `3` included — is `X1`, so a saved `3` is neutral to V1 and must be
    // neutral here. A second `match` here would compile happily after V1's changed, which an
    // independent review pointed out about exactly this line.
    if crate::project_apply::saved_oversampling_factor(*oversampling)
        != synth_dsp::OversamplingFactor::X1
    {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "instrument oversampling, which changes the anti-aliasing of \
                             everything the voice does",
                owner: "Phase 5, with the node and parameter model",
            },
        ));
    }

    Continue::Yes
}

/// The instrument's fader, pan and mute as the mix channel's authored bases (`P08-S001`),
/// or `None` with the refusal recorded.
///
/// The fader is held to **V1's own bound**, `Gain::MIXER_RANGE`, which is the range V1's
/// channel stage clamps the instrument volume to per block; a saved volume outside it is
/// refused by name and by value rather than clamped, since clamping persisted input is the
/// reinterpretation `AGENTS.md` forbids. The pan is a `BipolarValue` and so within range by
/// its type; only a value that is not a number can fail, and it is refused the same way.
///
/// `soloed_out` is V1's instrument solo seen from the whole project (`P08-S002`): when any
/// instrument is soloed, every unsoloed one is skipped by V1's mix stage — `mix_channel_busses`
/// reads `any_soloed && !instrument.is_solo()` as inaudible — so its channel starts muted
/// here, and the silence V1 renders for it is the silence rendered.
fn channel_strip(
    saved: &InstrumentState,
    soloed_out: bool,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<ChannelStrip> {
    let subject = || ProjectSubject::Instrument {
        instrument: saved.id,
        name: saved.name.clone(),
    };
    let range = synth_core::Gain::MIXER_RANGE;
    let volume = saved.volume.as_f32();
    if !volume.is_finite() || volume < range.min || volume > range.max {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::UnsupportedParameterValue {
                value: format!(
                    "an instrument volume of {volume} is outside V1's mixer range \
                     {}..={}",
                    range.min, range.max
                ),
            },
        ));
        return None;
    }
    let fader = match Amplitude::new(volume) {
        Ok(fader) => fader,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return None;
        }
    };
    let pan = match synth_engine_v2::controller::BipolarLevel::new(saved.pan.as_f32()) {
        Ok(pan) => pan,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return None;
        }
    };
    Some(ChannelStrip {
        fader,
        pan,
        muted: saved.muted || soloed_out,
    })
}

/// The project's master volume as the master trim's authored level (`P08-S002`), or `None`
/// with the refusal recorded.
///
/// Held to V1's own bound: V1 clamps the master volume it applies into `0..=2` at every
/// stage that reads it — `handle_set_master_volume`, the mix-stage read and the automation
/// lane alike. A saved value outside that range is refused by name and by value rather than
/// clamped, as the instrument's fader is.
fn master_trim(
    global: &crate::project::GlobalProjectState,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Amplitude> {
    let volume = global.master_volume.as_f32();
    if !volume.is_finite() || !(0.0..=2.0).contains(&volume) {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Project,
            LoweringReason::UnsupportedParameterValue {
                value: format!("a master volume of {volume} is outside V1's range 0..=2"),
            },
        ));
        return None;
    }
    match Amplitude::new(volume) {
        Ok(level) => Some(level),
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            None
        }
    }
}

/// What the lowered plan does where V1 saturates (`P08-S002`).
///
/// V1 has two saturation stages a lowered project meets: each channel's post-fader signal
/// is soft-clipped as it is summed into the master (`mix_stereo_faded`), and the output is
/// hard-clamped to full scale after the master volume. Both are explicit nodes in the plan
/// rather than hidden mixing behaviour, and both are selected together: the parity policy
/// places them where V1 has them, and the headroom policy places neither, so a float
/// render preserves everything above full scale, which the master plan asks of offline
/// output unless the caller asks for clipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum OutputPolicy {
    /// V1's stages, for parity: a soft clipper after every channel and a hard clamp before
    /// the output.
    Parity,
    /// Neither stage: linear summation with float headroom preserved offline.
    Headroom,
}
/// The longest render this bounded scope admits, in seconds.
///
/// The master plan's initial Phase 4 scope is "one bounded in-process smoke render", and a
/// bound that exists only in the prose is not one: `render_offline` allocates the entire
/// interleaved buffer up front, so a malformed or simply long project would allocate until it
/// aborted. Ten minutes is generous for a smoke render and small enough to fail as a
/// diagnostic rather than as an out-of-memory kill. The unbounded case belongs to ADR-0028's
/// streaming contract, which is where a render stops needing to fit in memory at all.
const MAX_SMOKE_SECONDS: f64 = 600.0;

/// What one smoke render produced.
#[derive(Debug)]
#[must_use]
///
/// `#[non_exhaustive]` because this reports what one bounded render produced and that list
/// grows as the phase does. The type was introduced on this branch and has never been
/// released, so adding the fields below breaks nothing that exists; the attribute is what
/// keeps the next addition from being a break either.
#[non_exhaustive]
pub struct SmokeRender {
    /// Interleaved samples, or empty when the render was refused.
    pub samples: Vec<f32>,
    /// Everything the lowering had to say about the project.
    pub diagnostics: Vec<LoweringDiagnostic>,
    /// How many events the lowering produced: note edges, and since `P07-S002b` the
    /// override writes its automation lanes emit with the restoring writes at the song's end.
    ///
    /// `EventCount` rather than a `usize`, because it is a count of the same events admission
    /// partitions its capacity across, and a frame count or a sample count is the same shape.
    ///
    /// Reported separately from `samples` because the two answer different questions: a
    /// refused lowering produces an empty buffer beside whatever count it reached, and a test
    /// can observe which notes an arrangement contributes without reading its audio.
    pub lowered_events: EventCount,
    /// The frames the arrangement occupies, from its own tempo map.
    ///
    /// Independent of the render, so a tempo change is observable here even when the render
    /// was refused for an unrelated reason.
    pub lowered_frames: FrameCount,
}

impl SmokeRender {
    /// Whether a parity comparison may read this.
    ///
    /// Always [`Fidelity::UnsupportedScope`] for an arrangement that places a note: V2 applies
    /// velocity as one scale where V1 composes two sensitivities, and `lower_performance`
    /// raises that once per lowering. The method exists so the answer is read rather than
    /// assumed.
    pub fn fidelity(&self) -> Fidelity {
        Fidelity::of(&self.diagnostics)
    }

    /// Whether the render is finite and any sample is non-zero.
    ///
    /// The weakest useful thing to assert about a render, and the right one here: the claim
    /// is that the path connects, not that it matches V1.
    ///
    /// Finiteness is part of it rather than a separate check, because `NaN != 0.0` and so
    /// does an infinity: a DSP regression producing non-finite output would otherwise satisfy
    /// "audible" and pass the end-to-end test it exists to guard.
    #[must_use]
    pub fn is_audible(&self) -> bool {
        self.samples.iter().all(|s| s.is_finite()) && self.samples.iter().any(|s| *s != 0.0)
    }
}

/// Lower one saved instrument and its song, and render it under the parity policy.
///
/// The one-instrument form of [`smoke_render_project`]: the project is this instrument
/// alone, so a solo elsewhere cannot arise.
pub fn smoke_render(
    saved: &InstrumentState,
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
    profile: HostProfile,
    tail: FrameCount,
) -> SmokeRender {
    smoke_render_project(
        std::slice::from_ref(saved),
        song,
        global,
        profile,
        tail,
        OutputPolicy::Parity,
    )
}

/// A refused render: no samples beside the diagnostics that say why.
fn refused(diagnostics: Vec<LoweringDiagnostic>) -> SmokeRender {
    SmokeRender {
        samples: Vec::new(),
        diagnostics,
        lowered_events: EventCount::NONE,
        lowered_frames: FrameCount::new(0),
    }
}

/// Lower every saved instrument and the song into one plan, and render it (`P08-S002`).
///
/// `tail` is added to the arrangement's own length so a final release is heard rather than
/// cut at the last note-off. It is the caller's, because how long a release lasts is a
/// property of the patch rather than of this path. `policy` selects V1's saturation stages
/// or declines them.
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the project, stage by stage"
)]
pub fn smoke_render_project(
    instruments: &[InstrumentState],
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
    profile: HostProfile,
    tail: FrameCount,
    policy: OutputPolicy,
) -> SmokeRender {
    let sample_rate = profile.capabilities().sample_rate();
    let mut diagnostics = project_diagnostics(instruments, song, global);
    let stop = |diagnostics: &[LoweringDiagnostic]| {
        diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused)
    };
    if stop(&diagnostics) {
        return refused(diagnostics);
    }
    let Some(master_level) = master_trim(global, &mut diagnostics) else {
        return refused(diagnostics);
    };

    // Every instrument's dispositions first, so a project is refused with every
    // instrument's reasons rather than the first's alone.
    // V1's instrument solo, across the project: `any(is_solo)` over every instrument the
    // engine holds, playing or not.
    let any_soloed = instruments.iter().any(|saved| saved.solo);
    struct Prepared<'a> {
        saved: &'a InstrumentState,
        strip: ChannelStrip,
        amp_sensitivity: synth_engine_v2::quantities::NormalizedLevel,
        playing: Option<super::performance::PlayingTracks>,
        modulators: super::modulation::SongModulators,
    }
    let mut prepared: Vec<Prepared<'_>> = Vec::with_capacity(instruments.len());
    for saved in instruments {
        if let Continue::No = instrument_state_dispositions(saved, &mut diagnostics) {
            continue;
        }
        let subject = || ProjectSubject::Instrument {
            instrument: saved.id,
            name: saved.name.clone(),
        };
        let amp_sensitivity = match synth_engine_v2::quantities::NormalizedLevel::new(
            saved.velocity_amp_sensitivity.as_f32(),
        ) {
            Ok(level) => level,
            Err(error) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: error.to_string(),
                    },
                ));
                continue;
            }
        };
        // What the song's Mod Grid adds to this instrument's graph (`P07-S003`). Asked of
        // V1's own builder, so a graph with no routing sink or a track-scoped graph assigned
        // to no track — for which `build_instance` returns `None`, exactly as `audio::export`
        // and `audio::arrangement_render` see it — lowers to nothing. An earlier revision
        // refused on the pool being non-empty, so a freshly created, still-empty graph
        // blocked every render of a project V1 plays unchanged; an independent review found
        // it.
        let modulators = super::modulation::lower_mod_grid(song, saved.id, &saved.patch.modules);
        diagnostics.extend(modulators.diagnostics.iter().cloned());
        let Some(strip) = channel_strip(saved, any_soloed && !saved.solo, &mut diagnostics) else {
            continue;
        };
        // The track or tracks that play it, whose control its balance stage carries.
        let Ok(playing) = playing_tracks(saved.id, song, &mut diagnostics) else {
            continue;
        };
        prepared.push(Prepared {
            saved,
            strip,
            amp_sensitivity,
            playing,
            modulators,
        });
    }
    if stop(&diagnostics) || prepared.iter().any(|p| p.modulators.refused) {
        return refused(diagnostics);
    }

    // Admission needs the arrangement's event peak before the plan exists, so it is counted
    // from the timeline. `None` means the arrangement could not be read; the lowering below
    // then produces the refusal with its subject intact, and a declared peak of zero is
    // correct for a plan that will carry no events.
    let counted: Vec<(
        InstrumentId,
        &[crate::patch::ModuleState],
        &[synth_sequencer::TrackId],
        bool,
    )> = prepared
        .iter()
        .map(|p| {
            (
                p.saved.id,
                p.saved.patch.modules.as_slice(),
                p.playing
                    .as_ref()
                    .map_or(&[][..], |playing| playing.tracks.as_slice()),
                true,
            )
        })
        .collect();
    let peak = project_peak(&counted, true, song, sample_rate).unwrap_or(EventCount::NONE);
    let ids: Vec<InstrumentId> = prepared.iter().map(|p| p.saved.id).collect();
    let notes = peak_concurrency(&ids, song, sample_rate);

    let mut graph = GraphAccumulator::default();
    struct Lowered<'a> {
        saved: &'a InstrumentState,
        identities: super::identity::ResolvedIdentities,
        targets: AutomationTargets,
        playing: Vec<synth_sequencer::TrackId>,
    }
    let mut lowered: Vec<Lowered<'_>> = Vec::with_capacity(prepared.len());
    let mut refused_any = false;
    for p in &prepared {
        let stages = InstrumentStages {
            velocity: Some(p.amp_sensitivity),
            track: p.playing.as_ref().map(|playing| playing.stage),
            channel: Some(p.strip),
            soft_clip: policy == OutputPolicy::Parity,
        };
        let outcome = lower_instrument_into(
            &mut graph,
            p.saved.id,
            &p.saved.patch.modules,
            &p.saved.patch.connections,
            stages,
            &p.modulators,
            Sink::Node(MASTER_MIX),
        );
        diagnostics.extend(outcome.diagnostics);
        if outcome.refused {
            refused_any = true;
            continue;
        }
        let slot = outcome.identities.slot();
        let mut targets =
            AutomationTargets::resolve(&outcome.identities).with_channel(slot.channel());
        if let Some(playing) = &p.playing {
            targets = targets.with_balance(slot.balance(), playing.tracks.clone());
        }
        lowered.push(Lowered {
            saved: p.saved,
            identities: outcome.identities,
            targets,
            playing: p
                .playing
                .as_ref()
                .map(|playing| playing.tracks.clone())
                .unwrap_or_default(),
        });
    }
    if refused_any {
        return refused(diagnostics);
    }

    // The master, in V1's order: every channel into one sum, the master volume, V1's output
    // clamp under the parity policy, and the plan's one output.
    let mut master_ir = master_nodes(policy, master_level);
    master_ir.push((MASTER_OUTPUT, IrNodeKind::Output));
    let mut previous: Option<synth_engine_v2::ir::NodeId> = None;
    let mut builder_error = None;
    for (id, kind) in master_ir {
        if let Err(taken) = graph.node(id, kind, ExecutionScope::Global) {
            builder_error = Some(format!("{taken} is claimed by two different nodes"));
        }
        if let Some(from) = previous {
            graph.connect(
                (from, PortId::FIRST),
                (id, PortId::FIRST),
                SignalDomain::Audio,
            );
        }
        previous = Some(id);
    }
    if let Some(error) = builder_error {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Project,
            LoweringReason::UnresolvedEndpoint { spelling: error },
        ));
        return refused(diagnostics);
    }

    let tuning = match voice_tuning() {
        Ok(tuning) => tuning,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return refused(diagnostics);
        }
    };
    let ir = match graph.build(tuning, plan_declarations(notes, peak)) {
        Ok(ir) => ir,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnresolvedEndpoint {
                    spelling: error.to_string(),
                },
            ));
            return refused(diagnostics);
        }
    };

    // The node a note plays is the one whose kind declares a note control, and in this subset
    // that is the envelope. More than one is ambiguous: nothing in the project says which
    // note-on reaches which, and choosing would be inventing a rule Phase 6 owns.
    let mut gates: Vec<(InstrumentId, NodeId)> = Vec::with_capacity(lowered.len());
    for l in &lowered {
        let envelopes: Vec<NodeId> = l
            .identities
            .pairs()
            .filter(|(id, _)| id.module_type == ModuleType::Envelope)
            .map(|(_, node)| node)
            .collect();
        let subject = ProjectSubject::Instrument {
            instrument: l.saved.id,
            name: l.saved.name.clone(),
        };
        match envelopes.as_slice() {
            [one] => gates.push((l.saved.id, *one)),
            [] => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject,
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a voice patch with no envelope, so no node a note can play",
                        owner: "Phase 6",
                    },
                ));
                return refused(diagnostics);
            }
            _ => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject,
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a voice patch with more than one envelope, where nothing \
                                     says which one a note plays",
                        owner: "Phase 6, with the voice-instantiation model",
                    },
                ));
                return refused(diagnostics);
            }
        }
    }

    let outcome = compile(&ir, &RenderConfig::new(profile));
    let plan = match outcome.into_plan() {
        Ok(plan) => plan,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return refused(diagnostics);
        }
    };

    let performers: Vec<InstrumentPerformance<'_>> = lowered
        .iter()
        .zip(&gates)
        .map(|(l, (_, gate))| InstrumentPerformance {
            id: l.saved.id,
            name: &l.saved.name,
            gate: *gate,
            targets: &l.targets,
            playing: &l.playing,
        })
        .collect();
    let performance =
        lower_project_performance(song, &plan, &performers, Some(MASTER_TRIM), sample_rate);
    // A refusal and a genuinely note-free arrangement both leave the event list empty, and
    // only the second may render. Reading the list alone let a refused arrangement — an
    // overlap, an expression, an unrepresentable position — fall through and return a
    // tail-sized buffer beside its own `Refused` diagnostic. An independent review found it.
    let performance_refused = performance.refused();
    diagnostics.extend(performance.diagnostics);

    let lowered_events =
        EventCount::measured(u32::try_from(performance.events.len()).unwrap_or(u32::MAX));
    let lowered_frames = performance.frames;
    if performance_refused {
        return SmokeRender {
            samples: Vec::new(),
            diagnostics,
            lowered_events,
            lowered_frames,
        };
    }
    let requested = performance.frames.as_u64().saturating_add(tail.as_u64());
    let ceiling = MAX_SMOKE_SECONDS * f64::from(sample_rate.as_f32());
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ceiling = ceiling as u64;
    if requested > ceiling {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "a render longer than the bounded smoke scope admits",
                owner: "ADR-0028, with the long-running job contract",
            },
        ));
        return SmokeRender {
            samples: Vec::new(),
            diagnostics,
            lowered_events,
            lowered_frames,
        };
    }

    // `render_offline` allocates the whole interleaved buffer, so an arrangement that maps
    // to a huge but representable position — or a caller's huge tail — would allocate without
    // any ceiling. A bounded smoke render has to have a bound, and this is it.
    let frames = FrameCount::new(requested);
    match render_offline(plan, frames, PlanPosition::ZERO, &performance.events) {
        Ok(samples) => SmokeRender {
            samples,
            diagnostics,
            lowered_events,
            lowered_frames,
        },
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            SmokeRender {
                samples: Vec::new(),
                diagnostics,
                lowered_events,
                lowered_frames,
            }
        }
    }
}

/// The master's nodes in signal order, before the output: the sum, the trim, and V1's
/// clamp under the parity policy.
fn master_nodes(policy: OutputPolicy, level: Amplitude) -> Vec<(NodeId, IrNodeKind)> {
    let mut master = vec![
        (MASTER_MIX, IrNodeKind::Mix),
        (MASTER_TRIM, IrNodeKind::Trim { level }),
    ];
    if policy == OutputPolicy::Parity {
        master.push((MASTER_CLAMP, IrNodeKind::HardClamp));
    }
    master
}
