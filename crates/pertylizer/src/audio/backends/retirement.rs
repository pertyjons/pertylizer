//! Cold destruction of opt-in output processors after native callback closure teardown.
//!
//! Only the unique guard publishes a payload. The worker obtains exclusive mailbox
//! ownership before taking it; no reference-count snapshot authorizes destruction.
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use crate::audio::traits::AudioProcessor;
use crate::audio::types::{AudioError, AudioResult};
use atomicbox::AtomicOptionBox;

const SLOTS: usize = 8;
const WAIT: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RetirementState {
    Pending,
    Reclaiming,
    Complete,
    Failed,
}

/// This observation never keeps a callback mailbox alive.
#[derive(Clone)]
pub(crate) struct RetirementFence(Arc<AtomicU8>);
impl RetirementFence {
    #[cfg(test)]
    pub(crate) fn state(&self) -> RetirementState {
        match self.0.load(Ordering::Acquire) {
            1 => RetirementState::Reclaiming,
            2 => RetirementState::Complete,
            3 => RetirementState::Failed,
            _ => RetirementState::Pending,
        }
    }
    fn set(&self, state: RetirementState) {
        self.0.store(state as u8, Ordering::Release);
    }
}

struct Mailbox {
    payload: AtomicOptionBox<Box<dyn AudioProcessor>>,
    fence: RetirementFence,
}

/// Parent stream's lease. Drop it after the native stream field.
pub(super) struct RetirementLease {
    _mailbox: Arc<Mailbox>,
}
impl RetirementLease {
    #[cfg(test)]
    pub(super) fn fence(&self) -> RetirementFence {
        self._mailbox.fence.clone()
    }
}

pub(super) struct ProcessorGuard {
    payload: Option<Box<Box<dyn AudioProcessor>>>,
    mailbox: Arc<Mailbox>,
}
impl ProcessorGuard {
    pub(super) fn processor(&mut self) -> Option<&mut (dyn AudioProcessor + 'static)> {
        self.payload
            .as_mut()
            .map(|payload| payload.as_mut().as_mut())
    }
}
impl Drop for ProcessorGuard {
    fn drop(&mut self) {
        // Sole non-None writer, exactly once. The worker cannot unwrap this Arc
        // while this guard holds it. AcqRel is a valid atomicbox ordering.
        self.mailbox
            .payload
            .swap_mut(&mut self.payload, Ordering::AcqRel);
    }
}

struct RegistryState {
    reserved: AtomicUsize,
    failed: AtomicBool,
}
struct Registry {
    sender: mpsc::SyncSender<Arc<Mailbox>>,
    state: Arc<RegistryState>,
}

