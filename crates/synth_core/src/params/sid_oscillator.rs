//! SID oscillator (MOS 6581/8580 waveform generator) parameter types.
//!
//! Unlike the general `OscillatorParam`, the SID oscillator is register-driven:
//! waveforms are a combinable 4-bit mask (not an exclusive enum), pitch and
//! pulse width are raw chip registers (16-bit / 12-bit), and a per-frame
//! waveform-mask sequence can be programmed directly into the module (the SID
//! driver's waveform table). See `docs/sid-oscillator.md`.

use serde::{Deserialize, Serialize};

use crate::types::{Gain, Seconds};

/// Number of steps in the per-frame waveform-mask sequence.
pub const SID_SEQ_STEPS: usize = 16;

/// Maximum raw 16-bit SID frequency-register value — the single bound shared
/// by the param clamp, the module's apply paths, and the descriptor range.
pub const SID_FREQ_REG_MAX: u32 = 0xFFFF;

/// Maximum raw 12-bit SID pulse-width-register value.
pub const SID_PW_REG_MAX: u32 = 0xFFF;

/// Valid non-zero state of the SID oscillator's 23-bit noise LFSR.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SidNoiseSeed(u32);

impl SidNoiseSeed {
    pub const MAX: u32 = 0x7F_FFFF;
    pub const DEFAULT: Self = Self(Self::MAX);

    pub const fn new(value: u32) -> Self {
        Self(if value == 0 {
            1
        } else if value > Self::MAX {
            Self::MAX
        } else {
            value
        })
    }

    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl Default for SidNoiseSeed {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ============================================================================
// CHOICE ENUMS
// ============================================================================

/// SID chip model — selects the combined-waveform model and DAC curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SidModel {
    /// MOS 6581 (original, strongly non-linear DAC and bus behaviour).
    #[default]
    Mos6581,
    /// MOS 8580 (revised, near-linear DAC, cleaner waveform combining).
    Mos8580,
}

impl SidModel {
    pub const ALL: [Self; 2] = [Self::Mos6581, Self::Mos8580];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mos6581 => "6581",
            Self::Mos8580 => "8580",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Mos6581 => "6581",
            Self::Mos8580 => "8580",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Mos6581 => {
                "MOS 6581 — the original chip: non-linear DAC and gritty combined waveforms."
            }
            Self::Mos8580 => "MOS 8580 — the revision: near-linear DAC, cleaner combining.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Mos6581 => 0,
            Self::Mos8580 => 1,
        }
    }
}

/// SID master clock standard — sets the accumulator clock for pitch mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SidClock {
    /// PAL C64 clock (985 248 Hz).
    #[default]
    Pal,
    /// NTSC C64 clock (1 022 727 Hz).
    Ntsc,
}

impl SidClock {
    pub const ALL: [Self; 2] = [Self::Pal, Self::Ntsc];

    /// The chip clock frequency in Hz.
    #[must_use]
    pub fn clock_hz(self) -> f64 {
        match self {
            Self::Pal => 985_248.0,
            Self::Ntsc => 1_022_727.0,
        }
    }

    /// The driver frame rate in Hz (the waveform-sequence clock).
    #[must_use]
    pub fn frame_rate_hz(self) -> f32 {
        match self {
            Self::Pal => 50.0,
            Self::Ntsc => 60.0,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pal => "PAL",
            Self::Ntsc => "NTSC",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Pal => "pal",
            Self::Ntsc => "ntsc",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Pal => "PAL C64 master clock (985 248 Hz), 50 Hz driver frames.",
            Self::Ntsc => "NTSC C64 master clock (1 022 727 Hz), 60 Hz driver frames.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Pal => 0,
            Self::Ntsc => 1,
        }
    }
}

/// Clock-domain conversion quality (plan §2 strategy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SidQuality {
    /// Hybrid: host-rate accumulator with PolyBLEP on pure-waveform and sync
    /// edges; combined-waveform and noise selections auto-escalate to the 4x
    /// oversampled path their step structure needs.
    #[default]
    Fast,
    /// Always 4x oversampled generation with half-band decimation.
    High,
}

