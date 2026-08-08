//! Bundle project format — ZIP archive containing project data and samples.
//!
//! Format:
//! ```text
//! project.pertylizer (ZIP)
//! ├── project.json         — ProjectFile (same as v1 JSON format)
//! ├── metadata.json        — Sample metadata for all embedded samples
//! └── samples/
//!     ├── 1.wav            — Sample data keyed by SampleId
//!     ├── 2.wav
//!     └── ...
//! ```

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::patch::PatchError;
use crate::project::ProjectFile;
use synth_core::audio::DeviceSampleRate;
use synth_core::{ChannelCount, SampleCount};
use synth_sampler::{Sample, SampleLibrary, SampleMeta, SampleSource};

/// ZIP magic bytes (PK\x03\x04).
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

/// Metadata for all samples in the bundle.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BundleMetadata {
    /// Sample metadata entries, keyed by filename (e.g., "1.wav").
    pub samples: Vec<BundleSampleEntry>,
}

/// Metadata for a single sample in the bundle.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BundleSampleEntry {
    /// Sample ID (matches the SampleLibrary ID).
    pub id: u64,
    /// Display name.
    pub name: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
    /// Total frames.
    pub frame_count: usize,
    /// Root MIDI note (if set).
    pub root_note: Option<u8>,
    /// Loop region (if set).
    pub loop_region: Option<BundleLoopRegion>,
    /// Crop region (if set).
    pub crop: Option<BundleCropRegion>,
}

/// Serializable loop region.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct BundleLoopRegion {
    pub start: usize,
    pub end: usize,
    pub crossfade: usize,
}

/// Serializable crop region.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct BundleCropRegion {
    pub start: usize,
    pub end: usize,
}

/// Check if a file starts with ZIP magic bytes.
pub fn is_zip_file(path: &Path) -> bool {
    std::fs::File::open(path)
        .and_then(|mut f| {
            let mut buf = [0u8; 4];
            f.read_exact(&mut buf)?;
            Ok(buf == ZIP_MAGIC)
        })
        .unwrap_or(false)
}

/// Save a project as a ZIP bundle with embedded samples.
///
/// The archive is built in a temporary file beside `path` and moved into place
/// only once it is complete and synced. A bundle is written incrementally over
/// however many samples the library holds, so a failure partway through — a
/// sample that will not encode, a full disk — used to leave a truncated ZIP
/// where the user's project had been. Now the previous bundle survives intact.
pub fn save_bundle(
    project: &ProjectFile,
    library: &SampleLibrary,
    path: &Path,
) -> Result<(), PatchError> {
    crate::io::atomic::write_with(path, |file| write_bundle_contents(project, library, file))
}

