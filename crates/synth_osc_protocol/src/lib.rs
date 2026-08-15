#![forbid(unsafe_code)]

//! Shared OSC protocol constants and types for Pertylizer.
//!
//! This minimal crate defines all OSC address strings, protocol metadata,
//! and shared types so that both the main workspace (`synth_osc`) and the
//! standalone visualizer can reference the same definitions.

use core::fmt;
use core::net::Ipv4Addr;

// ---------------------------------------------------------------------------
// Protocol version
// ---------------------------------------------------------------------------

/// OSC protocol version. Both sender and receiver should agree on this value.
pub const PROTOCOL_VERSION: i32 = 1;

// ---------------------------------------------------------------------------
// Network defaults
// ---------------------------------------------------------------------------

/// Default UDP port for OSC communication.
pub const DEFAULT_OSC_PORT: u16 = 9000;

/// Default multicast group address for OSC telemetry.
///
/// Multiple visualizer instances can join this group simultaneously.
/// Uses the "administratively scoped" range (239.x.x.x) which is safe
/// for local/LAN use and will not be forwarded by routers.
pub const DEFAULT_MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 0, 0, 1);

/// Default telemetry update rate (Hz).
pub const DEFAULT_UPDATE_RATE_HZ: f32 = 30.0;

/// Default interval between `/synth/meta` beacon sends (seconds).
pub const DEFAULT_META_INTERVAL_SECS: f32 = 5.0;

/// Default idle timeout — seconds without `/viz/pong` before entering idle mode.
pub const DEFAULT_IDLE_TIMEOUT_SECS: f32 = 5.0;

/// How often the visualizer should send `/viz/pong` replies (seconds).
pub const PONG_REPLY_INTERVAL_SECS: f32 = 2.0;

// ---------------------------------------------------------------------------
// FFT / spectrum constants
// ---------------------------------------------------------------------------

/// Default number of FFT bands sent via OSC.
pub const NUM_FFT_BANDS: usize = 128;

/// Maximum FFT band count supported by receivers (array allocation size).
pub const MAX_FFT_BANDS: usize = 256;

// ---------------------------------------------------------------------------
// OSC address constants
// ---------------------------------------------------------------------------

/// OSC address constants.
pub mod addresses {
    /// Protocol metadata (sent at startup and periodically).
    pub const META: &str = "/synth/meta";

    /// Monotonic sequence number (included in every bundle).
    pub const META_SEQ: &str = "/synth/meta/seq";

    /// FFT band center frequencies in Hz (sent with meta).
    pub const META_FFT_FREQS: &str = "/synth/meta/fft_freqs";

    /// RMS audio levels (left, right).
    pub const AUDIO_RMS: &str = "/synth/audio/rms";

    /// Peak audio levels (left, right).
    pub const AUDIO_PEAK: &str = "/synth/audio/peak";

    /// FFT spectrum data (normalized 0.0-1.0).
    pub const AUDIO_FFT: &str = "/synth/audio/fft";

    /// Spectral centroid (brightness indicator, in Hz).
    pub const AUDIO_CENTROID: &str = "/synth/audio/centroid";

    /// Spectral flux (onset/change magnitude, arbitrary units).
    pub const AUDIO_FLUX: &str = "/synth/audio/flux";

    /// Note-on event (midi_note, velocity, instrument_id, category).
    pub const EVENT_NOTE_ON: &str = "/synth/event/note_on";

    /// Note-off event (midi_note, instrument_id, category).
    pub const EVENT_NOTE_OFF: &str = "/synth/event/note_off";

    /// MIDI CC event (cc_number, value_normalized, channel).
    pub const EVENT_CC: &str = "/synth/event/cc";

    /// Transport state (playing, tempo, beat_position).
    pub const TRANSPORT_STATE: &str = "/synth/transport/state";

    /// Beat phase within current beat (0.0-1.0).
    pub const TRANSPORT_PHASE: &str = "/synth/transport/phase";

