use super::*;

#[test]
fn linear_mapping_preserves_missing_positions_and_checked_exhaustion() {
    let last = QuantumOffset::new(63).unwrap();
    assert_eq!(QuantumTimeline::linear(None).position_at(last), None);
    assert_eq!(
        QuantumTimeline::linear(Some(PlanPosition::new(10))).position_at(last),
        Some(PlanPosition::new(73))
    );
    let end = QuantumTimeline::linear(Some(PlanPosition::new(u64::MAX)));
    assert_eq!(
        end.position_at(QuantumOffset::ZERO),
        Some(PlanPosition::new(u64::MAX))
    );
    assert_eq!(end.position_at(last), None);
}

#[test]
fn mapped_quantum_can_repeat_the_same_position_at_every_sample() {
    let positions = [PlanPosition::new(17); QUANTUM_FRAMES as usize];
    let timeline = QuantumTimeline::mapped(&positions);
    for offset in 0..64 {
        assert_eq!(
            timeline.position_at(QuantumOffset::new(offset).unwrap()),
            Some(PlanPosition::new(17))
        );
    }
}

#[test]
fn held_quantum_reports_its_position_at_every_offset_without_playing_it() {
    let held = QuantumTimeline::held(Some(PlanPosition::new(17)));
    for offset in 0..64 {
        assert_eq!(
            held.position_at(QuantumOffset::new(offset).unwrap()),
            Some(PlanPosition::new(17))
        );
    }
    assert!(held.is_held());
    assert!(held.mapped_positions().is_none());
    let positions = [PlanPosition::new(17); QUANTUM_FRAMES as usize];
    assert!(!QuantumTimeline::mapped(&positions).is_held());
    assert!(!QuantumTimeline::linear(Some(PlanPosition::new(17))).is_held());
    assert_eq!(
        QuantumTimeline::held(None).position_at(QuantumOffset::ZERO),
        None
    );
}
