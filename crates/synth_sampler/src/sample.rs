//! Sample data container.

use std::sync::Arc;

use crate::types::SampleMeta;

/// A loaded sample with audio data. Audio data is Arc-shared for
/// zero-copy access from multiple voices/modules on the audio thread.
///
/// `Clone` is cheap for the same reason: it copies the metadata and bumps the
/// `Arc`, so undo history can hold a deleted sample without duplicating its
/// audio.
#[derive(Clone)]
pub struct Sample {
    pub meta: SampleMeta,
    /// Interleaved audio data (mono: `[L,L,L,...]`, stereo: `[L,R,L,R,...]`).
    /// Arc allows zero-copy sharing across voices without allocation.
    pub data: Arc<[f32]>,
}

impl Sample {
    /// Create a new sample from metadata and audio data.
    pub fn new(meta: SampleMeta, data: Arc<[f32]>) -> Self {
        Self { meta, data }
    }

    /// Get the audio data as a shared reference.
    #[inline]
    pub fn data(&self) -> &Arc<[f32]> {
        &self.data
    }
}

impl std::fmt::Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("meta", &self.meta)
            .field("data_len", &self.data.len())
            .finish()
    }
}
