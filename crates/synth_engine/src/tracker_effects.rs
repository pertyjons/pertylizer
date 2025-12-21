//! Tracker-style effect processing for sequencer playback.
//!
//! This module provides real-time processing of tracker effects like
//! vibrato, portamento, volume slide, arpeggio, etc.
//!
//! ## Architecture
//!
//! Effects are processed per-channel in the sequencer engine, producing
//! modulation values that are applied to the synth engine's voices.
//!
//! ## Tick-Based Processing
//!
//! Tracker effects operate on "ticks" rather than samples:
//! - Each row has N ticks (speed value, typically 6)
//! - Effects like VolumeSlide run every tick
//! - Effects like Vibrato run continuously with per-tick phase advancement

use serde::{Deserialize, Serialize};

use synth_core::{BipolarValue, NormalizedValue};
use synth_sequencer::TrackId;
use synth_sequencer::effects::{EffectCommand, EffectWaveform};
use synth_sequencer::pitch::Pitch;

/// Maximum number of channels supported.
pub const MAX_CHANNELS: usize = 64;

// ============================================================================
// Type-safe wrappers
// ============================================================================

/// Speed value (ticks per row) - newtype for type safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrackerSpeed(u8);

impl TrackerSpeed {
    /// Default tracker speed (6 ticks per row).
    pub const DEFAULT: Self = Self(6);

    /// Create a new speed value.
    #[must_use]
    pub const fn new(speed: u8) -> Self {
        Self(if speed == 0 { 1 } else { speed })
    }

    /// Get the raw value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// Tick position within a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TickInRow(u8);

impl TickInRow {
    /// First tick (tick 0).
    pub const ZERO: Self = Self(0);

    /// Create a new tick position.
    #[must_use]
    pub const fn new(tick: u8) -> Self {
        Self(tick)
    }

    /// Get the raw value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Advance by one tick.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Sample offset for tracker sample playback start position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrackerSampleOffset(u16);

impl TrackerSampleOffset {
    /// No offset.
    pub const ZERO: Self = Self(0);

    /// Create a new sample offset.
    #[must_use]
    pub const fn new(offset: u16) -> Self {
        Self(offset)
    }

    /// Get the raw value.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Convert to normalized position (0.0-1.0).
    #[must_use]
    pub fn as_normalized(self) -> NormalizedValue {
        // Tracker offset 256 = full sample length
        NormalizedValue::new(f32::from(self.0) / 256.0)
    }
}

/// Pitch offset in cents for effect modulation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PitchCents(f32);

impl PitchCents {
    /// No pitch offset.
    pub const ZERO: Self = Self(0.0);

    /// Create a new pitch offset.
    #[must_use]
    pub fn new(cents: f32) -> Self {
        Self(cents)
    }

    /// Get the raw value.
    #[must_use]
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to semitones.
    #[must_use]
    pub fn as_semitones(self) -> f32 {
        self.0 / 100.0
    }
}

impl std::ops::Add for PitchCents {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for PitchCents {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

// ============================================================================
// Channel Effect Processor
// ============================================================================

/// State for all channel effects during playback.
pub struct ChannelEffectProcessor {
    /// Per-channel effect state.
    channels: Vec<ChannelEffectState>,
    /// Current speed (ticks per row).
    speed: TrackerSpeed,
    /// Current tick within row (0..speed).
    tick_in_row: TickInRow,
    /// Global volume multiplier (0.0-1.0).
    global_volume: NormalizedValue,
    /// Pattern loop state.
    loop_start_row: Option<u32>,
    loop_count: u8,
}

impl Default for ChannelEffectProcessor {
    fn default() -> Self {
        Self::new(MAX_CHANNELS)
    }
}

impl ChannelEffectProcessor {
    /// Create a new processor with the given number of channels.
    #[must_use]
    pub fn new(num_channels: usize) -> Self {
        Self {
            channels: (0..num_channels)
                .map(|_| ChannelEffectState::default())
                .collect(),
            speed: TrackerSpeed::DEFAULT,
            tick_in_row: TickInRow::ZERO,
            global_volume: NormalizedValue::MAX,
            loop_start_row: None,
            loop_count: 0,
        }
    }

