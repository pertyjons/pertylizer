//! Round-trip tests for the per-entity `description` fields (Song / Pattern /
//! Track) and track color added in channel-strip Phase 6, plus the
//! track->instrument binding the same phase made mandatory.
//!
//! These guard two things:
//!   1. The new fields survive serde serialization (persist on save/load).
//!   2. Pre-Phase-6 saves that lack the `description` keys still load, with
//!      `description` defaulting to `""` (no `deny_unknown_fields`,
//!      `#[serde(default)]`).

use synth_sequencer::{Duration, SeqInstrumentId, Song, TrackColor};

/// Recursively strip every `key` entry from a JSON value, simulating a save
/// written before the field existed.
fn strip_key(value: &mut serde_json::Value, key: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(key);
            for v in map.values_mut() {
                strip_key(v, key);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                strip_key(v, key);
            }
        }
        _ => {}
    }
}

#[test]
fn descriptions_color_and_track_instrument_binding_round_trip() {
    let mut song = Song::new("Round Trip");
    song.description = "moody synthwave, 110 BPM".to_string();

    let pid = song.create_pattern(Duration(1920));
    song.pattern_mut(pid).expect("pattern exists").description =
        "chorus drop, half-time feel".to_string();

    let tid = song.create_track("Bass");
    {
        let track = song.track_mut(tid).expect("track exists");
        track.description = "sidechain source".to_string();
        track.color = TrackColor::new(0x12, 0x34, 0x56);
        track.instrument = SeqInstrumentId(7);
    }

    let json = serde_json::to_string(&song).expect("serialize song");
    let reloaded: Song = serde_json::from_str(&json).expect("deserialize song");

    assert_eq!(reloaded.description, "moody synthwave, 110 BPM");

    let rp = reloaded
        .patterns()
        .find(|p| p.id == pid)
        .expect("pattern present after round-trip");
    assert_eq!(rp.description, "chorus drop, half-time feel");

    let rt = reloaded
        .tracks()
        .find(|t| t.id == tid)
        .expect("track present after round-trip");
    assert_eq!(rt.description, "sidechain source");
    assert_eq!(rt.color, TrackColor::new(0x12, 0x34, 0x56));
    assert_eq!(
        rt.instrument,
        SeqInstrumentId(7),
        "track->instrument binding lost"
    );
}

#[test]
fn legacy_save_without_description_keys_loads_with_empty_defaults() {
    let mut song = Song::new("Legacy");
    let _pid = song.create_pattern(Duration(1920));
    let tid = song.create_track("Lead");
    song.track_mut(tid).expect("track exists").instrument = SeqInstrumentId(3);

    let mut value = serde_json::to_value(&song).expect("serialize");
    strip_key(&mut value, "description");

    let reloaded: Song =
        serde_json::from_value(value).expect("legacy song (no description keys) must still load");

    assert_eq!(reloaded.description, "");
    assert!(reloaded.patterns().all(|p| p.description.is_empty()));
    assert!(reloaded.tracks().all(|t| t.description.is_empty()));
    // The mandatory binding still survives a legacy load.
    assert_eq!(
        reloaded.tracks().find(|t| t.id == tid).unwrap().instrument,
        SeqInstrumentId(3)
    );
}
