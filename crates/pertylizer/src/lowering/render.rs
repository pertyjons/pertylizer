//! One bounded in-process smoke render of a saved project through V2.
//!
//! This is the second half of the first Phase 4 slice: the lowerer can represent a bounded
//! subset, and this is what proves it by rendering. Deliberately narrow — one project, one
//! instrument, one in-memory buffer, no job contract, no streaming, no cancellation. ADR-0028
//! may remain `Deferred` for exactly this scope and must be `Accepted` before any of those.
//!
//! # What the render is not
//!
//! Not faithful, and it says so. Every note raises `P04-R001`, so the outcome's
//! [`Fidelity`] is [`Fidelity::UnsupportedScope`] and a parity comparison is refused. The
//! audio is evidence that the lowering, admission, scheduling and rendering path connects
//! end to end — not evidence that it matches V1.
//!
//! V1 remains the default renderer for the GUI, MCP, CLI and releases. Nothing here is
//! reachable without the non-default `v2-lowering` feature ADR-0056 selects.

use synth_core::ModuleType;
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::NodeId;
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::EventCount;
use synth_engine_v2::time::{FrameCount, PlanPosition};

use super::diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};
use super::graph::lower_voice_patch_with;
use super::performance::lower_performance;
use crate::patch::InstrumentState;

/// What the project as a whole asks for that V2 cannot do.
///
/// Read from `global` and the song's buses rather than from the instrument, and that is the
/// point: an earlier revision looked only at the instrument's voice patch, so a project with a
/// reverb on a return bus and a compressor on the master lowered as if neither existed. A
/// survey of every saved project in the repository found it — `sends-returns-master` counted
/// as eligible for a subset that cannot render either stage.
fn project_diagnostics(
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
) -> Vec<LoweringDiagnostic> {
    let mut diagnostics = Vec::new();

    // Every saved project-global field, dispositioned once, by the same mechanism as
    // `instrument_state_dispositions`: destructured **without `..`**, so a field added to
    // `GlobalProjectState` is a compile error here rather than a silent difference. The type
    // is this crate's own, so nothing stops the destructure; an earlier revision read it field
    // by field and recorded the gap as an open question, which an independent review read as
    // the invariant promising a mechanism it did not have.
    let crate::project::GlobalProjectState {
        // Reported below: a level stage V2 does not apply.
        master_volume,
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
    // engine applies it to track and instrument controls, so it changes pitch, level, pan or a
    // module parameter over time. V2 has no modulation at all. Refused here rather than in the
    // arrangement walk because a **global** graph runs whether or not any placement of this
    // instrument is audible.
    //
    // Whether a graph *runs* is asked of V1's own builder rather than of the pool. A graph
    // with no routing sink, or a track-scoped graph assigned to no track, builds no instance —
    // `build_instance` returns `None` for it — and `audio::export` and
    // `audio::arrangement_render` install exactly what this builder returns. An earlier
    // revision refused on the pool being non-empty, so a freshly created, still-empty graph
    // blocked every render of a project V1 plays unchanged; an independent review found it.
    // The builder allocates, which is fine off the audio thread, and it is the same function
    // rather than a second copy of its routing rules for the reason the oversampling decoder
    // is shared: a copy compiles happily after the original changes.
    if !crate::mod_grid_build::build_mod_grid_runtime(song)
        .instances
        .is_empty()
    {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "a Mod Grid graph, which modulates track and instrument controls \
                             while the song plays",
                owner: "Phase 7, with the unified modulation model",
            },
        ));
    }

    // Automation is checked over **every** placement, before any note filtering. V1 executes a
    // pattern's automation whether or not that track's notes are audible, and a lane can target
    // another track, an instrument, a module parameter or a global control — so a placement this
    // instrument's lowering skips can still change what V1 renders. An independent review found
    // this check sitting after the track filter, where those placements never reached it.
    //
    // A lane with no points is not automation: `AutomationLane::value_at` returns `None` for
    // it, so V1's sequencer emits nothing. Only a lane that holds a point is refused; a second
    // independent review found the check reading the lane list's length instead. A zero-length
    // pattern is never active either — `pattern_tick_at` resolves no tick inside it — so its
    // lanes are never read; the same review found that.
    for placement in song.arrangement() {
        if song.pattern(placement.pattern_id).is_some_and(|pattern| {
            // Both lengths: a zero-length pattern yields no pattern tick however long its
            // placement is, and a `length_override` of zero ends the placement where it
            // starts — `pattern_tick_at` resolves nothing in either. The squash review found
            // only the source pattern's length read here.
            pattern.length.0 > 0
                && placement.effective_length(pattern.length).0 > 0
                && pattern.automation.iter().any(|lane| !lane.is_empty())
        }) {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Pattern {
                    pattern: placement.pattern_id,
                    name: String::new(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "a placed pattern carrying automation, which V1 applies over \
                                 the track's own fader and over module parameters",
                    owner: "Phase 7, with the unified modulation model",
                },
            ));
            break;
        }
    }

    // The master volume and the global glide are stages V2 does not apply. They change what
    // V1 renders without stopping the lowering, so they are reported rather than refused.
    if *master_volume != synth_core::Gain::UNITY {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "a project master volume other than unity",
                owner: "Phase 8",
            },
        ));
    }
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
        // Mixer stages V2 has no place for. Reported rather than refused: the notes V1 plays
        // are still the notes lowered here, only their level and position differ.
        volume,
        pan,
        // Silent in V1. Rendering it audibly and calling that a smoke render would report sound
        // for a project the user silenced.
        muted,
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

    if *muted {
        diagnostics.push(LoweringDiagnostic::refused(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a muted instrument, which V1 renders silent",
                owner: "Phase 8, with the mixer model",
            },
        ));
        return Continue::No;
    }

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
    if *volume != synth_core::Gain::UNITY {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "an instrument volume other than unity",
                owner: "Phase 8",
            },
        ));
    }
    if *pan != synth_core::BipolarValue::CENTER {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "an instrument pan",
                owner: "Phase 8",
            },
        ));
    }

    Continue::Yes
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
    /// How many note edges the lowering produced.
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

