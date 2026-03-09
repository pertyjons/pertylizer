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

use crate::graph::ModuleGraph;
use synth_core::tuning::TuningTable;
use synth_core::{AudioBuffer, PortName, ProcessContext};
use synth_core::{
    BipolarValue, Cents, Hertz, MidiNote, NormalizedValue, SampleCount, SamplePosition, Seconds,
    Semitones, Velocity,
};
use synth_core::{ModuleType, OscillatorParam, Param};

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

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
        }
    }
}

impl GlideState {
    /// Start a new glide from current frequency to target.
    pub(crate) fn start(&mut self, target_freq: Hertz, glide_time: Seconds) {
        if glide_time.as_f32() > 0.0
            && (self.current_freq.as_f32() - target_freq.as_f32()).abs() > 0.01
        {
            self.from_freq = self.current_freq;
            self.to_freq = target_freq;
            self.time = glide_time;
            self.position = NormalizedValue::MIN;
            self.active = true;
        } else {
            // No glide - jump directly
            self.current_freq = target_freq;
            self.from_freq = target_freq;
            self.to_freq = target_freq;
            self.position = NormalizedValue::MAX;
            self.active = false;
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
            let ratio = self.to_freq.as_f32() / self.from_freq.as_f32();
            self.current_freq =
                Hertz::new(self.from_freq.as_f32() * ratio.powf(self.position.as_f32()));
        }

        self.current_freq
    }

    /// Get current frequency without updating.
    #[must_use]
    pub(crate) fn get_frequency(&self) -> Hertz {
        self.current_freq
    }
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
    pub id: u32,

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

    /// Cached output module ID for stereo output extraction.
    /// Priority: StereoOutput > Amplifier > Mixer
    output_module_id: Option<crate::ModuleId>,

    /// Cached mod matrix module ID (if present in graph).
    mod_matrix_id: Option<crate::ModuleId>,

    /// Pre-allocated buffer for mod matrix slot data (avoids per-frame Vec allocation).
    mod_slots_cache: Vec<(usize, synth_core::ModDestination, f32)>,

    /// Temporary mono buffer for graph processing.
    mono_buffer: AudioBuffer,

    /// Tuning table for note-to-frequency conversion.
    /// Defaults to 12-TET, can be changed for microtonal tuning.
    tuning_table: TuningTable,
}

