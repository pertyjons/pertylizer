//! Voice module - represents a single synthesizer voice.
//!
//! A voice contains a complete signal chain via a ModuleGraph and handles
//! note events for polyphonic playback.
//!
//! ## Architecture
//!
//! Each voice owns a `ModuleGraph` that defines its signal routing. This allows
//! the user to dynamically configure the signal chain through the UI, rather than
//! using a hardcoded DSP pipeline.
//!
//! ## Macro Controllers
//!
//! Each voice stores macro controller state (pitch bend, mod wheel) using type-safe
//! domain types to prevent unit mismatches.

use crate::ModuleId;
use crate::graph::ModuleGraph;
use synth_core::params::LfoWaveform;
use synth_core::tuning::TuningTable;
use synth_core::{AudioBuffer, Phase, PortName, ProcessContext};
use synth_core::{
    BipolarValue, Cents, Hertz, MidiNote, NormalizedValue, SampleCount, SamplePosition, Seconds,
    Semitones, Velocity,
};
use synth_core::{ModuleType, OscillatorParam, Param};

/// Unique identifier for a voice within an instrument's voice pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct VoiceId(pub(crate) u32);

impl VoiceId {
    /// Create a new voice ID.
    #[inline]
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

/// Per-voice values of the six true macro sources, computed once per block and
/// read when resolving a routing whose source is a [`MacroSource`].
struct MacroValues {
    velocity: f32,
    note: f32,
    aftertouch: f32,
    mod_wheel: f32,
    pitch_bend: f32,
    poly_aftertouch: f32,
}

/// Voice state with embedded data - "Make Invalid States Unrepresentable".
///
/// Each state variant carries only the data that is valid for that state:
/// - `Idle`: No data (a voice that isn't playing has no note/velocity)
/// - `Active`/`Releasing`: Has note, velocity, and start time
/// - `Stealing`: Has fade counter for smooth transition
///
/// This design ensures you can't accidentally read a "current note" from an idle voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceState {
    /// Voice is not playing - no note data available.
    Idle,

    /// Voice is actively playing a note.
    Active {
        /// MIDI note number
        note: MidiNote,
        /// Note velocity (type-safe normalized value 0.0-1.0)
        velocity: Velocity,
        /// When this voice was triggered (type-safe sample position)
        start_time: SamplePosition,
    },

    /// Voice is in release phase (note off received, envelope releasing).
    Releasing {
        /// MIDI note number (kept for potential re-trigger or display)
        note: MidiNote,
        /// Original velocity (kept for release velocity calculations)
        velocity: Velocity,
        /// Original start time (type-safe sample position)
        start_time: SamplePosition,
    },

    /// Voice was stolen and is fading out quickly.
    Stealing {
        /// Remaining fade-out samples
        fade_counter: SampleCount,
        /// Total fade samples (for calculating fade ratio)
        fade_total: SampleCount,
        /// Pending note to trigger after fade-out completes.
        pending_note: Option<(MidiNote, Velocity, SamplePosition)>,
    },
}

impl VoiceState {
    /// Check if voice is idle (not producing sound).
    #[inline]
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Check if voice is active (playing or releasing).
    #[inline]
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Check if voice is in stealing fade-out.
    #[inline]
    pub fn is_stealing(&self) -> bool {
        matches!(self, Self::Stealing { .. })
    }

    /// Get the note if the voice is playing (Active or Releasing).
    #[inline]
    pub fn note(&self) -> Option<MidiNote> {
        match self {
            Self::Active { note, .. } | Self::Releasing { note, .. } => Some(*note),
            _ => None,
        }
    }

    /// Get the velocity if the voice is playing.
    #[inline]
    pub fn velocity(&self) -> Option<Velocity> {
        match self {
            Self::Active { velocity, .. } | Self::Releasing { velocity, .. } => Some(*velocity),
            _ => None,
        }
    }

    /// Get the start time if the voice is playing.
    #[inline]
    pub fn start_time(&self) -> Option<SamplePosition> {
        match self {
            Self::Active { start_time, .. } | Self::Releasing { start_time, .. } => {
                Some(*start_time)
            }
            _ => None,
        }
    }
}

/// Glide (portamento) state for smooth pitch transitions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlideState {
    /// Source frequency.
    pub(crate) from_freq: Hertz,
    /// Target frequency.
    pub(crate) to_freq: Hertz,
    /// Current frequency (during glide).
    pub(crate) current_freq: Hertz,
    /// Glide time.
    pub(crate) time: Seconds,
    /// Current position in glide (0.0 = start, 1.0 = complete).
    pub(crate) position: NormalizedValue,
    /// Whether glide is active.
    pub(crate) active: bool,
    /// Stepped (glissando) interpolation: quantize to integer semitones.
    /// When `true` the trajectory holds at chromatic steps instead of sweeping
    /// continuously — the *intentional-holds* case (smoothing it would be the
    /// bug, the inverse of zipper noise).
    pub(crate) stepped: bool,
}

impl Default for GlideState {
    fn default() -> Self {
        Self {
            from_freq: Hertz::A4,
            to_freq: Hertz::A4,
            current_freq: Hertz::A4,
            time: Seconds::ZERO,
            position: NormalizedValue::MAX,
            active: false,
            stepped: false,
        }
    }
}

impl GlideState {
    /// Start a new glide from current frequency to target (continuous).
    pub(crate) fn start(&mut self, target_freq: Hertz, glide_time: Seconds) {
        self.start_from(self.current_freq, target_freq, glide_time, false);
    }

