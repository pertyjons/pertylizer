//! Mod Grid runtime — the engine-side running instances of the pooled
//! [`synth_sequencer::ModGraph`] data.
//!
//! The serializable graph data lives in `Song`; the **running instances** here
//! own the actual `Box<dyn PolyModule>` DSP (inside a [`ModuleGraph`]). Because
//! building those modules allocates, a [`ModGridRuntime`] is constructed off the
//! audio thread (in the app crate, via the module factory) and shipped to the
//! engine as a pre-built [`crate::EngineCommand::SetModGrid`]; the audio thread
//! only swaps the box in and processes it — never builds it.
//!
//! Each block, before instruments and before track-control composition, every
//! instance's [`ModuleGraph`] is processed once (control-rate, mono) and each of
//! its [`ResolvedTarget`]s reads a source output and contributes an additive,
//! block-constant offset into the automation target space.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, DestAddr, PortName, ProcessContext, SampleCount, SampleRate, Semitones,
};
use synth_sequencer::{
    AutoInstrumentParam, AutomationTarget, CombineMode, ModGraphId, SeqInstrumentId, TrackId,
    TransportSource,
};

use crate::{InstrumentId, ModuleId};

/// Audio-tap follower time constant (seconds). The one-pole coefficient is
/// derived from this **and the block length** ([`tap_coeff`]) so the follow speed
/// is independent of buffer size — offline and live renders at different block
/// sizes then have an approximately consistent time response (a fixed per-block
/// coefficient would smooth faster at 64 frames than at 512/1024). The match is
/// close but not bit-exact: a one-pole updated once per block still resolves the
/// step in coarser increments at larger blocks.
const TAP_TAU_SECONDS: f32 = 0.02;

/// Per-block one-pole coefficient giving a [`TAP_TAU_SECONDS`] time constant for a
/// block of `frames` at `sample_rate`, block-size-independent. RT-safe (one
/// `exp` per block, not per sample).
#[must_use]
pub(crate) fn tap_coeff(frames: SampleCount, sample_rate: SampleRate) -> f32 {
    let sr = sample_rate.as_f32();
    if sr <= 0.0 {
        return 1.0;
    }
    1.0 - (-(frames.as_usize() as f32) / (TAP_TAU_SECONDS * sr)).exp()
}

/// What drives a routing sink each block. A hosted DSP module output, or one of
/// the cheap grid-native sources computed directly by the pre-pass.
pub enum ModSource {
    /// A hosted control-rate module's output port (its block-representative
    /// last sample).
    Dsp(ModuleId, PortName),
    /// A constant value — the Macro knob's `value`.
    Constant(f32),
    /// A transport-derived value computed from the block's [`ProcessContext`].
    Transport(TransportSource),
    /// A pre-fader instrument level (a Track audio tap), smoothed into an
    /// envelope so it can duck directly. Resolved to the host instrument at
    /// build time. One-block latency (reads the previous block's output).
    InstrumentLevel(InstrumentId),
    /// The master-mix level (a Master audio tap), smoothed. One-block latency.
    MasterLevel,
    /// A live MIDI CC value in `0..1`, read from the engine's CC state each block.
    /// `channel: None` is omni (the last value seen on any channel).
    MidiCc { cc: u8, channel: Option<u8> },
}

/// Live MIDI CC state, read by [`ModSource::MidiCc`] grid sources. Values are
/// normalized `0..1`. Fixed-size arrays so updates and reads are RT-safe (no
/// allocation, no lock). Boxed by the engine to keep [`crate::SynthEngine`]
/// itself small.
pub struct MidiCcState {
    /// `per_channel[channel 0..15][cc 0..127]`.
    per_channel: [[f32; 128]; 16],
    /// The last value seen for each CC on *any* channel (omni sources).
    any: [f32; 128],
}

impl Default for MidiCcState {
    fn default() -> Self {
        Self {
            per_channel: [[0.0; 128]; 16],
            any: [0.0; 128],
        }
    }
}

impl MidiCcState {
    /// Record a CC value (RT-safe; indices are clamped into range).
    pub fn set(&mut self, channel: u8, cc: u8, value: f32) {
        let (ch, cc) = (usize::from(channel.min(15)), usize::from(cc.min(127)));
        self.per_channel[ch][cc] = value;
        self.any[cc] = value;
    }

    /// Read a CC value: a specific channel, or omni (`None`) for the last value
    /// on any channel. Out-of-range indices clamp; unseen CCs read `0.0`.
    #[must_use]
    pub fn get(&self, cc: u8, channel: Option<u8>) -> f32 {
        let cc = usize::from(cc.min(127));
        match channel {
            Some(ch) => self.per_channel[usize::from(ch.min(15))][cc],
            None => self.any[cc],
        }
    }
}

