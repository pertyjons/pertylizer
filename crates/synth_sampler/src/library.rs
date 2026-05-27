//! Sample library — central registry of all loaded samples.

use std::collections::HashMap;

use crate::sample::Sample;
use crate::types::{CropRegion, LoopRegion, SampleId, SampleMeta};
use std::sync::Arc;

/// Central registry of all loaded samples.
pub struct SampleLibrary {
    samples: HashMap<SampleId, Sample>,
    next_id: u64,
}

impl SampleLibrary {
    /// Create an empty library.
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
            next_id: 1,
        }
    }

    /// Add a sample, assigning and returning its ID.
    pub fn add(&mut self, mut sample: Sample) -> SampleId {
        let id = SampleId::new(self.next_id);
        self.next_id += 1;
        sample.meta.id = id;
        self.samples.insert(id, sample);
        id
    }

    /// Add a sample with a specific ID (for bundle loading).
    /// Updates `next_id` to avoid future collisions.
    pub fn add_with_id(&mut self, mut sample: Sample, id: SampleId) -> SampleId {
        sample.meta.id = id;
        self.samples.insert(id, sample);
        // Ensure next_id is always above the highest inserted ID
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
        id
    }

    /// Replace the audio data of an existing sample, preserving its ID and metadata.
    /// Returns `false` if the sample was not found.
    pub fn replace_data(&mut self, id: SampleId, data: Arc<[f32]>) -> bool {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.data = data;
            true
        } else {
            false
        }
    }

    /// Remove a sample by ID.
    pub fn remove(&mut self, id: SampleId) -> Option<Sample> {
        self.samples.remove(&id)
    }

    /// Remove all samples from the library.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Get a sample reference by ID.
    pub fn get(&self, id: SampleId) -> Option<&Sample> {
        self.samples.get(&id)
    }

    /// Get the audio data for a sample.
    pub fn get_data(&self, id: SampleId) -> Option<Arc<[f32]>> {
        self.samples.get(&id).map(|s| Arc::clone(&s.data))
    }

    /// Get metadata for a sample.
    pub fn get_meta(&self, id: SampleId) -> Option<&SampleMeta> {
        self.samples.get(&id).map(|s| &s.meta)
    }

    /// List metadata for all samples.
    pub fn list(&self) -> Vec<&SampleMeta> {
        self.samples.values().map(|s| &s.meta).collect()
    }

    /// Number of samples in the library.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the library is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Update metadata for a sample.
    pub fn update_meta(&mut self, id: SampleId, meta: SampleMeta) {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.meta = meta;
        }
    }

    /// Update crop region for a sample.
    pub fn update_crop(&mut self, id: SampleId, crop: Option<CropRegion>) {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.meta.crop = crop;
        }
    }

    /// Update loop region for a sample.
    pub fn update_loop(&mut self, id: SampleId, region: Option<LoopRegion>) {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.meta.loop_region = region;
        }
    }
}

impl Default for SampleLibrary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FrameIndex, SampleSource};
    use synth_core::audio::SampleRate;
    use synth_core::{ChannelCount, SampleCount};

    fn make_sample(name: &str) -> Sample {
        Sample::new(
            SampleMeta {
                id: SampleId::new(0),
                name: name.to_string(),
                description: String::new(),
                sample_rate: SampleRate(44100),
                channels: ChannelCount::Mono,
                frame_count: SampleCount::new(100),
                root_note: None,
                loop_region: None,
                crop: None,
                source: SampleSource::Generated,
            },
            vec![0.0_f32; 100].into(),
        )
    }

    #[test]
    fn add_and_get() {
        let mut lib = SampleLibrary::new();
        assert!(lib.is_empty());

        let id = lib.add(make_sample("kick"));
        assert_eq!(lib.len(), 1);
        assert!(!lib.is_empty());

        let sample = lib.get(id).unwrap();
        assert_eq!(sample.meta.name, "kick");
        assert_eq!(sample.meta.id, id);
    }

    #[test]
    fn ids_are_unique() {
        let mut lib = SampleLibrary::new();
        let id1 = lib.add(make_sample("a"));
        let id2 = lib.add(make_sample("b"));
        let id3 = lib.add(make_sample("c"));
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn remove_sample() {
        let mut lib = SampleLibrary::new();
        let id = lib.add(make_sample("snare"));
        assert_eq!(lib.len(), 1);

        let removed = lib.remove(id);
        assert!(removed.is_some());
        assert!(lib.is_empty());
        assert!(lib.get(id).is_none());
    }

    #[test]
    fn get_data_returns_arc() {
        let mut lib = SampleLibrary::new();
        let id = lib.add(make_sample("hi-hat"));
        let data = lib.get_data(id).unwrap();
        assert_eq!(data.len(), 100);
    }

    #[test]
    fn list_returns_all_meta() {
        let mut lib = SampleLibrary::new();
        lib.add(make_sample("a"));
        lib.add(make_sample("b"));
        lib.add(make_sample("c"));

        let metas = lib.list();
        assert_eq!(metas.len(), 3);
    }

    #[test]
    fn update_crop() {
        let mut lib = SampleLibrary::new();
        let id = lib.add(make_sample("test"));

        let crop = CropRegion {
            start: FrameIndex::new(10),
            end: FrameIndex::new(90),
        };
        lib.update_crop(id, Some(crop));

        let meta = lib.get_meta(id).unwrap();
        let c = meta.crop.unwrap();
        assert_eq!(c.start, FrameIndex::new(10));
        assert_eq!(c.end, FrameIndex::new(90));
    }

    #[test]
    fn update_loop_region() {
        let mut lib = SampleLibrary::new();
        let id = lib.add(make_sample("test"));

        let region = LoopRegion {
            start: FrameIndex::new(20),
            end: FrameIndex::new(80),
            crossfade: SampleCount::new(64),
        };
        lib.update_loop(id, Some(region));

        let meta = lib.get_meta(id).unwrap();
        assert!(meta.loop_region.is_some());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut lib = SampleLibrary::new();
        assert!(lib.remove(SampleId::new(999)).is_none());
    }
}
