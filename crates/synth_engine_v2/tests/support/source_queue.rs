//! Exact bounded source handoff shared by the example host and private mixed tests.

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use thiserror::Error;

#[must_use]
pub(super) struct SourceQueueWriter<T> {
    queue: HeapProd<T>,
}

#[must_use]
pub(super) struct SourceQueueReader<T> {
    queue: HeapCons<T>,
}

#[derive(Debug, Error, PartialEq)]
#[must_use]
pub(super) enum SourceQueueCommitError<T> {
    #[error("source queue is empty")]
    Empty,
    #[error("source queue head differs from expected")]
    Mismatched { actual: T },
}

impl<T> SourceQueueWriter<T> {
    pub(super) fn prepare(cells: usize) -> (Self, SourceQueueReader<T>) {
        let (writer, reader) = HeapRb::new(cells).split();
        (Self { queue: writer }, SourceQueueReader { queue: reader })
    }

    pub(super) fn try_push(&mut self, packet: T) -> Result<(), T> {
        self.queue.try_push(packet)
    }

    pub(super) fn occupied_len(&self) -> usize {
        self.queue.occupied_len()
    }
}

impl<T: Copy + PartialEq> SourceQueueReader<T> {
    pub(super) fn peek(&self) -> Option<T> {
        self.queue.try_peek().copied()
    }

    pub(super) fn commit_exact(&mut self, expected: T) -> Result<T, SourceQueueCommitError<T>> {
        let actual = self.peek().ok_or(SourceQueueCommitError::Empty)?;
        if actual != expected {
            return Err(SourceQueueCommitError::Mismatched { actual });
        }
        self.queue.try_pop().ok_or(SourceQueueCommitError::Empty)
    }

    pub(super) fn occupied_len(&self) -> usize {
        self.queue.occupied_len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