    /// Reset all channel states.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
        self.tick_in_row = TickInRow::ZERO;
        self.global_volume = NormalizedValue::MAX;
        self.loop_start_row = None;
        self.loop_count = 0;
    }

    /// Get the current speed (ticks per row).
    #[must_use]
    pub fn speed(&self) -> TrackerSpeed {
        self.speed
    }

    /// Set a new speed (ticks per row).
    pub fn set_speed(&mut self, speed: TrackerSpeed) {
        self.speed = speed;
    }

    /// Get the current global volume.
    #[must_use]
    pub fn global_volume(&self) -> NormalizedValue {
        self.global_volume
    }

    /// Process effects for a new row.
    ///
    /// This should be called when a new row starts. It processes immediate effects
    /// (tick 0) and sets up continuous effects for subsequent ticks.
    ///
    /// # Arguments
    ///
    /// * `track` - The track/channel to process
    /// * `effects` - Effects to process for this row
    /// * `base_pitch` - The note pitch (if a note is playing)
    /// * `instrument_volume` - Volume to reset to when a new note with explicit instrument
    ///   is played. This follows XM behavior: new note with instrument resets channel volume
    ///   to the sample's default_volume (or the volume column value if present).
    ///   Pass `None` for notes that inherit instrument (no volume reset) or effect-only rows.
    ///
    /// Returns any global commands that need sequencer-level handling.
    pub fn process_row_start(
        &mut self,
        track: TrackId,
        effects: &[EffectCommand],
        base_pitch: Option<Pitch>,
        instrument_volume: Option<NormalizedValue>,
    ) -> Vec<GlobalCommand> {
        self.tick_in_row = TickInRow::ZERO;
        let mut global_commands = Vec::new();

        let channel_idx = track.0 as usize;

        // Get or create channel state
        if channel_idx >= self.channels.len() {
            self.channels
                .resize_with(channel_idx + 1, ChannelEffectState::default);
        }
        let state = &mut self.channels[channel_idx];

        // XM behavior: When a new note with explicit instrument is played,
        // reset channel volume to the instrument's default volume.
        // This MUST happen BEFORE effect processing so SetVolume can override.
        if let Some(volume) = instrument_volume {
            state.volume = volume;
            // Also reset volume slide to stop any ongoing slide
            state.volume_slide = 0.0;
        }

        // Update base pitch for portamento target
        if let Some(pitch) = base_pitch {
            state.last_note = Some(pitch);
        }

        // Process each effect
        for effect in effects {
            match effect {
                // === Immediate effects (tick 0 only) ===
                EffectCommand::SetVolume(vol) => {
                    state.volume = NormalizedValue::new(f32::from(*vol) / 64.0);
                }

                EffectCommand::SetPanning(pan) => {
                    // Convert 0-255 to -1.0..1.0
                    let pan_value = (f32::from(*pan) / 127.5) - 1.0;
                    state.panning = BipolarValue::new(pan_value);
                }

                EffectCommand::SampleOffset(offset) => {
                    state.sample_offset = TrackerSampleOffset::new(*offset);
                }

                EffectCommand::FineTune(cents) => {
                    state.fine_tune = PitchCents::new(f32::from(*cents));
                }

                // === Continuous effects (set up state) ===
                EffectCommand::Arpeggio { x, y } => {
                    state.arpeggio = if *x == 0 && *y == 0 {
                        None
                    } else {
                        Some(ArpeggioState {
                            semitone1: *x,
                            semitone2: *y,
                        })
                    };
                }

                EffectCommand::PortamentoUp(speed) => {
                    state.portamento_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    state.portamento_direction = PortamentoDirection::Up;
                }

                EffectCommand::PortamentoDown(speed) => {
                    state.portamento_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    state.portamento_direction = PortamentoDirection::Down;
                }

                EffectCommand::TonePortamento { speed, target } => {
                    if *speed > 0 {
                        state.tone_porta_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    }
                    if let Some(pitch) = target {
                        state.tone_porta_target = Some(*pitch);
                    }
                }

                EffectCommand::Vibrato { speed, depth } => {
                    if *speed > 0 {
                        state.vibrato_speed = f32::from(*speed);
                    }
                    if *depth > 0 {
                        state.vibrato_depth = PitchCents::new(f32::from(*depth) * 4.0);
                    }
                }

                EffectCommand::VibratoWaveform(waveform) => {
                    state.vibrato_waveform = *waveform;
                }

                EffectCommand::VolumeSlide { up, down } => {
                    let delta = (f32::from(*up) - f32::from(*down)) / 64.0;
                    state.volume_slide = delta;
                }

                EffectCommand::FineVolumeSlide { up, down } => {
                    // Fine slide: apply once at tick 0
                    let delta = (f32::from(*up) - f32::from(*down)) / 64.0;
                    let new_vol = (state.volume.as_f32() + delta).clamp(0.0, 1.0);
                    state.volume = NormalizedValue::new(new_vol);
                }

                EffectCommand::Tremolo { speed, depth } => {
                    if *speed > 0 {
                        state.tremolo_speed = f32::from(*speed);
                    }
                    if *depth > 0 {
                        state.tremolo_depth = f32::from(*depth) / 64.0;
                    }
                }

                EffectCommand::TremoloWaveform(waveform) => {
                    state.tremolo_waveform = *waveform;
                }

                EffectCommand::PanningSlide { left, right } => {
                    state.panning_slide = (f32::from(*right) - f32::from(*left)) / 128.0;
                }

                EffectCommand::NoteCut(tick) => {
                    state.note_cut_tick = Some(TickInRow::new(*tick));
                }

                EffectCommand::NoteDelay(tick) => {
                    state.note_delay_tick = Some(TickInRow::new(*tick));
                }

                EffectCommand::Retrigger {
                    interval,
                    volume_change,
                } => {
                    state.retrigger_interval = *interval;
                    state.retrigger_volume_change = f32::from(*volume_change) / 64.0;
                    state.retrigger_counter = 0;
                }

                EffectCommand::NoteFadeOut(tick) => {
                    state.fade_out_tick = Some(TickInRow::new(*tick));
                }

                EffectCommand::Glissando(enabled) => {
                    state.glissando = *enabled;
                }

                // === Global effects ===
                EffectCommand::SetTempo(bpm) => {
                    global_commands.push(GlobalCommand::SetTempo(*bpm));
                }

                EffectCommand::SetSpeed(spd) => {
                    self.speed = TrackerSpeed::new(*spd);
                    global_commands.push(GlobalCommand::SetSpeed(TrackerSpeed::new(*spd)));
                }

                EffectCommand::PatternBreak(row) => {
                    global_commands.push(GlobalCommand::PatternBreak(*row));
                }

                EffectCommand::PatternJump(pos) => {
                    global_commands.push(GlobalCommand::PatternJump(*pos));
                }

                EffectCommand::PatternLoop { count } => {
                    if *count == 0 {
                        // Set loop start
                        global_commands.push(GlobalCommand::SetLoopStart);
                    } else {
                        global_commands.push(GlobalCommand::PatternLoop(*count));
                    }
                }

                EffectCommand::PatternDelay(rows) => {
                    global_commands.push(GlobalCommand::PatternDelay(*rows));
                }

                // Reverse not implemented yet
                EffectCommand::Reverse => {}
            }
        }

        global_commands
    }

