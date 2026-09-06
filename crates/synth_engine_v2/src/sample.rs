//! Prepared samples, sample zones and sample maps — ADR-0026.
//!
//! [ADR-0026](../../../plans/v2/decisions/ADR-0026-minimum-sample-map-and-zone-model.md)
//! separates three things V1 kept in one place: the **audio** a sampler plays, the
//! **mapping** that says which audio a note selects and how it is played, and the
//! **prepared** form the audio thread reads. This module holds the first two as the
//! authored model's types and the third as what admission builds from them.
//!
//! A [`PreparedSample`] is immutable PCM with its shape and a content digest. It is prepared
//! **off the audio thread** — the one place a sample can be refused — and then held once per
//! plan beside the prepared tunings, referenced by a [`crate::plan::SampleSlot`] that resolves
//! to one array index in the loop. The frames sit behind an `Arc` so that `N` voice instances
//! and two plans compiled from one IR share one allocation, as `SOUND-INV-025` requires of
//! every prepared record.
//!
//! A [`SampleZone`] is the unit of mapping: which keys and velocities select it, where its
//! root is, how it is tuned, which region of the sample it plays and whether a loop sits
//! inside that region. A [`SampleMap`] is an ordered list of zones. Phase 6 builds the
//! one-zone subset and refuses a longer map by name at admission, so the types admit `N`
//! zones and no type changes when a later slice extends the selection.
//!
//! What this module does **not** decide is the persisted asset behind a prepared sample —
//! its digest policy, its embedded-or-external form, its provenance. That is Phase 10A's and
//! 10D's, and clause 4 of the record states the one constraint the prepared boundary places
//! on it: a prepared sample is keyed by source digest plus preparation profile.

use crate::quantities::{
    Cents, ChannelLayout, GainFactor, KeyIdentity, NoteVelocity, QuantityError, SampleRate,
};
use std::sync::Arc;
use thiserror::Error;

/// Why a sample, a zone or a map could not be built.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SampleError {
    /// The audio holds no frames.
    #[error("a sample needs at least one frame")]
    Empty,
    /// The interleaved buffer's length is not a whole number of frames.
    #[error("{samples} samples do not divide into {channels:?} frames")]
    RaggedFrames {
        /// How many samples the buffer holds.
        samples: usize,
        /// The layout they were declared as.
        channels: ChannelLayout,
    },
    /// A sample value is not a finite number.
    #[error("sample {index} is {value}, which is not a finite value")]
    NotFinite {
        /// Which sample.
        index: usize,
        /// What it holds.
        value: f32,
    },
    /// A frame count the loop cannot index.
    #[error("{frames} frames is more than a region can address")]
    TooLong {
        /// How many frames the sample holds.
        frames: usize,
    },
    /// A region's end is not after its start.
    #[error("a region from {start} to {end} holds no frame")]
    EmptyRegion {
        /// Where it starts.
        start: SampleFrame,
        /// Where it ends, exclusive.
        end: SampleFrame,
    },
    /// A loop is not inside the region it repeats.
    #[error("a loop from {start} to {end} leaves the region {region_start}..{region_end}")]
    LoopOutsideRegion {
        /// The loop's start.
        start: SampleFrame,
        /// The loop's end, exclusive.
        end: SampleFrame,
        /// The region's start.
        region_start: SampleFrame,
        /// The region's end, exclusive.
        region_end: SampleFrame,
    },
    /// A range's high end is below its low end.
    #[error("a range whose low end is above its high end selects nothing")]
    InvertedRange,
    /// A quantity the zone carries was refused by its own type.
    #[error("a zone quantity was refused: {0}")]
    Quantity(#[from] QuantityError),
}

/// A frame position inside a sample.
///
/// Its own type rather than [`crate::time::FrameCount`]: that one is a position on the
/// stream's timeline, and a sample frame is a position in a buffer whose length the
/// prepared record fixes. `u32` because a region is indexed by it on the audio thread as a
/// `usize` after one lossless widening, and a sample longer than four billion frames is not
/// a sample this phase prepares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SampleFrame(u32);

impl SampleFrame {
    /// The first frame.
    pub const FIRST: Self = Self(0);

    /// A frame position.
    pub const fn new(frame: u32) -> Self {
        Self(frame)
    }

