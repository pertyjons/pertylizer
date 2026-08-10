//! Near-miss suggestions for string-keyed identifiers.
//!
//! Module prefixes, parameter ids, tool names and automation-DSL tokens are all
//! free-text on the wire, so a typo is rejected by a lookup that already holds
//! the full list of things it would have accepted. This module turns that list
//! into a suggestion, which is the difference between a failed call plus a
//! discovery round trip and a single self-correcting reply.
//!
//! It lives in `synth_core` because the callers span crates: the MCP bridge's
//! module search and the automation DSL in `pertylizer`, `ModuleId::from_str`
//! in `synth_engine`, the tool dispatch in `synth_mcp`, and the descriptor
//! parameter lookups shared by all of them
//! ([`ModuleDescriptor::param_id_hint`](crate::ModuleDescriptor::param_id_hint)).
//!
//! # Matching
//!
//! Candidates are ranked by how they relate to the needle, best first:
//!
//! | Rank | Relation                                    | Example              |
//! |------|---------------------------------------------|----------------------|
//! | 0    | equal, ignoring case                        | `flt` → `flt`        |
//! | 1    | the candidate *starts with* the needle      | `lim` → `limiter`    |
//! | 2    | either contains the other, at a word start  | `reverb` → `rev`     |
//! | 3+   | within the candidate's edit-distance budget | `fkt` → `flt`        |
//! | last | the candidate contains the needle mid-word  | `iter` → `limiter`   |
//!
//! The two containment rules are deliberately asymmetric. A needle is what a
//! caller typed, so any run of it inside a candidate is *some* signal — but
//! only a run starting at a word boundary is a strong one, so a mid-word run
//! is ranked below every genuine typo. Without that demotion `ent` (a slip for
//! `env`) would "mean" `tsh`, because `ent` sits in the middle of *Transient
//! Shaper* and a containment outranked a one-character edit. In the other
//! direction a candidate is drawn from a catalogue full of three-letter keys,
//! and an unanchored run of three letters lands inside ordinary words far too
//! readily to offer at all — `grain` would "mean" `ain` (Audio Input).
//!
//! That last example is about *containment* only, and the distinction is worth
//! keeping straight: the edit-distance tier below still reaches some of the
//! same wrong answers, because `gain` really is one character from `ain` and
//! `vcf` really is one from `amp`'s key. Nothing here can separate a genuine
//! one-character typo from a one-character coincidence, and the tier is worth
//! more than those cost. What the anchoring removes is the class where a
//! candidate had *no* metric relation to the needle at all and was offered
//! anyway.
//!
//! The prefix rank is what answers the case this module was written for: `lim`
//! is 4 edits from `limiter` and 2 from `lmt`, so no edit-distance threshold
//! tight enough to stay quiet on random input would ever have found it — but it
//! opens the word.
//!
//! The edit-distance threshold **scales with the candidate's length**. A flat
//! ceiling of 3 is discriminating among long names and useless among the
//! three-letter prefixes, where it matches almost every one of them (`osc` is 3
//! edits from `flt`). Substring matching is likewise floored at two or three
//! characters, so a one-character needle does not "match" every candidate that
//! happens to contain it.
//!
//! # What this deliberately does not do
//!
//! It does not relate an abbreviation to the word it abbreviates — `flt` to
//! `filter`, `dly` to `delay`. That relation is real and this codebase is full
//! of it (every module key is a consonant skeleton of its name), but no
//! *general* test for it is precise enough to live here: subsequence matching
//! is nearly free for a long needle, so `dly` "abbreviates"
//! `definitely_not_a_module` through the letters of *definitely* alone, and
//! anchoring the first character does not save it.
//!
//! The fix belongs to the caller, which knows both halves: offer the name as a
//! candidate alongside the key and map the winner back. See
//! [`ModuleType::suggest`](crate::ModuleType::suggest), which does exactly
//! that, and gets `filter` → `flt` exactly rather than approximately.

/// How many near misses a `Did you mean …?` clause names before it stops being
/// a hint. Shared by every caller that appends such a clause, so one phrasing
/// *and* one length hold across all of them. A caller that returns its near
/// misses as a *list field* rather than a clause has room for more and picks its
/// own cap (`search_modules`' `did_you_mean`).
pub const DEFAULT_MAX_HINTS: usize = 3;

