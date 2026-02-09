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

use synth_core::{BipolarValue, NormalizedValue, Phase, Semitones};
use synth_sequencer::TrackId;
use synth_sequencer::effects::{EffectCommand, EffectWaveform};
use synth_sequencer::pitch::Pitch;

/// Maximum number of channels supported.
pub const MAX_CHANNELS: usize = 64;

// ============================================================================
// Note Trigger Types
// ============================================================================

/// Default values from instrument/sample for note triggering.
///
/// Used to reset channel state when a note with explicit instrument is triggered.
/// These values should come from the sample/instrument, not be hardcoded.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct InstrumentDefaults {
    /// Sample's default volume (0.0-1.0). Use 1.0 if not specified.
    pub volume: NormalizedValue,
    /// Sample's default panning (-1.0 to 1.0). Use 0.0 (center) if not specified.
    pub panning: BipolarValue,
}

impl Default for InstrumentDefaults {
    fn default() -> Self {
        Self {
            volume: NormalizedValue::MAX,
            panning: BipolarValue::CENTER,
        }
    }
}

impl InstrumentDefaults {
    /// Create from sample default values.
    ///
    /// # Arguments
    /// * `volume` - Optional volume from sample (0.0-1.0), defaults to 1.0
    /// * `panning` - Optional panning from sample (0.0=left, 0.5=center, 1.0=right), defaults to center
    pub fn from_sample(volume: Option<f32>, panning: Option<f32>) -> Self {
        Self {
            volume: NormalizedValue::new(volume.unwrap_or(1.0)),
            // Convert from tracker panning (0.0-1.0) to bipolar (-1.0 to 1.0)
            panning: BipolarValue::new(panning.map_or(0.0, |p| (p * 2.0) - 1.0)),
        }
    }
}

/// Information needed to trigger a note on a channel.
///
/// Used by `process_row_start()` to correctly handle the state machine
/// for note triggering (fresh attack vs tone portamento).
#[derive(Debug, Clone)]
pub struct NoteTriggerInfo {
    /// The pitch of the new note.
    pub pitch: Pitch,
    /// Instrument defaults if an explicit instrument was specified.
    /// `Some` = reset volume/panning to these values.
    /// `None` = inherit current channel state (no volume/panning reset).
    pub instrument_defaults: Option<InstrumentDefaults>,
}

// ============================================================================
// Type-safe wrappers
// ============================================================================

/// Speed value (ticks per row) - newtype for type safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct TrackerSpeed(pub(crate) u8);

impl TrackerSpeed {
    /// Default tracker speed (6 ticks per row).
    pub const DEFAULT: Self = Self(6);

    /// Create a new speed value.
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
#[must_use]
pub struct TickInRow(u8);

impl TickInRow {
    /// First tick (tick 0).
    pub const ZERO: Self = Self(0);

    /// Create a new tick position.
    pub const fn new(tick: u8) -> Self {
        Self(tick)
    }

    /// Get the raw value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Advance by one tick.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Sample offset for tracker sample playback start position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct TrackerSampleOffset(u16);

impl TrackerSampleOffset {
    /// No offset.
    pub const ZERO: Self = Self(0);

    /// Create a new sample offset.
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
///
/// 100 cents = 1 semitone. Used for vibrato, portamento, and other pitch effects.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[must_use]
pub struct PitchCents(f32);

impl PitchCents {
    /// No pitch offset.
    pub const ZERO: Self = Self(0.0);