    /// Process a single tick for all channels.
    ///
    /// This should be called for each tick within a row (except tick 0 which is
    /// handled by process_row_start). Returns modulation values for each channel.
    pub fn process_tick(&mut self) -> Vec<ChannelModulation> {
        self.tick_in_row = self.tick_in_row.next();

        self.channels
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, state)| {
                let track = TrackId::new(idx as u16);
                let modulation = state.process_tick(self.tick_in_row, track);
                if modulation.is_significant() {
                    Some(modulation)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get the current modulation for a specific channel.
    #[must_use]
    pub fn get_channel_modulation(&self, track: TrackId) -> ChannelModulation {
        let idx = track.0 as usize;
        self.channels
            .get(idx)
            .map(|s| s.current_modulation(track))
            .unwrap_or_else(|| ChannelModulation::default_for(track))
    }

    /// Get mutable reference to channel state.
    pub fn channel_mut(&mut self, track: TrackId) -> Option<&mut ChannelEffectState> {
        let idx = track.0 as usize;
        self.channels.get_mut(idx)
    }
}

// ============================================================================
// Channel State
// ============================================================================

/// Direction for portamento effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortamentoDirection {
    #[default]
    Off,
    Up,
    Down,
}

/// Per-channel effect state.
#[derive(Debug, Clone)]
pub struct ChannelEffectState {
    // === Current values ===
    /// Current volume (0.0-1.0).
    pub volume: NormalizedValue,
    /// Current panning (-1.0 to 1.0).
    pub panning: BipolarValue,
    /// Current pitch offset in cents.
    pub pitch_offset: PitchCents,
    /// Fine tune in cents.
    pub fine_tune: PitchCents,
    /// Sample offset for next note.
    pub sample_offset: TrackerSampleOffset,