/// Serialize `project` plus every sample in `library` into `file` as a ZIP
/// archive. Split out from [`save_bundle`] so the atomic-replacement wrapper
/// stays readable; `file` is the temporary file, never the destination.
fn write_bundle_contents(
    project: &ProjectFile,
    library: &SampleLibrary,
    file: &mut std::fs::File,
) -> Result<(), PatchError> {
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write project.json
    let project_json =
        serde_json::to_string_pretty(project).map_err(|e| PatchError::Serialize(e.to_string()))?;
    zip.start_file("project.json", options)
        .map_err(|e| PatchError::Io(format!("ZIP write project.json: {e}")))?;
    zip.write_all(project_json.as_bytes())
        .map_err(|e| PatchError::Io(format!("ZIP write: {e}")))?;

    // Collect sample metadata and write WAV files
    let mut metadata = BundleMetadata {
        samples: Vec::new(),
    };

    for sample_meta in library.list() {
        let id = sample_meta.id.as_u64();
        let filename = format!("samples/{id}.wav");

        // Write WAV data into the ZIP (skip metadata if data missing)
        let Some(sample) = library.get(sample_meta.id) else {
            continue;
        };
        let wav_data =
            sample_to_wav_bytes(sample).map_err(|e| PatchError::Io(format!("WAV encode: {e}")))?;
        zip.start_file(&filename, options)
            .map_err(|e| PatchError::Io(format!("ZIP write {filename}: {e}")))?;
        zip.write_all(&wav_data)
            .map_err(|e| PatchError::Io(format!("ZIP write: {e}")))?;

        // Build metadata entry
        metadata.samples.push(BundleSampleEntry {
            id,
            name: sample_meta.name.clone(),
            sample_rate: sample_meta.sample_rate.as_u32(),
            channels: sample_meta.channels.count(),
            frame_count: sample_meta.frame_count.as_usize(),
            root_note: sample_meta.root_note.map(synth_core::MidiNote::as_u8),
            loop_region: sample_meta.loop_region.map(|l| BundleLoopRegion {
                start: l.start.0,
                end: l.end.0,
                crossfade: l.crossfade.as_usize(),
            }),
            crop: sample_meta.crop.map(|c| BundleCropRegion {
                start: c.start.0,
                end: c.end.0,
            }),
        });
    }

    // Write metadata.json
    let metadata_json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| PatchError::Serialize(e.to_string()))?;
    zip.start_file("metadata.json", options)
        .map_err(|e| PatchError::Io(format!("ZIP write metadata.json: {e}")))?;
    zip.write_all(metadata_json.as_bytes())
        .map_err(|e| PatchError::Io(format!("ZIP write: {e}")))?;

    zip.finish()
        .map_err(|e| PatchError::Io(format!("ZIP finalize: {e}")))?;

    Ok(())
}

/// Load a project from a ZIP bundle, restoring samples into the library.
///
/// Clears existing samples before loading to avoid stale data.
pub fn load_bundle(path: &Path, library: &mut SampleLibrary) -> Result<ProjectFile, PatchError> {
    library.clear();
    let file =
        std::fs::File::open(path).map_err(|e| PatchError::Io(format!("Open bundle: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| PatchError::Io(format!("ZIP open: {e}")))?;

    // Read project.json
    let project: ProjectFile = {
        let mut entry = archive
            .by_name("project.json")
            .map_err(|e| PatchError::Io(format!("ZIP read project.json: {e}")))?;
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|e| PatchError::Io(format!("Read project.json: {e}")))?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            crate::project::warn_if_legacy_awe(&value, path);
        }
        serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))?
    };

    // Read metadata.json (optional — old bundles might not have it)
    let metadata: Option<BundleMetadata> = if let Ok(mut entry) = archive.by_name("metadata.json") {
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|e| PatchError::Io(format!("Read metadata.json: {e}")))?;
        serde_json::from_str(&content).ok()
    } else {
        None
    };

    // Build a map from filename to metadata entry
    let meta_map: HashMap<u64, &BundleSampleEntry> = metadata
        .as_ref()
        .map(|m| m.samples.iter().map(|e| (e.id, e)).collect())
        .unwrap_or_default();

    // Load samples from samples/*.wav
    let sample_names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            let entry = archive.by_index(i).ok()?;
            let name = entry.name().to_string();
            if name.starts_with("samples/") && name.ends_with(".wav") {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    for name in &sample_names {
        // Extract sample ID from filename (e.g., "samples/42.wav" → 42)
        let id_str = name
            .strip_prefix("samples/")
            .and_then(|s| {
                if s.to_lowercase().ends_with(".wav") {
                    Some(&s[..s.len() - 4])
                } else {
                    None
                }
            })
            .unwrap_or("");
        let sample_id: u64 = match id_str.parse() {
            Ok(id) if id > 0 => id,
            _ => {
                eprintln!("Warning: skipping non-numeric sample file in bundle: {name}");
                continue;
            }
        };

        // Read WAV data
        let mut entry = archive
            .by_name(name)
            .map_err(|e| PatchError::Io(format!("ZIP read {name}: {e}")))?;
        let mut wav_data = Vec::new();
        entry
            .read_to_end(&mut wav_data)
            .map_err(|e| PatchError::Io(format!("Read {name}: {e}")))?;

        // Parse WAV
        let target_rate = DeviceSampleRate::DVD_QUALITY;
        match load_wav_from_bytes(&wav_data, target_rate) {
            Ok(mut sample) => {
                // Apply metadata if available
                if let Some(meta_entry) = meta_map.get(&sample_id) {
                    sample.meta.name = meta_entry.name.clone();
                    sample.meta.root_note = meta_entry.root_note.map(synth_core::MidiNote::new);
                    sample.meta.loop_region =
                        meta_entry.loop_region.map(|l| synth_sampler::LoopRegion {
                            start: synth_sampler::FrameIndex::new(l.start),
                            end: synth_sampler::FrameIndex::new(l.end),
                            crossfade: SampleCount::new(l.crossfade),
                        });
                    sample.meta.crop = meta_entry.crop.map(|c| synth_sampler::CropRegion {
                        start: synth_sampler::FrameIndex::new(c.start),
                        end: synth_sampler::FrameIndex::new(c.end),
                    });
                }
                let sid = synth_sampler::SampleId::new(sample_id);
                library.add_with_id(sample, sid);
            }
            Err(e) => {
                eprintln!("Warning: failed to load sample {name} from bundle: {e}");
            }
        }
    }

    Ok(project)
}