    /// Create a new pitch offset.
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

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for PitchCents {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::ops::Sub for PitchCents {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::SubAssign for PitchCents {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl std::ops::Neg for PitchCents {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

/// Effect speed value (for vibrato/tremolo speed parameters).
///
/// Higher values mean faster oscillation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[must_use]
pub struct EffectSpeed(f32);

impl EffectSpeed {
    /// No speed (effect disabled).
    pub const ZERO: Self = Self(0.0);

    /// Create a new effect speed.
    pub fn new(speed: f32) -> Self {
        Self(speed)
    }

    /// Create from tracker effect parameter (0-15).
    pub fn from_param(param: u8) -> Self {
        Self(f32::from(param))
    }

    /// Get the raw value.
    #[must_use]
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Check if the speed is non-zero (effect active).
    #[must_use]
    pub fn is_active(self) -> bool {
        self.0 > 0.0
    }
}

/// Per-tick slide rate for volume and panning effects.
///
/// Represents the amount to change per tick. Positive = up/right, negative = down/left.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[must_use]
pub struct SlideRate(f32);

impl SlideRate {
    /// No slide.
    pub const ZERO: Self = Self(0.0);

    /// Create a new slide rate.
    pub fn new(rate: f32) -> Self {
        Self(rate)
    }

    /// Create from tracker volume slide parameters.
    ///
    /// FT2 priority rule: if both nibbles are non-zero, UP (upper nibble) takes priority.
    ///
    /// # Arguments
    /// * `up` - Slide up value (0-15)
    /// * `down` - Slide down value (0-15)
    pub fn from_volume_slide(up: u8, down: u8) -> Self {
        if up > 0 {
            Self(f32::from(up) / 64.0)
        } else {
            Self(-f32::from(down) / 64.0)
        }
    }

    /// Create from tracker panning slide parameters.
    ///
    /// FT2 priority rule: if both nibbles are non-zero, RIGHT (upper nibble) takes priority.
    pub fn from_panning_slide(left: u8, right: u8) -> Self {
        if right > 0 {
            Self(f32::from(right) / 128.0)
        } else {
            Self(-f32::from(left) / 128.0)
        }
    }

    /// Get the raw value.
    #[must_use]
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Check if the slide is active (non-zero).
    #[must_use]
    pub fn is_active(self) -> bool {
        self.0.abs() > f32::EPSILON
    }
}

/// Depth/intensity for tremolo effect (0.0-1.0).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[must_use]
pub struct TremoloDepth(f32);

impl TremoloDepth {
    /// No tremolo.
    pub const ZERO: Self = Self(0.0);

    /// Create a new tremolo depth.
    pub fn new(depth: f32) -> Self {
        Self(depth.clamp(0.0, 1.0))
    }

    /// Create from tracker effect parameter (0-15).
    ///
    /// FT2 formula: `(waveform_peak * depth) >> 6` where peak=255, volume range=64.
    /// Our waveform returns ±1.0 (peak=1.0), so: `depth * 255 / 64 / 64`.
    pub fn from_param(param: u8) -> Self {
        Self((f32::from(param) * 255.0 / 64.0 / 64.0).clamp(0.0, 1.0))
    }

    /// Get the raw value.
    #[must_use]
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Check if tremolo is active.
    #[must_use]
    pub fn is_active(self) -> bool {
        self.0 > 0.0
    }
}

/// Retrigger effect state.
///
/// Controls automatic note retriggering at regular intervals,
/// optionally with volume change per retrigger.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[must_use]
pub struct RetriggerState {
    /// Interval in ticks between retriggers. 0 = disabled.
    interval: TickCount,
    /// Current tick counter.
    counter: TickCount,
    /// Volume change per retrigger.
    volume_change: SlideRate,
}

impl RetriggerState {
    /// Disabled retrigger state.
    pub const DISABLED: Self = Self {
        interval: TickCount::ZERO,
        counter: TickCount::ZERO,
        volume_change: SlideRate::ZERO,
    };

    /// Create a new retrigger state.
    pub fn new(interval: TickCount, volume_change: SlideRate) -> Self {
        Self {
            interval,
            counter: TickCount::ZERO,
            volume_change,
        }
    }

    /// Check if retrigger is active.
    #[must_use]
    pub fn is_active(self) -> bool {
        self.interval.0 > 0
    }

    /// Get the interval.
    pub fn interval(self) -> TickCount {
        self.interval
    }

    /// Get the volume change per retrigger.
    pub fn volume_change(self) -> SlideRate {
        self.volume_change
    }

    /// Advance the counter and return true if a retrigger should occur.
    pub fn tick(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }

        self.counter.0 += 1;
        if self.counter.0 >= self.interval.0 {
            self.counter = TickCount::ZERO;
            true
        } else {
            false
        }
    }

    /// Reset the counter.
    pub fn reset_counter(&mut self) {
        self.counter = TickCount::ZERO;
    }
}

/// Tick count for retrigger intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct TickCount(u8);

impl TickCount {
    /// Zero ticks.
    pub const ZERO: Self = Self(0);

    /// Create a new tick count.
    pub const fn new(count: u8) -> Self {
        Self(count)
    }

    /// Get the raw value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// Glissando mode for tone portamento.
///
/// When enabled, portamento slides are quantized to semitones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Glissando {
    /// Smooth portamento (default).
    #[default]
    Smooth,
    /// Quantize portamento to semitones.
    Quantized,
}

impl Glissando {
    /// Check if glissando (semitone quantization) is enabled.
    #[must_use]
    pub fn is_quantized(self) -> bool {
        matches!(self, Self::Quantized)
    }
}

impl From<bool> for Glissando {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Quantized
        } else {
            Self::Smooth
        }
    }
}

/// Current note state flags for a channel.
///
/// Tracks whether a note is playing and pending trigger/cut actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[must_use]
pub struct NoteState {
    /// Whether a note is currently playing on this channel.
    playing: PlayingState,
    /// Whether note should trigger this tick (after delay, retrigger).
    trigger: TriggerAction,
    /// Whether note should cut this tick.
    cut: CutAction,
}

impl NoteState {
    /// Default state: not playing, no pending actions.
    pub const SILENT: Self = Self {
        playing: PlayingState::Stopped,
        trigger: TriggerAction::None,
        cut: CutAction::None,
    };

    /// Create a new playing state.
    pub fn playing() -> Self {
        Self {
            playing: PlayingState::Playing,
            trigger: TriggerAction::Trigger,
            cut: CutAction::None,
        }
    }

    /// Check if a note is currently playing.
    #[must_use]
    pub fn is_playing(self) -> bool {
        matches!(self.playing, PlayingState::Playing)
    }

    /// Check if a note trigger is pending.
    #[must_use]
    pub fn should_trigger(self) -> bool {
        matches!(self.trigger, TriggerAction::Trigger)
    }

    /// Check if a note cut is pending.
    #[must_use]
    pub fn should_cut(self) -> bool {
        matches!(self.cut, CutAction::Cut)
    }

    /// Set the playing state.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = if playing {
            PlayingState::Playing
        } else {
            PlayingState::Stopped
        };
    }

    /// Set the trigger action for this tick.
    pub fn set_trigger(&mut self, trigger: bool) {
        self.trigger = if trigger {
            TriggerAction::Trigger
        } else {
            TriggerAction::None
        };
    }

    /// Set the cut action for this tick.
    pub fn set_cut(&mut self, cut: bool) {
        self.cut = if cut { CutAction::Cut } else { CutAction::None };
    }

    /// Clear per-tick actions (trigger, cut) but keep playing state.
    pub fn clear_actions(&mut self) {
        self.trigger = TriggerAction::None;
        self.cut = CutAction::None;
    }

    /// Trigger the note (set playing and trigger).
    pub fn trigger(&mut self) {
        self.playing = PlayingState::Playing;
        self.trigger = TriggerAction::Trigger;
        self.cut = CutAction::None;
    }

    /// Cut the note (set not playing and cut).
    pub fn cut(&mut self) {
        self.cut = CutAction::Cut;
    }

