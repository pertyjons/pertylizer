//! Sample playback DSP — real-time safe sample player.
//!
//! `SamplePlayer` renders sample audio with pitch-shifted playback using
//! linear interpolation. It holds an `Arc<[f32]>` to the sample data
//! (zero-copy, no allocation on the audio thread).

use std::sync::Arc;

use synth_core::{ChannelCount, MidiNote};

use crate::types::{CropRegion, LoopRegion, PlaybackPosition, PlaybackSpeed};

/// Playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// Actively playing (or sustaining).
    Playing,
    /// In release phase (waiting to fade out).
    Releasing,
    /// Done — voice can be reclaimed.
    Finished,
}

/// A real-time safe sample player for one voice.
///
/// Holds an `Arc<[f32]>` to the sample data. All operations are lock-free
/// and allocation-free after construction.
#[derive(Clone)]
pub struct SamplePlayer {
    /// Shared sample data (interleaved).
    data: Arc<[f32]>,
    /// Number of channels.
    channels: usize,
    /// Total frames in the sample.
    frame_count: usize,
    /// Current playback position (fractional frame).
    position: PlaybackPosition,
    /// Playback speed (pitch ratio).
    speed: PlaybackSpeed,
    /// Crop region (audible portion).
    _crop_start: usize,
    crop_end: usize,
    /// Loop region (optional).
    loop_start: Option<usize>,
    loop_end: Option<usize>,
    /// Whether looping is enabled.
    looping: bool,
    /// Current state.
    state: PlaybackState,
    /// Velocity gain (0.0–1.0).
    velocity_gain: f32,
    /// Simple release envelope (linear fade, frames remaining).
    release_frames: usize,
    release_counter: usize,
}

impl SamplePlayer {
    /// Create a new sample player.
    ///
    /// `data` is the interleaved audio buffer, `channels` is 1 (mono) or 2 (stereo).
    pub fn new(
        data: Arc<[f32]>,
        channels: ChannelCount,
        frame_count: usize,
        crop: Option<CropRegion>,
        loop_region: Option<LoopRegion>,
    ) -> Self {
        let ch = channels.count() as usize;
        let (crop_start, crop_end) = if let Some(c) = crop {
            (c.start.as_usize(), c.end.as_usize().min(frame_count))
        } else {
            (0, frame_count)
        };

        let (loop_start, loop_end) = if let Some(l) = loop_region {
            (
                Some(l.start.as_usize()),
                Some(l.end.as_usize().min(frame_count)),
            )
        } else {
            (None, None)
        };

        Self {
            data,
            channels: ch,
            frame_count,
            position: PlaybackPosition::new(crop_start as f64),
            speed: PlaybackSpeed::ORIGINAL,
            _crop_start: crop_start,
            crop_end,
            loop_start,
            loop_end,
            looping: loop_region.is_some(),
            state: PlaybackState::Playing,
            velocity_gain: 1.0,
            release_frames: 512, // ~10ms at 48kHz
            release_counter: 0,
        }
    }

    /// Set the playback speed from a MIDI note offset with optional fine-tune in cents.
    pub fn set_pitch(&mut self, target_note: MidiNote, root_note: MidiNote, fine_tune_cents: f64) {
        let semitones = f64::from(target_note.0) - f64::from(root_note.0) + fine_tune_cents / 100.0;
        self.speed = PlaybackSpeed(2.0_f64.powf(semitones / 12.0));
    }

    /// Multiply the current speed by -1 (reverse playback).
    /// Sets position to crop_end - 1 if currently at the start.
    pub fn set_reverse(&mut self) {
        self.speed = PlaybackSpeed(-self.speed.0.abs());
        // Start from the end for reverse playback
        if self.position.0 <= self.crop_end as f64 * 0.01 {
            self.position = PlaybackPosition::new((self.crop_end.saturating_sub(1)) as f64);
        }
    }

    /// Set velocity gain (0.0–1.0).
    pub fn set_velocity(&mut self, gain: f32) {
        self.velocity_gain = gain.clamp(0.0, 1.0);
    }

