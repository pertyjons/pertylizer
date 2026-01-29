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

use std::sync::LazyLock;

use crate::graph::ModuleGraph;
use synth_core::{AudioBuffer, ProcessContext};
use synth_core::{
    BipolarValue, Cents, Hertz, MidiNote, NormalizedValue, SampleCount, SamplePosition, Seconds,
    Semitones, Velocity,
};
use synth_core::{ModuleType, OscillatorParam, Param};

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

/// Pre-computed MIDI note to frequency lookup table.
/// Avoids expensive powf() calls during audio processing.
/// Initialized once on first access.
static NOTE_FREQ_TABLE: LazyLock<[f32; 128]> = LazyLock::new(|| {
    // 440.0 * 2^((note - 69) / 12)
    std::array::from_fn(|i| 440.0 * 2.0f32.powf((i as f32 - 69.0) / 12.0))
});

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
        fade_counter: usize,
        /// Total fade samples (for calculating fade ratio)
        fade_total: usize,
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
pub struct GlideState {
    /// Source frequency in Hz
    pub from_freq: f32,
    /// Target frequency in Hz
    pub to_freq: f32,
    /// Current frequency (during glide)
    pub current_freq: f32,
    /// Glide time in seconds
    pub time: f32,
    /// Current position in glide (0.0 = start, 1.0 = complete)
    pub position: f32,
    /// Whether glide is active
    pub active: bool,
}

impl Default for GlideState {
    fn default() -> Self {
        Self {
            from_freq: 440.0,
            to_freq: 440.0,
            current_freq: 440.0,
            time: 0.0,
            position: 1.0,
            active: false,
        }
    }
}

impl GlideState {
    /// Start a new glide from current frequency to target.
    pub fn start(&mut self, target_freq: f32, glide_time: f32) {
        if glide_time > 0.0 && (self.current_freq - target_freq).abs() > 0.01 {
            self.from_freq = self.current_freq;
            self.to_freq = target_freq;
            self.time = glide_time;
            self.position = 0.0;
            self.active = true;
        } else {
            // No glide - jump directly
            self.current_freq = target_freq;
            self.from_freq = target_freq;
            self.to_freq = target_freq;
            self.position = 1.0;
            self.active = false;
        }
    }

    /// Update the glide state (call once per sample or per block).
    /// Returns the current frequency.
    pub fn update(&mut self, delta_time: f32) -> f32 {
        if !self.active {
            return self.current_freq;
        }

        // Update position
        if self.time > 0.0 {
            self.position += delta_time / self.time;
        } else {
            self.position = 1.0;
        }

        if self.position >= 1.0 {
            self.position = 1.0;
            self.active = false;
            self.current_freq = self.to_freq;
        } else {
            // Exponential interpolation (sounds more natural for pitch)
            // f(t) = from * (to/from)^t
            let ratio = self.to_freq / self.from_freq;
            self.current_freq = self.from_freq * ratio.powf(self.position);
        }

        self.current_freq
    }

    /// Get current frequency without updating.
    pub fn get_frequency(&self) -> f32 {
        self.current_freq
    }

