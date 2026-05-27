//! WAV file loading and saving using the `hound` crate.

use std::path::Path;
use std::sync::Arc;

use hound::{SampleFormat, WavReader, WavWriter};
use synth_core::audio::SampleRate;
use synth_core::{ChannelCount, SampleCount};

use crate::error::SampleError;
use crate::sample::Sample;
use crate::types::{BitDepth, SampleId, SampleMeta, SampleSource};

/// Load a WAV file into a Sample. Resamples to `target_rate` if needed.
pub fn load_wav(path: &Path, target_rate: SampleRate) -> Result<Sample, SampleError> {
    if !path.exists() {
        return Err(SampleError::FileNotFound {
            path: path.to_path_buf(),
        });
    }

    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();

    if spec.sample_rate == 0 {
        return Err(SampleError::InvalidSampleRate(0));
    }

    let channels = ChannelCount::from(spec.channels);
    let channel_count = spec.channels as usize;

    // Read all samples as f32
    let float_samples = read_samples_as_f32(&mut reader, &spec)?;

    let source_rate = SampleRate(spec.sample_rate);
    let frame_count = float_samples.len() / channel_count;

    // Resample if needed
    let (final_samples, final_frame_count, final_rate) = if source_rate != target_rate
        && target_rate.0 > 0
    {
        let resampled = resample_linear(&float_samples, channel_count, source_rate, target_rate);
        let new_frames = resampled.len() / channel_count;
        (resampled, new_frames, target_rate)
    } else {
        (float_samples, frame_count, source_rate)
    };

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let meta = SampleMeta {
        id: SampleId::new(0), // Assigned by library
        name,
        description: String::new(),
        sample_rate: final_rate,
        channels,
        frame_count: SampleCount::new(final_frame_count),
        root_note: None,
        loop_region: None,
        crop: None,
        source: SampleSource::Imported {
            original_path: Some(path.to_path_buf()),
        },
    };

    let data: Arc<[f32]> = final_samples.into();

    Ok(Sample::new(meta, data))
}

/// Save a Sample to a WAV file.
pub fn save_wav(sample: &Sample, path: &Path, bit_depth: BitDepth) -> Result<(), SampleError> {
    let channel_count = sample.meta.channels.count();
    let spec = hound::WavSpec {
        channels: channel_count,
        sample_rate: sample.meta.sample_rate.0,
        bits_per_sample: match bit_depth {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        },
        sample_format: match bit_depth {
            BitDepth::Int16 | BitDepth::Int24 => SampleFormat::Int,
            BitDepth::Float32 => SampleFormat::Float,
        },
    };

    let mut writer = WavWriter::create(path, spec)?;

    // Determine which portion to write (crop or full)
    let (start, end) = if let Some(crop) = sample.meta.crop {
        let ch = channel_count as usize;
        (crop.start.0 * ch, crop.end.0 * ch)
    } else {
        (0, sample.data.len())
    };

    let data = &sample.data[start..end.min(sample.data.len())];

    match bit_depth {
        BitDepth::Int16 => {
            for &s in data {
                let clamped = s.clamp(-1.0, 1.0);
                let val = (clamped * f32::from(i16::MAX)) as i16;
                writer.write_sample(val)?;
            }
        }
        BitDepth::Int24 => {
            for &s in data {
                let clamped = s.clamp(-1.0, 1.0);
                let val = (clamped * 8_388_607.0) as i32;
                writer.write_sample(val)?;
            }
        }
        BitDepth::Float32 => {
            for &s in data {
                writer.write_sample(s)?;
            }
        }
    }

    writer.finalize()?;
    Ok(())
}

/// Read all samples from a WAV reader as f32 in [-1.0, 1.0].
fn read_samples_as_f32(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    spec: &hound::WavSpec,
) -> Result<Vec<f32>, SampleError> {
    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            Ok(samples?)
        }
        (SampleFormat::Int, 8) => {
            let samples: Result<Vec<i8>, _> = reader.samples::<i8>().collect();
            Ok(samples?.into_iter().map(|s| f32::from(s) / 128.0).collect())
        }
        (SampleFormat::Int, 16) => {
            let samples: Result<Vec<i16>, _> = reader.samples::<i16>().collect();
            Ok(samples?
                .into_iter()
                .map(|s| f32::from(s) / f32::from(i16::MAX))
                .collect())
        }
        (SampleFormat::Int, 24) => {
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            Ok(samples?
                .into_iter()
                .map(|s| s as f32 / 8_388_607.0)
                .collect())
        }
        (SampleFormat::Int, 32) => {
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            Ok(samples?
                .into_iter()
                .map(|s| s as f32 / i32::MAX as f32)
                .collect())
        }
        (format, bits) => Err(SampleError::UnsupportedFormat {
            reason: format!("{format:?} {bits}-bit"),
        }),
    }
}