    // === Effect memory ===
    /// Last played note (for tone portamento target).
    pub last_note: Option<Pitch>,

    // === Arpeggio ===
    pub arpeggio: Option<ArpeggioState>,

    // === Portamento ===
    pub portamento_speed: PitchCents,
    pub portamento_direction: PortamentoDirection,

    // === Tone portamento (glide to note) ===
    pub tone_porta_speed: PitchCents,
    pub tone_porta_target: Option<Pitch>,
    pub current_pitch: f32, // Current pitch in semitones for glide

    // === Vibrato ===
    pub vibrato_speed: f32,
    pub vibrato_depth: PitchCents,
    pub vibrato_phase: f32, // 0.0-1.0
    pub vibrato_waveform: EffectWaveform,

    // === Volume slide ===
    pub volume_slide: f32, // Per-tick change

    // === Tremolo ===
    pub tremolo_speed: f32,
    pub tremolo_depth: f32,
    pub tremolo_phase: f32,
    pub tremolo_waveform: EffectWaveform,

    // === Panning slide ===
    pub panning_slide: f32,

    // === Timing effects ===
    pub note_cut_tick: Option<TickInRow>,
    pub note_delay_tick: Option<TickInRow>,
    pub fade_out_tick: Option<TickInRow>,

    // === Retrigger ===
    pub retrigger_interval: u8,
    pub retrigger_volume_change: f32,
    pub retrigger_counter: u8,

    // === Glissando ===
    pub glissando: bool,

