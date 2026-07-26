//! OSC address constants.
//!
//! Re-exported from [`synth_osc_protocol`] so that downstream crates in the
//! workspace can keep using `synth_osc::addresses::*` unchanged.

pub use synth_osc_protocol::addresses::{
    AUDIO_CENTROID, AUDIO_FFT, AUDIO_FLUX, AUDIO_PEAK, AUDIO_RMS, ENGINE_CPU, ENGINE_EVENT_DROPS,
    ENGINE_VOICE_COUNT, EVENT_CC, EVENT_NOTE_OFF, EVENT_NOTE_ON, META, META_FFT_FREQS, META_SEQ,
    TRANSPORT_PHASE, TRANSPORT_STATE, VIZ_CAMERA_MODE, VIZ_PING, VIZ_PONG,
};
