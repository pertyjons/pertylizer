//! Resolving `--solo-track` / `--mute-track` against a song.
//!
//! The command's mix flags are the *whole* mix state. On load, `mute` and
//! `solo` are cleared on every track and then exactly what the flags say is
//! applied — the file's saved flags are deliberately not honoured.
//!
//! That rule exists so a render is a function of the command plus the input's
//! content. If a saved solo leaked in, two projects with identical audio
//! content but different saved mix state would render differently under the
//! same command, and the receipt could not explain why. Because the override is
//! otherwise invisible, a project whose stored flags were non-default reports a
//! warning instead of silently disagreeing with what the app plays.

use std::fmt;
use std::str::FromStr;

use synth_sequencer::{Song, TrackId};

/// How one flag names a track.
///
/// [`TrackId`] is canonical: it is stable under both renaming and reordering,
/// and the producer of the file already holds it. A name is accepted as a
/// convenience but must resolve to exactly one track, because `create_track`
/// does not enforce unique names.
///
/// Display index is deliberately not a selector — it shifts when tracks are
/// reordered, so the same command would render different audio from the same
/// file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackSelector {
    /// The track whose `id` field is this value.
    Id(TrackId),
    /// The single track with this name.
    Name(String),
}

impl FromStr for TrackSelector {
    type Err = std::convert::Infallible;

    /// Anything that parses as a `TrackId` **is** one. A track named `7` is
    /// therefore not reachable by name, which is the price of not having to
    /// guess which of the two a caller meant — and guessing is what would make
    /// one command line mean different things in different files.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.parse::<u16>()
            .map_or_else(|_| Self::Name(s.to_string()), |id| Self::Id(TrackId(id))))
    }
}

impl fmt::Display for TrackSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(f, "{}", id.0),
            Self::Name(name) => write!(f, "{name}"),
        }
    }
}

/// A mix selection that could not be resolved against the song.
///
/// Every variant is raised before any rendering starts, so a ten-second render
/// never runs on arguments that were never going to work.
#[derive(Debug, thiserror::Error)]
pub enum MixSelectionError {
    /// No track in the song carries this id.
    #[error("no track with id {0} in this project")]
    UnknownTrackId(u16),
    /// No track in the song carries this name.
    #[error("no track named {0:?} in this project")]
    UnknownTrackName(String),
    /// Several tracks share this name, so it does not identify one of them.
    #[error(
        "{count} tracks are named {name:?} (ids {ids}) — \
         select one by id instead, names are not unique"
    )]
    AmbiguousTrackName {
        /// The name that matched more than one track.
        name: String,
        /// How many tracks matched.
        count: usize,
        /// The matching ids, comma-separated, so the caller can pick one.
        ids: String,
    },
    /// The same track was both soloed and muted.
    ///
    /// Refused rather than resolved, because there is no reading of it that
    /// does what the caller meant. Mute wins over solo per
    /// [`SequencerTrack::is_audible`](synth_sequencer::SequencerTrack::is_audible),
    /// but the application's setters are mutually exclusive, so muting the one
    /// soloed track clears the solo entirely — and the render comes back as the
    /// *complement* of the requested solo, with every other track audible.
    #[error(
        "track {id} is both soloed and muted; that would clear the solo \
         and render every other track instead — pass one flag or the other"
    )]
    SoloedAndMuted {
        /// The track named by both flags.
        id: u16,
    },
}

/// The mix flags exactly as the command line gave them.
#[derive(Debug, Clone, Default)]
pub struct MixSelection {
    /// Tracks to solo, in the order given.
    pub solo: Vec<TrackSelector>,
    /// Tracks to mute, in the order given.
    pub mute: Vec<TrackSelector>,
}

impl MixSelection {
    /// True when neither flag was given, i.e. the render is the full mix.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.solo.is_empty() && self.mute.is_empty()
    }
}

/// What [`apply_mix_selection`] actually did, for the result receipt.
#[derive(Debug, Clone, Default)]
pub struct AppliedMix {
    /// Resolved ids of the soloed tracks, in the order given.
    pub soloed: Vec<TrackId>,
    /// Resolved ids of the muted tracks, in the order given.
    pub muted: Vec<TrackId>,
    /// Tracks left audible, in display order. This is the effective mix.
    pub audible: Vec<TrackId>,
    /// Notes worth putting in the receipt — currently the saved-flags override.
    pub warnings: Vec<String>,
}

