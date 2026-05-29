//! Return bus channel: the engine-side runtime of an effect-send destination.
//!
//! A return bus is a sub-mix channel. Tracks tap a fraction of their signal
//! into it (see [`synth_sequencer::TrackSend`]); the channel runs that sum
//! through its own [`EffectChain`] (e.g. a shared reverb), applies its
//! fader/pan, and mixes the wet result back into the master bus. This is the
//! classic aux-send / return-track topology — one reverb shared by many
//! channels instead of an instance per channel.
//!
//! The *definition* (id, name, fader) lives in the song
//! ([`synth_sequencer::ReturnBus`]); this runtime channel holds the effect
//! chain and audio buffers, and caches the fader (volume/pan/mute) refreshed
//! from the song each block — exactly like per-track controls (Model C).
//!
//! Real-time model mirrors the per-instrument channel bus: the `input` buffer
//! is pre-allocated and only ever resized at the start of a block (never grown
//! mid-process), send taps accumulate into it, the effect chain processes it
//! in place, and [`ReturnBusChannel::mix_into`] sums the post-fader result into
//! the master mix.

use synth_core::{AudioBuffer, BipolarValue, Gain, NormalizedValue, ProcessContext};
use synth_sequencer::ReturnBusId;

use crate::effect_chain::EffectChain;
use crate::instrument::soft_clip;

/// A return-bus channel — the runtime sub-mix with an effect chain and a
/// per-block fader snapshot (the authoritative fader lives in the song).
pub struct ReturnBusChannel {
    id: ReturnBusId,
    /// Output fader level (0.0 = silent, 1.0 = unity). Snapshot of the song def.
    volume: NormalizedValue,
    /// Output pan (-1.0 = left, 0.0 = centre, 1.0 = right). Snapshot.
    pan: BipolarValue,
    /// When muted the channel still processes its chain (so reverb tails
    /// advance) but contributes nothing to the master mix. Snapshot.
    muted: bool,
    /// Effect chain applied to the summed send signal.
    effect_chain: EffectChain,
    /// Interleaved-stereo accumulation buffer for this block's send taps.
    input: AudioBuffer,
}

impl ReturnBusChannel {
    /// Maximum interleaved frame size pre-allocated for the input buffer
    /// (4096 stereo frames), matching the per-instrument sidechain cache.
    const MAX_FRAME: usize = 4096 * 2;

    /// Create a new, empty return-bus channel at unity gain / centre pan.
    #[must_use]
    pub fn new(id: ReturnBusId) -> Self {
        Self {
            id,
            volume: NormalizedValue::MAX,
            pan: BipolarValue::CENTER,
            muted: false,
            effect_chain: EffectChain::new(),
            input: AudioBuffer::new(Self::MAX_FRAME),
        }
    }

    /// The channel id.
    pub fn id(&self) -> ReturnBusId {
        self.id
    }

    /// Output fader level.
    pub fn volume(&self) -> NormalizedValue {
        self.volume
    }

    /// Output pan.
    pub fn pan(&self) -> BipolarValue {
        self.pan
    }

    /// Whether the channel is muted (excluded from the master mix).
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Refresh the cached fader snapshot from the song definition. Called each
    /// block before mixing — Real-time safe (plain field copies).
    pub fn set_fader(&mut self, volume: NormalizedValue, pan: BipolarValue, muted: bool) {
        self.volume = volume;
        self.pan = pan;
        self.muted = muted;
    }

    /// Mutable access to the effect chain (for add/remove/reorder/params).
    pub fn effect_chain_mut(&mut self) -> &mut EffectChain {
        &mut self.effect_chain
    }

    /// Read-only access to the effect chain.
    #[must_use]
    pub fn effect_chain(&self) -> &EffectChain {
        &self.effect_chain
    }

    /// Resize the input buffer to the current block and clear it to silence.
    /// Called once per block before send taps accumulate into it.
    pub fn prepare_block(&mut self, frame_len: usize) {
        if self.input.len() < frame_len {
            self.input.resize(frame_len);
        }
        // Only clear the active region; the buffer may be larger than the block.
        self.input.as_mut_slice()[..frame_len].fill(0.0);
    }

    /// Mutable slice of the input accumulation buffer (send taps write here).
    pub fn input_mut(&mut self) -> &mut [f32] {
        self.input.as_mut_slice()
    }