    /// Start a glide with an explicit source frequency.
    ///
    /// Unlike [`start`](Self::start) (which glides from the voice's current
    /// frequency), this seeds both endpoints — used by per-note glide where the
    /// source pitch is specified by the note, not by whatever was playing.
    pub(crate) fn start_from(
        &mut self,
        from_freq: Hertz,
        target_freq: Hertz,
        glide_time: Seconds,
        stepped: bool,
    ) {
        if glide_time.as_f32() > 0.0 && (from_freq.as_f32() - target_freq.as_f32()).abs() > 0.01 {
            self.from_freq = from_freq;
            self.to_freq = target_freq;
            self.current_freq = from_freq;
            self.time = glide_time;
            self.position = NormalizedValue::MIN;
            self.active = true;
            self.stepped = stepped;
        } else {
            // No glide - jump directly
            self.current_freq = target_freq;
            self.from_freq = target_freq;
            self.to_freq = target_freq;
            self.position = NormalizedValue::MAX;
            self.active = false;
            self.stepped = false;
        }
    }

    /// Update the glide state (call once per sample or per block).
    /// Returns the current frequency.
    pub(crate) fn update(&mut self, delta_time: Seconds) -> Hertz {
        if !self.active {
            return self.current_freq;
        }

        // Update position
        if self.time.as_f32() > 0.0 {
            self.position = NormalizedValue::new(
                self.position.as_f32() + delta_time.as_f32() / self.time.as_f32(),
            );
        } else {
            self.position = NormalizedValue::MAX;
        }

        if self.position.as_f32() >= 1.0 {
            self.position = NormalizedValue::MAX;
            self.active = false;
            self.current_freq = self.to_freq;
        } else {
            // Exponential interpolation (sounds more natural for pitch)
            // f(t) = from * (to/from)^t
            let ratio = self.to_freq.as_f32() / self.from_freq.as_f32().max(f32::EPSILON);
            let raw = self.from_freq.as_f32() * ratio.powf(self.position.as_f32());
            self.current_freq = if self.stepped {
                // Glissando: hold at integer semitones relative to the source so
                // the sweep lands on chromatic steps. Both endpoints are note
                // frequencies, so the steps coincide with real notes.
                let from = self.from_freq.as_f32().max(f32::EPSILON);
                let semis = (12.0 * (raw / from).log2()).round();
                Hertz::new(from * (semis / 12.0).exp2())
            } else {
                Hertz::new(raw)
            };
        }

        self.current_freq
    }

    /// Get current frequency without updating.
    #[must_use]
    pub(crate) fn get_frequency(&self) -> Hertz {
        self.current_freq
    }
}

/// Per-note glide specification, resolved to engine-native types.
///
/// `from_offset` is the source pitch expressed as a signed semitone offset from
/// the note's *own* target pitch — so it is key/transpose invariant and the
/// engine never needs the sequencer's `GlideFrom`. Built at the sequencer-event
/// consumer from `synth_sequencer::Glide`.
#[derive(Debug, Clone, Copy)]
pub struct GlideSpec {
    /// Source pitch as a signed semitone offset from the target note.
    pub from_offset: Semitones,
    /// Glide time.
    pub time: Seconds,
    /// Stepped (glissando) vs continuous (portamento).
    pub stepped: bool,
}

/// Evaluate a bipolar LFO value in `[-1, 1]` for the given waveform and phase,
/// reusing `synth_core::Phase`'s shape math (shared with the LFO module and the
/// mod matrix, so a per-note vibrato and a mod-matrix LFO of the same shape stay
/// in sync). `SampleAndHold`/`SmoothRandom` are unreachable from a per-note
/// `VibratoShape` and fall back to sine.
#[inline]
fn lfo_shape_value(shape: LfoWaveform, phase: Phase) -> f32 {
    match shape {
        LfoWaveform::Triangle => phase.triangle(),
        LfoWaveform::Sawtooth => phase.sawtooth(),
        LfoWaveform::Square => phase.pulse(NormalizedValue::CENTER),
        _ => phase.sin(),
    }
}

/// Per-note vibrato specification, resolved to engine-native types.
///
/// Drives an additive per-voice pitch LFO (taxonomy primitive 1). The depth is
/// applied as a semitone offset *on top of* the base pitch (the same additive
/// model as the mod-matrix `OscPitch` destination — never a destructive write),
/// so a mod-matrix vibrato and a per-note vibrato compose additively.
///
/// **Why a separate LFO, not the mod matrix:** the mod matrix is patch-scoped
/// (one shared LFO config for the instrument), whereas this is *per note* —
/// seeded, phase-reset and faded-in independently for each note, with its own
/// depth/rate/shape. Routing it through the mod matrix would require per-note
/// overrides of a per-patch destination (the deferred A2 "two controllers, one
/// param" problem). The shape math is shared via `synth_core::Phase`
/// (`lfo_shape_value`) so the two never visually/audibly diverge.
#[derive(Debug, Clone, Copy)]
pub struct VibratoSpec {
    /// Peak pitch deviation.
    pub depth: Semitones,
    /// LFO rate.
    pub rate: Hertz,
    /// Depth fades in 0→full linearly over this time from note onset (`0` =
    /// instant). Click-free onset.
    pub fade_in: Seconds,
    /// LFO shape (the shared `synth_core` waveform enum).
    pub shape: LfoWaveform,
}

/// Per-note expression carried into a voice trigger.
///
/// `Copy`/alloc-free so it threads through the audio-thread trigger path without
/// allocation. Default = no per-note expression (current behavior).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoteTrigger {
    /// Force legato (no envelope retrigger), overriding the allocation mode
    /// (taxonomy primitive 2).
    pub legato: bool,
    /// Optional per-note glide; `None` falls back to the instrument glide time
    /// (taxonomy primitive 2).
    pub glide: Option<GlideSpec>,
    /// Optional per-note vibrato (taxonomy primitive 1).
    pub vibrato: Option<VibratoSpec>,
}

