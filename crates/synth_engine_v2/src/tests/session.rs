//! The session scheduler's checks.

use super::*;
use crate::quantities::SampleRate;
use crate::tempo::{Bpm, TICKS_PER_QUARTER, TempoChange};

const Q: u64 = TICKS_PER_QUARTER as u64;

fn rate() -> SampleRate {
    SampleRate::new(48_000.0).expect("a valid rate")
}

fn map(bpm_value: f64) -> TempoMap {
    TempoMap::new(Bpm::new(bpm_value).expect("a valid tempo"), &[], rate()).expect("a valid map")
}

#[test]
fn play_makes_the_anchored_tick_land_on_the_anchored_frame() {
    // The defining property of an anchor: the position it was built from maps to the time
    // it was built with. Everything else this module does is measured from that.
    let session =
        SessionScheduler::play(map(120.0), MusicalTick::new(8 * Q), SampleTime::new(1_000))
            .expect("a valid start");
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(8 * Q))
            .expect("the anchored tick has a time"),
        SampleTime::new(1_000)
    );
}

#[test]
fn later_musical_time_advances_engine_time_by_the_same_frames() {
    // The composition, in the only direction it runs. Four beats at 120 BPM is two seconds,
    // so the engine time advances by 96 000 frames from wherever the anchor put it.
    let session = SessionScheduler::play(map(120.0), MusicalTick::ZERO, SampleTime::new(500))
        .expect("a valid start");
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(4 * Q))
            .expect("inside the stream"),
        SampleTime::new(500 + 96_000)
    );
}

#[test]
fn a_step_and_a_ramp_reach_stable_engine_times_through_the_anchor() {
    // **Phase 3's exit gate asks for tempo steps and ramps at stable *sample* positions**,
    // and until this test nothing carried either across the anchor. `tempo.rs` proves the
    // conversion law thoroughly — and only to [`PlanPosition`], which is the module's whole
    // point: ADR-0032 clause 27 keeps engine time out of it. The session tests that do reach
    // engine time all used a **constant** map, so neither a step nor a ramp had ever been
    // anchored. The gate's two halves met nowhere.
    //
    // The figures are the conversion law's own, computed here rather than read back from it.
    // The map is 120 BPM, ramping to 240 BPM over the two beats from tick `2Q`, and holding
    // 240 from `4Q`.
    //
    // - Beat 2 is two beats of 120 BPM: `2 * 0.5 s` = 48 000 frames.
    // - Beat 4 ends the ramp, which lasts its beats times the **mean of its two periods**
    //   (`SOUND-INV-019`): `2 * (0.5 + 0.25) / 2` = 0.75 s = 36 000 frames.
    // - Beat 6 is two further beats at 240 BPM: `2 * 0.25 s` = 24 000 frames.
    //
    // The ramp's mean-period duration is what makes this more than a plumbing test: a
    // conversion that interpolated the *tempo number* rather than the period would put beat
    // 4 at a different frame, which is the difference ADR-0049 decided and V1 gets the other
    // way.
    let ramped = TempoMap::new(
        Bpm::new(120.0).expect("a valid tempo"),
        &[
            TempoChange::ramp(MusicalTick::new(2 * Q), Bpm::new(120.0).expect("valid")),
            TempoChange::step(MusicalTick::new(4 * Q), Bpm::new(240.0).expect("valid")),
        ],
        rate(),
    )
    .expect("a valid map");

    let anchor = SampleTime::new(7_000);
    let session = SessionScheduler::play(ramped, MusicalTick::ZERO, anchor).expect("a valid start");
    let at = |tick: u64| {
        session
            .engine_time_of(MusicalTick::new(tick))
            .expect("inside the stream")
    };

    assert_eq!(at(0), anchor, "the anchor itself");
    assert_eq!(at(2 * Q), SampleTime::new(7_000 + 48_000), "the step's end");
    assert_eq!(at(4 * Q), SampleTime::new(7_000 + 84_000), "the ramp's end");
    assert_eq!(at(6 * Q), SampleTime::new(7_000 + 108_000), "past the ramp");

    // **Stable** means the same tick answers the same frame however it is asked for, which
    // is what a caller stepping through a timeline depends on. `position_of` sums a stored
    // prefix, so an implementation that accumulated per call would drift here.
    for tick in [2 * Q, 4 * Q, 6 * Q, 2 * Q, 6 * Q, 4 * Q] {
        assert_eq!(
            at(tick),
            at(tick),
            "tick {tick} answers the same frame twice"
        );
    }
    assert_eq!(
        at(4 * Q),
        SampleTime::new(7_000 + 84_000),
        "and after the walk"
    );
}