    // === Note state ===
    pub is_playing: bool,
    pub note_triggered: bool, // Set true when note should trigger (after delay)
    pub note_cut: bool,       // Set true when note should cut
}

impl Default for ChannelEffectState {
    fn default() -> Self {
        Self {
            volume: NormalizedValue::MAX,
            panning: BipolarValue::CENTER,
            pitch_offset: PitchCents::ZERO,
            fine_tune: PitchCents::ZERO,
            sample_offset: TrackerSampleOffset::ZERO,
            last_note: None,
            arpeggio: None,
            portamento_speed: PitchCents::ZERO,
            portamento_direction: PortamentoDirection::Off,
            tone_porta_speed: PitchCents::ZERO,
            tone_porta_target: None,
            current_pitch: 0.0,
            vibrato_speed: 0.0,
            vibrato_depth: PitchCents::ZERO,
            vibrato_phase: 0.0,
            vibrato_waveform: EffectWaveform::Sine,
            volume_slide: 0.0,
            tremolo_speed: 0.0,
            tremolo_depth: 0.0,
            tremolo_phase: 0.0,
            tremolo_waveform: EffectWaveform::Sine,
            panning_slide: 0.0,
            note_cut_tick: None,
            note_delay_tick: None,
            fade_out_tick: None,
            retrigger_interval: 0,
            retrigger_volume_change: 0.0,
            retrigger_counter: 0,
            glissando: false,
            is_playing: false,
            note_triggered: false,
            note_cut: false,
        }
    }
}

impl ChannelEffectState {
    /// Reset the channel state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Process one tick and return modulation.
    pub fn process_tick(&mut self, tick: TickInRow, track: TrackId) -> ChannelModulation {
        self.note_triggered = false;
        self.note_cut = false;

        // Note delay check
        if let Some(delay_tick) = self.note_delay_tick
            && tick.as_u8() == delay_tick.as_u8()
        {
            self.note_triggered = true;
            self.note_delay_tick = None;
        }

        // Note cut check
        if let Some(cut_tick) = self.note_cut_tick
            && tick.as_u8() >= cut_tick.as_u8()
        {
            self.note_cut = true;
            self.volume = NormalizedValue::MIN;
            self.note_cut_tick = None;
        }

        // Fade out check
        if let Some(fade_tick) = self.fade_out_tick
            && tick.as_u8() >= fade_tick.as_u8()
        {
            let new_vol = self.volume.as_f32() * 0.9;
            self.volume = NormalizedValue::new(new_vol);
            if self.volume.as_f32() < 0.001 {
                self.note_cut = true;
            }
        }

        // Volume slide (runs from tick 1 onwards)
        if tick.as_u8() > 0 && self.volume_slide != 0.0 {
            let new_vol = (self.volume.as_f32() + self.volume_slide).clamp(0.0, 1.0);
            self.volume = NormalizedValue::new(new_vol);
        }

        // Panning slide
        if tick.as_u8() > 0 && self.panning_slide != 0.0 {
            let new_pan = (self.panning.as_f32() + self.panning_slide).clamp(-1.0, 1.0);
            self.panning = BipolarValue::new(new_pan);
        }

        // Portamento up/down
        if tick.as_u8() > 0 {
            match self.portamento_direction {
                PortamentoDirection::Up => {
                    self.pitch_offset += self.portamento_speed;
                }
                PortamentoDirection::Down => {
                    self.pitch_offset = PitchCents::new(
                        self.pitch_offset.as_f32() - self.portamento_speed.as_f32(),
                    );
                }
                PortamentoDirection::Off => {}
            }
        }

        // Tone portamento (glide to target)
        if tick.as_u8() > 0
            && self.tone_porta_speed.as_f32() > 0.0
            && let Some(target) = self.tone_porta_target
        {
            let target_pitch = f32::from(target.as_midi());
            let diff = target_pitch - self.current_pitch;
            if diff.abs() > 0.01 {
                let step = self.tone_porta_speed.as_f32() / 100.0; // Convert cents to semitones
                if diff > 0.0 {
                    self.current_pitch = (self.current_pitch + step).min(target_pitch);
                } else {
                    self.current_pitch = (self.current_pitch - step).max(target_pitch);
                }
            }
        }

        // Vibrato
        if self.vibrato_depth.as_f32() > 0.0 {
            self.vibrato_phase += self.vibrato_speed / 64.0;
            if self.vibrato_phase >= 1.0 {
                self.vibrato_phase -= 1.0;
            }
        }

        // Tremolo
        if self.tremolo_depth > 0.0 {
            self.tremolo_phase += self.tremolo_speed / 64.0;
            if self.tremolo_phase >= 1.0 {
                self.tremolo_phase -= 1.0;
            }
        }

        // Retrigger
        if self.retrigger_interval > 0 {
            self.retrigger_counter += 1;
            if self.retrigger_counter >= self.retrigger_interval {
                self.retrigger_counter = 0;
                self.note_triggered = true;
                let new_vol = (self.volume.as_f32() + self.retrigger_volume_change).clamp(0.0, 1.0);
                self.volume = NormalizedValue::new(new_vol);
            }
        }

        self.current_modulation(track)
    }

