//! Effect-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{
    BeatDivision, BipolarValue, BitDepth, Decibels, Hertz, MidiNote, Milliseconds, NormalizedValue,
    Ratio, Seconds, Semitones, VoiceCount,
};

use super::FftSizeOption;

// ============================================================================
// EFFECT MODE ENUMS
// ============================================================================

/// Delay modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DelayMode {
    #[default]
    Mono,
    Stereo,
    PingPong,
}

impl DelayMode {
    pub const ALL: [Self; 3] = [Self::Mono, Self::Stereo, Self::PingPong];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::PingPong => "Ping-Pong",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Mono => "Single delay line summed to both channels.",
            Self::Stereo => "Independent left and right delay times for a wide image.",
            Self::PingPong => "Echoes bounce alternately between left and right.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::PingPong => "ping_pong",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

/// Distortion types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DistortionMode {
    #[default]
    SoftClip,
    HardClip,
    Tube,
    Foldback,
    Bitcrush,
    /// Variable hardness clipping (arctan-based, continuously adjustable).
    /// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Effects/104-variable-hardness-clipping-function.rst>
    VariableClip,
}

impl DistortionMode {
    pub const ALL: [Self; 6] = [
        Self::SoftClip,
        Self::HardClip,
        Self::Tube,
        Self::Foldback,
        Self::Bitcrush,
        Self::VariableClip,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SoftClip => "Soft Clip",
            Self::HardClip => "Hard Clip",
            Self::Tube => "Tube",
            Self::Foldback => "Foldback",
            Self::Bitcrush => "Bitcrush",
            Self::VariableClip => "Variable",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::SoftClip => "Smooth tanh-style saturation — warm, gentle overdrive.",
            Self::HardClip => "Abrupt clipping at the ceiling — harsh, buzzy edge.",
            Self::Tube => "Asymmetric tube-style saturation — warm, even-harmonic grit.",
            Self::Foldback => "Folds peaks back on themselves — aggressive, ring-mod-like timbres.",
            Self::Bitcrush => "Reduces bit depth — quantized, lo-fi digital distortion.",
            Self::VariableClip => {
                "Arctan clipping with continuously adjustable hardness — soft to hard via Drive."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SoftClip => "soft_clip",
            Self::HardClip => "hard_clip",
            Self::Tube => "tube",
            Self::Foldback => "foldback",
            Self::Bitcrush => "bitcrush",
            Self::VariableClip => "variable_clip",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

// ============================================================================
// DELAY PARAMETER ENUM (with typed values)
// ============================================================================

/// Delay effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DelayParam {
    Mode(DelayMode),
    Time(Seconds),
    TimeLeft(Seconds),
    TimeRight(Seconds),
    Feedback(NormalizedValue),
    Mix(NormalizedValue),
    Damping(NormalizedValue),
    TempoSync(bool),
    SyncDivision(BeatDivision),
}

impl Default for DelayParam {
    fn default() -> Self {
        Self::Time(Seconds::new(0.25))
    }
}

// ============================================================================
// REVERB PARAMETER ENUM (with typed values)
// ============================================================================

/// Reverb effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReverbParam {
    RoomSize(NormalizedValue),
    PreDelay(Seconds),
    Damping(NormalizedValue),
    Width(NormalizedValue),
    Mix(NormalizedValue),
    /// Decay time (0.0 = short, 1.0 = long). FDN reverb.
    Decay(NormalizedValue),
    /// Diffusion amount (0.0 = sparse, 1.0 = dense). FDN reverb.
    Diffusion(NormalizedValue),
    /// Low-cut frequency for reverb tail. FDN reverb.
    LowCut(Hertz),
}

impl Default for ReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.3))
    }
}

// ============================================================================
// DISTORTION PARAMETER ENUM (with typed values)
// ============================================================================

/// Distortion effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DistortionParam {
    Mode(DistortionMode),
    Drive(NormalizedValue),
    Tone(NormalizedValue),
    Mix(NormalizedValue),
    /// Bit depth for bitcrush mode (1-16 bits).
    BitDepth(BitDepth),
}

impl Default for DistortionParam {
    fn default() -> Self {
        Self::Drive(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// CHORUS PARAMETER ENUM (with typed values)
// ============================================================================

/// Chorus effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChorusParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Delay(Milliseconds),
    Feedback(NormalizedValue),
    Mix(NormalizedValue),
    Voices(VoiceCount),
}

impl Default for ChorusParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// PHASER PARAMETER ENUM (with typed values)
// ============================================================================

/// Phaser effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PhaserParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Feedback(BipolarValue),
    Stages(u8),
    CenterFreq(Hertz),
    Mix(NormalizedValue),
}

impl Default for PhaserParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// FLANGER PARAMETER ENUM (with typed values)
// ============================================================================

/// Flanger effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FlangerParam {
    Rate(Hertz),
    Depth(NormalizedValue),
    Feedback(BipolarValue),
    Delay(Milliseconds),
    Mix(NormalizedValue),
}

