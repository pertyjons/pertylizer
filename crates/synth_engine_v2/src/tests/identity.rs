//! Note identity's checks.

use super::*;
use crate::quantities::HeldNoteCount;

/// A producer's range, in **held notes** — the obligations an index addresses, not event
/// throughput. An earlier revision used `EventCount`, which named the wrong domain.
fn held(n: u32) -> HeldNoteCount {
    HeldNoteCount::limit(n).expect("a positive capacity")
}

const A: ProducerId = ProducerId::new(0);

/// A node to play. Which node does not matter here — what matters is that the table
/// remembers it, so a release can carry the identity alone.
fn node() -> crate::plan::NoteSlot {
    crate::plan::NoteSlot::new(crate::plan::PlanId::FILL, 0)
}
const B: ProducerId = ProducerId::new(1);

fn table() -> IdentityTable {
    IdentityTable::new(held(512), &[held(4), held(4)]).expect("a valid table")
}

#[test]
fn a_minted_identity_names_a_live_note_and_a_released_one_does_not() {
    // The whole point in one case. ADR-0046 clause 3 promises an orphan edge "is counted
    // rather than allowed to release another note", and with `{ node, edge }` that sentence
    // had no implementable meaning — a release whose note-on was dropped was
    // indistinguishable from a legitimate one.
    let mut table = table();
    let note = table.mint(A, node()).expect("the range has room");
    assert_eq!(table.resolve(note), Resolution::Live);
    assert_eq!(table.live(), 1);

    assert_eq!(
        table.release(note),
        Resolution::Live,
        "the first release works"
    );
    assert_eq!(table.live(), 0);
    assert_eq!(
        table.release(note),
        Resolution::Orphan(OrphanCause::FreeIndex),
        "the second is an orphan, not a second release"
    );
}

#[test]
fn a_reused_index_supersedes_the_identity_that_held_it() {
    // The second orphan branch, and the one a free-index check alone would miss: the index
    // *is* live, just not with this note. Without the generation, a stale release would look
    // exactly like a valid one and would end somebody else's note.
    let mut table = table();
    let first = table.mint(A, node()).expect("room");
    table.release(first);
    let second = table.mint(A, node()).expect("room");

    assert_eq!(table.resolve(second), Resolution::Live);
    assert_eq!(
        table.resolve(first),
        Resolution::Orphan(OrphanCause::SupersededGeneration),
        "the old identity names a live index at a generation that has moved on"
    );
    assert_eq!(
        table.release(first),
        Resolution::Orphan(OrphanCause::SupersededGeneration),
        "and releasing it must not end the note now holding that index"
    );
    assert_eq!(
        table.resolve(second),
        Resolution::Live,
        "which is still live"
    );
}

#[test]
fn an_index_whose_generations_run_out_is_retired_and_counted() {
    // The third orphan branch, and the reason the rule is stated as a definition rather than
    // a list of two: retirement creates a state that is neither free nor live, and an
    // earlier draft of the specification had no classification for it at all.
    //
    // The generation ceiling is a construction parameter because of this test. Walking a
    // `u32` to its ceiling by minting would take longer than this project will exist, and a
    // rule no test can reach is a rule nobody has checked — so the ceiling is what ADR-0047
    // says it is, a measured width, and here it is two.
    let mut table =
        IdentityTable::with_generation_ceiling(held(8), &[held(1)], 2).expect("a valid table");

    for expected in 0..2 {
        let note = table.mint(A, node()).expect("the index is free");
        assert_eq!(table.retired(), 0, "no retirement at generation {expected}");
        assert_eq!(table.release(note), Resolution::Live);
    }

    // The third mint takes the ceiling generation, and releasing it retires the index.
    let last = table.mint(A, node()).expect("the index is free once more");
    assert_eq!(table.release(last), Resolution::Live);
    assert_eq!(table.retired(), 1, "the retirement is counted");
    assert_eq!(
        table.resolve(last),
        Resolution::Orphan(OrphanCause::RetiredIndex),
        "the index is neither free nor live, which is why the third branch exists"
    );
    assert!(
        matches!(
            table.mint(A, node()),
            Err(IdentityError::ProducerRangeEroded { producer, retired, .. })
                if producer == A && retired == 1
        ),
        "a producer whose only index retired has nothing left to mint — and that is erosion, \
         a named condition, rather than the over-emission a producer would have to fix"
    );
}

