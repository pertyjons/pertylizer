//! Voice allocator - manages polyphonic voice allocation.
//!
//! Features:
//! - Multiple allocation modes (poly, mono, legato)
//! - Voice stealing strategies
//! - Glide/portamento support

use crate::engine::voice::{Voice, VoiceState};
use crate::types::{Cents, NormalizedValue, Seconds};

/// Voice allocation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationMode {
    /// Unlimited polyphony (up to max voices).
    Polyphonic,
    /// Monophonic with retriggering.
    Mono,
    /// Monophonic legato (no retrigger on overlapping notes).
    Legato,
    /// Unison - all voices play same note.
    Unison,
}

/// Strategy for stealing voices when all are busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StealingStrategy {
    /// Don't steal - ignore new notes when full.
    None,
    /// Steal the oldest active voice.
    Oldest,
    /// Steal the quietest voice (based on envelope level).
    Quietest,
    /// Steal the lowest priority voice.
    LowestPriority,
    /// Steal the same note if playing.
    SameNote,
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
    /// Maximum number of voices.
    pub max_voices: usize,
    /// Allocation mode.
    pub mode: AllocationMode,
    /// Voice stealing strategy.
    pub stealing: StealingStrategy,
    /// Note priority (for mono modes).
    pub priority: NotePriority,
    /// Glide time in seconds.
    pub glide_time: Seconds,
    /// Unison detune amount (type-safe Cents).
    pub unison_detune: Cents,
}

