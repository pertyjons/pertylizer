//! The sample format every WAV this application writes is encoded in.
//!
//! One type behind all three writers — the GUI's Export WAV dialog, the
//! `pertylizer render` command, and the MCP `render_to_wav` tool — so a bit
//! depth means the same thing, and quantizes the same way, wherever it is
//! chosen. Before this existed the dialog carried its own three-variant enum
//! with its own conversion arithmetic and the other two were hardcoded to
//! 32-bit float.
//!
//! The engine renders in `f32` end to end ([`synth_core::AudioBuffer`] holds
//! `Vec<f32>`), so `f32`'s 24-bit mantissa is the real information ceiling:
//! [`WavFormat::Int32`] writes a wider container, not a more accurate signal.
//! That is why 32-bit float is the lossless option and the default.

/// Sample format of a written WAV file.
///
/// The variants are exactly what `hound` can encode. Integer output is signed
/// little-endian except at 8 bits, which WAV stores unsigned — `hound` applies
/// that conversion, so quantization produces signed values at every depth.
///
/// The serde spellings are the same strings `--bit-depth` accepts, so a bit
/// depth written into a document (the V2 reference corpus manifest, above all)
/// reads back as the command line that would reproduce it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum WavFormat {
    /// 8-bit integer. Written unsigned, as WAV requires at this depth.
    #[value(name = "8")]
    #[serde(rename = "8")]
    Int8,
    /// 16-bit signed integer — CD depth.
    #[value(name = "16")]
    #[serde(rename = "16")]
    Int16,
    /// 24-bit signed integer.
    #[value(name = "24")]
    #[serde(rename = "24")]
    Int24,
    /// 32-bit signed integer. A wider container than the renderer's `f32`
    /// output can fill, so it carries no more detail than 24-bit.
    ///
    /// Spelled `32i`, never a bare `32`: "32-bit WAV" means the float format
    /// far more often than it means this one, so a bare `32` would hand the
    /// wrong encoding to whoever typed the obvious thing. Rejecting it makes
    /// the choice explicit — and `clap` lists both spellings in the error.
    #[value(name = "32i")]
    #[serde(rename = "32i")]
    Int32,
    /// 32-bit IEEE float. Lossless against the renderer's own `f32` output, and
    /// therefore the default.
    #[default]
    #[value(name = "32f")]
    #[serde(rename = "32f")]
    Float32,
}

impl WavFormat {
    /// Every format, in ascending width. Drives the GUI dropdown.
    pub const ALL: [Self; 5] = [
        Self::Int8,
        Self::Int16,
        Self::Int24,
        Self::Int32,
        Self::Float32,
    ];

    /// Display label for the UI and for `--help`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Int8 => "8-bit",
            Self::Int16 => "16-bit",
            Self::Int24 => "24-bit",
            Self::Int32 => "32-bit int",
            Self::Float32 => "32-bit float",
        }
    }

    /// Bits per sample, as the WAV header declares it.
    #[must_use]
    pub fn bits_per_sample(self) -> u16 {
        match self {
            Self::Int8 => 8,
            Self::Int16 => 16,
            Self::Int24 => 24,
            Self::Int32 | Self::Float32 => 32,
        }
    }

    /// Bytes one sample occupies on disk. `bits / 8`, kept as its own method
    /// because two size estimates and one allocation guard depend on it.
    #[must_use]
    pub fn bytes_per_sample(self) -> u32 {
        u32::from(self.bits_per_sample()) / 8
    }

    /// `"float"` or `"int"` — the encoding named in a render receipt, so a
    /// harness reading the JSON does not have to infer it from the bit depth.
    #[must_use]
    pub fn encoding(self) -> &'static str {
        match self {
            Self::Float32 => "float",
            _ => "int",
        }
    }

    /// The `hound` header for `channels` channels at `sample_rate`.
    #[must_use]
    pub fn spec(self, channels: u16, sample_rate: u32) -> hound::WavSpec {
        hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: self.bits_per_sample(),
            sample_format: match self {
                Self::Float32 => hound::SampleFormat::Float,
                _ => hound::SampleFormat::Int,
            },
        }
    }
}