impl Registry {
    fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(SLOTS);
        let state = Arc::new(RegistryState {
            reserved: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
        });
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("audio-retirement".into())
            .spawn(move || {
                let mut slots: [Option<Arc<Mailbox>>; SLOTS] = std::array::from_fn(|_| None);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    loop {
                        let mut disconnected = false;
                        for slot in &mut slots {
                            if slot.is_none() {
                                match receiver.try_recv() {
                                    Ok(mailbox) => *slot = Some(mailbox),
                                    Err(mpsc::TryRecvError::Empty) => {}
                                    Err(mpsc::TryRecvError::Disconnected) => disconnected = true,
                                }
                            }
                            let Some(mailbox) = slot.take() else {
                                continue;
                            };
                            match Arc::try_unwrap(mailbox) {
                                Err(mailbox) => *slot = Some(mailbox),
                                Ok(mailbox) => {
                                    mailbox.fence.set(RetirementState::Reclaiming);
                                    let payload = mailbox.payload.take(Ordering::AcqRel);
                                    let dropped = std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| drop(payload)),
                                    );
                                    match dropped {
                                        Ok(()) => {
                                            worker_state.reserved.fetch_sub(1, Ordering::AcqRel);
                                            mailbox.fence.set(RetirementState::Complete);
                                        }
                                        Err(panic) => {
                                            mailbox.fence.set(RetirementState::Failed);
                                            // A panic payload may itself panic on Drop. Keep this
                                            // failed reservation permanently: at most SLOTS such
                                            // payloads can be retained by this registry.
                                            std::mem::forget(panic);
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                        if disconnected && slots.iter().all(Option::is_none) {
                            return;
                        }
                        std::thread::park_timeout(WAIT);
                    }
                }));
                if let Err(panic) = result {
                    worker_state.failed.store(true, Ordering::Release);
                    // Unexpected worker failure must not return final ownership to a
                    // delayed native callback. The admission bound includes queued slots.
                    std::mem::forget(slots);
                    std::mem::forget(receiver);
                    std::mem::forget(panic);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { sender, state })
    }

    fn register(
        &self,
        processor: Box<dyn AudioProcessor>,
    ) -> AudioResult<(ProcessorGuard, RetirementLease)> {
        if self.state.failed.load(Ordering::Acquire) {
            return Err(AudioError::StreamCreationFailed(
                "audio retirement worker failed".into(),
            ));
        }
        let mut count = self.state.reserved.load(Ordering::Acquire);
        loop {
            if count >= SLOTS {
                return Err(AudioError::StreamCreationFailed(
                    "audio retirement capacity exhausted".into(),
                ));
            }
            match self.state.reserved.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => count = actual,
            }
        }
        let mailbox = Arc::new(Mailbox {
            payload: AtomicOptionBox::new(None),
            fence: RetirementFence(Arc::new(AtomicU8::new(RetirementState::Pending as u8))),
        });
        if self.sender.try_send(mailbox.clone()).is_err() {
            self.state.reserved.fetch_sub(1, Ordering::AcqRel);
            return Err(AudioError::StreamCreationFailed(
                "audio retirement registration failed".into(),
            ));
        }
        Ok((
            ProcessorGuard {
                payload: Some(Box::new(processor)),
                mailbox: mailbox.clone(),
            },
            RetirementLease { _mailbox: mailbox },
        ))
    }
}

