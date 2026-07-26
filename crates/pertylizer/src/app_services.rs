//! Application-level mutation services shared by GUI and remote control paths.

use synth_core::Bpm;
use synth_sequencer::{SharedSong, Tick};

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TempoPointEdit {
    tick: Tick,
    bpm: Bpm,
    ramp: bool,
}

impl TempoPointEdit {
    pub(crate) const fn new(tick: Tick, bpm: Bpm, ramp: bool) -> Self {
        Self { tick, bpm, ramp }
    }
}

/// Serializes song mutations that must behave identically across front ends.
///
/// The service owns no state beyond the shared song reference. Its purpose is
/// to keep locking and multi-step edit semantics out of GUI and MCP adapters.
pub(crate) struct SongMutationService<'a> {
    song: &'a SharedSong,
}

impl<'a> SongMutationService<'a> {
    pub(crate) const fn new(song: &'a SharedSong) -> Self {
        Self { song }
    }

    pub(crate) fn set_tempo_point(&self, edit: TempoPointEdit) {
        self.song
            .write()
            .set_tempo_ramp_at(edit.tick, edit.bpm, edit.ramp);
    }

    pub(crate) fn set_tempo_points(&self, edits: &[TempoPointEdit]) {
        let mut song = self.song.write();
        for edit in edits {
            song.set_tempo_ramp_at(edit.tick, edit.bpm, edit.ramp);
        }
    }

    pub(crate) fn remove_tempo_point(&self, tick: Tick) -> bool {
        self.song.write().remove_tempo_change(tick)
    }

    pub(crate) fn remove_tempo_points(&self, ticks: &[Tick]) -> usize {
        let mut song = self.song.write();
        ticks
            .iter()
            .filter(|&&tick| song.remove_tempo_change(tick))
            .count()
    }

    pub(crate) fn apply_tempo_point(&self, tick: Tick, value: Option<(Bpm, bool)>) {
        if let Some((bpm, ramp)) = value {
            self.set_tempo_point(TempoPointEdit::new(tick, bpm, ramp));
        } else {
            self.remove_tempo_point(tick);
        }
    }

    pub(crate) fn move_tempo_point(&self, old_tick: Tick, new: TempoPointEdit) {
        let mut song = self.song.write();
        if old_tick != new.tick {
            song.remove_tempo_change(old_tick);
        }
        song.set_tempo_ramp_at(new.tick, new.bpm, new.ramp);
    }
}

#[cfg(test)]
mod tests {
    use super::{SongMutationService, TempoPointEdit};
    use synth_core::Bpm;
    use synth_sequencer::{SharedSong, Song, Tick};

    #[test]
    fn moving_a_tempo_point_is_one_coherent_mutation() {
        let song = SharedSong::new(Song::new("Test"));
        let service = SongMutationService::new(&song);
        service.set_tempo_point(TempoPointEdit::new(Tick(960), Bpm::new(130.0), false));

        service.move_tempo_point(
            Tick(960),
            TempoPointEdit::new(Tick(1_920), Bpm::new(140.0), true),
        );

        let song = song.read();
        assert!(
            song.tempo_changes()
                .iter()
                .all(|point| point.tick != Tick(960))
        );
        let moved = song
            .tempo_changes()
            .iter()
            .find(|point| point.tick == Tick(1_920))
            .expect("moved tempo point");
        assert_eq!(moved.bpm, Bpm::new(140.0));
        assert!(moved.ramp);
    }
}