/// Default pitch bend range in semitones (standard MIDI is ±2).
pub const DEFAULT_PITCH_BEND_RANGE: Semitones = Semitones(2.0);

/// Default maximum vibrato depth (2.5% = ~43 cents at full mod wheel).
pub const DEFAULT_VIBRATO_DEPTH: NormalizedValue = NormalizedValue(0.025);

/// Performance expression settings for a voice/instrument.
///
/// These control how MIDI controllers affect the sound.
#[derive(Debug, Clone, Copy)]
pub struct ExpressionSettings {
    /// Pitch bend range in semitones (typically 2-24).
    pub pitch_bend_range: Semitones,
    /// Maximum vibrato depth when mod wheel is at 100% (0.0-0.1 typical).
    pub vibrato_depth: NormalizedValue,
    /// How much velocity affects amplitude (0 = constant, 1 = full dynamic).
    pub velocity_to_amp: NormalizedValue,
    /// How much velocity affects filter cutoff (0 = none, 1 = full).
    pub velocity_to_filter: NormalizedValue,
}

impl Default for ExpressionSettings {
    fn default() -> Self {
        Self {
            pitch_bend_range: DEFAULT_PITCH_BEND_RANGE,
            vibrato_depth: DEFAULT_VIBRATO_DEPTH,
            velocity_to_amp: NormalizedValue::MAX, // Full velocity sensitivity
            velocity_to_filter: NormalizedValue::CENTER, // 50% velocity to filter
        }
    }
}

/// A single synthesizer voice.
///
/// The voice now uses a `ModuleGraph` for its signal chain, enabling dynamic
/// routing that can be configured by the user.
///
/// ## State Management
///
/// All note-specific data (note, velocity, start_time) is now stored inside
/// `VoiceState`. This ensures you cannot access "the current note" from an
/// idle voice - the type system prevents it.
pub struct Voice {
    /// Unique voice ID.
    pub id: VoiceId,

    /// Current state (carries note/velocity data when active).
    pub state: VoiceState,

    /// Age in samples since note-on (for voice stealing priority).
    /// Incremented each process() call when voice is active.
    pub age: SampleCount,

    // === Macro controllers (type-safe) ===
    /// Pitch bend amount (-1.0 to +1.0, type-safe).
    pub pitch_bend: BipolarValue,
    /// Mod wheel amount (0.0 to 1.0, type-safe).
    pub mod_wheel: NormalizedValue,
    /// Channel aftertouch amount (0.0 to 1.0, type-safe).
    pub aftertouch: NormalizedValue,
    /// Per-note polyphonic aftertouch (0.0 to 1.0, type-safe).
    pub poly_aftertouch: NormalizedValue,

    /// Expression settings (pitch bend range, velocity sensitivity, etc.).
    pub expression: ExpressionSettings,

    /// The module graph that defines this voice's signal chain.
    pub graph: ModuleGraph,

    /// Default steal fade duration in samples.
    steal_fade_samples: SampleCount,

    /// Glide state for portamento.
    pub(crate) glide: GlideState,
    /// Configured glide time.
    glide_time: Seconds,

    /// Per-note vibrato spec (`None` = no per-note vibrato).
    pub(crate) vibrato: Option<VibratoSpec>,
    /// Vibrato LFO phase.
    vibrato_phase: Phase,
    /// Time since the vibrato note's onset (drives the depth fade-in).
    vibrato_elapsed: Seconds,
    /// Current vibrato pitch offset, recomputed per block by `advance_vibrato`
    /// and read in `process_audio` (mirrors the glide-update pattern).
    pub(crate) vibrato_offset: Semitones,

    /// Cached output module ID for stereo output extraction.
    /// Priority: StereoOutput > Amplifier > Mixer
    output_module_id: Option<crate::ModuleId>,

    /// Cached mod matrix module ID (if present in graph).
    mod_matrix_id: Option<crate::ModuleId>,

    /// Pre-allocated buffer for mod matrix slot data (avoids per-frame Vec allocation).
    mod_slots_cache: Vec<(f32, synth_core::DestAddr, f32)>,

    /// Temporary mono buffer for graph processing.
    mono_buffer: AudioBuffer,

    /// Tuning table for note-to-frequency conversion.
    /// Defaults to 12-TET, can be changed for microtonal tuning.
    tuning_table: TuningTable,
}

impl Voice {
    /// Create a new voice with the given ID.
    pub fn new(id: VoiceId) -> Self {
        Self {
            id,
            state: VoiceState::Idle,
            age: SampleCount::ZERO,
            // Macro controllers default to neutral positions
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            poly_aftertouch: NormalizedValue::MIN,
            expression: ExpressionSettings::default(),
            graph: ModuleGraph::new(),
            steal_fade_samples: SampleCount::new(128),
            glide: GlideState::default(),
            glide_time: Seconds::ZERO,
            vibrato: None,
            vibrato_phase: Phase::ZERO,
            vibrato_elapsed: Seconds::ZERO,
            vibrato_offset: Semitones::ZERO,
            output_module_id: None,
            mod_matrix_id: None,
            mod_slots_cache: Vec::with_capacity(16),
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            tuning_table: TuningTable::default(),
        }
    }

