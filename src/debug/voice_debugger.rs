//! Voice debugger for analyzing voice allocation behavior.
//!
//! Tracks voice allocation, state changes, and helps debug polyphony issues.

use std::collections::HashMap;

use crate::engine::VoiceState;
use crate::sequencer::Tick;
use crate::types::MidiNote;

/// Reason why a voice was released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// Explicit NoteOff event.
    NoteOff,
    /// Note duration ended.
    NoteDurationEnd,
    /// Replaced by a new note on the same track (mono-per-track).
    MonoReplacement,
    /// All notes off command.
    AllNotesOff,
    /// Panic/emergency stop.
    Panic,
    /// Voice was stolen for a new note.
    VoiceStolen,
}

/// An event in the voice allocation history.
#[derive(Debug, Clone)]
pub enum VoiceEvent {
    /// A voice was allocated for a note.
    Allocated {
        /// Voice index.
        voice_id: u32,
        /// MIDI note being played.
        note: MidiNote,
        /// Velocity (0.0 - 1.0).
        velocity: f32,
        /// Sample position when allocated.
        sample_position: u64,
        /// Tick position (if from sequencer).
        tick: Option<Tick>,
    },
    /// A voice was released.
    Released {
        /// Voice index.
        voice_id: u32,
        /// Note that was released.
        note: MidiNote,
        /// Sample position when released.
        sample_position: u64,
        /// Why the voice was released.
        reason: ReleaseReason,
    },
    /// A voice was stolen to play a new note.
    Stolen {
        /// Voice index.
        voice_id: u32,
        /// Note that was playing.
        old_note: MidiNote,
        /// New note that is taking over.
        new_note: MidiNote,
        /// Sample position when stolen.
        sample_position: u64,
    },
    /// Voice state changed (Active, Releasing, Idle, Stealing).
    StateChanged {
        /// Voice index.
        voice_id: u32,
        /// Previous state (as string for serialization).
        from_state: String,
        /// New state (as string for serialization).
        to_state: String,
        /// Sample position when state changed.
        sample_position: u64,
    },
    /// Voice was retriggered (same voice, new note without release).
    Retriggered {
        /// Voice index.
        voice_id: u32,
        /// Previous note.
        old_note: MidiNote,
        /// New note.
        new_note: MidiNote,
        /// New velocity.
        velocity: f32,
        /// Sample position when retriggered.
        sample_position: u64,
    },
}

impl VoiceEvent {
    /// Get the voice ID for this event.
    pub fn voice_id(&self) -> u32 {
        match self {
            Self::Allocated { voice_id, .. } => *voice_id,
            Self::Released { voice_id, .. } => *voice_id,
            Self::Stolen { voice_id, .. } => *voice_id,
            Self::StateChanged { voice_id, .. } => *voice_id,
            Self::Retriggered { voice_id, .. } => *voice_id,
        }
    }

    /// Get the sample position for this event.
    pub fn sample_position(&self) -> u64 {
        match self {
            Self::Allocated {
                sample_position, ..
            } => *sample_position,
            Self::Released {
                sample_position, ..
            } => *sample_position,
            Self::Stolen {
                sample_position, ..
            } => *sample_position,
            Self::StateChanged {
                sample_position, ..
            } => *sample_position,
            Self::Retriggered {
                sample_position, ..
            } => *sample_position,
        }
    }
}

/// Information about a voice at a point in time.
#[derive(Debug, Clone)]
pub struct VoiceInfo {
    /// Voice index.
    pub id: u32,
    /// Current state.
    pub state: VoiceStateInfo,
    /// Currently playing note (if any).
    pub note: Option<MidiNote>,
    /// Envelope stage (if known).
    pub envelope_stage: Option<EnvelopeStage>,
    /// Current envelope level (0.0-1.0).
    pub envelope_level: Option<f32>,
    /// Number of samples since voice was allocated.
    pub age_samples: u64,
}