    /// Active voice count.
    pub const ENGINE_VOICE_COUNT: &str = "/synth/engine/voice_count";

    /// CPU usage percentage.
    pub const ENGINE_CPU: &str = "/synth/engine/cpu";

    /// Event ring buffer drop count since last report.
    pub const ENGINE_EVENT_DROPS: &str = "/synth/engine/event_drops";

    /// Recorded-note **flushes** refused because the retry slot was occupied.
    ///
    /// The unit is a flush, not a note and not a take: one flush carries many
    /// notes, and loop recording emits a partial flush at every loop boundary,
    /// so one take can produce several. Cumulative total, not a delta.
    pub const ENGINE_RECORDED_NOTE_LOSSES: &str = "/synth/engine/recorded_flush_losses";

    /// Track-blocks in which a track hit the per-channel send cap with authored
    /// sends still unexamined. Cumulative total, not a delta.
    pub const ENGINE_SEND_TRUNCATIONS: &str = "/synth/engine/send_truncations";

    /// Handoffs to a deferred-drop channel that failed, leaving the `Arc` to be
    /// dropped on the audio thread. Not every one frees memory — a non-final
    /// reference only decrements — but each means the ring filled.
    pub const ENGINE_RT_ARC_DROPS: &str = "/synth/engine/deferred_drop_failures";

    /// Blocks in which a sequencer working buffer grew on the audio thread.
    pub const ENGINE_SEQ_BUFFER_GROWTHS: &str = "/synth/engine/seq_buffer_growths";

    /// Ping from sender — included in bundles so visualizer knows sender is alive.
    pub const VIZ_PING: &str = "/viz/ping";

    /// Pong from visualizer — reply to indicate a client is connected.
    pub const VIZ_PONG: &str = "/viz/pong";

    /// Select camera mode (string: "orbit", "top-down", "front", "fly-through", "free-orbit").
    pub const VIZ_CAMERA_MODE: &str = "/viz/camera/mode";
}

// ---------------------------------------------------------------------------
// Instrument category
// ---------------------------------------------------------------------------

/// Category of an instrument, used for visualization and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum InstrumentCategory {
    /// No specific category assigned.
    #[default]
    Uncategorized = 0,
    /// Drum / percussion instruments.
    Drums = 1,
    /// Bass instruments.
    Bass = 2,
    /// Sustained pad / string sounds.
    Pad = 3,
    /// Lead / melody instruments.
    Lead = 4,
    /// Arpeggiated / sequenced parts.
    Arp = 5,
    /// Keyboard / piano sounds.
    Keys = 6,
    /// Sound effects / atmosphere.
    FX = 7,
}

impl InstrumentCategory {
    /// Create from a raw `u8` value, returning `Uncategorized` for unknown values.
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Drums,
            2 => Self::Bass,
            3 => Self::Pad,
            4 => Self::Lead,
            5 => Self::Arp,
            6 => Self::Keys,
            7 => Self::FX,
            _ => Self::Uncategorized,
        }
    }

    /// Get the raw `u8` value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Get a human-readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uncategorized => "Uncategorized",
            Self::Drums => "Drums",
            Self::Bass => "Bass",
            Self::Pad => "Pad",
            Self::Lead => "Lead",
            Self::Arp => "Arp",
            Self::Keys => "Keys",
            Self::FX => "FX",
        }
    }
}

impl fmt::Display for InstrumentCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl core::str::FromStr for InstrumentCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "drums" => Ok(Self::Drums),
            "bass" => Ok(Self::Bass),
            "pad" => Ok(Self::Pad),
            "lead" => Ok(Self::Lead),
            "arp" => Ok(Self::Arp),
            "keys" => Ok(Self::Keys),
            "fx" => Ok(Self::FX),
            "uncategorized" | "" => Ok(Self::Uncategorized),
            _ => Err(format!("unknown category: {s}")),
        }
    }
}