    /// Create a new voice from a ModuleGraph template.
    pub fn from_graph(id: VoiceId, graph: ModuleGraph) -> Self {
        // Find output module with priority: StereoOutput > Amplifier > Mixer
        let output_id = graph
            .find_module_by_type(ModuleType::StereoOutput)
            .or_else(|| graph.find_module_by_type(ModuleType::Amplifier))
            .or_else(|| graph.find_module_by_type(ModuleType::Mixer));

        let mod_matrix_id = graph.find_module_by_type(ModuleType::ModMatrix);

        Self {
            id,
            state: VoiceState::Idle,
            age: SampleCount::ZERO,
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            poly_aftertouch: NormalizedValue::MIN,
            expression: ExpressionSettings::default(),
            graph,
            steal_fade_samples: SampleCount::new(128),
            glide: GlideState::default(),
            glide_time: Seconds::ZERO,
            vibrato: None,
            vibrato_phase: Phase::ZERO,
            vibrato_elapsed: Seconds::ZERO,
            vibrato_offset: Semitones::ZERO,
            output_module_id: output_id,
            mod_matrix_id,
            mod_slots_cache: Vec::with_capacity(16),
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            tuning_table: TuningTable::default(),
        }
    }

    /// Set parameter on a module in the graph.
    pub fn set_param(&mut self, module_id: crate::ModuleId, param: Param) {
        self.graph.set_param(module_id, param);
    }

    /// Apply a transient automation override to a module in this voice's graph.
    /// The base parameter is never mutated. Real-time safe.
    pub fn apply_param_override(&mut self, module_id: crate::ModuleId, param: Param) {
        self.graph.apply_param_override(module_id, param);
    }

    /// Clear all transient automation overrides in this voice's graph. Called
    /// on transport stop. Real-time safe.
    pub fn clear_param_overrides(&mut self) {
        self.graph.clear_param_overrides();
    }

    /// Get a parameter from a module in the graph.
    pub fn get_param(&self, module_id: crate::ModuleId, param: &Param) -> Option<f32> {
        self.graph.get_param(module_id, param)
    }

    /// Set the glide time (type-safe Seconds).
    pub fn set_glide_time(&mut self, time: Seconds) {
        self.glide_time = Seconds::new(time.as_f32().max(0.0));
    }

    /// Get the current glide time.
    pub fn get_glide_time(&self) -> Seconds {
        self.glide_time
    }

    /// Get the current frequency (accounting for glide).
    pub fn get_current_frequency(&self) -> Hertz {
        self.glide.get_frequency()
    }

    /// Set detune on all oscillators in the voice (type-safe Cents).
    /// Used for unison mode.
    pub fn set_oscillator_detune(&mut self, detune: Cents) {
        // Collect oscillator IDs into a small fixed-size buffer to avoid
        // heap allocation on the audio thread (can't iterate and mutate graph).
        let mut osc_ids: [Option<ModuleId>; 8] = [None; 8];
        let mut count = 0;
        for module_id in self.graph.module_ids() {
            if module_id.module_type == ModuleType::Oscillator && count < osc_ids.len() {
                osc_ids[count] = Some(module_id);
                count += 1;
            }
        }
        for module_id in osc_ids.into_iter().flatten() {
            self.graph.set_param(
                module_id,
                Param::Oscillator(OscillatorParam::Detune(detune)),
            );
        }
    }

    /// Convert MIDI note to frequency using the voice's tuning table.
    /// Falls back to the pre-computed 12-TET table if tuning is default.
    #[inline]
    fn note_to_freq(&self, note: MidiNote) -> f32 {
        self.tuning_table.note_to_freq(note).as_f32()
    }

    /// Set the tuning table for microtonal support.
    pub fn set_tuning_table(&mut self, table: TuningTable) {
        self.tuning_table = table;
    }

    /// Get the current tuning table.
    pub fn tuning_table(&self) -> &TuningTable {
        &self.tuning_table
    }

