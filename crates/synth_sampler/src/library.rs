//! Sample library — central registry of all loaded samples.

use std::collections::HashMap;

use crate::sample::Sample;
use crate::types::{CropRegion, LoopRegion, SampleId, SampleMeta};
use std::sync::Arc;
use synth_core::ContentRevision;

/// Central registry of all loaded samples.
///
/// `Clone` is cheap — samples share their audio through `Arc`, so a clone
/// copies metadata and bumps refcounts. Autosave uses it to take an owned
/// snapshot under the read lock and write it off the UI thread.
#[derive(Clone)]
pub struct SampleLibrary {
    samples: HashMap<SampleId, Sample>,
    next_id: u64,
    /// Bumped by every mutating method below, so the application shell can tell
    /// that samples were imported, deleted, or edited without deep-comparing
    /// the audio buffers. Mirrors `SharedSong::revision`; the two together plus
    /// the engine graph version make up the project's dirty state.
    revision: ContentRevision,
}

impl SampleLibrary {
    /// Create an empty library.
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
            next_id: 1,
            revision: ContentRevision::INITIAL,
        }
    }

    /// The current edit revision, advanced by every mutation of the library.
    pub fn revision(&self) -> ContentRevision {
        self.revision
    }

    /// Record that the library changed. Every mutating method funnels through
    /// here rather than touching the field, so a new one cannot forget.
    fn mark_mutated(&mut self) {
        self.revision = self.revision.next();
    }

    /// Add a sample, assigning and returning its ID.
    pub fn add(&mut self, mut sample: Sample) -> SampleId {
        let id = SampleId::new(self.next_id);
        self.next_id += 1;
        sample.meta.id = id;
        self.samples.insert(id, sample);
        self.mark_mutated();
        id
    }

    /// Add a sample with a specific ID (for bundle loading).
    /// Updates `next_id` to avoid future collisions.
    pub fn add_with_id(&mut self, mut sample: Sample, id: SampleId) -> SampleId {
        sample.meta.id = id;
        self.samples.insert(id, sample);
        // Ensure next_id is always above the highest inserted ID
        if id.as_u64() >= self.next_id {
            self.next_id = id.as_u64() + 1;
        }
        self.mark_mutated();
        id
    }

    /// Replace the audio data of an existing sample, preserving its ID and metadata.
    /// Returns `false` if the sample was not found.
    pub fn replace_data(&mut self, id: SampleId, data: Arc<[f32]>) -> bool {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.data = data;
            self.mark_mutated();
            true
        } else {
            false
        }
    }

    /// Remove a sample by ID.
    pub fn remove(&mut self, id: SampleId) -> Option<Sample> {
        let removed = self.samples.remove(&id);
        if removed.is_some() {
            self.mark_mutated();
        }
        removed
    }

    /// Remove all samples from the library.
    pub fn clear(&mut self) {
        if self.samples.is_empty() {
            return;
        }
        self.samples.clear();
        self.mark_mutated();
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
            self.mark_mutated();
        }
    }

    /// Update crop region for a sample.
    pub fn update_crop(&mut self, id: SampleId, crop: Option<CropRegion>) {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.meta.crop = crop;
            self.mark_mutated();
        }
    }

    /// Update loop region for a sample.
    pub fn update_loop(&mut self, id: SampleId, region: Option<LoopRegion>) {
        if let Some(sample) = self.samples.get_mut(&id) {
            sample.meta.loop_region = region;
            self.mark_mutated();
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
    use synth_core::audio::DeviceSampleRate;
    use synth_core::{ChannelCount, ContentRevision, SampleCount};

    fn make_sample(name: &str) -> Sample {
        Sample::new(
            SampleMeta {
                id: SampleId::new(0),
                name: name.to_string(),
                description: String::new(),
                sample_rate: DeviceSampleRate::new(44100),
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

    #[test]
    fn a_fresh_library_starts_at_the_initial_revision() {
        assert_eq!(SampleLibrary::new().revision(), ContentRevision::INITIAL);
    }

    /// Every mutating entry point must advance the revision, or an edit made
    /// through it would leave the project looking saved.
    #[test]
    fn every_mutation_advances_the_revision() {
        let mut lib = SampleLibrary::new();
        let mut previous = lib.revision();
        let mut assert_advanced = |lib: &SampleLibrary, what: &str| {
            assert_ne!(lib.revision(), previous, "{what} must advance the revision");
            previous = lib.revision();
        };

        let id = lib.add(make_sample("kick"));
        assert_advanced(&lib, "add");

        lib.add_with_id(make_sample("snare"), SampleId::new(42));
        assert_advanced(&lib, "add_with_id");

        lib.replace_data(id, vec![1.0_f32; 10].into());
        assert_advanced(&lib, "replace_data");

        let mut meta = lib.get_meta(id).expect("meta").clone();
        meta.name = "renamed".to_string();
        lib.update_meta(id, meta);
        assert_advanced(&lib, "update_meta");

        lib.update_crop(
            id,
            Some(CropRegion {
                start: FrameIndex::new(1),
                end: FrameIndex::new(9),
            }),
        );
        assert_advanced(&lib, "update_crop");

        lib.update_loop(
            id,
            Some(LoopRegion {
                start: FrameIndex::new(2),
                end: FrameIndex::new(8),
                crossfade: SampleCount::new(0),
            }),
        );
        assert_advanced(&lib, "update_loop");

        lib.remove(id);
        assert_advanced(&lib, "remove");

        lib.clear();
        assert_advanced(&lib, "clear");
    }

    /// A no-op must not look like an edit: removing a missing sample or
    /// clearing an already-empty library changes nothing the user would call
    /// unsaved work.
    #[test]
    fn no_op_mutations_leave_the_revision_alone() {
        let mut lib = SampleLibrary::new();
        let before = lib.revision();

        assert!(lib.remove(SampleId::new(999)).is_none());
        lib.clear();
        lib.update_meta(SampleId::new(999), make_sample("ghost").meta);
        lib.update_crop(SampleId::new(999), None);
        lib.update_loop(SampleId::new(999), None);
        assert!(!lib.replace_data(SampleId::new(999), vec![0.0_f32; 1].into()));

        assert_eq!(lib.revision(), before);
    }
}
