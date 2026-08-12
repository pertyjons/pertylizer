//! Decoded audio, and the WAV reader that produces it.
//!
//! The renderer writes WAVs in five formats ([`crate::audio::wav_format`]) and a
//! comparison has to read any of them, including files it did not write. The
//! reader therefore normalizes everything to interleaved `f32` in `[-1, 1]`,
//! which is the engine's own representation, so a comparison of a 32-bit float
//! render against itself is exact rather than round-tripped through an integer.

use std::path::Path;

use super::CompareError;

/// Bytes one decoded sample occupies, whatever depth the file stores it at.
const DECODED_BYTES_PER_SAMPLE: u64 = 4;

/// Largest decoded signal one comparison will load, in bytes.
///
/// Matches the render command's [`crate::render::MAX_RENDER_BYTES`], because the
/// files this reads are the files that command writes — a render the tool
/// produced must be a render it can read back. Two of these are held at once and
/// the loudness meter allocates an `f64` copy of one on top, so the real ceiling
/// is nearer three times this; the bound exists to turn an abort into an error,
/// not to fit a budget exactly.
pub const MAX_SIGNAL_BYTES: u64 = crate::render::MAX_RENDER_BYTES;

/// One decoded audio file.
#[derive(Debug, Clone)]
pub struct Signal {
    /// Interleaved samples, normalized to `[-1, 1]`.
    pub interleaved: Vec<f32>,
    /// Sample rate the file declares.
    pub sample_rate: u32,
    /// Channel count the file declares.
    pub channels: u16,
}

impl Signal {
    /// Frames, i.e. samples per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            return 0;
        }
        self.interleaved.len() / usize::from(self.channels)
    }

    /// Length in seconds.
    #[must_use]
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f32 / self.sample_rate as f32
    }

    /// The channels summed to mono.
    ///
    /// Summed and divided by the channel count rather than taking the left
    /// channel: a difference confined to one channel would be invisible to a
    /// left-only view, and every whole-signal metric here (pitch, spectrum,
    /// envelope) is about the programme rather than about one side of it.
    #[must_use]
    pub fn mono(&self) -> Vec<f32> {
        let channels = usize::from(self.channels);
        if channels <= 1 {
            return self.interleaved.clone();
        }
        let inverse = 1.0 / channels as f32;
        self.interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() * inverse)
            .collect()
    }

    /// One channel on its own, or an empty vector if there is no such channel.
    #[must_use]
    pub fn channel(&self, index: u16) -> Vec<f32> {
        let channels = usize::from(self.channels);
        if index >= self.channels || channels == 0 {
            return Vec::new();
        }
        self.interleaved
            .chunks_exact(channels)
            .map(|frame| frame[usize::from(index)])
            .collect()
    }

    /// Load and decode a WAV.
    ///
    /// # Errors
    ///
    /// Returns [`CompareError::ReadWav`] if the file cannot be opened or parsed,
    /// [`CompareError::UnsupportedWav`] for a format this reader does not decode,
    /// [`CompareError::EmptyWav`] for a file with no frames — so a zero-length
    /// comparison reports what it is rather than emitting a page of zeroes that
    /// read as perfect agreement — and [`CompareError::SignalTooLarge`] for one
    /// past [`MAX_SIGNAL_BYTES`].
    pub fn load(path: &Path) -> Result<Self, CompareError> {
        let mut reader = hound::WavReader::open(path).map_err(|source| CompareError::ReadWav {
            path: path.to_path_buf(),
            source,
        })?;
        let spec = reader.spec();
        let read = |path: &Path, source| CompareError::ReadWav {
            path: path.to_path_buf(),
            source,
        };

        // Checked from the header, before a single sample is decoded. Nothing
        // downstream bounds this: the decode is one `Vec<f32>`, and the loudness
        // meter allocates an `f64` copy on top, so an over-large input aborts
        // the process on the allocation instead of returning an error. That is
        // the same failure `--seconds` and `--tail-seconds` are bounded against
        // on the render side, arriving through a file instead of a flag.
        let bytes = u64::from(reader.len()) * DECODED_BYTES_PER_SAMPLE;
        if bytes > MAX_SIGNAL_BYTES {
            return Err(CompareError::SignalTooLarge {
                path: path.to_path_buf(),
                bytes,
                maximum: MAX_SIGNAL_BYTES,
            });
        }

        // Widths are matched exactly rather than by a bit count, because the
        // scale factor differs per depth and an unlisted one must fail loudly
        // instead of being decoded at the wrong gain.
        let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, 32) => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(|e| read(path, e))?,
            // 8-bit WAV is stored unsigned; `hound` returns it already converted
            // to signed, so the same signed scaling applies at every depth.
            (hound::SampleFormat::Int, bits @ (8 | 16 | 24 | 32)) => {
                let scale = 1.0 / (1_u64 << (bits - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|s| s as f32 * scale))
                    .collect::<Result<_, _>>()
                    .map_err(|e| read(path, e))?
            }
            _ => {
                return Err(CompareError::UnsupportedWav {
                    path: path.to_path_buf(),
                    description: format!("{:?} {}-bit", spec.sample_format, spec.bits_per_sample),
                });
            }
        };

        if spec.channels == 0 || interleaved.is_empty() {
            return Err(CompareError::EmptyWav {
                path: path.to_path_buf(),
            });
        }

        Ok(Self {
            interleaved,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
        })
    }
}

/// Describes a signal in the report, without its samples.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SignalInfo {
    /// Where it came from.
    pub path: String,
    /// Declared sample rate.
    pub sample_rate: u32,
    /// Declared channel count.
    pub channels: u16,
    /// Frames (samples per channel).
    pub frames: u64,
    /// Length in seconds.
    pub duration_seconds: f32,
}