/// Longest edit distance any candidate may sit at, however long it is.
const MAX_EDIT_DISTANCE: usize = 3;

/// Rank offset applied to edit-distance matches, so they sort after every
/// exact/prefix/anchored-substring relation. The smallest distance that reaches
/// it is 1, so the lowest edit-distance rank is 3.
const DISTANCE_RANK_BASE: usize = 2;

/// Rank of a needle found *inside* a candidate but not at a word start. Sorted
/// below every edit-distance match, because a mid-word run is a coincidence far
/// more often than it is what the caller meant: `ent` is `env` mistyped, not the
/// middle of *Transient Shaper*.
const MIDWORD_RANK: usize = DISTANCE_RANK_BASE + MAX_EDIT_DISTANCE + 1;

/// Shortest needle that may match by the candidate's leading characters.
const MIN_PREFIX_LEN: usize = 2;

/// Shortest string that may match by containment, in either direction.
const MIN_CONTAINS_LEN: usize = 3;

/// Levenshtein edit distance between `a` and `b`, counted in `char`s.
///
/// Inputs here are identifiers — a handful of characters — so the quadratic
/// table and its allocation are not worth avoiding. Never called from the audio
/// thread.
///
/// Deliberately private: a bare distance plus a hand-picked threshold is exactly
/// the per-caller policy this module was written to replace, and three of them
/// had already drifted apart. Callers rank through [`match_rank`] instead.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    // Two rolling rows rather than the full table: the distance only ever reads
    // the previous row.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// How far a candidate of `len` characters may sit from the needle and still be
/// offered.
///
/// Proportional to the candidate so that a three-letter prefix admits only a
/// single-character slip while a long display name tolerates more. Capped at
/// [`MAX_EDIT_DISTANCE`], and floored at 1 so every candidate admits at least a
/// one-character typo.
fn distance_threshold(len: usize) -> usize {
    (len / 3).clamp(1, MAX_EDIT_DISTANCE)
}