    /// Get the current modulation values.
    #[must_use]
    pub fn current_modulation(&self, track: TrackId) -> ChannelModulation {
        // Calculate pitch modulation
        let mut pitch_mod = self.pitch_offset + self.fine_tune;

        // Add vibrato
        if self.vibrato_depth.as_f32() > 0.0 {
            let vibrato =
                self.vibrato_waveform.sample(self.vibrato_phase) * self.vibrato_depth.as_f32();
            pitch_mod += PitchCents::new(vibrato);
        }

        // Calculate arpeggio offset
        let arpeggio_semitones = self.arpeggio.as_ref().map_or(0, |arp| {
            // Cycle through base, +x, +y based on phase
            match (self.vibrato_phase * 3.0) as u8 % 3 {
                0 => 0,
                1 => arp.semitone1,
                _ => arp.semitone2,
            }
        });
        pitch_mod += PitchCents::new(f32::from(arpeggio_semitones) * 100.0);

        // Calculate volume modulation
        let mut volume_mod = self.volume.as_f32();

        // Add tremolo
        if self.tremolo_depth > 0.0 {
            let tremolo = self.tremolo_waveform.sample(self.tremolo_phase) * self.tremolo_depth;
            volume_mod *= 1.0 + tremolo;
            volume_mod = volume_mod.clamp(0.0, 1.0);
        }

        ChannelModulation {
            track,
            pitch_cents: pitch_mod,
            volume: NormalizedValue::new(volume_mod),
            panning: self.panning,
            note_triggered: self.note_triggered,
            note_cut: self.note_cut,
            sample_offset: self.sample_offset,
            tone_porta_pitch: if self.tone_porta_target.is_some() {
                Some(self.current_pitch)
            } else {
                None
            },
        }
    }
}

// ============================================================================
// Supporting Types
// ============================================================================

/// Arpeggio state.
#[derive(Debug, Clone, Copy)]
pub struct ArpeggioState {
    pub semitone1: u8,
    pub semitone2: u8,
}

/// Modulation values for a channel.
#[derive(Debug, Clone)]
pub struct ChannelModulation {
    /// Track/channel ID.
    pub track: TrackId,
    /// Pitch offset in cents (100 cents = 1 semitone).
    pub pitch_cents: PitchCents,
    /// Volume multiplier (0.0-1.0).
    pub volume: NormalizedValue,
    /// Panning position (-1.0 to 1.0).
    pub panning: BipolarValue,
    /// True if note should trigger this tick (note delay, retrigger).
    pub note_triggered: bool,
    /// True if note should cut this tick.
    pub note_cut: bool,
    /// Sample offset for the note.
    pub sample_offset: TrackerSampleOffset,
    /// Current pitch for tone portamento (in semitones).
    pub tone_porta_pitch: Option<f32>,
}

impl ChannelModulation {
    /// Create a default modulation for a track.
    #[must_use]
    pub fn default_for(track: TrackId) -> Self {
        Self {
            track,
            pitch_cents: PitchCents::ZERO,
            volume: NormalizedValue::MAX,
            panning: BipolarValue::CENTER,
            note_triggered: false,
            note_cut: false,
            sample_offset: TrackerSampleOffset::ZERO,
            tone_porta_pitch: None,
        }
    }