    /// The position as a number.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// The position as an index.
    pub const fn as_index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for SampleFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame {}", self.0)
    }
}

/// What a prepared sample is, for the resource report and for telling two apart.
///
/// FNV-1a over the bit pattern of every sample, as a tuning's digest is: deterministic,
/// allocation-free, and exact — a tolerance would make two audibly different samples report
/// as one. It is **not** what deduplication compares; that compares the frames themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct SampleDigest(u64);

impl SampleDigest {
    /// The digest as a number.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for SampleDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sample {:016x}", self.0)
    }
}

/// Immutable PCM with its shape and digest — ADR-0026 clause 3.
///
/// Interleaved `f32` frames, one or two channels, a frame count and the rate the audio was
/// recorded at. Prepared off the audio thread, held once per plan, and read in the loop
/// through one array index. Compared by **content**: two preparations of one buffer are
/// equal, which is what lets admission hold them once.
#[derive(Debug, Clone)]
#[must_use]
pub struct PreparedSample {
    /// The interleaved frames. `pub(crate)` so the kernel reads them as a field rather than
    /// through a call the real-time scan would have to allow by name.
    pub(crate) frames: Arc<[f32]>,
    /// How the frames are laid out.
    pub(crate) channels: ChannelLayout,
    /// How many frames there are.
    pub(crate) frame_count: SampleFrame,
    rate: SampleRate,
    digest: SampleDigest,
}

impl PartialEq for PreparedSample {
    fn eq(&self, other: &Self) -> bool {
        self.channels == other.channels
            && self.rate == other.rate
            && self.digest == other.digest
            && self.frames.len() == other.frames.len()
            && self
                .frames
                .iter()
                .zip(other.frames.iter())
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl PreparedSample {
    /// Prepare a sample from interleaved audio.
    ///
    /// Runs off the audio thread. Refuses an empty buffer, a buffer that is not a whole
    /// number of frames, a non-finite sample and a length the loop cannot index — this is the
    /// last place a diagnostic can be produced, and a value refused here never reaches a
    /// kernel.
    pub fn prepare(
        frames: Vec<f32>,
        channels: ChannelLayout,
        rate: SampleRate,
    ) -> Result<Self, SampleError> {
        let width = channels.channels();
        if frames.is_empty() {
            return Err(SampleError::Empty);
        }
        if !frames.len().is_multiple_of(width) {
            return Err(SampleError::RaggedFrames {
                samples: frames.len(),
                channels,
            });
        }
        let count = frames.len() / width;
        let Ok(count) = u32::try_from(count) else {
            return Err(SampleError::TooLong { frames: count });
        };
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for (index, value) in frames.iter().copied().enumerate() {
            if !value.is_finite() {
                return Err(SampleError::NotFinite { index, value });
            }
            for byte in value.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        Ok(Self {
            frames: Arc::from(frames),
            channels,
            frame_count: SampleFrame::new(count),
            rate,
            digest: SampleDigest(hash),
        })
    }

    /// The interleaved frames.
    #[must_use]
    pub fn frames(&self) -> &[f32] {
        &self.frames
    }

    /// How the frames are laid out.
    #[must_use]
    pub const fn channels(&self) -> ChannelLayout {
        self.channels
    }

    /// How many frames there are.
    pub const fn frame_count(&self) -> SampleFrame {
        self.frame_count
    }

    /// The rate the audio was recorded at.
    pub const fn rate(&self) -> SampleRate {
        self.rate
    }

    /// What this sample is, for the report and for comparing two preparations.
    pub const fn digest(&self) -> SampleDigest {
        self.digest
    }

    /// The bytes this sample occupies once prepared: its frames and its record.
    ///
    /// Charged **once** to a plan's immutable prepared total however many zones reference
    /// it, which is what makes a second sample visible in the report as something other
    /// than a second node.
    #[must_use]
    pub fn prepared_bytes(&self) -> u64 {
        (self.frames.len() as u64)
            .saturating_mul(size_of::<f32>() as u64)
            .saturating_add(size_of::<Self>() as u64)
    }
}

/// Which prepared sample of the plan a zone plays, as the IR names it.
///
/// The IR carries its samples in a table and a zone names one by this reference, for the
/// reason a node is named by a [`crate::ir::NodeId`]: [`crate::ir::IrNodeKind`] is `Copy` and
/// a buffer is not, so the kind carries an index and the plan carries the table. Admission
/// resolves it to a [`crate::plan::SampleSlot`], deduplicated by content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SampleRef(u32);

impl SampleRef {
    /// A reference.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The index into the IR's sample table.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for SampleRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sample ref {}", self.0)
    }
}

/// Which sample map of the plan a sampler consumes, as the IR names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SampleMapRef(u32);

impl SampleMapRef {
    /// A reference.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The index into the IR's map table.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for SampleMapRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sample map {}", self.0)
    }
}

/// The frames a zone plays: a start and an exclusive end inside the sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PlaybackRegion {
    start: SampleFrame,
    end: SampleFrame,
}

