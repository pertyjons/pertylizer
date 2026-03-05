//! Shared telemetry resource updated by the OSC receiver.

use bevy::prelude::*;

/// Number of FFT bands received from the synth.
pub const NUM_FFT_BANDS: usize = 128;

/// All telemetry data received from the synth via OSC.
#[derive(Resource)]
pub struct SynthTelemetry {
    // ── Protocol metadata ──
    /// Protocol version from `/synth/meta`.
    pub protocol_version: i32,
    /// Sample rate (Hz).
    pub sample_rate: f32,
    /// OSC update rate (Hz).
    pub update_rate_hz: f32,

    // ── Sequence / staleness ──
    /// Monotonic sequence number from `/synth/meta/seq`.
    pub seq: i32,
    /// Frames since last packet (stale detection).
    pub stale_frames: u32,

    // ── Audio levels ──
    /// RMS levels (left, right), linear amplitude.
    pub rms: [f32; 2],
    /// Peak levels (left, right), linear amplitude.
    pub peak: [f32; 2],

    // ── FFT spectrum ──
    /// FFT magnitude bands, normalized 0.0–1.0.
    pub fft: [f32; NUM_FFT_BANDS],

    // ── Note events ──
    /// Most recent note-on: (midi_note, velocity, channel).
    pub last_note_on: Option<(u8, u8, u8)>,
    /// Frame counter since last note-on (for decay).
    pub note_age_frames: u32,

    // ── Transport ──
    /// Whether the sequencer is playing.
    pub playing: bool,
    /// Tempo in BPM.
    pub tempo: f32,
    /// Current beat position.
    pub beat_position: f32,

    // ── Engine ──
    /// Active voice count.
    pub voice_count: u32,
    /// CPU usage 0–100.
    pub cpu: f32,
}

impl Default for SynthTelemetry {
    fn default() -> Self {
        Self {
            protocol_version: 0,
            sample_rate: 0.0,
            update_rate_hz: 0.0,
            seq: 0,
            stale_frames: 0,
            rms: [0.0; 2],
            peak: [0.0; 2],
            fft: [0.0; NUM_FFT_BANDS],
            last_note_on: None,
            note_age_frames: u32::MAX,
            playing: false,
            tempo: 120.0,
            beat_position: 0.0,
            voice_count: 0,
            cpu: 0.0,
        }
    }
}
