//! Local adapters preserve the library's owned host-queue boundary.
use super::*;
pub(super) struct Ring<T>(pub HeapRb<T>);
pub(super) struct Writer<T>(HeapProd<T>);
pub(super) struct Reader<T>(HeapCons<T>);
impl<T> MixedTransferQueue<T> for Ring<T> {
    type Producer = Writer<T>;
    type Consumer = Reader<T>;
    fn capacity(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.0.capacity().get())
    }
    fn occupied(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.0.occupied_len())
    }
    fn split(self) -> (Writer<T>, Reader<T>) {
        let (writer, reader) = self.0.split();
        (Writer(writer), Reader(reader))
    }
}
impl<T> MixedTransferProducer<T> for Writer<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        self.0.try_push(packet)
    }
}
impl<T> MixedTransferConsumer<T> for Reader<T> {
    fn try_pop(&mut self) -> Option<T> {
        self.0.try_pop()
    }
}