/// Convert one normalized sample to the signed integer `bits` encodes.
///
/// Scales by `2^(bits-1)` and rounds, so -1.0 lands exactly on the minimum and
/// the full code range is used. Values past ±1.0 are clipped to the range
/// rather than wrapping — a wrapped sample is a full-scale discontinuity, which
/// is far worse than the clip it came from. Callers measure peak on the `f32`
/// input, before this runs, so a receipt still reports that the signal exceeded
/// full scale.
///
/// `f32 as i32` saturates in Rust rather than being undefined, and `NaN`
/// converts to 0, so no input can panic or produce a wild code.
#[must_use]
fn quantize(sample: f32, bits: u16) -> i32 {
    // 2^(bits-1): 128, 32_768, 8_388_608, 2_147_483_648.
    let scale = (1_u64 << (bits - 1)) as f32;
    (sample * scale).round().clamp(-scale, scale - 1.0) as i32
}

/// Write `samples` (interleaved, normalized `f32`) to `writer` in `format`, and
/// return the largest absolute amplitude seen.
///
/// The peak is measured on the `f32` input rather than on the encoded output,
/// so it reports what the render produced independently of the format it was
/// written in — including when integer output clipped it back to full scale.
///
/// # Errors
///
/// Returns whatever `hound` reports while encoding or writing.
pub(crate) fn write_samples<W>(
    writer: &mut hound::WavWriter<W>,
    samples: &[f32],
    format: WavFormat,
) -> Result<f32, hound::Error>
where
    W: std::io::Write + std::io::Seek,
{
    let mut peak = 0.0_f32;
    if format == WavFormat::Float32 {
        for &sample in samples {
            peak = peak.max(sample.abs());
            writer.write_sample(sample)?;
        }
    } else {
        let bits = format.bits_per_sample();
        for &sample in samples {
            peak = peak.max(sample.abs());
            writer.write_sample(quantize(sample, bits))?;
        }
    }
    Ok(peak)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header must agree with the arithmetic: a format that declared one
    /// width and quantized to another would produce a file that parses and
    /// sounds wrong.
    #[test]
    fn the_header_width_matches_the_byte_width() {
        for format in WavFormat::ALL {
            let spec = format.spec(2, 48_000);
            assert_eq!(spec.bits_per_sample, format.bits_per_sample());
            assert_eq!(
                u32::from(spec.bits_per_sample) / 8,
                format.bytes_per_sample(),
                "{}",
                format.label()
            );
        }
    }

    /// Full scale in either direction must land on the extreme code, not one
    /// short of it and not wrapped past it.
    #[test]
    fn full_scale_reaches_the_extremes() {
        assert_eq!(quantize(-1.0, 8), -128);
        assert_eq!(quantize(1.0, 8), 127);
        assert_eq!(quantize(-1.0, 16), -32_768);
        assert_eq!(quantize(1.0, 16), 32_767);
        assert_eq!(quantize(-1.0, 24), -8_388_608);
        assert_eq!(quantize(1.0, 24), 8_388_607);
        assert_eq!(quantize(-1.0, 32), i32::MIN);
        assert_eq!(quantize(1.0, 32), i32::MAX);
    }

    /// Silence must encode as the zero code at every depth. An off-by-one scale
    /// here would put a DC offset in every silent render.
    #[test]
    fn silence_is_the_zero_code() {
        for bits in [8, 16, 24, 32] {
            assert_eq!(quantize(0.0, bits), 0, "{bits}-bit");
        }
    }

    /// Over-full-scale input clips instead of wrapping. Wrapping would turn a
    /// mild overshoot into a full-scale discontinuity.
    #[test]
    fn overs_clip_rather_than_wrap() {
        assert_eq!(quantize(2.0, 16), 32_767);
        assert_eq!(quantize(-2.0, 16), -32_768);
        assert_eq!(quantize(1e30, 16), 32_767);
    }

    /// A NaN sample must not panic and must not produce a wild code. Rust's
    /// saturating float casts give 0, which is silence.
    #[test]
    fn a_nan_sample_becomes_silence() {
        assert_eq!(quantize(f32::NAN, 16), 0);
        assert_eq!(quantize(f32::INFINITY, 16), 32_767);
        assert_eq!(quantize(f32::NEG_INFINITY, 16), -32_768);
    }

    /// Rounding, not truncation: half a code up must move to the next code.
    /// Truncating toward zero — what the GUI export used to do — biases every
    /// sample toward silence.
    #[test]
    fn conversion_rounds_rather_than_truncates() {
        // 0.6 of one 16-bit code.
        let sample = 0.6 / 32_768.0;
        assert_eq!(quantize(sample, 16), 1);
        assert_eq!(quantize(-sample, 16), -1);
    }

    /// The peak a writer reports is the `f32` input's, not the clipped output's,
    /// so an integer render still tells the caller it went over full scale.
    #[test]
    fn peak_is_measured_before_clipping() {
        let samples = [0.0, 1.5, -0.25];
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer =
            hound::WavWriter::new(&mut buffer, WavFormat::Int16.spec(1, 48_000)).expect("header");
        let peak = write_samples(&mut writer, &samples, WavFormat::Int16).expect("write");
        writer.finalize().expect("finalize");
        assert!((peak - 1.5).abs() < f32::EPSILON);
    }

    /// Every format must actually encode through `hound`. This is the guard
    /// that would have caught 64-bit float, which `hound` panics on rather than
    /// rejecting.
    #[test]
    fn every_format_round_trips_through_hound() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        for format in WavFormat::ALL {
            let mut buffer = std::io::Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(&mut buffer, format.spec(1, 44_100))
                .unwrap_or_else(|e| panic!("{}: header: {e}", format.label()));
            write_samples(&mut writer, &samples, format)
                .unwrap_or_else(|e| panic!("{}: write: {e}", format.label()));
            writer
                .finalize()
                .unwrap_or_else(|e| panic!("{}: finalize: {e}", format.label()));

            let bytes = buffer.into_inner();
            let reader = hound::WavReader::new(std::io::Cursor::new(bytes))
                .unwrap_or_else(|e| panic!("{}: reread: {e}", format.label()));
            assert_eq!(
                reader.spec().bits_per_sample,
                format.bits_per_sample(),
                "{}",
                format.label()
            );
            assert_eq!(reader.len(), samples.len() as u32, "{}", format.label());
        }
    }

    /// The `#[value(name = …)]` and `#[serde(rename = …)]` spellings are two
    /// independent attributes making the same promise: a bit depth written into
    /// a document reads back as the `--bit-depth` argument that reproduces it.
    /// Nothing but this test stops one from being edited without the other, and
    /// the drift would be silent — a corpus manifest saying `"32"` for a depth
    /// the CLI only spells `32f`.
    #[test]
    fn the_serde_and_command_line_spellings_are_the_same_strings() {
        use clap::ValueEnum;
        for format in WavFormat::ALL {
            let serialized = serde_json::to_string(&format).expect("serialize");
            let json_name = serialized.trim_matches('"');
            let cli_name = format
                .to_possible_value()
                .expect("every variant is selectable on the command line");
            assert_eq!(
                json_name,
                cli_name.get_name(),
                "{} serializes as {json_name:?} but --bit-depth spells it {:?}",
                format.label(),
                cli_name.get_name()
            );
            let parsed: WavFormat = serde_json::from_str(&serialized).expect("parse");
            assert_eq!(parsed, format);
        }
    }
}
