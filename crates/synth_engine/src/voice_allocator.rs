//! Voice allocator - manages polyphonic voice allocation.
//!
//! Features:
//! - Multiple allocation modes (poly, mono, legato)
//! - Voice stealing strategies
//! - Glide/portamento support

use serde::{Deserialize, Serialize};

use crate::voice::{NoteTrigger, Voice, VoiceId, VoiceState};
use synth_core::{
    BipolarValue, Cents, Gain, MidiNote, NormalizedValue, SampleCount, SamplePosition, Seconds,
    Velocity, VoiceCount,
};

/// Voice allocation mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum AllocationMode {
    /// Unlimited polyphony (up to max voices).
    #[default]
    Polyphonic,
    /// Monophonic with retriggering.
    Mono,
    /// Monophonic legato (no retrigger on overlapping notes).
    Legato,
    /// Unison - all voices play same note.
    Unison,
}

impl std::fmt::Display for AllocationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Polyphonic => "Polyphonic",
            Self::Mono => "Mono",
            Self::Legato => "Legato",
            Self::Unison => "Unison",
        })
    }
}

impl std::str::FromStr for AllocationMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Polyphonic" => Ok(Self::Polyphonic),
            "Mono" => Ok(Self::Mono),
            "Legato" => Ok(Self::Legato),
            "Unison" => Ok(Self::Unison),
            _ => Err(format!(
                "invalid allocation_mode {s:?}; expected Polyphonic, Mono, Legato, or Unison"
            )),
        }
    }
}

/// Strategy for stealing voices when all are busy.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum StealingStrategy {
    /// Don't steal - ignore new notes when full.
    None,
    /// Steal the oldest active voice.
    #[default]
    Oldest,
    /// Steal the quietest voice (based on envelope level).
    Quietest,
    /// Steal the lowest priority voice.
    LowestPriority,
    /// Steal the same note if playing.
    SameNote,
}

impl std::fmt::Display for StealingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Oldest => "Oldest",
            Self::Quietest => "Quietest",
            Self::LowestPriority => "LowestPriority",
            Self::SameNote => "SameNote",
        })
    }
}

impl std::str::FromStr for StealingStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "None" => Ok(Self::None),
            "Oldest" => Ok(Self::Oldest),
            "Quietest" => Ok(Self::Quietest),
            "LowestPriority" => Ok(Self::LowestPriority),
            "SameNote" => Ok(Self::SameNote),
            _ => Err(format!(
                "invalid stealing_strategy {s:?}; expected None, Oldest, Quietest, \
                 LowestPriority, or SameNote"
            )),
        }
    }
}

/// Note priority for mono/legato modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePriority {
    /// Last note played takes priority.
    Last,
    /// Highest note takes priority.
    High,
    /// Lowest note takes priority.
    Low,
}

/// Voice allocator configuration.
#[derive(Debug, Clone)]
pub struct AllocatorConfig {
    /// Maximum number of voices (type-safe count).
    pub max_voices: VoiceCount,
    /// Allocation mode.
    pub mode: AllocationMode,
    /// Voice stealing strategy.
    pub stealing: StealingStrategy,
    /// Note priority (for mono modes).
    pub priority: NotePriority,
    /// Glide time in seconds.
    pub glide_time: Seconds,
    /// Total unison detune spread across all `AllocationMode::Unison` voices, in
    /// cents. Voices are detuned evenly around the centre pitch.
    pub unison_detune: Cents,
    /// Stereo spread (0..1) for `AllocationMode::Unison` voices: pans them across
    /// the stereo field (0 = no spread / centred, 1 = full L↔R). Default 0.
    pub unison_spread: NormalizedValue,
}

impl Default for AllocatorConfig {
    fn default() -> Self {
        Self {
            max_voices: VoiceCount::OCTO,
            mode: AllocationMode::Polyphonic,
            stealing: StealingStrategy::Oldest,
            priority: NotePriority::Last,
            glide_time: Seconds::ZERO,
            unison_detune: Cents::new(10.0),
            unison_spread: NormalizedValue::MIN,
        }
    }
}

