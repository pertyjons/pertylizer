//! Delay effect module.
//!
//! Features:
//! - Stereo delay with independent L/R times
//! - Tempo sync option
//! - Ping-pong mode
//! - Feedback with soft limiting
//! - Modulation input for chorus-like effects

use synth_core::{
    AudioEffect, BeatDivision, Bpm, BufferIndex, DelayMode, DelayParam, Describable, FilterState,
    Hertz, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param,
    ParameterDescriptor, ProcessContext, ResponseCurve, SampleCount, SampleRate, Seconds,
    StereoSample, TempoSyncState, WidgetHint,
};

/// Maximum delay time in seconds.
const MAX_DELAY_SECONDS: f32 = 2.0;

/// Stereo delay effect.
pub struct Delay {
    // Parameters
    mode: DelayMode,
    time_left: Seconds,
    time_right: Seconds,
    feedback: NormalizedValue,
    mix: NormalizedValue,
    high_cut: Hertz,

    // Tempo sync
    tempo_sync: TempoSyncState,
    sync_division: BeatDivision,

    // Delay buffers
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: BufferIndex,

    // Filter state for feedback
    filter_left: FilterState,
    filter_right: FilterState,

    // State
    sample_rate: SampleRate,
}

impl Delay {
    pub fn new() -> Self {
        let buffer_size = (MAX_DELAY_SECONDS * 48000.0) as usize;
        Self {
            mode: DelayMode::Mono,
            time_left: Seconds::new(0.375),
            time_right: Seconds::new(0.5),
            feedback: NormalizedValue::new(0.4),
            mix: NormalizedValue::CENTER,
            high_cut: Hertz::new(8000.0),
            tempo_sync: TempoSyncState::Free,
            sync_division: BeatDivision::QUARTER,
            buffer_left: vec![0.0; buffer_size],
            buffer_right: vec![0.0; buffer_size],
            write_pos: BufferIndex::ZERO,
            filter_left: FilterState::ZERO,
            filter_right: FilterState::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Calculate delay time in seconds from BPM and sync division.
    #[inline]
    fn synced_delay_time(&self, bpm: f32) -> Seconds {
        if bpm <= 0.0 {
            return self.time_left;
        }
        let tempo = Bpm::new(bpm);
        let duration = self.sync_division.to_duration(tempo);
        Seconds::new(duration.as_f32().min(MAX_DELAY_SECONDS))
    }

    /// Resize buffers for new sample rate.
    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_SECONDS * self.sample_rate.as_f32()) as usize;
        if self.buffer_left.len() != size {
            self.buffer_left.resize(size, 0.0);
            self.buffer_right.resize(size, 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    /// One-pole lowpass for feedback damping.
    #[inline]
    fn lowpass(input: f32, state: &mut FilterState, cutoff: Hertz, sample_rate: SampleRate) -> f32 {
        let coef = cutoff.to_exp_coeff(sample_rate);
        state.one_pole(input, coef)
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Delay {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("delay", "Delay")
            .description("Stereo delay with ping-pong mode")
            .category(ModuleCategory::Effect)
            .tag("delay")
            .tag("effect")
            .tag("time")
            // No ports - effect chain modules are processed automatically
            .parameter(
                ParameterDescriptor::choice(
                    "mode",
                    Param::Delay(DelayParam::Mode(DelayMode::Mono)),
                    "Mode",
                    DelayMode::to_choices(),
                )
                .description("Delay mode"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "time",
                    Param::Delay(DelayParam::Time(Seconds::new(0.375))),
                    "Time",
                )
                .description("Delay time (link: sets both left and right)")
                .range(0.001, MAX_DELAY_SECONDS)
                .default(0.375)
                .curve(ResponseCurve::Logarithmic)
                // "Link both" macro: settable via MCP/automation (sets L and R
                // together) but Hidden from the GUI auto-renderer — the displayed
                // and persisted state lives in time_left/time_right, so a visible
                // Time knob would show a permanently stale value (it is not emitted
                // by get_params). Retained in the descriptor so older patches that
                // saved a "time" key still resolve and load.
                .widget(WidgetHint::Hidden),
            )
            .parameter(
                ParameterDescriptor::float(
                    "time_left",
                    Param::Delay(DelayParam::TimeLeft(Seconds::new(0.375))),
                    "Time L",
                )
                .description("Left-channel delay time")
                .range(0.001, MAX_DELAY_SECONDS)
                .default(0.375)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "time_right",
                    Param::Delay(DelayParam::TimeRight(Seconds::new(0.5))),
                    "Time R",
                )
                .description("Right-channel delay time")
                .range(0.001, MAX_DELAY_SECONDS)
                .default(0.5)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "feedback",
                    Param::Delay(DelayParam::Feedback(NormalizedValue::new(0.4))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(0.0, 0.95)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Delay(DelayParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tone",
                    Param::Delay(DelayParam::Damping(NormalizedValue::new(0.4))),
                    "Tone",
                )
                .description("Feedback high-cut filter")
                .range(0.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tempo_sync",
                    Param::Delay(DelayParam::TempoSync(false)),
                    "Tempo Sync",
                )
                .description("Sync delay time to host tempo (uses Division instead of Time)")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sync_division",
                    Param::Delay(DelayParam::SyncDivision(BeatDivision::QUARTER)),
                    "Division",
                )
                .description("Beats per echo when tempo-synced (1 = quarter note)")
                .range(0.125, 4.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Delay {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        // Note: sample_rate is set via set_sample_rate() which is called from main thread
        // We don't resize buffers here to avoid allocation in audio thread
        debug_assert_eq!(
            self.sample_rate, context.sample_rate,
            "Delay sample rate mismatch - call set_sample_rate() before processing"
        );

        // Calculate effective delay time (synced or manual)
        let effective_time = if self.tempo_sync.is_tempo_sync() {
            self.synced_delay_time(context.tempo.as_f32())
        } else {
            self.time_left
        };

        let delay_samples_left = (effective_time.as_f32() * self.sample_rate.as_f32())
            .min((self.buffer_left.len() - 1) as f32);
        let delay_samples_right = if self.tempo_sync.is_tempo_sync() {
            delay_samples_left
        } else {
            (self.time_right.as_f32() * self.sample_rate.as_f32())
                .min((self.buffer_right.len() - 1) as f32)
        };

        let len = self.buffer_left.len();
        let feedback = self.feedback.as_f32();
        let mix = self.mix.as_f32();

        // Process assuming interleaved stereo
        for frame in 0..context.samples.as_usize() {
            // Get input as stereo sample
            let dry = StereoSample::read_frame(input, frame);

            // Read from delay buffers
            let delayed = StereoSample::new(
                self.write_pos
                    .read_interpolated(&self.buffer_left, delay_samples_left),
                self.write_pos
                    .read_interpolated(&self.buffer_right, delay_samples_right),
            );

            // Apply feedback filtering
            let fb = StereoSample::new(
                Self::lowpass(
                    delayed.left,
                    &mut self.filter_left,
                    self.high_cut,
                    self.sample_rate,
                ),
                Self::lowpass(
                    delayed.right,
                    &mut self.filter_right,
                    self.high_cut,
                    self.sample_rate,
                ),
            );

            // Calculate feedback signal based on mode
            let write = match self.mode {
                DelayMode::Mono => {
                    let mono_in = dry.to_mono();
                    let mono_fb = fb.to_mono();
                    StereoSample::from_mono(mono_in + mono_fb * feedback)
                }
                DelayMode::Stereo => StereoSample::new(
                    dry.left + fb.left * feedback,
                    dry.right + fb.right * feedback,
                ),
                DelayMode::PingPong => StereoSample::new(
                    dry.left + fb.right * feedback,
                    dry.right + fb.left * feedback,
                ),
            };

            // Soft limit feedback to prevent runaway
            let write = write.soft_clip();

            // Write to delay buffers
            self.buffer_left[self.write_pos.as_usize()] = write.left;
            self.buffer_right[self.write_pos.as_usize()] = write.right;

            // Advance write position
            self.write_pos = self.write_pos.advance(len);

            // Mix dry/wet
            let wet = dry.blend(delayed, mix);

            StereoSample::write_frame(output, frame, wet);
        }
    }

    fn reset(&mut self) {
        self.buffer_left.fill(0.0);
        self.buffer_right.fill(0.0);
        self.filter_left = FilterState::ZERO;
        self.filter_right = FilterState::ZERO;
        self.write_pos = BufferIndex::ZERO;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        let decay_time =
            crate::math::feedback_decay_time(self.time_left.as_f32(), self.feedback.as_f32());
        SampleCount::new((decay_time * self.sample_rate.as_f32()) as usize)
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Delay(delay_param) = param {
            match delay_param {
                DelayParam::Mode(m) => self.mode = m,
                DelayParam::Time(t) => {
                    let clamped = t.as_f32().clamp(0.001, MAX_DELAY_SECONDS);
                    self.time_left = Seconds::new(clamped);
                    self.time_right = Seconds::new(clamped);
                }
                DelayParam::TimeLeft(t) => {
                    self.time_left = Seconds::new(t.as_f32().clamp(0.001, MAX_DELAY_SECONDS))
                }
                DelayParam::TimeRight(t) => {
                    self.time_right = Seconds::new(t.as_f32().clamp(0.001, MAX_DELAY_SECONDS))
                }
                DelayParam::Feedback(f) => {
                    self.feedback = NormalizedValue::new(f.as_f32().clamp(0.0, 0.95))
                }
                DelayParam::Mix(m) => self.mix = m,
                DelayParam::Damping(d) => {
                    // Map normalized 0-1 to frequency 200-20000 Hz
                    let freq = 200.0 + d.as_f32() * (20000.0 - 200.0);
                    self.high_cut = Hertz::new(freq);
                }
                DelayParam::TempoSync(s) => self.tempo_sync = TempoSyncState::from(s),
                DelayParam::SyncDivision(d) => self.sync_division = d,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Delay(delay_param) = param {
            Some(match delay_param {
                DelayParam::Mode(_) => self.mode.index() as f32,
                DelayParam::Time(_) => self.time_left.as_f32(),
                DelayParam::TimeLeft(_) => self.time_left.as_f32(),
                DelayParam::TimeRight(_) => self.time_right.as_f32(),
                DelayParam::Feedback(_) => self.feedback.as_f32(),
                DelayParam::Mix(_) => self.mix.as_f32(),
                DelayParam::Damping(_) => {
                    // Convert frequency back to normalized
                    (self.high_cut.as_f32() - 200.0) / (20000.0 - 200.0)
                }
                DelayParam::TempoSync(_) => {
                    if self.tempo_sync.is_tempo_sync() {
                        1.0
                    } else {
                        0.0
                    }
                }
                DelayParam::SyncDivision(_) => self.sync_division.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Delay(DelayParam::Mode(self.mode)),
            // `Time` is the write-only link-both macro; the persisted state lives
            // in TimeLeft/TimeRight, so it is intentionally not emitted here.
            Param::Delay(DelayParam::TimeLeft(self.time_left)),
            Param::Delay(DelayParam::TimeRight(self.time_right)),
            Param::Delay(DelayParam::Feedback(self.feedback)),
            Param::Delay(DelayParam::Mix(self.mix)),
            Param::Delay(DelayParam::Damping(NormalizedValue::new(
                (self.high_cut.as_f32() - 200.0) / (20000.0 - 200.0),
            ))),
            Param::Delay(DelayParam::TempoSync(self.tempo_sync.is_tempo_sync())),
            Param::Delay(DelayParam::SyncDivision(self.sync_division)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Delay
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        // Resize buffers when sample rate changes (called from main thread, not audio thread)
        self.resize_buffers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_creation() {
        let delay = Delay::new();
        assert_eq!(delay.mode, DelayMode::Mono);
        assert!((delay.time_left.as_f32() - 0.375).abs() < 0.001);
    }

    #[test]
    fn test_delay_no_explosion() {
        let mut delay = Delay::new();
        delay.sample_rate = SampleRate::DVD_QUALITY;
        delay.feedback = NormalizedValue::new(0.9);
        delay.resize_buffers();

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        let input = vec![1.0; 512];
        let mut output = vec![0.0; 512];

        for _ in 0..100 {
            delay.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(sample.abs() < 10.0, "Delay output exploded");
        }
    }
}
