//! A single-writer latest complete publication with bounded single-attempt reads.
//! Every payload field is atomic; there is no data race, unsafe access, spinning or
//! callback reclamation. SeqCst gives one total order for both validation stamps and
//! payload fields: equal even stamps therefore enclose one complete publication.
use super::*;
use thiserror::Error;
mod hot;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy)]
struct PublicationVersion(u64);
#[derive(Debug)]
struct Shared {
    root: MixedPublicationOwner,
    sources: [ConnectionGeneration; 2],
    version: AtomicU64,
    binding: AtomicU64,
    successor: AtomicU64,
    boundary: AtomicU64,
    sound: AtomicU64,
    raw: AtomicU64,
    raw_interrupted: AtomicU64,
}
/// Unique control-side writer. It replaces the latest value without waiting for
/// audio to consume the preceding one. Allocate and destroy both halves off audio.
#[derive(Debug)]
#[must_use]
pub struct MixedPublicationSender {
    shared: Arc<Shared>,
    version: PublicationVersion,
}
/// Audio-owned reader. A colliding writer yields no update, leaving the caller's
/// previous complete proof intact. Each call executes at most eight atomic loads.
#[derive(Debug)]
#[must_use]
pub struct MixedPublicationReceiver {
    shared: Arc<Shared>,
    seen: PublicationVersion,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedPublicationPublishError {
    #[error("publication mailbox belongs to another bridge or raw sources")]
    Foreign,
    #[error("publication mailbox version space exhausted")]
    Exhausted,
}
impl MixedPublicationSender {
    pub(in super::super) fn new(
        root: MixedTransferBinding,
        sources: [ConnectionGeneration; 2],
    ) -> (Self, MixedPublicationReceiver) {
        let shared = Arc::new(Shared {
            root: MixedPublicationOwner::of(root),
            sources,
            version: AtomicU64::new(0),
            binding: AtomicU64::new(0),
            successor: AtomicU64::new(0),
            boundary: AtomicU64::new(0),
            sound: AtomicU64::new(0),
            raw: AtomicU64::new(0),
            raw_interrupted: AtomicU64::new(0),
        });
        (
            Self {
                shared: Arc::clone(&shared),
                version: PublicationVersion(0),
            },
            MixedPublicationReceiver {
                shared,
                seen: PublicationVersion(0),
            },
        )
    }
    /// Replace the complete pair after all covered FIFO pushes have succeeded.
    ///
    /// # Errors
    /// Foreign bridge/source bindings or version exhaustion; nothing is overwritten.
    pub fn publish(
        &mut self,
        sound: MixedPublication,
        raw: MixedRawPublication,
    ) -> Result<(), MixedPublicationPublishError> {
        if sound.root != self.shared.root
            || sound.sources != self.shared.sources
            || raw.sources() != self.shared.sources
        {
            return Err(MixedPublicationPublishError::Foreign);
        }
        let next = self
            .version
            .0
            .checked_add(2)
            .ok_or(MixedPublicationPublishError::Exhausted)?;
        self.shared
            .version
            .store(self.version.0 + 1, Ordering::SeqCst);
        self.shared.binding.store(sound.binding.0, Ordering::SeqCst);
        let (successor, boundary) = sound
            .successor
            .map_or((0, 0), |(owner, at)| (owner.0, at.as_u64()));
        self.shared.successor.store(successor, Ordering::SeqCst);
        self.shared.boundary.store(boundary, Ordering::SeqCst);
        self.shared
            .sound
            .store(sound.through.as_u64(), Ordering::SeqCst);
        self.shared
            .raw
            .store(raw.through().as_u64(), Ordering::SeqCst);
        self.shared
            .raw_interrupted
            .store(u64::from(raw.is_interrupted()), Ordering::SeqCst);
        self.shared.version.store(next, Ordering::SeqCst);
        self.version = PublicationVersion(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        MixedPublicationSender,
        MixedPublicationReceiver,
        MixedPublication,
    ) {
        let sources = [
            crate::host::issue_capture_source_generation().expect("source"),
            crate::host::issue_capture_source_generation().expect("source"),
        ];
        // Internal transport-copy fixture, independent of plan/renderer admission.
        let root = MixedPublicationOwner(1);
        let shared = Arc::new(Shared {
            root,
            sources,
            version: AtomicU64::new(0),
            binding: AtomicU64::new(0),
            successor: AtomicU64::new(0),
            boundary: AtomicU64::new(0),
            sound: AtomicU64::new(0),
            raw: AtomicU64::new(0),
            raw_interrupted: AtomicU64::new(0),
        });
        let sound = MixedPublication {
            root,
            sources,
            binding: MixedPublicationOwner(2),
            successor: None,
            through: SampleTime::new(128),
        };
        (
            MixedPublicationSender {
                shared: Arc::clone(&shared),
                version: PublicationVersion(0),
            },
            MixedPublicationReceiver {
                shared,
                seen: PublicationVersion(0),
            },
            sound,
        )
    }
    #[test]
    fn newest_complete_pair_replaces_unread_values_without_heap_work() {
        let (mut writer, mut reader, mut sound) = fixture();
        let raw = |through| {
            MixedRawPublication::from_mailbox(sound.sources, SampleTime::new(through), false)
        };
        writer.publish(sound, raw(128)).expect("first");
        sound.through = SampleTime::new(256);
        writer.publish(sound, raw(256)).expect("replace unread");
        let measured = crate::render_allocation::count_allocs(|| {
            assert_eq!(reader.try_read(), Some((sound, raw(256))));
            assert_eq!(reader.try_read(), None);
        });
        assert_eq!(measured, 0);
    }
    #[test]
    fn an_odd_in_flight_version_returns_immediately_without_a_partial_pair() {
        let (mut writer, mut reader, sound) = fixture();
        let raw = MixedRawPublication::from_mailbox(sound.sources, sound.through, false);
        writer.publish(sound, raw).expect("first");
        writer.shared.version.store(3, Ordering::SeqCst);
        assert_eq!(reader.try_read(), None);
        writer.shared.version.store(2, Ordering::SeqCst);
        assert_eq!(reader.try_read(), Some((sound, raw)));
    }
    #[test]
    fn version_exhaustion_preserves_the_last_complete_pair() {
        let (mut writer, mut reader, sound) = fixture();
        let raw = MixedRawPublication::from_mailbox(sound.sources, sound.through, false);
        writer.publish(sound, raw).expect("first");
        writer.version = PublicationVersion(u64::MAX - 1);
        assert_eq!(
            writer.publish(sound, raw),
            Err(MixedPublicationPublishError::Exhausted)
        );
        assert_eq!(writer.shared.version.load(Ordering::SeqCst), 2);
        assert_eq!(reader.try_read(), Some((sound, raw)));
    }
    #[test]
    fn classified_raw_interruption_is_copied_without_inventing_raw_coverage() {
        let (mut writer, mut reader, sound) = fixture();
        let raw = MixedRawPublication::from_mailbox(sound.sources, SampleTime::ZERO, true);
        writer.publish(sound, raw).expect("interruption");
        let (copied_sound, copied_raw) = reader.try_read().expect("whole pair");
        assert_eq!(copied_sound, sound);
        assert_eq!(copied_raw, raw);
        assert!(copied_raw.is_interrupted());
        assert_eq!(copied_raw.through(), SampleTime::ZERO);
    }
    #[test]
    fn concurrent_replacements_never_combine_two_publications() {
        let (mut writer, mut reader, mut sound) = fixture();
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for value in 1..=10_000 {
                    sound.through = SampleTime::new(value);
                    writer
                        .publish(
                            sound,
                            MixedRawPublication::from_mailbox(sound.sources, sound.through, false),
                        )
                        .expect("bounded versions");
                }
                done.store(true, Ordering::Release);
            });
            while !done.load(Ordering::Acquire) {
                if let Some((sound, raw)) = reader.try_read() {
                    assert_eq!(sound.through, raw.through());
                }
            }
        });
        if let Some((sound, raw)) = reader.try_read() {
            assert_eq!(sound.through, raw.through());
        }
        assert_eq!(reader.shared.sound.load(Ordering::SeqCst), 10_000);
    }
}
