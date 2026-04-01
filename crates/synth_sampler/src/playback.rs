//! Sample playback DSP — real-time safe sample player.
//!
//! `SamplePlayer` renders sample audio with pitch-shifted playback using
//! linear interpolation. It holds an `Arc<[f32]>` to the sample data
//! (zero-copy, no allocation on the audio thread).

use std::sync::Arc;

use synth_core::{ChannelCount, MidiNote, Velocity};

use crate::types::{CropRegion, LoopRegion, PlaybackPosition};

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

/// Playback direction (runtime state for PingPong).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Reverse,
}

/// A real-time safe sample player for one voice.
///
/// Holds an `Arc<[f32]>` to the sample data. All operations are lock-free
/// and allocation-free after construction.
#[derive(Clone)]
pub struct SamplePlayer {
    /// Shared sample data (interleaved).
    data: Arc<[f32]>,
    /// Number of channels (1 or 2).
    channels: usize,
    /// Total frames in the sample.
    #[expect(dead_code, reason = "retained for future diagnostics/debugging")]
    frame_count: usize,
    /// Current playback position (fractional frame).
    position: PlaybackPosition,
    /// Playback speed magnitude (always positive).
    speed: f64,
    /// Current direction (for PingPong this flips at boundaries).
    direction: Direction,
    /// Whether this is a ping-pong player (direction flips at loop boundaries).
    ping_pong: bool,
    /// Crop start frame.
    crop_start: usize,
    /// Crop end frame (exclusive).
    crop_end: usize,
    /// Loop start frame (if looping).
    loop_start: Option<usize>,
    /// Loop end frame (exclusive, if looping).
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
    /// Start offset (0.0 = crop start, 1.0 = crop end).
    start_offset: f64,
}

impl SamplePlayer {
    /// Create a new sample player.
    ///
    /// `data` is the interleaved audio buffer, `channels` must be 1 (mono) or 2 (stereo).
    pub fn new(
        data: Arc<[f32]>,
        channels: ChannelCount,
        frame_count: usize,
        crop: Option<CropRegion>,
        loop_region: Option<LoopRegion>,
    ) -> Self {
        let ch = (channels.count() as usize).clamp(1, 2);
        let (crop_start, crop_end) = if let Some(c) = crop {
            (c.start.as_usize(), c.end.as_usize().min(frame_count))
        } else {
            (0, frame_count)
        };

        let (loop_start, loop_end) = if let Some(l) = loop_region {
            (
                Some(l.start.as_usize().max(crop_start)),
                Some(l.end.as_usize().min(crop_end)),
            )
        } else {
            (None, None)
        };

        Self {
            data,
            channels: ch,
            frame_count,
            position: PlaybackPosition::new(crop_start as f64),
            speed: 1.0,
            direction: Direction::Forward,
            ping_pong: false,
            crop_start,
            crop_end,
            loop_start,
            loop_end,
            looping: loop_region.is_some(),
            state: PlaybackState::Playing,
            velocity_gain: 1.0,
            release_frames: 512, // ~10ms at 48kHz
            release_counter: 0,
            start_offset: 0.0,
        }
    }

    /// Set the playback speed from a MIDI note offset with optional fine-tune in cents.
    pub fn set_pitch(&mut self, target_note: MidiNote, root_note: MidiNote, fine_tune_cents: f64) {
        let semitones = f64::from(target_note.0) - f64::from(root_note.0) + fine_tune_cents / 100.0;
        self.speed = 2.0_f64.powf(semitones / 12.0);
    }

    /// Set fine-tune in cents without pitch tracking (speed relative to 1.0).
    pub fn set_fine_tune(&mut self, cents: f64) {
        self.speed = 2.0_f64.powf(cents / 1200.0);
    }

    /// Set reverse playback direction.
    pub fn set_reverse(&mut self) {
        self.direction = Direction::Reverse;
        // Start from crop end for reverse
        self.position = PlaybackPosition::new((self.crop_end.saturating_sub(1)) as f64);
    }

    /// Enable ping-pong mode (direction reverses at loop boundaries).
    pub fn set_ping_pong(&mut self) {
        self.ping_pong = true;
        self.direction = Direction::Forward;
    }

    /// Set start offset (0.0 = crop start, 1.0 = crop end).
    pub fn set_start_offset(&mut self, offset: f64) {
        self.start_offset = offset.clamp(0.0, 1.0);
        let range = (self.crop_end - self.crop_start) as f64;
        let start = self.crop_start as f64 + range * self.start_offset;
        self.position = PlaybackPosition::new(start);
    }