    /// Enable or disable looping.
    pub fn set_looping(&mut self, enabled: bool) {
        self.looping = enabled;
    }

    /// Trigger note-off (start release).
    pub fn note_off(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Releasing;
            self.release_counter = self.release_frames;
        }
    }

    /// Check if playback is finished.
    pub fn is_finished(&self) -> bool {
        self.state == PlaybackState::Finished
    }

    /// Get current state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Render `frame_count` stereo frames into `output`.
    ///
    /// Output is always stereo interleaved `[L, R, L, R, ...]`.
    /// Caller must ensure `output.len() >= frame_count * 2`.
    /// Returns `true` if still active.
    pub fn render(&mut self, output: &mut [f32], frame_count: usize) -> bool {
        if self.state == PlaybackState::Finished {
            return false;
        }

        let speed = self.speed.0;
        let out_len = frame_count * 2;
        // Bounds guard (W1): avoid panic on undersized buffer
        if output.len() < out_len {
            return false;
        }

        for i in 0..frame_count {
            // Wrap position if looping, or finish if past end
            let idx = self.position.0 as usize;
            let end = if self.looping {
                self.loop_end.unwrap_or(self.crop_end)
            } else {
                self.crop_end
            };

            if idx >= end {
                if self.looping {
                    if let Some(ls) = self.loop_start {
                        // Wrap to loop start, preserving fractional offset
                        let overshoot = self.position.0 - end as f64;
                        self.position = PlaybackPosition::new(ls as f64 + overshoot);
                    } else {
                        self.finish_silence(output, i, frame_count);
                        return false;
                    }
                } else {
                    self.finish_silence(output, i, frame_count);
                    return false;
                }
            }

            // Read with linear interpolation (using current, possibly wrapped, position)
            let frac = self.position.fraction() as f32;
            let (left, right) = self.read_frame_interpolated(self.position.0 as usize, frac);

            // Apply velocity and release envelope
            let mut gain = self.velocity_gain;
            if self.state == PlaybackState::Releasing {
                if self.release_counter > 0 {
                    gain *= self.release_counter as f32 / self.release_frames as f32;
                    self.release_counter -= 1;
                } else {
                    self.state = PlaybackState::Finished;
                    output[i * 2] = 0.0;
                    output[i * 2 + 1] = 0.0;
                    continue;
                }
            }

            output[i * 2] = left * gain;
            output[i * 2 + 1] = right * gain;

            // Advance position
            self.position = PlaybackPosition::new(self.position.0 + speed);
        }

        self.state != PlaybackState::Finished
    }

    /// Fill remaining output with silence and mark finished.
    fn finish_silence(&mut self, output: &mut [f32], from: usize, total: usize) {
        self.state = PlaybackState::Finished;
        for j in from..total {
            output[j * 2] = 0.0;
            output[j * 2 + 1] = 0.0;
        }
    }

    /// Read an interpolated stereo frame at the given position.
    #[inline]
    fn read_frame_interpolated(&self, idx: usize, frac: f32) -> (f32, f32) {
        if self.channels == 1 {
            // Mono → duplicate to stereo
            let s0 = self.read_mono(idx);
            let s1 = self.read_mono(idx + 1);
            let val = s0 + (s1 - s0) * frac;
            (val, val)
        } else {
            // Stereo
            let (l0, r0) = self.read_stereo(idx);
            let (l1, r1) = self.read_stereo(idx + 1);
            let left = l0 + (l1 - l0) * frac;
            let right = r0 + (r1 - r0) * frac;
            (left, right)
        }
    }

    #[inline]
    fn read_mono(&self, frame: usize) -> f32 {
        if frame < self.frame_count {
            self.data[frame]
        } else {
            0.0
        }
    }

    #[inline]
    fn read_stereo(&self, frame: usize) -> (f32, f32) {
        if frame < self.frame_count {
            let i = frame * 2;
            if i + 1 < self.data.len() {
                (self.data[i], self.data[i + 1])
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        }
    }
}