#[test]
fn producer_ranges_are_disjoint() {
    // Disjointness is what makes an identity attributable without a producer tag, and what
    // stops one producer exhausting another. Filling A entirely must leave B untouched.
    let mut table = table();
    let mut mine = Vec::new();
    for _ in 0..4 {
        mine.push(table.mint(A, node()).expect("A has four indices"));
    }
    assert!(
        matches!(
            table.mint(A, node()),
            Err(IdentityError::ProducerOverEmitted { producer, .. }) if producer == A
        ),
        "A is full with nothing retired, so this is over-emission and the refusal names A"
    );

    // **B must be able to mint its whole range**, not merely one identity. An earlier
    // version asked for one and passed against a *partially* overlapping layout — A `[0,4)`
    // and B `[1,5)` still leaves B one free index — so it established nothing about the
    // overlap it was named for.
    let mut theirs = Vec::new();
    for index in 0..4 {
        theirs.push(
            table
                .mint(B, node())
                .unwrap_or_else(|error| panic!("B's index {index} must be its own: {error:?}")),
        );
    }
    for note in mine.into_iter().chain(theirs) {
        assert_eq!(
            table.resolve(note),
            Resolution::Live,
            "eight distinct identities, so no index was handed out twice"
        );
    }
    assert_eq!(table.live(), 8);
}

#[test]
fn an_eroded_range_is_not_reported_as_over_emission() {
    // `SOUND-INV-017` distinguishes the two, and that is the whole point of the second
    // error: a producer whose indices were consumed by retirement declared correctly and did
    // not over-emit. Reporting it as over-emission would send someone to fix a producer that
    // is behaving.
    let mut table =
        IdentityTable::with_generation_ceiling(held(8), &[held(2)], 0).expect("a valid table");

    // A ceiling of zero retires each index on its first release.
    let first = table.mint(A, node()).expect("room");
    table.release(first);
    assert_eq!(table.retired(), 1);

    // One index left, and holding it makes the range both eroded and full.
    let _second = table.mint(A, node()).expect("one index remains");
    match table.mint(A, node()) {
        Err(IdentityError::ProducerRangeEroded {
            producer,
            admitted,
            retired,
        }) => {
            assert_eq!(producer, A);
            assert_eq!(admitted, HeldNoteCount::measured(2));
            assert_eq!(retired, 1, "the refusal says how much of the range is gone");
        }
        other => panic!("expected erosion, not over-emission, got {other:?}"),
    }

    // With nothing retired, the same shape of failure is over-emission instead.
    let mut fresh = IdentityTable::new(held(8), &[held(1)]).expect("a valid table");
    let _only = fresh.mint(A, node()).expect("room");
    assert!(matches!(
        fresh.mint(A, node()),
        Err(IdentityError::ProducerOverEmitted { .. })
    ));
}

#[test]
fn a_rebuild_is_refused_while_an_obligation_is_outstanding() {
    // `SOUND-INV-017`, and ADR-0046 clause 3 is why it is a refusal rather than a reset:
    // rejecting the eventual release would refuse an accepted obligation, and stranding it
    // would leave a note nothing can release. Refusing the rebuild breaks neither.
    let mut table = table();
    let note = table.mint(A, node()).expect("room");
    let id = table.id();

    match table.rebuild(held(512), &[held(4), held(4)]) {
        Err(IdentityError::RebuildWithObligationsOutstanding {
            table: named,
            outstanding,
        }) => {
            assert_eq!(named, id);
            assert_eq!(outstanding, 1);
        }
        other => panic!("expected a rebuild refusal, got {other:?}"),
    }

    // **The refusal must not have destroyed what it was protecting.** A consuming signature
    // would have dropped the table here, stranding the very obligation it refused to
    // abandon — which is what the first version of `rebuild` did.
    assert_eq!(
        table.resolve(note),
        Resolution::Live,
        "the outstanding note is still resolvable after the refusal"
    );
    assert_eq!(
        table.release(note),
        Resolution::Live,
        "and still releasable"
    );
}