/// A routing sink resolved for one running instance: which source drives it, the
/// absolute (host-resolved) target address, and the depth.
pub struct ResolvedTarget {
    /// The source driving this routing. `None` when the `Target` node has no
    /// incoming cable — the routing is then inert.
    pub source: Option<ModSource>,
    /// The address to write, already resolved against the instance's host track
    /// (relative `Track { track: None }` targets are made absolute at build time
    /// for Track scope, or dropped for Global scope so they never reach here).
    pub target: AutomationTarget,
    /// For an [`AutomationTarget::Module`] target: the DSP address, interned
    /// **off the audio thread** at build time (`DestAddr::new` locks a global
    /// intern pool and would violate RT-safety in the pre-pass). `None` for
    /// every non-module target. The offset is applied additively to each active
    /// voice of the target instrument via [`crate::graph::ModuleGraph::apply_mod_offset_addr`].
    pub dest_addr: Option<DestAddr>,
    /// Depth in the target's own units; the source signal is multiplied by this.
    pub amount: f32,
    /// How the contribution composes (additive for now).
    pub combine: CombineMode,
    /// One-pole smoothing accumulator for audio-tap sources (unused otherwise).
    pub smooth: f32,
}

/// A cheap grid source (Macro/Transport/AudioTap) injected into a hosted
/// module's control input port as a block-constant value — `Macro → LFO.rate_cv`.
/// Applied to the module graph *before* it processes (unlike a [`ResolvedTarget`],
/// which reads a module output *after*). The DSP graph pre-creates the matching
/// injection slot at build time, so the pre-pass only updates the value.
pub struct InputInjection {
    /// The hosted module whose input port receives the value.
    pub module: ModuleId,
    /// The module's control input port (e.g. `rate_cv`).
    pub port: PortName,
    /// The cheap source computed each block. Never [`ModSource::Dsp`] — a
    /// module→module cable is a real DSP connection, not an injection.
    pub source: ModSource,
    /// One-pole smoothing accumulator for audio-tap sources (unused otherwise).
    pub smooth: f32,
}

impl InputInjection {
    /// Advance the one-pole envelope follower toward `level` and return it
    /// (audio-tap sources only); `coeff` is the block-size-independent
    /// [`tap_coeff`]. RT-safe scalar update.
    pub fn follow(&mut self, level: f32, coeff: f32) -> f32 {
        self.smooth += coeff * (level - self.smooth);
        self.smooth
    }
}

/// One running mod-grid graph instance. A Global graph has exactly one; a Track
/// graph has one per assigned track (each with its own `host_track`).
pub struct ModGridInstance {
    /// The pooled graph this instance was built from (for provenance/rebuild).
    pub graph_id: ModGraphId,
    /// The host track for Track-scope instances; `None` for Global.
    pub host_track: Option<TrackId>,
    /// The control-rate source modules, processed once per block.
    pub dsp: crate::graph::ModuleGraph,
    /// Cheap sources injected into hosted-module input ports, applied before
    /// [`Self::dsp`] processes.
    pub injections: Vec<InputInjection>,
    /// The routing sinks this instance drives.
    pub targets: Vec<ResolvedTarget>,
}

/// The full set of running mod-grid instances. Swapped wholesale when the
/// pooled data changes (generation bump) — cheap on the audio thread (a box
/// move); the old runtime is dropped off the hot path by the command sender
/// having built the new one.
#[derive(Default)]
pub struct ModGridRuntime {
    pub instances: Vec<ModGridInstance>,
    /// Per-track offset accumulators, **pre-keyed by the builder off the audio
    /// thread** for every `Track` target — so the pre-pass only ever `get_mut`s
    /// (never inserts / allocates on the audio thread). Values reset each block.
    pub track_offsets: HashMap<TrackId, GridTrackOffset>,
    /// Per-instrument channel Volume/Pan offset accumulators, keyed by
    /// **`SeqInstrumentId`** (known off-thread, so building the slot needs no
    /// engine mapping — the offline-export ordering trap can't recur). Values
    /// reset each block; the channel-bus stage maps its engine id back to seq.
    pub instrument_offsets: HashMap<SeqInstrumentId, GridInstrumentOffset>,
}

impl ModGridRuntime {
    /// Whether there are any instances to process.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Zero every offset accumulator for a new block (RT-safe: value writes over
    /// pre-built keys, no allocation).
    pub fn reset_offsets(&mut self) {
        for o in self.track_offsets.values_mut() {
            *o = GridTrackOffset::default();
        }
        for o in self.instrument_offsets.values_mut() {
            *o = GridInstrumentOffset::default();
        }
    }

