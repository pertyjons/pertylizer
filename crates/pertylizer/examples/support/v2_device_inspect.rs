//! Passive ALSA capability inspection. No stream is created or started, and no
//! PCM is captured. Advertised configurations are not negotiated timing evidence.
use cpal::{SampleFormat, SupportedStreamConfig, SupportedStreamConfigRange, traits::DeviceTrait};
use std::io::{self, Write};
use synth_core::audio::{ChannelCount, DeviceSampleRate};

type Error = Box<dyn std::error::Error>;
enum Direction {
    Input,
    Output,
}
struct Capabilities {
    default: Result<SupportedStreamConfig, Error>,
    ranges: Result<Vec<SupportedStreamConfigRange>, Error>,
}
impl Capabilities {
    fn query(device: &cpal::Device, direction: Direction) -> Self {
        match direction {
            Direction::Input => Self {
                default: device.default_input_config().map_err(Into::into),
                ranges: device
                    .supported_input_configs()
                    .map(Iterator::collect)
                    .map_err(Into::into),
            },
            Direction::Output => Self {
                default: device.default_output_config().map_err(Into::into),
                ranges: device
                    .supported_output_configs()
                    .map(Iterator::collect)
                    .map_err(Into::into),
            },
        }
    }
}

fn formats(
    ranges: &[SupportedStreamConfigRange],
    rate: DeviceSampleRate,
    layout: ChannelCount,
) -> Vec<SampleFormat> {
    [SampleFormat::F32, SampleFormat::I32, SampleFormat::I16]
        .into_iter()
        .filter(|format| {
            ranges.iter().any(|range| {
                range.channels() == layout.count()
                    && range.sample_format() == *format
                    && range.min_sample_rate() <= rate.as_u32()
                    && range.max_sample_rate() >= rate.as_u32()
            })
        })
        .collect()
}

fn write_report(
    writer: &mut impl Write,
    input: &Capabilities,
    output: &Capabilities,
) -> Result<(), Error> {
    writeln!(writer, "inspection_only=true streams_started=false")?;
    for (direction, capabilities) in [("input", input), ("output", output)] {
        match &capabilities.default {
            Ok(config) => writeln!(writer, "{direction}_default={config:?}")?,
            Err(error) => writeln!(writer, "{direction}_default_error={error}")?,
        }
        match &capabilities.ranges {
            Ok(ranges) => {
                writeln!(writer, "{direction}_advertised_ranges={}", ranges.len())?;
                for range in ranges {
                    writeln!(
                        writer,
                        "{direction} channels={} format={:?} rate_hz={}..={} buffer_frames={:?}",
                        range.channels(),
                        range.sample_format(),
                        range.min_sample_rate(),
                        range.max_sample_rate(),
                        range.buffer_size(),
                    )?;
                }
            }
            Err(error) => writeln!(writer, "{direction}_ranges_error={error}")?,
        }
    }
    if let (Ok(input), Ok(output)) = (&input.default, &output.default) {
        match crate::duplex::validate_pair(input, output) {
            Ok(_) => writeln!(writer, "default_pair_format_check=accepted")?,
            Err(error) => writeln!(writer, "default_pair_format_check=refused reason={error}")?,
        }
    }
    writeln!(
        writer,
        "advertised_pair_filter=44100,48000,96000,192000_hz;formats=F32,I32,I16;layouts=Mono,Stereo"
    )?;
    if let (Ok(input), Ok(output)) = (&input.ranges, &output.ranges) {
        for rate in [
            DeviceSampleRate::CD_QUALITY,
            DeviceSampleRate::DVD_QUALITY,
            DeviceSampleRate::STUDIO_QUALITY,
            DeviceSampleRate::MAX_SUPPORTED,
        ] {
            for layout in [ChannelCount::Mono, ChannelCount::Stereo] {
                let input = formats(input, rate, layout);
                let output = formats(output, rate, layout);
                if !input.is_empty() && !output.is_empty() {
                    writeln!(
                        writer,
                        "advertised_pair rate_hz={} layout={layout:?} input_formats={input:?} output_formats={output:?}",
                        rate.as_u32()
                    )?;
                }
            }
        }
    }
    writeln!(writer, "advertised_pairs_are_advisory=true")?;
    writeln!(
        writer,
        "not every advertised pair is selectable by duplex; it keeps default channels and chooses default/alternate rates with an optional shared format"
    )?;
    writeln!(
        writer,
        "unverified=stream_open,negotiated_buffer,clock_mapping,physical_loopback,latency,production_capacity"
    )?;
    if input.default.is_err()
        || input.ranges.is_err()
        || output.default.is_err()
        || output.ranges.is_err()
    {
        return Err("capability query failed; endpoint diagnostics are retained above".into());
    }
    Ok(())
}

