//! What a prepared tuning guarantees.
//!
//! `SOUND-INV-021` requires it to be total over the key range, deterministic, digested, and
//! refusing of any key that does not resolve usably. These check each of those on its own.

use crate::quantities::KeyIdentity;
use crate::tuning::{PreparedTuning, TuningError};
use synth_core::tuning::TuningTable;

#[test]
fn every_key_in_range_resolves() {
    let tuning = PreparedTuning::equal_temperament().expect("equal temperament prepares");
    for raw in 0..=127_u8 {
        let key = KeyIdentity::new(raw).expect("a keyboard position");
        let hz = tuning.frequency_of(key).as_f32();
        assert!(
            hz.is_finite() && hz > 0.0,
            "{key} resolved to {hz}, and a prepared tuning is total over the range"
        );
    }
}

#[test]
fn a_key_outside_the_keyboard_cannot_be_built() {
    assert!(
        KeyIdentity::new(128).is_err(),
        "128 is not a keyboard position, and refusing it is what stops it becoming 127"
    );
    assert!(KeyIdentity::new(127).is_ok());
}

/// The property that motivates refusing rather than clamping.
#[test]
fn an_out_of_range_key_is_refused_rather_than_becoming_a_different_note() {
    // `synth_core`'s own type clamps, which is what this boundary must not do: a project
    // asking for key 200 would silently play key 127 and nothing would say so.
    assert_eq!(synth_core::MidiNote::new(200).as_u8(), 127);
    assert!(KeyIdentity::new(200).is_err());
}

#[test]
fn equal_temperament_puts_a4_at_440_hz() {
    let tuning = PreparedTuning::equal_temperament().expect("prepares");
    let a4 = KeyIdentity::new(69).expect("A4 is a keyboard position");
    let hz = tuning.frequency_of(a4).as_f32();
    assert!(
        (hz - 440.0).abs() < 0.01,
        "A4 is the reference pitch equal temperament is defined against; got {hz}"
    );
}

/// Two preparations of one definition are the same table, which is what the digest asserts.
#[test]
fn preparation_is_deterministic() {
    let first = PreparedTuning::equal_temperament().expect("prepares");
    let second = PreparedTuning::equal_temperament().expect("prepares");
    assert_eq!(first.digest(), second.digest());
    assert_eq!(first, second);
}

/// And two different scales are two different digests, or the digest says nothing.
#[test]
fn two_scales_do_not_share_a_digest() {
    let equal = PreparedTuning::equal_temperament().expect("prepares");
    let just = PreparedTuning::prepare(&TuningTable::just_intonation()).expect("prepares");
    assert_ne!(
        equal.digest(),
        just.digest(),
        "just intonation and equal temperament are audibly different and must not report as one"
    );
}

/// A scale with more than twelve notes to the octave prepares like any other.
///
/// This is the case the whole decision exists for: under option A the producer would resolve
/// it and V2 would never see a key at all.
#[test]
fn a_non_twelve_tone_scale_prepares() {
    let nineteen =
        PreparedTuning::prepare(&TuningTable::equal_temperament_19()).expect("19-TET prepares");
    let equal = PreparedTuning::equal_temperament().expect("prepares");
    assert_ne!(nineteen.digest(), equal.digest());

    // Its steps are smaller, so a semitone up from A4 is a smaller interval than in 12-TET.
    let a4 = KeyIdentity::new(69).expect("a key");
    let next = KeyIdentity::new(70).expect("a key");
    let step_19 = nineteen.frequency_of(next).as_f32() / nineteen.frequency_of(a4).as_f32();
    let step_12 = equal.frequency_of(next).as_f32() / equal.frequency_of(a4).as_f32();
    assert!(
        step_19 < step_12,
        "19 steps to the octave is a smaller step than 12: {step_19} against {step_12}"
    );
}

/// A key that does not resolve usably is refused where a diagnostic is still possible.
#[test]
fn a_table_with_an_unusable_key_is_refused_and_names_it() {
    // A Scala definition of one note whose ratio is zero: every key maps onto it, so every key
    // resolves to zero hertz.
    let broken = TuningTable::from_scala("silent\n1\n0/1\n", None);
    let Ok(table) = broken else {
        // The parser may refuse it first, which is also a refusal where a diagnostic exists.
        return;
    };
    match PreparedTuning::prepare(&table) {
        Err(TuningError::KeyNotUsable { frequency, .. }) => {
            assert!(
                !frequency.is_finite() || frequency <= 0.0,
                "the refusal must name a frequency that is actually unusable; got {frequency}"
            );
        }
        Ok(_) => panic!("a table resolving a key to zero hertz must not prepare"),
    }
}

#[test]
fn velocity_refuses_what_it_cannot_represent() {
    use crate::quantities::NoteVelocity;

    assert!(
        NoteVelocity::new(0.0).is_ok(),
        "a silent note is representable"
    );
    assert!(NoteVelocity::new(1.0).is_ok());
    assert!(NoteVelocity::new(0.5).is_ok());
    assert!(NoteVelocity::new(-0.1).is_err());
    assert!(NoteVelocity::new(1.1).is_err());
    assert!(NoteVelocity::new(f32::NAN).is_err());
    assert!(NoteVelocity::new(f32::INFINITY).is_err());

    // `synth_core`'s type clamps where this refuses, which is the whole reason for a second
    // type rather than a reuse.
    assert!((synth_core::Velocity::new(1.5).as_f32() - 1.0).abs() < f32::EPSILON);
}

/// The limitation this boundary has, asserted rather than left to be discovered.
///
/// `synth_core::TuningTable` is `[Hertz; 128]` and carries no record of which keys the
/// authored definition actually mapped. `from_scala` synthesises an entry for every key by
/// extrapolating from the reference, so a KBM that maps one key still arrives here as 128
/// finite frequencies. Preparation therefore validates **values** and cannot validate
/// **definedness** — an independent review found the module claiming otherwise.
///
/// Measured: a one-note scale with a KBM mapping only key 60 prepares, and key 0 resolves to
/// roughly `7.5e-19` Hz — positive, finite, and not a note. Completing a partial definition is
/// the authored model's job, and Phase 10A's; this test exists so the gap is visible here
/// rather than surprising someone later.
#[test]
fn preparation_validates_values_and_not_definedness() {
    let kbm = "1\n0\n127\n60\n69\n440.0\n60\n0\n";
    let Ok(table) = TuningTable::from_scala("probe\n1\n2/1\n", Some(kbm)) else {
        // If the parser refuses it, the gap does not exist on this path and there is nothing
        // for this test to record.
        return;
    };

    let unmapped = KeyIdentity::new(0).expect("a key");
    let raw = table.note_to_freq(synth_core::MidiNote::new(0)).as_f32();
    assert!(
        raw.is_finite() && raw > 0.0,
        "the gap is that an unmapped key arrives here looking usable; it was {raw}"
    );
    assert!(
        PreparedTuning::prepare(&table).is_ok(),
        "so preparation accepts it, which is the limitation this test records"
    );
    let prepared = PreparedTuning::prepare(&table).expect("prepares");
    assert!(
        prepared.frequency_of(unmapped).as_f32() < 1.0,
        "and the key resolves to a frequency no note has, without anything refusing it"
    );
}