impl Default for AllocatorConfig {
    fn default() -> Self {
        Self {
            max_voices: 8,
            mode: AllocationMode::Polyphonic,
            stealing: StealingStrategy::Oldest,
            priority: NotePriority::Last,
            glide_time: Seconds::ZERO,
            unison_detune: Cents::new(10.0),
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
    held_notes: Vec<(u8, NormalizedValue)>,
    /// Current time counter.
    time: u64,
    /// Last played note (for glide).
    last_note: Option<u8>,
}

impl VoiceAllocator {
    /// Create a new voice allocator.
    pub fn new(config: AllocatorConfig) -> Self {
        let voices = (0..config.max_voices)
            .map(|i| Voice::new(i as u32))
            .collect();

        Self {
            config,
            voices,
            held_notes: Vec::new(),
            time: 0,
            last_note: None,
        }
    }

    /// Create with a template voice that will be cloned.
    pub fn with_template(config: AllocatorConfig, template: &Voice) -> Self {
        let voices = (0..config.max_voices)
            .map(|i| {
                let mut v = template.clone_structure();
                v.id = i as u32;
                v
            })
            .collect();

        Self {
            config,
            voices,
            held_notes: Vec::new(),
            time: 0,
            last_note: None,
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
    /// Accepts f32 velocity for backward compatibility (converted internally).
    pub fn note_on(&mut self, note: u8, velocity: f32) -> Option<u32> {
        // Track held notes with type-safe velocity
        let velocity_norm = NormalizedValue::new(velocity);
        self.held_notes.retain(|(n, _)| *n != note);
        self.held_notes.push((note, velocity_norm));

        match self.config.mode {
            AllocationMode::Polyphonic => self.allocate_poly(note, velocity_norm),
            AllocationMode::Mono => self.allocate_mono(note, velocity_norm, true),
            AllocationMode::Legato => self.allocate_mono(note, velocity_norm, false),
            AllocationMode::Unison => self.allocate_unison(note, velocity_norm),
        }
    }

    /// Handle note off event.
    pub fn note_off(&mut self, note: u8) {
        // Remove from held notes
        self.held_notes.retain(|(n, _)| *n != note);

        match self.config.mode {
            AllocationMode::Polyphonic => {
                // Release all voices playing this note
                for voice in &mut self.voices {
                    if voice.note == note && voice.state == VoiceState::Active {
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
                        
                        if self.config.mode == AllocationMode::Legato {
                            // Legato: just glide pitch, don't retrigger
                            voice.glide_to_note(prev_note);
                        } else {
                            // Mono: retrigger with glide
                            voice.note_on(prev_note, prev_vel, self.time);
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
                // Release all voices
                for voice in &mut self.voices {
                    if voice.note == note {
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

    /// Kill all voices immediately.
    pub fn panic(&mut self) {
        self.held_notes.clear();
        for voice in &mut self.voices {
            voice.reset();
        }
    }

    /// Advance time (call once per audio block).
    pub fn advance_time(&mut self, samples: u64) {
        self.time += samples;
    }

    /// Allocate voice for polyphonic mode.
    fn allocate_poly(&mut self, note: u8, velocity: NormalizedValue) -> Option<u32> {
        // First, try to find an idle voice
        if let Some(voice) = self.voices.iter_mut().find(|v| v.is_available()) {
            voice.note_on(note, velocity, self.time);
            self.last_note = Some(note);
            return Some(voice.id);
        }

        // Try to reuse a voice playing the same note
        if self.config.stealing == StealingStrategy::SameNote {
            if let Some(voice) = self.voices.iter_mut().find(|v| v.note == note) {
                voice.note_on(note, velocity, self.time);
                return Some(voice.id);
            }
        }

        // Need to steal a voice
        if self.config.stealing == StealingStrategy::None {
            return None;
        }

        let voice_idx = self.find_voice_to_steal()?;
        let voice = &mut self.voices[voice_idx];
        voice.steal();

        // We need to wait for the steal to complete before triggering
        // For now, just trigger immediately (could be improved)
        voice.note_on(note, velocity, self.time);
        self.last_note = Some(note);
        Some(voice.id)
    }

    /// Allocate voice for mono/legato mode.
    fn allocate_mono(&mut self, note: u8, velocity: NormalizedValue, retrigger: bool) -> Option<u32> {
        // Find active voice index or use first
        let voice_idx = self.voices.iter()
            .position(|v| v.is_active())
            .unwrap_or(0);

        if voice_idx >= self.voices.len() {
            return None;
        }

        let voice = &mut self.voices[voice_idx];

        // Set glide time on the voice
        voice.set_glide_time(self.config.glide_time);

        if retrigger || !voice.is_active() {
            // Mono mode: retrigger envelope with glide
            voice.note_on(note, velocity, self.time);
        } else {
            // Legato mode: glide to new pitch without retriggering
            voice.glide_to_note(note);
        }

        self.last_note = Some(note);
        Some(voice.id)
    }

    /// Allocate all voices for unison mode.
    fn allocate_unison(&mut self, note: u8, velocity: NormalizedValue) -> Option<u32> {
        let num_voices = self.voices.len();
        let detune_per_voice = self.config.unison_detune / num_voices as f32;

        for i in 0..num_voices {
            // Apply detune spread: voices spread evenly around center pitch
            // e.g. with 4 voices and 10 cents total: -7.5, -2.5, +2.5, +7.5
            let detune = detune_per_voice * (i as f32 - (num_voices as f32 - 1.0) / 2.0);
            
            self.voices[i].note_on(note, velocity, self.time);
            
            // Apply detune to oscillators
            self.voices[i].set_oscillator_detune(detune);
        }

        self.last_note = Some(note);
        Some(0)
    }

    /// Find the best voice to steal based on strategy.
    fn find_voice_to_steal(&self) -> Option<usize> {
        match self.config.stealing {
            StealingStrategy::None => None,
            
            StealingStrategy::Oldest => {
                self.voices
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
            }
            
            StealingStrategy::Quietest => {
                // For now, use oldest releasing voice, then oldest active
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.state == VoiceState::Releasing)
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
                            VoiceState::Releasing => 0,
                            VoiceState::Stealing => 1,
                            VoiceState::Active => 2,
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
            StealingStrategy::Oldest => {
                self.voices
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
            }
            _ => None,
        }
    }

    /// Get the priority note based on configuration.
    fn get_priority_note(&self) -> Option<&(u8, NormalizedValue)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocator_creation() {
        let config = AllocatorConfig {
            max_voices: 4,
            ..Default::default()
        };
        let allocator = VoiceAllocator::new(config);
        assert_eq!(allocator.voices.len(), 4);
        assert_eq!(allocator.active_voice_count(), 0);
    }

    #[test]
    fn test_poly_allocation() {
        let config = AllocatorConfig {
            max_voices: 4,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        // Allocate 4 notes
        for note in 60..64 {
            let id = allocator.note_on(note, 0.8);
            assert!(id.is_some());
        }

        assert_eq!(allocator.active_voice_count(), 4);

        // Release one
        allocator.note_off(60);
        
        // Should have one releasing
        let releasing = allocator.voices.iter()
            .filter(|v| v.state == VoiceState::Releasing)
            .count();
        assert_eq!(releasing, 1);
    }

    #[test]
    fn test_mono_allocation() {
        let config = AllocatorConfig {
            max_voices: 4,
            mode: AllocationMode::Mono,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);

        // Only one voice should be active
        assert_eq!(allocator.active_voice_count(), 1);
        
        // And it should be playing the latest note
        let active = allocator.voices.iter().find(|v| v.is_active()).unwrap();
        assert_eq!(active.note, 64);
    }

    #[test]
    fn test_voice_stealing() {
        let config = AllocatorConfig {
            max_voices: 2,
            mode: AllocationMode::Polyphonic,
            stealing: StealingStrategy::Oldest,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        // Fill all voices
        allocator.note_on(60, 0.8);
        allocator.advance_time(100);
        allocator.note_on(64, 0.8);
        allocator.advance_time(100);

        // Third note should steal oldest
        let id = allocator.note_on(67, 0.8);
        assert!(id.is_some());

        // Voice 0 (oldest) should now be playing note 67
        // (or be in stealing state)
    }
}