/// Simplified voice state for debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStateInfo {
    /// Voice is not playing.
    Idle,
    /// Voice is actively playing.
    Active,
    /// Voice is in release phase.
    Releasing,
    /// Voice is being stolen (fading out).
    Stealing,
}

impl From<&VoiceState> for VoiceStateInfo {
    fn from(state: &VoiceState) -> Self {
        match state {
            VoiceState::Idle => Self::Idle,
            VoiceState::Active { .. } => Self::Active,
            VoiceState::Releasing { .. } => Self::Releasing,
            VoiceState::Stealing { .. } => Self::Stealing,
        }
    }
}

impl std::fmt::Display for VoiceStateInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Active => write!(f, "Active"),
            Self::Releasing => write!(f, "Releasing"),
            Self::Stealing => write!(f, "Stealing"),
        }
    }
}

/// Envelope stage for debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeStage {
    /// Idle (not triggered).
    Idle,
    /// Attack phase.
    Attack,
    /// Decay phase.
    Decay,
    /// Sustain phase.
    Sustain,
    /// Release phase.
    Release,
}

impl std::fmt::Display for EnvelopeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Attack => write!(f, "Attack"),
            Self::Decay => write!(f, "Decay"),
            Self::Sustain => write!(f, "Sustain"),
            Self::Release => write!(f, "Release"),
        }
    }
}

/// Snapshot of all voices at a point in time.
#[derive(Debug, Clone)]
pub struct VoiceSnapshot {
    /// Sample position when this snapshot was taken.
    pub sample_position: u64,
    /// State of each voice.
    pub voices: Vec<VoiceInfo>,
}

impl VoiceSnapshot {
    /// Get the number of active voices (non-idle).
    pub fn active_count(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != VoiceStateInfo::Idle)
            .count()
    }

    /// Get all active voices.
    pub fn active_voices(&self) -> Vec<&VoiceInfo> {
        self.voices
            .iter()
            .filter(|v| v.state != VoiceStateInfo::Idle)
            .collect()
    }
}

/// Debugger for voice allocation.
///
/// Tracks voice allocation events and provides analysis tools.
#[derive(Debug, Default)]
pub struct VoiceDebugger {
    /// Event history.
    history: Vec<VoiceEvent>,
    /// Snapshots taken at specific points.
    snapshots: Vec<VoiceSnapshot>,
    /// Current sample position.
    sample_position: u64,
    /// Map of voice_id to currently playing note.
    voice_notes: HashMap<u32, MidiNote>,
    /// Map of voice_id to allocation time.
    voice_start_times: HashMap<u32, u64>,
}

impl VoiceDebugger {
    /// Create a new voice debugger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a voice allocation event.
    pub fn record_allocation(
        &mut self,
        voice_id: u32,
        note: MidiNote,
        velocity: f32,
        tick: Option<Tick>,
    ) {
        self.voice_notes.insert(voice_id, note);
        self.voice_start_times
            .insert(voice_id, self.sample_position);

        self.history.push(VoiceEvent::Allocated {
            voice_id,
            note,
            velocity,
            sample_position: self.sample_position,
            tick,
        });
    }

    /// Record a voice release event.
    pub fn record_release(&mut self, voice_id: u32, reason: ReleaseReason) {
        if let Some(note) = self.voice_notes.remove(&voice_id) {
            self.history.push(VoiceEvent::Released {
                voice_id,
                note,
                sample_position: self.sample_position,
                reason,
            });
        }
        self.voice_start_times.remove(&voice_id);
    }

    /// Record a voice stealing event.
    pub fn record_steal(&mut self, voice_id: u32, new_note: MidiNote) {
        if let Some(old_note) = self.voice_notes.get(&voice_id).copied() {
            self.history.push(VoiceEvent::Stolen {
                voice_id,
                old_note,
                new_note,
                sample_position: self.sample_position,
            });
        }
        self.voice_notes.insert(voice_id, new_note);
        self.voice_start_times
            .insert(voice_id, self.sample_position);
    }