#[test]
fn a_position_before_the_anchor_is_refused_rather_than_clamped() {
    // ADR-0032 clause 27: a position before the anchor "is a scheduler error rather than a
    // clamp". Clamping would answer with the anchor's own frame, which is a plausible number
    // and the wrong one — every event before the anchor would pile onto a single sample.
    let session =
        SessionScheduler::play(map(120.0), MusicalTick::new(8 * Q), SampleTime::new(1_000))
            .expect("a valid start");
    match session.engine_time_of(MusicalTick::new(4 * Q)) {
        Err(SessionError::PositionPrecedesAnchor {
            tick,
            position,
            anchor,
        }) => {
            assert_eq!(tick, MusicalTick::new(4 * Q));
            assert!(position < anchor, "the refusal names both sides");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn seeking_re_anchors_and_the_old_pairing_does_not_survive() {
    // A seek is not an offset applied to the old anchor. After it, the sought tick lands on
    // the new time, and the tick the old anchor was built from is *behind* the new one — so
    // it has no time at all, which is the observable difference between re-anchoring and
    // adjusting.
    let mut session =
        SessionScheduler::play(map(120.0), MusicalTick::ZERO, SampleTime::ZERO).expect("start");
    assert_eq!(
        session.engine_time_of(MusicalTick::ZERO).expect("anchored"),
        SampleTime::ZERO
    );

    session
        .seek(MusicalTick::new(16 * Q), SampleTime::new(200_000))
        .expect("a valid seek");
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(16 * Q))
            .expect("the sought tick"),
        SampleTime::new(200_000)
    );
    assert!(
        session.engine_time_of(MusicalTick::ZERO).is_err(),
        "the old anchor's tick is behind the new anchor and has no time"
    );
}

#[test]
fn a_loop_wrap_re_anchors_rather_than_mapping_the_second_pass_onto_the_first() {
    // Clause 27's named case, and the failure it prevents. Playing bars 0..8 twice, the
    // second pass runs at engine times *after* the first — but its musical positions repeat.
    // Without a re-anchor the second pass's tick 0 would map to the first pass's frame 0,
    // which is not stale, it is contradictory: two different instants claiming one sample.
    let mut session =
        SessionScheduler::play(map(120.0), MusicalTick::ZERO, SampleTime::ZERO).expect("start");

    // Eight beats at 120 BPM is four seconds.
    let loop_length = 8 * Q;
    let wrap_at = SampleTime::new(192_000);
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(loop_length))
            .expect("the loop end"),
        wrap_at
    );

    session
        .loop_wrap(MusicalTick::ZERO, wrap_at)
        .expect("a valid wrap");

    assert_eq!(
        session
            .engine_time_of(MusicalTick::ZERO)
            .expect("second pass"),
        wrap_at,
        "the second pass starts where the first ended, not where the first started"
    );
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(4 * Q))
            .expect("second pass"),
        SampleTime::new(192_000 + 96_000),
        "and runs forward from there"
    );
}

#[test]
fn an_offline_range_start_anchors_the_same_way_a_play_does() {
    // Named separately because the master plan names it separately: an offline job
    // re-anchors too, and folding it into `play` would hide that.
    let offline = SessionScheduler::offline_range_start(
        map(90.0),
        MusicalTick::new(12 * Q),
        SampleTime::ZERO,
    )
    .expect("a valid range start");
    let played = SessionScheduler::play(map(90.0), MusicalTick::new(12 * Q), SampleTime::ZERO)
        .expect("a valid start");
    assert_eq!(offline.anchor(), played.anchor());
}

#[test]
fn replacing_the_tempo_map_re_anchors_the_transport() {
    // A new map moves every musical position, so continuing to sound at the same instant
    // means re-anchoring. The test's point is that both change together: the tick keeps its
    // engine time, and the *next* tick does not keep its old one.
    let mut session =
        SessionScheduler::play(map(120.0), MusicalTick::ZERO, SampleTime::ZERO).expect("start");
    let before = session
        .engine_time_of(MusicalTick::new(8 * Q))
        .expect("inside");

    session
        .replace_tempo_map(map(60.0), MusicalTick::new(4 * Q), SampleTime::new(96_000))
        .expect("a valid replacement");

    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(4 * Q))
            .expect("the transport's own tick"),
        SampleTime::new(96_000),
        "the transport keeps sounding at the instant it was at"
    );
    let after = session
        .engine_time_of(MusicalTick::new(8 * Q))
        .expect("inside");
    assert_ne!(
        before, after,
        "at half the tempo the next four beats take twice as long"
    );
    assert_eq!(after, SampleTime::new(96_000 + 192_000));
}

#[test]
fn a_refused_replacement_leaves_the_old_map_and_anchor_in_force() {
    // The master plan's "failure leaves the old map and plan active". A partial activation
    // would leave events whose engine times were computed under a tempo no longer in force,
    // which is worse than refusing: nothing would report it.
    let mut session =
        SessionScheduler::play(map(120.0), MusicalTick::ZERO, SampleTime::ZERO).expect("start");
    let anchor_before = session.anchor();
    let mapping_before = session
        .engine_time_of(MusicalTick::new(8 * Q))
        .expect("inside");

    // A tick past exact conversion range: the new map cannot place the transport, so the
    // replacement is refused before anything is swapped.
    let unplaceable = MusicalTick::new((1_u64 << 53) + 1);
    assert!(
        session
            .replace_tempo_map(map(60.0), unplaceable, SampleTime::ZERO)
            .is_err(),
        "a map that cannot place the transport is refused"
    );

    assert_eq!(session.anchor(), anchor_before, "the anchor did not move");
    assert_eq!(
        session
            .engine_time_of(MusicalTick::new(8 * Q))
            .expect("inside"),
        mapping_before,
        "and the old map is still the one answering"
    );
}

#[test]
fn the_only_route_is_musical_to_plan_to_engine() {
    // A structural check rather than a value one. `SessionScheduler` exposes no method that
    // turns musical time into engine time without going through a `PlanPosition`, and the
    // tempo map exposes no `SampleTime` at all — clause 27's "anchoring is the only place
    // the two vocabularies meet", enforced by what exists rather than by what is documented.
    // Code lines only. Scanning the prose too would flag the module header, which states
    // this very property — the same reason the render loop's purity scan strips comments
    // before it looks for the constructs it forbids.
    let code: String = include_str!("../tempo.rs")
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("SampleTime"),
        "the tempo map must not name engine time in its code"
    );

    let session = include_str!("../session.rs");
    let body = session
        .split("pub fn engine_time_of")
        .nth(1)
        .expect("the composition is named");
    let body = body.split("\n    }").next().expect("it has a body");
    assert!(
        body.contains("position_of") && body.contains("time_of"),
        "the composition must run through both steps: {body}"
    );
}