/// Resolve `selection` against `song`, then make `song`'s mix state say exactly
/// that and nothing else.
///
/// Resolution happens in full before a single flag is written, so a selection
/// that names a missing track leaves `song` untouched.
///
/// Solos are applied before mutes, and both go through
/// [`synth_sequencer::SequencerTrack::set_solo`] /
/// [`set_mute`](synth_sequencer::SequencerTrack::set_mute) rather than writing
/// the fields directly. Those setters are mutually exclusive — soloing clears
/// mute and vice versa — so this keeps the command inside the mix states the
/// application itself can express, and a track named by both flags is refused
/// (see [`MixSelectionError::SoloedAndMuted`]) rather than silently inverting
/// the mix.
///
/// # Errors
///
/// Returns [`MixSelectionError`] if any selector names an unknown id, an
/// unknown name, a name shared by several tracks, or one track twice under
/// opposite flags.
pub fn apply_mix_selection(
    song: &mut Song,
    selection: &MixSelection,
) -> Result<AppliedMix, MixSelectionError> {
    let soloed = resolve_all(song, &selection.solo)?;
    let muted = resolve_all(song, &selection.mute)?;
    if let Some(id) = soloed.iter().find(|id| muted.contains(id)) {
        return Err(MixSelectionError::SoloedAndMuted { id: id.0 });
    }

    let mut warnings = Vec::new();
    if let Some(warning) = saved_mix_warning(song) {
        warnings.push(warning);
    }

    for track in song.tracks_mut() {
        track.mute = false;
        track.solo = false;
    }
    for id in &soloed {
        if let Some(track) = song.track_mut(*id) {
            track.set_solo(true);
        }
    }
    for id in &muted {
        if let Some(track) = song.track_mut(*id) {
            track.set_mute(true);
        }
    }

    let any_solo = song.any_solo();
    let audible = song
        .tracks()
        .filter(|t| t.is_audible(any_solo))
        .map(|t| t.id)
        .collect();

    Ok(AppliedMix {
        soloed,
        muted,
        audible,
        warnings,
    })
}

/// Describe the mix state the command is about to discard, or `None` when the
/// file was already at defaults and there is nothing to explain.
fn saved_mix_warning(song: &Song) -> Option<String> {
    let muted = song.tracks().filter(|t| t.mute).count();
    let soloed = song.tracks().filter(|t| t.solo).count();
    if muted == 0 && soloed == 0 {
        return None;
    }
    Some(format!(
        "the project was saved with {muted} muted and {soloed} soloed track(s); \
         that state was cleared and the render reflects only the command's mix flags"
    ))
}

/// Resolve every selector, or fail on the first one that does not name exactly
/// one track.
fn resolve_all(
    song: &Song,
    selectors: &[TrackSelector],
) -> Result<Vec<TrackId>, MixSelectionError> {
    selectors.iter().map(|s| resolve(song, s)).collect()
}

