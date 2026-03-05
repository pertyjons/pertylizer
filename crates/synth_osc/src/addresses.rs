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