    /// Set velocity gain (0.0–1.0).
    pub fn set_velocity(&mut self, velocity: Velocity) {
        self.velocity_gain = velocity.as_f32().clamp(0.0, 1.0);
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

        // Bounds guard: avoid panic on undersized buffer
        if output.len() < frame_count * 2 {
            return false;
        }

        for i in 0..frame_count {
            // Clamp position and check boundaries
            if !self.check_and_wrap_position() {
                self.finish_silence(output, i, frame_count);
                return false;
            }

            // Read with linear interpolation, wrapping next-frame for loop boundary
            let pos = self.position.0;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let next_idx = self.next_frame_index(idx);
            let (left, right) = self.read_interpolated(idx, next_idx, frac);

            // Apply velocity and release envelope
            let mut gain = self.velocity_gain;
            if self.state == PlaybackState::Releasing {
                if self.release_counter > 0 {
                    gain *= self.release_counter as f32 / self.release_frames.max(1) as f32;
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
            let delta = match self.direction {
                Direction::Forward => self.speed,
                Direction::Reverse => -self.speed,
            };
            self.position = PlaybackPosition::new(self.position.0 + delta);
        }

        self.state != PlaybackState::Finished
    }

    /// Check position is in bounds; wrap for looping or finish for one-shot.
    /// Returns `false` if playback should stop.
    fn check_and_wrap_position(&mut self) -> bool {
        let pos = self.position.0;

        match self.direction {
            Direction::Forward => {
                let end = if self.looping {
                    self.loop_end.unwrap_or(self.crop_end)
                } else {
                    self.crop_end
                };

                if pos >= end as f64 {
                    if self.looping {
                        let ls = self.loop_start.unwrap_or(self.crop_start);
                        let le = self.loop_end.unwrap_or(self.crop_end);
                        let loop_len = (le - ls).max(1);

                        if self.ping_pong {
                            // Reverse direction at forward boundary
                            let overshoot = pos - le as f64;
                            self.direction = Direction::Reverse;
                            self.position =
                                PlaybackPosition::new((le as f64 - 1.0 - overshoot).max(ls as f64));
                        } else {
                            // Wrap with modulo for high-speed overshoot
                            let overshoot = pos - le as f64;
                            let wrapped = overshoot % loop_len as f64;
                            self.position = PlaybackPosition::new(ls as f64 + wrapped);
                        }
                    } else {
                        return false;
                    }
                }
            }
            Direction::Reverse => {
                let start = if self.looping {
                    self.loop_start.unwrap_or(self.crop_start)
                } else {
                    self.crop_start
                };

                if pos < start as f64 {
                    if self.looping {
                        let ls = self.loop_start.unwrap_or(self.crop_start);
                        let le = self.loop_end.unwrap_or(self.crop_end);
                        let loop_len = (le - ls).max(1);

                        if self.ping_pong {
                            // Reverse direction at backward boundary
                            let undershoot = ls as f64 - pos;
                            self.direction = Direction::Forward;
                            self.position = PlaybackPosition::new(
                                (ls as f64 + undershoot).min(le as f64 - 1.0),
                            );
                        } else {
                            // Wrap to loop end
                            let undershoot = ls as f64 - pos;
                            let wrapped = undershoot % loop_len as f64;
                            self.position = PlaybackPosition::new(le as f64 - 1.0 - wrapped);
                        }
                    } else {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Get the next frame index for interpolation, wrapping at loop boundary.
    #[inline]
    fn next_frame_index(&self, idx: usize) -> usize {
        let next = idx + 1;
        if self.looping {
            let le = self.loop_end.unwrap_or(self.crop_end);
            let ls = self.loop_start.unwrap_or(self.crop_start);
            if next >= le {
                ls // Wrap to loop start for seamless interpolation
            } else {
                next
            }
        } else if next >= self.crop_end {
            idx // Clamp at end
        } else {
            next
        }
    }

    /// Fill remaining output with silence and mark finished.
    fn finish_silence(&mut self, output: &mut [f32], from: usize, total: usize) {
        self.state = PlaybackState::Finished;
        for j in from..total {
            output[j * 2] = 0.0;
            output[j * 2 + 1] = 0.0;
        }
    }

    /// Read an interpolated stereo frame from two frame indices.
    #[inline]
    fn read_interpolated(&self, idx: usize, next_idx: usize, frac: f32) -> (f32, f32) {
        if self.channels == 1 {
            let s0 = self.read_mono(idx);
            let s1 = self.read_mono(next_idx);
            let val = s0 + (s1 - s0) * frac;
            (val, val)
        } else {
            let (l0, r0) = self.read_stereo(idx);
            let (l1, r1) = self.read_stereo(next_idx);
            let left = l0 + (l1 - l0) * frac;
            let right = r0 + (r1 - r0) * frac;
            (left, right)
        }
    }

    #[inline]
    fn read_mono(&self, frame: usize) -> f32 {
        if frame < self.data.len() {
            self.data[frame]
        } else {
            0.0
        }
    }

    #[inline]
    fn read_stereo(&self, frame: usize) -> (f32, f32) {
        let i = frame * 2;
        if i + 1 < self.data.len() {
            (self.data[i], self.data[i + 1])
        } else {
            (0.0, 0.0)
        }
    }
}