    /// Process the summed send signal through the effect chain, then apply the
    /// channel fader/pan and sum the wet result into `mix_buffer`.
    ///
    /// The chain always runs (so effect tails advance even with no input this
    /// block); a muted channel simply skips the contribution to `mix_buffer`.
    pub fn mix_into(&mut self, context: &ProcessContext<'_>, mix_buffer: &mut AudioBuffer) {
        self.effect_chain.process(&mut self.input, context);
        if self.muted {
            return;
        }
        let (pan_left, pan_right) = Gain::from_pan(self.pan);
        let volume = self.volume.as_f32();
        let left_gain = pan_left.as_f32() * volume;
        let right_gain = pan_right.as_f32() * volume;

        let src = self.input.as_slice();
        let dst = mix_buffer.as_mut_slice();
        let frames = src.len().min(dst.len());
        let mut i = 0;
        while i + 1 < frames {
            dst[i] += soft_clip(src[i] * left_gain);
            dst[i + 1] += soft_clip(src[i + 1] * right_gain);
            i += 2;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Fill the channel input with a constant (interleaved L/R) test signal
    /// small enough that the soft-clip stage stays in its linear region.
    fn fill_input(bus: &mut ReturnBusChannel, value: f32, frames: usize) {
        bus.prepare_block(frames * 2);
        let buf = bus.input_mut();
        for s in &mut buf[..frames * 2] {
            *s = value;
        }
    }

    fn new_bus() -> ReturnBusChannel {
        ReturnBusChannel::new(ReturnBusId(0))
    }

    #[test]
    fn volume_fader_scales_output() {
        let ctx = ProcessContext::default();
        let frames = 4;

        let mut unity = new_bus();
        fill_input(&mut unity, 0.1, frames);
        let mut mix_unity = AudioBuffer::new(frames * 2);
        unity.mix_into(&ctx, &mut mix_unity);

        let mut half = new_bus();
        half.set_fader(NormalizedValue::new(0.5), BipolarValue::CENTER, false);
        fill_input(&mut half, 0.1, frames);
        let mut mix_half = AudioBuffer::new(frames * 2);
        half.mix_into(&ctx, &mut mix_half);

        let u = mix_unity.as_slice()[0];
        let h = mix_half.as_slice()[0];
        assert!(u > 0.0, "unity output should be non-zero (got {u})");
        assert!(
            (h - u * 0.5).abs() < 1e-6,
            "half volume should halve the level (unity={u}, half={h})"
        );
    }

    #[test]
    fn pan_full_left_silences_right() {
        let ctx = ProcessContext::default();
        let frames = 4;
        let mut bus = new_bus();
        bus.set_fader(NormalizedValue::MAX, BipolarValue::new(-1.0), false);
        fill_input(&mut bus, 0.1, frames);
        let mut mix = AudioBuffer::new(frames * 2);
        bus.mix_into(&ctx, &mut mix);

        assert!(mix.as_slice()[0] > 0.0, "left channel should be non-zero");
        assert!(
            mix.as_slice()[1].abs() < 1e-6,
            "right channel should be silent at full-left pan (got {})",
            mix.as_slice()[1]
        );
    }

    #[test]
    fn muted_bus_contributes_nothing() {
        let ctx = ProcessContext::default();
        let frames = 4;
        let mut bus = new_bus();
        bus.set_fader(NormalizedValue::MAX, BipolarValue::CENTER, true);
        fill_input(&mut bus, 0.5, frames);
        let mut mix = AudioBuffer::new(frames * 2);
        bus.mix_into(&ctx, &mut mix);
        assert!(
            mix.as_slice().iter().all(|s| s.abs() < 1e-9),
            "a muted return bus must add nothing to the master mix"
        );
    }

    #[test]
    fn empty_chain_passes_signal_through() {
        // With no effects the channel is a plain sub-mix: centre/unity should
        // pass the input through scaled only by the constant-power centre gain.
        let ctx = ProcessContext::default();
        let frames = 4;
        let mut bus = new_bus();
        fill_input(&mut bus, 0.1, frames);
        let mut mix = AudioBuffer::new(frames * 2);
        bus.mix_into(&ctx, &mut mix);
        let expected = 0.1 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (mix.as_slice()[0] - expected).abs() < 1e-6,
            "centre/unity should pass input through at the centre gain (got {}, expected {expected})",
            mix.as_slice()[0]
        );
    }
}