    /// Record a state change.
    pub fn record_state_change(&mut self, voice_id: u32, from: &VoiceState, to: &VoiceState) {
        let from_state = format!("{:?}", VoiceStateInfo::from(from));
        let to_state = format!("{:?}", VoiceStateInfo::from(to));

        self.history.push(VoiceEvent::StateChanged {
            voice_id,
            from_state,
            to_state,
            sample_position: self.sample_position,
        });
    }

    /// Record a retrigger event (same voice, new note).
    pub fn record_retrigger(
        &mut self,
        voice_id: u32,
        old_note: MidiNote,
        new_note: MidiNote,
        velocity: f32,
    ) {
        self.voice_notes.insert(voice_id, new_note);
        self.voice_start_times
            .insert(voice_id, self.sample_position);

        self.history.push(VoiceEvent::Retriggered {
            voice_id,
            old_note,
            new_note,
            velocity,
            sample_position: self.sample_position,
        });
    }

    /// Advance the sample position.
    pub fn advance(&mut self, samples: u64) {
        self.sample_position += samples;
    }

    /// Get the current sample position.
    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// Take a snapshot of the current voice state.
    pub fn take_snapshot(&mut self, voices: &[VoiceInfo]) {
        self.snapshots.push(VoiceSnapshot {
            sample_position: self.sample_position,
            voices: voices.to_vec(),
        });
    }

    /// Get all recorded events.
    pub fn events(&self) -> &[VoiceEvent] {
        &self.history
    }

    /// Get events for a specific voice.
    pub fn events_for_voice(&self, voice_id: u32) -> Vec<&VoiceEvent> {
        self.history
            .iter()
            .filter(|e| e.voice_id() == voice_id)
            .collect()
    }

    /// Get allocation events only.
    pub fn allocations(&self) -> Vec<&VoiceEvent> {
        self.history
            .iter()
            .filter(|e| matches!(e, VoiceEvent::Allocated { .. }))
            .collect()
    }

    /// Get release events only.
    pub fn releases(&self) -> Vec<&VoiceEvent> {
        self.history
            .iter()
            .filter(|e| matches!(e, VoiceEvent::Released { .. }))
            .collect()
    }

    /// Get steal events only.
    pub fn steals(&self) -> Vec<&VoiceEvent> {
        self.history
            .iter()
            .filter(|e| matches!(e, VoiceEvent::Stolen { .. }))
            .collect()
    }

    /// Get all snapshots.
    pub fn snapshots(&self) -> &[VoiceSnapshot] {
        &self.snapshots
    }