#[test]
fn a_rebuild_succeeds_once_the_obligations_are_gone() {
    // The other half: the refusal is about outstanding obligations, not about rebuilding.
    let mut table = table();
    let note = table.mint(A, node()).expect("room");
    table.release(note);
    let id = table.id();

    let rebuilt = table
        .rebuild(held(512), &[held(4), held(4)])
        .expect("nothing is outstanding");
    drop(table);
    assert_ne!(rebuilt.id(), id, "a rebuild is a new table, never a reset");
    assert_eq!(
        rebuilt.resolve(note),
        Resolution::ForeignTable { minted_by: id },
        "and an identity from the old table is foreign to it"
    );
}

#[test]
fn a_scoped_mass_release_leaves_other_producers_alone() {
    // ADR-0046 clause 6 applies the operation "to owned voices within the source event".
    // A sustain lift on one source must not end another source's notes, and an earlier
    // revision ended everything unconditionally while its documentation claimed sustain
    // lift.
    let mut table = table();
    let mine: Vec<_> = (0..3)
        .map(|_| table.mint(A, node()).expect("room"))
        .collect();
    let theirs = table.mint(B, node()).expect("room");

    assert_eq!(
        table.release_all(ReleaseScope::Producer(A)),
        HeldNoteCount::measured(3),
        "only A's obligations end"
    );
    assert_eq!(
        table.resolve(theirs),
        Resolution::Live,
        "B's note is untouched by A's sustain lift"
    );
    for note in mine {
        assert_eq!(
            table.resolve(note),
            Resolution::Orphan(OrphanCause::FreeIndex)
        );
    }
    assert_eq!(table.live(), 1);
}

#[test]
fn an_identity_from_another_table_is_not_an_orphan() {
    // Neither existing value scopes an identity: a re-admission changes the plan without
    // changing the epoch, and a re-preparation changes the epoch without changing the plan.
    // A foreign identity is reported as foreign rather than as an orphan, because it says
    // nothing about whether the note it named is live — only that this table cannot answer.
    let mut first = table();
    let note = first.mint(A, node()).expect("room");
    let second = table();
    assert_ne!(first.id(), second.id(), "table identities are never reused");

    match second.resolve(note) {
        Resolution::ForeignTable { minted_by } => assert_eq!(minted_by, first.id()),
        other => panic!("expected a foreign-table resolution, got {other:?}"),
    }
}

#[test]
fn a_mass_release_ends_every_live_note_and_leaves_orphans_behind() {
    // ADR-0046 clause 6: panic, transport stop and sustain lift are one bounded operation,
    // not one event per voice. A release arriving afterwards for a note the operation took
    // is refused by the ordinary orphan rule rather than double-freeing anything.
    let mut table = table();
    let notes: Vec<_> = (0..3)
        .map(|_| table.mint(A, node()).expect("room"))
        .collect();
    let other = table.mint(B, node()).expect("room");
    assert_eq!(table.live(), 4);

    assert_eq!(
        table.release_all(ReleaseScope::Everything),
        HeldNoteCount::measured(4),
        "a panic or transport stop reaches every producer"
    );
    assert_eq!(table.live(), 0);
    for note in notes {
        assert_eq!(
            table.release(note),
            Resolution::Orphan(OrphanCause::FreeIndex),
            "a later release for a note the mass operation took is an orphan"
        );
    }
    assert_eq!(
        table.release(other),
        Resolution::Orphan(OrphanCause::FreeIndex)
    );
}

#[test]
fn a_profile_holding_more_notes_than_an_identity_can_name_is_refused() {
    // The relation is normative as a **profile** refusal, not merely as a table one: a
    // profile that cannot name its own held notes should not exist, rather than fail later
    // when someone builds a table from it. An independent review found the evidence testing
    // only the table while the claim was about the profile.
    use crate::profile::{ProfileError, VoiceLimits};
    use crate::quantities::VoiceCount;
    use crate::time::FrameCount;

    let voices = |held_notes: u32| {
        VoiceLimits::new(
            VoiceCount::limit(1).expect("positive"),
            VoiceCount::limit(128).expect("positive"),
            VoiceCount::limit(512).expect("positive"),
            HeldNoteCount::limit(held_notes).expect("positive"),
            FrameCount::new(128),
        )
    };

    assert!(voices(1 << 16).is_ok(), "the boundary itself is admissible");
    match voices((1 << 16) + 1) {
        Err(ProfileError::HeldNotesExceedIdentitySpace { held, available }) => {
            assert_eq!(held, HeldNoteCount::measured((1 << 16) + 1));
            assert_eq!(available, 1 << 16);
        }
        other => panic!("expected a profile refusal, got {other:?}"),
    }
}