impl Voice {
    /// Create a new voice with the given ID.
    pub fn new(id: u32) -> Self {
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
            output_module_id: None,
            mod_matrix_id: None,
            mod_slots_cache: Vec::with_capacity(16),
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            tuning_table: TuningTable::default(),
        }
    }

    /// Create a new voice from a ModuleGraph template.
    pub fn from_graph(id: u32, graph: ModuleGraph) -> Self {
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
        // Apply to all oscillators in the graph
        for module_id in self.graph.module_ids().collect::<Vec<_>>() {
            if module_id.module_type == ModuleType::Oscillator {
                self.graph.set_param(
                    module_id,
                    Param::Oscillator(OscillatorParam::Detune(detune)),
                );
            }
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
        let target_freq = Hertz::new(self.note_to_freq(note));

        // Start glide from current position if we have a glide time and were already active
        let was_active = matches!(self.state, VoiceState::Active { .. });
        if self.glide_time.as_f32() > 0.0 && was_active {
            self.glide.start(target_freq, self.glide_time);
        } else {
            // No glide - set frequency immediately
            self.glide.current_freq = target_freq;
            self.glide.from_freq = target_freq;
            self.glide.to_freq = target_freq;
            self.glide.active = false;
        }

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
    /// Updates the note in the current state if active.
    pub fn glide_to_note(&mut self, new_note: MidiNote) {
        let target_freq = Hertz::new(self.note_to_freq(new_note));

        // Update glide target
        if self.glide_time.as_f32() > 0.0 {
            self.glide.start(target_freq, self.glide_time);
        } else {
            self.glide.current_freq = target_freq;
            self.glide.to_freq = target_freq;
            self.glide.active = false;
        }

        // Update the note in the state (only if active)
        if let VoiceState::Active {
            note,
            velocity: _,
            start_time: _,
        } = &mut self.state
        {
            *note = new_note;
        }
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

        // Apply pitch bend: bend_semitones = pitch_bend * range
        let bend_semitones = self.expression.pitch_bend_range * self.pitch_bend.as_f32();
        let freq = bend_semitones.apply(base_freq);

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

    /// Apply modulation matrix: update source values and apply offsets to destination modules.
    fn apply_mod_matrix(&mut self, mm_id: crate::ModuleId) {
        // Gather source values from the voice state and previous block outputs
        let velocity_val = self.state.velocity().map(|v| v.as_f32()).unwrap_or(0.0);
        let note_val = self
            .state
            .note()
            .map(|n| n.as_u8() as f32 / 127.0)
            .unwrap_or(0.0);
        let aftertouch_val = self.aftertouch.as_f32();
        let mod_wheel_val = self.mod_wheel.as_f32();
        let pitch_bend_val = self.pitch_bend.as_f32();

        // Read LFO/Envelope/Kinetic outputs from previous block
        // Uses internal graph iteration to avoid collect::<Vec<_>>() allocation
        let (lfo_values, env_values, kinetic_pos, kinetic_vel, kinetic_acc) =
            self.graph.gather_mod_source_values();

        // Build source values array matching ModSource::ALL indices
        // ModSource::ALL: [None, Lfo(0), Lfo(1), Env(0), Env(1), Velocity, NoteNumber,
        //   Aftertouch, ModWheel, PitchBend, PolyAftertouch, KineticPos, KineticVel, KineticAcc]
        let poly_aftertouch_val = self.poly_aftertouch.as_f32();
        let source_values: [f32; 14] = [
            0.0,                 // None
            lfo_values[0],       // Lfo(0)
            lfo_values[1],       // Lfo(1)
            env_values[0],       // Envelope(0)
            env_values[1],       // Envelope(1)
            velocity_val,        // Velocity
            note_val,            // NoteNumber
            aftertouch_val,      // Aftertouch
            mod_wheel_val,       // ModWheel
            pitch_bend_val,      // PitchBend
            poly_aftertouch_val, // PolyAftertouch
            kinetic_pos,         // KineticPos
            kinetic_vel,         // KineticVel
            kinetic_acc,         // KineticAcc
        ];

        // We can't downcast dyn PolyModule to ModMatrix, so we read the slot config
        // through get_param and calculate modulations here in Voice.

        // Read mod matrix configuration and apply modulations (uses pre-allocated cache)
        self.read_mod_matrix_slots_into_cache(mm_id);
        for i in 0..self.mod_slots_cache.len() {
            let (src_idx, dst, amount) = self.mod_slots_cache[i];

            if src_idx == 0 || matches!(dst, synth_core::ModDestination::None) {
                continue;
            }
            let src_value = if src_idx < source_values.len() {
                source_values[src_idx]
            } else {
                0.0
            };
            let scaled = src_value * amount;
            self.graph.apply_mod_offset(dst, scaled);
        }
    }

    /// Read mod matrix slot configurations into the pre-allocated cache.
    /// Reuses `self.mod_slots_cache` to avoid per-frame Vec allocation.
    fn read_mod_matrix_slots_into_cache(&mut self, mm_id: crate::ModuleId) {
        use synth_core::{ModDestination, ModMatrixGridSize, ModMatrixParam, ModSource as MS};

        self.mod_slots_cache.clear();

        // Read grid size to determine how many slots to process
        let grid_size_idx = self
            .graph
            .get_param(
                mm_id,
                &Param::ModMatrix(ModMatrixParam::GridSize(ModMatrixGridSize::default())),
            )
            .map(|v| v as usize)
            .unwrap_or(ModMatrixGridSize::default().index());
        let grid_size = ModMatrixGridSize::from_index(grid_size_idx);
        let slot_count = grid_size.slot_count();

        for i in 0..slot_count {
            let slot = i as u8;
            let src_idx = self
                .graph
                .get_param(
                    mm_id,
                    &Param::ModMatrix(ModMatrixParam::SlotSource(slot, MS::None)),
                )
                .map(|v| v as usize)
                .unwrap_or(0);
            let dst_idx = self
                .graph
                .get_param(
                    mm_id,
                    &Param::ModMatrix(ModMatrixParam::SlotDestination(slot, ModDestination::None)),
                )
                .map(|v| v as usize)
                .unwrap_or(0);
            let amount = self
                .graph
                .get_param(
                    mm_id,
                    &Param::ModMatrix(ModMatrixParam::SlotAmount(slot, BipolarValue::CENTER)),
                )
                .unwrap_or(0.0);

            let dst = ModDestination::from_index(dst_idx);
            self.mod_slots_cache.push((src_idx, dst, amount));
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
        let mut glide = GlideState::default();

        // Start at 440 Hz
        glide.current_freq = Hertz::A4;

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
    fn test_note_to_freq() {
        let voice = Voice::new(0);

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
        let mut voice = Voice::new(0);
        voice.set_tuning_table(synth_core::tuning::TuningTable::just_intonation());

        // A4 should still be 440 Hz in just intonation
        let freq = voice.note_to_freq(MidiNote::A4);
        assert!((freq - 440.0).abs() < 0.5);
    }

    #[test]
    fn test_voice_with_graph() {
        let voice = Voice::new(0);
        assert_eq!(voice.state, VoiceState::Idle);
        assert!(voice.graph.is_empty());
    }
}