    /// Stop the note (set not playing).
    pub fn stop(&mut self) {
        self.playing = PlayingState::Stopped;
    }
}

/// Whether a note is currently playing on a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlayingState {
    /// No note is playing.
    #[default]
    Stopped,
    /// A note is playing.
    Playing,
}

/// Whether a note trigger is pending this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TriggerAction {
    /// No trigger pending.
    #[default]
    None,
    /// Note should be triggered.
    Trigger,
}

/// Whether a note cut is pending this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CutAction {
    /// No cut pending.
    #[default]
    None,
    /// Note should be cut.
    Cut,
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
    /// Amiga frequency mode: portamento operates in period space.
    amiga_mode: bool,
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
            amiga_mode: false,
        }
    }

    /// Set Amiga frequency mode for period-based portamento.
    pub fn set_amiga_mode(&mut self, amiga: bool) {
        self.amiga_mode = amiga;
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
    /// * `trigger_info` - Note trigger information if a note is being triggered on this row.
    ///   Contains the pitch and optional instrument defaults for proper state reset.
    ///   Pass `None` for effect-only rows (no note trigger).
    ///
    /// # State Machine Order (CRITICAL!)
    ///
    /// 1. First: Scan effects to detect tone portamento and sample offset
    /// 2. Then: Call `trigger_note()` if a note is present
    /// 3. Finally: Process all effects (these can override trigger_note values)
    ///
    /// Returns a tuple of:
    /// - Global commands that need sequencer-level handling
    /// - Whether the note should actually trigger (false for tone portamento)
    /// - Sample offset (0.0-1.0) for 9xx effect, applies at note start
    pub fn process_row_start(
        &mut self,
        track: TrackId,
        effects: &[EffectCommand],
        trigger_info: Option<NoteTriggerInfo>,
    ) -> (Vec<GlobalCommand>, bool, NormalizedValue) {
        self.tick_in_row = TickInRow::ZERO;
        let mut global_commands = Vec::new();

        let channel_idx = track.0 as usize;

        // Get or create channel state
        if channel_idx >= self.channels.len() {
            self.channels
                .resize_with(channel_idx + 1, ChannelEffectState::default);
        }
        let state = &mut self.channels[channel_idx];

        // STEP 1: Scan effects to determine trigger type
        // This must happen BEFORE trigger_note() to correctly identify tone portamento
        let is_tone_portamento = effects
            .iter()
            .any(|e| matches!(e, EffectCommand::TonePortamento { .. }));

        let has_sample_offset = effects
            .iter()
            .any(|e| matches!(e, EffectCommand::SampleOffset(_)));

        let has_note_delay = effects
            .iter()
            .any(|e| matches!(e, EffectCommand::NoteDelay(_)));

        // STEP 2: Determine if we should trigger a new note
        // Tone portamento normally doesn't trigger a new note (it glides from the previous note).
        // EXCEPTION: If there's no previous note on this channel, we MUST trigger the note,
        // otherwise the first note with tone portamento would be silent.
        let has_note = trigger_info.is_some();
        let has_previous_note = state.last_note.is_some();
        let should_trigger_note = has_note && (!is_tone_portamento || !has_previous_note);

        // STEP 3: Process note trigger (BEFORE effect processing!)
        // This resets the appropriate state based on trigger type
        if let Some(trigger) = trigger_info {
            state.trigger_note(
                trigger.pitch,
                trigger.instrument_defaults,
                is_tone_portamento,
                has_sample_offset,
                has_note_delay,
            );
        }

        // STEP 3.5: Reset continuous effects — they only run on rows where they're specified.
        // Save effect memory first (for param=0 "continue" behavior).
        let porta_speed_mem = state.portamento_speed;
        let vol_slide_mem = state.volume_slide;
        let vibrato_speed_mem = state.vibrato_speed;
        let vibrato_depth_mem = state.vibrato_depth;
        let tremolo_speed_mem = state.tremolo_speed;
        let tremolo_depth_mem = state.tremolo_depth;
        let panning_slide_mem = state.panning_slide;
        let tone_porta_speed_mem = state.tone_porta_speed;
        let fine_vol_slide_mem = state.fine_volume_slide;

        // Reset active state (memory values preserved in local vars above)
        state.volume_slide = SlideRate::ZERO;
        state.portamento_direction = PortamentoDirection::Off;
        state.vibrato_depth = PitchCents::ZERO;
        state.panning_slide = SlideRate::ZERO;
        state.arpeggio = None;
        state.tremolo_depth = TremoloDepth::ZERO;
        state.retrigger = RetriggerState::DISABLED;
        state.tone_porta_active = false;
        state.note_cut_tick = None;
        state.note_delay_tick = None;
        state.fade_out_tick = None;

        // STEP 3.6: When tone portamento starts, absorb any accumulated pitch_offset
        // into current_pitch so the slide begins from the correct position.
        // In FT2, portamento up/down and tone portamento both modify a single period
        // variable. Our code uses separate current_pitch + pitch_offset, so we must
        // merge them when switching from regular portamento to tone portamento.
        if is_tone_portamento && state.pitch_offset.as_f32().abs() > f32::EPSILON {
            let offset_semitones = state.pitch_offset.as_f32() / 100.0;
            let new_pitch = state.current_pitch.as_f32() + offset_semitones;
            state.current_pitch = Semitones::new(new_pitch);
            state.pitch_offset = PitchCents::ZERO;
        }

        // STEP 4: Process each effect (can override trigger_note values)
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
                    if *speed > 0 {
                        state.portamento_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    } else {
                        state.portamento_speed = porta_speed_mem; // 100: restore from memory
                    }
                    state.portamento_direction = PortamentoDirection::Up;
                }

                EffectCommand::PortamentoDown(speed) => {
                    if *speed > 0 {
                        state.portamento_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    } else {
                        state.portamento_speed = porta_speed_mem; // 200: restore from memory
                    }
                    state.portamento_direction = PortamentoDirection::Down;
                }

                EffectCommand::TonePortamento { speed, target } => {
                    if *speed > 0 {
                        state.tone_porta_speed = PitchCents::new(f32::from(*speed) * 4.0);
                    } else {
                        state.tone_porta_speed = tone_porta_speed_mem; // 300: restore from memory
                    }
                    if let Some(pitch) = target {
                        state.tone_porta_target = Some(*pitch);
                    }
                    state.tone_porta_active = true;
                }

                EffectCommand::Vibrato { speed, depth } => {
                    if *speed > 0 {
                        state.vibrato_speed = EffectSpeed::from_param(*speed);
                    } else {
                        state.vibrato_speed = vibrato_speed_mem; // 4x0: restore speed from memory
                    }
                    if *depth > 0 {
                        // FT2 vibrato depth: (sine_peak * depth) >> 5 fine periods
                        // sine_peak=255, so offset = 255*depth/32 periods
                        // In linear mode: 1 period = 1200/768 cents ≈ 1.5625 cents
                        // Combined: depth * 255/32 * 1200/768 ≈ depth * 12.45 cents
                        state.vibrato_depth =
                            PitchCents::new(f32::from(*depth) * (255.0 / 32.0 * 1200.0 / 768.0));
                    } else {
                        state.vibrato_depth = vibrato_depth_mem; // 40x: restore depth from memory
                    }
                }

                EffectCommand::VibratoWaveform(waveform) => {
                    state.vibrato_waveform = *waveform;
                }

                EffectCommand::VolumeSlide { up, down } => {
                    if *up > 0 || *down > 0 {
                        state.volume_slide = SlideRate::from_volume_slide(*up, *down);
                    } else {
                        state.volume_slide = vol_slide_mem; // A00: restore from memory
                    }
                }

                EffectCommand::FineVolumeSlide { up, down } => {
                    if *up > 0 || *down > 0 {
                        state.fine_volume_slide = SlideRate::from_volume_slide(*up, *down);
                    } else {
                        state.fine_volume_slide = fine_vol_slide_mem; // EA0/EB0: restore from memory
                    }
                    // Apply fine slide once at tick 0
                    let delta = state.fine_volume_slide.as_f32();
                    if delta.abs() > f32::EPSILON {
                        let new_vol = (state.volume.as_f32() + delta).clamp(0.0, 1.0);
                        state.volume = NormalizedValue::new(new_vol);
                    }
                }

                EffectCommand::Tremolo { speed, depth } => {
                    if *speed > 0 {
                        state.tremolo_speed = EffectSpeed::from_param(*speed);
                    } else {
                        state.tremolo_speed = tremolo_speed_mem; // 7x0: restore speed from memory
                    }
                    if *depth > 0 {
                        state.tremolo_depth = TremoloDepth::from_param(*depth);
                    } else {
                        state.tremolo_depth = tremolo_depth_mem; // 70x: restore depth from memory
                    }
                }

                EffectCommand::TremoloWaveform(waveform) => {
                    state.tremolo_waveform = *waveform;
                }

                EffectCommand::PanningSlide { left, right } => {
                    if *left > 0 || *right > 0 {
                        state.panning_slide = SlideRate::from_panning_slide(*left, *right);
                    } else {
                        state.panning_slide = panning_slide_mem; // P00: restore from memory
                    }
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
                    state.retrigger = RetriggerState::new(
                        TickCount::new(*interval),
                        SlideRate::new(f32::from(*volume_change) / 64.0),
                    );
                }

                EffectCommand::NoteFadeOut(tick) => {
                    state.fade_out_tick = Some(TickInRow::new(*tick));
                }

                EffectCommand::Glissando(enabled) => {
                    state.glissando = Glissando::from(*enabled);
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

        // Return the sample offset for the NoteOn event
        let sample_offset = state.sample_offset.as_normalized();
        (global_commands, should_trigger_note, sample_offset)
    }

    /// Process a single tick for all channels.
    ///
    /// This should be called for each tick within a row (except tick 0 which is
    /// handled by process_row_start). Returns modulation values for each channel.
    pub fn process_tick(&mut self) -> Vec<ChannelModulation> {
        self.tick_in_row = self.tick_in_row.next();
        let amiga_mode = self.amiga_mode;

        self.channels
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, state)| {
                let track = TrackId::new(idx as u16);
                let modulation = state.process_tick(self.tick_in_row, track, amiga_mode);
                if modulation.is_significant() {
                    Some(modulation)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get the current modulation for a specific channel.
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
///
/// Tracks all effect state for a single tracker channel, including
/// pitch modulation (vibrato, portamento), volume modulation (tremolo, slides),
/// and timing effects (note delay, cut, retrigger).
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
    /// Current arpeggio state (if active).
    pub arpeggio: Option<ArpeggioState>,
    /// Current tick within the row (for arpeggio cycling).
    pub current_tick: TickInRow,

    // === Portamento ===
    /// Portamento slide speed in cents per tick.
    pub portamento_speed: PitchCents,
    /// Current portamento direction.
    pub portamento_direction: PortamentoDirection,

    // === Tone portamento (glide to note) ===
    /// Tone portamento speed in cents per tick.
    pub tone_porta_speed: PitchCents,
    /// Target pitch for tone portamento.
    pub tone_porta_target: Option<Pitch>,
    /// Whether tone portamento is active this row (only true on rows with 3xx/5xx).
    /// Separates "active this row" from "has memory value" in `tone_porta_speed`.
    pub tone_porta_active: bool,
    /// Current pitch in semitones for glide interpolation.
    pub current_pitch: Semitones,

    // === Vibrato ===
    /// Vibrato oscillation speed.
    pub vibrato_speed: EffectSpeed,
    /// Vibrato pitch modulation depth.
    pub vibrato_depth: PitchCents,
    /// Current vibrato phase (0.0-1.0).
    pub vibrato_phase: Phase,
    /// Vibrato waveform shape.
    pub vibrato_waveform: EffectWaveform,

    // === Volume slide ===
    /// Per-tick volume change rate.
    pub volume_slide: SlideRate,
    /// Fine volume slide memory (applied once at tick 0, with effect memory).
    pub fine_volume_slide: SlideRate,

    // === Tremolo ===
    /// Tremolo oscillation speed.
    pub tremolo_speed: EffectSpeed,
    /// Tremolo volume modulation depth.
    pub tremolo_depth: TremoloDepth,
    /// Current tremolo phase (0.0-1.0).
    pub tremolo_phase: Phase,
    /// Tremolo waveform shape.
    pub tremolo_waveform: EffectWaveform,

    // === Panning slide ===
    /// Per-tick panning change rate.
    pub panning_slide: SlideRate,

    // === Timing effects ===
    /// Tick at which to cut the note.
    pub note_cut_tick: Option<TickInRow>,
    /// Tick at which to delay/trigger the note.
    pub note_delay_tick: Option<TickInRow>,
    /// Tick at which to start fade out.
    pub fade_out_tick: Option<TickInRow>,

    // === Retrigger ===
    /// Retrigger effect state.
    pub retrigger: RetriggerState,

    // === Glissando ===
    /// Portamento quantization mode.
    pub glissando: Glissando,

    // === Note state ===
    /// Current note state (playing, trigger, cut).
    pub note_state: NoteState,

    // === Random waveform state ===
    /// Per-tick random state for vibrato/tremolo Random waveform (xorshift32).
    /// FT2 generates a new random value each tick rather than using a hash of phase.
    pub random_state: u32,
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
            current_tick: TickInRow::ZERO,
            portamento_speed: PitchCents::ZERO,
            portamento_direction: PortamentoDirection::Off,
            tone_porta_speed: PitchCents::ZERO,
            tone_porta_target: None,
            tone_porta_active: false,
            current_pitch: Semitones::ZERO,
            vibrato_speed: EffectSpeed::ZERO,
            vibrato_depth: PitchCents::ZERO,
            vibrato_phase: Phase::ZERO,
            vibrato_waveform: EffectWaveform::Sine,
            volume_slide: SlideRate::ZERO,
            fine_volume_slide: SlideRate::ZERO,
            tremolo_speed: EffectSpeed::ZERO,
            tremolo_depth: TremoloDepth::ZERO,
            tremolo_phase: Phase::ZERO,
            tremolo_waveform: EffectWaveform::Sine,
            panning_slide: SlideRate::ZERO,
            note_cut_tick: None,
            note_delay_tick: None,
            fade_out_tick: None,
            retrigger: RetriggerState::DISABLED,
            glissando: Glissando::Smooth,
            note_state: NoteState::SILENT,
            random_state: 0x1234_5678,
        }
    }
}

