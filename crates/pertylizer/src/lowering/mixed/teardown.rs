//! Cold, process-local accounting of the returned source and audio owners.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct MixedRetirement {
    physical_keys: HeldNoteCount,
    pending_packets: MixedTransferPlaces,
    open_handles: MixedTransferPlaces,
    refused_keys: MixedTransferPlaces,
    pedal_scopes: MixedTransferPlaces,
}
impl MixedRetirement {
    pub const fn physical_keys(self) -> HeldNoteCount {
        self.physical_keys
    }
    pub const fn pending_packets(self) -> MixedTransferPlaces {
        self.pending_packets
    }
    pub const fn open_handles(self) -> MixedTransferPlaces {
        self.open_handles
    }
    pub const fn refused_keys(self) -> MixedTransferPlaces {
        self.refused_keys
    }
    pub const fn pedal_scopes(self) -> MixedTransferPlaces {
        self.pedal_scopes
    }

    pub(super) fn account(
        teardown: &synth_engine_v2::stream::MixedBridgeTeardown,
        physical: &[MixedSourcePacketId],
    ) -> Result<Self, MixedPlaybackError> {
        if let Some(fault) = teardown.fault() {
            return Err(preparation(fault));
        }
        if teardown.transfer().custody() != synth_engine_v2::stream::MixedTransferCustody::Complete
        {
            return Err(preparation(
                "retirement has lost or unknown command custody",
            ));
        }
        for onset in physical {
            let found = teardown.held().iter().any(|(packet, _)| packet == onset)
                || teardown
                    .open()
                    .iter()
                    .any(|(packet, _)| packet.as_ref() == Some(onset))
                || teardown
                    .tombstones()
                    .iter()
                    .any(|(packet, _)| packet == onset);
            if !found {
                return Err(preparation(
                    "a physical software key has no teardown custody record",
                ));
            }
        }
        Ok(Self {
            physical_keys: HeldNoteCount::measured(
                u32::try_from(physical.len()).map_err(preparation)?,
            ),
            pending_packets: MixedTransferPlaces::new(teardown.held().len()),
            open_handles: MixedTransferPlaces::new(teardown.open().len()),
            refused_keys: MixedTransferPlaces::new(teardown.tombstones().len()),
            pedal_scopes: MixedTransferPlaces::new(teardown.pedals().len()),
        })
    }
}