impl SidQuality {
    pub const ALL: [Self; 2] = [Self::Fast, Self::High];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::High => "High",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::High => "high",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Fast => {
                "Hybrid — PolyBLEP at host rate; combined/noise auto-oversample. Lowest CPU."
            }
            Self::High => "Always 4x oversampled with half-band decimation — cleanest.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Fast => 0,
            Self::High => 1,
        }
    }
}

// ============================================================================
// SID OSCILLATOR PARAMETER ENUM (with typed values)
// ============================================================================

/// SID oscillator parameter with typed value.
///
/// The waveform bits (`Triangle`/`Sawtooth`/`Pulse`/`Noise`) form a combinable
/// mask like the chip's control register; `FreqReg`/`PulseWidthReg` are the raw
/// 16-/12-bit registers. `SeqStep(i, mask)` holds step `i` of the per-frame
/// waveform-mask program (index is structural, like `MsegParam::SegmentTime`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SidOscillatorParam {
    /// Triangle waveform bit.
    Triangle(bool),
    /// Sawtooth waveform bit.
    Sawtooth(bool),
    /// Pulse waveform bit.
    Pulse(bool),
    /// Noise waveform bit.
    Noise(bool),
    /// Non-zero 23-bit state reloaded by module reset and TEST rising edges.
    NoiseSeed(SidNoiseSeed),
    /// Raw 16-bit SID frequency register (0-65535). Used when
    /// `TrackVoicePitch` is off (ring/sync-source tuning).
    FreqReg(u32),
    /// Whether `set_voice_pitch` drives `FreqReg` from the played note
    /// (default true). Off = hold the authored `FreqReg` (ring/sync source).
    TrackVoicePitch(bool),
    /// Raw 12-bit pulse-width register (0-4095).
    PulseWidthReg(u32),
    /// TEST bit: zeroes and holds the accumulator, reloads the noise LFSR.
    Test(bool),
    /// RING bit: triangle folding XORs with the `ring` input's MSB.
    RingMod(bool),
    /// SYNC bit: the accumulator resets on the `sync` input's MSB rising edge.
    HardSync(bool),
    /// Chip model (6581/8580): combined-waveform model + DAC curve.
    Model(SidModel),
    /// Master clock standard (PAL/NTSC) for pitch mapping.
    Clock(SidClock),
    /// Clock-domain conversion quality.
    Quality(SidQuality),
    /// One-pole DC blocker (~16 Hz, the C64's output coupling) on the audio
    /// output. Default on: combined 6581 waveforms carry real DC (measured
    /// ≈ −0.22 for tri+pulse) that eats mix headroom; the chip is AC-coupled
    /// downstream, and so is reSID's reference path.
    DcBlock(bool),
    /// Output level (0.0 to 1.0) — the one continuously automatable param.
    Level(Gain),
    /// Number of active waveform-sequence steps (0 = sequence off, static mask).
    SeqLength(u8),
    /// Driver frames per sequence step (1 = one step per 50/60 Hz frame).
    SeqRate(u8),
    /// Loop the waveform-mask sequence (`idx = pos % len`) instead of holding
    /// the last step — the canonical SID alternation (e.g. tri↔noise) repeats
    /// for the whole note; hold stays the default for drum-attack programs.
    SeqLoop(bool),
    /// Waveform-mask sequence step: (step index, 4-bit mask 0-15).
    /// Bit 0 = triangle, 1 = sawtooth, 2 = pulse, 3 = noise.
    SeqStep(u8, u8),
    /// Bit mask selecting sequence steps with an explicit frequency register.
    SeqFreqMask(u32),
    /// Per-step raw SID frequency-register override.
    SeqStepFreq(u8, u32),
    /// Per-oscillator glide (portamento) time in seconds (0 = follow the
    /// voice-level glide). Only affects the tuning when `TrackVoicePitch` is on.
    GlideTime(Seconds),
}

impl Default for SidOscillatorParam {
    fn default() -> Self {
        Self::Sawtooth(true)
    }
}