/// Whether `part` occurs inside `whole` at the start of a word — the start of
/// `whole` itself, or just after a non-alphanumeric separator.
///
/// A bare `whole.contains(part)` is far too weak, because the candidates
/// include three-letter keys and a three-letter run lands inside ordinary words
/// by accident: `grain` "means" `ain` (Audio Input) and `sample` "means" `amp`.
/// Anchoring the occurrence keeps every relation that is real — `reverb` →
/// `rev`, `output` → `out`, `envelope` → `env`, `lowpass_filter` → `filter` —
/// and drops the accidents.
fn contains_at_word_start(whole: &str, part: &str) -> bool {
    whole.match_indices(part).any(|(at, _)| {
        whole[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

/// The form [`rank`] compares in: case-folded, and free of the surrounding
/// space a hand-typed argument arrives with. Applied to needle *and* candidate
/// by every entry point, so the same pair ranks the same however it is reached.
fn normalized(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Rank `candidate` against `needle`, or `None` if it is not close enough to
/// offer. Lower is better. Callers must pass both through [`normalized`].
fn rank(needle: &str, candidate: &str) -> Option<usize> {
    let needle_len = needle.chars().count();
    let candidate_len = candidate.chars().count();

    if needle == candidate {
        return Some(0);
    }
    if needle_len >= MIN_PREFIX_LEN && candidate.starts_with(needle) {
        return Some(1);
    }
    // The needle inside the candidate is worth offering wherever it lands, but
    // only a word-start landing outranks a typo (see `MIDWORD_RANK`). The
    // reverse — a short catalogue key buried in a long needle — is only ever
    // offered anchored, because unanchored it is noise.
    let needle_inside = needle_len >= MIN_CONTAINS_LEN && candidate.contains(needle);
    let anchored = (needle_inside && contains_at_word_start(candidate, needle))
        || (candidate_len >= MIN_CONTAINS_LEN && contains_at_word_start(needle, candidate));
    if anchored {
        return Some(2);
    }
    let distance = edit_distance(needle, candidate);
    if distance <= distance_threshold(candidate_len) {
        return Some(DISTANCE_RANK_BASE + distance);
    }
    needle_inside.then_some(MIDWORD_RANK)
}

/// How close `candidate` is to `needle` on the shared ladder, or `None` when it
/// is not close enough to offer. Lower is better; the values themselves are not
/// meaningful beyond their order.
///
/// [`similar`] and [`nearest`] are the entry points when the candidates are
/// plain strings. This one exists for a caller ranking a *record* with more
/// than one spelling — a module type has a key and a display name, a parameter
/// has an id and a name — that wants the best of them so it can answer with the
/// record rather than with whichever field happened to match.
#[must_use]
pub fn match_rank(needle: &str, candidate: &str) -> Option<usize> {
    let needle = normalized(needle);
    if needle.is_empty() {
        return None;
    }
    rank(&needle, &normalized(candidate))
}

/// The candidates closest to `needle`, best first, at most `max_results` of
/// them. Returns an empty vector when nothing is close enough — an empty needle
/// always yields nothing.
///
/// Ties keep the order the candidates arrived in, so a caller iterating a stable
/// catalogue (`ModuleType::all()`, a descriptor's parameters) gets a stable
/// suggestion.
#[must_use]
pub fn similar<'a, I>(needle: &str, candidates: I, max_results: usize) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let needle = normalized(needle);
    if needle.is_empty() || max_results == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &'a str)> = candidates
        .into_iter()
        .filter_map(|candidate| rank(&needle, &normalized(candidate)).map(|rank| (rank, candidate)))
        .collect();
    scored.sort_by_key(|(rank, _)| *rank);
    scored
        .into_iter()
        .take(max_results)
        .map(|(_, candidate)| candidate)
        .collect()
}

/// The records closest to `needle`, best first, at most `max_results` of them —
/// each record ranked by the best any one of its `spellings` achieves.
///
/// [`similar`] cannot serve a caller whose candidates are *records* carrying
/// more than one spelling: a module type has a key and a display name, a
/// parameter an id and a name, and both spellings must be ranked or a
/// recoverable typo goes unanswered. Ranking them as loose strings makes one
/// record occupy two result slots, so the caller must over-request, map the
/// winners back and collapse the duplicates — and still loses slots to records
/// that matched twice. Ranking the record once removes all of that, and the
/// caller answers with whichever spelling its own surface requires.
///
/// Ties keep the order the records arrived in, as in [`similar`].
#[must_use]
pub fn similar_by<'a, T, I, F, S>(
    needle: &str,
    records: I,
    spellings: F,
    max_results: usize,
) -> Vec<T>
where
    T: Copy,
    I: IntoIterator<Item = T>,
    F: Fn(T) -> S,
    S: IntoIterator<Item = &'a str>,
{
    let needle = normalized(needle);
    if needle.is_empty() || max_results == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(usize, T)> = records
        .into_iter()
        .filter_map(|record| {
            spellings(record)
                .into_iter()
                .filter_map(|spelling| rank(&needle, &normalized(spelling)))
                .min()
                .map(|best| (best, record))
        })
        .collect();
    scored.sort_by_key(|(rank, _)| *rank);
    scored
        .into_iter()
        .take(max_results)
        .map(|(_, record)| record)
        .collect()
}

/// The single closest candidate to `needle`, or `None` if none is close enough.
///
/// Ranked in one pass rather than via [`similar`]: the hot caller is
/// [`ModuleType::suggest`](crate::ModuleType::suggest), which offers a key *and*
/// a name for all ~75 types on every rejected token, and sorting that to read
/// one element back is pure waste.
#[must_use]
pub fn nearest<'a, I>(needle: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let needle = normalized(needle);
    if needle.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let Some(rank) = rank(&needle, &normalized(candidate)) else {
            continue;
        };
        // Strictly better only, so a tie keeps the earlier candidate — the same
        // stable-order guarantee `similar` gives.
        if best.is_none_or(|(best_rank, _)| rank < best_rank) {
            best = Some((rank, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// A sentence-ending hint naming the closest candidates, or an empty string when
/// none is close enough.
///
/// Formatted to append directly to an error message that does not end in
/// punctuation: `"unknown module type 'lim'"` + `". Did you mean 'lmt'?"`. Every
/// call site sharing this keeps one phrasing across the whole surface.
#[must_use]
pub fn did_you_mean<'a, I>(needle: &str, candidates: I, max_results: usize) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    hint_from(similar(needle, candidates, max_results))
}

/// The same clause as [`did_you_mean`], for a caller that has already chosen its
/// candidates — typically because it ranked one set of strings (display names)
/// and answers with another (the keys those names map back to).
#[must_use]
pub fn hint_from<'a, I>(matches: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let list: Vec<String> = matches.into_iter().map(|m| format!("'{m}'")).collect();
    if list.is_empty() {
        return String::new();
    }
    format!(". Did you mean {}?", list.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for the module-type prefixes, which are the shortest and
    /// therefore hardest candidates in the codebase.
    const PREFIXES: &[&str] = &["osc", "flt", "lmt", "lfo", "env", "amp", "dly", "spp"];

    #[test]
    fn edit_distance_is_symmetric_and_zero_on_equality() {
        assert_eq!(edit_distance("flt", "flt"), 0);
        assert_eq!(edit_distance("flt", "fkt"), 1);
        assert_eq!(edit_distance("fkt", "flt"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    /// The case this module exists for: the id `lim-1` for a `limiter`, whose
    /// canonical prefix is `lmt`. `lim` is 4 edits from `limiter` and 2 from
    /// `lmt`, so only the prefix relation finds it.
    #[test]
    fn a_name_fragment_suggests_the_full_name() {
        assert_eq!(
            nearest("lim", ["limiter", "filter", "reverb"]),
            Some("limiter")
        );
    }

    #[test]
    fn a_one_character_slip_suggests_the_prefix() {
        assert_eq!(nearest("fkt", PREFIXES.iter().copied()), Some("flt"));
    }

    /// Spelling a module out is **not** this module's job: `filter` relates to
    /// `flt` only as an abbreviation, and no general form of that test is
    /// precise enough to live here (see the module docs). The caller offers the
    /// name as its own candidate instead — `ModuleType::suggest` — so what must
    /// hold here is that the ranking stays *quiet* rather than guessing.
    #[test]
    fn a_spelled_out_name_does_not_guess_at_a_key() {
        for name in ["filter", "delay", "compressor", "distortion"] {
            assert_eq!(
                nearest(name, PREFIXES.iter().copied()),
                None,
                "{name} must not be guessed from the keys alone"
            );
        }
        // Offered the name too, the caller gets an exact answer.
        assert_eq!(nearest("filter", ["flt", "filter"]), Some("filter"));
    }

    /// A long needle threads almost any three-letter consonant skeleton as a
    /// subsequence — `dly` runs straight through *definitely* — so a
    /// subsequence rule would have answered nonsense with a confident guess.
    #[test]
    fn a_long_unrelated_needle_suggests_nothing() {
        assert_eq!(
            nearest("definitely_not_a_module", PREFIXES.iter().copied()),
            None
        );
        assert_eq!(nearest("harmony", PREFIXES.iter().copied()), None);
    }

    /// A three-letter key lands inside ordinary words by accident, so the
    /// needle-contains-candidate direction only counts at a word start.
    /// `sample` is not an `amp`; `grain` is not an `ain`.
    #[test]
    fn a_key_buried_mid_word_is_not_a_match() {
        assert_eq!(nearest("sample", ["amp", "dly"]), None);
        assert_eq!(nearest("grain", ["ain", "dly"]), None);
        // Anchored at a word start, the same relation still counts.
        assert_eq!(nearest("reverb", ["rev", "dly"]), Some("rev"));
        assert_eq!(nearest("lowpass_filter", ["filter", "dly"]), Some("filter"));
    }

    /// A needle buried mid-word in a candidate is a coincidence far more often
    /// than it is the answer, so it must rank *below* a genuine typo: `ent` is
    /// `env` mistyped, not the middle of "Transient Shaper".
    #[test]
    fn a_mid_word_containment_ranks_below_a_one_character_typo() {
        assert_eq!(nearest("ent", ["Transient Shaper", "env"]), Some("env"));
        assert_eq!(nearest("vol", ["Convolver", "vox"]), Some("vox"));
        // Still offered when nothing better exists — it is a demotion, not a
        // rejection.
        assert_eq!(nearest("iter", ["limiter"]), Some("limiter"));
    }

    /// A word-start containment is a strong relation and keeps outranking the
    /// edit-distance tier.
    #[test]
    fn a_word_start_containment_outranks_a_typo() {
        // "pad" is one edit away; "Keyboard Panner" opens a word with it.
        assert_eq!(
            nearest("pan", ["pad", "Keyboard Panner"]),
            Some("Keyboard Panner")
        );
    }

    #[test]
    fn an_exact_match_outranks_every_partial_one() {
        // "amp" is also a substring of "amplifier"; equality must win.
        assert_eq!(nearest("amp", ["amplifier", "amp"]), Some("amp"));
    }

    /// A flat distance-3 ceiling would match nearly every three-letter prefix,
    /// which is worse than saying nothing.
    #[test]
    fn random_input_suggests_nothing() {
        assert!(similar("xyz", PREFIXES.iter().copied(), 3).is_empty());
        assert!(similar("qqqq", PREFIXES.iter().copied(), 3).is_empty());
    }

    #[test]
    fn an_empty_needle_suggests_nothing() {
        assert!(similar("", PREFIXES.iter().copied(), 3).is_empty());
        assert!(similar("   ", PREFIXES.iter().copied(), 3).is_empty());
    }

    /// A single character must not "contain-match" every candidate holding it.
    #[test]
    fn a_single_character_does_not_match_by_containment() {
        assert!(similar("l", ["flt", "lmt", "lfo"], 3).is_empty());
    }

    #[test]
    fn matching_ignores_case_and_surrounding_space() {
        assert_eq!(nearest("  FLT ", PREFIXES.iter().copied()), Some("flt"));
    }

    #[test]
    fn results_are_capped_and_ordered_best_first() {
        let hits = similar("cut", ["cutoff", "cutoff_cv", "resonance"], 2);
        assert_eq!(hits, vec!["cutoff", "cutoff_cv"]);
    }

    #[test]
    fn ties_keep_the_candidate_order() {
        // Both start with "cut", so the catalogue's own order decides.
        assert_eq!(nearest("cut", ["cutoff_cv", "cutoff"]), Some("cutoff_cv"));
    }

    #[test]
    fn did_you_mean_is_empty_when_nothing_is_close() {
        assert_eq!(did_you_mean("xyz", PREFIXES.iter().copied(), 3), "");
    }

    #[test]
    fn did_you_mean_reads_as_a_sentence() {
        assert_eq!(
            did_you_mean("lim", ["limiter"], 3),
            ". Did you mean 'limiter'?"
        );
        assert_eq!(
            did_you_mean("cut", ["cutoff", "cutoff_cv"], 2),
            ". Did you mean 'cutoff', 'cutoff_cv'?"
        );
    }

    #[test]
    fn zero_results_requested_yields_nothing() {
        assert!(similar("flt", PREFIXES.iter().copied(), 0).is_empty());
        assert!(similar_by("flt", PREFIXES.iter().copied(), |p| [p], 0).is_empty());
    }

    /// A record is ranked by its *best* spelling, so a key that says nothing
    /// still reaches its owner through the name — the relation `similar` alone
    /// cannot express.
    #[test]
    fn similar_by_ranks_a_record_by_its_best_spelling() {
        let types = [("lmt", "Limiter"), ("flt", "Filter"), ("rev", "Reverb")];
        let hits = similar_by("lim", types.iter(), |(key, name)| [*key, *name], 3);
        assert_eq!(hits.first().map(|(key, _)| *key), Some("lmt"));
    }

    /// A record matching by *both* spellings must not eat two of the caller's
    /// result slots — the bug the per-caller "rank twice, map back, dedupe"
    /// dance kept re-introducing.
    #[test]
    fn similar_by_spends_one_slot_per_record() {
        // "cutoff" matches by id and by name; both other records match too, so
        // a duplicate-consuming ranking would drop one of them.
        let params = [
            ("cutoff", "Cutoff"),
            ("cutoff_cv", "Cutoff CV"),
            ("cutoff_env", "Cutoff Env"),
        ];
        let hits = similar_by("cutof", params.iter(), |(id, name)| [*id, *name], 3);
        let ids: Vec<&str> = hits.iter().map(|(id, _)| *id).collect();
        for id in ["cutoff", "cutoff_cv", "cutoff_env"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
    }

    #[test]
    fn similar_by_stays_quiet_on_nonsense() {
        let types = [("lmt", "Limiter"), ("flt", "Filter")];
        assert!(similar_by("zzzzzz", types.iter(), |(k, n)| [*k, *n], 3).is_empty());
        assert!(similar_by("", types.iter(), |(k, n)| [*k, *n], 3).is_empty());
    }
}
