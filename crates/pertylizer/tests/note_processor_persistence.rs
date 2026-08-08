//! X-cut persistence round-trip for note processors (NP plan, X-cut).
//!
//! Proves that a pattern's processor rack AND a per-note ornament both survive
//! the real project file path (`ProjectFile::save` → `ProjectFile::load`), and
//! that the reloaded pattern *plays identically* — the playback-time expansion
//! produces the same notes tick-for-tick. The per-processor serde round-trips
//! are covered in `synth_sequencer`; this guards the whole-project save/load.

use pertylizer::project::{GlobalProjectState, ProjectFile};
use synth_core::NormalizedValue;
use synth_sequencer::{
    ArpMode, Arpeggiator, Chord, Duration, ExpansionBuffer, Humanize, NoteProcessor, Ornament,
    OrnamentDynamics, OrnamentPlacement, OrnamentSpacing, PatternTick, Pitch, PitchClass,
    ScaleMask, ScaleQuantize, Song, Velocity,
};

/// Collect every expanded (tick, pitch, velocity-milli, duration) the pattern
/// produces over `ticks`, so two patterns can be compared for identical play.
fn play(
    song: &Song,
    pattern_id: synth_sequencer::PatternId,
    ticks: u32,
) -> Vec<(u32, u8, i32, u32)> {
    let pattern = song.pattern(pattern_id).expect("pattern present");
    let mut buf = ExpansionBuffer::new();
    let mut out = Vec::new();
    for t in 0..ticks {
        pattern.expand_at_tick(
            PatternTick(t),
            |_| true,
            synth_core::Bpm::DEFAULT,
            None,
            &mut buf,
        );
        for n in buf.notes() {
            #[allow(clippy::cast_possible_truncation)]
            out.push((
                t,
                n.pitch.as_midi(),
                (n.velocity.as_f32() * 1000.0) as i32,
                n.duration.map_or(0, |d| d.0),
            ));
        }
    }
    out
}

#[test]
fn rack_and_ornament_survive_project_save_load_and_play_identically() {
    // A pattern with a full rack (quantize → chord → arp → humanize) and a
    // per-note ornament on the source note — every NP feature in one pattern.
    let mut song = Song::new("Persistence");
    let pid = song.create_pattern(Duration(3840));
    {
        let pattern = song.pattern_mut(pid).expect("pattern exists");
        let nid = pattern.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
        let _ = pattern.resize_note(nid, Duration(3840));
        // A distinctive (non-default) ornament so a dropped field is visible.
        if let Some(note) = pattern.note_mut(nid) {
            note.ornament = Some(Ornament {
                count: 3,
                spacing: Duration(80),
                spacing_curve: OrnamentSpacing::Accelerate,
                dynamics: OrnamentDynamics::Crescendo,
                placement: OrnamentPlacement::OnBeat,
                pitch_offset: synth_core::Semitones::new(2.0),
                grace_gate: NormalizedValue::new(0.3),
            });
        }
        let _ = pattern.add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
            root: PitchClass::new(0),
            mask: ScaleMask::MAJOR,
        }));
        let _ = pattern.add_processor(NoteProcessor::Chord(Chord::major()));
        let _ = pattern.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            mode: ArpMode::UpDown,
            octaves: 2,
            ..Arpeggiator::default()
        }));
        let _ = pattern.add_processor(NoteProcessor::Humanize(Humanize {
            velocity: NormalizedValue::new(0.3),
            gate: NormalizedValue::new(0.2),
            seed: 0xABCD_1234,
        }));
    }

    let before = play(&song, pid, 1920);
    assert!(!before.is_empty(), "the rack must produce notes to compare");

    // Round-trip through the real .ptz file path.
    let project = ProjectFile::new(Vec::new(), 0, None, song, GlobalProjectState::default());
    // A path inside a temp directory rather than a `NamedTempFile`, which would
    // hold the destination open: an atomic save replaces by rename, and Windows
    // denies that while another handle is on the file.
    let dir = tempfile::tempdir().expect("temp dir");
    let tmp = dir.path().join("round_trip.ptz");
    project.save(&tmp).expect("save project");
    let reloaded = ProjectFile::load(&tmp).expect("load project");

    // The rack and the ornament config survived verbatim.
    let orig_pattern = project.song.pattern(pid).expect("orig pattern");
    let new_pattern = reloaded.song.pattern(pid).expect("reloaded pattern");
    assert_eq!(
        new_pattern.processors(),
        orig_pattern.processors(),
        "processor rack lost or altered by save/load"
    );
    assert_eq!(
        new_pattern.notes()[0].ornament,
        orig_pattern.notes()[0].ornament,
        "per-note ornament lost or altered by save/load"
    );

    // And it plays identically (seeded humanize + deterministic expansion).
    let after = play(&reloaded.song, pid, 1920);
    assert_eq!(before, after, "reloaded pattern must play identically");
}
