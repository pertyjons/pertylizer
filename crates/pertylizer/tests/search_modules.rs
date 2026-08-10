//! Regression tests for `search_modules` capability discovery.
//!
//! Gap #1 from the MCP-discoverability plan: a capability query for a module
//! that exists (`RingMod`, type key `rng`) returned nothing, making an agent
//! believe Pertylizer had no ring modulator. These assert the weighted token
//! scoring surfaces `rng` for natural phrasings, that score-0 noise is dropped,
//! and that a typo yields a `did_you_mean` hint while a random string does not.

use pertylizer::mcp_bridge::search_module_types;

/// Every phrasing an agent would try for "does this have a ring modulator?"
/// must surface the `rng` module type.
#[test]
fn ring_modulator_queries_surface_rng() {
    for query in ["ring mod", "ring modulator", "multiply two signals"] {
        let result = search_module_types(None, None, None, Some(query));
        let keys: Vec<&str> = result.modules.iter().map(|m| m.type_key.as_str()).collect();
        assert!(
            keys.contains(&"rng"),
            "query {query:?} should surface `rng`, got {keys:?}"
        );
    }
}

/// "ring mod" is a near-exact name hit, so `rng` should rank at the very top,
/// not merely appear somewhere in a padded list.
#[test]
fn ring_mod_ranks_first_for_exact_name() {
    let result = search_module_types(None, None, None, Some("ring mod"));
    assert_eq!(
        result.modules.first().map(|m| m.type_key.as_str()),
        Some("rng"),
        "`rng` should be the top hit for 'ring mod'"
    );
}

/// A query that matches nothing must not pad with low-relevance modules.
#[test]
fn nonsense_query_returns_no_modules() {
    let result = search_module_types(None, None, None, Some("zzqqxx"));
    assert!(
        result.modules.is_empty(),
        "nonsense query should return no modules, got {:?}",
        result
            .modules
            .iter()
            .map(|m| m.type_key.as_str())
            .collect::<Vec<_>>()
    );
}

/// A near-miss typo of "ring mod" should produce a `did_you_mean` pointing at
/// Ring Mod, so the empty result reads as "mis-spelled" not "feature absent".
#[test]
fn typo_yields_did_you_mean() {
    let result = search_module_types(None, None, None, Some("ringmd"));
    assert!(
        result.modules.is_empty(),
        "typo should not produce direct matches"
    );
    assert!(
        result.did_you_mean.iter().any(|s| s.contains("rng")),
        "expected a Ring Mod suggestion, got {:?}",
        result.did_you_mean
    );
}

/// A genuinely random string must not false-match into a suggestion.
#[test]
fn random_string_has_no_suggestion() {
    let result = search_module_types(None, None, None, Some("xyzq"));
    assert!(
        result.did_you_mean.is_empty(),
        "random string should yield no did_you_mean, got {:?}",
        result.did_you_mean
    );
}

/// A `did_you_mean` suggestion must respect the caller's category filter: a typo
/// of an effect's name while filtering for `voice` must not suggest that effect.
#[test]
fn did_you_mean_respects_category_filter() {
    // Unfiltered, the typo "equ"/"eqq" surfaces EQ as a suggestion.
    let unfiltered = search_module_types(None, None, None, Some("eqq"));
    assert!(
        unfiltered.modules.is_empty() && unfiltered.did_you_mean.iter().any(|s| s.contains("equ")),
        "baseline: typo should suggest the EQ effect, got {:?}",
        unfiltered.did_you_mean
    );
    // Filtering for voice modules must drop the effect-only suggestion.
    let filtered = search_module_types(Some("voice"), None, None, Some("eqq"));
    assert!(
        !filtered.did_you_mean.iter().any(|s| s.contains("equ")),
        "EQ is an effect and must not be suggested under category=voice, got {:?}",
        filtered.did_you_mean
    );
}

/// This suggestion and the one a mistyped module id gets are now ranked by the
/// same policy in `synth_core::suggest`, rather than by two hand-tuned
/// thresholds that could drift apart. The same slip must reach the same module
/// through both doors.
#[test]
fn the_search_hint_and_the_module_id_hint_agree() {
    for (typo, key) in [("lmit", "lmt"), ("fkt", "flt"), ("revrb", "rev")] {
        let searched = search_module_types(None, None, None, Some(typo));
        assert!(
            searched.did_you_mean.iter().any(|s| s.contains(key)),
            "search for {typo:?} should suggest {key}, got {:?}",
            searched.did_you_mean
        );

        let parsed = format!("{typo}-1").parse::<synth_engine::ModuleId>();
        let error = parsed.expect_err("the typo must not parse as a module id");
        assert!(
            error.contains(key),
            "parsing {typo:?}-1 should suggest {key}, got: {error}"
        );
    }
}

/// An empty/whitespace query behaves like "no query": filters alone decide, and
/// the full registry comes back (no scoring drop).
#[test]
fn empty_query_returns_all_modules() {
    let all = search_module_types(None, None, None, None);
    let blank = search_module_types(None, None, None, Some("   "));
    assert!(!all.modules.is_empty());
    assert_eq!(all.modules.len(), blank.modules.len());
}