#[test]
fn an_index_space_below_max_held_notes_is_refused_at_construction() {
    // `SOUND-INV-017`'s construction relation. `max_held_notes` is otherwise constrained
    // only to be nonzero, so nothing else would catch a profile that can hold more notes
    // than it can name — and a profile that could would collapse two notes onto one index.
    let too_many = HeldNoteCount::limit((1 << 16) + 1).expect("positive");
    match IdentityTable::new(too_many, &[held(1)]) {
        Err(IdentityError::IndexSpaceBelowHeldNotes { available, needed }) => {
            assert_eq!(available, 1 << 16);
            assert_eq!(needed, too_many);
        }
        other => panic!("expected the relation to refuse, got {other:?}"),
    }
    assert!(
        IdentityTable::new(held(1 << 16), &[held(1)]).is_ok(),
        "the boundary itself is admissible"
    );
}

#[test]
fn ranges_that_do_not_fit_the_index_space_are_refused() {
    // Two producers that each fit alone but not together — the same shape as ADR-0046's
    // plan-wide aggregate rule, and for the same reason: checking one at a time is not
    // checking the partition.
    let half = held(1 << 15);
    assert!(IdentityTable::new(held(8), &[half]).is_ok());
    assert!(
        IdentityTable::new(held(8), &[half, half]).is_ok(),
        "exactly full"
    );
    match IdentityTable::new(held(8), &[half, half, held(1)]) {
        Err(IdentityError::RangesExceedIndexSpace { needed, available }) => {
            assert_eq!(needed, u64::from(1_u32 << 16) + 1);
            assert_eq!(available, 1 << 16);
        }
        other => panic!("expected an aggregate refusal, got {other:?}"),
    }
}

#[test]
fn an_unadmitted_producer_cannot_mint() {
    let mut table = table();
    let stranger = ProducerId::new(7);
    match table.mint(stranger, node()) {
        Err(IdentityError::UnknownProducer { producer }) => assert_eq!(producer, stranger),
        other => panic!("expected an unknown-producer refusal, got {other:?}"),
    }
}

#[test]
fn the_occurrence_remembers_the_node_so_a_release_need_not_carry_one() {
    // `SOUND-INV-017` removes the node address from a release rather than carrying it and
    // requiring agreement — carrying both admits an event whose identity and node disagree,
    // and no reading of that is safe. Something has to remember the node, and the occurrence
    // is the only thing that can: it was named when the occurrence was created.
    let mut table = table();
    let played = crate::plan::NoteSlot::new(crate::plan::PlanId::FILL, 3);
    let note = table.mint(A, played).expect("room");

    assert_eq!(table.note_of(note), Some(played));

    // And an identity that names no live note has no node either — the same refusal
    // `resolve` gives, rather than a guess.
    table.release(note);
    assert_eq!(table.note_of(note), None);
    assert_eq!(
        table.resolve(note),
        Resolution::Orphan(OrphanCause::FreeIndex)
    );

    let other = IdentityTable::new(held(512), &[held(4), held(4)]).expect("a valid table");
    assert_eq!(
        other.note_of(note),
        None,
        "and neither does a foreign table pretend to know"
    );
}

