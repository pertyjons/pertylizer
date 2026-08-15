//! Shared telemetry resource updated by the OSC receiver.

use bevy::prelude::*;
use synth_osc_protocol::InstrumentCategory;

/// Maximum number of FFT bands the arrays can hold.
pub const MAX_FFT_BANDS: usize = synth_osc_protocol::MAX_FFT_BANDS;

/// Expected protocol version — warn on mismatch.
pub const EXPECTED_PROTOCOL_VERSION: i32 = synth_osc_protocol::PROTOCOL_VERSION;

/// A single note-on event received from the synth.
#[derive(Message, Debug, Clone, Copy)]
pub struct NoteOnEvent {
    pub midi_note: u8,
    pub velocity: u8,
    pub instrument_id: u32,
    pub category: InstrumentCategory,
}

/// All telemetry data received from the synth via OSC.
#[derive(Resource)]
pub struct SynthTelemetry {
    // -- Protocol metadata --
    /// Protocol version from `/synth/meta`.
    pub protocol_version: i32,
    /// Sample rate (Hz).
    pub sample_rate: f32,
    /// OSC update rate (Hz).
    pub update_rate_hz: f32,

    // -- Sequence / staleness --
    /// Monotonic sequence number from `/synth/meta/seq`.
    pub seq: i32,
    /// Frames since last packet (stale detection).
    pub stale_frames: u32,
    /// Seconds since last packet (stale detection, time-based).
    pub stale_seconds: f32,

    // -- Audio levels --
    /// RMS levels (left, right), linear amplitude.
    pub rms: [f32; 2],
    /// Peak levels (left, right), linear amplitude.
    pub peak: [f32; 2],

    // -- FFT spectrum --
    /// FFT magnitude bands, normalized 0.0-1.0.
    pub fft: [f32; MAX_FFT_BANDS],
    /// Spectral centroid in Hz (brightness indicator).
    pub centroid_hz: f32,
    /// Spectral flux (onset/change magnitude).
    pub flux: f32,
    /// FFT band center frequencies in Hz (from `/synth/meta/fft_freqs`).
    pub fft_freqs: [f32; MAX_FFT_BANDS],
    /// Actual number of FFT bins in use (set from received OSC data).
    pub fft_bin_count: usize,

    // -- Note events --
    /// Most recent note-on event.
    pub last_note_on: Option<NoteOnEvent>,
    /// Frame counter since last note-on (for decay).
    pub note_age_frames: u32,

    // -- Transport --
    /// Whether the sequencer is playing.
    pub playing: bool,
    /// Tempo in BPM.
    pub tempo: f32,
    /// Current beat position.
    pub beat_position: f32,
    /// Beat phase within current beat (0.0-1.0).
    pub beat_phase: f32,

    // -- MIDI CC --
    /// Most recent CC event: (cc_number, value 0.0-1.0, channel).
    pub last_cc: Option<(u8, f32, u8)>,
    /// Pitch bend, normalized -1.0 to 1.0.
    pub pitch_bend: f32,
    /// Aftertouch, normalized 0.0 to 1.0.
    pub aftertouch: f32,

    // -- Engine --
    /// Active voice count.
    pub voice_count: u32,
    /// CPU usage 0-100.
    pub cpu: f32,
    /// Event ring buffer drops since last report.
    pub event_drops: u32,
    /// Recorded-note flushes the engine could not hand over.
    ///
    /// A flush, not a note and not a take: one flush holds many notes, and loop
    /// recording emits one per loop boundary.
    pub recorded_flush_losses: u64,
    /// Track-blocks where a track hit the per-channel send cap with authored
    /// sends still unexamined.
    pub send_truncations: u64,
    /// Deferred-drop handoffs that failed, leaving the `Arc` to drop in the
    /// audio callback. Not every one frees memory; each means a ring filled.
    pub rt_arc_drops: u64,
    /// Blocks where a sequencer working buffer grew on the audio thread.
    pub seq_buffer_growths: u64,
}

impl Default for SynthTelemetry {
    fn default() -> Self {
        Self {
            protocol_version: 0,
            sample_rate: 0.0,
            update_rate_hz: 0.0,
            seq: 0,
            stale_frames: 0,
            stale_seconds: 0.0,
            rms: [0.0; 2],
            peak: [0.0; 2],
            fft: [0.0; MAX_FFT_BANDS],
            centroid_hz: 0.0,
            flux: 0.0,
            fft_freqs: [0.0; MAX_FFT_BANDS],
            fft_bin_count: synth_osc_protocol::NUM_FFT_BANDS,
            last_note_on: None,
            note_age_frames: u32::MAX,
            playing: false,
            tempo: 120.0,
            beat_position: 0.0,
            beat_phase: 0.0,
            last_cc: None,
            pitch_bend: 0.0,
            aftertouch: 0.0,
            voice_count: 0,
            cpu: 0.0,
            event_drops: 0,
            recorded_flush_losses: 0,
            send_truncations: 0,
            rt_arc_drops: 0,
            seq_buffer_growths: 0,
        }
    }
}