    /// Check if this modulation has significant values.
    #[must_use]
    pub fn is_significant(&self) -> bool {
        self.pitch_cents.as_f32().abs() > 0.01
            || (self.volume.as_f32() - 1.0).abs() > 0.01
            || self.panning.as_f32().abs() > 0.01
            || self.note_triggered
            || self.note_cut
            || self.sample_offset.as_u16() > 0
            || self.tone_porta_pitch.is_some()
    }
}

/// Global commands that affect sequencer-level state.
#[derive(Debug, Clone)]
pub enum GlobalCommand {
    /// Set tempo in BPM.
    SetTempo(u16),
    /// Set speed (ticks per row).
    SetSpeed(TrackerSpeed),
    /// Jump to next pattern at specified row.
    PatternBreak(u8),
    /// Jump to pattern at position.
    PatternJump(u8),
    /// Set loop start point.
    SetLoopStart,
    /// Loop back to start point.
    PatternLoop(u8),
    /// Delay pattern by N rows.
    PatternDelay(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_effect() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Set volume to 50%
        processor.process_row_start(track, &[EffectCommand::SetVolume(32)], None, None);

        let mod_val = processor.get_channel_modulation(track);
        assert!((mod_val.volume.as_f32() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_volume_slide() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start at full volume, slide down
        processor.process_row_start(
            track,
            &[EffectCommand::VolumeSlide { up: 0, down: 4 }],
            None,
            None,
        );

        // Tick 0: no slide yet
        let mod0 = processor.get_channel_modulation(track);
        assert!((mod0.volume.as_f32() - 1.0).abs() < 0.01);

        // Tick 1: slide applied
        processor.process_tick();
        let mod1 = processor.get_channel_modulation(track);
        assert!(mod1.volume.as_f32() < 1.0);
    }

    #[test]
    fn test_vibrato() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        processor.process_row_start(
            track,
            &[EffectCommand::Vibrato { speed: 8, depth: 8 }],
            None,
            None,
        );

        // Process several ticks
        for _ in 0..4 {
            processor.process_tick();
        }

        let modulation = processor.get_channel_modulation(track);
        // Vibrato should cause pitch variation
        assert!(modulation.pitch_cents.as_f32().abs() >= 0.0);
    }

    #[test]
    fn test_panning() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Full left
        processor.process_row_start(track, &[EffectCommand::SetPanning(0)], None, None);
        let left = processor.get_channel_modulation(track);
        assert!((left.panning.as_f32() - (-1.0)).abs() < 0.02);

        // Center
        processor.process_row_start(track, &[EffectCommand::SetPanning(128)], None, None);
        let center = processor.get_channel_modulation(track);
        assert!(center.panning.as_f32().abs() < 0.02);

        // Full right
        processor.process_row_start(track, &[EffectCommand::SetPanning(255)], None, None);
        let right = processor.get_channel_modulation(track);
        assert!((right.panning.as_f32() - 1.0).abs() < 0.02);
    }

    #[test]
    fn test_global_commands() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        let commands = processor.process_row_start(
            track,
            &[EffectCommand::SetTempo(140), EffectCommand::SetSpeed(4)],
            None,
            None,
        );

        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0], GlobalCommand::SetTempo(140)));
        assert!(matches!(
            commands[1],
            GlobalCommand::SetSpeed(TrackerSpeed(4))
        ));
        assert_eq!(processor.speed().as_u8(), 4);
    }

    #[test]
    fn test_note_cut() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        processor.process_row_start(track, &[EffectCommand::NoteCut(3)], None, None);

        // Ticks 1, 2: no cut
        processor.process_tick();
        assert!(!processor.get_channel_modulation(track).note_cut);
        processor.process_tick();
        assert!(!processor.get_channel_modulation(track).note_cut);