/// Resolve one selector to the track it names.
fn resolve(song: &Song, selector: &TrackSelector) -> Result<TrackId, MixSelectionError> {
    match selector {
        TrackSelector::Id(id) => song
            .track(*id)
            .map(|t| t.id)
            .ok_or(MixSelectionError::UnknownTrackId(id.0)),
        TrackSelector::Name(name) => {
            let matches: Vec<TrackId> = song
                .tracks()
                .filter(|t| t.name == *name)
                .map(|t| t.id)
                .collect();
            match matches.as_slice() {
                [] => Err(MixSelectionError::UnknownTrackName(name.clone())),
                [only] => Ok(*only),
                many => Err(MixSelectionError::AmbiguousTrackName {
                    name: name.clone(),
                    count: many.len(),
                    ids: many
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_sequencer::SequencerTrack;

    /// Three tracks; two share a name so the ambiguity path has something to
    /// trip over, and ids are not equal to display indices so a resolver that
    /// quietly used the index would be caught.
    fn song_with_tracks() -> Song {
        let mut song = Song::new("mix");
        for (id, name) in [(5_u16, "Bass"), (2, "Lead"), (9, "Lead")] {
            let inserted = song.insert_track(SequencerTrack::new(TrackId(id), name), None);
            assert!(inserted, "test ids must be distinct");
        }
        song
    }

    fn selectors(raw: &[&str]) -> Vec<TrackSelector> {
        raw.iter()
            .map(|s| s.parse().unwrap_or(TrackSelector::Name(String::new())))
            .collect()
    }

    #[test]
    fn a_numeric_argument_is_an_id_and_anything_else_is_a_name() {
        assert_eq!("7".parse(), Ok(TrackSelector::Id(TrackId(7))));
        assert_eq!("Bass".parse(), Ok(TrackSelector::Name("Bass".into())));
        // Out of a u16's range, so it cannot be an id — treated as a name and
        // left to fail at resolution rather than at parse time.
        assert_eq!("70000".parse(), Ok(TrackSelector::Name("70000".into())));
    }

    #[test]
    fn no_flags_leave_every_track_audible() {
        let mut song = song_with_tracks();
        let applied =
            apply_mix_selection(&mut song, &MixSelection::default()).expect("nothing to resolve");
        assert_eq!(applied.audible, vec![TrackId(5), TrackId(2), TrackId(9)]);
        assert!(applied.soloed.is_empty() && applied.muted.is_empty());
        assert!(applied.warnings.is_empty());
    }

    #[test]
    fn solo_by_id_leaves_only_that_track_audible() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: selectors(&["9"]),
            mute: Vec::new(),
        };
        let applied = apply_mix_selection(&mut song, &selection).expect("id 9 exists");
        assert_eq!(applied.soloed, vec![TrackId(9)]);
        assert_eq!(applied.audible, vec![TrackId(9)]);
    }

    #[test]
    fn mute_by_unique_name_leaves_everything_else_audible() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: Vec::new(),
            mute: selectors(&["Bass"]),
        };
        let applied = apply_mix_selection(&mut song, &selection).expect("one track named Bass");
        assert_eq!(applied.muted, vec![TrackId(5)]);
        assert_eq!(applied.audible, vec![TrackId(2), TrackId(9)]);
    }

    /// The saved state is not the render's state, so it must be wiped — and
    /// said out loud, since nothing else would reveal the difference.
    #[test]
    fn a_saved_solo_is_cleared_and_reported() {
        let mut song = song_with_tracks();
        song.track_mut(TrackId(2)).expect("track 2").solo = true;

        let applied =
            apply_mix_selection(&mut song, &MixSelection::default()).expect("nothing to resolve");
        assert_eq!(applied.audible, vec![TrackId(5), TrackId(2), TrackId(9)]);
        assert_eq!(applied.warnings.len(), 1, "{:?}", applied.warnings);
        assert!(
            applied.warnings[0].contains("1 soloed"),
            "{}",
            applied.warnings[0]
        );
        assert!(!song.any_solo());
    }

    /// Soloing and muting one track is not "mute wins" — the setters are
    /// mutually exclusive, so muting the only soloed track clears the solo and
    /// the render becomes the *complement* of what was asked for. Refusing is
    /// the only answer that cannot surprise.
    #[test]
    fn soloing_and_muting_one_track_is_refused() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: selectors(&["5"]),
            mute: selectors(&["5"]),
        };
        assert!(matches!(
            apply_mix_selection(&mut song, &selection),
            Err(MixSelectionError::SoloedAndMuted { id: 5 })
        ));
        // Refused before anything was written, so the song still renders as it
        // did — not as the inverted mix the flags would have produced.
        assert!(song.tracks().all(|t| !t.mute && !t.solo));
    }

    /// Soloing one track while muting a *different* one stays legal: that is a
    /// coherent request and `is_audible` answers it without ambiguity.
    #[test]
    fn soloing_one_track_and_muting_another_is_allowed() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: selectors(&["5"]),
            mute: selectors(&["2"]),
        };
        let applied = apply_mix_selection(&mut song, &selection).expect("distinct tracks");
        assert_eq!(applied.audible, vec![TrackId(5)]);
    }

    #[test]
    fn an_unknown_id_is_rejected() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: selectors(&["42"]),
            mute: Vec::new(),
        };
        assert!(matches!(
            apply_mix_selection(&mut song, &selection),
            Err(MixSelectionError::UnknownTrackId(42))
        ));
    }

    #[test]
    fn an_unknown_name_is_rejected() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: Vec::new(),
            mute: selectors(&["Strings"]),
        };
        assert!(matches!(
            apply_mix_selection(&mut song, &selection),
            Err(MixSelectionError::UnknownTrackName(_))
        ));
    }

    #[test]
    fn a_name_shared_by_two_tracks_is_rejected() {
        let mut song = song_with_tracks();
        let selection = MixSelection {
            solo: selectors(&["Lead"]),
            mute: Vec::new(),
        };
        let err = apply_mix_selection(&mut song, &selection).expect_err("Lead is ambiguous");
        let message = err.to_string();
        assert!(message.contains("2 tracks"), "{message}");
        assert!(message.contains("2, 9"), "{message}");
    }

    /// A selection that cannot be resolved must not half-apply: the song still
    /// has to be renderable as it was.
    #[test]
    fn a_failed_resolution_leaves_the_song_untouched() {
        let mut song = song_with_tracks();
        song.track_mut(TrackId(2)).expect("track 2").solo = true;

        let selection = MixSelection {
            solo: selectors(&["5"]),
            mute: selectors(&["nope"]),
        };
        assert!(apply_mix_selection(&mut song, &selection).is_err());
        assert!(
            song.track(TrackId(2)).expect("track 2").solo,
            "the saved solo must survive a rejected selection"
        );
        assert!(!song.track(TrackId(5)).expect("track 5").solo);
    }
}