pub(super) fn run(host: &cpal::Host, input_id: &str, output_id: &str) -> Result<(), Error> {
    let input = crate::duplex::select(host, input_id, true)?;
    let output = crate::duplex::select(host, output_id, false)?;
    // Collect both directions even if one query fails, preserving each diagnostic.
    let input = Capabilities::query(&input, Direction::Input);
    let output = Capabilities::query(&output, Direction::Output);
    let mut writer = io::stdout().lock();
    writeln!(writer, "input_id={input_id}\noutput_id={output_id}")?;
    write_report(&mut writer, &input, &output)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::SupportedBufferSize;

    fn capabilities(channels: u16, format: SampleFormat) -> Capabilities {
        let range = SupportedStreamConfigRange::new(
            channels,
            44_100,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 512 },
            format,
        );
        Capabilities {
            default: Ok(range.with_sample_rate(48_000)),
            ranges: Ok(vec![range]),
        }
    }
    #[test]
    fn matching_ranges_allow_different_input_and_output_formats_without_claiming_negotiation() {
        let input = capabilities(2, SampleFormat::F32);
        let output = capabilities(2, SampleFormat::I16);
        let mut report = Vec::new();
        write_report(&mut report, &input, &output).expect("report");
        let report = String::from_utf8(report).expect("text");
        assert!(report.contains("default_pair_format_check=accepted"));
        assert!(report.contains(
            "advertised_pair rate_hz=48000 layout=Stereo input_formats=[F32] output_formats=[I16]"
        ));
        assert!(!report.contains("advertised_pair rate_hz=96000"));
        assert!(report.contains("unverified=stream_open,negotiated_buffer"));
        assert!(report.contains("not every advertised pair is selectable by duplex"));
    }
    #[test]
    fn unequal_layouts_and_unsupported_formats_are_refused_by_the_shared_checks() {
        let input = capabilities(1, SampleFormat::F32);
        let output = capabilities(2, SampleFormat::F32);
        assert!(
            crate::duplex::validate_pair(
                input.default.as_ref().expect("input"),
                output.default.as_ref().expect("output")
            )
            .is_err()
        );
        let unsupported = capabilities(2, SampleFormat::F64);
        assert!(
            formats(
                unsupported.ranges.as_ref().expect("ranges"),
                DeviceSampleRate::DVD_QUALITY,
                ChannelCount::Stereo
            )
            .is_empty()
        );
        assert!(
            crate::duplex::validate_pair(
                unsupported.default.as_ref().expect("input"),
                output.default.as_ref().expect("output")
            )
            .is_err()
        );
    }
    #[test]
    fn failed_queries_are_reported_for_both_directions_and_fail_the_command() {
        let input = Capabilities {
            default: Err("input default unavailable".into()),
            ranges: Err("input ranges unavailable".into()),
        };
        let output = Capabilities {
            default: Err("output default unavailable".into()),
            ranges: Err("output ranges unavailable".into()),
        };
        let mut report = Vec::new();
        assert!(write_report(&mut report, &input, &output).is_err());
        let report = String::from_utf8(report).expect("text");
        for expected in [
            "input default unavailable",
            "input ranges unavailable",
            "output default unavailable",
            "output ranges unavailable",
        ] {
            assert!(report.contains(expected));
        }
        assert!(!report.contains("default_pair_format_check=accepted"));
    }
    #[test]
    fn reporting_propagates_output_failures() {
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("write failed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(
            write_report(
                &mut FailedWriter,
                &capabilities(2, SampleFormat::F32),
                &capabilities(2, SampleFormat::F32)
            )
            .is_err()
        );
    }
}
