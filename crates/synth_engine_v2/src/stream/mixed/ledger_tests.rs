//! Reduced two-source custody model. It does not stand in for raw input or audio.

use std::collections::VecDeque;

use crate::{
    quantities::KeyIdentity,
    recording::notes::{Midi1Event, Midi1Input},
};
use synth_core::MidiChannel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelSource {
    First,
    Second,
}

impl ModelSource {
    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OccurrenceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Key {
    source: ModelSource,
    channel: MidiChannel,
    note: KeyIdentity,
}

#[derive(Debug)]
struct Attempt {
    id: OccurrenceId,
    key: Key,
    input: Midi1Input,
}

#[derive(Debug)]
struct RetryToken {
    id: OccurrenceId,
    source: ModelSource,
}

struct PendingRetry {
    attempt: Attempt,
    charge: Counts,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Retirement {
    id: OccurrenceId,
    key: Key,
    input: Midi1Input,
    returned: Counts,
}

#[derive(Debug)]
enum SubmitError {
    NoCredit,
    Retry(RetryToken),
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleaseRoute {
    id: OccurrenceId,
    raw: bool,
    ingress: bool,
}

struct Entry {
    id: OccurrenceId,
    key: Key,
    released: bool,
    raw_held: bool,
    ingress_held: bool,
    result_held: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counts {
    tracker: usize,
    ingress: usize,
    results: usize,
    ledger: usize,
}

/// The model acquires all four credits before queue custody. Each consumer
/// returns only its own credit after its outcome, not after ring removal.
struct Model {
    rings: [VecDeque<Attempt>; 2],
    retry_pending: [Option<PendingRetry>; 2],
    entries: Vec<Entry>,
    next: u64,
    used: Counts,
    limits: Counts,
    ring_limit: usize,
}

impl Model {
    fn new(limits: Counts, ring_limit: usize) -> Self {
        Self {
            rings: [VecDeque::new(), VecDeque::new()],
            retry_pending: [None, None],
            entries: Vec::new(),
            next: 0,
            used: Counts {
                tracker: 0,
                ingress: 0,
                results: 0,
                ledger: 0,
            },
            limits,
            ring_limit,
        }
    }

    fn submit(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
    ) -> Result<OccurrenceId, SubmitError> {
        if self.retry_pending[source.index()].is_some() {
            return Err(SubmitError::Blocked);
        }
        let Midi1Event::NoteOn { key, .. } = input.event() else {
            panic!("the reduced submit operation accepts only note onsets");
        };
        if self.used.tracker >= self.limits.tracker
            || self.used.ingress >= self.limits.ingress
            || self.used.results >= self.limits.results
            || self.used.ledger >= self.limits.ledger
        {
            return Err(SubmitError::NoCredit);
        }
        let Some(next) = self.next.checked_add(1) else {
            return Err(SubmitError::NoCredit);
        };
        self.next = next;
        self.used.tracker += 1;
        self.used.ingress += 1;
        self.used.results += 1;
        self.used.ledger += 1;
        let attempt = Attempt {
            id: OccurrenceId(next),
            key: Key {
                source,
                channel: input.channel(),
                note: key,
            },
            input,
        };
        if self.rings[source.index()].len() == self.ring_limit {
            let token = RetryToken {
                id: attempt.id,
                source,
            };
            self.retry_pending[source.index()] = Some(PendingRetry {
                attempt,
                charge: limits(1),
            });
            return Err(SubmitError::Retry(token));
        }
        let id = attempt.id;
        self.rings[source.index()].push_back(attempt);
        Ok(id)
    }

    fn retry(&mut self, token: RetryToken) -> Result<OccurrenceId, RetryToken> {
        let index = token.source.index();
        if self.retry_pending[index]
            .as_ref()
            .is_none_or(|pending| pending.attempt.id != token.id)
            || self.rings[index].len() == self.ring_limit
        {
            return Err(token);
        }
        let pending = self.retry_pending[index]
            .take()
            .expect("matching pending retry");
        let id = pending.attempt.id;
        self.rings[index].push_back(pending.attempt);
        Ok(id)
    }

    fn retire(&mut self, token: RetryToken) -> Result<Retirement, RetryToken> {
        let index = token.source.index();
        if self.retry_pending[index]
            .as_ref()
            .is_none_or(|pending| pending.attempt.id != token.id)
        {
            return Err(token);
        }
        let pending = self.retry_pending[index]
            .take()
            .expect("matching pending retry");
        self.used.tracker -= pending.charge.tracker;
        self.used.ingress -= pending.charge.ingress;
        self.used.results -= pending.charge.results;
        self.used.ledger -= pending.charge.ledger;
        Ok(Retirement {
            id: pending.attempt.id,
            key: pending.attempt.key,
            input: pending.attempt.input,
            returned: pending.charge,
        })
    }

