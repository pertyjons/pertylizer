//! Tests for the `completion/complete` candidate filtering.
//!
//! Completion and the near-miss hints share a ranking but not a *policy*, and
//! the difference is the whole point of these: a hint guesses at what was meant
//! and must stay quiet when unsure, while a completion filters a list the
//! caller is looking at and should stay generous.

use super::{filter_completions, patch_slug};

/// Module keys with their display names, the shape `complete_module_type_key`
/// builds.
fn module_types() -> Vec<(String, String)> {
    [
        ("flt", "Filter"),
        ("lmt", "Limiter"),
        ("lfo", "LFO"),
        ("dly", "Delay"),
    ]
    .into_iter()
    .map(|(key, name)| (key.to_string(), name.to_string()))
    .collect()
}

/// Patch slugs with the names they were rendered from.
fn patches() -> Vec<(String, String)> {
    ["Sub Bass", "Spacey Bass", "Grand Piano", "Ambient Keys"]
        .into_iter()
        .map(|name| (patch_slug(name), name.to_string()))
        .collect()
}

#[test]
fn an_empty_value_offers_everything() {
    let all = filter_completions("", &module_types());
    assert_eq!(all, vec!["flt", "lmt", "lfo", "dly"]);
    // A client opening a dropdown before typing sends "" — the one case the
    // hint ranking deliberately answers with nothing.
    assert_eq!(filter_completions("   ", &patches()).len(), 4);
}

/// Two characters into a dropdown means "narrow this", not "I misspelled
/// something". The hint ranking answers `ba` with nothing because a 2-char
/// needle is below its containment floor; completion must not.
#[test]
fn a_short_fragment_still_filters() {
    let hits = filter_completions("ba", &patches());
    assert_eq!(hits, vec!["sub-bass", "spacey-bass"]);
}

#[test]
fn a_prefix_match_outranks_a_mid_string_one() {
    // "sub-bass" and "spacey-bass" open with it; "ambient-keys" only carries it
    // in the middle ("key*s*"), so it comes last rather than being dropped —
    // a dropdown narrows, it does not adjudicate.
    let hits = filter_completions("s", &patches());
    assert_eq!(hits, vec!["sub-bass", "spacey-bass", "ambient-keys"]);

    // "am" opens "ambient-keys" and appears nowhere else.
    assert_eq!(filter_completions("am", &patches()), vec!["ambient-keys"]);
}

/// The value offered is the key, but it is findable by the display name — the
/// relation no string metric connects (`limi` is nowhere near `lmt`).
#[test]
fn a_display_name_finds_a_value_keyed_differently() {
    assert_eq!(filter_completions("limi", &module_types()), vec!["lmt"]);
    assert_eq!(filter_completions("Grand", &patches()), vec!["grand-piano"]);
}

/// Nothing contains a genuine typo, so the shared near-miss ranking takes over.
/// This is the one case where completion falls back to hint behaviour.
#[test]
fn a_typo_falls_back_to_the_near_miss_ranking() {
    assert_eq!(filter_completions("lmit", &module_types()), vec!["lmt"]);
    assert_eq!(filter_completions("fkt", &module_types()), vec!["flt"]);
}

#[test]
fn nonsense_offers_nothing() {
    assert!(filter_completions("zzzqqq", &module_types()).is_empty());
    assert!(filter_completions("zzzqqq", &patches()).is_empty());
}

#[test]
fn matching_ignores_case() {
    assert_eq!(filter_completions("FLT", &module_types()), vec!["flt"]);
    assert_eq!(filter_completions("sUb", &patches()), vec!["sub-bass"]);
}

/// The slug is what the resource URI carries, so a completion that did not
/// render it the same way `read_resource` does would offer values the reader
/// then rejects.
#[test]
fn offered_slugs_are_the_ones_the_reader_accepts() {
    assert_eq!(patch_slug("PWM E-Piano"), "pwm-e-piano");
    for (slug, name) in patches() {
        assert_eq!(slug, patch_slug(&name));
    }
}