#[test]
fn the_registry_scopes_a_mass_release_to_one_producer_and_names_what_it_ended() {
    // The audio-thread half of ADR-0046 clause 6's bounded mass release, which ADR-0050
    // clause 5 puts at an activation boundary. It clears what is **sounding**; the table
    // clears what a retired schedule still **reserves**, and clause 3 is explicit that the
    // two sets overlap without containing each other.
    let minter = table();
    let mut registry =
        LiveNotes::for_ranges(minter.id(), &[held(4), held(4)]).expect("a valid registry");
    let mut table = table();

    // Admitted with the *minter's* table id, because the registry accepts only its own —
    // and building a second table here is what would silently make every event foreign.
    let mut mine = Vec::new();
    for index in 0..3 {
        let identity = table.mint(A, node()).expect("room");
        // The registry answers to `minter`, so the occurrence has to carry that id.
        let identity = rekey(identity, minter.id());
        registry.admit(
            identity,
            slot(index),
            crate::quantities::KeyIdentity::LOWEST,
        );
        mine.push(identity);
    }
    let theirs = rekey(table.mint(B, node()).expect("room"), minter.id());
    registry.admit(theirs, slot(9), crate::quantities::KeyIdentity::LOWEST);

    let mut ended = [None; 8];
    assert_eq!(
        registry.release_all(ReleaseScope::Producer(A), &mut ended),
        HeldNoteCount::measured(3),
        "only A's sounding notes end"
    );
    // Each ended note names its node **and** the partition index it held, which is the
    // voice instance its gate-down belongs to (`P06-S001`); A's range starts at zero.
    let ended_note = |index: u16| {
        Some(crate::identity::EndedNote {
            note: slot(usize::from(index)),
            index,
        })
    };
    assert_eq!(
        &ended[..3],
        &[ended_note(0), ended_note(1), ended_note(2)],
        "and it names each one's node, so the caller can lower those gates without a second walk"
    );
    assert_eq!(
        registry.note_of(theirs),
        Some(slot(9)),
        "B's note is untouched: a seek moves plan time, it does not lift a performer's finger"
    );
    for identity in mine {
        assert_eq!(
            registry.note_of(identity),
            None,
            "an occurrence the mass release took resolves through nothing afterwards"
        );
    }
}

#[test]
fn the_registrys_mass_release_ends_nothing_it_cannot_name() {
    // All or nothing, and the alternative is the reason. Clearing what a short buffer cannot
    // name would leave those notes with no registry entry and no reported node: nothing could
    // release them and nothing could lower their gates, so they would sound forever. Refusing
    // leaves every one of them exactly where it was.
    //
    // The bound is the scope's **span**, not its live count, because that is what a caller can
    // size against an admitted declaration — so this branch is unreachable for a conforming
    // caller and exists to make the failure safe rather than to be relied on.
    let minter = table();
    let mut registry =
        LiveNotes::for_ranges(minter.id(), &[held(4), held(4)]).expect("a valid registry");
    let mut table = table();
    let mut mine = Vec::new();
    for index in 0..3 {
        let identity = rekey(table.mint(A, node()).expect("room"), minter.id());
        registry.admit(
            identity,
            slot(index),
            crate::quantities::KeyIdentity::LOWEST,
        );
        mine.push(identity);
    }

    // Three notes are sounding, but A's span is four, so three slots is short.
    let mut ended = [None; 3];
    assert_eq!(
        registry.release_all(ReleaseScope::Producer(A), &mut ended),
        HeldNoteCount::NONE,
        "a buffer too short for the span ends nothing"
    );
    assert_eq!(ended, [None; 3], "and names nothing");
    for identity in &mine {
        assert_eq!(
            registry.note_of(*identity),
            Some(slot(
                mine.iter()
                    .position(|other| other == identity)
                    .expect("present")
            )),
            "every note is still sounding and still reachable"
        );
    }

    // The same call with a buffer sized to the span succeeds, which is what makes the
    // refusal a refusal rather than a permanent failure.
    let mut ended = [None; 4];
    assert_eq!(
        registry.release_all(ReleaseScope::Producer(A), &mut ended),
        HeldNoteCount::measured(3)
    );
}

#[test]
fn the_registry_ignores_a_producer_it_has_no_range_for() {
    // A scope naming a producer outside the partition ends nothing rather than clearing the
    // whole registry. The audio thread cannot report a bad scope, so the safe answer is the
    // empty one.
    let minter = table();
    let mut registry = LiveNotes::for_ranges(minter.id(), &[held(4)]).expect("a valid registry");
    let mut table = IdentityTable::new(held(512), &[held(4)]).expect("a valid table");
    let identity = rekey(table.mint(A, node()).expect("room"), minter.id());
    registry.admit(identity, slot(0), crate::quantities::KeyIdentity::LOWEST);

    let mut ended = [None; 4];
    assert_eq!(
        registry.release_all(ReleaseScope::Producer(ProducerId::new(7)), &mut ended),
        HeldNoteCount::NONE
    );
    assert_eq!(registry.note_of(identity), Some(slot(0)));
}