    /// Check if glide is complete.
    pub fn is_complete(&self) -> bool {
        !self.active
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
    /// Aftertouch amount (0.0 to 1.0, type-safe).
    pub aftertouch: NormalizedValue,

    // === Tracker modulation (from vibrato, portamento, volume slide, etc.) ===
    /// Tracker pitch modulation in cents (from vibrato, portamento effects).
    pub tracker_pitch_cents: Cents,
    /// Tracker volume modulation (from volume slide effects).
    pub tracker_volume: NormalizedValue,
    /// Tracker panning modulation (from panning slide effects).
    pub tracker_panning: BipolarValue,
    /// Tone portamento target pitch in semitones (when active, overrides note pitch).
    /// This is set by the 3xx/5xx effects and represents the current interpolated pitch
    /// during a glide. When Some, this pitch is used instead of the note's base pitch.
    pub tracker_tone_porta_pitch: Option<Semitones>,

    /// Expression settings (pitch bend range, velocity sensitivity, etc.).
    pub expression: ExpressionSettings,

    /// The module graph that defines this voice's signal chain.
    pub graph: ModuleGraph,

    /// Default steal fade duration in samples.
    steal_fade_samples: usize,

    /// Glide state for portamento.
    pub glide: GlideState,
    /// Configured glide time.
    glide_time: Seconds,

    /// Cached output module ID for stereo output extraction.
    /// Priority: StereoOutput > Amplifier > Mixer
    output_module_id: Option<crate::ModuleId>,

    /// Temporary mono buffer for graph processing.
    mono_buffer: AudioBuffer,
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
            // Tracker modulation default to neutral
            tracker_pitch_cents: Cents::ZERO,
            tracker_volume: NormalizedValue::MAX, // 1.0 = no volume change
            tracker_panning: BipolarValue::CENTER,
            tracker_tone_porta_pitch: None,
            expression: ExpressionSettings::default(),
            graph: ModuleGraph::new(),
            steal_fade_samples: 128,
            glide: GlideState::default(),
            glide_time: Seconds::ZERO,
            output_module_id: None,
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
        }
    }

    /// Create a new voice from a ModuleGraph template.
    pub fn from_graph(id: u32, graph: ModuleGraph) -> Self {
        // Find output module with priority: StereoOutput > Amplifier > Mixer
        let output_id = graph
            .find_module_by_type(ModuleType::StereoOutput)
            .or_else(|| graph.find_module_by_type(ModuleType::Amplifier))
            .or_else(|| graph.find_module_by_type(ModuleType::Mixer));

        Self {
            id,
            state: VoiceState::Idle,
            age: SampleCount::ZERO,
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            tracker_pitch_cents: Cents::ZERO,
            tracker_volume: NormalizedValue::MAX,
            tracker_panning: BipolarValue::CENTER,
            tracker_tone_porta_pitch: None,
            expression: ExpressionSettings::default(),
            graph,
            steal_fade_samples: 128,
            glide: GlideState::default(),
            glide_time: Seconds::ZERO,
            output_module_id: output_id,
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
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
    pub fn get_current_frequency(&self) -> f32 {
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

    /// Convert MIDI note to frequency (A4 = 440 Hz).
    /// Uses a pre-computed lookup table to avoid expensive powf() calls.
    #[inline]
    fn note_to_freq(note: MidiNote) -> f32 {
        NOTE_FREQ_TABLE[note.as_u8() as usize]
    }

    /// Trigger note on with type-safe velocity.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity, time: SamplePosition) {
        let target_freq = Self::note_to_freq(note);

        // Start glide from current position if we have a glide time and were already active
        let was_active = matches!(self.state, VoiceState::Active { .. });
        if self.glide_time.as_f32() > 0.0 && was_active {
            self.glide.start(target_freq, self.glide_time.as_f32());
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
        let target_freq = Self::note_to_freq(new_note);

        // Update glide target
        if self.glide_time.as_f32() > 0.0 {
            self.glide.start(target_freq, self.glide_time.as_f32());
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

    /// Retrigger the voice with a sample offset.
    ///
    /// Used for tracker retrigger effects (Exy) and note delay (EDx).
    /// Restarts the sample playback from the given offset position.
    pub fn retrigger_with_offset(&mut self, sample_offset: NormalizedValue) {
        // Retrigger all modules in the graph (this restarts sample players, etc.)
        self.graph.retrigger_with_offset(sample_offset);
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

        // === Calculate frequency with pitch bend and tracker modulation ===
        // If tone portamento is active, use the interpolated pitch from tracker effects.
        // Otherwise, use the note's base frequency from glide state.
        let base_freq = if let Some(tone_porta_semitones) = self.tracker_tone_porta_pitch {
            // Tone portamento active: convert semitones to frequency
            // semitones 60 = middle C = 261.63 Hz, using A4 = 440 Hz reference
            Hertz::new(440.0 * 2.0f32.powf((tone_porta_semitones.as_f32() - 69.0) / 12.0))
        } else {
            Hertz::new(self.glide.get_frequency())
        };

        // Apply pitch bend: bend_semitones = pitch_bend * range
        let bend_semitones = self.expression.pitch_bend_range * self.pitch_bend.as_f32();
        let freq_after_bend = bend_semitones.apply(base_freq);

        // Apply tracker pitch modulation (vibrato, portamento up/down effects)
        // tracker_pitch_cents is in cents, convert to semitones for frequency ratio
        // Note: This is additive modulation (vibrato, etc.), separate from tone portamento
        let tracker_semitones = Semitones::new(self.tracker_pitch_cents.as_f32() / 100.0);
        let freq = tracker_semitones.apply(freq_after_bend);

        // Set oscillator frequencies in the graph before processing
        self.graph.set_oscillator_frequency(freq);

        // Set pitch modulation for SamplePlayer modules (tracker effects)
        // This allows vibrato/portamento to modulate sample playback speed
        self.graph
            .set_sample_player_pitch_mod(tracker_semitones.as_f32());

        // Ensure buffers are sized correctly
        self.mono_buffer.resize(samples.as_usize());
        self.mono_buffer.clear();

        // Process the entire graph
        self.graph.process(&mut self.mono_buffer, context);

        // Extract stereo output from the output module if available
        if let Some(out_id) = self.output_module_id {
            // Try different port naming conventions:
            // 1. "left"/"right" (StereoOutput, Amplifier)
            // 2. "out_l"/"out_r" (some effects)
            // 3. "out" (Mixer, mono modules) - duplicate to stereo
            let left = self
                .graph
                .get_module_output(out_id, "left")
                .or_else(|| self.graph.get_module_output(out_id, "out_l"))
                .or_else(|| self.graph.get_module_output(out_id, "out"));

            let right = self
                .graph
                .get_module_output(out_id, "right")
                .or_else(|| self.graph.get_module_output(out_id, "out_r"))
                .or_else(|| self.graph.get_module_output(out_id, "out"));

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

        // Apply velocity scaling and tracker volume modulation
        let amp_sens = self.expression.velocity_to_amp.as_f32();
        let velocity_scale = (1.0 - amp_sens) + amp_sens * velocity.as_f32();
        let tracker_vol = self.tracker_volume.as_f32();
        let amp_scale = velocity_scale * tracker_vol;

        for i in 0..samples.as_usize() {
            left_out[i] *= amp_scale;
            right_out[i] *= amp_scale;
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

        Self {
            id: self.id,
            state: VoiceState::Idle,
            age: SampleCount::ZERO,
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            tracker_pitch_cents: Cents::ZERO,
            tracker_volume: NormalizedValue::MAX,
            tracker_panning: BipolarValue::CENTER,
            tracker_tone_porta_pitch: None,
            expression: self.expression,
            graph: cloned_graph,
            steal_fade_samples: self.steal_fade_samples,
            glide: GlideState::default(),
            glide_time: self.glide_time,
            output_module_id: output_id,
            mono_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
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
        glide.current_freq = 440.0;

        // Glide to 880 Hz (one octave up) over 0.1 seconds
        glide.start(880.0, 0.1);

        assert!(glide.active);
        assert_eq!(glide.from_freq, 440.0);
        assert_eq!(glide.to_freq, 880.0);

        // Update halfway
        let freq = glide.update(0.05);
        assert!(freq > 440.0 && freq < 880.0);

        // Complete the glide
        let freq = glide.update(0.1);
        assert!(!glide.active);
        assert!((freq - 880.0).abs() < 1.0);
    }

    #[test]
    fn test_note_to_freq() {
        // A4 should be 440 Hz
        let freq = Voice::note_to_freq(MidiNote::A4);
        assert!((freq - 440.0).abs() < 0.01);

        // A5 should be 880 Hz (one octave up)
        let freq = Voice::note_to_freq(MidiNote::new(81));
        assert!((freq - 880.0).abs() < 0.01);

        // A3 should be 220 Hz (one octave down)
        let freq = Voice::note_to_freq(MidiNote::new(57));
        assert!((freq - 220.0).abs() < 0.01);
    }

    #[test]
    fn test_voice_with_graph() {
        let voice = Voice::new(0);
        assert_eq!(voice.state, VoiceState::Idle);
        assert!(voice.graph.is_empty());
    }
}