impl SignalInfo {
    /// Summarize `signal`, loaded from `path`.
    #[must_use]
    pub fn of(signal: &Signal, path: &Path) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            sample_rate: signal.sample_rate,
            channels: signal.channels,
            frames: signal.frames() as u64,
            duration_seconds: signal.duration_seconds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav_format::{WavFormat, write_samples};

    /// Write `samples` to a temporary WAV in `format` and read them back.
    fn round_trip(samples: &[f32], format: WavFormat, channels: u16) -> Signal {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("signal.wav");
        {
            let mut writer = hound::WavWriter::create(&path, format.spec(channels, 44_100))
                .expect("create writer");
            write_samples(&mut writer, samples, format).expect("write");
            writer.finalize().expect("finalize");
        }
        Signal::load(&path).expect("load")
    }

    /// A 32-bit float round trip must be exact. This is the property that lets
    /// the corpus render at `32f` and expect a self-comparison to report zero
    /// difference rather than a quantization floor.
    #[test]
    fn a_float_round_trip_is_bit_exact() {
        let samples = [0.0, 0.5, -0.5, 0.123_456_79, -1.0, 1.0];
        let signal = round_trip(&samples, WavFormat::Float32, 1);
        assert_eq!(signal.interleaved, samples);
        assert_eq!(signal.sample_rate, 44_100);
        assert_eq!(signal.channels, 1);
    }

    /// Every integer depth must decode back to roughly what went in. A wrong
    /// scale factor at one depth would make a comparison against a 24-bit
    /// reference report a constant gain error.
    #[test]
    fn every_integer_depth_decodes_at_the_right_scale() {
        let samples = [0.0_f32, 0.5, -0.5, 0.25];
        for format in [
            WavFormat::Int8,
            WavFormat::Int16,
            WavFormat::Int24,
            WavFormat::Int32,
        ] {
            let signal = round_trip(&samples, format, 1);
            // One code at the narrowest depth is 1/128; allow two of them.
            let tolerance = 2.0 / (1_u64 << (format.bits_per_sample() - 1)) as f32;
            for (got, want) in signal.interleaved.iter().zip(&samples) {
                assert!(
                    (got - want).abs() <= tolerance,
                    "{}: {got} against {want}",
                    format.label()
                );
            }
        }
    }

    /// Frames, mono summing, and per-channel extraction all have to agree about
    /// the interleaving, or every stereo metric reads the wrong samples.
    #[test]
    fn stereo_deinterleaves_consistently() {
        let samples = [1.0_f32, -1.0, 0.5, -0.5, 0.25, -0.25];
        let signal = round_trip(&samples, WavFormat::Float32, 2);
        assert_eq!(signal.frames(), 3);
        assert_eq!(signal.channel(0), vec![1.0, 0.5, 0.25]);
        assert_eq!(signal.channel(1), vec![-1.0, -0.5, -0.25]);
        // Anti-phase channels sum to silence, which is the point of summing
        // rather than taking one side.
        assert_eq!(signal.mono(), vec![0.0, 0.0, 0.0]);
        assert!(signal.channel(2).is_empty());
    }

    /// A file with no frames must be refused. Decoding it would produce a report
    /// full of zeroes, and a zero difference reads as perfect agreement.
    #[test]
    fn an_empty_wav_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("empty.wav");
        hound::WavWriter::create(&path, WavFormat::Float32.spec(2, 44_100))
            .expect("create")
            .finalize()
            .expect("finalize");
        assert!(matches!(
            Signal::load(&path),
            Err(CompareError::EmptyWav { .. })
        ));
    }

    /// An over-large input must be refused from the header, before anything is
    /// allocated. Nothing downstream bounds the decode, so without this the
    /// process aborts on the allocation — which a caller cannot catch, report,
    /// or distinguish from a crash.
    ///
    /// The header is forged rather than the file being written, because writing
    /// half a gigabyte of audio to check a bound would make this test cost more
    /// than the bug.
    #[test]
    fn an_over_large_wav_is_refused_from_its_header() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("huge.wav");
        {
            let mut writer = hound::WavWriter::create(&path, WavFormat::Float32.spec(2, 44_100))
                .expect("create writer");
            write_samples(&mut writer, &[0.0; 8], WavFormat::Float32).expect("write");
            writer.finalize().expect("finalize");
        }
        // Overwrite the `data` chunk's declared length with one past the bound.
        // `hound` reports `len()` from the header, so this is exactly what a
        // truncated or mis-declared file looks like.
        let mut bytes = std::fs::read(&path).expect("read back");
        let data_at = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("a data chunk");
        // A whole number of stereo float frames, or `hound` rejects the header
        // as malformed before the size guard ever runs.
        let declared = (MAX_SIGNAL_BYTES + 8) as u32;
        bytes[data_at + 4..data_at + 8].copy_from_slice(&declared.to_le_bytes());
        std::fs::write(&path, &bytes).expect("rewrite");

        assert!(
            matches!(
                Signal::load(&path),
                Err(CompareError::SignalTooLarge { .. })
            ),
            "an over-large declared length must be refused"
        );
    }

    /// A missing file is a read failure naming the path, not a panic.
    #[test]
    fn a_missing_file_reports_its_path() {
        let error =
            Signal::load(Path::new("/nonexistent/comparison-input.wav")).expect_err("should fail");
        assert!(matches!(error, CompareError::ReadWav { .. }));
        assert!(error.to_string().contains("comparison-input.wav"));
    }
}