    fn service(&mut self, source: ModelSource, raw: bool, ingress: bool) -> OccurrenceId {
        let attempt = self.rings[source.index()]
            .pop_front()
            .expect("queued source occurrence");
        if !raw {
            self.used.tracker -= 1;
        }
        if !ingress {
            self.used.ingress -= 1;
        }
        self.entries.push(Entry {
            id: attempt.id,
            key: attempt.key,
            released: false,
            raw_held: raw,
            ingress_held: ingress,
            result_held: true,
        });
        attempt.id
    }

    fn release(&mut self, source: ModelSource, input: Midi1Input) -> Option<ReleaseRoute> {
        let Midi1Event::KeyRelease { key, .. } = input.event() else {
            panic!("the reduced release operation accepts only key releases");
        };
        let key = Key {
            source,
            channel: input.channel(),
            note: key,
        };
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.key == key && !entry.released)?;
        entry.released = true;
        let route = ReleaseRoute {
            id: entry.id,
            raw: entry.raw_held,
            ingress: entry.ingress_held,
        };
        self.reap(route.id);
        Some(route)
    }

    fn settle_raw_release(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.raw_held);
        entry.raw_held = false;
        self.used.tracker -= 1;
        self.reap(id);
    }

    fn settle_ingress_release(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.ingress_held);
        entry.ingress_held = false;
        self.used.ingress -= 1;
        self.reap(id);
    }

    fn collect_result(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.result_held);
        entry.result_held = false;
        self.used.results -= 1;
        self.reap(id);
    }

    fn entry_mut(&mut self, id: OccurrenceId) -> &mut Entry {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .expect("source occurrence remains owned")
    }

    fn reap(&mut self, id: OccurrenceId) {
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.id == id
                && entry.released
                && !entry.raw_held
                && !entry.ingress_held
                && !entry.result_held
        }) {
            self.entries.remove(index);
            self.used.ledger -= 1;
        }
    }
}

fn input(status: u8, key: u8, velocity: u8) -> Midi1Input {
    Midi1Input::from_bytes([status, key, velocity]).expect("valid test MIDI")
}

fn limits(count: usize) -> Counts {
    Counts {
        tracker: count,
        ingress: count,
        results: count,
        ledger: count,
    }
}

#[test]
fn same_key_fifo_retains_accepted_then_refused_occurrences() {
    let mut model = Model::new(limits(3), 3);
    let first = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("first queue custody");
    let second = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("second queue custody");
    assert_eq!(model.service(ModelSource::First, true, true), first);
    assert_eq!(model.service(ModelSource::First, false, false), second);
    let first_route = model
        .release(ModelSource::First, input(0x90, 60, 0))
        .expect("velocity-zero release");
    assert_eq!(
        first_route,
        ReleaseRoute {
            id: first,
            raw: true,
            ingress: true
        }
    );
    let second_route = model
        .release(ModelSource::First, input(0x80, 60, 0))
        .expect("refused tombstone release");
    assert_eq!(
        second_route,
        ReleaseRoute {
            id: second,
            raw: false,
            ingress: false
        }
    );
    assert_eq!(model.release(ModelSource::First, input(0x80, 60, 0)), None);
}

#[test]
fn stray_release_from_other_source_cannot_spend_same_key_credit() {
    let mut model = Model::new(limits(1), 1);
    let held = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("queue custody");
    model.service(ModelSource::First, true, true);
    assert_eq!(model.release(ModelSource::Second, input(0x80, 60, 0)), None);
    assert_eq!(model.used, limits(1));
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 60, 0)),
        Some(ReleaseRoute {
            id: held,
            raw: true,
            ingress: true,
        })
    );
}

#[test]
fn partial_consumer_admission_keeps_each_release_path() {
    let mut model = Model::new(limits(2), 2);
    let raw_only = model
        .submit(ModelSource::First, input(0x90, 61, 100))
        .expect("first queue custody");
    let ingress_only = model
        .submit(ModelSource::First, input(0x90, 61, 100))
        .expect("second queue custody");
    model.service(ModelSource::First, true, false);
    model.service(ModelSource::First, false, true);
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 61, 0)),
        Some(ReleaseRoute {
            id: raw_only,
            raw: true,
            ingress: false
        })
    );
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 61, 0)),
        Some(ReleaseRoute {
            id: ingress_only,
            raw: false,
            ingress: true
        })
    );
    model.settle_raw_release(raw_only);
    model.settle_ingress_release(ingress_only);
    assert_eq!(model.used.tracker, 0);
    assert_eq!(model.used.ingress, 0);
}