/// Manages voice allocation for polyphonic playback.
pub struct VoiceAllocator {
    /// Configuration.
    config: AllocatorConfig,
    /// All available voices.
    voices: Vec<Voice>,
    /// Currently held notes (for legato/mono) - (note, velocity).
    held_notes: Vec<(MidiNote, Velocity)>,
    /// Current time counter (type-safe sample position).
    time: SamplePosition,
    /// Last played note (for glide).
    last_note: Option<MidiNote>,
}

impl VoiceAllocator {
    /// Create a new voice allocator.
    pub fn new(config: AllocatorConfig) -> Self {
        let num_voices = config.max_voices.as_usize();
        let voices = (0..num_voices)
            .map(|i| Voice::new(VoiceId::new(i as u32)))
            .collect();

        Self {
            config,
            voices,
            held_notes: Vec::with_capacity(32),
            time: SamplePosition::ZERO,
            last_note: None,
        }
    }

    /// Create with a template voice that will be cloned.
    pub fn with_template(config: AllocatorConfig, template: &Voice) -> Self {
        let voices = (0..config.max_voices.as_usize())
            .map(|i| {
                let mut v = template.clone_structure();
                // `set_id` re-seeds the per-voice script PRNG streams for the new
                // id; a bare `v.id = …` would leave every voice on the template's.
                v.set_id(VoiceId::new(i as u32));
                v
            })
            .collect();

        Self {
            config,
            voices,
            held_notes: Vec::with_capacity(32),
            time: SamplePosition::ZERO,
            last_note: None,
        }
    }

    /// Create with a ModuleGraph template.
    /// Each voice will receive a cloned copy of the graph.
    pub fn with_graph_template(
        config: AllocatorConfig,
        graph_template: &crate::graph::ModuleGraph,
    ) -> Self {
        let voices = (0..config.max_voices.as_usize())
            .map(|i| Voice::from_graph(VoiceId::new(i as u32), graph_template.clone_structure()))
            .collect();

        Self {
            config,
            voices,
            held_notes: Vec::with_capacity(32),
            time: SamplePosition::ZERO,
            last_note: None,
        }
    }

    /// Rebuild all voices from a new graph template.
    /// Used when the voice template changes (module added/removed/reconnected).
    pub fn rebuild_from_graph(&mut self, graph_template: &crate::graph::ModuleGraph) {
        for (i, voice) in self.voices.iter_mut().enumerate() {
            // VoiceState now contains all note data (note, velocity, start_time)
            let old_state = voice.state;
            let old_age = voice.age;

            // Replace with new graph
            *voice = Voice::from_graph(VoiceId::new(i as u32), graph_template.clone_structure());

            // Restore state if voice was active (preserves playing notes during template changes)
            if old_state.is_active() {
                voice.state = old_state;
                voice.age = old_age;
            }
        }
    }

    /// Get the allocator configuration.
    pub fn config(&self) -> &AllocatorConfig {
        &self.config
    }

    /// Set allocation mode.
    pub fn set_mode(&mut self, mode: AllocationMode) {
        self.config.mode = mode;
    }

    /// Set stealing strategy.
    pub fn set_stealing(&mut self, strategy: StealingStrategy) {
        self.config.stealing = strategy;
    }

    /// Set glide time.
    pub fn set_glide_time(&mut self, time: Seconds) {
        self.config.glide_time = time;
    }

    /// Set the unison detune spread (total cents across all `Unison`-mode voices).
    pub fn set_unison_detune(&mut self, detune: Cents) {
        self.config.unison_detune = detune;
    }

    /// Set the unison stereo spread (0..1) for `Unison`-mode voices.
    pub fn set_unison_spread(&mut self, spread: NormalizedValue) {
        self.config.unison_spread = spread;
    }

    /// Set the maximum polyphony (voice count).
    ///
    /// This updates the stored config **only** — it deliberately does *not*
    /// resize the live voice pool. A live resize would allocate/deallocate
    /// voices on the audio thread, which is forbidden in real-time code (see
    /// [`Self::resize`], which is only safe to call off the audio thread). The
    /// new count therefore takes effect the next time the instrument's voice
    /// graph is reconstructed — e.g. on project load, where
    /// [`AllocatorConfig::max_voices`] drives how many voices are created.
    pub fn set_max_voices(&mut self, count: VoiceCount) {
        self.config.max_voices = count;
    }