/// Lower one saved instrument and its song, and render it.
///
/// `tail` is added to the arrangement's own length so a final release is heard rather than
/// cut at the last note-off. It is the caller's, because how long a release lasts is a
/// property of the patch rather than of this path.
pub fn smoke_render(
    saved: &InstrumentState,
    song: &synth_sequencer::Song,
    global: &crate::project::GlobalProjectState,
    profile: HostProfile,
    tail: FrameCount,
) -> SmokeRender {
    let sample_rate = profile.capabilities().sample_rate();
    let mut mixer_diagnostics = project_diagnostics(song, global);
    let project_refused = mixer_diagnostics
        .iter()
        .any(|d| d.severity() == Severity::Refused);
    if project_refused {
        return SmokeRender {
            samples: Vec::new(),
            diagnostics: mixer_diagnostics,
            lowered_events: EventCount::NONE,
            lowered_frames: FrameCount::new(0),
        };
    }

    if let Continue::No = instrument_state_dispositions(saved, &mut mixer_diagnostics) {
        return SmokeRender {
            samples: Vec::new(),
            diagnostics: mixer_diagnostics,
            lowered_events: EventCount::NONE,
            lowered_frames: FrameCount::new(0),
        };
    }

    // Admission needs the arrangement's event peak before the plan exists, so it is counted
    // from the timeline. `None` means the arrangement could not be read; the lowering below
    // then produces the refusal with its subject intact, and a declared peak of zero is
    // correct for a plan that will carry no events.
    let peak = super::performance::peak_events_per_quantum(saved.id, song, sample_rate)
        .unwrap_or(EventCount::NONE);

    let amp_sensitivity = match synth_engine_v2::quantities::NormalizedLevel::new(
        saved.velocity_amp_sensitivity.as_f32(),
    ) {
        Ok(level) => level,
        Err(error) => {
            let mut diagnostics = mixer_diagnostics;
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument: saved.id,
                    name: saved.name.clone(),
                },
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return SmokeRender {
                samples: Vec::new(),
                diagnostics,
                lowered_events: EventCount::NONE,
                lowered_frames: FrameCount::new(0),
            };
        }
    };
    let lowered = lower_voice_patch_with(
        saved.id,
        &saved.patch.modules,
        &saved.patch.connections,
        peak,
        Some(amp_sensitivity),
    );
    let mut diagnostics = mixer_diagnostics;
    diagnostics.extend(lowered.diagnostics);

    let Some(ir) = lowered.ir else {
        return SmokeRender {
            samples: Vec::new(),
            diagnostics,
            lowered_events: EventCount::NONE,
            lowered_frames: FrameCount::new(0),
        };
    };

    // The node a note plays is the one whose kind declares a note control, and in this subset
    // that is the envelope. More than one is ambiguous: nothing in the project says which
    // note-on reaches which, and choosing would be inventing a rule Phase 6 owns.
    let envelopes: Vec<NodeId> = lowered
        .identities
        .pairs()
        .filter(|(id, _)| id.module_type == ModuleType::Envelope)
        .map(|(_, node)| node)
        .collect();
    let gate = match envelopes.as_slice() {
        [one] => *one,
        [] => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument: saved.id,
                    name: saved.name.clone(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "a voice patch with no envelope, so no node a note can play",
                    owner: "Phase 6",
                },
            ));
            return SmokeRender {
                samples: Vec::new(),
                diagnostics,
                lowered_events: EventCount::NONE,
                lowered_frames: FrameCount::new(0),
            };
        }
        _ => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument: saved.id,
                    name: saved.name.clone(),
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "a voice patch with more than one envelope, where nothing says \
                                 which one a note plays",
                    owner: "Phase 6, with the voice-instantiation model",
                },
            ));
            return SmokeRender {
                samples: Vec::new(),
                diagnostics,
                lowered_events: EventCount::NONE,
                lowered_frames: FrameCount::new(0),
            };
        }
    };

    let outcome = compile(&ir, &RenderConfig::new(profile));
    let plan = match outcome.into_plan() {
        Ok(plan) => plan,
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Instrument {
                    instrument: saved.id,
                    name: saved.name.clone(),
                },
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return SmokeRender {
                samples: Vec::new(),
                diagnostics,
                lowered_events: EventCount::NONE,
                lowered_frames: FrameCount::new(0),
            };
        }
    };

    let performance = lower_performance(saved.id, &saved.name, song, &plan, gate, sample_rate);
    // A refusal and a genuinely note-free arrangement both leave the event list empty, and
    // only the second may render. Reading the list alone let a refused arrangement — an
    // overlap, an expression, an unrepresentable position — fall through and return a
    // tail-sized buffer beside its own `Refused` diagnostic. An independent review found it.
    let performance_refused = performance.refused();
    diagnostics.extend(performance.diagnostics);

    // **A saved note renders here now**, and `P04-R001`'s precondition is what released it.
    // The work list's rule is "before rendering the first saved pitched note, close P03-R003
    // with minimum typed pitch and velocity payload semantics", restated by `SOUND-INV-017`;
    // a lowered note carries the project's own key and velocity, so that residual is closed
    // and the refusal that stood here is gone.
    //
    // The **fidelity marker** is a separate question and is unchanged: it governs reporting,
    // and `lower_performance` still marks the outcome `UnsupportedScope` because V2 applies
    // velocity as one scale where V1 composes two sensitivities. The work list is explicit
    // that closing this residual "does not decide Phase 6's tuning or expression-composition
    // model", so the render is admissible and a parity claim over it is not.
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
            ProjectSubject::Instrument {
                instrument: saved.id,
                name: saved.name.clone(),
            },
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
                ProjectSubject::Instrument {
                    instrument: saved.id,
                    name: saved.name.clone(),
                },
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