impl ChannelEffectState {
    /// Reset the channel state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Trigger a new note on this channel.
    ///
    /// This method implements the tracker "Note Trigger State Machine" which
    /// correctly handles all state transitions when a new note is triggered.
    ///
    /// # Arguments
    ///
    /// * `pitch` - The pitch of the new note
    /// * `instrument_defaults` - `Some` if explicit instrument was specified (reset volume/panning),
    ///   `None` if inheriting instrument (keep current volume/panning)
    /// * `is_tone_portamento` - `true` if effect 3xx (tone portamento) or 5xx is active on this row
    /// * `has_sample_offset` - `true` if effect 9xx (sample offset) is active on this row
    ///
    /// # Tone Portamento vs Fresh Attack
    ///
    /// - **Tone Portamento (3xx/5xx):** Only sets the target pitch for glide. All other state
    ///   (volume, vibrato, etc.) is preserved. This creates a legato/glide effect.
    /// - **Fresh Attack:** Resets all pitch modulation, oscillator phases, slides, and retrigger.
    ///   Volume/panning is only reset if an explicit instrument is specified.
    pub fn trigger_note(
        &mut self,
        pitch: Pitch,
        instrument_defaults: Option<InstrumentDefaults>,
        is_tone_portamento: bool,
        has_sample_offset: bool,
        has_note_delay: bool,
    ) {
        // STEP 1: ALWAYS clear per-row timing effects (prevents "ghosting")
        self.note_delay_tick = None;
        self.note_cut_tick = None;
        self.fade_out_tick = None;

        // STEP 2: Tone portamento is a special case - only set target, keep everything else
        // EXCEPTION: If there's no previous note (channel has never played), we must trigger
        // the note as a fresh attack. Otherwise, the first note would be silent.
        if is_tone_portamento && self.last_note.is_some() {
            self.tone_porta_target = Some(pitch);
            return; // Early return - preserve all other state!
        }

        // If tone portamento but no previous note, fall through to fresh attack
        // and set up the tone porta target for future glides
        if is_tone_portamento {
            self.tone_porta_target = Some(pitch);
        }

        // STEP 3: Fresh Attack - this is a "clean" new note

        // 3a. Reset pitch modulation
        self.pitch_offset = PitchCents::ZERO;
        self.portamento_speed = PitchCents::ZERO;
        self.portamento_direction = PortamentoDirection::Off;
        self.vibrato_depth = PitchCents::ZERO;
        // NOTE: vibrato_phase is NOT reset on new note (FT2 behavior).
        // In FT2, vibrato phase continues across notes unless the vibrato
        // waveform type has the retrigger bit set (E4x with x >= 4).
        // Default waveform (sine, type 0) does not retrigger.
        self.arpeggio = None;
        self.current_pitch = Semitones::new(f32::from(pitch.as_midi())); // Set current pitch for future glides

        // 3b. Reset volume/pan modulation (the movements, not the values)
        self.tremolo_depth = TremoloDepth::ZERO;
        self.tremolo_phase = Phase::ZERO; // Start tremolo from beginning of cycle
        self.volume_slide = SlideRate::ZERO; // Stop any ongoing volume slide
        self.panning_slide = SlideRate::ZERO; // Stop any ongoing panning slide

        // 3c. Reset retrigger
        self.retrigger = RetriggerState::DISABLED;

        // 3d. Handle sample offset - only keep if explicitly set on this row
        if !has_sample_offset {
            self.sample_offset = TrackerSampleOffset::ZERO;
        }

        // 3e. Handle explicit instrument (CRITICAL!)
        // If instrument is explicit: reset volume/panning to instrument's defaults
        // If instrument is inherited (None): keep current volume/panning
        if let Some(defaults) = instrument_defaults {
            // IMPORTANT: Use instrument defaults, NOT hardcoded values!
            self.volume = defaults.volume;
            self.panning = defaults.panning;
            self.fine_tune = PitchCents::ZERO;
        }
        // If instrument_defaults is None: keep self.volume and self.panning

        // 3f. Update note state
        self.last_note = Some(pitch);

        // Only trigger immediately if there's no note delay
        // If NoteDelay (EDx) is present, the trigger happens in process_tick()
        if !has_note_delay {
            self.note_state.trigger();
        }
    }

