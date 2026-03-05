//! OSC address constants.

/// Protocol metadata (sent at startup and periodically).
pub const META: &str = "/synth/meta";

/// Monotonic sequence number (included in every bundle).
pub const META_SEQ: &str = "/synth/meta/seq";

/// RMS audio levels (left, right).
pub const AUDIO_RMS: &str = "/synth/audio/rms";

/// Peak audio levels (left, right).
pub const AUDIO_PEAK: &str = "/synth/audio/peak";

/// FFT spectrum data (128 bands, normalized 0.0–1.0).
pub const AUDIO_FFT: &str = "/synth/audio/fft";

/// Transport state (playing, tempo, beat_position).
pub const TRANSPORT_STATE: &str = "/synth/transport/state";

/// Note-on event (midi_note, velocity, channel).
pub const EVENT_NOTE_ON: &str = "/synth/event/note_on";

/// Note-off event (midi_note, channel).
pub const EVENT_NOTE_OFF: &str = "/synth/event/note_off";

/// Active voice count.
pub const ENGINE_VOICE_COUNT: &str = "/synth/engine/voice_count";

/// CPU usage percentage.
pub const ENGINE_CPU: &str = "/synth/engine/cpu";

/// Spectral centroid (brightness indicator, in Hz).
pub const AUDIO_CENTROID: &str = "/synth/audio/centroid";

/// Spectral flux (onset/change magnitude, arbitrary units).
pub const AUDIO_FLUX: &str = "/synth/audio/flux";

/// Beat phase within current beat (0.0–1.0).
pub const TRANSPORT_PHASE: &str = "/synth/transport/phase";

/// MIDI CC event (cc_number, value_normalized, channel).
pub const EVENT_CC: &str = "/synth/event/cc";

/// Event ring buffer drop count since last report.
pub const ENGINE_EVENT_DROPS: &str = "/synth/engine/event_drops";

/// FFT band center frequencies in Hz (128 floats, sent with meta).
pub const META_FFT_FREQS: &str = "/synth/meta/fft_freqs";

/// Ping from sender — included in bundles so visualizer knows sender is alive.
pub const VIZ_PING: &str = "/viz/ping";

/// Pong from visualizer — reply to indicate a client is connected.
pub const VIZ_PONG: &str = "/viz/pong";