impl PlaybackRegion {
    /// A region. Refused when it holds no frame; whether it lies inside the sample is
    /// checked where the sample is known, at IR construction.
    pub const fn new(start: SampleFrame, end: SampleFrame) -> Result<Self, SampleError> {
        if end.0 <= start.0 {
            return Err(SampleError::EmptyRegion { start, end });
        }
        Ok(Self { start, end })
    }

    /// Where it starts.
    pub const fn start(self) -> SampleFrame {
        self.start
    }

    /// Where it ends, exclusive.
    pub const fn end(self) -> SampleFrame {
        self.end
    }

    /// How many frames it holds; never zero, by construction.
    #[must_use]
    pub const fn frames(self) -> u32 {
        self.end.0 - self.start.0
    }
}

/// The frames a zone repeats while its trigger is held, inside its region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopRegion {
    start: SampleFrame,
    end: SampleFrame,
}

impl LoopRegion {
    /// A loop. Refused when it holds no frame or leaves the region.
    pub const fn new(
        start: SampleFrame,
        end: SampleFrame,
        region: PlaybackRegion,
    ) -> Result<Self, SampleError> {
        if end.0 <= start.0 {
            return Err(SampleError::EmptyRegion { start, end });
        }
        if start.0 < region.start.0 || end.0 > region.end.0 {
            return Err(SampleError::LoopOutsideRegion {
                start,
                end,
                region_start: region.start,
                region_end: region.end,
            });
        }
        Ok(Self { start, end })
    }

    /// Where it starts.
    pub const fn start(self) -> SampleFrame {
        self.start
    }

    /// Where it ends, exclusive.
    pub const fn end(self) -> SampleFrame {
        self.end
    }
}

/// The keys that select a zone, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct KeyRange {
    low: KeyIdentity,
    high: KeyIdentity,
}

impl KeyRange {
    /// The whole keyboard.
    pub const FULL: Self = Self {
        low: KeyIdentity::LOWEST,
        high: KeyIdentity::HIGHEST,
    };

    /// A range. Refused when inverted.
    pub const fn new(low: KeyIdentity, high: KeyIdentity) -> Result<Self, SampleError> {
        if high.as_u8() < low.as_u8() {
            return Err(SampleError::InvertedRange);
        }
        Ok(Self { low, high })
    }

    /// Whether a key selects this zone.
    #[must_use]
    pub const fn holds(self, key: KeyIdentity) -> bool {
        key.as_u8() >= self.low.as_u8() && key.as_u8() <= self.high.as_u8()
    }
}

/// The velocities that select a zone, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct VelocityRange {
    low: NoteVelocity,
    high: NoteVelocity,
}

impl VelocityRange {
    /// Every velocity.
    pub const FULL: Self = Self {
        low: NoteVelocity::SILENT,
        high: NoteVelocity::FULL,
    };

    /// A range. Refused when inverted.
    pub fn new(low: NoteVelocity, high: NoteVelocity) -> Result<Self, SampleError> {
        if high.as_f32() < low.as_f32() {
            return Err(SampleError::InvertedRange);
        }
        Ok(Self { low, high })
    }

    /// Whether a velocity selects this zone.
    #[must_use]
    pub fn holds(self, velocity: NoteVelocity) -> bool {
        velocity.as_f32() >= self.low.as_f32() && velocity.as_f32() <= self.high.as_f32()
    }
}