#[test]
fn retry_retains_shared_charge_until_exact_retry_or_retirement() {
    let mut model = Model::new(limits(4), 1);
    let first = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .expect("first queue custody");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(retry)) => retry,
        _ => panic!("full ring must return owned retry"),
    };
    assert_eq!(model.used, limits(2));
    assert!(matches!(
        model.submit(ModelSource::First, input(0x90, 64, 100)),
        Err(SubmitError::Blocked)
    ));
    let retry = model
        .retry(retry)
        .expect_err("full ring retains original token");
    assert_eq!(model.service(ModelSource::First, true, true), first);
    let stale = RetryToken {
        id: first,
        source: ModelSource::First,
    };
    assert!(model.retry(stale).is_err());
    let wrong_source = RetryToken {
        id: retry.id,
        source: ModelSource::Second,
    };
    assert!(model.retry(wrong_source).is_err());
    assert_eq!(
        model.retry_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.attempt.id),
        Some(retry.id)
    );
    let retried = model.retry(retry).expect("space available");
    assert_eq!(
        model.rings[ModelSource::First.index()]
            .front()
            .map(|attempt| attempt.input),
        Some(input(0x90, 63, 100))
    );
    assert_eq!(model.service(ModelSource::First, false, false), retried);
    assert_eq!(
        model.used,
        Counts {
            tracker: 1,
            ingress: 1,
            results: 2,
            ledger: 2,
        }
    );

    let queued = model
        .submit(ModelSource::First, input(0x90, 65, 100))
        .expect("next queue custody");
    let retired = match model.submit(ModelSource::First, input(0x90, 66, 100)) {
        Err(SubmitError::Retry(retry)) => {
            let stale = RetryToken {
                id: queued,
                source: ModelSource::First,
            };
            assert!(model.retire(stale).is_err());
            model.retire(retry).expect("exact retirement")
        }
        _ => panic!("full ring must return owned retry"),
    };
    assert_eq!(retired.id, OccurrenceId(queued.0 + 1));
    assert_eq!(retired.key.source, ModelSource::First);
    assert_eq!(retired.key.note, KeyIdentity::new(66).expect("test key"));
    assert_eq!(retired.input, input(0x90, 66, 100));
    assert_eq!(retired.returned, limits(1));
    assert_eq!(
        model.used,
        Counts {
            tracker: 2,
            ingress: 2,
            results: 3,
            ledger: 3,
        }
    );
    assert_eq!(model.service(ModelSource::First, false, false), queued);
    assert!(
        model
            .submit(ModelSource::First, input(0x90, 67, 100))
            .is_ok()
    );
}

#[test]
fn cross_source_credit_waits_for_consumer_settlement_after_result_collection() {
    let mut model = Model::new(limits(1), 1);
    let held = model
        .submit(ModelSource::First, input(0x90, 67, 100))
        .expect("first queue custody");
    model.service(ModelSource::First, true, true);
    model.collect_result(held);
    assert_eq!(
        model.used,
        Counts {
            tracker: 1,
            ingress: 1,
            results: 0,
            ledger: 1,
        }
    );
    assert!(matches!(
        model.submit(ModelSource::Second, input(0x91, 67, 100)),
        Err(SubmitError::NoCredit)
    ));
    let route = model
        .release(ModelSource::First, input(0x80, 67, 0))
        .expect("held release");
    assert_eq!(route.id, held);
    assert!(matches!(
        model.submit(ModelSource::Second, input(0x91, 67, 100)),
        Err(SubmitError::NoCredit)
    ));
    model.settle_raw_release(held);
    assert!(matches!(
        model.submit(ModelSource::Second, input(0x91, 67, 100)),
        Err(SubmitError::NoCredit)
    ));
    model.settle_ingress_release(held);
    assert!(
        model
            .submit(ModelSource::Second, input(0x91, 67, 100))
            .is_ok()
    );
}

#[test]
fn refused_onset_keeps_ledger_cell_after_result_collection() {
    let mut model = Model::new(limits(1), 1);
    let refused = model
        .submit(ModelSource::First, input(0x90, 68, 100))
        .expect("queue custody");
    model.service(ModelSource::First, false, false);
    model.collect_result(refused);
    assert_eq!(
        model.used,
        Counts {
            tracker: 0,
            ingress: 0,
            results: 0,
            ledger: 1,
        }
    );
    assert!(matches!(
        model.submit(ModelSource::Second, input(0x91, 69, 100)),
        Err(SubmitError::NoCredit)
    ));
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 68, 0)),
        Some(ReleaseRoute {
            id: refused,
            raw: false,
            ingress: false,
        })
    );
    assert_eq!(model.used, limits(0));
    assert!(
        model
            .submit(ModelSource::Second, input(0x91, 69, 100))
            .is_ok()
    );
}