    /// Get number of active voices.
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.is_active()).count()
    }

    /// Get all voices (for processing).
    pub fn voices(&self) -> &[Voice] {
        &self.voices
    }

    /// Get mutable access to all voices.
    pub fn voices_mut(&mut self) -> &mut [Voice] {
        &mut self.voices
    }

    /// Handle note on event.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity) -> Option<VoiceId> {
        self.note_on_expr(note, velocity, NoteTrigger::default())
    }

    /// Handle note on event with per-note expression (legato/glide).
    ///
    /// A per-note `legato` flag overrides the configured allocation mode: the
    /// note glides onto the active voice without retriggering, regardless of
    /// whether the instrument is Poly/Mono/Legato/Unison. Per-note `glide`
    /// overrides the instrument's default glide time; absent it, behavior is
    /// identical to [`note_on`](Self::note_on).
    pub fn note_on_expr(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        trigger: NoteTrigger,
    ) -> Option<VoiceId> {
        if trigger.legato {
            // A legato NoteOn glides the active voice ONTO `note`; the pitch it
            // leaves is no longer sounding. The sequencer's cross-pitch legato
            // coalesce emits these with NO matching NoteOff (arp/tie figures), so
            // without dropping the departed pitch it accumulates in `held_notes`
            // and a later single NoteOff falls back to it instead of releasing —
            // a stuck +12 octave on a mono arp. Replace, don't stack.
            if let Some(active_note) = self
                .voices
                .iter()
                .find(|v| v.is_active())
                .and_then(|v| v.note())
            {
                self.held_notes.retain(|(n, _)| *n != active_note);
            }
            self.held_notes.retain(|(n, _)| *n != note);
            self.held_notes.push((note, velocity));
            // Per-note legato: force the no-retrigger glide path on the active
            // voice (or trigger normally if no voice is active yet).
            return self.allocate_mono(note, velocity, false, trigger);
        }

        self.held_notes.retain(|(n, _)| *n != note);
        self.held_notes.push((note, velocity));

        match self.config.mode {
            AllocationMode::Polyphonic => self.allocate_poly(note, velocity, trigger),
            AllocationMode::Mono => self.allocate_mono(note, velocity, true, trigger),
            AllocationMode::Legato => self.allocate_mono(note, velocity, false, trigger),
            AllocationMode::Unison => self.allocate_unison(note, velocity, trigger),
        }
    }

    /// Handle note off event.
    pub fn note_off(&mut self, note: MidiNote) {
        // Remove from held notes
        self.held_notes.retain(|(n, _)| *n != note);

        match self.config.mode {
            AllocationMode::Polyphonic => {
                // Release all voices playing this note
                for voice in &mut self.voices {
                    // Use pattern matching on VoiceState::Active to check note
                    if let VoiceState::Active {
                        note: voice_note, ..
                    } = voice.state
                        && voice_note == note
                    {
                        voice.note_off();
                    }
                }
            }
            AllocationMode::Mono | AllocationMode::Legato => {
                // Check if there's another held note to switch to
                if let Some(&(prev_note, prev_vel)) = self.get_priority_note() {
                    // Glide to previous note
                    if let Some(voice) = self.voices.iter_mut().find(|v| v.is_active()) {
                        voice.set_glide_time(self.config.glide_time);

                        // The fallback repitch must keep the voice's track
                        // tag: the still-held note belongs to the same
                        // channel, and the default trigger would strip it —
                        // dropping TrackParam::Pitch mid-phrase on exactly
                        // the mono/legato tracks that use it most.
                        let keep_track = NoteTrigger {
                            track: voice.track,
                            ..NoteTrigger::default()
                        };
                        if self.config.mode == AllocationMode::Legato {
                            // Legato: just glide pitch, don't retrigger
                            voice.glide_to_note_expr(prev_note, prev_vel, keep_track);
                        } else {
                            // Mono: retrigger with glide
                            voice.note_on_expr(prev_note, prev_vel, self.time, keep_track);
                        }
                    }
                } else {
                    // No more held notes, release
                    for voice in &mut self.voices {
                        if voice.is_active() {
                            voice.note_off();
                        }
                    }
                }
            }
            AllocationMode::Unison => {
                // Release all voices playing this note
                for voice in &mut self.voices {
                    if voice.note() == Some(note) {
                        voice.note_off();
                    }
                }
            }
        }
    }

    /// Release all voices.
    pub fn all_notes_off(&mut self) {
        self.held_notes.clear();
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.note_off();
            }
        }
    }

    /// Hard-reset every voice to idle, zeroing its DSP state (envelopes,
    /// filters, oscillator phase, per-voice delay lines) **instantly** — unlike
    /// [`all_notes_off`](Self::all_notes_off), which only triggers the release
    /// phase. Used for tail-proof isolation between offline renders.
    pub fn reset_voices(&mut self) {
        self.held_notes.clear();
        for voice in &mut self.voices {
            voice.reset();
        }
    }

    /// Kill all voices immediately.
    pub fn panic(&mut self) {
        self.held_notes.clear();
        for voice in &mut self.voices {
            voice.reset();
        }
    }

    /// Advance time (call once per audio block).
    pub fn advance_time(&mut self, samples: SampleCount) {
        self.time = self.time + samples;
    }

    /// Resize the voice pool to the specified count.
    ///
    /// If shrinking, excess voices are released and removed.
    /// If growing, new voices are created using the first voice as template
    /// (or empty voices if no template exists).
    ///
    /// # Arguments
    /// * `new_count` - New number of voices (will be clamped to valid range)
    pub fn resize(&mut self, new_count: VoiceCount) {
        let new_size = new_count.as_usize();
        let current_size = self.voices.len();

        if new_size == current_size || new_size == 0 {
            return;
        }

        if new_size < current_size {
            // Release and remove excess voices
            for voice in &mut self.voices[new_size..] {
                voice.reset();
            }
            self.voices.truncate(new_size);
        } else {
            // Add new voices - clone structure from first voice if available
            for i in current_size..new_size {
                let new_voice = if let Some(template) = self.voices.first() {
                    let mut v = template.clone_structure();
                    v.set_id(VoiceId::new(i as u32));
                    v
                } else {
                    Voice::new(VoiceId::new(i as u32))
                };
                self.voices.push(new_voice);
            }
        }

        self.config.max_voices = new_count;
    }

    /// Resize the voice pool with a specific graph template.
    ///
    /// Like `resize()` but uses a provided `ModuleGraph` as template for new voices.
    pub fn resize_with_graph(
        &mut self,
        new_count: VoiceCount,
        graph_template: &crate::graph::ModuleGraph,
    ) {
        let new_size = new_count.as_usize();
        let current_size = self.voices.len();

        if new_size == current_size || new_size == 0 {
            return;
        }

        if new_size < current_size {
            // Release and remove excess voices
            for voice in &mut self.voices[new_size..] {
                voice.reset();
            }
            self.voices.truncate(new_size);
        } else {
            // Add new voices from graph template
            for i in current_size..new_size {
                let new_voice =
                    Voice::from_graph(VoiceId::new(i as u32), graph_template.clone_structure());
                self.voices.push(new_voice);
            }
        }

        self.config.max_voices = new_count;
    }

    /// Allocate voice for polyphonic mode.
    fn allocate_poly(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        trigger: NoteTrigger,
    ) -> Option<VoiceId> {
        // First, try to find an idle voice
        if let Some(voice) = self.voices.iter_mut().find(|v| v.is_available()) {
            voice.note_on_expr(note, velocity, self.time, trigger);
            self.last_note = Some(note);
            return Some(voice.id);
        }

        // Try to reuse a voice playing the same note
        if self.config.stealing == StealingStrategy::SameNote
            && let Some(voice) = self.voices.iter_mut().find(|v| v.note() == Some(note))
        {
            voice.note_on_expr(note, velocity, self.time, trigger);
            return Some(voice.id);
        }

        // Need to steal a voice
        if self.config.stealing == StealingStrategy::None {
            return None;
        }

        let voice_idx = self.find_voice_to_steal()?;
        let voice = &mut self.voices[voice_idx];

        // Start fade-out with pending note; instrument.rs will trigger
        // note_on once the fade completes. The trigger rides along so the
        // stolen voice keeps its track tag + per-note expression.
        voice.steal_for(note, velocity, self.time, trigger);
        self.last_note = Some(note);
        Some(voice.id)
    }

    /// Allocate voice for mono/legato mode.
    fn allocate_mono(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        retrigger: bool,
        trigger: NoteTrigger,
    ) -> Option<VoiceId> {
        // Find active voice index or use first
        let voice_idx = self.voices.iter().position(|v| v.is_active()).unwrap_or(0);

        if voice_idx >= self.voices.len() {
            return None;
        }

        let voice = &mut self.voices[voice_idx];

        // Set glide time on the voice (per-note glide in `trigger` overrides this).
        voice.set_glide_time(self.config.glide_time);

        if retrigger || !voice.is_active() {
            // Mono mode (or first note): retrigger envelope with glide.
            voice.note_on_expr(note, velocity, self.time, trigger);
        } else {
            // Legato mode: glide to new pitch without retriggering. The shaped
            // velocity is still written into the voice state so accent/ghost
            // apply on legato notes too.
            voice.glide_to_note_expr(note, velocity, trigger);
        }

        self.last_note = Some(note);
        Some(voice.id)
    }

    /// Allocate all voices for unison mode.
    fn allocate_unison(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        trigger: NoteTrigger,
    ) -> Option<VoiceId> {
        let num_voices = self.voices.len();
        let spread = self.config.unison_spread.as_f32();
        for i in 0..num_voices {
            let detune = unison_voice_detune(i, num_voices, self.config.unison_detune);
            // note_on_expr resets the spread gains, so set them after it.
            self.voices[i].note_on_expr(note, velocity, self.time, trigger);
            self.voices[i].set_oscillator_detune(detune);
            self.voices[i].unison_pan_gains = unison_spread_gains(i, num_voices, spread);
        }

        self.last_note = Some(note);
        Some(VoiceId::new(0))
    }

    /// Find the best voice to steal based on strategy.
    fn find_voice_to_steal(&self) -> Option<usize> {
        match self.config.stealing {
            StealingStrategy::None => None,

            StealingStrategy::Oldest => self
                .voices
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| v.age)
                .map(|(i, _)| i),

            StealingStrategy::Quietest => {
                // For now, use oldest releasing voice, then oldest active
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| matches!(v.state, VoiceState::Releasing { .. }))
                    .max_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
                    .or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .max_by_key(|(_, v)| v.age)
                            .map(|(i, _)| i)
                    })
            }

            StealingStrategy::LowestPriority => {
                // Releasing voices first, then oldest
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| {
                        let state_priority = match v.state {
                            VoiceState::Releasing { .. } => 0,
                            VoiceState::Stealing { .. } => 1,
                            VoiceState::Active { .. } => 2,
                            VoiceState::Idle => 3,
                        };
                        (state_priority, std::cmp::Reverse(v.age))
                    })
                    .map(|(i, _)| i)
            }

            StealingStrategy::SameNote => {
                // Handled separately in allocate_poly
                self.find_voice_to_steal_with_strategy(StealingStrategy::Oldest)
            }
        }
    }

    fn find_voice_to_steal_with_strategy(&self, strategy: StealingStrategy) -> Option<usize> {
        match strategy {
            StealingStrategy::Oldest => self
                .voices
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| v.age)
                .map(|(i, _)| i),
            _ => None,
        }
    }

    /// Get the priority note based on configuration.
    fn get_priority_note(&self) -> Option<&(MidiNote, Velocity)> {
        if self.held_notes.is_empty() {
            return None;
        }

        match self.config.priority {
            NotePriority::Last => self.held_notes.last(),
            NotePriority::High => self.held_notes.iter().max_by_key(|(n, _)| *n),
            NotePriority::Low => self.held_notes.iter().min_by_key(|(n, _)| *n),
        }
    }
}