/// Linear interpolation resampling.
///
/// Resamples interleaved audio data from `source_rate` to `target_rate`.
fn resample_linear(
    data: &[f32],
    channels: usize,
    source_rate: SampleRate,
    target_rate: SampleRate,
) -> Vec<f32> {
    let source_frames = data.len() / channels;
    if source_frames == 0 {
        return Vec::new();
    }

    let ratio = f64::from(source_rate.0) / f64::from(target_rate.0);
    let target_frames = ((source_frames as f64) / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(target_frames * channels);

    for frame in 0..target_frames {
        let src_pos = frame as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;

        for ch in 0..channels {
            let s0 = if idx < source_frames {
                data[idx * channels + ch]
            } else {
                0.0
            };
            let s1 = if idx + 1 < source_frames {
                data[(idx + 1) * channels + ch]
            } else {
                s0
            };
            output.push(s0 + (s1 - s0) * frac as f32);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_test_sample(sample_rate: u32, channels: u16, frames: usize) -> Sample {
        let ch = channels as usize;
        let mut data = Vec::with_capacity(frames * ch);
        for i in 0..frames {
            let t = i as f32 / frames as f32;
            let val = (t * std::f32::consts::TAU * 440.0 / sample_rate as f32).sin();
            for _ in 0..ch {
                data.push(val);
            }
        }

        Sample::new(
            SampleMeta {
                id: SampleId::new(0),
                name: "test".to_string(),
                description: String::new(),
                sample_rate: SampleRate(sample_rate),
                channels: ChannelCount::from(channels),
                frame_count: SampleCount::new(frames),
                root_note: None,
                loop_region: None,
                crop: None,
                source: SampleSource::Generated,
            },
            data.into(),
        )
    }

    #[test]
    fn wav_round_trip_16bit() {
        let sample = make_test_sample(44100, 1, 1000);
        let file = NamedTempFile::new().unwrap();
        save_wav(&sample, file.path(), BitDepth::Int16).unwrap();
        let loaded = load_wav(file.path(), SampleRate(44100)).unwrap();

        assert_eq!(loaded.meta.sample_rate, SampleRate(44100));
        assert_eq!(loaded.meta.channels, ChannelCount::Mono);
        assert_eq!(loaded.meta.frame_count, SampleCount::new(1000));
        assert_eq!(loaded.data.len(), 1000);

        // Check values are close (16-bit quantization)
        for i in 0..loaded.data.len() {
            assert!(
                (loaded.data[i] - sample.data[i]).abs() < 0.001,
                "sample {i}: {} vs {}",
                loaded.data[i],
                sample.data[i]
            );
        }
    }

    #[test]
    fn wav_round_trip_float32() {
        let sample = make_test_sample(48000, 2, 500);
        let file = NamedTempFile::new().unwrap();
        save_wav(&sample, file.path(), BitDepth::Float32).unwrap();
        let loaded = load_wav(file.path(), SampleRate(48000)).unwrap();

        assert_eq!(loaded.meta.channels, ChannelCount::Stereo);
        assert_eq!(loaded.meta.frame_count, SampleCount::new(500));
        assert_eq!(loaded.data.len(), 1000); // 500 frames * 2 channels

        for i in 0..loaded.data.len() {
            assert!(
                (loaded.data[i] - sample.data[i]).abs() < 1e-6,
                "sample {i}: {} vs {}",
                loaded.data[i],
                sample.data[i]
            );
        }
    }

    #[test]
    fn wav_round_trip_24bit() {
        let sample = make_test_sample(44100, 1, 100);
        let file = NamedTempFile::new().unwrap();
        save_wav(&sample, file.path(), BitDepth::Int24).unwrap();
        let loaded = load_wav(file.path(), SampleRate(44100)).unwrap();

        assert_eq!(loaded.meta.frame_count, SampleCount::new(100));
        for i in 0..loaded.data.len() {
            assert!(
                (loaded.data[i] - sample.data[i]).abs() < 0.0001,
                "sample {i}: {} vs {}",
                loaded.data[i],
                sample.data[i]
            );
        }
    }

    #[test]
    fn wav_load_with_resample() {
        let sample = make_test_sample(44100, 1, 44100);
        let file = NamedTempFile::new().unwrap();
        save_wav(&sample, file.path(), BitDepth::Float32).unwrap();

        let loaded = load_wav(file.path(), SampleRate(48000)).unwrap();
        assert_eq!(loaded.meta.sample_rate, SampleRate(48000));
        // 44100 frames at 44100 Hz = 1 second → ~48000 frames at 48000 Hz
        let expected_frames = 48000;
        let actual_frames = loaded.meta.frame_count.as_usize();
        assert!(
            (actual_frames as i64 - expected_frames as i64).unsigned_abs() <= 1,
            "expected ~{expected_frames} frames, got {actual_frames}",
        );
    }

    #[test]
    fn wav_load_nonexistent_file() {
        let result = load_wav(Path::new("/nonexistent/file.wav"), SampleRate(44100));
        assert!(result.is_err());
    }

    #[test]
    fn resample_identity() {
        let data = vec![0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5];
        let result = resample_linear(&data, 1, SampleRate(44100), SampleRate(44100));
        assert_eq!(result.len(), data.len());
        for (a, b) in result.iter().zip(data.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn resample_upsample() {
        // 4 frames mono at 22050 → 8 frames at 44100 (2x)
        let data = vec![0.0, 1.0, 0.0, -1.0];
        let result = resample_linear(&data, 1, SampleRate(22050), SampleRate(44100));
        assert_eq!(result.len(), 8);
        // First and interpolated values
        assert!((result[0] - 0.0).abs() < 1e-6);
        assert!((result[1] - 0.5).abs() < 1e-6);
        assert!((result[2] - 1.0).abs() < 1e-6);
    }
}