    /// Trigger note on with type-safe velocity.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity, time: SamplePosition) {
        self.note_on_expr(note, velocity, time, NoteTrigger::default());
    }

    /// Trigger note on, retriggering the envelope, with per-note expression.
    ///
    /// With `trigger.glide` the pitch ramps from the note-specified source; with
    /// no glide it falls back to the instrument glide time from the current
    /// frequency (identical to [`note_on`](Self::note_on)).
    pub fn note_on_expr(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        time: SamplePosition,
        trigger: NoteTrigger,
    ) {
        let target_freq = Hertz::new(self.note_to_freq(note));
        let was_active = matches!(self.state, VoiceState::Active { .. });
        self.seed_glide(target_freq, was_active, trigger.glide);
        self.seed_vibrato(trigger.vibrato);

        // Set state with embedded note data
        self.state = VoiceState::Active {
            note,
            velocity,
            start_time: time,
        };
        self.age = SampleCount::ZERO;

        // Notify all modules in the graph
        self.graph.note_on(note, velocity);
    }

    /// Change pitch without retriggering (for legato mode).
    ///
    /// Updates the note (and velocity) in the current state if active.
    pub fn glide_to_note(&mut self, new_note: MidiNote, velocity: Velocity) {
        self.glide_to_note_expr(new_note, velocity, NoteTrigger::default());
    }

    /// Change pitch without retriggering, honoring per-note glide.
    ///
    /// The new note's `velocity` is written into the active voice state so that
    /// per-note velocity shaping (accent/ghost, resolved sequencer-side) still
    /// affects amplitude on a legato glide — `process_audio` reads velocity from
    /// the state each block. The envelope is *not* retriggered.
    pub fn glide_to_note_expr(
        &mut self,
        new_note: MidiNote,
        velocity: Velocity,
        trigger: NoteTrigger,
    ) {
        let target_freq = Hertz::new(self.note_to_freq(new_note));
        // Re-seed vibrato for the new (legato) note so its per-note vibrato takes
        // effect even though the envelope isn't retriggered.
        self.seed_vibrato(trigger.vibrato);

        match trigger.glide {
            Some(g) => {
                let from_freq = g.from_offset.apply(target_freq);
                self.glide
                    .start_from(from_freq, target_freq, g.time, g.stepped);
            }
            None if self.glide_time.as_f32() > 0.0 => {
                self.glide.start(target_freq, self.glide_time);
            }
            None => {
                self.glide.current_freq = target_freq;
                self.glide.to_freq = target_freq;
                self.glide.active = false;
            }
        }

        // Update the note + velocity in the state (only if active).
        if let VoiceState::Active {
            note,
            velocity: vel,
            start_time: _,
        } = &mut self.state
        {
            *note = new_note;
            *vel = velocity;
        }
    }

    /// Seed the glide state for a (re)triggered note. With an explicit per-note
    /// glide the source is the note-relative offset; otherwise fall back to the
    /// instrument glide time from the current frequency.
    fn seed_glide(&mut self, target_freq: Hertz, was_active: bool, glide: Option<GlideSpec>) {
        match glide {
            Some(g) => {
                let from_freq = g.from_offset.apply(target_freq);
                self.glide
                    .start_from(from_freq, target_freq, g.time, g.stepped);
            }
            None if self.glide_time.as_f32() > 0.0 && was_active => {
                self.glide.start(target_freq, self.glide_time);
            }
            None => {
                // No glide - set frequency immediately
                self.glide.current_freq = target_freq;
                self.glide.from_freq = target_freq;
                self.glide.to_freq = target_freq;
                self.glide.active = false;
                self.glide.stepped = false;
            }
        }
    }

    /// Seed (or clear) the per-note vibrato for a newly triggered note. Resets
    /// the LFO phase and the fade-in clock so each note starts cleanly.
    fn seed_vibrato(&mut self, vibrato: Option<VibratoSpec>) {
        self.vibrato = vibrato;
        self.vibrato_phase = Phase::ZERO;
        self.vibrato_elapsed = Seconds::ZERO;
        self.vibrato_offset = Semitones::ZERO;
    }

    /// Advance the per-note vibrato LFO by one block and recompute the current
    /// pitch offset (semitones). Called once per block alongside `glide.update`.
    /// Pure arithmetic — RT-safe (no alloc/lock/panic).
    pub fn advance_vibrato(&mut self, delta_time: Seconds) {
        let Some(spec) = self.vibrato else {
            self.vibrato_offset = Semitones::ZERO;
            return;
        };
        self.vibrato_elapsed = Seconds::new(self.vibrato_elapsed.as_f32() + delta_time.as_f32());
        // Advance phase (rate * dt); `Phase::advance` wraps to [0,1).
        self.vibrato_phase = self
            .vibrato_phase
            .advance(spec.rate.as_f32() * delta_time.as_f32());
        // Linear depth fade-in over `fade_in` from onset (0 = instant), click-free.
        let fade = if spec.fade_in.as_f32() > 0.0 {
            (self.vibrato_elapsed.as_f32() / spec.fade_in.as_f32()).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let lfo = lfo_shape_value(spec.shape, self.vibrato_phase);
        let offset = spec.depth.as_f32() * lfo * fade;
        // Defensive: a non-finite depth/rate (only reachable via a programmatic
        // write that bypasses validation) must not propagate NaN into the
        // oscillator frequency and poison the mix/AWE buffers.
        self.vibrato_offset = if offset.is_finite() {
            Semitones::new(offset)
        } else {
            Semitones::ZERO
        };
    }

    /// Trigger note off.
    pub fn note_off(&mut self) {
        // Transition from Active to Releasing, preserving the note data
        if let VoiceState::Active {
            note,
            velocity,
            start_time,
        } = self.state
        {
            self.state = VoiceState::Releasing {
                note,
                velocity,
                start_time,
            };
            self.graph.note_off();
        }
    }

    /// Start voice stealing (quick fade-out).
    pub fn steal(&mut self) {
        self.state = VoiceState::Stealing {
            fade_counter: self.steal_fade_samples,
            fade_total: self.steal_fade_samples,
            pending_note: None,
        };
    }

    /// Start voice stealing with a pending note to trigger after fade-out.
    pub fn steal_for(&mut self, note: MidiNote, velocity: Velocity, time: SamplePosition) {
        self.state = VoiceState::Stealing {
            fade_counter: self.steal_fade_samples,
            fade_total: self.steal_fade_samples,
            pending_note: Some((note, velocity, time)),
        };
    }

    /// Check if voice is available for new notes.
    #[inline]
    pub fn is_available(&self) -> bool {
        self.state.is_idle()
    }

    /// Check if voice is producing sound.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Get the note this voice is playing, if any.
    #[inline]
    pub fn note(&self) -> Option<MidiNote> {
        self.state.note()
    }

    /// Get the velocity of this voice, if playing.
    #[inline]
    pub fn velocity(&self) -> Option<Velocity> {
        self.state.velocity()
    }

    /// Reset the voice to idle state.
    pub fn reset(&mut self) {
        self.state = VoiceState::Idle;
        self.age = SampleCount::ZERO;
        self.glide = GlideState::default();
        self.vibrato = None;
        self.vibrato_phase = Phase::ZERO;
        self.vibrato_elapsed = Seconds::ZERO;
        self.vibrato_offset = Semitones::ZERO;
        // Note: We don't reset macro controllers here since they are channel-wide,
        // not per-voice. They persist across notes.

        self.graph.reset();
    }

    /// Process audio for this voice through the module graph.
    ///
    /// Returns stereo output in left/right buffers.
    ///
    /// # Real-time Safety
    /// This method delegates to the ModuleGraph which manages its own buffers.
    pub fn process_audio(
        &mut self,
        left_out: &mut AudioBuffer,
        right_out: &mut AudioBuffer,
        context: &ProcessContext,
    ) {
        let samples = context.samples;

        // Get velocity from state (defaults to 1.0 if not playing - shouldn't happen)
        let velocity = self.state.velocity().unwrap_or(Velocity::MAX);

        // === Calculate frequency with pitch bend ===
        let base_freq = self.glide.get_frequency();

        // Apply pitch bend + per-note vibrato as additive semitone offsets on top
        // of the base pitch (base/glide pitch untouched). The mod-matrix OscPitch
        // destination still adds its own offset to the oscillator afterwards, so
        // mod-matrix vibrato and per-note vibrato compose additively.
        let bend_semitones = self.expression.pitch_bend_range * self.pitch_bend.as_f32();
        let freq = (bend_semitones + self.vibrato_offset).apply(base_freq);

        // Set oscillator frequencies in the graph before processing
        self.graph.set_oscillator_frequency(freq);

        // === Mod Matrix: update sources and apply modulations ===
        if let Some(mm_id) = self.mod_matrix_id {
            // Update performance controller sources on the mod matrix module
            if let Some(mm_module) = self.graph.get_module_mut(mm_id) {
                // Downcast to ModMatrix to call update_source
                // We use set_param indirectly — but update_source needs direct access.
                // Since PolyModule doesn't expose update_source, we read LFO/Env outputs
                // from graph and use a two-phase approach:
                // Phase 1: Gather source values
                // Phase 2: Apply modulations to graph
                let _ = mm_module; // drop borrow
            }
            self.apply_mod_matrix(mm_id);
        }

        // Ensure buffers are sized correctly
        self.mono_buffer.resize(samples.as_usize());
        self.mono_buffer.clear();

        // Process the entire graph
        self.graph.process(&mut self.mono_buffer, context);

        // Clear mod offsets after processing
        if self.mod_matrix_id.is_some() {
            self.graph.clear_mod_offsets();
        }

        // Extract stereo output from the output module if available
        if let Some(out_id) = self.output_module_id {
            // Try different port naming conventions:
            // 1. "left"/"right" (StereoOutput, Amplifier)
            // 2. "out_l"/"out_r" (some effects)
            // 3. "out" (Mixer, mono modules) - duplicate to stereo
            let left = self
                .graph
                .get_module_output(out_id, PortName::LEFT)
                .or_else(|| self.graph.get_module_output(out_id, PortName::OUT_L))
                .or_else(|| self.graph.get_module_output(out_id, PortName::OUT));

            let right = self
                .graph
                .get_module_output(out_id, PortName::RIGHT)
                .or_else(|| self.graph.get_module_output(out_id, PortName::OUT_R))
                .or_else(|| self.graph.get_module_output(out_id, PortName::OUT));

            if let Some(l) = left {
                left_out.copy_from(l);
            } else {
                left_out.copy_from(&self.mono_buffer);
            }

            if let Some(r) = right {
                right_out.copy_from(r);
            } else {
                right_out.copy_from(&self.mono_buffer);
            }
        } else {
            // No output module found - use graph's mono buffer for both channels
            left_out.copy_from(&self.mono_buffer);
            right_out.copy_from(&self.mono_buffer);
        }

        // Apply velocity scaling
        let amp_sens = self.expression.velocity_to_amp.as_f32();
        let amp_scale = (1.0 - amp_sens) + amp_sens * velocity.as_f32();

        for i in 0..samples.as_usize() {
            left_out[i] *= amp_scale;
            right_out[i] *= amp_scale;
        }
    }

    /// Apply modulation matrix: resolve each routing's source (macro / module
    /// output / param) and add the scaled offset to its destination.
    fn apply_mod_matrix(&mut self, mm_id: crate::ModuleId) {
        // Per-voice macro source values.
        let macros = MacroValues {
            velocity: self.state.velocity().map(|v| v.as_f32()).unwrap_or(0.0),
            note: self
                .state
                .note()
                .map(|n| n.as_u8() as f32 / 127.0)
                .unwrap_or(0.0),
            aftertouch: self.aftertouch.as_f32(),
            mod_wheel: self.mod_wheel.as_f32(),
            pitch_bend: self.pitch_bend.as_f32(),
            poly_aftertouch: self.poly_aftertouch.as_f32(),
        };

        // Resolve each routing's source value into the pre-allocated cache while
        // the graph is borrowed immutably; then apply (mutable) from the cache.
        self.resolve_routings_into_cache(mm_id, &macros);
        for i in 0..self.mod_slots_cache.len() {
            let (src_value, dst, amount) = self.mod_slots_cache[i];
            self.graph.apply_mod_offset_addr(dst, src_value * amount);
        }
    }

    /// Resolve a single source address to its previous-block value.
    fn resolve_source(graph: &ModuleGraph, src: synth_core::SrcAddr, macros: &MacroValues) -> f32 {
        use synth_core::{KineticParam, MacroSource, Param, SrcAddr};
        match src {
            SrcAddr::Macro(m) => match m {
                MacroSource::Velocity => macros.velocity,
                MacroSource::NoteNumber => macros.note,
                MacroSource::Aftertouch => macros.aftertouch,
                MacroSource::ModWheel => macros.mod_wheel,
                MacroSource::PitchBend => macros.pitch_bend,
                MacroSource::PolyAftertouch => macros.poly_aftertouch,
            },
            SrcAddr::Module {
                module_type,
                instance,
                name,
            } => {
                let id = crate::ModuleId::new(module_type, instance);
                // Kinetic vel/acc are parameter outputs, not ports.
                if module_type == ModuleType::KineticModulator {
                    match name.as_str() {
                        "vel" => {
                            return graph
                                .get_param(id, &Param::Kinetic(KineticParam::OutputVel(0.0)))
                                .unwrap_or(0.0);
                        }
                        "acc" => {
                            return graph
                                .get_param(id, &Param::Kinetic(KineticParam::OutputAcc(0.0)))
                                .unwrap_or(0.0);
                        }
                        _ => {}
                    }
                }
                // Output port: previous block's first sample (0 if absent — a
                // dangling address or empty buffer).
                graph
                    .get_module_output(id, name)
                    .map(|buf| if buf.is_empty() { 0.0 } else { buf[0] })
                    .unwrap_or(0.0)
            }
        }
    }

    /// Resolve every active routing's source value into the pre-allocated cache.
    ///
    /// Stores `(resolved source value, destination, amount)` so the immutable
    /// borrow of the graph (reading source ports/params) is released before the
    /// apply loop takes a mutable borrow. Reuses `mod_slots_cache` to stay
    /// allocation-free on the audio thread.
    fn resolve_routings_into_cache(&mut self, mm_id: crate::ModuleId, macros: &MacroValues) {
        self.mod_slots_cache.clear();

        let Some(routings) = self.graph.get_module(mm_id).and_then(|m| m.mod_routings()) else {
            return;
        };
        for routing in routings {
            // Only enabled routings with both a source and a destination.
            if !routing.enabled {
                continue;
            }
            let (Some(source), Some(dest)) = (routing.source, routing.destination) else {
                continue;
            };
            let value = Self::resolve_source(&self.graph, source, macros);
            self.mod_slots_cache
                .push((value, dest, routing.amount.as_f32()));
        }
    }

    /// Clone the voice structure (for voice allocation).
    pub fn clone_structure(&self) -> Self {
        let cloned_graph = self.graph.clone_structure();

        // Find output module with priority: StereoOutput > Amplifier > Mixer
        let output_id = cloned_graph
            .find_module_by_type(ModuleType::StereoOutput)
            .or_else(|| cloned_graph.find_module_by_type(ModuleType::Amplifier))
            .or_else(|| cloned_graph.find_module_by_type(ModuleType::Mixer));

        let mod_matrix_id = cloned_graph.find_module_by_type(ModuleType::ModMatrix);

        Self {
            id: self.id,
            state: VoiceState::Idle,
            age: SampleCount::ZERO,
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            poly_aftertouch: NormalizedValue::MIN,
            expression: self.expression,
            graph: cloned_graph,
            steal_fade_samples: self.steal_fade_samples,
            glide: GlideState::default(),
            glide_time: self.glide_time,
            vibrato: None,
            vibrato_phase: Phase::ZERO,
            vibrato_elapsed: Seconds::ZERO,
            vibrato_offset: Semitones::ZERO,
            output_module_id: output_id,
            mod_matrix_id,
            mod_slots_cache: Vec::with_capacity(16),
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            tuning_table: self.tuning_table.clone(),
        }
    }

    /// Update the cached output module ID.
    /// Call this after modifying the graph structure.
    pub fn update_output_cache(&mut self) {
        self.output_module_id = self
            .graph
            .find_module_by_type(ModuleType::StereoOutput)
            .or_else(|| self.graph.find_module_by_type(ModuleType::Amplifier))
            .or_else(|| self.graph.find_module_by_type(ModuleType::Mixer));
        self.mod_matrix_id = self.graph.find_module_by_type(ModuleType::ModMatrix);
    }
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        self.clone_structure()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glide_state() {
        let mut glide = GlideState {
            current_freq: Hertz::A4,
            ..Default::default()
        };

        // Glide to 880 Hz (one octave up) over 0.1 seconds
        glide.start(Hertz::new(880.0), Seconds::new(0.1));

        assert!(glide.active);
        assert_eq!(glide.from_freq, Hertz::A4);
        assert_eq!(glide.to_freq, Hertz::new(880.0));

        // Update halfway
        let freq = glide.update(Seconds::new(0.05));
        assert!(freq.as_f32() > 440.0 && freq.as_f32() < 880.0);

        // Complete the glide
        let freq = glide.update(Seconds::new(0.1));
        assert!(!glide.active);
        assert!((freq.as_f32() - 880.0).abs() < 1.0);
    }

    #[test]
    fn test_glide_start_from_explicit_source() {
        // Per-note glide seeds both endpoints regardless of current_freq.
        let mut glide = GlideState {
            current_freq: Hertz::new(100.0),
            ..Default::default()
        };
        glide.start_from(Hertz::A4, Hertz::new(880.0), Seconds::new(0.1), false);

        assert!(glide.active);
        assert_eq!(glide.from_freq, Hertz::A4);
        assert_eq!(glide.current_freq, Hertz::A4); // starts at source, not 100 Hz
        assert_eq!(glide.to_freq, Hertz::new(880.0));

        let freq = glide.update(Seconds::new(0.05));
        assert!(freq.as_f32() > 440.0 && freq.as_f32() < 880.0);
    }

    #[test]
    fn test_glide_stepped_holds_at_semitones() {
        // Stepped (glissando) quantizes the trajectory to integer semitones
        // relative to the source. A4 (440) -> A5 (880), one octave = 12 steps.
        let mut glide = GlideState::default();
        glide.start_from(Hertz::A4, Hertz::new(880.0), Seconds::new(1.0), true);
        assert!(glide.stepped);

        // Advance partway and assert the frequency lands on a chromatic step
        // (440 * 2^(n/12) for integer n), not a continuous value.
        let freq = glide.update(Seconds::new(0.3)).as_f32();
        let semis = 12.0 * (freq / 440.0).log2();
        assert!(
            (semis - semis.round()).abs() < 1e-3,
            "stepped glide freq {freq} = {semis} semitones above A4 is not on a step"
        );
    }

    fn vibrato_trigger(depth: f32, rate: f32, fade_in: f32) -> NoteTrigger {
        NoteTrigger {
            legato: false,
            glide: None,
            vibrato: Some(VibratoSpec {
                depth: Semitones::new(depth),
                rate: Hertz::new(rate),
                fade_in: Seconds::new(fade_in),
                shape: LfoWaveform::Sine,
            }),
        }
    }

    #[test]
    fn vibrato_modulates_pitch_offset() {
        let mut voice = Voice::new(VoiceId::new(0));
        voice.note_on_expr(
            MidiNote::A4,
            Velocity::MF,
            SamplePosition::ZERO,
            vibrato_trigger(0.5, 5.0, 0.0),
        );
        // 5 Hz → period 0.2 s; a quarter period (0.05 s) → phase 0.25 → sin = 1 →
        // offset ≈ +depth.
        voice.advance_vibrato(Seconds::new(0.05));
        let off = voice.vibrato_offset.as_f32();
        assert!(
            (off - 0.5).abs() < 0.05,
            "quarter-period sine vibrato ≈ +depth, got {off}"
        );
        // Offset stays bounded by depth across a full cycle.
        for _ in 0..40 {
            voice.advance_vibrato(Seconds::new(0.01));
            assert!(voice.vibrato_offset.as_f32().abs() <= 0.5 + 1e-4);
        }
    }

    #[test]
    fn vibrato_zero_depth_is_silent() {
        let mut voice = Voice::new(VoiceId::new(0));
        voice.note_on_expr(
            MidiNote::A4,
            Velocity::MF,
            SamplePosition::ZERO,
            vibrato_trigger(0.0, 5.0, 0.0),
        );
        for _ in 0..10 {
            voice.advance_vibrato(Seconds::new(0.02));
            assert_eq!(voice.vibrato_offset.as_f32(), 0.0);
        }
    }

    #[test]
    fn no_vibrato_keeps_offset_zero() {
        let mut voice = Voice::new(VoiceId::new(0));
        voice.note_on(MidiNote::A4, Velocity::MF, SamplePosition::ZERO);
        voice.advance_vibrato(Seconds::new(0.05));
        assert_eq!(voice.vibrato_offset.as_f32(), 0.0);
    }

    #[test]
    fn vibrato_fade_in_ramps_depth() {
        let mut voice = Voice::new(VoiceId::new(0));
        // 0.2 s fade-in; at a quarter period (0.05 s) the sine is at +1 but the
        // fade scale is only 0.25, so the offset is ~depth * 0.25.
        voice.note_on_expr(
            MidiNote::A4,
            Velocity::MF,
            SamplePosition::ZERO,
            vibrato_trigger(0.8, 5.0, 0.2),
        );
        voice.advance_vibrato(Seconds::new(0.05));
        let off = voice.vibrato_offset.as_f32();
        assert!(
            (off - 0.2).abs() < 0.05,
            "faded vibrato ≈ depth*0.25, got {off}"
        );
    }

    #[test]
    fn test_note_to_freq() {
        let voice = Voice::new(VoiceId::new(0));

        // A4 should be 440 Hz
        let freq = voice.note_to_freq(MidiNote::A4);
        assert!((freq - 440.0).abs() < 0.01);

        // A5 should be 880 Hz (one octave up)
        let freq = voice.note_to_freq(MidiNote::new(81));
        assert!((freq - 880.0).abs() < 0.01);

        // A3 should be 220 Hz (one octave down)
        let freq = voice.note_to_freq(MidiNote::new(57));
        assert!((freq - 220.0).abs() < 0.01);
    }

    #[test]
    fn test_microtonal_tuning() {
        let mut voice = Voice::new(VoiceId::new(0));
        voice.set_tuning_table(synth_core::tuning::TuningTable::just_intonation());

        // A4 should still be 440 Hz in just intonation
        let freq = voice.note_to_freq(MidiNote::A4);
        assert!((freq - 440.0).abs() < 0.5);
    }

    #[test]
    fn test_voice_with_graph() {
        let voice = Voice::new(VoiceId::new(0));
        assert_eq!(voice.state, VoiceState::Idle);
        assert!(voice.graph.is_empty());
    }

    #[test]
    fn test_voice_param_override_delegates_to_graph() {
        use synth_core::{AmplifierParam, Gain, SampleCount, SampleRate};
        use synth_modules::{Amplifier, Oscillator};

        let mut voice = Voice::new(VoiceId::new(0));
        let osc_id = voice.graph.add_module(Box::new(Oscillator::new()));
        let amp_id = voice.graph.add_module(Box::new(Amplifier::new()));
        voice
            .graph
            .connect(osc_id, "out", amp_id, "in")
            .expect("osc out -> amp in");

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        fn peak(voice: &mut Voice, ctx: &ProcessContext<'_>) -> f32 {
            let mut out = AudioBuffer::new(256);
            voice.graph.process(&mut out, ctx);
            (0..256).map(|i| out[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut voice, &ctx);
        assert!(base > 0.01);

        voice.apply_param_override(
            amp_id,
            Param::Amplifier(AmplifierParam::Level(Gain::new(0.0))),
        );
        // Level is de-zippered with a per-block ramp, so the first block after
        // the override fades down; the next block is fully silent.
        let _ = peak(&mut voice, &ctx);
        assert!(
            peak(&mut voice, &ctx) < 1e-4,
            "override should silence the amp once the ramp settles"
        );

        // Free-running oscillator: assert audible output is restored, not an
        // exact peak (phase advances between blocks). One settling block first.
        voice.clear_param_overrides();
        let _ = peak(&mut voice, &ctx);
        assert!(
            peak(&mut voice, &ctx) > 0.01,
            "clearing must restore audible output"
        );
    }
}