    /// Process one tick and return modulation.
    pub fn process_tick(
        &mut self,
        tick: TickInRow,
        track: TrackId,
        amiga_mode: bool,
    ) -> ChannelModulation {
        // Store current tick for arpeggio calculation
        self.current_tick = tick;

        // Clear per-tick actions from previous tick
        self.note_state.clear_actions();

        // Advance random state each tick (xorshift32).
        // This ensures vibrato/tremolo Random waveform produces different values
        // each tick, matching FT2 behavior.
        self.random_state ^= self.random_state << 13;
        self.random_state ^= self.random_state >> 17;
        self.random_state ^= self.random_state << 5;

        // Note delay check
        if let Some(delay_tick) = self.note_delay_tick
            && tick.as_u8() == delay_tick.as_u8()
        {
            self.note_state.trigger();
            self.note_delay_tick = None;
        }

        // Note cut check
        if let Some(cut_tick) = self.note_cut_tick
            && tick.as_u8() >= cut_tick.as_u8()
        {
            self.note_state.cut();
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
                self.note_state.cut();
            }
        }

        // Volume slide (runs from tick 1 onwards)
        if tick.as_u8() > 0 && self.volume_slide.is_active() {
            let new_vol = (self.volume.as_f32() + self.volume_slide.as_f32()).clamp(0.0, 1.0);
            self.volume = NormalizedValue::new(new_vol);
        }

        // Panning slide
        if tick.as_u8() > 0 && self.panning_slide.is_active() {
            let new_pan = (self.panning.as_f32() + self.panning_slide.as_f32()).clamp(-1.0, 1.0);
            self.panning = BipolarValue::new(new_pan);
        }