/// A note slot that is distinguishable from its neighbours.
///
/// The registry stores the node a note plays, and a mass release reports it; fixtures that
/// used one slot everywhere could not tell a correct report from a constant.
fn slot(index: usize) -> crate::plan::NoteSlot {
    crate::plan::NoteSlot::new(crate::plan::PlanId::FILL, index)
}

/// The same occurrence, attributed to another table.
///
/// Only the tests need this: they mint from one table and admit into a registry built for
/// another, which production never does — `StreamControl::open` pairs the two halves. It
/// exists so a fixture can exercise the registry without also exercising the minter's
/// allocation policy.
fn rekey(identity: NoteIdentity, table: TableId) -> NoteIdentity {
    NoteIdentity {
        table,
        index: identity.index,
        generation: identity.generation,
    }
}

#[test]
fn the_admission_budget_covers_what_both_identity_halves_actually_allocate() {
    // `render::identity_bytes` is what admission charges a plan for its identity partition,
    // and preparation is what spends it. The two are written apart, so this compares the
    // prediction against what the objects report holding rather than restating the formula.
    //
    // The producer **count** is varied independently of the index count, because that is the
    // term an earlier revision got wrong: both halves keep their own copy of the per-producer
    // spans, and the budget charged one table. A fixture that only varied polyphony moves the
    // per-index term and cannot see it.
    for partition in [
        vec![held(4)],
        vec![held(2), held(2)],
        vec![held(1), held(1), held(1), held(1)],
        vec![held(64), held(1)],
    ] {
        let minter = IdentityTable::from_admitted_ranges(&partition).expect("a valid table");
        let registry = LiveNotes::for_ranges(minter.id(), &partition).expect("a valid registry");
        let held_by_both = minter.storage_bytes() + registry.storage_bytes();

        assert!(
            crate::render::identity_bytes(&partition) >= held_by_both as u64,
            "a partition of {} producers over {} indices is charged {} bytes but holds {}",
            partition.len(),
            partition.iter().map(|range| range.get()).sum::<u32>(),
            crate::render::identity_bytes(&partition),
            held_by_both
        );
    }
}

#[test]
fn a_release_scoped_to_a_producer_refuses_another_producers_occurrence() {
    // `SOUND-INV-017` gives every producer a disjoint index range, and ADR-0046 clause 6
    // partitions holds so that "no producer borrows another's unused holds".
    // `IdentityTable::release` enforces the first through the occurrence but not the
    // second through the producer: one source presenting another's live occurrence ends
    // a note it never opened and spends its own hold on it, leaving its own note
    // occupied — so its next note-on finds the range full with a hold still free, which
    // is the identity shortage otherwise unreachable. An independent review found it in
    // the live boundary.
    //
    // **Checked over a bare table rather than through a plan**, and that is not a
    // convenience: the end-to-end fixture would need one plan declaring two note
    // producers, which is ADR-0051 clause 6's shared-gate construction, and
    // `PROCESS.md` forbids building on an undecided coupled policy. A table's producer
    // ranges reach no gate.
    let partition = [HeldNoteCount::measured(2), HeldNoteCount::measured(2)];
    let mut table = IdentityTable::from_admitted_ranges(&partition).expect("a valid table");
    let first = ProducerId::new(0);
    let second = ProducerId::new(1);
    let note = crate::plan::NoteSlot::new(crate::plan::PlanId::FILL, 0);

    let theirs = table.mint(first, note).expect("the first producer mints");
    let mine = table.mint(second, note).expect("and so does the second");

    // The second producer may not end the first's occurrence, and the refusal is an
    // orphan **to it** — an identity naming no note it holds open.
    assert!(matches!(
        table.release_for(second, theirs),
        Resolution::Orphan(_)
    ));
    // And the refusal took nothing: the first producer's own release still resolves.
    assert_eq!(table.release_for(first, theirs), Resolution::Live);
    assert_eq!(table.release_for(second, mine), Resolution::Live);
}
