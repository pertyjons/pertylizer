//! Lock-free live-input buffering and asynchronous sample-rate conversion.

use ringbuf::traits::{Consumer, Observer};
use synth_core::StereoSample;
use synth_core::audio::DeviceSampleRate;

/// Audio-input stream state owned exclusively by the audio thread.
pub(crate) struct AudioInputStream {
    consumer: ringbuf::HeapCons<StereoSample>,
    sample_rate: DeviceSampleRate,
    current: StereoSample,
    next: StereoSample,
    current_valid: bool,
    next_valid: bool,
    phase: f64,
}

impl AudioInputStream {
    pub(crate) fn new(
        consumer: ringbuf::HeapCons<StereoSample>,
        sample_rate: DeviceSampleRate,
    ) -> Self {
        Self {
            consumer,
            sample_rate,
            current: StereoSample::ZERO,
            next: StereoSample::ZERO,
            current_valid: false,
            next_valid: false,
            phase: 0.0,
        }
    }

    /// Render live input into a stereo-interleaved output block.
    ///
    /// Linear interpolation handles differing hardware rates. A very small
    /// fill-level correction tracks independent input/output clocks, while a
    /// bounded backlog trim prevents latency from growing without bound.
    pub(crate) fn render(
        &mut self,
        output: &mut [f32],
        frames: usize,
        output_sample_rate: DeviceSampleRate,
    ) {
        output.fill(0.0);
        let frames = frames.min(output.len() / 2);
        if frames == 0 {
            return;
        }

        let nominal_step = f64::from(self.sample_rate.as_u32().max(1))
            / f64::from(output_sample_rate.as_u32().max(1));
        let target_input_frames = expected_input_frames(frames, nominal_step);
        self.trim_excess_backlog(target_input_frames);

        let buffered_frames = self
            .consumer
            .occupied_len()
            .saturating_add(usize::from(self.current_valid))
            .saturating_add(usize::from(self.next_valid));
        let target = target_input_frames.max(2);
        let fill_error = buffered_frames as f64 / target as f64 - 1.0;
        let clock_correction = (fill_error * 0.002).clamp(-0.005, 0.005);
        let step = nominal_step * (1.0 + clock_correction);

        for frame_index in 0..frames {
            if !self.ensure_current() {
                break;
            }
            if !self.ensure_next() {
                // A lone sample at an exact source position is still valid.
                // Once emitted it is consumed; fractional positions wait for
                // the following input callback rather than repeating audio.
                if self.phase <= f64::EPSILON {
                    write_frame(output, frame_index, self.current);
                    self.current_valid = false;
                }
                break;
            }

            write_frame(
                output,
                frame_index,
                interpolate(self.current, self.next, self.phase),
            );
            self.phase += step;

            while self.phase >= 1.0 {
                if !self.next_valid && !self.ensure_next() {
                    self.current_valid = false;
                    self.phase = 0.0;
                    return;
                }
                self.current = self.next;
                self.current_valid = true;
                self.next_valid = false;
                self.phase -= 1.0;
            }
        }
    }

    fn ensure_current(&mut self) -> bool {
        if !self.current_valid {
            let Some(frame) = self.consumer.try_pop() else {
                return false;
            };
            self.current = frame;
            self.current_valid = true;
            self.phase = 0.0;
        }
        true
    }

    fn ensure_next(&mut self) -> bool {
        if !self.next_valid {
            let Some(frame) = self.consumer.try_pop() else {
                return false;
            };
            self.next = frame;
            self.next_valid = true;
        }
        true
    }

    fn trim_excess_backlog(&mut self, target_input_frames: usize) {
        let target = target_input_frames.max(2);
        let high_watermark = target.saturating_mul(4);
        let retain = target.saturating_mul(2);
        let available = self.consumer.occupied_len();
        if available <= high_watermark {
            return;
        }

        let discard = available.saturating_sub(retain);
        for _ in 0..discard {
            let _ = self.consumer.try_pop();
        }
        self.current_valid = false;
        self.next_valid = false;
        self.phase = 0.0;
    }
}

#[allow(clippy::cast_possible_truncation)]
fn expected_input_frames(output_frames: usize, step: f64) -> usize {
    (output_frames as f64 * step).ceil().max(2.0) as usize
}

fn interpolate(from: StereoSample, to: StereoSample, phase: f64) -> StereoSample {
    let phase = phase as f32;
    StereoSample::new(
        from.left + (to.left - from.left) * phase,
        from.right + (to.right - from.right) * phase,
    )
}

fn write_frame(output: &mut [f32], frame_index: usize, frame: StereoSample) {
    let sample_index = frame_index * 2;
    if let Some(left) = output.get_mut(sample_index) {
        *left = frame.left;
    }
    if let Some(right) = output.get_mut(sample_index + 1) {
        *right = frame.right;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::HeapRb;
    use ringbuf::traits::{Producer, Split};

    #[test]
    fn backlog_trimming_preserves_stereo_frame_alignment() {
        let ring = HeapRb::<StereoSample>::new(32);
        let (mut producer, consumer) = ring.split();
        for index in 0..32 {
            let value = index as f32 + 1.0;
            assert!(producer.try_push(StereoSample::new(value, -value)).is_ok());
        }
        let mut stream = AudioInputStream::new(consumer, DeviceSampleRate::DVD_QUALITY);
        let mut output = [0.0; 8];

        stream.render(&mut output, 4, DeviceSampleRate::DVD_QUALITY);

        assert!(output[0] > 20.0, "old backlog should be discarded");
        for frame in output.as_chunks::<2>().0 {
            assert_eq!(frame[1], -frame[0]);
        }
    }

    #[test]
    fn linear_resampling_tracks_the_input_output_rate_ratio() {
        let ring = HeapRb::<StereoSample>::new(512);
        let (mut producer, consumer) = ring.split();
        for index in 0..482 {
            let value = index as f32;
            assert!(producer.try_push(StereoSample::new(value, -value)).is_ok());
        }
        let mut stream = AudioInputStream::new(consumer, DeviceSampleRate::DVD_QUALITY);
        let mut output = vec![0.0; 441 * 2];

        stream.render(&mut output, 441, DeviceSampleRate::CD_QUALITY);

        let last = output.as_chunks::<2>().0.last().unwrap_or(&[0.0, 0.0]);
        assert!((475.0..=481.0).contains(&last[0]));
        assert_eq!(last[1], -last[0]);
        assert!(
            output
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| frame[1] == -frame[0])
        );
    }
}