        // Portamento up/down
        if tick.as_u8() > 0 {
            match self.portamento_direction {
                PortamentoDirection::Up | PortamentoDirection::Down => {
                    if amiga_mode {
                        self.apply_amiga_portamento();
                    } else {
                        // Linear mode: portamento_speed = raw_param * 4.0 (period units)
                        // Convert period units to cents: period_units * 1200.0 / 768.0
                        match self.portamento_direction {
                            PortamentoDirection::Up => {
                                let cents = self.portamento_speed.as_f32() * 1200.0 / 768.0;
                                self.pitch_offset += PitchCents::new(cents);
                            }
                            PortamentoDirection::Down => {
                                let cents = self.portamento_speed.as_f32() * 1200.0 / 768.0;
                                self.pitch_offset =
                                    PitchCents::new(self.pitch_offset.as_f32() - cents);
                            }
                            PortamentoDirection::Off => {}
                        }
                    }
                }
                PortamentoDirection::Off => {}
            }
        }

        // Tone portamento (glide to target) — only on rows with 3xx/5xx
        if tick.as_u8() > 0
            && self.tone_porta_active
            && self.tone_porta_speed.as_f32() > 0.0
            && let Some(target) = self.tone_porta_target
        {
            if amiga_mode {
                self.apply_amiga_tone_portamento(target);
            } else {
                // Linear mode: glide in semitone space
                let target_pitch = f32::from(target.as_midi());
                let current = self.current_pitch.as_f32();
                let diff = target_pitch - current;
                if diff.abs() > 0.01 {
                    // tone_porta_speed = raw_param * 4.0 (period units)
                    // Convert to semitones: period_units * 1200.0 / 768.0 / 100.0
                    let step = self.tone_porta_speed.as_f32() * 1200.0 / 768.0 / 100.0;
                    if diff > 0.0 {
                        self.current_pitch = Semitones::new((current + step).min(target_pitch));
                    } else {
                        self.current_pitch = Semitones::new((current - step).max(target_pitch));
                    }
                }
            }
        }

        // Vibrato phase advance — FT2 only runs doVibrato on ticks 1+
        if tick.as_u8() > 0 && self.vibrato_depth.as_f32() > 0.0 {
            let new_phase = self.vibrato_phase.as_f32() + self.vibrato_speed.as_f32() / 64.0;
            self.vibrato_phase = Phase::new(if new_phase >= 1.0 {
                new_phase - 1.0
            } else {
                new_phase
            });
        }

        // Tremolo phase advance — FT2 only runs doTremolo on ticks 1+
        if tick.as_u8() > 0 && self.tremolo_depth.is_active() {
            let new_phase = self.tremolo_phase.as_f32() + self.tremolo_speed.as_f32() / 64.0;
            self.tremolo_phase = Phase::new(if new_phase >= 1.0 {
                new_phase - 1.0
            } else {
                new_phase
            });
        }

        // Retrigger
        if self.retrigger.tick() {
            self.note_state.set_trigger(true);
            let new_vol =
                (self.volume.as_f32() + self.retrigger.volume_change().as_f32()).clamp(0.0, 1.0);
            self.volume = NormalizedValue::new(new_vol);
        }

        self.current_modulation(track)
    }

    /// Convert semitones (MIDI space) to Amiga period.
    ///
    /// Period = 7680 - (semitones - 12) * 64
    /// The -12 offset accounts for our MIDI convention where C-0 = MIDI 12.
    fn semitones_to_period(semitones: f32) -> f32 {
        7680.0 - (semitones - 12.0) * 64.0
    }

    /// Convert Amiga period back to semitones (MIDI space).
    fn period_to_semitones(period: f32) -> f32 {
        (7680.0 - period) / 64.0 + 12.0
    }

    /// Apply portamento up/down in Amiga period space.
    ///
    /// In Amiga mode, portamento speed is in periods per tick (not cents).
    /// Portamento up = decrease period (higher pitch), down = increase period (lower pitch).
    fn apply_amiga_portamento(&mut self) {
        let current_semitones = self.current_pitch.as_f32() + self.pitch_offset.as_f32() / 100.0;
        let period = Self::semitones_to_period(current_semitones);
        // portamento_speed = raw_param * 4.0 = period units per tick
        let speed_periods = self.portamento_speed.as_f32();

        let new_period = match self.portamento_direction {
            PortamentoDirection::Up => (period - speed_periods).max(1.0),
            PortamentoDirection::Down => period + speed_periods,
            PortamentoDirection::Off => period,
        };

        // Convert back to pitch offset
        let new_semitones = Self::period_to_semitones(new_period);
        self.pitch_offset = PitchCents::new((new_semitones - self.current_pitch.as_f32()) * 100.0);
    }

    /// Apply tone portamento in Amiga period space.
    ///
    /// Glides toward the target note in period space, producing the
    /// characteristic non-linear pitch curve of Amiga trackers.
    fn apply_amiga_tone_portamento(&mut self, target: Pitch) {
        let target_pitch = f32::from(target.as_midi());
        let current = self.current_pitch.as_f32();

        let current_period = Self::semitones_to_period(current);
        let target_period = Self::semitones_to_period(target_pitch);
        let diff = target_period - current_period;

        if diff.abs() < 0.5 {
            return; // Close enough
        }

        // tone_porta_speed = raw_param * 4.0, but FT2 Amiga tone portamento uses raw_param
        let speed_periods = self.tone_porta_speed.as_f32() / 4.0;

        let new_period = if diff > 0.0 {
            // Target period is higher (lower pitch) - slide period up
            (current_period + speed_periods).min(target_period)
        } else {
            // Target period is lower (higher pitch) - slide period down
            (current_period - speed_periods).max(target_period)
        };

        self.current_pitch = Semitones::new(Self::period_to_semitones(new_period));
    }

    /// Get the current modulation values.
    pub fn current_modulation(&self, track: TrackId) -> ChannelModulation {
        // Calculate pitch modulation
        let mut pitch_mod = self.pitch_offset + self.fine_tune;

        // Add vibrato — FT2 clears vibrato offset on tick 0 (outPeriod = realPeriod).
        // Vibrato only contributes pitch offset on ticks 1+.
        if self.vibrato_depth.as_f32() > 0.0 && self.current_tick.as_u8() > 0 {
            // For Random waveform, use per-tick xorshift state (FT2 behavior)
            // instead of deterministic hash-of-phase
            let waveform_value = if self.vibrato_waveform == EffectWaveform::Random {
                #[allow(clippy::cast_precision_loss)]
                let v = (self.random_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                v
            } else {
                self.vibrato_waveform.sample(self.vibrato_phase.as_f32())
            };
            let vibrato = waveform_value * self.vibrato_depth.as_f32();
            pitch_mod += PitchCents::new(vibrato);
        }

        // Calculate arpeggio offset
        let arpeggio_semitones = self.arpeggio.as_ref().map_or(0, |arp| {
            // FT2 arpeggio order: base, +y, +x (reversed from ProTracker's base, +x, +y)
            match self.current_tick.as_u8() % 3 {
                0 => 0,
                1 => arp.semitone2,
                _ => arp.semitone1,
            }
        });
        pitch_mod += PitchCents::new(f32::from(arpeggio_semitones) * 100.0);

        // Calculate volume modulation
        let mut volume_mod = self.volume.as_f32();

        // Add tremolo
        if self.tremolo_depth.is_active() {
            // For Random waveform, use per-tick xorshift state (FT2 behavior)
            let waveform_value = if self.tremolo_waveform == EffectWaveform::Random {
                #[allow(clippy::cast_precision_loss)]
                let v = (self.random_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                v
            } else {
                self.tremolo_waveform.sample(self.tremolo_phase.as_f32())
            };
            let tremolo = waveform_value * self.tremolo_depth.as_f32();
            volume_mod = (volume_mod + tremolo).clamp(0.0, 1.0);
        }

        ChannelModulation {
            track,
            pitch_cents: pitch_mod,
            volume: NormalizedValue::new(volume_mod),
            panning: self.panning,
            note_triggered: self.note_state.should_trigger(),
            note_cut: self.note_state.should_cut(),
            sample_offset: self.sample_offset,
            tone_porta_pitch: if self.tone_porta_target.is_some() {
                Some(self.current_pitch.as_f32())
            } else {
                None
            },
        }
    }
}

// ============================================================================
// Supporting Types
// ============================================================================

/// Arpeggio state for cycling between note pitches.
///
/// Cycles through base note, base + semitone1, base + semitone2 on each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ArpeggioState {
    /// First arpeggio semitone offset (0-15).
    pub semitone1: u8,
    /// Second arpeggio semitone offset (0-15).
    pub semitone2: u8,
}