/// Encode a sample as WAV bytes in memory.
fn sample_to_wav_bytes(sample: &Sample) -> Result<Vec<u8>, synth_sampler::SampleError> {
    let mut cursor = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: sample.meta.channels.count(),
        sample_rate: sample.meta.sample_rate.as_u32(),
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    for &s in sample.data.iter() {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

/// Load a WAV from in-memory bytes.
fn load_wav_from_bytes(
    data: &[u8],
    _target_rate: DeviceSampleRate,
) -> Result<Sample, synth_sampler::SampleError> {
    let cursor = Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor)?;
    let spec = reader.spec();

    let channels = ChannelCount::from(spec.channels);
    let channel_count = spec.channels as usize;

    // Read all samples as f32
    let float_samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            samples?
        }
        (hound::SampleFormat::Int, 16) => {
            let samples: Result<Vec<i16>, _> = reader.samples::<i16>().collect();
            samples?
                .into_iter()
                .map(|s| f32::from(s) / f32::from(i16::MAX))
                .collect()
        }
        (hound::SampleFormat::Int, 24) => {
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            samples?
                .into_iter()
                .map(|s| s as f32 / 8_388_607.0)
                .collect()
        }
        _ => {
            return Err(synth_sampler::SampleError::UnsupportedFormat {
                reason: format!("{:?} {}-bit", spec.sample_format, spec.bits_per_sample),
            });
        }
    };

    let frame_count = float_samples.len() / channel_count;

    let meta = SampleMeta {
        id: synth_sampler::SampleId::new(0),
        name: "imported".to_string(),
        description: String::new(),
        sample_rate: DeviceSampleRate::new(spec.sample_rate),
        channels,
        frame_count: SampleCount::new(frame_count),
        root_note: None,
        loop_region: None,
        crop: None,
        source: SampleSource::Imported {
            original_path: None,
        },
    };

    let data: Arc<[f32]> = float_samples.into();
    Ok(Sample::new(meta, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::GlobalProjectState;

    /// A minimal project plus one tiny sample, so the bundle exercises both the
    /// `project.json` entry and the `samples/` WAV path.
    fn seeded_library() -> SampleLibrary {
        let mut library = SampleLibrary::new();
        let meta = SampleMeta {
            id: synth_sampler::SampleId::new(0),
            name: "blip".to_string(),
            description: String::new(),
            sample_rate: DeviceSampleRate::new(44_100),
            channels: ChannelCount::Mono,
            frame_count: SampleCount::new(4),
            root_note: None,
            loop_region: None,
            crop: None,
            source: SampleSource::Imported {
                original_path: None,
            },
        };
        let data: Arc<[f32]> = vec![0.0, 0.5, -0.5, 0.0].into();
        library.add(Sample::new(meta, data));
        library
    }

    fn sample_project(name: &str) -> ProjectFile {
        ProjectFile::new(
            Vec::new(),
            0,
            None,
            synth_sequencer::Song::new(name),
            GlobalProjectState::default(),
        )
    }

    /// A bundle written for the first time is a loadable ZIP with its samples.
    #[test]
    fn saves_a_new_bundle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fresh.zip");
        let library = seeded_library();

        save_bundle(&sample_project("fresh"), &library, &path).expect("save");

        assert!(is_zip_file(&path), "bundle must be a ZIP");
        let mut loaded_library = SampleLibrary::new();
        let project = load_bundle(&path, &mut loaded_library).expect("load");
        assert_eq!(project.song.name, "fresh");
        assert_eq!(loaded_library.len(), 1);
    }

    /// Overwriting an existing bundle leaves a complete archive, not a mix of
    /// the old and new ZIP central directories.
    #[test]
    fn overwriting_a_bundle_leaves_a_loadable_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.zip");
        let library = seeded_library();
        save_bundle(&sample_project("first"), &library, &path).expect("first save");

        save_bundle(&sample_project("second"), &library, &path).expect("second save");

        let mut loaded_library = SampleLibrary::new();
        let project = load_bundle(&path, &mut loaded_library).expect("load");
        assert_eq!(project.song.name, "second");
        assert_eq!(loaded_library.len(), 1);
    }

    /// The data-safety guarantee for bundles: a failed save must leave the
    /// previous bundle loadable rather than truncating it to a partial ZIP.
    ///
    /// Unix-only *mechanism*, not a Unix-only guarantee — see the twin in
    /// `project.rs` for why a read-only directory does not stop a save on
    /// Windows, and where the portable coverage lives.
    #[cfg(unix)]
    #[test]
    fn a_failed_bundle_save_preserves_the_previous_bundle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.zip");
        let library = seeded_library();
        save_bundle(&sample_project("last good save"), &library, &path).expect("seed");
        let before = std::fs::read(&path).expect("read seed");

        let mut perms = std::fs::metadata(dir.path())
            .expect("metadata")
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(dir.path(), perms).expect("set dir read-only");

        let result = save_bundle(&sample_project("would-be corruption"), &library, &path);

        let mut perms = std::fs::metadata(dir.path())
            .expect("metadata")
            .permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(dir.path(), perms).expect("restore dir perms");

        assert!(result.is_err(), "save into a read-only directory must fail");
        assert_eq!(
            std::fs::read(&path).expect("read after failed save"),
            before,
            "the previous bundle must survive a failed save",
        );
        let mut loaded_library = SampleLibrary::new();
        assert_eq!(
            load_bundle(&path, &mut loaded_library)
                .expect("previous bundle must still load")
                .song
                .name,
            "last good save",
        );
    }

    /// The replace step, portably: a bundle written onto a path that is a
    /// directory must fail and leave that directory as it was, with no
    /// scratch file abandoned beside it.
    #[test]
    fn a_bundle_save_that_cannot_replace_the_destination_leaves_it_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("song.ptz.zip");
        std::fs::create_dir(&path).expect("destination directory");
        std::fs::write(path.join("keep.txt"), b"untouched").expect("marker");

        let result = save_bundle(
            &sample_project("would-be corruption"),
            &seeded_library(),
            &path,
        );

        assert!(result.is_err(), "saving onto a directory must fail");
        assert!(path.is_dir(), "the destination directory must survive");
        assert_eq!(
            std::fs::read(path.join("keep.txt")).expect("read marker"),
            b"untouched",
            "the destination's contents must be untouched",
        );

        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| *p != path)
            .collect();
        assert!(strays.is_empty(), "temp file left behind: {strays:?}");
    }
}