impl Default for FlangerParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// COMPRESSOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Compressor effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CompressorParam {
    Threshold(Decibels),
    Ratio(Ratio),
    Attack(Milliseconds),
    Release(Milliseconds),
    Makeup(Decibels),
    Mix(NormalizedValue),
    /// Sidechain enabled (use external sidechain input for detection).
    SidechainEnabled(bool),
    /// Sidechain high-pass filter frequency (20-500 Hz).
    SidechainFilter(Hertz),
}

impl Default for CompressorParam {
    fn default() -> Self {
        Self::Threshold(Decibels::new(-20.0))
    }
}

/// Transient-shaper effect parameter with typed value.
///
/// A differential-envelope transient designer: it boosts or cuts a signal's
/// attack (onset) and sustain (body/tail) independently, without touching the
/// underlying amplitude — useful for adding punch to reconstructed drums whose
/// detailed amplitude automation must stay intact.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TransientShaperParam {
    /// Onset gain in dB (negative softens the attack, positive adds punch).
    Attack(Decibels),
    /// Body/tail gain in dB (negative tightens/gates, positive lengthens).
    Sustain(Decibels),
    /// Overall effect amount (0 = bypass-equivalent, 1 = full shaping).
    Sensitivity(NormalizedValue),
    /// Detection window in ms — how quickly the transient detector responds.
    Window(Milliseconds),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl Default for TransientShaperParam {
    fn default() -> Self {
        Self::Attack(Decibels::new(0.0))
    }
}

// ============================================================================
// EQ PARAMETER ENUM (with typed values)
// ============================================================================

/// EQ effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EqParam {
    LowFreq(Hertz),
    LowGain(Decibels),
    MidFreq(Hertz),
    MidGain(Decibels),
    MidQ(NormalizedValue),
    HighFreq(Hertz),
    HighGain(Decibels),
    Mix(NormalizedValue),
}

impl Default for EqParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// BBD DELAY PARAMETER ENUM (with typed values)
// ============================================================================

/// BBD (Bucket Brigade Device) analog delay parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BbdDelayParam {
    Time(Seconds),
    Feedback(NormalizedValue),
    Tone(NormalizedValue),
    WowFlutter(NormalizedValue),
    ClockNoise(NormalizedValue),
    Mix(NormalizedValue),
}

impl Default for BbdDelayParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::CENTER)
    }
}

// ============================================================================
// MID-SIDE PROCESSING PARAMETER ENUM (with typed values)
// ============================================================================

/// Mid-Side processing parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MidSideParam {
    /// Stereo width (0=mono, 1=normal, 2=extra wide).
    Width(NormalizedValue),
    /// Mid channel gain.
    MidGain(Decibels),
    /// Side channel gain.
    SideGain(Decibels),
    /// Stereo field rotation angle in degrees (-180 to +180).
    /// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Effects/255-stereo-field-rotation-via-transformation-matrix.rst>
    Rotation(BipolarValue),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl Default for MidSideParam {
    fn default() -> Self {
        Self::Width(NormalizedValue::CENTER)
    }
}

// ============================================================================
// LIMITER PARAMETER ENUM (with typed values)
// ============================================================================

/// Brickwall limiter parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LimiterParam {
    /// Ceiling level.
    Ceiling(Decibels),
    /// Look-ahead time.
    LookAhead(Milliseconds),
    /// Release time.
    Release(Milliseconds),
    /// Dry/wet mix.
    Mix(NormalizedValue),
}

impl Default for LimiterParam {
    fn default() -> Self {
        Self::Ceiling(Decibels::new(-0.3))
    }
}

// ============================================================================
// ENSEMBLE CHORUS (BBD) PARAMETER ENUM
// ============================================================================

/// Juno-style BBD ensemble chorus parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EnsembleChorusParam {
    Rate(Hertz),
    Depth(Milliseconds),
    BaseDelay(Milliseconds),
    Mix(NormalizedValue),
    Tone(NormalizedValue),
    Noise(NormalizedValue),
    StereoWidth(NormalizedValue),
    Voices(VoiceCount),
}

impl Default for EnsembleChorusParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::CENTER)
    }
}

// ============================================================================
// SHIMMER REVERB PARAMETER ENUM
// ============================================================================

/// Shimmer reverb parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShimmerReverbParam {
    RoomSize(NormalizedValue),
    Decay(NormalizedValue),
    Damping(NormalizedValue),
    PreDelay(Seconds),
    PitchSemitones(Semitones),
    ShimmerMix(NormalizedValue),
    Mix(NormalizedValue),
}