    /// Pre-create the offset accumulator slot for every `Track`/`Instrument`
    /// Volume/Pan target, so the audio-thread pre-pass only ever `get_mut`s.
    /// Called **off the audio thread** at build time (allocates). Instrument slots
    /// are keyed by `SeqInstrumentId` — no engine mapping needed, so pre-keying is
    /// order-independent (the channel-bus stage maps engine id → seq at read time).
    pub fn prekey_offsets(&mut self) {
        for inst in &self.instances {
            for routing in &inst.targets {
                match &routing.target {
                    AutomationTarget::Track {
                        track: Some(track), ..
                    } => {
                        self.track_offsets.entry(*track).or_default();
                    }
                    // Module-backed instrument params carry a `dest_addr` and take
                    // the per-voice path, so only channel-level Volume/Pan here.
                    AutomationTarget::Instrument { instrument, param }
                        if routing.dest_addr.is_none()
                            && matches!(
                                param,
                                AutoInstrumentParam::Volume | AutoInstrumentParam::Pan
                            ) =>
                    {
                        self.instrument_offsets.entry(*instrument).or_default();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Per-track additive offsets accumulated by the mod-grid pre-pass for one
/// block, folded into the track fader/pitch composition after lane automation.
/// Units match the target: `volume`/`pan` are normalized/bipolar deltas,
/// `pitch` is a semitone delta.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridTrackOffset {
    pub volume: f32,
    pub pan: f32,
    pub pitch: Semitones,
}

/// Per-instrument additive channel-level offsets from the mod-grid pre-pass for
/// one block, folded into the channel-bus fader/pan composition (the instrument
/// sibling of [`GridTrackOffset`], for `AutomationTarget::Instrument { Volume |
/// Pan }`). `volume` is a normalized gain delta, `pan` a bipolar delta.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridInstrumentOffset {
    pub volume: f32,
    pub pan: f32,
}

/// Read a control module's block-representative output value: the last sample of
/// its output buffer (control signals are block-constant enough that the final
/// sample is the current value). Returns `0.0` for an unknown port/module.
#[must_use]
pub fn read_source_value(buffer: Option<&AudioBuffer>) -> f32 {
    buffer
        .and_then(|b| b.as_slice().last().copied())
        .unwrap_or(0.0)
}

/// Compute a transport-derived value for the block. Bar phase assumes 4 beats
/// per bar (the common case); tempo is normalized against a 20..300 BPM span.
#[must_use]
pub fn transport_value(kind: TransportSource, ctx: &ProcessContext<'_>) -> f32 {
    let beats = ctx.position_beats.as_f32();
    match kind {
        TransportSource::BeatPhase => beats.rem_euclid(1.0),
        TransportSource::BarPhase => (beats / 4.0).rem_euclid(1.0),
        TransportSource::Tempo => ((ctx.tempo.as_f32() - 20.0) / 280.0).clamp(0.0, 1.0),
        TransportSource::PositionBeats => beats,
    }
}

/// RMS level of an interleaved stereo block, used by the audio-tap follower.
/// Explicit DSP loop (RT hot path), per the project's for-loops-in-DSP rule.
#[must_use]
pub fn buffer_level(interleaved: &[f32]) -> f32 {
    if interleaved.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for &x in interleaved {
        sum += x * x;
    }
    (sum / interleaved.len() as f32).sqrt()
}

impl ResolvedTarget {
    /// Advance the one-pole envelope follower toward `level` and return it;
    /// `coeff` is the block-size-independent [`tap_coeff`]. RT-safe scalar update.
    pub fn follow(&mut self, level: f32, coeff: f32) -> f32 {
        self.smooth += coeff * (level - self.smooth);
        self.smooth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_runtime_is_empty() {
        assert!(ModGridRuntime::default().is_empty());
    }

    #[test]
    fn buffer_level_is_rms() {
        assert_eq!(buffer_level(&[]), 0.0);
        assert_eq!(buffer_level(&[0.0, 0.0]), 0.0);
        // RMS of [1, -1] = 1.
        assert!((buffer_level(&[1.0, -1.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tap_coeff_is_block_size_independent() {
        // Follow a unit step for the same ~50 ms of wall-clock at two block sizes;
        // the smoothed value must land close regardless of buffer size (a fixed
        // per-block coefficient would diverge — fast at 64, slow at 512).
        let sr = 48_000.0;
        let run = |frames: usize| -> f32 {
            let coeff = tap_coeff(SampleCount::new(frames), SampleRate::new(sr));
            let blocks = (0.05 * sr / frames as f32).round() as usize;
            let mut smooth = 0.0f32;
            for _ in 0..blocks {
                smooth += coeff * (1.0 - smooth);
            }
            smooth
        };
        let (small, large) = (run(64), run(512));
        assert!(
            (small - large).abs() < 0.03,
            "tap follow should be block-size independent: 64→{small}, 512→{large}"
        );
    }

    #[test]
    fn read_source_value_takes_last_sample_or_zero() {
        assert_eq!(read_source_value(None), 0.0);
        let mut buf = AudioBuffer::new(4);
        buf.as_mut_slice().copy_from_slice(&[0.1, 0.2, 0.3, 0.42]);
        assert_eq!(read_source_value(Some(&buf)), 0.42);
        // A zero-length buffer yields the safe default.
        let empty = AudioBuffer::new(0);
        assert_eq!(read_source_value(Some(&empty)), 0.0);
    }
}