pub(super) fn prepare(
    processor: Box<dyn AudioProcessor>,
) -> AudioResult<(ProcessorGuard, RetirementLease)> {
    static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();
    let registry = REGISTRY
        .get_or_init(Registry::start)
        .as_ref()
        .map_err(|error| {
            AudioError::StreamCreationFailed(format!("audio retirement worker: {error}"))
        })?;
    registry.register(processor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::types::AudioCallbackContext;
    use std::sync::Mutex;
    use std::time::Instant;

    struct Probe {
        dropped: Arc<Mutex<Option<std::thread::ThreadId>>>,
    }
    impl AudioProcessor for Probe {
        fn process(&mut self, output: &mut [f32], _: &AudioCallbackContext) {
            output.fill(0.25);
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            *self.dropped.lock().unwrap() = Some(std::thread::current().id());
        }
    }
    fn wait(fence: &RetirementFence) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while fence.state() != RetirementState::Complete {
            assert!(Instant::now() < deadline, "retirement did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn terminal_guard_never_destroys_a_processor_in_either_parent_drop_order() {
        for parent_first in [false, true] {
            let registry = Registry::start().unwrap();
            let dropped = Arc::new(Mutex::new(None));
            let (guard, parent) = registry
                .register(Box::new(Probe {
                    dropped: dropped.clone(),
                }))
                .unwrap();
            let fence = parent.fence();
            let (start, started) = mpsc::sync_channel(1);
            let callback_thread = std::thread::spawn(move || {
                started.recv().unwrap();
                let measured = allocation_counter::measure(|| drop(guard));
                assert_eq!((measured.count_total, measured.count_current), (0, 0));
                std::thread::current().id()
            });
            if parent_first {
                drop(parent);
                start.send(()).unwrap();
            } else {
                start.send(()).unwrap();
                while !callback_thread.is_finished() {
                    std::thread::yield_now();
                }
                assert_eq!(fence.state(), RetirementState::Pending);
                assert!(dropped.lock().unwrap().is_none());
                drop(parent);
            }
            let callback_id = callback_thread.join().unwrap();
            wait(&fence);
            let destructor_id = dropped.lock().unwrap().unwrap();
            assert_ne!(destructor_id, callback_id);
            assert_ne!(destructor_id, std::thread::current().id());
        }
    }
    #[test]
    fn delayed_closures_hold_the_fixed_admission_bound() {
        let registry = Registry::start().unwrap();
        let mut guards = Vec::new();
        let mut fences = Vec::new();
        for _ in 0..SLOTS {
            let (guard, parent) = registry
                .register(Box::new(Probe {
                    dropped: Arc::new(Mutex::new(None)),
                }))
                .unwrap();
            fences.push(parent.fence());
            drop(parent);
            guards.push(guard);
        }
        assert!(
            registry
                .register(Box::new(Probe {
                    dropped: Arc::new(Mutex::new(None))
                }))
                .is_err()
        );
        assert!(
            fences
                .iter()
                .all(|fence| fence.state() == RetirementState::Pending)
        );
        drop(guards);
        for fence in fences {
            wait(&fence);
        }
    }
    struct Stalled {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl AudioProcessor for Stalled {
        fn process(&mut self, _: &mut [f32], _: &AudioCallbackContext) {}
    }
    impl Drop for Stalled {
        fn drop(&mut self) {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
    }
    #[test]
    fn completion_waits_for_the_destructor_and_stalled_work_cannot_spawn_workers() {
        let registry = Registry::start().unwrap();
        let (entered, entry) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let (guard, parent) = registry
            .register(Box::new(Stalled {
                entered,
                release: released,
            }))
            .unwrap();
        let fence = parent.fence();
        drop(parent);
        drop(guard);
        entry.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(fence.state(), RetirementState::Reclaiming);
        let mut held = Vec::new();
        for _ in 1..SLOTS {
            held.push(
                registry
                    .register(Box::new(Probe {
                        dropped: Arc::new(Mutex::new(None)),
                    }))
                    .unwrap(),
            );
        }
        assert!(
            registry
                .register(Box::new(Probe {
                    dropped: Arc::new(Mutex::new(None))
                }))
                .is_err()
        );
        release.send(()).unwrap();
        wait(&fence);
        drop(held);
    }
    #[test]
    fn failed_registration_returns_cold_destruction_without_consuming_a_reservation() {
        let (sender, receiver) = mpsc::sync_channel(SLOTS);
        drop(receiver);
        let registry = Registry {
            sender,
            state: Arc::new(RegistryState {
                reserved: AtomicUsize::new(0),
                failed: AtomicBool::new(false),
            }),
        };
        let dropped = Arc::new(Mutex::new(None));
        assert!(
            registry
                .register(Box::new(Probe {
                    dropped: dropped.clone()
                }))
                .is_err()
        );
        assert_eq!(*dropped.lock().unwrap(), Some(std::thread::current().id()));
        assert_eq!(registry.state.reserved.load(Ordering::Acquire), 0);
        registry.state.failed.store(true, Ordering::Release);
        assert!(registry.register(Box::new(Probe { dropped })).is_err());
        assert_eq!(registry.state.reserved.load(Ordering::Acquire), 0);
    }
    struct Panicking;
    impl AudioProcessor for Panicking {
        fn process(&mut self, _: &mut [f32], _: &AudioCallbackContext) {}
    }
    impl Drop for Panicking {
        fn drop(&mut self) {
            panic!("controlled cold destructor panic");
        }
    }
    #[test]
    fn an_unwinding_destructor_is_failed_and_the_same_worker_continues() {
        let registry = Registry::start().unwrap();
        let (guard, parent) = registry.register(Box::new(Panicking)).unwrap();
        let fence = parent.fence();
        drop(guard);
        drop(parent);
        let deadline = Instant::now() + Duration::from_secs(5);
        while fence.state() != RetirementState::Failed {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let dropped = Arc::new(Mutex::new(None));
        let (guard, parent) = registry.register(Box::new(Probe { dropped })).unwrap();
        let next = parent.fence();
        drop(guard);
        drop(parent);
        wait(&next);
        assert_eq!(registry.state.reserved.load(Ordering::Acquire), 1);
    }
}