impl Default for ShimmerReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.4))
    }
}

// ============================================================================
// GRANULAR FX PARAMETER ENUM
// ============================================================================

/// Granular FX parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GranularFxParam {
    BufferTime(Seconds),
    GrainSize(Milliseconds),
    Density(NormalizedValue),
    Position(NormalizedValue),
    PositionSpread(NormalizedValue),
    PitchSpread(NormalizedValue),
    PanSpread(NormalizedValue),
    Freeze(bool),
    Mix(NormalizedValue),
}

impl Default for GranularFxParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// SPECTRAL BLUR PARAMETER ENUM
// ============================================================================

/// Spectral blur/smear parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpectralBlurParam {
    FftSize(FftSizeOption),
    BlurTime(NormalizedValue),
    BlurFreq(NormalizedValue),
    Freeze(bool),
    Mix(NormalizedValue),
}

impl Default for SpectralBlurParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// MODAL RESONATOR PARAMETER ENUM
// ============================================================================

/// Modal resonator bank parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ModalResonatorParam {
    BaseNote(MidiNote),
    Spread(NormalizedValue),
    Modes(VoiceCount),
    Decay(NormalizedValue),
    Brightness(NormalizedValue),
    Mix(NormalizedValue),
}

impl Default for ModalResonatorParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// REVERSE/GATE REVERB MODE ENUMS
// ============================================================================

/// Playback mode for reverse/gate reverb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReverseGateMode {
    #[default]
    Reverse,
    Gate,
    Stutter,
}

impl ReverseGateMode {
    pub const ALL: [Self; 3] = [Self::Reverse, Self::Gate, Self::Stutter];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Reverse => "Reverse",
            Self::Gate => "Gate",
            Self::Stutter => "Stutter",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Reverse => {
                "Plays the reverb tail backwards — swelling, sucked-in reverse reverb."
            }
            Self::Gate => "Cuts the reverb tail abruptly — the classic gated-reverb snare effect.",
            Self::Stutter => "Re-triggers the captured tail in slices — rhythmic stutter.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Reverse => "reverse",
            Self::Gate => "gate",
            Self::Stutter => "stutter",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

/// Trigger mode for reverse/gate reverb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReverseGateTrigger {
    #[default]
    Periodic,
    Threshold,
}

impl ReverseGateTrigger {
    pub const ALL: [Self; 2] = [Self::Periodic, Self::Threshold];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Periodic => "Periodic",
            Self::Threshold => "Threshold",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Periodic => "Re-captures the buffer at a fixed time interval.",
            Self::Threshold => "Re-captures when the input level crosses a threshold.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::Threshold => "threshold",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

// ============================================================================
// REVERSE/GATE REVERB PARAMETER ENUM
// ============================================================================

/// Reverse/gate reverb parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReverseGateReverbParam {
    WindowTime(Milliseconds),
    Mode(ReverseGateMode),
    Trigger(ReverseGateTrigger),
    Threshold(Decibels),
    GateTime(Milliseconds),
    Mix(NormalizedValue),
}

impl Default for ReverseGateReverbParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// TILT EQ PARAMETERS
// ============================================================================

/// Tilt EQ parameter with typed value.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Filters/267-simple-tilt-equalizer.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TiltEqParam {
    /// Tilt amount (-1.0 = dark, 0.0 = flat, 1.0 = bright)
    Tilt(BipolarValue),
    /// Center frequency (pivot point)
    CenterFreq(Hertz),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl Default for TiltEqParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// UNIVIBE PARAMETERS
// ============================================================================

/// Univibe effect parameter with typed value.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Effects/277-univox-univibe-emulator.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum UnivibeParam {
    /// LFO rate (speed of sweep)
    Rate(Hertz),
    /// Depth of modulation (0.0 to 1.0)
    Depth(NormalizedValue),
    /// Feedback amount (0.0 to 0.95)
    Feedback(NormalizedValue),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl Default for UnivibeParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.5))
    }
}

// ============================================================================
// CROSSOVER SPLITTER PARAMETERS
// ============================================================================

/// Crossover splitter parameter with typed value.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Filters/266-4th-order-linkwitz-riley-filters.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CrossoverParam {
    /// Crossover frequency (Hz)
    Frequency(Hertz),
    /// Low band gain
    LowGain(NormalizedValue),
    /// High band gain
    HighGain(NormalizedValue),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl Default for CrossoverParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}

// ============================================================================
// VOCODER PARAMETERS
// ============================================================================

/// LPC Vocoder parameter with typed value.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VocoderParam {
    /// LPC analysis order (4-32)
    Order(NormalizedValue),
    /// Analysis window size in milliseconds
    WindowSize(Milliseconds),
    /// Dry/wet mix
    Mix(NormalizedValue),
}

impl Default for VocoderParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
}