/// How a zone plays once triggered — V1's three modes, ADR-0026 clause 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayMode {
    /// Plays the region out and ignores the off edge.
    OneShot,
    /// Plays once; the off edge starts a linear fade of [`SUSTAIN_FADE_FRAMES`] frames.
    Sustain,
    /// Repeats the loop while the trigger is held, then fades as `Sustain` does. A zone
    /// declaring no loop is refused at admission under this mode rather than played as
    /// `Sustain`.
    Loop,
}

/// The frames V1's player fades over on the off edge in `Sustain` and `Loop` mode:
/// "~10 ms at 48 kHz", hard-coded there and reproduced here.
pub const SUSTAIN_FADE_FRAMES: u32 = 512;

/// Which way a zone plays. Only `Forward` is built; the others are refused by name at
/// admission until a consumer reaches them (ADR-0026 option Q3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayDirection {
    /// From the region's start toward its end.
    Forward,
    /// From the region's end toward its start. Not built.
    Reverse,
    /// Alternating at the loop's boundaries. Not built.
    PingPong,
}

/// One zone of a sample map — ADR-0026 clause 1.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct SampleZone {
    keys: KeyRange,
    velocities: VelocityRange,
    root: KeyIdentity,
    fine_tune: Cents,
    region: PlaybackRegion,
    loop_region: Option<LoopRegion>,
    gain: GainFactor,
    sample: SampleRef,
}

impl SampleZone {
    /// A zone playing `region` of `sample` with `root` at the recorded rate, selected by
    /// every key and velocity, untuned, unlooped and at unity gain until the builders
    /// below say otherwise. The region against the sample is checked at IR construction,
    /// where the sample is; a loop against the region by [`LoopRegion::new`].
    pub const fn new(sample: SampleRef, root: KeyIdentity, region: PlaybackRegion) -> Self {
        Self {
            keys: KeyRange::FULL,
            velocities: VelocityRange::FULL,
            root,
            fine_tune: Cents::ZERO,
            region,
            loop_region: None,
            gain: GainFactor::UNITY,
            sample,
        }
    }

    /// The keys that select it.
    pub const fn selected_by_keys(mut self, keys: KeyRange) -> Self {
        self.keys = keys;
        self
    }

    /// The velocities that select it.
    pub const fn selected_by_velocities(mut self, velocities: VelocityRange) -> Self {
        self.velocities = velocities;
        self
    }

    /// An offset on top of the root, in cents.
    pub const fn tuned_by(mut self, fine_tune: Cents) -> Self {
        self.fine_tune = fine_tune;
        self
    }

    /// The frames it repeats while its trigger is held.
    pub const fn looping(mut self, loop_region: LoopRegion) -> Self {
        self.loop_region = Some(loop_region);
        self
    }

    /// Its own level.
    pub const fn at_gain(mut self, gain: GainFactor) -> Self {
        self.gain = gain;
        self
    }

    /// The keys that select it.
    pub const fn keys(&self) -> KeyRange {
        self.keys
    }

    /// The velocities that select it.
    pub const fn velocities(&self) -> VelocityRange {
        self.velocities
    }

    /// The key the sample plays at its recorded rate.
    pub const fn root(&self) -> KeyIdentity {
        self.root
    }

    /// The offset applied on top of the root, in cents.
    pub const fn fine_tune(&self) -> Cents {
        self.fine_tune
    }

    /// The frames it plays.
    pub const fn region(&self) -> PlaybackRegion {
        self.region
    }

    /// The frames it repeats, if any.
    pub const fn loop_region(&self) -> Option<LoopRegion> {
        self.loop_region
    }

    /// Its own level.
    pub const fn gain(&self) -> GainFactor {
        self.gain
    }

    /// The sample it plays.
    pub const fn sample(&self) -> SampleRef {
        self.sample
    }
}

/// An ordered list of zones — ADR-0026 clause 1.
///
/// Phase 6 admits a map of exactly one zone and refuses a longer one by name; the type
/// admits `N` so that extending the selection later changes no type.
#[derive(Debug, Clone, PartialEq, Default)]
#[must_use]
pub struct SampleMap {
    zones: Vec<SampleZone>,
}

impl SampleMap {
    /// A map holding these zones, in this order.
    pub fn new(zones: Vec<SampleZone>) -> Self {
        Self { zones }
    }

    /// The zones, in order.
    pub fn zones(&self) -> &[SampleZone] {
        &self.zones
    }
}