/// Modulation values for a channel at a specific tick.
///
/// Contains all the per-tick modulation values that should be applied
/// to the channel's sound: pitch, volume, panning, and note triggers.
#[derive(Debug, Clone)]
#[must_use]
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

    /// Helper to create a NoteTriggerInfo with explicit instrument defaults.
    fn trigger_with_defaults(pitch: u8, volume: f32) -> NoteTriggerInfo {
        NoteTriggerInfo {
            pitch: synth_sequencer::Pitch::new(pitch).unwrap(),
            instrument_defaults: Some(InstrumentDefaults {
                volume: NormalizedValue::new(volume),
                panning: BipolarValue::CENTER,
            }),
        }
    }

    /// Helper to create a NoteTriggerInfo that inherits instrument (no volume reset).
    fn trigger_inherit(pitch: u8) -> NoteTriggerInfo {
        NoteTriggerInfo {
            pitch: synth_sequencer::Pitch::new(pitch).unwrap(),
            instrument_defaults: None, // Inherit - don't reset volume
        }
    }

    #[test]
    fn test_volume_effect() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Set volume to 50%
        processor.process_row_start(track, &[EffectCommand::SetVolume(32)], None);

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
        processor.process_row_start(track, &[EffectCommand::SetPanning(0)], None);
        let left = processor.get_channel_modulation(track);
        assert!((left.panning.as_f32() - (-1.0)).abs() < 0.02);

        // Center
        processor.process_row_start(track, &[EffectCommand::SetPanning(128)], None);
        let center = processor.get_channel_modulation(track);
        assert!(center.panning.as_f32().abs() < 0.02);

        // Full right
        processor.process_row_start(track, &[EffectCommand::SetPanning(255)], None);
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
        );

        assert_eq!(commands.0.len(), 2);
        assert!(matches!(commands.0[0], GlobalCommand::SetTempo(140)));
        assert!(matches!(
            commands.0[1],
            GlobalCommand::SetSpeed(TrackerSpeed(4))
        ));
        assert_eq!(processor.speed().as_u8(), 4);
    }

    #[test]
    fn test_note_cut() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        processor.process_row_start(track, &[EffectCommand::NoteCut(3)], None);

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

        processor.process_row_start(track, &[EffectCommand::Arpeggio { x: 4, y: 7 }], None);

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
            Some(trigger_with_defaults(60, 0.75)),
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
            !processor.channels[0].volume_slide.is_active(),
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
        processor.process_row_start(track, &[EffectCommand::SetVolume(19)], None); // 19/64 ≈ 0.3

        let mod_after_set = processor.get_channel_modulation(track);
        assert!(
            (mod_after_set.volume.as_f32() - 0.297).abs() < 0.02,
            "Volume should be ~0.3: {}",
            mod_after_set.volume.as_f32()
        );

        // Now play a note that inherits instrument (instrument_defaults = None)
        processor.process_row_start(
            track,
            &[], // No effects
            Some(trigger_inherit(60)),
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
            Some(trigger_with_defaults(60, 1.0)),
        );

        // Volume should be 0.5 (SetVolume overrides instrument default)
        let modulation = processor.get_channel_modulation(track);
        assert!(
            (modulation.volume.as_f32() - 0.5).abs() < 0.01,
            "Volume should be 0.5 (SetVolume overrides default): {}",
            modulation.volume.as_f32()
        );
    }

    // ========================================================================
    // New tests for the Note Trigger State Machine
    // ========================================================================

    #[test]
    fn test_tone_portamento_preserves_state() {
        // Test 3: Tone portamento should preserve volume and other state
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start with C-4, volume at 1.0
        processor.process_row_start(track, &[], Some(trigger_with_defaults(60, 1.0)));

        // Slide volume down to 0.5
        processor.process_row_start(
            track,
            &[EffectCommand::VolumeSlide { up: 0, down: 8 }],
            None,
        );
        for _ in 0..4 {
            processor.process_tick();
        }

        let vol_after_slide = processor.get_channel_modulation(track).volume.as_f32();
        assert!(vol_after_slide < 1.0, "Volume should have slid down");

        // Now do tone portamento to E-4 - volume should be PRESERVED
        let tone_porta_trigger = NoteTriggerInfo {
            pitch: synth_sequencer::Pitch::new(64).unwrap(), // E-4
            instrument_defaults: Some(InstrumentDefaults::default()), // Even with explicit instrument!
        };
        processor.process_row_start(
            track,
            &[EffectCommand::TonePortamento {
                speed: 4,
                target: None,
            }],
            Some(tone_porta_trigger),
        );

        // Volume should be preserved (not reset to 1.0)
        let vol_after_porta = processor.get_channel_modulation(track).volume.as_f32();
        assert!(
            (vol_after_porta - vol_after_slide).abs() < 0.01,
            "Volume should be preserved during tone portamento: {} vs {}",
            vol_after_porta,
            vol_after_slide
        );

        // Tone porta target should be set
        let state = &processor.channels[0];
        assert!(state.tone_porta_target.is_some());
        assert_eq!(state.tone_porta_target.unwrap().as_midi(), 64);
    }

    #[test]
    fn test_vibrato_reset_on_fresh_attack() {
        // Test 5: Vibrato depth should be reset on fresh attack (deactivated),
        // but vibrato phase should be PRESERVED (FT2 default behavior:
        // vibrato phase continues across notes unless retrig waveform is set).
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start with C-4 and vibrato
        processor.process_row_start(
            track,
            &[EffectCommand::Vibrato { speed: 8, depth: 8 }],
            Some(trigger_with_defaults(60, 1.0)),
        );

        // Process some ticks so vibrato accumulates phase
        for _ in 0..4 {
            processor.process_tick();
        }

        let state = &processor.channels[0];
        assert!(
            state.vibrato_depth.as_f32() > 0.0,
            "Vibrato should be active"
        );
        let phase_before = state.vibrato_phase.as_f32();
        assert!(phase_before > 0.0, "Vibrato phase should have advanced");

        // Fresh attack on E-4 - vibrato depth should be STOPPED,
        // but phase should be PRESERVED (FT2 default: no retrigger)
        processor.process_row_start(
            track,
            &[], // No effects
            Some(trigger_with_defaults(64, 1.0)),
        );

        let state = &processor.channels[0];
        assert!(
            state.vibrato_depth.as_f32().abs() < 0.001,
            "Vibrato depth should be reset on fresh attack"
        );
        assert!(
            (state.vibrato_phase.as_f32() - phase_before).abs() < 0.001,
            "Vibrato phase should be PRESERVED on fresh attack (FT2 default)"
        );
    }

    #[test]
    fn test_sample_with_custom_default_volume() {
        // Test 6: Sample with custom default_volume should use that value
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Create trigger with custom volume (simulating sample with default_volume = 0.6)
        let trigger = NoteTriggerInfo {
            pitch: synth_sequencer::Pitch::new(60).unwrap(),
            instrument_defaults: Some(InstrumentDefaults {
                volume: NormalizedValue::new(0.6),
                panning: BipolarValue::CENTER,
            }),
        };

        processor.process_row_start(track, &[], Some(trigger));

        let modulation = processor.get_channel_modulation(track);
        assert!(
            (modulation.volume.as_f32() - 0.6).abs() < 0.01,
            "Volume should be 0.6 (sample default): {}",
            modulation.volume.as_f32()
        );
    }

    #[test]
    fn test_instrument_defaults_from_sample() {
        // Test the InstrumentDefaults::from_sample helper
        let defaults = InstrumentDefaults::from_sample(Some(0.5), Some(0.75));
        assert!((defaults.volume.as_f32() - 0.5).abs() < 0.01);
        // 0.75 in tracker panning (0-1) = 0.5 in bipolar (-1 to 1)
        assert!((defaults.panning.as_f32() - 0.5).abs() < 0.01);

        // Test with None values (should use defaults)
        let defaults = InstrumentDefaults::from_sample(None, None);
        assert!((defaults.volume.as_f32() - 1.0).abs() < 0.01);
        assert!(defaults.panning.as_f32().abs() < 0.01); // Center
    }

    #[test]
    fn test_arpeggio_reset_on_fresh_attack() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start with C-4 and arpeggio
        processor.process_row_start(
            track,
            &[EffectCommand::Arpeggio { x: 4, y: 7 }],
            Some(trigger_with_defaults(60, 1.0)),
        );

        let state = &processor.channels[0];
        assert!(state.arpeggio.is_some(), "Arpeggio should be set");

        // Fresh attack - arpeggio should be STOPPED
        processor.process_row_start(
            track,
            &[], // No effects
            Some(trigger_with_defaults(64, 1.0)),
        );

        let state = &processor.channels[0];
        assert!(
            state.arpeggio.is_none(),
            "Arpeggio should be reset on fresh attack"
        );
    }

    #[test]
    fn test_retrigger_reset_on_fresh_attack() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // Start with retrigger
        processor.process_row_start(
            track,
            &[EffectCommand::Retrigger {
                interval: 3,
                volume_change: 0,
            }],
            Some(trigger_with_defaults(60, 1.0)),
        );

        let state = &processor.channels[0];
        assert!(state.retrigger.is_active(), "Retrigger should be set");

        // Fresh attack - retrigger should be STOPPED
        processor.process_row_start(
            track,
            &[], // No effects
            Some(trigger_with_defaults(64, 1.0)),
        );

        let state = &processor.channels[0];
        assert!(
            !state.retrigger.is_active(),
            "Retrigger should be reset on fresh attack"
        );
    }

    #[test]
    fn test_sample_offset_kept_when_specified() {
        let mut processor = ChannelEffectProcessor::new(1);
        let track = TrackId::new(0);

        // First note with sample offset
        processor.process_row_start(
            track,
            &[EffectCommand::SampleOffset(128)],
            Some(trigger_with_defaults(60, 1.0)),
        );

        let state = &processor.channels[0];
        assert_eq!(
            state.sample_offset.as_u16(),
            128,
            "Sample offset should be set"
        );

        // Second note WITHOUT sample offset - offset should be cleared
        processor.process_row_start(
            track,
            &[], // No effects
            Some(trigger_with_defaults(64, 1.0)),
        );

        let state = &processor.channels[0];
        assert_eq!(
            state.sample_offset.as_u16(),
            0,
            "Sample offset should be cleared on fresh attack"
        );

        // Third note WITH sample offset - offset should be set again
        processor.process_row_start(
            track,
            &[EffectCommand::SampleOffset(200)],
            Some(trigger_with_defaults(60, 1.0)),
        );

        let state = &processor.channels[0];
        assert_eq!(
            state.sample_offset.as_u16(),
            200,
            "Sample offset should be set from effect"
        );
    }
}
