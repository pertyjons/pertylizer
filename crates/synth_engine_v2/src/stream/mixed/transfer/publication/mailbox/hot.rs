//! Bounded atomic publication reads; no allocation, retry loop or reclamation.
use super::{
    MixedPublication, MixedPublicationOwner, MixedPublicationReceiver, MixedRawPublication,
    Ordering, PublicationVersion, SampleTime,
};

impl MixedPublicationReceiver {
    /// Read the latest complete pair once. Never waits or retries a writer in flight.
    pub fn try_read(&mut self) -> Option<(MixedPublication, MixedRawPublication)> {
        let before = self.shared.version.load(Ordering::SeqCst);
        if before == self.seen.0 || before == 0 || !before.is_multiple_of(2) {
            return None;
        }
        let binding = self.shared.binding.load(Ordering::SeqCst);
        let successor = self.shared.successor.load(Ordering::SeqCst);
        let boundary = self.shared.boundary.load(Ordering::SeqCst);
        let sound = self.shared.sound.load(Ordering::SeqCst);
        let raw = self.shared.raw.load(Ordering::SeqCst);
        let interrupted = self.shared.raw_interrupted.load(Ordering::SeqCst) == 1;
        if before != self.shared.version.load(Ordering::SeqCst) {
            return None;
        }
        self.seen = PublicationVersion(before);
        // IDs and source bindings came from an opaque library certificate. The
        // version test validates their complete atomic copy before reconstruction.
        Some((
            MixedPublication {
                root: self.shared.root,
                sources: self.shared.sources,
                binding: MixedPublicationOwner(binding),
                successor: (successor != 0)
                    .then_some((MixedPublicationOwner(successor), SampleTime::new(boundary))),
                through: SampleTime::new(sound),
            },
            MixedRawPublication::from_mailbox(
                self.shared.sources,
                SampleTime::new(raw),
                interrupted,
            ),
        ))
    }
}