    /// Get snapshot at or before a specific sample position.
    pub fn snapshot_at(&self, sample_position: u64) -> Option<&VoiceSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|s| s.sample_position <= sample_position)
    }

    /// Count unique voices that were used.
    pub fn unique_voices_used(&self) -> usize {
        let mut used: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for event in &self.history {
            if let VoiceEvent::Allocated { voice_id, .. } = event {
                used.insert(*voice_id);
            }
        }
        used.len()
    }

    /// Check if a voice was reused (allocated, released, then allocated again).
    pub fn was_voice_reused(&self, voice_id: u32) -> bool {
        let events: Vec<_> = self.events_for_voice(voice_id);
        let allocation_count = events
            .iter()
            .filter(|e| matches!(e, VoiceEvent::Allocated { .. }))
            .count();
        allocation_count > 1
    }

    /// Clear all recorded history.
    pub fn clear(&mut self) {
        self.history.clear();
        self.snapshots.clear();
        self.voice_notes.clear();
        self.voice_start_times.clear();
        self.sample_position = 0;
    }

    /// Generate a summary report.
    pub fn summarize(&self) -> String {
        let mut lines = Vec::new();

        lines.push("=== Voice Allocation Summary ===".to_string());
        lines.push(String::new());

        let total_allocations = self.allocations().len();
        let total_releases = self.releases().len();
        let total_steals = self.steals().len();
        let unique_voices = self.unique_voices_used();

        lines.push(format!("Total allocations: {}", total_allocations));
        lines.push(format!("Total releases: {}", total_releases));
        lines.push(format!("Voice steals: {}", total_steals));
        lines.push(format!("Unique voices used: {}", unique_voices));
        lines.push(String::new());

        // List events chronologically
        lines.push("Event Timeline:".to_string());
        for event in &self.history {
            let pos = event.sample_position();
            match event {
                VoiceEvent::Allocated {
                    voice_id,
                    note,
                    velocity,
                    ..
                } => {
                    lines.push(format!(
                        "  @{}: Voice {} allocated - note {:?}, vel {:.2}",
                        pos, voice_id, note, velocity
                    ));
                }
                VoiceEvent::Released {
                    voice_id,
                    note,
                    reason,
                    ..
                } => {
                    lines.push(format!(
                        "  @{}: Voice {} released - note {:?}, reason {:?}",
                        pos, voice_id, note, reason
                    ));
                }
                VoiceEvent::Stolen {
                    voice_id,
                    old_note,
                    new_note,
                    ..
                } => {
                    lines.push(format!(
                        "  @{}: Voice {} stolen - {:?} → {:?}",
                        pos, voice_id, old_note, new_note
                    ));
                }
                VoiceEvent::StateChanged {
                    voice_id,
                    from_state,
                    to_state,
                    ..
                } => {
                    lines.push(format!(
                        "  @{}: Voice {} state {} → {}",
                        pos, voice_id, from_state, to_state
                    ));
                }
                VoiceEvent::Retriggered {
                    voice_id,
                    old_note,
                    new_note,
                    ..
                } => {
                    lines.push(format!(
                        "  @{}: Voice {} retriggered - {:?} → {:?}",
                        pos, voice_id, old_note, new_note
                    ));
                }
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_debugger_basic() {
        let mut dbg = VoiceDebugger::new();

        // Allocate a voice
        dbg.record_allocation(0, MidiNote::C4, 0.8, None);
        assert_eq!(dbg.events().len(), 1);
        assert_eq!(dbg.unique_voices_used(), 1);

        // Release it
        dbg.record_release(0, ReleaseReason::NoteOff);
        assert_eq!(dbg.events().len(), 2);
        assert_eq!(dbg.releases().len(), 1);
    }

    #[test]
    fn test_voice_debugger_steal() {
        let mut dbg = VoiceDebugger::new();

        dbg.record_allocation(0, MidiNote::C4, 0.8, None);
        dbg.advance(1000);
        dbg.record_steal(0, MidiNote::new(62)); // D4

        assert_eq!(dbg.steals().len(), 1);
    }

    #[test]
    fn test_voice_reuse() {
        let mut dbg = VoiceDebugger::new();

        // First allocation
        dbg.record_allocation(0, MidiNote::C4, 0.8, None);
        dbg.advance(1000);
        dbg.record_release(0, ReleaseReason::NoteOff);
        dbg.advance(1000);

        // Second allocation on same voice
        dbg.record_allocation(0, MidiNote::new(64), 0.7, None); // E4

        assert!(dbg.was_voice_reused(0));
        assert!(!dbg.was_voice_reused(1)); // Voice 1 never used
    }

    #[test]
    fn test_events_for_voice() {
        let mut dbg = VoiceDebugger::new();

        dbg.record_allocation(0, MidiNote::C4, 0.8, None);
        dbg.record_allocation(1, MidiNote::new(64), 0.7, None); // E4
        dbg.record_release(0, ReleaseReason::NoteOff);

        let voice0_events = dbg.events_for_voice(0);
        assert_eq!(voice0_events.len(), 2); // allocation + release

        let voice1_events = dbg.events_for_voice(1);
        assert_eq!(voice1_events.len(), 1); // just allocation
    }
}