/// Detune (in cents) for unison voice `index` of `num_voices`, given the total
/// peak-to-peak `spread`. Voices are placed symmetrically around the centre pitch
/// so the lowest and highest differ by exactly `spread`, independent of the voice
/// count; a single voice gets no detune.
fn unison_voice_detune(index: usize, num_voices: usize, spread: Cents) -> Cents {
    if num_voices <= 1 {
        return Cents::new(0.0);
    }
    let per_voice = spread / (num_voices as f32 - 1.0);
    per_voice * (index as f32 - (num_voices as f32 - 1.0) / 2.0)
}

/// Per-voice `(left, right)` gains for unison voice `index` of `num_voices`, given
/// a 0..1 `spread`. Blends from unity (`spread = 0` → `(1, 1)`) toward the
/// constant-power pan at full spread, so existing detune-only Unison patches stay
/// loudness-identical until spread is dialled up; a single voice stays centred
/// (unity). Uses the canonical `Gain::from_pan` law (matches instrument/track/
/// return pan). Because `Gain::from_pan` returns gains in `[0, 1]`, the blended
/// value never exceeds 1.0 — a spread sweep cannot clip.
fn unison_spread_gains(index: usize, num_voices: usize, spread: f32) -> (f32, f32) {
    if num_voices <= 1 || spread <= 0.0 {
        return (1.0, 1.0);
    }
    let t = index as f32 / (num_voices as f32 - 1.0) * 2.0 - 1.0; // [-1, 1]
    let (gl, gr) = Gain::from_pan(BipolarValue::new(t * spread));
    let (gl, gr) = (gl.as_f32(), gr.as_f32());
    (1.0 + spread * (gl - 1.0), 1.0 + spread * (gr - 1.0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn unison_detune_spread_equals_total_for_any_count() {
        let spread = Cents::new(10.0);
        for n in [2_usize, 4, 8] {
            let lo = unison_voice_detune(0, n, spread).as_f32();
            let hi = unison_voice_detune(n - 1, n, spread).as_f32();
            // Lowest..highest spans exactly the configured spread, symmetric
            // around the centre — independent of the voice count.
            assert!(
                (hi - lo - 10.0).abs() < 1e-4,
                "n={n}: span {} != 10",
                hi - lo
            );
            assert!((lo + hi).abs() < 1e-4, "n={n}: not symmetric ({lo}, {hi})");
        }
        // A single voice gets no detune (no division by zero).
        assert!(unison_voice_detune(0, 1, spread).as_f32().abs() < 1e-6);
    }

    #[test]
    fn unison_spread_gains_edge_cases() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

        // spread = 0 → unity for every voice (preserves detune-only Unison loudness).
        for n in [1_usize, 2, 3, 8] {
            for i in 0..n {
                let (l, r) = unison_spread_gains(i, n, 0.0);
                assert!(close(l, 1.0) && close(r, 1.0), "spread 0 must be unity");
            }
        }
        // n <= 1 → unity regardless of spread (no division by zero).
        let (l, r) = unison_spread_gains(0, 1, 1.0);
        assert!(close(l, 1.0) && close(r, 1.0), "single voice must be unity");

        // Full spread, 4 voices: edges biased outward and the pair mirrors.
        let (l0, r0) = unison_spread_gains(0, 4, 1.0);
        let (ln, rn) = unison_spread_gains(3, 4, 1.0);
        assert!(l0 > r0 && rn > ln, "edge voices biased outward");
        assert!(close(l0, rn) && close(r0, ln), "edge pair must mirror");
        assert!(
            close(l0, 1.0) && close(r0, 0.0),
            "voice 0 ≈ (1,0) at full spread"
        );

        // Odd-count centre voice → equal L/R ≈ 0.7071 (−3 dB) at full spread.
        let (lc, rc) = unison_spread_gains(1, 3, 1.0);
        assert!(
            close(lc, rc) && close(lc, std::f32::consts::FRAC_1_SQRT_2),
            "centre voice ≈ 0.7071 at full spread"
        );
    }

    #[test]
    fn test_allocator_creation() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            ..Default::default()
        };
        let allocator = VoiceAllocator::new(config);
        assert_eq!(allocator.voices.len(), 4);
        assert_eq!(allocator.active_voice_count(), 0);
    }

    #[test]
    fn test_poly_allocation() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        // Allocate 4 notes
        for note in 60..64 {
            let id = allocator.note_on(MidiNote::new(note), Velocity::new(0.8));
            assert!(id.is_some());
        }

        assert_eq!(allocator.active_voice_count(), 4);

        // Release one
        allocator.note_off(MidiNote::C4);

        // Should have one releasing
        let releasing = allocator
            .voices
            .iter()
            .filter(|v| matches!(v.state, VoiceState::Releasing { .. }))
            .count();
        assert_eq!(releasing, 1);
    }

    #[test]
    fn test_mono_allocation() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Mono,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(MidiNote::C4, Velocity::new(0.8));
        allocator.note_on(MidiNote::new(64), Velocity::new(0.8));

        // Only one voice should be active
        assert_eq!(allocator.active_voice_count(), 1);

        // And it should be playing the latest note
        let active = allocator.voices.iter().find(|v| v.is_active()).unwrap();
        assert_eq!(active.note(), Some(MidiNote::new(64)));
    }

    #[test]
    fn mono_note_off_fallback_preserves_track_tag() {
        use crate::voice::NoteTrigger;
        use synth_sequencer::TrackId;

        // A mono track carrying TrackParam::Pitch relies on the still-held
        // note keeping its track tag when the newer note releases — else the
        // fallback would drop track pitch mid-phrase.
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Mono,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);
        let track = Some(TrackId(7));
        let tagged = NoteTrigger {
            track,
            ..NoteTrigger::default()
        };

        allocator.note_on_expr(MidiNote::C4, Velocity::new(0.8), tagged);
        allocator.note_on_expr(MidiNote::new(64), Velocity::new(0.8), tagged);
        // Release the newer note → falls back to the still-held C4.
        allocator.note_off(MidiNote::new(64));

        let active = allocator.voices.iter().find(|v| v.is_active()).unwrap();
        assert_eq!(active.note(), Some(MidiNote::C4));
        assert_eq!(active.track, track, "fallback must keep the track tag");
    }

    #[test]
    fn test_per_note_legato_overrides_poly_no_retrigger() {
        use crate::voice::NoteTrigger;

        // Polyphonic instrument, but a per-note legato flag must force the
        // no-retrigger glide path onto the active voice.
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(MidiNote::C4, Velocity::new(0.8));
        allocator.advance_time(SampleCount::new(100));

        let legato = NoteTrigger {
            legato: true,
            glide: None,
            vibrato: None,
            track: None,
        };
        allocator.note_on_expr(MidiNote::new(64), Velocity::new(0.8), legato);

        // Still one voice (no new allocation), now playing E4, and its
        // start_time is unchanged → the envelope was NOT retriggered.
        assert_eq!(allocator.active_voice_count(), 1);
        let active = allocator.voices.iter().find(|v| v.is_active()).unwrap();
        assert_eq!(active.note(), Some(MidiNote::new(64)));
        assert_eq!(active.state.start_time(), Some(SamplePosition::ZERO));
    }

    #[test]
    fn mono_legato_arp_figure_fully_releases_after_final_note_off() {
        use crate::voice::NoteTrigger;

        // Reproduces the SID octave-stab "leftover note": a Custom legato arp
        // (offsets [0,12]) on a MONO instrument. The sequencer's cross-pitch
        // legato coalesce emits one legato NoteOn per step with NO matching
        // NoteOff, then a single NoteOff for the figure's final pitch. Each step
        // alternates D5 (74) and D6 (86, the +12 octave). Without the fix,
        // `held_notes` accumulates BOTH pitches, so the final NoteOff(D5) leaves
        // D6 "held" and mono falls back to it — the octave rings on forever.
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Mono,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);
        let legato = NoteTrigger {
            legato: true,
            glide: None,
            vibrato: None,
            track: None,
        };
        let (d5, d6) = (MidiNote::new(74), MidiNote::new(86));
        // D5,D6,D5,D6,D5 — the figure ends on D5, so the lone NoteOff is D5.
        for &n in &[d5, d6, d5, d6, d5] {
            allocator.note_on_expr(n, Velocity::new(0.8), legato);
            allocator.advance_time(SampleCount::new(50));
        }
        allocator.note_off(d5);

        // After the figure's single NoteOff no voice may still be GATED (Active):
        // the bug re-gated the voice on the "still-held" D6 (a fresh attack that
        // rings forever); the fix leaves the voice Releasing, so its envelope
        // fades out. (`is_active()` is "not Idle", so it also counts a Releasing
        // voice — hence assert specifically on the gated `Active` state.)
        let still_gated = allocator
            .voices
            .iter()
            .filter(|v| matches!(v.state, VoiceState::Active { .. }))
            .collect::<Vec<_>>();
        assert!(
            still_gated.is_empty(),
            "the +12 octave must not linger re-gated after the figure's NoteOff; \
             still-gated voices: {:?}",
            still_gated.iter().map(|v| v.note()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_per_note_glide_seeds_explicit_source() {
        use crate::voice::{GlideSpec, NoteTrigger};
        use synth_core::Semitones;

        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        // A5 (880 Hz) with a glide starting one octave below (A4, 440 Hz).
        let trigger = NoteTrigger {
            legato: false,
            glide: Some(GlideSpec {
                from_offset: Semitones::new(-12.0),
                time: Seconds::new(0.1),
                stepped: false,
            }),
            vibrato: None,
            track: None,
        };
        allocator.note_on_expr(MidiNote::new(81), Velocity::new(0.8), trigger);

        let voice = allocator.voices.iter().find(|v| v.is_active()).unwrap();
        assert!(voice.glide.active);
        assert!((voice.glide.to_freq.as_f32() - 880.0).abs() < 1.0);
        assert!((voice.glide.from_freq.as_f32() - 440.0).abs() < 1.0);
    }

    #[test]
    fn test_voice_stealing() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::DUAL,
            mode: AllocationMode::Polyphonic,
            stealing: StealingStrategy::Oldest,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        // Fill all voices
        allocator.note_on(MidiNote::C4, Velocity::new(0.8));
        allocator.advance_time(SampleCount::new(100));
        allocator.note_on(MidiNote::new(64), Velocity::new(0.8));
        allocator.advance_time(SampleCount::new(100));

        // Third note should steal oldest
        let id = allocator.note_on(MidiNote::new(67), Velocity::new(0.8));
        assert!(id.is_some());

        // Voice 0 (oldest) should now be playing note 67
        // (or be in stealing state)
    }

    #[test]
    fn set_max_voices_updates_config_without_resizing_live_pool() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);
        assert_eq!(allocator.voices.len(), 4);

        // The setter must update the stored config but NOT resize the live pool
        // (resizing would allocate/deallocate on the audio thread). The new
        // count only takes effect on the next voice-graph reconstruction.
        allocator.set_max_voices(VoiceCount::SIXTEEN);
        assert_eq!(
            allocator.config().max_voices,
            VoiceCount::SIXTEEN,
            "config max_voices should reflect the new value"
        );
        assert_eq!(
            allocator.voices.len(),
            4,
            "live voice pool must stay at its original size (no live resize)"
        );
    }

    #[test]
    fn allocation_mode_and_stealing_strategy_string_round_trip() {
        for mode in [
            AllocationMode::Polyphonic,
            AllocationMode::Mono,
            AllocationMode::Legato,
            AllocationMode::Unison,
        ] {
            assert_eq!(mode.to_string().parse::<AllocationMode>(), Ok(mode));
        }
        for strat in [
            StealingStrategy::None,
            StealingStrategy::Oldest,
            StealingStrategy::Quietest,
            StealingStrategy::LowestPriority,
            StealingStrategy::SameNote,
        ] {
            assert_eq!(strat.to_string().parse::<StealingStrategy>(), Ok(strat));
        }
        assert!("Duophonic".parse::<AllocationMode>().is_err());
        assert!("Bogus".parse::<StealingStrategy>().is_err());
    }
}