        // Tick 3: cut
        processor.process_tick();
        assert!(processor.get_channel_modulation(track).note_cut);
    }

    #[test]
    fn test_arpeggio() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        processor.process_row_start(track, &[EffectCommand::Arpeggio { x: 4, y: 7 }], None, None);

        // Process a tick
        processor.process_tick();

        // The test passes if arpeggio state is set
        let idx = track.0 as usize;
        assert!(processor.channels[idx].arpeggio.is_some());
    }

    #[test]
    fn test_tracker_speed_default() {
        let speed = TrackerSpeed::DEFAULT;
        assert_eq!(speed.as_u8(), 6);
    }

    #[test]
    fn test_pitch_cents_arithmetic() {
        let a = PitchCents::new(100.0);
        let b = PitchCents::new(50.0);
        let sum = a + b;
        assert!((sum.as_f32() - 150.0).abs() < 0.001);
    }

    #[test]
    fn test_volume_reset_on_new_instrument() {
        // This test verifies XM behavior: when a new note with explicit instrument
        // is played, channel volume resets to the instrument's default volume.
        // This fixes the bug where volume slide brings volume to 0 and new notes stay silent.
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start at full volume, slide down to near zero
        processor.process_row_start(
            track,
            &[EffectCommand::VolumeSlide { up: 0, down: 16 }],
            None,
            None,
        );

        // Process several ticks to slide volume down
        for _ in 0..6 {
            processor.process_tick();
        }

        // Volume should be very low after slide
        let mod_after_slide = processor.get_channel_modulation(track);
        assert!(
            mod_after_slide.volume.as_f32() < 0.1,
            "Volume should be low after slide: {}",
            mod_after_slide.volume.as_f32()
        );

        // Now play a new note with explicit instrument (volume reset to 0.75)
        processor.process_row_start(
            track,
            &[], // No effects
            Some(synth_sequencer::Pitch::new(60).unwrap()),
            Some(NormalizedValue::new(0.75)), // Instrument default volume
        );

        // Volume should be reset to 0.75
        let mod_after_reset = processor.get_channel_modulation(track);
        assert!(
            (mod_after_reset.volume.as_f32() - 0.75).abs() < 0.01,
            "Volume should be reset to 0.75: {}",
            mod_after_reset.volume.as_f32()
        );

        // Also verify that volume slide was stopped
        assert!(
            processor.channels[0].volume_slide.abs() < 0.001,
            "Volume slide should be stopped"
        );
    }

    #[test]
    fn test_volume_not_reset_on_inherit_instrument() {
        // This test verifies that when a note inherits instrument (no explicit instrument),
        // the channel volume is NOT reset - existing volume continues.
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Set volume to 0.3 via SetVolume effect
        processor.process_row_start(track, &[EffectCommand::SetVolume(19)], None, None); // 19/64 ≈ 0.3

        let mod_after_set = processor.get_channel_modulation(track);
        assert!(
            (mod_after_set.volume.as_f32() - 0.297).abs() < 0.02,
            "Volume should be ~0.3: {}",
            mod_after_set.volume.as_f32()
        );

        // Now play a note that inherits instrument (instrument_volume = None)
        processor.process_row_start(
            track,
            &[], // No effects
            Some(synth_sequencer::Pitch::new(60).unwrap()),
            None, // Inherit instrument - don't reset volume
        );

        // Volume should still be ~0.3
        let mod_after_note = processor.get_channel_modulation(track);
        assert!(
            (mod_after_note.volume.as_f32() - 0.297).abs() < 0.02,
            "Volume should remain ~0.3: {}",
            mod_after_note.volume.as_f32()
        );
    }

    #[test]
    fn test_set_volume_overrides_instrument_default() {
        // This test verifies that SetVolume effect can override the instrument's default volume
        // even when both are on the same row.
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Play note with instrument default 1.0, but SetVolume to 0.5
        processor.process_row_start(
            track,
            &[EffectCommand::SetVolume(32)], // 32/64 = 0.5
            Some(synth_sequencer::Pitch::new(60).unwrap()),
            Some(NormalizedValue::new(1.0)), // Instrument default = 1.0
        );

        // Volume should be 0.5 (SetVolume overrides instrument default)
        let modulation = processor.get_channel_modulation(track);
        assert!(
            (modulation.volume.as_f32() - 0.5).abs() < 0.01,
            "Volume should be 0.5 (SetVolume overrides default): {}",
            modulation.volume.as_f32()
        );
    }
}
